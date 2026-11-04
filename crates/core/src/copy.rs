use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::transcript::{Origin, Parsed, Provider};

pub const SOURCES_DIR: &str = "sources";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogueCopy {
    pub text: String,
    pub lines: BTreeMap<usize, usize>,
}

fn file_stem(session_id: &str, fallback: &str) -> String {
    let cleaned: String = session_id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    if cleaned.is_empty() {
        fallback.to_string()
    } else {
        cleaned
    }
}

pub fn copy_path(knowledge_root: &Path, provider: Provider, session_id: &str, fallback: &str) -> PathBuf {
    knowledge_root.join(SOURCES_DIR).join(provider.as_str()).join(format!("{}.md", file_stem(session_id, fallback)))
}

fn speaker(role: &str, origin: Origin) -> &'static str {
    match (role, origin) {
        ("assistant", _) => "어시스턴트",
        (_, Origin::User) => "사용자",
        (_, Origin::Context) => "시스템 문맥",
        (_, Origin::Agent) => "에이전트 전달 메시지",
        _ => "확인되지 않은 입력",
    }
}

fn provider_label(provider: Provider) -> &'static str {
    match provider {
        Provider::Claude => "Claude",
        Provider::Codex => "Codex",
    }
}

pub fn render(parsed: &Parsed, provider: Provider, transcript: &Path, end_line: usize) -> DialogueCopy {
    let mut text = format!(
        "# {} 세션 {}\n\n- 원본: `{}`\n- 작업 폴더: `{}`\n\n기억 문서의 근거를 확인하기 위한 대화 사본입니다. 사용자와 어시스턴트 발언만 원본 줄 번호(L)와 함께 담았습니다. 도구 실행 결과는 들어 있지 않습니다.\n",
        provider_label(provider),
        parsed.session_id,
        transcript.to_string_lossy().replace('\\', "/"),
        parsed.cwd.replace('\\', "/"),
    );
    let mut lines = BTreeMap::new();
    for turn in parsed.turns.iter().filter(|turn| turn.line <= end_line) {
        text.push('\n');
        lines.insert(turn.line, text.matches('\n').count() + 1);
        let stamp = if turn.timestamp.is_empty() { String::new() } else { format!(" · {}", turn.timestamp) };
        text.push_str(&format!("## L{} · {}{}\n\n", turn.line, speaker(&turn.role, turn.origin), stamp));
        let body = turn.text.replace("\r\n", "\n");
        text.push_str(body.trim_end());
        text.push('\n');
    }
    DialogueCopy { text, lines }
}

pub fn link_target(copy_file: &Path, copy_line: usize) -> String {
    format!("<{}:{}>", copy_file.to_string_lossy().replace('\\', "/"), copy_line)
}

pub fn parse_headings(text: &str) -> BTreeMap<usize, usize> {
    let mut lines = BTreeMap::new();
    for (index, line) in text.lines().enumerate() {
        let Some(rest) = line.strip_prefix("## L") else { continue };
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() || !rest[digits.len()..].starts_with(" · ") {
            continue;
        }
        if let Ok(original) = digits.parse() {
            lines.entry(original).or_insert(index + 1);
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::Turn;

    fn parsed() -> Parsed {
        let turn = |line: usize, role: &str, origin: Origin, text: &str| Turn {
            reference: format!("L{line}"),
            line,
            role: role.into(),
            origin,
            timestamp: "2026-11-04T01:00:00Z".into(),
            text: text.into(),
        };
        Parsed {
            session_id: "abc-123".into(),
            cwd: "D:\\src\\demo".into(),
            worker_session: false,
            complete_lines: 9,
            turns: vec![
                turn(2, "user", Origin::User, "경고는 빼고\r\n로그만 남기십시오."),
                turn(5, "assistant", Origin::Assistant, "로그만 남기겠습니다.\n"),
                turn(9, "user", Origin::Unverified, "나중 발언"),
            ],
        }
    }

    #[test]
    fn copy_lists_turns_with_original_line_numbers_and_maps_them() {
        let copy = render(&parsed(), Provider::Claude, Path::new("C:\\Users\\me\\.claude\\projects\\x\\abc-123.jsonl"), 5);
        let lines: Vec<&str> = copy.text.lines().collect();
        assert!(lines[0].starts_with("# Claude 세션 abc-123"));
        assert!(copy.text.contains("- 원본: `C:/Users/me/.claude/projects/x/abc-123.jsonl`"));
        assert_eq!(lines[copy.lines[&2] - 1], "## L2 · 사용자 · 2026-11-04T01:00:00Z");
        assert_eq!(lines[copy.lines[&2] + 1], "경고는 빼고");
        assert_eq!(lines[copy.lines[&5] - 1], "## L5 · 어시스턴트 · 2026-11-04T01:00:00Z");
        assert!(!copy.lines.contains_key(&9));
        assert!(!copy.text.contains("나중 발언"));
    }

    #[test]
    fn headings_can_be_read_back_from_a_saved_copy() {
        let copy = render(&parsed(), Provider::Codex, Path::new("C:/s.jsonl"), 9);
        assert_eq!(parse_headings(&copy.text), copy.lines);
        assert!(copy.text.contains("## L9 · 확인되지 않은 입력"));
    }

    #[test]
    fn copy_path_uses_provider_folder_and_safe_session_name() {
        let root = Path::new("D:/doc/knowledge");
        assert_eq!(copy_path(root, Provider::Codex, "01a1-..:x", "job"), root.join("sources").join("codex").join("01a1-x.md"));
        assert_eq!(copy_path(root, Provider::Claude, "", "job1"), root.join("sources").join("claude").join("job1.md"));
        assert_eq!(link_target(Path::new("D:\\doc\\knowledge\\sources\\claude\\a.md"), 12), "<D:/doc/knowledge/sources/claude/a.md:12>");
    }
}
