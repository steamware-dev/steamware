//! The update gate.
//!
//! One file on the internet decides whether this build may still be used: the newest published release in
//! the repository. Nothing is signed and nothing is licensed, so the only honest way to keep a community on
//! the same version is to say so plainly — an old build puts the user on a storefront the server no longer
//! answers for, and the bugs they report were fixed months ago.
//!
//! Three rules shape the whole module:
//!
//! * **It fails open on a first look.** A build that cannot reach GitHub — offline, on a network that blocks
//!   it, on a launch-day spike — has learned nothing and therefore blocks nobody. A launch is exactly when
//!   the feed is least likely to answer.
//! * **It never fails open once it knows.** After one successful read, an outage does not unlock the gate:
//!   otherwise "block github.com" would be the way past it. Only the feed saying "you are current" clears it.
//! * **It only ever blocks on a positive, comparable answer.** A tag that is not a version number, a release
//!   that is older than this build, a draft — all of them leave `required` false. A gate that misfires is
//!   worse than no gate, because it locks everybody out of an app that works.
//!
//! `STEAMWARE_UPDATE_FEED` overrides the endpoint. It exists so the gate can be exercised against a fixture
//! before a release is really published; it can only make a build look older, since the version it is
//! compared against is compiled in.

use crate::error::{AppError, AppResult};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

/// Where a build asks what the newest version is. `/latest` ignores drafts and pre-releases, so publishing
/// one of those is a safe way to stage a release without moving the gate.
pub const DEFAULT_FEED: &str = "https://api.github.com/repos/steamware-dev/steamware/releases/latest";

/// How long a read of the feed is reused. GitHub's unauthenticated limit is per address, and a whole
/// household or campus sits behind one, so this is deliberately not "on every page".
const RECHECK_SECS: i64 = 15 * 60;

/// How soon to try again after a failed read. Short, because the usual failure is a laptop that opened
/// before its network did.
const RETRY_SECS: i64 = 2 * 60;

/// Release notes are shown in the gate, not stored: a long changelog is trimmed rather than allowed to push
/// the buttons off the card.
const NOTES_LIMIT: usize = 1_400;

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

/// The version this process was compiled as.
pub fn current() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// `v1.0.4`, `1.0.4`, `1.0.4-rc1` → `(1, 0, 4)`.
///
/// Pre-release suffixes are dropped: the feed's `/latest` never returns one, and a build tagged
/// `1.0.4-beta` must still compare as 1.0.4 rather than as unparseable.
fn parse_version(raw: &str) -> Option<(u64, u64, u64)> {
    let core = raw.trim().trim_start_matches(['v', 'V']);
    let core = core.split(['-', '+']).next().unwrap_or("");
    let mut parts = core.split('.');
    let major: u64 = parts.next()?.parse().ok()?;
    let minor: u64 = match parts.next() {
        Some(part) => part.parse().ok()?,
        None => 0,
    };
    let patch: u64 = match parts.next() {
        Some(part) => part.parse().ok()?,
        None => 0,
    };
    Some((major, minor, patch))
}

fn normalised(raw: &str) -> Option<String> {
    parse_version(raw).map(|(major, minor, patch)| format!("{major}.{minor}.{patch}"))
}

/// What the feed said, in the shape the rest of the app uses.
#[derive(Clone, Default)]
struct Release {
    version: String,
    page: String,
    notes: String,
    setup: Option<Setup>,
}

/// The `…-setup.exe` attached to a release.
#[derive(Clone)]
struct Setup {
    name: String,
    url: String,
    /// `sha256:…` when GitHub publishes one. Verified when present; the DOS header is checked regardless.
    digest: Option<String>,
}

/// What the gate needs to draw itself.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub current: String,
    pub latest: String,
    /// A strictly newer release exists. The frontend renders the gate on this and nothing else.
    pub required: bool,
    pub release_url: String,
    pub setup_url: String,
    /// The installer can be fetched and run from here, rather than sending the user to GitHub.
    pub can_install: bool,
    pub notes: String,
    pub checked_at: i64,
    /// Why the last look failed, when it did. Shown in small print; never a reason to block.
    pub error: Option<String>,
}

#[derive(Default)]
struct Cache {
    release: Option<Release>,
    checked_at: i64,
    error: Option<String>,
}

/// The app's one piece of update state, shared by the boot poll, the background re-check and the UI.
pub struct Updater {
    cache: Mutex<Cache>,
}

impl Updater {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { cache: Mutex::new(Cache::default()) })
    }

    /// The cached answer. This never touches the network, so the window may poll it as often as it likes.
    pub async fn status(&self) -> UpdateStatus {
        let current = current();
        let cache = self.cache.lock().await;
        let release = cache.release.clone();
        let theirs = release.as_ref().and_then(|release| parse_version(&release.version));
        let required = match (parse_version(&current), theirs) {
            (Some(here), Some(there)) => there > here,
            _ => false,
        };
        let setup_url = release
            .as_ref()
            .and_then(|release| release.setup.as_ref())
            .map(|setup| setup.url.clone())
            .unwrap_or_default();
        UpdateStatus {
            latest: release
                .as_ref()
                .map(|release| release.version.clone())
                .unwrap_or_else(|| current.clone()),
            required,
            // Any Windows copy can update from here. The installer is opened visibly rather than forced
            // over the current folder, so a portable build is not silently replaced by a second install.
            can_install: required && !setup_url.is_empty() && cfg!(windows),
            release_url: release.as_ref().map(|release| release.page.clone()).unwrap_or_default(),
            notes: release
                .as_ref()
                .map(|release| trim_notes(&release.notes))
                .unwrap_or_default(),
            setup_url,
            current,
            checked_at: cache.checked_at,
            error: cache.error.clone(),
        }
    }

    /// Ask GitHub, unless the last answer is still fresh. `force` is the "check again" in the gate.
    pub async fn refresh(&self, force: bool) -> UpdateStatus {
        let skip = {
            let cache = self.cache.lock().await;
            let window = if cache.error.is_some() { RETRY_SECS } else { RECHECK_SECS };
            !force && cache.checked_at > 0 && now_secs() - cache.checked_at < window
        };
        if skip {
            return self.status().await;
        }

        let answer = fetch().await;
        {
            let mut cache = self.cache.lock().await;
            cache.checked_at = now_secs();
            match answer {
                Ok(release) => {
                    cache.release = Some(release);
                    cache.error = None;
                }
                // Deliberately keeps `release`: a build that already knows it is old stays gated while the
                // feed is unreachable, so blocking the request is not a way past the gate.
                Err(error) => cache.error = Some(error.message),
            }
        }
        self.status().await
    }

    /// Download the release's installer, hand it to a helper that waits for this process to end, and quit.
    ///
    /// The order matters: NSIS cannot replace `steamware.exe` while it is running, so the installer is
    /// started by a detached process that waits on our own pid rather than racing our exit.
    pub async fn install(&self, app: &AppHandle) -> AppResult<()> {
        let status = self.refresh(true).await;
        if !status.required {
            return Err(AppError::new("up_to_date", "This copy is already the newest version."));
        }
        let release = self
            .cache
            .lock()
            .await
            .release
            .clone()
            .ok_or_else(|| AppError::new("no_release", "The newest release could not be read."))?;
        let Some(setup) = release.setup else {
            return Err(AppError::new(
                "no_installer",
                "This release has no installer attached. Download it from the release page.",
            ));
        };
        let installer = download(app, &setup).await?;
        launch_installer_after_exit(&installer)?;
        app.exit(0);
        Ok(())
    }
}

/// A Windows program begins with `MZ`. Anything else is an error page saved under an `.exe` name.
fn looks_like_installer(head: &[u8]) -> bool {
    head.len() >= 2 && &head[..2] == b"MZ"
}

async fn fetch() -> AppResult<Release> {
    let url = std::env::var("STEAMWARE_UPDATE_FEED").unwrap_or_else(|_| DEFAULT_FEED.to_string());
    // GitHub rejects a request without a User-Agent, and names the app in its logs when one is present.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .user_agent(concat!("Steamware/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let response = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(AppError::new(
            "update_feed_unavailable",
            "GitHub did not answer the update check.",
        ));
    }
    let body = response.text().await?;
    let json: Value = serde_json::from_str(&body).map_err(|_| {
        AppError::new("update_feed_unavailable", "The update check got an answer it could not read.")
    })?;
    parse_release(&json).ok_or_else(|| {
        AppError::new("update_feed_unavailable", "The newest release does not name a version.")
    })
}

fn parse_release(json: &Value) -> Option<Release> {
    let version = normalised(json.get("tag_name")?.as_str()?)?;
    let text = |key: &str| {
        json.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let setup = json.get("assets").and_then(Value::as_array).and_then(|assets| {
        assets.iter().find_map(|asset| {
            let name = asset.get("name")?.as_str()?;
            let lower = name.to_lowercase();
            // Tauri names the bundle `Steamware_<version>_x64-setup.exe`. "installer" is accepted too, so a
            // rename in the release does not quietly disable in-app updates — but a bare `.exe` is not,
            // because the releases also carry the standalone builds, and running one of those would put a
            // second copy next to the installed app instead of upgrading it.
            if !lower.ends_with(".exe") || !(lower.contains("setup") || lower.contains("installer")) {
                return None;
            }
            Some(Setup {
                name: name.to_string(),
                url: asset.get("browser_download_url")?.as_str()?.to_string(),
                digest: asset.get("digest").and_then(Value::as_str).map(str::to_string),
            })
        })
    });
    Some(Release {
        version,
        page: text("html_url"),
        notes: text("body"),
        setup,
    })
}

/// Fetch the installer into a scratch folder, refusing anything that is not an executable.
async fn download(app: &AppHandle, setup: &Setup) -> AppResult<PathBuf> {
    let dir = std::env::temp_dir().join("Steamware-update");
    tokio::fs::create_dir_all(&dir).await?;
    let path = dir.join(safe_name(&setup.name));

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .user_agent(concat!("Steamware/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let response = client.get(&setup.url).send().await?;
    if !response.status().is_success() {
        return Err(AppError::new(
            "update_download_failed",
            "The installer could not be downloaded from GitHub.",
        ));
    }

    use futures_util::StreamExt;
    let total = response.content_length().unwrap_or(0);
    let mut file = tokio::fs::File::create(&path).await?;
    let mut hasher = Sha256::new();
    let mut received: u64 = 0;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(AppError::from)?;
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
        received += chunk.len() as u64;
        // The gate listens for this; a fraction of a megabyte arriving silently looks like a hang.
        let _ = app.emit(
            "update:progress",
            serde_json::json!({ "received": received, "total": total }),
        );
    }
    file.flush().await?;
    drop(file);

    let digest = format!("sha256:{:x}", hasher.finalize());
    if let Some(expected) = &setup.digest {
        if !digest.eq_ignore_ascii_case(expected.trim()) {
            let _ = tokio::fs::remove_file(&path).await;
            return Err(AppError::new(
                "update_invalid",
                "The downloaded installer did not match the checksum GitHub published. Nothing was run.",
            ));
        }
    }
    // A captive portal or a rate-limited proxy answers with HTML, which would otherwise be saved as `.exe`
    // and handed straight to PowerShell.
    let mut head = [0u8; 2];
    let mut probe = tokio::fs::File::open(&path).await?;
    use tokio::io::AsyncReadExt;
    let _ = probe.read(&mut head).await?;
    if !looks_like_installer(&head) {
        let _ = tokio::fs::remove_file(&path).await;
        return Err(AppError::new(
            "update_invalid",
            "What GitHub sent was not an installer. Nothing was run.",
        ));
    }
    Ok(path)
}

/// Keep only characters a file name may carry, and insist on `.exe`.
fn safe_name(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' '))
        .collect();
    let cleaned = cleaned.trim_matches(['.', ' ']).to_string();
    if cleaned.to_lowercase().ends_with(".exe") && cleaned.len() > 4 {
        cleaned
    } else {
        "Steamware-setup.exe".to_string()
    }
}

fn trim_notes(notes: &str) -> String {
    let trimmed = notes.trim();
    if trimmed.chars().count() <= NOTES_LIMIT {
        return trimmed.to_string();
    }
    let cut: String = trimmed.chars().take(NOTES_LIMIT).collect();
    format!("{cut}…")
}

/// A PowerShell string literal is single-quoted, with a lone quote doubled.
fn ps_quote(value: &str) -> String {
    value.replace('\'', "''")
}

/// Open the installer once this process has ended.
///
/// A detached shell is the only reliable way to do this: the installer must not be extracting while the exe
/// it replaces is still mapped, and the app must not stay running while it waits. The installer is shown
/// rather than run silently: a portable copy has no uninstall entry for NSIS to find, and a silent setup
/// would either install a second copy or fail without saying so.
#[cfg(windows)]
fn launch_installer_after_exit(installer: &Path) -> AppResult<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const DETACHED_PROCESS: u32 = 0x0000_0008;

    let script = format!(
        "$ErrorActionPreference = 'SilentlyContinue'; \
         Wait-Process -Id {pid} -ErrorAction SilentlyContinue; \
         Start-Sleep -Milliseconds 700; \
         Start-Process -FilePath '{installer}'",
        pid = std::process::id(),
        installer = ps_quote(&installer.to_string_lossy()),
    );
    std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-WindowStyle",
            "Hidden",
            "-Command",
            &script,
        ])
        .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
        .spawn()
        .map_err(|_| {
            AppError::new(
                "update_launch_failed",
                "The installer could not be started. Download it from the release page instead.",
            )
        })?;
    Ok(())
}

#[cfg(not(windows))]
fn launch_installer_after_exit(_installer: &Path) -> AppResult<()> {
    Err(AppError::new(
        "update_launch_failed",
        "In-app updates are Windows-only. Download the new version from the release page.",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_the_way_people_expect() {
        assert_eq!(parse_version("v1.0.3"), Some((1, 0, 3)));
        assert_eq!(parse_version("1.0.4"), Some((1, 0, 4)));
        assert_eq!(parse_version("  V2.1 "), Some((2, 1, 0)));
        assert_eq!(parse_version("1.0.4-rc1"), Some((1, 0, 4)));
        assert!(parse_version("v1.0.10").unwrap() > parse_version("v1.0.9").unwrap());
        assert_eq!(parse_version("nightly"), None);
    }

    #[test]
    fn a_release_without_a_version_cannot_gate_anyone() {
        let json: Value = serde_json::from_str(r#"{"tag_name":"latest","html_url":"https://x"}"#).unwrap();
        assert!(parse_release(&json).is_none());
    }

    #[test]
    fn the_setup_asset_is_found_among_the_others() {
        let json: Value = serde_json::from_str(
            r#"{
                "tag_name": "v1.0.4",
                "html_url": "https://github.com/steamware-dev/steamware/releases/tag/v1.0.4",
                "body": "notes",
                "assets": [
                    { "name": "Steamware_1.0.4_x64-setup.exe.sig", "browser_download_url": "https://e/sig" },
                    { "name": "Steamware_1.0.4_x64-setup.exe", "browser_download_url": "https://e/setup", "digest": "sha256:abc" }
                ]
            }"#,
        )
        .unwrap();
        let release = parse_release(&json).unwrap();
        assert_eq!(release.version, "1.0.4");
        let setup = release.setup.unwrap();
        assert_eq!(setup.url, "https://e/setup");
        assert_eq!(setup.digest.as_deref(), Some("sha256:abc"));
    }

    #[test]
    fn the_standalone_build_is_never_mistaken_for_the_installer() {
        // The real releases carry these two next to the setup; running either would install a second copy.
        let json: Value = serde_json::from_str(
            r#"{
                "tag_name": "v1.0.4",
                "assets": [
                    { "name": "Steamware-tiny.exe", "browser_download_url": "https://e/tiny" },
                    { "name": "Steamware-regular.exe", "browser_download_url": "https://e/regular" },
                    { "name": "Steamware_1.0.4_x64-installer.exe", "browser_download_url": "https://e/installer" }
                ]
            }"#,
        )
        .unwrap();
        assert_eq!(parse_release(&json).unwrap().setup.unwrap().url, "https://e/installer");

        let only_standalones: Value = serde_json::from_str(
            r#"{ "tag_name": "v1.0.4", "assets": [ { "name": "Steamware-tiny.exe", "browser_download_url": "https://e/tiny" } ] }"#,
        )
        .unwrap();
        assert!(parse_release(&only_standalones).unwrap().setup.is_none());
    }

    #[test]
    fn a_saved_error_page_is_not_run() {
        assert!(looks_like_installer(b"MZ\x90\x00"));
        assert!(!looks_like_installer(b"<!DOCTYPE html>"));
        assert!(!looks_like_installer(b"M"));
        assert!(!looks_like_installer(b""));
    }

    #[test]
    fn file_names_stay_inside_the_scratch_folder() {
        assert_eq!(safe_name("Steamware_1.0.4_x64-setup.exe"), "Steamware_1.0.4_x64-setup.exe");
        assert_eq!(safe_name("../../evil.exe"), "evil.exe");
        assert_eq!(safe_name("not-an-exe.txt"), "Steamware-setup.exe");
    }
}
