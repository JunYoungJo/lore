use lore_core::settings::{self, Paths};
use lore_core::worker;
use tauri_plugin_notification::NotificationExt;
use tauri::Manager;

mod background;
mod commands;
mod review;
mod state;
mod sync;
mod tray;
mod watch;
mod setup;
mod setup_terminal;
mod updates;

pub fn run() {
    let paths = Paths::standard();
    let first_launch = !paths.settings().exists();
    let loaded = settings::load_or_init(&paths.settings(), worker::now());
    let (initial, problem) = match loaded {
        Ok(settings) => (settings, None),
        Err(error) => (settings::Settings::default(), Some(format!("Could not read the settings file: {error}"))),
    };
    let app_state = state::AppState::new(paths, initial);
    app_state.set_problem(problem);

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| tray::open_popover(app)))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(app_state)
        .manage(updates::Updates::default())
        .manage(setup_terminal::Terminal::default())
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let handle = app.handle().clone();
            tray::setup_tray(&handle)?;
            tray::create_popover(&handle);
            let progress = lore_core::setup::progress(&handle.state::<state::AppState>().paths).unwrap_or_default();
            let show_setup = first_launch || (!progress.completed && !progress.deferred && lore_core::setup::progress_path(&handle.state::<state::AppState>().paths).exists());
            if first_launch { lore_core::setup::save_progress(&handle.state::<state::AppState>().paths, &progress)?; }
            setup::create_window(&handle, show_setup)?;
            let opener = handle.clone();
            let _ = handle.notification().on_action(move |_| tray::open_popover(&opener));
            watch::spawn(handle.clone());
            commands::refresh_catalog(&handle);
            background::spawn(handle.clone());
            updates::check_on_startup(handle);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::get_queue,
            commands::set_paused,
            commands::retry_job,
            commands::discard_job,
            commands::clear_activity,
            commands::save_settings,
            commands::set_autostart,
            commands::open_note,
            commands::open_folder,
            commands::hide_window,
            commands::open_settings,
            updates::check_app_update,
            updates::install_app_update,
            review::open_review,
            review::begin_review,
            review::end_review,
            review::get_review,
            review::review_diff,
            review::commit_review,
            sync::open_sync,
            sync::get_sync,
            sync::set_sync_enabled,
            sync::prepare_sync,
            sync::analyze_sync,
            sync::preview_sync,
            sync::apply_sync,
            sync::resume_sync,
            sync::sync_now,
            sync::defer_sync,
            setup::open_setup,
            setup::open_integrations,
            setup::setup_check_tools,
            setup::setup_pick_folder,
            setup::get_setup,
            setup::save_setup_progress,
            setup::setup_detect,
            setup::setup_begin_test,
            setup::setup_repository,
            setup::setup_connect,
            setup::setup_disconnect,
            setup::setup_index,
            setup::setup_embed_model,
            setup::setup_prepare,
            setup::setup_run,
            setup::setup_open_docs,
            setup_terminal::setup_terminal_read,
            setup_terminal::setup_terminal_input,
            setup_terminal::setup_terminal_resize,
            setup_terminal::setup_terminal_stop,
        ])
        .build(tauri::generate_context!())
        .expect("Could not start Lore")
        .run(|_app, event| {
            // Tray-only app: closing the popover must not end the process; tray "Quit" passes an exit code.
            if let tauri::RunEvent::ExitRequested { code, api, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
