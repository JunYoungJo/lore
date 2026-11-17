//! Preparing candidates: what one step of a session offers, and how each stands to the notes.
//!
//! Two model calls. The first lists candidates with the turns they rest on; the program checks
//! every quotation. The second is shown the notes a search finds for each candidate and says
//! whether a note already holds it, differs from it or is ended by it; the program checks that
//! the passages it quotes are in the note.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::candidate::{Found, Kind, Quote, Relation, RelationKind, Scope, Source, Speaker};
use crate::error::{Error, Result};
use crate::notes::title_of;
use crate::proposal::{body_digest, without_sources};
use crate::session::{in_local_time, Session};
use crate::transcript::{Origin, Turn};

pub const EXTRACT_PROMPT: &str = include_str!("extract-prompt.md");
pub const RELATE_PROMPT: &str = include_str!("relate-prompt.md");
/// A candidate is a sentence or three. A longer one is a report, which the user cannot check at a glance.
pub const TEXT_LIMIT: usize = 600;
/// How many notes the search contributes for one candidate.
pub const NOTES_PER_CANDIDATE: usize = 4;
/// How much note text one model request carries.
pub const NOTES_CHARS: usize = 120_000;

/// Send a prompt and a JSON schema to the recording model; get its JSON answer.
pub type Ask<'a> = &'a mut dyn FnMut(&str, &Value, &Path) -> Result<Value>;
/// The paths of the notes most likely to be about a text, best first.
pub type Search<'a> = &'a dyn Fn(&str) -> Vec<String>;

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
                    "evidence": evidence_schema()
                },
                "required": ["kind", "text", "scope", "mark", "evidence"],
                "additionalProperties": false
            }}
        },
        "required": ["language", "candidates"],
        "additionalProperties": false
    })
}

pub fn relate_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "relations": {"type": "array", "description": "Exactly one entry for every candidate.", "items": {
                "type": "object",
                "properties": {
                    "index": {"type": "integer", "description": "The index of the candidate."},
                    "type": {"type": "string", "enum": ["new", "same", "differs", "obsolete"], "description": "How the candidate stands to the notes."},
                    "note": {"type": "string", "description": "same, differs, obsolete: the path of the note. new: empty."},
                    "quotes": {"type": "array", "description": "same, differs, obsolete: one to three passages copied exactly from that note. new: empty.", "items": {"type": "string"}}
                },
                "required": ["index", "type", "note", "quotes"],
                "additionalProperties": false
            }}
        },
        "required": ["relations"],
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
struct Drafted {
    kind: Kind,
    text: String,
    scope: Scope,
    mark: i64,
    evidence: Vec<Cited>,
}

#[derive(Deserialize)]
struct Extracted {
    language: String,
    candidates: Vec<Drafted>,
}

#[derive(Deserialize)]
struct Related {
    index: i64,
    #[serde(rename = "type")]
    kind: RelationKind,
    note: String,
    quotes: Vec<String>,
}

#[derive(Deserialize)]
struct Relations {
    relations: Vec<Related>,
}

struct Draft {
    kind: Kind,
    text: String,
    source: Source,
    scope: Scope,
    evidence: Vec<Quote>,
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

/// Check what the model listed against the turns it was shown.
fn check_drafts(listed: Vec<Drafted>, session: &Session) -> Result<Vec<Draft>> {
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
        if drafts.iter().any(|draft| draft.text == text) {
            continue;
        }
        let marked = usize::try_from(item.mark).is_ok_and(|index| index < session.marks.len());
        drafts.push(Draft { kind: item.kind, text, source: if marked { Source::Marked } else { Source::Found }, scope: item.scope, evidence });
    }
    Ok(drafts)
}

/// The notes a model is shown for a set of texts: the required ones, then what the search finds
/// for each text in turn, as long as they fit. Each is shown without its source list.
pub fn shown_notes(texts: &[&str], required: &[String], notes: &BTreeMap<String, String>, search: Search) -> BTreeMap<String, String> {
    let mut found: Vec<Vec<String>> = texts.iter().map(|text| search(text).into_iter().filter(|path| notes.contains_key(path)).take(NOTES_PER_CANDIDATE).collect()).collect();
    let mut order: Vec<String> = required.iter().filter(|path| notes.contains_key(*path)).cloned().collect();
    for rank in 0..NOTES_PER_CANDIDATE {
        for paths in &mut found {
            if let Some(path) = paths.get(rank) {
                order.push(path.clone());
            }
        }
    }
    let mut shown = BTreeMap::new();
    let mut used = 0;
    for path in order {
        if shown.contains_key(&path) {
            continue;
        }
        let text = without_sources(&notes[&path]);
        let size = text.chars().count();
        if used + size > NOTES_CHARS && !required.contains(&path) {
            continue;
        }
        used += size;
        shown.insert(path, text);
    }
    shown
}

fn folder_scope(note: &str) -> Scope {
    if note.starts_with("common/") {
        Scope::Common
    } else {
        Scope::Project
    }
}

/// Ask how each draft stands to the notes shown, and check the passages the answer quotes.
fn relate(drafts: &[Draft], shown: &BTreeMap<String, String>, notes: &BTreeMap<String, String>, project: &str, previous_failure: Option<&str>, artifacts: &Path, ask: Ask) -> Result<Vec<Relation>> {
    let listed: Vec<Value> = drafts.iter().enumerate().map(|(index, draft)| json!({"index": index, "kind": draft.kind, "text": draft.text})).collect();
    let mut payload = json!({"project": project, "candidates": listed, "notes": shown});
    if let Some(reason) = previous_failure {
        payload["previous_failure"] = json!(reason);
    }
    let prompt = RELATE_PROMPT.replacen("{{data}}", &payload.to_string(), 1);
    let related: Relations = answer(ask(&prompt, &relate_schema(), &artifacts.join("relate"))?)?;
    let mut relations: Vec<Option<Relation>> = vec![None; drafts.len()];
    for item in related.relations {
        let slot = usize::try_from(item.index).ok().and_then(|index| relations.get_mut(index)).ok_or_else(|| Error::rejected(format!("There is no candidate {}.", item.index)))?;
        if slot.is_some() {
            return Err(Error::rejected(format!("Candidate {} has more than one relation.", item.index)));
        }
        if item.kind == RelationKind::New {
            *slot = Some(Relation::new());
            continue;
        }
        let Some(text) = shown.get(&item.note) else { return Err(Error::rejected(format!("The relation of candidate {} names a note that was not provided: {}", item.index, item.note))) };
        let quotes: Vec<String> = item.quotes.iter().map(|quote| quote.replace("\r\n", "\n").trim().to_string()).filter(|quote| !quote.is_empty()).collect();
        if quotes.is_empty() {
            return Err(Error::rejected(format!("The relation of candidate {} quotes no passage of {}.", item.index, item.note)));
        }
        if let Some(missing) = quotes.iter().find(|quote| !text.contains(quote.as_str())) {
            return Err(Error::rejected(format!("A quoted passage is not in {} word for word: \"{}\"", item.note, missing.chars().take(80).collect::<String>())));
        }
        let note = &notes[&item.note];
        *slot = Some(Relation::to(item.kind, &item.note, &title_of(note), quotes, body_digest(note)));
    }
    relations.into_iter().enumerate().map(|(index, relation)| relation.ok_or_else(|| Error::rejected(format!("Candidate {index} has no relation.")))).collect()
}

/// List what one step of a session offers. Nothing is stored here.
pub fn prepare(session: &Session, notes: &BTreeMap<String, String>, previous_failure: Option<&str>, artifacts: &Path, ask: Ask, search: Search) -> Result<Prepared> {
    let mut payload = json!({
        "project": session.project,
        "provider": session.provider.as_str(),
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
    let drafts = check_drafts(extracted.candidates, session)?;
    let mut prepared = Prepared { language: extracted.language.trim().to_string(), ..Prepared::default() };
    if drafts.is_empty() {
        return Ok(prepared);
    }
    let texts: Vec<&str> = drafts.iter().map(|draft| draft.text.as_str()).collect();
    let shown = shown_notes(&texts, &[], notes, search);
    // With no note to compare with, everything is new and the model is not asked.
    let relations = if shown.is_empty() { vec![Relation::new(); drafts.len()] } else { relate(&drafts, &shown, notes, &session.project, previous_failure, artifacts, ask)? };
    for (draft, relation) in drafts.into_iter().zip(relations) {
        let scope = if relation.kind == RelationKind::New { draft.scope } else { folder_scope(&relation.note) };
        let left_out = relation.kind == RelationKind::Same;
        let found = Found { kind: draft.kind, text: draft.text, source: draft.source, scope, evidence: draft.evidence, relation };
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

    fn candidate(kind: &str, text: &str, mark: i64, evidence: &[(&str, &str)]) -> Value {
        let evidence: Vec<Value> = evidence.iter().map(|(reference, quote)| json!({"ref": reference, "quote": quote})).collect();
        json!({"kind": kind, "text": text, "scope": "project", "mark": mark, "evidence": evidence})
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

        fn run(&self, session: &Session, notes: &BTreeMap<String, String>, search: Search) -> Result<Prepared> {
            let mut ask = |prompt: &str, _: &Value, _: &Path| {
                self.prompts.borrow_mut().push(prompt.to_string());
                Ok(self.answers.borrow_mut().remove(0))
            };
            prepare(session, notes, None, Path::new("/tmp/artifacts"), &mut ask, search)
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
            candidate("decision", ALERT, 0, &[("L5", "Finished jobs get no desktop alert.")]),
            candidate("lesson", "The export must page through rows; loading them all times out.", -1, &[("L6", "loaded every row at once"), ("L8", "paging fixed the timeout")]),
        ]})]);
        let marks = vec![Placed { line: 7, mark: Mark { kind: Kind::Decision, text: "No alerts.".into() } }];
        let prepared = model.run(&session(marks), &BTreeMap::new(), &|_| Vec::new()).unwrap();
        assert_eq!(prepared.language, "English");
        assert_eq!(prepared.offered.len(), 2);
        let decision = &prepared.offered[0];
        assert_eq!((decision.source, decision.relation.kind), (Source::Marked, RelationKind::New));
        assert_eq!(decision.evidence[0].speaker, Speaker::User);
        // The quotation keeps the time of its turn as the clock of this PC showed it.
        assert_eq!(decision.evidence[0].at, crate::session::local_stamp("2026-11-17T01:00:00Z", &chrono::Local));
        let lesson = &prepared.offered[1];
        assert_eq!((lesson.source, lesson.evidence.iter().map(|quote| quote.speaker).collect::<Vec<_>>()), (Source::Found, vec![Speaker::Agent, Speaker::Agent]));
        // With no notes at all the model is asked once.
        assert_eq!(model.prompts.borrow().len(), 1);
        let prompt = &model.prompts.borrow()[0];
        assert!(prompt.find("\"new_turns\"").unwrap() < prompt.find("# Your task").unwrap());
        assert!(prompt.contains("\"marks\":[{\"line\":7,\"kind\":\"decision\",\"text\":\"No alerts.\"}]"));
    }

    #[test]
    fn an_answer_whose_evidence_does_not_hold_is_rejected() {
        let rejected = |item: Value| {
            let model = Model::new(vec![json!({"language": "English", "candidates": [item]})]);
            model.run(&session(Vec::new()), &BTreeMap::new(), &|_| Vec::new()).unwrap_err().message
        };
        assert!(rejected(candidate("decision", ALERT, -1, &[("L5", "no alerts at all")])).contains("does not match"));
        assert!(rejected(candidate("decision", ALERT, -1, &[("L99", "Finished jobs")])).contains("does not match"));
        assert!(rejected(candidate("decision", ALERT, -1, &[])).contains("needs evidence"));
        // Only the assistant said it: that is not the user's decision.
        assert!(rejected(candidate("decision", "The export pages through rows.", -1, &[("L6", "loaded every row at once")])).contains("must quote a turn of the user"));
        // An agent's message in the user's input is not the user either.
        assert!(rejected(candidate("correction", "Report paging fixes.", -1, &[("L8", "paging fixed the timeout")])).contains("must quote a turn of the user"));
        assert!(rejected(candidate("decision", "Keep the status panel.", -1, &[("L2", "keep the status panel")])).contains("new_turns"));
        assert!(rejected(candidate("lesson", &"word ".repeat(200), -1, &[("L6", "timed out")])).contains("at most"));
    }

    #[test]
    fn each_candidate_is_related_to_the_notes_a_search_finds_for_it() {
        let model = Model::new(vec![
            json!({"language": "English", "candidates": [
                candidate("decision", ALERT, -1, &[("L5", "Finished jobs get no desktop alert.")]),
                candidate("correction", "Answer the user in short sentences.", -1, &[("L5", "They distract me")]),
            ]}),
            json!({"relations": [
                {"index": 1, "type": "same", "note": "common/style.md", "quotes": ["Answer in short sentences."]},
                {"index": 0, "type": "differs", "note": "projects/demo/alerts.md", "quotes": ["Every finished job raises a desktop alert."]},
            ]}),
        ]);
        let search = |text: &str| if text.contains("alert") { vec!["projects/demo/alerts.md".to_string(), "projects/demo/gone.md".to_string()] } else { vec!["common/style.md".to_string()] };
        let prepared = model.run(&session(Vec::new()), &notes(), &search).unwrap();
        assert_eq!(prepared.offered.len(), 1);
        let relation = &prepared.offered[0].relation;
        assert_eq!((relation.kind, relation.note.as_str(), relation.title.as_str()), (RelationKind::Differs, "projects/demo/alerts.md", "Alerts"));
        assert_eq!(prepared.left_out.iter().map(|found| (found.text.as_str(), found.scope)).collect::<Vec<_>>(), vec![("Answer the user in short sentences.", Scope::Common)]);
        let relate_prompt = &model.prompts.borrow()[1];
        assert!(relate_prompt.contains("Every finished job raises a desktop alert."));
        // The program owns the source list, so the model is not shown it.
        assert!(!relate_prompt.contains("raise alerts"));
    }

    #[test]
    fn a_relation_must_quote_the_note_word_for_word_and_cover_every_candidate() {
        let extract = json!({"language": "English", "candidates": [candidate("decision", ALERT, -1, &[("L5", "Finished jobs get no desktop alert.")])]});
        let search = |_: &str| vec!["projects/demo/alerts.md".to_string()];
        let rejected = |relations: Value| Model::new(vec![extract.clone(), json!({"relations": relations})]).run(&session(Vec::new()), &notes(), &search).unwrap_err().message;
        assert!(rejected(json!([{"index": 0, "type": "differs", "note": "projects/demo/alerts.md", "quotes": ["Jobs raise alerts."]}])).contains("word for word"));
        assert!(rejected(json!([{"index": 0, "type": "differs", "note": "projects/demo/alerts.md", "quotes": []}])).contains("quotes no passage"));
        assert!(rejected(json!([{"index": 0, "type": "obsolete", "note": "common/style.md", "quotes": ["Answer in short sentences."]}])).contains("was not provided"));
        assert!(rejected(json!([])).contains("has no relation"));
        assert!(rejected(json!([{"index": 3, "type": "new", "note": "", "quotes": []}])).contains("no candidate 3"));
    }

    #[test]
    fn notes_are_shown_in_search_order_until_the_request_is_full() {
        let big = "x".repeat(NOTES_CHARS - 10);
        let notes = BTreeMap::from([("common/a.md".to_string(), big), ("common/b.md".to_string(), "# B\n\nsmall\n".to_string()), ("common/c.md".to_string(), "# C\n\nsmall\n".to_string())]);
        let search = |text: &str| if text == "one" { vec!["common/a.md".to_string(), "common/b.md".to_string()] } else { vec!["common/c.md".to_string(), "common/missing.md".to_string()] };
        let shown = shown_notes(&["one", "two"], &[], &notes, &search);
        // a is the best result of the first search and fills the request; nothing else fits.
        assert_eq!(shown.keys().map(String::as_str).collect::<Vec<_>>(), vec!["common/a.md"]);
        let required = shown_notes(&["two"], &["common/b.md".to_string()], &notes, &search);
        assert_eq!(required.keys().map(String::as_str).collect::<Vec<_>>(), vec!["common/b.md", "common/c.md"]);
    }

    #[test]
    fn prompts_are_written_in_english_and_keep_the_data_ahead_of_the_instructions() {
        let korean = |text: &str| text.chars().any(|c| ('\u{AC00}'..='\u{D7A3}').contains(&c));
        for (prompt, marker) in [(EXTRACT_PROMPT, "{{session}}"), (RELATE_PROMPT, "{{data}}")] {
            assert!(!korean(prompt));
            assert_eq!(prompt.matches(marker).count(), 1);
            assert!(prompt.find(marker).unwrap() < prompt.find("# Your task").unwrap());
        }
        assert!(!korean(&extract_schema().to_string()) && !korean(&relate_schema().to_string()));
    }
}
