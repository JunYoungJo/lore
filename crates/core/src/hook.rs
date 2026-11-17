use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::json;

use crate::error::{Error, Result};
use crate::mark;
use crate::transcript::{canonical_path, Provider};

#[derive(Debug, Deserialize)]
struct Event {
    hook_event_name: Option<String>,
    transcript_path: Option<String>,
    session_id: Option<String>,
}

/// What a hook call asks of Lore.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hook {
    /// A session ended or is about to be compacted: its file is to be read.
    Session { transcript: PathBuf, ended: bool },
    /// A prompt is about to be sent: the session is told how to mark what is worth remembering.
    Prompt,
    Ignored,
}

pub fn find_codex_session(root: &Path, session_id: &str) -> Option<PathBuf> {
    if session_id.is_empty() {
        return None;
    }
    let mut stack = vec![root.to_path_buf()];
    let mut found: Option<(std::time::SystemTime, PathBuf)> = None;
    while let Some(folder) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                stack.push(path);
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.contains(session_id) || !(name.ends_with(".jsonl") || name.ends_with(".jsonl.zst")) {
                continue;
            }
            let modified = entry.metadata().and_then(|meta| meta.modified()).unwrap_or(std::time::UNIX_EPOCH);
            if found.as_ref().map(|(time, _)| modified > *time).unwrap_or(true) {
                found = Some((modified, canonical_path(&path)));
            }
        }
    }
    found.map(|(_, path)| path)
}

/// Read the event a hook passes on its standard input. `codex_sessions_dir` is asked for only
/// when a Codex event names a session without its file. A prompt hook runs on every prompt, so
/// it must not depend on the settings being readable.
pub fn read(provider: Provider, event_json: &str, codex_sessions_dir: impl FnOnce() -> Result<PathBuf>) -> Result<Hook> {
    let event: Event = serde_json::from_str(event_json.trim_start_matches('\u{feff}').trim()).map_err(|error| Error::permanent(format!("Could not read hook input: {error}")))?;
    let ended = match event.hook_event_name.as_deref() {
        Some("UserPromptSubmit") => return Ok(Hook::Prompt),
        Some("SessionEnd") => true,
        Some("PreCompact") => false,
        _ => return Ok(Hook::Ignored),
    };
    let transcript = match (event.transcript_path.filter(|path| !path.trim().is_empty()), event.session_id.as_deref()) {
        (Some(path), _) => Some(PathBuf::from(path)),
        (None, Some(id)) if provider == Provider::Codex && !id.is_empty() => find_codex_session(&codex_sessions_dir()?, id),
        _ => None,
    };
    Ok(transcript.map_or(Hook::Ignored, |transcript| Hook::Session { transcript, ended }))
}

/// What the hook prints for a submitted prompt. Claude Code and Codex both add it to the prompt.
pub fn prompt_output(cli: &Path) -> String {
    let instruction = mark::instruction(&mark::shell_command(&cli.to_string_lossy()));
    json!({"hookSpecificOutput": {"hookEventName": "UserPromptSubmit", "additionalContext": instruction}}).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(path: &str, ended: bool) -> Hook {
        Hook::Session { transcript: PathBuf::from(path), ended }
    }

    /// The sessions folder of an event that never needs it.
    fn unused() -> Result<PathBuf> {
        panic!("this event names its session file, so the sessions folder is not looked up")
    }

    #[test]
    fn session_end_with_a_transcript_path_is_read_as_an_ended_session() {
        let event = r#"{"hook_event_name": "SessionEnd", "transcript_path": "C:/s/a.jsonl", "session_id": "a", "cwd": "D:/src/x", "reason": "exit"}"#;
        assert_eq!(read(Provider::Claude, event, unused).unwrap(), session("C:/s/a.jsonl", true));
        let bom = format!("\u{feff}{event}");
        assert_eq!(read(Provider::Claude, &bom, unused).unwrap(), session("C:/s/a.jsonl", true));
    }

    #[test]
    fn a_compaction_is_read_as_a_session_that_goes_on() {
        let event = r#"{"hook_event_name": "PreCompact", "session_id": "0aa1", "transcript_path": "C:/u/.codex/sessions/2026/11/05/rollout-x-0aa1.jsonl", "cwd": "D:/src/x", "model": "gpt", "turn_id": "t1", "trigger": "auto"}"#;
        assert_eq!(read(Provider::Codex, event, unused).unwrap(), session("C:/u/.codex/sessions/2026/11/05/rollout-x-0aa1.jsonl", false));
    }

    #[test]
    fn a_submitted_prompt_gets_the_marking_instruction_without_reading_any_settings() {
        // Codex sends no transcript path here on some versions; the prompt still needs no lookup.
        for event in [
            r#"{"hook_event_name": "UserPromptSubmit", "session_id": "a", "transcript_path": "C:/s/a.jsonl", "prompt": "Do not delete without asking."}"#,
            r#"{"hook_event_name": "UserPromptSubmit", "session_id": "a", "prompt": "Do not delete without asking."}"#,
        ] {
            assert_eq!(read(Provider::Codex, event, unused).unwrap(), Hook::Prompt);
        }
        let output: serde_json::Value = serde_json::from_str(&prompt_output(Path::new("/Applications/Lore.app/Contents/MacOS/lore-cli"))).unwrap();
        assert_eq!(output["hookSpecificOutput"]["hookEventName"], "UserPromptSubmit");
        assert!(output["hookSpecificOutput"]["additionalContext"].as_str().unwrap().contains("/Applications/Lore.app/Contents/MacOS/lore-cli mark <decision|correction|lesson>"));
    }

    #[test]
    fn other_events_are_ignored() {
        let event = r#"{"hook_event_name": "Stop", "transcript_path": "C:/s/a.jsonl"}"#;
        assert_eq!(read(Provider::Claude, event, unused).unwrap(), Hook::Ignored);
        assert!(read(Provider::Claude, "not json", unused).is_err());
    }

    #[test]
    fn codex_event_without_a_path_is_resolved_by_session_id() {
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("2026").join("11").join("04");
        std::fs::create_dir_all(&day).unwrap();
        let file = day.join("rollout-2026-11-04T10-00-00-0aa1-bb.jsonl");
        std::fs::write(&file, "{}\n").unwrap();
        std::fs::write(day.join("rollout-2026-11-04T09-00-00-other.jsonl"), "{}\n").unwrap();
        let sessions = || Ok(dir.path().to_path_buf());
        let event = r#"{"hook_event_name": "SessionEnd", "transcript_path": null, "session_id": "0aa1-bb"}"#;
        assert_eq!(read(Provider::Codex, event, sessions).unwrap(), Hook::Session { transcript: file.clone(), ended: true });
        let compressed = day.join("rollout-2026-11-03T10-00-00-0cc2.jsonl.zst");
        std::fs::write(&compressed, "x").unwrap();
        let event = r#"{"hook_event_name": "SessionEnd", "session_id": "0cc2"}"#;
        assert_eq!(read(Provider::Codex, event, sessions).unwrap(), Hook::Session { transcript: day.join("rollout-2026-11-03T10-00-00-0cc2.jsonl"), ended: true });
        // Claude always names the file, so an event without one has nothing to read.
        assert_eq!(read(Provider::Claude, event, unused).unwrap(), Hook::Ignored);
        // Settings that cannot be read fail the session event, which the queue's `scan` can make up for.
        assert!(read(Provider::Codex, event, || Err(Error::permanent("settings.json is not valid JSON"))).is_err());
    }
}
