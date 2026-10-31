use crate::api::ApiClient;
use crate::error::AppResult;
use crate::settings;
use base64::Engine as _;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Covers, served from the Worker and cached on disk.
///
/// The webview cannot attach a bearer token to an `<img src>`, and putting a session token in a URL would
/// undo the reason the token is a header in the first place. So the Rust side fetches the bytes through the
/// proxied endpoint and hands the UI a `data:` URL. Disk caching means a page of 40 capsules is one round
/// trip the first time and zero afterwards.
pub struct MediaCache {
    memory: Mutex<HashMap<String, Arc<String>>>,
    dir: PathBuf,
    /// Used for the direct Valve fetch. A `data:` URL never leaves this process, so nothing here carries
    /// a token: these are the same public image URLs the proxy accepts, fetched without going through it.
    http: reqwest::Client,
}

const MEMORY_ENTRIES: usize = 320;

/// The hosts a cover may be fetched from directly — the same set the Worker proxy allows.
///
/// Load-bearing: a URL in store metadata is untrusted input, and without this list a crafted response
/// could point the app's own HTTP client at an arbitrary host from the user's machine.
const DIRECT_HOSTS: [&str; 15] = [
    "cdn.akamai.steamstatic.com",
    "cdn.cloudflare.steamstatic.com",
    "cdn.fastly.steamstatic.com",
    "cdn.steamstatic.com",
    "shared.steamstatic.com",
    "store.steamstatic.com",
    "shared.akamai.steamstatic.com",
    "shared.cloudflare.steamstatic.com",
    "shared.fastly.steamstatic.com",
    "store.akamai.steamstatic.com",
    "store.cloudflare.steamstatic.com",
    "steamcdn-a.akamaihd.net",
    "media.steampowered.com",
    "clan.cloudflare.steamstatic.com",
    "avatars.akamai.steamstatic.com",
];

fn direct_host(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    if parsed.scheme() != "https" {
        return false;
    }
    parsed
        .host_str()
        .map(|host| DIRECT_HOSTS.contains(&host.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

impl MediaCache {
    pub fn new() -> Self {
        let dir = settings::app_dir().join("media");
        let _ = fs::create_dir_all(&dir);
        Self {
            memory: Mutex::new(HashMap::new()),
            dir,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(12))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }

    /// Fetch one cover straight from Valve, bypassing the Worker.
    ///
    /// Every capsule used to be one proxied request, which made a first browse the largest consumer of the
    /// Worker's request quota: on the free plan a launch day of new installs could exhaust 100,000 requests
    /// before the evening. Valve's CDN serves these images without a token, so they are fetched from there
    /// and cached exactly as before; `None` means "try the proxy instead", never "no cover".
    async fn direct(&self, url: &str) -> Option<Vec<u8>> {
        if !direct_host(url) {
            return None;
        }
        let response = self
            .http
            .get(url)
            .header("accept", "image/avif,image/webp,image/*,*/*;q=0.8")
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        let bytes = response.bytes().await.ok()?.to_vec();
        if bytes.is_empty() {
            None
        } else {
            Some(bytes)
        }
    }

    fn sniff_mime(bytes: &[u8]) -> &'static str {
        if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            "image/jpeg"
        } else if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
            "image/png"
        } else if bytes.starts_with(b"GIF8") {
            "image/gif"
        } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
            "image/webp"
        } else if bytes.len() > 12 && &bytes[4..12] == b"ftypavif" {
            "image/avif"
        } else {
            "application/octet-stream"
        }
    }

    fn encode(bytes: &[u8]) -> String {
        format!(
            "data:{};base64,{}",
            Self::sniff_mime(bytes),
            base64::engine::general_purpose::STANDARD.encode(bytes)
        )
    }

    fn path_for(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.img"))
    }

    pub async fn resolve(&self, api: &ApiClient, url: &str) -> AppResult<String> {
        if url.trim().is_empty() {
            return Ok(String::new());
        }

        let key = url
            .as_bytes()
            .iter()
            .fold(Sha256::new(), |mut hasher, byte| {
                hasher.update([*byte]);
                hasher
            })
            .finalize();
        let key: String = key.iter().map(|b| format!("{b:02x}")).collect();

        if let Some(hit) = self.memory.lock().ok().and_then(|cache| cache.get(&key).cloned()) {
            return Ok((*hit).clone());
        }

        let file = self.path_for(&key);
        if let Ok(bytes) = fs::read(&file) {
            if !bytes.is_empty() {
                let encoded = Arc::new(Self::encode(&bytes));
                self.remember(key, encoded.clone());
                return Ok((*encoded).clone());
            }
        }

        let bytes = match self.direct(url).await {
            Some(bytes) => bytes,
            // The proxy is the fallback, not the main path: a blocked CDN still draws the grid, and an
            // unverified install is still handed the URL itself rather than being asked to verify for art.
            None => match api.media(url).await {
                Ok(bytes) => bytes,
                // Unverified, or the server is out of budget: the proxy is closed either way, so hand back
                // the CDN URL itself. An `<img src>` can load it without a token, which is what keeps a first
                // run from verifying to draw a cover — and unlike a data URL it costs this process nothing.
                Err(error) if error.cheap_fallback() => return Ok(url.to_string()),
                Err(error) => return Err(error),
            },
        };
        if bytes.is_empty() {
            return Ok(String::new());
        }
        let _ = fs::write(&file, &bytes);
        let encoded = Arc::new(Self::encode(&bytes));
        self.remember(key, encoded.clone());
        Ok((*encoded).clone())
    }

    fn remember(&self, key: String, value: Arc<String>) {
        if let Ok(mut cache) = self.memory.lock() {
            if cache.len() >= MEMORY_ENTRIES {
                cache.clear();
            }
            cache.insert(key, value);
        }
    }
}
