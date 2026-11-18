//! What a session offers to remember, and where that waits for the user.
//!
//! A candidate is one statement with the words it rests on. It is stored in the app's database
//! on this PC and never synced: the user approves it where the session ran. Approved candidates
//! of one project form an apply batch, which the worker turns into note changes.

use std::collections::BTreeMap;

use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Failure, Result};
use crate::queue::{backoff_seconds, FailureOutcome, Queue, MAX_REJECTED_ATTEMPTS, MAX_TRANSIENT_ATTEMPTS};
use crate::transcript::Provider;

const PRUNE_WRITTEN: &str = "DELETE FROM applies WHERE status = 'done' AND updated_at < ?1";

/// How long a candidate the user has dealt with, and a batch that was written, stay in the database.
const KEPT_SECS: i64 = 30 * 24 * 3600;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Something the user decided.
    Decision,
    /// The user rejected or redirected how the agent worked.
    Correction,
    /// A cause or a way of working that was not obvious.
    Lesson,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Decision, Kind::Correction, Kind::Lesson];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Decision => "decision",
            Self::Correction => "correction",
            Self::Lesson => "lesson",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }
}

/// Who found the candidate: the session marked it while working, or the model found it afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Marked,
    Found,
}

/// Where new content goes: the session's project, or the notes shared by every project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Project,
    Common,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Speaker {
    User,
    Agent,
}

impl Speaker {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Agent => "agent",
        }
    }
}

/// Words of the conversation a candidate rests on. The program has checked them against the turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quote {
    #[serde(rename = "ref")]
    pub reference: String,
    pub quote: String,
    pub speaker: Speaker,
    /// When the turn was said, in the user's local time. Empty when the session file has no time.
    pub at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RelationKind {
    /// No existing note is about this.
    New,
    /// An existing note already says this. Such a candidate is not offered, only counted.
    Same,
    /// An existing note says something this changes.
    Differs,
    /// An existing note is about something this ends.
    Obsolete,
}

/// How a candidate stands to the notes that exist. `quotes` are passages of `note`, word for word.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Relation {
    #[serde(rename = "type")]
    pub kind: RelationKind,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub quotes: Vec<String>,
    /// A digest of the note's text as the user is shown it (`proposal::body_digest`). A note
    /// that reads differently when the candidate is approved was changed in between, and is
    /// then edited instead of removed: the user has not seen what was added to it.
    #[serde(default)]
    pub digest: String,
}

impl Relation {
    pub fn new() -> Self {
        Self { kind: RelationKind::New, note: String::new(), title: String::new(), quotes: Vec::new(), digest: String::new() }
    }

    /// How a candidate stands to an existing note, shown with these passages of it.
    pub fn to(kind: RelationKind, note: &str, title: &str, quotes: Vec<String>, digest: String) -> Self {
        Self { kind, note: note.to_string(), title: title.to_string(), quotes, digest }
    }
}

impl Default for Relation {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Waiting,
    Approved,
    Applied,
    Discarded,
    /// An existing note already says it. Kept to tell the user what was left out.
    Duplicate,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Approved => "approved",
            Self::Applied => "applied",
            Self::Discarded => "discarded",
            Self::Duplicate => "duplicate",
        }
    }

    fn parse(value: &str) -> Self {
        match value {
            "approved" => Self::Approved,
            "applied" => Self::Applied,
            "discarded" => Self::Discarded,
            "duplicate" => Self::Duplicate,
            _ => Self::Waiting,
        }
    }
}

/// A candidate as the preparation found it, before it is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub kind: Kind,
    pub text: String,
    pub source: Source,
    pub scope: Scope,
    pub evidence: Vec<Quote>,
    pub relation: Relation,
}

/// The session a batch of candidates came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin<'a> {
    pub job_id: &'a str,
    pub session_id: &'a str,
    pub provider: Provider,
    pub project: &'a str,
    pub language: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub id: i64,
    pub job_id: String,
    pub session_id: String,
    pub provider: Provider,
    pub project: String,
    pub language: String,
    pub kind: Kind,
    pub text: String,
    pub source: Source,
    pub scope: Scope,
    pub evidence: Vec<Quote>,
    pub relation: Relation,
    pub state: State,
    /// The user changed the sentence.
    pub edited: bool,
    pub created_at: i64,
    /// When it last entered the waiting list: when it was found, or when a batch that could not
    /// be written was given up. The days of automatic approval count from here.
    pub waiting_since: i64,
    pub apply_id: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyStatus {
    Pending,
    Applying,
    IndexPending,
    Done,
    Failed,
}

impl ApplyStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Applying => "applying",
            Self::IndexPending => "index_pending",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }

    fn parse(value: &str) -> Self {
        match value {
            "applying" => Self::Applying,
            "index_pending" => Self::IndexPending,
            "done" => Self::Done,
            "failed" => Self::Failed,
            _ => Self::Pending,
        }
    }
}

/// What became of one note when a batch was applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NoteAction {
    Created,
    Edited,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteResult {
    pub path: String,
    pub action: NoteAction,
    /// The approved sentences this note took in.
    pub items: Vec<String>,
}

/// What an applied batch did, as the window shows it afterwards.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    pub summary: String,
    pub notes: Vec<NoteResult>,
}

/// Approved candidates of one project, to be turned into note changes together.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Batch {
    pub id: i64,
    pub project: String,
    pub status: ApplyStatus,
    /// Approved by the waiting period, not by the user.
    pub auto: bool,
    pub attempts: i64,
    pub next_attempt_at: i64,
    #[serde(skip)]
    pub manifest: Option<String>,
    pub last_error: Option<String>,
    pub result: Option<ApplyResult>,
    pub candidates: usize,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Waiting {
    pub candidates: usize,
    pub sessions: usize,
    /// When the oldest waiting candidate was found. Zero when none waits.
    pub oldest_at: i64,
    /// How many still wait from each session job, so a past "offered" entry can say whether any is left.
    pub by_job: BTreeMap<String, usize>,
}

const CANDIDATE_COLUMNS: &str = "id, job_id, session_id, provider, project, language, kind, text, source, scope, evidence, relation, state, edited, created_at, apply_id, waiting_since";

fn word<T: Serialize>(value: T) -> String {
    serde_json::to_value(value).ok().and_then(|value| value.as_str().map(str::to_string)).unwrap_or_default()
}

fn from_word<T: for<'de> Deserialize<'de>>(value: &str, fallback: T) -> T {
    serde_json::from_value(serde_json::Value::String(value.to_string())).unwrap_or(fallback)
}

fn candidate_from_row(row: &Row) -> rusqlite::Result<Candidate> {
    let provider: String = row.get(3)?;
    let kind: String = row.get(6)?;
    let source: String = row.get(8)?;
    let scope: String = row.get(9)?;
    let evidence: String = row.get(10)?;
    let relation: String = row.get(11)?;
    let state: String = row.get(12)?;
    Ok(Candidate {
        id: row.get(0)?,
        job_id: row.get(1)?,
        session_id: row.get(2)?,
        provider: Provider::parse(&provider).unwrap_or(Provider::Claude),
        project: row.get(4)?,
        language: row.get(5)?,
        kind: Kind::parse(&kind).unwrap_or(Kind::Decision),
        text: row.get(7)?,
        source: from_word(&source, Source::Found),
        scope: from_word(&scope, Scope::Project),
        evidence: serde_json::from_str(&evidence).unwrap_or_default(),
        relation: serde_json::from_str(&relation).unwrap_or_default(),
        state: State::parse(&state),
        edited: row.get(13)?,
        created_at: row.get(14)?,
        waiting_since: row.get(16)?,
        apply_id: row.get(15)?,
    })
}

const BATCH_COLUMNS: &str = "id, project, status, auto, attempts, next_attempt_at, manifest, last_error, result, updated_at, (SELECT COUNT(*) FROM candidates WHERE apply_id = applies.id)";

fn batch_from_row(row: &Row) -> rusqlite::Result<Batch> {
    let status: String = row.get(2)?;
    let result: Option<String> = row.get(8)?;
    let candidates: i64 = row.get(10)?;
    Ok(Batch {
        id: row.get(0)?,
        project: row.get(1)?,
        status: ApplyStatus::parse(&status),
        auto: row.get(3)?,
        attempts: row.get(4)?,
        next_attempt_at: row.get(5)?,
        manifest: row.get(6)?,
        last_error: row.get(7)?,
        result: result.and_then(|text| serde_json::from_str(&text).ok()),
        candidates: candidates.max(0) as usize,
        updated_at: row.get(9)?,
    })
}

pub(crate) const TABLES: &str = "
    CREATE TABLE IF NOT EXISTS candidates (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        job_id TEXT NOT NULL,
        end_line INTEGER NOT NULL,
        session_id TEXT NOT NULL DEFAULT '',
        provider TEXT NOT NULL,
        project TEXT NOT NULL,
        language TEXT NOT NULL DEFAULT '',
        kind TEXT NOT NULL,
        text TEXT NOT NULL,
        source TEXT NOT NULL,
        scope TEXT NOT NULL,
        evidence TEXT NOT NULL DEFAULT '[]',
        relation TEXT NOT NULL DEFAULT '{}',
        state TEXT NOT NULL,
        edited INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL,
        waiting_since INTEGER NOT NULL DEFAULT 0,
        announced INTEGER NOT NULL DEFAULT 0,
        decided_at INTEGER NOT NULL DEFAULT 0,
        apply_id INTEGER
    );
    CREATE INDEX IF NOT EXISTS candidates_state ON candidates (state);
    CREATE TABLE IF NOT EXISTS applies (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        project TEXT NOT NULL,
        status TEXT NOT NULL,
        auto INTEGER NOT NULL DEFAULT 0,
        attempts INTEGER NOT NULL DEFAULT 0,
        next_attempt_at INTEGER NOT NULL DEFAULT 0,
        manifest TEXT,
        last_error TEXT,
        result TEXT,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    );";

impl Queue {
    /// Store what one step of a session offered and move the session's cursor past it, as one
    /// change: a step that is run again after an interruption replaces its own candidates and
    /// leaves alone the ones the user has already decided. `seen_requests` is the job's
    /// `requests` as the step found it. Returns whether the job is done (see `queue::advance`).
    #[allow(clippy::too_many_arguments)]
    pub fn store_candidates(&self, origin: &Origin, end_line: usize, end_hash: &str, has_more: bool, seen_requests: i64, offered: &[Found], duplicates: &[Found], now: i64) -> Result<bool> {
        let transaction = self.connection.unchecked_transaction()?;
        let decided: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM candidates WHERE job_id = ?1 AND end_line = ?2 AND state NOT IN ('waiting', 'duplicate')",
            params![origin.job_id, end_line as i64],
            |row| row.get(0),
        )?;
        if decided == 0 {
            transaction.execute("DELETE FROM candidates WHERE job_id = ?1 AND end_line = ?2", params![origin.job_id, end_line as i64])?;
            for (state, items) in [(State::Waiting, offered), (State::Duplicate, duplicates)] {
                for item in items {
                    transaction.execute(
                        "INSERT INTO candidates (job_id, end_line, session_id, provider, project, language, kind, text, source, scope, evidence, relation, state, created_at, waiting_since)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?14)",
                        params![
                            origin.job_id,
                            end_line as i64,
                            origin.session_id,
                            origin.provider.as_str(),
                            origin.project,
                            origin.language,
                            item.kind.as_str(),
                            item.text,
                            word(item.source),
                            word(item.scope),
                            serde_json::to_string(&item.evidence)?,
                            serde_json::to_string(&item.relation)?,
                            state.as_str(),
                            now
                        ],
                    )?;
                }
            }
        }
        transaction.execute("DELETE FROM candidates WHERE state IN ('applied', 'discarded', 'duplicate') AND created_at < ?1", params![now - KEPT_SECS])?;
        transaction.execute(PRUNE_WRITTEN, params![now - KEPT_SECS])?;
        let done = crate::queue::advance(&transaction, origin.job_id, end_line, end_hash, has_more, seen_requests, now)?;
        transaction.commit()?;
        Ok(done)
    }

    /// The candidates of a session that wait and that the user has not been told of yet. They
    /// are counted once: the next call returns only what was found after this one.
    pub fn announce(&self, job_id: &str) -> Result<usize> {
        Ok(self.connection.execute("UPDATE candidates SET announced = 1 WHERE job_id = ?1 AND state = 'waiting' AND announced = 0", params![job_id])?)
    }

    fn candidates_where(&self, condition: &str) -> Result<Vec<Candidate>> {
        let sql = format!("SELECT {CANDIDATE_COLUMNS} FROM candidates WHERE {condition} ORDER BY created_at, id");
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement.query_map([], candidate_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// The candidates the user has not decided yet, oldest session first.
    pub fn waiting_candidates(&self) -> Result<Vec<Candidate>> {
        self.candidates_where("state = 'waiting'")
    }

    /// What was left out of the sessions that still have candidates waiting, because a note already says it.
    pub fn left_out(&self) -> Result<Vec<Candidate>> {
        self.candidates_where("state = 'duplicate' AND job_id IN (SELECT job_id FROM candidates WHERE state = 'waiting')")
    }

    pub fn candidate(&self, id: i64) -> Result<Option<Candidate>> {
        let sql = format!("SELECT {CANDIDATE_COLUMNS} FROM candidates WHERE id = ?1");
        Ok(self.connection.query_row(&sql, params![id], candidate_from_row).optional()?)
    }

    pub fn waiting(&self) -> Result<Waiting> {
        let mut waiting = self.connection.query_row(
            "SELECT COUNT(*), COUNT(DISTINCT job_id), COALESCE(MIN(created_at), 0) FROM candidates WHERE state = 'waiting'",
            [],
            |row| Ok(Waiting { candidates: row.get::<_, i64>(0)?.max(0) as usize, sessions: row.get::<_, i64>(1)?.max(0) as usize, oldest_at: row.get(2)?, by_job: BTreeMap::new() }),
        )?;
        let mut statement = self.connection.prepare("SELECT job_id, COUNT(*) FROM candidates WHERE state = 'waiting' GROUP BY job_id")?;
        let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?.max(0) as usize)))?;
        waiting.by_job = rows.collect::<rusqlite::Result<_>>()?;
        Ok(waiting)
    }

    /// Change the sentence or the scope of a candidate that still waits. Returns whether it did.
    pub fn edit_candidate(&self, id: i64, text: Option<&str>, scope: Option<Scope>) -> Result<bool> {
        let mut changed = 0;
        if let Some(text) = text.map(str::trim).filter(|text| !text.is_empty()) {
            changed += self.connection.execute("UPDATE candidates SET text = ?2, edited = 1 WHERE id = ?1 AND state = 'waiting' AND text <> ?2", params![id, text])?;
        }
        if let Some(scope) = scope {
            // Where a candidate changes or ends a note, the note decides the folder.
            changed += self.connection.execute(
                "UPDATE candidates SET scope = ?2 WHERE id = ?1 AND state = 'waiting' AND relation LIKE '%\"type\":\"new\"%'",
                params![id, word(scope)],
            )?;
        }
        Ok(changed > 0)
    }

    pub fn discard_candidates(&self, ids: &[i64], now: i64) -> Result<usize> {
        let mut discarded = 0;
        for id in ids {
            discarded += self.connection.execute("UPDATE candidates SET state = 'discarded', decided_at = ?2 WHERE id = ?1 AND state = 'waiting'", params![id, now])?;
        }
        Ok(discarded)
    }

    /// Approve the waiting candidates among `ids`. Each project gets one apply batch. Returns the batches.
    pub fn approve_candidates(&self, ids: &[i64], auto: bool, now: i64) -> Result<Vec<i64>> {
        let transaction = self.connection.unchecked_transaction()?;
        let mut by_project: BTreeMap<String, Vec<i64>> = BTreeMap::new();
        for id in ids {
            let project: Option<String> = transaction.query_row("SELECT project FROM candidates WHERE id = ?1 AND state = 'waiting'", params![id], |row| row.get(0)).optional()?;
            if let Some(project) = project {
                by_project.entry(project).or_default().push(*id);
            }
        }
        let mut batches = Vec::new();
        for (project, members) in by_project {
            transaction.execute("INSERT INTO applies (project, status, auto, created_at, updated_at) VALUES (?1, 'pending', ?2, ?3, ?3)", params![project, auto, now])?;
            let batch = transaction.last_insert_rowid();
            for id in members {
                transaction.execute("UPDATE candidates SET state = 'approved', decided_at = ?3, apply_id = ?2 WHERE id = ?1", params![id, batch, now])?;
            }
            batches.push(batch);
        }
        transaction.commit()?;
        Ok(batches)
    }

    /// The candidates that entered the waiting list at `before` or earlier, for the automatic approval.
    pub fn candidates_waiting_since(&self, before: i64) -> Result<Vec<i64>> {
        let mut statement = self.connection.prepare("SELECT id FROM candidates WHERE state = 'waiting' AND waiting_since <= ?1 ORDER BY id")?;
        let rows = statement.query_map(params![before], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<i64>>>()?)
    }

    pub fn batch(&self, id: i64) -> Result<Option<Batch>> {
        let sql = format!("SELECT {BATCH_COLUMNS} FROM applies WHERE id = ?1");
        Ok(self.connection.query_row(&sql, params![id], batch_from_row).optional()?)
    }

    /// A batch was interrupted after it began to write its notes. Nothing else may change the
    /// notes until it has finished or has been taken back: not a sync, not a cleanup.
    pub fn write_interrupted(&self) -> Result<bool> {
        Ok(self.connection.prepare("SELECT 1 FROM applies WHERE status = 'applying'")?.exists([])?)
    }

    /// The next batch to work on. One that was interrupted while writing goes first.
    pub fn next_batch(&self, now: i64) -> Result<Option<Batch>> {
        let sql = format!(
            "SELECT {BATCH_COLUMNS} FROM applies WHERE status IN ('pending', 'applying', 'index_pending') AND next_attempt_at <= ?1
             ORDER BY CASE status WHEN 'pending' THEN 1 ELSE 0 END, next_attempt_at, id LIMIT 1"
        );
        Ok(self.connection.query_row(&sql, params![now], batch_from_row).optional()?)
    }

    pub fn batch_candidates(&self, batch: i64) -> Result<Vec<Candidate>> {
        let sql = format!("SELECT {CANDIDATE_COLUMNS} FROM candidates WHERE apply_id = ?1 ORDER BY id");
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement.query_map(params![batch], candidate_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// The plan is made: from here the batch writes notes. The plan is kept so that an
    /// interrupted write is finished from it, or taken back with it.
    pub fn begin_batch(&self, id: i64, manifest: &str, now: i64) -> Result<()> {
        self.connection.execute("UPDATE applies SET status = 'applying', manifest = ?2, updated_at = ?3 WHERE id = ?1", params![id, manifest, now])?;
        Ok(())
    }

    /// Every note of the batch is written. From here the batch cannot fail any more: its
    /// candidates are applied and its result is what the user is shown. Only the commit and the
    /// search index are still to do, and they are tried again until they succeed. `reviewed`
    /// are the notes that count as reviewed once committed; they are kept until then.
    pub fn batch_written(&self, id: i64, result: &ApplyResult, reviewed: &[String], now: i64) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "UPDATE applies SET status = 'index_pending', attempts = 0, next_attempt_at = 0, manifest = ?4, last_error = NULL, result = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, serde_json::to_string(result)?, now, serde_json::to_string(reviewed)?],
        )?;
        transaction.execute("UPDATE candidates SET state = 'applied' WHERE apply_id = ?1", params![id])?;
        transaction.commit()?;
        Ok(())
    }

    /// The notes are committed and the search index has them.
    pub fn finish_batch(&self, id: i64, now: i64) -> Result<()> {
        self.connection.execute("UPDATE applies SET status = 'done', attempts = 0, next_attempt_at = 0, manifest = NULL, last_error = NULL, updated_at = ?2 WHERE id = ?1", params![id, now])?;
        Ok(())
    }

    /// Record a failed attempt. A batch whose notes are written never fails for good: only its
    /// index refresh is left, which is tried again at the longest interval. A batch that gives
    /// up has no note of its own left in the folder (the worker takes back what it began to
    /// write before it calls this), so trying it again or releasing it starts from clean notes.
    pub fn batch_failed(&self, id: i64, failure: Failure, message: &str, now: i64) -> Result<FailureOutcome> {
        let batch = self.batch(id)?.ok_or_else(|| Error::permanent(format!("Apply batch not found: {id}")))?;
        let attempts = batch.attempts + 1;
        let gave_up = batch.gives_up(failure);
        let next_attempt_at = if gave_up { 0 } else { now + backoff_seconds(attempts) };
        let status = if gave_up { ApplyStatus::Failed } else { batch.status };
        self.connection.execute(
            "UPDATE applies SET status = ?2, attempts = ?3, next_attempt_at = ?4, last_error = ?5, manifest = CASE WHEN ?2 = 'failed' THEN NULL ELSE manifest END, updated_at = ?6 WHERE id = ?1",
            params![id, status.as_str(), attempts, next_attempt_at, message, now],
        )?;
        Ok(FailureOutcome { gave_up, attempts, next_attempt_at })
    }

    /// Try a failed batch again.
    pub fn retry_batch(&self, id: i64, now: i64) -> Result<bool> {
        let changed = self.connection.execute(
            "UPDATE applies SET status = 'pending', attempts = 0, next_attempt_at = 0, manifest = NULL, last_error = NULL, updated_at = ?2 WHERE id = ?1 AND status = 'failed'",
            params![id, now],
        )?;
        Ok(changed > 0)
    }

    /// Give up a failed batch: its candidates wait for the user again, as if they had just been
    /// listed. An automatic approval counts its days from now, not from when they were found.
    pub fn release_batch(&self, id: i64, now: i64) -> Result<bool> {
        let transaction = self.connection.unchecked_transaction()?;
        let released = transaction.execute("DELETE FROM applies WHERE id = ?1 AND status = 'failed'", params![id])?;
        if released > 0 {
            transaction.execute("UPDATE candidates SET state = 'waiting', apply_id = NULL, decided_at = 0, waiting_since = ?2 WHERE apply_id = ?1", params![id, now])?;
        }
        transaction.commit()?;
        Ok(released > 0)
    }

    fn batches_where(&self, condition: &str, limit: usize) -> Result<Vec<Batch>> {
        let sql = format!("SELECT {BATCH_COLUMNS} FROM applies WHERE {condition} ORDER BY updated_at DESC, id DESC LIMIT ?1");
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement.query_map(params![limit as i64], batch_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Batches whose notes are not written yet: waiting for their turn or being written.
    pub fn open_batches(&self) -> Result<Vec<Batch>> {
        self.batches_where("status IN ('pending', 'applying')", 100)
    }

    pub fn failed_batches(&self) -> Result<Vec<Batch>> {
        self.batches_where("status = 'failed'", 100)
    }

    /// The batches written most recently, newest first. One whose search index is still to be
    /// refreshed is among them: its notes are in the folder.
    pub fn applied_batches(&self, limit: usize) -> Result<Vec<Batch>> {
        self.batches_where("status IN ('index_pending', 'done') AND result IS NOT NULL", limit)
    }

    /// Forget written batches older than the candidates kept with them. The notes and their
    /// commits keep what was written; a batch still being written or that failed waits for the user.
    pub fn prune_written(&self, now: i64) -> Result<usize> {
        Ok(self.connection.execute(PRUNE_WRITTEN, params![now - KEPT_SECS])?)
    }

    /// Forget every written batch now, at the user's request. What is still being written or failed stays.
    pub fn clear_written(&self) -> Result<usize> {
        Ok(self.connection.execute("DELETE FROM applies WHERE status = 'done'", [])?)
    }
}

impl Batch {
    /// Whether one more failure of this kind ends the attempts.
    pub fn gives_up(&self, failure: Failure) -> bool {
        if self.status == ApplyStatus::IndexPending {
            return false;
        }
        match failure {
            Failure::Permanent => true,
            Failure::Rejected => self.attempts + 1 >= MAX_REJECTED_ATTEMPTS,
            Failure::Transient => self.attempts + 1 >= MAX_TRANSIENT_ATTEMPTS,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn queue() -> (tempfile::TempDir, Queue) {
        let dir = tempfile::tempdir().unwrap();
        let queue = Queue::open(&dir.path().join("state.sqlite")).unwrap();
        (dir, queue)
    }

    fn found(text: &str) -> Found {
        Found {
            kind: Kind::Decision,
            text: text.into(),
            source: Source::Found,
            scope: Scope::Project,
            evidence: vec![Quote { reference: "L2".into(), quote: "no alerts".into(), speaker: Speaker::User, at: "2026-11-17 10:00:00 +09:00".into() }],
            relation: Relation::new(),
        }
    }

    fn origin<'a>(job: &'a str, project: &'a str) -> Origin<'a> {
        Origin { job_id: job, session_id: "s1", provider: Provider::Claude, project, language: "English" }
    }

    #[test]
    fn written_batches_are_forgotten_after_thirty_days_or_when_cleared_but_open_and_failed_ones_stay() {
        let (_dir, queue) = queue();
        let add = |status: &str, updated: i64| {
            queue.connection.execute("INSERT INTO applies (project, status, result, created_at, updated_at) VALUES ('demo', ?1, '{}', ?2, ?2)", params![status, updated]).unwrap();
        };
        let left = || queue.connection.prepare("SELECT status FROM applies ORDER BY id").unwrap().query_map([], |row| row.get::<_, String>(0)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
        let now = 100 * 24 * 3600;
        for status in ["done", "failed", "pending", "applying", "index_pending"] {
            add(status, now - KEPT_SECS - 1);
        }
        add("done", now - KEPT_SECS + 1);
        assert_eq!(queue.prune_written(now).unwrap(), 1);
        assert_eq!(left(), vec!["failed", "pending", "applying", "index_pending", "done"]);
        assert_eq!(queue.clear_written().unwrap(), 1);
        assert_eq!(left(), vec!["failed", "pending", "applying", "index_pending"]);
    }

    #[test]
    fn a_step_stores_its_candidates_and_moves_the_cursor_together() {
        let (_dir, queue) = queue();
        let job = queue.enqueue(Provider::Claude, Path::new("/tmp/a.jsonl"), false, 1).unwrap();
        let same = Found { relation: Relation::to(RelationKind::Same, "common/a.md", "A", vec!["x".into()], String::new()), ..found("Already there.") };
        assert!(queue.store_candidates(&origin(&job, "demo"), 12, "hash", false, 1, &[found("No alerts."), found("Use pages.")], &[same], 100).unwrap());
        let waiting = queue.waiting_candidates().unwrap();
        assert_eq!(waiting.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(), vec!["No alerts.", "Use pages."]);
        assert_eq!(waiting[0].evidence[0].speaker, Speaker::User);
        assert_eq!(queue.left_out().unwrap().len(), 1);
        assert_eq!(queue.waiting().unwrap(), Waiting { candidates: 2, sessions: 1, oldest_at: 100, by_job: BTreeMap::from([(job.clone(), 2)]) });
        let stored = queue.get(&job).unwrap().unwrap();
        assert_eq!((stored.cursor_line, stored.cursor_hash.as_str(), stored.status), (12, "hash", crate::queue::Status::Done));
    }

    #[test]
    fn a_step_run_again_replaces_its_own_candidates_but_not_decided_ones() {
        let (_dir, queue) = queue();
        let job = queue.enqueue(Provider::Claude, Path::new("/tmp/a.jsonl"), false, 1).unwrap();
        queue.store_candidates(&origin(&job, "demo"), 12, "h", true, 1, &[found("First wording.")], &[], 100).unwrap();
        queue.store_candidates(&origin(&job, "demo"), 12, "h", true, 1, &[found("Second wording.")], &[], 101).unwrap();
        let waiting = queue.waiting_candidates().unwrap();
        assert_eq!(waiting.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(), vec!["Second wording."]);
        queue.discard_candidates(&[waiting[0].id], 102).unwrap();
        queue.store_candidates(&origin(&job, "demo"), 12, "h", true, 1, &[found("Third wording.")], &[], 103).unwrap();
        assert!(queue.waiting_candidates().unwrap().is_empty());
    }

    #[test]
    fn only_a_waiting_candidate_is_edited_and_only_new_content_changes_scope() {
        let (_dir, queue) = queue();
        let job = queue.enqueue(Provider::Claude, Path::new("/tmp/a.jsonl"), false, 1).unwrap();
        let differs = Found { relation: Relation::to(RelationKind::Differs, "projects/demo/a.md", "A", vec!["x".into()], "d1".into()), ..found("Changes a note.") };
        queue.store_candidates(&origin(&job, "demo"), 5, "h", false, 1, &[found("New content."), differs], &[], 100).unwrap();
        let ids: Vec<i64> = queue.waiting_candidates().unwrap().iter().map(|c| c.id).collect();
        assert!(queue.edit_candidate(ids[0], Some("  Reworded.  "), Some(Scope::Common)).unwrap());
        assert!(!queue.edit_candidate(ids[1], None, Some(Scope::Common)).unwrap());
        assert!(!queue.edit_candidate(ids[0], Some("   "), None).unwrap());
        let first = queue.candidate(ids[0]).unwrap().unwrap();
        assert_eq!((first.text.as_str(), first.scope, first.edited), ("Reworded.", Scope::Common, true));
        assert_eq!(queue.candidate(ids[1]).unwrap().unwrap().scope, Scope::Project);
        queue.discard_candidates(&[ids[0]], 101).unwrap();
        assert!(!queue.edit_candidate(ids[0], Some("Too late."), None).unwrap());
    }

    #[test]
    fn approval_makes_one_batch_per_project_and_a_finished_batch_marks_its_candidates_applied() {
        let (_dir, queue) = queue();
        let a = queue.enqueue(Provider::Claude, Path::new("/tmp/a.jsonl"), false, 1).unwrap();
        let b = queue.enqueue(Provider::Codex, Path::new("/tmp/b.jsonl"), false, 1).unwrap();
        queue.store_candidates(&origin(&a, "demo"), 5, "h", false, 1, &[found("One."), found("Two.")], &[], 100).unwrap();
        queue.store_candidates(&origin(&b, "other"), 5, "h", false, 1, &[found("Three.")], &[], 100).unwrap();
        let ids: Vec<i64> = queue.waiting_candidates().unwrap().iter().map(|c| c.id).collect();
        let batches = queue.approve_candidates(&[ids[0], ids[2], 999], false, 200).unwrap();
        assert_eq!(batches.len(), 2);
        assert_eq!(queue.waiting_candidates().unwrap().iter().map(|c| c.text.as_str()).collect::<Vec<_>>(), vec!["Two."]);
        // Approving again does nothing: the candidates no longer wait.
        assert!(queue.approve_candidates(&[ids[0]], false, 201).unwrap().is_empty());
        let next = queue.next_batch(300).unwrap().unwrap();
        assert_eq!((next.project.as_str(), next.candidates, next.auto), ("demo", 1, false));
        let members = queue.batch_candidates(next.id).unwrap();
        assert_eq!((members[0].text.as_str(), members[0].state), ("One.", State::Approved));
        let result = ApplyResult { summary: "Added one decision.".into(), notes: vec![NoteResult { path: "projects/demo/a.md".into(), action: NoteAction::Created, items: vec!["One.".into()] }] };
        queue.begin_batch(next.id, "{}", 300).unwrap();
        assert!(queue.write_interrupted().unwrap());
        queue.batch_written(next.id, &result, &["projects/demo/a.md".to_string()], 301).unwrap();
        assert!(!queue.write_interrupted().unwrap());
        assert_eq!(queue.candidate(ids[0]).unwrap().unwrap().state, State::Applied);
        // Written is what the user is shown, also while the index is still to be refreshed.
        let written = &queue.applied_batches(5).unwrap()[0];
        assert_eq!((written.status, written.result.as_ref(), written.manifest.as_deref()), (ApplyStatus::IndexPending, Some(&result), Some(r#"["projects/demo/a.md"]"#)));
        assert!(queue.open_batches().unwrap().iter().all(|batch| batch.id != next.id));
        queue.finish_batch(next.id, 302).unwrap();
        let done = &queue.applied_batches(5).unwrap()[0];
        assert_eq!((done.status, done.manifest.as_deref()), (ApplyStatus::Done, None));
        assert_eq!(queue.next_batch(302).unwrap().unwrap().project, "other");
    }

    #[test]
    fn a_batch_whose_notes_are_written_never_fails_for_good() {
        let (_dir, queue) = queue();
        let job = queue.enqueue(Provider::Claude, Path::new("/tmp/a.jsonl"), false, 1).unwrap();
        queue.store_candidates(&origin(&job, "demo"), 5, "h", false, 1, &[found("One.")], &[], 100).unwrap();
        let id = queue.waiting_candidates().unwrap()[0].id;
        let batch = queue.approve_candidates(&[id], false, 200).unwrap()[0];
        queue.batch_written(batch, &ApplyResult::default(), &[], 201).unwrap();
        for attempt in 0..2 * MAX_TRANSIENT_ATTEMPTS {
            assert!(!queue.batch_failed(batch, Failure::Transient, "qmd is not installed", 300 + attempt).unwrap().gave_up);
        }
        assert!(!queue.batch_failed(batch, Failure::Permanent, "qmd is not installed", 400).unwrap().gave_up);
        let waiting = queue.batch(batch).unwrap().unwrap();
        assert_eq!((waiting.status, waiting.last_error.as_deref()), (ApplyStatus::IndexPending, Some("qmd is not installed")));
        assert!(queue.failed_batches().unwrap().is_empty());
        // It cannot be tried from the start again or released: its candidates are in the notes.
        assert!(!queue.retry_batch(batch, 500).unwrap() && !queue.release_batch(batch, 500).unwrap());
        assert_eq!(queue.candidate(id).unwrap().unwrap().state, State::Applied);
        assert_eq!(queue.next_batch(400 + 3600).unwrap().unwrap().id, batch);
    }

    #[test]
    fn candidates_of_a_session_are_announced_once() {
        let (_dir, queue) = queue();
        let job = queue.enqueue(Provider::Claude, Path::new("/tmp/a.jsonl"), false, 1).unwrap();
        queue.store_candidates(&origin(&job, "demo"), 5, "h", true, 1, &[found("One."), found("Two.")], &[found("Already there.")], 100).unwrap();
        queue.discard_candidates(&[queue.waiting_candidates().unwrap()[0].id], 101).unwrap();
        assert_eq!(queue.announce(&job).unwrap(), 1);
        assert_eq!(queue.announce(&job).unwrap(), 0);
        queue.store_candidates(&origin(&job, "demo"), 9, "h", false, 1, &[found("Three.")], &[], 200).unwrap();
        assert_eq!(queue.announce(&job).unwrap(), 1);
    }

    #[test]
    fn a_batch_that_keeps_failing_stops_and_can_be_retried_or_released() {
        let (_dir, queue) = queue();
        let job = queue.enqueue(Provider::Claude, Path::new("/tmp/a.jsonl"), false, 1).unwrap();
        queue.store_candidates(&origin(&job, "demo"), 5, "h", false, 1, &[found("One.")], &[], 100).unwrap();
        let id = queue.waiting_candidates().unwrap()[0].id;
        let batch = queue.approve_candidates(&[id], true, 200).unwrap()[0];
        let first = queue.batch_failed(batch, Failure::Rejected, "bad answer", 300).unwrap();
        assert!(!first.gave_up && first.next_attempt_at > 300);
        assert!(queue.next_batch(300).unwrap().is_none());
        for _ in 1..MAX_REJECTED_ATTEMPTS {
            queue.batch_failed(batch, Failure::Rejected, "bad answer", 300).unwrap();
        }
        assert_eq!(queue.failed_batches().unwrap().iter().map(|b| (b.id, b.auto)).collect::<Vec<_>>(), vec![(batch, true)]);
        assert!(queue.retry_batch(batch, 400).unwrap());
        assert_eq!(queue.next_batch(400).unwrap().unwrap().id, batch);
        queue.batch_failed(batch, Failure::Permanent, "the note changed", 401).unwrap();
        assert!(queue.release_batch(batch, 500).unwrap());
        assert!(queue.batch(batch).unwrap().is_none());
        let back = queue.candidate(id).unwrap().unwrap();
        // It waits as if it had just been listed: an automatic approval does not take it at once.
        assert_eq!((back.state, back.apply_id, back.created_at, back.waiting_since), (State::Waiting, None, 100, 500));
        assert!(queue.candidates_waiting_since(499).unwrap().is_empty());
    }

    #[test]
    fn candidates_that_waited_long_enough_are_listed_for_automatic_approval() {
        let (_dir, queue) = queue();
        let job = queue.enqueue(Provider::Claude, Path::new("/tmp/a.jsonl"), false, 1).unwrap();
        queue.store_candidates(&origin(&job, "demo"), 5, "h", true, 1, &[found("Old.")], &[], 100).unwrap();
        queue.store_candidates(&origin(&job, "demo"), 9, "h", false, 1, &[found("Recent.")], &[], 500).unwrap();
        let old = queue.candidates_waiting_since(300).unwrap();
        assert_eq!(old.len(), 1);
        assert_eq!(queue.candidate(old[0]).unwrap().unwrap().text, "Old.");
    }
}
