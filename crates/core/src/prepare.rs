//! Preparing candidates: what the marks of one step of a session become, and how each stands to the notes.
//!
//! The session marked what is to be remembered, with whose it is and the user's words. The
//! program checks those words against what the user said in the session. One model call reads
//! the notes of the memory folder and says, for each mark, how it stands to them and whether a
//! candidate that already waits states it. Nothing is looked for in the conversation: what the
//! session did not mark is not offered.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use chrono::Local;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::candidate::{Found, Kind, Quote, Relation, RelationKind, Scope, Speaker};
use crate::error::{Error, Result};
use crate::mark::{Change, Marked, Target};
use crate::notes::title_of;
use crate::proposal::body_digest;
use crate::qmd::search_collections;
use crate::session::{local_stamp, Session};
use crate::transcript::{Origin, Turn};

pub const PROMPT: &str = include_str!("prepare-prompt.md");

/// Send a prompt and a JSON schema to the recording model; get its JSON answer. The model may
/// read the memory folder that the prompt names but change nothing.
pub type Ask<'a> = &'a mut dyn FnMut(&str, &Value, &Path) -> Result<Value>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Prepared {
    /// What the user is asked about.
    pub offered: Vec<Found>,
    /// What a note already says.
    pub left_out: Vec<Found>,
}

/// A candidate that waits for the user, as the model is shown it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiting {
    pub id: i64,
    pub kind: Kind,
    pub text: String,
}

fn relation_schema() -> Value {
    json!({"type": "object", "description": "How the mark stands to the notes in the memory folder.", "properties": {
        "type": {"type": "string", "enum": ["new", "same", "differs", "obsolete"], "description": "new: no note is about it. same: a note already states it. differs: a note states something it changes. obsolete: it ends what a note is about."},
        "note": {"type": "string", "description": "same, differs, obsolete: the note's path relative to the memory folder, such as \"projects/demo/alerts.md\". new: empty."},
        "quotes": {"type": "array", "description": "same, differs, obsolete: one to three passages copied exactly from that note. new: empty.", "items": {"type": "string"}}
    }, "required": ["type", "note", "quotes"], "additionalProperties": false})
}

// Every field is required so both CLIs emit them in this order.
pub fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "marks": {"type": "array", "description": "One entry for every mark, in the order of `marks`.", "items": {
                "type": "object",
                "properties": {
                    "mark": {"type": "integer", "description": "The position of the mark in `marks`, counting from 0."},
                    "waiting": {"type": "integer", "description": "The id of a candidate in `waiting` that already states what the mark states, or 0."},
                    "relation": relation_schema()
                },
                "required": ["mark", "waiting", "relation"],
                "additionalProperties": false
            }}
        },
        "required": ["marks"],
        "additionalProperties": false
    })
}

#[derive(Deserialize)]
struct Related {
    #[serde(rename = "type")]
    kind: RelationKind,
    note: String,
    quotes: Vec<String>,
}

#[derive(Deserialize)]
struct Placed {
    mark: i64,
    waiting: i64,
    relation: Related,
}

#[derive(Deserialize)]
struct Answer {
    marks: Vec<Placed>,
}

/// A quotation shorter than this must be the whole turn. A few characters lifted out of a long
/// turn are in it, but they show nothing of what was said.
const QUOTE_MIN: usize = 10;

/// The words a quotation takes from a turn, as they are kept. Line ends are compared as written
/// on any system, since a turn pasted on Windows carries its own.
fn quoted(turn: &Turn, quote: &str) -> Option<String> {
    let said = turn.text.replace("\r\n", "\n");
    let words = quote.replace("\r\n", "\n").trim().to_string();
    let whole = said.trim() == words;
    (!words.is_empty() && said.contains(&words) && (whole || words.chars().count() >= QUOTE_MIN)).then_some(words)
}

/// What a mark rests on, as the user is shown it and the note's sources list it. A mark of the
/// agent carries its own sentence as the agent's words. Of the user's words the session copied,
/// only what the user said before the mark is kept: words the program cannot find in a turn of
/// the user are not shown as the user's.
fn evidence(marked: &Marked, turns: &[Turn]) -> Vec<Quote> {
    let mut evidence = Vec::new();
    if marked.mark.by == Speaker::Agent {
        evidence.push(Quote { reference: format!("L{}", marked.line), quote: marked.mark.text.clone(), speaker: Speaker::Agent, at: local_stamp(&marked.at, &Local) });
    }
    for words in &marked.mark.said {
        let found = turns.iter().rev().filter(|turn| turn.line < marked.line && turn.role == "user" && turn.origin == Origin::User).find_map(|turn| quoted(turn, words).map(|quote| (turn, quote)));
        let Some((turn, quote)) = found else { continue };
        if evidence.iter().any(|known: &Quote| known.reference == turn.reference && known.quote == quote) {
            continue;
        }
        evidence.push(Quote { reference: turn.reference.clone(), quote, speaker: Speaker::User, at: local_stamp(&turn.timestamp, &Local) });
    }
    evidence
}

fn folder_scope(note: &str) -> Scope {
    if note.starts_with("common/") {
        Scope::Common
    } else {
        Scope::Project
    }
}

/// The relation as the user is shown it. The note must be one of the notes this project may
/// change, and every passage must be in it word for word.
fn related(related: Related, notes: &BTreeMap<String, String>, project: &str) -> Result<Relation> {
    if related.kind == RelationKind::New {
        return Ok(Relation::new());
    }
    let path = related.note.trim().replace('\\', "/");
    let path = path.trim_start_matches("./");
    let Some(note) = notes.get(path) else {
        return Err(Error::rejected(format!("The relation names a note that is not one of the notes in common/ or projects/{project}/: {path}")));
    };
    let quotes: Vec<String> = related.quotes.iter().map(|quote| quote.replace("\r\n", "\n").trim().to_string()).filter(|quote| !quote.is_empty()).collect();
    if quotes.is_empty() {
        return Err(Error::rejected(format!("A relation to {path} quotes no passage of it.")));
    }
    let text = note.replace("\r\n", "\n");
    if let Some(missing) = quotes.iter().find(|quote| !text.contains(quote.as_str())) {
        return Err(Error::rejected(format!("A quoted passage is not in {path} word for word: \"{}\"", missing.chars().take(80).collect::<String>())));
    }
    Ok(Relation::to(related.kind, path, &title_of(note), quotes, body_digest(note)))
}

/// The path, relative to the memory folder, of a note a session named. It may name it as qmd
/// lists it (`project-<project>/…`, also as a `qmd://` link), with backslashes, or by its full path.
fn note_path(given: &str, memory: &Path) -> String {
    let mut path = given.trim().trim_start_matches("qmd://").replace('\\', "/");
    let root = memory.to_string_lossy().replace('\\', "/");
    let root = root.trim_end_matches('/');
    let inside = path.get(..root.len()).is_some_and(|start| start.eq_ignore_ascii_case(root)) && path[root.len()..].starts_with('/');
    if inside {
        path = path[root.len() + 1..].to_string();
    }
    let path = path.trim_start_matches("./");
    match path.strip_prefix("project-") {
        Some(rest) => format!("projects/{rest}"),
        None => path.to_string(),
    }
}

/// How a mark stands to the note the session named, when that note is one of the notes this
/// project may change and every passage the session copied is in it word for word. Otherwise the
/// session's choice is not used and the mark is compared with the notes like any other.
fn named(target: &Target, notes: &BTreeMap<String, String>, memory: &Path) -> Option<Relation> {
    let path = note_path(&target.path, memory);
    let note = notes.get(&path)?;
    let text = note.replace("\r\n", "\n");
    let quotes: Vec<String> = target.quotes.iter().map(|quote| quote.replace("\r\n", "\n").trim().to_string()).filter(|quote| !quote.is_empty()).collect();
    if quotes.is_empty() || quotes.iter().any(|quote| !text.contains(quote.as_str())) {
        return None;
    }
    let kind = match target.change {
        Change::Updates => RelationKind::Differs,
        Change::Retires => RelationKind::Obsolete,
    };
    Some(Relation::to(kind, &path, &title_of(note), quotes, body_digest(note)))
}

/// Ask the model how the marks at `asked` stand to the notes and to what waits. Returns, for
/// each of them in that order, the relation and the id of the waiting candidate that states it (0 for none).
#[allow(clippy::too_many_arguments)]
fn compare(session: &Session, asked: &[&Marked], notes: &BTreeMap<String, String>, waiting: &[Waiting], memory: &Path, previous_failure: Option<&str>, artifacts: &Path, ask: Ask) -> Result<Vec<(Relation, i64)>> {
    let shown: Vec<Value> = asked.iter().map(|marked| json!({"kind": marked.mark.kind, "text": marked.mark.text, "scope": marked.mark.scope})).collect();
    let mut payload = json!({
        "project": session.project,
        "memory_folder": memory.to_string_lossy().replace('\\', "/"),
        "note_folders": ["common/", format!("projects/{}/", session.project)],
        "search_collections": search_collections(&session.project),
        "waiting": waiting.iter().map(|candidate| json!({"id": candidate.id, "kind": candidate.kind, "text": candidate.text})).collect::<Vec<_>>(),
        "marks": shown,
    });
    if let Some(reason) = previous_failure {
        payload["previous_failure"] = json!(reason);
    }
    // The data goes where the prompt marks it, ahead of the instructions: models follow
    // instructions better when a long input comes first and what to do with it comes after.
    let prompt = PROMPT.replacen("{{session}}", &payload.to_string(), 1);
    let answer: Answer = serde_json::from_value(ask(&prompt, &schema(), &artifacts.join("prepare"))?).map_err(|error| Error::rejected(format!("Invalid model response format: {error}")))?;

    let waiting_ids: BTreeSet<i64> = waiting.iter().map(|candidate| candidate.id).collect();
    let mut placed: Vec<Option<Placed>> = asked.iter().map(|_| None).collect();
    for entry in answer.marks {
        let Some(slot) = usize::try_from(entry.mark).ok().and_then(|index| placed.get_mut(index)) else {
            return Err(Error::rejected(format!("There is no mark at position {} of `marks`.", entry.mark)));
        };
        if slot.is_some() {
            return Err(Error::rejected(format!("The mark at position {} has more than one entry.", entry.mark)));
        }
        if entry.waiting != 0 && !waiting_ids.contains(&entry.waiting) {
            return Err(Error::rejected(format!("`waiting` names no candidate in `waiting`: {}", entry.waiting)));
        }
        *slot = Some(entry);
    }
    placed
        .into_iter()
        .enumerate()
        .map(|(index, entry)| {
            let entry = entry.ok_or_else(|| Error::rejected(format!("The mark at position {index} has no entry. Every mark needs one.")))?;
            Ok((related(entry.relation, notes, &session.project)?, entry.waiting))
        })
        .collect()
}

/// Prepare what one step of a session marked. Nothing is stored here. The model is called only
/// for marks that do not name a note of their own; without such a mark it is not called.
///
/// `notes` are the notes of `common/` and of the project, as the memory folder holds them now;
/// the model reads them from `memory` itself, and they are what every relation is checked against.
/// `waiting` are the candidates of the project that already wait for the user.
pub fn prepare(session: &Session, notes: &BTreeMap<String, String>, waiting: &[Waiting], memory: &Path, previous_failure: Option<&str>, artifacts: &Path, ask: Ask) -> Result<Prepared> {
    let marks = &session.marks.marks;
    let mut placed: Vec<Option<(Relation, i64)>> = marks.iter().map(|marked| marked.mark.note.as_ref().and_then(|target| named(target, notes, memory)).map(|relation| (relation, 0))).collect();
    let asked: Vec<&Marked> = marks.iter().zip(&placed).filter(|(_, placed)| placed.is_none()).map(|(marked, _)| marked).collect();
    if !asked.is_empty() {
        let mut answers = compare(session, &asked, notes, waiting, memory, previous_failure, artifacts, ask)?.into_iter();
        for slot in placed.iter_mut().filter(|slot| slot.is_none()) {
            *slot = answers.next();
        }
    }
    let mut prepared = Prepared::default();
    for (marked, (relation, waiting)) in marks.iter().zip(placed.into_iter().flatten()) {
        if waiting != 0 {
            continue;
        }
        let scope = if relation.kind == RelationKind::New { marked.mark.scope } else { folder_scope(&relation.note) };
        let left_out = relation.kind == RelationKind::Same;
        let found = Found { kind: marked.mark.kind, text: marked.mark.text.clone(), mark: marked.mark.id.clone(), scope, evidence: evidence(marked, &session.parsed.turns), relation };
        if left_out {
            prepared.left_out.push(found);
        } else {
            prepared.offered.push(found);
        }
    }
    Ok(prepared)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mark::{Mark, Resolved};
    use crate::transcript::{Parsed, Provider};
    use std::cell::RefCell;
    use std::path::PathBuf;

    fn turn(line: usize, role: &str, origin: Origin, text: &str) -> Turn {
        Turn { reference: format!("L{line}"), line, role: role.into(), origin, timestamp: "2026-11-17T01:00:00Z".into(), text: text.into() }
    }

    fn mark(id: &str, kind: Kind, text: &str, by: Speaker, said: &[&str]) -> Mark {
        Mark { id: id.into(), kind, text: text.into(), by, said: said.iter().map(|words| words.to_string()).collect(), scope: Scope::Project, replaces: None, note: None }
    }

    fn naming(change: Change, path: &str, quotes: &[&str], mark: Mark) -> Mark {
        Mark { note: Some(Target { change, path: path.into(), quotes: quotes.iter().map(|quote| quote.to_string()).collect() }), ..mark }
    }

    fn session(marks: Vec<(usize, Mark)>) -> Session {
        let turns = vec![
            turn(2, "user", Origin::User, "Finished jobs get no desktop alert. They distract me during long runs."),
            turn(3, "assistant", Origin::Assistant, "Shall I page through the rows instead of loading them at once?"),
            turn(4, "user", Origin::User, "알겠어"),
            turn(6, "user", Origin::Agent, "Worker report: paging fixed the timeout."),
            turn(9, "user", Origin::User, "Later words of the user."),
        ];
        Session {
            transcript: PathBuf::from("/tmp/session.jsonl"),
            provider: Provider::Claude,
            parsed: Parsed { turns, ..Parsed::default() },
            project: "demo".into(),
            end_line: 9,
            end_hash: "h".into(),
            marks: Resolved { marks: marks.into_iter().map(|(line, mark)| Marked { line, at: "2026-11-17T01:05:00Z".into(), mark }).collect(), withdrawn: Vec::new() },
        }
    }

    fn new() -> Value {
        json!({"type": "new", "note": "", "quotes": []})
    }

    fn entry(mark: i64, waiting: i64, relation: Value) -> Value {
        json!({"mark": mark, "waiting": waiting, "relation": relation})
    }

    /// Answers the model gives, in order, and the prompts it was sent.
    struct Model {
        answers: RefCell<Vec<Value>>,
        prompts: RefCell<Vec<String>>,
    }

    impl Model {
        fn new(answers: Vec<Value>) -> Self {
            Self { answers: RefCell::new(answers), prompts: RefCell::new(Vec::new()) }
        }

        fn run(&self, session: &Session, notes: &BTreeMap<String, String>, waiting: &[Waiting]) -> Result<Prepared> {
            let mut ask = |prompt: &str, _: &Value, _: &Path| {
                self.prompts.borrow_mut().push(prompt.to_string());
                Ok(self.answers.borrow_mut().remove(0))
            };
            prepare(session, notes, waiting, Path::new("D:/memory"), None, Path::new("/tmp/artifacts"), &mut ask)
        }
    }

    fn notes() -> BTreeMap<String, String> {
        BTreeMap::from([
            ("projects/demo/alerts.md".to_string(), "# Alerts\n\nEvery finished job raises a desktop alert.\n\n## Sources\n- 2026-10-02, agent: \"raise alerts\"\n".to_string()),
            ("common/style.md".to_string(), "# Style\n\nAnswer in short sentences.\n".to_string()),
        ])
    }

    const ALERT: &str = "Finished background jobs raise no desktop alert, because alerts distract the user during long runs.";
    const PAGING: &str = "The export pages through rows instead of loading them at once.";

    #[test]
    fn a_mark_becomes_a_candidate_with_whose_it_is_and_the_users_words_found_in_the_session() {
        let model = Model::new(vec![json!({"marks":[entry(0, 0, new()), entry(1, 0, new())]})]);
        let marks = vec![
            (5, mark("a1", Kind::Decision, ALERT, Speaker::User, &["Finished jobs get no desktop alert.", "Later words of the user.", "not said by anyone"])),
            (7, mark("b2", Kind::Decision, PAGING, Speaker::Agent, &["알겠어", "Worker report: paging fixed the timeout."])),
        ];
        let prepared = model.run(&session(marks), &BTreeMap::new(), &[]).unwrap();
        let (alert, paging) = (&prepared.offered[0], &prepared.offered[1]);
        assert_eq!((alert.text.as_str(), alert.mark.as_str(), alert.relation.kind), (ALERT, "a1", RelationKind::New));
        // Only what the user said before the mark is the user's: not later words, not words found nowhere.
        assert_eq!(alert.evidence.iter().map(|quote| (quote.reference.as_str(), quote.speaker, quote.quote.as_str())).collect::<Vec<_>>(), vec![("L2", Speaker::User, "Finished jobs get no desktop alert.")]);
        assert_eq!(alert.evidence[0].at, local_stamp("2026-11-17T01:00:00Z", &Local));
        // The agent's proposal is the agent's words; a short approval counts as the whole turn; another agent's report is not the user.
        assert_eq!(paging.evidence.iter().map(|quote| (quote.reference.as_str(), quote.speaker, quote.quote.as_str())).collect::<Vec<_>>(), vec![("L7", Speaker::Agent, PAGING), ("L4", Speaker::User, "알겠어")]);
        assert_eq!(paging.evidence[0].at, local_stamp("2026-11-17T01:05:00Z", &Local));
        assert_eq!(model.prompts.borrow().len(), 1);
    }

    #[test]
    fn a_step_without_marks_calls_no_model() {
        let model = Model::new(Vec::new());
        assert_eq!(model.run(&session(Vec::new()), &notes(), &[]).unwrap(), Prepared::default());
        assert!(model.prompts.borrow().is_empty());
    }

    #[test]
    fn the_model_is_told_the_marks_where_the_notes_are_and_what_already_waits_instead_of_being_given_the_notes_or_the_conversation() {
        let model = Model::new(vec![json!({"marks":[entry(0, 0, new())]})]);
        let waiting = [Waiting { id: 12, kind: Kind::Decision, text: "Keep the status panel on the first screen.".into() }];
        let common = Mark { scope: Scope::Common, ..mark("a1", Kind::Correction, "Answer in short sentences.", Speaker::User, &["They distract me"]) };
        model.run(&session(vec![(5, common)]), &notes(), &waiting).unwrap();
        let prompt = &model.prompts.borrow()[0];
        assert!(prompt.contains("\"memory_folder\":\"D:/memory\""));
        assert!(prompt.contains("\"note_folders\":[\"common/\",\"projects/demo/\"]"));
        assert!(prompt.contains("\"search_collections\":{\"common\":\"common/\",\"project-demo\":\"projects/demo/\"}"));
        assert!(prompt.contains("\"waiting\":[{\"id\":12,\"kind\":\"decision\",\"text\":\"Keep the status panel on the first screen.\"}]"));
        assert!(prompt.contains("\"marks\":[{\"kind\":\"correction\",\"text\":\"Answer in short sentences.\",\"scope\":\"common\"}]"));
        assert!(!prompt.contains("Every finished job raises a desktop alert."));
        assert!(!prompt.contains("They distract me"));
        assert!(prompt.find("\"marks\"").unwrap() < prompt.find("# Your task").unwrap());
    }

    #[test]
    fn what_a_note_already_says_is_left_out_what_already_waits_is_dropped_and_a_note_decides_the_folder() {
        let model = Model::new(vec![json!({"marks":[
            entry(2, 12, new()),
            entry(0, 0, json!({"type": "differs", "note": "projects/demo/alerts.md", "quotes": ["Every finished job raises a desktop alert."]})),
            entry(1, 0, json!({"type": "same", "note": "./common/style.md", "quotes": ["Answer in short sentences."]})),
        ]})]);
        let marks = vec![
            (5, Mark { scope: Scope::Common, ..mark("a1", Kind::Decision, ALERT, Speaker::User, &["Finished jobs get no desktop alert."]) }),
            (5, mark("b2", Kind::Correction, "Answer the user in short sentences.", Speaker::User, &["They distract me"])),
            (5, mark("c3", Kind::Decision, "Keep the status panel first.", Speaker::User, &["Finished jobs"])),
        ];
        let waiting = [Waiting { id: 12, kind: Kind::Decision, text: "Keep the status panel on the first screen.".into() }];
        let prepared = model.run(&session(marks), &notes(), &waiting).unwrap();
        assert_eq!(prepared.offered.len(), 1);
        let relation = &prepared.offered[0].relation;
        assert_eq!((relation.kind, relation.note.as_str(), relation.title.as_str()), (RelationKind::Differs, "projects/demo/alerts.md", "Alerts"));
        assert_eq!(relation.digest, body_digest(&notes()["projects/demo/alerts.md"]));
        assert_eq!(prepared.offered[0].scope, Scope::Project);
        assert_eq!(prepared.left_out.iter().map(|found| (found.mark.as_str(), found.scope, found.relation.note.as_str())).collect::<Vec<_>>(), vec![("b2", Scope::Common, "common/style.md")]);
    }

    #[test]
    fn an_answer_that_does_not_account_for_every_mark_or_names_what_is_not_there_is_rejected() {
        let marks = || session(vec![(5, mark("a1", Kind::Decision, ALERT, Speaker::User, &["Finished jobs"])), (7, mark("b2", Kind::Lesson, PAGING, Speaker::Agent, &[]))]);
        let rejected = |entries: Vec<Value>| {
            let model = Model::new(vec![json!({"marks":entries})]);
            model.run(&marks(), &notes(), &[Waiting { id: 12, kind: Kind::Decision, text: "x".into() }]).unwrap_err().message
        };
        assert!(rejected(vec![entry(0, 0, new())]).contains("position 1 has no entry"));
        assert!(rejected(vec![entry(0, 0, new()), entry(0, 0, new()), entry(1, 0, new())]).contains("more than one entry"));
        assert!(rejected(vec![entry(0, 0, new()), entry(2, 0, new())]).contains("no mark at position 2"));
        assert!(rejected(vec![entry(0, 0, new()), entry(1, 7, new())]).contains("names no candidate"));
    }

    #[test]
    fn a_relation_must_name_a_note_of_this_project_or_common_and_quote_it_word_for_word() {
        let mut notes = notes();
        notes.insert("projects/other/alerts.md".to_string(), "# Other\n\nEvery finished job raises a desktop alert.\n".to_string());
        let rejected = |relation: Value| {
            let model = Model::new(vec![json!({"marks":[entry(0, 0, relation)]})]);
            let scoped: BTreeMap<String, String> = notes.iter().filter(|(path, _)| !path.starts_with("projects/other/")).map(|(path, text)| (path.clone(), text.clone())).collect();
            model.run(&session(vec![(5, mark("a1", Kind::Decision, ALERT, Speaker::User, &["Finished jobs"]))]), &scoped, &[]).unwrap_err().message
        };
        assert!(rejected(json!({"type": "differs", "note": "projects/demo/alerts.md", "quotes": ["Jobs raise alerts."]})).contains("word for word"));
        assert!(rejected(json!({"type": "differs", "note": "projects/demo/alerts.md", "quotes": []})).contains("quotes no passage"));
        assert!(rejected(json!({"type": "obsolete", "note": "projects/other/alerts.md", "quotes": ["Every finished job raises a desktop alert."]})).contains("not one of the notes"));
        assert!(rejected(json!({"type": "same", "note": "projects/demo/gone.md", "quotes": ["x"]})).contains("not one of the notes"));
    }

    #[test]
    fn a_note_the_session_names_is_taken_without_the_model_once_the_program_finds_the_words_in_it() {
        let changes = naming(Change::Updates, "project-demo/alerts.md", &["Every finished job raises a desktop alert."], mark("a1", Kind::Decision, ALERT, Speaker::User, &["Finished jobs get no desktop alert."]));
        let ends = naming(Change::Retires, "D:\\memory\\common\\style.md", &["Answer in short sentences."], Mark { scope: Scope::Project, ..mark("b2", Kind::Correction, "Answers may be long.", Speaker::User, &["They distract me"]) });
        // `run` would panic if the model were asked: there is no answer for it.
        let model = Model::new(Vec::new());
        let prepared = model.run(&session(vec![(5, changes), (5, ends)]), &notes(), &[]).unwrap();
        assert!(model.prompts.borrow().is_empty());
        let shown: Vec<(RelationKind, &str, &str, Scope)> = prepared.offered.iter().map(|found| (found.relation.kind, found.relation.note.as_str(), found.relation.title.as_str(), found.scope)).collect();
        assert_eq!(shown, vec![(RelationKind::Differs, "projects/demo/alerts.md", "Alerts", Scope::Project), (RelationKind::Obsolete, "common/style.md", "Style", Scope::Common)]);
        assert_eq!(prepared.offered[0].relation.quotes, vec!["Every finished job raises a desktop alert.".to_string()]);
        assert_eq!(prepared.offered[0].relation.digest, body_digest(&notes()["projects/demo/alerts.md"]));
        assert_eq!(note_path("qmd://project-demo/alerts.md", Path::new("D:/memory")), "projects/demo/alerts.md");
        assert_eq!(note_path("d:/MEMORY/projects/demo/alerts.md", Path::new("D:/memory")), "projects/demo/alerts.md");
    }

    #[test]
    fn a_named_note_that_is_not_there_or_lacks_the_words_leaves_the_mark_to_the_model() {
        let marks = vec![
            (5, naming(Change::Updates, "projects/demo/alerts.md", &["Jobs raise alerts."], mark("a1", Kind::Decision, ALERT, Speaker::User, &["Finished jobs"]))),
            (5, naming(Change::Updates, "projects/other/alerts.md", &["Every finished job raises a desktop alert."], mark("b2", Kind::Decision, PAGING, Speaker::User, &["Finished jobs"]))),
            (5, naming(Change::Retires, "projects/demo/style.md", &["x"], mark("c3", Kind::Lesson, "Pages time out.", Speaker::Agent, &[]))),
            (5, naming(Change::Updates, "projects/demo/alerts.md", &["Every finished job raises a desktop alert."], mark("d4", Kind::Decision, "No alerts at all.", Speaker::User, &["Finished jobs"]))),
        ];
        let model = Model::new(vec![json!({"marks": [entry(0, 0, new()), entry(1, 0, new()), entry(2, 0, json!({"type": "same", "note": "common/style.md", "quotes": ["Answer in short sentences."]}))]})]);
        let prepared = model.run(&session(marks), &notes(), &[]).unwrap();
        // The three the program could not take are asked about, in their order; the fourth is not.
        let prompt = model.prompts.borrow()[0].clone();
        assert!(prompt.contains(&format!("\"marks\":[{{\"kind\":\"decision\",\"text\":\"{ALERT}\",\"scope\":\"project\"}},{{\"kind\":\"decision\",\"text\":\"{PAGING}\",\"scope\":\"project\"}},{{\"kind\":\"lesson\",\"text\":\"Pages time out.\",\"scope\":\"project\"}}]")), "{prompt}");
        assert_eq!(prepared.offered.iter().map(|found| (found.mark.as_str(), found.relation.kind)).collect::<Vec<_>>(), vec![("a1", RelationKind::New), ("b2", RelationKind::New), ("d4", RelationKind::Differs)]);
        assert_eq!(prepared.left_out.iter().map(|found| found.mark.as_str()).collect::<Vec<_>>(), vec!["c3"]);
    }

    #[test]
    fn the_prompt_is_written_in_english_and_keeps_the_data_ahead_of_the_instructions() {
        let korean = |text: &str| text.chars().any(|c| ('\u{AC00}'..='\u{D7A3}').contains(&c));
        assert!(!korean(PROMPT));
        assert_eq!(PROMPT.matches("{{session}}").count(), 1);
        assert!(PROMPT.find("{{session}}").unwrap() < PROMPT.find("# Your task").unwrap());
        assert!(!korean(&schema().to_string()));
    }
}
