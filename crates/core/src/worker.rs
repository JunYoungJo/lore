use std::fs::File;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::cleanup::{self, Target};
use crate::error::{Error, Failure, Result};
use crate::notes;
use crate::pipeline::{self, Plan, Preparation};
use crate::proposal::summary_with_log;
use crate::qmd::{collection_name, Qmd};
use crate::queue::{Activity, Job, Queue, Status};
use crate::recorder::Recorder;
use crate::settings::{Paths, Settings};
use crate::transcript::Provider;

const REVIEWS_KEPT: usize = 50;

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|elapsed| elapsed.as_secs() as i64).unwrap_or(0)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    Recorded { project: String, notes: Vec<String>, summary: String },
    NoChange { project: String, summary: String },
    Skipped { reason: String },
    CaughtUp,
    Failed { project: Option<String>, message: String, gave_up: bool, next_attempt_at: i64 },
}

pub trait Reporter {
    fn started(&self, job: &Job, project: &str);
    fn finished(&self, job: &Job, outcome: &Outcome);
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

fn finish_plan(queue: &Queue, qmd: &Qmd, settings: &Settings, job: &Job, plan: &Plan, resumed: bool) -> Result<Outcome> {
    let root = settings.knowledge_root();
    if job.status != Status::IndexPending {
        pipeline::apply(&root, plan)?;
        queue.mark_index_pending(&job.id, now())?;
    }
    qmd.ensure_index(&root)?;
    qmd.refresh()?;
    queue.finish(&job.id, plan.end_line, &plan.end_hash, plan.has_more || resumed, now())?;
    Ok(Outcome::Recorded { project: plan.project.clone(), notes: plan.touched(), summary: plan.summary.clone() })
}

fn process(queue: &Queue, settings: &Settings, paths: &Paths, recorder: &dyn Recorder, qmd: &Qmd, job: &Job, reporter: &dyn Reporter) -> Result<Outcome> {
    if matches!(job.status, Status::Applying | Status::IndexPending) {
        let manifest = job.manifest.as_deref().ok_or_else(|| Error::permanent("No interrupted apply state is available."))?;
        let plan: Plan = serde_json::from_str(manifest).map_err(|error| Error::permanent(format!("Could not read interrupted apply state: {error}")))?;
        return finish_plan(queue, qmd, settings, job, &plan, true);
    }
    let prepared = match pipeline::prepare(job, settings.max_chars, &paths.recorder_workdir())? {
        Preparation::Skip(reason) => {
            queue.skip(&job.id, &reason, now())?;
            return Ok(Outcome::Skipped { reason });
        }
        Preparation::NothingNew { end_line, end_hash } => {
            queue.finish(&job.id, end_line, &end_hash, false, now())?;
            return Ok(Outcome::CaughtUp);
        }
        Preparation::AgentOnly { end_line, end_hash, has_more } => {
            queue.finish(&job.id, end_line, &end_hash, has_more, now())?;
            return Ok(Outcome::CaughtUp);
        }
        Preparation::Ready(prepared) => prepared,
    };
    queue.set_session(&job.id, Some(&prepared.project), &prepared.parsed.session_id)?;
    if !qmd.available() {
        return Err(Error::transient("qmd was not found, so the recording model was not called. Run lore-cli doctor to check the executable path."));
    }
    reporter.started(job, &prepared.project);
    let root = settings.knowledge_root();
    let scope = notes::load_scope(&root, &prepared.project)?;
    let total: usize = scope.values().map(|content| content.chars().count()).sum();
    let ranked = if total > settings.notes_budget {
        let collections = vec!["common".to_string(), collection_name(&prepared.project)];
        qmd.rank(&pipeline::ranking_query(&prepared.batch), &collections, 20).unwrap_or_default()
    } else {
        Vec::new()
    };
    let artifacts = paths.reviews().join(format!("{}-{}", job.id, prepared.batch.end_line));
    let mut request = pipeline::after_failure(pipeline::build_request(&prepared, &scope, &ranked, &[], settings.notes_budget), job);
    let mut proposal = recorder.record(&request.payload, &artifacts)?;
    if proposal.changes.is_empty() && !proposal.needs_notes.is_empty() {
        request = pipeline::after_failure(pipeline::build_request(&prepared, &scope, &ranked, &proposal.needs_notes, settings.notes_budget), job);
        proposal = recorder.record(&request.payload, &artifacts.join("followup"))?;
        if proposal.changes.is_empty() && !proposal.needs_notes.is_empty() {
            return Err(Error::rejected("The model requested the existing note body again."));
        }
    }
    let plan = pipeline::plan(&prepared, &request, &proposal, &root, &job.id)?;
    prune_reviews(&paths.reviews());
    if plan.touched().is_empty() {
        queue.finish(&job.id, plan.end_line, &plan.end_hash, plan.has_more, now())?;
        return Ok(Outcome::NoChange { project: plan.project, summary: plan.summary });
    }
    queue.begin_apply(&job.id, &serde_json::to_string(&plan)?, now())?;
    let applying = Job { status: Status::Applying, ..job.clone() };
    finish_plan(queue, qmd, settings, &applying, &plan, false)
}

fn activity(job: &Job, outcome: &Outcome) -> Option<Activity> {
    let base = |kind: &str, project: Option<String>| Activity {
        id: 0,
        at: now(),
        job_id: job.id.clone(),
        provider: job.provider,
        project,
        kind: kind.into(),
        summary: None,
        notes: Vec::new(),
        error: None,
    };
    Some(match outcome {
        Outcome::Recorded { project, notes, summary } => Activity { summary: Some(summary.clone()), notes: notes.clone(), ..base("recorded", Some(project.clone())) },
        Outcome::NoChange { project, summary } => Activity { summary: Some(summary.clone()), ..base("no_change", Some(project.clone())) },
        Outcome::Skipped { reason } => Activity { summary: Some(reason.clone()), ..base("skipped", job.project.clone()) },
        Outcome::Failed { project, message, gave_up, .. } => {
            Activity { error: Some(message.clone()), ..base(if *gave_up { "failed" } else { "retry_scheduled" }, project.clone()) }
        }
        Outcome::CaughtUp => return None,
    })
}

pub fn step(queue: &Queue, settings: &Settings, paths: &Paths, recorder: &dyn Recorder, qmd: &Qmd, reporter: &dyn Reporter) -> Result<Option<(Job, Outcome)>> {
    let Some(job) = queue.next_job(now())? else { return Ok(None) };
    let outcome = match process(queue, settings, paths, recorder, qmd, &job, reporter) {
        Ok(outcome) => outcome,
        Err(error) => {
            let project = queue.get(&job.id)?.and_then(|current| current.project);
            let failure = queue.record_failure(&job.id, error.failure, &error.message, now())?;
            Outcome::Failed { project, message: error.message, gave_up: failure.gave_up, next_attempt_at: failure.next_attempt_at }
        }
    };
    if let Some(entry) = activity(&job, &outcome) {
        queue.add_activity(&entry)?;
    }
    // Recorded notes are committed right away so sync never waits for a manual
    // commit. A failure here is reported by the next sync, which commits again.
    if matches!(outcome, Outcome::Recorded { .. }) {
        if let Err(error) = crate::review::record_commit(&settings.knowledge_root()) {
            eprintln!("Could not commit the recorded notes: {error}");
        }
    }
    if outcome != Outcome::CaughtUp {
        reporter.finished(&job, &outcome);
    }
    Ok(Some((job, outcome)))
}

pub fn cleanup_activity(provider: Provider, target: &Target, result: &Result<cleanup::Outcome>, at: i64) -> Activity {
    let base = Activity { id: 0, at, job_id: "cleanup".into(), provider, project: Some(target.folder.clone()), kind: "cleaned".into(), summary: None, notes: Vec::new(), error: None };
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
/// activity. The caller holds the same gate as a recording, so the two never write at once.
pub fn clean(queue: &Queue, settings: &Settings, paths: &Paths, qmd: &Qmd, target: &Target, ask: cleanup::Ask) -> Result<cleanup::Outcome> {
    let result = clean_folder(settings, paths, qmd, target, ask);
    let provider = Provider::parse(&settings.backend).unwrap_or(Provider::Claude);
    queue.add_activity(&cleanup_activity(provider, target, &result, now()))?;
    result
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Discarded {
    Batch { after_line: usize, end_line: usize },
    Session { reason: String },
}

pub fn discard(queue: &Queue, settings: &Settings, paths: &Paths, id: &str, now: i64) -> Result<Option<Discarded>> {
    let Some(job) = queue.get(id)?.filter(|job| job.status == Status::Failed) else { return Ok(None) };
    let batch = match pipeline::prepare(&job, settings.max_chars, &paths.recorder_workdir()) {
        Ok(Preparation::Ready(prepared)) => Ok((prepared.batch.end_line, prepared.end_hash.clone(), prepared.parsed.complete_lines > prepared.batch.end_line)),
        Ok(Preparation::AgentOnly { end_line, end_hash, has_more }) => Ok((end_line, end_hash, has_more)),
        Ok(Preparation::NothingNew { end_line, end_hash }) => Ok((end_line, end_hash, false)),
        Ok(Preparation::Skip(reason)) => Err(reason),
        Err(error) if error.failure == Failure::Permanent => Err(error.message),
        Err(error) => return Err(error),
    };
    Ok(Some(match batch {
        Ok((end_line, end_hash, has_more)) => {
            queue.finish(id, end_line, &end_hash, has_more, now)?;
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
    use crate::queue::MAX_REJECTED_ATTEMPTS;
    use crate::transcript::{compressed_path, prefix_hash};
    use serde_json::json;
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
        let settings = Settings { max_chars: 1_000, ..Settings::default() };
        let paths = Paths { root: dir.path().join("app") };
        Fixture { dir, queue, settings, paths }
    }

    fn session(fixture: &Fixture, name: &str, turns: usize) -> PathBuf {
        let cwd = fixture.dir.path().join("demo");
        let mut text = String::new();
        for index in 0..turns {
            let body = index.to_string().repeat(400);
            let mut record = if index % 2 == 0 {
                json!({"type": "user", "uuid": format!("u{index}"), "userType": "external", "timestamp": "2026-11-05T00:00:00Z", "message": {"content": body}})
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

    fn fail(queue: &Queue, id: &str) {
        for _ in 0..MAX_REJECTED_ATTEMPTS {
            queue.record_failure(id, Failure::Rejected, "The model requested the existing note body again.", 100).unwrap();
        }
        assert_eq!(queue.get(id).unwrap().unwrap().status, Status::Failed);
    }

    fn failed(fixture: &Fixture, transcript: &Path) -> String {
        let id = fixture.queue.enqueue(Provider::Claude, transcript, 100).unwrap();
        fail(&fixture.queue, &id);
        id
    }

    fn discard_job(fixture: &Fixture, id: &str) -> Result<Option<Discarded>> {
        discard(&fixture.queue, &fixture.settings, &fixture.paths, id, 200)
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
        let changed = fixture.queue.enqueue(Provider::Claude, &session(&fixture, "changed.jsonl", 4), 100).unwrap();
        fixture.queue.finish(&changed, 2, "stale", true, 100).unwrap();
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
        let id = fixture.queue.enqueue(Provider::Claude, &session(&fixture, "a.jsonl", 4), 100).unwrap();
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
        let error = clean(&fixture.queue, &fixture.settings, &fixture.paths, &qmd, &demo_folder(), &mut |_, _, _| panic!("the model must not run without qmd")).unwrap_err();
        assert!(error.message.contains("qmd"), "{error}");
        let recent = fixture.queue.recent_activity(5).unwrap();
        assert_eq!((recent[0].kind.as_str(), recent[0].project.as_deref(), recent[0].job_id.as_str()), ("cleanup_failed", Some("demo"), "cleanup"));
        assert!(recent[0].error.as_deref().unwrap().contains("qmd"));
        assert!(!fixture.paths.reviews().exists());
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
        let entry = cleanup_activity(Provider::Codex, &demo_folder(), &Ok(outcome.clone()), 100);
        assert_eq!((entry.kind.as_str(), entry.provider, entry.at, entry.project.as_deref()), ("cleaned", Provider::Codex, 100, Some("demo")));
        assert_eq!(entry.notes, vec!["projects/demo/bg.md".to_string(), "projects/demo/popup.md".to_string()]);
        assert_eq!(entry.summary.as_deref(), Some("팝업 노트를 합쳤습니다.\nRemoved projects/demo/popup.md: BG 노트에 들어 있습니다."));
        assert_eq!(entry.error, None);

        let stopped = cleanup_activity(Provider::Claude, &demo_folder(), &Ok(cleanup::Outcome { incomplete: Some("usage limit reached".into()), ..outcome }), 100);
        assert_eq!((stopped.kind.as_str(), stopped.error.as_deref()), ("cleaned", Some("usage limit reached")));
        let failed = cleanup_activity(Provider::Claude, &demo_folder(), &Err(Error::rejected("Invalid memory path: x")), 100);
        assert_eq!((failed.kind.as_str(), failed.error.as_deref(), failed.notes.len()), ("cleanup_failed", Some("Invalid memory path: x"), 0));
    }

    #[test]
    fn missing_qmd_is_reported_before_any_model_is_called() {
        struct MustNotRun;
        impl Recorder for MustNotRun {
            fn record(&self, _: &serde_json::Value, _: &Path) -> Result<crate::proposal::Proposal> { panic!("model must not run without qmd") }
        }
        impl Reporter for MustNotRun {
            fn started(&self, _: &Job, _: &str) { panic!("recording must not start") }
            fn finished(&self, _: &Job, _: &Outcome) {}
        }
        let fixture = fixture();
        let transcript = session(&fixture, "a.jsonl", 2);
        fixture.queue.enqueue(Provider::Claude, &transcript, 100).unwrap();
        let qmd = Qmd::new(&fixture.settings, &fixture.paths.recorder_workdir(), crate::run::fixed(5));
        let (_, outcome) = step(&fixture.queue, &fixture.settings, &fixture.paths, &MustNotRun, &qmd, &MustNotRun).unwrap().unwrap();
        assert!(matches!(outcome, Outcome::Failed { message, .. } if message.contains("qmd")));
        assert!(!fixture.paths.reviews().exists());
    }
}
