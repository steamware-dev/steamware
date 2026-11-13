mod api;
mod builds;
mod commands;
mod downloads;
mod error;
mod history;
mod injector;
mod local_api;
mod lua;
mod media;
mod modes;
mod plugin;
mod presence;
mod rebrand;
mod secrets;
mod settings;
mod skin;
mod startup;
mod steam;
mod update;

use commands::AppState;
use tauri::Manager;

pub fn run() {
    let settings = settings::Settings::load();
    // Reconciled before the window opens, not just when saved: an app that was reinstalled to another folder
    // leaves a stale Run entry behind, and the switch in Settings should describe what Windows will do.
    let _ = startup::apply(settings.start_with_windows);

    tauri::Builder::default()
        // First, so a second launch is refused before it can open a window or start anything. Two copies of
        // this app are two injectors into Steam, two ticks announcing on Discord and two listeners competing
        // for the bridge port — a double-clicked shortcut must raise the window that already exists instead.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new(settings))
        .setup(|app| {
            // The in-Steam plugin's backend, and the injector that puts it on the store page. Both are
            // Windows-only: macOS and Linux remove any debug marker an older build left instead of opting
            // Steam into remote debugging. When Steam is closed the injector's polls fail fast and the API
            // simply waits.
            let state = app.state::<AppState>();
            // The CEF marker is dropped on quit (see the exit hook below), so put it back when the plugin is
            // installed: while this app runs is the only time the injected bundle can reach the bridge.
            {
                let steam_path = state
                    .settings
                    .lock()
                    .map(|settings| settings.steam_path.clone())
                    .unwrap_or_default();
                plugin::ensure_marker_if_installed(&steam_path);
            }
            // Steam keeps one library record per app, and a stub this app wrote can outlive Steam's own
            // record for a game it has since installed elsewhere — two records, one of them empty, is how
            // an installed game comes back as "Install". Repair what is on disk before Steam draws the
            // library, and quiet any older stub that would start downloading by itself.
            {
                let steam_path = state
                    .settings
                    .lock()
                    .map(|settings| settings.steam_path.clone())
                    .unwrap_or_default();
                steam::reconcile_library(&steam_path);
            }
            let token = local_api::bridge_token();
            local_api::LocalApi::start(
                app.handle().clone(),
                state.api.clone(),
                state.downloads.clone(),
                state.settings.clone(),
                token.clone(),
            );
            injector::start(token, state.settings.clone());
            // Discord Rich Presence follows the setting on its own tick, so this is started once and never
            // managed again: turning it off in Settings clears the card within a second.
            presence::start(state.settings.clone(), state.downloads.clone(), state.presence.clone());
            // The update gate keeps its own clock. The first read happens now, so a gate can be up before the
            // user has clicked anything, and every later read is throttled by the module itself.
            {
                let updater = state.updater.clone();
                tauri::async_runtime::spawn(async move {
                    loop {
                        updater.refresh(false).await;
                        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                    }
                });
            }
            // The notification area is how a hidden window is got back. Best effort: a shell that refuses a
            // tray icon should not stop the app from running.
            let _ = tray::build(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap,
            commands::save_settings,
            commands::steam_refresh,
            commands::library_list,
            commands::library_owned,
            commands::library_remove,
            commands::steam_launch,
            commands::steam_restart,
            commands::auth_begin,
            commands::auth_sign_out,
            commands::auth_status,
            commands::server_health,
            commands::server_config,
            commands::update_status,
            commands::update_check,
            commands::update_install,
            commands::catalog_stats,
            commands::store_search,
            commands::store_featured,
            commands::store_genre,
            commands::store_app,
            commands::store_cover,
            commands::store_stats,
            commands::fixes_catalog,
            commands::manifest_availability,
            commands::steam_app,
            commands::builds_list,
            commands::builds_state,
            commands::builds_set_entry,
            commands::builds_save_text,
            commands::builds_save_variant,
            commands::builds_apply_variant,
            commands::builds_rename_variant,
            commands::builds_delete_variant,
            commands::modes_state,
            commands::modes_install,
            commands::modes_cloud_redirect,
            commands::plugin_state,
            commands::plugin_install,
            commands::plugin_uninstall,
            commands::start_manifest_download,
            commands::start_fix_download,
            commands::downloads_list,
            commands::download_cancel,
            commands::download_history,
            commands::download_history_clear,
            commands::download_history_remove,
            commands::open_media,
            commands::open_external,
            commands::reveal_path,
            commands::install_lua_from_zip,
            commands::install_files,
            commands::pick_archive,
            commands::pick_files,
            commands::pick_folder,
            commands::window_minimize,
            commands::window_toggle_maximize,
            commands::window_close,
            commands::set_presence_page,
            commands::hubcap_check,
            commands::discord_check,
            commands::startup_status,
        ])
        .build(tauri::generate_context!())
        .expect("Steamware failed to start")
        // Nothing is torn down on exit on purpose. The CEF marker has to be in Steam's root *before* Steam
        // boots, because that is the only moment Steam decides whether to open its debugging port. Removing
        // it on quit meant a Steam that started before Steamware — the normal case, since Steam autostarts
        // with Windows — came up with no port, and the store-page plugin stayed invisible until the user
        // restarted Steam by hand. The marker is removed by the uninstall path instead, which is where it
        // belongs: the port exists to serve this plugin, and the plugin is no longer there.
        .run(|_app, _event| {});
}

/// The notification-area icon and its two commands.
///
/// It exists whether or not "Minimize to System Tray" is on: with the setting off, closing the window quits
/// the app and the icon goes with it, which is exactly what that setting asks for. With it on, this is the
/// only way back to the window, so a double-click and a menu item both restore it.
mod tray {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
    use tauri::{AppHandle, Manager};

    pub fn build(app: &AppHandle) -> tauri::Result<()> {
        let open = MenuItem::with_id(app, "open", "Open Steamware", true, None::<&str>)?;
        let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
        let menu = Menu::with_items(app, &[&open, &quit])?;

        let mut builder = TrayIconBuilder::with_id("steamware")
            .tooltip("Steamware")
            .menu(&menu)
            .show_menu_on_left_click(false)
            .on_menu_event(|app, event| match event.id.as_ref() {
                "open" => reveal(app),
                "quit" => app.exit(0),
                _ => {}
            })
            .on_tray_icon_event(|icon, event| {
                if matches!(event, TrayIconEvent::DoubleClick { button: MouseButton::Left, .. }) {
                    reveal(icon.app_handle());
                }
            });

        // The window icon doubles as the tray icon, so the two can never drift apart.
        if let Some(icon) = app.default_window_icon().cloned() {
            builder = builder.icon(icon);
        }

        builder.build(app)?;
        Ok(())
    }

    fn reveal(app: &AppHandle) {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
    }
}
