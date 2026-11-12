use lore_core::queue::Queue;
use lore_core::review::{self, ChangeKind, DiffRow};
use lore_core::transcript::Provider;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder, WindowEvent};

use crate::state::AppState;

pub const REVIEW: &str = "review";
const RECENT_KEPT: usize = 500;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSession {
    pub at: i64,
    pub provider: Provider,
    pub project: Option<String>,
    pub summary: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFile {
    pub path: String,
    pub kind: ChangeKind,
    pub sessions: Vec<ReviewSession>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSnapshot {
    pub repository: bool,
    pub waiting: bool,
    pub root: String,
    pub files: Vec<ReviewFile>,
}

fn error(value: impl std::fmt::Display) -> String {
    value.to_string()
}

fn changed(app: &AppHandle) {
    crate::tray::refresh_tooltip(app);
    let _ = app.emit("state-changed", ());
}

fn finish_review(app: &AppHandle) {
    app.state::<AppState>().set_reviewing(false);
    changed(app);
}

// Must be async: a synchronous command runs on the main thread, and building a webview there waits
// for that same thread's event loop, which deadlocks on Windows (wry#583) and leaves a white window.
#[tauri::command]
pub async fn open_review(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(REVIEW) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }
    let window = WebviewWindowBuilder::new(&app, REVIEW, WebviewUrl::App("index.html".into()))
        .title("Review memory changes")
        .inner_size(1120.0, 760.0)
        .min_inner_size(760.0, 520.0)
        .center()
        .build()
        .map_err(error)?;
    crate::webview::harden(&window);
    let handle = app.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::Destroyed = event {
            finish_review(&handle);
        }
    });
    Ok(())
}

#[tauri::command]
pub fn begin_review(app: AppHandle, state: State<'_, AppState>) {
    state.set_reviewing(true);
    changed(&app);
}

#[tauri::command]
pub fn end_review(app: AppHandle) {
    finish_review(&app);
}

#[tauri::command]
pub fn get_review(state: State<'_, AppState>) -> Result<ReviewSnapshot, String> {
    let root = state.settings().knowledge_root();
    let waiting = state.current().is_some() || state.operation_busy();
    let mut snapshot = ReviewSnapshot { repository: review::is_repository(&root), waiting, root: root.to_string_lossy().replace('\\', "/"), files: Vec::new() };
    if waiting || !snapshot.repository {
        return Ok(snapshot);
    }
    let changes = review::working_changes(&root).map_err(error)?.unwrap_or_default();
    let recent = Queue::open(&state.paths.database()).map_err(error)?.recent_activity(RECENT_KEPT).map_err(error)?;
    let since = review::since_commit(&recent, review::last_review_time(&root));
    snapshot.files = changes
        .notes
        .into_iter()
        .map(|change| ReviewFile {
            sessions: review::touching(&since, &change.path)
                .into_iter()
                .map(|activity| ReviewSession { at: activity.at, provider: activity.provider, project: activity.project.clone(), summary: activity.summary.clone() })
                .collect(),
            path: change.path,
            kind: change.kind,
        })
        .collect();
    Ok(snapshot)
}

#[tauri::command]
pub fn review_diff(state: State<'_, AppState>, path: String) -> Result<Vec<DiffRow>, String> {
    if !review::is_note(&path) || path.split('/').any(|part| part == ".." || part.is_empty()) {
        return Err("This is not a memory note path.".into());
    }
    review::file_diff(&state.settings().knowledge_root(), &path).map_err(error)
}

#[tauri::command]
pub async fn mark_reviewed(app: AppHandle, notes: Vec<String>) -> Result<usize, String> {
    let handle = app.clone();
    let marked = tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<AppState>();
        let _operation = state.operation();
        review::mark_reviewed(&state.settings().knowledge_root(), &notes).map_err(error)
    }).await.map_err(error)??;
    changed(&app);
    Ok(marked)
}
