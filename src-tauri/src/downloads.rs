use crate::api::ApiClient;
use crate::error::{AppError, AppResult};
use futures_util::StreamExt;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadItem {
    pub id: String,
    pub app_id: String,
    pub title: String,
    /// "manifest" or "fix"
    pub kind: String,
    /// queued | downloading | done | failed | cancelled
    pub status: String,
    pub received: u64,
    pub total: u64,
    pub file_name: String,
    pub path: String,
    pub error: Option<String>,
    pub started_at: i64,
    /// Lua files this archive put into Steam. Non-zero only once the install ran.
    pub installed_lua: u32,
    /// Manifests this archive put into `depotcache`.
    pub installed_manifests: u32,
    /// Why the install did not happen, when the archive landed but Steam refused it (Steam closed, no
    /// `depotcache`, a locked file). The download itself still succeeded.
    pub install_error: Option<String>,
    /// The plugin folder is registered with the unlocker, so a running Steam picks this up without a
    /// restart. False means the files are in place and Steam reads them on its next start.
    pub install_live: bool,
    /// The game was also written into Steam's library, so it is now a tile there rather than a hidden
    /// licence. This is what makes an add show up without the restart the note above can still need.
    pub install_library: bool,
}

pub struct NewDownload {
    /// Path on the API host, e.g. `/v1/manifest/730`.
    pub path: String,
    /// An absolute URL to stream from instead, bypassing the API host.
    ///
    /// Only a Hubcap download sets this: the request carries the user's own key and is deliberately not
    /// proxied, so nothing about it reaches the Steamware server.
    pub url: Option<String>,
    pub app_id: String,
    pub title: String,
    pub kind: String,
    pub fallback_name: String,
    pub directory: PathBuf,
    /// Steam's folder, so a finished manifest archive can be installed without a second trip through the UI.
    pub steam_path: String,
    /// Install on completion. True for manifests; a fix package is installed by whoever asked for it.
    pub install: bool,
    /// Install the lua with its manifest pins commented out, so Steam keeps the game updated. Comes from
    /// the "Auto Update Apps (Don't Lock Manifests)" setting, and is false for fixes, which must keep the
    /// version they were built for.
    pub unpin: bool,
}

/// The download queue.
///
/// Files are streamed straight to disk by the Rust side, not fetched into memory by the webview: a manifest
/// archive can be a gigabyte, and a `fetch()` in the frontend would hold all of it. Each job writes a
/// `.part` file and only renames it on completion, so a cancelled or failed download can never leave
/// something that looks like a finished archive.
pub struct Downloads {
    items: Arc<Mutex<Vec<DownloadItem>>>,
    cancels: Arc<Mutex<HashMap<String, CancellationToken>>>,
    slots: Arc<Semaphore>,
    /// Shares of the four-slot semaphore each job holds. Mutable so a Settings change applies to the next
    /// download without restarting the app.
    permits_per_job: Arc<Mutex<usize>>,
}

impl Downloads {
    pub fn new(max_concurrent: u8) -> Self {
        Self {
            items: Arc::new(Mutex::new(Vec::new())),
            cancels: Arc::new(Mutex::new(HashMap::new())),
            slots: Arc::new(Semaphore::new(4)),
            permits_per_job: Arc::new(Mutex::new(Self::permits_for(max_concurrent))),
        }
    }

    /// A 4-slot semaphore divided into equal shares preserves the requested concurrency for 1–4.
    fn permits_for(max_concurrent: u8) -> usize {
        (4usize / max_concurrent.clamp(1, 4) as usize).max(1)
    }

    pub fn set_concurrency(&self, max_concurrent: u8) {
        if let Ok(mut permits) = self.permits_per_job.lock() {
            *permits = Self::permits_for(max_concurrent);
        }
    }

    pub fn list(&self) -> Vec<DownloadItem> {
        self.items
            .lock()
            .map(|items| {
                let mut copy = items.clone();
                copy.sort_by(|a, b| b.started_at.cmp(&a.started_at));
                copy
            })
            .unwrap_or_default()
    }

    pub fn cancel(&self, id: &str) {
        if let Ok(cancels) = self.cancels.lock() {
            if let Some(token) = cancels.get(id) {
                token.cancel();
            }
        }
    }

    pub fn start(
        &self,
        app: AppHandle,
        api: Arc<ApiClient>,
        job: NewDownload,
    ) -> AppResult<DownloadItem> {
        let id = format!("{:016x}", rand::random::<u64>());
        let item = DownloadItem {
            id: id.clone(),
            app_id: job.app_id.clone(),
            title: job.title.clone(),
            kind: job.kind.clone(),
            status: "queued".to_string(),
            received: 0,
            total: 0,
            file_name: job.fallback_name.clone(),
            path: String::new(),
            error: None,
            started_at: now_secs(),
            installed_lua: 0,
            installed_manifests: 0,
            install_error: None,
            install_live: false,
            install_library: false,
        };

        self.items.lock().map_err(|_| AppError::new("state", "Download list unavailable."))?.push(item.clone());

        let token = CancellationToken::new();
        self.cancels
            .lock()
            .map_err(|_| AppError::new("state", "Download list unavailable."))?
            .insert(id.clone(), token.clone());

        let items = self.items.clone();
        let cancels = self.cancels.clone();
        let slots = self.slots.clone();
        let permits = self.permits_per_job.lock().map(|value| *value).unwrap_or(2);
        let task_id = id.clone();

        tauri::async_runtime::spawn(async move {
            let permit = slots.acquire_many_owned(permits as u32).await;
            if token.is_cancelled() {
                if let Some(update) = finish(&items, &task_id, "cancelled", None, None) {
                    crate::history::record(&update);
                }
                if let Ok(mut map) = cancels.lock() {
                    map.remove(&task_id);
                }
                return;
            }
            let _permit = permit;

            if let Some(update) = set_status(&items, &task_id, "downloading") {
                let _ = app.emit("download:progress", &update);
            }

            let result = run(&app, api, &job, &task_id, &token, &items).await;
            match result {
                Ok(dest) => {
                    // The archive is on disk. Put it into Steam now, in the same step: a fetched game that
                    // stops at "zip saved" is the one thing the old build did not do.
                    let mut install_note = None;
                    let mut installed = (0u32, 0u32);
                    let mut install_live = false;
                    let mut install_library = false;
                    if job.install {
                        match crate::lua::install_archive(
                            &dest.to_string_lossy(),
                            &job.steam_path,
                            job.unpin,
                        ) {
                            Ok(summary) => {
                                installed = (summary.lua as u32, summary.manifests as u32);
                                install_live = summary.live;
                                install_library = summary.library;
                            }
                            Err(error) => install_note = Some(error.message),
                        }
                    }
                    if let Some(update) = finish_installed(
                        &items,
                        &task_id,
                        "done",
                        None,
                        Some(dest),
                        installed,
                        install_note,
                        install_live,
                        install_library,
                    ) {
                        crate::history::record(&update);
                        let _ = app.emit("download:progress", &update);
                        let _ = app.emit("download:done", &update);
                    }
                }
                Err(error) => {
                    let cancelled = token.is_cancelled() || error.code == "cancelled";
                    if let Some(update) = finish(
                        &items,
                        &task_id,
                        if cancelled { "cancelled" } else { "failed" },
                        if cancelled { None } else { Some(error.message) },
                        None,
                    ) {
                        crate::history::record(&update);
                        let _ = app.emit("download:progress", &update);
                        let _ = app.emit("download:failed", &update);
                    }
                }
            }

            if let Ok(mut map) = cancels.lock() {
                map.remove(&task_id);
            }
        });

        Ok(item)
    }
}

fn set_status(items: &Arc<Mutex<Vec<DownloadItem>>>, id: &str, status: &str) -> Option<DownloadItem> {
    let mut guard = items.lock().ok()?;
    let item = guard.iter_mut().find(|item| item.id == id)?;
    item.status = status.to_string();
    Some(item.clone())
}

fn finish(
    items: &Arc<Mutex<Vec<DownloadItem>>>,
    id: &str,
    status: &str,
    error: Option<String>,
    path: Option<PathBuf>,
) -> Option<DownloadItem> {
    finish_installed(items, id, status, error, path, (0, 0), None, false, false)
}

#[allow(clippy::too_many_arguments)]
fn finish_installed(
    items: &Arc<Mutex<Vec<DownloadItem>>>,
    id: &str,
    status: &str,
    error: Option<String>,
    path: Option<PathBuf>,
    installed: (u32, u32),
    install_error: Option<String>,
    install_live: bool,
    install_library: bool,
) -> Option<DownloadItem> {
    let mut guard = items.lock().ok()?;
    let item = guard.iter_mut().find(|item| item.id == id)?;
    item.status = status.to_string();
    item.error = error;
    item.installed_lua = installed.0;
    item.installed_manifests = installed.1;
    item.install_error = install_error;
    item.install_live = install_live;
    item.install_library = install_library;
    if let Some(path) = path {
        item.path = path.to_string_lossy().to_string();
        if item.total == 0 {
            item.total = item.received;
        }
    }
    Some(item.clone())
}

fn emit_progress(app: &AppHandle, items: &Arc<Mutex<Vec<DownloadItem>>>, id: &str) {
    if let Ok(guard) = items.lock() {
        if let Some(item) = guard.iter().find(|item| item.id == id) {
            let _ = app.emit("download:progress", item.clone());
        }
    }
}

fn sanitize_file_name(raw: &str) -> Option<String> {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or(raw).trim();
    let cleaned: String = base
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' '))
        .take(120)
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').to_string();
    if cleaned.is_empty() || cleaned == ".." {
        None
    } else {
        Some(cleaned)
    }
}

fn name_from_disposition(header: Option<&str>, fallback: &str) -> String {
    if let Some(value) = header {
        if let Some(index) = value.to_ascii_lowercase().find("filename=") {
            let raw = value[index + 9..].trim().trim_matches('"').trim_matches(';');
            if let Some(name) = sanitize_file_name(raw) {
                return name;
            }
        }
    }
    sanitize_file_name(fallback).unwrap_or_else(|| "download.zip".to_string())
}

/// `name.zip`, then `name (1).zip`, … so two downloads of the same app never overwrite each other.
fn unique_path(directory: &Path, file_name: &str) -> PathBuf {
    let candidate = directory.join(file_name);
    if !candidate.exists() {
        return candidate;
    }
    let stem = Path::new(file_name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| file_name.to_string());
    let extension = Path::new(file_name)
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    for index in 1..1000 {
        let next = directory.join(format!("{stem} ({index}){extension}"));
        if !next.exists() {
            return next;
        }
    }
    directory.join(format!("{stem}-{}{extension}", now_secs()))
}

/// A ceiling on one file. Manifest archives are kilobytes and a fix package is tens of megabytes, so this is
/// generous — it only exists because `Content-Length` is a claim, not a fact: a broken or hostile response
/// must not be able to fill the disk. Enforced against the header AND while streaming.
const MAX_DOWNLOAD_BYTES: u64 = 1 << 30;

fn too_large() -> AppError {
    AppError::new("too_large", "That download is larger than Steamware will save.")
}

async fn run(
    app: &AppHandle,
    api: Arc<ApiClient>,
    job: &NewDownload,
    id: &str,
    token: &CancellationToken,
    items: &Arc<Mutex<Vec<DownloadItem>>>,
) -> AppResult<PathBuf> {
    tokio::fs::create_dir_all(&job.directory).await?;

    let response = match &job.url {
        Some(url) => api.stream_absolute(url).await?,
        None => api.stream(app, &job.path).await?,
    };
    let total = response.content_length().unwrap_or(0);
    if total > MAX_DOWNLOAD_BYTES {
        return Err(too_large());
    }
    let file_name = name_from_disposition(
        response
            .headers()
            .get("content-disposition")
            .and_then(|value| value.to_str().ok()),
        &job.fallback_name,
    );
    let dest = unique_path(&job.directory, &file_name);
    let part = dest.with_file_name(format!(
        "{}.part",
        dest.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "download".into())
    ));

    if let Ok(mut guard) = items.lock() {
        if let Some(item) = guard.iter_mut().find(|item| item.id == id) {
            item.total = total;
            item.file_name = file_name;
        }
    }

    let mut file = tokio::fs::File::create(&part).await?;
    let mut stream = response.bytes_stream();
    let mut received: u64 = 0;
    let mut last_emit = Instant::now();

    while let Some(chunk) = stream.next().await {
        if token.is_cancelled() {
            drop(file);
            let _ = tokio::fs::remove_file(&part).await;
            drop(stream);
            return Err(AppError::new("cancelled", "Cancelled."));
        }
        let chunk = chunk?;
        if let Err(error) = file.write_all(&chunk).await {
            drop(file);
            let _ = tokio::fs::remove_file(&part).await;
            return Err(error.into());
        }
        received += chunk.len() as u64;
        if received > MAX_DOWNLOAD_BYTES {
            drop(file);
            let _ = tokio::fs::remove_file(&part).await;
            return Err(too_large());
        }

        if let Ok(mut guard) = items.lock() {
            if let Some(item) = guard.iter_mut().find(|item| item.id == id) {
                item.received = received;
            }
        }

        if last_emit.elapsed() >= Duration::from_millis(180) {
            emit_progress(app, items, id);
            last_emit = Instant::now();
        }
    }

    file.flush().await?;
    drop(file);
    tokio::fs::rename(&part, &dest).await?;
    Ok(dest)
}
