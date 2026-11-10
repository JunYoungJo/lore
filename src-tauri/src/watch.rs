use std::time::Duration;

use lore_core::settings;
use tauri::{AppHandle, Emitter, Manager};

use crate::state::AppState;
use crate::tray;

const CHECK_EVERY: Duration = Duration::from_secs(2);

pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let path = state.paths.settings();
        let mut seen = std::fs::read_to_string(&path).ok();
        let mut reported: Option<String> = None;
        loop {
            std::thread::sleep(CHECK_EVERY);
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            if seen.as_deref() == Some(text.as_str()) {
                continue;
            }
            seen = Some(text.clone());
            let running = state.settings();
            match settings::reloaded(&text, &running) {
                Ok(loaded) => {
                    if reported.take().is_some_and(|message| state.problem().as_deref() == Some(message.as_str())) {
                        state.set_problem(None);
                    }
                    if loaded != running {
                        let paused = loaded.paused;
                        state.replace_settings(loaded);
                        tray::show_paused(&app, paused);
                    }
                }
                Err(error) => {
                    let message = error.to_string();
                    state.set_problem(Some(message.clone()));
                    reported = Some(message);
                }
            }
            tray::refresh_tooltip(&app);
            let _ = app.emit("state-changed", ());
        }
    });
}
