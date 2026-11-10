use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::Duration;

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

pub struct AppState {
    pub paths: Paths,
    settings: Mutex<Settings>,
    current: Mutex<Option<Current>>,
    problem: Mutex<Option<String>>,
    reviewing: Mutex<bool>,
    wake: Mutex<bool>,
    signal: Condvar,
    operation: Mutex<()>,
    syncing: AtomicBool,
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
            problem: Mutex::new(None),
            reviewing: Mutex::new(false),
            wake: Mutex::new(false),
            signal: Condvar::new(),
            operation: Mutex::new(()),
            syncing: AtomicBool::new(false),
        }
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

    pub fn syncing(&self) -> bool {
        self.syncing.load(Ordering::SeqCst)
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
