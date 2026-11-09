use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::Duration;

use qmd_memory_core::settings::{Paths, Settings};
use qmd_memory_core::transcript::Provider;
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
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
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
        }
    }

    pub fn reviewing(&self) -> bool {
        *lock(&self.reviewing)
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
        let (mut guard, _) = self.signal.wait_timeout_while(guard, timeout, |woken| !*woken).unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = false;
    }
}
