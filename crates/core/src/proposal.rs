use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::transcript::{Origin, Turn};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    #[serde(rename = "ref")]
    pub reference: String,
    pub quote: String,
}

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    pub evidence: Vec<Evidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    pub summary: String,
    pub changes: Vec<Change>,
    #[serde(default)]
    pub needs_notes: Vec<String>,
}

pub struct Scope<'a> {
    pub project: &'a str,
    pub turns: &'a [Turn],
    pub existing: &'a BTreeSet<String>,
    pub editable: &'a BTreeSet<String>,
}

const KINDS: [&str; 3] = ["decision", "lesson", "issue"];
const AUTHORITIES: [&str; 3] = ["user_decision", "assistant_report", "mixed_evidence"];
pub const EDIT_OPS: [&str; 4] = ["replace", "append", "add_section", "replace_section"];
const RESERVED: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7",
    "lpt8", "lpt9",
];
pub const SOURCES_HEADING: &str = "\n## Sources\n";
// Notes written before the heading became English end with this one (Korean for "Sources").
// It is still read, and a note is given the English heading the next time it is changed.
const EARLIER_SOURCES_HEADING: &str = "\n## \u{c6d0}\u{bb38} \u{adfc}\u{ac70}\n";

fn split_sources(text: &str) -> (&str, Option<&str>) {
    let found = [SOURCES_HEADING, EARLIER_SOURCES_HEADING].into_iter().filter_map(|heading| text.find(heading).map(|at| (at, heading.len()))).min();
    match found {
        Some((at, length)) => (&text[..at], Some(&text[at + length..])),
        None => (text, None),
    }
}

/// A note as the recording model is shown it. The program owns the source list, and the model
/// cannot edit it, so the list is left out rather than spent on the model's input.
pub fn without_sources(note: &str) -> String {
    let text = note.replace("\r\n", "\n");
    match split_sources(&text) {
        (main, Some(_)) => format!("{}\n", main.trim_end()),
        (main, None) => main.to_string(),
    }
}

pub fn validate_note_path(path: &str, project: &str) -> Result<()> {
    let fail = || Error::rejected(format!("Invalid memory path: {path}"));
    if path.contains('\\') {
        return Err(fail());
    }
    let parts: Vec<&str> = path.split('/').collect();
    let filename = match parts.as_slice() {
        ["common", file] => *file,
        ["projects", owner, file] if *owner == project => *file,
        _ => return Err(fail()),
    };
    let stem = filename.strip_suffix(".md").ok_or_else(fail)?;
    let mut chars = stem.chars();
    let well_formed = matches!(chars.next(), Some(first) if first.is_ascii_alphanumeric()) && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !well_formed || RESERVED.contains(&stem.to_ascii_lowercase().as_str()) {
        return Err(fail());
    }
    Ok(())
}

pub fn validate(proposal: &Proposal, scope: &Scope) -> Result<()> {
    let available: HashMap<&str, &Turn> = scope.turns.iter().map(|turn| (turn.reference.as_str(), turn)).collect();
    let mut paths = BTreeSet::new();
    for change in &proposal.changes {
        validate_note_path(&change.path, scope.project)?;
        if !paths.insert(change.path.to_ascii_lowercase()) {
            return Err(Error::rejected("A note cannot be changed twice in one proposal."));
        }
        if let Some(existing) = scope.existing.iter().find(|existing| existing.eq_ignore_ascii_case(&change.path)) {
            if *existing != change.path {
                return Err(Error::rejected(format!("The path differs from an existing note only by letter case: {}", change.path)));
            }
            if !scope.editable.contains(existing) {
                return Err(Error::rejected(format!("Cannot edit an existing note whose body was not provided: {}", change.path)));
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
        if change.evidence.is_empty() {
            return Err(Error::rejected("Source evidence is required."));
        }
        let mut origins = BTreeSet::new();
        for citation in &change.evidence {
            let turn = available.get(citation.reference.as_str());
            match turn {
                Some(turn) if !citation.quote.trim().is_empty() && turn.text.contains(&citation.quote) => {
                    origins.insert(turn.origin as u8);
                }
                _ => return Err(Error::rejected("The evidence location or quotation does not match the provided source.")),
            }
        }
        if change.authority == "user_decision" && !origins.contains(&(Origin::User as u8)) {
            return Err(Error::rejected("A user decision requires a verified user statement."));
        }
    }
    Ok(())
}

fn validate_edit(edit: &Edit, path: &str) -> Result<()> {
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

pub fn render_note(change: &Change, project: &str, link: &dyn Fn(&Evidence) -> String, old: Option<&str>) -> Result<String> {
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
    for evidence in &change.evidence {
        sources.push(format!("- [{}]({}) — {}", evidence.reference, link(evidence), evidence.quote.replace(['\r', '\n'], " ")));
    }
    let mut seen = BTreeSet::new();
    sources.retain(|line| seen.insert(line.clone()));
    let body = body.trim();
    let body = split_sources(body).0.trim();
    if body.is_empty() {
        return Err(Error::rejected(format!("The edited note body is empty: {}", change.path)));
    }
    let title = title.replace(['\r', '\n'], " ");
    Ok(format!("{header}# {}\n\n{body}\n{SOURCES_HEADING}{}\n", title.trim(), sources.join("\n")))
}

pub fn check_snapshot(current: &BTreeMap<String, String>, base: &BTreeMap<String, String>, rendered: &BTreeMap<String, String>) -> Result<()> {
    let names: BTreeSet<&String> = current.keys().chain(base.keys()).chain(rendered.keys()).collect();
    for name in names {
        let now = current.get(name);
        let unchanged = now == base.get(name);
        let ours = rendered.contains_key(name) && now == rendered.get(name);
        if !unchanged && !ours {
            return Err(Error::permanent(format!("The memory note changed after the proposal was created: {name}")));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turns() -> Vec<Turn> {
        vec![Turn {
            reference: "L3".into(),
            line: 3,
            role: "user".into(),
            origin: Origin::User,
            timestamp: String::new(),
            text: "경고는 빼고 로그만 남기십시오.".into(),
        }]
    }

    fn proposal() -> Proposal {
        Proposal {
            summary: "경고 방침 변경".into(),
            changes: vec![Change {
                path: "projects/demo/bg-policy.md".into(),
                title: "BG 불일치 처리".into(),
                kind: "decision".into(),
                authority: "user_decision".into(),
                body: "## 현재 결정\nTP 로그만 남깁니다.".into(),
                edits: vec![],
                evidence: vec![Evidence { reference: "L3".into(), quote: "경고는 빼고 로그만 남기십시오.".into() }],
            }],
            needs_notes: vec![],
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

    fn editing(edits: Vec<Edit>) -> Change {
        Change { title: String::new(), body: String::new(), edits, ..proposal().changes[0].clone() }
    }

    const OLD_NOTE: &str = "---\nqmd:\n  metadata:\n    project: \"demo\"\n    kind: \"decision\"\n    authority: \"assistant_report\"\n---\n\n# BG 불일치 처리\n\n## 결정\n\n경고 팝업을 띄웁니다. 장비 화면은 그대로 둡니다.\n\n## 남은 점\n\n로그 위치는 정하지 않았습니다.\n\n## 원문 근거\n- [L1](<D:/k/sources/claude/s.md:1>) — 팝업을 띄우십시오\n";

    fn check(proposal: &Proposal, turns: &[Turn], existing: &[&str], editable: &[&str]) -> Result<()> {
        let existing: BTreeSet<String> = existing.iter().map(|s| s.to_string()).collect();
        let editable: BTreeSet<String> = editable.iter().map(|s| s.to_string()).collect();
        validate(proposal, &Scope { project: "demo", turns, existing: &existing, editable: &editable })
    }

    fn link(evidence: &Evidence) -> String {
        format!("<D:/k/sources/claude/s.md:{}>", &evidence.reference[1..])
    }

    #[test]
    fn notes_end_with_an_english_sources_section_and_older_notes_are_converted_when_edited() {
        let rendered = render_note(&proposal().changes[0], "demo", &link, None).unwrap();
        assert!(rendered.ends_with("\n\n## Sources\n- [L3](<D:/k/sources/claude/s.md:3>) — 경고는 빼고 로그만 남기십시오.\n"), "{rendered}");
        let edited = render_note(&editing(vec![append("남은 점", "보관 기간")]), "demo", &link, Some(OLD_NOTE)).unwrap();
        assert!(!edited.contains("원문 근거"), "{edited}");
        assert!(edited.ends_with("보관 기간\n\n## Sources\n- [L1](<D:/k/sources/claude/s.md:1>) — 팝업을 띄우십시오\n- [L3](<D:/k/sources/claude/s.md:3>) — 경고는 빼고 로그만 남기십시오.\n"), "{edited}");
    }

    #[test]
    fn replace_section_restates_one_section_and_leaves_the_rest() {
        let restate = |section: &str, text: &str| render_note(&editing(vec![edit("replace_section", "", section, "", text)]), "demo", &link, Some(OLD_NOTE));
        let rendered = restate("결정", "TP 로그만 남깁니다.\n").unwrap();
        assert!(rendered.contains("# BG 불일치 처리\n\n## 결정\n\nTP 로그만 남깁니다.\n\n## 남은 점\n\n로그 위치는 정하지 않았습니다.\n\n## Sources"), "{rendered}");
        assert!(!rendered.contains("경고 팝업을 띄웁니다."));
        let last = restate("남은 점", "없습니다.").unwrap();
        assert!(last.contains("## 남은 점\n\n없습니다.\n\n## Sources"), "{last}");
        assert!(restate("없는 절", "x").unwrap_err().to_string().contains("was not found"));
        let mut blank = proposal();
        blank.changes[0] = editing(vec![edit("replace_section", "", "결정", "", " ")]);
        assert!(check(&blank, &turns(), &["projects/demo/bg-policy.md"], &["projects/demo/bg-policy.md"]).is_err());
    }

    #[test]
    fn the_text_shown_to_the_model_leaves_out_the_source_list() {
        assert_eq!(without_sources("# A\n\nBody.\n\n## Sources\n- [L1](<x>) — q\n"), "# A\n\nBody.\n");
        let earlier = without_sources(OLD_NOTE);
        assert!(earlier.ends_with("로그 위치는 정하지 않았습니다.\n") && earlier.starts_with("---\nqmd:"), "{earlier}");
        assert_eq!(without_sources("# A\n\nNo sources yet.\n"), "# A\n\nNo sources yet.\n");
    }

    #[test]
    fn supported_decision_renders_markdown_with_program_owned_sources() {
        let proposal = proposal();
        check(&proposal, &turns(), &[], &[]).unwrap();
        let rendered = render_note(&proposal.changes[0], "demo", &link, None).unwrap();
        assert!(rendered.contains("TP 로그만 남깁니다."));
        assert!(rendered.contains("- [L3](<D:/k/sources/claude/s.md:3>) — 경고는 빼고 로그만 남기십시오."));
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
            assert!(check(&candidate, &turns(), &[], &[]).is_err(), "{path}");
        }
        let mut common = proposal();
        common.changes[0].path = "common/review-practice.md".into();
        check(&common, &turns(), &[], &[]).unwrap();
    }

    #[test]
    fn unknown_or_fabricated_evidence_is_rejected() {
        for evidence in [
            vec![Evidence { reference: "L900".into(), quote: "경고는 빼고".into() }],
            vec![Evidence { reference: "L3".into(), quote: "사용자가 경고를 원함".into() }],
            vec![Evidence { reference: "L3".into(), quote: "  ".into() }],
            vec![],
        ] {
            let mut candidate = proposal();
            candidate.changes[0].evidence = evidence;
            assert!(check(&candidate, &turns(), &[], &[]).is_err());
        }
    }

    #[test]
    fn agent_text_cannot_back_a_user_decision() {
        for origin in [Origin::Assistant, Origin::Unverified, Origin::Context] {
            let mut agent = turns();
            agent[0].origin = origin;
            assert!(check(&proposal(), &agent, &[], &[]).is_err());
            let mut report = proposal();
            report.changes[0].authority = "assistant_report".into();
            check(&report, &agent, &[], &[]).unwrap();
        }
    }

    #[test]
    fn duplicate_changes_are_rejected_and_empty_changes_are_valid() {
        let mut twice = proposal();
        let mut copy = twice.changes[0].clone();
        copy.path = "projects/demo/BG-policy.md".into();
        twice.changes.push(copy);
        assert!(check(&twice, &turns(), &[], &[]).is_err());
        let empty = Proposal { summary: "기록 대상 없음".into(), changes: vec![], needs_notes: vec![] };
        check(&empty, &turns(), &[], &[]).unwrap();
    }

    fn with_change(change: Change) -> Proposal {
        Proposal { changes: vec![change], ..proposal() }
    }

    #[test]
    fn existing_note_can_change_only_when_its_body_was_provided() {
        let path = "projects/demo/bg-policy.md";
        let edited = with_change(editing(vec![append("결정", "로그만 남깁니다.")]));
        assert!(check(&edited, &turns(), &[path], &[]).is_err());
        check(&edited, &turns(), &[path], &[path]).unwrap();
        assert!(check(&edited, &turns(), &["projects/demo/BG-Policy.md"], &["projects/demo/BG-Policy.md"]).is_err());
    }

    #[test]
    fn existing_note_is_changed_by_edits_and_a_new_note_is_written_whole() {
        let path = "projects/demo/bg-policy.md";
        let rewritten = proposal();
        assert_eq!(check(&rewritten, &turns(), &[path], &[path]).unwrap_err().failure, crate::error::Failure::Rejected);
        let mut both = editing(vec![append("결정", "로그만 남깁니다.")]);
        both.body = "본문 전체".into();
        assert!(check(&with_change(both), &turns(), &[path], &[path]).is_err());
        assert!(check(&with_change(editing(vec![])), &turns(), &[path], &[path]).is_err());

        let mut new_with_edits = proposal().changes[0].clone();
        new_with_edits.edits = vec![append("결정", "로그만 남깁니다.")];
        assert!(check(&with_change(new_with_edits), &turns(), &[], &[]).is_err());
        let mut new_without_title = proposal().changes[0].clone();
        new_without_title.title = " ".into();
        assert!(check(&with_change(new_without_title), &turns(), &[], &[]).is_err());
    }

    #[test]
    fn each_edit_names_what_its_kind_needs() {
        let path = "projects/demo/bg-policy.md";
        for bad in [
            edit("rewrite", "경고", "", "", "로그"),
            replace(" ", "로그"),
            append("", "로그만 남깁니다."),
            append("결정", "  "),
            add_section("결정", "", "로그만 남깁니다."),
            add_section("결정", "변경 이유", ""),
        ] {
            assert!(check(&with_change(editing(vec![bad.clone()])), &turns(), &[path], &[path]).is_err(), "{bad:?}");
        }
        for good in [replace("경고 팝업을 띄웁니다.", ""), add_section("", "변경 이유", "방해가 됩니다.")] {
            check(&with_change(editing(vec![good])), &turns(), &[path], &[path]).unwrap();
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
        let rendered = render_note(&change, "demo", &link, Some(OLD_NOTE)).unwrap();
        let expected = "---\nqmd:\n  metadata:\n    project: \"demo\"\n    kind: \"decision\"\n    authority: \"user_decision\"\n---\n\n# BG 불일치 처리\n\n## 결정\n\n경고 팝업 없이 TP 로그만 남깁니다. 장비 화면은 그대로 둡니다.\n\n## 변경 이유\n\n장비 쓰는 사람에게 방해가 됩니다.\n\n## 남은 점\n\n로그 위치는 정하지 않았습니다.\n\n로그 보관 기간도 정하지 않았습니다.\n\n## Sources\n- [L1](<D:/k/sources/claude/s.md:1>) — 팝업을 띄우십시오\n- [L3](<D:/k/sources/claude/s.md:3>) — 경고는 빼고 로그만 남기십시오.\n";
        assert_eq!(rendered, expected);
    }

    #[test]
    fn appended_list_items_continue_the_list() {
        let old = "# 방침\n\n## 남은 점\n\n- 로그 위치\n  - 장비별 경로\n\n## 원문 근거\n- [L1](<x>) — y\n";
        let rendered = render_note(&editing(vec![append("남은 점", "- 보관 기간")]), "demo", &link, Some(old)).unwrap();
        assert!(rendered.contains("## 남은 점\n\n- 로그 위치\n  - 장비별 경로\n- 보관 기간\n\n## Sources"), "{rendered}");
        let numbered = "# 방침\n\n## 순서\n\n1. 확인\n\n## 원문 근거\n- [L1](<x>) — y\n";
        let rendered = render_note(&editing(vec![append("순서", "2. 반영")]), "demo", &link, Some(numbered)).unwrap();
        assert!(rendered.contains("1. 확인\n2. 반영\n\n## Sources"), "{rendered}");
        let paragraph = render_note(&editing(vec![append("순서", "반영은 나중에 합니다.")]), "demo", &link, Some(numbered)).unwrap();
        assert!(paragraph.contains("1. 확인\n\n반영은 나중에 합니다.\n\n## Sources"), "{paragraph}");
    }

    #[test]
    fn new_section_without_an_anchor_goes_last_and_a_new_title_replaces_the_old_one() {
        let mut change = editing(vec![add_section("", "후속 확인", "11/5에 다시 봅니다.")]);
        change.title = "BG 불일치 처리 방침".into();
        let rendered = render_note(&change, "demo", &link, Some(OLD_NOTE)).unwrap();
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
        let rendered = render_note(&chained, "demo", &link, Some(OLD_NOTE)).unwrap();
        assert!(rendered.contains("경고 팝업을 띄우지 않고 로그만 남깁니다. 장비 화면은"), "{rendered}");
        let stale = editing(vec![replace("경고 팝업을 띄웁니다.", "로그만 남깁니다."), replace("경고 팝업을 띄웁니다.", "다시")]);
        assert!(render_note(&stale, "demo", &link, Some(OLD_NOTE)).is_err());
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
            let error = render_note(&editing(vec![bad.clone()]), "demo", &link, Some(old)).unwrap_err();
            assert_eq!(error.failure, crate::error::Failure::Rejected, "{bad:?}");
        }
    }

    #[test]
    fn deleting_text_and_editing_a_crlf_note_keep_earlier_sources() {
        let old = OLD_NOTE.replace('\n', "\r\n");
        let rendered = render_note(&editing(vec![replace(" 장비 화면은 그대로 둡니다.", "")]), "demo", &link, Some(&old)).unwrap();
        assert!(rendered.contains("## 결정\n\n경고 팝업을 띄웁니다.\n\n## 남은 점"), "{rendered}");
        assert!(rendered.contains("- [L1](<D:/k/sources/claude/s.md:1>) — 팝업을 띄우십시오\n- [L3]"));
        let twice = render_note(&editing(vec![replace("경고 팝업을 띄웁니다.", "경고 팝업을 띄웁니다!")]), "demo", &link, Some(&rendered)).unwrap();
        assert_eq!(twice.matches("- [L3]").count(), 1);
    }

    #[test]
    fn a_title_repeated_at_the_top_of_the_body_is_dropped() {
        let mut new_note = proposal().changes[0].clone();
        new_note.body = "# BG 불일치 처리\n\n## 현재 결정\nTP 로그만 남깁니다.".into();
        let rendered = render_note(&new_note, "demo", &link, None).unwrap();
        assert_eq!(rendered.matches("# BG 불일치 처리\n").count(), 1, "{rendered}");
        assert!(rendered.contains("# BG 불일치 처리\n\n## 현재 결정\nTP 로그만 남깁니다.\n\n## Sources"), "{rendered}");

        let doubled = OLD_NOTE.replace("# BG 불일치 처리\n\n", "# BG 불일치 처리\n\n# BG 불일치 처리\n\n");
        let edited = render_note(&editing(vec![append("남은 점", "보관 기간")]), "demo", &link, Some(&doubled)).unwrap();
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
        let rendered = render_note(&change, "demo", &link, Some(&october)).unwrap();
        assert!(
            rendered.starts_with("---\nqmd:\n  metadata:\n    project: \"demo\"\n    period: \"2026-10\"\n    kind: \"documentation_decision\"\n    status: \"historical\"\n    authority: \"mixed_evidence\"\n---\n\n# BG 불일치 처리\n"),
            "{rendered}"
        );

        let bare = "# 방침\n\n## 결정\n\n로그만 남깁니다.\n";
        let rendered = render_note(&editing(vec![append("결정", "보관 기간")]), "demo", &link, Some(bare)).unwrap();
        assert!(rendered.starts_with("---\nqmd:\n  metadata:\n    project: \"demo\"\n    kind: \"decision\"\n    authority: \"user_decision\"\n---\n\n# 방침\n"), "{rendered}");

        let partial = "---\nqmd:\n  metadata:\n    project: \"demo\"\n---\n\n# 방침\n\n## 결정\n\n로그만 남깁니다.\n";
        let rendered = render_note(&editing(vec![append("결정", "보관 기간")]), "demo", &link, Some(partial)).unwrap();
        assert!(rendered.starts_with("---\nqmd:\n  metadata:\n    project: \"demo\"\n    kind: \"decision\"\n    authority: \"user_decision\"\n---\n\n# 방침\n"), "{rendered}");
    }

    #[test]
    fn model_written_sources_section_is_replaced_by_the_program() {
        let mut change = proposal().changes[0].clone();
        change.body = "본문\n## Sources\n- 모델이 만든 근거".into();
        let rendered = render_note(&change, "demo", &link, None).unwrap();
        assert!(!rendered.contains("모델이 만든 근거"));
        assert!(rendered.contains("본문\n\n## Sources\n- [L3]"));
        let appended = render_note(&editing(vec![append("남은 점", "보관 기간\n## 원문 근거\n- 모델이 만든 근거")]), "demo", &link, Some(OLD_NOTE)).unwrap();
        assert!(!appended.contains("모델이 만든 근거"), "{appended}");
    }

    #[test]
    fn snapshot_rejects_outside_edits_and_accepts_own_partial_write() {
        let map = |items: &[(&str, &str)]| items.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<BTreeMap<_, _>>();
        let base = map(&[("projects/demo/a.md", "old"), ("common/b.md", "context")]);
        let rendered = map(&[("projects/demo/a.md", "new")]);
        check_snapshot(&base, &base, &rendered).unwrap();
        check_snapshot(&map(&[("projects/demo/a.md", "new"), ("common/b.md", "context")]), &base, &rendered).unwrap();
        for current in [
            map(&[("projects/demo/a.md", "old")]),
            map(&[("projects/demo/a.md", "edited"), ("common/b.md", "context")]),
            map(&[("projects/demo/a.md", "old"), ("common/b.md", "context"), ("common/c.md", "added")]),
        ] {
            assert!(check_snapshot(&current, &base, &rendered).is_err());
        }
    }
}
