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
pub struct Change {
    pub path: String,
    pub title: String,
    pub kind: String,
    pub authority: String,
    pub body: String,
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
const RESERVED: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7",
    "lpt8", "lpt9",
];
pub const SOURCES_HEADING: &str = "\n## 원문 근거\n";

pub fn validate_note_path(path: &str, project: &str) -> Result<()> {
    let fail = || Error::rejected(format!("허용되지 않은 기억 경로입니다: {path}"));
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
            return Err(Error::rejected("같은 문서를 두 번 변경할 수 없습니다."));
        }
        if let Some(existing) = scope.existing.iter().find(|existing| existing.eq_ignore_ascii_case(&change.path)) {
            if *existing != change.path {
                return Err(Error::rejected(format!("기존 문서와 대소문자만 다른 경로입니다: {}", change.path)));
            }
            if !scope.editable.contains(existing) {
                return Err(Error::rejected(format!("본문을 받지 않은 기존 문서는 고칠 수 없습니다: {}", change.path)));
            }
        }
        if !KINDS.contains(&change.kind.as_str()) || !AUTHORITIES.contains(&change.authority.as_str()) {
            return Err(Error::rejected("기록 종류 또는 근거 구분이 올바르지 않습니다."));
        }
        if change.title.trim().is_empty() || change.body.trim().is_empty() {
            return Err(Error::rejected("기억 제목과 본문이 필요합니다."));
        }
        if change.evidence.is_empty() {
            return Err(Error::rejected("원문 근거가 필요합니다."));
        }
        let mut origins = BTreeSet::new();
        for citation in &change.evidence {
            let turn = available.get(citation.reference.as_str());
            match turn {
                Some(turn) if !citation.quote.trim().is_empty() && turn.text.contains(&citation.quote) => {
                    origins.insert(turn.origin as u8);
                }
                _ => return Err(Error::rejected("근거 위치 또는 인용문이 제공된 원문과 일치하지 않습니다.")),
            }
        }
        if change.authority == "user_decision" && !origins.contains(&(Origin::User as u8)) {
            return Err(Error::rejected("확인된 사용자 발언 없이 사용자 결정으로 기록할 수 없습니다."));
        }
    }
    Ok(())
}

fn quoted(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

pub fn render_note(change: &Change, project: &str, link: &dyn Fn(&Evidence) -> String, old: &str) -> String {
    let scope = if change.path.starts_with("common/") { "common" } else { project };
    let header = format!(
        "---\nqmd:\n  metadata:\n    project: {}\n    kind: {}\n    authority: {}\n---\n\n",
        quoted(scope),
        quoted(&change.kind),
        quoted(&change.authority)
    );
    let old = old.replace("\r\n", "\n");
    let mut sources: Vec<String> = old.split_once(SOURCES_HEADING).map(|(_, rest)| rest.trim().lines().map(str::to_string).collect()).unwrap_or_default();
    for evidence in &change.evidence {
        sources.push(format!("- [{}]({}) — {}", evidence.reference, link(evidence), evidence.quote.replace(['\r', '\n'], " ")));
    }
    let mut seen = BTreeSet::new();
    sources.retain(|line| seen.insert(line.clone()));
    let body = change.body.replace("\r\n", "\n");
    let body = body.trim();
    let body = body.split_once(SOURCES_HEADING).map(|(before, _)| before).unwrap_or(body).trim_end();
    let title = change.title.replace(['\r', '\n'], " ");
    format!("{header}# {}\n\n{body}\n\n## 원문 근거\n{}\n", title.trim(), sources.join("\n"))
}

pub fn check_snapshot(current: &BTreeMap<String, String>, base: &BTreeMap<String, String>, rendered: &BTreeMap<String, String>) -> Result<()> {
    let names: BTreeSet<&String> = current.keys().chain(base.keys()).chain(rendered.keys()).collect();
    for name in names {
        let now = current.get(name);
        let unchanged = now == base.get(name);
        let ours = rendered.contains_key(name) && now == rendered.get(name);
        if !unchanged && !ours {
            return Err(Error::permanent(format!("변경안을 만든 뒤 기억 문서가 바뀌었습니다: {name}")));
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
                evidence: vec![Evidence { reference: "L3".into(), quote: "경고는 빼고 로그만 남기십시오.".into() }],
            }],
            needs_notes: vec![],
        }
    }

    fn check(proposal: &Proposal, turns: &[Turn], existing: &[&str], editable: &[&str]) -> Result<()> {
        let existing: BTreeSet<String> = existing.iter().map(|s| s.to_string()).collect();
        let editable: BTreeSet<String> = editable.iter().map(|s| s.to_string()).collect();
        validate(proposal, &Scope { project: "demo", turns, existing: &existing, editable: &editable })
    }

    fn link(evidence: &Evidence) -> String {
        format!("<D:/k/sources/claude/s.md:{}>", &evidence.reference[1..])
    }

    #[test]
    fn supported_decision_renders_markdown_with_program_owned_sources() {
        let proposal = proposal();
        check(&proposal, &turns(), &[], &[]).unwrap();
        let rendered = render_note(&proposal.changes[0], "demo", &link, "");
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

    #[test]
    fn existing_note_can_change_only_when_its_body_was_provided() {
        let path = "projects/demo/bg-policy.md";
        assert!(check(&proposal(), &turns(), &[path], &[]).is_err());
        check(&proposal(), &turns(), &[path], &[path]).unwrap();
        assert!(check(&proposal(), &turns(), &["projects/demo/BG-Policy.md"], &["projects/demo/BG-Policy.md"]).is_err());
    }

    #[test]
    fn earlier_sources_survive_rewriting_even_with_crlf() {
        let old = "# 방침\r\n\r\n## 원문 근거\r\n- [기존 발언](C:/old.jsonl:4)\r\n";
        let rendered = render_note(&proposal().changes[0], "demo", &link, old);
        assert!(rendered.contains("- [기존 발언](C:/old.jsonl:4)\n- [L3]"));
        let twice = render_note(&proposal().changes[0], "demo", &link, &rendered);
        assert_eq!(twice.matches("- [L3]").count(), 1);
    }

    #[test]
    fn model_written_sources_section_is_replaced_by_the_program() {
        let mut change = proposal().changes[0].clone();
        change.body = "본문\n## 원문 근거\n- 모델이 만든 근거".into();
        let rendered = render_note(&change, "demo", &link, "");
        assert!(!rendered.contains("모델이 만든 근거"));
        assert!(rendered.contains("본문\n\n## 원문 근거\n- [L3]"));
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
