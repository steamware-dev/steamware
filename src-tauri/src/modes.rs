//! The Mode page: the mutually-exclusive Steam unlockers, plus the CloudRedirect add-on.
//!
//! The bytes still come from the same two upstream channels the Windows build used — OpenSteamTools'
//! nightly mirror and BetterSteamTools' release feed — but this build never talks to GitHub itself. It asks
//! the Worker for the release metadata and then for the named asset, so a user's IP is never in GitHub's
//! logs, a blocked region still works, and the app's only outbound host is still the one in Settings.
//!
//! Installing means: stop Steam, verify, copy three DLLs into the Steam root, point
//! `opensteamtool.toml` at `config/stplug-in`, start Steam. Verification is a sha256 of the downloaded
//! archive against the release's own published digest (and, for BetterSteamTools, of the payload DLL against
//! its manifest), so a truncated download or a substituted asset is refused rather than installed.

use crate::api::ApiClient;
use crate::error::{AppError, AppResult};
use crate::settings;
use crate::steam;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::AppHandle;
use tokio::io::AsyncWriteExt;

const PLACE_FILES: [&str; 3] = ["dwmapi.dll", "xinput1_4.dll", "OpenSteamTool.dll"];
const CLOUD_DLL: &str = "cloud_redirect.dll";
const CLOUD_REPO: &str = "Selectively11/CloudRedirect";
const LUA_PATH_VALUE: &str = "config/stplug-in";

struct ModeDef {
    id: &'static str,
    title: &'static str,
    description: &'static str,
    repo: &'static str,
    /// BetterSteamTools publishes version + payload hash in a raw manifest instead of release assets.
    manifest: bool,
}

fn defs() -> [ModeDef; 2] {
    [
        ModeDef {
            id: "ost",
            title: "OpenSteamTools",
            description: "The nightly channel of upstream OpenSteamTool. Newest features, and the channel \
                          CloudRedirect is built for.",
            repo: "madoiscool/OST-Nightly",
            manifest: false,
        },
        ModeDef {
            id: "bst",
            title: "BetterSteamTools",
            description: "A fork of OpenSteamTool with fixes on top. The recommended default.",
            repo: "madoiscool/BetterSteamTools",
            manifest: true,
        },
    ]
}

fn def(id: &str) -> Option<ModeDef> {
    defs().into_iter().find(|entry| entry.id == id)
}

/// The loader is three Windows DLLs written into Steam's root. A Linux or macOS Steam has no such loader, so
/// an "install" there would copy Windows binaries into its folder and report success.
fn windows_only() -> AppResult<()> {
    #[cfg(windows)]
    {
        Ok(())
    }
    #[cfg(not(windows))]
    {
        Err(AppError::new(
            "unsupported_platform",
            "That unlocker is a Windows loader. On this platform, use lua files and the store plugin instead.",
        ))
    }
}

/* ── Persisted selection ─────────────────────────────────────────────────────────────────────── */

#[derive(Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Store {
    selected: Option<String>,
    installed_version: Option<String>,
    cloud_version: Option<String>,
}

fn store_path() -> PathBuf {
    settings::app_dir().join("modes.json")
}

fn load_store() -> Store {
    std::fs::read_to_string(store_path())
        .ok()
        .and_then(|text| serde_json::from_str::<Store>(&text).ok())
        .unwrap_or_default()
}

fn save_store(store: &Store) -> AppResult<()> {
    let path = store_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, serde_json::to_vec_pretty(store)?)?;
    let _ = std::fs::remove_file(&path);
    std::fs::rename(&temp, &path)?;
    Ok(())
}

/* ── Shapes ─────────────────────────────────────────────────────────────────────────────────── */

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudState {
    pub installed: bool,
    pub enabled: bool,
    pub update_available: bool,
    pub unlocked: bool,
    pub status: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModeCard {
    pub id: String,
    pub title: String,
    pub description: String,
    pub active: bool,
    pub installed: bool,
    pub version: Option<String>,
    pub latest: Option<String>,
    pub update_available: bool,
    pub status: String,
    pub recommended: bool,
    pub experimental: bool,
    pub managed: bool,
    /// The loader can be installed on this platform; false on macOS and Linux.
    pub supported: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModesState {
    pub steam_path: Option<String>,
    pub cards: Vec<ModeCard>,
    pub cloud: CloudState,
}

/* ── Helpers ────────────────────────────────────────────────────────────────────────────────── */

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn str(value: &serde_json::Value) -> String {
    value.as_str().unwrap_or_default().to_string()
}

fn sha256_file(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Some(hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect())
}

fn release_tag(digest: &str) -> Option<String> {
    digest.strip_prefix("sha256:").map(|value| value.to_ascii_lowercase())
}

/// Pick the Release archive out of a release's assets: the exact `-Release.zip` first, then any zip that
/// says Release and is not a Debug build.
fn pick_zip(assets: &serde_json::Value) -> Option<(String, Option<String>)> {
    let list = assets.as_array()?;
    let mut fallback: Option<(String, Option<String>)> = None;
    for asset in list {
        let name = str(&asset["name"]);
        let lower = name.to_ascii_lowercase();
        if !lower.ends_with(".zip") || lower.contains("debug") {
            continue;
        }
        let digest = asset["digest"].as_str().map(|value| value.to_string());
        if lower.contains("release") {
            if lower.contains("-release.zip") {
                return Some((name, digest));
            }
            fallback.get_or_insert((name, digest));
        }
    }
    fallback
}

async fn download(api: &ApiClient, app: &AppHandle, path: &str, dest: &Path) -> AppResult<u64> {
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let response = api.stream(app, path).await?;
    let mut file = tokio::fs::File::create(dest).await?;
    let mut stream = response.bytes_stream();
    let mut written = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        written += chunk.len() as u64;
        if written > MAX_RELEASE_BYTES {
            return Err(AppError::new("too_large", "That release is larger than Steamware will save."));
        }
    }
    file.flush().await?;
    Ok(written)
}

/// Only these three names are ever written, so a traversal is impossible; the size ceiling is here so a
/// corrupted or swapped release archive cannot decompress into gigabytes of memory.
const MAX_ENTRY_BYTES: u64 = 128 << 20;

/// Ceiling for the archive itself while it streams to disk.
const MAX_RELEASE_BYTES: u64 = 1 << 30;

fn extract_files(zip_path: &Path, wanted: &[&str], dest_dir: &Path) -> AppResult<Vec<String>> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|_| AppError::new("bad_archive", "That release is not a readable archive."))?;
    std::fs::create_dir_all(dest_dir)?;
    let mut found: Vec<String> = Vec::new();

    for index in 0..archive.len() {
        let Ok(mut entry) = archive.by_index(index) else { continue };
        let Some(name) = entry.enclosed_name().and_then(|path| {
            path.file_name().map(|name| name.to_string_lossy().to_string())
        }) else {
            continue;
        };
        let Some(wanted_name) = wanted.iter().find(|entry_name| entry_name.eq_ignore_ascii_case(&name))
        else {
            continue;
        };
        if found.iter().any(|existing| existing.eq_ignore_ascii_case(wanted_name)) {
            continue;
        }
        let mut buffer = Vec::new();
        entry.by_ref().take(MAX_ENTRY_BYTES + 1).read_to_end(&mut buffer)?;
        if buffer.len() as u64 > MAX_ENTRY_BYTES {
            return Err(AppError::new("bad_archive", "A file in that release is too large."));
        }
        std::fs::write(dest_dir.join(*wanted_name), buffer)?;
        found.push((*wanted_name).to_string());
    }
    Ok(found)
}

/* ── opensteamtool.toml ─────────────────────────────────────────────────────────────────────── */

fn section_bounds(lines: &[String], section: &str) -> Option<(usize, usize)> {
    let header = lines.iter().position(|line| {
        let trimmed = line.trim();
        !trimmed.starts_with('#') && trimmed.eq_ignore_ascii_case(&format!("[{section}]"))
    })?;
    let end = lines
        .iter()
        .enumerate()
        .skip(header + 1)
        .find(|(_, line)| {
            let trimmed = line.trim();
            !trimmed.starts_with('#') && trimmed.starts_with('[') && trimmed.ends_with(']')
        })
        .map(|(index, _)| index)
        .unwrap_or(lines.len());
    Some((header, end))
}

/// Point the unlocker at `config/stplug-in` so lua files written there hot-reload instead of needing a
/// restart. Append-only and comment-preserving: a path another tool already added is left exactly as is.
///
/// Called from two places, and the second is the one that matters in normal use: installing a mode, and
/// every lua install (`lua::install_archive`). A user who set their unlocker up outside this app, or whose
/// `opensteamtool.toml` was rewritten by a mode update, would otherwise get files in the folder with
/// nothing watching them — an install that reports success and changes nothing until Steam restarts.
pub(crate) fn ensure_lua_path(steam_root: &Path) -> AppResult<()> {
    let path = steam_root.join("opensteamtool.toml");
    let Ok(text) = std::fs::read_to_string(&path) else {
        std::fs::write(&path, format!("[lua]\npaths = [\"{LUA_PATH_VALUE}\"]\n"))?;
        return Ok(());
    };
    if text.contains(LUA_PATH_VALUE) {
        return Ok(());
    }

    let mut lines: Vec<String> = text.split('\n').map(|line| line.to_string()).collect();
    match section_bounds(&lines, "lua") {
        Some((header, end)) => {
            let key = lines
                .iter()
                .enumerate()
                .take(end)
                .skip(header + 1)
                .find(|(_, line)| {
                    let trimmed = line.trim();
                    !trimmed.starts_with('#') && trimmed.starts_with("paths")
                })
                .map(|(index, _)| index);
            match key {
                Some(index) => {
                    let line = lines[index].clone();
                    if let Some(close) = line.rfind(']') {
                        let mut updated = line.clone();
                        let separator = if line[..close].trim_end().ends_with('[') { "" } else { ", " };
                        updated.insert_str(close, &format!("{separator}\"{LUA_PATH_VALUE}\""));
                        lines[index] = updated;
                    }
                }
                None => lines.insert(header + 1, format!("paths = [\"{LUA_PATH_VALUE}\"]")),
            }
        }
        None => {
            lines.push(String::new());
            lines.push("[lua]".to_string());
            lines.push(format!("paths = [\"{LUA_PATH_VALUE}\"]"));
        }
    }

    std::fs::write(&path, lines.join("\n"))?;
    Ok(())
}

fn read_cloud_enabled(steam_root: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(steam_root.join("opensteamtool.toml")) else {
        return false;
    };
    let lines: Vec<String> = text.split('\n').map(|line| line.to_string()).collect();
    let Some((header, end)) = section_bounds(&lines, "cloud") else {
        return false;
    };
    lines
        .iter()
        .take(end)
        .skip(header + 1)
        .find_map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with('#') {
                return None;
            }
            let (key, value) = trimmed.split_once('=')?;
            if !key.trim().eq_ignore_ascii_case("enabled") {
                return None;
            }
            Some(value.trim().trim_matches('"').eq_ignore_ascii_case("true"))
        })
        .unwrap_or(false)
}

fn set_cloud_enabled(steam_root: &Path, enabled: bool) -> AppResult<()> {
    let path = steam_root.join("opensteamtool.toml");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = if text.is_empty() {
        Vec::new()
    } else {
        text.split('\n').map(|line| line.to_string()).collect()
    };
    let value = if enabled { "true" } else { "false" };

    match section_bounds(&lines, "cloud") {
        Some((header, end)) => {
            let key = lines
                .iter()
                .enumerate()
                .take(end)
                .skip(header + 1)
                .find(|(_, line)| {
                    let trimmed = line.trim();
                    !trimmed.starts_with('#') && trimmed.split('=').next().unwrap_or("").trim().eq_ignore_ascii_case("enabled")
                })
                .map(|(index, _)| index);
            match key {
                Some(index) => lines[index] = format!("enabled = {value}"),
                None => lines.insert(header + 1, format!("enabled = {value}")),
            }
        }
        None => {
            lines.push(String::new());
            lines.push("[cloud]".to_string());
            lines.push(format!("enabled = {value}"));
        }
    }

    std::fs::write(&path, lines.join("\n"))?;
    Ok(())
}

/* ── State ─────────────────────────────────────────────────────────────────────────────────── */

fn installed_files(steam_root: &Path) -> bool {
    PLACE_FILES.iter().all(|name| steam_root.join(name).exists())
}

/// The newest version a mode could be installed at, for the Mode page's "update available" line.
///
/// Quiet on purpose: the Mode page and the first-run sheet read this as they mount, and the interactive path
/// answered "no session yet" by opening the browser — a page render must not do that. An unverified app just
/// shows the mode without a version to compare against; pressing Install verifies once, properly.
async fn latest_version(api: &ApiClient, def: &ModeDef) -> Option<String> {
    if def.manifest {
        let body = api.get_json_quiet("/v1/registry/bst").await.ok()?;
        return Some(str(&body["version"]));
    }
    let body = api
        .get_json_quiet(&format!("/v1/registry/release?repo={}", def.repo))
        .await
        .ok()?;
    Some(str(&body["tag"]))
}

fn cloud_state(steam_root: Option<&Path>, store: &Store, selected: Option<&str>, latest: Option<&str>) -> CloudState {
    let Some(root) = steam_root else {
        return CloudState {
            installed: false,
            enabled: false,
            update_available: false,
            unlocked: false,
            status: "Steam was not found.".to_string(),
        };
    };
    let unlocked = matches!(selected, Some("ost") | Some("bst"));
    let installed = root.join(CLOUD_DLL).exists();
    let enabled = read_cloud_enabled(root);
    let update_available = unlocked
        && installed
        && match (latest, store.cloud_version.as_deref()) {
            (Some(latest), Some(current)) => latest != current,
            (Some(_), None) => true,
            _ => false,
        };
    let status = if !unlocked {
        "Available with OpenSteamTools or BetterSteamTools.".to_string()
    } else if !installed {
        "Not installed.".to_string()
    } else if update_available {
        "An update is available.".to_string()
    } else if enabled {
        "Enabled — takes effect the next time Steam starts.".to_string()
    } else {
        "Installed but disabled.".to_string()
    };
    CloudState { installed, enabled, update_available, unlocked, status }
}

pub async fn state(api: &ApiClient, steam_path: &str) -> ModesState {
    let root = steam::detect_install(steam_path);
    let store = load_store();
    let selected = store.selected.clone();
    let mut cards = Vec::new();

    for def in defs() {
        let active = selected.as_deref() == Some(def.id);
        let installed = root.as_ref().map(|path| installed_files(path)).unwrap_or(false);
        let latest = if active { latest_version(api, &def).await } else { None };
        let version = if active { store.installed_version.clone() } else { None };
        let update_available = active
            && installed
            && matches!((&latest, &version), (Some(latest), Some(current)) if latest != current);

        let supported = cfg!(windows);
        let status = if !supported {
            "Windows only. Nothing is written on this platform.".to_string()
        } else if active {
            if installed {
                if update_available {
                    "An update is available.".to_string()
                } else if latest.is_none() {
                    "Installed. Could not check for updates.".to_string()
                } else {
                    "Installed and up to date.".to_string()
                }
            } else {
                "Active, but its files are missing. Reinstall it.".to_string()
            }
        } else {
            "Not active.".to_string()
        };

        cards.push(ModeCard {
            id: def.id.to_string(),
            title: def.title.to_string(),
            description: def.description.to_string(),
            active,
            installed,
            version,
            latest,
            update_available,
            status,
            recommended: def.id == "bst",
            experimental: def.id == "ost",
            managed: true,
            supported,
        });
    }

    cards.push(ModeCard {
        id: "custom".to_string(),
        title: "Custom".to_string(),
        description: "You install and update your own unlocker. Steamware places and verifies nothing."
            .to_string(),
        active: selected.as_deref() == Some("custom"),
        installed: root.as_ref().map(|path| installed_files(path)).unwrap_or(false),
        version: None,
        latest: None,
        update_available: false,
        status: "You manage these files.".to_string(),
        recommended: false,
        experimental: false,
        managed: false,
        supported: true,
    });

    // Quiet like `latest_version`: this is a status read on page mount, so an unverified app shows the mode
    // without a version to compare rather than opening a browser to find one out.
    let cloud_latest = if matches!(selected.as_deref(), Some("ost") | Some("bst")) {
        api.get_json_quiet(&format!("/v1/registry/release?repo={CLOUD_REPO}"))
            .await
            .ok()
            .map(|body| str(&body["tag"]))
    } else {
        None
    };

    let cloud = cloud_state(
        root.as_deref(),
        &store,
        selected.as_deref(),
        cloud_latest.as_deref(),
    );

    ModesState {
        steam_path: root.map(|path| path.to_string_lossy().to_string()),
        cards,
        cloud,
    }
}

/* ── Install ───────────────────────────────────────────────────────────────────────────────── */

pub async fn install(
    api: &ApiClient,
    app: &AppHandle,
    steam_path: &str,
    mode_id: &str,
) -> AppResult<ModesState> {
    if mode_id == "custom" {
        let mut store = load_store();
        store.selected = Some("custom".to_string());
        save_store(&store)?;
        return Ok(state(api, steam_path).await);
    }

    windows_only()?;

    let def = def(mode_id)
        .ok_or_else(|| AppError::new("unknown_mode", "That mode does not exist."))?;
    let root = steam::detect_install(steam_path).ok_or_else(|| {
        AppError::new("steam_missing", "Steam was not found. Set its folder in Settings, then try again.")
    })?;

    let staging = std::env::temp_dir().join("Steamware").join(format!("mode-{}", now_secs()));
    std::fs::create_dir_all(&staging)?;

    let result = install_inner(api, app, &root, &def, &staging).await;
    let _ = std::fs::remove_dir_all(&staging);
    result?;

    let mut store = load_store();
    store.selected = Some(def.id.to_string());
    save_store(&store)?;
    Ok(state(api, steam_path).await)
}

async fn install_inner(
    api: &ApiClient,
    app: &AppHandle,
    root: &Path,
    def: &ModeDef,
    staging: &Path,
) -> AppResult<()> {
    // 1. Resolve the release through the Worker.
    let (tag, asset_name, expected_archive, expected_payload) = if def.manifest {
        let manifest = api.get_json(app, "/v1/registry/bst").await?;
        let version = str(&manifest["version"]);
        if version.is_empty() {
            return Err(AppError::new("registry_failed", "The release feed returned no version."));
        }
        let payload_file = str(&manifest["file"]);
        let payload_sha = str(&manifest["sha256"]);
        if !payload_file.eq_ignore_ascii_case(PLACE_FILES[2]) || payload_sha.is_empty() {
            return Err(AppError::new(
                "registry_failed",
                "That release's manifest does not describe OpenSteamTool.dll.",
            ));
        }
        (
            version.clone(),
            format!("OpenSteamTool-{version}-Release.zip"),
            None,
            Some(payload_sha),
        )
    } else {
        let body = api
            .get_json(app, &format!("/v1/registry/release?repo={}", def.repo))
            .await?;
        let tag = str(&body["tag"]);
        let Some((name, digest)) = pick_zip(&body["assets"]) else {
            return Err(AppError::new(
                "registry_failed",
                "That release has no installable archive.",
            ));
        };
        (tag, name, digest.as_deref().and_then(release_tag), None)
    };

    if tag.is_empty() {
        return Err(AppError::new("registry_failed", "That release has no version tag."));
    }

    // 2. Download the archive, through the Worker only.
    let zip_path = staging.join("release.zip");
    let encoded_tag: String = url::form_urlencoded::byte_serialize(tag.as_bytes()).collect();
    let encoded_name: String = url::form_urlencoded::byte_serialize(asset_name.as_bytes()).collect();
    download(
        api,
        app,
        &format!("/v1/registry/asset?repo={}&tag={encoded_tag}&name={encoded_name}", def.repo),
        &zip_path,
    )
    .await?;

    // 3. Verify before touching Steam.
    if let Some(expected) = expected_archive {
        let actual = sha256_file(&zip_path)
            .ok_or_else(|| AppError::new("verify_failed", "Could not hash the download."))?;
        if actual != expected {
            return Err(AppError::new(
                "verify_failed",
                "The download did not match its published checksum, so nothing was installed.",
            ));
        }
    }

    let staged_dir = staging.join("files");
    let found = extract_files(&zip_path, &PLACE_FILES, &staged_dir)?;
    let missing: Vec<&str> = PLACE_FILES
        .iter()
        .copied()
        .filter(|name| !found.iter().any(|entry| entry.eq_ignore_ascii_case(name)))
        .collect();
    if !missing.is_empty() {
        return Err(AppError::new(
            "release_missing_files",
            format!("That release is missing {}.", missing.join(", ")),
        ));
    }

    if let Some(expected) = expected_payload {
        let payload = staged_dir.join(PLACE_FILES[2]);
        let actual = sha256_file(&payload)
            .ok_or_else(|| AppError::new("verify_failed", "Could not hash the payload."))?;
        if actual != expected.to_ascii_lowercase() {
            return Err(AppError::new(
                "verify_failed",
                "The payload did not match its published checksum, so nothing was installed.",
            ));
        }
    }

    // 4. Swap the files with Steam closed.
    let was_running = steam::is_running();
    if was_running {
        steam::stop(20);
    }

    let mut failed: Vec<&str> = Vec::new();
    for name in PLACE_FILES {
        if std::fs::copy(staged_dir.join(name), root.join(name)).is_err() {
            failed.push(name);
        }
    }
    if let Err(error) = ensure_lua_path(root) {
        eprintln!("opensteamtool.toml update failed: {}", error.code);
    }

    if was_running {
        let _ = steam::start(root);
    }

    if !failed.is_empty() {
        return Err(AppError::new(
            "write_failed",
            format!(
                "Could not write {}. Close Steam and try again.",
                failed.join(", ")
            ),
        ));
    }

    let mut store = load_store();
    store.installed_version = Some(tag);
    save_store(&store)?;
    Ok(())
}

/* ── CloudRedirect ─────────────────────────────────────────────────────────────────────────── */

pub async fn cloud_redirect_set(
    api: &ApiClient,
    app: &AppHandle,
    steam_path: &str,
    enable: bool,
    force_update: bool,
) -> AppResult<ModesState> {
    windows_only()?;

    let root = steam::detect_install(steam_path).ok_or_else(|| {
        AppError::new("steam_missing", "Steam was not found. Set its folder in Settings, then try again.")
    })?;
    let store = load_store();
    if !matches!(store.selected.as_deref(), Some("ost") | Some("bst")) {
        return Err(AppError::new(
            "cloud_locked",
            "CloudRedirect needs OpenSteamTools or BetterSteamTools as the active mode.",
        ));
    }

    let dll = root.join(CLOUD_DLL);
    let needs_file = force_update || !dll.exists();
    if needs_file {
        let release = api
            .get_json(app, &format!("/v1/registry/release?repo={CLOUD_REPO}"))
            .await?;
        let tag = str(&release["tag"]);
        let Some(assets) = release["assets"].as_array() else {
            return Err(AppError::new("registry_failed", "That release has no assets."));
        };
        let asset = assets
            .iter()
            .map(|entry| (str(&entry["name"]), entry["digest"].as_str().map(|v| v.to_string())))
            .find(|(name, _)| name.to_ascii_lowercase().ends_with(".dll"))
            .ok_or_else(|| AppError::new("registry_failed", "That release has no DLL."))?;

        let staging = std::env::temp_dir().join("Steamware").join(format!("cloud-{}", now_secs()));
        std::fs::create_dir_all(&staging)?;
        let target = staging.join(CLOUD_DLL);
        let encoded_tag: String = url::form_urlencoded::byte_serialize(tag.as_bytes()).collect();
        let encoded_name: String = url::form_urlencoded::byte_serialize(asset.0.as_bytes()).collect();
        let outcome = async {
            download(
                api,
                app,
                &format!("/v1/registry/asset?repo={CLOUD_REPO}&tag={encoded_tag}&name={encoded_name}"),
                &target,
            )
            .await?;
            if let Some(expected) = asset.1.as_deref().and_then(release_tag) {
                let actual = sha256_file(&target)
                    .ok_or_else(|| AppError::new("verify_failed", "Could not hash the download."))?;
                if actual != expected {
                    return Err(AppError::new("verify_failed", "The download did not match its checksum."));
                }
            }
            let was_running = steam::is_running();
            if was_running {
                steam::stop(20);
            }
            let copied = std::fs::copy(&target, &dll).is_ok();
            if was_running {
                let _ = steam::start(&root);
            }
            if !copied {
                return Err(AppError::new(
                    "write_failed",
                    "Could not write cloud_redirect.dll. Close Steam and try again.",
                ));
            }
            Ok::<(), AppError>(())
        }
        .await;
        let _ = std::fs::remove_dir_all(&staging);
        outcome?;

        let mut store = load_store();
        store.cloud_version = Some(tag);
        save_store(&store)?;
    }

    set_cloud_enabled(&root, enable)?;

    // Enabling without the file present cannot work; the caller above guarantees it exists by now.
    Ok(state(api, steam_path).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lua_path_is_added_once() {
        let dir = std::env::temp_dir().join("steamware-modes-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        ensure_lua_path(&dir).unwrap();
        let created = std::fs::read_to_string(dir.join("opensteamtool.toml")).unwrap();
        assert!(created.contains("[lua]"));

        ensure_lua_path(&dir).unwrap();
        let again = std::fs::read_to_string(dir.join("opensteamtool.toml")).unwrap();
        assert_eq!(created, again, "a second run must not duplicate the path");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn lua_path_appends_to_an_existing_array() {
        let dir = std::env::temp_dir().join("steamware-modes-array");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("opensteamtool.toml"),
            "[lua]\npaths = [\"other\"]\n\n[log]\nlevel = \"info\"\n",
        )
        .unwrap();

        ensure_lua_path(&dir).unwrap();
        let text = std::fs::read_to_string(dir.join("opensteamtool.toml")).unwrap();
        assert!(text.contains("paths = [\"other\", \"config/stplug-in\"]"), "{text}");
        assert!(text.contains("[log]"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn cloud_flag_round_trips() {
        let dir = std::env::temp_dir().join("steamware-modes-cloud");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        set_cloud_enabled(&dir, true).unwrap();
        assert!(read_cloud_enabled(&dir));
        set_cloud_enabled(&dir, false).unwrap();
        assert!(!read_cloud_enabled(&dir));

        let text = std::fs::read_to_string(dir.join("opensteamtool.toml")).unwrap();
        assert_eq!(text.matches("[cloud]").count(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn picks_the_release_zip() {
        let assets = serde_json::json!([
            { "name": "OpenSteamTool-1.2-Debug.zip", "digest": null },
            { "name": "OpenSteamTool-1.2-Release.zip", "digest": "sha256:AB" }
        ]);
        let pick = pick_zip(&assets).expect("asset");
        assert_eq!(pick.0, "OpenSteamTool-1.2-Release.zip");
        assert_eq!(pick.1.as_deref(), Some("sha256:AB"));
    }
}
