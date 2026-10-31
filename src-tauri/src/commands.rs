use crate::api::{ApiClient, SessionInfo};
use crate::builds;
use crate::downloads::{DownloadItem, Downloads, NewDownload};
use crate::modes;
use crate::plugin;
use crate::presence;
use crate::startup;
use crate::error::{AppError, AppResult};
use crate::media::MediaCache;
use crate::settings::{self, Settings};
use crate::steam::{self, InstalledGame, SteamStatus};
use crate::update::{UpdateStatus, Updater};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, State, WebviewWindow};

pub struct AppState {
    pub settings: Arc<Mutex<Settings>>,
    pub api: Arc<ApiClient>,
    pub media: MediaCache,
    pub downloads: Arc<Downloads>,
    /// What the app is showing, for Discord Rich Presence. Shared with the presence thread, which reads it
    /// every tick rather than being pushed to.
    pub presence: Arc<crate::presence::Presence>,
    /// Whether this build is still the newest release. Read by the window, refreshed by a background tick.
    pub updater: Arc<Updater>,
}

impl AppState {
    pub fn new(settings: Settings) -> Self {
        let api = Arc::new(ApiClient::new(&settings.api_base, &settings::device_id()));
        let downloads = Arc::new(Downloads::new(settings.max_concurrent));
        Self {
            settings: Arc::new(Mutex::new(settings)),
            api,
            media: MediaCache::new(),
            downloads,
            presence: Arc::new(crate::presence::Presence::new()),
            updater: Updater::new(),
        }
    }

    pub fn settings_snapshot(&self) -> Settings {
        self.settings.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

fn encode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn require_app_id(value: &str) -> AppResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 10 || !trimmed.chars().all(|c| c.is_ascii_digit()) {
        return Err(AppError::new("invalid_appid", "That app id is not valid."));
    }
    Ok(trimmed.to_string())
}

fn download_dir(settings: &Settings) -> PathBuf {
    let dir = PathBuf::from(&settings.download_dir);
    if dir.as_os_str().is_empty() {
        settings::app_dir().join("downloads")
    } else {
        dir
    }
}

/* ── Bootstrap ─────────────────────────────────────────────────────────────────────────────────── */

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    settings: Settings,
    steam: SteamStatus,
    version: String,
    session: Option<SessionInfo>,
    /// The address this build was made for — the value the first-run screen offers, and empty only for a
    /// source checkout built without one. Reported separately from the profile's own value so the wizard can
    /// tell "this build knows a server" from "this profile has none yet".
    default_api_base: String,
    /// The folder this profile lives in, shown in Settings so a user can find their files.
    data_dir: String,
}

#[tauri::command]
pub async fn bootstrap(state: State<'_, AppState>) -> AppResult<Bootstrap> {
    let settings = state.settings_snapshot();
    let steam = steam::status(&settings.steam_path);
    Ok(Bootstrap {
        default_api_base: settings::built_in_api_base().to_string(),
        settings,
        steam,
        version: env!("CARGO_PKG_VERSION").to_string(),
        session: state.api.session_info().await,
        data_dir: settings::app_dir().to_string_lossy().to_string(),
    })
}

#[tauri::command]
pub async fn save_settings(
    state: State<'_, AppState>,
    settings: Settings,
) -> AppResult<Settings> {
    let saved = settings.save()?;
    // A different server means a different session; the same server does not, and clearing it on every save
    // made changing the accent or the download folder look like being signed out until the next poll.
    if state.settings_snapshot().api_base != saved.api_base {
        state.api.set_base(&saved.api_base).await;
        state.api.clear_session().await;
    }
    state.downloads.set_concurrency(saved.max_concurrent);
    if let Ok(mut current) = state.settings.lock() {
        *current = saved.clone();
    }
    // "Start with Windows" is a registry value, not a profile field, so it is written once the profile is
    // safely on disk. Best effort: a machine that refuses the write must still keep every other setting.
    let _ = crate::startup::apply(saved.start_with_windows);
    Ok(saved)
}

#[tauri::command]
pub async fn steam_refresh(state: State<'_, AppState>) -> AppResult<SteamStatus> {
    let settings = state.settings_snapshot();
    Ok(steam::status(&settings.steam_path))
}

#[tauri::command]
pub async fn library_list(state: State<'_, AppState>) -> AppResult<Vec<InstalledGame>> {
    let settings = state.settings_snapshot();
    Ok(steam::library(&settings.steam_path))
}

/// Every game Steamware has added to Steam, whether or not Steam has downloaded it.
///
/// Separate from `library_list` on purpose: that one reads `appmanifest_*.acf` and can only ever know about
/// games already installed. The library page lists luas, which is the whole point — 312 games owned, not the
/// 33 that happen to sit on disk.
async fn owned_blocking(steam_path: String) -> AppResult<Vec<steam::OwnedGame>> {
    tokio::task::spawn_blocking(move || steam::owned(&steam_path))
        .await
        .map_err(|_| AppError::new("internal", "Could not read Steam's plugin folder."))
}

#[tauri::command]
pub async fn library_owned(state: State<'_, AppState>) -> AppResult<Vec<steam::OwnedGame>> {
    let settings = state.settings_snapshot();
    owned_blocking(settings.steam_path).await
}

/// Remove one game from both libraries at once.
///
/// The frontend names an app id, never a path: `lua::remove_installed` validates the id and deletes only
/// `<appid>.lua` (or its parked `.disabled` twin) inside Steam's `config/stplug-in` folder, plus the
/// `appmanifest_<appid>.acf` Steamware wrote into `steamapps`. The lua is Steamware's list and the licence;
/// the manifest is the tile in Steam itself, and leaving it behind is what made a removed game linger.
#[tauri::command]
pub fn library_remove(
    state: State<'_, AppState>,
    app_id: String,
) -> AppResult<crate::lua::RemovalSummary> {
    let app_id = require_app_id(&app_id)?;
    crate::lua::remove_installed(&state.settings_snapshot().steam_path, &app_id)
}

/// Start Steam if it is installed and not already running.
///
/// Detached on purpose: Steamware must not hold a handle that keeps Steam alive or ties its lifetime to
/// ours. If Steam is already running this is a no-op, because a second `steam.exe` just focuses the first.
#[tauri::command]
pub fn steam_launch(state: State<'_, AppState>) -> AppResult<()> {
    let settings = state.settings_snapshot();
    let Some(root) = steam::detect_install(&settings.steam_path) else {
        return Err(AppError::new(
            "steam_missing",
            "Steam was not found. Set its folder in Settings, then try again.",
        ));
    };

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        let executable = root.join("steam.exe");
        if !executable.exists() {
            return Err(AppError::new("steam_missing", "steam.exe is missing from that folder."));
        }
        std::process::Command::new(executable)
            .current_dir(&root)
            .creation_flags(DETACHED_PROCESS)
            .spawn()
            .map(|_| ())
            .map_err(|_| AppError::new("steam_launch_failed", "Could not start Steam."))
    }

    #[cfg(not(windows))]
    {
        if steam::start(&root) {
            Ok(())
        } else {
            Err(AppError::new(
                "steam_launch_failed",
                "Could not start Steam. Open it from your applications menu.",
            ))
        }
    }
}

/// Close Steam and start it again, or just start it when it is not running.
///
/// Steam only re-reads `config\stplug-in` and `depotcache` at startup, so a lua installed while it is open
/// does nothing until it restarts. Stopping blocks until the process is really gone (file handles are what
/// make a half-restart corrupt a download), so the whole thing runs off the UI thread.
#[tauri::command]
pub async fn steam_restart(state: State<'_, AppState>) -> AppResult<()> {
    let settings = state.settings_snapshot();
    let Some(root) = steam::detect_install(&settings.steam_path) else {
        return Err(AppError::new(
            "steam_missing",
            "Steam was not found. Set its folder in Settings, then try again.",
        ));
    };

    tokio::task::spawn_blocking(move || {
        if steam::is_running() && !steam::stop(25) {
            return Err(AppError::new(
                "steam_stop_failed",
                "Steam did not close. Quit it yourself, then start it again.",
            ));
        }
        if !steam::start(&root) {
            return Err(AppError::new(
                "steam_launch_failed",
                "Steam closed but would not start again. Open it from its shortcut.",
            ));
        }
        Ok(())
    })
    .await
    .map_err(|_| AppError::new("internal", "The restart task stopped unexpectedly."))?
}

/// Steam's app-info for one game: depots, manifest ids, DLC and the public build id.
///
/// Proxied rather than fetched here so the depots a user is browsing stay off Valve's per-IP logs and the
/// app keeps one outbound host.
#[tauri::command]
pub async fn steam_app(
    app: AppHandle,
    state: State<'_, AppState>,
    app_id: String,
) -> AppResult<Value> {
    let app_id = require_app_id(&app_id)?;
    state.api.get_json(&app, &format!("/v1/steam/app/{app_id}")).await
}

/* ── Builds ────────────────────────────────────────────────────────────────────────────────── */

#[tauri::command]
pub fn builds_list(state: State<'_, AppState>) -> Vec<builds::BuildGame> {
    builds::list(&state.settings_snapshot().steam_path)
}

#[tauri::command]
pub fn builds_state(state: State<'_, AppState>, app_id: String) -> AppResult<builds::LuaState> {
    let app_id = require_app_id(&app_id)?;
    builds::state(&state.settings_snapshot().steam_path, &app_id)
}

#[tauri::command]
pub fn builds_set_entry(
    state: State<'_, AppState>,
    app_id: String,
    entry_id: String,
    field: String,
    value: bool,
) -> AppResult<builds::LuaState> {
    let app_id = require_app_id(&app_id)?;
    builds::set_entry(
        &state.settings_snapshot().steam_path,
        &app_id,
        &entry_id,
        &field,
        value,
    )
}

#[tauri::command]
pub fn builds_save_text(
    state: State<'_, AppState>,
    app_id: String,
    text: String,
) -> AppResult<builds::LuaState> {
    let app_id = require_app_id(&app_id)?;
    builds::write_lua(&state.settings_snapshot().steam_path, &app_id, &text)
}

#[tauri::command]
pub fn builds_save_variant(
    state: State<'_, AppState>,
    app_id: String,
    name: String,
) -> AppResult<builds::LuaState> {
    let app_id = require_app_id(&app_id)?;
    builds::save_variant(&state.settings_snapshot().steam_path, &app_id, &name)
}

#[tauri::command]
pub fn builds_apply_variant(
    state: State<'_, AppState>,
    app_id: String,
    variant_id: String,
) -> AppResult<builds::LuaState> {
    let app_id = require_app_id(&app_id)?;
    builds::apply_variant(&state.settings_snapshot().steam_path, &app_id, &variant_id)
}

#[tauri::command]
pub fn builds_rename_variant(
    state: State<'_, AppState>,
    app_id: String,
    variant_id: String,
    name: String,
) -> AppResult<builds::LuaState> {
    let app_id = require_app_id(&app_id)?;
    builds::rename_variant(&state.settings_snapshot().steam_path, &app_id, &variant_id, &name)
}

#[tauri::command]
pub fn builds_delete_variant(
    state: State<'_, AppState>,
    app_id: String,
    variant_id: String,
) -> AppResult<builds::LuaState> {
    let app_id = require_app_id(&app_id)?;
    builds::delete_variant(&state.settings_snapshot().steam_path, &app_id, &variant_id)
}

/* ── Modes ─────────────────────────────────────────────────────────────────────────────────── */

#[tauri::command]
pub async fn modes_state(state: State<'_, AppState>) -> AppResult<modes::ModesState> {
    let steam_path = state.settings_snapshot().steam_path;
    Ok(modes::state(state.api.as_ref(), &steam_path).await)
}

#[tauri::command]
pub async fn modes_install(
    app: AppHandle,
    state: State<'_, AppState>,
    mode: String,
) -> AppResult<modes::ModesState> {
    let steam_path = state.settings_snapshot().steam_path;
    modes::install(state.api.as_ref(), &app, &steam_path, &mode).await
}

#[tauri::command]
pub async fn modes_cloud_redirect(
    app: AppHandle,
    state: State<'_, AppState>,
    enable: bool,
    update: bool,
) -> AppResult<modes::ModesState> {
    let steam_path = state.settings_snapshot().steam_path;
    modes::cloud_redirect_set(state.api.as_ref(), &app, &steam_path, enable, update).await
}

/* ── Plugin ────────────────────────────────────────────────────────────────────────────────── */


#[tauri::command]
pub async fn plugin_state(state: State<'_, AppState>) -> AppResult<plugin::PluginState> {
    let steam_path = state.settings_snapshot().steam_path;
    Ok(plugin::state(state.api.as_ref(), &steam_path).await)
}

#[tauri::command]
pub async fn plugin_install(app: AppHandle, state: State<'_, AppState>) -> AppResult<plugin::PluginState> {
    let steam_path = state.settings_snapshot().steam_path;
    plugin::install(state.api.as_ref(), &app, &steam_path).await
}

#[tauri::command]
pub async fn plugin_uninstall(state: State<'_, AppState>) -> AppResult<()> {
    plugin::uninstall(&state.settings_snapshot().steam_path)
}

/* ── Verification ─────────────────────────────────────────────────────────────────────────────── */

#[tauri::command]
pub async fn auth_begin(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    let api = state.api.clone();
    // Spawned rather than awaited so the UI stays responsive: the flow emits `auth:status` events and can
    // take as long as the user takes in the browser.
    tauri::async_runtime::spawn(async move {
        let _ = api.ensure_session(&app).await;
    });
    Ok(())
}

#[tauri::command]
pub async fn auth_sign_out(state: State<'_, AppState>) -> AppResult<()> {
    state.api.clear_session().await;
    Ok(())
}

#[tauri::command]
pub async fn auth_status(state: State<'_, AppState>) -> AppResult<Option<SessionInfo>> {
    Ok(state.api.session_info().await)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerHealth {
    /// True while the app is deliberately holding back optional requests.
    degraded: bool,
    /// What to tell the user, already written for a person.
    message: Option<String>,
    /// Unix seconds the hold lifts: midnight UTC for the daily cap, five minutes for an outage.
    until: i64,
}

/// Whether the server is usable right now, and if not, why.
///
/// The UI polls this instead of waiting for a failure: during a cap the point is that most of the app keeps
/// working, and a banner explains the one part that does not. Costs nothing — it reads local state only.
#[tauri::command]
pub async fn server_health(state: State<'_, AppState>) -> AppResult<ServerHealth> {
    let (degraded, message, until) = state.api.health().await;
    Ok(ServerHealth { degraded, message, until })
}

/* ── Update gate ─────────────────────────────────────────────────────────────────────────────── */

/// Cached state only. The window polls this, so it must never cost a request.
#[tauri::command]
pub async fn update_status(state: State<'_, AppState>) -> AppResult<UpdateStatus> {
    Ok(state.updater.status().await)
}

/// The gate's "Check again": ignores the fifteen-minute cache and asks GitHub now.
#[tauri::command]
pub async fn update_check(state: State<'_, AppState>) -> AppResult<UpdateStatus> {
    Ok(state.updater.refresh(true).await)
}

/// Download the new installer, run it after this process exits, and quit. The app relaunches itself.
#[tauri::command]
pub async fn update_install(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    state.updater.install(&app).await
}

/* ── Server capabilities ─────────────────────────────────────────────────────────────────────── */

/// Which sources this server can actually serve. Public on purpose: the UI uses it to explain an empty
/// page ("waiting for the provider key") instead of showing a failure the user cannot act on.
#[tauri::command]
pub async fn server_config(state: State<'_, AppState>) -> AppResult<Value> {
    state.api.get_public_json("/v1/config").await
}

/* ── Catalog ─────────────────────────────────────────────────────────────────────────────────── */

#[tauri::command]
pub async fn catalog_stats(app: AppHandle, state: State<'_, AppState>) -> AppResult<Value> {
    state.api.get_json(&app, "/v1/catalog/stats").await
}

#[tauri::command]
pub async fn store_search(app: AppHandle, state: State<'_, AppState>, query: String) -> AppResult<Value> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(serde_json::json!({ "ok": true, "items": [] }));
    }
    state
        .api
        .get_json(&app, &format!("/v1/store/search?q={}", encode(query)))
        .await
}

#[tauri::command]
pub async fn store_featured(app: AppHandle, state: State<'_, AppState>) -> AppResult<Value> {
    state.api.get_json(&app, "/v1/store/featured").await
}

#[tauri::command]
pub async fn store_genre(
    app: AppHandle,
    state: State<'_, AppState>,
    genre: String,
    start: u32,
) -> AppResult<Value> {
    state
        .api
        .get_json(
            &app,
            &format!("/v1/store/genre?genre={}&start={}", encode(&genre), start.min(2000)),
        )
        .await
}

#[tauri::command]
pub async fn store_app(app: AppHandle, state: State<'_, AppState>, app_id: String) -> AppResult<Value> {
    let app_id = require_app_id(&app_id)?;
    state.api.get_json(&app, &format!("/v1/store/app/{app_id}")).await
}

/// The store's header art for one app, without a session.
///
/// Apps published after Steam moved to hashed asset paths have no `header.jpg` under `steam/apps/<id>/` at
/// all, so every CDN guess a library tile makes comes back 404 and the tile falls back to a name strip.
/// The store API is the only thing that knows the hashed path, and the picture it returns is the same
/// public art. Public on purpose: a library of local lua files must not open a Cloudflare check just to
/// draw its thumbnails.
#[tauri::command]
pub async fn store_cover(state: State<'_, AppState>, app_id: String) -> AppResult<Value> {
    let app_id = require_app_id(&app_id)?;
    state.api.get_public_json(&format!("/v1/store/app/{app_id}")).await
}

/// Steam's review score and live player count for a row of cards.
///
/// One call carries a whole batch of ids — the Worker answers for the first twelve and drops anything past that
/// — so a grid of eight games costs one request rather than eight, and the ids are sent in batches of twelve.
/// Ids the server could not answer for come back as `null`, which the card reads as "not known yet" rather than
/// as zero.
#[tauri::command]
pub async fn store_stats(
    app: AppHandle,
    state: State<'_, AppState>,
    app_ids: Vec<String>,
) -> AppResult<Value> {
    let mut ids: Vec<String> = Vec::new();
    for id in &app_ids {
        if let Ok(id) = require_app_id(id) {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    if ids.is_empty() {
        return Err(AppError::config("Give one or more numeric app ids."));
    }
    state
        .api
        .get_json(&app, &format!("/v1/store/stats?appids={}", ids.join(",")))
        .await
}

#[tauri::command]
pub async fn fixes_catalog(
    app: AppHandle,
    state: State<'_, AppState>,
    tag: Option<String>,
    query: Option<String>,
) -> AppResult<Value> {
    let mut path = String::from("/v1/fixes");
    let mut params: Vec<String> = Vec::new();
    if let Some(tag) = tag.filter(|tag| !tag.is_empty() && tag != "all") {
        params.push(format!("tag={}", encode(&tag)));
    }
    if let Some(query) = query.filter(|query| !query.trim().is_empty()) {
        params.push(format!("q={}", encode(query.trim())));
    }
    if !params.is_empty() {
        path.push('?');
        path.push_str(&params.join("&"));
    }
    state.api.get_json(&app, &path).await
}

#[tauri::command]
pub async fn manifest_availability(
    app: AppHandle,
    state: State<'_, AppState>,
    app_id: String,
    source: Option<String>,
) -> AppResult<Value> {
    let app_id = require_app_id(&app_id)?;
    let source = source.unwrap_or_else(|| "depotbox".to_string());
    let source = match source.as_str() {
        "depotbox" | "hubcap" => source,
        _ => "depotbox".to_string(),
    };
    state
        .api
        .get_json(&app, &format!("/v1/manifest/{app_id}/availability?source={source}"))
        .await
}

/* ── Downloads ──────────────────────────────────────────────────────────────────────────────── */

#[tauri::command]
pub async fn start_manifest_download(
    app: AppHandle,
    state: State<'_, AppState>,
    app_id: String,
    source: Option<String>,
    title: Option<String>,
) -> AppResult<DownloadItem> {
    let app_id = require_app_id(&app_id)?;
    let settings = state.settings_snapshot();

    // Hubcap is the user's own account, so its download goes straight from this PC to Hubcap with the key
    // they typed into Settings: nothing about it passes through the Steamware server. DepotBox stays behind
    // the Worker, which is the only place its key exists.
    let hubcap_key = settings.hubcap_key.trim().to_string();
    let requested = source.unwrap_or_else(|| settings.preferred_source.clone());

    // "Automatic" means whichever source can serve this game: DepotBox first, then Hubcap with the user's own
    // key. FastFetch is what turns that probe on — with it off the fetch goes straight to the default source
    // and fails visibly instead of hunting, which is the point of turning it off.
    let wanted = if requested == "hubcap" && hubcap_key.is_empty() {
        "depotbox".to_string()
    } else if requested == "auto" {
        let mut chosen = "depotbox".to_string();
        if settings.fast_fetch && !hubcap_key.is_empty() {
            let probe = state
                .api
                .get_json(&app, &format!("/v1/manifest/{app_id}/availability?source=depotbox"))
                .await;
            // A probe that cannot answer leaves the default alone: an unreachable server is not a reason to
            // spend the user's Hubcap quota.
            let served = probe.ok().and_then(|body| body.get("available").and_then(|v| v.as_bool()));
            if served == Some(false) {
                chosen = "hubcap".to_string();
            }
        }
        chosen
    } else {
        requested
    };

    let via_hubcap = wanted == "hubcap" && !hubcap_key.is_empty();
    let (path, url) = if via_hubcap {
        (
            String::new(),
            Some(format!(
                "https://hubcapmanifest.com/api/v1/manifest/{}?api_key={}",
                encode(&app_id),
                encode(&hubcap_key)
            )),
        )
    } else {
        (format!("/v1/manifest/{app_id}?source=depotbox"), None)
    };

    let title = title.unwrap_or_else(|| format!("App {app_id}"));
    let started = state.downloads.start(
        app,
        state.api.clone(),
        NewDownload {
            path,
            url,
            app_id: app_id.clone(),
            title: title.clone(),
            kind: "manifest".to_string(),
            fallback_name: format!("{app_id}.zip"),
            directory: download_dir(&settings),
            steam_path: settings.steam_path.clone(),
            // A fetched game is installed as it lands: lua into the plugin folder, manifests into
            // `depotcache`. Saving a zip and stopping there left every "fetched" game unplayable.
            install: true,
            unpin: settings.auto_update_apps,
        },
    )?;

    // Announced from here rather than from the queue: the card the presence thread builds follows running
    // downloads, and a manifest fetch is over in about a second, so the queue would never show it. This pins
    // "Getting <game> for free" for half a minute, then hands the card back to wherever the user is.
    state.presence.hold_game(&title);

    Ok(started)
}

#[tauri::command]
pub async fn start_fix_download(
    app: AppHandle,
    state: State<'_, AppState>,
    id: Option<String>,
    file: Option<String>,
    title: Option<String>,
) -> AppResult<DownloadItem> {
    let settings = state.settings_snapshot();
    let (query, fallback) = match (id.filter(|v| !v.trim().is_empty()), file.filter(|v| !v.trim().is_empty())) {
        (Some(id), _) if id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') => {
            (format!("id={}", encode(&id)), format!("{id}.zip"))
        }
        (_, Some(file)) => (format!("file={}", encode(&file)), file),
        _ => return Err(AppError::new("invalid_target", "That fix has no download.")),
    };

    state.downloads.start(
        app,
        state.api.clone(),
        NewDownload {
            path: format!("/v1/fixes/download?{query}"),
            url: None,
            app_id: String::new(),
            title: title.unwrap_or_else(|| "Game fix".to_string()),
            kind: "fix".to_string(),
            fallback_name: fallback,
            directory: download_dir(&settings),
            steam_path: settings.steam_path.clone(),
            // A fix is unpacked by the user (or the Fixes page), not dropped into Steam's own folders.
            install: false,
            // A fix has to stay on the version it was built against, so its pins are never commented out.
            unpin: false,
        },
    )
}

#[tauri::command]
pub fn downloads_list(state: State<'_, AppState>) -> Vec<DownloadItem> {
    state.downloads.list()
}

#[tauri::command]
pub fn download_cancel(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.downloads.cancel(&id);
    Ok(())
}

/// Finished jobs from previous runs. The live queue is per-run; this is the record the page shows.
#[tauri::command]
pub fn download_history() -> Vec<crate::history::HistoryEntry> {
    crate::history::load()
}

#[tauri::command]
pub fn download_history_clear() -> AppResult<()> {
    crate::history::clear();
    Ok(())
}

#[tauri::command]
pub fn download_history_remove(id: String) -> AppResult<()> {
    crate::history::remove(&id);
    Ok(())
}

/* ── Covers ─────────────────────────────────────────────────────────────────────────────────── */

#[tauri::command]
pub async fn open_media(state: State<'_, AppState>, url: String) -> AppResult<String> {
    state.media.resolve(state.api.as_ref(), &url).await
}

/* ── Shell ──────────────────────────────────────────────────────────────────────────────────── */

#[tauri::command]
pub fn open_external(url: String) -> AppResult<()> {
    let parsed = url::Url::parse(&url)?;
    // Only the two schemes the UI actually opens. `file:` and friends stay unreachable from the frontend.
    if !matches!(parsed.scheme(), "https" | "http" | "steam") {
        return Err(AppError::new("invalid_url", "That link cannot be opened."));
    }
    tauri_plugin_opener::open_url(url, None::<&str>)
        .map_err(|_| AppError::new("open_failed", "Could not open that link."))
}

#[tauri::command]
pub fn reveal_path(path: String) -> AppResult<()> {
    let target = Path::new(&path);
    if !target.exists() {
        return Err(AppError::new("missing_path", "That file is no longer there."));
    }
    tauri_plugin_opener::reveal_item_in_dir(target)
        .map_err(|_| AppError::new("open_failed", "Could not open that folder."))
}

/// Install a downloaded manifest archive into Steam: its lua files into the plugin folder, its manifests
/// into `depotcache`. Both halves are required for the game to actually verify.
#[tauri::command]
pub fn install_lua_from_zip(
    state: State<'_, AppState>,
    path: String,
) -> AppResult<crate::lua::InstallSummary> {
    let settings = state.settings_snapshot();
    crate::lua::install_archive(&path, &settings.steam_path, settings.auto_update_apps)
}

/// Native file picker for “install the archive I already have”. Implemented as a Rust command rather than a
/// frontend plugin so the path never has to round-trip through JavaScript just to be handed back.
#[tauri::command]
pub async fn pick_archive(app: AppHandle) -> AppResult<Option<String>> {
    use tauri_plugin_dialog::DialogExt;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Choose a manifest or fix archive")
        .add_filter("Archives", &["zip"])
        .pick_file(move |path| {
            let _ = sender.send(path);
        });
    let picked = receiver
        .await
        .map_err(|_| AppError::new("dialog", "The file picker closed unexpectedly."))?;
    Ok(picked
        .and_then(|path| path.into_path().ok())
        .map(|path| path.to_string_lossy().to_string()))
}

#[tauri::command]
pub async fn pick_folder(app: AppHandle) -> AppResult<Option<String>> {
    use tauri_plugin_dialog::DialogExt;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Choose where downloads go")
        .pick_folder(move |path| {
            let _ = sender.send(path);
        });
    let picked = receiver
        .await
        .map_err(|_| AppError::new("dialog", "The folder picker closed unexpectedly."))?;
    Ok(picked
        .and_then(|path| path.into_path().ok())
        .map(|path| path.to_string_lossy().to_string()))
}

/* ── Discord presence ──────────────────────────────────────────────────────────────────────────── */

/// The page the shell just opened, as its own route key. Called on every navigation; the presence thread
/// turns it into the card's first line.
#[tauri::command]
pub fn set_presence_page(state: State<'_, AppState>, page: String) -> AppResult<()> {
    state.presence.set_page(&page);
    Ok(())
}

/* ── Discord ───────────────────────────────────────────────────────────────────────────────────── */

#[derive(Serialize)]
pub struct DiscordCheck {
    pub ok: bool,
    pub message: String,
}

/// Check the application id the user pasted into Settings against the local Discord client. It never
/// publishes an activity: a client that reaches READY is proof that Discord knows the application, which
/// is the only thing the Settings test needs to say.
#[tauri::command]
pub fn discord_check(app_id: String) -> DiscordCheck {
    match presence::check(&app_id) {
        Ok(()) => DiscordCheck {
            ok: true,
            message: "Discord accepted that application ID. The card will appear on your profile.".to_string(),
        },
        Err(message) => DiscordCheck { ok: false, message },
    }
}

/* ── Hubcap key ────────────────────────────────────────────────────────────────────────────────── */

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubcapCheck {
    ok: bool,
    message: String,
}

/// Check a Hubcap key the way its owner would want: against Hubcap, not against a format guess.
///
/// The key belongs to the person using it and goes nowhere else — this is a direct request to
/// hubcapmanifest.com, the same host a Hubcap download already uses, carrying the same key. The Steamware
/// server never sees it, and the key is never written anywhere by this command: saving it is the settings
/// save's job, so a key that fails its check never reaches the profile.
#[tauri::command]
pub async fn hubcap_check(key: String) -> AppResult<HubcapCheck> {
    let key = key.trim().to_string();
    if key.is_empty() {
        return Ok(HubcapCheck { ok: false, message: "Paste your Hubcap key first.".to_string() });
    }
    // A prefix check only: Hubcap is the authority on its own keys, and a soft check is what keeps a
    // reissued key format from being rejected by this app before Hubcap ever sees it.
    if !key.starts_with("smm_") {
        return Ok(HubcapCheck {
            ok: false,
            message: "That does not look like a Hubcap key — they start with smm_".to_string(),
        });
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|_| AppError::new("network", "That request could not be prepared."))?;
    let url = format!(
        "https://hubcapmanifest.com/api/v1/user/stats?api_key={}",
        encode(&key)
    );

    let response = match client.get(url).header("accept", "application/json").send().await {
        Ok(response) => response,
        Err(_) => {
            return Ok(HubcapCheck {
                ok: false,
                message: "Hubcap could not be reached, so the key was not checked.".to_string(),
            })
        }
    };

    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Ok(HubcapCheck { ok: false, message: "Hubcap rejected that key.".to_string() });
    }
    if !response.status().is_success() {
        return Ok(HubcapCheck {
            ok: false,
            message: format!(
                "Hubcap answered {} — the key was not accepted.",
                response.status().as_u16()
            ),
        });
    }

    let body: Value = response.json().await.unwrap_or(Value::Null);
    let used = body.get("daily_usage").and_then(Value::as_i64);
    let limit = body.get("daily_limit").and_then(Value::as_i64);
    let message = match (used, limit) {
        (Some(used), Some(limit)) => {
            format!("Key works — {used} of {limit} downloads used today.")
        }
        _ => "Key works.".to_string(),
    };
    Ok(HubcapCheck { ok: true, message })
}

/// Whether Windows would currently start Steamware. Read back rather than assumed, because Task Manager
/// lets a user disable the entry without telling the app.
#[tauri::command]
pub fn startup_status() -> AppResult<bool> {
    Ok(startup::registered())
}

#[tauri::command]
pub fn window_minimize(window: WebviewWindow) -> AppResult<()> {
    window.minimize().map_err(|_| AppError::new("window", "Could not minimise the window."))
}

#[tauri::command]
pub fn window_toggle_maximize(window: WebviewWindow) -> AppResult<()> {
    let maximised = window.is_maximized().unwrap_or(false);
    let result = if maximised { window.unmaximize() } else { window.maximize() };
    result.map_err(|_| AppError::new("window", "Could not resize the window."))
}

#[tauri::command]
pub fn window_close(window: WebviewWindow, state: State<'_, AppState>) -> AppResult<()> {
    // "Minimize to System Tray": the close button hides the window and Steamware keeps running in the
    // notification area, which is what makes downloads and the store-page plugin outlive the window.
    if state.settings.lock().map(|settings| settings.minimize_to_tray).unwrap_or(false) {
        return window.hide().map_err(|_| AppError::new("window", "Could not hide the window."));
    }
    window.close().map_err(|_| AppError::new("window", "Could not close the window."))
}
