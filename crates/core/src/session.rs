//! What a queued session has to offer since it was last read.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, TimeZone};

use crate::error::Result;
use crate::mark::{resolve, Placed, Resolved};
use crate::project;
use crate::queue::Job;
use crate::transcript::{check_prefix, parse, prefix_hash, read_identity, read_transcript, Origin, Parsed, Provider, Turn};

/// One step of a session: everything the file holds since the last step.
pub struct Session {
    pub transcript: PathBuf,
    pub provider: Provider,
    pub parsed: Parsed,
    pub project: String,
    pub end_line: usize,
    pub end_hash: String,
    /// What the session marked in this step, with what it replaced and withdrew applied.
    pub marks: Resolved,
}

pub enum Reading {
    Ready(Box<Session>),
    Skip(String),
    NothingNew { end_line: usize, end_hash: String },
    AgentOnly { end_line: usize, end_hash: String },
}

fn is_input(turn: &Turn) -> bool {
    turn.role == "user" && turn.origin != Origin::Context
}

/// The new turns answer another agent, not the user: what was marked there is not the user's.
pub fn agent_only(parsed: &Parsed, new_turns: &[&Turn], after_line: usize) -> bool {
    let inputs: Vec<&&Turn> = new_turns.iter().filter(|turn| is_input(turn)).collect();
    if !inputs.is_empty() {
        return inputs.iter().all(|turn| turn.origin == Origin::Agent);
    }
    parsed.turns.iter().rfind(|turn| turn.line <= after_line && is_input(turn)).map(|turn| turn.origin == Origin::Agent).unwrap_or(false)
}

pub fn read(job: &Job, recorder_workdir: &Path) -> Result<Reading> {
    let transcript = PathBuf::from(&job.transcript);
    let data = read_transcript(&transcript)?;
    check_prefix(&data, job.cursor_line, &job.cursor_hash)?;
    let parsed = parse(&data, job.provider)?;
    if parsed.worker_session {
        return Ok(Reading::Skip("Codex subagent sessions are skipped.".into()));
    }
    let cwd = PathBuf::from(&parsed.cwd);
    if !parsed.cwd.is_empty() && cwd.starts_with(recorder_workdir) {
        return Ok(Reading::Skip("This session was created by the recording model.".into()));
    }
    let Some(repository) = project::resolve_checked(&cwd)? else {
        return Ok(Reading::Skip(format!("The session was outside a Git repository: {}", parsed.cwd)));
    };
    let end_line = parsed.complete_lines.max(job.cursor_line);
    let end_hash = prefix_hash(&data, end_line);
    let new_turns: Vec<&Turn> = parsed.turns.iter().filter(|turn| turn.line > job.cursor_line).collect();
    let placed: Vec<Placed> = parsed.marks.iter().filter(|placed| placed.line > job.cursor_line && placed.line <= end_line).cloned().collect();
    if new_turns.is_empty() && placed.is_empty() {
        return Ok(Reading::NothingNew { end_line, end_hash });
    }
    if agent_only(&parsed, &new_turns, job.cursor_line) {
        return Ok(Reading::AgentOnly { end_line, end_hash });
    }
    let marks = resolve(&placed);
    Ok(Reading::Ready(Box::new(Session { transcript, provider: job.provider, parsed, project: repository.name, end_line, end_hash, marks })))
}

// The worker stores the project and session id only when it starts a job, so a job still waiting is identified from its transcript.
pub fn identify(mut job: Job) -> Job {
    if job.project.is_some() && job.session_id.is_some() {
        return job;
    }
    let Ok(identity) = read_identity(Path::new(&job.transcript), job.provider) else { return job };
    if job.session_id.is_none() && !identity.session_id.is_empty() {
        job.session_id = Some(identity.session_id);
    }
    if job.project.is_none() {
        job.project = project::resolve(Path::new(&identity.cwd)).map(|repository| repository.name);
    }
    job
}

pub fn local_stamp<Zone: TimeZone>(stamp: &str, zone: &Zone) -> String
where
    Zone::Offset: std::fmt::Display,
{
    match DateTime::parse_from_rfc3339(stamp) {
        Ok(time) => time.with_timezone(zone).format("%Y-%m-%d %H:%M:%S %:z").to_string(),
        Err(_) => stamp.to_string(),
    }
}

/// Turns as a model is shown them: with the time the user's clock showed.
pub fn in_local_time(turns: &[Turn]) -> Vec<Turn> {
    turns.iter().map(|turn| Turn { timestamp: local_stamp(&turn.timestamp, &Local), ..turn.clone() }).collect()
}

/// On a retry, why the previous attempt failed. Without it a rejected answer is repeated blind.
pub fn previous_failure(attempts: i64, last_error: Option<&str>) -> Option<&str> {
    last_error.filter(|_| attempts > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::{Kind, Scope, Speaker};
    use crate::mark::{line, Entry, Mark};
    use crate::queue::Status;
    use serde_json::{json, Value};
    use std::fs;

    fn user(uuid: &str, text: &str) -> Value {
        json!({"type": "user", "uuid": uuid, "userType": "external", "timestamp": "2026-11-04T00:00:00Z", "message": {"content": text}})
    }

    fn assistant(uuid: &str, text: &str) -> Value {
        json!({"type": "assistant", "uuid": uuid, "message": {"content": [{"type": "text", "text": text}]}})
    }

    fn tool_result(uuid: &str, output: &str) -> Value {
        json!({"type": "user", "uuid": uuid, "userType": "external", "timestamp": "2026-11-04T00:01:00Z", "message": {"content": [{"type": "tool_result", "tool_use_id": "t", "content": output}]}})
    }

    fn marked(id: &str, text: &str, replaces: Option<&str>) -> String {
        line(&Entry::Mark(Mark { id: id.into(), kind: Kind::Correction, text: text.into(), by: Speaker::User, said: vec!["Do not delete".into()], scope: Scope::Project, replaces: replaces.map(str::to_string), note: None }))
    }

    fn job(path: &Path, cursor_line: usize, cursor_hash: &str) -> Job {
        Job {
            id: "job1".into(),
            provider: Provider::Claude,
            transcript: path.to_string_lossy().to_string(),
            status: Status::Pending,
            cursor_line,
            cursor_hash: cursor_hash.into(),
            project: None,
            session_id: None,
            attempts: 0,
            next_attempt_at: 0,
            requests: 1,
            last_error: None,
            updated_at: 0,
            ended: false,
        }
    }

    fn session_file(dir: &Path, cwd: &Path, lines: &[Value]) -> PathBuf {
        let path = dir.join("session.jsonl");
        let mut text = String::new();
        for line in lines {
            let mut record = line.clone();
            record["cwd"] = json!(cwd.to_string_lossy());
            record["sessionId"] = json!("sess-1");
            text.push_str(&format!("{record}\n"));
        }
        fs::write(&path, text).unwrap();
        path
    }

    fn repository(dir: &Path) -> PathBuf {
        let repo = dir.join("demo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        repo
    }

    #[test]
    fn a_step_reads_to_the_end_of_the_file_and_holds_the_marks_made_since_the_last_one() {
        let dir = tempfile::tempdir().unwrap();
        let repo = repository(dir.path());
        let lines = [
            user("u1", "Start."),
            tool_result("r1", &marked("e1", "Marked before the cursor.", None)),
            assistant("a1", "Done."),
            user("u2", "Do not delete without asking."),
            tool_result("r2", &marked("n1", "Ask before deleting.", None)),
            tool_result("r3", &marked("n2", "Ask before deleting anything.", Some("n1"))),
            tool_result("r4", &line(&Entry::Cancel("e1".into()))),
            assistant("a2", "Understood."),
        ];
        let path = session_file(dir.path(), &repo, &lines);
        let data = fs::read(&path).unwrap();
        let Reading::Ready(session) = read(&job(&path, 3, &prefix_hash(&data, 3)), Path::new("/nowhere")).unwrap() else { panic!("expected a session") };
        assert_eq!((session.project.as_str(), session.end_line), ("demo", 8));
        assert_eq!(session.marks.marks.iter().map(|marked| (marked.line, marked.at.as_str(), marked.mark.text.as_str())).collect::<Vec<_>>(), vec![(6, "2026-11-04T00:01:00Z", "Ask before deleting anything.")]);
        assert_eq!(session.marks.withdrawn, vec!["e1".to_string()]);
    }

    #[test]
    fn a_mark_without_a_turn_after_it_is_still_read() {
        let dir = tempfile::tempdir().unwrap();
        let repo = repository(dir.path());
        let path = session_file(dir.path(), &repo, &[user("u1", "Do not delete without asking."), assistant("a1", "Understood."), tool_result("r1", &marked("n1", "Ask before deleting.", None))]);
        let data = fs::read(&path).unwrap();
        let Reading::Ready(session) = read(&job(&path, 2, &prefix_hash(&data, 2)), Path::new("/nowhere")).unwrap() else { panic!("expected a session") };
        assert_eq!(session.marks.marks.len(), 1);
    }

    #[test]
    fn sessions_with_nothing_of_the_user_are_not_offered() {
        let dir = tempfile::tempdir().unwrap();
        let repo = repository(dir.path());
        let path = session_file(dir.path(), &repo, &[user("u1", "You are a dispatched worker. Build it."), tool_result("r1", &marked("w1", "Build it.", None)), assistant("a1", "Built.")]);
        assert!(matches!(read(&job(&path, 0, ""), Path::new("/nowhere")).unwrap(), Reading::AgentOnly { end_line: 3, .. }));
        let data = fs::read(&path).unwrap();
        assert!(matches!(read(&job(&path, 3, &prefix_hash(&data, 3)), Path::new("/nowhere")).unwrap(), Reading::NothingNew { end_line: 3, .. }));
        let outside = session_file(dir.path(), dir.path(), &[user("u1", "Hello.")]);
        assert!(matches!(read(&job(&outside, 0, ""), Path::new("/nowhere")).unwrap(), Reading::Skip(_)));
        let own = session_file(dir.path(), &repo, &[user("u1", "Hello.")]);
        assert!(matches!(read(&job(&own, 0, ""), &repo).unwrap(), Reading::Skip(_)));
    }

    #[test]
    fn a_changed_prefix_stops_the_reading() {
        let dir = tempfile::tempdir().unwrap();
        let repo = repository(dir.path());
        let path = session_file(dir.path(), &repo, &[user("u1", "One."), assistant("a1", "Two.")]);
        assert!(read(&job(&path, 1, "not-the-hash"), Path::new("/nowhere")).is_err());
    }

    #[test]
    fn times_are_shown_as_the_users_clock_had_them() {
        let zone = chrono::FixedOffset::east_opt(9 * 3600).unwrap();
        assert_eq!(local_stamp("2026-11-04T23:30:00Z", &zone), "2026-11-05 08:30:00 +09:00");
        assert_eq!(local_stamp("", &zone), "");
        assert_eq!(previous_failure(0, Some("x")), None);
        assert_eq!(previous_failure(2, Some("x")), Some("x"));
    }
}
