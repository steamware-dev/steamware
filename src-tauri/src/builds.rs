//! The Builds page: what a game's Steam lua declares, and saved variants of it.
//!
//! A lua under `config/stplug-in/<appid>.lua` is a flat list of `addappid(...)` and `setManifestid(...)`
//! lines. Two properties matter to the UI and they are NOT the same thing:
//!
//!   * whether an `addappid` line is active — a `--` in front of it switches that depot/DLC off;
//!   * whether a `setManifestid` line is active — that is a version PIN, and commenting it out means
//!     "track whatever Steam ships" rather than "pin to this manifest".
//!
//! A commented pin is still reported (`manifest`) because it is the size and the last known build of a
//! depot; only `locked` reflects whether it is actually in force. Every edit is a targeted rewrite of the
//! affected line(s), so the rest of the file — comments, ordering, anything another tool wrote — survives.

use crate::error::{AppError, AppResult};
use crate::settings;
use crate::steam;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// A lua is tens of kilobytes. Anything larger is not one of ours, and reading it into the webview would
/// be a memory problem rather than a feature.
const MAX_LUA_BYTES: u64 = 4 * 1024 * 1024;
const MAX_VARIANTS: usize = 40;
const MAX_VARIANT_TEXT: usize = 512 * 1024;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildGame {
    pub app_id: String,
    pub name: String,
    pub has_lua: bool,
    pub depot_count: usize,
    pub pin_count: usize,
    pub variant_count: usize,
    pub last_played: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub id: String,
    /// `base`, `depot` or `dlc`.
    pub kind: String,
    pub enabled: bool,
    /// True when the lua carries a `setManifestid` line for this id, i.e. there is a pin to toggle.
    pub can_lock: bool,
    pub locked: bool,
    pub manifest: Option<String>,
    pub comment: Option<String>,
    pub size: Option<u64>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Variant {
    pub id: String,
    pub name: String,
    pub created_at: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LuaState {
    pub app_id: String,
    pub name: String,
    pub lua_path: String,
    pub exists: bool,
    pub text: String,
    pub rows: Vec<Row>,
    pub depot_count: usize,
    pub dlc_count: usize,
    pub pinned: usize,
    pub variants: Vec<Variant>,
    pub active_variant: Option<String>,
}

/* ── Storage ─────────────────────────────────────────────────────────────────────────────────── */

#[derive(Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Store {
    /// Id of the variant currently applied, or `None` when the live lua is the source of truth.
    active: Option<String>,
    variants: Vec<StoredVariant>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredVariant {
    id: String,
    name: String,
    created_at: i64,
    text: String,
}

/// An app id becomes a file name in two places (the lua under Steam, the saved variants here), so only
/// digits are ever accepted. The command layer checks this before calling in; repeating it where the path
/// is built means a new caller cannot skip the check and walk out of the profile with `..`.
fn safe_app_id(app_id: &str) -> AppResult<&str> {
    let id = app_id.trim();
    if id.is_empty() || id.len() > 10 || !id.chars().all(|c| c.is_ascii_digit()) {
        return Err(AppError::new("invalid_appid", "That app id is not valid."));
    }
    Ok(id)
}

fn store_path(app_id: &str) -> AppResult<PathBuf> {
    Ok(settings::app_dir().join("builds").join(format!("{}.json", safe_app_id(app_id)?)))
}

fn load_store(app_id: &str) -> Store {
    store_path(app_id)
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<Store>(&text).ok())
        .unwrap_or_default()
}

fn save_store(app_id: &str, store: &Store) -> AppResult<()> {
    let path = store_path(app_id)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, serde_json::to_vec_pretty(store)?)?;
    let _ = std::fs::remove_file(&path);
    std::fs::rename(&temp, &path)?;
    Ok(())
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn require_app_id(value: &str) -> AppResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 10 || !trimmed.chars().all(|c| c.is_ascii_digit()) {
        return Err(AppError::new("invalid_appid", "That app id is not valid."));
    }
    Ok(trimmed.to_string())
}

/* ── lua parsing ────────────────────────────────────────────────────────────────────────────── */

/// A call's arguments and its trailing `-- comment`, from one already-comment-stripped line.
///
/// Hand-rolled rather than regex: the grammar is three fixed shapes, and a hand parse keeps the
/// comment intact for the row title without a second pass.
fn call_args<'a>(body: &'a str, name: &str) -> Option<(Vec<String>, Option<String>)> {
    let trimmed = body.trim_start();
    let rest = trimmed.strip_prefix(name)?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('(')?;
    let close = rest.find(')')?;
    let inside = &rest[..close];
    let after = &rest[close + 1..];

    let args: Vec<String> = inside
        .split(',')
        .map(|part| part.trim().trim_matches('"').trim().to_string())
        .collect();
    if args.is_empty() || !args[0].chars().all(|c| c.is_ascii_digit()) {
        return None;
    }

    let comment = after
        .find("--")
        .map(|index| after[index + 2..].trim().to_string())
        .filter(|value| !value.is_empty())
        // A commented-out statement is not a display name; it would make a row read
        // "setManifestid(...)" instead of the depot it describes.
        .filter(|value| {
            let lower = value.to_ascii_lowercase();
            !lower.contains("addappid") && !lower.contains("setmanifestid") && !lower.contains("addtoken")
        });

    Some((args, comment))
}

/// Split a line's trailing `\r` off, so edits keep the file's own line endings.
fn split_eol(line: &str) -> (&str, &str) {
    match line.strip_suffix('\r') {
        Some(body) => (body, "\r"),
        None => (line, ""),
    }
}

fn is_commented(body: &str) -> bool {
    body.trim_start().starts_with("--")
}

fn uncommented(body: &str) -> &str {
    let trimmed = body.trim_start();
    match trimmed.strip_prefix("--") {
        Some(rest) => rest.trim_start(),
        None => trimmed,
    }
}

#[derive(Default)]
struct Acc {
    enabled: bool,
    has_key: bool,
    comment: Option<String>,
    pin_active: Option<String>,
    pin_commented: Option<String>,
    size: Option<u64>,
}

fn parse_rows(text: &str, base_app_id: u64) -> Vec<Row> {
    let mut order: Vec<u64> = Vec::new();
    let mut acc: HashMap<u64, Acc> = HashMap::new();

    for raw in text.split('\n') {
        let (body, _) = split_eol(raw);
        let commented = is_commented(body);
        let code = uncommented(body);

        if let Some((args, comment)) = call_args(code, "addappid") {
            let Ok(id) = args[0].parse::<u64>() else { continue };
            let entry = acc.entry(id).or_insert_with(|| {
                order.push(id);
                Acc::default()
            });
            if !commented {
                entry.enabled = true;
            }
            if args.get(2).map(|key| !key.is_empty()).unwrap_or(false) {
                entry.has_key = true;
            }
            if let Some(comment) = comment {
                if entry.comment.as_ref().map(|old| comment.len() > old.len()).unwrap_or(true) {
                    entry.comment = Some(comment);
                }
            }
        }

        if let Some((args, _)) = call_args(code, "setManifestid") {
            let Ok(id) = args[0].parse::<u64>() else { continue };
            let manifest = args.get(1).cloned().unwrap_or_default();
            if manifest.is_empty() || !manifest.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            let entry = acc.entry(id).or_insert_with(|| {
                order.push(id);
                Acc::default()
            });
            if commented {
                entry.pin_commented.get_or_insert(manifest);
            } else {
                entry.pin_active.get_or_insert(manifest);
            }
            if let Some(size) = args.get(2).and_then(|value| value.parse::<u64>().ok()) {
                entry.size.get_or_insert(size);
            }
        }
    }

    let mut rows: Vec<Row> = order
        .into_iter()
        .map(|id| {
            let entry = acc.remove(&id).unwrap_or_default();
            let kind = if id == base_app_id {
                "base"
            } else if entry.has_key {
                "depot"
            } else {
                "dlc"
            };
            Row {
                id: id.to_string(),
                kind: kind.to_string(),
                enabled: entry.enabled,
                can_lock: entry.pin_active.is_some() || entry.pin_commented.is_some(),
                locked: entry.pin_active.is_some(),
                manifest: entry.pin_active.or(entry.pin_commented),
                comment: entry.comment,
                size: entry.size,
            }
        })
        .collect();

    // Base first, then content depots, then DLC entitlements — the order the WPF list used, and the only
    // ordering that makes the "switched off" group readable.
    let rank = |row: &Row| match row.kind.as_str() {
        "base" => 0,
        "depot" => 1,
        _ => 2,
    };
    rows.sort_by(|a, b| {
        rank(a)
            .cmp(&rank(b))
            .then_with(|| a.id.parse::<u64>().unwrap_or(0).cmp(&b.id.parse::<u64>().unwrap_or(0)))
    });
    rows
}

fn counts(rows: &[Row]) -> (usize, usize, usize) {
    let depots = rows.iter().filter(|row| row.kind == "depot").count();
    let dlcs = rows.iter().filter(|row| row.kind == "dlc").count();
    let pins = rows.iter().filter(|row| row.locked).count();
    (depots, dlcs, pins)
}

/// Comment or uncomment every `name (id, …)` line.
fn rewrite_line_state(text: &str, name: &str, id: u64, enable: bool) -> String {
    let mut changed = false;
    let mut out = String::with_capacity(text.len() + 8);

    for (index, raw) in text.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        let (body, eol) = split_eol(raw);
        let commented = is_commented(body);
        let code = uncommented(body);

        let matches = match call_args(code, name) {
            Some((args, _)) => args[0].parse::<u64>().ok() == Some(id),
            None => false,
        };

        if !matches || commented == !enable {
            out.push_str(body);
            out.push_str(eol);
            continue;
        }

        changed = true;
        if enable {
            // Drop the marker but keep the line's own indentation.
            let indent_len = body.len() - body.trim_start().len();
            let (indent, rest) = body.split_at(indent_len);
            let rest = rest.trim_start().strip_prefix("--").unwrap_or(rest).trim_start();
            out.push_str(indent);
            out.push_str(rest);
        } else {
            let indent_len = body.len() - body.trim_start().len();
            let (indent, rest) = body.split_at(indent_len);
            out.push_str(indent);
            out.push_str("--");
            out.push_str(rest);
        }
        out.push_str(eol);
    }

    if changed {
        out
    } else {
        text.to_string()
    }
}

/* ── Paths ──────────────────────────────────────────────────────────────────────────────────── */

fn lua_dir(steam_path: &str) -> AppResult<PathBuf> {
    steam::stub_plugin_dir(steam_path).ok_or_else(|| {
        AppError::new(
            "steam_missing",
            "Steam was not found. Set its folder in Settings, then try again.",
        )
    })
}

fn lua_file(steam_path: &str, app_id: &str) -> AppResult<PathBuf> {
    Ok(lua_dir(steam_path)?.join(format!("{}.lua", safe_app_id(app_id)?)))
}

fn read_text(path: &Path) -> AppResult<(bool, String)> {
    match std::fs::metadata(path) {
        Ok(meta) => {
            if meta.len() > MAX_LUA_BYTES {
                return Err(AppError::new("lua_too_large", "That lua is too large to open here."));
            }
            Ok((true, std::fs::read_to_string(path)?))
        }
        Err(_) => Ok((false, String::new())),
    }
}

fn write_text(path: &Path, text: &str) -> AppResult<()> {
    if text.len() > MAX_VARIANT_TEXT {
        return Err(AppError::new("lua_too_large", "That lua is too large to save."));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension("lua.tmp");
    std::fs::write(&temp, text)?;
    let _ = std::fs::remove_file(path);
    std::fs::rename(&temp, path)?;
    Ok(())
}

/* ── Public surface ──────────────────────────────────────────────────────────────────────────── */

pub fn list(steam_path: &str) -> Vec<BuildGame> {
    let Ok(dir) = lua_dir(steam_path) else {
        return Vec::new();
    };
    let store_dir = settings::app_dir().join("builds");

    steam::library(steam_path)
        .into_iter()
        .filter(|game| game.is_installed())
        .map(|game| {
            let path = dir.join(format!("{}.lua", game.app_id));
            let (has_lua, text) = read_text(&path).unwrap_or((false, String::new()));
            let (depots, _, pins) = counts(&parse_rows(&text, game.app_id.parse().unwrap_or(0)));
            let variant_count = std::fs::read_to_string(store_dir.join(format!("{}.json", game.app_id)))
                .ok()
                .and_then(|raw| serde_json::from_str::<Store>(&raw).ok())
                .map(|store| store.variants.len())
                .unwrap_or(0);
            BuildGame {
                app_id: game.app_id,
                name: game.name,
                has_lua,
                depot_count: depots,
                pin_count: pins,
                variant_count,
                last_played: game.last_played,
            }
        })
        .collect()
}

fn assemble(steam_path: &str, app_id: &str) -> AppResult<LuaState> {
    let path = lua_file(steam_path, app_id)?;
    let (exists, text) = read_text(&path)?;
    let rows = parse_rows(&text, app_id.parse().unwrap_or(0));
    let (depot_count, dlc_count, pinned) = counts(&rows);
    let name = steam::library(steam_path)
        .into_iter()
        .find(|game| game.app_id == app_id)
        .map(|game| game.name)
        .unwrap_or_else(|| format!("App {app_id}"));

    let store = load_store(app_id);
    Ok(LuaState {
        app_id: app_id.to_string(),
        name,
        lua_path: path.to_string_lossy().to_string(),
        exists,
        text,
        rows,
        depot_count,
        dlc_count,
        pinned,
        variants: store
            .variants
            .iter()
            .map(|variant| Variant {
                id: variant.id.clone(),
                name: variant.name.clone(),
                created_at: variant.created_at,
            })
            .collect(),
        active_variant: store.active,
    })
}

pub fn state(steam_path: &str, app_id: &str) -> AppResult<LuaState> {
    let app_id = require_app_id(app_id)?;
    assemble(steam_path, &app_id)
}

pub fn set_entry(
    steam_path: &str,
    app_id: &str,
    entry_id: &str,
    field: &str,
    value: bool,
) -> AppResult<LuaState> {
    let app_id = require_app_id(app_id)?;
    let entry_id = require_app_id(entry_id)?;
    let id: u64 = entry_id
        .parse()
        .map_err(|_| AppError::new("invalid_appid", "That depot id is not valid."))?;

    let path = lua_file(steam_path, &app_id)?;
    let (_, text) = read_text(&path)?;
    let name = match field {
        "enabled" => "addappid",
        "locked" => "setManifestid",
        _ => return Err(AppError::new("invalid_field", "Unknown lua field.")),
    };
    let updated = rewrite_line_state(&text, name, id, value);
    if updated != text {
        write_text(&path, &updated)?;
        // Editing the live file means it is no longer any saved variant.
        let mut store = load_store(&app_id);
        store.active = None;
        let _ = save_store(&app_id, &store);
    }
    assemble(steam_path, &app_id)
}

pub fn write_lua(steam_path: &str, app_id: &str, text: &str) -> AppResult<LuaState> {
    let app_id = require_app_id(app_id)?;
    let path = lua_file(steam_path, &app_id)?;
    write_text(&path, text)?;
    let mut store = load_store(&app_id);
    store.active = None;
    let _ = save_store(&app_id, &store);
    assemble(steam_path, &app_id)
}

/* ── Variants ───────────────────────────────────────────────────────────────────────────────── */

pub fn save_variant(steam_path: &str, app_id: &str, name: &str) -> AppResult<LuaState> {
    let app_id = require_app_id(app_id)?;
    let path = lua_file(steam_path, &app_id)?;
    let (_, text) = read_text(&path)?;
    if text.trim().is_empty() {
        return Err(AppError::new("no_lua", "There is no lua to save yet — download or paste one first."));
    }

    let label = name.trim();
    let label = if label.is_empty() {
        format!("Build {}", load_store(&app_id).variants.len() + 1)
    } else {
        label.chars().take(60).collect()
    };

    let mut store = load_store(&app_id);
    if store.variants.len() >= MAX_VARIANTS {
        return Err(AppError::new("too_many_variants", "Delete a build before saving another."));
    }
    let id = format!("{:016x}", rand::random::<u64>());
    store.variants.push(StoredVariant { id: id.clone(), name: label, created_at: now_secs(), text });
    store.active = Some(id);
    save_store(&app_id, &store)?;
    assemble(steam_path, &app_id)
}

pub fn apply_variant(steam_path: &str, app_id: &str, variant_id: &str) -> AppResult<LuaState> {
    let app_id = require_app_id(app_id)?;
    let mut store = load_store(&app_id);
    let Some(variant) = store.variants.iter().find(|variant| variant.id == variant_id).cloned() else {
        return Err(AppError::new("missing_variant", "That build no longer exists."));
    };
    let path = lua_file(steam_path, &app_id)?;
    write_text(&path, &variant.text)?;
    store.active = Some(variant.id);
    save_store(&app_id, &store)?;
    assemble(steam_path, &app_id)
}

pub fn rename_variant(steam_path: &str, app_id: &str, variant_id: &str, name: &str) -> AppResult<LuaState> {
    let app_id = require_app_id(app_id)?;
    let label: String = name.trim().chars().take(60).collect();
    if label.is_empty() {
        return Err(AppError::new("invalid_name", "Give that build a name."));
    }
    let mut store = load_store(&app_id);
    let Some(variant) = store.variants.iter_mut().find(|variant| variant.id == variant_id) else {
        return Err(AppError::new("missing_variant", "That build no longer exists."));
    };
    variant.name = label;
    save_store(&app_id, &store)?;
    assemble(steam_path, &app_id)
}

pub fn delete_variant(steam_path: &str, app_id: &str, variant_id: &str) -> AppResult<LuaState> {
    let app_id = require_app_id(app_id)?;
    let mut store = load_store(&app_id);
    store.variants.retain(|variant| variant.id != variant_id);
    if store.active.as_deref() == Some(variant_id) {
        store.active = None;
    }
    save_store(&app_id, &store)?;
    assemble(steam_path, &app_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LUA: &str = "addappid(730)\naddappid(731, 1, \"abc\")\n--addappid(732, 1, \"def\") -- Extra Depot\nsetManifestid(731, \"111\", 2048)\n--setManifestid(732, \"222\", 4096)\n";

    #[test]
    fn parses_active_disabled_depots_and_pins() {
        let rows = parse_rows(LUA, 730);
        assert_eq!(rows.len(), 3);

        let base = &rows[0];
        assert_eq!(base.id, "730");
        assert_eq!(base.kind, "base");
        assert!(base.enabled);

        let depot = &rows[1];
        assert_eq!(depot.id, "731");
        assert_eq!(depot.kind, "depot");
        assert!(depot.locked);
        assert_eq!(depot.manifest.as_deref(), Some("111"));
        assert_eq!(depot.size, Some(2048));

        let disabled = &rows[2];
        assert_eq!(disabled.kind, "depot");
        assert!(!disabled.enabled);
        assert!(!disabled.locked, "a commented pin is not in force");
        assert!(disabled.can_lock);
        assert_eq!(disabled.comment.as_deref(), Some("Extra Depot"));
    }

    #[test]
    fn toggling_is_reversible_and_keeps_line_endings() {
        let off = rewrite_line_state(LUA, "addappid", 731, false);
        assert!(off.contains("--addappid(731, 1, \"abc\")"));
        let on = rewrite_line_state(&off, "addappid", 731, true);
        assert_eq!(on, LUA.replace('\n', "\n"));

        let crlf = LUA.replace('\n', "\r\n");
        let round = rewrite_line_state(&rewrite_line_state(&crlf, "addappid", 731, false), "addappid", 731, true);
        assert_eq!(round, crlf, "CRLF files must not be rewritten as LF");
    }

    #[test]
    fn locking_comments_only_the_pin_line() {
        let unlocked = rewrite_line_state(LUA, "setManifestid", 731, false);
        assert!(unlocked.contains("--setManifestid(731, \"111\", 2048)"));
        assert!(unlocked.contains("\naddappid(731, 1, \"abc\")"));
        let rows = parse_rows(&unlocked, 730);
        assert!(!rows[1].locked);
        assert_eq!(rows[1].manifest.as_deref(), Some("111"), "the pin is still reported, just not in force");
    }
}

#[cfg(test)]
mod path_tests {
    use super::*;

    #[test]
    fn app_ids_never_name_a_path_outside_the_profile() {
        assert_eq!(safe_app_id("730").unwrap(), "730");
        assert!(safe_app_id("../../Steam/config/loginusers").is_err());
        assert!(safe_app_id(r"..\..\evil").is_err());
        assert!(safe_app_id("").is_err());
        assert!(safe_app_id("730/../1").is_err());
    }
}
