use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use lore_core::error::Failure;
use lore_core::qmd::Qmd;
use lore_core::queue::Queue;
use lore_core::recorder::Recorders;
use lore_core::run::Limit;
use lore_core::settings::Settings;
use lore_core::transcript::Provider;
use lore_core::worker::{self, Outcome, Reporter, Work, WorkerLock};

const SYNC_EVERY: Duration = Duration::from_secs(300);
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::state::{AppState, Current, RunTotals};
use crate::tray;

const IDLE_WAIT: Duration = Duration::from_secs(5);
const ERROR_WAIT: Duration = Duration::from_secs(30);

fn notify(app: &AppHandle, title: &str, body: &str) {
    if let Err(error) = app.notification().builder().title(title).body(body).show() {
        eprintln!("Could not show a notification: {error}");
    }
}

fn changed(app: &AppHandle) {
    tray::refresh_tooltip(app);
    let _ = app.emit("state-changed", ());
}

fn live_limit(app: &AppHandle, seconds: fn(&Settings) -> u64) -> Limit {
    let app = app.clone();
    Arc::new(move || Duration::from_secs(app.state::<AppState>().read_settings(seconds)))
}

fn minutes_until(timestamp: i64) -> i64 {
    ((timestamp - worker::now()).max(0) + 59) / 60
}

fn count(number: usize, one: &str, many: &str) -> String {
    format!("{number} {}", if number == 1 { one } else { many })
}

/// The one notification a finished cleanup run gets. `folder` and `error` are those of its last
/// folder, which is the whole run when it had only one.
fn cleanup_report(folder: &str, totals: &RunTotals, error: Option<String>) -> (&'static str, String) {
    let scope = if totals.folders == 1 { folder.to_string() } else { count(totals.folders, "folder", "folders") };
    if totals.failed == totals.folders {
        let why = if totals.folders == 1 { error.unwrap_or_else(|| "an internal error occurred.".into()) } else { "none could be cleaned up. Open Lore to see why.".into() };
        return ("Cleanup failed", format!("{scope}: {why}"));
    }
    let done = if totals.changed + totals.removed == 0 { "nothing needed changing.".to_string() } else { format!("{} changed, {} removed.", count(totals.changed, "note", "notes"), totals.removed) };
    let failed = if totals.failed > 0 { format!(" {} failed.", count(totals.failed, "folder", "folders")) } else { String::new() };
    ("Cleanup complete", format!("{scope}: {done}{failed}"))
}

struct AppReporter {
    app: AppHandle,
}

/// What the user is told when a session has ended and all of it is read.
fn ended(project: &str, candidates: usize) -> (&'static str, String) {
    ("Candidates to review", format!("{project}: {} from the session that just ended. Open Lore to review.", count(candidates, "candidate", "candidates")))
}

/// What the user is told when a piece of work is done. What a step of a session offered is
/// announced when the session ends (see `ended`), and what the user approved a moment ago in the
/// window is not announced at all. An index that will be refreshed later needs no one's attention.
fn report(work: &Work, outcome: &Outcome) -> Option<(&'static str, String)> {
    match outcome {
        Outcome::Applied { project, notes, candidates, auto: true, .. } => {
            Some(("Candidates approved automatically", format!("{project}: {} written into {}.", count(*candidates, "candidate", "candidates"), count(notes.len(), "note", "notes"))))
        }
        Outcome::Failed { project, message, gave_up, next_attempt_at } => {
            let target = project.clone().unwrap_or_else(|| "Session".into());
            let next = if *gave_up { "No more automatic retries.".to_string() } else { format!("Retrying in about {} min.", minutes_until(*next_attempt_at)) };
            let title = if matches!(work, Work::Apply(_)) { "Approved candidates were not written" } else { "Reading a session failed" };
            Some((title, format!("{target}: {message} {next}")))
        }
        _ => None,
    }
}

impl Reporter for AppReporter {
    fn started(&self, work: &Work, project: &str) {
        let state = self.app.state::<AppState>();
        let provider = match work {
            Work::Session(job) => job.provider,
            Work::Apply(_) => Provider::parse(&state.settings().backend).unwrap_or(Provider::Claude),
        };
        state.set_current(Some(Current { job_id: work.id(), provider, project: project.to_string(), started_at: worker::now(), applying: matches!(work, Work::Apply(_)) }));
        changed(&self.app);
    }

    fn finished(&self, work: &Work, outcome: &Outcome) {
        self.app.state::<AppState>().set_current(None);
        if let Some((title, body)) = report(work, outcome) {
            notify(&self.app, title, &body);
        }
        changed(&self.app);
    }

    fn session_ended(&self, project: &str, candidates: usize) {
        let (title, body) = ended(project, candidates);
        notify(&self.app, title, &body);
    }
}

pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let paths = state.paths.clone();
        let _lock = match WorkerLock::try_acquire(&paths.lock()) {
            Ok(Some(lock)) => lock,
            Ok(None) => {
                state.set_problem(Some("Another worker (lore-cli work) is running. Restart the app after it finishes.".into()));
                changed(&app);
                return;
            }
            Err(error) => {
                state.set_problem(Some(format!("Could not open the worker lock: {error}")));
                changed(&app);
                return;
            }
        };
        let queue = match Queue::open(&paths.database()) {
            Ok(queue) => queue,
            Err(error) => {
                state.set_problem(Some(format!("Could not open the state database: {error}")));
                changed(&app);
                return;
            }
        };
        let reporter = AppReporter { app: app.clone() };
        let model_limit = live_limit(&app, |settings| settings.model_timeout_secs);
        let qmd_limit = live_limit(&app, |settings| settings.qmd_timeout_secs);
        let mut last_sync = Instant::now().checked_sub(SYNC_EVERY).unwrap_or_else(Instant::now);
        // Notes recorded since the last sync. They are shared as soon as the queue is idle.
        let mut unsent = false;
        // Whether the folders should be looked at for a cleanup: at launch and after each recording.
        let mut look_for_cleanup = true;
        // The cleanup run whose start was already announced.
        let mut announced: Option<i64> = None;
        loop {
            // A comparison whose model could not run becomes due again later.
            crate::sync::resume_comparisons(&app);
            let settings = state.settings();
            if settings.sync_enabled && !state.setup_busy() && !state.reviewing() && !state.syncing() && last_sync.elapsed() >= SYNC_EVERY {
                last_sync = Instant::now();
                unsent = false;
                crate::sync::automatic(&app);
            }
            if state.setup_busy() || settings.paused || state.reviewing() || state.syncing() {
                state.wait(ERROR_WAIT);
                continue;
            }
            if let Err(error) = settings.validate() {
                state.set_problem(Some(error.to_string()));
                changed(&app);
                state.wait(ERROR_WAIT);
                continue;
            }
            let catalog = state.catalog();
            let recorder = Recorders::from_settings(&settings, &paths.recorder_workdir(), model_limit.clone(), |backend| catalog.default_model(backend));
            let qmd = Qmd::new(&settings, &paths.recorder_workdir(), qmd_limit.clone());
            let operation = state.operation();
            // Re-check under the same gate used by sync and review commits.
            if state.setup_busy() || state.syncing() || state.reviewing() || state.settings().paused || state.settings().knowledge_root != settings.knowledge_root { drop(operation); continue; }
            // A cleanup run works here, one folder per turn, between recordings and under the same gate.
            // A batch that was interrupted while it wrote its notes is finished or taken back
            // first (by the step below, when it is due): a cleanup must not rewrite notes that
            // are half written.
            let half_written = queue.write_interrupted().unwrap_or(true);
            if let Some((run, target)) = (!half_written).then(|| state.begin_cleanup(worker::now())).flatten() {
                if announced != Some(run) {
                    announced = Some(run);
                    let folders = state.cleanup().map_or(1, |run| run.items.len());
                    let what = if folders == 1 { format!("Cleaning up the {} notes.", target.folder) } else { format!("Cleaning up {folders} folders of notes.") };
                    notify(&app, "Cleanup started", &what);
                }
                changed(&app);
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    worker::clean(&queue, &settings, &paths, &qmd, &target, run, &mut |prompt, schema, artifacts| recorder.request_json(prompt, schema, artifacts))
                }));
                let counts = match &result {
                    Ok(Ok(outcome)) => {
                        unsent |= !outcome.touched().is_empty();
                        Some((outcome.written.len(), outcome.removed.len()))
                    }
                    Ok(Err(_)) => None,
                    Err(_) => {
                        // The worker did not get to remember this failure itself.
                        let _ = queue.cleanup_failed(&target.folder, worker::now(), "An internal error occurred.");
                        None
                    }
                };
                let finished = state.finish_cleanup(counts);
                drop(operation);
                let _ = crate::cleanup::survey(&state);
                if let Some(totals) = finished {
                    let (title, body) = cleanup_report(&target.folder, &totals, result.ok().and_then(|result| result.err()).map(|error| error.to_string()));
                    notify(&app, title, &body);
                }
                changed(&app);
                continue;
            }
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                // Candidates that waited out the set number of days are approved first, so that this turn can write them.
                worker::approve_overdue(&queue, &settings, worker::now())?;
                let memory = settings.knowledge_root();
                worker::step(&queue, &settings, &paths, &mut |prompt, schema, artifacts| recorder.request_json_reading(prompt, schema, artifacts, &memory), &qmd, &reporter)
            }));
            drop(operation);
            changed(&app);
            match result {
                Ok(Ok(Some((_, outcome)))) => {
                    // Notes are in the folder in both cases; in the second only the index is late.
                    let written = matches!(outcome, Outcome::Applied { .. } | Outcome::Unindexed { .. });
                    unsent |= written;
                    look_for_cleanup |= written;
                    state.set_problem(None);
                }
                Ok(Ok(None)) => {
                    // New notes may have made a folder due. With automatic cleanup on, the due
                    // folders are cleaned now that the queue is idle, before the notes are shared.
                    if look_for_cleanup {
                        look_for_cleanup = false;
                        if let Ok(folders) = crate::cleanup::survey(&state) {
                            let targets = crate::cleanup::automatic(&folders, worker::now());
                            let started = state.settings().auto_cleanup && state.cleanup().is_none() && state.request_cleanup(targets, worker::now());
                            changed(&app);
                            if started {
                                continue;
                            }
                        }
                    }
                    if unsent && state.settings().sync_enabled {
                        unsent = false;
                        last_sync = Instant::now();
                        crate::sync::automatic(&app);
                    }
                    state.wait(IDLE_WAIT);
                }
                Ok(Err(error)) => {
                    state.set_current(None);
                    state.set_problem(Some(format!("Could not process the job: {error}")));
                    changed(&app);
                    state.wait(ERROR_WAIT);
                }
                Err(_) => {
                    state.set_current(None);
                    // What was being worked on is what comes up next: a batch before a session.
                    if let Ok(Some(batch)) = queue.next_batch(worker::now()) {
                        // Recorded the way the worker records a failure, so that a batch that
                        // gives up here also takes back what it had begun to write.
                        let _ = worker::batch_failed(&queue, &settings, &batch, lore_core::error::Error::transient("An internal recording error occurred."));
                    } else if let Ok(Some(job)) = queue.next_job(worker::now()) {
                        let _ = queue.record_failure(&job.id, Failure::Transient, "An internal recording error occurred.", worker::now());
                    }
                    state.set_problem(Some("An internal recording error occurred. The job has been deferred so other jobs can continue.".into()));
                    changed(&app);
                    state.wait(ERROR_WAIT);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cleanup_run_is_reported_once_with_what_all_its_folders_came_to() {
        let one = RunTotals { folders: 1, failed: 0, changed: 1, removed: 2 };
        assert_eq!(cleanup_report("demo", &one, None), ("Cleanup complete", "demo: 1 note changed, 2 removed.".into()));
        let idle = RunTotals { folders: 1, ..RunTotals::default() };
        assert_eq!(cleanup_report("demo", &idle, None), ("Cleanup complete", "demo: nothing needed changing.".into()));
        let failed = RunTotals { folders: 1, failed: 1, ..RunTotals::default() };
        assert_eq!(cleanup_report("demo", &failed, Some("qmd was not found.".into())), ("Cleanup failed", "demo: qmd was not found.".into()));

        let several = RunTotals { folders: 5, failed: 1, changed: 21, removed: 11 };
        assert_eq!(cleanup_report("common", &several, None), ("Cleanup complete", "5 folders: 21 notes changed, 11 removed. 1 folder failed.".into()));
        let none = RunTotals { folders: 2, failed: 2, ..RunTotals::default() };
        assert_eq!(cleanup_report("common", &none, Some("x".into())).0, "Cleanup failed");
    }

    #[test]
    fn the_user_is_told_of_candidates_only_when_a_session_has_ended_and_of_writes_only_when_they_did_not_approve_them() {
        let job = lore_core::queue::Job {
            id: "job".into(),
            provider: Provider::Claude,
            transcript: "t.jsonl".into(),
            status: lore_core::queue::Status::Pending,
            cursor_line: 0,
            cursor_hash: String::new(),
            project: None,
            session_id: None,
            attempts: 0,
            next_attempt_at: 0,
            requests: 1,
            last_error: None,
            updated_at: 0,
            ended: true,
        };
        let session = Work::Session(job);
        assert_eq!(ended("demo", 2), ("Candidates to review", "demo: 2 candidates from the session that just ended. Open Lore to review.".into()));
        assert_eq!(ended("demo", 1).1, "demo: 1 candidate from the session that just ended. Open Lore to review.");
        // A step that offered candidates says nothing by itself: the session may go on.
        assert_eq!(report(&session, &Outcome::Offered { project: "demo".into(), candidates: 2, left_out: 1 }), None);
        assert_eq!(report(&session, &Outcome::Nothing { project: "demo".into() }), None);
        assert_eq!(report(&session, &Outcome::Unindexed { project: "demo".into(), message: "qmd failed".into(), next_attempt_at: 0 }), None);
        let applied = |auto| Outcome::Applied { project: "demo".into(), notes: vec!["projects/demo/a.md".into()], summary: "s".into(), candidates: 3, auto };
        assert_eq!(report(&session, &applied(false)), None);
        assert_eq!(report(&session, &applied(true)).unwrap().1, "demo: 3 candidates written into 1 note.");
        let failed = Outcome::Failed { project: None, message: "qmd was not found.".into(), gave_up: true, next_attempt_at: 0 };
        assert_eq!(report(&session, &failed), Some(("Reading a session failed", "Session: qmd was not found. No more automatic retries.".into())));
    }
}
