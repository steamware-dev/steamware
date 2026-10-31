use crate::error::{AppError, AppResult};
use base64::Engine as _;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};
use tokio::sync::{Mutex, RwLock};

/// A verified session. The token lives in memory only: it is never written to disk, never put in a URL,
/// and never logged. Losing it costs one Turnstile check, which is the point.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub expires_at: i64,
    pub device_hash: String,
}

struct Session {
    token: String,
    expires_at: i64,
    device_hash: String,
}

/// How long a server error parks the optional traffic. Long enough that a fleet of clients retrying cannot
/// pile onto a Worker that is already struggling, short enough that a blip does not last the session.
const SERVER_ERROR_COOLDOWN: i64 = 300;

/// Said when the Worker is over its free daily request budget.
const CAP_MESSAGE: &str = "The Steamware server has used its free requests for today. Browsing, your library and downloads already running still work — new manifests and fixes resume after midnight UTC.";

/// Said when the Worker answers with errors rather than JSON.
const BUSY_MESSAGE: &str = "The Steamware server is not answering right now. Showing what is already on this PC.";

/// A period during which the Worker has stopped answering usefully.
///
/// The free plan allows 100,000 requests a day, and once that is gone the platform answers at the edge — so
/// the Worker itself never runs and cannot explain itself. What matters then is what the clients do: a
/// hundred installs retrying a dead endpoint is how a short outage becomes a long one. Recognising that
/// state and going quiet is the cheapest thing the app can possibly do.
#[derive(Clone)]
struct Degraded {
    /// Unix seconds. Past this the state is stale and the client may probe again.
    until: i64,
    message: String,
}

/// Whether a failure means "the server is out of action" rather than "this one lookup failed".
///
/// The distinction is the difference between an app that goes quiet for five minutes and one that reports a
/// single bad answer. Our Worker explains its own 5xx: `upstream_busy` is DepotBox rate-limiting one game,
/// `not_configured` is a source with no key on the server, `upstream_failed` is Steam's API hiccuping. None
/// of those say anything about the next request, so tripping the hold on them parked the whole app — store,
/// search, stats — because one game's metadata could not be fetched.
///
/// A 5xx with no body is the platform itself (1102, 1015, 52x): the Worker never ran and cannot explain, and
/// every request is in the same position, so that — and a bare `internal` from our own code — trips the hold.
fn trips_hold(status: u16, code: &str) -> bool {
    status >= 500 && (code.is_empty() || code == "internal")
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

/// The moment the platform's request budget refills, which is midnight UTC.
fn next_utc_midnight() -> i64 {
    let now = now_secs();
    now - now % 86_400 + 86_400
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn random_bytes<const N: usize>() -> [u8; N] {
    use rand::RngCore;
    let mut bytes = [0u8; N];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes
}

/// The app's only outbound client.
///
/// Every request goes to `base` — the Worker origin from Settings — and nothing else. There is no branch
/// that can reach a provider directly, because this type has no notion of a provider: it has one host, a
/// bearer token and paths. That is what makes "all requests go through my site" a property of the code
/// rather than a promise.
pub struct ApiClient {
    base: RwLock<String>,
    /// JSON calls: short, bounded.
    http: reqwest::Client,
    /// File streams: connect timeout only. A manifest build legitimately takes minutes, and a total
    /// timeout here would abort a download that has already been authorised.
    stream_http: reqwest::Client,
    session: Mutex<Option<Session>>,
    /// Set while the Worker is known to be over budget or down; see `Degraded`.
    degraded: Mutex<Option<Degraded>>,
    /// Serialises verification so two page loads cannot open two tabs.
    auth_gate: Mutex<()>,
    device: String,
}

impl ApiClient {
    pub fn new(base: &str, device: &str) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!("Steamware/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(25))
            // The Worker never redirects. Refusing to follow one means a compromised or misconfigured
            // endpoint cannot bounce this client to a different host with its bearer token attached.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("http client");

        let stream_http = reqwest::Client::builder()
            .user_agent(concat!("Steamware/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("stream client");

        Self {
            base: RwLock::new(base.trim_end_matches('/').to_string()),
            http,
            stream_http,
            session: Mutex::new(None),
            degraded: Mutex::new(None),
            auth_gate: Mutex::new(()),
            device: device.to_string(),
        }
    }

    pub async fn set_base(&self, base: &str) {
        *self.base.write().await = base.trim_end_matches('/').to_string();
    }

    pub async fn base(&self) -> String {
        self.base.read().await.clone()
    }

    pub async fn session_info(&self) -> Option<SessionInfo> {
        let session = self.session.lock().await;
        session.as_ref().map(|s| SessionInfo {
            expires_at: s.expires_at,
            device_hash: s.device_hash.clone(),
        })
    }

    pub async fn clear_session(&self) {
        *self.session.lock().await = None;
    }

    /// The reason the server is unusable right now, if it still is. Clears itself once the window passes, so
    /// the app comes back on its own rather than needing a restart.
    pub async fn degraded(&self) -> Option<String> {
        let mut state = self.degraded.lock().await;
        let live = state.as_ref().is_some_and(|current| current.until > now_secs());
        if !live {
            *state = None;
            return None;
        }
        state.as_ref().map(|current| current.message.clone())
    }

    /// The same state, plus when it lifts, for the UI's banner.
    pub async fn health(&self) -> (bool, Option<String>, i64) {
        let message = self.degraded().await;
        let until = self.degraded.lock().await.as_ref().map(|current| current.until).unwrap_or(0);
        (message.is_some(), message, until)
    }

    /// Trip the state, keeping the later deadline: a five-minute server error must not cut short a known
    /// cap that lasts until midnight.
    async fn trip(&self, until: i64, message: &str) {
        let mut state = self.degraded.lock().await;
        let extend = state.as_ref().is_none_or(|current| current.until < until);
        if extend {
            *state = Some(Degraded { until, message: message.to_string() });
        }
    }

    /// Refuses work that the server cannot do right now, without spending a request to find out.
    async fn offline(&self) -> Option<AppError> {
        self.degraded().await.map(|message| AppError::new("degraded", message))
    }

    /// A token that is valid for at least another 30 seconds, verifying the caller first if needed.
    pub async fn ensure_session(&self, app: &AppHandle) -> AppResult<String> {
        // A capped server cannot mint a session, and opening a browser to watch the check fail is worse than
        // saying so. The caller is told the same thing every other gated path says while the cap holds.
        if let Some(error) = self.offline().await {
            return Err(error);
        }
        {
            let session = self.session.lock().await;
            if let Some(current) = session.as_ref() {
                if current.expires_at > now_secs() + 30 {
                    return Ok(current.token.clone());
                }
            }
        }
        self.authenticate(app).await
    }

    /// The device-code handshake.
    ///
    /// The browser page passes Turnstile on the Worker's own origin, then binds the result to this device's
    /// challenge. The verifier never leaves this process, so knowing the device id is not enough to claim
    /// the session, and the token is delivered over HTTPS in a POST body rather than a URL.
    pub async fn authenticate(&self, app: &AppHandle) -> AppResult<String> {
        let _gate = self.auth_gate.lock().await;

        // Another caller may have completed the flow while we waited for the gate.
        {
            let session = self.session.lock().await;
            if let Some(current) = session.as_ref() {
                if current.expires_at > now_secs() + 30 {
                    return Ok(current.token.clone());
                }
            }
        }

        let base = self.base().await;
        if base.is_empty() {
            return Err(AppError::config("Set your server address in Settings first."));
        }

        let verifier = b64url(&random_bytes::<32>());
        let challenge = hex(&Sha256::digest(verifier.as_bytes()));
        let link = format!("{base}/link?device={}&challenge={}", self.device, challenge);

        crate::local_api::log_line("verification started: opening the browser check");
        let _ = app.emit(
            "auth:status",
            json!({ "state": "pending", "url": link, "message": "Finish the check in your browser." }),
        );
        if let Err(error) = tauri_plugin_opener::open_url(&link, None::<&str>) {
            let _ = app.emit(
                "auth:status",
                json!({ "state": "failed", "message": format!("Could not open a browser: {error}") }),
            );
            return Err(AppError::verification("Could not open a browser to verify you."));
        }

        let mut network_failures = 0u32;
        for _ in 0..150 {
            tokio::time::sleep(Duration::from_millis(1_200)).await;

            let response = self
                .http
                .post(format!("{base}/v1/session/claim"))
                .json(&json!({ "device": self.device, "verifier": verifier }))
                .send()
                .await;

            let response = match response {
                Ok(response) => response,
                Err(_) => {
                    network_failures += 1;
                    if network_failures >= 5 {
                        let _ = app.emit(
                            "auth:status",
                            json!({ "state": "failed", "message": "Could not reach the server." }),
                        );
                        return Err(AppError::new(
                            "network_unreachable",
                            "Could not reach the server while verifying.",
                        ));
                    }
                    continue;
                }
            };

            match response.status().as_u16() {
                200 => {
                    let body: Value = response.json().await?;
                    let token = body["token"].as_str().unwrap_or_default().to_string();
                    let expires_at = body["expiresAt"].as_i64().unwrap_or(0);
                    let device_hash = body["deviceHash"].as_str().unwrap_or_default().to_string();
                    if token.is_empty() {
                        return Err(AppError::verification("The server returned no session."));
                    }
                    *self.session.lock().await = Some(Session {
                        token: token.clone(),
                        expires_at,
                        device_hash,
                    });
                    let _ = app.emit("auth:status", json!({ "state": "ok", "expiresAt": expires_at }));
                    return Ok(token);
                }
                404 => continue,
                429 => {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }
                403 | 410 => {
                    let _ = app.emit(
                        "auth:status",
                        json!({ "state": "failed", "message": "That check did not pass. Try again." }),
                    );
                    return Err(AppError::verification("Verification did not pass."));
                }
                _ => {
                    network_failures += 1;
                    if network_failures >= 3 {
                        let _ = app.emit(
                            "auth:status",
                            json!({ "state": "failed", "message": "Verification failed." }),
                        );
                        return Err(AppError::verification("Verification failed."));
                    }
                }
            }
        }

        let _ = app.emit(
            "auth:status",
            json!({ "state": "failed", "message": "The check timed out." }),
        );
        Err(AppError::verification("The verification timed out. Try again."))
    }

    /// Turn a non-success response into the server's own error, never a raw body.
    ///
    /// This is also where "the server is out of budget" is noticed. Our own rate limiter answers 429 with a
    /// JSON body describing a one-minute window; the platform's daily cap answers at the edge, before this
    /// Worker runs, with its own page and no JSON at all. Only the second means every request is refused
    /// until tomorrow, so the two are told apart here rather than by guesswork in each caller.
    async fn failure(&self, response: reqwest::Response) -> AppError {
        let status = response.status().as_u16();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        let code = body["error"]["code"].as_str().unwrap_or("").to_string();
        let message = body["error"]["message"].as_str().unwrap_or("").to_string();

        if status == 429 && code.is_empty() {
            self.trip(next_utc_midnight(), CAP_MESSAGE).await;
            return AppError::new("daily_cap_reached", CAP_MESSAGE);
        }
        if trips_hold(status, &code) {
            self.trip(now_secs() + SERVER_ERROR_COOLDOWN, BUSY_MESSAGE).await;
            return AppError::new("server_busy", BUSY_MESSAGE);
        }

        let code = if code.is_empty() { "request_failed".to_string() } else { code };
        let message = if message.is_empty() {
            "The server refused the request.".to_string()
        } else {
            message
        };

        match status {
            401 => AppError::new(if code == "session_expired" { "session_expired" } else { code.as_str() }, message),
            429 => AppError::new("rate_limited", message),
            _ => AppError::new(code, message),
        }
    }

    async fn authorized(&self, app: &AppHandle) -> AppResult<(String, String)> {
        let base = self.base().await;
        if base.is_empty() {
            return Err(AppError::config("Set your server address in Settings first."));
        }
        let token = self.ensure_session(app).await?;
        Ok((base, token))
    }

    /// A gated GET that never starts verification.
    ///
    /// `authorized` answers "no session yet" by opening the browser on the Cloudflare check, which is right
    /// for something the user clicked and wrong for something a page mounted. The plugin's release feed is
    /// read the moment Home or the first-run sheet renders, and through the interactive path that made a
    /// brand-new install launch the default browser before the wizard had been answered. Status checks use
    /// this instead: the session if one is already valid, one refresh if it lapsed mid-call, and otherwise
    /// `session_required` for the caller to describe — nothing opens.
    pub async fn get_json_quiet(&self, path: &str) -> AppResult<Value> {
        // Status checks are the definition of optional: while the cap holds they answer "nothing to show"
        // rather than costing a doomed request.
        if let Some(error) = self.offline().await {
            return Err(error);
        }
        let base = self.base().await;
        if base.is_empty() {
            return Err(AppError::config("Set your server address first."));
        }
        let mut refreshed = false;
        loop {
            let Some(token) = self.valid_token().await else {
                return Err(AppError::new("session_required", "Verify to see this."));
            };
            let response = self
                .http
                .get(format!("{base}{path}"))
                .bearer_auth(&token)
                .send()
                .await?;

            if response.status() == 401 && !refreshed {
                self.clear_session().await;
                refreshed = true;
                continue;
            }
            if !response.status().is_success() {
                return Err(self.failure(response).await);
            }
            return Ok(response.json::<Value>().await?);
        }
    }

    /// The token already held, if it is still good. Asking this never verifies anybody.
    pub async fn valid_token(&self) -> Option<String> {
        let session = self.session.lock().await;
        session
            .as_ref()
            .filter(|current| current.expires_at > now_secs() + 30)
            .map(|current| current.token.clone())
    }

    pub async fn get_json(&self, app: &AppHandle, path: &str) -> AppResult<Value> {
        if let Some(error) = self.offline().await {
            return Err(error);
        }
        let mut refreshed = false;
        loop {
            let (base, token) = self.authorized(app).await?;
            let response = self
                .http
                .get(format!("{base}{path}"))
                .bearer_auth(&token)
                .send()
                .await?;

            if response.status() == 401 && !refreshed {
                // The session lapsed while the app sat idle. Drop it and verify once.
                self.clear_session().await;
                refreshed = true;
                continue;
            }
            if !response.status().is_success() {
                return Err(self.failure(response).await);
            }
            return Ok(response.json::<Value>().await?);
        }
    }

    /// A public endpoint, without a session.
    ///
    /// `/v1/config` is the only one: it lists which sources the server can serve, which the UI needs
    /// BEFORE it can ask a user to verify, to say "this server has no provider key yet" instead of failing
    /// a gated call.
    pub async fn get_public_json(&self, path: &str) -> AppResult<Value> {
        let base = self.base().await;
        if base.is_empty() {
            return Err(AppError::config("Set your server address in Settings first."));
        }
        let response = self.http.get(format!("{base}{path}")).send().await?;
        if !response.status().is_success() {
            return Err(self.failure(response).await);
        }
        Ok(response.json::<Value>().await?)
    }

    /// A streamed response, with the same one-shot re-verification as `get_json`.
    pub async fn stream(&self, app: &AppHandle, path: &str) -> AppResult<reqwest::Response> {
        if let Some(error) = self.offline().await {
            return Err(error);
        }
        let (base, token) = self.authorized(app).await?;
        let response = self
            .stream_http
            .get(format!("{base}{path}"))
            .bearer_auth(&token)
            .send()
            .await?;

        if response.status() == 401 {
            self.clear_session().await;
            let (base, token) = self.authorized(app).await?;
            let retry = self
                .stream_http
                .get(format!("{base}{path}"))
                .bearer_auth(&token)
                .send()
                .await?;
            if !retry.status().is_success() {
                return Err(self.failure(retry).await);
            }
            return Ok(retry);
        }
        if !response.status().is_success() {
            return Err(self.failure(response).await);
        }
        Ok(response)
    }

    /// A streamed response from a URL the caller built, with no session attached.
    ///
    /// Exactly one caller has this: a Hubcap download made with the user's own key, which by design never
    /// passes through the Worker. That URL carries the key in its query string, so nothing here logs it and
    /// no error repeats it.
    pub async fn stream_absolute(&self, url: &str) -> AppResult<reqwest::Response> {
        let response = self.stream_http.get(url).send().await?;
        if response.status().is_success() {
            return Ok(response);
        }
        Err(match response.status().as_u16() {
            401 | 403 => AppError::new("hubcap_key_rejected", "Hubcap rejected that API key."),
            404 => AppError::new("hubcap_no_manifest", "Hubcap has no manifest for this game."),
            429 => AppError::new("hubcap_limit", "That Hubcap key has reached its limit for today."),
            _ => AppError::new("hubcap_failed", "Hubcap could not be reached. Try again."),
        })
    }

    /// Cover art, fetched without ever starting verification.
    ///
    /// The webview cannot put a bearer token on an `<img>`, so the bytes come through the Worker and Rust
    /// hands the UI a data URL — which makes every cover a gated request. Through the interactive path that
    /// was enough for a page render to ask a brand-new install to verify: drawing the home shelf opened the
    /// browser by itself, before the user had clicked anything. Art is not worth a browser window, so with
    /// no session this reports `session_required` and the media cache hands back the CDN URL to load
    /// directly until the user verifies of their own accord.
    pub async fn media(&self, url: &str) -> AppResult<Vec<u8>> {
        let encoded: String = url::form_urlencoded::byte_serialize(url.as_bytes()).collect();
        let base = self.base().await;
        if base.is_empty() {
            return Err(AppError::config("Set your server address first."));
        }
        let unverified = || AppError::new("session_required", "Verify to load covers through the proxy.");
        // Same answer as "unverified": the media cache hands the CDN URL straight to the webview, which needs
        // no server at all. Covers are the one surface that costs the Worker nothing during a cap.
        if let Some(error) = self.offline().await {
            return Err(error);
        }
        let Some(token) = self.valid_token().await else {
            return Err(unverified());
        };
        let response = self
            .http
            .get(format!("{base}/v1/media?u={encoded}"))
            .bearer_auth(&token)
            .send()
            .await?;
        if response.status() == 401 {
            // Lapsed rather than refused: drop it so the next real action verifies once, and serve this
            // image from the CDN in the meantime.
            self.clear_session().await;
            return Err(unverified());
        }
        if !response.status().is_success() {
            return Err(self.failure(response).await);
        }
        Ok(response.bytes().await?.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::trips_hold;

    #[test]
    fn one_upstream_failure_does_not_park_the_whole_app() {
        // What our Worker says when a source is busy, unkeyed or unreachable: the next request is fine.
        assert!(!trips_hold(503, "upstream_busy"));
        assert!(!trips_hold(502, "upstream_failed"));
        assert!(!trips_hold(502, "upstream_rejected"));
        assert!(!trips_hold(503, "not_configured"));
        assert!(!trips_hold(200, ""));
        assert!(!trips_hold(401, "session_required"));
    }

    #[test]
    fn the_platform_itself_failing_does_park_it() {
        // No JSON at all: Cloudflare's own error page, so every request is in the same position.
        assert!(trips_hold(502, ""));
        assert!(trips_hold(503, ""));
        assert!(trips_hold(500, "internal"));
    }
}
