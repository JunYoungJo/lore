//! Tidying the notes that recordings have piled up. A recording sees one session, so it cannot
//! tell that two notes repeat each other or that a note's subject is gone. A cleanup shows the
//! model a whole folder and applies what it proposes: merges, splits, moves and removals.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{Error, Failure, Result};
use crate::notes::{self, CatalogEntry};
use crate::pipeline::write_notes;
use crate::project::is_valid_name;
use crate::proposal::{carried_sources, change_log, check_snapshot, moved_note, render_note, validate, with_numbered_sources, Proposal, Scope};
use crate::recorder::edits_schema;
use crate::review;

pub const PROMPT: &str = include_str!("cleanup-prompt.md");
/// How much note text one request carries. A folder with more is cleaned in several requests,
/// which keeps both the input and the answer within what a recording model handles in one call.
pub const REQUEST_CHARS: usize = 80_000;
const COMMON: &str = "common";

/// The folder to tidy: a project's notes, or the shared ones. A project can take in the notes
/// of other project folders, such as the one it had under an earlier name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub folder: String,
    #[serde(default)]
    pub absorb: Vec<String>,
}

fn prefix(folder: &str) -> String {
    if folder == COMMON {
        format!("{COMMON}/")
    } else {
        format!("projects/{folder}/")
    }
}

impl Target {
    pub fn check(&self) -> Result<()> {
        let named = |folder: &str| folder == COMMON || is_valid_name(folder);
        if !named(&self.folder) || !self.absorb.iter().all(|folder| folder != COMMON && is_valid_name(folder)) {
            return Err(Error::permanent("A cleanup names a project folder or the shared folder."));
        }
        if self.absorb.contains(&self.folder) || (self.folder == COMMON && !self.absorb.is_empty()) {
            return Err(Error::permanent("A project folder can only take in other project folders."));
        }
        Ok(())
    }

    fn folders(&self) -> Vec<String> {
        std::iter::once(&self.folder).chain(&self.absorb).map(|folder| prefix(folder)).collect()
    }

    /// The shared folder, when a project is cleaned. A project cleanup may add to the shared
    /// notes what applies beyond the project; it removes and moves nothing there.
    fn shared(&self) -> Option<String> {
        (self.folder != COMMON).then(|| prefix(COMMON))
    }
}

/// A folder is due for a cleanup once this many of its notes are new or changed since the last one.
pub const DUE_NOTES: usize = 10;
const CLEANED: &str = "Clean up memory notes (";
const UNDONE: &str = "Undo cleanup of memory notes (";

/// What the last cleanup run on this PC came to. It is kept in the app's database, because a
/// run that changes nothing leaves no commit and a run that fails leaves nothing at all.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Check {
    /// When the last run that finished did so; 0 when none has.
    pub at: i64,
    pub changed: bool,
    /// When the last run failed; 0 when the last run finished.
    pub failed_at: i64,
    /// Why the last run failed, or why it stopped before the whole folder was done.
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Gone {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Moved {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LastKind {
    Changed,
    Undone,
}

/// The last cleanup that changed a folder, read from its commit, or the undoing of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Last {
    pub at: i64,
    pub kind: LastKind,
    /// The commit that undoing would revert. `None` once it is undone, or when the memory
    /// folder is not a repository.
    pub commit: Option<String>,
    pub summary: String,
    pub changed: Vec<String>,
    pub removed: Vec<Gone>,
    pub moved: Vec<Moved>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub name: String,
    pub notes: usize,
    /// Notes that are new or changed since the last cleanup; all of them when there was none.
    pub fresh: usize,
    pub due: bool,
    pub last: Option<Last>,
    /// When a later run looked at the folder and found nothing to change.
    pub checked_at: Option<i64>,
    pub failed_at: Option<i64>,
    pub failure: Option<String>,
}

/// The folder a cleanup commit, or the commit undoing one, is about.
fn marked(commit: &review::Commit) -> Option<&str> {
    let rest = commit.subject.strip_prefix(CLEANED).or_else(|| commit.subject.strip_prefix(UNDONE))?;
    rest.split_once(", ").map(|(folder, _)| folder)
}

fn last_of(commit: &review::Commit) -> Last {
    if commit.subject.starts_with(UNDONE) {
        return Last { at: commit.at, kind: LastKind::Undone, commit: None, summary: String::new(), changed: Vec::new(), removed: Vec::new(), moved: Vec::new() };
    }
    let mut summary = String::new();
    let mut removed = Vec::new();
    let mut moved = Vec::new();
    // The message is the summary and then the lines of `proposal::change_log`.
    for line in commit.body.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let gone = line.strip_prefix("Removed ").and_then(|rest| rest.split_once(": "));
        let went = line.strip_prefix("Moved ").and_then(|rest| rest.split_once(" to "));
        match (gone, went) {
            (Some((path, reason)), _) if review::is_note(path) => removed.push(Gone { path: path.into(), reason: reason.into() }),
            (_, Some((from, to))) if review::is_note(from) && review::is_note(to) => moved.push(Moved { from: from.into(), to: to.into() }),
            _ if summary.is_empty() => summary = line.to_string(),
            _ => {}
        }
    }
    let written = |change: &&review::FileChange| change.kind != review::ChangeKind::Deleted && review::is_note(&change.path);
    let changed = commit.files.iter().filter(written).map(|change| change.path.clone()).collect();
    Last { at: commit.at, kind: LastKind::Changed, commit: Some(commit.id.clone()), summary, changed, removed, moved }
}

fn folder_of(path: &str) -> String {
    match path.split('/').collect::<Vec<_>>().as_slice() {
        ["projects", project, _] => project.to_string(),
        _ => COMMON.to_string(),
    }
}

/// The folders that hold notes, the shared one first and then the projects by name, each with
/// what its last cleanup did and how many notes have changed since. Nothing here calls a model:
/// the commits of the memory folder and `checks` say all of it.
pub fn folders(knowledge_root: &Path, checks: &BTreeMap<String, Check>) -> Result<Vec<Folder>> {
    let mut held: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for path in notes::load_all(knowledge_root)?.into_keys() {
        held.entry(folder_of(&path)).or_default().insert(path);
    }
    let history = review::history(knowledge_root)?;
    let describe = |(name, paths): (String, BTreeSet<String>)| {
        let marker = history.iter().find(|commit| marked(commit) == Some(name.as_str()));
        let check = checks.get(&name);
        let finished = check.filter(|check| check.at > 0);
        let since = marker.map_or(0, |commit| commit.at).max(finished.map_or(0, |check| check.at));
        let fresh = if since == 0 {
            paths.len()
        } else {
            let later = history.iter().filter(|commit| commit.at > since).flat_map(|commit| &commit.files);
            later.filter(|change| paths.contains(&change.path)).map(|change| &change.path).collect::<BTreeSet<_>>().len()
        };
        let last = marker.map(last_of).or_else(|| {
            // Without a repository there is no commit to read; the run is still remembered.
            let check = finished.filter(|check| check.changed)?;
            Some(Last { at: check.at, kind: LastKind::Changed, commit: None, summary: String::new(), changed: Vec::new(), removed: Vec::new(), moved: Vec::new() })
        });
        let checked_at = finished.filter(|check| !check.changed && marker.is_none_or(|commit| check.at >= commit.at)).map(|check| check.at);
        let failed_at = check.map(|check| check.failed_at).filter(|at| *at > 0);
        Folder { notes: paths.len(), fresh, due: fresh >= DUE_NOTES, last, checked_at, failed_at, failure: check.and_then(|check| check.error.clone()), name }
    };
    let shared = held.remove(COMMON).map(|paths| describe((COMMON.to_string(), paths)));
    Ok(shared.into_iter().chain(held.into_iter().map(describe)).collect())
}

/// The targets of a run over several folders, in the order they are cleaned: the projects by
/// name and the shared folder last, since cleaning a project can add to the shared notes.
pub fn in_order(mut targets: Vec<Target>) -> Vec<Target> {
    targets.sort_by(|a, b| (a.folder == COMMON, &a.folder).cmp(&(b.folder == COMMON, &b.folder)));
    targets.dedup_by(|a, b| a.folder == b.folder);
    targets
}

/// Undo the last cleanup of a folder with one commit that reverts it. Returns the notes that
/// came back or changed back.
pub fn undo(knowledge_root: &Path, folder: &str) -> Result<Vec<String>> {
    let history = review::history(knowledge_root)?;
    let newest = history.iter().find(|commit| marked(commit) == Some(folder));
    let commit = newest.filter(|commit| commit.subject.starts_with(CLEANED)).ok_or_else(|| Error::permanent(format!("{folder} has no cleanup to undo.")))?;
    let message = format!("{UNDONE}{folder}, {})\n\nThis reverts commit {}.\n", review::local_date(), commit.id);
    review::revert(knowledge_root, &commit.id, &message)?;
    Ok(commit.files.iter().filter(|change| review::is_note(&change.path)).map(|change| change.path.clone()).collect())
}

/// The notes a cleanup can touch: those of its folders and, for a project, the shared ones.
fn load(knowledge_root: &Path, target: &Target) -> Result<BTreeMap<String, String>> {
    let mut found = BTreeMap::new();
    for folder in target.folders().into_iter().chain(target.shared()) {
        found.extend(notes::load_folder(knowledge_root, folder.trim_end_matches('/'))?);
    }
    Ok(found)
}

pub fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "summary": {"type": "string", "description": "One sentence for the app's activity list, in the language the notes are written in: what the cleanup changed, or that the folder needed nothing."},
            "changes": {"type": "array", "description": "One entry per note to edit or create. Empty when no note needs it.", "items": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "A path in the folder being cleaned."},
                    "title": {"type": "string", "description": "New note: the topic and, when there is one, its conclusion. Existing note: empty to keep the title."},
                    "kind": {"type": "string", "enum": ["decision", "lesson", "issue"], "description": "What the whole note is after this change: a decision, a lesson learned, or an open issue."},
                    "authority": {"type": "string", "enum": ["user_decision", "assistant_report", "mixed_evidence"], "description": "What the whole note rests on after this change: the user's own statements, only assistant or agent statements, or both."},
                    "body": {"type": "string", "description": "New note: the complete Markdown body, without the title line. Existing note: empty."},
                    "edits": edits_schema(),
                    "sources_from": {"type": "array", "description": "The notes whose content this change takes in. A new note names at least one.", "items": {
                        "type": "object",
                        "properties": {
                            "path": {"type": "string", "description": "The path of a note whose text you were given."},
                            "lines": {"type": "array", "description": "The numbers, in that note's Sources list, of the quotations that support the content taken from it. Empty takes the whole list, which is right when the note is merged whole.", "items": {"type": "integer"}}
                        },
                        "required": ["path", "lines"],
                        "additionalProperties": false
                    }}
                },
                "required": ["path", "title", "kind", "authority", "body", "edits", "sources_from"],
                "additionalProperties": false
            }},
            "removals": {"type": "array", "description": "Notes to delete.", "items": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "The path of a note whose text you were given."},
                    "reason": {"type": "string", "description": "One sentence: why a future agent no longer needs this note, or which note now holds what it said."}
                },
                "required": ["path", "reason"],
                "additionalProperties": false
            }},
            "moves": {"type": "array", "description": "Notes kept exactly as they are under another path.", "items": {
                "type": "object",
                "properties": {
                    "from": {"type": "string", "description": "The current path of the note."},
                    "to": {"type": "string", "description": "A path in the folder being cleaned that no note uses yet."}
                },
                "required": ["from", "to"],
                "additionalProperties": false
            }},
            "needs_notes": {"type": "array", "description": "Paths from note_catalog whose text is needed before a change can be written.", "items": {"type": "string"}}
        },
        "required": ["summary", "changes", "removals", "moves", "needs_notes"],
        "additionalProperties": false
    })
}

// The notes go where the prompt marks them, ahead of the instructions, as in a recording.
pub fn build_prompt(payload: &Value) -> String {
    PROMPT.replacen("{{notes}}", &payload.to_string(), 1)
}

/// The notes of each request, in path order, so that no request carries more than `budget`.
fn parts(notes: &BTreeMap<String, String>, budget: usize) -> Vec<Vec<String>> {
    let mut parts: Vec<Vec<String>> = Vec::new();
    let mut used = 0;
    for (path, text) in notes {
        let size = with_numbered_sources(text).chars().count();
        match parts.last_mut() {
            Some(part) if used + size <= budget => part.push(path.clone()),
            _ => {
                parts.push(vec![path.clone()]);
                used = 0;
            }
        }
        used += size;
    }
    parts
}

struct Request {
    payload: Value,
    shown: BTreeSet<String>,
}

fn build_request(target: &Target, notes: &BTreeMap<String, String>, shown: &BTreeSet<String>, today: &str, failure: Option<&str>) -> Request {
    let texts: BTreeMap<&String, String> = notes.iter().filter(|(path, _)| shown.contains(*path)).map(|(path, text)| (path, with_numbered_sources(text))).collect();
    let catalog: Vec<CatalogEntry> = notes.iter().filter(|(path, _)| !shown.contains(*path)).map(|(path, text)| CatalogEntry { path: path.clone(), title: notes::title_of(text) }).collect();
    let mut payload = json!({
        "folder": prefix(&target.folder),
        "absorbed_folders": target.absorb.iter().map(|folder| prefix(folder)).collect::<Vec<_>>(),
        "shared_folder": target.shared(),
        "today": today,
        "notes": texts,
        "note_catalog": catalog,
    });
    if let Some(reason) = failure {
        payload["previous_failure"] = json!(reason);
    }
    Request { payload, shown: shown.clone() }
}

/// What one answer does to the folder, checked and rendered but not yet written.
struct Changes {
    summary: String,
    base: BTreeMap<String, String>,
    rendered: BTreeMap<String, String>,
    removed: BTreeSet<String>,
    log: Vec<String>,
}

fn plan(target: &Target, notes: &BTreeMap<String, String>, shown: &BTreeSet<String>, proposal: &Proposal) -> Result<Changes> {
    let existing: BTreeSet<String> = notes.keys().cloned().collect();
    let own = prefix(&target.folder);
    // A note enters the shared folder only as text written for it, never by being moved there whole.
    if let Some(moved) = proposal.moves.iter().find(|moved| !moved.to.starts_with(&own)) {
        return Err(Error::rejected(format!("A note can be moved only into the folder being cleaned: {}", moved.to)));
    }
    let writable = std::iter::once(own).chain(target.shared()).collect();
    let scope = Scope { project: &target.folder, turns: None, existing: &existing, editable: shown, writable, removable: target.folders() };
    validate(proposal, &scope)?;
    let mut changes = Changes { summary: proposal.summary.replace(['\r', '\n'], " ").trim().to_string(), base: notes.clone(), rendered: BTreeMap::new(), removed: BTreeSet::new(), log: change_log(proposal) };
    for moved in &proposal.moves {
        changes.rendered.insert(moved.to.clone(), moved_note(&notes[&moved.from], &moved.to));
        changes.removed.insert(moved.from.clone());
    }
    for change in &proposal.changes {
        let carried = carried_sources(change, |path| notes.get(path).map(String::as_str))?;
        // A cleanup quotes no session, so it adds no source of its own.
        let change = crate::proposal::Change { evidence: Vec::new(), ..change.clone() };
        let rendered = render_note(&change, &target.folder, &|_| String::new(), notes.get(&change.path).map(String::as_str), &carried)?;
        changes.rendered.insert(change.path.clone(), rendered);
    }
    changes.removed.extend(proposal.removals.iter().map(|removal| removal.path.clone()));
    Ok(changes)
}

pub type Ask<'a> = &'a mut dyn FnMut(&str, &Value, &Path) -> Result<Value>;

fn answer(request: &Request, artifacts: &Path, ask: Ask) -> Result<Proposal> {
    let value = ask(&build_prompt(&request.payload), &schema(), artifacts)?;
    serde_json::from_value(value).map_err(|error| Error::rejected(format!("Invalid model response format: {error}")))
}

fn propose(target: &Target, notes: &BTreeMap<String, String>, part: &BTreeSet<String>, today: &str, failure: Option<&str>, artifacts: &Path, ask: Ask) -> Result<Changes> {
    let mut request = build_request(target, notes, part, today, failure);
    let mut proposal = answer(&request, artifacts, ask)?;
    let idle = |proposal: &Proposal| proposal.changes.is_empty() && proposal.removals.is_empty() && proposal.moves.is_empty();
    if idle(&proposal) && !proposal.needs_notes.is_empty() {
        let wider: BTreeSet<String> = part.iter().chain(proposal.needs_notes.iter().filter(|path| notes.contains_key(*path))).cloned().collect();
        request = build_request(target, notes, &wider, today, failure);
        proposal = answer(&request, &artifacts.join("followup"), ask)?;
        if idle(&proposal) && !proposal.needs_notes.is_empty() {
            return Err(Error::rejected("The model requested the existing note body again."));
        }
    }
    plan(target, notes, &request.shown, &proposal)
}

fn apply(knowledge_root: &Path, target: &Target, changes: &Changes) -> Result<()> {
    check_snapshot(&load(knowledge_root, target)?, &changes.base, &changes.rendered, &changes.removed)?;
    write_notes(knowledge_root, &changes.rendered, &changes.removed)?;
    // A folder whose last note was taken in is not left behind empty; Git would not keep it
    // either. Removing a folder that still holds a file fails, which is what should happen.
    for folder in &target.absorb {
        let _ = std::fs::remove_dir(knowledge_root.join(prefix(folder)));
    }
    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Outcome {
    pub summary: String,
    pub written: Vec<String>,
    pub removed: Vec<String>,
    /// What was removed and why, and what was moved where, for the commit message.
    pub log: Vec<String>,
    /// Why a later part of a large folder stopped after earlier parts had been applied.
    pub incomplete: Option<String>,
}

impl Outcome {
    pub fn touched(&self) -> Vec<String> {
        self.written.iter().chain(&self.removed).cloned().collect()
    }
}

/// Clean one folder. With `write` off nothing is changed and the outcome says what would be.
pub fn run(knowledge_root: &Path, target: &Target, budget: usize, today: &str, artifacts: &Path, write: bool, ask: Ask) -> Result<Outcome> {
    target.check()?;
    let is_shared = |path: &String| target.shared().is_some_and(|shared| path.starts_with(&shared));
    let (shared, own): (BTreeMap<String, String>, BTreeMap<String, String>) = load(knowledge_root, target)?.into_iter().partition(|(path, _)| is_shared(path));
    if !own.keys().any(|path| path.starts_with(&prefix(&target.folder))) && target.absorb.is_empty() {
        return Err(Error::permanent(format!("The folder holds no notes: {}", target.folder)));
    }
    // The shared notes go with every request so that nothing is added to them twice. When
    // they would take more than half of a request they are listed by title instead, and the
    // model asks for the ones it needs.
    let shared_size: usize = shared.values().map(|text| with_numbered_sources(text).chars().count()).sum();
    let with_shared = shared_size <= budget / 2;
    let part_budget = if with_shared { budget - shared_size } else { budget };
    let mut written = BTreeSet::new();
    let mut removed = BTreeSet::new();
    let mut summaries = Vec::new();
    let mut last = String::new();
    let mut log = Vec::new();
    let mut incomplete = None;
    for (index, part) in parts(&own, part_budget).into_iter().enumerate() {
        // An earlier part may have removed or rewritten notes of this one.
        let notes = load(knowledge_root, target)?;
        let mut part: BTreeSet<String> = part.into_iter().filter(|path| notes.contains_key(path)).collect();
        if part.is_empty() {
            continue;
        }
        if with_shared {
            part.extend(notes.keys().filter(|path| is_shared(path)).cloned());
        }
        let folder = artifacts.join(format!("part-{}", index + 1));
        // An answer the program rejects is asked for once more, with the reason.
        let mut failure: Option<String> = None;
        let result = loop {
            match propose(target, &notes, &part, today, failure.as_deref(), &folder, ask) {
                Err(error) if error.failure == Failure::Rejected && failure.is_none() => failure = Some(error.message),
                Ok(changes) if write => break apply(knowledge_root, target, &changes).map(|()| changes),
                result => break result,
            }
        };
        let changes = match result {
            Ok(changes) => changes,
            // What earlier parts changed is on disk, so it is reported rather than hidden behind the error.
            Err(error) if write && !(written.is_empty() && removed.is_empty()) => {
                incomplete = Some(error.message);
                break;
            }
            Err(error) => return Err(error),
        };
        last = changes.summary.clone();
        if !changes.rendered.is_empty() || !changes.removed.is_empty() {
            summaries.push(changes.summary);
        }
        log.extend(changes.log);
        // A note written by one part and removed by a later one ends up removed.
        for path in changes.rendered.into_keys() {
            removed.remove(&path);
            written.insert(path);
        }
        for path in changes.removed {
            written.remove(&path);
            removed.insert(path);
        }
    }
    let summary = if summaries.is_empty() { last } else { summaries.join(" ") };
    Ok(Outcome { summary, written: written.into_iter().collect(), removed: removed.into_iter().collect(), log, incomplete })
}

pub fn commit_message(target: &Target, date: &str, summary: &str, log: &[String]) -> String {
    let mut message = format!("{CLEANED}{}, {date})\n", target.folder);
    for paragraph in [summary.trim().to_string(), log.join("\n")] {
        if !paragraph.is_empty() {
            message.push_str(&format!("\n{paragraph}\n"));
        }
    }
    message
}

/// Clean one folder of the memory repository and commit the result as one commit of its own,
/// so that a whole cleanup can be undone by reverting that commit.
pub fn clean(knowledge_root: &Path, target: &Target, artifacts: &Path, ask: Ask) -> Result<Outcome> {
    let date = review::local_date();
    // Notes written but not committed yet get their own commit first.
    review::record_commit(knowledge_root)?;
    let result = run(knowledge_root, target, REQUEST_CHARS, &date, artifacts, true, ask);
    let (summary, log) = result.as_ref().map(|outcome| (outcome.summary.as_str(), outcome.log.as_slice())).unwrap_or_default();
    review::commit_all(knowledge_root, &commit_message(target, &date, summary, log))?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proposal::{Change, Edit, Move, Removal, SourceRef};
    use serde_json::json;
    use std::cell::RefCell;
    use std::fs;

    const BG: &str = "---\nqmd:\n  metadata:\n    project: \"demo\"\n    kind: \"decision\"\n    authority: \"user_decision\"\n---\n\n# BG 불일치는 로그만 남긴다\n\n## 결정\n\n경고 없이 TP 로그만 남깁니다.\n\n## Sources\n- [L4](<../../sources/claude/a.md#L4>) — 로그만 남기십시오\n";
    const POPUP: &str = "---\nqmd:\n  metadata:\n    project: \"demo\"\n    kind: \"decision\"\n    authority: \"assistant_report\"\n---\n\n# 경고 팝업 문구\n\n팝업 문구는 짧게 씁니다.\n\n## Sources\n- [L9](<../../sources/codex/b.md#L9>) — 문구는 짧게\n";
    const OLD: &str = "---\nqmd:\n  metadata:\n    project: \"old\"\n    kind: \"lesson\"\n    authority: \"mixed_evidence\"\n---\n\n# 큐 잠금\n\n큐는 한 번에 하나만 처리합니다.\n\n## Sources\n- [L2](<../../sources/claude/c.md#L2>) — 하나씩 처리\n";

    struct Fixture {
        dir: tempfile::TempDir,
    }

    impl Fixture {
        fn new(notes: &[(&str, &str)]) -> Self {
            let fixture = Self { dir: tempfile::tempdir().unwrap() };
            for (path, text) in notes {
                let file = fixture.root().join(path);
                fs::create_dir_all(file.parent().unwrap()).unwrap();
                fs::write(file, text).unwrap();
            }
            fixture
        }

        fn root(&self) -> &Path {
            self.dir.path()
        }

        fn read(&self, path: &str) -> Option<String> {
            fs::read_to_string(self.root().join(path)).ok()
        }
    }

    fn demo() -> Target {
        Target { folder: "demo".into(), absorb: vec![] }
    }

    fn taking_in_old() -> Target {
        Target { folder: "demo".into(), absorb: vec!["old".into()] }
    }

    fn append(path: &str, section: &str, text: &str, sources_from: &[&str]) -> Change {
        Change {
            path: path.into(),
            kind: "decision".into(),
            authority: "mixed_evidence".into(),
            edits: vec![Edit { op: "append".into(), find: String::new(), section: section.into(), heading: String::new(), text: text.into() }],
            sources_from: sources_from.iter().map(|path| SourceRef::from(*path)).collect(),
            ..Change::default()
        }
    }

    fn remove(path: &str) -> Removal {
        Removal { path: path.into(), reason: "다른 노트에 모두 들어 있습니다.".into(), evidence: vec![] }
    }

    /// The data a prompt carries between its tags. The prompt file is checked out with CRLF line
    /// endings on Windows, and its opening sentences mention the tag, so the data is what lies
    /// between the closing tag and the last opening tag before it, whatever the line breaks are.
    fn payload_of(prompt: &str) -> Value {
        let end = prompt.find("</notes>").unwrap();
        let start = prompt[..end].rfind("<notes>").unwrap() + "<notes>".len();
        serde_json::from_str(prompt[start..end].trim()).unwrap()
    }

    /// Answers each request with the next prepared proposal and keeps what it was asked.
    struct Model {
        answers: RefCell<Vec<Value>>,
        asked: RefCell<Vec<Value>>,
    }

    impl Model {
        fn answering(answers: Vec<Proposal>) -> Self {
            Self { answers: RefCell::new(answers.into_iter().rev().map(|proposal| serde_json::to_value(proposal).unwrap()).collect()), asked: RefCell::new(Vec::new()) }
        }

        fn ask(&self, prompt: &str, schema: &Value, _: &Path) -> Result<Value> {
            assert_eq!(schema, &super::schema());
            self.asked.borrow_mut().push(payload_of(prompt));
            Ok(self.answers.borrow_mut().pop().expect("the model was asked more often than expected"))
        }

        fn asked(&self, index: usize) -> Value {
            self.asked.borrow()[index].clone()
        }
    }

    fn tidy(fixture: &Fixture, target: &Target, budget: usize, write: bool, model: &Model) -> Result<Outcome> {
        run(fixture.root(), target, budget, "2026-11-12", &fixture.root().join("artifacts"), write, &mut |prompt, schema, artifacts| model.ask(prompt, schema, artifacts))
    }

    #[test]
    fn the_folders_that_can_be_cleaned_are_listed_with_their_note_counts() {
        let fixture = Fixture::new(&[("common/a.md", "# A"), ("projects/demo/bg.md", BG), ("projects/demo/popup.md", POPUP), ("projects/old/queue.md", OLD), ("projects/demo/notes.txt", "x"), ("sources/claude/a.md", "copy")]);
        fs::create_dir_all(fixture.root().join("projects/empty")).unwrap();
        let listed = folders(fixture.root(), &BTreeMap::new()).unwrap();
        let counts: Vec<(&str, usize)> = listed.iter().map(|folder| (folder.name.as_str(), folder.notes)).collect();
        assert_eq!(counts, vec![("common", 1), ("demo", 2), ("old", 1)]);
        // A folder that was never cleaned counts every note as new, and is due once it holds enough.
        assert!(listed.iter().all(|folder| folder.fresh == folder.notes && !folder.due && folder.last.is_none() && folder.checked_at.is_none()));
        let many: Vec<(String, &str)> = (0..DUE_NOTES).map(|index| (format!("projects/big/n{index}.md"), "# N")).collect();
        let grown = Fixture::new(&many.iter().map(|(path, text)| (path.as_str(), *text)).collect::<Vec<_>>());
        assert!(folders(grown.root(), &BTreeMap::new()).unwrap()[0].due);
    }

    #[test]
    fn a_run_over_several_folders_cleans_the_shared_folder_last_and_each_folder_once() {
        let named = |folder: &str| Target { folder: folder.into(), absorb: vec![] };
        let order: Vec<String> = in_order(vec![named("common"), named("zeta"), named("alpha"), named("zeta")]).into_iter().map(|target| target.folder).collect();
        assert_eq!(order, vec!["alpha", "zeta", "common"]);
    }

    #[test]
    fn a_merge_carries_the_sources_over_and_removes_the_note_that_was_taken_in() {
        let fixture = Fixture::new(&[("projects/demo/bg.md", BG), ("projects/demo/popup.md", POPUP), ("common/a.md", "# A\n\n공용\n")]);
        let proposal = Proposal {
            summary: "팝업 노트를 BG 노트에 합쳤습니다.".into(),
            changes: vec![append("projects/demo/bg.md", "결정", "팝업 문구는 짧게 씁니다.", &["projects/demo/popup.md"])],
            removals: vec![remove("projects/demo/popup.md")],
            ..Proposal::default()
        };
        let model = Model::answering(vec![proposal]);
        let outcome = tidy(&fixture,&demo(), 100_000, true, &model).unwrap();
        assert_eq!(
            outcome,
            Outcome {
                summary: "팝업 노트를 BG 노트에 합쳤습니다.".into(),
                written: vec!["projects/demo/bg.md".into()],
                removed: vec!["projects/demo/popup.md".into()],
                log: vec!["Removed projects/demo/popup.md: 다른 노트에 모두 들어 있습니다.".into()],
                incomplete: None,
            }
        );
        assert_eq!(fixture.read("projects/demo/popup.md"), None);
        let merged = fixture.read("projects/demo/bg.md").unwrap();
        assert!(merged.contains("경고 없이 TP 로그만 남깁니다.\n\n팝업 문구는 짧게 씁니다.\n"), "{merged}");
        assert!(merged.ends_with("## Sources\n- [L4](<../../sources/claude/a.md#L4>) — 로그만 남기십시오\n- [L9](<../../sources/codex/b.md#L9>) — 문구는 짧게\n"), "{merged}");
        assert!(merged.contains("authority: \"mixed_evidence\""));

        // The model saw this folder's notes and the shared ones, without their source lists.
        let asked = model.asked(0);
        assert_eq!((&asked["folder"], &asked["shared_folder"]), (&json!("projects/demo/"), &json!("common/")));
        assert_eq!(asked["today"], "2026-11-12");
        assert_eq!(asked["notes"].as_object().unwrap().keys().cloned().collect::<Vec<_>>(), vec!["common/a.md", "projects/demo/bg.md", "projects/demo/popup.md"]);
        // Each note's source list is shown as numbered quotations, without the links.
        let shown = asked["notes"]["projects/demo/bg.md"].as_str().unwrap();
        assert!(shown.ends_with("경고 없이 TP 로그만 남깁니다.\n\n## Sources\n1. 로그만 남기십시오\n"), "{shown}");
    }

    #[test]
    fn a_note_taken_apart_gives_each_piece_only_the_source_lines_named_for_it() {
        let ticket = "---\nqmd:\n  metadata:\n    project: \"demo\"\n    kind: \"issue\"\n    authority: \"mixed_evidence\"\n---\n\n# ISSUE-88\n\n지적과 미해결이 섞인 노트\n\n## Sources\n- [L30](<../../sources/claude/i.md#L30>) — 끝난 것부터 말해\n- [L41](<../../sources/claude/i.md#L41>) — 4GB 미만에서는 실행하지 않았습니다\n";
        let fixture = Fixture::new(&[("projects/demo/bg.md", BG), ("projects/demo/issue-88.md", ticket)]);
        let lines = |numbers: Vec<usize>| vec![SourceRef::Lines { path: "projects/demo/issue-88.md".into(), lines: numbers }];
        let proposal = Proposal {
            summary: "이슈 노트를 나눴습니다.".into(),
            changes: vec![
                Change { sources_from: lines(vec![1]), ..shared_note("common/answer-status-first.md", &[]) },
                Change { sources_from: lines(vec![2]), ..append("projects/demo/bg.md", "결정", "4GB 미만 PC에서는 확인하지 않았습니다.", &[]) },
            ],
            removals: vec![remove("projects/demo/issue-88.md")],
            ..Proposal::default()
        };
        let model = Model::answering(vec![proposal]);
        tidy(&fixture, &demo(), 100_000, true, &model).unwrap();
        let shared = fixture.read("common/answer-status-first.md").unwrap();
        assert!(shared.ends_with("## Sources\n- [L30](<../sources/claude/i.md#L30>) — 끝난 것부터 말해\n"), "{shared}");
        let topic = fixture.read("projects/demo/bg.md").unwrap();
        assert!(topic.ends_with("## Sources\n- [L4](<../../sources/claude/a.md#L4>) — 로그만 남기십시오\n- [L41](<../../sources/claude/i.md#L41>) — 4GB 미만에서는 실행하지 않았습니다\n"), "{topic}");

        let wrong = Proposal { summary: "s".into(), changes: vec![Change { sources_from: lines(vec![3]), ..append("projects/demo/bg.md", "결정", "추가", &[]) }], ..Proposal::default() };
        let refused = Fixture::new(&[("projects/demo/bg.md", BG), ("projects/demo/issue-88.md", ticket)]);
        let model = Model::answering(vec![wrong.clone(), wrong]);
        assert_eq!(tidy(&refused, &demo(), 100_000, true, &model).unwrap_err().failure, Failure::Rejected);
        assert!(model.asked(1)["previous_failure"].as_str().unwrap().contains("no source line 3"));
    }

    const RULE: &str = "---\nqmd:\n  metadata:\n    project: \"common\"\n    kind: \"lesson\"\n    authority: \"user_decision\"\n---\n\n# 보고 방식\n\n## 규칙\n\n확인한 것만 보고합니다.\n\n## Sources\n- [L1](<../sources/claude/r.md#L1>) — 확인한 것만\n";

    fn shared_note(path: &str, sources_from: &[&str]) -> Change {
        Change {
            path: path.into(),
            title: "상태를 물으면 상태부터 답한다".into(),
            kind: "lesson".into(),
            authority: "user_decision".into(),
            body: "원인 조사를 이어 가지 않고 진행 정도부터 답합니다. demo 프로젝트의 작업에서 나온 지적입니다.".into(),
            sources_from: sources_from.iter().map(|path| SourceRef::from(*path)).collect(),
            ..Change::default()
        }
    }

    #[test]
    fn a_project_cleanup_can_put_a_working_rule_into_the_shared_folder() {
        let fixture = Fixture::new(&[("projects/demo/bg.md", BG), ("projects/demo/popup.md", POPUP), ("common/reporting.md", RULE)]);
        let proposal = Proposal {
            summary: "작업 방식 지적을 공용 폴더로 옮겼습니다.".into(),
            changes: vec![shared_note("common/answer-status-first.md", &["projects/demo/popup.md"]), append("common/reporting.md", "규칙", "문구는 짧게 씁니다.", &["projects/demo/popup.md"])],
            removals: vec![remove("projects/demo/popup.md")],
            ..Proposal::default()
        };
        let model = Model::answering(vec![proposal]);
        let outcome = tidy(&fixture, &demo(), 100_000, true, &model).unwrap();
        assert_eq!(outcome.written, vec!["common/answer-status-first.md".to_string(), "common/reporting.md".to_string()]);
        assert_eq!(outcome.removed, vec!["projects/demo/popup.md".to_string()]);

        // A note in the shared folder names that folder and reaches the conversation copies from its own depth.
        let created = fixture.read("common/answer-status-first.md").unwrap();
        assert!(created.starts_with("---\nqmd:\n  metadata:\n    project: \"common\"\n"), "{created}");
        assert!(created.ends_with("## Sources\n- [L9](<../sources/codex/b.md#L9>) — 문구는 짧게\n"), "{created}");
        let edited = fixture.read("common/reporting.md").unwrap();
        assert!(edited.contains("확인한 것만 보고합니다.\n\n문구는 짧게 씁니다.\n"), "{edited}");
        assert!(edited.ends_with("## Sources\n- [L1](<../sources/claude/r.md#L1>) — 확인한 것만\n- [L9](<../sources/codex/b.md#L9>) — 문구는 짧게\n"), "{edited}");
    }

    #[test]
    fn a_project_cleanup_removes_and_moves_nothing_in_the_shared_folder() {
        let moving = |from: &str, to: &str| Proposal { summary: "s".into(), moves: vec![Move { from: from.into(), to: to.into() }], ..Proposal::default() };
        for refused in [
            Proposal { summary: "s".into(), removals: vec![remove("common/reporting.md")], ..Proposal::default() },
            moving("projects/demo/bg.md", "common/bg.md"),
            moving("common/reporting.md", "projects/demo/reporting.md"),
            moving("common/reporting.md", "common/reporting-rules.md"),
        ] {
            let fixture = Fixture::new(&[("projects/demo/bg.md", BG), ("common/reporting.md", RULE)]);
            let model = Model::answering(vec![refused.clone(), refused.clone()]);
            assert_eq!(tidy(&fixture, &demo(), 100_000, true, &model).unwrap_err().failure, Failure::Rejected, "{refused:?}");
            assert_eq!((fixture.read("common/reporting.md").as_deref(), fixture.read("projects/demo/bg.md").as_deref()), (Some(RULE), Some(BG)));
        }
    }

    #[test]
    fn the_shared_notes_go_with_every_part_and_are_listed_by_title_when_they_are_too_many() {
        let fixture = Fixture::new(&[("projects/demo/bg.md", BG), ("projects/demo/popup.md", POPUP), ("common/reporting.md", RULE)]);
        let nothing = || Proposal { summary: "없음".into(), ..Proposal::default() };
        let keys = |asked: &Value| asked["notes"].as_object().unwrap().keys().cloned().collect::<Vec<_>>();
        let size = |text: &str| with_numbered_sources(text).chars().count();

        // Room for the shared note and one project note: two parts, each with the shared note.
        let model = Model::answering(vec![nothing(), nothing()]);
        tidy(&fixture, &demo(), size(RULE) + size(BG) + 10, true, &model).unwrap();
        assert_eq!(keys(&model.asked(0)), vec!["common/reporting.md", "projects/demo/bg.md"]);
        assert_eq!(keys(&model.asked(1)), vec!["common/reporting.md", "projects/demo/popup.md"]);

        // Shared notes that would take more than half of a request are named, and their text is sent when asked for.
        let long = format!("# 긴 공용 노트\n\n{}\n", "가".repeat(400));
        let fixture = Fixture::new(&[("projects/demo/bg.md", BG), ("projects/demo/popup.md", POPUP), ("common/reporting.md", RULE), ("common/long.md", &long)]);
        let wants_rule = Proposal { summary: "s".into(), needs_notes: vec!["common/reporting.md".into()], ..Proposal::default() };
        let model = Model::answering(vec![wants_rule, nothing()]);
        tidy(&fixture, &demo(), size(BG) + size(POPUP) + 10, true, &model).unwrap();
        assert_eq!(keys(&model.asked(0)), vec!["projects/demo/bg.md", "projects/demo/popup.md"]);
        assert_eq!(model.asked(0)["note_catalog"], json!([{"path": "common/long.md", "title": "긴 공용 노트"}, {"path": "common/reporting.md", "title": "보고 방식"}]));
        assert_eq!(keys(&model.asked(1)), vec!["common/reporting.md", "projects/demo/bg.md", "projects/demo/popup.md"]);
    }

    #[test]
    fn a_folder_taken_in_is_moved_or_removed_note_by_note_and_only_the_target_is_written() {
        let fixture = Fixture::new(&[("projects/demo/bg.md", BG), ("projects/old/queue.md", OLD), ("projects/old/popup.md", POPUP)]);
        let proposal = Proposal {
            summary: "old 폴더를 옮겼습니다.".into(),
            moves: vec![Move { from: "projects/old/queue.md".into(), to: "projects/demo/queue-lock.md".into() }],
            removals: vec![remove("projects/old/popup.md")],
            ..Proposal::default()
        };
        let model = Model::answering(vec![proposal]);
        let outcome = tidy(&fixture,&taking_in_old(), 100_000, true, &model).unwrap();
        assert_eq!(outcome.written, vec!["projects/demo/queue-lock.md".to_string()]);
        assert_eq!(outcome.removed, vec!["projects/old/popup.md".to_string(), "projects/old/queue.md".to_string()]);
        assert_eq!(fixture.read("projects/demo/queue-lock.md").unwrap(), OLD.replace("project: \"old\"", "project: \"demo\""));
        assert_eq!((fixture.read("projects/old/queue.md"), fixture.read("projects/old/popup.md")), (None, None));
        // The folder that was taken in whole does not stay behind empty.
        assert!(!fixture.root().join("projects/old").exists());
        assert_eq!(model.asked(0)["absorbed_folders"], json!(["projects/old/"]));

        let into_old = Proposal { summary: "s".into(), moves: vec![Move { from: "projects/demo/bg.md".into(), to: "projects/old/bg.md".into() }], ..Proposal::default() };
        let refused = Fixture::new(&[("projects/demo/bg.md", BG), ("projects/old/queue.md", OLD)]);
        let model = Model::answering(vec![into_old.clone(), into_old]);
        assert_eq!(tidy(&refused,&taking_in_old(), 100_000, true, &model).unwrap_err().failure, Failure::Rejected);
        assert_eq!(refused.read("projects/demo/bg.md").as_deref(), Some(BG));
    }

    #[test]
    fn a_rejected_answer_is_asked_again_once_with_the_reason() {
        let fixture = Fixture::new(&[("projects/demo/bg.md", BG), ("projects/demo/popup.md", POPUP)]);
        let invented = Proposal { summary: "s".into(), changes: vec![Change { path: "projects/demo/new.md".into(), title: "새 노트".into(), kind: "lesson".into(), authority: "mixed_evidence".into(), body: "근거 없는 내용".into(), ..Change::default() }], ..Proposal::default() };
        let fixed = Proposal { summary: "정리할 것이 없습니다.".into(), ..Proposal::default() };
        let model = Model::answering(vec![invented, fixed]);
        let outcome = tidy(&fixture,&demo(), 100_000, true, &model).unwrap();
        assert_eq!(outcome, Outcome { summary: "정리할 것이 없습니다.".into(), ..Outcome::default() });
        assert!(model.asked(0).get("previous_failure").is_none());
        assert!(model.asked(1)["previous_failure"].as_str().unwrap().contains("sources_from"));
    }

    #[test]
    fn a_preview_changes_no_file() {
        let fixture = Fixture::new(&[("projects/demo/bg.md", BG), ("projects/demo/popup.md", POPUP)]);
        let model = Model::answering(vec![Proposal { summary: "s".into(), removals: vec![remove("projects/demo/popup.md")], ..Proposal::default() }]);
        let outcome = tidy(&fixture,&demo(), 100_000, false, &model).unwrap();
        assert_eq!(outcome.removed, vec!["projects/demo/popup.md".to_string()]);
        assert_eq!(fixture.read("projects/demo/popup.md").as_deref(), Some(POPUP));
    }

    #[test]
    fn a_large_folder_is_cleaned_in_parts_that_list_the_other_notes_by_title() {
        let fixture = Fixture::new(&[("projects/demo/bg.md", BG), ("projects/demo/popup.md", POPUP), ("projects/demo/queue.md", OLD)]);
        let nothing = || Proposal { summary: "없음".into(), ..Proposal::default() };
        let wants_bg = Proposal { summary: "s".into(), needs_notes: vec!["projects/demo/bg.md".into()], ..Proposal::default() };
        let merge = Proposal {
            summary: "합침".into(),
            changes: vec![append("projects/demo/bg.md", "결정", "팝업 문구는 짧게 씁니다.", &["projects/demo/popup.md"])],
            removals: vec![remove("projects/demo/popup.md")],
            ..Proposal::default()
        };
        // Part 1 holds bg, part 2 holds popup and asks for bg, part 3 holds queue.
        let model = Model::answering(vec![nothing(), wants_bg, merge, nothing()]);
        let budget = with_numbered_sources(POPUP).chars().count() + 10;
        let outcome = tidy(&fixture,&demo(), budget, true, &model).unwrap();
        assert_eq!((outcome.written, outcome.removed), (vec!["projects/demo/bg.md".to_string()], vec!["projects/demo/popup.md".to_string()]));
        assert_eq!(outcome.summary, "합침");

        let keys = |asked: &Value| asked["notes"].as_object().unwrap().keys().cloned().collect::<Vec<_>>();
        assert_eq!(keys(&model.asked(0)), vec!["projects/demo/bg.md"]);
        assert_eq!(model.asked(0)["note_catalog"], json!([{"path": "projects/demo/popup.md", "title": "경고 팝업 문구"}, {"path": "projects/demo/queue.md", "title": "큐 잠금"}]));
        assert_eq!(keys(&model.asked(1)), vec!["projects/demo/popup.md"]);
        assert_eq!(keys(&model.asked(2)), vec!["projects/demo/bg.md", "projects/demo/popup.md"]);
        // The last part no longer lists the note the part before it removed.
        assert_eq!(keys(&model.asked(3)), vec!["projects/demo/queue.md"]);
        assert_eq!(model.asked(3)["note_catalog"], json!([{"path": "projects/demo/bg.md", "title": "BG 불일치는 로그만 남긴다"}]));
    }

    #[test]
    fn the_shared_folder_is_cleaned_on_its_own_and_takes_in_nothing() {
        let fixture = Fixture::new(&[("common/a.md", "# A\n\n공용\n"), ("common/b.md", "# B\n\n공용\n"), ("projects/demo/bg.md", BG)]);
        let common = Target { folder: "common".into(), absorb: vec![] };
        let model = Model::answering(vec![Proposal { summary: "s".into(), removals: vec![remove("common/b.md")], ..Proposal::default() }]);
        let outcome = tidy(&fixture,&common, 100_000, true, &model).unwrap();
        assert_eq!(outcome.removed, vec!["common/b.md".to_string()]);
        assert_eq!((&model.asked(0)["folder"], &model.asked(0)["shared_folder"]), (&json!("common/"), &Value::Null));
        assert_eq!(model.asked(0)["notes"].as_object().unwrap().len(), 2);

        let never = Model::answering(vec![]);
        for bad in [
            Target { folder: "common".into(), absorb: vec!["demo".into()] },
            Target { folder: "demo".into(), absorb: vec!["common".into()] },
            Target { folder: "demo".into(), absorb: vec!["demo".into()] },
            Target { folder: "../demo".into(), absorb: vec![] },
            Target { folder: "missing".into(), absorb: vec![] },
        ] {
            assert_eq!(tidy(&fixture,&bad, 100_000, true, &never).unwrap_err().failure, Failure::Permanent, "{bad:?}");
        }
    }

    #[test]
    fn a_note_edited_while_the_model_was_working_stops_the_cleanup_before_anything_is_written() {
        let fixture = Fixture::new(&[("projects/demo/bg.md", BG), ("projects/demo/popup.md", POPUP)]);
        let proposal = Proposal { summary: "s".into(), changes: vec![append("projects/demo/bg.md", "결정", "추가", &[])], removals: vec![remove("projects/demo/popup.md")], ..Proposal::default() };
        let root = fixture.root().to_path_buf();
        let result = run(&root, &demo(), 100_000, "2026-11-12", &root.join("artifacts"), true, &mut |_, _, _| {
            fs::write(root.join("projects/demo/popup.md"), "# 경고 팝업 문구\n\n사용자가 고친 내용\n").unwrap();
            Ok(serde_json::to_value(&proposal).unwrap())
        });
        assert_eq!(result.unwrap_err().failure, Failure::Permanent);
        assert_eq!(fixture.read("projects/demo/bg.md").as_deref(), Some(BG));
        assert!(fixture.read("projects/demo/popup.md").unwrap().contains("사용자가 고친 내용"));
    }

    #[test]
    fn a_part_that_fails_after_an_earlier_part_was_applied_reports_what_was_done() {
        let fixture = Fixture::new(&[("projects/demo/bg.md", BG), ("projects/demo/popup.md", POPUP), ("projects/demo/queue.md", OLD)]);
        let root = fixture.root().to_path_buf();
        let first = Proposal { summary: "BG 노트를 지웠습니다.".into(), removals: vec![remove("projects/demo/bg.md")], ..Proposal::default() };
        let mut asked = 0;
        let budget = with_numbered_sources(POPUP).chars().count() + 10;
        let outcome = run(&root, &demo(), budget, "2026-11-12", &root.join("artifacts"), true, &mut |_, _, _| {
            asked += 1;
            if asked == 1 { Ok(serde_json::to_value(&first).unwrap()) } else { Err(Error::transient("usage limit reached")) }
        })
        .unwrap();
        assert_eq!(outcome.removed, vec!["projects/demo/bg.md".to_string()]);
        assert_eq!(outcome.incomplete.as_deref(), Some("usage limit reached"));
        assert_eq!(asked, 2);

        let nothing_done = run(&root, &demo(), budget, "2026-11-12", &root.join("artifacts"), true, &mut |_, _, _| Err(Error::transient("usage limit reached")));
        assert_eq!(nothing_done.unwrap_err().failure, Failure::Transient);
    }

    fn git(root: &Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git").args(args).current_dir(root).output().unwrap();
        assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stdout).to_string()
    }

    fn repository(root: &Path) {
        git(root, &["init", "-q"]);
        git(root, &["config", "user.name", "Lore Test"]);
        git(root, &["config", "user.email", "lore@example.invalid"]);
        git(root, &["config", "commit.gpgsign", "false"]);
        // Git for Windows converts line endings by default; the tests compare file contents exactly.
        git(root, &["config", "core.autocrlf", "false"]);
    }

    /// Commit everything with a commit time of `at` seconds, so that tests can order commits.
    fn commit_at(root: &Path, at: i64, message: &str) {
        git(root, &["add", "-A"]);
        let date = format!("@{at} +0000");
        let output = std::process::Command::new("git").args(["commit", "-q", "-m", message]).env("GIT_AUTHOR_DATE", &date).env("GIT_COMMITTER_DATE", &date).current_dir(root).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    }

    /// A folder whose `popup.md` was merged into `bg.md` by a cleanup, with a recording after it.
    fn cleaned_once() -> Fixture {
        let fixture = Fixture::new(&[("projects/demo/bg.md", BG), ("projects/demo/popup.md", POPUP), ("projects/other/queue.md", OLD)]);
        let root = fixture.root();
        repository(root);
        commit_at(root, 100, "Record memory notes (2026-11-01)");
        fs::write(root.join("projects/demo/bg.md"), BG.replace("TP 로그만", "TP 로그와 팝업 문구만")).unwrap();
        fs::remove_file(root.join("projects/demo/popup.md")).unwrap();
        commit_at(root, 200, &commit_message(&demo(), "2026-11-02", "팝업 노트를 BG 노트에 합쳤습니다.", &["Removed projects/demo/popup.md: BG 노트가 모두 담습니다.".to_string()]));
        fs::write(root.join("projects/demo/new.md"), "# 새 노트\n").unwrap();
        fs::write(root.join("projects/other/queue.md"), OLD.replace("하나만", "둘까지")).unwrap();
        commit_at(root, 300, "Record memory notes (2026-11-03)");
        fixture
    }

    fn named<'a>(folders: &'a [Folder], name: &str) -> &'a Folder {
        folders.iter().find(|folder| folder.name == name).unwrap()
    }

    #[test]
    fn a_folder_shows_what_its_last_cleanup_did_and_how_many_notes_changed_since() {
        let fixture = cleaned_once();
        let listed = folders(fixture.root(), &BTreeMap::new()).unwrap();
        let demo = named(&listed, "demo");
        assert_eq!((demo.notes, demo.fresh, demo.checked_at), (2, 1, None));
        let last = demo.last.as_ref().unwrap();
        assert_eq!((last.at, last.kind, last.summary.as_str()), (200, LastKind::Changed, "팝업 노트를 BG 노트에 합쳤습니다."));
        assert!(last.commit.is_some());
        assert_eq!(last.changed, vec!["projects/demo/bg.md"]);
        assert_eq!(last.removed, vec![Gone { path: "projects/demo/popup.md".into(), reason: "BG 노트가 모두 담습니다.".into() }]);
        // The other folder was never cleaned, so its one note counts as new.
        let other = named(&listed, "other");
        assert_eq!((other.fresh, other.last.is_none()), (1, true));

        // A run that found nothing to change is known only from the app's database.
        let checks = BTreeMap::from([
            ("other".to_string(), Check { at: 250, ..Check::default() }),
            ("demo".to_string(), Check { at: 400, failed_at: 500, error: Some("The model timed out.".into()), ..Check::default() }),
        ]);
        let listed = folders(fixture.root(), &checks).unwrap();
        let other = named(&listed, "other");
        assert_eq!((other.fresh, other.checked_at, other.last.is_none()), (1, Some(250), true));
        let demo = named(&listed, "demo");
        assert_eq!((demo.fresh, demo.checked_at, demo.failed_at, demo.failure.as_deref()), (0, Some(400), Some(500), Some("The model timed out.")));
        assert_eq!(demo.last.as_ref().unwrap().at, 200);
    }

    #[test]
    fn undoing_a_cleanup_brings_the_notes_back_with_one_commit() {
        let fixture = cleaned_once();
        let root = fixture.root();
        let mut touched = undo(root, "demo").unwrap();
        touched.sort();
        assert_eq!(touched, vec!["projects/demo/bg.md", "projects/demo/popup.md"]);
        assert_eq!(fixture.read("projects/demo/popup.md").as_deref(), Some(POPUP));
        assert_eq!(fixture.read("projects/demo/bg.md").as_deref(), Some(BG));
        // What was recorded after the cleanup stays.
        assert!(fixture.read("projects/demo/new.md").is_some());
        assert!(git(root, &["log", "-1", "--format=%s"]).starts_with("Undo cleanup of memory notes (demo, "));
        assert_eq!(git(root, &["status", "--porcelain"]), "");

        // An undone cleanup is not offered again, and undoing does not make the folder due at once.
        let listed = folders(root, &BTreeMap::new()).unwrap();
        let demo = named(&listed, "demo");
        let last = demo.last.as_ref().unwrap();
        assert_eq!((last.kind, last.commit.as_ref(), demo.fresh), (LastKind::Undone, None, 0));
        assert!(undo(root, "demo").unwrap_err().to_string().contains("no cleanup to undo"));
        assert!(undo(root, "other").unwrap_err().to_string().contains("no cleanup to undo"));
    }

    #[test]
    fn a_cleanup_whose_notes_changed_again_is_not_undone_and_nothing_is_left_half_done() {
        let fixture = cleaned_once();
        let root = fixture.root();
        let edited = BG.replace("TP 로그만", "아무것도 남기지 않습니다. 로그도");
        fs::write(root.join("projects/demo/bg.md"), &edited).unwrap();
        commit_at(root, 400, "Record memory notes (2026-11-04)");
        let before = git(root, &["rev-parse", "HEAD"]);

        let error = undo(root, "demo").unwrap_err();
        assert!(error.to_string().contains("cannot be undone automatically"), "{error}");
        assert_eq!(fixture.read("projects/demo/bg.md").as_deref(), Some(edited.as_str()));
        assert_eq!(fixture.read("projects/demo/popup.md"), None);
        assert_eq!(git(root, &["status", "--porcelain"]), "");
        assert_eq!(git(root, &["rev-parse", "HEAD"]), before);
    }

    #[test]
    fn a_cleanup_is_one_commit_of_its_own_that_says_what_was_removed_and_moved() {
        let fixture = Fixture::new(&[("projects/demo/bg.md", BG), ("projects/demo/popup.md", POPUP), ("projects/old/queue.md", OLD)]);
        let root = fixture.root();
        repository(root);
        git(root, &["add", "-A"]);
        git(root, &["commit", "-q", "-m", "Start"]);
        // A recording that was written but not committed yet stays out of the cleanup commit.
        fs::write(root.join("projects/demo/recorded.md"), "# 방금 기록\n\n본문\n").unwrap();

        let proposal = Proposal {
            summary: "팝업 노트를 지우고 큐 노트를 옮겼습니다.".into(),
            removals: vec![remove("projects/demo/popup.md")],
            moves: vec![Move { from: "projects/old/queue.md".into(), to: "projects/demo/queue-lock.md".into() }],
            ..Proposal::default()
        };
        let model = Model::answering(vec![proposal]);
        let artifacts = tempfile::tempdir().unwrap();
        let outcome = clean(root, &taking_in_old(), artifacts.path(), &mut |prompt, schema, folder| model.ask(prompt, schema, folder)).unwrap();
        assert_eq!(outcome.incomplete, None);

        let subjects = git(root, &["log", "--format=%s"]);
        let subjects: Vec<&str> = subjects.lines().collect();
        assert_eq!(subjects.len(), 3, "{subjects:?}");
        assert!(subjects[0].starts_with("Clean up memory notes (demo, ") && subjects[1].starts_with("Record memory notes ("), "{subjects:?}");
        let message = git(root, &["log", "-1", "--format=%B"]);
        assert!(message.contains("\n팝업 노트를 지우고 큐 노트를 옮겼습니다.\n"), "{message}");
        assert!(message.contains("Removed projects/demo/popup.md: 다른 노트에 모두 들어 있습니다."), "{message}");
        assert!(message.contains("Moved projects/old/queue.md to projects/demo/queue-lock.md"), "{message}");
        let files = git(root, &["show", "--name-status", "--no-renames", "--format=", "HEAD"]);
        assert_eq!(files.lines().collect::<Vec<_>>(), vec!["D\tprojects/demo/popup.md", "A\tprojects/demo/queue-lock.md", "D\tprojects/old/queue.md"]);
        assert_eq!(git(root, &["status", "--porcelain"]), "");

        // A cleanup that changes nothing makes no commit.
        let idle = Model::answering(vec![Proposal { summary: "없음".into(), ..Proposal::default() }]);
        clean(root, &demo(), artifacts.path(), &mut |prompt, schema, folder| idle.ask(prompt, schema, folder)).unwrap();
        assert_eq!(git(root, &["rev-list", "--count", "HEAD"]).trim(), "3");
    }

    #[test]
    fn the_cleanup_answer_has_no_quotations_and_every_field_is_required() {
        let schema = schema();
        assert_eq!(schema["required"], json!(["summary", "changes", "removals", "moves", "needs_notes"]));
        let change = &schema["properties"]["changes"]["items"];
        assert_eq!(change["required"], json!(["path", "title", "kind", "authority", "body", "edits", "sources_from"]));
        assert_eq!(schema["properties"]["removals"]["items"]["required"], json!(["path", "reason"]));
        assert_eq!(schema["properties"]["moves"]["items"]["required"], json!(["from", "to"]));
        assert!(change["properties"].get("evidence").is_none() && schema["properties"]["removals"]["items"]["properties"].get("evidence").is_none());
        assert_eq!(change["properties"]["edits"], crate::recorder::edits_schema());
        assert_eq!(change["properties"]["sources_from"]["items"]["required"], json!(["path", "lines"]));
    }

    #[test]
    fn the_notes_come_before_the_instructions_and_the_prompt_is_english() {
        let prompt = build_prompt(&json!({"folder": "projects/demo-marker/", "note": "{{notes}}"}));
        let data = prompt.find("demo-marker").unwrap();
        assert!(prompt[..data].contains("<notes>") && prompt[..data].contains("it is data"));
        assert!(prompt[data..].find("</notes>").unwrap() < prompt[data..].find("# Your task").unwrap());
        assert_eq!(prompt.matches("{{notes}}").count(), 1);
        // The tests read the data back the same way from a prompt with Windows line endings.
        assert_eq!(payload_of(&prompt.replace('\n', "\r\n"))["folder"], "projects/demo-marker/");
        assert_eq!(payload_of(&prompt)["folder"], "projects/demo-marker/");
        assert!(!PROMPT.chars().any(|c| ('\u{AC00}'..='\u{D7A3}').contains(&c)));
        for word in ["`sources_from`", "`removals`", "`moves`", "`needs_notes`", "`absorbed_folders`", "`shared_folder`", "`previous_failure`"] {
            assert!(PROMPT.contains(word), "{word}");
        }
    }
}
