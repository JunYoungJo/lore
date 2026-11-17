use std::path::{Path, PathBuf};

use lore_core::catalog::{self, Catalog};
use lore_core::candidate::Waiting;
use lore_core::hooks::{self, HookState, HookStatus};
use lore_core::queue::{Activity, Counts, Job, Queue, Status};
use lore_core::session;
use lore_core::recorder::{DEFAULT_CLAUDE_MODEL, DEFAULT_CODEX_EFFORT, DEFAULT_CODEX_MODEL};
use lore_core::review;
use lore_core::settings::{self, EditableSettings, Efforts, Models, Paths, Settings};
use lore_core::transcript::Provider;
use lore_core::{diagnostics, platform, setup, worker};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_opener::OpenerExt;

use crate::state::{AppState, CleanupRun, Current, Due, ItemState};
use crate::tray;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub state: &'static str,
    pub current: Option<Current>,
    pub cleanup: Option<CleanupRun>,
    /// The folders with enough changed notes to be worth a cleanup, as of the last look.
    pub cleanup_due: Vec<Due>,
    pub counts: Counts,
    pub recent: Vec<Activity>,
    pub attention: Vec<Job>,
    /// The candidates that wait for the user.
    pub candidates: Waiting,
    /// Approved batches that could not be written into the notes.
    pub failed_applies: usize,
    pub problem: Option<String>,
    pub settings: Settings,
    pub autostart: bool,
    pub hooks: Vec<HookStatus>,
    pub connections: Vec<Connection>,
    pub review: Option<usize>,
    pub reviewing: bool,
    pub version: &'static str,
    pub sync: lore_core::sync::Summary,
    pub syncing: bool,
    pub catalog: Catalog,
    pub default_models: Models,
    pub default_efforts: Efforts,
    pub cli_available: CliAvailable,
    pub update: crate::updates::Status,
}

/// Whether each recording CLI's command is found on this PC.
#[derive(Serialize)]
pub struct CliAvailable {
    pub codex: bool,
    pub claude: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub provider: Provider,
    pub state: &'static str,
}

fn error(value: impl std::fmt::Display) -> String {
    value.to_string()
}

pub(crate) fn cli_path() -> PathBuf {
    let name = lore_core::platform::CLI_FILENAME;
    let sibling = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|dir| dir.join(name))).unwrap_or_else(|| PathBuf::from(name));
    #[cfg(debug_assertions)]
    if !sibling.is_file() {
        let extension = if cfg!(windows) { ".exe" } else { "" };
        let built = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries").join(format!("lore-cli-{}{extension}", env!("TAURI_ENV_TARGET_TRIPLE")));
        if built.is_file() { return built; }
    }
    sibling
}

fn hook_statuses() -> Vec<HookStatus> {
    let cli = cli_path();
    [Provider::Claude, Provider::Codex].into_iter().map(|provider| hooks::status_at(&hooks::config_path(provider), provider, &cli)).collect()
}

fn connections(settings: &Settings, paths: &Paths, hooks: &[HookStatus]) -> Vec<Connection> {
    hooks
        .iter()
        .map(|hook| {
            let states: Option<Vec<String>> = setup::plan(&platform::home(), paths, hook.provider, settings).ok().map(|plan| plan.resources.into_iter().map(|resource| resource.state).collect());
            let ready = states.as_ref().is_some_and(|states| states.iter().all(|state| state == "ready" || state == "preserved"));
            let untouched = states.as_ref().map_or(true, |states| states.iter().all(|state| state == "add"));
            let state = match hook.state {
                HookState::Connected if ready => "connected",
                HookState::Missing if untouched => "missing",
                _ => "partial",
            };
            Connection { provider: hook.provider, state }
        })
        .collect()
}

#[tauri::command]
pub fn get_snapshot(app: AppHandle, state: State<'_, AppState>) -> Result<Snapshot, String> {
    let queue = Queue::open(&state.paths.database()).map_err(error)?;
    let now = worker::now();
    let settings = state.settings();
    let current = state.current();
    let cleanup = state.cleanup();
    let reviewing = state.reviewing();
    let sync = lore_core::sync::summary(&lore_core::sync::folder(&state.paths, &settings.knowledge_root()));
    let syncing = state.syncing();
    let label = if current.is_some() || cleanup.as_ref().is_some_and(|run| run.items.iter().any(|item| item.state == ItemState::Running)) {
        "working"
    } else if reviewing || syncing {
        "reviewing"
    } else if settings.paused {
        "paused"
    } else {
        "idle"
    };
    let review = review::working_changes(&settings.knowledge_root()).ok().flatten().map(|changes| changes.notes.len());
    let counts = queue.counts(now, current.as_ref().map(|current| current.job_id.as_str())).map_err(error)?;
    let hooks = hook_statuses();
    let connections = connections(&settings, &state.paths, &hooks);
    let catalog = state.catalog();
    let default_models = Models { codex: Some(catalog.default_model("codex").unwrap_or_else(|| DEFAULT_CODEX_MODEL.into())), claude: Some(DEFAULT_CLAUDE_MODEL.into()) };
    let cli_available = CliAvailable { codex: diagnostics::command_available(&settings.commands.codex), claude: diagnostics::command_available(&settings.commands.claude) };
    Ok(Snapshot {
        state: label,
        current,
        cleanup,
        cleanup_due: state.due(),
        counts,
        // A cleanup run lists one entry per folder, and the window shows a run as one line.
        recent: queue.recent_activity(60).map_err(error)?,
        attention: queue
            .jobs_with_status(&[Status::Failed], 20)
            .map_err(error)?,
        candidates: queue.waiting().map_err(error)?,
        failed_applies: queue.failed_batches().map_err(error)?.len(),
        problem: state.problem(),
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        settings,
        hooks,
        connections,
        review,
        reviewing,
        version: env!("CARGO_PKG_VERSION"),
        update: app.state::<crate::updates::Updates>().snapshot(),
        sync,
        syncing,
        catalog,
        default_models,
        default_efforts: Efforts { codex: Some(DEFAULT_CODEX_EFFORT.into()), claude: None },
        cli_available,
    })
}

// Model lists are read once at launch and again after setup commands finish;
// the CLIs take seconds to answer, so this never runs on a settings view.
pub fn refresh_catalog(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let fetched = catalog::fetch(&state.settings(), &state.paths.recorder_workdir());
        state.set_catalog(fetched);
        let _ = app.emit("state-changed", ());
    });
}

#[tauri::command]
pub fn set_paused(app: AppHandle, paused: bool) -> Result<(), String> {
    tray::set_paused(&app, paused)
}

#[tauri::command]
pub fn retry_job(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<bool, String> {
    let queue = Queue::open(&state.paths.database()).map_err(error)?;
    let retried = queue.retry(&id, worker::now()).map_err(error)?;
    state.wake();
    let _ = app.emit("state-changed", ());
    Ok(retried)
}

// Async and off the main thread: a job that has not started yet is named by opening its transcript.
#[tauri::command]
pub async fn get_queue(app: AppHandle) -> Result<Vec<Job>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let queue = Queue::open(&app.state::<AppState>().paths.database()).map_err(error)?;
        Ok(queue.unfinished_jobs().map_err(error)?.into_iter().map(session::identify).collect())
    })
    .await
    .map_err(error)?
}

// Async and off the main thread: discarding reads and parses the whole transcript, which can be tens of MB.
#[tauri::command]
pub async fn discard_job(app: AppHandle, id: String) -> Result<bool, String> {
    let handle = app.clone();
    let discarded = tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<AppState>();
        let queue = Queue::open(&state.paths.database()).map_err(error)?;
        worker::discard(&queue, &state.settings(), &state.paths, &id, worker::now()).map_err(error)
    })
    .await
    .map_err(error)??;
    app.state::<AppState>().wake();
    let _ = app.emit("state-changed", ());
    Ok(discarded.is_some())
}

#[tauri::command]
pub fn clear_activity(app: AppHandle, state: State<'_, AppState>) -> Result<usize, String> {
    let queue = Queue::open(&state.paths.database()).map_err(error)?;
    let cleared = queue.clear_activity().map_err(error)?;
    let _ = app.emit("state-changed", ());
    Ok(cleared)
}

#[tauri::command]
pub fn save_settings(app: AppHandle, state: State<'_, AppState>, settings: EditableSettings) -> Result<(), String> {
    let mut merged = state.settings().edited(settings);
    let moved = merged.knowledge_root != state.settings().knowledge_root;
    if moved && state.operation_busy() {
        return Err("The memory folder cannot be changed while recording or syncing.".into());
    }
    merged.validate().map_err(error)?;
    merged.fill_missing_commands();
    settings::save(&state.paths.settings(), &merged).map_err(error)?;
    state.set_problem(None);
    state.replace_settings(merged);
    if moved {
        // Another memory folder has other folders to clean up.
        state.set_due(Vec::new());
        crate::cleanup::refresh_due(&app);
    }
    tray::refresh_tooltip(&app);
    let _ = app.emit("state-changed", ());
    Ok(())
}

#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    let manager = app.autolaunch();
    let result = if enabled { manager.enable() } else { manager.disable() };
    result.map_err(error)?;
    let _ = app.emit("state-changed", ());
    Ok(())
}

fn inside(root: &Path, target: &Path) -> bool {
    let root = root.to_string_lossy().replace('\\', "/").to_lowercase();
    let target = target.to_string_lossy().replace('\\', "/").to_lowercase();
    let root = root.trim_end_matches('/');
    target.starts_with(&format!("{root}/")) && !target.contains("/../")
}

#[tauri::command]
pub fn open_note(app: AppHandle, state: State<'_, AppState>, path: String) -> Result<(), String> {
    let root = state.settings().knowledge_root();
    let target = if Path::new(&path).is_absolute() { PathBuf::from(&path) } else { root.join(&path) };
    if !inside(&root, &target) {
        return Err("Files outside the memory repository cannot be opened.".into());
    }
    app.opener().open_path(target.to_string_lossy().to_string(), None::<&str>).map_err(error)
}

#[tauri::command]
pub fn open_folder(app: AppHandle, state: State<'_, AppState>, kind: String) -> Result<(), String> {
    let folder = match kind.as_str() {
        "knowledge" => state.settings().knowledge_root(),
        "data" => state.paths.root.clone(),
        _ => return Err("Unknown folder.".into()),
    };
    app.opener().open_path(folder.to_string_lossy().to_string(), None::<&str>).map_err(error)
}

#[tauri::command]
pub fn hide_window(window: WebviewWindow) {
    let _ = window.hide();
}

#[tauri::command]
pub fn open_settings(app: AppHandle) -> Result<(), String> {
    tray::open_popover(&app);
    app.emit_to(tray::POPOVER, "show-settings", ()).map_err(error)
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_paths_inside_the_knowledge_root_can_be_opened() {
        let root = Path::new("D:/doc/knowledge");
        assert!(inside(root, Path::new("D:\\doc\\knowledge\\projects\\demo\\a.md")));
        assert!(inside(root, Path::new("d:/DOC/knowledge/common/b.md")));
        assert!(!inside(root, Path::new("D:/doc/knowledge-other/a.md")));
        assert!(!inside(root, Path::new("D:/doc/knowledge/../secret.md")));
        assert!(!inside(root, Path::new("C:/Windows/notepad.exe")));
    }
}
