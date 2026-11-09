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
        "# {} 세션 {}\n\n- 원본: `{}`\n- 작업 폴더: `{}`\n\n기억 문서의 근거를 확인하기 위한 대화 사본입니다. 사용자·어시스턴트 발언과 다른 에이전트가 보낸 메시지를 원본 줄 번호(L)와 함께 담았습니다. 시각은 UTC입니다. 도구 실행 결과는 들어 있지 않습니다.\n",
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

pub fn line_moves(old_copy: &str, new_lines: &BTreeMap<usize, usize>) -> BTreeMap<usize, usize> {
    parse_headings(old_copy)
        .into_iter()
        .filter_map(|(original, old_line)| new_lines.get(&original).filter(|new_line| **new_line != old_line).map(|new_line| (old_line, *new_line)))
        .collect()
}

pub fn relink(text: &str, copy_file: &Path, moves: &BTreeMap<usize, usize>) -> Option<String> {
    let prefix = format!("<{}:", copy_file.to_string_lossy().replace('\\', "/"));
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    let mut changed = false;
    while let Some(found) = rest.find(&prefix) {
        let digits_start = found + prefix.len();
        let digits: String = rest[digits_start..].chars().take_while(char::is_ascii_digit).collect();
        let closes = rest[digits_start + digits.len()..].starts_with('>');
        let moved = digits.parse::<usize>().ok().filter(|_| closes).and_then(|line| moves.get(&line));
        result.push_str(&rest[..digits_start]);
        match moved {
            Some(line) => {
                result.push_str(&line.to_string());
                changed = true;
            }
            None => result.push_str(&digits),
        }
        rest = &rest[digits_start + digits.len()..];
    }
    result.push_str(rest);
    changed.then_some(result)
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
    fn links_follow_their_turn_when_a_rewritten_copy_moves_it() {
        let transcript = Path::new("C:/s.jsonl");
        let before = render(&parsed(), Provider::Claude, transcript, 9);
        let mut grown = parsed();
        grown.turns.insert(1, Turn { reference: "L3".into(), line: 3, role: "user".into(), origin: Origin::User, timestamp: String::new(), text: "도중에 보낸 말".into() });
        let after = render(&grown, Provider::Claude, transcript, 9);
        let moves = line_moves(&before.text, &after.lines);
        assert_eq!(moves.get(&before.lines[&2]), None);
        assert_eq!(moves[&before.lines[&5]], after.lines[&5]);
        assert_eq!(moves[&before.lines[&9]], after.lines[&9]);

        let file = Path::new("D:\\k\\sources\\claude\\abc-123.md");
        let other = Path::new("D:\\k\\sources\\claude\\other.md");
        let note = format!(
            "본문 ([근거]({})) 다른 세션 ([근거]({}))\n## 원문 근거\n- [L5]({}) — 로그만\n- [L2]({}) — 경고는",
            link_target(file, before.lines[&9]),
            link_target(other, before.lines[&5]),
            link_target(file, before.lines[&5]),
            link_target(file, before.lines[&2])
        );
        let relinked = relink(&note, file, &moves).unwrap();
        let expected = format!(
            "본문 ([근거]({})) 다른 세션 ([근거]({}))\n## 원문 근거\n- [L5]({}) — 로그만\n- [L2]({}) — 경고는",
            link_target(file, after.lines[&9]),
            link_target(other, before.lines[&5]),
            link_target(file, after.lines[&5]),
            link_target(file, after.lines[&2])
        );
        assert_eq!(relinked, expected);
        assert_eq!(relink(&expected.replace("abc-123", "unrelated"), file, &moves), None);
        assert!(line_moves(&before.text, &before.lines).is_empty());
    }

    #[test]
    fn copy_path_uses_provider_folder_and_safe_session_name() {
        let root = Path::new("D:/doc/knowledge");
        assert_eq!(copy_path(root, Provider::Codex, "01a1-..:x", "job"), root.join("sources").join("codex").join("01a1-x.md"));
        assert_eq!(copy_path(root, Provider::Claude, "", "job1"), root.join("sources").join("claude").join("job1.md"));
        assert_eq!(link_target(Path::new("D:\\doc\\knowledge\\sources\\claude\\a.md"), 12), "<D:/doc/knowledge/sources/claude/a.md:12>");
    }
}
