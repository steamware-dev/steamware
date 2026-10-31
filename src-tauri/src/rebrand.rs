//! Rebranding the store-page plugin so it is Steamware's, not the upstream bundle's.
//!
//! The frontend is authored in `madoiscool/LTSP` and its text, links, manifest and embedded icons all say
//! LuaTools. The Windows build rewrote those at install time and again on every load, so an install made by
//! an older build ended up correct without a reinstall. This is the same pass, with the same rules:
//!
//!   * `LuaTools` as PROSE becomes `Steamware`; identifiers such as `HasLuaToolsForApp`, `luatools` and
//!     `__LuaToolsReady` are the protocol and are never touched, because the replacement requires a
//!     non-identifier character on both sides;
//!   * the old community host is repointed at Steamware's;
//!   * `plugin.json`'s product identity is written explicitly (`name`, `common_name`, `description`,
//!     `version`) and the dead Lua backend declaration is dropped;
//!   * the bundle's small fallback mark is replaced with Steamware's icon;
//!   * `backend/` is removed — it only ever ran inside Millennium's Lua host, which this app does not use.
//!
//! Every step is best-effort and idempotent: a file that already reads correctly is not rewritten, and a
//! locked file never fails an install.

use base64::Engine;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

/// The version Steamware presents for its plugin. The bundle comes from an upstream release whose tag is
/// that project's own version, so this is fixed to the app's plugin release and shown instead.
pub const PLUGIN_VERSION: &str = "v1.0.1";

/// The `version` field of the installed `plugin.json`. Numeric, no leading `v`, matching the manifest's
/// convention.
pub const PLUGIN_MANIFEST_VERSION: &str = "1.0.1";

/// The Steamware mark, used for the plugin's embedded icon and its logo artwork.
pub const ICON_PNG: &[u8] = include_bytes!("../resources/steamware-icon.png");

const BRAND_FROM: &str = "LuaTools";
const BRAND_TO: &str = "Steamware";

const LINK_REPLACEMENTS: [(&str, &str); 1] = [("discord.gg/luatools", "discord.gg/steamtools")];

/// Names a bundle may still carry for sources this build no longer offers.
const RETIRED_SOURCE_REPLACEMENTS: [(&str, &str); 1] = [("ManifestHub", "DepotBox")];

const TEXT_EXTENSIONS: [&str; 9] = [".js", ".mjs", ".cjs", ".css", ".html", ".htm", ".json", ".txt", ".lua"];

fn is_identifier(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '_' | '$' | '.')
}

/// Rewrite the brand wherever it reads as prose rather than as an identifier.
pub fn rebrand_brand_text(text: &str) -> String {
    if !text.contains(BRAND_FROM) {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find(BRAND_FROM) {
        let before = rest[..index].chars().next_back();
        let after = rest[index + BRAND_FROM.len()..].chars().next();
        let prose = before.map_or(true, |c| !is_identifier(c)) && after.map_or(true, |c| !is_identifier(c));
        out.push_str(&rest[..index]);
        if prose {
            out.push_str(BRAND_TO);
        } else {
            out.push_str(BRAND_FROM);
        }
        rest = &rest[index + BRAND_FROM.len()..];
    }
    out.push_str(rest);
    out
}

/// Case-insensitive literal replacement, used for retired source names that also appear lower-cased in ids.
fn replace_ignore_case(text: &str, from: &str, to: &str) -> String {
    if from.is_empty() {
        return text.to_string();
    }
    let lower = text.to_ascii_lowercase();
    let needle = from.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    while let Some(found) = lower[cursor..].find(&needle) {
        let start = cursor + found;
        out.push_str(&text[cursor..start]);
        out.push_str(to);
        cursor = start + from.len();
    }
    out.push_str(&text[cursor..]);
    out
}

fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    if bytes.len() < 24 || bytes[..8] != SIGNATURE {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((width, height))
}

/// Swap the bundle's embedded offline-fallback app icon for Steamware's. Only small PNGs are touched, so a
/// data URL that is a screenshot or a theme preview is left alone.
fn replace_small_icon_data_urls(text: &str) -> String {
    const PREFIX: &str = "data:image/png;base64,";
    if !text.contains(PREFIX) {
        return text.to_string();
    }
    let replacement = format!(
        "{PREFIX}{}",
        base64::engine::general_purpose::STANDARD.encode(ICON_PNG)
    );
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    while let Some(found) = text[cursor..].find(PREFIX) {
        let start = cursor + found;
        out.push_str(&text[cursor..start]);
        let payload_start = start + PREFIX.len();
        let payload_end = text[payload_start..]
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '+' && c != '/' && c != '=')
            .map(|offset| payload_start + offset)
            .unwrap_or(text.len());
        let payload = &text[payload_start..payload_end];
        let replace = (64..=40_000).contains(&payload.len())
            && base64::engine::general_purpose::STANDARD
                .decode(payload)
                .ok()
                .and_then(|bytes| png_dimensions(&bytes))
                .map(|(width, height)| width > 0 && width <= 64 && height > 0 && height <= 64)
                .unwrap_or(false);
        if replace {
            out.push_str(&replacement);
        } else {
            out.push_str(&text[start..payload_end]);
        }
        cursor = payload_end;
    }
    out.push_str(&text[cursor..]);
    out
}

/// Rewrite a text file in place. Returns true when it changed.
pub fn rebrand_text_file(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let mut updated = rebrand_brand_text(&text);
    for (from, to) in LINK_REPLACEMENTS {
        updated = updated.replace(from, to);
    }
    for (from, to) in RETIRED_SOURCE_REPLACEMENTS {
        updated = replace_ignore_case(&updated, from, to);
    }
    updated = replace_small_icon_data_urls(&updated);
    if updated == text {
        return false;
    }
    std::fs::write(path, updated).is_ok()
}

/// Rewrite the plugin manifest's product identity, leaving every other key intact.
pub fn rebrand_manifest(plugin_root: &Path) -> bool {
    let path = plugin_root.join("plugin.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return false;
    };
    let Ok(Value::Object(mut root)) = serde_json::from_str::<Value>(&text) else {
        return false;
    };
    let identity = [
        ("name", "steamware"),
        ("common_name", "Steamware"),
        ("description", "Steamware Steam Plugin!"),
        ("version", PLUGIN_MANIFEST_VERSION),
    ];
    let mut changed = false;
    for (key, value) in identity {
        if root.get(key).and_then(Value::as_str) == Some(value) {
            continue;
        }
        root.insert(key.to_string(), json!(value));
        changed = true;
    }
    changed |= root.remove("useBackend").is_some();
    changed |= root.remove("backendType").is_some();
    if !changed {
        return false;
    }
    let mut ordered = Map::new();
    if let Some(schema) = root.remove("$schema") {
        ordered.insert("$schema".to_string(), schema);
    }
    for (key, value) in root {
        ordered.insert(key, value);
    }
    match serde_json::to_string_pretty(&Value::Object(ordered)) {
        Ok(body) => std::fs::write(path, body).is_ok(),
        Err(_) => false,
    }
}

fn write_if_changed(path: &Path, bytes: &[u8]) -> bool {
    if std::fs::read(path).map(|current| current == bytes).unwrap_or(false) {
        return false;
    }
    std::fs::write(path, bytes).is_ok()
}

fn walk_files(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_files(&path, out);
        } else {
            out.push(path);
        }
    }
}

/// Rebrand an installed plugin tree in place: manifest, brand text, links, logos, and the dead Lua backend.
pub fn rebrand_dir(plugin_root: &Path) {
    if !plugin_root.is_dir() {
        return;
    }
    rebrand_manifest(plugin_root);
    let backend = plugin_root.join("backend");
    if backend.is_dir() {
        let _ = std::fs::remove_dir_all(backend);
    }

    let mut files = Vec::new();
    walk_files(plugin_root, &mut files);
    for file in files {
        let extension = file
            .extension()
            .map(|value| format!(".{}", value.to_string_lossy().to_ascii_lowercase()))
            .unwrap_or_default();
        if extension == ".png" {
            // Only logo art: "icon" is in the name of the bundle's own mark and nothing else in the tree.
            let named_icon = file
                .file_stem()
                .map(|stem| stem.to_string_lossy().to_ascii_lowercase().contains("icon"))
                .unwrap_or(false);
            if named_icon {
                write_if_changed(&file, ICON_PNG);
            }
            continue;
        }
        if TEXT_EXTENSIONS.iter().any(|allowed| *allowed == extension) {
            rebrand_text_file(&file);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_prose_but_not_identifiers() {
        let text = "LuaTools · Menu HasLuaToolsForApp StartLuaToolsAdd __LuaToolsReady luatools; \"LuaTools\"";
        let out = rebrand_brand_text(text);
        assert!(out.contains("Steamware · Menu"));
        assert!(out.contains("HasLuaToolsForApp"));
        assert!(out.contains("StartLuaToolsAdd"));
        assert!(out.contains("__LuaToolsReady"));
        assert!(out.contains("luatools;"));
        assert!(out.contains("\"Steamware\""));
        assert!(!out.contains("LuaTools · Menu"));
    }

    #[test]
    fn rewrites_the_old_community_link() {
        let dir = std::env::temp_dir().join("steamware-rebrand-link");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.js");
        std::fs::write(&file, "open('https://discord.gg/luatools')").unwrap();
        assert!(rebrand_text_file(&file));
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("discord.gg/steamtools"));
        assert!(!text.contains("discord.gg/luatools"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn manifest_becomes_steamware_v1() {
        let dir = std::env::temp_dir().join("steamware-rebrand-manifest");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("plugin.json"),
            r#"{"name":"luatools","common_name":"LuaTools","version":"9.0.1","useBackend":true,"backendType":"lua","include":["public"]}"#,
        )
        .unwrap();
        assert!(rebrand_manifest(&dir));
        let body: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("plugin.json")).unwrap()).unwrap();
        assert_eq!(body["name"], "steamware");
        assert_eq!(body["common_name"], "Steamware");
        assert_eq!(body["version"], PLUGIN_MANIFEST_VERSION);
        assert!(body.get("useBackend").is_none());
        assert!(body.get("backendType").is_none());
        assert_eq!(body["include"][0], "public");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn small_embedded_icons_are_swapped_for_steamwares() {
        // A 1x1 PNG, which is inside the "app icon" size window.
        let tiny = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
        let text = format!("var icon='data:image/png;base64,{tiny}';");
        let out = replace_small_icon_data_urls(&text);
        assert!(out.contains(&base64::engine::general_purpose::STANDARD.encode(ICON_PNG)));
        assert!(!out.contains(tiny));
    }

    #[test]
    fn large_png_data_urls_are_left_alone() {
        let big = base64::engine::general_purpose::STANDARD.encode({
            let mut bytes = ICON_PNG.to_vec();
            // Pretend the mark is 128 px wide: still a PNG, but outside the app-icon window.
            bytes[16..20].copy_from_slice(&128u32.to_be_bytes());
            bytes
        });
        let text = format!("var shot='data:image/png;base64,{big}';");
        assert_eq!(replace_small_icon_data_urls(&text), text);
    }
}
