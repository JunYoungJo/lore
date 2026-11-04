use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::error::Result;
use crate::queue::Queue;
use crate::settings::Settings;
use crate::transcript::{canonical_path, Provider};

fn transcripts(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(folder) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else { continue };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else { continue };
            let name = entry.file_name().to_string_lossy().to_string();
            if kind.is_dir() {
                if name != "subagents" {
                    stack.push(entry.path());
                }
            } else if kind.is_file() && (name.ends_with(".jsonl") || name.ends_with(".jsonl.zst")) {
                found.push(entry.path());
            }
        }
    }
    found.sort();
    found
}

pub fn scan(queue: &Queue, settings: &Settings, now: i64) -> Result<usize> {
    let mut count = 0;
    let roots = [(Provider::Claude, &settings.claude_projects_dir), (Provider::Codex, &settings.codex_sessions_dir)];
    for (provider, root) in roots {
        for path in transcripts(Path::new(root)) {
            let Ok(metadata) = std::fs::metadata(&path) else { continue };
            let modified = metadata.modified().ok().and_then(|time| time.duration_since(UNIX_EPOCH).ok()).map(|elapsed| elapsed.as_secs() as i64).unwrap_or(0);
            if modified < settings.since || now - modified < settings.idle_secs as i64 {
                continue;
            }
            let key = canonical_path(&path).to_string_lossy().to_string();
            let size = metadata.len() as i64;
            if queue.seen(&key)? == Some((size, modified)) {
                continue;
            }
            queue.enqueue(provider, &path, now)?;
            queue.mark_seen(&key, size, modified)?;
            count += 1;
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;

    #[test]
    fn idle_sessions_after_the_start_time_are_queued_once_and_subagents_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join("claude");
        let codex = dir.path().join("codex");
        let session = claude.join("D--src-demo").join("a.jsonl");
        let subagent = claude.join("D--src-demo").join("a").join("subagents").join("agent-1.jsonl");
        let compressed = codex.join("2026").join("rollout-x.jsonl.zst");
        let stamp: i64 = 1_800_000_000;
        for file in [&session, &subagent, &compressed] {
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, "{}\n").unwrap();
            fs::File::options().write(true).open(file).unwrap().set_modified(UNIX_EPOCH + Duration::from_secs(stamp as u64)).unwrap();
        }
        let queue = Queue::open(&dir.path().join("state.sqlite")).unwrap();
        let settings = Settings {
            claude_projects_dir: claude.to_string_lossy().to_string(),
            codex_sessions_dir: codex.to_string_lossy().to_string(),
            since: 0,
            idle_secs: 600,
            ..Settings::default()
        };
        assert_eq!(scan(&queue, &settings, stamp + 10).unwrap(), 0);
        assert_eq!(scan(&queue, &settings, stamp + 600).unwrap(), 2);
        assert_eq!(scan(&queue, &settings, stamp + 700).unwrap(), 0);
        assert_eq!(queue.counts(stamp + 700).unwrap().ready, 2);
        let later = Settings { since: stamp + 1, ..settings };
        let fresh = Queue::open(&dir.path().join("other.sqlite")).unwrap();
        assert_eq!(scan(&fresh, &later, stamp + 700).unwrap(), 0);
    }
}
