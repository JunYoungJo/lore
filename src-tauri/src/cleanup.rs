//! The cleanup window: which folders of notes have grown since their last cleanup, the run that
//! cleans them one after another, and undoing a cleanup.

use lore_core::cleanup::{self, Folder, Target};
use lore_core::qmd::Qmd;
use lore_core::queue::Queue;
use lore_core::run::fixed;
use lore_core::{review, settings, worker};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};

use crate::state::{AppState, CleanupRun, Due, ItemState};

pub const WINDOW: &str = "cleanup";
/// A folder whose cleanup failed is not started again by itself for a day. Starting it by hand
/// is always possible.
const RETRY_AFTER: i64 = 24 * 3600;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub repository: bool,
    pub folders: Vec<Folder>,
    pub run: Option<CleanupRun>,
    /// Why a run that waits has not started yet.
    pub blocked: Option<&'static str>,
    pub auto: bool,
    pub due_notes: usize,
}

fn error(value: impl std::fmt::Display) -> String {
    value.to_string()
}

fn changed(app: &AppHandle) {
    crate::tray::refresh_tooltip(app);
    let _ = app.emit("state-changed", ());
}

/// Read every folder's cleanup state and remember which are due, for the tray window.
pub fn survey(state: &AppState) -> Result<Vec<Folder>, String> {
    let checks = Queue::open(&state.paths.database()).and_then(|queue| queue.cleanup_checks()).map_err(error)?;
    let folders = cleanup::folders(&state.settings().knowledge_root(), &checks).map_err(error)?;
    state.set_due(folders.iter().filter(|folder| folder.due).map(|folder| Due { name: folder.name.clone(), fresh: folder.fresh }).collect());
    Ok(folders)
}

/// The same off the main thread, for moments when nothing waits for the answer.
pub fn refresh_due(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        if survey(&app.state::<AppState>()).is_ok() {
            let _ = app.emit("state-changed", ());
        }
    });
}

/// The folders an automatic run cleans: the due ones, except those that failed recently.
pub fn automatic(folders: &[Folder], now: i64) -> Vec<Target> {
    let ready = folders.iter().filter(|folder| folder.due && folder.failed_at.is_none_or(|at| now - at >= RETRY_AFTER));
    cleanup::in_order(ready.map(|folder| Target { folder: folder.name.clone(), absorb: Vec::new() }).collect())
}

fn blocked(state: &AppState, run: Option<&CleanupRun>) -> Option<&'static str> {
    if run?.items.iter().any(|item| item.state == ItemState::Running) {
        return None;
    }
    if state.setup_busy() {
        Some("Starts when setup is done.")
    } else if state.settings().paused {
        Some("Starts when recording is resumed.")
    } else if state.reviewing() {
        Some("Starts when the review window is closed.")
    } else if state.syncing() {
        Some("Starts when the sync is done.")
    } else if state.current().is_some() {
        Some("Starts when the current recording is done.")
    } else {
        None
    }
}

// Must be async: building a webview from a synchronous command deadlocks on Windows (see `open_review`).
#[tauri::command]
pub async fn open_cleanup(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(WINDOW) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }
    let window = WebviewWindowBuilder::new(&app, WINDOW, WebviewUrl::App("index.html".into()))
        .title("Clean up notes")
        .inner_size(1040.0, 720.0)
        .min_inner_size(760.0, 520.0)
        .center()
        .build()
        .map_err(error)?;
    crate::webview::harden(&window);
    Ok(())
}

// Async and off the main thread: it reads every note and the whole commit history.
#[tauri::command]
pub async fn get_cleanup(app: AppHandle) -> Result<Snapshot, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let folders = survey(&state)?;
        let run = state.cleanup();
        let settings = state.settings();
        Ok(Snapshot { repository: review::is_repository(&settings.knowledge_root()), blocked: blocked(&state, run.as_ref()), folders, run, auto: settings.auto_cleanup, due_notes: cleanup::DUE_NOTES })
    })
    .await
    .map_err(error)?
}

/// Hold a run over these folders for the worker, which cleans them one after another between
/// recordings under the same gate.
#[tauri::command]
pub fn start_cleanup(app: AppHandle, state: State<'_, AppState>, targets: Vec<Target>) -> Result<(), String> {
    for target in &targets {
        target.check().map_err(error)?;
        if target.absorb.iter().any(|folder| targets.iter().any(|other| &other.folder == folder)) {
            return Err("A folder that is merged into another one cannot be cleaned up in the same run.".into());
        }
    }
    if targets.is_empty() {
        return Err("Choose at least one folder.".into());
    }
    if !state.request_cleanup(cleanup::in_order(targets), worker::now()) {
        return Err("A cleanup is already waiting or running.".into());
    }
    changed(&app);
    Ok(())
}

#[tauri::command]
pub fn stop_cleanup(app: AppHandle, state: State<'_, AppState>) {
    state.stop_cleanup();
    changed(&app);
}

/// Undo the last cleanup of a folder. Returns a note when the notes are back but the search
/// index could not be refreshed.
#[tauri::command]
pub async fn undo_cleanup(app: AppHandle, folder: String) -> Result<Option<String>, String> {
    let handle = app.clone();
    let note = tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<AppState>();
        let Some(_operation) = state.try_operation() else {
            return Err("Recording, cleaning up or syncing is in progress. Try again when it is done.".to_string());
        };
        let settings = state.settings();
        let queue = Queue::open(&state.paths.database()).map_err(error)?;
        let qmd = Qmd::new(&settings, &state.paths.recorder_workdir(), fixed(settings.qmd_timeout_secs));
        let note = worker::undo_cleanup(&queue, &settings, &qmd, &folder).map_err(error)?;
        let _ = survey(&state);
        Ok(note)
    })
    .await
    .map_err(error)??;
    // The notes that came back are shared like any other change.
    app.state::<AppState>().wake();
    changed(&app);
    Ok(note)
}

#[tauri::command]
pub fn set_auto_cleanup(app: AppHandle, state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    let mut edited = state.settings();
    edited.auto_cleanup = enabled;
    settings::save(&state.paths.settings(), &edited).map_err(error)?;
    state.replace_settings(edited);
    changed(&app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str, fresh: usize, failed_at: Option<i64>) -> Folder {
        Folder { name: name.into(), notes: fresh, fresh, due: fresh >= cleanup::DUE_NOTES, last: None, checked_at: None, failed_at, failure: None }
    }

    #[test]
    fn an_automatic_run_takes_the_due_folders_and_leaves_a_recent_failure_alone() {
        let now = 1_000_000;
        let folders = [folder("common", 12, None), folder("small", 3, None), folder("zeta", 10, None), folder("broken", 40, Some(now - 3600)), folder("alpha", 25, Some(now - RETRY_AFTER))];
        let names: Vec<String> = automatic(&folders, now).into_iter().map(|target| target.folder).collect();
        assert_eq!(names, vec!["alpha", "zeta", "common"]);
        assert!(automatic(&[folder("small", 3, None)], now).is_empty());
    }
}
