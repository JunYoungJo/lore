use std::path::Path;
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;

use crate::error::{Error, Failure, Result};
use crate::transcript::{canonical_path, digest, Provider};

pub const MAX_TRANSIENT_ATTEMPTS: i64 = 8;
pub const MAX_REJECTED_ATTEMPTS: i64 = 3;
const ACTIVITY_KEPT: i64 = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pending,
    Applying,
    IndexPending,
    Done,
    Failed,
    Skipped,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Applying => "applying",
            Self::IndexPending => "index_pending",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }

    fn parse(value: &str) -> Self {
        match value {
            "applying" => Self::Applying,
            "index_pending" => Self::IndexPending,
            "done" => Self::Done,
            "failed" => Self::Failed,
            "skipped" => Self::Skipped,
            _ => Self::Pending,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub provider: Provider,
    pub transcript: String,
    pub status: Status,
    pub cursor_line: usize,
    pub cursor_hash: String,
    pub project: Option<String>,
    pub session_id: Option<String>,
    pub attempts: i64,
    pub next_attempt_at: i64,
    #[serde(skip)]
    pub manifest: Option<String>,
    pub last_error: Option<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub ready: i64,
    pub waiting: i64,
    pub failed: i64,
    pub done: i64,
    pub skipped: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    pub id: i64,
    pub at: i64,
    pub job_id: String,
    pub provider: Provider,
    pub project: Option<String>,
    pub kind: String,
    pub summary: Option<String>,
    pub notes: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureOutcome {
    pub gave_up: bool,
    pub attempts: i64,
    pub next_attempt_at: i64,
}

pub struct Queue {
    connection: Connection,
}

const JOB_COLUMNS: &str = "id, provider, transcript, status, cursor_line, cursor_hash, project, session_id, attempts, next_attempt_at, manifest, last_error, updated_at";

fn job_from_row(row: &Row) -> rusqlite::Result<Job> {
    let provider: String = row.get(1)?;
    let status: String = row.get(3)?;
    let cursor: i64 = row.get(4)?;
    Ok(Job {
        id: row.get(0)?,
        provider: Provider::parse(&provider).unwrap_or(Provider::Claude),
        transcript: row.get(2)?,
        status: Status::parse(&status),
        cursor_line: cursor.max(0) as usize,
        cursor_hash: row.get(5)?,
        project: row.get(6)?,
        session_id: row.get(7)?,
        attempts: row.get(8)?,
        next_attempt_at: row.get(9)?,
        manifest: row.get(10)?,
        last_error: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

pub fn job_id(provider: Provider, transcript: &Path) -> String {
    let key = transcript.to_string_lossy().replace('/', "\\").to_lowercase();
    digest(format!("{}:{key}", provider.as_str()).as_bytes())[..16].to_string()
}

pub fn backoff_seconds(attempts: i64) -> i64 {
    let exponent = (attempts - 1).clamp(0, 6) as u32;
    (60 * 2_i64.pow(exponent)).min(3600)
}

impl Queue {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS jobs (
                id TEXT PRIMARY KEY,
                provider TEXT NOT NULL,
                transcript TEXT NOT NULL,
                status TEXT NOT NULL,
                cursor_line INTEGER NOT NULL DEFAULT 0,
                cursor_hash TEXT NOT NULL DEFAULT '',
                project TEXT,
                session_id TEXT,
                attempts INTEGER NOT NULL DEFAULT 0,
                next_attempt_at INTEGER NOT NULL DEFAULT 0,
                manifest TEXT,
                last_error TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS activity (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                at INTEGER NOT NULL,
                job_id TEXT NOT NULL,
                provider TEXT NOT NULL,
                project TEXT,
                kind TEXT NOT NULL,
                summary TEXT,
                notes TEXT NOT NULL DEFAULT '[]',
                error TEXT
            );
            CREATE TABLE IF NOT EXISTS seen_files (
                path TEXT PRIMARY KEY,
                size INTEGER NOT NULL,
                modified INTEGER NOT NULL
            );",
        )?;
        Ok(Self { connection })
    }

    pub fn enqueue(&self, provider: Provider, transcript: &Path, now: i64) -> Result<String> {
        let path = std::path::absolute(canonical_path(transcript))?;
        let id = job_id(provider, &path);
        self.connection.execute(
            "INSERT INTO jobs (id, provider, transcript, status, created_at, updated_at) VALUES (?1, ?2, ?3, 'pending', ?4, ?4)
             ON CONFLICT(id) DO UPDATE SET status = CASE WHEN jobs.status = 'done' THEN 'pending' ELSE jobs.status END, updated_at = ?4",
            params![id, provider.as_str(), path.to_string_lossy(), now],
        )?;
        Ok(id)
    }

    pub fn get(&self, id: &str) -> Result<Option<Job>> {
        let sql = format!("SELECT {JOB_COLUMNS} FROM jobs WHERE id = ?1");
        Ok(self.connection.query_row(&sql, params![id], job_from_row).optional()?)
    }

    pub fn next_job(&self, now: i64) -> Result<Option<Job>> {
        let sql = format!(
            "SELECT {JOB_COLUMNS} FROM jobs
             WHERE status IN ('pending', 'applying', 'index_pending') AND next_attempt_at <= ?1
             ORDER BY CASE status WHEN 'pending' THEN 1 ELSE 0 END, next_attempt_at, rowid LIMIT 1"
        );
        Ok(self.connection.query_row(&sql, params![now], job_from_row).optional()?)
    }

    pub fn record_failure(&self, id: &str, failure: Failure, message: &str, now: i64) -> Result<FailureOutcome> {
        let job = self.get(id)?.ok_or_else(|| Error::permanent(format!("작업이 없습니다: {id}")))?;
        let attempts = job.attempts + 1;
        let gave_up = match failure {
            Failure::Permanent => true,
            Failure::Rejected => attempts >= MAX_REJECTED_ATTEMPTS,
            Failure::Transient => attempts >= MAX_TRANSIENT_ATTEMPTS,
        };
        let next_attempt_at = if gave_up { 0 } else { now + backoff_seconds(attempts) };
        let status = if gave_up { Status::Failed } else { job.status };
        self.connection.execute(
            "UPDATE jobs SET status = ?2, attempts = ?3, next_attempt_at = ?4, last_error = ?5, updated_at = ?6 WHERE id = ?1",
            params![id, status.as_str(), attempts, next_attempt_at, message, now],
        )?;
        Ok(FailureOutcome { gave_up, attempts, next_attempt_at })
    }

    pub fn set_session(&self, id: &str, project: Option<&str>, session_id: &str) -> Result<()> {
        self.connection.execute("UPDATE jobs SET project = ?2, session_id = ?3 WHERE id = ?1", params![id, project, session_id])?;
        Ok(())
    }

    pub fn begin_apply(&self, id: &str, manifest: &str, now: i64) -> Result<()> {
        self.connection.execute(
            "UPDATE jobs SET status = 'applying', manifest = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, manifest, now],
        )?;
        Ok(())
    }

    pub fn mark_index_pending(&self, id: &str, now: i64) -> Result<()> {
        self.connection.execute("UPDATE jobs SET status = 'index_pending', updated_at = ?2 WHERE id = ?1", params![id, now])?;
        Ok(())
    }

    pub fn finish(&self, id: &str, end_line: usize, end_hash: &str, has_more: bool, now: i64) -> Result<()> {
        let status = if has_more { Status::Pending } else { Status::Done };
        self.connection.execute(
            "UPDATE jobs SET cursor_line = ?2, cursor_hash = ?3, status = ?4, attempts = 0, next_attempt_at = 0, manifest = NULL, last_error = NULL, updated_at = ?5
             WHERE id = ?1",
            params![id, end_line as i64, end_hash, status.as_str(), now],
        )?;
        Ok(())
    }

    pub fn skip(&self, id: &str, reason: &str, now: i64) -> Result<()> {
        self.connection.execute(
            "UPDATE jobs SET status = 'skipped', last_error = ?2, attempts = 0, next_attempt_at = 0, manifest = NULL, updated_at = ?3 WHERE id = ?1",
            params![id, reason, now],
        )?;
        Ok(())
    }

    pub fn retry(&self, id: &str, now: i64) -> Result<bool> {
        let changed = self.connection.execute(
            "UPDATE jobs SET status = 'pending', attempts = 0, next_attempt_at = 0, manifest = NULL, last_error = NULL, updated_at = ?2
             WHERE id = ?1 AND status IN ('failed', 'skipped')",
            params![id, now],
        )?;
        Ok(changed > 0)
    }

    pub fn counts(&self, now: i64) -> Result<Counts> {
        Ok(self.connection.query_row(
            "SELECT
                COALESCE(SUM(status IN ('pending', 'applying', 'index_pending') AND next_attempt_at <= ?1), 0),
                COALESCE(SUM(status IN ('pending', 'applying', 'index_pending') AND next_attempt_at > ?1), 0),
                COALESCE(SUM(status = 'failed'), 0),
                COALESCE(SUM(status = 'done'), 0),
                COALESCE(SUM(status = 'skipped'), 0)
             FROM jobs",
            params![now],
            |row| Ok(Counts { ready: row.get(0)?, waiting: row.get(1)?, failed: row.get(2)?, done: row.get(3)?, skipped: row.get(4)? }),
        )?)
    }

    pub fn jobs_with_status(&self, statuses: &[Status], limit: usize) -> Result<Vec<Job>> {
        let list = statuses.iter().map(|status| format!("'{}'", status.as_str())).collect::<Vec<_>>().join(", ");
        let sql = format!("SELECT {JOB_COLUMNS} FROM jobs WHERE status IN ({list}) ORDER BY updated_at DESC LIMIT ?1");
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement.query_map(params![limit as i64], job_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn add_activity(&self, activity: &Activity) -> Result<()> {
        self.connection.execute(
            "INSERT INTO activity (at, job_id, provider, project, kind, summary, notes, error) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                activity.at,
                activity.job_id,
                activity.provider.as_str(),
                activity.project,
                activity.kind,
                activity.summary,
                serde_json::to_string(&activity.notes)?,
                activity.error
            ],
        )?;
        let last = self.connection.last_insert_rowid();
        self.connection.execute("DELETE FROM activity WHERE id <= ?1", params![last - ACTIVITY_KEPT])?;
        Ok(())
    }

    pub fn recent_activity(&self, limit: usize) -> Result<Vec<Activity>> {
        let mut statement = self.connection.prepare(
            "SELECT id, at, job_id, provider, project, kind, summary, notes, error FROM activity ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = statement.query_map(params![limit as i64], |row| {
            let provider: String = row.get(3)?;
            let notes: String = row.get(7)?;
            Ok(Activity {
                id: row.get(0)?,
                at: row.get(1)?,
                job_id: row.get(2)?,
                provider: Provider::parse(&provider).unwrap_or(Provider::Claude),
                project: row.get(4)?,
                kind: row.get(5)?,
                summary: row.get(6)?,
                notes: serde_json::from_str(&notes).unwrap_or_default(),
                error: row.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn seen(&self, path: &str) -> Result<Option<(i64, i64)>> {
        Ok(self
            .connection
            .query_row("SELECT size, modified FROM seen_files WHERE path = ?1", params![path], |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()?)
    }

    pub fn mark_seen(&self, path: &str, size: i64, modified: i64) -> Result<()> {
        self.connection.execute(
            "INSERT INTO seen_files (path, size, modified) VALUES (?1, ?2, ?3) ON CONFLICT(path) DO UPDATE SET size = ?2, modified = ?3",
            params![path, size, modified],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue() -> (tempfile::TempDir, Queue) {
        let dir = tempfile::tempdir().unwrap();
        let queue = Queue::open(&dir.path().join("state.sqlite")).unwrap();
        (dir, queue)
    }

    fn add(queue: &Queue, dir: &tempfile::TempDir, name: &str) -> String {
        queue.enqueue(Provider::Claude, &dir.path().join(name), 100).unwrap()
    }

    #[test]
    fn permanently_failed_job_no_longer_blocks_the_next_one() {
        let (dir, queue) = queue();
        let first = add(&queue, &dir, "oversized.jsonl");
        let second = add(&queue, &dir, "normal.jsonl");
        assert_eq!(queue.next_job(100).unwrap().unwrap().id, first);
        let outcome = queue.record_failure(&first, Failure::Permanent, "원문 파일이 없습니다", 100).unwrap();
        assert!(outcome.gave_up);
        for _ in 0..3 {
            assert_eq!(queue.next_job(100).unwrap().unwrap().id, second);
        }
        assert_eq!(queue.get(&first).unwrap().unwrap().status, Status::Failed);
    }

    #[test]
    fn transient_failure_waits_and_lets_later_jobs_run_first() {
        let (dir, queue) = queue();
        let first = add(&queue, &dir, "a.jsonl");
        let second = add(&queue, &dir, "b.jsonl");
        let outcome = queue.record_failure(&first, Failure::Transient, "모델 시간 초과", 100).unwrap();
        assert_eq!(outcome, FailureOutcome { gave_up: false, attempts: 1, next_attempt_at: 160 });
        assert_eq!(queue.next_job(159).unwrap().unwrap().id, second);
        queue.finish(&second, 3, "h", false, 159).unwrap();
        assert!(queue.next_job(159).unwrap().is_none());
        assert_eq!(queue.next_job(160).unwrap().unwrap().id, first);
    }

    #[test]
    fn repeated_failures_give_up_at_their_limits() {
        let (dir, queue) = queue();
        let rejected = add(&queue, &dir, "rejected.jsonl");
        let outcomes: Vec<bool> = (0..MAX_REJECTED_ATTEMPTS).map(|_| queue.record_failure(&rejected, Failure::Rejected, "검증 실패", 100).unwrap().gave_up).collect();
        assert_eq!(outcomes, vec![false, false, true]);
        let transient = add(&queue, &dir, "transient.jsonl");
        let outcomes: Vec<bool> = (0..MAX_TRANSIENT_ATTEMPTS).map(|_| queue.record_failure(&transient, Failure::Transient, "실행 실패", 100).unwrap().gave_up).collect();
        assert_eq!(outcomes.iter().filter(|gave_up| **gave_up).count(), 1);
        assert!(outcomes[MAX_TRANSIENT_ATTEMPTS as usize - 1]);
    }

    #[test]
    fn backoff_doubles_up_to_an_hour() {
        let delays: Vec<i64> = (1..=9).map(backoff_seconds).collect();
        assert_eq!(delays, vec![60, 120, 240, 480, 960, 1920, 3600, 3600, 3600]);
    }

    #[test]
    fn interrupted_apply_resumes_before_new_work() {
        let (dir, queue) = queue();
        let waiting = add(&queue, &dir, "a.jsonl");
        let applying = add(&queue, &dir, "b.jsonl");
        queue.begin_apply(&applying, "{}", 100).unwrap();
        assert_eq!(queue.next_job(100).unwrap().unwrap().id, applying);
        queue.record_failure(&applying, Failure::Transient, "qmd 실패", 100).unwrap();
        assert_eq!(queue.get(&applying).unwrap().unwrap().status, Status::Applying);
        assert_eq!(queue.next_job(100).unwrap().unwrap().id, waiting);
    }

    #[test]
    fn enqueue_reopens_finished_jobs_but_keeps_failed_and_skipped_until_retry() {
        let (dir, queue) = queue();
        let done = add(&queue, &dir, "done.jsonl");
        queue.finish(&done, 5, "h", false, 100).unwrap();
        add(&queue, &dir, "done.jsonl");
        let reopened = queue.get(&done).unwrap().unwrap();
        assert_eq!((reopened.status, reopened.cursor_line), (Status::Pending, 5));
        let failed = add(&queue, &dir, "failed.jsonl");
        queue.record_failure(&failed, Failure::Permanent, "없음", 100).unwrap();
        add(&queue, &dir, "failed.jsonl");
        assert_eq!(queue.get(&failed).unwrap().unwrap().status, Status::Failed);
        assert!(queue.retry(&failed, 101).unwrap());
        let retried = queue.get(&failed).unwrap().unwrap();
        assert_eq!((retried.status, retried.attempts, retried.last_error), (Status::Pending, 0, None));
        assert!(!queue.retry(&done, 101).unwrap());
    }

    #[test]
    fn finish_keeps_the_job_open_when_more_lines_remain() {
        let (dir, queue) = queue();
        let id = add(&queue, &dir, "a.jsonl");
        queue.finish(&id, 4, "h4", true, 100).unwrap();
        let job = queue.get(&id).unwrap().unwrap();
        assert_eq!((job.status, job.cursor_line, job.cursor_hash.as_str()), (Status::Pending, 4, "h4"));
    }

    #[test]
    fn same_transcript_maps_to_one_job_regardless_of_case_separator_or_compression() {
        let (dir, queue) = queue();
        let lower = dir.path().join("Session.jsonl");
        let first = queue.enqueue(Provider::Codex, &lower, 100).unwrap();
        let upper = std::path::PathBuf::from(lower.to_string_lossy().to_uppercase().replace(".JSONL", ".jsonl"));
        assert_eq!(queue.enqueue(Provider::Codex, &upper, 100).unwrap(), first);
        assert_eq!(queue.enqueue(Provider::Codex, &crate::transcript::compressed_path(&lower), 100).unwrap(), first);
        assert_ne!(queue.enqueue(Provider::Claude, &lower, 100).unwrap(), first);
        assert_eq!(queue.counts(100).unwrap().ready, 2);
    }

    #[test]
    fn counts_split_ready_from_waiting_and_activity_is_listed_newest_first() {
        let (dir, queue) = queue();
        let a = add(&queue, &dir, "a.jsonl");
        add(&queue, &dir, "b.jsonl");
        queue.record_failure(&a, Failure::Transient, "x", 100).unwrap();
        assert_eq!(queue.counts(100).unwrap(), Counts { ready: 1, waiting: 1, failed: 0, done: 0, skipped: 0 });
        for kind in ["started", "recorded"] {
            queue
                .add_activity(&Activity { id: 0, at: 100, job_id: a.clone(), provider: Provider::Claude, project: Some("demo".into()), kind: kind.into(), summary: None, notes: vec!["projects/demo/a.md".into()], error: None })
                .unwrap();
        }
        let recent = queue.recent_activity(10).unwrap();
        assert_eq!(recent.iter().map(|a| a.kind.as_str()).collect::<Vec<_>>(), vec!["recorded", "started"]);
        assert_eq!(recent[0].notes, vec!["projects/demo/a.md".to_string()]);
    }
}
