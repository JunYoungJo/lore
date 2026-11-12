use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::Duration;

use lore_core::catalog::Catalog;
use lore_core::cleanup::Target;
use lore_core::settings::{Paths, Settings};
use lore_core::transcript::Provider;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Current {
    pub job_id: String,
    pub provider: Provider,
    pub project: String,
    pub started_at: i64,
}

/// A cleanup the user asked for. It waits for the worker, which runs it between recordings.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cleaning {
    pub folder: String,
    pub absorb: Vec<String>,
    /// When the worker started it; `None` while it waits.
    pub started_at: Option<i64>,
}

pub struct AppState {
    pub paths: Paths,
    settings: Mutex<Settings>,
    current: Mutex<Option<Current>>,
    cleanup: Mutex<Option<Cleaning>>,
    problem: Mutex<Option<String>>,
    reviewing: Mutex<bool>,
    wake: Mutex<bool>,
    signal: Condvar,
    operation: Mutex<()>,
    syncing: AtomicBool,
    setup_busy: AtomicBool,
    catalog: Mutex<Catalog>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl AppState {
    pub fn new(paths: Paths, settings: Settings) -> Self {
        Self {
            paths,
            settings: Mutex::new(settings),
            current: Mutex::new(None),
            cleanup: Mutex::new(None),
            problem: Mutex::new(None),
            reviewing: Mutex::new(false),
            wake: Mutex::new(false),
            signal: Condvar::new(),
            operation: Mutex::new(()),
            syncing: AtomicBool::new(false),
            setup_busy: AtomicBool::new(false),
            catalog: Mutex::new(Catalog::default()),
        }
    }

    pub fn catalog(&self) -> Catalog {
        lock(&self.catalog).clone()
    }

    pub fn set_catalog(&self, catalog: Catalog) {
        *lock(&self.catalog) = catalog;
    }

    pub fn reviewing(&self) -> bool {
        *lock(&self.reviewing)
    }

    pub fn operation(&self) -> MutexGuard<'_, ()> {
        lock(&self.operation)
    }

    pub fn operation_busy(&self) -> bool {
        self.operation.try_lock().is_err()
    }

    pub fn try_operation(&self) -> Option<MutexGuard<'_, ()>> {
        self.operation.try_lock().ok()
    }

    pub fn syncing(&self) -> bool {
        self.syncing.load(Ordering::SeqCst)
    }

    pub fn setup_busy(&self) -> bool { self.setup_busy.load(Ordering::SeqCst) }

    pub fn begin_setup(&self) -> Option<SetupGuard<'_>> {
        self.setup_busy.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).ok().map(|_| SetupGuard(self))
    }

    pub fn begin_sync(&self) -> Option<SyncGuard<'_>> {
        self.syncing
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| SyncGuard(self))
    }

    pub fn set_reviewing(&self, reviewing: bool) {
        *lock(&self.reviewing) = reviewing;
        self.wake();
    }

    pub fn settings(&self) -> Settings {
        lock(&self.settings).clone()
    }

    pub fn read_settings<T>(&self, read: impl FnOnce(&Settings) -> T) -> T {
        read(&lock(&self.settings))
    }

    pub fn replace_settings(&self, settings: Settings) {
        *lock(&self.settings) = settings;
        self.wake();
    }

    pub fn current(&self) -> Option<Current> {
        lock(&self.current).clone()
    }

    pub fn set_current(&self, current: Option<Current>) {
        *lock(&self.current) = current;
    }

    pub fn cleanup(&self) -> Option<Cleaning> {
        lock(&self.cleanup).clone()
    }

    /// Hold a cleanup for the worker. Returns false while another one waits or runs.
    pub fn request_cleanup(&self, target: Target) -> bool {
        let mut held = lock(&self.cleanup);
        if held.is_some() {
            return false;
        }
        *held = Some(Cleaning { folder: target.folder, absorb: target.absorb, started_at: None });
        drop(held);
        self.wake();
        true
    }

    /// The waiting cleanup, marked as started. `None` when none waits.
    pub fn begin_cleanup(&self, now: i64) -> Option<Target> {
        let mut held = lock(&self.cleanup);
        let waiting = held.as_mut().filter(|cleaning| cleaning.started_at.is_none())?;
        waiting.started_at = Some(now);
        Some(Target { folder: waiting.folder.clone(), absorb: waiting.absorb.clone() })
    }

    pub fn finish_cleanup(&self) {
        *lock(&self.cleanup) = None;
    }

    pub fn problem(&self) -> Option<String> {
        lock(&self.problem).clone()
    }

    pub fn set_problem(&self, problem: Option<String>) {
        *lock(&self.problem) = problem;
    }

    pub fn wake(&self) {
        *lock(&self.wake) = true;
        self.signal.notify_all();
    }

    pub fn wait(&self, timeout: Duration) {
        let guard = lock(&self.wake);
        let (mut guard, _) = self
            .signal
            .wait_timeout_while(guard, timeout, |woken| !*woken)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = false;
    }
}

pub struct SetupGuard<'a>(&'a AppState);
impl Drop for SetupGuard<'_> {
    fn drop(&mut self) { self.0.setup_busy.store(false, Ordering::SeqCst); self.0.wake(); }
}

pub struct SyncGuard<'a>(&'a AppState);
impl Drop for SyncGuard<'_> {
    fn drop(&mut self) {
        self.0.syncing.store(false, Ordering::SeqCst);
        self.0.wake();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_guard_is_exclusive_and_preserves_user_pause_and_review() {
        let state = AppState::new(
            Paths {
                root: "unused".into(),
            },
            Settings {
                paused: true,
                ..Settings::default()
            },
        );
        state.set_reviewing(true);
        let guard = state.begin_sync().unwrap();
        assert!(state.syncing());
        assert!(state.begin_sync().is_none());
        drop(guard);
        assert!(!state.syncing());
        assert!(state.settings().paused);
        assert!(state.reviewing());
        assert!(state.begin_sync().is_some());
    }

    #[test]
    fn one_cleanup_waits_for_the_worker_and_shows_when_it_started() {
        let state = AppState::new(Paths { root: "unused".into() }, Settings::default());
        let target = Target { folder: "demo".into(), absorb: vec!["old".into()] };
        assert!(state.request_cleanup(target.clone()));
        assert!(!state.request_cleanup(Target { folder: "other".into(), absorb: vec![] }));
        let waiting = state.cleanup().unwrap();
        assert_eq!((waiting.folder.as_str(), waiting.started_at), ("demo", None));

        assert_eq!(state.begin_cleanup(100), Some(target));
        assert_eq!(state.cleanup().unwrap().started_at, Some(100));
        // The worker takes a request once; it is not started again while it runs.
        assert_eq!(state.begin_cleanup(200), None);
        assert!(!state.request_cleanup(Target { folder: "other".into(), absorb: vec![] }));

        state.finish_cleanup();
        assert!(state.cleanup().is_none());
        assert!(state.request_cleanup(Target { folder: "other".into(), absorb: vec![] }));
    }

    #[test]
    fn recording_and_sync_share_the_operation_gate() {
        let state = AppState::new(
            Paths {
                root: "unused".into(),
            },
            Settings::default(),
        );
        let recording = state.operation();
        assert!(state.operation_busy());
        assert!(state.operation.try_lock().is_err());
        drop(recording);
        let sync = state.begin_sync().unwrap();
        let applying = state.operation();
        assert!(state.operation_busy());
        drop(applying);
        drop(sync);
        assert!(!state.operation_busy());
    }
}
