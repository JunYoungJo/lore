use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::batch::{make_batch, Batch};
use crate::copy;
use crate::error::{Error, Result};
use crate::notes::{self, CatalogEntry};
use crate::project;
use crate::proposal::{check_snapshot, render_note, validate, Change, Edit, Proposal, Scope};
use crate::queue::Job;
use crate::settings::atomic_write;
use crate::transcript::{check_prefix, parse, prefix_hash, read_identity, read_transcript, Origin, Parsed, Provider, Turn};

pub struct Prepared {
    pub transcript: PathBuf,
    pub provider: Provider,
    pub parsed: Parsed,
    pub project: String,
    pub batch: Batch,
    pub end_hash: String,
}

pub enum Preparation {
    Ready(Box<Prepared>),
    Skip(String),
    NothingNew { end_line: usize, end_hash: String },
    AgentOnly { end_line: usize, end_hash: String, has_more: bool },
}

fn is_input(turn: &Turn) -> bool {
    turn.role == "user" && turn.origin != Origin::Context
}

pub fn agent_only(parsed: &Parsed, batch: &Batch, after_line: usize) -> bool {
    let inputs: Vec<&Turn> = batch.new_turns.iter().filter(|turn| is_input(turn)).collect();
    if !inputs.is_empty() {
        return inputs.iter().all(|turn| turn.origin == Origin::Agent);
    }
    parsed.turns.iter().filter(|turn| turn.line <= after_line && is_input(turn)).last().map(|turn| turn.origin == Origin::Agent).unwrap_or(false)
}

pub fn prepare(job: &Job, max_chars: usize, recorder_workdir: &Path) -> Result<Preparation> {
    let transcript = PathBuf::from(&job.transcript);
    let data = read_transcript(&transcript)?;
    check_prefix(&data, job.cursor_line, &job.cursor_hash)?;
    let parsed = parse(&data, job.provider)?;
    if parsed.worker_session {
        return Ok(Preparation::Skip("Codex subagent sessions are skipped.".into()));
    }
    let cwd = PathBuf::from(&parsed.cwd);
    if !parsed.cwd.is_empty() && cwd.starts_with(recorder_workdir) {
        return Ok(Preparation::Skip("This session was created by the recording model.".into()));
    }
    let Some(repository) = project::resolve_checked(&cwd)? else {
        return Ok(Preparation::Skip(format!("The session was outside a Git repository: {}", parsed.cwd)));
    };
    let batch = make_batch(&parsed, job.cursor_line, max_chars);
    let end_hash = prefix_hash(&data, batch.end_line);
    if batch.new_turns.is_empty() {
        return Ok(Preparation::NothingNew { end_line: batch.end_line, end_hash });
    }
    if agent_only(&parsed, &batch, job.cursor_line) {
        return Ok(Preparation::AgentOnly { end_line: batch.end_line, end_hash, has_more: parsed.complete_lines > batch.end_line });
    }
    Ok(Preparation::Ready(Box::new(Prepared { transcript, provider: job.provider, parsed, project: repository.name, batch, end_hash })))
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

pub fn ranking_query(batch: &Batch) -> String {
    let user: Vec<&str> = batch.new_turns.iter().filter(|turn| turn.origin == Origin::User).map(|turn| turn.text.as_str()).collect();
    let source = if user.is_empty() { batch.new_turns.iter().map(|turn| turn.text.as_str()).collect() } else { user };
    source.join("\n").chars().take(1_500).collect()
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

fn in_local_time(turns: &[Turn]) -> Vec<Turn> {
    turns.iter().map(|turn| Turn { timestamp: local_stamp(&turn.timestamp, &Local), ..turn.clone() }).collect()
}

pub struct Request {
    pub payload: Value,
    pub base: BTreeMap<String, String>,
    pub existing: BTreeSet<String>,
    pub editable: BTreeSet<String>,
}

pub fn build_request(prepared: &Prepared, scope_notes: &BTreeMap<String, String>, ranked: &[String], required: &[String], budget: usize) -> Request {
    let selection = notes::select(scope_notes, ranked, required, budget);
    let catalog: Vec<CatalogEntry> = selection.catalog.into_iter().filter(|entry| !selection.bodies.contains_key(&entry.path)).collect();
    let payload = json!({
        "project": prepared.project,
        "provider": prepared.provider.as_str(),
        "session_id": prepared.parsed.session_id,
        "new_turns": in_local_time(&prepared.batch.new_turns),
        "context_turns": in_local_time(&prepared.batch.context_turns),
        "omitted_context_turns": prepared.batch.omitted_context_turns,
        "existing_notes": selection.bodies,
        "note_catalog": catalog,
    });
    Request {
        payload,
        base: scope_notes.clone(),
        existing: scope_notes.keys().cloned().collect(),
        editable: selection.bodies.keys().cloned().collect(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub project: String,
    pub summary: String,
    pub end_line: usize,
    pub end_hash: String,
    pub has_more: bool,
    pub base: BTreeMap<String, String>,
    pub rendered: BTreeMap<String, String>,
    pub copy_file: Option<String>,
    pub copy_text: Option<String>,
}

fn offered_turns(prepared: &Prepared) -> Vec<Turn> {
    let offered: BTreeSet<&str> = prepared.batch.context_turns.iter().chain(prepared.batch.new_turns.iter()).map(|turn| turn.reference.as_str()).collect();
    prepared.parsed.turns.iter().filter(|turn| offered.contains(turn.reference.as_str())).cloned().collect()
}

fn relinked_change(change: &Change, copy_file: &Path, moves: &BTreeMap<usize, usize>) -> Change {
    if moves.is_empty() {
        return change.clone();
    }
    let fix = |text: &str| copy::relink(text, copy_file, moves).unwrap_or_else(|| text.to_string());
    Change {
        body: fix(&change.body),
        edits: change.edits.iter().map(|edit| Edit { find: fix(&edit.find), text: fix(&edit.text), ..edit.clone() }).collect(),
        ..change.clone()
    }
}

pub fn plan(prepared: &Prepared, request: &Request, proposal: &Proposal, knowledge_root: &Path, fallback_name: &str) -> Result<Plan> {
    let turns = offered_turns(prepared);
    validate(proposal, &Scope { project: &prepared.project, turns: &turns, existing: &request.existing, editable: &request.editable })?;
    let mut result = Plan {
        project: prepared.project.clone(),
        summary: proposal.summary.clone(),
        end_line: prepared.batch.end_line,
        end_hash: prepared.end_hash.clone(),
        has_more: prepared.parsed.complete_lines > prepared.batch.end_line,
        base: request.base.clone(),
        rendered: BTreeMap::new(),
        copy_file: None,
        copy_text: None,
    };
    if proposal.changes.is_empty() {
        return Ok(result);
    }
    let dialogue = copy::render(&prepared.parsed, prepared.provider, &prepared.transcript, prepared.batch.end_line);
    let copy_file = copy::copy_path(knowledge_root, prepared.provider, &prepared.parsed.session_id, fallback_name);
    let moves = copy::line_moves(&std::fs::read_to_string(&copy_file).unwrap_or_default(), &dialogue.lines);
    let mut relinked = BTreeMap::new();
    if !moves.is_empty() {
        for (note, text) in notes::load_all(knowledge_root)? {
            let original = request.base.get(&note).cloned().unwrap_or(text);
            if let Some(updated) = copy::relink(&original, &copy_file, &moves) {
                result.base.entry(note.clone()).or_insert(original);
                relinked.insert(note, updated);
            }
        }
    }
    for change in &proposal.changes {
        let change = relinked_change(change, &copy_file, &moves);
        let old = relinked.get(&change.path).or_else(|| request.base.get(&change.path)).map(String::as_str);
        let link = |evidence: &crate::proposal::Evidence| copy::portable_link(&change.path, prepared.provider, &prepared.parsed.session_id, fallback_name, &evidence.reference);
        result.rendered.insert(change.path.clone(), render_note(&change, &prepared.project, &link, old)?);
    }
    for (note, text) in relinked {
        result.rendered.entry(note).or_insert(text);
    }
    result.copy_file = Some(copy_file.to_string_lossy().to_string());
    result.copy_text = Some(dialogue.text);
    Ok(result)
}

pub fn apply(knowledge_root: &Path, plan: &Plan) -> Result<()> {
    let mut current = notes::load_scope(knowledge_root, &plan.project)?;
    for note in plan.base.keys().chain(plan.rendered.keys()) {
        if !current.contains_key(note) {
            if let Ok(text) = std::fs::read_to_string(knowledge_root.join(note)) {
                current.insert(note.clone(), text);
            }
        }
    }
    check_snapshot(&current, &plan.base, &plan.rendered)?;
    if let (Some(file), Some(text)) = (&plan.copy_file, &plan.copy_text) {
        atomic_write(Path::new(file), text.as_bytes())?;
    }
    for (note, content) in &plan.rendered {
        let target = knowledge_root.join(note);
        if let Ok(metadata) = std::fs::symlink_metadata(&target) {
            if metadata.file_type().is_symlink() {
                return Err(Error::permanent(format!("Symlinks are not allowed for memory notes: {note}")));
            }
        }
        atomic_write(&target, content.as_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proposal::{Change, Edit, Evidence};
    use crate::queue::Status;
    use serde_json::json;
    use std::fs;

    fn write_transcript(dir: &Path, cwd: &Path, lines: &[Value]) -> PathBuf {
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

    fn user(uuid: &str, text: &str) -> Value {
        json!({"type": "user", "uuid": uuid, "userType": "external", "timestamp": "2026-11-04T00:00:00Z", "message": {"content": text}})
    }

    fn assistant(uuid: &str, text: &str) -> Value {
        json!({"type": "assistant", "uuid": uuid, "message": {"content": [{"type": "text", "text": text}]}})
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
            manifest: None,
            last_error: None,
            updated_at: 0,
        }
    }

    struct Fixture {
        dir: tempfile::TempDir,
        knowledge: PathBuf,
        transcript: PathBuf,
    }

    fn fixture(lines: &[Value]) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("demo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        let knowledge = dir.path().join("knowledge");
        fs::create_dir_all(knowledge.join("common")).unwrap();
        let transcript = write_transcript(dir.path(), &repo, lines);
        Fixture { dir, knowledge, transcript }
    }

    fn ready(fixture: &Fixture, cursor_line: usize, cursor_hash: &str) -> Prepared {
        match prepare(&job(&fixture.transcript, cursor_line, cursor_hash), 30_000, &fixture.dir.path().join("recorder")).unwrap() {
            Preparation::Ready(prepared) => *prepared,
            _ => panic!("expected new turns"),
        }
    }

    fn decision(path: &str) -> Proposal {
        Proposal {
            summary: "로그 전용".into(),
            changes: vec![Change {
                path: path.into(),
                title: "BG 불일치".into(),
                kind: "decision".into(),
                authority: "user_decision".into(),
                body: "TP 로그만 남깁니다.".into(),
                edits: vec![],
                evidence: vec![Evidence { reference: "L1".into(), quote: "로그만 남기십시오".into() }],
            }],
            needs_notes: vec![],
        }
    }

    fn appended(path: &str, section: &str, text: &str) -> Proposal {
        let mut proposal = decision(path);
        let change = &mut proposal.changes[0];
        change.title = String::new();
        change.body = String::new();
        change.edits = vec![Edit { op: "append".into(), find: String::new(), section: section.into(), heading: String::new(), text: text.into() }];
        proposal
    }

    #[test]
    fn session_in_a_repository_is_prepared_with_its_project_name() {
        let fixture = fixture(&[user("a", "경고는 빼고 로그만 남기십시오."), assistant("b", "그렇게 하겠습니다.")]);
        let prepared = ready(&fixture, 0, "");
        assert_eq!(prepared.project, "demo");
        assert_eq!(prepared.batch.new_turns.len(), 2);
        assert!(!prepared.end_hash.is_empty());
    }

    fn line_hash(fixture: &Fixture, line: usize) -> String {
        prefix_hash(&fs::read(&fixture.transcript).unwrap(), line)
    }

    #[test]
    fn portion_with_only_orca_instructions_is_passed_over_without_the_model() {
        let fixture = fixture(&[
            user("a", "Please carry out this task from my Orca coordinator by following the brief"),
            assistant("b", "작업을 끝냈습니다."),
            user("c", "Continue work from the prior Orca session using the context"),
        ]);
        let recorder = fixture.dir.path().join("recorder");
        let result = prepare(&job(&fixture.transcript, 0, ""), 30_000, &recorder).unwrap();
        assert!(matches!(result, Preparation::AgentOnly { end_line: 3, has_more: false, .. }));
    }

    #[test]
    fn portion_where_the_user_typed_once_goes_to_the_model_with_orca_input_labelled() {
        let fixture = fixture(&[
            user("a", "Please carry out this task from my Orca coordinator by following the brief"),
            assistant("b", "작업 중입니다."),
            user("c", "이건 둘째치고 앱으로 만들어야 한다고 봐."),
        ]);
        let prepared = ready(&fixture, 0, "");
        let origins: Vec<Origin> = prepared.batch.new_turns.iter().map(|turn| turn.origin).collect();
        assert_eq!(origins, vec![Origin::Agent, Origin::Assistant, Origin::User]);
    }

    #[test]
    fn assistant_only_portion_follows_whoever_gave_the_last_input() {
        let orca = fixture(&[user("a", "You are working inside Orca, a multi-agent IDE."), assistant("b", "진행합니다."), assistant("c", "끝났습니다.")]);
        let recorder = orca.dir.path().join("recorder");
        assert!(matches!(prepare(&job(&orca.transcript, 2, &line_hash(&orca, 2)), 30_000, &recorder).unwrap(), Preparation::AgentOnly { .. }));

        let human = fixture(&[user("a", "알아서 진행해."), assistant("b", "진행합니다."), assistant("c", "실패 원인을 찾았습니다.")]);
        assert_eq!(ready(&human, 2, &line_hash(&human, 2)).batch.new_turns.len(), 1);
    }

    #[test]
    fn subagent_and_repository_less_sessions_are_skipped_and_finished_sessions_report_nothing_new() {
        let fixture = fixture(&[user("a", "경고를 추가하십시오.")]);
        let recorder = fixture.dir.path().join("recorder");
        let subagent = fixture.dir.path().join("subagent.jsonl");
        fs::write(&subagent, "{\"type\":\"session_meta\",\"payload\":{\"id\":\"w\",\"cwd\":\"D:/demo\",\"source\":{\"subagent\":{\"parent_thread_id\":\"p\"}}}}\n").unwrap();
        let codex_job = Job { provider: Provider::Codex, ..job(&subagent, 0, "") };
        assert!(matches!(prepare(&codex_job, 30_000, &recorder).unwrap(), Preparation::Skip(_)));

        let outside = tempfile::tempdir().unwrap();
        let path = write_transcript(outside.path(), &outside.path().join("plain"), &[user("a", "질문")]);
        let result = prepare(&job(&path, 0, ""), 30_000, &recorder).unwrap();
        let skipped = matches!(result, Preparation::Skip(_)) || matches!(result, Preparation::Ready(ref p) if p.project != "plain");
        assert!(skipped);

        let done = fixture_with_two_lines();
        let data = fs::read(&done.transcript).unwrap();
        let hash = prefix_hash(&data, 2);
        assert!(matches!(prepare(&job(&done.transcript, 2, &hash), 30_000, &recorder).unwrap(), Preparation::NothingNew { end_line: 2, .. }));
    }

    fn fixture_with_two_lines() -> Fixture {
        fixture(&[user("a", "경고는 빼고 로그만 남기십시오."), assistant("b", "그렇게 하겠습니다.")])
    }

    #[test]
    fn changed_prefix_is_a_permanent_failure() {
        let fixture = fixture_with_two_lines();
        let error = prepare(&job(&fixture.transcript, 1, "wrong"), 30_000, &fixture.dir.path().join("recorder")).err().unwrap();
        assert_eq!(error.failure, crate::error::Failure::Permanent);
    }

    #[test]
    fn notes_without_bodies_are_listed_in_the_catalog_only() {
        let fixture = fixture_with_two_lines();
        let prepared = ready(&fixture, 0, "");
        let mut scope = BTreeMap::new();
        scope.insert("common/a.md".to_string(), format!("# A\n{}", "x".repeat(50)));
        scope.insert("projects/demo/b.md".to_string(), format!("# B\n{}", "y".repeat(50)));
        let request = build_request(&prepared, &scope, &["projects/demo/b.md".into()], &[], 60);
        assert_eq!(request.editable, BTreeSet::from(["projects/demo/b.md".to_string()]));
        assert_eq!(request.payload["note_catalog"], json!([{"path": "common/a.md", "title": "A"}]));
        assert!(request.payload["existing_notes"].get("projects/demo/b.md").is_some());
        assert_eq!(request.existing.len(), 2);
    }

    #[test]
    fn plan_writes_a_dialogue_copy_and_links_evidence_to_it() {
        let fixture = fixture_with_two_lines();
        let prepared = ready(&fixture, 0, "");
        let request = build_request(&prepared, &BTreeMap::new(), &[], &[], 60_000);
        let plan = plan(&prepared, &request, &decision("projects/demo/bg.md"), &fixture.knowledge, "job1").unwrap();
        let copy_file = PathBuf::from(plan.copy_file.as_ref().unwrap());
        assert_eq!(copy_file, fixture.knowledge.join("sources").join("claude").join("sess-1.md"));
        let copy_text = plan.copy_text.as_ref().unwrap();
        assert!(copy_text.contains("<a id=\"L1\"></a>\n## L1 · 사용자"));
        let note = &plan.rendered["projects/demo/bg.md"];
        let expected = "- [L1](<../../sources/claude/sess-1.md#L1>) — 로그만 남기십시오";
        assert!(note.contains(&expected), "{note}");
        assert!(!plan.has_more);

        apply(&fixture.knowledge, &plan).unwrap();
        assert_eq!(fs::read_to_string(fixture.knowledge.join("projects/demo/bg.md")).unwrap(), *note);
        assert_eq!(fs::read_to_string(&copy_file).unwrap(), *copy_text);
        apply(&fixture.knowledge, &plan).unwrap();
    }

    #[test]
    fn plan_rejects_unprovided_notes_and_apply_rejects_outside_edits() {
        let fixture = fixture_with_two_lines();
        let prepared = ready(&fixture, 0, "");
        let mut scope = BTreeMap::new();
        scope.insert("projects/demo/bg.md".to_string(), format!("# 기존\n\n## 결정\n\n경고를 띄웁니다.{}\n", " ".repeat(100)));
        let edit = appended("projects/demo/bg.md", "결정", "TP 로그만 남깁니다.");
        let narrow = build_request(&prepared, &scope, &[], &[], 10);
        assert!(plan(&prepared, &narrow, &edit, &fixture.knowledge, "job1").is_err());

        let wide = build_request(&prepared, &scope, &[], &[], 60_000);
        let planned = plan(&prepared, &wide, &edit, &fixture.knowledge, "job1").unwrap();
        assert!(planned.rendered["projects/demo/bg.md"].contains("# 기존\n\n## 결정\n\n경고를 띄웁니다.\n\nTP 로그만 남깁니다.\n\n## 원문 근거"));
        let unplaceable = appended("projects/demo/bg.md", "없는 절", "TP 로그만 남깁니다.");
        assert_eq!(plan(&prepared, &wide, &unplaceable, &fixture.knowledge, "job1").unwrap_err().failure, crate::error::Failure::Rejected);
        let note = fixture.knowledge.join("projects/demo/bg.md");
        fs::create_dir_all(note.parent().unwrap()).unwrap();
        fs::write(&note, "사용자가 고친 내용").unwrap();
        let error = apply(&fixture.knowledge, &planned).unwrap_err();
        assert_eq!(error.failure, crate::error::Failure::Permanent);
        assert_eq!(fs::read_to_string(&note).unwrap(), "사용자가 고친 내용");
    }

    #[test]
    fn links_in_every_note_follow_a_copy_that_gains_a_turn_in_the_middle() {
        let fixture = fixture(&[
            user("a", "경고는 빼고 로그만 남기십시오."),
            json!({"type": "attachment", "uuid": "q", "attachment": {"type": "queued_command", "prompt": "보관 기간도 정하십시오.", "origin": {"kind": "human"}}}),
            assistant("b", "그렇게 하겠습니다."),
        ]);
        let prepared = ready(&fixture, 0, "");
        let copy_file = copy::copy_path(&fixture.knowledge, Provider::Claude, "sess-1", "job1");
        let mut without_queued = prepared.parsed.clone();
        without_queued.turns.retain(|turn| turn.line != 2);
        let old_copy = copy::render(&without_queued, Provider::Claude, &fixture.transcript, 3);
        fs::create_dir_all(copy_file.parent().unwrap()).unwrap();
        fs::write(&copy_file, &old_copy.text).unwrap();
        let old_link = copy::link_target(&copy_file, old_copy.lines[&3]);
        let note = format!("# 기존\n\n## 결정\n\n경고를 띄웁니다. ([근거]({old_link}))\n\n## 원문 근거\n- [L3]({old_link}) — 그렇게 하겠습니다.\n");
        let other = format!("# 다른 프로젝트\n\n같은 대화 ([근거]({old_link}))\n");
        for (path, text) in [("projects/demo/bg.md", &note), ("projects/other/o.md", &other)] {
            let file = fixture.knowledge.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, text).unwrap();
        }

        let scope = notes::load_scope(&fixture.knowledge, "demo").unwrap();
        let request = build_request(&prepared, &scope, &[], &[], 60_000);
        let mut proposal = appended("projects/demo/bg.md", "결정", "보관 기간도 정합니다.");
        let change = &mut proposal.changes[0];
        change.edits.insert(0, Edit { op: "replace".into(), find: format!("경고를 띄웁니다. ([근거]({old_link}))"), section: String::new(), heading: String::new(), text: format!("경고 없이 로그만 남깁니다. ([근거]({old_link}))") });
        change.evidence = vec![Evidence { reference: "L2".into(), quote: "보관 기간도 정하십시오.".into() }];
        let planned = plan(&prepared, &request, &proposal, &fixture.knowledge, "job1").unwrap();

        let new_copy = planned.copy_text.as_ref().unwrap();
        let lines = copy::parse_headings(new_copy);
        assert!(lines[&3] > old_copy.lines[&3]);
        let moved = copy::link_target(&copy_file, lines[&3]);
        let added = copy::portable_link("projects/demo/bg.md", Provider::Claude, "sess-1", "job", "L2");
        let edited = &planned.rendered["projects/demo/bg.md"];
        assert!(edited.contains(&format!("경고 없이 로그만 남깁니다. ([근거]({moved}))\n\n보관 기간도 정합니다.")), "{edited}");
        assert!(edited.contains(&format!("- [L3]({moved}) — 그렇게 하겠습니다.\n- [L2]({added}) — 보관 기간도 정하십시오.")), "{edited}");
        assert_eq!(planned.rendered["projects/other/o.md"], format!("# 다른 프로젝트\n\n같은 대화 ([근거]({moved}))\n"));

        apply(&fixture.knowledge, &planned).unwrap();
        assert_eq!(fs::read_to_string(fixture.knowledge.join("projects/other/o.md")).unwrap(), planned.rendered["projects/other/o.md"]);
        assert!(fs::read_to_string(&copy_file).unwrap().contains("## L2 · 사용자"));
    }

    #[test]
    fn empty_proposal_plans_no_writes() {
        let fixture = fixture_with_two_lines();
        let prepared = ready(&fixture, 0, "");
        let request = build_request(&prepared, &BTreeMap::new(), &[], &[], 60_000);
        let empty = Proposal { summary: "없음".into(), changes: vec![], needs_notes: vec![] };
        let planned = plan(&prepared, &request, &empty, &fixture.knowledge, "job1").unwrap();
        assert!(planned.rendered.is_empty());
        assert!(planned.copy_file.is_none());
    }

    #[test]
    fn turn_times_are_given_to_the_model_in_local_time_with_the_offset() {
        let seoul = chrono::FixedOffset::east_opt(9 * 3600).unwrap();
        assert_eq!(local_stamp("2026-11-04T07:50:45.012Z", &seoul), "2026-11-04 16:50:45 +09:00");
        assert_eq!(local_stamp("2026-11-03T22:06:04.948Z", &seoul), "2026-11-04 07:06:04 +09:00");
        assert_eq!(local_stamp("", &seoul), "");
        assert_eq!(local_stamp("어제", &seoul), "어제");
    }

    #[test]
    fn ranking_query_prefers_user_turns() {
        let fixture = fixture_with_two_lines();
        let prepared = ready(&fixture, 0, "");
        assert_eq!(ranking_query(&prepared.batch), "경고는 빼고 로그만 남기십시오.");
    }

    #[test]
    fn a_waiting_job_is_named_from_its_transcript_and_a_started_one_keeps_what_the_worker_stored() {
        let fixture = fixture(&[user("u1", "시작")]);
        let waiting = identify(job(&fixture.transcript, 0, ""));
        assert_eq!((waiting.project.as_deref(), waiting.session_id.as_deref()), (Some("demo"), Some("sess-1")));
        let started = identify(Job { project: Some("stored".into()), session_id: Some("kept".into()), ..job(&fixture.transcript, 0, "") });
        assert_eq!((started.project.as_deref(), started.session_id.as_deref()), (Some("stored"), Some("kept")));
        let missing = identify(job(&fixture.dir.path().join("missing.jsonl"), 0, ""));
        assert_eq!((missing.project, missing.session_id), (None, None));
    }
}
