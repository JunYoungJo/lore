use crate::state::AppState;
use lore_core::error::{Error, Result};
use lore_core::qmd::Qmd;
use lore_core::queue::{Queue, Status};
use lore_core::recorder::CliRecorder;
use lore_core::review::{self, DiffRow};
use lore_core::run;
use lore_core::settings::{self, Paths, Settings};
use lore_core::sync::{self, Item, Outcome, Summary};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_notification::NotificationExt;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    root: String,
    enabled: bool,
    busy: bool,
    summary: Summary,
    tracking: Option<String>,
    model: String,
    review: Option<usize>,
    items: Vec<Item>,
    comparison: Option<sync::ComparisonProgress>,
    comparison_error: Option<String>,
    index_pending: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemFile {
    path: String,
    base: Option<String>,
    local: Option<String>,
    remote: Option<String>,
    merged: Option<String>,
    /// This PC's version before the merge against the merged note.
    rows: Vec<DiffRow>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemView {
    item: Item,
    files: Vec<ItemFile>,
    issue: Option<sync::Issue>,
    sources: Vec<sync::Source>,
}

fn model_label(settings: &Settings) -> String {
    let (name, chosen) = if settings.backend == "claude" { ("Claude", &settings.models.claude) } else { ("Codex", &settings.models.codex) };
    chosen.as_deref().map_or_else(|| name.to_string(), |model| format!("{name} {model}"))
}

fn changed(app: &AppHandle) {
    crate::tray::refresh_tooltip(app);
    let _ = app.emit("state-changed", ());
}

fn folder(state: &AppState) -> PathBuf {
    sync::folder(&state.paths, &state.settings().knowledge_root())
}

fn notify(app: &AppHandle, title: &str, body: &str) {
    let _ = app.notification().builder().title(title).body(body).show();
}

#[tauri::command]
pub async fn open_sync(app: AppHandle) -> std::result::Result<(), String> {
    if let Some(window) = app.get_webview_window("sync") {
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }
    let window = WebviewWindowBuilder::new(&app, "sync", WebviewUrl::App("index.html".into()))
        .title("Memory sync")
        .inner_size(1120.0, 820.0)
        .min_inner_size(740.0, 620.0)
        .decorations(false)
        .center()
        .build()
        .map_err(|e| e.to_string())?;
    crate::webview::harden(&window);
    Ok(())
}

#[tauri::command]
pub fn get_sync(state: State<'_, AppState>) -> std::result::Result<Snapshot, String> {
    let settings = state.settings();
    let root = settings.knowledge_root();
    let folder = sync::folder(&state.paths, &root);
    let stored = sync::state(&folder);
    let waiting = stored.comparisons.iter().find(|comparison| comparison.error.is_none());
    Ok(Snapshot {
        root: root.to_string_lossy().into(),
        enabled: settings.sync_enabled,
        busy: state.syncing(),
        summary: sync::summary(&folder),
        tracking: sync::tracking(&root),
        model: model_label(&settings),
        review: review::working_changes(&root).ok().flatten().map(|changes| changes.notes.len()),
        comparison: waiting.and_then(|comparison| sync::comparison_progress(&folder, &comparison.plan)),
        comparison_error: stored.comparisons.iter().find_map(|comparison| comparison.error.clone()),
        index_pending: stored.index_pending,
        items: stored.items,
    })
}

#[tauri::command]
pub fn get_sync_item(state: State<'_, AppState>, id: String) -> std::result::Result<ItemView, String> {
    let detail = sync::detail(&folder(&state), &id).map_err(|e| e.to_string())?;
    let files = detail
        .files
        .into_iter()
        .map(|file| ItemFile {
            rows: review::side_by_side(file.local.as_deref().unwrap_or(""), file.merged.as_deref().unwrap_or("")),
            path: file.path,
            base: file.base,
            local: file.local,
            remote: file.remote,
            merged: file.merged,
        })
        .collect();
    Ok(ItemView { item: detail.item, files, issue: detail.issue, sources: detail.sources })
}

#[tauri::command]
pub fn preview_sync_item(state: State<'_, AppState>, id: String, option: usize, custom_text: Option<String>) -> std::result::Result<BTreeMap<String, Vec<DiffRow>>, String> {
    let root = state.settings().knowledge_root();
    let changes = sync::preview_item(&root, &folder(&state), &id, option, custom_text.as_deref()).map_err(|e| e.to_string())?;
    Ok(changes.into_iter().map(|(path, (before, after))| (path, review::side_by_side(&before, &after))).collect())
}

#[tauri::command]
pub fn set_sync_enabled(app: AppHandle, enabled: bool) -> std::result::Result<(), String> {
    let state = app.state::<AppState>();
    let mut settings = state.settings();
    settings.sync_enabled = enabled;
    settings::save(&state.paths.settings(), &settings).map_err(|e| e.to_string())?;
    state.replace_settings(settings);
    state.wake();
    changed(&app);
    Ok(())
}

#[tauri::command]
pub fn defer_sync(app: AppHandle) -> std::result::Result<(), String> {
    if let Some(window) = app.get_webview_window("sync") {
        window.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Why a sync did not run. These are not failures: the next cycle tries again.
fn unavailable(state: &AppState) -> Result<Option<&'static str>> {
    if state.setup_busy() {
        return Ok(Some("Wait for setup to finish before syncing."));
    }
    if state.reviewing() {
        return Ok(Some("Close the memory review window before syncing."));
    }
    let queue = Queue::open(&state.paths.database())?;
    if !queue.jobs_with_status(&[Status::Applying], 1)?.is_empty() {
        return Ok(Some("A recording was interrupted while writing notes. Retry that job, then sync."));
    }
    Ok(None)
}

/// Run one operation that changes the memory folder, exclusive with recording.
/// `Ok(None)` means it could not start now; the reason is in the second value.
fn run_exclusive<T>(app: &AppHandle, operation: impl FnOnce(&Settings, &Paths) -> Result<T>) -> Result<std::result::Result<T, &'static str>> {
    let state = app.state::<AppState>();
    let Some(busy) = state.begin_sync() else { return Ok(Err("Another sync is in progress.")) };
    changed(app);
    let result = {
        let _operation = state.operation();
        let settings = state.settings();
        settings.validate().and_then(|_| match unavailable(&state)? {
            Some(reason) => Ok(Err(reason)),
            None => operation(&settings, &state.paths).map(Ok),
        })
    };
    drop(busy);
    changed(app);
    result
}

fn exchange(settings: &Settings, paths: &Paths) -> Result<Outcome> {
    let root = settings.knowledge_root();
    sync::sync(&root, &sync::folder(paths, &root), || {
        let qmd = Qmd::new(settings, &paths.recorder_workdir(), run::fixed(settings.qmd_timeout_secs));
        if !qmd.available() {
            return Err(Error::transient("qmd was not found. Run doctor to check its installation and executable path."));
        }
        qmd.ensure_index(&root)?;
        qmd.refresh()
    })
}

fn after_sync(app: &AppHandle, outcome: &Outcome) {
    if outcome.items > 0 {
        notify(app, "Memory synced", &format!("{} to check when you have time. Nothing is waiting on it.", if outcome.items == 1 { "1 note".to_string() } else { format!("{} notes", outcome.items) }));
    }
    resume_comparisons(app);
}

#[tauri::command]
pub async fn sync_now(app: AppHandle) -> std::result::Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || match run_exclusive(&app, exchange) {
        Ok(Ok(outcome)) => {
            after_sync(&app, &outcome);
            Ok(())
        }
        Ok(Err(reason)) => Err(reason.to_string()),
        Err(error) => Err(error.to_string()),
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Background sync. A failure is retried on the next cycle and only reported
/// once it has failed several times in a row.
pub fn automatic(app: &AppHandle) {
    match run_exclusive(app, exchange) {
        Ok(Ok(outcome)) => after_sync(app, &outcome),
        Ok(Err(_)) => {}
        Err(error) => {
            let summary = sync::summary(&folder(&app.state::<AppState>()));
            if summary.phase == "failing" && sync::state(&folder(&app.state::<AppState>())).failures == sync::FAILURES_SHOWN {
                notify(app, "Memory sync needs attention", &error.to_string());
            }
        }
    }
    changed(app);
}

#[tauri::command]
pub async fn resolve_sync_item(app: AppHandle, id: String, option: usize, custom_text: Option<String>) -> std::result::Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let resolved = run_exclusive(&app, |settings, paths| {
            let root = settings.knowledge_root();
            sync::resolve_item(&root, &sync::folder(paths, &root), &id, option, custom_text.as_deref())
        });
        match resolved {
            Ok(Ok(())) => {
                // Share the decision right away when sync is on; otherwise the next manual sync sends it.
                if app.state::<AppState>().settings().sync_enabled {
                    automatic(&app);
                }
                Ok(())
            }
            Ok(Err(reason)) => Err(reason.to_string()),
            Err(error) => Err(error.to_string()),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn dismiss_sync_item(app: AppHandle, id: String) -> std::result::Result<(), String> {
    sync::dismiss_item(&folder(&app.state::<AppState>()), &id).map_err(|e| e.to_string())?;
    changed(&app);
    Ok(())
}

#[tauri::command]
pub fn retry_sync_comparison(app: AppHandle) -> std::result::Result<(), String> {
    sync::retry_comparison(&folder(&app.state::<AppState>())).map_err(|e| e.to_string())?;
    resume_comparisons(&app);
    changed(&app);
    Ok(())
}

#[tauri::command]
pub fn skip_sync_comparison(app: AppHandle) -> std::result::Result<(), String> {
    sync::skip_comparisons(&folder(&app.state::<AppState>())).map_err(|e| e.to_string())?;
    changed(&app);
    Ok(())
}

static COMPARING: AtomicBool = AtomicBool::new(false);

/// Compare the notes both PCs changed, in the background. It reads only the
/// versions saved at merge time, so recording and sync continue meanwhile.
pub fn resume_comparisons(app: &AppHandle) {
    if sync::pending_comparison(&folder(&app.state::<AppState>())).is_none() || COMPARING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        loop {
            let settings = state.settings();
            let folder = sync::folder(&state.paths, &settings.knowledge_root());
            let Some(id) = sync::pending_comparison(&folder) else { break };
            let recorder = CliRecorder::from_settings(&settings, &state.paths.recorder_workdir(), run::fixed(settings.model_timeout_secs)).defaulting_to(state.catalog().default_model(&settings.backend));
            let identity = serde_json::json!([recorder.backend, recorder.model, recorder.effort, recorder.command]).to_string();
            changed(&app);
            let found = sync::compare(&folder, &id, &identity, &mut |prompt, schema, artifacts| recorder.request_json(prompt, schema, artifacts), &mut || {
                if let Some(progress) = sync::comparison_progress(&folder, &id) {
                    let _ = app.emit("sync-comparison-progress", progress);
                }
            });
            if let Ok(found) = found {
                if found > 0 {
                    notify(&app, "Memory sync found notes that disagree", &format!("{} to decide when you have time. Open Sync to see them.", if found == 1 { "1 item".to_string() } else { format!("{found} items") }));
                }
            }
            changed(&app);
        }
        COMPARING.store(false, Ordering::SeqCst);
        changed(&app);
    });
}
