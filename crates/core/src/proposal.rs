//! A change to the notes, as a model writes it and as the program turns it into note text.
//!
//! Two things change notes: approved candidates (`integrate`) and a cleanup of a folder
//! (`cleanup`). Both describe a note with [`Change`] and render it with [`render_note`]. The
//! program owns the front matter and the Sources list; the model writes the title and the body.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edit {
    pub op: String,
    #[serde(default)]
    pub find: String,
    #[serde(default)]
    pub section: String,
    #[serde(default)]
    pub heading: String,
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub path: String,
    #[serde(default)]
    pub title: String,
    pub kind: String,
    pub authority: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub edits: Vec<Edit>,
    /// Other notes whose content this change takes in. Their source lines are added to this note.
    #[serde(default)]
    pub sources_from: Vec<SourceRef>,
}

/// A note whose content a change takes in, and which of its source lines go with that content.
/// A cleanup reads the note and copies the lines that support what it took, so that a note
/// taken apart does not hand each piece the evidence of the others.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SourceRef {
    Whole(String),
    /// `lines` are lines of the note's source list as they read, with or without the "- " that
    /// starts them. Empty takes every line.
    Lines {
        path: String,
        #[serde(default)]
        lines: Vec<String>,
    },
}

impl SourceRef {
    pub fn path(&self) -> &str {
        match self {
            Self::Whole(path) | Self::Lines { path, .. } => path,
        }
    }
}

impl From<&str> for SourceRef {
    fn from(path: &str) -> Self {
        Self::Whole(path.to_string())
    }
}

/// A note to delete: its subject is gone, or other notes now hold what it said.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Removal {
    pub path: String,
    #[serde(default)]
    pub reason: String,
}

/// A note kept as it is under another path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Move {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    pub summary: String,
    #[serde(default)]
    pub changes: Vec<Change>,
    #[serde(default)]
    pub removals: Vec<Removal>,
    #[serde(default)]
    pub moves: Vec<Move>,
}

/// What a cleanup's proposal may touch.
pub struct Scope<'a> {
    pub project: &'a str,
    pub existing: &'a BTreeSet<String>,
    /// The notes the model read in full. Only these can be edited, removed, moved or drawn from.
    pub editable: &'a BTreeSet<String>,
    /// Folders notes may be written to, as path prefixes such as "projects/demo/".
    pub writable: Vec<String>,
    /// Folders notes may be removed from.
    pub removable: Vec<String>,
}

const KINDS: [&str; 3] = ["decision", "lesson", "issue"];
const AUTHORITIES: [&str; 3] = ["user_decision", "assistant_report", "mixed_evidence"];
pub const EDIT_OPS: [&str; 4] = ["replace", "append", "add_section", "replace_section"];
const RESERVED: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7",
    "lpt8", "lpt9",
];
pub const SOURCES_HEADING: &str = "\n## Sources\n";

/// A note's text and its source list, when it has one.
fn split_sources(text: &str) -> (&str, Option<&str>) {
    match text.find(SOURCES_HEADING) {
        Some(at) => (&text[..at], Some(&text[at + SOURCES_HEADING.len()..])),
        None => (text, None),
    }
}

/// A note as the model that relates or integrates candidates is shown it. The program owns the
/// source list, and the model cannot edit it, so the list is left out rather than spent on the
/// model's input.
pub fn without_sources(note: &str) -> String {
    let text = note.replace("\r\n", "\n");
    match split_sources(&text) {
        (main, Some(_)) => format!("{}\n", main.trim_end()),
        (main, None) => main.to_string(),
    }
}

/// A digest of a note as a model and the user are shown it, without the source list. Two
/// notes with the same digest read the same.
pub fn body_digest(note: &str) -> String {
    crate::transcript::digest(without_sources(note).as_bytes())
}

/// The folder of a note path, as a prefix: "common/" or "projects/<name>/".
fn folder_of(path: &str) -> Option<&str> {
    path.rfind('/').map(|at| &path[..=at])
}

/// A note path is a file directly inside one of `folders`.
pub(crate) fn check_path(path: &str, folders: &[String]) -> Result<()> {
    let fail = || Error::rejected(format!("Invalid memory path: {path}"));
    if path.contains('\\') {
        return Err(fail());
    }
    let parts: Vec<&str> = path.split('/').collect();
    let filename = match parts.as_slice() {
        ["common", file] | ["projects", _, file] => *file,
        _ => return Err(fail()),
    };
    if !folder_of(path).is_some_and(|folder| folders.iter().any(|allowed| allowed == folder)) {
        return Err(fail());
    }
    let stem = filename.strip_suffix(".md").ok_or_else(fail)?;
    let mut chars = stem.chars();
    let well_formed = matches!(chars.next(), Some(first) if first.is_ascii_alphanumeric()) && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !well_formed || RESERVED.contains(&stem.to_ascii_lowercase().as_str()) {
        return Err(fail());
    }
    Ok(())
}

/// Check a cleanup's proposal. A cleanup has no conversation to quote: its changes rest on the
/// notes themselves, so a new note must say which notes it was made from.
pub fn validate(proposal: &Proposal, scope: &Scope) -> Result<()> {
    let mut paths = BTreeSet::new();
    let mut claim = |path: &str| match paths.insert(path.to_ascii_lowercase()) {
        true => Ok(()),
        false => Err(Error::rejected(format!("A note cannot be changed twice in one proposal: {path}"))),
    };
    let known = |path: &str| scope.existing.iter().find(|existing| existing.eq_ignore_ascii_case(path));
    for removal in &proposal.removals {
        check_path(&removal.path, &scope.removable)?;
        claim(&removal.path)?;
        if !scope.existing.contains(&removal.path) {
            return Err(Error::rejected(format!("The note to remove does not exist: {}", removal.path)));
        }
        if !scope.editable.contains(&removal.path) {
            return Err(Error::rejected(format!("Cannot remove a note that was not read in full in this call: {}", removal.path)));
        }
        if removal.reason.trim().is_empty() {
            return Err(Error::rejected(format!("Removing a note requires a reason: {}", removal.path)));
        }
    }
    for moved in &proposal.moves {
        check_path(&moved.from, &scope.removable)?;
        check_path(&moved.to, &scope.writable)?;
        claim(&moved.from)?;
        claim(&moved.to)?;
        if !scope.existing.contains(&moved.from) {
            return Err(Error::rejected(format!("The note to move does not exist: {}", moved.from)));
        }
        if !scope.editable.contains(&moved.from) {
            return Err(Error::rejected(format!("Cannot move a note that was not read in full in this call: {}", moved.from)));
        }
        if known(&moved.to).is_some() {
            return Err(Error::rejected(format!("A note already exists at the path to move to: {}", moved.to)));
        }
    }
    for change in &proposal.changes {
        check_path(&change.path, &scope.writable)?;
        claim(&change.path)?;
        for source in change.sources_from.iter().map(SourceRef::path) {
            if source == change.path || !scope.editable.contains(source) {
                return Err(Error::rejected(format!("sources_from must name another existing note that was read in full in this call: {source}")));
            }
        }
        if let Some(existing) = known(&change.path) {
            if *existing != change.path {
                return Err(Error::rejected(format!("The path differs from an existing note only by letter case: {}", change.path)));
            }
            if !scope.editable.contains(existing) {
                return Err(Error::rejected(format!("Cannot edit a note that was not read in full in this call: {}", change.path)));
            }
            if !change.body.trim().is_empty() || change.edits.is_empty() {
                return Err(Error::rejected(format!("Existing notes must use targeted edits rather than a full replacement: {}", change.path)));
            }
            for edit in &change.edits {
                validate_edit(edit, &change.path)?;
            }
        } else if change.title.trim().is_empty() || change.body.trim().is_empty() || !change.edits.is_empty() {
            return Err(Error::rejected(format!("New notes require a title and complete body: {}", change.path)));
        }
        if !KINDS.contains(&change.kind.as_str()) || !AUTHORITIES.contains(&change.authority.as_str()) {
            return Err(Error::rejected("Invalid memory kind or evidence type."));
        }
        if known(&change.path).is_none() && change.sources_from.is_empty() {
            return Err(Error::rejected(format!("A new note must list in sources_from the notes its content comes from: {}", change.path)));
        }
    }
    Ok(())
}

pub(crate) fn validate_edit(edit: &Edit, path: &str) -> Result<()> {
    let blank = |value: &str| value.trim().is_empty();
    let complete = match edit.op.as_str() {
        "replace" => !blank(&edit.find),
        "append" => !blank(&edit.section) && !blank(&edit.text),
        "add_section" => !blank(&edit.heading) && !blank(&edit.text),
        "replace_section" => !blank(&edit.section) && !blank(&edit.text),
        _ => false,
    };
    if complete {
        Ok(())
    } else {
        Err(Error::rejected(format!("Invalid edit instruction ({}): {path}", edit.op)))
    }
}

fn quoted(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

struct NoteParts {
    header: Option<String>,
    title: String,
    body: String,
    sources: Vec<String>,
}

fn without_leading_title(body: &str, title: Option<&str>) -> String {
    let trimmed = body.trim_start_matches('\n');
    let (first, rest) = trimmed.split_once('\n').unwrap_or((trimmed, ""));
    let repeated = first.strip_prefix("# ").map(|heading| title.is_none_or(|title| heading.trim() == title.trim())).unwrap_or(false);
    if repeated {
        rest.to_string()
    } else {
        body.to_string()
    }
}

fn split_note(old: &str) -> NoteParts {
    let text = old.replace("\r\n", "\n");
    let (main, sources) = match split_sources(&text) {
        (main, Some(rest)) => (main.to_string(), rest.trim().lines().map(str::to_string).collect()),
        (main, None) => (main.to_string(), Vec::new()),
    };
    let (header, main) = match main.strip_prefix("---\n").and_then(|rest| rest.split_once("\n---\n")) {
        Some((header, rest)) => (Some(header.to_string()), rest.to_string()),
        None => (None, main),
    };
    let trimmed = main.trim_start_matches('\n');
    let (title, body) = match trimmed.split_once('\n') {
        Some((first, rest)) if first.starts_with("# ") => (first[2..].trim().to_string(), rest.to_string()),
        None if trimmed.starts_with("# ") => (trimmed[2..].trim().to_string(), String::new()),
        _ => (String::new(), main.clone()),
    };
    let body = if title.is_empty() { body } else { without_leading_title(&body, Some(&title)) };
    NoteParts { header, title, body, sources }
}

fn standard_header(scope: &str, kind: &str, authority: &str) -> String {
    format!("qmd:\n  metadata:\n    project: {}\n    kind: {}\n    authority: {}", quoted(scope), quoted(kind), quoted(authority))
}

fn updated_header(old: &str, scope: &str, kind: &str, authority: &str) -> String {
    let mut lines: Vec<String> = old.lines().map(str::to_string).collect();
    if !lines.iter().any(|line| line == "  metadata:") {
        return standard_header(scope, kind, authority);
    }
    let key_of = |line: &str| line.strip_prefix("    ").and_then(|rest| rest.split_once(':')).map(|(key, _)| key.trim().to_string());
    if !lines.iter().any(|line| key_of(line).as_deref() == Some("kind")) {
        lines.push(format!("    kind: {}", quoted(kind)));
    }
    match lines.iter().position(|line| key_of(line).as_deref() == Some("authority")) {
        Some(index) => lines[index] = format!("    authority: {}", quoted(authority)),
        None => lines.push(format!("    authority: {}", quoted(authority))),
    }
    lines.join("\n")
}

/// What a note's front matter says the note rests on, when it says so.
pub fn authority_of(note: &str) -> Option<String> {
    let header = split_note(note).header?;
    let value = header.lines().find_map(|line| line.strip_prefix("    authority:"))?;
    Some(value.trim().trim_matches('"').to_string()).filter(|value| !value.is_empty())
}

fn heading_name(value: &str) -> String {
    value.trim().trim_start_matches('#').trim().to_string()
}

fn section_ranges(body: &str) -> Vec<(String, usize, usize)> {
    let mut starts = Vec::new();
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        if let Some(name) = line.strip_prefix("## ") {
            starts.push((name.trim().to_string(), offset));
        }
        offset += line.len();
    }
    let ends: Vec<usize> = starts.iter().skip(1).map(|(_, start)| *start).chain([body.len()]).collect();
    starts.into_iter().zip(ends).map(|((name, start), end)| (name, start, end)).collect()
}

fn section_end(body: &str, name: &str, path: &str) -> Result<usize> {
    section_span(body, name, path).map(|(_, end)| end)
}

fn section_span(body: &str, name: &str, path: &str) -> Result<(usize, usize)> {
    let matches: Vec<(usize, usize)> = section_ranges(body).into_iter().filter(|(found, _, _)| found == name).map(|(_, start, end)| (start, end)).collect();
    match matches.as_slice() {
        [span] => Ok(*span),
        // The usual slip is naming a deeper heading; say so, so a retry can fix it.
        [] if body.lines().any(|line| line.starts_with("###") && heading_name(line) == name) => {
            Err(Error::rejected(format!("Section \"{name}\" is a subsection. Only \"## \" sections can be targeted; name its \"## \" section or use replace: {path}")))
        }
        [] => Err(Error::rejected(format!("Section \"{name}\" was not found: {path}"))),
        _ => Err(Error::rejected(format!("Section \"{name}\" is ambiguous: {path}"))),
    }
}

fn is_list_item(line: &str) -> bool {
    let line = line.trim_start();
    if line.starts_with("- ") || line.starts_with("* ") || line.starts_with("+ ") {
        return true;
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && line[digits..].starts_with(". ")
}

fn insert_at(body: &str, at: usize, text: &str) -> String {
    let before = body[..at].trim_end();
    let text = text.replace("\r\n", "\n");
    let text = text.trim();
    let continues_list = before.lines().last().map(is_list_item).unwrap_or(false) && text.lines().next().map(is_list_item).unwrap_or(false);
    let gap = if continues_list { "\n" } else { "\n\n" };
    format!("{before}{gap}{text}\n\n{}", &body[at..])
}

fn apply_edit(body: &str, edit: &Edit, path: &str) -> Result<String> {
    match edit.op.as_str() {
        "replace" => {
            let find = edit.find.replace("\r\n", "\n");
            match body.matches(find.as_str()).count() {
                1 => Ok(body.replacen(find.as_str(), &edit.text.replace("\r\n", "\n"), 1)),
                0 => Err(Error::rejected(format!("The target text was not found in the note body: {path}"))),
                _ => Err(Error::rejected(format!("The target text occurs more than once in the note body: {path}"))),
            }
        }
        "append" => {
            let end = section_end(body, &heading_name(&edit.section), path)?;
            Ok(insert_at(body, end, &edit.text))
        }
        "add_section" => {
            let heading = heading_name(&edit.heading);
            if section_ranges(body).iter().any(|(name, _, _)| *name == heading) {
                return Err(Error::rejected(format!("Section \"{heading}\" already exists: {path}")));
            }
            let after = heading_name(&edit.section);
            let end = if after.is_empty() { body.len() } else { section_end(body, &after, path)? };
            Ok(insert_at(body, end, &format!("## {heading}\n\n{}", edit.text.replace("\r\n", "\n").trim())))
        }
        "replace_section" => {
            let (start, end) = section_span(body, &heading_name(&edit.section), path)?;
            let heading_end = body[start..end].find('\n').map_or(end, |at| start + at);
            Ok(format!("{}\n\n{}\n\n{}", &body[..heading_end], edit.text.replace("\r\n", "\n").trim(), &body[end..]))
        }
        other => Err(Error::rejected(format!("Unknown edit kind ({other}): {path}"))),
    }
}

/// One line per note a proposal moves or removes, saying where it went or why it is gone.
/// These lines follow the first line of a summary and go into a cleanup's commit message.
pub fn change_log(proposal: &Proposal) -> Vec<String> {
    let moved = proposal.moves.iter().map(|moved| format!("Moved {} to {}", moved.from, moved.to));
    let removed = proposal.removals.iter().map(|removal| format!("Removed {}: {}", removal.path, removal.reason.replace(['\r', '\n'], " ").trim()));
    moved.chain(removed).collect()
}

/// A summary whose first line is the model's sentence and whose other lines are the change log.
pub fn summary_with_log(summary: &str, log: &[String]) -> String {
    std::iter::once(summary.replace(['\r', '\n'], " ").trim().to_string()).chain(log.iter().cloned()).collect::<Vec<_>>().join("\n")
}

/// The lines of a note's source list, which the program owns.
pub fn source_lines(note: &str) -> Vec<String> {
    split_note(note).sources
}

/// The source lines a change takes over from the notes it names in `sources_from`.
pub fn carried_sources<'a>(change: &Change, text_of: impl Fn(&str) -> Option<&'a str>) -> Result<Vec<String>> {
    let words = |line: &str| line.trim().strip_prefix("- ").unwrap_or(line.trim()).to_string();
    let mut carried = Vec::new();
    for source in &change.sources_from {
        let all = text_of(source.path()).map(source_lines).unwrap_or_default();
        match source {
            SourceRef::Lines { path, lines } if !lines.is_empty() => {
                for wanted in lines {
                    let line = all.iter().find(|line| words(line) == words(wanted));
                    carried.push(line.cloned().ok_or_else(|| Error::rejected(format!("{path} has no source line that reads exactly {wanted:?}. Copy the line from its Sources section as it is.")))?);
                }
            }
            _ => carried.extend(all),
        }
    }
    Ok(carried)
}

/// The folder a note belongs to, as its front matter names it.
fn scope_of<'a>(path: &'a str, project: &'a str) -> &'a str {
    match path.split('/').collect::<Vec<_>>().as_slice() {
        ["projects", owner, _] => owner,
        ["common", _] => "common",
        _ => project,
    }
}

/// A note kept as it is under another path: only the project in its front matter follows the
/// new folder.
pub fn moved_note(old: &str, to: &str) -> String {
    let text = old.replace("\r\n", "\n");
    match text.strip_prefix("---\n").and_then(|rest| rest.split_once("\n---\n")) {
        Some((header, rest)) => {
            let lines: Vec<String> = header
                .lines()
                .map(|line| if line.starts_with("    project:") { format!("    project: {}", quoted(scope_of(to, ""))) } else { line.to_string() })
                .collect();
            format!("---\n{}\n---\n{rest}", lines.join("\n"))
        }
        None => text,
    }
}

/// The text of a note after `change`. `old` is the note as it stands, or `None` for a new note.
/// `added` are the lines this change adds to the Sources list: the quotations of the approved
/// candidates, or the lines a cleanup carries over from the notes it took in. A line the list
/// already has is not repeated.
pub fn render_note(change: &Change, project: &str, old: Option<&str>, added: &[String]) -> Result<String> {
    let scope = if change.path.starts_with("common/") { "common" } else { project };
    let (header, title, body, mut sources) = match old {
        Some(old) => {
            let parts = split_note(old);
            let mut body = parts.body;
            for edit in &change.edits {
                body = apply_edit(&body, edit, &change.path)?;
            }
            let title = if change.title.trim().is_empty() { parts.title } else { change.title.clone() };
            let header = match &parts.header {
                Some(existing) => updated_header(existing, scope, &change.kind, &change.authority),
                None => standard_header(scope, &change.kind, &change.authority),
            };
            (header, title, body, parts.sources)
        }
        None => (standard_header(scope, &change.kind, &change.authority), change.title.clone(), without_leading_title(&change.body.replace("\r\n", "\n"), None), Vec::new()),
    };
    let header = format!("---\n{header}\n---\n\n");
    sources.extend(added.iter().cloned());
    let mut seen = BTreeSet::new();
    sources.retain(|line| seen.insert(line.clone()));
    // A source list the model wrote into the body is dropped: the program writes the list.
    let body = split_sources(body.trim()).0.trim();
    if body.is_empty() {
        return Err(Error::rejected(format!("The edited note body is empty: {}", change.path)));
    }
    let title = title.replace(['\r', '\n'], " ");
    // Only a cleanup can leave a note without sources: the notes it was made from had none.
    let sources = if sources.is_empty() { String::new() } else { format!("{SOURCES_HEADING}{}\n", sources.join("\n")) };
    Ok(format!("{header}# {}\n\n{body}\n{sources}", title.trim()))
}

/// The notes must be as the proposal saw them, except where this plan already wrote or removed
/// them: applying a plan again after an interruption is not an outside edit.
pub fn check_snapshot(current: &BTreeMap<String, String>, base: &BTreeMap<String, String>, rendered: &BTreeMap<String, String>, removed: &BTreeSet<String>) -> Result<()> {
    let names: BTreeSet<&String> = current.keys().chain(base.keys()).chain(rendered.keys()).collect();
    for name in names {
        let now = current.get(name);
        let unchanged = now == base.get(name);
        let ours = if removed.contains(name) { now.is_none() } else { rendered.contains_key(name) && now == rendered.get(name) };
        if !unchanged && !ours {
            return Err(Error::permanent(format!("The memory note changed after the proposal was created: {name}")));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE: &str = "projects/demo/bg-policy.md";
    const OTHER: &str = "projects/demo/log-only.md";
    /// A source line as approved candidates add it: the day, who said it and the words.
    const ADDED: &str = "- 2026-11-17, user: \"경고는 빼고 로그만 남기십시오.\"";

    /// A new note at `NOTE`, made from what `OTHER` says.
    fn proposal() -> Proposal {
        Proposal {
            summary: "경고 방침 변경".into(),
            changes: vec![Change {
                path: NOTE.into(),
                title: "BG 불일치 처리".into(),
                kind: "decision".into(),
                authority: "user_decision".into(),
                body: "## 현재 결정\nTP 로그만 남깁니다.".into(),
                edits: vec![],
                sources_from: vec![OTHER.into()],
            }],
            ..Proposal::default()
        }
    }

    fn edit(op: &str, find: &str, section: &str, heading: &str, text: &str) -> Edit {
        Edit { op: op.into(), find: find.into(), section: section.into(), heading: heading.into(), text: text.into() }
    }

    fn replace(find: &str, text: &str) -> Edit {
        edit("replace", find, "", "", text)
    }

    fn append(section: &str, text: &str) -> Edit {
        edit("append", "", section, "", text)
    }

    fn add_section(after: &str, heading: &str, text: &str) -> Edit {
        edit("add_section", "", after, heading, text)
    }

    /// Edits to the note at `NOTE`.
    fn editing(edits: Vec<Edit>) -> Change {
        Change { title: String::new(), body: String::new(), edits, sources_from: vec![], ..proposal().changes[0].clone() }
    }

    /// The note after `change`, with `ADDED` as the source this change brings.
    fn render(change: &Change, old: Option<&str>) -> Result<String> {
        render_note(change, "demo", old, &[ADDED.to_string()])
    }

    const OLD_NOTE: &str = "---\nqmd:\n  metadata:\n    project: \"demo\"\n    kind: \"decision\"\n    authority: \"assistant_report\"\n---\n\n# BG 불일치 처리\n\n## 결정\n\n경고 팝업을 띄웁니다. 장비 화면은 그대로 둡니다.\n\n## 남은 점\n\n로그 위치는 정하지 않았습니다.\n\n## Sources\n- 2026-10-02, user: \"팝업을 띄우십시오\"\n";

    /// A cleanup that writes and removes in `projects/demo/` and in the shared folder, and was
    /// given the text of the `editable` notes.
    fn check(proposal: &Proposal, existing: &[&str], editable: &[&str]) -> Result<()> {
        let existing: BTreeSet<String> = existing.iter().map(|s| s.to_string()).collect();
        let editable: BTreeSet<String> = editable.iter().map(|s| s.to_string()).collect();
        let folders = vec!["common/".to_string(), "projects/demo/".to_string()];
        validate(proposal, &Scope { project: "demo", existing: &existing, editable: &editable, writable: folders.clone(), removable: folders })
    }

    fn removing(path: &str) -> Proposal {
        Proposal { changes: vec![], removals: vec![Removal { path: path.into(), reason: "경고 팝업을 없애 이 노트의 대상이 없습니다.".into() }], ..proposal() }
    }

    /// A cleanup of `projects/demo/` that also takes in `projects/old/`, given every note.
    fn tidy(proposal: &Proposal, existing: &[&str]) -> Result<()> {
        let existing: BTreeSet<String> = existing.iter().map(|s| s.to_string()).collect();
        let scope = Scope { project: "demo", existing: &existing, editable: &existing, writable: vec!["projects/demo/".into()], removable: vec!["projects/demo/".into(), "projects/old/".into()] };
        validate(proposal, &scope)
    }

    #[test]
    fn a_note_ends_with_the_sources_section_and_an_edit_adds_to_it() {
        let rendered = render(&proposal().changes[0], None).unwrap();
        assert!(rendered.ends_with(&format!("\n\n## Sources\n{ADDED}\n")), "{rendered}");
        let edited = render(&editing(vec![append("남은 점", "보관 기간")]), Some(OLD_NOTE)).unwrap();
        assert!(edited.ends_with(&format!("보관 기간\n\n## Sources\n- 2026-10-02, user: \"팝업을 띄우십시오\"\n{ADDED}\n")), "{edited}");
    }

    #[test]
    fn replace_section_restates_one_section_and_leaves_the_rest() {
        let restate = |section: &str, text: &str| render(&editing(vec![edit("replace_section", "", section, "", text)]), Some(OLD_NOTE));
        let rendered = restate("결정", "TP 로그만 남깁니다.\n").unwrap();
        assert!(rendered.contains("# BG 불일치 처리\n\n## 결정\n\nTP 로그만 남깁니다.\n\n## 남은 점\n\n로그 위치는 정하지 않았습니다.\n\n## Sources"), "{rendered}");
        assert!(!rendered.contains("경고 팝업을 띄웁니다."));
        let last = restate("남은 점", "없습니다.").unwrap();
        assert!(last.contains("## 남은 점\n\n없습니다.\n\n## Sources"), "{last}");
        assert!(restate("없는 절", "x").unwrap_err().to_string().contains("was not found"));
        let mut blank = proposal();
        blank.changes[0] = editing(vec![edit("replace_section", "", "결정", "", " ")]);
        assert!(check(&blank, &[NOTE], &[NOTE]).is_err());
    }

    #[test]
    fn the_text_shown_to_the_model_leaves_out_the_source_list() {
        assert_eq!(without_sources("# A\n\nBody.\n\n## Sources\n- 2026-10-02, user: \"q\"\n"), "# A\n\nBody.\n");
        let earlier = without_sources(OLD_NOTE);
        assert!(earlier.ends_with("로그 위치는 정하지 않았습니다.\n") && earlier.starts_with("---\nqmd:"), "{earlier}");
        assert_eq!(without_sources("# A\n\nNo sources yet.\n"), "# A\n\nNo sources yet.\n");
    }

    #[test]
    fn a_new_note_renders_markdown_with_program_owned_sources() {
        let proposal = proposal();
        check(&proposal, &[OTHER], &[OTHER]).unwrap();
        let rendered = render(&proposal.changes[0], None).unwrap();
        assert!(rendered.contains("TP 로그만 남깁니다."));
        assert!(rendered.contains(ADDED));
        assert!(rendered.contains("authority: \"user_decision\""));
        assert!(rendered.starts_with("---\nqmd:\n  metadata:\n    project: \"demo\""));
    }

    #[test]
    fn paths_must_stay_in_the_project_or_common() {
        for path in [
            "../escape.md",
            "projects/other/policy.md",
            "projects/demo/../../escape.md",
            "C:/escape.md",
            "projects/demo/policy.txt",
            "projects/demo/link/policy.md",
            "projects/demo/CON.md",
            "projects/demo/.hidden.md",
            "projects\\demo\\policy.md",
            "common/",
        ] {
            let mut candidate = proposal();
            candidate.changes[0].path = path.into();
            assert!(check(&candidate, &[OTHER], &[OTHER]).is_err(), "{path}");
        }
        let mut common = proposal();
        common.changes[0].path = "common/review-practice.md".into();
        check(&common, &[OTHER], &[OTHER]).unwrap();
    }

    #[test]
    fn a_kind_or_an_authority_the_notes_do_not_use_is_rejected() {
        for (kind, authority) in [("opinion", "user_decision"), ("decision", "user_statement"), ("", "")] {
            let mut odd = proposal();
            odd.changes[0].kind = kind.into();
            odd.changes[0].authority = authority.into();
            assert!(check(&odd, &[OTHER], &[OTHER]).is_err(), "{kind} {authority}");
        }
    }

    #[test]
    fn duplicate_changes_are_rejected_and_empty_changes_are_valid() {
        let mut twice = proposal();
        let mut copy = twice.changes[0].clone();
        copy.path = "projects/demo/BG-policy.md".into();
        twice.changes.push(copy);
        assert!(check(&twice, &[OTHER], &[OTHER]).unwrap_err().to_string().contains("twice"));
        let empty = Proposal { summary: "정리할 것이 없습니다.".into(), ..Proposal::default() };
        check(&empty, &[], &[]).unwrap();
    }

    fn with_change(change: Change) -> Proposal {
        Proposal { changes: vec![change], ..proposal() }
    }

    #[test]
    fn existing_note_can_change_only_when_it_was_read_in_full() {
        let edited = with_change(editing(vec![append("결정", "로그만 남깁니다.")]));
        assert!(check(&edited, &[NOTE], &[]).unwrap_err().to_string().contains("not read in full"));
        check(&edited, &[NOTE], &[NOTE]).unwrap();
        assert!(check(&edited, &["projects/demo/BG-Policy.md"], &["projects/demo/BG-Policy.md"]).unwrap_err().to_string().contains("letter case"));
    }

    #[test]
    fn existing_note_is_changed_by_edits_and_a_new_note_is_written_whole() {
        let both_notes = [NOTE, OTHER];
        let rewritten = proposal();
        assert!(check(&rewritten, &both_notes, &both_notes).unwrap_err().to_string().contains("targeted edits"));
        let mut both = editing(vec![append("결정", "로그만 남깁니다.")]);
        both.body = "본문 전체".into();
        assert!(check(&with_change(both), &[NOTE], &[NOTE]).is_err());
        assert!(check(&with_change(editing(vec![])), &[NOTE], &[NOTE]).is_err());

        let mut new_with_edits = proposal().changes[0].clone();
        new_with_edits.edits = vec![append("결정", "로그만 남깁니다.")];
        assert!(check(&with_change(new_with_edits), &[OTHER], &[OTHER]).unwrap_err().to_string().contains("title and complete body"));
        let mut new_without_title = proposal().changes[0].clone();
        new_without_title.title = " ".into();
        assert!(check(&with_change(new_without_title), &[OTHER], &[OTHER]).unwrap_err().to_string().contains("title and complete body"));
    }

    #[test]
    fn each_edit_names_what_its_kind_needs() {
        for bad in [
            edit("rewrite", "경고", "", "", "로그"),
            replace(" ", "로그"),
            append("", "로그만 남깁니다."),
            append("결정", "  "),
            add_section("결정", "", "로그만 남깁니다."),
            add_section("결정", "변경 이유", ""),
        ] {
            assert!(check(&with_change(editing(vec![bad.clone()])), &[NOTE], &[NOTE]).unwrap_err().to_string().contains("Invalid edit instruction"), "{bad:?}");
        }
        for good in [replace("경고 팝업을 띄웁니다.", ""), add_section("", "변경 이유", "방해가 됩니다.")] {
            check(&with_change(editing(vec![good])), &[NOTE], &[NOTE]).unwrap();
        }
    }

    #[test]
    fn edits_change_only_the_places_they_point_to() {
        let mut change = editing(vec![
            replace("경고 팝업을 띄웁니다.", "경고 팝업 없이 TP 로그만 남깁니다."),
            append("남은 점", "로그 보관 기간도 정하지 않았습니다."),
            add_section("결정", "변경 이유", "장비 쓰는 사람에게 방해가 됩니다."),
        ]);
        change.authority = "user_decision".into();
        let rendered = render(&change, Some(OLD_NOTE)).unwrap();
        let expected = format!("---\nqmd:\n  metadata:\n    project: \"demo\"\n    kind: \"decision\"\n    authority: \"user_decision\"\n---\n\n# BG 불일치 처리\n\n## 결정\n\n경고 팝업 없이 TP 로그만 남깁니다. 장비 화면은 그대로 둡니다.\n\n## 변경 이유\n\n장비 쓰는 사람에게 방해가 됩니다.\n\n## 남은 점\n\n로그 위치는 정하지 않았습니다.\n\n로그 보관 기간도 정하지 않았습니다.\n\n## Sources\n- 2026-10-02, user: \"팝업을 띄우십시오\"\n{ADDED}\n");
        assert_eq!(rendered, expected);
    }

    #[test]
    fn appended_list_items_continue_the_list() {
        let old = "# 방침\n\n## 남은 점\n\n- 로그 위치\n  - 장비별 경로\n\n## Sources\n- 2026-10-02, user: \"y\"\n";
        let rendered = render(&editing(vec![append("남은 점", "- 보관 기간")]), Some(old)).unwrap();
        assert!(rendered.contains("## 남은 점\n\n- 로그 위치\n  - 장비별 경로\n- 보관 기간\n\n## Sources"), "{rendered}");
        let numbered = "# 방침\n\n## 순서\n\n1. 확인\n\n## Sources\n- 2026-10-02, user: \"y\"\n";
        let rendered = render(&editing(vec![append("순서", "2. 반영")]), Some(numbered)).unwrap();
        assert!(rendered.contains("1. 확인\n2. 반영\n\n## Sources"), "{rendered}");
        let paragraph = render(&editing(vec![append("순서", "반영은 나중에 합니다.")]), Some(numbered)).unwrap();
        assert!(paragraph.contains("1. 확인\n\n반영은 나중에 합니다.\n\n## Sources"), "{paragraph}");
    }

    #[test]
    fn new_section_without_an_anchor_goes_last_and_a_new_title_replaces_the_old_one() {
        let mut change = editing(vec![add_section("", "후속 확인", "11/5에 다시 봅니다.")]);
        change.title = "BG 불일치 처리 방침".into();
        let rendered = render(&change, Some(OLD_NOTE)).unwrap();
        assert!(rendered.contains("# BG 불일치 처리 방침\n\n## 결정"), "{rendered}");
        assert!(rendered.contains("로그 위치는 정하지 않았습니다.\n\n## 후속 확인\n\n11/5에 다시 봅니다.\n\n## Sources"), "{rendered}");
    }

    #[test]
    fn naming_a_subsection_says_that_only_second_level_sections_can_be_targeted() {
        let body = "## 결정\n\n본문입니다.\n\n### 주의할 점\n\n세부입니다.\n";
        let error = section_end(body, "주의할 점", "projects/demo/bg.md").unwrap_err().to_string();
        assert!(error.contains("is a subsection") && error.contains("\"## \""), "{error}");
        assert!(section_end(body, "없는 절", "projects/demo/bg.md").unwrap_err().to_string().contains("was not found"));
        assert!(section_end(body, "결정", "projects/demo/bg.md").is_ok());
    }

    #[test]
    fn edits_apply_in_order_against_the_text_so_far() {
        let chained = editing(vec![replace("경고 팝업을 띄웁니다.", "경고 팝업을 띄우지 않습니다."), replace("띄우지 않습니다.", "띄우지 않고 로그만 남깁니다.")]);
        let rendered = render(&chained, Some(OLD_NOTE)).unwrap();
        assert!(rendered.contains("경고 팝업을 띄우지 않고 로그만 남깁니다. 장비 화면은"), "{rendered}");
        let stale = editing(vec![replace("경고 팝업을 띄웁니다.", "로그만 남깁니다."), replace("경고 팝업을 띄웁니다.", "다시")]);
        assert!(render(&stale, Some(OLD_NOTE)).is_err());
    }

    #[test]
    fn edit_that_cannot_be_placed_exactly_once_is_rejected() {
        let twice = "# 방침\n\n## 결정\n\n로그만 남깁니다.\n\n## 남은 점\n\n로그만 남깁니다.\n\n## 남은 점\n\n보관 기간\n";
        for (old, bad) in [
            (OLD_NOTE, replace("없는 문장입니다.", "로그")),
            (twice, replace("로그만 남깁니다.", "TP 로그만 남깁니다.")),
            (OLD_NOTE, replace("# BG 불일치 처리", "# 새 제목")),
            (OLD_NOTE, replace("팝업을 띄우십시오", "로그")),
            (OLD_NOTE, append("없는 절", "로그")),
            (twice, append("남은 점", "로그")),
            (OLD_NOTE, add_section("결정", "남은 점", "중복 제목")),
            (OLD_NOTE, add_section("없는 절", "변경 이유", "로그")),
        ] {
            let error = render(&editing(vec![bad.clone()]), Some(old)).unwrap_err();
            assert_eq!(error.failure, crate::error::Failure::Rejected, "{bad:?}");
        }
    }

    #[test]
    fn deleting_text_and_editing_a_crlf_note_keep_earlier_sources() {
        let old = OLD_NOTE.replace('\n', "\r\n");
        let rendered = render(&editing(vec![replace(" 장비 화면은 그대로 둡니다.", "")]), Some(&old)).unwrap();
        assert!(rendered.contains("## 결정\n\n경고 팝업을 띄웁니다.\n\n## 남은 점"), "{rendered}");
        assert!(rendered.contains(&format!("- 2026-10-02, user: \"팝업을 띄우십시오\"\n{ADDED}\n")), "{rendered}");
        // The same source brought again, as when an interrupted write is repeated, is listed once.
        let twice = render(&editing(vec![replace("경고 팝업을 띄웁니다.", "경고 팝업을 띄웁니다!")]), Some(&rendered)).unwrap();
        assert_eq!(twice.matches(ADDED).count(), 1);
    }

    #[test]
    fn a_title_repeated_at_the_top_of_the_body_is_dropped() {
        let mut new_note = proposal().changes[0].clone();
        new_note.body = "# BG 불일치 처리\n\n## 현재 결정\nTP 로그만 남깁니다.".into();
        let rendered = render(&new_note, None).unwrap();
        assert_eq!(rendered.matches("# BG 불일치 처리\n").count(), 1, "{rendered}");
        assert!(rendered.contains("# BG 불일치 처리\n\n## 현재 결정\nTP 로그만 남깁니다.\n\n## Sources"), "{rendered}");

        let doubled = OLD_NOTE.replace("# BG 불일치 처리\n\n", "# BG 불일치 처리\n\n# BG 불일치 처리\n\n");
        let edited = render(&editing(vec![append("남은 점", "보관 기간")]), Some(&doubled)).unwrap();
        assert_eq!(edited.matches("# BG 불일치 처리\n").count(), 1, "{edited}");
        assert!(edited.contains("# BG 불일치 처리\n\n## 결정\n"), "{edited}");
    }

    #[test]
    fn editing_keeps_the_existing_header_and_updates_only_the_evidence_kind() {
        let october = OLD_NOTE.replace(
            "    project: \"demo\"\n    kind: \"decision\"\n    authority: \"assistant_report\"\n",
            "    project: \"demo\"\n    period: \"2026-10\"\n    kind: \"documentation_decision\"\n    status: \"historical\"\n    authority: \"user_statement\"\n",
        );
        let mut change = editing(vec![append("남은 점", "보관 기간")]);
        change.kind = "issue".into();
        change.authority = "mixed_evidence".into();
        let rendered = render(&change, Some(&october)).unwrap();
        assert!(
            rendered.starts_with("---\nqmd:\n  metadata:\n    project: \"demo\"\n    period: \"2026-10\"\n    kind: \"documentation_decision\"\n    status: \"historical\"\n    authority: \"mixed_evidence\"\n---\n\n# BG 불일치 처리\n"),
            "{rendered}"
        );

        let bare = "# 방침\n\n## 결정\n\n로그만 남깁니다.\n";
        let rendered = render(&editing(vec![append("결정", "보관 기간")]), Some(bare)).unwrap();
        assert!(rendered.starts_with("---\nqmd:\n  metadata:\n    project: \"demo\"\n    kind: \"decision\"\n    authority: \"user_decision\"\n---\n\n# 방침\n"), "{rendered}");

        let partial = "---\nqmd:\n  metadata:\n    project: \"demo\"\n---\n\n# 방침\n\n## 결정\n\n로그만 남깁니다.\n";
        let rendered = render(&editing(vec![append("결정", "보관 기간")]), Some(partial)).unwrap();
        assert!(rendered.starts_with("---\nqmd:\n  metadata:\n    project: \"demo\"\n    kind: \"decision\"\n    authority: \"user_decision\"\n---\n\n# 방침\n"), "{rendered}");
    }

    #[test]
    fn model_written_sources_section_is_replaced_by_the_program() {
        let mut change = proposal().changes[0].clone();
        change.body = "본문\n## Sources\n- 모델이 만든 근거".into();
        let rendered = render(&change, None).unwrap();
        assert!(!rendered.contains("모델이 만든 근거"));
        assert!(rendered.contains(&format!("본문\n\n## Sources\n{ADDED}\n")), "{rendered}");
        let appended = render(&editing(vec![append("남은 점", "보관 기간\n## Sources\n- 모델이 만든 근거")]), Some(OLD_NOTE)).unwrap();
        assert!(!appended.contains("모델이 만든 근거"), "{appended}");
    }

    #[test]
    fn a_note_is_removed_only_with_a_reason_and_when_it_was_read_in_full() {
        check(&removing(NOTE), &[NOTE], &[NOTE]).unwrap();
        assert!(check(&removing(NOTE), &[NOTE], &[]).unwrap_err().to_string().contains("not read in full"));
        assert!(check(&removing(NOTE), &[], &[]).unwrap_err().to_string().contains("does not exist"));
        assert!(check(&removing("projects/other/bg-policy.md"), &["projects/other/bg-policy.md"], &["projects/other/bg-policy.md"]).is_err());
        let mut silent = removing(NOTE);
        silent.removals[0].reason = " ".into();
        assert!(check(&silent, &[NOTE], &[NOTE]).unwrap_err().to_string().contains("requires a reason"));
    }

    #[test]
    fn a_note_cannot_be_changed_and_removed_in_one_proposal() {
        let mut both = removing(NOTE);
        both.changes = vec![editing(vec![append("결정", "로그만 남깁니다.")])];
        assert!(check(&both, &[NOTE], &[NOTE]).unwrap_err().to_string().contains("twice"));
        let mut merged = removing(OTHER);
        merged.changes = vec![Change { sources_from: vec![OTHER.into()], ..editing(vec![append("결정", "로그만 남깁니다.")]) }];
        check(&merged, &[NOTE, OTHER], &[NOTE, OTHER]).unwrap();
    }

    #[test]
    fn sources_can_be_taken_only_from_another_note_whose_text_was_provided() {
        let taking = |from: &str| with_change(Change { sources_from: vec![from.into()], ..editing(vec![append("결정", "로그만 남깁니다.")]) });
        check(&taking(OTHER), &[NOTE, OTHER], &[NOTE, OTHER]).unwrap();
        assert!(check(&taking(OTHER), &[NOTE, OTHER], &[NOTE]).is_err());
        assert!(check(&taking("projects/demo/missing.md"), &[NOTE], &[NOTE]).is_err());
        assert!(check(&taking(NOTE), &[NOTE], &[NOTE]).is_err());
    }

    #[test]
    fn a_note_takes_over_the_sources_of_the_notes_it_absorbed() {
        let carried = source_lines("# 옛 노트\n\n본문\n\n## Sources\n- 2026-09-01, agent: \"옛 근거\"\n- 2026-10-02, user: \"팝업을 띄우십시오\"\n");
        assert_eq!(carried.len(), 2);
        let rendered = render_note(&editing(vec![append("남은 점", "보관 기간")]), "demo", Some(OLD_NOTE), &carried).unwrap();
        assert!(rendered.ends_with("## Sources\n- 2026-10-02, user: \"팝업을 띄우십시오\"\n- 2026-09-01, agent: \"옛 근거\"\n"), "{rendered}");
        assert!(source_lines("# 근거 없는 노트\n\n본문\n").is_empty());
    }

    #[test]
    fn only_the_chosen_source_lines_follow_the_content_and_a_line_that_does_not_exist_is_rejected() {
        let ticket = "# 이슈 노트\n\n본문\n\n## Sources\n- 2026-11-01, user: \"끝난 것부터 말해\"\n- 2026-11-01, agent: \"4GB 미만에서는 실행하지 않았습니다\"\n";
        let text_of = |path: &str| (path == OTHER).then_some(ticket);
        let taking = |lines: &[&str]| Change { sources_from: vec![SourceRef::Lines { path: OTHER.into(), lines: lines.iter().map(|line| line.to_string()).collect() }], ..editing(vec![append("남은 점", "4GB 미만 미확인")]) };
        let agent = "- 2026-11-01, agent: \"4GB 미만에서는 실행하지 않았습니다\"";
        // A line is named as it reads, with or without the dash that starts it.
        assert_eq!(carried_sources(&taking(&[agent]), text_of).unwrap(), vec![agent.to_string()]);
        assert_eq!(carried_sources(&taking(&[" 2026-11-01, agent: \"4GB 미만에서는 실행하지 않았습니다\" "]), text_of).unwrap(), vec![agent.to_string()]);
        assert_eq!(carried_sources(&taking(&[]), text_of).unwrap().len(), 2);
        assert_eq!(carried_sources(&Change { sources_from: vec![OTHER.into()], ..taking(&[]) }, text_of).unwrap().len(), 2);
        for missing in [&["- 2026-11-01, agent: \"4GB에서는 실행하지 않았습니다\""][..], &["2"], &[agent, "- 2026-11-02, user: \"없는 말\""]] {
            let error = carried_sources(&taking(missing), text_of).unwrap_err();
            assert!(error.failure == crate::error::Failure::Rejected && error.to_string().contains("source line"), "{error}");
        }

        // Both forms of an answer read: a path alone, and a path with the lines it takes.
        let read: Change = serde_json::from_str(r#"{"path": "projects/demo/a.md", "kind": "decision", "authority": "mixed_evidence", "sources_from": ["projects/demo/b.md", {"path": "projects/demo/c.md", "lines": ["- 2026-11-01, user: \"끝난 것부터 말해\""]}]}"#).unwrap();
        assert_eq!(read.sources_from, vec![SourceRef::Whole("projects/demo/b.md".into()), SourceRef::Lines { path: "projects/demo/c.md".into(), lines: vec!["- 2026-11-01, user: \"끝난 것부터 말해\"".into()] }]);
    }

    #[test]
    fn a_cleanup_rests_on_the_notes_and_a_new_note_names_the_notes_it_came_from() {
        let split = |sources_from: Vec<SourceRef>| with_change(Change { path: "projects/demo/log-retention.md".into(), sources_from, ..proposal().changes[0].clone() });
        tidy(&split(vec![NOTE.into()]), &[NOTE]).unwrap();
        assert!(tidy(&split(vec![]), &[NOTE]).unwrap_err().to_string().contains("sources_from"));
        tidy(&with_change(editing(vec![append("결정", "로그만 남깁니다.")])), &[NOTE]).unwrap();
        tidy(&removing(NOTE), &[NOTE]).unwrap();

        let rendered = render_note(&split(vec![NOTE.into()]).changes[0], "demo", None, &source_lines(OLD_NOTE)).unwrap();
        assert!(rendered.ends_with("TP 로그만 남깁니다.\n\n## Sources\n- 2026-10-02, user: \"팝업을 띄우십시오\"\n"), "{rendered}");
        // The notes it was made from had no sources, so neither has the new one.
        let bare = render_note(&split(vec![NOTE.into()]).changes[0], "demo", None, &[]).unwrap();
        assert!(bare.ends_with("TP 로그만 남깁니다.\n") && !bare.contains("## Sources"), "{bare}");
    }

    #[test]
    fn a_cleanup_writes_only_in_its_folder_and_removes_only_there_and_in_the_folders_it_takes_in() {
        let old = "projects/old/bg-policy.md";
        let common = "common/review-practice.md";
        tidy(&removing(old), &[NOTE, old]).unwrap();
        assert!(tidy(&removing(common), &[NOTE, common]).is_err());
        assert!(tidy(&removing("projects/else/a.md"), &["projects/else/a.md"]).is_err());
        let writing = |path: &str| with_change(Change { path: path.into(), sources_from: vec![NOTE.into()], ..proposal().changes[0].clone() });
        tidy(&writing("projects/demo/new-topic.md"), &[NOTE]).unwrap();
        assert!(tidy(&writing("projects/old/new-topic.md"), &[NOTE]).is_err());
        assert!(tidy(&writing("common/new-topic.md"), &[NOTE]).is_err());
        let mut edit_old = editing(vec![append("결정", "로그만 남깁니다.")]);
        edit_old.path = old.into();
        assert!(tidy(&with_change(edit_old), &[NOTE, old]).is_err());
    }

    fn moving(from: &str, to: &str) -> Proposal {
        Proposal { changes: vec![], moves: vec![Move { from: from.into(), to: to.into() }], ..proposal() }
    }

    #[test]
    fn a_note_moves_only_to_a_free_path_in_the_folder_being_cleaned() {
        let old = "projects/old/bg-policy.md";
        tidy(&moving(old, "projects/demo/bg-mismatch.md"), &[NOTE, old]).unwrap();
        tidy(&moving(NOTE, "projects/demo/bg-mismatch.md"), &[NOTE]).unwrap();
        assert!(check(&moving(NOTE, "projects/demo/bg-mismatch.md"), &[NOTE], &[]).unwrap_err().to_string().contains("not read in full"));
        assert!(tidy(&moving(old, NOTE), &[NOTE, old]).unwrap_err().to_string().contains("already exists"));
        assert!(tidy(&moving(old, "projects/demo/BG-Policy.md"), &[NOTE, old]).is_err());
        assert!(tidy(&moving(old, "projects/old/renamed.md"), &[NOTE, old]).is_err());
        assert!(tidy(&moving("projects/old/missing.md", "projects/demo/a.md"), &[NOTE]).is_err());
        assert!(tidy(&moving("common/a.md", "projects/demo/a.md"), &[NOTE, "common/a.md"]).is_err());
        let mut twice = moving(old, "projects/demo/bg-mismatch.md");
        twice.removals = removing(old).removals;
        assert!(tidy(&twice, &[NOTE, old]).unwrap_err().to_string().contains("twice"));
    }

    #[test]
    fn a_moved_note_keeps_its_text_and_takes_the_project_of_its_new_folder() {
        let moved = moved_note(OLD_NOTE, "projects/lore/bg-policy.md");
        assert_eq!(moved, OLD_NOTE.replace("    project: \"demo\"\n", "    project: \"lore\"\n"));
        // Only the front matter names the project: the same words in the body stay.
        let shared = "---\nqmd:\n  metadata:\n    project: \"demo\"\n---\n\n# 방침\n\n본문 project: \"demo\"\n\n## Sources\n- 2026-10-02, user: \"근거\"\n";
        assert_eq!(moved_note(shared, "common/policy.md"), shared.replacen("\"demo\"", "\"common\"", 1));
        assert_eq!(moved_note("# 머리말 없는 노트\n\n본문\n", "projects/lore/a.md"), "# 머리말 없는 노트\n\n본문\n");
    }

    #[test]
    fn snapshot_rejects_outside_edits_and_accepts_own_partial_write() {
        let map = |items: &[(&str, &str)]| items.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<BTreeMap<_, _>>();
        let base = map(&[("projects/demo/a.md", "old"), ("common/b.md", "context")]);
        let rendered = map(&[("projects/demo/a.md", "new")]);
        let none = BTreeSet::new();
        check_snapshot(&base, &base, &rendered, &none).unwrap();
        check_snapshot(&map(&[("projects/demo/a.md", "new"), ("common/b.md", "context")]), &base, &rendered, &none).unwrap();
        for current in [
            map(&[("projects/demo/a.md", "old")]),
            map(&[("projects/demo/a.md", "edited"), ("common/b.md", "context")]),
            map(&[("projects/demo/a.md", "old"), ("common/b.md", "context"), ("common/c.md", "added")]),
        ] {
            assert!(check_snapshot(&current, &base, &rendered, &none).is_err());
        }
    }

    #[test]
    fn snapshot_accepts_a_note_this_plan_already_removed_but_not_one_removed_by_someone_else() {
        let map = |items: &[(&str, &str)]| items.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<BTreeMap<_, _>>();
        let base = map(&[("projects/demo/a.md", "old"), ("projects/demo/gone.md", "obsolete")]);
        let rendered = map(&[("projects/demo/a.md", "new")]);
        let removed = BTreeSet::from(["projects/demo/gone.md".to_string()]);
        check_snapshot(&base, &base, &rendered, &removed).unwrap();
        check_snapshot(&map(&[("projects/demo/a.md", "new")]), &base, &rendered, &removed).unwrap();
        assert!(check_snapshot(&map(&[("projects/demo/a.md", "old"), ("projects/demo/gone.md", "edited")]), &base, &rendered, &removed).is_err());
        assert!(check_snapshot(&map(&[("projects/demo/gone.md", "obsolete")]), &base, &rendered, &removed).is_err());
    }
}
