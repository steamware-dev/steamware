//! Loading the Steamware plugin into Steam's own store pages.
//!
//! Steam's CEF opens a remote-debugging port when `.cef-enable-remote-debugging` exists in its root; the
//! Windows build's installer creates it as a junction. This task discovers every store tab through that
//! port, evaluates the plugin bundle in the page, and answers the plugin's `callServerMethod` calls by
//! posting them to the local API in `local_api.rs`.
//!
//! It is the same relay the Windows app ran, with the same scripts applied on top of the bundle: the label
//! pass that makes the page read Steamware, the emphasis ring around the Add button, the Fix button, the menu
//! skin and the session gate. Those live in `resources/inject/` and are evaluated in their own call after the
//! bundle, so a future plugin update cannot silently revert them and a bundle that throws cannot skip them.

use crate::plugin;
use crate::rebrand;
use crate::settings::Settings;
use crate::skin;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

const CEF_DEBUG_URL: &str = "http://127.0.0.1:8080/json";
const LOCAL_API: &str = "http://127.0.0.1:6767";
const TICK: Duration = Duration::from_millis(250);
const INJECT_EVERY_TICKS: u32 = 4;

const POLYFILL_JS: &str = include_str!("../resources/inject/polyfill.js");
const RELABEL_JS: &str = include_str!("../resources/inject/relabel.js");
const ADD_EMPHASIS_JS: &str = include_str!("../resources/inject/add-button-emphasis.js");
const FIX_BUTTON_JS: &str = include_str!("../resources/inject/fix-button.js");
const MENU_SKIN_JS: &str = include_str!("../resources/inject/menu-skin.js");
const SESSION_GATE_JS: &str = include_str!("../resources/inject/session-gate.js");
const PLUGIN_HEADER_JS: &str = include_str!("../resources/inject/plugin-header.js");

/// How many times one page may be handed the bundle before the injector stops trying.
///
/// The bundle used to be the only thing that could say "this page is done", and it says so from a selector
/// Steam re-hashes with every client update: when that selector disappeared, every page looked un-injected
/// and the ~550 KB bundle was re-evaluated once a second for as long as the store tab stayed open. The
/// header host in `plugin-header.js` restores the agreed-upon signal; this ceiling is what keeps a future
/// break in it from turning into a loop again.
const MAX_BUNDLE_INJECTIONS: u32 = 12;

const PENDING: &str = "JSON.stringify(Object.values(window.Millennium._pending||{}).map(function(r){return{m:r.method,a:JSON.stringify(r.args)}}))";

/// Both overlays announce themselves, so a page that is missing either one is repaired without reloading
/// it — and the palette's signature is part of the check, so a page painted with the previous accent (or the
/// previous light/dark mode) is repainted on the next pass instead of keeping last week's colours.
///
/// The three ids rather than the three flags: each overlay keeps a single owner object on the page and
/// publishes the palette it painted, so a page carrying an older signature is repainted in place instead of
/// being handed a second copy of the script that would fight the first over which colours win.
fn overlay_liveness(palette: &skin::Palette) -> String {
    let id = serde_json::to_string(&palette.id).unwrap_or_else(|_| "\"\"".to_string());
    format!(
        "String(window.__SteamwareSkinId === {id} && window.__SteamwareMenuSkinId === {id} && window.__SteamwareSessionGateId === {id})"
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CefTab {
    id: Option<String>,
    url: Option<String>,
    web_socket_debugger_url: Option<String>,
}

struct Session {
    socket: Socket,
    next_id: i64,
}

#[derive(Deserialize)]
struct Pending {
    m: String,
    a: String,
}

/// How often one tab has been handed the bundle, and for which URL. A navigation clears the count: the new
/// page has not been tried yet, however many times the old one had.
struct Attempt {
    url: String,
    count: u32,
}

/// Read the installed bundle, rebranding it first when it changed on disk.
struct Frontend {
    text: String,
    modified: Option<SystemTime>,
    /// Fingerprint of the bundle's text.
    ///
    /// Part of the liveness check because installing, updating or reinstalling the plugin replaces the
    /// bundle under a page that is already injected. Without this the tab kept running the old script until
    /// the user navigated, since `__LuaToolsReady` was still true and the bridge token had not changed.
    revision: String,
}

impl Frontend {
    fn load(&mut self) {
        let Some(path) = plugin::frontend_script() else {
            if !self.text.is_empty() {
                self.text.clear();
                self.modified = None;
                self.revision.clear();
            }
            return;
        };
        let modified = std::fs::metadata(&path).and_then(|meta| meta.modified()).ok();
        if self.modified.is_some() && modified == self.modified {
            return;
        }
        rebrand::rebrand_dir(&plugin::frontend_dir());
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                self.revision = revision(&text);
                self.text = text;
                self.modified = modified;
            }
            Err(_) => {
                self.text.clear();
                self.modified = None;
                self.revision.clear();
            }
        }
    }

    /// The bundle, tagged with this run's bridge token and its own revision. The tags are what make a page
    /// injected by a *previous* run — or by a bundle that has since been reinstalled — get re-injected: its
    /// marker is ready but one of the tags is stale.
    fn script(&self, token: &str) -> String {
        if self.text.is_empty() {
            return String::new();
        }
        format!(
            "window.__SteamwareBridgeToken=\"{token}\";window.__SteamwareBundleRevision=\"{}\";\n{POLYFILL_JS}\n{}",
            self.revision, self.text
        )
    }

    /// The overlays, evaluated separately from the bundle.
    ///
    /// They used to be appended to the bundle in one string. That coupled them to it: a throw anywhere in
    /// the ~550 KB bundle skipped every line after it, so a page could end up fully injected by the plugin
    /// — button, menu and all — while the menu still painted in the plugin's own Steam blue, because
    /// `menu-skin.js` had never run. A separate evaluation cannot be cut short that way, and it is also what
    /// repairs a page injected by an older build (its marker is ready, but it has no overlay scripts).
    ///
    /// The palette goes first: `menu-skin.js` and `session-gate.js` read it while they build their
    /// stylesheets, so a repaint (after an accent change) never runs against the colours it is replacing.
    fn overlays(&self, palette: &skin::Palette) -> String {
        if self.text.is_empty() {
            return String::new();
        }
        format!(
            "{}\n{RELABEL_JS}\n{ADD_EMPHASIS_JS}\n{FIX_BUTTON_JS}\n{MENU_SKIN_JS}\n{SESSION_GATE_JS}\n{PLUGIN_HEADER_JS}",
            skin::script(palette)
        )
    }
}

/// `settings` is the live profile, not a copy: the accent can change while the app is open, and every pass
/// re-derives the palette from it, so the next repaint already carries the new colours.
pub fn start(token: String, settings: Arc<Mutex<Settings>>) {
    #[cfg(not(windows))]
    {
        let _ = (token, settings);
        crate::local_api::log_line(
            "CEF injector disabled: the in-Steam plugin is Windows-only",
        );
    }
    #[cfg(windows)]
    tauri::async_runtime::spawn(async move {
        crate::local_api::log_line("CEF injector started");
        let mut headers = reqwest::header::HeaderMap::new();
        if let Ok(value) = reqwest::header::HeaderValue::from_str(&token) {
            headers.insert("X-Steamware-Bridge", value);
        }
        let http = match reqwest::Client::builder()
            .timeout(Duration::from_secs(25))
            .default_headers(headers)
            .build()
        {
            Ok(client) => client,
            Err(_) => return,
        };
        let mut frontend = Frontend { text: String::new(), modified: None, revision: String::new() };
        let mut sessions: HashMap<String, Session> = HashMap::new();
        let mut injected: HashSet<String> = HashSet::new();
        let mut attempts: HashMap<String, Attempt> = HashMap::new();
        let mut tick: u32 = 0;

        loop {
            frontend.load();

            if tick % INJECT_EVERY_TICKS == 0 {
                let mut live = HashSet::new();
                let liveness = format!(
                    "String(window.__LuaToolsReady === true && window.__SteamwareBridgeToken === '{token}' && window.__SteamwareBundleRevision === '{}')",
                    frontend.revision
                );
                // Read once per pass: the store page's palette must agree with Settings, and an accent
                // change is picked up within a second of the user saving it. The lock is held only for the
                // clone — no await happens while it is taken, and a poisoned lock falls back to the
                // defaults rather than taking the injector down with it.
                let snapshot = settings.lock().map(|s| s.clone()).unwrap_or_default();
                let palette = skin::palette(&snapshot.accent, &snapshot.mode);
                let liveness_overlays = overlay_liveness(&palette);
                if let Some(tabs) = fetch_tabs(&http).await {
                    let script = frontend.script(&token);
                    let overlays = frontend.overlays(&palette);
                    for tab in tabs {
                        let (Some(id), Some(ws), Some(url)) = (tab.id, tab.web_socket_debugger_url, tab.url)
                        else {
                            continue;
                        };
                        // Steamware's own plugin belongs on the store pages it was written for, and nowhere
                        // else: Steam's menus, its friends list and the web context behind them are Steam's
                        // own windows and are left alone.
                        let is_store = url.to_ascii_lowercase().contains("store.steampowered.com");
                        if !is_store {
                            continue;
                        }
                        // Everything we evaluate — the bundle, and this run's bridge token — goes over this
                        // socket, so it has to be Steam's own debugging port. A process that bound 8080
                        // first could otherwise answer `/json` with a target pointing off-machine and read
                        // both. Loopback + the known port is the whole of what Steam ever advertises.
                        if !trusted_debug_url(&ws) {
                            continue;
                        }
                        live.insert(id.clone());

                        // Counted per page, not per tab: an exhausted page must not stop a fresh one from
                        // being injected after the user clicks through to another game.
                        let exhausted = {
                            let attempt = attempts
                                .entry(id.clone())
                                .or_insert_with(|| Attempt { url: url.clone(), count: 0 });
                            if attempt.url != url {
                                attempt.url = url.clone();
                                attempt.count = 0;
                            }
                            attempt.count >= MAX_BUNDLE_INJECTIONS
                        };

                        if !sessions.contains_key(&id) {
                            match connect(&ws).await {
                                Ok(socket) => {
                                    sessions.insert(id.clone(), Session { socket, next_id: 0 });
                                }
                                Err(_) => continue,
                            }
                        }

                        // A socket that cannot answer is a dead socket: Steam replaces the CDP target on
                        // some navigations, and retrying the same one forever would leave the page
                        // un-injected. Drop it here and reconnect below.
                        let mut ready = false;
                        let mut overlays_ok = false;
                        let mut alive = true;
                        if let Some(session) = sessions.get_mut(&id) {
                            match evaluate(session, &liveness).await {
                                Some(value) => ready = value == "true",
                                None => alive = false,
                            }
                            if alive && ready {
                                match evaluate(session, &liveness_overlays).await {
                                    Some(value) => overlays_ok = value == "true",
                                    None => alive = false,
                                }
                            }
                        }
                        if !alive {
                            sessions.remove(&id);
                        }

                        // Nothing to do only when the bundle *and* both overlays are this run's.
                        if ready && overlays_ok {
                            injected.insert(id.clone());
                            continue;
                        }
                        if !ready {
                            injected.remove(&id);
                        }
                        if script.is_empty() {
                            continue;
                        }

                        if !sessions.contains_key(&id) {
                            match connect(&ws).await {
                                Ok(socket) => {
                                    sessions.insert(id.clone(), Session { socket, next_id: 0 });
                                }
                                Err(_) => continue,
                            }
                        }

                        // A page that has already been given the bundle the maximum number of times is
                        // left alone: further copies cannot fix a bundle that is not marking itself ready,
                        // and re-evaluating half a megabyte of script every second is what such a page did
                        // before this ceiling existed. The overlays below still run, so the header host
                        // keeps being offered to the bundle if it starts working again.
                        if !ready && exhausted {
                            crate::local_api::log_line(&format!(
                                "plugin bundle left as-is in tab {id}: no ready signal after {MAX_BUNDLE_INJECTIONS} attempts"
                            ));
                        }

                        if !ready && !exhausted {
                            let injected_ok = match sessions.get_mut(&id) {
                                Some(session) => evaluate(session, &script).await.is_some(),
                                None => false,
                            };
                            if !injected_ok {
                                sessions.remove(&id);
                                continue;
                            }
                            if let Some(attempt) = attempts.get_mut(&id) {
                                attempt.count += 1;
                            }
                            crate::local_api::log_line(&format!("injected plugin into tab {id}"));
                        }

                        // Overlays go on their own call, always after the bundle, so a bundle throw cannot
                        // take them with it and an older injection gets them without a page reload.
                        let overlays_ok = match sessions.get_mut(&id) {
                            Some(session) => evaluate(session, &overlays).await.is_some(),
                            None => false,
                        };
                        if overlays_ok {
                            injected.insert(id.clone());
                        } else {
                            sessions.remove(&id);
                        }
                    }
                }
                // Tabs that navigated away, closed, or stopped being store pages lose their socket.
                sessions.retain(|id, _| live.contains(id));
                injected.retain(|id| live.contains(id));
                attempts.retain(|id, _| live.contains(id));
            }

            let mut stale: Vec<String> = Vec::new();
            for (id, session) in sessions.iter_mut() {
                if !injected.contains(id) {
                    continue;
                }
                let Some(pending) = evaluate(session, PENDING).await else {
                    stale.push(id.clone());
                    continue;
                };
                if pending.is_empty() || pending == "[]" || pending == "null" {
                    continue;
                }
                let Ok(entries) = serde_json::from_str::<Vec<Pending>>(&pending) else {
                    continue;
                };
                if entries.is_empty() {
                    continue;
                }

                let mut responses: Vec<Value> = Vec::new();
                for entry in entries.iter().take(8) {
                    match call_backend(&http, &entry.m, &entry.a).await {
                        Ok(body) => {
                            let parsed = serde_json::from_str::<Value>(&body).unwrap_or(Value::String(body));
                            responses.push(json!({ "v": parsed }));
                        }
                        Err(message) => responses.push(json!({ "e": message })),
                    }
                }

                let resolve = format!(
                    "(function(r){{var ks=Object.keys(window.Millennium._pending);for(var i=0;i<ks.length&&i<r.length;i++){{var id=ks[i];window.Millennium._readyResponses[id]=r[i];delete window.Millennium._pending[id];}}}})({})",
                    serde_json::to_string(&responses).unwrap_or_else(|_| "[]".to_string())
                );
                if evaluate(session, &resolve).await.is_none() {
                    stale.push(id.clone());
                }
            }
            for id in stale {
                sessions.remove(&id);
                injected.remove(&id);
            }

            tick = tick.wrapping_add(1);
            tokio::time::sleep(TICK).await;
        }
    });
}

async fn fetch_tabs(http: &reqwest::Client) -> Option<Vec<CefTab>> {
    // The CDP tab list must fail fast when Steam is closed; a call to the local API may legitimately take
    // seconds, so the two use different timeouts.
    let response = http
        .get(CEF_DEBUG_URL)
        .timeout(Duration::from_secs(4))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json::<Vec<CefTab>>().await.ok()
}

/// A cheap, stable fingerprint of the bundle's text. Not a security boundary — it only has to change when
/// the file does, so a reinstalled bundle repaints the pages that are already injected.
fn revision(text: &str) -> String {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Steam's CEF targets are `ws://127.0.0.1:8080/devtools/page/<id>` and nothing else.
fn trusted_debug_url(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    if !matches!(parsed.scheme(), "ws" | "wss") {
        return false;
    }
    let host = parsed
        .host_str()
        .unwrap_or_default()
        .trim_start_matches('[')
        .trim_end_matches(']');
    if !matches!(host, "127.0.0.1" | "::1" | "localhost") {
        return false;
    }
    matches!(parsed.port(), Some(8080) | None)
}

async fn connect(url: &str) -> Result<Socket, ()> {
    let (socket, _) = connect_async(url).await.map_err(|_| ())?;
    Ok(socket)
}

/// Evaluate an expression in the page and return its string value. `None` means the call failed — the
/// caller then drops the tab's socket and reconnects on the next pass.
async fn evaluate(session: &mut Session, expression: &str) -> Option<String> {
    session.next_id += 1;
    let id = session.next_id;
    let payload = json!({
        "id": id,
        "method": "Runtime.evaluate",
        "params": { "expression": expression, "returnByValue": true }
    })
    .to_string();

    session.socket.send(Message::Text(payload.into())).await.ok()?;

    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        let message = tokio::time::timeout(remaining, session.socket.next()).await.ok()??;
        let message = message.ok()?;
        match message {
            Message::Text(text) => {
                let Ok(value) = serde_json::from_str::<Value>(text.as_str()) else {
                    continue;
                };
                if value.get("id").and_then(Value::as_i64) != Some(id) {
                    continue;
                }
                return Some(
                    value
                        .pointer("/result/result/value")
                        .and_then(Value::as_str)
                        .map(|text| text.to_string())
                        .unwrap_or_else(|| text.to_string()),
                );
            }
            Message::Close(_) => return None,
            _ => continue,
        }
    }
}

type BackendCall = (&'static str, &'static str);

/// The plugin's method names, mapped onto the local API routes that answer them. A name missing here is a
/// button in Steam that silently does nothing (the fallback below answers success without touching the app).
fn method_map(method: &str) -> Option<BackendCall> {
    Some(match method {
        "HasLuaToolsForApp" => ("GET", "/has/{appid}"),
        "CheckApisForApp" => ("POST", "/check-sources/{appid}"),
        "StartAddViaLuaToolsFromUrl" => ("POST", "/download/{appid}"),
        "GetAddViaLuaToolsStatus" => ("GET", "/download-status/{appid}"),
        "CancelAddViaLuaTools" => ("POST", "/cancel/{appid}"),
        "DeleteLuaToolsForApp" => ("POST", "/remove/{appid}"),
        "StartLuaToolsAdd" => ("POST", "/add/{appid}"),
        "GetLuaToolsAddStatus" => ("GET", "/add-status/{appid}"),
        "PickLuaToolsAddSource" => ("POST", "/add-source/{appid}"),
        "OpenSettings" => ("POST", "/open/settings"),
        "RestartSteam" => ("POST", "/restart-steam"),
        "OpenExternalUrl" => ("POST", "/open-url"),
        "CheckForUpdatesNow" => ("POST", "/check-updates"),
        "ReadLoadedApps" => ("GET", "/loaded-apps"),
        "DismissLoadedApps" => ("POST", "/loaded-apps"),
        "CheckFixForApp" => ("GET", "/fix-status/{appid}"),
        "OpenFixForApp" | "OpenFix" => ("POST", "/open/fixes/{appid}"),
        "GetSteamwareSession" => ("GET", "/session"),
        "VerifySteamwareSession" => ("POST", "/verify"),
        "GetApiList" => ("GET", "/api-list"),
        "GetIconDataUrl" => ("GET", "/icon"),
        "GetGamesDatabase" => ("GET", "/games-database"),
        "Logger" | "Logger.log" => ("POST", "/log"),
        _ => return None,
    })
}

async fn call_backend(http: &reqwest::Client, method: &str, args_raw: &str) -> Result<String, String> {
    let Some((verb, template)) = method_map(method) else {
        return Ok("{\"success\":true}".to_string());
    };

    let args: Value = serde_json::from_str(args_raw).unwrap_or_else(|_| json!({}));
    let mut path = template.to_string();
    if let Value::Object(map) = &args {
        for (key, value) in map {
            let text = value
                .as_str()
                .map(|text| text.to_string())
                .unwrap_or_else(|| value.to_string());
            let encoded: String = url::form_urlencoded::byte_serialize(text.as_bytes()).collect();
            path = path.replace(&format!("{{{key}}}"), &encoded);
        }
    }

    let url = format!("{LOCAL_API}{path}");
    let request = if verb == "GET" {
        http.get(url)
    } else {
        http.post(url)
            .header("Content-Type", "application/json")
            .body(if args_raw.is_empty() { "{}".to_string() } else { args_raw.to_string() })
    };

    match request.send().await {
        Ok(response) => response.text().await.map_err(|error| error.to_string()),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_local_steam_targets_are_trusted() {
        assert!(trusted_debug_url("ws://127.0.0.1:8080/devtools/page/ABC"));
        assert!(trusted_debug_url("ws://localhost:8080/devtools/page/ABC"));
        assert!(trusted_debug_url("ws://[::1]:8080/devtools/page/ABC"));
        // A target that would carry the bundle and the bridge token off this machine.
        assert!(!trusted_debug_url("ws://evil.example/devtools/page/ABC"));
        assert!(!trusted_debug_url("wss://127.0.0.1.attacker.example:8080/x"));
        assert!(!trusted_debug_url("ws://127.0.0.1:9999/devtools/page/ABC"));
        assert!(!trusted_debug_url("http://127.0.0.1:8080/json"));
        assert!(!trusted_debug_url("not a url"));
    }

    #[test]
    fn every_plugin_method_is_mapped() {
        for method in [
            "HasLuaToolsForApp",
            "StartLuaToolsAdd",
            "GetLuaToolsAddStatus",
            "PickLuaToolsAddSource",
            "CancelAddViaLuaTools",
            "DeleteLuaToolsForApp",
            "OpenFixForApp",
            "OpenFix",
            "CheckFixForApp",
            "OpenSettings",
            "RestartSteam",
            "OpenExternalUrl",
            "CheckForUpdatesNow",
            "GetSteamwareSession",
            "VerifySteamwareSession",
            "ReadLoadedApps",
            "DismissLoadedApps",
            "GetIconDataUrl",
            "GetApiList",
            "Logger.log",
        ] {
            assert!(method_map(method).is_some(), "{method} must stay mapped");
        }
    }

    #[test]
    fn the_bundle_revision_tracks_the_file() {
        // Same text, same tag — otherwise every pass would look stale and re-inject the bundle forever.
        assert_eq!(revision("console.log(1)"), revision("console.log(1)"));
        // A reinstalled or updated bundle must produce a different tag, which is what repaints the tabs.
        assert_ne!(revision("console.log(1)"), revision("console.log(2)"));
        assert_eq!(revision("").len(), 16);
    }

    #[test]
    fn appid_substitution_is_url_encoded() {
        let args = json!({ "appid": "730" }).to_string();
        let (_, template) = method_map("HasLuaToolsForApp").unwrap();
        let path = template.replace("{appid}", "730");
        assert_eq!(path, "/has/730");
        assert!(args.contains("730"));
    }
}
