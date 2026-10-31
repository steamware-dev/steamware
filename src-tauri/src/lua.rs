//! Installing what a manifest archive carries into Steam.
//!
//! A downloaded archive holds two kinds of file and they belong in two different places:
//!
//!   * `<appid>.lua` (sometimes `<appid>_<build>.lua`) → `config/stplug-in`, the folder Steam Tools reads;
//!   * `<depot>_<manifest>.manifest` → `depotcache`, a sibling of `steamapps`.
//!
//! Both halves matter. A lua without its manifests loads, lists its depots and then fails every content
//! check, which is exactly what happened when only the lua was installed.
//!
//! This is the one place the app writes outside its own profile, and it writes only files Steam itself
//! loads. Entry names come from the archive, so each is flattened to a bare file name and sized — a crafted
//! archive must not be able to escape the target directory, or to fill the disk through one entry.

use crate::error::{AppError, AppResult};
use crate::steam;
use serde::Serialize;
use std::io::Read as _;

/// An archive with more entries than this is not a manifest bundle.
const MAX_ENTRIES: usize = 4_096;
/// A single lua is a few kilobytes; a manifest is megabytes, never hundreds.
const MAX_LUA_BYTES: u64 = 8 << 20;
const MAX_MANIFEST_BYTES: u64 = 512 << 20;
/// Ceiling on everything unpacked from one archive.
const MAX_TOTAL_BYTES: u64 = 4 << 30;

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallSummary {
    pub lua: usize,
    pub manifests: usize,
    /// The plugin folder is registered with the unlocker, so a running Steam picks these up by itself. False
    /// means the files are in place and Steam needs one restart to read the folder.
    pub live: bool,
    /// Steam is running right now, which is only interesting next to `live`: together they say whether the
    /// game is in the library already or on the next start.
    pub steam_running: bool,
    /// The game was written into Steam's library (a manifest plus, with Steam open, a live install
    /// request), so it shows up as a library entry rather than only as a licence.
    pub library: bool,
}

impl InstallSummary {
    pub fn total(&self) -> usize {
        self.lua + self.manifests
    }
}

/// What taking one game back out of Steam actually did, so the UI can say what is gone and what is not.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemovalSummary {
    /// The lua Steam loads. Without it the game is gone from Steamware's library and the licence is gone.
    pub lua: bool,
    /// The `appmanifest` record Steamware wrote, which is what makes Steam list the game as an entry.
    pub library: bool,
    /// Steam has the game's own record with content behind it, so it stays listed as installed.
    pub still_installed: bool,
    /// Steam only re-reads these folders at startup, so a running client keeps the old tile until then.
    pub steam_running: bool,
}

/// The bare file name of an entry, or None if the name is unusable.
fn entry_file_name<R: std::io::Read>(entry: &zip::read::ZipFile<'_, R>) -> Option<String> {
    let candidate = entry
        .enclosed_name()
        .and_then(|path| path.file_name().map(|name| name.to_string_lossy().to_string()))
        .or_else(|| {
            entry
                .name()
                .rsplit(['/', '\\'])
                .next()
                .map(|name| name.to_string())
        })?;
    if candidate.is_empty() || candidate.contains("..") || candidate.contains(['/', '\\', ':']) {
        return None;
    }
    Some(candidate)
}

/// A lua's on-disk name, as Steam should see it.
///
/// `<appid>_<build>.lua` is renamed to `<appid>.lua`: that is the name Steam Tools loads, and the build id
/// only ever labelled which variant of the file it was. A name that carries no number is left alone rather
/// than guessed at.
fn lua_target_name(name: &str) -> String {
    let stem = name.trim_end_matches(".lua").trim_end_matches(".LUA");
    let app_id = stem.split('_').next().unwrap_or(stem);
    if !app_id.is_empty() && app_id.len() <= 10 && app_id.chars().all(|c| c.is_ascii_digit()) {
        return format!("{app_id}.lua");
    }
    name.to_string()
}

fn read_entry<R: std::io::Read>(entry: &mut zip::read::ZipFile<'_, R>, limit: u64) -> Option<Vec<u8>> {
    let mut buffer = Vec::new();
    std::io::copy(&mut entry.by_ref().take(limit + 1), &mut buffer).ok()?;
    (buffer.len() as u64 <= limit).then_some(buffer)
}

/// Comment out every active `setManifestid(...)` line, which is what un-pins a depot: a pinned lua holds
/// the game at the build it was fetched from, and without the pin Steam fetches the current one.
///
/// Same rule as the old standalone's `LuaInstaller`: already-commented lines are left alone, leading
/// whitespace is preserved, and only `setManifestid` is touched (`addappid` still supplies the depot's key).
pub fn unpin(lua: &str) -> String {
    let mut lines: Vec<String> = Vec::with_capacity(lua.lines().count() + 1);
    for line in lua.split('\n') {
        let indent = line.len() - line.trim_start().len();
        let body = &line[indent..];
        if body.starts_with("setManifestid") || body.starts_with("setmanifestid") {
            lines.push(format!("{}-- {}", &line[..indent], body));
        } else {
            lines.push(line.to_string());
        }
    }
    lines.join("\n")
}

/// Install every `.lua` and `.manifest` in `archive`. Fails when Steam is missing or the archive carries
/// neither kind of file.
///
/// `unpin` is "Auto Update Apps (Don't Lock Manifests)": the lua is installed with its manifest pins
/// commented out, so Steam keeps the game updated. It is off for fixes, which must stay on the version
/// they were built for or the fix stops applying.
pub fn install_archive(archive: &str, steam_path: &str, unpin_luas: bool) -> AppResult<InstallSummary> {
    let Some(plugin_dir) = steam::stub_plugin_dir(steam_path) else {
        return Err(AppError::new(
            "steam_missing",
            "Steam was not found. Set its folder in Settings, then try again.",
        ));
    };
    let Some(cache_dir) = steam::depotcache_dir(steam_path) else {
        return Err(AppError::new("steam_missing", "Steam was not found."));
    };

    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|_| AppError::new("bad_archive", "That download is not a readable archive."))?;

    if zip.len() > MAX_ENTRIES {
        return Err(AppError::new("bad_archive", "That archive has too many files."));
    }

    let mut summary = InstallSummary::default();
    let mut unpacked: u64 = 0;
    // The first lua in the archive is its base app; anything after it is DLC, which must not get a library
    // entry of its own.
    let mut base: Option<(String, String)> = None;

    for index in 0..zip.len() {
        let Ok(mut entry) = zip.by_index(index) else {
            continue;
        };
        if entry.is_dir() {
            continue;
        }
        let lower = entry.name().to_ascii_lowercase();
        let is_lua = lower.ends_with(".lua");
        let is_manifest = lower.ends_with(".manifest");
        if !is_lua && !is_manifest {
            continue;
        }
        let Some(name) = entry_file_name(&entry) else {
            continue;
        };

        if is_lua {
            let Some(bytes) = read_entry(&mut entry, MAX_LUA_BYTES) else {
                continue;
            };
            unpacked += bytes.len() as u64;
            if unpacked > MAX_TOTAL_BYTES {
                break;
            }
            if std::fs::create_dir_all(&plugin_dir).is_err() {
                continue;
            }
            let target = lua_target_name(&name);
            if base.is_none() {
                let app_id = target.trim_end_matches(".lua").to_string();
                let game = steam::lua_name(&String::from_utf8_lossy(&bytes)).unwrap_or_default();
                base = Some((app_id, game));
            }
            // Rewritten only when the setting asks for it, and only as text: a lua is plain Lua, so
            // commenting a line is a safe transform even for one this app has never seen before.
            let bytes = if unpin_luas {
                unpin(&String::from_utf8_lossy(&bytes)).into_bytes()
            } else {
                bytes
            };
            if std::fs::write(plugin_dir.join(target), bytes).is_ok() {
                summary.lua += 1;
            }
            continue;
        }

        // Manifests are content-addressed: the same id is the same bytes, so an existing file is done.
        let destination = cache_dir.join(&name);
        if destination.is_file() {
            summary.manifests += 1;
            continue;
        }
        let Some(bytes) = read_entry(&mut entry, MAX_MANIFEST_BYTES) else {
            continue;
        };
        unpacked += bytes.len() as u64;
        if unpacked > MAX_TOTAL_BYTES {
            break;
        }
        if std::fs::create_dir_all(&cache_dir).is_err() {
            continue;
        }
        if std::fs::write(&destination, bytes).is_ok() {
            summary.manifests += 1;
        }
    }

    if summary.total() == 0 {
        return Err(AppError::new(
            "no_lua",
            "That archive did not contain any .lua or .manifest files.",
        ));
    }

    // A lua only reaches a *running* Steam when the unlocker watches the folder it landed in. Registering
    // that is what makes an add show up in the library in seconds instead of on the next start, so it rides
    // along with the install rather than depending on the user having installed their mode through this app.
    // Best-effort: a read-only Steam root must report the install, not fail it.
    if summary.lua > 0 {
        if let Some(root) = steam::detect_install(steam_path) {
            summary.live = crate::modes::ensure_lua_path(&root).is_ok();
        }
        // A lua fakes the licence; the library needs its own record, or the game is owned and invisible.
        if let Some((app_id, game)) = base {
            summary.library = steam::register_in_library(steam_path, &app_id, &game);
        }
    }
    summary.steam_running = steam::is_running();
    Ok(summary)
}

/// The installed lua for one app, if any. `.lua.disabled` counts as installed: this app only ever writes
/// the active name, but a user may have switched it off by hand.
pub fn installed_lua(steam_path: &str, app_id: &str) -> Option<std::path::PathBuf> {
    // The id is joined into a path; digits only means a caller cannot name `..\..\something.lua`.
    if app_id.is_empty() || app_id.len() > 10 || !app_id.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let dir = steam::stub_plugin_dir(steam_path)?;
    let active = dir.join(format!("{app_id}.lua"));
    if active.is_file() {
        return Some(active);
    }
    let disabled = dir.join(format!("{app_id}.lua.disabled"));
    disabled.is_file().then_some(disabled)
}

/// Take one game out of Steam: its `<appid>.lua` and a parked `.disabled` twin from the plugin folder, and
/// the `appmanifest` record `install_archive` wrote into `steamapps`.
///
/// Both halves are the game's presence in the two libraries. The lua is what Steamware lists and what fakes
/// the licence; the manifest is the tile in Steam's own library. Removing only the lua left a game that was
/// still listed and could never start.
///
/// The app id is digits-only and the manifest name is derived from it, so the delete can only ever reach
/// `<appid>.lua` and `appmanifest_<appid>.acf` inside Steam's own folders — never a path from the frontend.
/// A downloaded game keeps Steam's record; `still_installed` says so rather than pretending it is gone.
pub fn remove_installed(steam_path: &str, app_id: &str) -> AppResult<RemovalSummary> {
    if app_id.is_empty() || app_id.len() > 10 || !app_id.chars().all(|c| c.is_ascii_digit()) {
        return Err(AppError::new("invalid_app_id", "That is not a Steam App ID."));
    }
    let mut summary = RemovalSummary::default();
    if let Some(dir) = steam::stub_plugin_dir(steam_path) {
        for name in [format!("{app_id}.lua"), format!("{app_id}.lua.disabled")] {
            let path = dir.join(name);
            if path.is_file() {
                std::fs::remove_file(&path)?;
                summary.lua = true;
            }
        }
    }
    let library = steam::unregister_from_library(steam_path, app_id);
    summary.library = library.removed;
    summary.still_installed = library.downloaded;
    summary.steam_running = steam::is_running();
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::path::PathBuf;

    /// A throwaway Steam root: `steamapps` is what `detect_install` looks for, `config` is where the lua
    /// goes and `depotcache` is where the manifests go.
    fn fake_steam(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("steamware-test-{tag}-{}", rand::random::<u32>()));
        std::fs::create_dir_all(root.join("steamapps")).unwrap();
        std::fs::create_dir_all(root.join("config").join("stplug-in")).unwrap();
        root
    }

    fn zip_with(path: &std::path::Path, entries: &[(&str, &[u8])]) {
        let file = std::fs::File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (name, bytes) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
    }

    #[test]
    fn a_manifest_archive_lands_in_both_steam_folders() {
        let root = fake_steam("manifest");
        let archive = root.join("download.zip");
        zip_with(
            &archive,
            &[
                ("386940_18234567.lua", b"addappid(386940)\n"),
                ("386940_998877.manifest", b"MANIFEST"),
                ("readme.txt", b"ignored"),
            ],
        );

        let summary = install_archive(&archive.to_string_lossy(), &root.to_string_lossy(), false).unwrap();

        assert_eq!(summary.lua, 1);
        assert_eq!(summary.manifests, 1);
        assert!(root.join("config").join("stplug-in").join("386940.lua").is_file());
        assert!(root.join("depotcache").join("386940_998877.manifest").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn installing_a_lua_points_the_unlocker_at_the_folder() {
        let root = fake_steam("hotreload");
        let archive = root.join("download.zip");
        zip_with(&archive, &[("386940.lua", b"addappid(386940)\n")]);

        let summary = install_archive(&archive.to_string_lossy(), &root.to_string_lossy(), false).unwrap();

        assert!(summary.live, "a lua install has to register the path it wrote to");
        let toml = std::fs::read_to_string(root.join("opensteamtool.toml")).unwrap();
        assert!(toml.contains("[lua]"), "{toml}");
        assert!(toml.contains("config/stplug-in"), "{toml}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_existing_lua_path_is_not_duplicated() {
        let root = fake_steam("hotreload-once");
        std::fs::write(
            root.join("opensteamtool.toml"),
            "[lua]\npaths = [\"config/stplug-in\"]\n",
        )
        .unwrap();
        let archive = root.join("download.zip");
        zip_with(&archive, &[("730.lua", b"addappid(730)\n")]);

        let summary = install_archive(&archive.to_string_lossy(), &root.to_string_lossy(), false).unwrap();

        assert!(summary.live);
        let toml = std::fs::read_to_string(root.join("opensteamtool.toml")).unwrap();
        assert_eq!(toml.matches("config/stplug-in").count(), 1, "{toml}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn removing_a_game_deletes_only_its_own_lua_pair() {
        let root = fake_steam("remove");
        let dir = root.join("config").join("stplug-in");
        std::fs::write(dir.join("730.lua"), b"addappid(730)\n").unwrap();
        std::fs::write(dir.join("730.lua.disabled"), b"addappid(730)\n").unwrap();
        std::fs::write(dir.join("731.lua"), b"addappid(731)\n").unwrap();

        assert!(remove_installed(&root.to_string_lossy(), "730").unwrap().lua);
        assert!(!dir.join("730.lua").exists());
        assert!(!dir.join("730.lua.disabled").exists());
        assert!(dir.join("731.lua").is_file(), "another game's lua must survive");
        assert!(!remove_installed(&root.to_string_lossy(), "730").unwrap().lua);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn removing_a_game_clears_both_libraries() {
        let root = fake_steam("remove-both");
        let dir = root.join("config").join("stplug-in");
        std::fs::write(dir.join("2668510.lua"), b"addappid(2668510)\n").unwrap();
        assert!(crate::steam::register_in_library(
            &root.to_string_lossy(),
            "2668510",
            "Red Dead Redemption"
        ));
        let manifest = root.join("steamapps").join("appmanifest_2668510.acf");
        assert!(manifest.is_file());

        let summary = remove_installed(&root.to_string_lossy(), "2668510").unwrap();
        assert!(summary.lua, "the lua is what Steamware lists");
        assert!(summary.library, "the manifest is what Steam lists");
        assert!(!summary.still_installed);
        assert!(!manifest.exists(), "a stub entry must not outlive the lua");
        let _ = std::fs::remove_dir_all(&root);

        // A game Steam has downloaded keeps Steam's record, and says so instead of claiming success.
        let root = fake_steam("remove-downloaded");
        let dir = root.join("config").join("stplug-in");
        std::fs::write(dir.join("440.lua"), b"addappid(440)\n").unwrap();
        std::fs::write(
            root.join("steamapps").join("appmanifest_440.acf"),
            "\"AppState\"\n{\n \"appid\" \"440\"\n \"StateFlags\" \"4\"\n \"SizeOnDisk\" \"9000\"\n}\n",
        )
        .unwrap();

        let summary = remove_installed(&root.to_string_lossy(), "440").unwrap();
        assert!(summary.lua);
        assert!(!summary.library);
        assert!(summary.still_installed);
        assert!(root.join("steamapps").join("appmanifest_440.acf").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn removing_a_game_refuses_an_id_that_is_a_path() {
        let root = fake_steam("remove-guard");
        assert!(remove_installed(&root.to_string_lossy(), "..\\..\\x").is_err());
        assert!(remove_installed(&root.to_string_lossy(), "").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_entry_cannot_escape_the_steam_folders() {
        let root = fake_steam("traversal");
        let archive = root.join("hostile.zip");
        zip_with(
            &archive,
            &[
                ("../../escaped.lua", b"addappid(1)\n"),
                ("..\\..\\escaped2.manifest", b"X"),
            ],
        );

        let summary = install_archive(&archive.to_string_lossy(), &root.to_string_lossy(), false).unwrap();
        assert!(summary.total() > 0, "the files land, just not where they asked to");
        assert!(!root.join("escaped.lua").exists());
        assert!(!root.parent().unwrap().join("escaped.lua").exists());
        assert!(!root.parent().unwrap().join("escaped2.manifest").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn build_lua_is_renamed_to_the_app_id() {
        assert_eq!(lua_target_name("730.lua"), "730.lua");
        assert_eq!(lua_target_name("386940_18234567.lua"), "386940.lua");
        assert_eq!(lua_target_name("386940_18234567.LUA"), "386940.lua");
    }

    #[test]
    fn a_name_without_a_number_is_kept() {
        assert_eq!(lua_target_name("steam.lua"), "steam.lua");
        assert_eq!(lua_target_name("notes.txt.lua"), "notes.txt.lua");
    }

    #[test]
    fn auto_update_comments_every_active_manifest_pin() {
        let lua = "addappid(730)
  setManifestid(730, \"111\", 2048)
--setManifestid(731, \"222\")
";
        let out = unpin(lua);
        assert!(out.contains("addappid(730)"), "the depot key stays");
        assert!(out.contains("  -- setManifestid(730"), "indentation is preserved");
        assert!(out.contains("--setManifestid(731"), "an already-commented pin is left as it was");
        assert!(!out.contains("
  setManifestid"), "{out}");
    }

    #[test]
    fn auto_update_leaves_a_pinned_install_untouched_when_it_is_off() {
        let root = fake_steam("pinned");
        let archive = root.join("download.zip");
        let lua = b"addappid(386940)
setManifestid(386940, \"111\")
";
        zip_with(&archive, &[("386940.lua", lua)]);

        let summary = install_archive(&archive.to_string_lossy(), &root.to_string_lossy(), false).unwrap();
        assert_eq!(summary.lua, 1);
        let installed = std::fs::read_to_string(root.join("config").join("stplug-in").join("386940.lua")).unwrap();
        assert!(installed.contains("setManifestid(386940"), "{installed}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn auto_update_installs_a_lua_with_its_pins_commented_out() {
        let root = fake_steam("unpinned");
        let archive = root.join("download.zip");
        let lua = b"addappid(386940)
setManifestid(386940, \"111\")
";
        zip_with(&archive, &[("386940.lua", lua)]);

        let summary = install_archive(&archive.to_string_lossy(), &root.to_string_lossy(), true).unwrap();
        assert_eq!(summary.lua, 1);
        let installed = std::fs::read_to_string(root.join("config").join("stplug-in").join("386940.lua")).unwrap();
        assert!(installed.contains("-- setManifestid(386940"), "{installed}");
        assert!(installed.contains("addappid(386940)"), "{installed}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
