use std::collections::HashSet;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Claude,
    Codex,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    User,
    Assistant,
    Unverified,
    Context,
    Agent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    #[serde(rename = "ref")]
    pub reference: String,
    pub line: usize,
    pub role: String,
    pub origin: Origin,
    pub timestamp: String,
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub struct Parsed {
    pub session_id: String,
    pub cwd: String,
    pub worker_session: bool,
    pub turns: Vec<Turn>,
    pub complete_lines: usize,
}

const INJECTED_PREFIXES: [&str; 5] = [
    "# AGENTS.md instructions",
    "<environment_context>",
    "<INSTRUCTIONS>",
    "<system-reminder>",
    "<task-notification>",
];
const AGENT_PREFIXES: [&str; 7] = [
    "You are working inside Orca",
    "You are a dispatched worker",
    "Please carry out this task from my Orca coordinator",
    "Continue work from the prior Orca session",
    "The prior provider session is read-only context",
    "Original agent: ",
    "Another Claude session sent a message",
];

pub fn is_agent_message(text: &str) -> bool {
    let start = text.trim_start();
    let first_line = start.lines().next().unwrap_or("");
    AGENT_PREFIXES.iter().any(|prefix| start.starts_with(prefix)) || (first_line.starts_with("You have ") && first_line.contains("orchestration message"))
}

pub fn split_lines(data: &[u8]) -> Vec<&[u8]> {
    data.split_inclusive(|byte| *byte == b'\n').collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn digest(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

pub fn prefix_hash(data: &[u8], line: usize) -> String {
    let mut hasher = Sha256::new();
    for raw in split_lines(data).into_iter().take(line) {
        hasher.update(raw);
    }
    hex(&hasher.finalize())
}

pub fn check_prefix(data: &[u8], line: usize, hash: &str) -> Result<()> {
    if line > 0 && prefix_hash(data, line) != hash {
        return Err(Error::permanent("이미 처리한 원문 앞부분이 바뀌었습니다. 같은 원문으로 이어서 기록할 수 없습니다."));
    }
    Ok(())
}

pub fn compressed_path(path: &Path) -> PathBuf {
    let mut value = path.as_os_str().to_owned();
    value.push(".zst");
    PathBuf::from(value)
}

pub fn canonical_path(path: &Path) -> PathBuf {
    match path.to_str().and_then(|text| text.strip_suffix(".zst")) {
        Some(stripped) => PathBuf::from(stripped),
        None => path.to_path_buf(),
    }
}

pub fn read_transcript(path: &Path) -> Result<Vec<u8>> {
    match std::fs::read(path) {
        Ok(data) => Ok(data),
        Err(error) if error.kind() == ErrorKind::NotFound => match std::fs::File::open(compressed_path(path)) {
            Ok(file) => zstd::stream::decode_all(file).map_err(|error| Error::transient(format!("압축된 원문을 풀 수 없습니다: {error}"))),
            Err(error) if error.kind() == ErrorKind::NotFound => Err(Error::permanent(format!("원문 파일이 없습니다: {}", path.display()))),
            Err(error) => Err(error.into()),
        },
        Err(error) => Err(error.into()),
    }
}

fn text_field(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64().map(|n| n != 0.0).unwrap_or(false),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(map)) => !map.is_empty(),
    }
}

fn message_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter(|block| matches!(block.get("type").and_then(Value::as_str), Some("text" | "input_text" | "output_text")))
            .map(|block| block.get("text").and_then(Value::as_str).unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

pub fn parse(data: &[u8], provider: Provider) -> Result<Parsed> {
    let mut result = Parsed::default();
    let mut seen = HashSet::new();
    let mut worker = false;
    let lines = split_lines(data);
    let count = lines.len();
    for (index, raw) in lines.iter().enumerate() {
        let number = index + 1;
        if number == count && !raw.ends_with(b"\n") {
            break;
        }
        let invalid = || Error::permanent(format!("원문 L{number}: JSON 형식이 올바르지 않습니다."));
        let line = std::str::from_utf8(raw).map_err(|_| invalid())?.trim_start_matches('\u{feff}');
        if line.trim().is_empty() {
            result.complete_lines = number;
            continue;
        }
        let record: Value = serde_json::from_str(line).map_err(|_| invalid())?;
        result.complete_lines = number;
        let (role, text, identifier, mut origin) = match provider {
            Provider::Codex => {
                let payload = record.get("payload").cloned().unwrap_or(Value::Null);
                let kind = record.get("type").and_then(Value::as_str);
                if kind == Some("session_meta") {
                    let id = text_field(&payload, "id");
                    result.session_id = if id.is_empty() { text_field(&payload, "session_id") } else { id };
                    result.cwd = text_field(&payload, "cwd");
                    worker = payload.get("source").filter(|source| source.is_object()).map(|source| truthy(source.get("subagent"))).unwrap_or(false);
                    if worker {
                        result.worker_session = true;
                    }
                    continue;
                }
                if kind != Some("response_item") || payload.get("type").and_then(Value::as_str) != Some("message") {
                    continue;
                }
                let kinds = payload.get("internal_chat_message_metadata_passthrough").and_then(|meta| meta.get("content_item_kinds")).and_then(Value::as_array);
                let typed_by_user = kinds.map(|items| items.iter().any(|item| item.as_str() == Some("user.text"))).unwrap_or(false);
                let origin = if typed_by_user && !worker { Origin::User } else { Origin::Unverified };
                (text_field(&payload, "role"), message_text(payload.get("content")), text_field(&payload, "id"), origin)
            }
            Provider::Claude => {
                let text = match record.get("message") {
                    Some(message) if message.is_object() => message_text(message.get("content")),
                    _ => String::new(),
                };
                if result.session_id.is_empty() {
                    result.session_id = text_field(&record, "sessionId");
                }
                if result.cwd.is_empty() {
                    result.cwd = text_field(&record, "cwd");
                }
                let external = record.get("userType").and_then(Value::as_str) == Some("external");
                let mut origin = if external && !truthy(record.get("isSidechain")) && !worker { Origin::User } else { Origin::Unverified };
                if truthy(record.get("isCompactSummary")) || truthy(record.get("isMeta")) {
                    origin = Origin::Context;
                }
                (text_field(&record, "type"), text, text_field(&record, "uuid"), origin)
            }
        };
        if (role != "user" && role != "assistant") || text.trim().is_empty() {
            continue;
        }
        if role == "user" {
            let start = text.trim_start();
            if INJECTED_PREFIXES.iter().any(|prefix| start.starts_with(prefix)) {
                continue;
            }
            if is_agent_message(start) {
                origin = Origin::Agent;
            }
        }
        if !identifier.is_empty() && !seen.insert(identifier) {
            continue;
        }
        if role == "assistant" {
            origin = Origin::Assistant;
        }
        result.turns.push(Turn {
            reference: format!("L{number}"),
            line: number,
            role,
            origin,
            timestamp: text_field(&record, "timestamp"),
            text,
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn jsonl(records: &[Value]) -> Vec<u8> {
        records.iter().flat_map(|record| format!("{record}\n").into_bytes()).collect()
    }

    fn codex_message(role: &str, text: &str) -> Value {
        let kind = if role == "user" { "input_text" } else { "output_text" };
        json!({"type": "response_item", "payload": {"type": "message", "role": role, "content": [{"type": kind, "text": text}]}})
    }

    fn typed(mut message: Value) -> Value {
        message["payload"]["internal_chat_message_metadata_passthrough"] = json!({"content_item_kinds": ["user.text"]});
        message
    }

    fn summary(parsed: &Parsed) -> Vec<(String, String, Origin)> {
        parsed.turns.iter().map(|turn| (turn.reference.clone(), turn.role.clone(), turn.origin)).collect()
    }

    #[test]
    fn codex_separates_typed_user_input_from_other_input() {
        let data = jsonl(&[
            json!({"type": "session_meta", "payload": {"id": "session-a", "cwd": "C:/demo"}}),
            codex_message("user", "# AGENTS.md instructions\nignore"),
            typed(codex_message("user", "경고는 빼고 로그만 남기십시오.")),
            codex_message("assistant", "로그만 남기는 것으로 변경했습니다."),
            codex_message("user", "작업자에게 전달된 지시"),
            json!({"type": "response_item", "payload": {"type": "reasoning", "text": "private"}}),
        ]);
        let parsed = parse(&data, Provider::Codex).unwrap();
        assert_eq!((parsed.session_id.as_str(), parsed.cwd.as_str()), ("session-a", "C:/demo"));
        assert_eq!(
            summary(&parsed),
            vec![
                ("L3".into(), "user".into(), Origin::User),
                ("L4".into(), "assistant".into(), Origin::Assistant),
                ("L5".into(), "user".into(), Origin::Unverified),
            ]
        );
        assert!(!parsed.worker_session);
    }

    #[test]
    fn claude_omits_tool_results_and_marks_compaction_as_context() {
        let data = jsonl(&[
            json!({"type": "user", "uuid": "a", "sessionId": "session-b", "cwd": "C:/demo", "userType": "external", "message": {"role": "user", "content": "알림을 제외하십시오."}}),
            json!({"type": "user", "uuid": "tool", "message": {"content": [{"type": "tool_result", "content": "output"}]}}),
            json!({"type": "user", "uuid": "summary", "isCompactSummary": true, "message": {"content": "이전 대화 요약"}}),
            json!({"type": "assistant", "uuid": "b", "message": {"content": [{"type": "thinking", "thinking": "private"}, {"type": "text", "text": "제외하겠습니다."}]}}),
            json!({"type": "assistant", "uuid": "b", "message": {"content": [{"type": "text", "text": "제외하겠습니다."}]}}),
        ]);
        let parsed = parse(&data, Provider::Claude).unwrap();
        let turns: Vec<_> = parsed.turns.iter().map(|turn| (turn.reference.as_str(), turn.origin, turn.text.as_str())).collect();
        assert_eq!(
            turns,
            vec![("L1", Origin::User, "알림을 제외하십시오."), ("L3", Origin::Context, "이전 대화 요약"), ("L4", Origin::Assistant, "제외하겠습니다.")]
        );
        assert_eq!(parsed.session_id, "session-b");
    }

    #[test]
    fn codex_subagent_session_is_a_worker_session() {
        let data = jsonl(&[
            json!({"type": "session_meta", "payload": {"id": "worker", "source": {"subagent": {"parent_thread_id": "p"}}}}),
            typed(codex_message("user", "경고를 추가하십시오.")),
        ]);
        let parsed = parse(&data, Provider::Codex).unwrap();
        assert!(parsed.worker_session);
        assert_eq!(parsed.turns[0].origin, Origin::Unverified);
    }

    #[test]
    fn orca_instructions_are_labelled_per_input_and_typed_followups_stay_user_input() {
        let data = jsonl(&[
            json!({"type": "user", "uuid": "a", "userType": "external", "message": {"content": "You are working inside Orca as a dispatched worker"}}),
            json!({"type": "user", "uuid": "b", "userType": "external", "message": {"content": "기존 경고를 없애십시오."}}),
            json!({"type": "user", "uuid": "c", "userType": "external", "message": {"content": "Please carry out this task from my Orca coordinator by following"}}),
            json!({"type": "user", "uuid": "d", "userType": "external", "message": {"content": "Continue work from the prior Orca session using the context"}}),
            json!({"type": "user", "uuid": "e", "userType": "external", "message": {"content": "  You are a dispatched worker"}}),
        ]);
        let parsed = parse(&data, Provider::Claude).unwrap();
        assert!(!parsed.worker_session);
        assert_eq!(
            parsed.turns.iter().map(|turn| turn.origin).collect::<Vec<_>>(),
            vec![Origin::Agent, Origin::User, Origin::Agent, Origin::Agent, Origin::Agent]
        );
    }

    #[test]
    fn other_fixed_relay_messages_are_agent_messages_but_ordinary_typing_is_not() {
        for text in [
            "You have 1 orchestration message. Run `orca orchestration check`.",
            "You have 2 orchestration messages. Run `orca orchestration check`.",
            "Another Claude session sent a message:\n<teammate-message teammate_id=\"x\">",
            "The prior provider session is read-only context; do not resume or modify it.",
            "Original agent: codex",
        ] {
            assert!(is_agent_message(text), "{text}");
        }
        for text in ["You have to keep the log only.", "v11 스킬 쓰면서 발생하는 버그들 모니터링해.", "orchestration message 확인해봐"] {
            assert!(!is_agent_message(text), "{text}");
        }
    }

    #[test]
    fn codex_orca_instruction_typed_as_user_text_is_still_labelled_orca() {
        let data = jsonl(&[
            json!({"type": "session_meta", "payload": {"id": "s", "cwd": "D:/w"}}),
            typed(codex_message("user", "You are working inside Orca, a multi-agent IDE.")),
            typed(codex_message("user", "로그만 남기십시오.")),
        ]);
        let parsed = parse(&data, Provider::Codex).unwrap();
        assert_eq!(parsed.turns.iter().map(|turn| turn.origin).collect::<Vec<_>>(), vec![Origin::Agent, Origin::User]);
    }

    #[test]
    fn incomplete_tail_is_not_consumed_and_bad_middle_line_fails() {
        let mut data = jsonl(&[codex_message("assistant", "완료")]);
        data.extend_from_slice(b"{\"type\":");
        assert_eq!(parse(&data, Provider::Codex).unwrap().complete_lines, 1);
        let mut broken = b"{broken}\n".to_vec();
        broken.extend(jsonl(&[codex_message("assistant", "완료")]));
        let error = parse(&broken, Provider::Codex).unwrap_err();
        assert_eq!(error.failure, crate::error::Failure::Permanent);
        assert!(error.message.contains("L1"));
    }

    #[test]
    fn tail_without_newline_waits_for_the_writer() {
        let data = jsonl(&[codex_message("assistant", "완료")]);
        let unterminated = &data[..data.len() - 1];
        assert_eq!(parse(unterminated, Provider::Codex).unwrap().complete_lines, 0);
        assert_eq!(parse(&data, Provider::Codex).unwrap().complete_lines, 1);
    }

    #[test]
    fn changed_processed_prefix_is_rejected() {
        let original = b"first\nsecond\n";
        let hash = prefix_hash(original, 1);
        let mut appended = original.to_vec();
        appended.extend_from_slice(b"third\n");
        check_prefix(&appended, 1, &hash).unwrap();
        assert!(check_prefix(b"changed\nsecond\n", 1, &hash).is_err());
        check_prefix(b"anything\n", 0, "").unwrap();
    }

    #[test]
    fn compressed_transcript_is_read_when_original_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout.jsonl");
        let data = jsonl(&[codex_message("assistant", "완료")]);
        std::fs::write(compressed_path(&path), zstd::stream::encode_all(&data[..], 3).unwrap()).unwrap();
        assert_eq!(read_transcript(&path).unwrap(), data);
        assert_eq!(canonical_path(&compressed_path(&path)), path);
        let missing = read_transcript(&dir.path().join("missing.jsonl")).unwrap_err();
        assert_eq!(missing.failure, crate::error::Failure::Permanent);
    }
}
