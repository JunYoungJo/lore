//! Applying approved candidates: one model call turns them into note changes, and the program
//! writes those changes only if each rests on what the user approved.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use chrono::{Local, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::candidate::{ApplyResult, Candidate, Kind, NoteAction, NoteResult, RelationKind, Scope, Speaker};
use crate::error::{Error, Result};
use crate::notes::{self, title_of, CatalogEntry};
use crate::prepare::{shown_notes, Ask, Search};
use crate::proposal::{authority_of, body_digest, check_path, check_snapshot, render_note, validate_edit, Change, Edit};
use crate::recorder::edits_schema;

pub const PROMPT: &str = include_str!("integrate-prompt.md");

/// The notes an apply batch writes, and what the user is told about them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub project: String,
    /// The notes this plan changes or removes, as they were when the plan was made. A note the
    /// plan creates is not among them.
    pub base: BTreeMap<String, String>,
    pub rendered: BTreeMap<String, String>,
    pub removed: BTreeSet<String>,
    pub result: ApplyResult,
}

impl Plan {
    /// Every note this plan writes or deletes.
    pub fn touched(&self) -> Vec<String> {
        self.rendered.keys().chain(self.removed.iter()).cloned().collect()
    }
}

fn basis_schema() -> Value {
    json!({"type": "array", "description": "The ids of the approved statements this rests on. At least one.", "items": {"type": "integer"}})
}

pub fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "summary": {"type": "string", "description": "One sentence for the app's list of what was recorded."},
            "changes": {"type": "array", "description": "One entry per note to create or edit.", "items": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "projects/<project>/<english-topic>.md or common/<english-topic>.md."},
                    "title": {"type": "string", "description": "New note: the topic and, when there is one, its conclusion. Existing note: empty to keep the title."},
                    "body": {"type": "string", "description": "New note: the complete Markdown body, without the title line. Existing note: empty."},
                    "edits": edits_schema(),
                    "basis": basis_schema()
                },
                "required": ["path", "title", "body", "edits", "basis"],
                "additionalProperties": false
            }},
            "removals": {"type": "array", "description": "Notes that an approved statement ends. Usually empty.", "items": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "The note an `obsolete` relation names."},
                    "reason": {"type": "string", "description": "One sentence: why a future agent no longer needs this note."},
                    "basis": basis_schema()
                },
                "required": ["path", "reason", "basis"],
                "additionalProperties": false
            }}
        },
        "required": ["summary", "changes", "removals"],
        "additionalProperties": false
    })
}

#[derive(Deserialize)]
struct Changed {
    path: String,
    title: String,
    body: String,
    edits: Vec<Edit>,
    basis: Vec<i64>,
}

#[derive(Deserialize)]
struct Removed {
    path: String,
    reason: String,
    basis: Vec<i64>,
}

#[derive(Deserialize)]
struct Answer {
    summary: String,
    changes: Vec<Changed>,
    removals: Vec<Removed>,
}

/// The note a candidate changes or ends, while that note still exists. A note that was renamed
/// or removed since the candidate was prepared no longer binds it: the candidate is new content.
fn bound_note<'a>(candidate: &'a Candidate, notes: &BTreeMap<String, String>) -> Option<&'a str> {
    let bound = matches!(candidate.relation.kind, RelationKind::Differs | RelationKind::Obsolete) && notes.contains_key(&candidate.relation.note);
    bound.then_some(candidate.relation.note.as_str())
}

/// The note a candidate ends still reads as it did when the user was shown it.
fn seen_as_it_is(candidate: &Candidate, notes: &BTreeMap<String, String>) -> bool {
    notes.get(&candidate.relation.note).is_some_and(|text| !candidate.relation.digest.is_empty() && body_digest(text) == candidate.relation.digest)
}

fn folder_scope(path: &str) -> Scope {
    if path.starts_with("common/") {
        Scope::Common
    } else {
        Scope::Project
    }
}

fn said_by(candidate: &Candidate) -> Speaker {
    if candidate.evidence.iter().any(|quote| quote.speaker == Speaker::User) {
        Speaker::User
    } else {
        Speaker::Agent
    }
}

fn date_of(stamp: &str) -> Option<&str> {
    let date = stamp.get(..10)?;
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    let mut parts = date.split('-');
    let well_formed = parts.next().is_some_and(|year| year.len() == 4 && digits(year)) && parts.next().is_some_and(digits) && parts.next().is_some_and(digits) && parts.next().is_none();
    well_formed.then_some(date)
}

fn found_on(candidate: &Candidate) -> String {
    Local.timestamp_opt(candidate.created_at, 0).single().map(|time| time.format("%Y-%m-%d").to_string()).unwrap_or_default()
}

/// The day a candidate was said: that of its first quotation, or the day it was found.
fn said_on(candidate: &Candidate) -> String {
    candidate.evidence.iter().find_map(|quote| date_of(&quote.at)).map(str::to_string).unwrap_or_else(|| found_on(candidate))
}

/// The lines a note's source list gets for the candidates it took in: what was said, by whom, and when.
fn source_lines(members: &[&Candidate]) -> Vec<String> {
    let mut lines = Vec::new();
    for candidate in members {
        for quote in &candidate.evidence {
            let date = date_of(&quote.at).map(str::to_string).unwrap_or_else(|| found_on(candidate));
            let words = quote.quote.split_whitespace().collect::<Vec<_>>().join(" ");
            lines.push(format!("- {date}, {}: \"{words}\"", quote.speaker.as_str()));
        }
    }
    lines
}

/// What the note rests on after taking in `members`: the user's own words, the agent's findings, or both.
fn authority(members: &[&Candidate], old: Option<&str>) -> String {
    let speakers: BTreeSet<&str> = members.iter().flat_map(|candidate| candidate.evidence.iter().map(|quote| quote.speaker.as_str())).collect();
    let new = match (speakers.contains("user"), speakers.contains("agent")) {
        (true, false) => "user_decision",
        (false, true) => "assistant_report",
        _ => "mixed_evidence",
    };
    match old.and_then(authority_of) {
        Some(previous) if previous != new => "mixed_evidence".to_string(),
        _ => new.to_string(),
    }
}

fn kind(members: &[&Candidate]) -> &'static str {
    if members.iter().all(|candidate| candidate.kind == Kind::Lesson) {
        "lesson"
    } else {
        "decision"
    }
}

fn members<'a>(basis: &[i64], candidates: &'a [Candidate], what: &str) -> Result<Vec<&'a Candidate>> {
    if basis.is_empty() {
        return Err(Error::rejected(format!("{what} names no approved statement in its basis.")));
    }
    basis
        .iter()
        .map(|id| usize::try_from(*id - 1).ok().and_then(|index| candidates.get(index)).ok_or_else(|| Error::rejected(format!("{what} names a statement that was not approved: {id}"))))
        .collect()
}

/// Turn approved candidates into the notes to write. Nothing is written here.
pub fn plan(project: &str, candidates: &[Candidate], notes: &BTreeMap<String, String>, previous_failure: Option<&str>, artifacts: &Path, ask: Ask, search: Search) -> Result<Plan> {
    let texts: Vec<&str> = candidates.iter().map(|candidate| candidate.text.as_str()).collect();
    let required: Vec<String> = candidates.iter().filter_map(|candidate| bound_note(candidate, notes)).map(str::to_string).collect();
    let shown = shown_notes(&texts, &required, notes, search);
    let catalog: Vec<CatalogEntry> = notes.iter().filter(|(path, _)| !shown.contains_key(*path)).map(|(path, text)| CatalogEntry { path: path.clone(), title: title_of(text) }).collect();
    let approved: Vec<Value> = candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            let relation = match bound_note(candidate, notes) {
                Some(note) if candidate.relation.kind == RelationKind::Obsolete => json!({"type": candidate.relation.kind, "note": note, "note_changed": !seen_as_it_is(candidate, notes)}),
                Some(note) => json!({"type": candidate.relation.kind, "note": note}),
                None => json!({"type": RelationKind::New, "note": ""}),
            };
            json!({"id": index + 1, "kind": candidate.kind, "text": candidate.text, "said_by": said_by(candidate), "date": said_on(candidate), "scope": candidate.scope, "relation": relation})
        })
        .collect();
    let language = candidates.iter().map(|candidate| candidate.language.as_str()).find(|language| !language.is_empty()).unwrap_or("English");
    let mut payload = json!({"project": project, "language": language, "approved": approved, "notes": shown, "note_catalog": catalog});
    if let Some(reason) = previous_failure {
        payload["previous_failure"] = json!(reason);
    }
    let prompt = PROMPT.replacen("{{data}}", &payload.to_string(), 1);
    let answer: Answer = serde_json::from_value(ask(&prompt, &schema(), artifacts)?).map_err(|error| Error::rejected(format!("Invalid model response format: {error}")))?;

    let folders = vec!["common/".to_string(), format!("projects/{project}/")];
    let mut claimed = BTreeSet::new();
    let mut claim = |path: &str| match claimed.insert(path.to_ascii_lowercase()) {
        true => Ok(()),
        false => Err(Error::rejected(format!("A note cannot be changed twice in one answer: {path}"))),
    };
    let mut used: BTreeSet<i64> = BTreeSet::new();
    // Which statements each note was changed or removed for.
    let mut served: BTreeMap<&str, BTreeSet<i64>> = BTreeMap::new();
    let mut result = Plan { project: project.to_string(), base: BTreeMap::new(), rendered: BTreeMap::new(), removed: BTreeSet::new(), result: ApplyResult::default() };

    for removal in &answer.removals {
        check_path(&removal.path, &folders)?;
        claim(&removal.path)?;
        let basis = members(&removal.basis, candidates, &format!("The removal of {}", removal.path))?;
        if !notes.contains_key(&removal.path) {
            return Err(Error::rejected(format!("The note to remove does not exist: {}", removal.path)));
        }
        if removal.reason.trim().is_empty() {
            return Err(Error::rejected(format!("Removing a note requires a reason: {}", removal.path)));
        }
        let ending: Vec<&&Candidate> = basis.iter().filter(|candidate| candidate.relation.kind == RelationKind::Obsolete && bound_note(candidate, notes) == Some(removal.path.as_str())).collect();
        if ending.is_empty() {
            return Err(Error::rejected(format!("Only a note that an approved statement ends (relation obsolete) can be removed. Edit this note instead: {}", removal.path)));
        }
        // The user approved the end of the note they were shown. What was added to it since, they have not seen.
        if !ending.iter().any(|candidate| seen_as_it_is(candidate, notes)) {
            return Err(Error::rejected(format!("{} was changed after the user was shown it (note_changed), so it cannot be removed whole. Edit it instead: take out what the statement ends and keep the rest.", removal.path)));
        }
        used.extend(removal.basis.iter().copied());
        served.entry(removal.path.as_str()).or_default().extend(removal.basis.iter().copied());
        result.removed.insert(removal.path.clone());
        result.result.notes.push(NoteResult { path: removal.path.clone(), action: NoteAction::Removed, items: basis.iter().map(|candidate| candidate.text.clone()).collect() });
    }

    for change in &answer.changes {
        check_path(&change.path, &folders)?;
        claim(&change.path)?;
        let basis = members(&change.basis, candidates, &format!("The change to {}", change.path))?;
        // A statement goes only into the folder its scope names. A statement about one project
        // in a shared note would reach every project, and one the user wants everywhere would
        // stay in one project.
        if let Some(stray) = basis.iter().position(|candidate| candidate.scope != folder_scope(&change.path)) {
            let id = change.basis[stray];
            return Err(Error::rejected(match folder_scope(&change.path) {
                Scope::Common => format!("Statement {id} has scope project, so it cannot go into {}, which every project reads. Put it into a note under projects/{project}/.", change.path),
                Scope::Project => format!("Statement {id} has scope common, so it goes into a note under common/, not into {}.", change.path),
            }));
        }
        let existing = notes.keys().find(|known| known.eq_ignore_ascii_case(&change.path));
        match existing {
            Some(known) => {
                if *known != change.path {
                    return Err(Error::rejected(format!("The path differs from an existing note only by letter case: {}", change.path)));
                }
                if !shown.contains_key(known) {
                    return Err(Error::rejected(format!("Cannot edit an existing note whose text was not provided: {}", change.path)));
                }
                if !change.body.trim().is_empty() || change.edits.is_empty() {
                    return Err(Error::rejected(format!("Existing notes must use targeted edits rather than a full replacement: {}", change.path)));
                }
                for edit in &change.edits {
                    validate_edit(edit, &change.path)?;
                }
            }
            None => {
                if change.title.trim().is_empty() || change.body.trim().is_empty() || !change.edits.is_empty() {
                    return Err(Error::rejected(format!("New notes require a title and complete body: {}", change.path)));
                }
            }
        }
        let old = notes.get(&change.path).map(String::as_str);
        let note = Change {
            path: change.path.clone(),
            title: change.title.clone(),
            kind: kind(&basis).to_string(),
            authority: authority(&basis, old),
            body: change.body.clone(),
            edits: change.edits.clone(),
            sources_from: Vec::new(),
        };
        result.rendered.insert(change.path.clone(), render_note(&note, project, old, &source_lines(&basis))?);
        used.extend(change.basis.iter().copied());
        served.entry(change.path.as_str()).or_default().extend(change.basis.iter().copied());
        let action = if existing.is_some() { NoteAction::Edited } else { NoteAction::Created };
        result.result.notes.push(NoteResult { path: change.path.clone(), action, items: basis.iter().map(|candidate| candidate.text.clone()).collect() });
    }

    let missing: Vec<String> = (1..=candidates.len() as i64).filter(|id| !used.contains(id)).map(|id| id.to_string()).collect();
    if !missing.is_empty() {
        return Err(Error::rejected(format!("Every approved statement must be in the basis of a change or removal. Left out: {}", missing.join(", "))));
    }
    for (index, candidate) in candidates.iter().enumerate() {
        let id = index as i64 + 1;
        if let Some(note) = bound_note(candidate, notes) {
            if !served.get(note).is_some_and(|ids| ids.contains(&id)) {
                return Err(Error::rejected(format!("Statement {id} changes what {note} says, so that note must be edited (or removed, when the statement ends it) with {id} in its basis.")));
            }
        }
    }
    result.result.summary = answer.summary.replace(['\r', '\n'], " ").trim().to_string();
    result.base = notes.iter().filter(|(path, _)| result.rendered.contains_key(*path) || result.removed.contains(*path)).map(|(path, text)| (path.clone(), text.clone())).collect();
    Ok(result)
}

/// The notes a plan touches, as they are in the folder now.
fn in_folder(knowledge_root: &Path, plan: &Plan) -> Result<BTreeMap<String, String>> {
    let mut current = BTreeMap::new();
    for note in plan.rendered.keys().chain(plan.removed.iter()) {
        match std::fs::read_to_string(knowledge_root.join(note)) {
            Ok(text) => {
                current.insert(note.clone(), text);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(current)
}

/// Write a plan's notes, unless one of them changed since the plan was made. Notes the plan
/// does not touch are not looked at: a change to them does not make the plan wrong.
pub fn apply(knowledge_root: &Path, plan: &Plan) -> Result<()> {
    check_snapshot(&in_folder(knowledge_root, plan)?, &plan.base, &plan.rendered, &plan.removed)?;
    notes::write_notes(knowledge_root, &plan.rendered, &plan.removed)
}

/// Take back what a plan wrote before it was given up: a note that reads as the plan left it
/// goes back to what it said before, a note the plan created goes away, and a note the plan
/// removed comes back. A note someone has changed since is left as it is. Returns how many
/// notes were put back.
pub fn undo(knowledge_root: &Path, plan: &Plan) -> Result<usize> {
    let current = in_folder(knowledge_root, plan)?;
    let mut restored = BTreeMap::new();
    let mut deleted = BTreeSet::new();
    for (note, text) in &plan.rendered {
        if current.get(note) != Some(text) {
            continue;
        }
        match plan.base.get(note) {
            Some(before) if before != text => {
                restored.insert(note.clone(), before.clone());
            }
            Some(_) => {}
            None => {
                deleted.insert(note.clone());
            }
        }
    }
    for note in &plan.removed {
        if let (None, Some(before)) = (current.get(note), plan.base.get(note)) {
            restored.insert(note.clone(), before.clone());
        }
    }
    notes::write_notes(knowledge_root, &restored, &deleted)?;
    Ok(restored.len() + deleted.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::{Quote, Relation, Source, State};
    use crate::transcript::Provider;
    use std::cell::RefCell;

    fn quote(speaker: Speaker, words: &str) -> Quote {
        Quote { reference: "L5".into(), quote: words.into(), speaker, at: "2026-11-17 10:12:00 +09:00".into() }
    }

    fn approved(text: &str) -> Candidate {
        Candidate {
            id: 40,
            job_id: "job".into(),
            session_id: "s".into(),
            provider: Provider::Claude,
            project: "demo".into(),
            language: "English".into(),
            kind: Kind::Decision,
            text: text.into(),
            source: Source::Found,
            scope: Scope::Project,
            evidence: vec![quote(Speaker::User, "no alerts\nfor finished jobs")],
            relation: Relation::new(),
            state: State::Approved,
            edited: false,
            created_at: 1_794_000_000,
            waiting_since: 1_794_000_000,
            apply_id: Some(1),
        }
    }

    /// A statement that changes or ends `note`, which the user was shown as it reads in `notes()`.
    fn bound(text: &str, kind: RelationKind, note: &str) -> Candidate {
        let digest = notes().get(note).map(|text| body_digest(text)).unwrap_or_default();
        Candidate { scope: folder_scope(note), relation: Relation::to(kind, note, "T", vec!["x".into()], digest), ..approved(text) }
    }

    fn notes() -> BTreeMap<String, String> {
        BTreeMap::from([
            ("projects/demo/alerts.md".to_string(), "---\nqmd:\n  metadata:\n    project: \"demo\"\n    kind: \"decision\"\n    authority: \"assistant_report\"\n---\n\n# Alerts\n\nEvery finished job raises a desktop alert.\n\n## Sources\n- 2026-10-02, agent: \"raise alerts\"\n".to_string()),
            ("projects/demo/export.md".to_string(), "# CSV export\n\nThe export pages through rows.\n".to_string()),
            ("common/style.md".to_string(), "# Style\n\nAnswer in short sentences.\n".to_string()),
        ])
    }

    fn all(_: &str) -> Vec<String> {
        vec!["projects/demo/alerts.md".to_string(), "projects/demo/export.md".to_string()]
    }

    fn run(candidates: &[Candidate], answer: Value) -> Result<Plan> {
        let prompts = RefCell::new(Vec::new());
        let mut ask = |prompt: &str, _: &Value, _: &Path| {
            prompts.borrow_mut().push(prompt.to_string());
            Ok(answer.clone())
        };
        plan("demo", candidates, &notes(), None, Path::new("/tmp/artifacts"), &mut ask, &all)
    }

    fn edit(find: &str, text: &str) -> Value {
        json!({"op": "replace", "find": find, "section": "", "heading": "", "text": text})
    }

    const NO_ALERT: &str = "Finished jobs raise no desktop alert.";

    #[test]
    fn an_approved_statement_changes_the_note_it_differs_from_and_adds_what_was_said() {
        let candidates = [bound(NO_ALERT, RelationKind::Differs, "projects/demo/alerts.md")];
        let answer = json!({"summary": "Alerts are off\nfor finished jobs.", "removals": [], "changes": [
            {"path": "projects/demo/alerts.md", "title": "", "body": "", "basis": [1], "edits": [edit("Every finished job raises a desktop alert.", "Finished jobs raise no desktop alert (user decision, 2026-11-17).")]},
        ]});
        let plan = run(&candidates, answer).unwrap();
        let note = &plan.rendered["projects/demo/alerts.md"];
        assert!(note.contains("Finished jobs raise no desktop alert (user decision, 2026-11-17)."));
        // The earlier source line stays and the user's words join it, on one line.
        assert!(note.ends_with("## Sources\n- 2026-10-02, agent: \"raise alerts\"\n- 2026-11-17, user: \"no alerts for finished jobs\"\n"));
        // The note rested on the assistant and now also on the user.
        assert!(note.contains("authority: \"mixed_evidence\"") && note.contains("kind: \"decision\""));
        assert_eq!(plan.result.summary, "Alerts are off for finished jobs.");
        assert_eq!(plan.result.notes, vec![NoteResult { path: "projects/demo/alerts.md".into(), action: NoteAction::Edited, items: vec![NO_ALERT.into()] }]);
        // The plan keeps what the notes it touches said before, and no other note.
        assert!(plan.removed.is_empty());
        assert_eq!(plan.base, BTreeMap::from([("projects/demo/alerts.md".to_string(), notes()["projects/demo/alerts.md"].clone())]));
    }

    #[test]
    fn new_content_becomes_a_note_in_the_folder_of_its_scope() {
        let lesson = Candidate { kind: Kind::Lesson, evidence: vec![quote(Speaker::Agent, "paging fixed it")], scope: Scope::Common, ..approved("Loading every row at once times out.") };
        let answer = |path: &str| json!({"summary": "Recorded.", "removals": [], "changes": [{"path": path, "title": "Load rows in pages", "body": "Loading every row at once times out.", "edits": [], "basis": [1]}]});
        let plan = run(std::slice::from_ref(&lesson), answer("common/paging.md")).unwrap();
        let note = &plan.rendered["common/paging.md"];
        assert!(note.starts_with("---\nqmd:\n  metadata:\n    project: \"common\"\n    kind: \"lesson\"\n    authority: \"assistant_report\"\n---\n\n# Load rows in pages\n"));
        assert!(note.ends_with("## Sources\n- 2026-11-17, agent: \"paging fixed it\"\n"));
        assert_eq!(plan.result.notes[0].action, NoteAction::Created);
        // A statement for all projects does not become a note of one project, nor the reverse.
        assert!(run(std::slice::from_ref(&lesson), answer("projects/demo/paging.md")).unwrap_err().message.contains("has scope common"));
        assert!(run(&[approved(NO_ALERT)], answer("common/paging.md")).unwrap_err().message.contains("has scope project"));
        assert!(run(&[approved(NO_ALERT)], answer("projects/other/paging.md")).unwrap_err().message.contains("Invalid memory path"));
    }

    #[test]
    fn a_statement_goes_only_into_notes_of_the_folder_its_scope_names() {
        // Added to an existing shared note, a statement about this project would reach every project.
        let into_shared = json!({"summary": "s", "removals": [], "changes": [{"path": "common/style.md", "title": "", "body": "", "basis": [1], "edits": [edit("short sentences.", "short sentences. Finished jobs raise no desktop alert.")]}]});
        let error = run(&[approved(NO_ALERT)], into_shared).unwrap_err();
        assert!(error.failure == crate::error::Failure::Rejected && error.message.contains("Statement 1 has scope project") && error.message.contains("projects/demo/"), "{}", error.message);
        // One note cannot take a statement for this project together with one for all projects.
        let shared = Candidate { scope: Scope::Common, ..approved("Answer in brief sentences.") };
        let mixed = json!({"summary": "s", "removals": [], "changes": [{"path": "projects/demo/alerts.md", "title": "", "body": "", "basis": [1, 2], "edits": [edit("Every finished job raises a desktop alert.", NO_ALERT)]}]});
        assert!(run(&[approved(NO_ALERT), shared.clone()], mixed).unwrap_err().message.contains("Statement 2 has scope common"));
        let apart = json!({"summary": "s", "removals": [], "changes": [
            {"path": "projects/demo/alerts.md", "title": "", "body": "", "basis": [1], "edits": [edit("Every finished job raises a desktop alert.", NO_ALERT)]},
            {"path": "common/brief-answers.md", "title": "Brief answers", "body": "Answer in brief sentences.", "edits": [], "basis": [2]},
        ]});
        let plan = run(&[approved(NO_ALERT), shared], apart).unwrap();
        assert_eq!(plan.rendered.keys().collect::<Vec<_>>(), vec!["common/brief-answers.md", "projects/demo/alerts.md"]);
    }

    #[test]
    fn only_a_note_that_an_approved_statement_ends_is_removed() {
        let gone = bound("The CSV export was removed.", RelationKind::Obsolete, "projects/demo/export.md");
        let removal = |path: &str| json!({"summary": "Removed.", "changes": [], "removals": [{"path": path, "reason": "The export is gone.", "basis": [1]}]});
        let plan = run(std::slice::from_ref(&gone), removal("projects/demo/export.md")).unwrap();
        assert_eq!(plan.removed.iter().collect::<Vec<_>>(), vec!["projects/demo/export.md"]);
        assert_eq!(plan.result.notes[0].action, NoteAction::Removed);
        assert!(run(std::slice::from_ref(&gone), removal("projects/demo/alerts.md")).unwrap_err().message.contains("can be removed"));
        let differs = bound(NO_ALERT, RelationKind::Differs, "projects/demo/alerts.md");
        assert!(run(&[differs], removal("projects/demo/alerts.md")).unwrap_err().message.contains("can be removed"));

        // The note was added to after the user was shown it: what they approved the end of is
        // not what is there now, so it is edited, not removed.
        let stale = Candidate { relation: Relation { digest: body_digest("# CSV export\n\nThe export writes a BOM.\n"), ..gone.relation.clone() }, ..gone.clone() };
        let error = run(std::slice::from_ref(&stale), removal("projects/demo/export.md")).unwrap_err();
        assert!(error.failure == crate::error::Failure::Rejected && error.message.contains("changed after the user was shown it"), "{}", error.message);
        let trimmed = json!({"summary": "s", "removals": [], "changes": [{"path": "projects/demo/export.md", "title": "", "body": "", "basis": [1], "edits": [edit("The export pages through rows.", "The CSV export was removed.")]}]});
        assert!(run(std::slice::from_ref(&stale), trimmed).is_ok());
        // A source list that grew is not a change to what the note says.
        let sourced = Candidate { relation: Relation { digest: body_digest("# CSV export\n\nThe export pages through rows.\n\n## Sources\n- 2026-11-01, user: \"page it\"\n"), ..gone.relation.clone() }, ..gone };
        assert!(run(&[sourced], removal("projects/demo/export.md")).is_ok());
    }

    #[test]
    fn every_statement_is_used_and_a_bound_note_is_the_one_changed() {
        let two = [approved(NO_ALERT), approved("Use short sentences.")];
        let one_used = json!({"summary": "s", "removals": [], "changes": [{"path": "projects/demo/alerts.md", "title": "", "body": "", "basis": [1], "edits": [edit("Every finished job raises a desktop alert.", NO_ALERT)]}]});
        assert!(run(&two, one_used).unwrap_err().message.contains("Left out: 2"));
        let unknown = json!({"summary": "s", "removals": [], "changes": [{"path": "projects/demo/alerts.md", "title": "", "body": "", "basis": [7], "edits": [edit("alert", "x")]}]});
        assert!(run(&two, unknown).unwrap_err().message.contains("not approved"));
        let none = json!({"summary": "s", "removals": [], "changes": [{"path": "projects/demo/alerts.md", "title": "", "body": "", "basis": [], "edits": [edit("alert", "x")]}]});
        assert!(run(&two, none).unwrap_err().message.contains("names no approved statement"));
        // The statement differs from alerts.md, so writing it somewhere else leaves the contradiction in place.
        let elsewhere = json!({"summary": "s", "removals": [], "changes": [{"path": "projects/demo/quiet.md", "title": "Quiet", "body": NO_ALERT, "edits": [], "basis": [1]}]});
        assert!(run(&[bound(NO_ALERT, RelationKind::Differs, "projects/demo/alerts.md")], elsewhere.clone()).unwrap_err().message.contains("must be edited"));
        // A note that no longer exists binds nothing.
        assert!(run(&[bound(NO_ALERT, RelationKind::Differs, "projects/demo/renamed.md")], elsewhere).is_ok());
    }

    #[test]
    fn existing_notes_take_targeted_edits_and_unseen_notes_none() {
        let candidates = [approved(NO_ALERT)];
        let whole = json!({"summary": "s", "removals": [], "changes": [{"path": "projects/demo/alerts.md", "title": "", "body": "Rewritten.", "edits": [], "basis": [1]}]});
        assert!(run(&candidates, whole).unwrap_err().message.contains("targeted edits"));
        let unseen = json!({"summary": "s", "removals": [], "changes": [{"path": "common/style.md", "title": "", "body": "", "basis": [1], "edits": [edit("short", "brief")]}]});
        let shared = Candidate { scope: Scope::Common, ..approved("Answer in brief sentences.") };
        assert!(run(&[shared], unseen).unwrap_err().message.contains("was not provided"));
        let twice = json!({"summary": "s", "removals": [], "changes": [
            {"path": "projects/demo/alerts.md", "title": "", "body": "", "basis": [1], "edits": [edit("Every", "No")]},
            {"path": "projects/demo/Alerts.md", "title": "", "body": "", "basis": [1], "edits": [edit("Every", "No")]},
        ]});
        assert!(run(&candidates, twice).unwrap_err().message.contains("twice"));
        let absent = json!({"summary": "s", "removals": [], "changes": [{"path": "projects/demo/alerts.md", "title": "", "body": "", "basis": [1], "edits": [edit("not in the note", "x")]}]});
        assert!(run(&candidates, absent).unwrap_err().message.contains("was not found"));
    }

    #[test]
    fn the_model_is_shown_the_statements_and_the_notes_without_their_sources() {
        let candidates = [bound(NO_ALERT, RelationKind::Differs, "common/style.md")];
        let prompts = RefCell::new(Vec::new());
        let mut ask = |prompt: &str, _: &Value, _: &Path| {
            prompts.borrow_mut().push(prompt.to_string());
            Err::<Value, _>(Error::transient("stop here"))
        };
        let _ = plan("demo", &candidates, &notes(), Some("the last answer left a statement out"), Path::new("/tmp/a"), &mut ask, &|_| vec!["projects/demo/alerts.md".to_string()]);
        let prompt = prompts.borrow()[0].clone();
        assert!(prompt.contains(r#""approved":[{"id":1,"kind":"decision","text":"Finished jobs raise no desktop alert.","said_by":"user","date":"2026-11-17","scope":"common","relation":{"type":"differs","note":"common/style.md"}}]"#));
        // The note the statement differs from is shown even though the search did not find it.
        assert!(prompt.contains("Answer in short sentences.") && prompt.contains("Every finished job raises a desktop alert."));
        assert!(!prompt.contains("raise alerts"));
        assert!(prompt.contains(r#""note_catalog":[{"path":"projects/demo/export.md","title":"CSV export"}]"#));
        assert!(prompt.contains("the last answer left a statement out"));
        assert!(prompt.find("\"approved\"").unwrap() < prompt.find("# Your task").unwrap());
        assert!(!PROMPT.chars().any(|c| ('\u{AC00}'..='\u{D7A3}').contains(&c)));

        // For a note a statement ends, the model is told whether it still reads as the user saw it.
        let gone = bound("The CSV export was removed.", RelationKind::Obsolete, "projects/demo/export.md");
        let stale = Candidate { relation: Relation { digest: "other".into(), ..gone.relation.clone() }, ..gone.clone() };
        for (candidate, changed) in [(gone, false), (stale, true)] {
            let _ = plan("demo", &[candidate], &notes(), None, Path::new("/tmp/a"), &mut ask, &|_| Vec::new());
            let prompt = prompts.borrow().last().unwrap().clone();
            assert!(prompt.contains(&format!(r#""relation":{{"type":"obsolete","note":"projects/demo/export.md","note_changed":{changed}}}"#)), "{prompt}");
        }
        for word in ["`note_changed`", "`scope`", "`basis`", "`previous_failure`"] {
            assert!(PROMPT.contains(word), "{word}");
        }
    }

    #[test]
    fn a_plan_is_written_only_while_the_notes_are_as_it_saw_them() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("projects").join("demo");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("alerts.md"), "# Alerts\n\nold\n").unwrap();
        std::fs::write(folder.join("export.md"), "# Export\n\nold\n").unwrap();
        let base = notes::load_scope(dir.path(), "demo").unwrap();
        let plan = Plan {
            project: "demo".into(),
            base,
            rendered: BTreeMap::from([("projects/demo/alerts.md".to_string(), "# Alerts\n\nnew\n".to_string())]),
            removed: BTreeSet::from(["projects/demo/export.md".to_string()]),
            result: ApplyResult::default(),
        };
        apply(dir.path(), &plan).unwrap();
        assert_eq!(std::fs::read_to_string(folder.join("alerts.md")).unwrap(), "# Alerts\n\nnew\n");
        assert!(!folder.join("export.md").exists());
        // Applying again after an interruption is not an outside edit.
        apply(dir.path(), &plan).unwrap();
        // Neither is a change to a note the plan does not touch, such as one a sync brought in.
        std::fs::write(folder.join("unrelated.md"), "# Unrelated\n\nfrom another PC\n").unwrap();
        apply(dir.path(), &plan).unwrap();
        std::fs::write(folder.join("alerts.md"), "# Alerts\n\nedited by hand\n").unwrap();
        assert!(apply(dir.path(), &plan).unwrap_err().message.contains("changed after"));
    }

    #[test]
    fn a_plan_that_is_given_up_takes_back_what_it_wrote() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("projects").join("demo");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("alerts.md"), "# Alerts\n\nold\n").unwrap();
        std::fs::write(folder.join("export.md"), "# Export\n\nold\n").unwrap();
        std::fs::write(folder.join("style.md"), "# Style\n\nold\n").unwrap();
        let base = BTreeMap::from([
            ("projects/demo/alerts.md".to_string(), "# Alerts\n\nold\n".to_string()),
            ("projects/demo/export.md".to_string(), "# Export\n\nold\n".to_string()),
            ("projects/demo/style.md".to_string(), "# Style\n\nold\n".to_string()),
        ]);
        let plan = Plan {
            project: "demo".into(),
            base,
            rendered: BTreeMap::from([
                ("projects/demo/alerts.md".to_string(), "# Alerts\n\nnew\n".to_string()),
                ("projects/demo/paging.md".to_string(), "# Paging\n\nnew\n".to_string()),
                ("projects/demo/style.md".to_string(), "# Style\n\nnew\n".to_string()),
            ]),
            removed: BTreeSet::from(["projects/demo/export.md".to_string()]),
            result: ApplyResult::default(),
        };
        // Nothing was written yet: there is nothing to take back.
        assert_eq!(undo(dir.path(), &plan).unwrap(), 0);
        apply(dir.path(), &plan).unwrap();
        // Someone changed one of the written notes before the plan was given up.
        std::fs::write(folder.join("style.md"), "# Style\n\nedited by hand\n").unwrap();
        assert_eq!(undo(dir.path(), &plan).unwrap(), 3);
        assert_eq!(std::fs::read_to_string(folder.join("alerts.md")).unwrap(), "# Alerts\n\nold\n");
        assert_eq!(std::fs::read_to_string(folder.join("export.md")).unwrap(), "# Export\n\nold\n");
        assert!(!folder.join("paging.md").exists());
        assert_eq!(std::fs::read_to_string(folder.join("style.md")).unwrap(), "# Style\n\nedited by hand\n");
        assert_eq!(undo(dir.path(), &plan).unwrap(), 0);
    }
}
