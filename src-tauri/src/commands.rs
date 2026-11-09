use std::path::{Path, PathBuf};

use qmd_memory_core::hooks::{self, HookStatus};
use qmd_memory_core::pipeline;
use qmd_memory_core::queue::{Activity, Counts, Job, Queue, Status};
use qmd_memory_core::review;
use qmd_memory_core::settings::{self, EditableSettings, Settings};
use qmd_memory_core::transcript::Provider;
use qmd_memory_core::worker;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_opener::OpenerExt;

use crate::state::{AppState, Current};
use crate::tray;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub state: &'static str,
    pub current: Option<Current>,
    pub counts: Counts,
    pub recent: Vec<Activity>,
    pub attention: Vec<Job>,
    pub problem: Option<String>,
    pub settings: Settings,
    pub autostart: bool,
    pub hooks: Vec<HookStatus>,
    pub review: Option<usize>,
    pub reviewing: bool,
    pub version: &'static str,
}

fn error(value: impl std::fmt::Display) -> String {
    value.to_string()
}

fn cli_path() -> PathBuf {
    let name = qmd_memory_core::platform::CLI_FILENAME;
    let sibling = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|dir| dir.join(name))).unwrap_or_else(|| PathBuf::from(name));
    #[cfg(debug_assertions)]
    if !sibling.is_file() {
        let extension = if cfg!(windows) { ".exe" } else { "" };
        let built = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries").join(format!("qmd-memory-cli-{}{extension}", env!("TAURI_ENV_TARGET_TRIPLE")));
        if built.is_file() { return built; }
    }
    sibling
}

fn hook_statuses() -> Vec<HookStatus> {
    let cli = cli_path();
    [Provider::Claude, Provider::Codex].into_iter().map(|provider| hooks::status_at(&hooks::config_path(provider), provider, &cli)).collect()
}

#[tauri::command]
pub fn connect_hook(app: AppHandle, provider: String) -> Result<HookStatus, String> {
    let provider = Provider::parse(&provider).ok_or("알 수 없는 도구입니다.")?;
    let cli = cli_path();
    if !qmd_memory_core::platform::is_executable(&cli) {
        return Err(format!("훅이 실행할 CLI를 찾지 못했습니다: {}", cli.display()));
    }
    let config = hooks::config_path(provider);
    hooks::register_at(&config, provider, &cli, worker::now()).map_err(error)?;
    let _ = app.emit("state-changed", ());
    Ok(hooks::status_at(&config, provider, &cli))
}

#[tauri::command]
pub fn disconnect_hook(app: AppHandle, provider: String) -> Result<HookStatus, String> {
    let provider = Provider::parse(&provider).ok_or("알 수 없는 도구입니다.")?;
    let config = hooks::config_path(provider);
    hooks::unregister_at(&config, provider, worker::now()).map_err(error)?;
    let _ = app.emit("state-changed", ());
    Ok(hooks::status_at(&config, provider, &cli_path()))
}

#[tauri::command]
pub fn get_snapshot(app: AppHandle, state: State<'_, AppState>) -> Result<Snapshot, String> {
    let queue = Queue::open(&state.paths.database()).map_err(error)?;
    let now = worker::now();
    let settings = state.settings();
    let current = state.current();
    let reviewing = state.reviewing();
    let label = if current.is_some() {
        "working"
    } else if reviewing {
        "reviewing"
    } else if settings.paused {
        "paused"
    } else {
        "idle"
    };
    let review = review::working_changes(&settings.knowledge_root()).ok().flatten().map(|changes| changes.notes.len());
    let counts = queue.counts(now, current.as_ref().map(|current| current.job_id.as_str())).map_err(error)?;
    Ok(Snapshot {
        state: label,
        current,
        counts,
        recent: queue.recent_activity(30).map_err(error)?,
        attention: queue
            .jobs_with_status(&[Status::Failed, Status::Applying, Status::IndexPending], 20)
            .map_err(error)?
            .into_iter()
            .filter(|job| job.status == Status::Failed || job.attempts > 0)
            .collect(),
        problem: state.problem(),
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        settings,
        hooks: hook_statuses(),
        review,
        reviewing,
        version: env!("CARGO_PKG_VERSION"),
    })
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
        Ok(queue.unfinished_jobs().map_err(error)?.into_iter().map(pipeline::identify).collect())
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
    merged.validate().map_err(error)?;
    merged.fill_missing_commands();
    settings::save(&state.paths.settings(), &merged).map_err(error)?;
    state.set_problem(None);
    state.replace_settings(merged);
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
        return Err("기억 저장소 밖의 파일은 열 수 없습니다.".into());
    }
    app.opener().open_path(target.to_string_lossy().to_string(), None::<&str>).map_err(error)
}

#[tauri::command]
pub fn open_folder(app: AppHandle, state: State<'_, AppState>, kind: String) -> Result<(), String> {
    let folder = match kind.as_str() {
        "knowledge" => state.settings().knowledge_root(),
        "data" => state.paths.root.clone(),
        _ => return Err("알 수 없는 폴더입니다.".into()),
    };
    app.opener().open_path(folder.to_string_lossy().to_string(), None::<&str>).map_err(error)
}

#[tauri::command]
pub fn hide_window(window: WebviewWindow) {
    let _ = window.hide();
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
