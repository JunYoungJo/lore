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
    /// Approved candidates are being written into the notes, as opposed to a session being read.
    pub applying: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemState {
    Waiting,
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupItem {
    pub folder: String,
    pub absorb: Vec<String>,
    pub state: ItemState,
    /// When the worker started it; `None` while it waits.
    pub started_at: Option<i64>,
}

/// The folders one request asked to clean up. The worker cleans them one after another between
/// recordings, and the run is gone once the last one is done.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupRun {
    /// When it was asked for. The activity entries of its folders carry this id.
    pub id: i64,
    pub items: Vec<CleanupItem>,
    #[serde(skip)]
    totals: RunTotals,
}

/// What a whole run came to, for the one notification that reports it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RunTotals {
    pub folders: usize,
    pub failed: usize,
    pub changed: usize,
    pub removed: usize,
}

/// A folder with enough changed notes to be worth a cleanup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Due {
    pub name: String,
    pub fresh: usize,
}

pub struct AppState {
    pub paths: Paths,
    settings: Mutex<Settings>,
    current: Mutex<Option<Current>>,
    cleanup: Mutex<Option<CleanupRun>>,
    due: Mutex<Vec<Due>>,
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
            due: Mutex::new(Vec::new()),
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

    pub fn cleanup(&self) -> Option<CleanupRun> {
        lock(&self.cleanup).clone()
    }

    /// Hold a run over these folders for the worker. Returns false while another run waits or
    /// works, or when no folder was named.
    pub fn request_cleanup(&self, targets: Vec<Target>, now: i64) -> bool {
        let mut held = lock(&self.cleanup);
        if held.is_some() || targets.is_empty() {
            return false;
        }
        let items = targets.into_iter().map(|target| CleanupItem { folder: target.folder, absorb: target.absorb, state: ItemState::Waiting, started_at: None }).collect();
        *held = Some(CleanupRun { id: now, items, totals: RunTotals::default() });
        drop(held);
        self.wake();
        true
    }

    /// The next waiting folder of the run, marked as started, with the run's id. `None` when
    /// nothing waits or a folder is being cleaned.
    pub fn begin_cleanup(&self, now: i64) -> Option<(i64, Target)> {
        let mut held = lock(&self.cleanup);
        let run = held.as_mut().filter(|run| run.items.iter().all(|item| item.state != ItemState::Running))?;
        let id = run.id;
        let next = run.items.iter_mut().find(|item| item.state == ItemState::Waiting)?;
        next.state = ItemState::Running;
        next.started_at = Some(now);
        Some((id, Target { folder: next.folder.clone(), absorb: next.absorb.clone() }))
    }

    /// The folder being cleaned is done. `changed` is `None` when it failed. Returns what the
    /// whole run came to once no folder is left, and forgets the run.
    pub fn finish_cleanup(&self, changed: Option<(usize, usize)>) -> Option<RunTotals> {
        let mut held = lock(&self.cleanup);
        let run = held.as_mut()?;
        if let Some(item) = run.items.iter_mut().find(|item| item.state == ItemState::Running) {
            item.state = if changed.is_some() { ItemState::Done } else { ItemState::Failed };
            run.totals.folders += 1;
            match changed {
                Some((written, removed)) => {
                    run.totals.changed += written;
                    run.totals.removed += removed;
                }
                None => run.totals.failed += 1,
            }
        }
        if run.items.iter().any(|item| matches!(item.state, ItemState::Waiting | ItemState::Running)) {
            return None;
        }
        held.take().map(|run| run.totals)
    }

    /// Drop the folders that still wait. The folder being cleaned is finished first.
    pub fn stop_cleanup(&self) {
        let mut held = lock(&self.cleanup);
        if let Some(run) = held.as_mut() {
            run.items.retain(|item| item.state != ItemState::Waiting);
            if run.items.iter().all(|item| item.state != ItemState::Running) {
                *held = None;
            }
        }
    }

    pub fn due(&self) -> Vec<Due> {
        lock(&self.due).clone()
    }

    pub fn set_due(&self, due: Vec<Due>) {
        *lock(&self.due) = due;
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

    fn folder(name: &str) -> Target {
        Target { folder: name.into(), absorb: vec![] }
    }

    fn states(state: &AppState) -> Vec<(String, ItemState)> {
        state.cleanup().map(|run| run.items.into_iter().map(|item| (item.folder, item.state)).collect()).unwrap_or_default()
    }

    #[test]
    fn a_run_cleans_its_folders_one_after_another_and_reports_the_totals_at_the_end() {
        let state = AppState::new(Paths { root: "unused".into() }, Settings::default());
        let first = Target { folder: "demo".into(), absorb: vec!["old".into()] };
        assert!(!state.request_cleanup(vec![], 50));
        assert!(state.request_cleanup(vec![first.clone(), folder("other"), folder("common")], 50));
        // One run at a time: a second request is refused while the first waits or works.
        assert!(!state.request_cleanup(vec![folder("late")], 60));
        assert!(state.cleanup().unwrap().items.iter().all(|item| item.state == ItemState::Waiting && item.started_at.is_none()));

        assert_eq!(state.begin_cleanup(100), Some((50, first)));
        assert_eq!(state.cleanup().unwrap().items[0].started_at, Some(100));
        // The worker takes one folder at a time.
        assert_eq!(state.begin_cleanup(110), None);
        assert_eq!(state.finish_cleanup(Some((3, 2))), None);

        assert_eq!(state.begin_cleanup(200), Some((50, folder("other"))));
        assert_eq!(state.finish_cleanup(None), None);
        assert_eq!(states(&state), vec![("demo".into(), ItemState::Done), ("other".into(), ItemState::Failed), ("common".into(), ItemState::Waiting)]);

        assert_eq!(state.begin_cleanup(300), Some((50, folder("common"))));
        assert_eq!(state.finish_cleanup(Some((1, 0))), Some(RunTotals { folders: 3, failed: 1, changed: 4, removed: 2 }));
        assert!(state.cleanup().is_none());
        assert!(state.request_cleanup(vec![folder("late")], 400));
    }

    #[test]
    fn stopping_a_run_drops_the_folders_that_wait_and_lets_the_one_being_cleaned_finish() {
        let state = AppState::new(Paths { root: "unused".into() }, Settings::default());
        state.request_cleanup(vec![folder("a"), folder("b"), folder("c")], 50);
        state.begin_cleanup(100);
        state.stop_cleanup();
        assert_eq!(states(&state), vec![("a".into(), ItemState::Running)]);
        assert_eq!(state.finish_cleanup(Some((2, 1))), Some(RunTotals { folders: 1, failed: 0, changed: 2, removed: 1 }));

        // Stopped before the worker took anything, nothing is left of the run.
        state.request_cleanup(vec![folder("a")], 500);
        state.stop_cleanup();
        assert!(state.cleanup().is_none());
        assert_eq!(state.begin_cleanup(600), None);
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
