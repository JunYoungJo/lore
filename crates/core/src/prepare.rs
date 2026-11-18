//! Preparing candidates: what one step of a session offers, and how each stands to the notes.
//!
//! One model call. It is given the new turns and reads the notes of the memory folder itself; it
//! lists candidates with the turns they rest on and says how each stands to the notes. The program
//! checks every quotation of a turn and every passage it quotes from a note.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::candidate::{Found, Kind, Quote, Relation, RelationKind, Scope, Source, Speaker};
use crate::error::{Error, Result};
use crate::notes::title_of;
use crate::proposal::body_digest;
use crate::qmd::search_collections;
use crate::session::{in_local_time, Session};
use crate::transcript::{Origin, Turn};

pub const EXTRACT_PROMPT: &str = include_str!("extract-prompt.md");
/// A candidate is a sentence or three. A longer one is a report, which the user cannot check at a glance.
pub const TEXT_LIMIT: usize = 600;

/// Send a prompt and a JSON schema to the recording model; get its JSON answer. The model may
/// read the memory folder that the prompt names but change nothing.
pub type Ask<'a> = &'a mut dyn FnMut(&str, &Value, &Path) -> Result<Value>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Prepared {
    pub language: String,
    /// What the user is asked about.
    pub offered: Vec<Found>,
    /// What a note already says.
    pub left_out: Vec<Found>,
}

fn evidence_schema() -> Value {
    json!({"type": "array", "description": "The turns this candidate rests on.", "items": {
        "type": "object",
        "properties": {
            "ref": {"type": "string", "description": "The ref of a turn in the data, such as \"L123\"."},
            "quote": {"type": "string", "description": "Text copied exactly from that turn."}
        },
        "required": ["ref", "quote"],
        "additionalProperties": false
    }})
}

fn relation_schema() -> Value {
    json!({"type": "object", "description": "How the candidate stands to the notes in the memory folder.", "properties": {
        "type": {"type": "string", "enum": ["new", "same", "differs", "obsolete"], "description": "new: no note is about it. same: a note already states it. differs: a note states something it changes. obsolete: it ends what a note is about."},
        "note": {"type": "string", "description": "same, differs, obsolete: the note's path relative to the memory folder, such as \"projects/demo/alerts.md\". new: empty."},
        "quotes": {"type": "array", "description": "same, differs, obsolete: one to three passages copied exactly from that note. new: empty.", "items": {"type": "string"}}
    }, "required": ["type", "note", "quotes"], "additionalProperties": false})
}

// Every field is required so both CLIs emit them in this order; `language` comes first so the model
// settles the language of the candidates before it writes any of them.
pub fn extract_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "language": {"type": "string", "description": "The language the human user writes in during this session, for example \"Korean\" or \"English\". Every candidate is written in this language."},
            "candidates": {"type": "array", "description": "What may be worth remembering. Empty when the turns hold nothing of that kind.", "items": {
                "type": "object",
                "properties": {
                    "kind": {"type": "string", "enum": ["decision", "correction", "lesson"], "description": "A decision of the user, a correction of how the assistant works, or a lesson the session established."},
                    "text": {"type": "string", "description": "One to three sentences that can be understood without this session."},
                    "scope": {"type": "string", "enum": ["project", "common"], "description": "project: about this project. common: about how the user wants agents to work in any project."},
                    "mark": {"type": "integer", "description": "The position in `marks` of the mark this candidate confirms, counting from 0, or -1."},
                    "evidence": evidence_schema(),
                    "relation": relation_schema()
                },
                "required": ["kind", "text", "scope", "mark", "evidence", "relation"],
                "additionalProperties": false
            }}
        },
        "required": ["language", "candidates"],
        "additionalProperties": false
    })
}

#[derive(Deserialize)]
struct Cited {
    #[serde(rename = "ref")]
    reference: String,
    quote: String,
}

#[derive(Deserialize)]
struct Related {
    #[serde(rename = "type")]
    kind: RelationKind,
    note: String,
    quotes: Vec<String>,
}

#[derive(Deserialize)]
struct Drafted {
    kind: Kind,
    text: String,
    scope: Scope,
    mark: i64,
    evidence: Vec<Cited>,
    relation: Related,
}

#[derive(Deserialize)]
struct Extracted {
    language: String,
    candidates: Vec<Drafted>,
}

struct Draft {
    kind: Kind,
    text: String,
    source: Source,
    scope: Scope,
    evidence: Vec<Quote>,
    relation: Relation,
}

fn answer<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|error| Error::rejected(format!("Invalid model response format: {error}")))
}

/// A quotation shorter than this must be the whole turn. A few characters lifted out of a long
/// turn are in it, but they show nothing of what was said.
const QUOTE_MIN: usize = 10;

/// The words a quotation takes from a turn, as they are kept. Line ends are compared as written
/// on any system, since a turn pasted on Windows carries its own.
fn quoted(turn: &Turn, quote: &str) -> Result<String> {
    let said = turn.text.replace("\r\n", "\n");
    let words = quote.replace("\r\n", "\n").trim().to_string();
    if words.is_empty() || !said.contains(&words) {
        return Err(Error::rejected(format!("The evidence location or quotation does not match the provided source ({}).", turn.reference)));
    }
    if words.chars().count() < QUOTE_MIN && said.trim() != words {
        return Err(Error::rejected(format!("The quotation from {} is too short to show what was said. Quote at least {QUOTE_MIN} characters, or the whole turn when the turn is shorter.", turn.reference)));
    }
    Ok(words)
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

/// Check what the model listed against the turns it was given and the notes in the folder.
fn check_drafts(listed: Vec<Drafted>, session: &Session, notes: &BTreeMap<String, String>) -> Result<Vec<Draft>> {
    let new_turns = in_local_time(&session.batch.new_turns);
    let context_turns = in_local_time(&session.batch.context_turns);
    let fresh: BTreeSet<&str> = new_turns.iter().map(|turn| turn.reference.as_str()).collect();
    let shown: HashMap<&str, &Turn> = context_turns.iter().chain(new_turns.iter()).map(|turn| (turn.reference.as_str(), turn)).collect();
    let mut drafts: Vec<Draft> = Vec::new();
    for item in listed {
        let text = item.text.trim().to_string();
        if text.is_empty() {
            return Err(Error::rejected("A candidate has no text."));
        }
        if text.chars().count() > TEXT_LIMIT {
            return Err(Error::rejected(format!("A candidate is one to three sentences, at most {TEXT_LIMIT} characters: \"{}…\"", text.chars().take(60).collect::<String>())));
        }
        if item.evidence.is_empty() {
            return Err(Error::rejected("Every candidate needs evidence."));
        }
        let mut evidence = Vec::new();
        for cited in &item.evidence {
            let Some(turn) = shown.get(cited.reference.as_str()) else {
                return Err(Error::rejected(format!("The evidence location or quotation does not match the provided source ({}).", cited.reference)));
            };
            let speaker = if turn.origin == Origin::User { Speaker::User } else { Speaker::Agent };
            evidence.push(Quote { reference: turn.reference.clone(), quote: quoted(turn, &cited.quote)?, speaker, at: turn.timestamp.clone() });
        }
        if !evidence.iter().any(|quote| fresh.contains(quote.reference.as_str())) {
            return Err(Error::rejected("A candidate must quote at least one of new_turns; what only context_turns hold was read before."));
        }
        if item.kind != Kind::Lesson && !evidence.iter().any(|quote| quote.speaker == Speaker::User) {
            return Err(Error::rejected("A decision or a correction must quote a turn of the user."));
        }
        let relation = related(item.relation, notes, &session.project)?;
        if drafts.iter().any(|draft| draft.text == text) {
            continue;
        }
        let marked = usize::try_from(item.mark).is_ok_and(|index| index < session.marks.len());
        drafts.push(Draft { kind: item.kind, text, source: if marked { Source::Marked } else { Source::Found }, scope: item.scope, evidence, relation });
    }
    Ok(drafts)
}

/// List what one step of a session offers. Nothing is stored here.
///
/// `notes` are the notes of `common/` and of the project, as the memory folder holds them now;
/// the model reads them from `memory` itself, and they are what its relations are checked against.
/// `waiting` are the candidates of the project that already wait for the user, so that the model
/// lists only what they do not already say.
pub fn prepare(session: &Session, notes: &BTreeMap<String, String>, waiting: &[(Kind, String)], memory: &Path, previous_failure: Option<&str>, artifacts: &Path, ask: Ask) -> Result<Prepared> {
    let waiting: Vec<Value> = waiting.iter().map(|(kind, text)| json!({"kind": kind, "text": text})).collect();
    let mut payload = json!({
        "project": session.project,
        "provider": session.provider.as_str(),
        "memory_folder": memory.to_string_lossy().replace('\\', "/"),
        "note_folders": ["common/", format!("projects/{}/", session.project)],
        "search_collections": search_collections(&session.project),
        "waiting": waiting,
        "new_turns": in_local_time(&session.batch.new_turns),
        "context_turns": in_local_time(&session.batch.context_turns),
        "omitted_context_turns": session.batch.omitted_context_turns,
        "marks": session.marks,
    });
    if let Some(reason) = previous_failure {
        payload["previous_failure"] = json!(reason);
    }
    // The session goes where the prompt marks it, ahead of the instructions: models follow
    // instructions better when a long input comes first and what to do with it comes after.
    let prompt = EXTRACT_PROMPT.replacen("{{session}}", &payload.to_string(), 1);
    let extracted: Extracted = answer(ask(&prompt, &extract_schema(), &artifacts.join("extract"))?)?;
    let mut prepared = Prepared { language: extracted.language.trim().to_string(), ..Prepared::default() };
    for draft in check_drafts(extracted.candidates, session, notes)? {
        let scope = if draft.relation.kind == RelationKind::New { draft.scope } else { folder_scope(&draft.relation.note) };
        let left_out = draft.relation.kind == RelationKind::Same;
        let found = Found { kind: draft.kind, text: draft.text, source: draft.source, scope, evidence: draft.evidence, relation: draft.relation };
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
    use crate::batch::Batch;
    use crate::mark::{Mark, Placed};
    use crate::transcript::{Parsed, Provider};
    use std::cell::RefCell;
    use std::path::PathBuf;

    fn turn(line: usize, role: &str, origin: Origin, text: &str) -> Turn {
        Turn { reference: format!("L{line}"), line, role: role.into(), origin, timestamp: "2026-11-17T01:00:00Z".into(), text: text.into() }
    }

    fn session(marks: Vec<Placed>) -> Session {
        let context = vec![turn(2, "user", Origin::User, "Earlier: keep the status panel."), turn(3, "assistant", Origin::Assistant, "Kept.")];
        let new = vec![
            turn(5, "user", Origin::User, "Finished jobs get no desktop alert. They distract me during long runs."),
            turn(6, "assistant", Origin::Assistant, "The export timed out because the job loaded every row at once."),
            turn(8, "user", Origin::Agent, "Worker report: paging fixed the timeout."),
        ];
        Session {
            transcript: PathBuf::from("/tmp/session.jsonl"),
            provider: Provider::Claude,
            parsed: Parsed::default(),
            project: "demo".into(),
            batch: Batch { new_turns: new, context_turns: context, omitted_context_turns: 0, end_line: 8 },
            end_hash: "h".into(),
            marks,
        }
    }

    fn new() -> Value {
        json!({"type": "new", "note": "", "quotes": []})
    }

    fn candidate(kind: &str, text: &str, mark: i64, evidence: &[(&str, &str)], relation: Value) -> Value {
        let evidence: Vec<Value> = evidence.iter().map(|(reference, quote)| json!({"ref": reference, "quote": quote})).collect();
        json!({"kind": kind, "text": text, "scope": "project", "mark": mark, "evidence": evidence, "relation": relation})
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

        fn run(&self, session: &Session, notes: &BTreeMap<String, String>, waiting: &[(Kind, String)]) -> Result<Prepared> {
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

    #[test]
    fn candidates_carry_checked_quotations_and_who_said_them() {
        let model = Model::new(vec![json!({"language": "English", "candidates": [
            candidate("decision", ALERT, 0, &[("L5", "Finished jobs get no desktop alert.")], new()),
            candidate("lesson", "The export must page through rows; loading them all times out.", -1, &[("L6", "loaded every row at once"), ("L8", "paging fixed the timeout")], new()),
        ]})]);
        let marks = vec![Placed { line: 7, mark: Mark { kind: Kind::Decision, text: "No alerts.".into() } }];
        let prepared = model.run(&session(marks), &BTreeMap::new(), &[]).unwrap();
        assert_eq!(prepared.language, "English");
        assert_eq!(prepared.offered.len(), 2);
        let decision = &prepared.offered[0];
        assert_eq!((decision.source, decision.relation.kind), (Source::Marked, RelationKind::New));
        assert_eq!(decision.evidence[0].speaker, Speaker::User);
        // The quotation keeps the time of its turn as the clock of this PC showed it.
        assert_eq!(decision.evidence[0].at, crate::session::local_stamp("2026-11-17T01:00:00Z", &chrono::Local));
        let lesson = &prepared.offered[1];
        assert_eq!((lesson.source, lesson.evidence.iter().map(|quote| quote.speaker).collect::<Vec<_>>()), (Source::Found, vec![Speaker::Agent, Speaker::Agent]));
        assert_eq!(model.prompts.borrow().len(), 1);
        let prompt = &model.prompts.borrow()[0];
        assert!(prompt.find("\"new_turns\"").unwrap() < prompt.find("# Your task").unwrap());
        assert!(prompt.contains("\"marks\":[{\"line\":7,\"kind\":\"decision\",\"text\":\"No alerts.\"}]"));
    }

    #[test]
    fn the_model_is_told_where_the_notes_are_and_what_already_waits_instead_of_being_given_the_notes() {
        let model = Model::new(vec![json!({"language": "English", "candidates": []})]);
        let waiting = [(Kind::Decision, "Keep the status panel on the first screen.".to_string())];
        model.run(&session(Vec::new()), &notes(), &waiting).unwrap();
        let prompt = &model.prompts.borrow()[0];
        assert!(prompt.contains("\"memory_folder\":\"D:/memory\""));
        assert!(prompt.contains("\"note_folders\":[\"common/\",\"projects/demo/\"]"));
        assert!(prompt.contains("\"search_collections\":{\"common\":\"common/\",\"project-demo\":\"projects/demo/\"}"));
        assert!(prompt.contains("\"waiting\":[{\"kind\":\"decision\",\"text\":\"Keep the status panel on the first screen.\"}]"));
        assert!(!prompt.contains("Every finished job raises a desktop alert."));
    }

    #[test]
    fn an_answer_whose_evidence_does_not_hold_is_rejected() {
        let rejected = |item: Value| {
            let model = Model::new(vec![json!({"language": "English", "candidates": [item]})]);
            model.run(&session(Vec::new()), &BTreeMap::new(), &[]).unwrap_err().message
        };
        assert!(rejected(candidate("decision", ALERT, -1, &[("L5", "no alerts at all")], new())).contains("does not match"));
        assert!(rejected(candidate("decision", ALERT, -1, &[("L99", "Finished jobs")], new())).contains("does not match"));
        assert!(rejected(candidate("decision", ALERT, -1, &[], new())).contains("needs evidence"));
        // Only the assistant said it: that is not the user's decision.
        assert!(rejected(candidate("decision", "The export pages through rows.", -1, &[("L6", "loaded every row at once")], new())).contains("must quote a turn of the user"));
        // An agent's message in the user's input is not the user either.
        assert!(rejected(candidate("correction", "Report paging fixes.", -1, &[("L8", "paging fixed the timeout")], new())).contains("must quote a turn of the user"));
        assert!(rejected(candidate("decision", "Keep the status panel.", -1, &[("L2", "keep the status panel")], new())).contains("new_turns"));
        assert!(rejected(candidate("lesson", &"word ".repeat(200), -1, &[("L6", "timed out")], new())).contains("at most"));
    }

    #[test]
    fn each_candidate_says_how_it_stands_to_the_notes_and_what_a_note_already_says_is_left_out() {
        let model = Model::new(vec![json!({"language": "English", "candidates": [
            candidate("decision", ALERT, -1, &[("L5", "Finished jobs get no desktop alert.")], json!({"type": "differs", "note": "projects/demo/alerts.md", "quotes": ["Every finished job raises a desktop alert."]})),
            candidate("correction", "Answer the user in short sentences.", -1, &[("L5", "They distract me")], json!({"type": "same", "note": "./common/style.md", "quotes": ["Answer in short sentences."]})),
        ]})]);
        let prepared = model.run(&session(Vec::new()), &notes(), &[]).unwrap();
        assert_eq!(prepared.offered.len(), 1);
        let relation = &prepared.offered[0].relation;
        assert_eq!((relation.kind, relation.note.as_str(), relation.title.as_str()), (RelationKind::Differs, "projects/demo/alerts.md", "Alerts"));
        assert_eq!(relation.digest, body_digest(&notes()["projects/demo/alerts.md"]));
        assert_eq!(prepared.left_out.iter().map(|found| (found.text.as_str(), found.scope, found.relation.note.as_str())).collect::<Vec<_>>(), vec![("Answer the user in short sentences.", Scope::Common, "common/style.md")]);
    }

    #[test]
    fn a_relation_must_name_a_note_of_this_project_or_common_and_quote_it_word_for_word() {
        let mut notes = notes();
        notes.insert("projects/other/alerts.md".to_string(), "# Other\n\nEvery finished job raises a desktop alert.\n".to_string());
        let rejected = |relation: Value| {
            let model = Model::new(vec![json!({"language": "English", "candidates": [candidate("decision", ALERT, -1, &[("L5", "Finished jobs get no desktop alert.")], relation)]})]);
            let scoped: BTreeMap<String, String> = notes.iter().filter(|(path, _)| !path.starts_with("projects/other/")).map(|(path, text)| (path.clone(), text.clone())).collect();
            model.run(&session(Vec::new()), &scoped, &[]).unwrap_err().message
        };
        assert!(rejected(json!({"type": "differs", "note": "projects/demo/alerts.md", "quotes": ["Jobs raise alerts."]})).contains("word for word"));
        assert!(rejected(json!({"type": "differs", "note": "projects/demo/alerts.md", "quotes": []})).contains("quotes no passage"));
        assert!(rejected(json!({"type": "obsolete", "note": "projects/other/alerts.md", "quotes": ["Every finished job raises a desktop alert."]})).contains("not one of the notes"));
        assert!(rejected(json!({"type": "same", "note": "projects/demo/gone.md", "quotes": ["x"]})).contains("not one of the notes"));
    }

    #[test]
    fn the_prompt_is_written_in_english_and_keeps_the_data_ahead_of_the_instructions() {
        let korean = |text: &str| text.chars().any(|c| ('\u{AC00}'..='\u{D7A3}').contains(&c));
        assert!(!korean(EXTRACT_PROMPT));
        assert_eq!(EXTRACT_PROMPT.matches("{{session}}").count(), 1);
        assert!(EXTRACT_PROMPT.find("{{session}}").unwrap() < EXTRACT_PROMPT.find("# Your task").unwrap());
        assert!(!korean(&extract_schema().to_string()));
    }
}
