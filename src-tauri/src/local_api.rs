//! The local bridge the in-Steam plugin talks to.
//!
//! The store page is HTTPS and this process is not, so the page cannot `fetch()` it directly (mixed content).
//! Every call instead rides the CDP bridge in `injector.rs`, which makes the HTTP request from here. The
//! routes and field names below are the plugin's published contract — `luatools.js` polls shapes like
//! `{ checking, sourcesLoaded, sources: [{ downloading, progress }], installStatus }` by field name, so the
//! JSON is written out explicitly rather than derived from a Rust struct.
//!
//! Bound to loopback only. It is not exposed to the network, and every action it exposes is one the user
//! could trigger from the app's own window.

use crate::api::ApiClient;
use crate::downloads::{Downloads, NewDownload};
use crate::lua;
use crate::rebrand;
use crate::settings::{self, Settings};
use crate::steam;
use base64::Engine;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tiny_http::{Header, Response, Server};

const PORT: u16 = 6767;
const ADD_TIMEOUT: Duration = Duration::from_secs(20 * 60);

#[derive(Clone, Default)]
struct AddState {
    title: String,
    checking: bool,
    sources_loaded: bool,
    source_status: String,
    downloading: bool,
    progress: f64,
    indeterminate: bool,
    installed: bool,
    install_status: Option<String>,
    error: Option<String>,
    game_unavailable: bool,
}

/// An add is in flight while either half of it is: the availability/source check before the queue starts,
/// or the archive download itself. A finished, failed or cancelled add is not in flight and can be retried.
fn add_in_progress(state: &AddState) -> bool {
    state.checking || state.downloading
}

struct Inner {
    app: AppHandle,
    api: Arc<ApiClient>,
    downloads: Arc<Downloads>,
    settings: Arc<Mutex<Settings>>,
    adds: Mutex<HashMap<String, AddState>>,
    legacy: Mutex<HashMap<String, String>>,
    fixes: Mutex<Option<(Instant, Vec<String>)>>,
    /// Only the injector knows this. The plugin never calls this server directly (mixed content blocks it),
    /// so a request without the header is a web page on this machine — not the bridge.
    token: String,
}

/// A per-run secret for the bridge. Loopback alone is not an access control: any local page can reach it.
pub fn bridge_token() -> String {
    format!("{:032x}", rand::random::<u128>())
}

pub struct LocalApi;

pub(crate) fn log_line(message: &str) {
    use std::io::Write;
    let dir = settings::app_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("plugin-api.log"))
    {
        let _ = writeln!(file, "{message}");
    }
}

fn download_dir(settings: &Settings) -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(&settings.download_dir);
    if dir.as_os_str().is_empty() {
        settings::app_dir().join("downloads")
    } else {
        dir
    }
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("static header")
}

/// No CORS headers on purpose. The bridge is called by the injector's CDP script, which runs same-process and
/// needs no permission, so a wildcard `Access-Control-Allow-Origin` would only ever help a web page on this
/// machine read the responses. Without it the browser refuses to hand them over.
fn respond(request: tiny_http::Request, status: u16, body: String) {
    let response = Response::from_string(body)
        .with_status_code(status)
        .with_header(header("Content-Type", "application/json; charset=utf-8"))
        .with_header(header("Cache-Control", "no-store"))
        .with_header(header("X-Content-Type-Options", "nosniff"));
    let _ = request.respond(response);
}

/// Constant-time token comparison: a byte-by-byte `!=` leaks how much of a guess was right.
fn token_matches(supplied: &str, expected: &str) -> bool {
    let supplied = supplied.as_bytes();
    let expected = expected.as_bytes();
    if expected.is_empty() || supplied.len() != expected.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in supplied.iter().zip(expected) {
        diff |= a ^ b;
    }
    diff == 0
}

/// Route ids become file names under Steam's plug-in folder (`<id>.lua`) and a segment of the upstream
/// manifest URL, so only digits reach a handler. `..` or a backslash in an id would otherwise let a crafted
/// request name a file outside Steam.
fn id_segment(app_id: &str) -> bool {
    !app_id.is_empty() && app_id.len() <= 10 && app_id.chars().all(|c| c.is_ascii_digit())
}

const ID_ROUTES: [&str; 10] = [
    "has",
    "add",
    "add-status",
    "add-source",
    "check-sources",
    "download",
    "download-status",
    "cancel",
    "remove",
    "fix-status",
];

fn replace_ignore_case(text: &str, from: &str, to: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let needle = from.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    while let Some(found) = lower[cursor..].find(&needle) {
        let start = cursor + found;
        out.push_str(&text[cursor..start]);
        out.push_str(to);
        cursor = start + from.len();
    }
    out.push_str(&text[cursor..]);
    out
}

fn body_json(body: &str) -> Value {
    serde_json::from_str(body).unwrap_or_else(|_| json!({}))
}

fn string_field(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or_default().trim().to_string()
}

impl LocalApi {
    /// Start the bridge. A busy port is not fatal: the plugin then simply has no backend, which is exactly
    /// what the old Windows app did when its listener failed to start.
    pub fn start(
        app: AppHandle,
        api: Arc<ApiClient>,
        downloads: Arc<Downloads>,
        settings: Arc<Mutex<Settings>>,
        token: String,
    ) {
        let inner = Arc::new(Inner {
            app,
            api,
            downloads,
            settings,
            adds: Mutex::new(HashMap::new()),
            legacy: Mutex::new(HashMap::new()),
            fixes: Mutex::new(None),
            token,
        });
        std::thread::Builder::new()
            .name("steamware-plugin-api".to_string())
            .spawn(move || match Server::http(("127.0.0.1", PORT)) {
                Ok(server) => {
                    log_line("plugin API listening on http://127.0.0.1:6767");
                    for request in server.incoming_requests() {
                        let inner = inner.clone();
                        std::thread::spawn(move || handle(inner, request));
                    }
                }
                Err(error) => log_line(&format!("plugin API could not bind 127.0.0.1:{PORT}: {error}")),
            })
            .ok();
    }
}

fn handle(inner: Arc<Inner>, mut request: tiny_http::Request) {
    let supplied = request
        .headers()
        .iter()
        .find(|header| header.field.equiv("X-Steamware-Bridge"))
        .map(|header| header.value.as_str().to_string())
        .unwrap_or_default();
    if !token_matches(&supplied, &inner.token) {
        respond(request, 403, json!({ "success": false, "error": "Forbidden" }).to_string());
        return;
    }

    let method = request.method().as_str().to_ascii_uppercase();
    let url = request.url().to_string();
    let path = url.split('?').next().unwrap_or("/").trim_end_matches('/').to_string();
    let path = if path.is_empty() { "/".to_string() } else { path };

    if method == "OPTIONS" {
        respond(request, 204, String::new());
        return;
    }

    // Bounded: this socket is reachable by any local process, and nothing the plugin sends is large.
    let mut body = String::new();
    let mut reader = std::io::Read::take(request.as_reader(), 1_048_576u64);
    let _ = std::io::Read::read_to_string(&mut reader, &mut body);

    if path != "/loaded-apps" && !path.starts_with("/add-status/") && !path.starts_with("/has/") {
        log_line(&format!("HTTP {method} {path}"));
    }

    let segments: Vec<&str> = path.trim_start_matches('/').split('/').filter(|s| !s.is_empty()).collect();

    let bad_id = match segments.as_slice() {
        [first, id] if ID_ROUTES.contains(first) => !id_segment(id),
        ["open", "fixes", id] => !id_segment(id),
        _ => false,
    };
    if bad_id {
        respond(request, 400, json!({ "success": false, "error": "Invalid app id" }).to_string());
        return;
    }

    let (status, payload) = match (method.as_str(), segments.as_slice()) {
        ("GET", ["has", id]) => inner.has(id),
        ("POST", ["add", id]) => inner.add(id, &body),
        ("GET", ["add-status", id]) => inner.add_status(id),
        ("POST", ["add-source", id]) => inner.add_source(id, &body),
        ("POST", ["check-sources", id]) => inner.check_sources(id),
        ("POST", ["download", id]) => inner.download(id, &body),
        ("GET", ["download-status", id]) => inner.download_status(id),
        ("POST", ["cancel", id]) => inner.cancel(id),
        ("POST", ["remove", id]) => inner.remove(id),
        ("GET", ["fix-status", id]) => inner.fix_status(id),
        ("POST", ["open", "fixes", id]) => inner.open_fixes(id),
        ("POST", ["open", "settings"]) => {
            inner.navigate("settings", None);
            (200, json!({ "success": true }).to_string())
        }
        ("POST", ["open-url"]) => inner.open_url(&body),
        ("POST", ["restart-steam"]) => inner.restart_steam(),
        ("GET", ["session"]) => inner.session(),
        ("POST", ["verify"]) => inner.verify(),
        ("POST", ["check-updates"]) => inner.check_updates(),
        ("GET", ["loaded-apps"]) => (200, json!({ "success": true, "apps": [] }).to_string()),
        ("POST", ["loaded-apps"]) => (200, json!({ "success": true }).to_string()),
        ("GET", ["api-list"]) => (
            200,
            json!({ "success": true, "apis": [{ "name": "DepotBox", "index": 0 }] }).to_string(),
        ),
        ("GET", ["icon"]) => (
            200,
            json!({
                "success": true,
                "dataUrl": format!(
                    "data:image/png;base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(rebrand::ICON_PNG)
                )
            })
            .to_string(),
        ),
        ("GET", ["games-database"]) => (200, json!({ "success": true, "database": {} }).to_string()),
        ("POST", ["log"]) => (200, json!({ "success": true }).to_string()),
        _ => (404, json!({ "success": false, "error": "Not found" }).to_string()),
    };

    respond(request, status, payload);
}

impl Inner {
    fn settings_snapshot(&self) -> Settings {
        self.settings.lock().map(|value| value.clone()).unwrap_or_default()
    }

    fn has(&self, app_id: &str) -> (u16, String) {
        let settings = self.settings_snapshot();
        let exists = lua::installed_lua(&settings.steam_path, app_id).is_some();
        (200, json!({ "success": true, "exists": exists }).to_string())
    }

    /// Whether the app is holding a verified session right now.
    ///
    /// The plugin asks this before it starts an add. There is one session, in the app, which is the point:
    /// a user who verified in the window does not verify again in Steam, and a check made from Steam
    /// unlocks the window too, because both read this same token. A snapshot, so it is answered without
    /// waiting on the network.
    fn session(&self) -> (u16, String) {
        let info = tauri::async_runtime::block_on(self.api.session_info());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or(0);
        // The same 30 second floor `ensure_session` uses, so this never says "verified" for a token the
        // very next request would have to redo.
        let verified = matches!(&info, Some(session) if session.expires_at > now + 30);
        let expires_at = info.map(|session| session.expires_at).unwrap_or(0);
        (
            200,
            json!({ "success": true, "verified": verified, "expiresAt": expires_at }).to_string(),
        )
    }

    /// Start the app's own verification handshake on behalf of the plugin.
    ///
    /// Deliberately fire-and-forget: the handshake opens the browser page and polls for up to three
    /// minutes, which is longer than the bridge's request timeout, and the caller only needs to know the
    /// check has begun. It reports its progress on `auth:status`, so the app window shows the same check
    /// running — one Turnstile, both surfaces.
    fn verify(self: &Arc<Self>) -> (u16, String) {
        let inner = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            if let Err(error) = inner.api.ensure_session(&inner.app).await {
                log_line(&format!("plugin verification failed: {}", error.code));
            }
        });
        (200, json!({ "success": true, "state": "pending" }).to_string())
    }

    fn add(self: &Arc<Self>, app_id: &str, body: &str) -> (u16, String) {
        let name = string_field(&body_json(body), "name");
        let title = if name.is_empty() { format!("App {app_id}") } else { name };
        self.start_add(app_id, title)
    }

    fn add_source(self: &Arc<Self>, app_id: &str, body: &str) -> (u16, String) {
        let source = string_field(&body_json(body), "source");
        if !source.is_empty() && !source.eq_ignore_ascii_case("depotbox") {
            return (400, json!({ "success": false, "error": format!("Unknown source '{source}'") }).to_string());
        }
        let title = self
            .adds
            .lock()
            .ok()
            .and_then(|map| map.get(app_id).map(|state| state.title.clone()))
            .unwrap_or_else(|| format!("App {app_id}"));
        self.start_add(app_id, title)
    }

    /// The single entry point for an add, whichever button asked for it.
    ///
    /// `add` and `add-source` used to race: `add` always replaced the state and spawned a fetch, and
    /// `add-source` only checked `downloading`, so a second request during the availability check started a
    /// second full archive fetch for the same game. DepotBox expands that game's DLCs when it builds the
    /// archive, so each duplicate add regenerated the DLC bundle. One state record and one queue job are the
    /// right units of work; both are reserved atomically here.
    fn start_add(self: &Arc<Self>, app_id: &str, title: String) -> (u16, String) {
        if !app_id.chars().all(|c| c.is_ascii_digit()) || app_id.is_empty() || app_id.len() > 10 {
            return (400, json!({ "success": false, "error": "Invalid app id" }).to_string());
        }

        let settings = self.settings_snapshot();
        if lua::installed_lua(&settings.steam_path, app_id).is_some() {
            return (
                200,
                json!({ "success": true, "alreadyInstalled": true }).to_string(),
            );
        }
        if self.downloads.active_manifest(app_id).is_some() {
            return (200, json!({ "success": true, "alreadyRunning": true }).to_string());
        }

        let Ok(mut map) = self.adds.lock() else {
            return (500, json!({ "success": false, "error": "Add state unavailable" }).to_string());
        };
        if let Some(state) = map.get(app_id) {
            if add_in_progress(state) {
                return (200, json!({ "success": true, "alreadyRunning": true }).to_string());
            }
        }
        map.insert(
            app_id.to_string(),
            AddState { title: title.clone(), checking: true, ..AddState::default() },
        );
        drop(map);

        let inner = Arc::clone(self);
        let id = app_id.to_string();
        tauri::async_runtime::spawn(async move { run_add(inner, id, title).await });
        (200, json!({ "success": true }).to_string())
    }

    fn add_status(&self, app_id: &str) -> (u16, String) {
        let settings = self.settings_snapshot();
        let installed_now = lua::installed_lua(&settings.steam_path, app_id).is_some();
        let state = self.adds.lock().ok().and_then(|map| map.get(app_id).cloned());

        let Some(state) = state else {
            return (
                200,
                json!({
                    "success": true,
                    "checking": false,
                    "sourcesLoaded": false,
                    "sources": [],
                    "installed": installed_now
                })
                .to_string(),
            );
        };

        let sources = if state.checking && !state.sources_loaded {
            json!([])
        } else {
            json!([{
                "name": "DepotBox",
                "displayName": "DepotBox",
                "status": if state.source_status.is_empty() { "unknown" } else { state.source_status.as_str() },
                "available": state.source_status == "available",
                "canDownload": state.source_status == "available" && !state.downloading,
                "locked": false,
                "needsKey": true,
                "stats": Value::Null,
                "downloading": state.downloading,
                "progress": state.progress,
                "indeterminate": state.indeterminate,
            }])
        };

        (
            200,
            json!({
                "success": true,
                "appid": app_id.parse::<i64>().unwrap_or(0),
                "checking": state.checking,
                "fastFetch": true,
                "sourcesLoaded": state.sources_loaded,
                "sources": sources,
                "installStatus": state.install_status,
                "installFailed": state.error.is_some(),
                "gameUnavailable": state.game_unavailable,
                "error": state.error,
                "installed": state.installed || installed_now,
            })
            .to_string(),
        )
    }

    fn check_sources(&self, app_id: &str) -> (u16, String) {
        let available = tauri::async_runtime::block_on(self.availability(app_id));
        match available {
            Ok(available) => (
                200,
                json!({
                    "success": true,
                    "results": [{ "name": "DepotBox", "available": available, "url": Value::Null }]
                })
                .to_string(),
            ),
            Err(error) => (
                200,
                json!({ "success": false, "error": error, "results": [] }).to_string(),
            ),
        }
    }

    fn download(&self, app_id: &str, body: &str) -> (u16, String) {
        let source = string_field(&body_json(body), "source");
        let source = if source.is_empty() { string_field(&body_json(body), "apiName") } else { source };
        if !source.is_empty() && !source.eq_ignore_ascii_case("depotbox") {
            return (400, json!({ "success": false, "error": format!("Unknown source '{source}'") }).to_string());
        }
        if self
            .downloads
            .list()
            .iter()
            .any(|item| item.app_id == app_id && matches!(item.status.as_str(), "queued" | "downloading"))
        {
            return (409, json!({ "success": false, "error": "Download already in progress" }).to_string());
        }

        let settings = self.settings_snapshot();
        match self.downloads.start(
            self.app.clone(),
            self.api.clone(),
            NewDownload {
                path: format!("/v1/manifest/{app_id}?source=depotbox"),
                url: None,
                app_id: app_id.to_string(),
                title: format!("App {app_id}"),
                kind: "manifest".to_string(),
                fallback_name: format!("{app_id}.zip"),
                directory: download_dir(&settings),
                steam_path: settings.steam_path.clone(),
                install: true,
                unpin: settings.auto_update_apps,
            },
        ) {
            Ok(item) => {
                if let Ok(mut map) = self.legacy.lock() {
                    map.insert(app_id.to_string(), item.id);
                }
                (200, json!({ "success": true }).to_string())
            }
            Err(error) => (500, json!({ "success": false, "error": error.message }).to_string()),
        }
    }

    fn download_status(&self, app_id: &str) -> (u16, String) {
        let id = self.legacy.lock().ok().and_then(|map| map.get(app_id).cloned());
        let Some(id) = id else {
            return (200, json!({ "success": true, "state": Value::Null }).to_string());
        };
        let Some(item) = self.downloads.list().into_iter().find(|item| item.id == id) else {
            return (200, json!({ "success": true, "state": Value::Null }).to_string());
        };

        let status = match item.status.as_str() {
            "queued" => "queued",
            "downloading" => "downloading",
            "done" => "done",
            "cancelled" => "cancelled",
            _ => "failed",
        };
        let (bytes_read, total_bytes) = if item.total > 0 { (item.received, item.total) } else { (0, 100) };
        let payload = json!({
            "status": status,
            "bytesRead": bytes_read,
            "totalBytes": total_bytes,
            "currentApi": "DepotBox",
            "apiErrors": Value::Null,
            "error": if matches!(status, "failed" | "cancelled") { item.error.clone() } else { None::<String> },
            "installedPath": Value::Null,
            "success": status == "done",
            "api": if status == "done" { Some("DepotBox") } else { None::<&str> },
        });
        (200, json!({ "success": true, "state": payload }).to_string())
    }

    fn cancel(&self, app_id: &str) -> (u16, String) {
        let active = self
            .downloads
            .list()
            .into_iter()
            .find(|item| item.app_id == app_id && matches!(item.status.as_str(), "queued" | "downloading"));
        if let Some(item) = active {
            self.downloads.cancel(&item.id);
            return (200, json!({ "success": true }).to_string());
        }
        (200, json!({ "success": true, "message": "Nothing to cancel" }).to_string())
    }

    fn remove(&self, app_id: &str) -> (u16, String) {
        let settings = self.settings_snapshot();
        let Some(path) = lua::installed_lua(&settings.steam_path, app_id) else {
            return (
                200,
                json!({ "success": true, "deleted": [], "count": 0 }).to_string(),
            );
        };
        let deleted = path.to_string_lossy().to_string();
        let result = std::fs::remove_file(&path);
        let disabled = path.with_file_name(format!("{app_id}.lua.disabled"));
        if disabled.is_file() {
            let _ = std::fs::remove_file(&disabled);
        }
        match result {
            Ok(_) => (
                200,
                json!({ "success": true, "deleted": [deleted], "count": 1 }).to_string(),
            ),
            Err(error) => (500, json!({ "success": false, "error": error.to_string() }).to_string()),
        }
    }

    fn fix_status(&self, app_id: &str) -> (u16, String) {
        match self.fix_appids() {
            Ok(appids) => (
                200,
                json!({ "success": true, "hasFix": appids.iter().any(|id| id == app_id) }).to_string(),
            ),
            // Could not ask (offline, rate limited, no key): the store page hides its button rather than
            // asserting this game has no fix.
            Err(error) => (200, json!({ "success": false, "error": error }).to_string()),
        }
    }

    fn open_fixes(&self, app_id: &str) -> (u16, String) {
        self.navigate("fixes", Some(app_id.to_string()));
        (200, json!({ "success": true }).to_string())
    }

    /// The plugin's "Check for updates" row.
    ///
    /// It used to answer `{success:true}` with no work behind it, so the menu always said "No updates
    /// available" whether or not one was. The plugin ships with this app, so the honest answer is the
    /// plugin's own state: installed or not, and whether the bundle on disk still matches the release it
    /// was installed from. `plugin::state` already computes exactly that for the Plugin page.
    fn check_updates(&self) -> (u16, String) {
        let settings = self.settings_snapshot();
        let state =
            tauri::async_runtime::block_on(crate::plugin::state(&self.api, &settings.steam_path));
        // `PLUGIN_VERSION` already carries its `v`; the tag is normalised once here so the alert never reads
        // "vv1.0.3" the way the About line once did.
        let raw = state.version.clone().unwrap_or_else(|| rebrand::PLUGIN_VERSION.to_string());
        let version = if raw.starts_with('v') { raw.clone() } else { format!("v{raw}") };
        let message = if !state.installed {
            "The Steamware plugin is not installed right now. Install it from the Plugin page.".to_string()
        } else if state.update_available {
            "An update is ready. Open Steamware · Plugin and press Update.".to_string()
        } else if state.status.is_some() {
            "Could not reach the update feed just now. Try again in a moment.".to_string()
        } else {
            format!("Steamware plugin {version} is up to date.")
        };
        (
            200,
            json!({
                "success": true,
                "message": message,
                "updateAvailable": state.update_available,
                "version": version,
                "latest": state.latest,
            })
            .to_string(),
        )
    }

    fn navigate(&self, route: &str, app_id: Option<String>) {
        let payload = match app_id {
            Some(app_id) => json!({ "route": route, "appId": app_id }),
            None => json!({ "route": route }),
        };
        let _ = self.app.emit("plugin:navigate", payload);
        if let Some(window) = self.app.webview_windows().values().next() {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
    }

    fn open_url(&self, body: &str) -> (u16, String) {
        let mut url = string_field(&body_json(body), "url");
        for (from, to) in [
            ("discord.gg/luatools", "discord.gg/steamtools"),
            ("discord.com/invite/luatools", "discord.gg/steamtools"),
        ] {
            url = replace_ignore_case(&url, from, to);
        }
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return (400, json!({ "success": false, "error": "Invalid URL" }).to_string());
        }
        match tauri_plugin_opener::open_url(url, None::<&str>) {
            Ok(_) => (200, json!({ "success": true }).to_string()),
            Err(error) => (500, json!({ "success": false, "error": error.to_string() }).to_string()),
        }
    }

    fn restart_steam(&self) -> (u16, String) {
        let settings = self.settings_snapshot();
        let Some(root) = steam::detect_install(&settings.steam_path) else {
            return (200, json!({ "success": false, "error": "Steam was not found" }).to_string());
        };
        let was_running = steam::is_running();
        if was_running && !steam::stop(20) {
            return (200, json!({ "success": false, "error": "Failed to restart Steam" }).to_string());
        }
        if !steam::start(&root) {
            return (200, json!({ "success": false, "error": "Failed to restart Steam" }).to_string());
        }
        (200, json!({ "success": true }).to_string())
    }

    /// Asked by the store page as it draws, so it must never start verification: an unverified app simply
    /// answers "not available" and the plugin shows its plain button until the user chooses to verify.
    async fn availability(&self, app_id: &str) -> Result<bool, String> {
        let path = format!("/v1/manifest/{app_id}/availability?source=depotbox");
        match self.api.get_json_quiet(&path).await {
            Ok(body) => Ok(body.get("available").and_then(Value::as_bool).unwrap_or(false)),
            Err(error) => Err(error.message),
        }
    }

    fn fix_appids(&self) -> Result<Vec<String>, String> {
        if let Ok(cache) = self.fixes.lock() {
            if let Some((at, appids)) = cache.as_ref() {
                if at.elapsed() < Duration::from_secs(60) {
                    return Ok(appids.clone());
                }
            }
        }
        // Quiet for the same reason as `availability`: this answers a question the store page asks every time
        // it loads, and the interactive path would answer it by opening a browser.
        let body = tauri::async_runtime::block_on(self.api.get_json_quiet("/v1/fixes"))
            .map_err(|error| error.message)?;
        let appids: Vec<String> = body
            .get("games")
            .and_then(Value::as_array)
            .map(|games| {
                games
                    .iter()
                    .filter_map(|game| {
                        game.get("appid")
                            .and_then(|value| value.as_str().map(|s| s.to_string()).or_else(|| value.as_i64().map(|n| n.to_string())))
                    })
                    .collect()
            })
            .unwrap_or_default();
        if let Ok(mut cache) = self.fixes.lock() {
            *cache = Some((Instant::now(), appids.clone()));
        }
        Ok(appids)
    }
}

async fn run_add(inner: Arc<Inner>, app_id: String, title: String) {
    match inner.availability(&app_id).await {
        Ok(true) => {}
        Ok(false) => {
            finish_error(
                &inner,
                &app_id,
                "This game has no manifest available yet. Try again later.".to_string(),
                true,
            );
            return;
        }
        Err(message) => {
            finish_error(&inner, &app_id, message, true);
            return;
        }
    }

    let settings = inner.settings_snapshot();
    let item = match inner.downloads.start(
        inner.app.clone(),
        inner.api.clone(),
        NewDownload {
            path: format!("/v1/manifest/{app_id}?source=depotbox"),
            url: None,
            app_id: app_id.clone(),
            title: title.clone(),
            kind: "manifest".to_string(),
            fallback_name: format!("{app_id}.zip"),
            directory: download_dir(&settings),
            steam_path: settings.steam_path.clone(),
            install: true,
            unpin: settings.auto_update_apps,
        },
    ) {
        Ok(item) => item,
        Err(error) => {
            finish_error(&inner, &app_id, error.message, false);
            return;
        }
    };

    if let Ok(mut map) = inner.adds.lock() {
        if let Some(state) = map.get_mut(&app_id) {
            state.checking = false;
            state.sources_loaded = true;
            state.source_status = "available".to_string();
            state.downloading = true;
            state.indeterminate = true;
            state.progress = 0.0;
        }
    }

    let started = Instant::now();
    loop {
        if started.elapsed() > ADD_TIMEOUT {
            finish_error(&inner, &app_id, "The download took too long and was stopped.".to_string(), false);
            return;
        }
        tokio::time::sleep(Duration::from_millis(350)).await;

        let Some(current) = inner.downloads.list().into_iter().find(|entry| entry.id == item.id) else {
            finish_error(&inner, &app_id, "The download disappeared from the queue.".to_string(), false);
            return;
        };

        if let Ok(mut map) = inner.adds.lock() {
            if let Some(state) = map.get_mut(&app_id) {
                state.downloading = matches!(current.status.as_str(), "queued" | "downloading");
                state.indeterminate = current.total == 0;
                if current.total > 0 {
                    state.progress = (current.received as f64 / current.total as f64 * 100.0).clamp(0.0, 100.0);
                }
            }
        }

        match current.status.as_str() {
            "done" => {
                // The download installs itself as it lands, so those figures are the truth. Older rows (or
                // an archive with nothing installable in it) fall back to installing here.
                let outcome: Result<(usize, usize), String> =
                    if current.installed_lua > 0 || current.installed_manifests > 0 {
                        Ok((current.installed_lua as usize, current.installed_manifests as usize))
                    } else if let Some(message) = current.install_error.clone() {
                        Err(message)
                    } else {
                        lua::install_archive(&current.path, &settings.steam_path, settings.auto_update_apps)
                            .map(|summary| (summary.lua, summary.manifests))
                            .map_err(|error| error.message)
                    };

                match outcome {
                    Ok((lua_files, manifests)) if lua_files + manifests > 0 => {
                        if let Ok(mut map) = inner.adds.lock() {
                            if let Some(state) = map.get_mut(&app_id) {
                                state.downloading = false;
                                state.progress = 100.0;
                                state.indeterminate = false;
                                state.installed = true;
                                state.install_status = Some(if manifests > 0 {
                                    format!(
                                        "{title} was added to Steam with {manifests} manifest{}.",
                                        if manifests == 1 { "" } else { "s" }
                                    )
                                } else {
                                    format!("{title} was added to Steam.")
                                });
                            }
                        }
                    }
                    Ok(_) => finish_error(
                        &inner,
                        &app_id,
                        "That archive did not contain any .lua files.".to_string(),
                        false,
                    ),
                    Err(message) => finish_error(&inner, &app_id, message, false),
                }
                return;
            }
            "failed" => {
                let message = current
                    .error
                    .clone()
                    .unwrap_or_else(|| "The download failed. Try again.".to_string());
                finish_error(&inner, &app_id, message, false);
                return;
            }
            "cancelled" => {
                finish_error(&inner, &app_id, "Download cancelled.".to_string(), false);
                return;
            }
            _ => {}
        }
    }
}

fn finish_error(inner: &Arc<Inner>, app_id: &str, message: String, unavailable: bool) {
    if let Ok(mut map) = inner.adds.lock() {
        if let Some(state) = map.get_mut(app_id) {
            state.checking = false;
            state.sources_loaded = true;
            state.downloading = false;
            state.indeterminate = false;
            state.error = Some(message);
            state.game_unavailable = unavailable;
            if state.source_status.is_empty() {
                state.source_status = "unknown".to_string();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_must_match_exactly() {
        assert!(token_matches("abc", "abc"));
        assert!(!token_matches("abc", "abd"));
        assert!(!token_matches("abc", "abcd"));
        assert!(!token_matches("", ""));
        assert!(!token_matches("", "abc"));
    }

    #[test]
    fn ids_must_be_plain_digits() {
        assert!(id_segment("730"));
        assert!(id_segment("0000000"));
        assert!(!id_segment(""));
        assert!(!id_segment(".."));
        assert!(!id_segment("../../../evil"));
        assert!(!id_segment(r"..\..\evil"));
        assert!(!id_segment("730%2F.."));
        assert!(!id_segment("12345678901"));
    }

    #[test]
    fn only_a_checking_or_downloading_add_blocks_a_retry() {
        let checking = AddState { checking: true, ..AddState::default() };
        let downloading = AddState { downloading: true, ..AddState::default() };
        let failed = AddState { error: Some("offline".to_string()), ..AddState::default() };
        let done = AddState { installed: true, ..AddState::default() };

        assert!(add_in_progress(&checking));
        assert!(add_in_progress(&downloading));
        assert!(!add_in_progress(&failed));
        assert!(!add_in_progress(&done));
    }
}
