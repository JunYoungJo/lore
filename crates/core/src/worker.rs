use std::fs::File;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::error::{Error, Result};
use crate::notes;
use crate::pipeline::{self, Plan, Preparation};
use crate::qmd::{collection_name, Qmd};
use crate::queue::{Activity, Job, Queue, Status};
use crate::recorder::Recorder;
use crate::settings::{Paths, Settings};

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
    if plan.rendered.keys().any(|note| note.starts_with("projects/")) {
        qmd.ensure_collection(&plan.project, &root.join("projects").join(&plan.project))?;
    }
    qmd.refresh()?;
    queue.finish(&job.id, plan.end_line, &plan.end_hash, plan.has_more || resumed, now())?;
    Ok(Outcome::Recorded { project: plan.project.clone(), notes: plan.rendered.keys().cloned().collect(), summary: plan.summary.clone() })
}

fn process(queue: &Queue, settings: &Settings, paths: &Paths, recorder: &dyn Recorder, job: &Job, reporter: &dyn Reporter) -> Result<Outcome> {
    let qmd = Qmd::new(settings, &paths.recorder_workdir());
    if matches!(job.status, Status::Applying | Status::IndexPending) {
        let manifest = job.manifest.as_deref().ok_or_else(|| Error::permanent("중단된 반영 정보가 없습니다."))?;
        let plan: Plan = serde_json::from_str(manifest).map_err(|error| Error::permanent(format!("중단된 반영 정보를 읽지 못했습니다: {error}")))?;
        return finish_plan(queue, &qmd, settings, job, &plan, true);
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
    let mut request = pipeline::build_request(&prepared, &scope, &ranked, &[], settings.notes_budget);
    let mut proposal = recorder.record(&request.payload, &artifacts)?;
    if proposal.changes.is_empty() && !proposal.needs_notes.is_empty() {
        request = pipeline::build_request(&prepared, &scope, &ranked, &proposal.needs_notes, settings.notes_budget);
        proposal = recorder.record(&request.payload, &artifacts.join("followup"))?;
        if proposal.changes.is_empty() && !proposal.needs_notes.is_empty() {
            return Err(Error::rejected("모델이 기존 문서 본문을 다시 요청했습니다."));
        }
    }
    let plan = pipeline::plan(&prepared, &request, &proposal, &root, &job.id)?;
    prune_reviews(&paths.reviews());
    if plan.rendered.is_empty() {
        queue.finish(&job.id, plan.end_line, &plan.end_hash, plan.has_more, now())?;
        return Ok(Outcome::NoChange { project: plan.project, summary: plan.summary });
    }
    queue.begin_apply(&job.id, &serde_json::to_string(&plan)?, now())?;
    let applying = Job { status: Status::Applying, ..job.clone() };
    finish_plan(queue, &qmd, settings, &applying, &plan, false)
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

pub fn step(queue: &Queue, settings: &Settings, paths: &Paths, recorder: &dyn Recorder, reporter: &dyn Reporter) -> Result<Option<(Job, Outcome)>> {
    let Some(job) = queue.next_job(now())? else { return Ok(None) };
    let outcome = match process(queue, settings, paths, recorder, &job, reporter) {
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
    if outcome != Outcome::CaughtUp {
        reporter.finished(&job, &outcome);
    }
    Ok(Some((job, outcome)))
}
