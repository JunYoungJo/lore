use std::fs::File;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::candidate::{ApplyResult, ApplyStatus, Batch, Origin};
use crate::cleanup::{self, Target};
use crate::error::{Error, Failure, Result};
use crate::integrate::{self, Plan};
use crate::notes;
use crate::prepare::{self, Ask, NOTES_PER_CANDIDATE};
use crate::proposal::summary_with_log;
use crate::qmd::{collection_name, Qmd};
use crate::queue::{Activity, Job, Queue, Status};
use crate::review;
use crate::session::{self, previous_failure, Reading};
use crate::settings::{Paths, Settings};
use crate::transcript::Provider;

const REVIEWS_KEPT: usize = 50;
const NO_QMD: &str = "qmd was not found, so the recording model was not called. Run lore-cli doctor to check the executable path.";

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|elapsed| elapsed.as_secs() as i64).unwrap_or(0)
}

/// One piece of work the worker takes up: a queued session, or candidates the user approved.
#[derive(Debug, Clone)]
pub enum Work {
    Session(Job),
    Apply(Batch),
}

impl Work {
    /// The id the activity list files this work under.
    pub fn id(&self) -> String {
        match self {
            Self::Session(job) => job.id.clone(),
            Self::Apply(batch) => apply_run_id(batch.id),
        }
    }
}

pub fn apply_run_id(batch: i64) -> String {
    format!("apply-{batch}")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    /// A step of a session offered candidates for the user to review.
    Offered { project: String, candidates: usize, left_out: usize },
    /// A session held nothing to remember.
    Nothing { project: String },
    /// Approved candidates were written into the notes.
    Applied { project: String, notes: Vec<String>, summary: String, candidates: usize, auto: bool },
    Skipped { reason: String },
    CaughtUp,
    Failed { project: Option<String>, message: String, gave_up: bool, next_attempt_at: i64 },
    /// The notes of a batch are written, but the search index could not be refreshed yet. It is tried again.
    Unindexed { project: String, message: String, next_attempt_at: i64 },
}

pub trait Reporter {
    fn started(&self, work: &Work, project: &str);
    fn finished(&self, work: &Work, outcome: &Outcome);
    /// A session has ended and all of it is read: `candidates` of it wait for the user.
    fn session_ended(&self, project: &str, candidates: usize);
}

/// The search index over the notes, as the worker uses it.
pub trait Index {
    fn available(&self) -> bool;
    /// The paths of the notes most likely to be about `query`, best first.
    fn search(&self, query: &str, collections: &[String], limit: usize) -> Vec<String>;
    /// Bring the index up to date with the notes in `root`.
    fn reindex(&self, root: &Path) -> Result<()>;
}

impl Index for Qmd {
    fn available(&self) -> bool {
        Qmd::available(self)
    }

    // A search that fails finds nothing: the candidate is then treated as new content.
    fn search(&self, query: &str, collections: &[String], limit: usize) -> Vec<String> {
        self.rank(query, collections, limit).unwrap_or_default()
    }

    fn reindex(&self, root: &Path) -> Result<()> {
        self.ensure_index(root)?;
        self.refresh()
    }
}

pub struct WorkerLock {
    _file: File,
}

impl WorkerLock {
    pub fn try_acquire(path: &Path) -> Result<Option<Self>> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = File::options().create(true).truncate(false).write(true).open(path)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self { _file: file })),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(error)) => Err(error.into()),
        }
    }
}

fn prune_reviews(folder: &Path) {
    let Ok(entries) = std::fs::read_dir(folder) else { return };
    let mut dirs: Vec<(SystemTime, std::path::PathBuf)> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
        .filter_map(|entry| entry.metadata().and_then(|meta| meta.modified()).ok().map(|time| (time, entry.path())))
        .collect();
    dirs.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, path) in dirs.into_iter().skip(REVIEWS_KEPT) {
        let _ = std::fs::remove_dir_all(path);
    }
}

fn collections(project: &str) -> Vec<String> {
    vec!["common".to_string(), collection_name(project)]
}

/// Read the new part of a session and store what it offers. No note is touched.
fn offer(queue: &Queue, settings: &Settings, paths: &Paths, ask: Ask, index: &dyn Index, job: &Job, reporter: &dyn Reporter) -> Result<Outcome> {
    let session = match session::read(job, settings.max_chars, &paths.recorder_workdir())? {
        Reading::Skip(reason) => {
            queue.skip(&job.id, &reason, now())?;
            return Ok(Outcome::Skipped { reason });
        }
        Reading::NothingNew { end_line, end_hash } => {
            let done = queue.finish(&job.id, end_line, &end_hash, false, job.requests, now())?;
            announce(queue, job, job.project.as_deref(), done, reporter)?;
            return Ok(Outcome::CaughtUp);
        }
        Reading::AgentOnly { end_line, end_hash, has_more } => {
            let done = queue.finish(&job.id, end_line, &end_hash, has_more, job.requests, now())?;
            announce(queue, job, job.project.as_deref(), done, reporter)?;
            return Ok(Outcome::CaughtUp);
        }
        Reading::Ready(session) => session,
    };
    queue.set_session(&job.id, Some(&session.project), &session.parsed.session_id)?;
    if !index.available() {
        return Err(Error::transient(NO_QMD));
    }
    reporter.started(&Work::Session(job.clone()), &session.project);
    let notes = notes::load_scope(&settings.knowledge_root(), &session.project)?;
    let scope = collections(&session.project);
    let search = |text: &str| index.search(text, &scope, NOTES_PER_CANDIDATE);
    let artifacts = paths.reviews().join(format!("{}-{}", job.id, session.batch.end_line));
    let prepared = prepare::prepare(&session, &notes, previous_failure(job.attempts, job.last_error.as_deref()), &artifacts, ask, &search)?;
    prune_reviews(&paths.reviews());
    let origin = Origin { job_id: &job.id, session_id: &session.parsed.session_id, provider: job.provider, project: &session.project, language: &prepared.language };
    let done = queue.store_candidates(&origin, session.batch.end_line, &session.end_hash, session.has_more(), job.requests, &prepared.offered, &prepared.left_out, now())?;
    announce(queue, job, Some(&session.project), done, reporter)?;
    if prepared.offered.is_empty() {
        return Ok(Outcome::Nothing { project: session.project.clone() });
    }
    Ok(Outcome::Offered { project: session.project.clone(), candidates: prepared.offered.len(), left_out: prepared.left_out.len() })
}

/// Tell the user what a session offered, once it has ended and the last of it is read. Every
/// candidate of the session that still waits is counted, whichever step found it, and none twice.
fn announce(queue: &Queue, job: &Job, project: Option<&str>, done: bool, reporter: &dyn Reporter) -> Result<()> {
    if done && job.ended {
        let waiting = queue.announce(&job.id)?;
        if waiting > 0 {
            reporter.session_ended(project.unwrap_or_default(), waiting);
        }
    }
    Ok(())
}

/// What an apply batch keeps while its notes are being written, so that an interrupted write
/// is finished from it, or taken back with it.
#[derive(Serialize, Deserialize)]
struct Writing {
    plan: Plan,
    /// The notes that count as reviewed once they are committed.
    reviewed: Vec<String>,
}

fn writing(batch: &Batch) -> Result<Writing> {
    let manifest = batch.manifest.as_deref().ok_or_else(|| Error::permanent("No interrupted apply state is available."))?;
    serde_json::from_str(manifest).map_err(|error| Error::permanent(format!("Could not read interrupted apply state: {error}")))
}

/// The notes that wait in the Review window now.
fn waiting_for_review(root: &Path) -> Vec<String> {
    review::working_changes(root).ok().flatten().map(|changes| changes.notes.into_iter().map(|change| change.path).collect()).unwrap_or_default()
}

/// Write a batch's notes, commit them and refresh the index. A batch that was interrupted picks
/// up where it stopped.
fn write(queue: &Queue, settings: &Settings, paths: &Paths, ask: Ask, index: &dyn Index, batch: &Batch, reporter: &dyn Reporter) -> Result<Outcome> {
    let root = settings.knowledge_root();
    let (result, reviewed) = match batch.status {
        // The notes are written. Only the commit and the index are left.
        ApplyStatus::IndexPending => (batch.result.clone().unwrap_or_default(), batch.manifest.as_deref().and_then(|list| serde_json::from_str(list).ok()).unwrap_or_default()),
        ApplyStatus::Applying => put(queue, &root, batch, writing(batch)?)?,
        _ => {
            let members = queue.batch_candidates(batch.id)?;
            if members.is_empty() {
                queue.finish_batch(batch.id, now())?;
                return Ok(Outcome::CaughtUp);
            }
            if !index.available() {
                return Err(Error::transient(NO_QMD));
            }
            reporter.started(&Work::Apply(batch.clone()), &batch.project);
            let notes = notes::load_scope(&root, &batch.project)?;
            let scope = collections(&batch.project);
            let search = |text: &str| index.search(text, &scope, NOTES_PER_CANDIDATE);
            let artifacts = paths.reviews().join(apply_run_id(batch.id));
            let plan = integrate::plan(&batch.project, &members, &notes, previous_failure(batch.attempts, batch.last_error.as_deref()), &artifacts, ask, &search)?;
            prune_reviews(&paths.reviews());
            // What the user approved by hand counts as reviewed. A note that already waits for
            // review keeps waiting: what an automatic approval or a cleanup changed in it has not
            // been looked at. An automatic approval marks nothing as reviewed.
            let waiting = waiting_for_review(&root);
            let reviewed = if batch.auto { Vec::new() } else { plan.touched().into_iter().filter(|note| !waiting.contains(note)).collect() };
            let state = Writing { plan, reviewed };
            queue.begin_batch(batch.id, &serde_json::to_string(&state)?, now())?;
            put(queue, &root, batch, state)?
        }
    };
    // Committed right away so sync never waits for a manual commit.
    let committed = if reviewed.is_empty() { review::record_commit(&root).map(|_| ()) } else { review::mark_reviewed(&root, &reviewed).map(|_| ()).or_else(|_| review::record_commit(&root).map(|_| ())) };
    if let Err(error) = committed {
        eprintln!("Could not commit the recorded notes: {error}");
    }
    index.reindex(&root)?;
    queue.finish_batch(batch.id, now())?;
    let notes = result.notes.iter().map(|note| note.path.clone()).collect();
    Ok(Outcome::Applied { project: batch.project.clone(), notes, summary: result.summary, candidates: batch.candidates, auto: batch.auto })
}

/// Write the notes of a plan. Once they are all written the batch counts as applied.
fn put(queue: &Queue, root: &Path, batch: &Batch, state: Writing) -> Result<(ApplyResult, Vec<String>)> {
    integrate::apply(root, &state.plan)?;
    queue.batch_written(batch.id, &state.plan.result, &state.reviewed, now())?;
    Ok((state.plan.result, state.reviewed))
}

/// Record that a batch failed. One that gives up while it was writing first takes back what it
/// wrote, so that a failed batch has left nothing in the notes: trying it again, or releasing
/// its candidates, starts from the notes as they were.
pub fn batch_failed(queue: &Queue, settings: &Settings, batch: &Batch, error: Error) -> Result<Outcome> {
    let current = queue.batch(batch.id)?.unwrap_or_else(|| batch.clone());
    let mut message = error.message;
    if current.status == ApplyStatus::IndexPending {
        let message = format!("The notes are written, but the search index could not be refreshed: {message}");
        let failure = queue.batch_failed(batch.id, error.failure, &message, now())?;
        return Ok(Outcome::Unindexed { project: batch.project.clone(), message, next_attempt_at: failure.next_attempt_at });
    }
    if current.status == ApplyStatus::Applying && current.gives_up(error.failure) {
        let root = settings.knowledge_root();
        match writing(&current).and_then(|state| integrate::undo(&root, &state.plan)) {
            Ok(0) => {}
            Ok(_) => {
                if let Err(error) = review::record_commit(&root) {
                    eprintln!("Could not commit the notes that were put back: {error}");
                }
            }
            Err(error) => message = format!("{message} What it had begun to write could not be taken back: {}", error.message),
        }
    }
    let failure = queue.batch_failed(batch.id, error.failure, &message, now())?;
    Ok(Outcome::Failed { project: Some(batch.project.clone()), message, gave_up: failure.gave_up, next_attempt_at: failure.next_attempt_at })
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

fn activity(work: &Work, outcome: &Outcome, fallback: Provider) -> Option<Activity> {
    let (provider, known_project) = match work {
        Work::Session(job) => (job.provider, job.project.clone()),
        Work::Apply(batch) => (fallback, Some(batch.project.clone())),
    };
    let base = |kind: &str, project: Option<String>| Activity { id: 0, at: now(), job_id: work.id(), provider, project, kind: kind.into(), summary: None, notes: Vec::new(), error: None };
    Some(match outcome {
        Outcome::Offered { project, candidates, left_out, .. } => {
            let waiting = format!("{} to review", plural(*candidates, "candidate", "candidates"));
            let summary = if *left_out == 0 { waiting } else { format!("{waiting}, {left_out} already in your notes") };
            Activity { summary: Some(summary), ..base("offered", Some(project.clone())) }
        }
        Outcome::Nothing { project } => Activity { summary: Some("Nothing to remember in this part of the session.".into()), ..base("no_change", Some(project.clone())) },
        Outcome::Applied { project, notes, summary, .. } => Activity { summary: Some(summary.clone()), notes: notes.clone(), ..base("recorded", Some(project.clone())) },
        Outcome::Skipped { reason } => Activity { summary: Some(reason.clone()), ..base("skipped", known_project) },
        Outcome::Failed { project, message, gave_up, .. } => Activity { error: Some(message.clone()), ..base(if *gave_up { "failed" } else { "retry_scheduled" }, project.clone()) },
        Outcome::Unindexed { project, message, .. } => Activity { error: Some(message.clone()), ..base("retry_scheduled", Some(project.clone())) },
        Outcome::CaughtUp => return None,
    })
}

/// Do the next piece of work: a batch the user approved comes before a session still to be read.
pub fn step(queue: &Queue, settings: &Settings, paths: &Paths, ask: Ask, index: &dyn Index, reporter: &dyn Reporter) -> Result<Option<(Work, Outcome)>> {
    let (work, outcome) = if let Some(batch) = queue.next_batch(now())? {
        let outcome = match write(queue, settings, paths, ask, index, &batch, reporter) {
            Ok(outcome) => outcome,
            Err(error) => batch_failed(queue, settings, &batch, error)?,
        };
        (Work::Apply(batch), outcome)
    } else if let Some(job) = queue.next_job(now())? {
        let outcome = match offer(queue, settings, paths, ask, index, &job, reporter) {
            Ok(outcome) => outcome,
            Err(error) => {
                let project = queue.get(&job.id)?.and_then(|current| current.project);
                let failure = queue.record_failure(&job.id, error.failure, &error.message, now())?;
                Outcome::Failed { project, message: error.message, gave_up: failure.gave_up, next_attempt_at: failure.next_attempt_at }
            }
        };
        (Work::Session(job), outcome)
    } else {
        return Ok(None);
    };
    let fallback = Provider::parse(&settings.backend).unwrap_or(Provider::Claude);
    if let Some(entry) = activity(&work, &outcome, fallback) {
        queue.add_activity(&entry)?;
    }
    if outcome != Outcome::CaughtUp {
        reporter.finished(&work, &outcome);
    }
    Ok(Some((work, outcome)))
}

/// Approve the candidates that have waited the set number of days. Returns how many there were.
pub fn approve_overdue(queue: &Queue, settings: &Settings, now: i64) -> Result<usize> {
    if !settings.auto_approve {
        return Ok(0);
    }
    let waited = settings.auto_approve_days.max(1) as i64 * 86_400;
    // The days count from the later of two times: when the candidate entered the list, and when
    // automatic approval was turned on. Turning it on approves nothing at once.
    if settings.auto_approve_since > now - waited {
        return Ok(0);
    }
    let overdue = queue.candidates_waiting_since(now - waited)?;
    if !overdue.is_empty() {
        queue.approve_candidates(&overdue, true, now)?;
    }
    Ok(overdue.len())
}

/// The activity entries of one cleanup run share this id, so the folders of a run are listed together.
pub fn cleanup_run_id(run: i64) -> String {
    format!("cleanup-{run}")
}

pub fn cleanup_activity(provider: Provider, target: &Target, run: i64, result: &Result<cleanup::Outcome>, at: i64) -> Activity {
    let base = Activity { id: 0, at, job_id: cleanup_run_id(run), provider, project: Some(target.folder.clone()), kind: "cleaned".into(), summary: None, notes: Vec::new(), error: None };
    match result {
        Ok(outcome) => Activity { summary: Some(summary_with_log(&outcome.summary, &outcome.log)), notes: outcome.touched(), error: outcome.incomplete.clone(), ..base },
        Err(error) => Activity { kind: "cleanup_failed".into(), error: Some(error.message.clone()), ..base },
    }
}

fn clean_folder(settings: &Settings, paths: &Paths, qmd: &Qmd, target: &Target, ask: cleanup::Ask) -> Result<cleanup::Outcome> {
    if !qmd.available() {
        return Err(Error::transient("qmd was not found, so the cleanup model was not called. Run lore-cli doctor to check the executable path."));
    }
    let root = settings.knowledge_root();
    let artifacts = paths.reviews().join(format!("cleanup-{}-{}", target.folder, now()));
    let result = cleanup::clean(&root, target, &artifacts, ask);
    prune_reviews(&paths.reviews());
    let mut outcome = result?;
    if !outcome.touched().is_empty() {
        // The notes are already cleaned and committed; a search index that could not be refreshed is refreshed by the next recording.
        if let Err(error) = qmd.ensure_index(&root).and_then(|()| qmd.refresh()) {
            outcome.incomplete.get_or_insert(format!("The notes were cleaned, but the search index was not refreshed: {error}"));
        }
    }
    Ok(outcome)
}

/// Tidy one folder of notes with the recording model, commit the result, and list it in the
/// activity under the run it belongs to. The caller holds the same gate as a recording, so the
/// two never write at once.
/// Nothing else rewrites the notes while a batch that was interrupted in the middle of writing
/// them is neither finished nor taken back.
fn notes_free(queue: &Queue) -> Result<()> {
    if queue.write_interrupted()? {
        return Err(Error::transient("Approved candidates were being written into the notes when Lore stopped. That is finished first; try again afterwards."));
    }
    Ok(())
}

pub fn clean(queue: &Queue, settings: &Settings, paths: &Paths, qmd: &Qmd, target: &Target, run: i64, ask: cleanup::Ask) -> Result<cleanup::Outcome> {
    notes_free(queue)?;
    let result = clean_folder(settings, paths, qmd, target, ask);
    let provider = Provider::parse(&settings.backend).unwrap_or(Provider::Claude);
    queue.add_activity(&cleanup_activity(provider, target, run, &result, now()))?;
    // What the run came to is remembered, so that the folder is not counted as never cleaned.
    match &result {
        Ok(outcome) => queue.cleanup_finished(&target.folder, now(), !outcome.touched().is_empty(), outcome.incomplete.as_deref())?,
        Err(error) => queue.cleanup_failed(&target.folder, now(), &error.message)?,
    }
    result
}

/// Undo the last cleanup of a folder and list that in the activity. Returns a note about the
/// search index when it could not be refreshed; the notes themselves are back either way.
pub fn undo_cleanup(queue: &Queue, settings: &Settings, qmd: &Qmd, folder: &str) -> Result<Option<String>> {
    notes_free(queue)?;
    let root = settings.knowledge_root();
    let notes = cleanup::undo(&root, folder)?;
    let stale = qmd.ensure_index(&root).and_then(|()| qmd.refresh()).err().map(|error| format!("The notes are back, but the search index was not refreshed: {error}"));
    let provider = Provider::parse(&settings.backend).unwrap_or(Provider::Claude);
    let summary = Some("The cleanup of this folder was undone.".to_string());
    queue.add_activity(&Activity { id: 0, at: now(), job_id: "cleanup-undo".into(), provider, project: Some(folder.to_string()), kind: "cleanup_undone".into(), summary, notes, error: stale.clone() })?;
    Ok(stale)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Discarded {
    Batch { after_line: usize, end_line: usize },
    Session { reason: String },
}

/// Pass over the part of a session that kept failing, so that the rest of it can be read.
pub fn discard(queue: &Queue, settings: &Settings, paths: &Paths, id: &str, now: i64) -> Result<Option<Discarded>> {
    let Some(job) = queue.get(id)?.filter(|job| job.status == Status::Failed) else { return Ok(None) };
    let batch = match session::read(&job, settings.max_chars, &paths.recorder_workdir()) {
        Ok(Reading::Ready(session)) => Ok((session.batch.end_line, session.end_hash.clone(), session.has_more())),
        Ok(Reading::AgentOnly { end_line, end_hash, has_more }) => Ok((end_line, end_hash, has_more)),
        Ok(Reading::NothingNew { end_line, end_hash }) => Ok((end_line, end_hash, false)),
        Ok(Reading::Skip(reason)) => Err(reason),
        Err(error) if error.failure == Failure::Permanent => Err(error.message),
        Err(error) => return Err(error),
    };
    Ok(Some(match batch {
        Ok((end_line, end_hash, has_more)) => {
            queue.finish(id, end_line, &end_hash, has_more, job.requests, now)?;
            Discarded::Batch { after_line: job.cursor_line, end_line }
        }
        Err(reason) => {
            queue.skip(id, &reason, now)?;
            Discarded::Session { reason }
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::{NoteAction, State};
    use crate::queue::MAX_REJECTED_ATTEMPTS;
    use crate::transcript::{compressed_path, prefix_hash};
    use serde_json::{json, Value};
    use std::cell::RefCell;
    use std::fs;
    use std::path::PathBuf;

    struct Fixture {
        dir: tempfile::TempDir,
        queue: Queue,
        settings: Settings,
        paths: Paths,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("demo").join(".git")).unwrap();
        let queue = Queue::open(&dir.path().join("state.sqlite")).unwrap();
        let settings = Settings { max_chars: 1_000, knowledge_root: dir.path().join("memory").to_string_lossy().to_string(), ..Settings::default() };
        let paths = Paths { root: dir.path().join("app") };
        Fixture { dir, queue, settings, paths }
    }

    fn session(fixture: &Fixture, name: &str, turns: usize) -> PathBuf {
        let cwd = fixture.dir.path().join("demo");
        let mut text = String::new();
        for index in 0..turns {
            let body = index.to_string().repeat(400);
            let mut record = if index % 2 == 0 {
                json!({"type": "user", "uuid": format!("u{index}"), "userType": "external", "timestamp": "2026-11-05T12:00:00Z", "message": {"content": body}})
            } else {
                json!({"type": "assistant", "uuid": format!("a{index}"), "message": {"content": [{"type": "text", "text": body}]}})
            };
            record["cwd"] = json!(cwd.to_string_lossy());
            record["sessionId"] = json!("sess-1");
            text.push_str(&format!("{record}\n"));
        }
        let path = fixture.dir.path().join(name);
        fs::write(&path, text).unwrap();
        path
    }

    /// A session in which the user decides one thing.
    fn decided(fixture: &Fixture) -> PathBuf {
        let cwd = fixture.dir.path().join("demo");
        let lines = [
            json!({"type": "user", "uuid": "u1", "userType": "external", "timestamp": "2026-11-05T12:00:00Z", "message": {"content": "Finished jobs get no desktop alert."}}),
            json!({"type": "assistant", "uuid": "a1", "message": {"content": [{"type": "text", "text": "Removed the alert."}]}}),
        ];
        let mut text = String::new();
        for mut record in lines {
            record["cwd"] = json!(cwd.to_string_lossy());
            record["sessionId"] = json!("sess-1");
            text.push_str(&format!("{record}\n"));
        }
        let path = fixture.dir.path().join("decided.jsonl");
        fs::write(&path, text).unwrap();
        path
    }

    fn fail(queue: &Queue, id: &str) {
        for _ in 0..MAX_REJECTED_ATTEMPTS {
            queue.record_failure(id, Failure::Rejected, "The evidence location or quotation does not match the provided source.", 100).unwrap();
        }
        assert_eq!(queue.get(id).unwrap().unwrap().status, Status::Failed);
    }

    fn failed(fixture: &Fixture, transcript: &Path) -> String {
        let id = fixture.queue.enqueue(Provider::Claude, transcript, false, 100).unwrap();
        fail(&fixture.queue, &id);
        id
    }

    fn discard_job(fixture: &Fixture, id: &str) -> Result<Option<Discarded>> {
        discard(&fixture.queue, &fixture.settings, &fixture.paths, id, 200)
    }

    /// A search index that is there, finds nothing, and counts how often it was refreshed.
    #[derive(Default)]
    struct FakeIndex {
        missing: bool,
        refreshed: RefCell<usize>,
        /// How many refreshes fail before one succeeds.
        failing: RefCell<usize>,
    }

    impl Index for FakeIndex {
        fn available(&self) -> bool {
            !self.missing
        }
        fn search(&self, _: &str, _: &[String], _: usize) -> Vec<String> {
            Vec::new()
        }
        fn reindex(&self, _: &Path) -> Result<()> {
            let mut failing = self.failing.borrow_mut();
            if *failing > 0 {
                *failing -= 1;
                return Err(Error::transient("qmd could not refresh the index."));
            }
            *self.refreshed.borrow_mut() += 1;
            Ok(())
        }
    }

    #[derive(Default)]
    struct Seen {
        started: RefCell<Vec<String>>,
        finished: RefCell<Vec<Outcome>>,
        ended: RefCell<Vec<(String, usize)>>,
    }

    impl Reporter for Seen {
        fn started(&self, work: &Work, project: &str) {
            self.started.borrow_mut().push(format!("{}:{project}", work.id()));
        }
        fn finished(&self, _: &Work, outcome: &Outcome) {
            self.finished.borrow_mut().push(outcome.clone());
        }
        fn session_ended(&self, project: &str, candidates: usize) {
            self.ended.borrow_mut().push((project.to_string(), candidates));
        }
    }

    fn extraction() -> Value {
        json!({"language": "English", "candidates": [{"kind": "decision", "text": "Finished jobs raise no desktop alert.", "scope": "project", "mark": -1, "evidence": [{"ref": "L1", "quote": "Finished jobs get no desktop alert."}]}]})
    }

    fn new_note() -> Value {
        json!({"summary": "Recorded the alert decision.", "removals": [], "changes": [{"path": "projects/demo/alerts.md", "title": "No desktop alert for finished jobs", "body": "Finished jobs raise no desktop alert (user decision, 2026-11-05).", "edits": [], "basis": [1]}]})
    }

    fn run(fixture: &Fixture, index: &FakeIndex, seen: &Seen, answers: Vec<Value>) -> Option<(Work, Outcome)> {
        let answers = RefCell::new(answers);
        let mut ask = |_: &str, _: &Value, _: &Path| {
            let mut left = answers.borrow_mut();
            if left.is_empty() {
                panic!("the model was asked more often than expected");
            }
            Ok(left.remove(0))
        };
        step(&fixture.queue, &fixture.settings, &fixture.paths, &mut ask, index, seen).unwrap()
    }

    #[test]
    fn a_session_offers_candidates_and_only_approval_writes_a_note() {
        let fixture = fixture();
        let (index, seen) = (FakeIndex::default(), Seen::default());
        let job = fixture.queue.enqueue(Provider::Claude, &decided(&fixture), true, 100).unwrap();
        let (_, outcome) = run(&fixture, &index, &seen, vec![extraction()]).unwrap();
        assert_eq!(outcome, Outcome::Offered { project: "demo".into(), candidates: 1, left_out: 0 });
        // The session has ended and all of it is read, so the user is told.
        assert_eq!(seen.ended.borrow().as_slice(), [("demo".to_string(), 1)]);
        let waiting = fixture.queue.waiting_candidates().unwrap();
        assert_eq!((waiting[0].text.as_str(), waiting[0].project.as_str(), waiting[0].language.as_str()), ("Finished jobs raise no desktop alert.", "demo", "English"));
        let stored = fixture.queue.get(&job).unwrap().unwrap();
        assert_eq!((stored.status, stored.cursor_line, stored.ended), (Status::Done, 2, false));
        let note = fixture.settings.knowledge_root().join("projects").join("demo").join("alerts.md");
        assert!(!note.exists());
        // With nothing approved there is nothing to do.
        assert!(run(&fixture, &index, &seen, Vec::new()).is_none());

        fixture.queue.approve_candidates(&[waiting[0].id], false, 200).unwrap();
        let (work, outcome) = run(&fixture, &index, &seen, vec![new_note()]).unwrap();
        assert_eq!(outcome, Outcome::Applied { project: "demo".into(), notes: vec!["projects/demo/alerts.md".into()], summary: "Recorded the alert decision.".into(), candidates: 1, auto: false });
        let text = fs::read_to_string(&note).unwrap();
        assert!(text.contains("# No desktop alert for finished jobs") && text.contains("authority: \"user_decision\""));
        // The source line carries the day the words were said, as the clock of this PC showed it.
        let said = session::local_stamp("2026-11-05T12:00:00Z", &chrono::Local);
        assert!(text.ends_with(&format!("## Sources\n- {}, user: \"Finished jobs get no desktop alert.\"\n", &said[..10])), "{text}");
        assert_eq!(*index.refreshed.borrow(), 1);
        assert_eq!(fixture.queue.candidate(waiting[0].id).unwrap().unwrap().state, State::Applied);
        let applied = &fixture.queue.applied_batches(1).unwrap()[0];
        assert_eq!(applied.result.as_ref().unwrap().notes[0].action, NoteAction::Created);
        let recent = fixture.queue.recent_activity(5).unwrap();
        assert_eq!(recent.iter().map(|entry| (entry.kind.as_str(), entry.job_id.clone())).collect::<Vec<_>>(), vec![("recorded", work.id()), ("offered", job.clone())]);
        assert_eq!(recent[1].summary.as_deref(), Some("1 candidate to review"));
        assert_eq!(seen.started.borrow().as_slice(), [format!("{job}:demo"), format!("{}:demo", work.id())]);
    }

    #[test]
    fn a_session_with_nothing_to_remember_finishes_without_candidates() {
        let fixture = fixture();
        let (index, seen) = (FakeIndex::default(), Seen::default());
        fixture.queue.enqueue(Provider::Claude, &decided(&fixture), false, 100).unwrap();
        let (_, outcome) = run(&fixture, &index, &seen, vec![json!({"language": "English", "candidates": []})]).unwrap();
        assert_eq!(outcome, Outcome::Nothing { project: "demo".into() });
        assert!(fixture.queue.waiting_candidates().unwrap().is_empty());
        assert_eq!(fixture.queue.recent_activity(1).unwrap()[0].kind, "no_change");
    }

    #[test]
    fn an_answer_the_program_rejects_is_retried_with_the_reason_and_writes_nothing() {
        let fixture = fixture();
        let (index, seen) = (FakeIndex::default(), Seen::default());
        let job = fixture.queue.enqueue(Provider::Claude, &decided(&fixture), false, 100).unwrap();
        let invented = json!({"language": "English", "candidates": [{"kind": "decision", "text": "No alerts.", "scope": "project", "mark": -1, "evidence": [{"ref": "L1", "quote": "never alert me"}]}]});
        let (_, outcome) = run(&fixture, &index, &seen, vec![invented]).unwrap();
        assert!(matches!(&outcome, Outcome::Failed { message, gave_up: false, .. } if message.contains("does not match")), "{outcome:?}");
        assert!(fixture.queue.waiting_candidates().unwrap().is_empty());
        let stored = fixture.queue.get(&job).unwrap().unwrap();
        assert_eq!((stored.status, stored.cursor_line, stored.attempts), (Status::Pending, 0, 1));
    }

    /// A candidate offered, approved, and its plan made: the state of a batch about to write.
    fn planned(fixture: &Fixture, index: &FakeIndex, seen: &Seen, auto: bool) -> (i64, i64, Plan) {
        fixture.queue.enqueue(Provider::Claude, &decided(fixture), false, 100).unwrap();
        run(fixture, index, seen, vec![extraction()]).unwrap();
        let id = fixture.queue.waiting_candidates().unwrap()[0].id;
        let batch = fixture.queue.approve_candidates(&[id], auto, 200).unwrap()[0];
        let members = fixture.queue.batch_candidates(batch).unwrap();
        let mut ask = |_: &str, _: &Value, _: &Path| Ok(new_note());
        let plan = integrate::plan("demo", &members, &Default::default(), None, Path::new("/tmp/a"), &mut ask, &|_| Vec::new()).unwrap();
        (id, batch, plan)
    }

    #[test]
    fn a_batch_interrupted_before_its_notes_were_written_is_finished_from_its_plan() {
        let fixture = fixture();
        let (index, seen) = (FakeIndex::default(), Seen::default());
        let (_, batch, plan) = planned(&fixture, &index, &seen, true);
        fixture.queue.begin_batch(batch, &serde_json::to_string(&Writing { plan, reviewed: Vec::new() }).unwrap(), 300).unwrap();
        // The model is not asked again: `run` panics if it is.
        let (_, outcome) = run(&fixture, &index, &seen, Vec::new()).unwrap();
        assert!(matches!(outcome, Outcome::Applied { auto: true, .. }), "{outcome:?}");
        assert!(fixture.settings.knowledge_root().join("projects/demo/alerts.md").exists());
    }

    #[test]
    fn a_batch_whose_notes_are_written_stays_written_while_the_index_cannot_be_refreshed() {
        let fixture = fixture();
        let (index, seen) = (FakeIndex { failing: RefCell::new(1), ..FakeIndex::default() }, Seen::default());
        fixture.queue.enqueue(Provider::Claude, &decided(&fixture), false, 100).unwrap();
        run(&fixture, &index, &seen, vec![extraction()]).unwrap();
        let id = fixture.queue.waiting_candidates().unwrap()[0].id;
        let batch = fixture.queue.approve_candidates(&[id], false, 200).unwrap()[0];
        let (_, outcome) = run(&fixture, &index, &seen, vec![new_note()]).unwrap();
        assert!(matches!(&outcome, Outcome::Unindexed { message, .. } if message.starts_with("The notes are written") && message.contains("refresh the index")), "{outcome:?}");
        assert_eq!(fixture.queue.recent_activity(1).unwrap()[0].kind, "retry_scheduled");
        // The note is in the folder, so the candidate is applied and the batch is not a failed one:
        // it can be neither tried from the start again nor released, which would write it twice.
        let note = fixture.settings.knowledge_root().join("projects/demo/alerts.md");
        let written = fs::read_to_string(&note).unwrap();
        assert_eq!(fixture.queue.candidate(id).unwrap().unwrap().state, State::Applied);
        let stored = fixture.queue.batch(batch).unwrap().unwrap();
        assert_eq!((stored.status, stored.attempts), (ApplyStatus::IndexPending, 1));
        assert_eq!(stored.result.as_ref().unwrap().notes[0].path, "projects/demo/alerts.md");
        assert!(fixture.queue.failed_batches().unwrap().is_empty() && !fixture.queue.retry_batch(batch, 300).unwrap() && !fixture.queue.release_batch(batch, 300).unwrap());
        // When it is due again only the index is refreshed: the model is not asked and the note is not rewritten.
        fixture.queue.connection.execute("UPDATE applies SET next_attempt_at = 0", []).unwrap();
        let (_, outcome) = run(&fixture, &index, &seen, Vec::new()).unwrap();
        assert_eq!(outcome, Outcome::Applied { project: "demo".into(), notes: vec!["projects/demo/alerts.md".into()], summary: "Recorded the alert decision.".into(), candidates: 1, auto: false });
        assert_eq!(fs::read_to_string(&note).unwrap(), written);
        assert_eq!((fixture.queue.batch(batch).unwrap().unwrap().status, *index.refreshed.borrow()), (ApplyStatus::Done, 1));
    }

    #[test]
    fn a_batch_that_gives_up_while_writing_takes_back_what_it_wrote() {
        let fixture = fixture();
        let (index, seen) = (FakeIndex::default(), Seen::default());
        let (id, batch, mut plan) = planned(&fixture, &index, &seen, false);
        // The plan also edits a note. The process died after it wrote the new note, and before
        // it came back someone edited the other one by hand.
        let folder = fixture.settings.knowledge_root().join("projects").join("demo");
        fs::create_dir_all(&folder).unwrap();
        plan.base.insert("projects/demo/style.md".into(), "# Style\n\nold\n".into());
        plan.rendered.insert("projects/demo/style.md".into(), "# Style\n\nnew\n".into());
        fs::write(folder.join("alerts.md"), &plan.rendered["projects/demo/alerts.md"]).unwrap();
        fs::write(folder.join("style.md"), "# Style\n\nedited by hand\n").unwrap();
        fixture.queue.begin_batch(batch, &serde_json::to_string(&Writing { plan, reviewed: Vec::new() }).unwrap(), 300).unwrap();
        assert!(fixture.queue.write_interrupted().unwrap());

        let (_, outcome) = run(&fixture, &index, &seen, Vec::new()).unwrap();
        assert!(matches!(&outcome, Outcome::Failed { message, gave_up: true, .. } if message.contains("changed after")), "{outcome:?}");
        // Nothing of the batch is left in the notes, so it can be tried again or released.
        assert!(!folder.join("alerts.md").exists());
        assert_eq!(fs::read_to_string(folder.join("style.md")).unwrap(), "# Style\n\nedited by hand\n");
        let stored = fixture.queue.batch(batch).unwrap().unwrap();
        assert_eq!((stored.status, stored.manifest), (ApplyStatus::Failed, None));
        assert!(!fixture.queue.write_interrupted().unwrap());
        assert_eq!(fixture.queue.candidate(id).unwrap().unwrap().state, State::Approved);
        assert!(fixture.queue.retry_batch(batch, 400).unwrap());
        let (_, outcome) = run(&fixture, &index, &seen, vec![new_note()]).unwrap();
        assert!(matches!(outcome, Outcome::Applied { .. }), "{outcome:?}");
        assert!(folder.join("alerts.md").exists());
    }

    fn git(root: &Path, args: &[&str]) {
        let output = std::process::Command::new("git").args(["-c", "commit.gpgsign=false", "-c", "core.autocrlf=false"]).args(args).current_dir(root).output().unwrap();
        assert!(output.status.success(), "{args:?}: {}", String::from_utf8_lossy(&output.stderr));
    }

    fn waiting_notes(root: &Path) -> Vec<String> {
        review::working_changes(root).unwrap().unwrap().notes.into_iter().map(|change| change.path).collect()
    }

    #[test]
    fn what_the_user_approved_counts_as_reviewed_but_a_note_that_already_waited_keeps_waiting() {
        let fixture = fixture();
        let (index, seen) = (FakeIndex::default(), Seen::default());
        let root = fixture.settings.knowledge_root();
        let folder = root.join("projects").join("demo");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("style.md"), "# Style\n\nold\n").unwrap();
        git(&root, &["init", "-q", "-b", "main"]);
        // Git for Windows converts line endings by default; the notes are compared as written.
        git(&root, &["config", "core.autocrlf", "false"]);
        git(&root, &["config", "user.name", "Worker test"]);
        git(&root, &["config", "user.email", "worker@example.invalid"]);
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "-q", "-m", "Start"]);
        review::ensure_reviewed(&root).unwrap();
        // A cleanup changed this note, and nobody has looked at that change yet.
        fs::write(folder.join("style.md"), "# Style\n\ncleaned up\n").unwrap();
        review::record_commit(&root).unwrap();
        assert_eq!(waiting_notes(&root), vec!["projects/demo/style.md"]);

        // The user approves a candidate. Its plan writes a new note and also edits that note.
        let (_, batch, mut plan) = planned(&fixture, &index, &seen, false);
        plan.base.insert("projects/demo/style.md".into(), "# Style\n\ncleaned up\n".into());
        plan.rendered.insert("projects/demo/style.md".into(), "# Style\n\ncleaned up, and approved\n".into());
        let reviewed: Vec<String> = plan.touched().into_iter().filter(|note| !waiting_for_review(&root).contains(note)).collect();
        assert_eq!(reviewed, vec!["projects/demo/alerts.md"]);
        fixture.queue.begin_batch(batch, &serde_json::to_string(&Writing { plan, reviewed }).unwrap(), 300).unwrap();
        let (_, outcome) = run(&fixture, &index, &seen, Vec::new()).unwrap();
        assert!(matches!(outcome, Outcome::Applied { auto: false, .. }), "{outcome:?}");
        // The new note is reviewed: the user approved what it says. The other still waits, with
        // the cleanup's change that the user has not looked at.
        assert_eq!(waiting_notes(&root), vec!["projects/demo/style.md"]);
        assert!(review::working_changes(&root).unwrap().unwrap().others.is_empty());
        assert_eq!(fs::read_to_string(folder.join("alerts.md")).unwrap().lines().nth(8), Some("# No desktop alert for finished jobs"));
    }

    #[test]
    fn the_user_is_told_once_what_a_session_offered_when_it_has_ended_and_is_read_to_its_end() {
        let fixture = fixture();
        let (index, seen) = (FakeIndex::default(), Seen::default());
        let transcript = decided(&fixture);
        // A compaction: the session goes on, so nothing is announced yet.
        let job = fixture.queue.enqueue(Provider::Claude, &transcript, false, 100).unwrap();
        run(&fixture, &index, &seen, vec![extraction()]).unwrap();
        assert!(seen.ended.borrow().is_empty());
        // The session ends without another word. What the earlier step found is announced now.
        fixture.queue.enqueue(Provider::Claude, &transcript, true, 200).unwrap();
        let (_, outcome) = run(&fixture, &index, &seen, Vec::new()).unwrap();
        assert_eq!(outcome, Outcome::CaughtUp);
        assert_eq!(seen.ended.borrow().as_slice(), [("demo".to_string(), 1)]);
        assert_eq!(fixture.queue.get(&job).unwrap().unwrap().status, Status::Done);
        // The session is resumed and ended again with nothing new: nothing is announced twice.
        fixture.queue.enqueue(Provider::Claude, &transcript, true, 300).unwrap();
        run(&fixture, &index, &seen, Vec::new()).unwrap();
        assert_eq!(seen.ended.borrow().len(), 1);
    }

    #[test]
    fn a_session_that_ends_while_one_of_its_steps_is_with_the_model_is_read_again() {
        let fixture = fixture();
        let (index, seen) = (FakeIndex::default(), Seen::default());
        let transcript = decided(&fixture);
        let job = fixture.queue.enqueue(Provider::Claude, &transcript, false, 100).unwrap();
        // While the model answers, the user says one more thing and the session ends.
        let mut ask = |_: &str, _: &Value, _: &Path| {
            let more = json!({"type": "user", "uuid": "u2", "userType": "external", "timestamp": "2026-11-05T12:05:00Z", "cwd": fixture.dir.path().join("demo").to_string_lossy(), "sessionId": "sess-1", "message": {"content": "Also keep the status panel."}});
            let mut text = fs::read_to_string(&transcript).unwrap();
            text.push_str(&format!("{more}\n"));
            fs::write(&transcript, text).unwrap();
            fixture.queue.enqueue(Provider::Claude, &transcript, true, 150).unwrap();
            Ok(extraction())
        };
        step(&fixture.queue, &fixture.settings, &fixture.paths, &mut ask, &index, &seen).unwrap().unwrap();
        let open = fixture.queue.get(&job).unwrap().unwrap();
        assert_eq!((open.status, open.cursor_line, open.ended), (Status::Pending, 2, true));
        assert!(seen.ended.borrow().is_empty());
        // The next step reads the line that was added and then tells the user of both candidates.
        let second = json!({"language": "English", "candidates": [{"kind": "decision", "text": "The status panel stays.", "scope": "project", "mark": -1, "evidence": [{"ref": "L3", "quote": "Also keep the status panel."}]}]});
        let (_, outcome) = run(&fixture, &index, &seen, vec![second]).unwrap();
        assert_eq!(outcome, Outcome::Offered { project: "demo".into(), candidates: 1, left_out: 0 });
        assert_eq!(seen.ended.borrow().as_slice(), [("demo".to_string(), 2)]);
        assert_eq!(fixture.queue.get(&job).unwrap().unwrap().status, Status::Done);
    }

    #[test]
    fn an_apply_that_fails_keeps_the_candidates_approved_for_a_retry() {
        let fixture = fixture();
        let (index, seen) = (FakeIndex::default(), Seen::default());
        fixture.queue.enqueue(Provider::Claude, &decided(&fixture), false, 100).unwrap();
        run(&fixture, &index, &seen, vec![extraction()]).unwrap();
        let id = fixture.queue.waiting_candidates().unwrap()[0].id;
        let batch = fixture.queue.approve_candidates(&[id], false, 200).unwrap()[0];
        let unused = json!({"summary": "Nothing.", "changes": [], "removals": []});
        let (_, outcome) = run(&fixture, &index, &seen, vec![unused]).unwrap();
        assert!(matches!(&outcome, Outcome::Failed { project: Some(project), message, gave_up: false, .. } if project == "demo" && message.contains("Left out: 1")), "{outcome:?}");
        assert_eq!(fixture.queue.candidate(id).unwrap().unwrap().state, State::Approved);
        let stored = fixture.queue.batch(batch).unwrap().unwrap();
        assert_eq!((stored.status, stored.attempts), (ApplyStatus::Pending, 1));
        assert!(stored.last_error.unwrap().contains("Left out"));
    }

    #[test]
    fn candidates_are_approved_automatically_only_when_that_is_on_and_they_waited_long_enough() {
        let fixture = fixture();
        let (index, seen) = (FakeIndex::default(), Seen::default());
        fixture.queue.enqueue(Provider::Claude, &decided(&fixture), false, 100).unwrap();
        run(&fixture, &index, &seen, vec![extraction()]).unwrap();
        let found = fixture.queue.waiting_candidates().unwrap()[0].created_at;
        let day = 86_400;
        assert_eq!(approve_overdue(&fixture.queue, &fixture.settings, found + 30 * day).unwrap(), 0);
        // Turned on after the candidate had waited ten days: it is not approved at once, but
        // three days after the setting was turned on.
        let mut late = fixture.settings.clone();
        late.set_auto_approve(true, 3, found + 10 * day);
        assert_eq!(late.auto_approve_since, found + 10 * day);
        assert_eq!(approve_overdue(&fixture.queue, &late, found + 10 * day).unwrap(), 0);
        assert_eq!(approve_overdue(&fixture.queue, &late, found + 12 * day).unwrap(), 0);
        // Changing the days keeps the time it was turned on; turning it off forgets it.
        late.set_auto_approve(true, 2, found + 11 * day);
        assert_eq!((late.auto_approve_days, late.auto_approve_since), (2, found + 10 * day));
        let mut off = late.clone();
        off.set_auto_approve(false, 2, found + 11 * day);
        assert_eq!(off.auto_approve_since, 0);
        // On since before the candidate was found: the days count from when it was found.
        let on = Settings { auto_approve: true, auto_approve_days: 3, auto_approve_since: found - day, ..fixture.settings.clone() };
        assert_eq!(approve_overdue(&fixture.queue, &on, found + 2 * day).unwrap(), 0);
        assert_eq!(approve_overdue(&fixture.queue, &on, found + 3 * day).unwrap(), 1);
        assert!(fixture.queue.waiting_candidates().unwrap().is_empty());
        assert!(fixture.queue.next_batch(found + 3 * day).unwrap().unwrap().auto);
    }

    #[test]
    fn missing_qmd_is_reported_before_any_model_is_called() {
        let fixture = fixture();
        let (index, seen) = (FakeIndex { missing: true, ..FakeIndex::default() }, Seen::default());
        fixture.queue.enqueue(Provider::Claude, &session(&fixture, "a.jsonl", 2), false, 100).unwrap();
        let (_, outcome) = run(&fixture, &index, &seen, Vec::new()).unwrap();
        assert!(matches!(outcome, Outcome::Failed { message, .. } if message.contains("qmd")));
        assert!(seen.started.borrow().is_empty());
        assert!(!fixture.paths.reviews().exists());
    }

    #[test]
    fn discarding_skips_only_the_failed_batch_and_queues_the_rest_of_the_session() {
        let fixture = fixture();
        let transcript = session(&fixture, "a.jsonl", 4);
        let id = failed(&fixture, &transcript);
        assert_eq!(discard_job(&fixture, &id).unwrap(), Some(Discarded::Batch { after_line: 0, end_line: 2 }));
        let job = fixture.queue.get(&id).unwrap().unwrap();
        assert_eq!((job.status, job.cursor_line, job.attempts, job.last_error), (Status::Pending, 2, 0, None));
        assert_eq!(job.cursor_hash, prefix_hash(&fs::read(&transcript).unwrap(), 2));
        assert_eq!(fixture.queue.next_job(200).unwrap().unwrap().id, id);
    }

    #[test]
    fn discarding_the_last_batch_finishes_the_job() {
        let fixture = fixture();
        let transcript = session(&fixture, "a.jsonl", 4);
        let id = failed(&fixture, &transcript);
        discard_job(&fixture, &id).unwrap();
        fail(&fixture.queue, &id);
        assert_eq!(discard_job(&fixture, &id).unwrap(), Some(Discarded::Batch { after_line: 2, end_line: 4 }));
        let job = fixture.queue.get(&id).unwrap().unwrap();
        assert_eq!((job.status, job.cursor_line), (Status::Done, 4));
    }

    #[test]
    fn a_transcript_that_cannot_be_read_drops_the_whole_session() {
        let fixture = fixture();
        let missing = failed(&fixture, &fixture.dir.path().join("missing.jsonl"));
        let broken_path = fixture.dir.path().join("broken.jsonl");
        fs::write(&broken_path, "not json\n").unwrap();
        let broken = failed(&fixture, &broken_path);
        let changed = fixture.queue.enqueue(Provider::Claude, &session(&fixture, "changed.jsonl", 4), false, 100).unwrap();
        fixture.queue.finish(&changed, 2, "stale", true, 1, 100).unwrap();
        fail(&fixture.queue, &changed);

        for (id, cause) in [(missing, "Transcript file not found"), (broken, "invalid JSON"), (changed, "previously processed source prefix has changed")] {
            match discard_job(&fixture, &id).unwrap() {
                Some(Discarded::Session { reason }) => assert!(reason.contains(cause), "{reason}"),
                other => panic!("{id}: {other:?}"),
            }
            let job = fixture.queue.get(&id).unwrap().unwrap();
            assert_eq!(job.status, Status::Skipped);
        }
    }

    #[test]
    fn a_transcript_that_fails_to_decompress_leaves_the_job_failed() {
        let fixture = fixture();
        let path = fixture.dir.path().join("packed.jsonl");
        fs::write(compressed_path(&path), "not zstd").unwrap();
        let id = failed(&fixture, &path);
        assert_eq!(discard_job(&fixture, &id).unwrap_err().failure, Failure::Transient);
        assert_eq!(fixture.queue.get(&id).unwrap().unwrap().status, Status::Failed);
    }

    #[test]
    fn only_failed_jobs_are_discarded() {
        let fixture = fixture();
        let id = fixture.queue.enqueue(Provider::Claude, &session(&fixture, "a.jsonl", 4), false, 100).unwrap();
        assert_eq!(discard_job(&fixture, &id).unwrap(), None);
        let job = fixture.queue.get(&id).unwrap().unwrap();
        assert_eq!((job.status, job.cursor_line), (Status::Pending, 0));
        assert_eq!(discard_job(&fixture, "unknown").unwrap(), None);
    }

    fn demo_folder() -> Target {
        Target { folder: "demo".into(), absorb: vec![] }
    }

    #[test]
    fn a_cleanup_without_qmd_never_calls_the_model_and_is_listed_as_failed() {
        let fixture = fixture();
        let qmd = Qmd::new(&fixture.settings, &fixture.paths.recorder_workdir(), crate::run::fixed(5));
        let error = clean(&fixture.queue, &fixture.settings, &fixture.paths, &qmd, &demo_folder(), 700, &mut |_, _, _| panic!("the model must not run without qmd")).unwrap_err();
        assert!(error.message.contains("qmd"), "{error}");
        let recent = fixture.queue.recent_activity(5).unwrap();
        assert_eq!((recent[0].kind.as_str(), recent[0].project.as_deref(), recent[0].job_id.as_str()), ("cleanup_failed", Some("demo"), "cleanup-700"));
        assert!(recent[0].error.as_deref().unwrap().contains("qmd"));
        assert!(!fixture.paths.reviews().exists());
        // The failure is remembered for the folder, without counting as a finished cleanup.
        let check = &fixture.queue.cleanup_checks().unwrap()["demo"];
        assert!(check.at == 0 && check.failed_at > 0 && check.error.as_deref().unwrap().contains("qmd"), "{check:?}");
    }

    #[test]
    fn a_cleanup_is_listed_with_every_note_it_touched_and_a_line_for_each_one_it_removed() {
        let outcome = cleanup::Outcome {
            summary: "팝업 노트를 합쳤습니다.".into(),
            written: vec!["projects/demo/bg.md".into()],
            removed: vec!["projects/demo/popup.md".into()],
            log: vec!["Removed projects/demo/popup.md: BG 노트에 들어 있습니다.".into()],
            incomplete: None,
        };
        let entry = cleanup_activity(Provider::Codex, &demo_folder(), 90, &Ok(outcome.clone()), 100);
        assert_eq!((entry.kind.as_str(), entry.provider, entry.at, entry.project.as_deref(), entry.job_id.as_str()), ("cleaned", Provider::Codex, 100, Some("demo"), "cleanup-90"));
        assert_eq!(entry.notes, vec!["projects/demo/bg.md".to_string(), "projects/demo/popup.md".to_string()]);
        assert_eq!(entry.summary.as_deref(), Some("팝업 노트를 합쳤습니다.\nRemoved projects/demo/popup.md: BG 노트에 들어 있습니다."));
        assert_eq!(entry.error, None);

        let stopped = cleanup_activity(Provider::Claude, &demo_folder(), 90, &Ok(cleanup::Outcome { incomplete: Some("usage limit reached".into()), ..outcome }), 100);
        assert_eq!((stopped.kind.as_str(), stopped.error.as_deref()), ("cleaned", Some("usage limit reached")));
        let failed = cleanup_activity(Provider::Claude, &demo_folder(), 90, &Err(Error::rejected("Invalid memory path: x")), 100);
        assert_eq!((failed.kind.as_str(), failed.error.as_deref(), failed.notes.len()), ("cleanup_failed", Some("Invalid memory path: x"), 0));
    }
}
