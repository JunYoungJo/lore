use qmd_memory_core::settings::{self, Paths};
use qmd_memory_core::worker;
use tauri_plugin_notification::NotificationExt;

mod background;
mod commands;
mod state;
mod tray;

pub fn run() {
    let paths = Paths::standard();
    let loaded = settings::load_or_init(&paths.settings(), worker::now());
    let (initial, problem) = match loaded {
        Ok(settings) => (settings, None),
        Err(error) => (settings::Settings::default(), Some(format!("설정 파일을 읽지 못했습니다: {error}"))),
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
            let handle = app.handle().clone();
            tray::setup_tray(&handle)?;
            tray::create_popover(&handle);
            let opener = handle.clone();
            let _ = handle.notification().on_action(move |_| tray::open_popover(&opener));
            background::spawn(handle);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::set_paused,
            commands::retry_job,
            commands::save_settings,
            commands::set_autostart,
            commands::open_note,
            commands::open_folder,
            commands::hide_window,
            commands::connect_hook,
            commands::disconnect_hook,
        ])
        .build(tauri::generate_context!())
        .expect("qmd-memory를 시작하지 못했습니다")
        .run(|_app, event| {
            // Tray-only app: closing the popover must not end the process; tray "종료" passes an exit code.
            if let tauri::RunEvent::ExitRequested { code, api, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
