//! Update metadata and installation stay in Rust. The frontend cannot choose an
//! update URL or key; the updater verifies every download against the bundled key.
use std::sync::{atomic::{AtomicBool, Ordering}, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::{setup_terminal, state::AppState};

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub phase: Phase,
    pub version: Option<String>,
    pub notes: Option<String>,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase { #[default] Idle, Checking, Current, Available, Downloading, Installing, Error }

#[derive(Default)]
pub struct Updates {
    status: Mutex<Status>,
    pending: Mutex<Option<Update>>,
    busy: AtomicBool,
}

impl Updates {
    pub fn snapshot(&self) -> Status { self.status.lock().unwrap_or_else(|e| e.into_inner()).clone() }

    fn publish(&self, app: &AppHandle, status: Status) {
        *self.status.lock().unwrap_or_else(|e| e.into_inner()) = status;
        let _ = app.emit("state-changed", ());
    }

    fn begin(&self) -> Result<Busy<'_>, String> {
        self.busy.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map(|_| Busy(self)).map_err(|_| "An app update operation is already running.".into())
    }

    fn fail(&self, app: &AppHandle, error: String) -> String {
        self.publish(app, Status { phase: Phase::Error, error: Some(error.clone()), ..self.snapshot() });
        error
    }
}

struct Busy<'a>(&'a Updates);
impl Drop for Busy<'_> { fn drop(&mut self) { self.0.busy.store(false, Ordering::SeqCst); } }

pub fn check_on_startup(app: AppHandle) {
    // Development binaries must never replace a locally built executable.
    if cfg!(debug_assertions) { return; }
    tauri::async_runtime::spawn(async move {
        // Do not contend with first-run setup and the recording model catalog.
        tauri::async_runtime::spawn_blocking(|| std::thread::sleep(Duration::from_secs(10))).await.ok();
        let _ = check_app_update(app).await;
    });
}

#[tauri::command]
pub async fn check_app_update(app: AppHandle) -> Result<(), String> {
    let updates = app.state::<Updates>();
    let _busy = updates.begin()?;
    updates.publish(&app, Status { phase: Phase::Checking, ..updates.snapshot() });
    let result = async {
        app.updater_builder().timeout(Duration::from_secs(120)).build()
            .map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())
    }.await;
    match result {
        Ok(update) => {
            let status = match &update {
                Some(update) => Status { phase: Phase::Available, version: Some(update.version.clone()), notes: update.body.clone(), ..Status::default() },
                None => Status { phase: Phase::Current, ..Status::default() },
            };
            *updates.pending.lock().unwrap_or_else(|e| e.into_inner()) = update;
            updates.publish(&app, status);
            Ok(())
        }
        Err(error) => Err(updates.fail(&app, format!("Could not check for app updates: {error}"))),
    }
}

#[tauri::command]
pub async fn install_app_update(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || install(&app)).await.map_err(|e| e.to_string())?
}

fn install(app: &AppHandle) -> Result<(), String> {
    let updates = app.state::<Updates>();
    let _busy = updates.begin()?;
    let result = (|| {
        if cfg!(debug_assertions) { return Err("Install a release build to use app updates.".into()); }
        if cfg!(target_os = "linux") && std::env::var_os("APPIMAGE").is_none() {
            return Err("Automatic updates require the AppImage build on Linux. Update a DEB installation through its package manager.".into());
        }
        let pending = updates.pending.lock().unwrap_or_else(|e| e.into_inner()).clone()
            .ok_or("Check for an available update first.")?;
        let state = app.state::<AppState>();
        // Share setup's exclusion flag and the recording/sync operation gate.
        // Never interrupt an in-flight model, sync, review, or setup command.
        let _setup = state.begin_setup().ok_or("Wait for setup to finish before updating Lore.")?;
        let _operation = state.try_operation().ok_or("Wait for recording or sync to finish before updating Lore.")?;
        can_install(state.current().is_some(), state.syncing(), state.reviewing(), setup_terminal::running(app))?;
        let mut status = Status { phase: Phase::Downloading, version: Some(pending.version.clone()), notes: pending.body.clone(), ..Status::default() };
        updates.publish(app, status.clone());
        let mut last_progress = Instant::now();
        let bytes = tauri::async_runtime::block_on(pending.download(|chunk, total| {
            status.downloaded += chunk as u64;
            status.total = total;
            if last_progress.elapsed() >= Duration::from_millis(500) {
                updates.publish(app, status.clone());
                last_progress = Instant::now();
            }
        }, || {})).map_err(|e| format!("Could not download or verify the update: {e}"))?;
        updates.publish(app, Status { phase: Phase::Installing, ..status });
        pending.install(bytes).map_err(|e| format!("Could not install the update: {e}"))?;
        // Windows hands off to NSIS and exits inside install(). The other
        // platforms need an explicit restart after successful installation.
        app.restart();
    })();
    result.map_err(|error| updates.fail(app, error))
}

fn can_install(recording: bool, syncing: bool, reviewing: bool, terminal: bool) -> Result<(), String> {
    if recording || syncing || reviewing || terminal {
        Err("Finish recording, sync, setup commands, and close the memory review window before updating Lore.".into())
    } else { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_operations_cannot_overlap_and_recover_after_failure() {
        let updates = Updates::default();
        let guard = updates.begin().unwrap();
        assert!(updates.begin().is_err());
        drop(guard);
        assert!(updates.begin().is_ok());
    }

    #[test]
    fn installation_requires_idle_recording_sync_review_and_terminal() {
        assert!(can_install(false, false, false, false).is_ok());
        for busy in 0..4 {
            let mut states = [false; 4]; states[busy] = true;
            assert!(can_install(states[0], states[1], states[2], states[3]).is_err());
        }
    }
}
