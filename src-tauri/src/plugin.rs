//! The Plugin page: the store-page plugin's installer and health check.
//!
//! What gets installed is the same release bundle the Windows build used (`plugin.zip` plus the `winmm.dll`
//! loader proxy from `madoiscool/LTSP`), and it lands in the same two places: the frontend under
//! `%APPDATA%\Steamware\plugin`, the loader in the Steam root next to a copy of the system `winmm.dll` it
//! forwards to. Nothing about it is bespoke to this build, which is why an install made by either app is
//! recognised by the other.
//!
//! The installed tree is then rebranded to Steamware and stamped as plugin v1.0.1, exactly as the Windows
//! build did: the upstream bundle's on-screen name, links, logo and manifest all said LuaTools, and that is
//! not the product this app ships.
//!
//! Everything is fetched through the Worker's registry proxy and verified against the release's own sha256
//! digests before anything is written.

use crate::api::ApiClient;
use crate::error::{AppError, AppResult};
use crate::rebrand;
use crate::steam;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use tauri::AppHandle;
use tokio::io::AsyncWriteExt;

const PLUGIN_REPO: &str = "madoiscool/LTSP";
const ZIP_ASSET: &str = "plugin.zip";
const DLL_ASSET: &str = "winmm.dll";
const DLL_REAL: &str = "winmm_real.dll";
const CDP_MARKER: &str = ".cef-enable-remote-debugging";
/// The junction target this app creates. Ownership is decided by this path, never by "a marker exists":
/// Millennium and other loaders keep their own entry at the same name, and deleting theirs would break it.
const CDP_MARKER_TARGET: &str = "C:\\Steamware\\cdp-marker-target";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginState {
    pub installed: bool,
    pub frontend: bool,
    pub loader: bool,
    pub dll_matches: bool,
    /// Whether `dll_matches` is an answer at all. It needs the release feed's digest for the loader, so an
    /// offline app must say "Installed" rather than claim the loader is out of date.
    pub dll_verified: bool,
    pub version: Option<String>,
    pub latest: Option<String>,
    pub update_available: bool,
    pub millennium: bool,
    pub steam_path: Option<String>,
    pub status: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Manifest {
    // The Windows build writes this file with PascalCase keys. Reading those as well means an install made
    // by either app is reported with its real version instead of "—" plus a phantom update.
    #[serde(alias = "Tag")]
    tag: Option<String>,
    #[serde(alias = "DllSha")]
    dll_sha: Option<String>,
    #[serde(alias = "ZipSha")]
    zip_sha: Option<String>,
}

pub fn frontend_dir() -> PathBuf {
    // Windows keeps the historical location so an install made by either app is still found. Elsewhere
    // there is no `%APPDATA%`, and the profile folder is the platform-correct place for it.
    #[cfg(windows)]
    {
        let appdata = std::env::var("APPDATA").unwrap_or_default();
        return PathBuf::from(appdata).join("Steamware").join("plugin");
    }
    #[cfg(not(windows))]
    {
        crate::settings::app_dir().join("plugin")
    }
}

/// The frontend script the injector loads into Steam. `public/` is the layout the loader expects; a flat
/// extract (from an older release) is accepted too.
pub fn frontend_script() -> Option<PathBuf> {
    let public = frontend_dir().join("public").join("luatools.js");
    if public.is_file() {
        return Some(public);
    }
    let flat = frontend_dir().join("luatools.js");
    flat.is_file().then_some(flat)
}

fn manifest_path() -> PathBuf {
    frontend_dir().join("installed.json")
}

fn read_manifest() -> Manifest {
    std::fs::read_to_string(manifest_path())
        .ok()
        .and_then(|text| serde_json::from_str::<Manifest>(&text).ok())
        .unwrap_or_default()
}

fn write_manifest(manifest: &Manifest) -> AppResult<()> {
    std::fs::create_dir_all(frontend_dir())?;
    std::fs::write(manifest_path(), serde_json::to_vec_pretty(manifest)?)?;
    Ok(())
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

fn digest_of(asset: &serde_json::Value) -> Option<String> {
    asset["digest"]
        .as_str()
        .and_then(|value| value.strip_prefix("sha256:"))
        .map(|value| value.to_ascii_lowercase())
}

fn str(value: &serde_json::Value) -> String {
    value.as_str().unwrap_or_default().to_string()
}

async fn download(api: &ApiClient, app: &AppHandle, path: &str, dest: &Path) -> AppResult<()> {
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let response = api.stream(app, path).await?;
    let mut file = tokio::fs::File::create(dest).await?;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        file.write_all(&chunk?).await?;
    }
    file.flush().await?;
    Ok(())
}

#[cfg(windows)]
fn system_winmm() -> PathBuf {
    PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string()))
        .join("System32")
        .join("winmm.dll")
}

/// Millennium branded onto Steam's own DLL loader. Read by the plugin status and by Settings, where a user
/// who runs Millennium beside Steamware is told which loader is painting.
pub(crate) fn millennium_present(steam_root: &Path) -> bool {
    steam_root.join("millennium").join("lib").join("millennium.dll").exists()
}

/// Whether the entry at `steam_root` is the junction this app made.
#[cfg(windows)]
fn marker_is_ours(steam_root: &Path) -> bool {
    std::fs::read_link(steam_root.join(CDP_MARKER))
        .map(|target| {
            let target = target.to_string_lossy();
            let target = target.trim_end_matches(|c| c == '\\' || c == '/');
            target.eq_ignore_ascii_case(CDP_MARKER_TARGET)
        })
        .unwrap_or(false)
}

/// Steam only enables its remote-debugging port when a `.cef-enable-remote-debugging` entry exists in its
/// root. A junction (not a file) is used because Steam's own cleanup deletes plain files of that name.
///
/// Installing the plugin creates this marker, and nothing else does: the relay in `injector.rs` polls the
/// port it opens, so a Steam install without it looks exactly like a plugin that does nothing at all.
///
/// A marker we did not create is left exactly as it is, and `release_cdp_marker` will not remove it.
pub(crate) fn ensure_cdp_marker(steam_root: &Path) {
    use std::os::windows::fs::MetadataExt;
    use std::os::windows::process::CommandExt;

    let marker = steam_root.join(CDP_MARKER);
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    if let Ok(meta) = std::fs::symlink_metadata(&marker) {
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return;
        }
        let _ = if meta.is_dir() {
            std::fs::remove_dir_all(&marker)
        } else {
            std::fs::remove_file(&marker)
        };
    }

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let _ = std::process::Command::new("cmd")
        .args([
            "/c",
            "mklink",
            "/j",
            &marker.to_string_lossy(),
            CDP_MARKER_TARGET,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
}

/// Where the unix symlink points. It only has to be a directory of our own: Steam looks at the marker's
/// presence in its root, not at what it resolves to.
#[cfg(not(windows))]
fn unix_marker_target() -> PathBuf {
    std::env::temp_dir().join("steamware-cdp-marker")
}

/// The unix marker is a symlink rather than a junction. A plain file is still avoided: Steam's own cleanup
/// deletes plain files of this name, which is exactly why the Windows build uses a directory link.
#[cfg(not(windows))]
pub(crate) fn ensure_cdp_marker(steam_root: &Path) {
    use std::os::unix::fs::symlink;

    let marker = steam_root.join(CDP_MARKER);
    // A marker somebody else made (Millennium keeps its own) is left alone.
    if std::fs::symlink_metadata(&marker).is_ok() {
        return;
    }
    let target = unix_marker_target();
    let _ = std::fs::create_dir_all(&target);
    let _ = symlink(&target, &marker);
}

/// Removes the marker, but only when it is the junction this app made.
///
/// This is what keeps the port from outliving the app. The injected bundle talks to the bridge on
/// `127.0.0.1:6767`, which only exists while Steamware is running, so a marker left behind after quit buys
/// nothing and hands every later Steam launch an unauthenticated CDP port on loopback.
#[cfg(windows)]
fn release_cdp_marker(steam_root: &Path) {
    if !marker_is_ours(steam_root) {
        return;
    }
    let marker = steam_root.join(CDP_MARKER);
    // A junction is removed as a directory; the file call is a no-op safety net if that ever changes.
    let _ = std::fs::remove_dir(&marker);
    let _ = std::fs::remove_file(&marker);
}

#[cfg(not(windows))]
fn release_cdp_marker(steam_root: &Path) {
    let marker = steam_root.join(CDP_MARKER);
    let ours = std::fs::read_link(&marker)
        .map(|target| target == unix_marker_target())
        .unwrap_or(false);
    if ours {
        let _ = std::fs::remove_file(&marker);
    }
}

/// Recreates the marker at launch when the plugin is installed, because quitting removed it.
///
/// Without this the marker would only ever exist right after an install, and a Steam that starts later would
/// come up with no port for the injector to find.
pub(crate) fn ensure_marker_if_installed(steam_path: &str) {
    let Some(root) = steam::detect_install(steam_path) else {
        return;
    };
    if manifest_path().is_file() {
        ensure_cdp_marker(&root);
    } else {
        // A crash or a forced kill skips the exit hook, so a marker from a previous run can still be here
        // after the plugin was uninstalled. Nothing needs the port without the plugin: drop it if it is ours.
        release_cdp_marker(&root);
    }
}

/// Called on exit and on uninstall: drops the marker only if this app made it.
pub(crate) fn release_marker_for(steam_path: &str) {
    if let Some(root) = steam::detect_install(steam_path) {
        release_cdp_marker(&root);
    }
}

/* ── Zip extraction ────────────────────────────────────────────────────────────────────────── */

/// Extraction ceilings. `enclosed_name` stops path escape, these stop a zip bomb: a few kilobytes of archive
/// can otherwise decompress to a full disk or to one enormous allocation.
const MAX_ENTRIES: usize = 4_096;
const MAX_ENTRY_BYTES: u64 = 64 << 20;
const MAX_TOTAL_BYTES: u64 = 512 << 20;

/// Extract an archive under `dest`, refusing anything that would escape it or blow up in size.
fn extract(zip_path: &Path, dest: &Path) -> AppResult<()> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|_| AppError::new("bad_archive", "That plugin bundle is not a readable archive."))?;
    if archive.len() > MAX_ENTRIES {
        return Err(AppError::new("bad_archive", "That plugin bundle has too many files."));
    }
    std::fs::create_dir_all(dest)?;

    let mut total: u64 = 0;
    for index in 0..archive.len() {
        let Ok(mut entry) = archive.by_index(index) else { continue };
        let Some(relative) = entry.enclosed_name() else { continue };
        let target = dest.join(&relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&target)?;
            continue;
        }
        if entry.size() > MAX_ENTRY_BYTES {
            return Err(AppError::new("bad_archive", "A file in that plugin bundle is too large."));
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Read at most one byte past the ceiling so a lying header is caught too.
        let mut buffer = Vec::new();
        entry.by_ref().take(MAX_ENTRY_BYTES + 1).read_to_end(&mut buffer)?;
        if buffer.len() as u64 > MAX_ENTRY_BYTES {
            return Err(AppError::new("bad_archive", "A file in that plugin bundle is too large."));
        }
        total += buffer.len() as u64;
        if total > MAX_TOTAL_BYTES {
            return Err(AppError::new("bad_archive", "That plugin bundle unpacks to too much data."));
        }
        std::fs::write(&target, buffer)?;
    }
    Ok(())
}

/// The bundle wraps everything in one top-level folder. Flatten it so `public/luatools.js` is where the
/// loader expects it, not `plugin/public/luatools.js`.
fn hoist_single_root(dir: &Path) -> AppResult<PathBuf> {
    let entries: Vec<PathBuf> = std::fs::read_dir(dir)?
        .flatten()
        .map(|entry| entry.path())
        .collect();
    if entries.len() == 1 && entries[0].is_dir() && !dir.join("public").exists() {
        return Ok(entries[0].clone());
    }
    Ok(dir.to_path_buf())
}

fn copy_tree(source: &Path, target: &Path) -> AppResult<()> {
    std::fs::create_dir_all(target)?;
    for entry in std::fs::read_dir(source)?.flatten() {
        let path = entry.path();
        let destination = target.join(entry.file_name());
        if path.is_dir() {
            copy_tree(&path, &destination)?;
        } else {
            std::fs::copy(&path, &destination)?;
        }
    }
    Ok(())
}

/* ── State ─────────────────────────────────────────────────────────────────────────────────── */

pub async fn state(api: &ApiClient, steam_path: &str) -> PluginState {
    let root = steam::detect_install(steam_path);
    let frontend = frontend_script().is_some();
    let loader = root
        .as_ref()
        .map(|path| path.join(DLL_ASSET).exists())
        .unwrap_or(false);
    let manifest = read_manifest();

    // An install made before a rebrand pass (or by an older build) reads correctly without a reinstall.
    if frontend {
        rebrand::rebrand_dir(&frontend_dir());
    }

    // Quiet on purpose: this runs whenever a page mounts, and the interactive path would start the browser
    // check as a side effect of drawing a status tile. An unverified app simply reports what is on disk.
    let release_result = api
        .get_json_quiet(&format!("/v1/registry/release?repo={PLUGIN_REPO}"))
        .await;
    let release = release_result.as_ref().ok().cloned();
    let latest = release.as_ref().map(|body| str(&body["tag"])).filter(|tag| !tag.is_empty());

    let (dll_matches, dll_verified) = match (&root, release.as_ref()) {
        (Some(root), Some(body)) => {
            let asset = body["assets"]
                .as_array()
                .and_then(|assets| assets.iter().find(|asset| str(&asset["name"]) == DLL_ASSET));
            match (asset.and_then(digest_of), sha256_file(&root.join(DLL_ASSET))) {
                (Some(expected), Some(actual)) => (expected == actual, true),
                _ => (false, false),
            }
        }
        _ => (false, false),
    };

    // The loader DLL is Windows' way of getting the bundle into Steam's CEF. On unix the frontend is the
    // whole install, injected through Steam's debugging port, so a missing DLL there must not report the
    // plugin as absent.
    let installed = frontend && (loader || !cfg!(windows));
    let update_available = installed
        && latest
            .as_deref()
            .map(|tag| manifest.tag.as_deref() != Some(tag) || !dll_matches)
            .unwrap_or(false);

    // Two different silences: unverified is not a failure, so it names what would fix it instead of blaming
    // the network. Anything else really is the feed being unreachable.
    let status = match (&release, &release_result) {
        (Some(_), _) => None,
        (None, Err(error)) if error.code == "session_required" => {
            Some("Verify to check for plugin updates.".to_string())
        }
        _ => Some("Could not reach the release feed. Showing what is installed.".to_string()),
    };

    PluginState {
        installed,
        frontend,
        loader,
        dll_matches,
        dll_verified,
        // Steamware versions the plugin it ships itself; the upstream release tag is only used to detect
        // whether the bundle underneath changed, never as the product's version.
        version: installed.then(|| rebrand::PLUGIN_VERSION.to_string()),
        latest: Some(rebrand::PLUGIN_VERSION.to_string()),
        update_available,
        millennium: root.as_ref().map(|path| millennium_present(path)).unwrap_or(false),
        steam_path: root.map(|path| path.to_string_lossy().to_string()),
        status,
    }
}

/* ── Install / uninstall ──────────────────────────────────────────────────────────────────── */

pub async fn install(api: &ApiClient, app: &AppHandle, steam_path: &str) -> AppResult<PluginState> {
    let root = steam::detect_install(steam_path).ok_or_else(|| {
        AppError::new("steam_missing", "Steam was not found. Set its folder in Settings, then try again.")
    })?;

    let release = api
        .get_json(app, &format!("/v1/registry/release?repo={PLUGIN_REPO}"))
        .await?;
    let tag = str(&release["tag"]);
    let assets = release["assets"].as_array().cloned().unwrap_or_default();
    let zip_asset = assets
        .iter()
        .find(|asset| str(&asset["name"]) == ZIP_ASSET)
        .cloned()
        .ok_or_else(|| AppError::new("registry_failed", "That release has no plugin.zip."))?;
    let dll_asset = assets
        .iter()
        .find(|asset| str(&asset["name"]) == DLL_ASSET)
        .cloned();
    #[cfg(windows)]
    let dll_asset = dll_asset
        .ok_or_else(|| AppError::new("registry_failed", "That release has no loader DLL."))?;
    #[cfg(not(windows))]
    let dll_asset = dll_asset.unwrap_or(serde_json::Value::Null);

    let staging = std::env::temp_dir().join("Steamware").join(format!(
        "plugin-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&staging)?;

    let outcome = install_inner(api, app, &root, &tag, &zip_asset, &dll_asset, &staging).await;
    let _ = std::fs::remove_dir_all(&staging);
    outcome?;
    Ok(state(api, steam_path).await)
}

async fn install_inner(
    api: &ApiClient,
    app: &AppHandle,
    root: &Path,
    tag: &str,
    zip_asset: &serde_json::Value,
    dll_asset: &serde_json::Value,
    staging: &Path,
) -> AppResult<()> {
    let encoded_tag: String = url::form_urlencoded::byte_serialize(tag.as_bytes()).collect();
    let zip_path = staging.join(ZIP_ASSET);
    let dll_path = staging.join(DLL_ASSET);

    // The loader is a Windows DLL; on unix the bundle is the whole install and nothing is written into
    // Steam's root next to its executable.
    let mut downloads: Vec<(&str, &PathBuf, &serde_json::Value)> = vec![(ZIP_ASSET, &zip_path, zip_asset)];
    #[cfg(windows)]
    downloads.push((DLL_ASSET, &dll_path, dll_asset));
    #[cfg(not(windows))]
    let _ = dll_asset;

    for (name, dest, asset) in downloads {
        let encoded: String = url::form_urlencoded::byte_serialize(name.as_bytes()).collect();
        download(
            api,
            app,
            &format!("/v1/registry/asset?repo={PLUGIN_REPO}&tag={encoded_tag}&name={encoded}"),
            dest,
        )
        .await?;
        if let Some(expected) = digest_of(asset) {
            let actual = sha256_file(dest)
                .ok_or_else(|| AppError::new("verify_failed", "Could not hash the download."))?;
            if actual != expected {
                return Err(AppError::new(
                    "verify_failed",
                    format!("{name} did not match its published checksum, so nothing was installed."),
                ));
            }
        }
    }

    // Frontend: fresh extract, so a removed file in the new bundle actually disappears.
    let unpacked = staging.join("unpacked");
    extract(&zip_path, &unpacked)?;
    let source = hoist_single_root(&unpacked)?;
    if !source.join("public").join("luatools.js").exists() {
        return Err(AppError::new(
            "bad_bundle",
            "That bundle has no public/luatools.js, so it would not load in Steam.",
        ));
    }
    let frontend = frontend_dir();
    if frontend.exists() {
        std::fs::remove_dir_all(&frontend)?;
    }
    copy_tree(&source, &frontend)?;

    // Steamware's plugin, not the upstream bundle's: prose, links, manifest identity and logo.
    rebrand::rebrand_dir(&frontend);

    // Loader: only when the bytes actually changed, so a frontend-only update never stops Steam.
    #[cfg(windows)]
    let dll_sha = {
        let dll_target = root.join(DLL_ASSET);
        let dll_sha = sha256_file(&dll_path);
        let needs_dll = sha256_file(&dll_target) != dll_sha;
        if needs_dll {
            let was_running = steam::is_running();
            if was_running {
                steam::stop(20);
            }
            let copied = std::fs::copy(&dll_path, &dll_target).is_ok();
            let system = system_winmm();
            if copied && system.exists() {
                let _ = std::fs::copy(&system, root.join(DLL_REAL));
            }
            if was_running {
                let _ = steam::start(root);
            }
            if !copied {
                return Err(AppError::new(
                    "write_failed",
                    "Could not write the loader DLL. Close Steam and try again.",
                ));
            }
        }
        dll_sha
    };
    #[cfg(not(windows))]
    let dll_sha: Option<String> = None;

    ensure_cdp_marker(root);

    write_manifest(&Manifest {
        tag: Some(tag.to_string()),
        dll_sha,
        zip_sha: sha256_file(&zip_path),
    })?;
    Ok(())
}

pub fn uninstall(steam_path: &str) -> AppResult<()> {
    if let Some(root) = steam::detect_install(steam_path) {
        let was_running = steam::is_running();
        if was_running {
            steam::stop(20);
        }
        let _ = std::fs::remove_file(root.join(DLL_ASSET));
        let _ = std::fs::remove_file(root.join(DLL_REAL));
        release_cdp_marker(&root);
        if was_running {
            let _ = steam::start(&root);
        }
    }
    if frontend_dir().exists() {
        std::fs::remove_dir_all(frontend_dir())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hoists_a_single_wrapped_folder() {
        let dir = std::env::temp_dir().join("steamware-plugin-hoist");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("paste").join("public")).unwrap();
        std::fs::write(dir.join("paste").join("public").join("luatools.js"), "//").unwrap();

        let root = hoist_single_root(&dir).unwrap();
        assert_eq!(root, dir.join("paste"), "the wrapper folder is dropped");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn keeps_a_flat_bundle_flat() {
        let dir = std::env::temp_dir().join("steamware-plugin-flat");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("public")).unwrap();
        std::fs::write(dir.join("public").join("luatools.js"), "//").unwrap();

        let root = hoist_single_root(&dir).unwrap();
        assert_eq!(root, dir);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn digest_parsing_ignores_other_algorithms() {
        let asset = serde_json::json!({ "name": "plugin.zip", "digest": "sha256:ABCD" });
        assert_eq!(digest_of(&asset).as_deref(), Some("abcd"));
        let missing = serde_json::json!({ "name": "plugin.zip", "digest": null });
        assert!(digest_of(&missing).is_none());
    }
}
