use crate::error::{AppError, AppResult};
use crate::secrets;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use url::Url;

/// The default Worker origin, baked in only when the build was given one. There is no key here and there
/// can never be one: the desktop build holds no upstream credentials at all.
const BUILT_IN_API_BASE: &str = match option_env!("STEAMWARE_API_BASE") {
    Some(value) => value,
    None => "",
};

/// This build's own Worker origin, without a trailing slash. Empty only for a source checkout built without
/// one, which is the one case where the first-run screen still has to ask.
pub fn built_in_api_base() -> &'static str {
    BUILT_IN_API_BASE.trim_end_matches('/')
}

/// This build's own profile folder.
///
/// Deliberately NOT `%APPDATA%\Steamware`: the WPF app already owns that directory, and its settings.json
/// uses a different schema (PascalCase keys, including the user's Hubcap key). Loading it here would
/// silently discard those fields on the first save, so the two apps get separate homes and can coexist
/// during the migration.
pub fn app_dir() -> PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        return PathBuf::from(appdata).join("SteamwareDesktop");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".config").join("steamware-desktop");
    }
    PathBuf::from(".steamware-desktop")
}

pub fn ensure_app_dir() -> AppResult<PathBuf> {
    let dir = app_dir();
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn default_download_dir() -> String {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default();
    PathBuf::from(home).join("Downloads").join("Steamware").to_string_lossy().to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// Base URL of the Steamware Worker. The ONLY host this app makes network requests to.
    pub api_base: String,
    /// Accent colour as #rrggbb. Drives every highlight in the UI.
    pub accent: String,
    /// "dark" or "light".
    pub mode: String,
    /// Where manifest archives and fix packages are written.
    pub download_dir: String,
    /// Optional manual Steam install path; empty means auto-detect.
    pub steam_path: String,
    /// How many downloads may run at once.
    pub max_concurrent: u8,
    /// Called out in Settings; the proxy is the only network path by design.
    pub show_security_banner: bool,
    pub completed_onboarding: bool,
    /// Whether the accent is this profile's own choice. Until it is, the first load may adopt the palette
    /// the WPF build was last using, so an existing user opens this app in the colours they already chose.
    #[serde(default)]
    pub accent_chosen: bool,
    /// The user's own Hubcap key, typed into Settings.
    ///
    /// This one IS a credential, and it is the only one this app ever holds: it belongs to the person using
    /// it, stays in this profile, and is sent to Hubcap's own API — never to the Steamware server, which
    /// does not want it and cannot see it. DepotBox's key is the opposite case and lives only on the Worker.
    ///
    /// In memory this is the key itself. On disk it is never this field: `persist` writes
    /// `hubcapKeySealed` instead, a DPAPI blob only this Windows user can open, and `load` puts it back.
    #[serde(default)]
    pub hubcap_key: String,
    /// Which source a fetch uses by default: "depotbox" (the built-in one) or "hubcap" (only meaningful
    /// once a key is present).
    #[serde(default = "default_source")]
    pub preferred_source: String,
    /// Launch Steamware when Windows starts (an HKCU Run entry, written by `startup`).
    #[serde(default)]
    pub start_with_windows: bool,
    /// Closing the window keeps Steamware in the notification area instead of quitting it.
    #[serde(default)]
    pub minimize_to_tray: bool,
    /// Publish what the app is doing on the user's Discord profile (`presence`).
    #[serde(default = "default_true")]
    pub discord_rpc: bool,
    /// The Discord application whose Rich Presence this profile publishes under.
    ///
    /// Discord only renders a card for a real application. The old WPF build shipped Steamware's own
    /// application id in `discord-app.json` (name, icon, `steamware` art and the Download button all live
    /// there), so this build starts from that same id instead of asking for a one-time setup. It is
    /// replaced, never required: a profile that names another application publishes under that one.
    ///
    /// The Settings page deliberately does not show it. It is a public identifier rather than a credential
    /// — no token, no permissions — but it is also nothing a user needs to read or edit, and a screenshot
    /// of that page should not be the place it travels.
    #[serde(default = "default_discord_app_id")]
    pub discord_app_id: String,
    /// Install a fetched lua with its manifest pins removed, so Steam keeps the game updated instead of
    /// holding it at the version that was fetched. Off is the "locked" install: pinned and reproducible.
    #[serde(default = "default_true")]
    pub auto_update_apps: bool,
    /// Probe a source before downloading from it, so a game DepotBox cannot serve comes from Hubcap instead
    /// of failing. Off means the chosen source is the only one tried — a visible failure rather than a hunt.
    #[serde(default = "default_true")]
    pub fast_fetch: bool,
}

fn default_true() -> bool {
    true
}

/// Steamware's own Discord application — the one the WPF build embedded. Public by design: it is the
/// identity the card renders under, not a secret, and no bot token is involved.
pub const DEFAULT_DISCORD_APP_ID: &str = "1532848200871051315";

fn default_discord_app_id() -> String {
    DEFAULT_DISCORD_APP_ID.to_string()
}

/// "auto" asks for whichever source can actually serve a game, which is what a new profile should do: the
/// old build's "Automatic (recommended)".
fn default_source() -> String {
    "auto".to_string()
}

/// Discord application ids are 17–20 decimal digits. The Developer Portal shows the id in that shape; a
/// pasted URL or a bot token is not one.
pub fn plausible_discord_app_id(value: &str) -> bool {
    let value = value.trim();
    (17..=20).contains(&value.len()) && value.chars().all(|c| c.is_ascii_digit())
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            api_base: BUILT_IN_API_BASE.trim_end_matches('/').to_string(),
            accent: "#a78bfa".to_string(),
            mode: "dark".to_string(),
            download_dir: default_download_dir(),
            steam_path: String::new(),
            max_concurrent: 2,
            show_security_banner: true,
            completed_onboarding: false,
            accent_chosen: false,
            hubcap_key: String::new(),
            preferred_source: default_source(),
            start_with_windows: false,
            minimize_to_tray: false,
            discord_rpc: true,
            discord_app_id: default_discord_app_id(),
            auto_update_apps: true,
            fast_fetch: true,
        }
    }
}

/// The WPF build's accent gallery, id → base hex (`ThemeService.Themes`). Only the id is ever read from
/// the other app's settings; that file also holds the user's Hubcap key and nothing else from it may enter
/// this profile.
const LEGACY_PALETTES: [(&str, &str); 12] = [
    ("violet", "#a78bfa"),
    ("orchid", "#d188dd"),
    ("blossom", "#e8b0d3"),
    ("rose", "#dd8894"),
    ("clay", "#e7c2ac"),
    ("olive", "#d3c569"),
    ("lime", "#b3d775"),
    ("fern", "#77d369"),
    ("mint", "#88dda7"),
    ("teal", "#69d3c5"),
    ("sky", "#a8cfe6"),
    ("indigo", "#8894dd"),
];

/// Read `AccentTheme` out of `%APPDATA%\Steamware\settings.json`, if that file exists and names a palette
/// we know. Read-only, and only that one key: the file's other fields (including a provider key) are none
/// of this build's business.
fn legacy_accent() -> Option<String> {
    let appdata = std::env::var("APPDATA").ok()?;
    let text = fs::read_to_string(PathBuf::from(appdata).join("Steamware").join("settings.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let id = value.get("AccentTheme")?.as_str()?.trim().to_ascii_lowercase();
    LEGACY_PALETTES.iter().find(|(key, _)| *key == id).map(|(_, hex)| hex.to_string())
}

impl Settings {
    pub fn path() -> PathBuf {
        app_dir().join("settings.json")
    }

    pub fn load() -> Self {
        let text = fs::read_to_string(Self::path()).ok();
        let mut settings: Self = text
            .as_deref()
            .and_then(|text| serde_json::from_str(text).ok())
            .unwrap_or_default();

        // A key written by an older build sits in the profile in plaintext. Adopt it as-is and mark the
        // profile for a rewrite below, which puts it back sealed and drops the cleartext copy.
        let mut migrate_key = !settings.hubcap_key.trim().is_empty();
        if !migrate_key {
            migrate_key = settings.adopt_sealed_key(text.as_deref().unwrap_or_default());
        }
        if !settings.accent_chosen {
            if let Some(accent) = legacy_accent() {
                settings.accent = accent;
            }
        }
        // A profile written by a build that had no address baked in carries an empty one, and leaving it
        // empty is what asked those users to deploy a Worker. Fill it from this build instead: the address
        // is public, and asking for something the app already knows is a dead end for a first run.
        if settings.api_base.trim().is_empty() {
            settings.api_base = built_in_api_base().to_string();
        }
        // The same for Rich Presence: a profile from a build that left this blank would otherwise look
        // switched on and never publish. Filling it is what makes the card work without a setup step.
        if settings.discord_app_id.trim().is_empty() {
            settings.discord_app_id = default_discord_app_id();
        }
        if migrate_key {
            // Best effort: a profile that cannot be rewritten still works for this run, and the next save
            // seals it. Written through `persist` rather than `save` so the legacy accent untouched here
            // does not get locked in as this profile's own choice.
            let _ = settings.persist();
        }
        settings
    }

    /// Read `hubcapKeySealed` out of the profile and decrypt it into `hubcap_key`.
    ///
    /// Returns true when the profile has to be rewritten: a blob that no longer opens (a copied profile, a
    /// different Windows user, a wiped DPAPI store) would otherwise sit there looking like a valid key while
    /// every fetch fell back to DepotBox.
    fn adopt_sealed_key(&mut self, text: &str) -> bool {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
            return false;
        };
        let Some(sealed) = value.get("hubcapKeySealed").and_then(|v| v.as_str()) else {
            return false;
        };
        match secrets::unseal(sealed) {
            Some(key) => {
                self.hubcap_key = key;
                false
            }
            // Unreadable: the key is gone (another user, another machine, or a wiped DPAPI store). Drop it
            // from disk rather than retrying a blob that will never open.
            None => true,
        }
    }

    /// Normalise and check every field, then write atomically.
    pub fn save(mut self) -> AppResult<Self> {
        // Saving is the point the profile's accent becomes its own: from here on the WPF palette is never
        // consulted again.
        self.accent_chosen = true;
        self.validate()?;
        self.persist()?;
        Ok(self)
    }

    /// Write the profile, sealing the Hubcap key on the way out.
    ///
    /// The struct's `hubcapKey` field is dropped from the JSON entirely and replaced by `hubcapKeySealed`, so
    /// the cleartext key is never part of the bytes that reach the disk — not as a leftover, not as a
    /// commented field, not even for the instant before a rewrite. If DPAPI refuses to seal (which takes a
    /// broken profile rather than a normal one), the key is simply not written; keeping it in memory for this
    /// run is better than committing it in the clear.
    fn persist(&self) -> AppResult<()> {
        let mut value = serde_json::to_value(self)?;
        if let Some(object) = value.as_object_mut() {
            object.remove("hubcapKey");
            match secrets::seal(&self.hubcap_key) {
                Some(sealed) => {
                    object.insert("hubcapKeySealed".to_string(), serde_json::Value::String(sealed));
                }
                None => {}
            }
        }

        let dir = ensure_app_dir()?;
        let target = dir.join("settings.json");
        let temp = dir.join("settings.json.tmp");
        fs::write(&temp, serde_json::to_vec_pretty(&value)?)?;
        // Windows refuses to rename over an existing file.
        let _ = fs::remove_file(&target);
        fs::rename(&temp, &target)?;
        Ok(())
    }

    pub fn validate(&mut self) -> AppResult<()> {
        self.api_base = self.api_base.trim().trim_end_matches('/').to_string();
        if self.api_base.is_empty() {
            self.api_base = built_in_api_base().to_string();
        }
        if self.api_base.is_empty() {
            return Err(AppError::config(
                "Set your Steamware server address first — it is the URL of the Worker you deployed.",
            ));
        }
        let url = Url::parse(&self.api_base)
            .map_err(|_| AppError::config("That server address is not a valid URL."))?;
        let local = matches!(url.host_str(), Some("127.0.0.1") | Some("localhost") | Some("::1"));
        if url.scheme() != "https" && !(url.scheme() == "http" && local) {
            return Err(AppError::config("The server address must use https."));
        }
        if url.host_str().is_none() {
            return Err(AppError::config("That server address has no host."));
        }

        if !matches!(self.mode.as_str(), "dark" | "light") {
            self.mode = "dark".to_string();
        }

        let accent = self.accent.trim().to_ascii_lowercase();
        let valid_accent = accent.len() == 7
            && accent.starts_with('#')
            && accent[1..].chars().all(|c| c.is_ascii_hexdigit());
        self.accent = if valid_accent { accent } else { "#a78bfa".to_string() };

        self.max_concurrent = self.max_concurrent.clamp(1, 4);

        // The Discord card is rendered by Discord, not by this app: without a real application id it
        // simply never appears. Reject a typo while the settings dialog can still show it, rather than
        // saving it and leaving the user with a switch that appears on and does nothing.
        self.discord_app_id = self.discord_app_id.trim().to_string();
        if self.discord_app_id.is_empty() {
            self.discord_app_id = default_discord_app_id();
        }
        if !self.discord_app_id.is_empty() && !plausible_discord_app_id(&self.discord_app_id) {
            return Err(AppError::config("A Discord application ID is 17–20 digits."));
        }

        if self.download_dir.trim().is_empty() {
            self.download_dir = default_download_dir();
        }
        fs::create_dir_all(&self.download_dir)
            .map_err(|_| AppError::config("That download folder could not be created."))?;

        self.steam_path = self.steam_path.trim().to_string();

        // A key pasted with a trailing newline or wrapping spaces must still work; an empty field means
        // "no Hubcap", which is the default state and not an error.
        self.hubcap_key = self.hubcap_key.trim().to_string();
        if self.hubcap_key.len() > 256 || self.hubcap_key.chars().any(|c| c.is_whitespace()) {
            self.hubcap_key.clear();
        }
        if !matches!(self.preferred_source.as_str(), "auto" | "depotbox" | "hubcap") {
            self.preferred_source = default_source();
        }
        // Hubcap cannot be the default while there is no key to reach it with; falling back to "auto" is what
        // keeps a fetch working after a key is removed ("auto" still prefers DepotBox).
        if self.hubcap_key.is_empty() && self.preferred_source == "hubcap" {
            self.preferred_source = default_source();
        }

        Ok(())
    }
}

/// A stable, anonymous machine id used to bind a session to this installation.
///
/// It is a random number, not a hardware fingerprint: it identifies this install for the length of a
/// session and nothing more. Stored beside settings so a reinstall or a wiped profile simply looks like a
/// new device.
pub fn device_id() -> String {
    let path = app_dir().join("device.id");
    if let Ok(existing) = fs::read_to_string(&path) {
        let trimmed = existing.trim().to_string();
        if trimmed.len() == 32 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
            return trimmed;
        }
    }

    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    let id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let _ = ensure_app_dir();
    let _ = fs::write(&path, &id);
    id
}
