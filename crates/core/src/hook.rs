use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::transcript::{canonical_path, Provider};

#[derive(Debug, Deserialize)]
struct Event {
    hook_event_name: Option<String>,
    transcript_path: Option<String>,
    session_id: Option<String>,
}

const RECORDED_EVENTS: [&str; 2] = ["SessionEnd", "PreCompact"];

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

pub fn transcript_for(provider: Provider, event_json: &str, codex_sessions_dir: &Path) -> Result<Option<PathBuf>> {
    let event: Event = serde_json::from_str(event_json.trim_start_matches('\u{feff}').trim()).map_err(|error| Error::permanent(format!("훅 입력을 읽지 못했습니다: {error}")))?;
    if !event.hook_event_name.as_deref().map(|name| RECORDED_EVENTS.contains(&name)).unwrap_or(false) {
        return Ok(None);
    }
    if let Some(path) = event.transcript_path.filter(|path| !path.trim().is_empty()) {
        return Ok(Some(PathBuf::from(path)));
    }
    if provider == Provider::Codex {
        return Ok(event.session_id.as_deref().and_then(|id| find_codex_session(codex_sessions_dir, id)));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_end_with_a_transcript_path_is_accepted() {
        let event = r#"{"hook_event_name": "SessionEnd", "transcript_path": "C:/s/a.jsonl", "session_id": "a", "cwd": "D:/src/x", "reason": "exit"}"#;
        assert_eq!(transcript_for(Provider::Claude, event, Path::new("")).unwrap(), Some(PathBuf::from("C:/s/a.jsonl")));
        let bom = format!("\u{feff}{event}");
        assert!(transcript_for(Provider::Claude, &bom, Path::new("")).unwrap().is_some());
    }

    #[test]
    fn other_events_are_ignored() {
        let event = r#"{"hook_event_name": "Stop", "transcript_path": "C:/s/a.jsonl"}"#;
        assert_eq!(transcript_for(Provider::Claude, event, Path::new("")).unwrap(), None);
        assert!(transcript_for(Provider::Claude, "not json", Path::new("")).is_err());
    }

    #[test]
    fn codex_event_without_a_path_is_resolved_by_session_id() {
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("2026").join("11").join("04");
        std::fs::create_dir_all(&day).unwrap();
        let file = day.join("rollout-2026-11-04T10-00-00-0aa1-bb.jsonl");
        std::fs::write(&file, "{}\n").unwrap();
        std::fs::write(day.join("rollout-2026-11-04T09-00-00-other.jsonl"), "{}\n").unwrap();
        let event = r#"{"hook_event_name": "SessionEnd", "transcript_path": null, "session_id": "0aa1-bb"}"#;
        assert_eq!(transcript_for(Provider::Codex, event, dir.path()).unwrap(), Some(file.clone()));
        let compressed = day.join("rollout-2026-11-03T10-00-00-0cc2.jsonl.zst");
        std::fs::write(&compressed, "x").unwrap();
        let event = r#"{"hook_event_name": "SessionEnd", "session_id": "0cc2"}"#;
        assert_eq!(transcript_for(Provider::Codex, event, dir.path()).unwrap(), Some(day.join("rollout-2026-11-03T10-00-00-0cc2.jsonl")));
        assert_eq!(transcript_for(Provider::Claude, event, dir.path()).unwrap(), None);
    }
}
