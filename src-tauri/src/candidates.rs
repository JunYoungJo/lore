use lore_core::candidate::{Batch, Candidate, Scope};
use lore_core::queue::Queue;
use lore_core::settings;
use lore_core::worker;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};

use crate::state::AppState;

pub const WINDOW: &str = "candidates";
/// How many applied batches the window keeps in view.
const RESULTS_SHOWN: usize = 5;

/// What the candidates window shows: what waits for the user, and what became of what they approved.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub waiting: Vec<Candidate>,
    /// What was left out of those sessions because a note already says it.
    pub left_out: Vec<Candidate>,
    /// Approved batches that are being written or wait their turn.
    pub applying: Vec<Batch>,
    pub failed: Vec<Batch>,
    /// The batches written most recently, newest first.
    pub applied: Vec<Batch>,
    pub auto_approve: bool,
    pub auto_approve_days: u64,
    /// When automatic approval was turned on. Its days count from the later of this and the
    /// time a candidate entered the list.
    pub auto_approve_since: i64,
    pub now: i64,
}

fn error(value: impl std::fmt::Display) -> String {
    value.to_string()
}

fn changed(app: &AppHandle) {
    crate::tray::refresh_tooltip(app);
    let _ = app.emit("state-changed", ());
}

fn queue(state: &AppState) -> Result<Queue, String> {
    Queue::open(&state.paths.database()).map_err(error)
}

// Must be async: building a webview from a synchronous command deadlocks on Windows (see `open_review`).
#[tauri::command]
pub async fn open_candidates(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(WINDOW) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }
    let window = WebviewWindowBuilder::new(&app, WINDOW, WebviewUrl::App("index.html".into()))
        .title("Candidates")
        .inner_size(1120.0, 760.0)
        .min_inner_size(820.0, 520.0)
        .center()
        .build()
        .map_err(error)?;
    crate::webview::harden(&window);
    Ok(())
}

#[tauri::command]
pub fn get_candidates(state: State<'_, AppState>) -> Result<Snapshot, String> {
    let queue = queue(&state)?;
    let settings = state.settings();
    Ok(Snapshot {
        waiting: queue.waiting_candidates().map_err(error)?,
        left_out: queue.left_out().map_err(error)?,
        applying: queue.open_batches().map_err(error)?,
        failed: queue.failed_batches().map_err(error)?,
        applied: queue.applied_batches(RESULTS_SHOWN).map_err(error)?,
        auto_approve: settings.auto_approve,
        auto_approve_days: settings.auto_approve_days,
        auto_approve_since: settings.auto_approve_since,
        now: worker::now(),
    })
}

/// Change the sentence or the scope of a candidate that still waits.
#[tauri::command]
pub fn edit_candidate(app: AppHandle, state: State<'_, AppState>, id: i64, text: Option<String>, scope: Option<Scope>) -> Result<bool, String> {
    let edited = queue(&state)?.edit_candidate(id, text.as_deref(), scope).map_err(error)?;
    if edited {
        changed(&app);
    }
    Ok(edited)
}

/// Approve candidates. The worker writes them into the notes, one project at a time.
#[tauri::command]
pub fn approve_candidates(app: AppHandle, state: State<'_, AppState>, ids: Vec<i64>) -> Result<usize, String> {
    let batches = queue(&state)?.approve_candidates(&ids, false, worker::now()).map_err(error)?;
    state.wake();
    changed(&app);
    Ok(batches.len())
}

#[tauri::command]
pub fn discard_candidates(app: AppHandle, state: State<'_, AppState>, ids: Vec<i64>) -> Result<usize, String> {
    let discarded = queue(&state)?.discard_candidates(&ids, worker::now()).map_err(error)?;
    changed(&app);
    Ok(discarded)
}

/// Try a batch again that could not be written.
#[tauri::command]
pub fn retry_apply(app: AppHandle, state: State<'_, AppState>, id: i64) -> Result<bool, String> {
    let retried = queue(&state)?.retry_batch(id, worker::now()).map_err(error)?;
    state.wake();
    changed(&app);
    Ok(retried)
}

/// Give up a batch that could not be written: its candidates wait for the user again.
#[tauri::command]
pub fn release_apply(app: AppHandle, state: State<'_, AppState>, id: i64) -> Result<bool, String> {
    let released = queue(&state)?.release_batch(id, worker::now()).map_err(error)?;
    changed(&app);
    Ok(released)
}

#[tauri::command]
pub fn set_auto_approve(app: AppHandle, state: State<'_, AppState>, enabled: bool, days: u64) -> Result<(), String> {
    let mut edited = state.settings();
    edited.set_auto_approve(enabled, days, worker::now());
    edited.validate().map_err(error)?;
    settings::save(&state.paths.settings(), &edited).map_err(error)?;
    state.replace_settings(edited);
    changed(&app);
    Ok(())
}
