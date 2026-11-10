use qmd_memory_core::settings::{self, Paths};
use qmd_memory_core::worker;
use tauri_plugin_notification::NotificationExt;

mod background;
mod commands;
mod review;
mod state;
mod sync;
mod tray;
mod watch;

pub fn run() {
    let paths = Paths::standard();
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
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .manage(app_state)
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let handle = app.handle().clone();
            tray::setup_tray(&handle)?;
            tray::create_popover(&handle);
            let opener = handle.clone();
            let _ = handle.notification().on_action(move |_| tray::open_popover(&opener));
            watch::spawn(handle.clone());
            background::spawn(handle);
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
            commands::connect_hook,
            commands::disconnect_hook,
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
            sync::defer_sync,
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
