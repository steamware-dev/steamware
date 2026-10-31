use serde::Serialize;

/// The single error shape a command can return.
///
/// It is serialised straight to the frontend as `{ code, message }`, and the code is what the UI branches
/// on ("session_expired" re-opens verification, "invalid_api_base" points at Settings). Messages are for a
/// person; no upstream body and no URL with credentials is ever put in one.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: String,
    pub message: String,
}

impl AppError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into() }
    }

    pub fn config(message: impl Into<String>) -> Self {
        Self::new("not_configured", message)
    }

    pub fn verification(message: impl Into<String>) -> Self {
        Self::new("verification_failed", message)
    }

    /// True for the refusals that mean "the Worker will not answer this right now, buy it somewhere free
    /// instead". Covers are the only caller: a capsule can be loaded from Valve's CDN with no server at all,
    /// so a capped or unverified proxy degrades to that rather than leaving a blank tile.
    ///
    /// The `upstream_*` codes belong here for the same reason: when the media proxy cannot reach Valve, the
    /// browser still can, so the tile is painted rather than left empty.
    pub fn cheap_fallback(&self) -> bool {
        matches!(
            self.code.as_str(),
            "session_required"
                | "session_expired"
                | "daily_cap_reached"
                | "server_busy"
                | "degraded"
                | "upstream_busy"
                | "upstream_failed"
                | "upstream_rejected"
                | "not_configured"
        )
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AppError {}

impl From<std::io::Error> for AppError {
    fn from(error: std::io::Error) -> Self {
        Self::new("io_error", format!("Could not read or write a file: {error}"))
    }
}

impl From<serde_json::Error> for AppError {
    fn from(error: serde_json::Error) -> Self {
        Self::new("bad_json", format!("Could not read a saved setting: {error}"))
    }
}

impl From<reqwest::Error> for AppError {
    fn from(error: reqwest::Error) -> Self {
        if error.is_timeout() {
            Self::new("network_timeout", "The server took too long to answer.")
        } else if error.is_connect() {
            Self::new("network_unreachable", "Could not reach the server. Check your connection.")
        } else {
            Self::new("network_error", "The request failed. Try again.")
        }
    }
}

impl From<url::ParseError> for AppError {
    fn from(_: url::ParseError) -> Self {
        Self::new("invalid_url", "That address is not valid.")
    }
}

pub type AppResult<T> = Result<T, AppError>;
