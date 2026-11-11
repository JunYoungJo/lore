use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::copy;
use crate::error::Result;
use crate::notes;
use crate::queue::Job;
use crate::settings::{atomic_write, home};
use crate::transcript::{parse, read_transcript, Provider};

const NEAREST_LINES: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkRef {
    pub start: usize,
    pub end: usize,
    pub path: String,
    pub line: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub notes: Vec<String>,
    pub copies: Vec<String>,
    pub rewritten: usize,
    pub approximate: usize,
    pub unresolved: Vec<String>,
    pub unavailable: Vec<String>,
}

pub fn find_links(text: &str) -> Vec<LinkRef> {
    let mut links = Vec::new();
    let mut offset = 0;
    while let Some(found) = text[offset..].find("](") {
        let start = offset + found + 2;
        let Some(length) = text[start..].find(')') else { break };
        let end = start + length;
        offset = end;
        let inner = text[start..end].trim();
        let inner = inner.strip_prefix('<').and_then(|rest| rest.strip_suffix('>')).unwrap_or(inner);
        let Some((path, line)) = inner.rsplit_once(':') else { continue };
        let Ok(line) = line.parse::<usize>() else { continue };
        if !path.to_ascii_lowercase().ends_with(".jsonl") {
            continue;
        }
        links.push(LinkRef { start, end, path: path.to_string(), line });
    }
    links
}

pub fn provider_of(path: &str) -> Option<Provider> {
    let normalized = path.replace('\\', "/").to_ascii_lowercase();
    if normalized.contains("/.claude/") {
        Some(Provider::Claude)
    } else if normalized.contains("/.codex/") {
        Some(Provider::Codex)
    } else {
        None
    }
}

fn backup_candidate(path: &str, backup: &Path) -> Option<PathBuf> {
    let normalized = path.replace('\\', "/");
    let home = home().to_string_lossy().replace('\\', "/");
    let prefix = format!("{}/", home.trim_end_matches('/'));
    if normalized.len() <= prefix.len() || !normalized[..prefix.len()].eq_ignore_ascii_case(&prefix) {
        return None;
    }
    Some(backup.join(&normalized[prefix.len()..]))
}

fn nearest(lines: &BTreeMap<usize, usize>, wanted: usize) -> Option<(usize, bool)> {
    if let Some(line) = lines.get(&wanted) {
        return Some((*line, true));
    }
    lines
        .iter()
        .filter(|(original, _)| original.abs_diff(wanted) <= NEAREST_LINES)
        .min_by_key(|(original, _)| (original.abs_diff(wanted), **original))
        .map(|(_, line)| (*line, false))
}

struct Target {
    copy_file: PathBuf,
    lines: BTreeMap<usize, usize>,
}

fn note_files(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut add_folder = |relative: String| {
        if let Ok(entries) = std::fs::read_dir(root.join(&relative)) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if entry.file_type().map(|kind| kind.is_file()).unwrap_or(false) && name.ends_with(".md") {
                    found.push(format!("{relative}/{name}"));
                }
            }
        }
    };
    add_folder("common".into());
    if let Ok(projects) = std::fs::read_dir(root.join("projects")) {
        for project in projects.flatten() {
            if project.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                add_folder(format!("projects/{}", project.file_name().to_string_lossy()));
            }
        }
    }
    found.sort();
    found
}

pub fn migrate_links(knowledge_root: &Path, backup: Option<&Path>, apply: bool) -> Result<Report> {
    let mut report = Report::default();
    let notes = note_files(knowledge_root);
    let mut texts = BTreeMap::new();
    let mut transcripts = BTreeSet::new();
    for note in &notes {
        let text = std::fs::read_to_string(knowledge_root.join(note))?;
        for link in find_links(&text) {
            transcripts.insert(link.path.clone());
        }
        texts.insert(note.clone(), text);
    }
    let mut targets: BTreeMap<String, Target> = BTreeMap::new();
    for path in &transcripts {
        let Some(provider) = provider_of(path) else { continue };
        let original = PathBuf::from(path);
        let data = read_transcript(&original).ok().or_else(|| backup.and_then(|dir| backup_candidate(path, dir)).and_then(|candidate| read_transcript(&candidate).ok()));
        let fallback = original.file_stem().map(|stem| stem.to_string_lossy().to_string()).unwrap_or_else(|| "session".into());
        let Some(data) = data else {
            report.unavailable.push(path.clone());
            continue;
        };
        let parsed = parse(&data, provider)?;
        let copy_file = copy::copy_path(knowledge_root, provider, &parsed.session_id, &fallback);
        let dialogue = copy::render(&parsed, provider, &original, parsed.complete_lines);
        if apply {
            atomic_write(&copy_file, dialogue.text.as_bytes())?;
        }
        report.copies.push(copy_file.to_string_lossy().replace('\\', "/"));
        targets.insert(path.clone(), Target { copy_file, lines: dialogue.lines });
    }
    for (note, text) in &texts {
        let mut rewritten = String::with_capacity(text.len());
        let mut cursor = 0;
        let mut changed = false;
        for link in find_links(text) {
            let Some(target) = targets.get(&link.path) else { continue };
            match nearest(&target.lines, link.line) {
                Some((line, exact)) => {
                    rewritten.push_str(&text[cursor..link.start]);
                    rewritten.push_str(&copy::link_target(&target.copy_file, line));
                    cursor = link.end;
                    changed = true;
                    report.rewritten += 1;
                    if !exact {
                        report.approximate += 1;
                    }
                }
                None => report.unresolved.push(format!("{note}: {}:{}", link.path, link.line)),
            }
        }
        if !changed {
            continue;
        }
        rewritten.push_str(&text[cursor..]);
        if apply {
            atomic_write(&knowledge_root.join(note), rewritten.as_bytes())?;
        }
        report.notes.push(note.clone());
    }
    Ok(report)
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshReport {
    pub copies: Vec<String>,
    pub added_turns: usize,
    pub notes: Vec<String>,
    pub relinked: usize,
    pub unavailable: Vec<String>,
}

pub fn refresh_copies(knowledge_root: &Path, jobs: &[Job], apply: bool) -> Result<RefreshReport> {
    let mut report = RefreshReport::default();
    let mut notes = notes::load_all(knowledge_root)?;
    for job in jobs.iter().filter(|job| job.cursor_line > 0) {
        let transcript = PathBuf::from(&job.transcript);
        let Ok(data) = read_transcript(&transcript) else {
            report.unavailable.push(job.transcript.clone());
            continue;
        };
        let parsed = parse(&data, job.provider)?;
        let copy_file = copy::copy_path(knowledge_root, job.provider, &parsed.session_id, &job.id);
        let Ok(old_copy) = std::fs::read_to_string(&copy_file) else { continue };
        let dialogue = copy::render(&parsed, job.provider, &transcript, job.cursor_line);
        if dialogue.text == old_copy {
            continue;
        }
        report.added_turns += dialogue.lines.len().saturating_sub(copy::parse_headings(&old_copy).len());
        let moves = copy::line_moves(&old_copy, &dialogue.lines);
        for (note, text) in notes.iter_mut() {
            let Some(updated) = copy::relink(text, &copy_file, &moves) else { continue };
            report.relinked += 1;
            if !report.notes.contains(note) {
                report.notes.push(note.clone());
            }
            if apply {
                atomic_write(&knowledge_root.join(note.as_str()), updated.as_bytes())?;
            }
            *text = updated;
        }
        if apply {
            atomic_write(&copy_file, dialogue.text.as_bytes())?;
        }
        report.copies.push(copy_file.to_string_lossy().replace('\\', "/"));
    }
    report.notes.sort();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn links_with_and_without_angle_brackets_are_found() {
        let text = "근거 ([2026-10-23 · codex:1404](C:/Users/me/.codex/sessions/a.jsonl:1404)) 그리고\n- [L3](<D:/x/.claude/projects/p/b.jsonl:3>) — 인용\n[문서](other.md)";
        let links = find_links(text);
        assert_eq!(links.len(), 2);
        assert_eq!((links[0].path.as_str(), links[0].line), ("C:/Users/me/.codex/sessions/a.jsonl", 1404));
        assert_eq!((links[1].path.as_str(), links[1].line), ("D:/x/.claude/projects/p/b.jsonl", 3));
        assert_eq!(&text[links[1].start..links[1].end], "<D:/x/.claude/projects/p/b.jsonl:3>");
        assert_eq!(provider_of(&links[0].path), Some(Provider::Codex));
        assert_eq!(provider_of("C:/tmp/a.jsonl"), None);
    }

    #[test]
    fn nearest_turn_is_used_only_within_a_few_lines() {
        let lines = BTreeMap::from([(10, 7), (20, 15)]);
        assert_eq!(nearest(&lines, 10), Some((7, true)));
        assert_eq!(nearest(&lines, 12), Some((7, false)));
        assert_eq!(nearest(&lines, 15), Some((7, false)));
        assert_eq!(nearest(&lines, 16), Some((15, false)));
        assert_eq!(nearest(&lines, 30), None);
    }

    fn job(id: &str, transcript: &Path, cursor_line: usize) -> Job {
        Job {
            id: id.into(),
            provider: Provider::Claude,
            transcript: transcript.to_string_lossy().to_string(),
            status: crate::queue::Status::Done,
            cursor_line,
            cursor_hash: String::new(),
            project: Some("demo".into()),
            session_id: Some("s1".into()),
            attempts: 0,
            next_attempt_at: 0,
            manifest: None,
            last_error: None,
            updated_at: 0,
        }
    }

    #[test]
    fn copies_are_rebuilt_with_turns_the_old_reader_missed_and_links_follow() {
        let dir = tempfile::tempdir().unwrap();
        let knowledge = dir.path().join("knowledge");
        let transcript = dir.path().join("s1.jsonl");
        let records = [
            r#"{"type":"user","uuid":"a","sessionId":"s1","cwd":"D:/demo","userType":"external","message":{"content":"로그만 남기십시오"}}"#,
            r#"{"type":"attachment","uuid":"q","attachment":{"type":"queued_command","prompt":"보관 기간도 정하십시오","origin":{"kind":"human"}}}"#,
            r#"{"type":"assistant","uuid":"b","message":{"content":[{"type":"text","text":"그렇게 하겠습니다"}]}}"#,
            r#"{"type":"user","uuid":"c","userType":"external","message":{"content":"아직 기록 전인 말"}}"#,
        ];
        fs::write(&transcript, records.iter().map(|line| format!("{line}\n")).collect::<String>()).unwrap();
        let parsed = parse(&fs::read(&transcript).unwrap(), Provider::Claude).unwrap();
        let mut old_reader = parsed.clone();
        old_reader.turns.retain(|turn| turn.line != 2);
        let old_copy = copy::render(&old_reader, Provider::Claude, &transcript, 3);
        let copy_file = copy::copy_path(&knowledge, Provider::Claude, "s1", "job1");
        fs::create_dir_all(copy_file.parent().unwrap()).unwrap();
        fs::write(&copy_file, &old_copy.text).unwrap();
        let note = knowledge.join("projects").join("demo").join("bg.md");
        fs::create_dir_all(note.parent().unwrap()).unwrap();
        let note_text = format!("- [L3]({}) — 그렇게 하겠습니다\n", copy::link_target(&copy_file, old_copy.lines[&3]));
        fs::write(&note, &note_text).unwrap();
        let jobs = vec![job("job1", &transcript, 3), job("gone", &dir.path().join("missing.jsonl"), 5)];

        let preview = refresh_copies(&knowledge, &jobs, false).unwrap();
        assert_eq!(preview.copies, vec![copy_file.to_string_lossy().replace('\\', "/")]);
        assert_eq!(preview.added_turns, 1);
        assert_eq!(preview.notes, vec!["projects/demo/bg.md".to_string()]);
        assert_eq!(preview.unavailable, vec![jobs[1].transcript.clone()]);
        assert_eq!(fs::read_to_string(&note).unwrap(), note_text);

        refresh_copies(&knowledge, &jobs, true).unwrap();
        let rebuilt = fs::read_to_string(&copy_file).unwrap();
        let lines = copy::parse_headings(&rebuilt);
        assert!(rebuilt.contains("## L2 · User") && !rebuilt.contains("아직 기록 전인 말"));
        assert_eq!(fs::read_to_string(&note).unwrap(), format!("- [L3]({}) — 그렇게 하겠습니다\n", copy::link_target(&copy_file, lines[&3])));
        assert!(refresh_copies(&knowledge, &jobs, true).unwrap().copies.is_empty());
    }

    #[test]
    fn notes_are_relinked_to_dialogue_copies_and_unavailable_sessions_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let knowledge = dir.path().join("knowledge");
        let sessions = dir.path().join(".claude").join("projects").join("p");
        fs::create_dir_all(&sessions).unwrap();
        let transcript = sessions.join("s1.jsonl");
        fs::write(&transcript, "{\"type\":\"user\",\"uuid\":\"a\",\"sessionId\":\"s1\",\"userType\":\"external\",\"message\":{\"content\":\"로그만 남기십시오\"}}\n").unwrap();
        let source = transcript.to_string_lossy().replace('\\', "/");
        let missing = format!("{}/gone.jsonl", sessions.to_string_lossy().replace('\\', "/"));
        let note = knowledge.join("projects").join("demo").join("bg.md");
        fs::create_dir_all(note.parent().unwrap()).unwrap();
        fs::write(&note, format!("결정 ([근거]({source}:1)), 사라진 근거 ([옛]({missing}:4))\n")).unwrap();

        let preview = migrate_links(&knowledge, None, false).unwrap();
        assert_eq!(preview.rewritten, 1);
        assert_eq!(preview.unavailable, vec![missing.clone()]);
        assert!(fs::read_to_string(&note).unwrap().contains(&source));

        let report = migrate_links(&knowledge, None, true).unwrap();
        let copy_file = knowledge.join("sources").join("claude").join("s1.md");
        let copy_text = fs::read_to_string(&copy_file).unwrap();
        let heading = copy_text.lines().position(|line| line.starts_with("## L1 ·")).unwrap() + 1;
        let updated = fs::read_to_string(&note).unwrap();
        assert!(updated.contains(&format!("([근거]({}))", copy::link_target(&copy_file, heading))));
        assert!(updated.contains(&format!("([옛]({missing}:4))")));
        assert_eq!(report.notes, vec!["projects/demo/bg.md".to_string()]);
        assert_eq!(migrate_links(&knowledge, None, true).unwrap().rewritten, 0);
    }
}
