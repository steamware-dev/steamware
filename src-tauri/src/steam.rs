use serde::Serialize;
use std::path::{Path, PathBuf};

/// What the footer and Home page need in one shot.
#[derive(Clone, Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SteamStatus {
    pub installed: bool,
    pub running: bool,
    pub install_path: Option<String>,
    pub library_count: usize,
    pub game_count: usize,
    /// Games Steamware has added to Steam (one lua in `config\stplug-in` each). This is what the library
    /// page lists and what "games you own" means here — NOT the installed count, which is only the subset
    /// Steam has downloaded.
    pub owned_count: usize,
}

/// One installed app, as Steam itself describes it in `appmanifest_<appid>.acf`.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledGame {
    pub app_id: String,
    pub name: String,
    pub install_dir: String,
    pub library: String,
    pub size_on_disk: u64,
    pub last_played: u64,
    pub state_flags: u32,
}

impl InstalledGame {
    /// Steam's own "fully installed" bit. Anything else is queued, updating or partially downloaded.
    pub fn is_installed(&self) -> bool {
        self.state_flags & 4 != 0
    }
}

/* ── Detection ─────────────────────────────────────────────────────────────────────────────────── */

#[cfg(windows)]
fn registry_steam_path() -> Option<PathBuf> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(key) = hkcu.open_subkey("Software\\Valve\\Steam") {
        if let Ok(path) = key.get_value::<String, _>("SteamPath") {
            let candidate = PathBuf::from(path);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    for subkey in ["SOFTWARE\\WOW6432Node\\Valve\\Steam", "SOFTWARE\\Valve\\Steam"] {
        if let Ok(key) = hklm.open_subkey(subkey) {
            if let Ok(path) = key.get_value::<String, _>("InstallPath") {
                let candidate = PathBuf::from(path);
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

#[cfg(not(windows))]
fn registry_steam_path() -> Option<PathBuf> {
    None
}

/// Where Steam keeps its root folder on each platform, best first.
///
/// Windows reads the registry first (`registry_steam_path`); these are the guesses for a machine whose
/// entry is missing or points at a folder that has since moved. The unix entries are the only detection
/// those platforms have: Steam's Linux client stores its root under the user's home (or the Flatpak
/// sandbox), and macOS under Application Support.
fn default_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();

    #[cfg(windows)]
    {
        roots.extend(
            [
                r"C:\Program Files (x86)\Steam",
                r"C:\Program Files\Steam",
                r"D:\Steam",
                r"D:\SteamLibrary",
                r"E:\Steam",
            ]
            .iter()
            .map(PathBuf::from),
        );
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(root) = std::env::var_os("STEAM_ROOT") {
            roots.push(PathBuf::from(root));
        }
        if let Some(home) = std::env::var_os("HOME") {
            let home = PathBuf::from(home);
            roots.push(home.join(".steam").join("steam"));
            roots.push(home.join(".local").join("share").join("Steam"));
            // The Flatpak build keeps its own prefix inside the sandbox's data directory.
            roots.push(
                home.join(".var")
                    .join("app")
                    .join("com.valvesoftware.Steam")
                    .join("data")
                    .join("Steam"),
            );
        }
        roots.push(PathBuf::from("/usr/local/share/Steam"));
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(
                PathBuf::from(home)
                    .join("Library")
                    .join("Application Support")
                    .join("Steam"),
            );
        }
    }

    roots
}

pub fn detect_install(override_path: &str) -> Option<PathBuf> {
    let trimmed = override_path.trim();
    if !trimmed.is_empty() {
        let candidate = PathBuf::from(trimmed);
        if candidate.join("steamapps").exists() || candidate.join("SteamApps").exists() {
            return Some(candidate);
        }
    }

    if let Some(path) = registry_steam_path() {
        return Some(path);
    }

    default_roots()
        .into_iter()
        .find(|path| path.join("steamapps").exists() || path.join("SteamApps").exists())
}

#[cfg(windows)]
pub fn is_running() -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq steam.exe", "/NH", "/FO", "CSV"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .to_ascii_lowercase()
                .contains("steam.exe")
        })
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
pub fn is_running() -> bool {
    // Steam's client process is literally `steam`; reading /proc avoids depending on pgrep being installed.
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    entries.flatten().any(|entry| {
        if !entry.file_name().to_string_lossy().chars().all(|ch| ch.is_ascii_digit()) {
            return false;
        }
        std::fs::read_to_string(entry.path().join("comm"))
            .map(|comm| comm.trim() == "steam")
            .unwrap_or(false)
    })
}

#[cfg(target_os = "macos")]
pub fn is_running() -> bool {
    std::process::Command::new("pgrep")
        .args(["-x", "steam_osx"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub fn is_running() -> bool {
    false
}

/// Start Steam from an already-detected root, detached.
///
/// Shared by the Mode and Plugin installs, which have to restart Steam after replacing files it held open.
pub fn start(root: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        let executable = root.join("steam.exe");
        if !executable.exists() {
            return false;
        }
        return std::process::Command::new(executable)
            .current_dir(root)
            .creation_flags(DETACHED_PROCESS)
            .spawn()
            .is_ok();
    }
    #[cfg(target_os = "linux")]
    {
        let script = root.join("steam.sh");
        let program = if script.is_file() { script } else { PathBuf::from("steam") };
        return std::process::Command::new(program)
            .current_dir(root)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .is_ok();
    }
    #[cfg(target_os = "macos")]
    {
        // The client lives in Steam.app; starting its binary directly loses the bundle's environment.
        let _ = root;
        return std::process::Command::new("open")
            .args(["-a", "Steam"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .is_ok();
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        let _ = root;
        false
    }
}

/// Ask Steam to close, then wait until it actually has.
///
/// `taskkill` rather than a graceful window message: the file handles that block a DLL swap are released
/// by process exit, and a shut-down prompt would leave the install hanging. Returns whether Steam stopped
/// within the timeout, so a caller can refuse to write rather than half-overwrite a running install.
pub fn stop(timeout_seconds: u64) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = std::process::Command::new("taskkill")
            .args(["/IM", "steam.exe", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_seconds.max(1));
        while std::time::Instant::now() < deadline {
            if !is_running() {
                // Handles are released a moment after the process table catches up.
                std::thread::sleep(std::time::Duration::from_millis(900));
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        !is_running()
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let binary = if cfg!(target_os = "macos") { "steam_osx" } else { "steam" };
        let _ = std::process::Command::new("pkill").args(["-x", binary]).output();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_seconds.max(1));
        while std::time::Instant::now() < deadline {
            if !is_running() {
                // Handles are released a moment after the process table catches up.
                std::thread::sleep(std::time::Duration::from_millis(900));
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        !is_running()
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        let _ = timeout_seconds;
        true
    }
}

/* ── VDF / ACF parsing ────────────────────────────────────────────────────────────────────────── */

/// Every double-quoted token in a VDF/ACF document, in order.
///
/// A full VDF parser is unnecessary here: both files are a flat sequence of `"key" "value"` pairs (with
/// nested braces we do not care about), and scanning quoted tokens survives Steam's tabs, CRLF and the
/// occasional unquoted brace.
fn quoted_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut inside = false;
    let mut escaped = false;

    for ch in text.chars() {
        if inside {
            if escaped {
                current.push(ch);
                escaped = false;
            } else if ch == '\\' {
                // Keep the backslash: Windows paths in these files are escaped, and re-emitting them
                // unescaped is what makes Steam paths break.
                current.push(ch);
                escaped = true;
            } else if ch == '"' {
                tokens.push(std::mem::take(&mut current));
                inside = false;
            } else {
                current.push(ch);
            }
        } else if ch == '"' {
            inside = true;
        }
    }
    tokens
}

fn field<'a>(tokens: &'a [String], key: &str) -> Option<&'a str> {
    tokens
        .iter()
        .position(|token| token.eq_ignore_ascii_case(key))
        .and_then(|index| tokens.get(index + 1))
        .map(String::as_str)
}

fn library_paths(steam_root: &Path) -> Vec<PathBuf> {
    let mut libraries = vec![steam_root.to_path_buf()];
    let vdf = steam_root.join("steamapps").join("libraryfolders.vdf");
    let Ok(text) = std::fs::read_to_string(&vdf) else {
        return libraries;
    };

    let tokens = quoted_tokens(&text);
    for (index, token) in tokens.iter().enumerate() {
        if token.eq_ignore_ascii_case("path") {
            if let Some(value) = tokens.get(index + 1) {
                let candidate = PathBuf::from(value.replace("\\\\", "\\"));
                if candidate.join("steamapps").exists() && !libraries.contains(&candidate) {
                    libraries.push(candidate);
                }
            }
        }
    }
    libraries
}

fn parse_acf(text: &str, library: &Path) -> Option<InstalledGame> {
    let tokens = quoted_tokens(text);
    let app_id = field(&tokens, "appid")?.to_string();
    let name = field(&tokens, "name").unwrap_or("Unknown app").to_string();
    let install_dir = field(&tokens, "installdir").unwrap_or("").to_string();
    let size_on_disk = field(&tokens, "SizeOnDisk").and_then(|v| v.parse().ok()).unwrap_or(0);
    let last_played = field(&tokens, "LastPlayed").and_then(|v| v.parse().ok()).unwrap_or(0);
    let state_flags = field(&tokens, "StateFlags").and_then(|v| v.parse().ok()).unwrap_or(4);

    Some(InstalledGame {
        app_id,
        name,
        install_dir,
        library: library.to_string_lossy().to_string(),
        size_on_disk,
        last_played,
        state_flags,
    })
}

/* ── Games Steamware owns ────────────────────────────────────────────────────────────────────── */

/// One game added to Steam by a lua in `config\stplug-in\<appid>.lua`.
///
/// The lua file *is* the record of ownership: Steam loads it, and its absence is why a game that is in
/// someone's Steam account still does not appear. Reading the folder is therefore the only honest source
/// for "what do I own?" — `appmanifest_*.acf` only knows what was downloaded.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnedGame {
    pub app_id: String,
    pub name: String,
    pub lua_path: String,
    /// Unix seconds the lua was written — the "recently added" order. Not the same as last played.
    pub added_at: u64,
    pub installed: bool,
    pub size_on_disk: u64,
    pub last_played: u64,
    pub install_dir: String,
    pub library: String,
    /// Active `addappid` lines in the lua: how many depots this game pins.
    pub depot_count: usize,
}

/// The game name a lua declares, from its `--Gamename ...` comment.
///
/// Both DepotBox and hand-written luas carry it, but the spelling of the comment varies (`--Gamename`,
/// `-- Gamename`, `----Gamename`), so the marker is matched after stripping leading dashes. A lua without
/// one falls back to the name Steam has on disk, then to the App ID.
pub(crate) fn lua_name(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim_start_matches(['-', ' ', '\t']);
        let Some(head) = trimmed.get(..8) else { continue };
        if !head.eq_ignore_ascii_case("gamename") {
            continue;
        }
        let value = trimmed
            .get(8..)
            .unwrap_or("")
            .trim_start_matches([':', '=', ' ', '\t'])
            .trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

/// Active `addappid(...)` lines, i.e. depots that are switched on.
fn active_addappid_lines(text: &str) -> usize {
    text.lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            !trimmed.starts_with("--") && trimmed.contains("addappid(")
        })
        .count()
}

/// How many luas are installed. Cheap: counts directory entries without reading any file.
pub fn owned_count(steam_path: &str) -> usize {
    let Some(dir) = stub_plugin_dir(steam_path) else {
        return 0;
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .map(|name| name.ends_with(".lua"))
                .unwrap_or(false)
        })
        .count()
}

/// Every game Steamware has added, joined with what Steam has installed.
pub fn owned(steam_path: &str) -> Vec<OwnedGame> {
    let Some(dir) = stub_plugin_dir(steam_path) else {
        return Vec::new();
    };
    let installed: std::collections::HashMap<String, InstalledGame> = library(steam_path)
        .into_iter()
        .map(|game| (game.app_id.clone(), game))
        .collect();

    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut games = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        // Only `<appid>.lua`: Steam ignores anything else in this folder, and neither should we.
        let Some(stem) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".lua"))
        else {
            continue;
        };
        if stem.is_empty() || !stem.chars().all(|ch| ch.is_ascii_digit()) {
            continue;
        }

        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let record = installed.get(stem);
        let added_at = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|age| age.as_secs())
            .unwrap_or(0);

        games.push(OwnedGame {
            app_id: stem.to_string(),
            name: lua_name(&text)
                .or_else(|| record.map(|game| game.name.clone()))
                .unwrap_or_else(|| format!("App {stem}")),
            lua_path: path.to_string_lossy().to_string(),
            added_at,
            installed: record.map(|game| game.is_installed()).unwrap_or(false),
            size_on_disk: record.map(|game| game.size_on_disk).unwrap_or(0),
            last_played: record.map(|game| game.last_played).unwrap_or(0),
            install_dir: record.map(|game| game.install_dir.clone()).unwrap_or_default(),
            library: record.map(|game| game.library.clone()).unwrap_or_default(),
            depot_count: active_addappid_lines(&text),
        });
    }

    games.sort_by(|a, b| {
        b.added_at
            .cmp(&a.added_at)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    games
}

/* ── Public surface ──────────────────────────────────────────────────────────────────────────── */

pub fn library(override_path: &str) -> Vec<InstalledGame> {
    let Some(root) = detect_install(override_path) else {
        return Vec::new();
    };

    let mut games = Vec::new();
    for library in library_paths(&root) {
        let apps_dir = library.join("steamapps");
        let Ok(entries) = std::fs::read_dir(&apps_dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_manifest = path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.starts_with("appmanifest_") && name.ends_with(".acf"))
                .unwrap_or(false);
            if !is_manifest {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(&path) {
                if let Some(game) = parse_acf(&text, &library) {
                    games.push(game);
                }
            }
        }
    }

    // One record per app. A stub in one library and Steam's own record in another are the same app, and
    // listing both — or letting the empty one stand in for it — is how an installed game reads as missing.
    let mut by_app: std::collections::HashMap<String, InstalledGame> = std::collections::HashMap::new();
    for game in games {
        let better = match by_app.get(&game.app_id) {
            Some(kept) => record_rank(&game) > record_rank(kept),
            None => true,
        };
        if better {
            by_app.insert(game.app_id.clone(), game);
        }
    }
    let mut games: Vec<InstalledGame> = by_app.into_values().collect();
    games.sort_by(|a, b| b.last_played.cmp(&a.last_played).then_with(|| a.name.cmp(&b.name)));
    games
}

/// Which of two records for one app to show: the installed one, then the one whose files are on disk, then
/// the larger. The first field is the same bit Steam uses, so it always wins.
fn record_rank(game: &InstalledGame) -> (bool, bool, u64) {
    let folder = Path::new(&game.library)
        .join("steamapps")
        .join("common")
        .join(&game.install_dir);
    (
        game.is_installed(),
        !game.install_dir.is_empty() && folder.is_dir(),
        game.size_on_disk,
    )
}

pub fn status(override_path: &str) -> SteamStatus {
    let root = detect_install(override_path);
    let games = library(override_path);
    let library_count = root
        .as_ref()
        .map(|path| library_paths(path).len())
        .unwrap_or(0);

    SteamStatus {
        installed: root.is_some(),
        running: root.is_some() && is_running(),
        install_path: root.clone().map(|path| path.to_string_lossy().to_string()),
        library_count,
        game_count: games.iter().filter(|game| game.is_installed()).count(),
        owned_count: owned_count(override_path),
    }
}

/// Where Steam loads lua plugins from. `None` when Steam is not installed.
pub fn stub_plugin_dir(override_path: &str) -> Option<PathBuf> {
    let root = detect_install(override_path)?;
    let dir = root.join("config").join("stplug-in");
    Some(dir)
}

/* ── Library hand-off ──────────────────────────────────────────────────────────────── */

/// VDF strings are quoted, so a quote or a control character in a game's name would break the file.
fn vdf_clean(value: &str) -> String {
    value.chars().filter(|ch| !ch.is_control() && *ch != '"').collect()
}

/// The state Steamware's library record carries: in the library, not installed, nothing queued.
///
/// `2` is `StateUpdateRequired`, and Steam acts on it — the app goes into the download queue and starts
/// fetching on the next start, which is the "it installs itself when I reopen Steam" report. `1` is
/// `StateUninstalled`: the tile and its Install button are there, and nothing moves until the user presses it.
const STUB_STATE_FLAGS: &str = "1";

/// A folder name Steam will accept as `installdir`.
///
/// It is the game's own name, and a name like `Half-Life: Alyx` carries a character Windows refuses in a path.
/// Steam creates this folder when the user does install, so a raw name makes that fail rather than merely
/// looking wrong.
fn install_dir_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|ch| match ch {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            ch => ch,
        })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches(['.', ' ']);
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        "Steamware app".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Put a game into Steam's library, the way an installed game is there.
///
/// A lua alone only fakes the licence: Steam knows the app is owned but never adds it to the library
/// list, which is why a fetched game used to stay invisible until Steam was restarted *and* the app was
/// downloaded. Two writes fix that, and they do different jobs:
///
///   * `steamapps/appmanifest_<appid>.acf` is the record. Steam reads it at startup, so the entry is still
///     there tomorrow, and it is what the app's own library page lists.
///   * `steam://install/<appid>` is the live part. A running Steam treats it as a real install request, so
///     the tile appears — and Steam's own Install dialog opens — without a restart.
///
/// Returns whether the game is now registered. An app already present in any library is left alone: the
/// stub must never overwrite Steam's real manifest.
pub fn register_in_library(override_path: &str, app_id: &str, name: &str) -> bool {
    if app_id.is_empty() || app_id.len() > 10 || !app_id.chars().all(|ch| ch.is_ascii_digit()) {
        return false;
    }
    let Some(root) = detect_install(override_path) else {
        return false;
    };

    let file = format!("appmanifest_{app_id}.acf");
    if library_paths(&root)
        .iter()
        .any(|library| library.join("steamapps").join(&file).is_file())
    {
        // Steam already has a record for this app. A stub this build wrote earlier — into the root library,
        // before Steam installed the game into another one — is a second record for one app, and Steam can
        // read the empty one and offer Install for a game that is on disk. Ours is the record that goes.
        let records = records_for_app(&root, app_id);
        let refs: Vec<&ManifestRecord> = records.iter().collect();
        drop_shadowed_stubs(&refs);
        return true;
    }

    let apps = root.join("steamapps");
    if std::fs::create_dir_all(&apps).is_err() {
        return false;
    }
    let clean_name = vdf_clean(name.trim());
    let name = if clean_name.is_empty() { format!("App {app_id}") } else { clean_name };
    let install_dir = install_dir_name(&name);
    let text = format!(
        "\"AppState\"\n{{\n\t\"appid\"\t\t\"{app_id}\"\n\t\"Universe\"\t\t\"1\"\n\t\"name\"\t\t\"{name}\"\n\t\"StateFlags\"\t\t\"{STUB_STATE_FLAGS}\"\n\t\"installdir\"\t\t\"{install_dir}\"\n\t\"LastUpdated\"\t\t\"0\"\n\t\"SizeOnDisk\"\t\t\"0\"\n\t\"StagingSize\"\t\t\"0\"\n\t\"buildid\"\t\t\"0\"\n\t\"LastPlayed\"\t\t\"0\"\n\t\"update_result\"\t\t\"0\"\n\t\"BytesToDownload\"\t\t\"0\"\n\t\"BytesDownloaded\"\t\t\"0\"\n}}\n"
    );
    if std::fs::write(apps.join(&file), text).is_err() {
        return false;
    }

    true
}

/// What removing a game from Steam's library found and did.
#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryRemoval {
    /// The stub `register_in_library` wrote is gone, so Steam has no record left to draw a tile from.
    pub removed: bool,
    /// Steam's own record has content behind it. It is left alone — deleting it would strand the files —
    /// so the caller can say the game is still listed as installed.
    pub downloaded: bool,
}

/// Take one app back out of Steam's library by deleting the record Steamware wrote for it.
///
/// `register_in_library` is what makes a game a library *entry*; the lua only fakes the licence. Deleting
/// one without the other is why a removed game used to stay behind as a tile that could never start.
///
/// Only our own stub is deleted: `StateFlags 2` (queued, never downloaded) with nothing on disk. A record
/// Steam wrote for a downloaded game carries the installed bit and a real size, and removing that would
/// tell Steam a game it has on disk had vanished — so it stays and is reported instead.
pub fn unregister_from_library(override_path: &str, app_id: &str) -> LibraryRemoval {
    let mut outcome = LibraryRemoval::default();
    if app_id.is_empty() || app_id.len() > 10 || !app_id.chars().all(|ch| ch.is_ascii_digit()) {
        return outcome;
    }
    let Some(root) = detect_install(override_path) else {
        return outcome;
    };

    let file = format!("appmanifest_{app_id}.acf");
    for library in library_paths(&root) {
        let path = library.join("steamapps").join(&file);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let tokens = quoted_tokens(&text);
        // `1` is what this build writes, `2` is what earlier builds wrote: both are Steamware's own stub.
        let stub = matches!(field(&tokens, "StateFlags"), Some("1") | Some("2"));
        let empty = matches!(field(&tokens, "SizeOnDisk"), None | Some("0"));
        if stub && empty {
            if std::fs::remove_file(&path).is_ok() {
                outcome.removed = true;
            }
        } else {
            outcome.downloaded = true;
        }
    }
    outcome
}

/// One `appmanifest_*.acf`, kept with the file it came from so a stale record can be removed.
struct ManifestRecord {
    path: PathBuf,
    game: InstalledGame,
}

impl ManifestRecord {
    /// Steamware's own record: in the library, nothing downloaded, nothing queued.
    ///
    /// Earlier builds wrote `2`, this one writes `1`; both mean there is a tile and there are no bytes,
    /// which is what makes the record safe to delete. Steam's record for a game it has is never this.
    fn is_stub(&self) -> bool {
        matches!(self.game.state_flags, 1 | 2) && self.game.size_on_disk == 0
    }

    fn is_installed(&self) -> bool {
        self.game.is_installed()
    }

    /// Whether this record's own install folder is on disk and not empty.
    ///
    /// A record that says "nothing here" while the game's files sit in its folder is not ours to rewrite:
    /// it may be a download Steam queued, or a record Steam has not refreshed yet.
    fn has_files(&self) -> bool {
        if self.game.install_dir.is_empty() {
            return false;
        }
        let folder = Path::new(&self.game.library)
            .join("steamapps")
            .join("common")
            .join(&self.game.install_dir);
        std::fs::read_dir(folder)
            .map(|mut entries| entries.next().is_some())
            .unwrap_or(false)
    }
}

fn read_record(path: PathBuf, library: &Path) -> Option<ManifestRecord> {
    let text = std::fs::read_to_string(&path).ok()?;
    parse_acf(&text, library).map(|game| ManifestRecord { path, game })
}

/// Every library record Steam has, across every library.
fn manifest_records(root: &Path) -> Vec<ManifestRecord> {
    let mut records = Vec::new();
    for library in library_paths(root) {
        let apps = library.join("steamapps");
        let Ok(entries) = std::fs::read_dir(&apps) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_manifest = path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.starts_with("appmanifest_") && name.ends_with(".acf"))
                .unwrap_or(false);
            if !is_manifest {
                continue;
            }
            if let Some(record) = read_record(path, &library) {
                records.push(record);
            }
        }
    }
    records
}

/// Every record for one app, without reading the rest of either library.
fn records_for_app(root: &Path, app_id: &str) -> Vec<ManifestRecord> {
    let file = format!("appmanifest_{app_id}.acf");
    let mut records = Vec::new();
    for library in library_paths(root) {
        if let Some(record) = read_record(library.join("steamapps").join(&file), &library) {
            records.push(record);
        }
    }
    records
}

/// Delete Steamware's stubs for one app while Steam has a real record for it.
///
/// Steam keeps one record per app. A stub written into the root library before Steam installed the game
/// into another one is a second record for the same app, and Steam can pick the empty one and offer
/// Install for a game that is on disk. The stub is the record this app wrote, so it is the one that goes;
/// Steam's own record is never touched.
fn drop_shadowed_stubs(records: &[&ManifestRecord]) -> usize {
    if !records.iter().any(|record| record.is_installed()) {
        return 0;
    }
    records
        .iter()
        .filter(|record| record.is_stub())
        .filter(|record| std::fs::remove_file(&record.path).is_ok())
        .count()
}

/// Repair the library records Steamware wrote, before Steam reads them.
///
/// Two jobs, both about a record that describes a game wrongly:
///
///   * A stub left behind for a game Steam has since installed elsewhere is a duplicate record, and Steam
///     can read the empty one and offer Install for a game that is fully on disk. It is removed.
///   * Older builds wrote their stubs as `2` (`StateUpdateRequired`), which Steam starts downloading on
///     its next start. Those become `1` (`StateUninstalled`) only when nothing is behind them: no bytes
///     recorded, and no game folder on disk.
///
/// Returns how many records were repaired.
pub fn reconcile_library(steam_path: &str) -> usize {
    let Some(root) = detect_install(steam_path) else {
        return 0;
    };
    let records = manifest_records(&root);

    let mut by_app: std::collections::HashMap<&str, Vec<&ManifestRecord>> = std::collections::HashMap::new();
    for record in &records {
        by_app.entry(record.game.app_id.as_str()).or_default().push(record);
    }
    let mut repaired: usize = by_app.values().map(|group| drop_shadowed_stubs(group)).sum();

    let Ok(pattern) = regex::Regex::new(r#""StateFlags"(\s*)"2""#) else {
        return repaired;
    };
    for record in &records {
        if !record.path.is_file() || record.game.state_flags != 2 || record.has_files() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&record.path) else {
            continue;
        };
        let tokens = quoted_tokens(&text);
        if field(&tokens, "StateFlags") != Some("2") {
            continue;
        }
        let untouched = |key: &str| matches!(field(&tokens, key), None | Some("0"));
        if !(untouched("SizeOnDisk") && untouched("BytesToDownload") && untouched("BytesDownloaded")) {
            continue;
        }
        let fixed = pattern.replacen(&text, 1, "\"StateFlags\"${1}\"1\"");
        if fixed.as_ref() != text && std::fs::write(&record.path, fixed.as_bytes()).is_ok() {
            repaired += 1;
        }
    }
    repaired
}

/// Where Steam keeps the manifest files the lua files pin to.
///
/// A sibling of `steamapps`, NOT under `config` — that asymmetry is Steam's, not a typo here. Getting it
/// wrong is silent: the lua loads, the depots look added, and every game then fails to verify its content.
pub fn depotcache_dir(override_path: &str) -> Option<PathBuf> {
    let root = detect_install(override_path)?;
    Some(root.join("depotcache"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_tokens_reads_pairs() {
        let tokens = quoted_tokens("\"appid\"\t\t\"730\"\n\"name\"\t\t\"Counter-Strike 2\"");
        assert_eq!(tokens, vec!["appid", "730", "name", "Counter-Strike 2"]);
    }

    #[test]
    fn parse_acf_pulls_the_fields_we_show() {
        let acf = r#"
"AppState"
{
    "appid"        "730"
    "name"         "Counter-Strike 2"
    "StateFlags"   "4"
    "installdir"   "Counter-Strike Global Offensive"
    "SizeOnDisk"   "38294562113"
    "LastPlayed"   "1750000000"
}
"#;
        let game = parse_acf(acf, Path::new(r"D:\SteamLibrary")).expect("parsed");
        assert_eq!(game.app_id, "730");
        assert_eq!(game.name, "Counter-Strike 2");
        assert_eq!(game.install_dir, "Counter-Strike Global Offensive");
        assert_eq!(game.size_on_disk, 38_294_562_113);
        assert!(game.is_installed());
    }

    #[test]
    fn lua_name_reads_every_spelling_of_the_comment() {
        assert_eq!(lua_name("--Gamename Cyberpunk 2077\naddappid(1091500)"), Some("Cyberpunk 2077".to_string()));
        assert_eq!(lua_name("----Gamename   DayZ  "), Some("DayZ".to_string()));
        assert_eq!(lua_name("-- Gamename: Enshrouded"), Some("Enshrouded".to_string()));
        assert_eq!(lua_name("addappid(730)"), None);
    }

    #[test]
    fn active_addappid_ignores_commented_depots() {
        let text = "addappid(730)\n--addappid(731)\n  -- addappid(732)\naddappid(733, 1)";
        assert_eq!(active_addappid_lines(text), 2);
    }

    #[test]
    fn owned_reads_luas_and_joins_installed_state() {
        let root = std::env::temp_dir().join(format!("steamware-owned-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("steamapps")).unwrap();
        std::fs::create_dir_all(root.join("config").join("stplug-in")).unwrap();

        std::fs::write(
            root.join("config").join("stplug-in").join("730.lua"),
            "--Gamename Counter-Strike 2\naddappid(730)\naddappid(731)\n",
        )
        .unwrap();
        // A lua with no name comment, for a game Steam also has on disk.
        std::fs::write(root.join("config").join("stplug-in").join("440.lua"), "addappid(440)\n").unwrap();
        // Not an appid: Steam never loads this, so neither do we.
        std::fs::write(root.join("config").join("stplug-in").join("notes.lua"), "").unwrap();
        std::fs::write(
            root.join("steamapps").join("appmanifest_440.acf"),
            "\"AppState\"\n{\n \"appid\" \"440\"\n \"name\" \"Team Fortress 2\"\n \"StateFlags\" \"4\"\n \"installdir\" \"Team Fortress 2\"\n \"SizeOnDisk\" \"1000\"\n}\n",
        )
        .unwrap();

        let games = owned(&root.to_string_lossy());
        assert_eq!(games.len(), 2, "only numeric luas are games");

        let cs = games.iter().find(|game| game.app_id == "730").expect("cs2");
        assert_eq!(cs.name, "Counter-Strike 2");
        assert_eq!(cs.depot_count, 2);
        assert!(!cs.installed);

        let tf = games.iter().find(|game| game.app_id == "440").expect("tf2");
        assert_eq!(tf.name, "Team Fortress 2", "falls back to Steam's name");
        assert!(tf.installed);
        assert_eq!(tf.size_on_disk, 1000);

        assert_eq!(owned_count(&root.to_string_lossy()), 3, "every lua file counts");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn register_in_library_writes_a_stub_and_never_overwrites() {
        let root = std::env::temp_dir().join(format!("steamware-register-{}", rand::random::<u32>()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("steamapps")).unwrap();
        std::fs::create_dir_all(root.join("config").join("stplug-in")).unwrap();

        assert!(register_in_library(&root.to_string_lossy(), "2668510", "Red Dead Redemption"));
        let path = root.join("steamapps").join("appmanifest_2668510.acf");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"appid\"\t\t\"2668510\""), "{text}");
        assert!(text.contains("\"name\"\t\t\"Red Dead Redemption\""), "{text}");
        assert!(
            text.contains(&format!("\"StateFlags\"\t\t\"{STUB_STATE_FLAGS}\"")),
            "in the library but nothing queued, so Steam never starts the download by itself: {text}"
        );
        assert!(text.contains("\"installdir\"\t\t\"Red Dead Redemption\""), "{text}");

        // A name Windows cannot hold as a folder is the only thing `installdir` is used for at install time.
        assert!(register_in_library(&root.to_string_lossy(), "999002", "A: B? C*"));
        let odd_dir = std::fs::read_to_string(root.join("steamapps").join("appmanifest_999002.acf")).unwrap();
        assert!(odd_dir.contains("\"installdir\"\t\t\"A_ B_ C_\""), "{odd_dir}");

        // Steam's own manifest must win: a second add is a no-op, not a rewrite.
        std::fs::write(&path, "\"AppState\"\n{\n \"appid\" \"2668510\"\n \"StateFlags\" \"4\"\n}\n").unwrap();
        assert!(register_in_library(&root.to_string_lossy(), "2668510", "Red Dead Redemption"));
        assert!(std::fs::read_to_string(&path).unwrap().contains("\"StateFlags\" \"4\""));

        // A quote in a name would end the VDF string early and corrupt the file.
        assert!(register_in_library(&root.to_string_lossy(), "999001", "A \"quoted\" name"));
        let odd = std::fs::read_to_string(root.join("steamapps").join("appmanifest_999001.acf")).unwrap();
        assert!(!odd.contains("A \"quoted\""), "{odd}");

        assert!(!register_in_library(&root.to_string_lossy(), "../evil", "nope"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn unregister_takes_out_the_stub_but_leaves_a_downloaded_game() {
        let root = std::env::temp_dir().join(format!("steamware-unregister-{}", rand::random::<u32>()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("steamapps")).unwrap();
        std::fs::create_dir_all(root.join("config").join("stplug-in")).unwrap();

        // The record an add leaves behind: queued, no bytes, so it is ours to remove.
        assert!(register_in_library(&root.to_string_lossy(), "2668510", "Red Dead Redemption"));
        let stub = root.join("steamapps").join("appmanifest_2668510.acf");
        assert!(stub.is_file());
        let removal = unregister_from_library(&root.to_string_lossy(), "2668510");
        assert!(removal.removed);
        assert!(!removal.downloaded);
        assert!(!stub.exists(), "the library entry has to go with the lua");

        // A second call has nothing left to delete and must not lie about it.
        assert!(!unregister_from_library(&root.to_string_lossy(), "2668510").removed);

        // Steam's own record for a game on disk is left where it is.
        let real = root.join("steamapps").join("appmanifest_440.acf");
        std::fs::write(
            &real,
            "\"AppState\"\n{\n \"appid\" \"440\"\n \"StateFlags\" \"4\"\n \"SizeOnDisk\" \"1000\"\n}\n",
        )
        .unwrap();
        let removal = unregister_from_library(&root.to_string_lossy(), "440");
        assert!(!removal.removed);
        assert!(removal.downloaded, "the caller has to know why it stayed");
        assert!(real.is_file());

        assert!(!unregister_from_library(&root.to_string_lossy(), "../evil").removed);
        std::fs::remove_dir_all(&root).ok();
    }

    /// A second library, as `libraryfolders.vdf` lists one. Paths are escaped the way Valve's files are.
    fn add_library(root: &Path, library: &Path) {
        let escaped = library.to_string_lossy().replace('\\', "\\\\");
        std::fs::write(
            root.join("steamapps").join("libraryfolders.vdf"),
            format!("\"libraryfolders\"\n{{\n \"1\"\n {{\n  \"path\"  \"{escaped}\"\n }}\n}}\n"),
        )
        .unwrap();
    }

    fn write_manifest(path: &Path, app_id: &str, flags: &str, size: &str, install_dir: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path,
            format!(
                "\"AppState\"\n{{\n\t\"appid\"\t\t\"{app_id}\"\n\t\"name\"\t\t\"Test game\"\n\t\"StateFlags\"\t\t\"{flags}\"\n\t\"installdir\"\t\t\"{install_dir}\"\n\t\"SizeOnDisk\"\t\t\"{size}\"\n}}\n"
            ),
        )
        .unwrap();
    }

    /// The reported bug: a game Steam installed into another library read as "install" after a restart,
    /// because Steamware's empty stub in the root library was still a second record for the same app.
    #[test]
    fn a_stub_is_removed_once_steam_has_the_game_in_another_library() {
        let root = std::env::temp_dir().join(format!("steamware-shadow-{}", rand::random::<u32>()));
        let _ = std::fs::remove_dir_all(&root);
        let other = std::env::temp_dir().join(format!("steamware-shadow-lib-{}", rand::random::<u32>()));
        let _ = std::fs::remove_dir_all(&other);
        std::fs::create_dir_all(root.join("steamapps")).unwrap();
        std::fs::create_dir_all(other.join("steamapps")).unwrap();
        add_library(&root, &other);

        // What an add leaves behind: a library entry, nothing downloaded.
        let stub = root.join("steamapps").join("appmanifest_2668510.acf");
        write_manifest(&stub, "2668510", "1", "0", "Red Dead Redemption");
        // Then Steam installs the game into the second library and keeps its own record there.
        let real = other.join("steamapps").join("appmanifest_2668510.acf");
        write_manifest(&real, "2668510", "4", "9000", "Red Dead Redemption");

        assert_eq!(reconcile_library(&root.to_string_lossy()), 1);
        assert!(!stub.exists(), "one app, one record: ours is the one that goes");
        assert!(real.is_file(), "Steam's own record is never touched");

        let games = library(&root.to_string_lossy());
        assert_eq!(games.len(), 1, "the same app must not be listed twice");
        assert!(games[0].is_installed(), "the installed record is the one shown");
        assert_eq!(games[0].size_on_disk, 9000);
        assert_eq!(reconcile_library(&root.to_string_lossy()), 0, "nothing left to repair");

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&other).ok();
    }

    #[test]
    fn adding_a_game_again_clears_the_stub_it_left_in_the_other_library() {
        let root = std::env::temp_dir().join(format!("steamware-shadow-add-{}", rand::random::<u32>()));
        let _ = std::fs::remove_dir_all(&root);
        let other = std::env::temp_dir().join(format!("steamware-shadow-add-lib-{}", rand::random::<u32>()));
        let _ = std::fs::remove_dir_all(&other);
        std::fs::create_dir_all(root.join("steamapps")).unwrap();
        std::fs::create_dir_all(other.join("steamapps")).unwrap();
        add_library(&root, &other);

        let stub = root.join("steamapps").join("appmanifest_2668510.acf");
        write_manifest(&stub, "2668510", "1", "0", "Red Dead Redemption");
        let real = other.join("steamapps").join("appmanifest_2668510.acf");
        write_manifest(&real, "2668510", "4", "9000", "Red Dead Redemption");

        assert!(register_in_library(&root.to_string_lossy(), "2668510", "Red Dead Redemption"));
        assert!(!stub.exists(), "the duplicate is gone as soon as the app is added again");
        assert!(std::fs::read_to_string(&real).unwrap().contains("\"StateFlags\"\t\t\"4\""));

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&other).ok();
    }

    #[test]
    fn a_record_for_a_game_on_disk_is_not_downgraded() {
        let root = std::env::temp_dir().join(format!("steamware-heal-files-{}", rand::random::<u32>()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("steamapps").join("common").join("Test game")).unwrap();
        std::fs::write(
            root.join("steamapps").join("common").join("Test game").join("game.exe"),
            b"MZ",
        )
        .unwrap();

        // Flags say "not installed", but the game is on disk. Rewriting this to `1` is what would turn a
        // downloaded game into an Install button, so the record is left exactly as Steam wrote it.
        let manifest = root.join("steamapps").join("appmanifest_2668510.acf");
        write_manifest(&manifest, "2668510", "2", "0", "Test game");

        assert_eq!(reconcile_library(&root.to_string_lossy()), 0);
        assert!(std::fs::read_to_string(&manifest).unwrap().contains("\"StateFlags\"\t\t\"2\""));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_stub_written_as_update_required_is_repaired_once() {
        let root = std::env::temp_dir().join(format!("steamware-heal-{}", rand::random::<u32>()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("steamapps")).unwrap();

        // What 1.0.3 left behind: `2` with nothing behind it, which Steam downloads on its next start.
        let stub = root.join("steamapps").join("appmanifest_2668510.acf");
        std::fs::write(
            &stub,
            "\"AppState\"\n{\n\t\"appid\"\t\t\"2668510\"\n\t\"StateFlags\"\t\t\"2\"\n\t\"SizeOnDisk\"\t\t\"0\"\n}\n",
        )
        .unwrap();
        // A game Steam is actually downloading keeps its flags: those bytes are Steam's, not ours.
        let real = root.join("steamapps").join("appmanifest_440.acf");
        std::fs::write(
            &real,
            "\"AppState\"\n{\n\t\"appid\"\t\t\"440\"\n\t\"StateFlags\"\t\t\"2\"\n\t\"SizeOnDisk\"\t\t\"4096\"\n}\n",
        )
        .unwrap();

        assert_eq!(reconcile_library(&root.to_string_lossy()), 1);
        let healed = std::fs::read_to_string(&stub).unwrap();
        assert!(healed.contains("\"StateFlags\"\t\t\"1\""), "{healed}");
        assert!(std::fs::read_to_string(&real).unwrap().contains("\"StateFlags\"\t\t\"2\""));
        assert_eq!(reconcile_library(&root.to_string_lossy()), 0, "already repaired");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn library_paths_reads_every_folder() {
        let dir = std::env::temp_dir().join("steamware-vdf-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("steamapps")).unwrap();
        std::fs::write(
            dir.join("steamapps").join("libraryfolders.vdf"),
            "\"libraryfolders\"\n{\n \"0\"\n {\n  \"path\"  \"C:\\\\Steam\"\n }\n}\n",
        )
        .unwrap();

        let paths = library_paths(&dir);
        assert_eq!(paths.len(), 1, "only the root exists on disk");
        std::fs::remove_dir_all(&dir).ok();
    }
}
