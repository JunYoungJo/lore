//! Explicit, resumable conversion of old absolute source links. No model calls.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::copy;
use crate::error::{Error, Result};
use crate::settings::atomic_write;
use crate::transcript::Provider;

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub notes: Vec<String>,
    pub copies: Vec<String>,
    pub rewritten: usize,
    pub unresolved: Vec<String>,
    pub resuming: bool,
    pub backup: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct Plan {
    root: String,
    base: BTreeMap<String, String>,
    writes: BTreeMap<String, String>,
    rewritten: usize,
    unresolved: Vec<String>,
}

struct Link {
    start: usize,
    end: usize,
    source: String,
    line: usize,
    provider: Provider,
    session: String,
}

fn file_at(root: &Path, relative: &str) -> Result<std::path::PathBuf> {
    if relative.contains('\\') || !relative.ends_with(".md") || Path::new(relative).components().any(|part| !matches!(part, std::path::Component::Normal(_))) {
        return Err(Error::permanent("링크 이전 계획에 잘못된 경로가 있습니다."));
    }
    let mut file = root.to_path_buf();
    for part in Path::new(relative).components() {
        file.push(part);
        match file.symlink_metadata() {
            Ok(metadata) if metadata.file_type().is_symlink() => return Err(Error::permanent(format!("심볼릭 링크를 통해 이전할 수 없습니다: {relative}"))),
            Ok(_) => {},
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(error.into()),
        }
    }
    Ok(file)
}

fn links(text: &str) -> Vec<Link> {
    let mut result = Vec::new();
    let mut offset = 0;
    while let Some(found) = text[offset..].find("](") {
        let start = offset + found + 2;
        let Some(length) = text[start..].find(')') else { break };
        let end = start + length;
        offset = end + 1;
        let inner = text[start..end].trim().trim_start_matches('<').trim_end_matches('>');
        let Some((path, number)) = inner.rsplit_once(':') else { continue };
        let Ok(line) = number.parse::<usize>() else { continue };
        let path = path.replace('\\', "/");
        let relative = path.rfind("/sources/").map(|index| &path[index + 1..]).unwrap_or(&path);
        let parts: Vec<_> = relative.split('/').collect();
        let ["sources", provider, file] = parts.as_slice() else { continue };
        let Some(provider) = Provider::parse(provider) else { continue };
        let Some(session) = file.strip_suffix(".md") else { continue };
        if session.is_empty() || !session.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') { continue; }
        result.push(Link { start, end, source: relative.to_string(), line, provider, session: session.to_string() });
    }
    result
}

fn anchored(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut previous = "";
    for line in text.split_inclusive('\n') {
        if let Some(rest) = line.strip_prefix("## L") {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            if !digits.is_empty() && rest[digits.len()..].starts_with(" · ") {
                let anchor = format!("<a id=\"L{digits}\"></a>");
                if previous.trim_end() != anchor { result.push_str(&format!("{anchor}\n")); }
            }
        }
        result.push_str(line);
        previous = line;
    }
    result
}

fn prepare(root: &Path) -> Result<Plan> {
    let mut notes = crate::notes::load_all(root)?;
    let readme = root.join("README.md");
    if readme.is_file() && !readme.is_symlink() { notes.insert("README.md".into(), std::fs::read_to_string(readme)?); }
    let mut sources = BTreeMap::new();
    let mut blocked = BTreeSet::new();
    let mut unresolved = Vec::new();
    for (note, text) in &notes {
        for link in links(text) {
            if !sources.contains_key(&link.source) {
                let file = file_at(root, &link.source)?;
                match std::fs::read_to_string(file) {
                    Ok(text) => { sources.insert(link.source.clone(), text); }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
            let unambiguous = sources.get(&link.source).map(|text| {
                let headings = copy::parse_headings(text);
                let count = text.lines().filter(|line| {
                    line.strip_prefix("## L").map(|rest| {
                        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
                        !digits.is_empty() && rest[digits.len()..].starts_with(" · ")
                    }).unwrap_or(false)
                }).count();
                count == headings.len() && headings.values().any(|line| *line == link.line)
            }).unwrap_or(false);
            if !unambiguous {
                blocked.insert(link.source.clone());
                unresolved.push(format!("{note}: {}:{}", link.source, link.line));
            }
        }
    }
    let mut plan = Plan { root: root.to_string_lossy().to_string(), base: BTreeMap::new(), writes: BTreeMap::new(), rewritten: 0, unresolved };
    for (note, text) in notes {
        let mut updated = String::new();
        let mut cursor = 0;
        for link in links(&text).into_iter().filter(|link| !blocked.contains(&link.source)) {
            let source = &sources[&link.source];
            let original = copy::parse_headings(source).into_iter().find(|(_, line)| *line == link.line).unwrap().0;
            updated.push_str(&text[cursor..link.start]);
            updated.push_str(&copy::portable_link(&note, link.provider, &link.session, "", &format!("L{original}")));
            cursor = link.end;
            plan.rewritten += 1;
            let with_anchors = anchored(source);
            if with_anchors != *source {
                plan.base.insert(link.source.clone(), source.clone());
                plan.writes.insert(link.source, with_anchors);
            }
        }
        if cursor > 0 {
            updated.push_str(&text[cursor..]);
            plan.base.insert(note.clone(), text);
            plan.writes.insert(note, updated);
        }
    }
    Ok(plan)
}

pub fn migrate(root: &Path, journal: &Path, apply: bool) -> Result<Report> {
    let pending = match std::fs::read(journal) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let plan = if let Some(bytes) = &pending { serde_json::from_slice::<Plan>(bytes)? } else { prepare(root)? };
    if plan.root != root.to_string_lossy() {
        return Err(Error::permanent("중단된 링크 이전의 기억 저장소가 다릅니다. 이전 설정으로 복구한 뒤 다시 실행하십시오."));
    }
    let mut report = Report { rewritten: plan.rewritten, unresolved: plan.unresolved.clone(), resuming: pending.is_some(), ..Report::default() };
    for path in plan.writes.keys() {
        if path.starts_with("sources/") { report.copies.push(path.clone()); } else { report.notes.push(path.clone()); }
    }
    if !apply || plan.writes.is_empty() { return Ok(report); }
    // A pending journal is also a full before/after backup. Accept our own
    // partial writes, but refuse to overwrite edits made outside this command.
    for (path, updated) in &plan.writes {
        let current = std::fs::read_to_string(file_at(root, path)?)?;
        let base = plan.base.get(path).ok_or_else(|| Error::permanent("링크 이전 계획의 원본 백업이 없습니다."))?;
        if current != *base && current != *updated {
            return Err(Error::permanent(format!("링크 이전 이후 외부에서 바뀐 파일입니다: {path}")));
        }
    }
    let bytes = serde_json::to_vec_pretty(&plan)?;
    atomic_write(journal, &bytes)?;
    // Notes first, then anchors. The journal makes either interruption resumable.
    for sources in [false, true] {
        for (path, text) in &plan.writes {
            if path.starts_with("sources/") == sources { atomic_write(&root.join(path), text.as_bytes())?; }
        }
    }
    let backup = journal.with_file_name(format!("portable-links-{}.backup.json", chrono::Utc::now().timestamp_micros()));
    std::fs::rename(journal, &backup)?;
    report.backup = Some(backup.to_string_lossy().to_string());
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_links_move_to_relative_turn_anchors_without_the_original_pc() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("knowledge");
        let journal = dir.path().join("app/portable-links.json");
        let note = root.join("projects/demo/decision.md");
        let source = root.join("sources/claude/s1.md");
        std::fs::create_dir_all(note.parent().unwrap()).unwrap();
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        let old_source = "# 세션\n\n## L7 · 사용자\n\n로그만 남기십시오.\n";
        let old_note = "- [L7](<D:/doc/knowledge/sources/claude/s1.md:3>) — 로그만 남기십시오.\n";
        std::fs::write(&note, old_note).unwrap();
        std::fs::write(&source, old_source).unwrap();
        let preview = migrate(&root, &journal, false).unwrap();
        assert_eq!(preview.rewritten, 1);
        assert_eq!(std::fs::read_to_string(&note).unwrap(), old_note);
        assert_eq!(std::fs::read_to_string(&source).unwrap(), old_source);
        assert!(!journal.exists());
        let report = migrate(&root, &journal, true).unwrap();
        assert!(std::fs::read_to_string(note).unwrap().contains("<../../sources/claude/s1.md#L7>"));
        assert!(std::fs::read_to_string(source).unwrap().contains("<a id=\"L7\"></a>\n## L7"));
        assert!(Path::new(report.backup.as_ref().unwrap()).exists());
        assert_eq!(migrate(&root, &journal, true).unwrap().rewritten, 0);
    }

    #[test]
    fn interrupted_migration_resumes_its_snapshot_and_refuses_outside_edits() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("knowledge");
        let journal = dir.path().join("journal.json");
        let note = root.join("common/a.md");
        let source = root.join("sources/codex/s.md");
        std::fs::create_dir_all(note.parent().unwrap()).unwrap();
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&note, "[L4](</old/sources/codex/s.md:1>)").unwrap();
        std::fs::write(&source, "## L4 · 사용자\n\n결정\n").unwrap();
        let plan = prepare(&root).unwrap();
        atomic_write(&journal, &serde_json::to_vec(&plan).unwrap()).unwrap();
        std::fs::write(&note, "외부 수정").unwrap();
        assert!(migrate(&root, &journal, true).is_err());
        std::fs::write(&note, &plan.writes["common/a.md"]).unwrap();
        let report = migrate(&root, &journal, true).unwrap();
        assert!(report.resuming && !journal.exists());
        assert_eq!(std::fs::read_to_string(source).unwrap(), plan.writes["sources/codex/s.md"]);
    }

    #[test]
    fn unresolved_links_block_every_conversion_of_the_same_copy() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("sources/codex/s.md");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::create_dir_all(dir.path().join("common")).unwrap();
        std::fs::write(source, "## L4 · 사용자\n\n결정\n").unwrap();
        std::fs::write(dir.path().join("common/a.md"), "[L4](</old/sources/codex/s.md:1>) [잘못된 줄](</old/sources/codex/s.md:30>)").unwrap();
        let report = migrate(dir.path(), &dir.path().join("journal.json"), true).unwrap();
        assert_eq!(report.rewritten, 0);
        assert_eq!(report.unresolved.len(), 1);
        assert!(report.copies.is_empty());
    }
}
