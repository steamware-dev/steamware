//! Discord Rich Presence, ported from the old standalone's `DiscordPresenceService`.
//!
//! **What is sent.** Only the activity itself: the page being browsed, or the game being downloaded, with
//! the community invite as a working "Download" button. Nothing is read from the Steam library, no account
//! data, nothing that is not already on screen.
//!
//! **How it works.** Discord's desktop client listens on a local IPC endpoint — a named pipe on Windows, a
//! Unix domain socket on Linux and macOS (`discord-ipc-0`…`-9`) — and speaks its own framing:
//! `[u32 opcode][u32 length][utf8 json]`, little-endian. There is no bot token and no
//! network call — presence is the application's own public identity. The application id ships with the
//! build, exactly as the old WPF build's one-time Discord dialog did; the preset art and button live in
//! that application. A profile may still override it, but the Settings page does not display it. It is spoken by hand rather than through a crate so that nothing extra
//! is compiled in and the reconnect logic is this file's own.
//!
//! **Why it is cheap.** No polling of anything: one thread wakes on a one-second tick, recomputes the card
//! from the page key and the download queue, and writes a frame only when the JSON actually changed, never
//! more often than `MIN_SEND` (which is Discord's documented 5-per-20-seconds ceiling). While Discord is not
//! running it retries on a slow tick; while the setting is off it does nothing at all.
//!
//! The writer and the reader share one handle: Discord sends PING frames and closes the socket of a
//! client that stops answering, which is the usual reason a presence disappears after ten minutes, so the
//! reader answers them and also watches for the READY dispatch (a `SET_ACTIVITY` sent before handshaking
//! finishes is silently dropped by some Discord builds).

use crate::downloads::Downloads;
use crate::settings::Settings;
use serde_json::{json, Value};
use std::fs::File;
use std::io::{ErrorKind, Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(windows)]
use std::os::windows::io::AsRawHandle;

const IMAGE_KEY: &str = "steamware";
const IMAGE_TEXT: &str = "Steamware";
const BUTTON_LABEL: &str = "Download";
const BUTTON_URL: &str = "https://discord.gg/steamtools";

/// Discord allows five activity updates per twenty seconds; four seconds is exactly inside that, and the
/// dedupe above means a quiet app sends nothing at all.
const MIN_SEND: Duration = Duration::from_secs(4);
/// How often a connect is retried while Discord is closed, or after Discord has restarted. The open fails
/// instantly when nothing is listening, so a short interval costs a syscall and buys a profile that comes back
/// within a second or two instead of staying blank until the next page change.
const RETRY: Duration = Duration::from_secs(2);
/// Discord refuses more than five activity updates per twenty seconds. Counted rather than assumed, because a
/// page change is allowed to jump the queue and a burst of them must still stay inside the ceiling.
const BUDGET_WINDOW: Duration = Duration::from_secs(20);
const BUDGET_SENDS: usize = 5;
const TICK: Duration = Duration::from_millis(1000);
/// How long a fetch stays announced as "Getting … for free". A manifest download is over in about a second,
/// so a card derived from the queue would flash by unseen; this is the window that makes the click itself
/// visible on a profile, and it ends on its own.
const GETTING: Duration = Duration::from_secs(30);
/// How long to wait for Discord's READY after the handshake before giving the pipe up and trying a fresh one.
const READY_WAIT: Duration = Duration::from_secs(3);
const MAX_SOCKET: u8 = 10;
const MAX_FRAME_BYTES: usize = 64 << 10;

const OP_HANDSHAKE: u32 = 0;
const OP_FRAME: u32 = 1;
const OP_CLOSE: u32 = 2;
const OP_PING: u32 = 3;
const OP_PONG: u32 = 4;

/// Discord caps these; it silently rejects over-long text rather than truncating it.
const DETAILS_MAX: usize = 128;
const LABEL_MAX: usize = 32;
const TITLE_MAX: usize = 90;

/// What the app tells Discord it is doing. Shared with the command that reports navigations.
pub struct Presence {
    page: Mutex<String>,
    /// The game just asked for, and the moment that announcement stops outranking the page. Written by a
    /// fetch, cleared by time.
    hold: Mutex<Option<(String, u64)>>,
    started: u64,
}

impl Default for Presence {
    fn default() -> Self {
        Self::new()
    }
}

impl Presence {
    pub fn new() -> Self {
        Self { page: Mutex::new("home".to_string()), hold: Mutex::new(None), started: now_ms() }
    }

    /// The page a navigation just opened, as the shell's own route key ("store", "library", …).
    pub fn set_page(&self, page: &str) {
        let page = page.trim().to_ascii_lowercase();
        if page.is_empty() {
            return;
        }
        if let Ok(mut current) = self.page.lock() {
            *current = page;
        }
    }

    fn page(&self) -> String {
        self.page.lock().map(|page| page.clone()).unwrap_or_else(|_| "home".to_string())
    }

    /// Announce a fetch for the next `GETTING`: "Getting <game> for free", whatever page is open and whether
    /// or not the download has already landed. A second fetch replaces the first rather than queueing.
    pub fn hold_game(&self, title: &str) {
        let title = title.trim();
        if title.is_empty() {
            return;
        }
        if let Ok(mut hold) = self.hold.lock() {
            *hold = Some((title.to_string(), now_ms() + GETTING.as_millis() as u64));
        }
    }

    /// The title being announced, if its window is still open. An expired hold is dropped here, so the card
    /// simply stops mentioning it.
    fn held_at(&self, now: u64) -> Option<String> {
        let mut hold = self.hold.lock().ok()?;
        match hold.as_ref() {
            Some((title, until)) if now < *until => Some(title.clone()),
            Some(_) => {
                *hold = None;
                None
            }
            None => None,
        }
    }

    fn held(&self) -> Option<String> {
        self.held_at(now_ms())
    }
}

/// Start the writer. One thread for the life of the app; it follows the setting live, so turning Discord
/// Rich Presence on in Settings starts publishing within a second and turning it off clears the card.
pub fn start(settings: Arc<Mutex<Settings>>, downloads: Arc<Downloads>, presence: Arc<Presence>) {
    std::thread::Builder::new()
        .name("steamware-presence".to_string())
        .spawn(move || run(settings, downloads, presence))
        .ok();
}

/// Ask Discord whether it accepts this application id, without publishing an activity.
///
/// The Settings "Test" button uses this: a wrong id otherwise looks exactly like a working one until the
/// card never appears. It opens a short-lived pipe connection, waits for READY, and closes it again.
pub fn check(app_id: &str) -> Result<(), String> {
    let app_id = app_id.trim();
    if !crate::settings::plausible_discord_app_id(app_id) {
        return Err("A Discord application ID is 17–20 digits.".to_string());
    }
    let mut connection = Connection::open(app_id)
        .ok_or_else(|| "Discord is not running, or its RPC pipe is unavailable.".to_string())?;
    if !connection.wait_ready() {
        return Err("Discord did not accept that application ID. Check it in the Developer Portal.".to_string());
    }
    // Dropped rather than closed explicitly: the pipe closes with the handle.
    Ok(())
}

fn run(settings: Arc<Mutex<Settings>>, downloads: Arc<Downloads>, presence: Arc<Presence>) {
    let mut connection: Option<Connection> = None;
    let mut last_json = String::new();
    let mut last_sent = Instant::now() - MIN_SEND;
    let mut last_attempt: Option<Instant> = None;
    // Consecutive handshakes that opened a pipe and never saw READY. A wrong application id, or a
    // Discord that is still starting, otherwise retries every two seconds and burns Discord's
    // per-client connection ceiling until it rejects everything.
    let mut handshake_failures: u32 = 0;
    let mut sample: Option<(String, u64, Instant)> = None;
    // The subject of the card, as opposed to its numbers: "Browsing Store", "Getting Strayed for free".
    let mut last_topic = String::new();
    // When the last few updates went out, for Discord's ceiling.
    let mut sends: Vec<Instant> = Vec::new();

    loop {
        std::thread::sleep(TICK);

        let (enabled, app_id) = settings
            .lock()
            .map(|settings| (settings.discord_rpc, settings.discord_app_id.trim().to_string()))
            .unwrap_or((false, String::new()));
        // No application id means the old app's "still needs setup": stay dormant rather than opening a
        // pipe Discord will never answer. The Settings page holds the one-time id.
        if !enabled || !crate::settings::plausible_discord_app_id(&app_id) {
            // Dropping the connection closes the pipe; Discord sees the disconnect. A polite CLOSE frame
            // is not worth a write that can block while the setting is being turned off.
            connection = None;
            last_json.clear();
            last_topic.clear();
            sample = None;
            sends.clear();
            continue;
        }

        // A socket Discord closed behind us — the client restarted, the user signed out, the machine slept —
        // is dropped here instead of being kept until a write happens to fail. Forgetting the card at the same
        // time is what makes the reconnect repaint it rather than suppressing it as "unchanged".
        let dead = match connection.as_mut() {
            Some(open) => {
                open.pump();
                open.closed
            }
            None => false,
        };
        if dead {
            connection = None;
            last_json.clear();
        }

        // Changing the application id must not keep publishing under the old one.
        if connection.as_ref().map(|open| open.app_id != app_id).unwrap_or(false) {
            connection = None;
            last_json.clear();
            last_topic.clear();
        }

        // Reconnecting happens on the tick rather than only when there is news to send: otherwise a Discord
        // restart left the profile empty until the user changed page. With Discord closed the open fails at
        // once, so this is one syscall every `RETRY` and not a wait.
        let mut reconnected = false;
        let retry = if handshake_failures == 0 {
            RETRY
        } else {
            // Back off to at most a minute. A closed Discord still retries quickly, because opening a
            // missing pipe fails instantly; only a pipe that accepts a handshake and stays silent counts.
            Duration::from_secs((RETRY.as_secs() << handshake_failures.min(5)).min(60))
        };
        if connection.is_none() && last_attempt.map_or(true, |at| at.elapsed() >= retry) {
            last_attempt = Some(Instant::now());
            if let Some(mut open) = Connection::open(&app_id) {
                if open.wait_ready() {
                    connection = Some(open);
                    handshake_failures = 0;
                    last_json.clear();
                    reconnected = true;
                } else {
                    handshake_failures = handshake_failures.saturating_add(1);
                }
            }
        }

        let active = downloads
            .list()
            .into_iter()
            .find(|item| item.status == "queued" || item.status == "downloading");

        let rate = match active.as_ref() {
            Some(item) => measure(item, &mut sample),
            None => {
                sample = None;
                None
            }
        };

        // A fetch outranks the page for `GETTING`, then stops: the page (or the download, if it is still
        // running) takes the card back on its own.
        let card = match presence.held().as_deref() {
            Some(title) => hold_activity(title, presence.started),
            None => activity(
                &presence.page(),
                active.as_ref().map(|item| Active {
                    title: &item.title,
                    kind: &item.kind,
                    status: &item.status,
                    percent: percent(item.received, item.total),
                    rate,
                }),
                presence.started,
            ),
        };
        let json = card.to_string();
        if json == last_json {
            continue;
        }
        let Some(open) = connection.as_mut() else {
            continue;
        };

        // A change of subject is sent at once — switching page, a fetch starting, a fetch ending — because it
        // is the thing the user is watching for. A progress tick is held to one every `MIN_SEND`. Either way
        // the write has to fit inside Discord's five-per-twenty-seconds ceiling, so an attempted burst falls
        // back to the next tick with the update still pending rather than being lost.
        let topic = card["details"].as_str().unwrap_or_default().to_string();
        let urgent = reconnected || topic != last_topic;
        if !urgent && last_sent.elapsed() < MIN_SEND {
            continue;
        }
        sends.retain(|at| at.elapsed() < BUDGET_WINDOW);
        if sends.len() >= BUDGET_SENDS {
            continue;
        }

        if !open.set_activity(&card) {
            connection = None;
            last_json.clear();
            continue;
        }
        sends.push(Instant::now());
        last_json = json;
        last_sent = Instant::now();
        last_topic = topic;
    }
}

/* ── The card ──────────────────────────────────────────────────────────────────────────────────── */

/// The queue item being announced, flattened so the card builder stays pure and testable.
pub(crate) struct Active<'a> {
    pub title: &'a str,
    pub kind: &'a str,
    pub status: &'a str,
    pub percent: Option<u32>,
    pub rate: Option<String>,
}

/// Page key → the card's first line. The keys are the shell's routes, so the wording is pinned here rather
/// than in the frontend.
fn page_details(page: &str) -> &'static str {
    match page {
        "store" => "Browsing Store",
        "library" => "Browsing Library",
        "builds" => "Browsing Builds",
        "downloads" => "Browsing Downloads",
        "fixes" => "Browsing Fixes",
        "plugin" => "Browsing Plugins",
        "mode" => "Browsing Modes",
        "settings" => "Browsing Settings",
        _ => "Browsing Home",
    }
}

/// The download line. Games read "…for free"; a fix package is a tool, not a game, so the claim is left off
/// rather than attached to something it does not describe.
fn download_details(title: &str, kind: &str) -> String {
    let name = clip(title, TITLE_MAX);
    if kind == "fix" {
        format!("Downloading {name}")
    } else {
        format!("Downloading {name} for free")
    }
}

fn download_state(active: &Active) -> Option<String> {
    match active.status {
        "queued" => Some("Queued".to_string()),
        _ => match (active.percent, active.rate.as_deref()) {
            (Some(percent), Some(rate)) => Some(format!("{percent}% · {rate}")),
            (Some(percent), None) => Some(format!("{percent}%")),
            (None, Some(rate)) => Some(rate.to_string()),
            (None, None) => None,
        },
    }
}

fn activity(page: &str, active: Option<Active>, started: u64) -> Value {
    let (details, state) = match active {
        Some(active) => (download_details(active.title, active.kind), download_state(&active)),
        None => (page_details(page).to_string(), None),
    };
    card(&details, state, started)
}

/// The fetch announcement: the game's name and what it costs, and nothing else. No state line, because a
/// download that has not started reporting yet has nothing true to put there.
fn hold_activity(title: &str, started: u64) -> Value {
    card(&getting_details(title), None, started)
}

fn getting_details(title: &str) -> String {
    format!("Getting {} for free", clip(title, TITLE_MAX))
}

fn card(details: &str, state: Option<String>, started: u64) -> Value {
    let mut card = json!({
        "details": clip(&details, DETAILS_MAX),
        // Elapsed time on every state, not just downloads: a card that keeps counting reads as live.
        "timestamps": { "start": started },
        "assets": { "large_image": IMAGE_KEY, "large_text": IMAGE_TEXT },
    });

    if let Some(state) = state {
        let state = clip(&state, DETAILS_MAX);
        if !state.is_empty() {
            card["state"] = Value::String(state);
        }
    }

    // Exactly one button, always the same destination: the community invite, which is where a download
    // actually comes from. Discord wants a 2–32 character label, so it is clipped rather than trusted.
    let label = clip(BUTTON_LABEL, LABEL_MAX);
    if label.len() >= 2 {
        card["buttons"] = json!([{ "label": label, "url": BUTTON_URL }]);
    }

    card
}

fn percent(received: u64, total: u64) -> Option<u32> {
    (total > 0).then(|| ((received.saturating_mul(100) / total).min(100)) as u32)
}

/// Trim and hard-cap a string at Discord's field limit.
fn clip(value: &str, max: usize) -> String {
    let text = value.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut clipped: String = text.chars().take(max.saturating_sub(1)).collect();
    clipped.push('…');
    clipped
}

/// Bytes per second for a download, from the change since the last tick. A single tick is too noisy, so the
/// baseline only moves when a rate was actually produced.
fn measure(
    item: &crate::downloads::DownloadItem,
    sample: &mut Option<(String, u64, Instant)>,
) -> Option<String> {
    let now = Instant::now();
    let previous = sample.replace((item.id.clone(), item.received, now))?;
    if previous.0 != item.id {
        *sample = Some((item.id.clone(), item.received, now));
        return None;
    }
    let seconds = now.duration_since(previous.2).as_secs_f64();
    if seconds < 0.4 {
        // Not enough elapsed time to mean anything: hold the old baseline for the next tick.
        *sample = Some(previous);
        return None;
    }
    let delta = item.received.saturating_sub(previous.1) as f64;
    (delta > 0.0).then(|| human_rate(delta / seconds))
}

fn human_rate(bytes_per_second: f64) -> String {
    const UNITS: [&str; 4] = ["B/s", "KB/s", "MB/s", "GB/s"];
    let mut value = bytes_per_second;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/* ── The pipe ──────────────────────────────────────────────────────────────────────────────────── */

/// One live Discord socket, owned by one thread and one handle.
///
/// The pipe is read and written through the same `File`. A duplicated handle with a thread parked in
/// `ReadFile` was the original design, and it does not work against Discord: the write for a card then
/// waits in `WriteFile` while the pending read sits on the same pipe instance. Peeking for input first
/// keeps the socket one-directional at a time and makes every write complete.
struct Connection {
    /// The application this socket handshook as, so a Settings change can replace it.
    app_id: String,
    pipe: File,
    ready: bool,
    /// A socket Discord closed or that failed a read; the loop drops it and reconnects.
    closed: bool,
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    /// How many bytes are waiting on a pipe, without blocking. Zero means "nothing to read yet".
    fn PeekNamedPipe(
        handle: *mut core::ffi::c_void,
        buffer: *mut core::ffi::c_void,
        size: u32,
        read: *mut u32,
        available: *mut u32,
        left: *mut u32,
    ) -> i32;
}

/// Bytes waiting on the pipe, or `None` when the handle is no longer usable.
#[cfg(windows)]
fn peek_bytes(pipe: &File) -> Option<u32> {
    let mut available: u32 = 0;
    let ok = unsafe {
        PeekNamedPipe(
            pipe.as_raw_handle() as *mut core::ffi::c_void,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            &mut available,
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(available)
}

/// Bytes waiting on a unix socket, or `None` when the socket is no longer usable.
///
/// Unix domain sockets have no `PeekNamedPipe`; `FIONREAD` asks the kernel the same question without
/// consuming anything, which is what lets the reader wait for a whole eight-byte header before reading.
#[cfg(unix)]
fn peek_bytes(pipe: &File) -> Option<u32> {
    use std::os::unix::io::AsRawFd;

    let mut available: libc::c_int = 0;
    let result = unsafe { libc::ioctl(pipe.as_raw_fd(), libc::FIONREAD, &mut available) };
    if result == 0 {
        Some(available.max(0) as u32)
    } else {
        None
    }
}

/// Open Discord's endpoint. Windows exposes a named pipe, which `OpenOptions` opens like a file; a unix host
/// has a socket file instead, where `open(2)` fails with `ENXIO` — it has to be dialled, then handed to the
/// same `File` so the framing above stays one implementation.
fn dial(path: &str) -> std::io::Result<File> {
    #[cfg(windows)]
    {
        std::fs::OpenOptions::new().read(true).write(true).open(path)
    }
    #[cfg(unix)]
    {
        use std::os::unix::io::{FromRawFd, IntoRawFd};
        let socket = std::os::unix::net::UnixStream::connect(path)?;
        // `File` is only the descriptor; the socket keeps its read/write and `FIONREAD` behaviour.
        Ok(unsafe { File::from_raw_fd(socket.into_raw_fd()) })
    }
}

impl Connection {
    fn open(app_id: &str) -> Option<Connection> {
        (0..MAX_SOCKET).find_map(|index| Self::open_at(index, app_id))
    }

    fn open_at(index: u8, app_id: &str) -> Option<Connection> {
        socket_paths(index).into_iter().find_map(|path| Self::connect(&path, app_id))
    }

    /// Dial one endpoint and send the handshake. Split from `open_at` so the unix test can point it at a
    /// temporary socket instead of the real Discord path.
    fn connect(path: &str, app_id: &str) -> Option<Connection> {
        let mut pipe = dial(path).ok()?;
        send(&mut pipe, OP_HANDSHAKE, &json!({ "v": 1, "client_id": app_id })).ok()?;
        Some(Connection { app_id: app_id.to_string(), pipe, ready: false, closed: false })
    }

    /// Read whatever Discord has sent, answering PINGs. Returns at once when the pipe is quiet.
    fn pump(&mut self) {
        loop {
            match peek_bytes(&self.pipe) {
                None => {
                    self.closed = true;
                    return;
                }
                // A header is eight bytes; wait for all of it before reading, or the read blocks.
                Some(available) if available >= 8 => {}
                Some(_) => return,
            }
            let Some((opcode, payload)) = read_frame(&mut self.pipe) else {
                self.closed = true;
                return;
            };
            match opcode {
                OP_PING => {
                    if send(&mut self.pipe, OP_PONG, &payload).is_err() {
                        self.closed = true;
                        return;
                    }
                }
                OP_FRAME => {
                    if is_ready(&payload) {
                        self.ready = true;
                    }
                }
                OP_CLOSE => {
                    self.closed = true;
                    return;
                }
                _ => {}
            }
        }
    }

    /// Wait for Discord's READY. Sending an activity before every lively build has finished handshaking is
    /// handled inconsistently, and a dropped `SET_ACTIVITY` is invisible — the card simply never appears.
    fn wait_ready(&mut self) -> bool {
        let deadline = Instant::now() + READY_WAIT;
        while Instant::now() < deadline {
            self.pump();
            if self.ready {
                return true;
            }
            if self.closed {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    fn set_activity(&mut self, activity: &Value) -> bool {
        self.pump();
        if self.closed {
            return false;
        }
        let frame = json!({
            "cmd": "SET_ACTIVITY",
            "args": { "pid": std::process::id(), "activity": activity },
            "nonce": nonce(),
        });
        send(&mut self.pipe, OP_FRAME, &frame).is_ok()
    }
}

#[cfg(windows)]
fn socket_paths(index: u8) -> Vec<String> {
    vec![format!(r"\\.\pipe\discord-ipc-{index}")]
}

/// Every place Discord's desktop client may have put its socket on a unix host.
///
/// A native install listens in `$XDG_RUNTIME_DIR` (or `/tmp` when that is unset), the Flatpak build keeps
/// its socket inside its sandbox runtime directory and the Snap build under its own. They are tried in
/// that order: the first one that opens belongs to the client that is actually running.
#[cfg(not(windows))]
fn socket_paths(index: u8) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    let mut push = |path: String| {
        if !paths.contains(&path) {
            paths.push(path);
        }
    };
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        let dir = dir.trim_end_matches('/');
        push(format!("{dir}/discord-ipc-{index}"));
        push(format!("{dir}/app/com.discordapp.Discord/discord-ipc-{index}"));
        push(format!("{dir}/snap.discord/discord-ipc-{index}"));
    }
    if let Ok(dir) = std::env::var("TMPDIR") {
        push(format!("{}/discord-ipc-{index}", dir.trim_end_matches('/')));
    }
    push(format!("/tmp/discord-ipc-{index}"));
    paths
}

fn send(pipe: &mut File, opcode: u32, payload: &Value) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(payload).map_err(std::io::Error::other)?;
    write_frame(pipe, opcode, &bytes)
}

/// `[u32 opcode][u32 length][json]`, little-endian, both frames and the handshake.
fn write_frame<W: Write>(pipe: &mut W, opcode: u32, bytes: &[u8]) -> std::io::Result<()> {
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(std::io::Error::other("presence frame too large"));
    }
    // One write for the whole frame. Discord reads the header and the body as a unit off its pipe, and
    // a body written separately from its header can leave the frame half-delivered — the write then
    // waits for a reader that is waiting for a length it has not been given yet.
    let mut frame = Vec::with_capacity(8 + bytes.len());
    frame.extend_from_slice(&opcode.to_le_bytes());
    frame.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    frame.extend_from_slice(bytes);
    pipe.write_all(&frame)?;
    // No flush: `File::flush` is `FlushFileBuffers`, which on a pipe waits for the other end to drain
    // the buffer — a handshake Discord answers at once, but a `SET_ACTIVITY` it dispatches lazily, so the
    // call can sit there holding the writer lock while the card is already on its way.
    Ok(())
}

/// Discord sends the handshake result as a DISPATCH event whose event name is `evt`. Older examples use
/// `ev`, which this build used to check; that made every handshake time out and threw the pipe away.
fn is_ready(payload: &Value) -> bool {
    payload.get("evt").and_then(Value::as_str) == Some("READY")
}

/// One incoming frame. `None` means the socket closed or is speaking something other than this protocol,
/// which is the reader's cue to give up on the connection.
fn read_frame<R: Read>(pipe: &mut R) -> Option<(u32, Value)> {
    let mut header = [0u8; 8];
    match pipe.read_exact(&mut header) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::Interrupted => return None,
        Err(_) => return None,
    }
    let opcode = u32::from_le_bytes(header[..4].try_into().ok()?);
    let length = u32::from_le_bytes(header[4..].try_into().ok()?) as usize;
    if length > MAX_FRAME_BYTES {
        return None;
    }
    let mut payload = vec![0u8; length];
    pipe.read_exact(&mut payload).ok()?;
    // A PING body is not JSON, so an unparseable frame is carried as null rather than dropped.
    Some((opcode, serde_json::from_slice(&payload).unwrap_or(Value::Null)))
}

fn nonce() -> String {
    // A monotonic-enough value that is not a dependency: Discord only echoes it back.
    format!("{:x}", now_ms())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn active<'a>(title: &'a str, kind: &'a str, status: &'a str) -> Active<'a> {
        Active { title, kind, status, percent: Some(42), rate: Some("3.5 MB/s".to_string()) }
    }

    #[test]
    fn a_page_card_names_the_page_and_carries_one_button() {
        let card = activity("store", None, 1_700_000_000_000);
        assert_eq!(card["details"], "Browsing Store");
        assert_eq!(card["assets"]["large_image"], "steamware");
        assert_eq!(card["timestamps"]["start"], 1_700_000_000_000u64);
        assert_eq!(card["buttons"][0]["label"], "Download");
        assert_eq!(card["buttons"][0]["url"], BUTTON_URL);
        assert!(card.get("state").is_none(), "a browsed page has no second line");
    }

    #[test]
    fn every_route_maps_to_a_line_and_unknown_ones_fall_back_to_home() {
        for (route, line) in [
            ("home", "Browsing Home"),
            ("store", "Browsing Store"),
            ("library", "Browsing Library"),
            ("builds", "Browsing Builds"),
            ("downloads", "Browsing Downloads"),
            ("fixes", "Browsing Fixes"),
            ("plugin", "Browsing Plugins"),
            ("mode", "Browsing Modes"),
            ("settings", "Browsing Settings"),
        ] {
            assert_eq!(page_details(route), line);
        }
        assert_eq!(page_details("nonsense"), "Browsing Home");
    }

    #[test]
    fn a_fetch_announces_the_game_itself() {
        let card = hold_activity("Strayed", 1_700_000_000_000);
        assert_eq!(card["details"], "Getting Strayed for free");
        assert_eq!(card["timestamps"]["start"], 1_700_000_000_000u64);
        assert_eq!(card["buttons"][0]["url"], BUTTON_URL);
        assert!(card.get("state").is_none(), "nothing true to say beside it yet");
    }

    #[test]
    fn a_hold_outranks_the_page_and_expires_by_itself() {
        let presence = Presence::new();
        presence.set_page("library");
        presence.hold_game("Strayed");
        assert_eq!(presence.held(), Some("Strayed".to_string()));
        // Still inside the window a moment before it closes, gone a moment after.
        let set = presence.hold.lock().unwrap().clone().unwrap().1;
        assert!(presence.held_at(set - 1).is_some());
        assert!(presence.held_at(set).is_none());
        assert_eq!(presence.page(), "library", "the page survives the announcement");
    }

    #[test]
    fn an_empty_title_is_not_announced() {
        let presence = Presence::new();
        presence.hold_game("   ");
        assert!(presence.held().is_none());
    }

    #[test]
    fn a_download_outranks_the_page_and_reads_as_a_percentage() {
        let card = activity("store", Some(active("Strayed", "manifest", "downloading")), 5);
        assert_eq!(card["details"], "Downloading Strayed for free");
        assert_eq!(card["state"], "42% · 3.5 MB/s");
    }

    #[test]
    fn a_queued_download_says_so_instead_of_showing_a_rate() {
        let card = activity("home", Some(active("Strayed", "manifest", "queued")), 5);
        assert_eq!(card["state"], "Queued");
    }

    #[test]
    fn a_fix_is_a_tool_and_is_not_advertised_as_a_free_game() {
        assert_eq!(download_details("Steamless", "fix"), "Downloading Steamless");
        assert_eq!(download_details("Strayed", "manifest"), "Downloading Strayed for free");
    }

    #[test]
    fn long_fields_are_clipped_to_discords_limits() {
        let title = "z".repeat(300);
        let details = download_details(&title, "manifest");
        assert!(details.chars().count() <= TITLE_MAX + 30);
        let card = activity("home", None, 0);
        assert!(card["details"].as_str().unwrap().chars().count() <= DETAILS_MAX);
    }

    #[test]
    fn a_total_of_zero_does_not_divide_by_it() {
        assert_eq!(percent(0, 0), None);
        assert_eq!(percent(50, 200), Some(25));
        assert_eq!(percent(200, 200), Some(100));
    }

    #[test]
    fn rates_read_in_the_unit_they_deserve() {
        assert_eq!(human_rate(512.0), "512 B/s");
        assert_eq!(human_rate(2048.0), "2.0 KB/s");
        assert_eq!(human_rate(1024.0 * 1024.0 * 3.5), "3.5 MB/s");
    }

    #[test]
    fn frames_are_little_endian_opcode_then_length() {
        let mut buffer = Vec::new();
        write_frame(&mut buffer, OP_FRAME, b"{\"a\":1}").unwrap();
        assert_eq!(&buffer[..4], &[1, 0, 0, 0]);
        assert_eq!(&buffer[4..8], &[7, 0, 0, 0]);
        assert_eq!(&buffer[8..], b"{\"a\":1}");
    }

    #[test]
    fn a_frame_round_trips_through_the_reader() {
        let mut buffer = Vec::new();
        write_frame(&mut buffer, OP_PING, b"{}").unwrap();
        let (opcode, payload) = read_frame(&mut buffer.as_slice()).unwrap();
        assert_eq!(opcode, OP_PING);
        assert_eq!(payload, json!({}));
    }

    #[test]
    fn a_truncated_frame_is_a_closed_socket_not_a_panic() {
        let mut truncated = vec![1u8, 0, 0, 0, 64, 0, 0, 0];
        truncated.extend_from_slice(b"{\"partial");
        assert!(read_frame(&mut truncated.as_slice()).is_none());
    }

    #[test]
    fn a_ping_body_that_is_not_json_is_still_carried() {
        let mut buffer = Vec::new();
        write_frame(&mut buffer, OP_PING, b"not-json").unwrap();
        let (opcode, payload) = read_frame(&mut buffer.as_slice()).unwrap();
        assert_eq!(opcode, OP_PING);
        assert!(payload.is_null());
    }

    #[test]
    fn there_is_always_a_discord_socket_to_try() {
        let paths = socket_paths(0);
        assert!(!paths.is_empty());
        assert!(paths.iter().all(|path| path.ends_with("discord-ipc-0")));
        // A second socket index must not collide with the first.
        assert_ne!(socket_paths(1), paths);
    }

    #[test]
    fn ready_is_the_evt_field_discord_sends() {
        assert!(is_ready(&json!({ "cmd": "DISPATCH", "evt": "READY" })));
        assert!(!is_ready(&json!({ "ev": "READY" })));
        assert!(!is_ready(&json!({ "evt": "ERROR" })));
    }

    /// The one check the unit tests cannot make: whether a running Discord accepts the bundled application
    /// and renders its card. Ignored by default because it needs the desktop client on this machine.
    /// `cargo test --release --lib live_discord -- --ignored --nocapture`
    #[test]
    #[ignore = "talks to the running Discord client"]
    fn live_discord_accepts_the_bundled_application_and_its_card() {
        let app = crate::settings::DEFAULT_DISCORD_APP_ID;
        // One connection for the whole probe: Discord accepts a single RPC client per process, and a
        // second pipe opened by the same test would wait forever for the first one's slot.
        let mut open = Connection::open(app).expect("a Discord RPC pipe");
        assert!(open.wait_ready(), "Discord sends READY for the bundled application");
        assert!(
            open.set_activity(&activity("store", None, now_ms())),
            "the store card is written to Discord"
        );
    }

    #[test]
    fn the_page_key_is_normalised_so_shouting_at_it_is_harmless() {
        let presence = Presence::new();
        presence.set_page("Store");
        assert_eq!(presence.page(), "store");
        presence.set_page("   ");
        assert_eq!(presence.page(), "store", "an empty key leaves the last page alone");
    }

    /// `open(2)` on a unix socket file fails with ENXIO; the endpoint has to be dialled. A temporary listener
    /// stands in for Discord, so this also pins the handshake framing.
    #[cfg(unix)]
    #[test]
    fn a_unix_socket_is_dialled_and_answers_with_a_handshake() {
        use std::io::Read as _;
        use std::os::unix::net::UnixListener;

        let path = std::env::temp_dir().join(format!("sw-presence-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).expect("binds a temporary socket");

        let connection = Connection::connect(path.to_str().unwrap(), "12345678901234567")
            .expect("dials the socket");
        let (mut accepted, _) = listener.accept().expect("accepts the handshake");

        let mut header = [0u8; 8];
        accepted.read_exact(&mut header).unwrap();
        assert_eq!(u32::from_le_bytes(header[..4].try_into().unwrap()), OP_HANDSHAKE);
        let length = u32::from_le_bytes(header[4..].try_into().unwrap()) as usize;
        let mut body = vec![0u8; length];
        accepted.read_exact(&mut body).unwrap();
        let hello: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(hello["v"], 1);
        assert_eq!(hello["client_id"], "12345678901234567");

        drop(connection);
        let _ = std::fs::remove_file(&path);
    }
}
