use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::error::{Error, Result};
use crate::settings::{atomic_write, home};
use crate::transcript::Provider;

pub const HOOK_TIMEOUT_SECS: u64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HookState {
    Connected,
    OtherPath,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookStatus {
    pub provider: Provider,
    pub state: HookState,
    pub config: String,
    pub command: String,
    pub found: Option<String>,
    pub error: Option<String>,
}

pub const EVENTS: [&str; 2] = ["SessionEnd", "PreCompact"];

pub fn config_path(provider: Provider) -> PathBuf {
    match provider {
        Provider::Claude => home().join(".claude").join("settings.json"),
        Provider::Codex => home().join(".codex").join("hooks.json"),
    }
}

pub fn command(cli: &Path, provider: Provider) -> String {
    let path = cli.to_string_lossy().replace('\\', "/");
    // POSIX shells expand $, backticks and command substitutions inside double
    // quotes. Keep paths literal, including installations with apostrophes.
    let program = if cfg!(windows) {
        if path.contains(' ') { format!("\"{path}\"") } else { path }
    } else {
        format!("'{}'", path.replace('\'', "'\"'\"'"))
    };
    format!("{program} hook {}", provider.as_str())
}

fn is_ours(command: &str, provider: Provider) -> bool {
    command.to_ascii_lowercase().contains("qmd-memory-cli") && command.trim_end().ends_with(&format!("hook {}", provider.as_str()))
}

fn parse(text: Option<&str>) -> Result<Value> {
    match text.map(|text| text.trim_start_matches('\u{feff}').trim()).filter(|text| !text.is_empty()) {
        None => Ok(Value::Object(Map::new())),
        Some(text) => {
            let value: Value = serde_json::from_str(text).map_err(|error| Error::permanent(format!("훅 설정 파일의 JSON을 읽지 못했습니다: {error}")))?;
            if value.is_object() {
                Ok(value)
            } else {
                Err(Error::permanent("훅 설정 파일의 최상위가 JSON 객체가 아닙니다."))
            }
        }
    }
}

fn event_commands(root: &Value, event: &str) -> Vec<String> {
    root.get("hooks")
        .and_then(|hooks| hooks.get(event))
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.get("hooks").and_then(Value::as_array))
                .flatten()
                .filter_map(|hook| hook.get("command").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

pub fn inspect(text: Option<&str>, provider: Provider, wanted: &str) -> Result<(HookState, Option<String>)> {
    let root = parse(text)?;
    let mut all_connected = true;
    let mut other = None;
    for event in EVENTS {
        let commands = event_commands(&root, event);
        if !commands.iter().any(|command| command == wanted) {
            all_connected = false;
        }
        if other.is_none() {
            other = commands.into_iter().find(|command| command != wanted && is_ours(command, provider));
        }
    }
    Ok(match (all_connected, other) {
        (true, _) => (HookState::Connected, Some(wanted.to_string())),
        (false, Some(found)) => (HookState::OtherPath, Some(found)),
        (false, None) => (HookState::Missing, None),
    })
}

pub fn add_hooks(text: Option<&str>, provider: Provider, wanted: &str) -> Result<String> {
    let mut root = parse(text)?;
    let object = root.as_object_mut().ok_or_else(|| Error::permanent("훅 설정 파일의 최상위가 JSON 객체가 아닙니다."))?;
    let hooks = object.entry("hooks").or_insert_with(|| Value::Object(Map::new()));
    let hooks = hooks.as_object_mut().ok_or_else(|| Error::permanent("훅 설정 파일의 hooks가 JSON 객체가 아닙니다."))?;
    for event in EVENTS {
        let entries = hooks.entry(event).or_insert_with(|| Value::Array(Vec::new()));
        let entries = entries.as_array_mut().ok_or_else(|| Error::permanent(format!("훅 설정 파일의 {event} 항목이 배열이 아닙니다.")))?;
        for entry in entries.iter_mut() {
            if let Some(list) = entry.get_mut("hooks").and_then(Value::as_array_mut) {
                list.retain(|hook| hook.get("command").and_then(Value::as_str).map(|command| command == wanted || !is_ours(command, provider)).unwrap_or(true));
            }
        }
        entries.retain(|entry| entry.get("hooks").and_then(Value::as_array).map(|list| !list.is_empty()).unwrap_or(true));
        let present = entries
            .iter()
            .filter_map(|entry| entry.get("hooks").and_then(Value::as_array))
            .flatten()
            .any(|hook| hook.get("command").and_then(Value::as_str) == Some(wanted));
        if !present {
            entries.push(json!({"hooks": [{"type": "command", "command": wanted, "timeout": HOOK_TIMEOUT_SECS}]}));
        }
    }
    Ok(format!("{}\n", serde_json::to_string_pretty(&root)?))
}

pub fn remove_hooks(text: Option<&str>, provider: Provider) -> Result<(String, usize)> {
    let mut root = parse(text)?;
    let mut removed = 0;
    if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        let mut emptied = Vec::new();
        for (event, entries) in hooks.iter_mut() {
            let Some(entries) = entries.as_array_mut() else { continue };
            let before = entries.len();
            for entry in entries.iter_mut() {
                if let Some(list) = entry.get_mut("hooks").and_then(Value::as_array_mut) {
                    let count = list.len();
                    list.retain(|hook| !hook.get("command").and_then(Value::as_str).map(|command| is_ours(command, provider)).unwrap_or(false));
                    removed += count - list.len();
                }
            }
            entries.retain(|entry| entry.get("hooks").and_then(Value::as_array).map(|list| !list.is_empty()).unwrap_or(true));
            if before > 0 && entries.is_empty() {
                emptied.push(event.clone());
            }
        }
        for event in emptied {
            hooks.remove(&event);
        }
    }
    Ok((format!("{}\n", serde_json::to_string_pretty(&root)?), removed))
}

fn write_backup(config: &Path, text: &str, now: i64) -> Result<PathBuf> {
    let mut suffix = 0;
    loop {
        let mut name = config.as_os_str().to_owned();
        name.push(if suffix == 0 { format!(".qmd-memory-backup-{now}") } else { format!(".qmd-memory-backup-{now}-{suffix}") });
        let path = PathBuf::from(name);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                std::io::Write::write_all(&mut file, text.as_bytes())?;
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => suffix += 1,
            Err(error) => return Err(error.into()),
        }
    }
}

pub fn unregister_at(config: &Path, provider: Provider, now: i64) -> Result<usize> {
    let Some(original) = read_optional(config)? else { return Ok(0) };
    let (updated, removed) = remove_hooks(Some(&original), provider)?;
    if removed == 0 {
        return Ok(0);
    }
    write_backup(config, &original, now)?;
    atomic_write(config, updated.as_bytes())?;
    Ok(removed)
}

fn read_optional(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub fn status_at(config: &Path, provider: Provider, cli: &Path) -> HookStatus {
    let wanted = command(cli, provider);
    let config_text = config.to_string_lossy().replace('\\', "/");
    match read_optional(config).and_then(|text| inspect(text.as_deref(), provider, &wanted)) {
        Ok((state, found)) => HookStatus { provider, state, config: config_text, command: wanted, found, error: None },
        Err(error) => HookStatus { provider, state: HookState::Missing, config: config_text, command: wanted, found: None, error: Some(error.message) },
    }
}

pub fn register_at(config: &Path, provider: Provider, cli: &Path, now: i64) -> Result<Option<PathBuf>> {
    let original = read_optional(config)?;
    let updated = add_hooks(original.as_deref(), provider, &command(cli, provider))?;
    let backup = match &original {
        Some(text) => Some(write_backup(config, text, now)?),
        None => None,
    };
    atomic_write(config, updated.as_bytes())?;
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLI: &str = "C:/Apps/qmd-memory/qmd-memory-cli.exe";

    fn wanted(provider: Provider) -> String {
        command(Path::new(CLI), provider)
    }

    #[test]
    fn hook_commands_use_platform_appropriate_quoting() {
        if cfg!(windows) {
            assert_eq!(wanted(Provider::Claude), "C:/Apps/qmd-memory/qmd-memory-cli.exe hook claude");
            assert_eq!(command(Path::new("C:\\Program Files\\q\\qmd-memory-cli.exe"), Provider::Codex), "\"C:/Program Files/q/qmd-memory-cli.exe\" hook codex");
        } else {
            assert_eq!(wanted(Provider::Claude), "'C:/Apps/qmd-memory/qmd-memory-cli.exe' hook claude");
            assert_eq!(command(Path::new("/Users/it's $me/`apps`/qmd-memory-cli"), Provider::Codex), "'/Users/it'\"'\"'s $me/`apps`/qmd-memory-cli' hook codex");
        }
    }

    #[test]
    #[cfg(unix)]
    fn posix_hook_executes_a_literal_path_with_shell_metacharacters() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let cli = dir.path().join("it's $HOME `date`").join("qmd-memory-cli");
        std::fs::create_dir_all(cli.parent().unwrap()).unwrap();
        std::fs::write(&cli, "#!/bin/sh\nprintf '%s\\n' \"$@\"\n").unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
        let output = std::process::Command::new("/bin/sh").args(["-c", &command(&cli, Provider::Claude)]).output().unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8(output.stdout).unwrap(), "hook\nclaude\n");
    }

    #[test]
    fn adding_keeps_existing_settings_and_hooks_in_order() {
        let existing = r#"{"model": "opus", "hooks": {"SessionEnd": [{"hooks": [{"type": "command", "command": "orca-agent-hook.exe claude", "timeout": 10}]}], "Stop": []}, "theme": "dark"}"#;
        let updated = add_hooks(Some(existing), Provider::Claude, &wanted(Provider::Claude)).unwrap();
        let value: Value = serde_json::from_str(&updated).unwrap();
        let keys: Vec<&String> = value.as_object().unwrap().keys().collect();
        assert_eq!(keys, vec!["model", "hooks", "theme"]);
        let session_end = event_commands(&value, "SessionEnd");
        assert_eq!(session_end, vec!["orca-agent-hook.exe claude".to_string(), wanted(Provider::Claude)]);
        assert_eq!(event_commands(&value, "PreCompact"), vec![wanted(Provider::Claude)]);
        assert!(value["hooks"]["Stop"].as_array().unwrap().is_empty());
        assert_eq!(add_hooks(Some(&updated), Provider::Claude, &wanted(Provider::Claude)).unwrap(), updated);
    }

    #[test]
    fn both_tools_are_hooked_on_session_end_and_before_compaction() {
        for provider in [Provider::Claude, Provider::Codex] {
            let value: Value = serde_json::from_str(&add_hooks(None, provider, &wanted(provider)).unwrap()).unwrap();
            assert_eq!(event_commands(&value, "SessionEnd"), vec![wanted(provider)]);
            assert_eq!(event_commands(&value, "PreCompact"), vec![wanted(provider)]);
        }
    }

    #[test]
    fn a_codex_connection_with_only_session_end_is_completed_by_reconnecting() {
        let old = format!(r#"{{"hooks": {{"SessionEnd": [{{"hooks": [{{"type": "command", "command": "{}", "timeout": 10}}]}}]}}}}"#, wanted(Provider::Codex));
        assert_eq!(inspect(Some(&old), Provider::Codex, &wanted(Provider::Codex)).unwrap().0, HookState::Missing);
        let updated = add_hooks(Some(&old), Provider::Codex, &wanted(Provider::Codex)).unwrap();
        let value: Value = serde_json::from_str(&updated).unwrap();
        assert_eq!(event_commands(&value, "SessionEnd"), vec![wanted(Provider::Codex)]);
        assert_eq!(event_commands(&value, "PreCompact"), vec![wanted(Provider::Codex)]);
        assert_eq!(inspect(Some(&updated), Provider::Codex, &wanted(Provider::Codex)).unwrap().0, HookState::Connected);
    }

    #[test]
    fn stale_install_path_is_replaced_not_duplicated() {
        let stale = r#"{"hooks": {"SessionEnd": [{"hooks": [{"type": "command", "command": "D:/old/qmd-memory-cli.exe hook codex", "timeout": 10}]}]}}"#;
        assert_eq!(inspect(Some(stale), Provider::Codex, &wanted(Provider::Codex)).unwrap(), (HookState::OtherPath, Some("D:/old/qmd-memory-cli.exe hook codex".into())));
        let updated = add_hooks(Some(stale), Provider::Codex, &wanted(Provider::Codex)).unwrap();
        let value: Value = serde_json::from_str(&updated).unwrap();
        assert_eq!(event_commands(&value, "SessionEnd"), vec![wanted(Provider::Codex)]);
        assert_eq!(value["hooks"]["SessionEnd"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn state_is_connected_only_when_every_event_has_the_command() {
        let only_end = format!(r#"{{"hooks": {{"SessionEnd": [{{"hooks": [{{"type": "command", "command": "{}"}}]}}]}}}}"#, wanted(Provider::Claude));
        assert_eq!(inspect(Some(&only_end), Provider::Claude, &wanted(Provider::Claude)).unwrap().0, HookState::Missing);
        assert_eq!(inspect(Some(&only_end), Provider::Codex, &wanted(Provider::Codex)).unwrap().0, HookState::Missing);
        let both = add_hooks(Some(&only_end), Provider::Claude, &wanted(Provider::Claude)).unwrap();
        assert_eq!(inspect(Some(&both), Provider::Claude, &wanted(Provider::Claude)).unwrap().0, HookState::Connected);
        assert_eq!(inspect(None, Provider::Codex, &wanted(Provider::Codex)).unwrap().0, HookState::Missing);
    }

    #[test]
    fn removal_takes_out_only_our_hooks_and_restores_the_original_shape() {
        let original = r#"{"model": "opus", "hooks": {"SessionEnd": [{"hooks": [{"type": "command", "command": "orca-agent-hook.exe claude", "timeout": 10}]}], "Stop": []}}"#;
        let connected = add_hooks(Some(original), Provider::Claude, &wanted(Provider::Claude)).unwrap();
        let (removed_text, removed) = remove_hooks(Some(&connected), Provider::Claude).unwrap();
        assert_eq!(removed, 2);
        let value: Value = serde_json::from_str(&removed_text).unwrap();
        assert_eq!(event_commands(&value, "SessionEnd"), vec!["orca-agent-hook.exe claude".to_string()]);
        assert!(value["hooks"].get("PreCompact").is_none());
        assert!(value["hooks"]["Stop"].as_array().unwrap().is_empty());
        assert_eq!(value["model"], "opus");
        let expected: Value = serde_json::from_str(original).unwrap();
        assert_eq!(value, expected);
        assert_eq!(remove_hooks(Some(&removed_text), Provider::Claude).unwrap().1, 0);
    }

    #[test]
    fn removal_leaves_the_other_tool_and_old_paths_are_removed_too() {
        let both = add_hooks(Some(&add_hooks(None, Provider::Codex, &wanted(Provider::Codex)).unwrap()), Provider::Claude, &wanted(Provider::Claude)).unwrap();
        let (text, removed) = remove_hooks(Some(&both), Provider::Codex).unwrap();
        assert_eq!(removed, 2);
        assert_eq!(inspect(Some(&text), Provider::Claude, &wanted(Provider::Claude)).unwrap().0, HookState::Connected);
        let stale = r#"{"hooks": {"SessionEnd": [{"hooks": [{"type": "command", "command": "D:/old/qmd-memory-cli.exe hook codex"}]}]}}"#;
        assert_eq!(remove_hooks(Some(stale), Provider::Codex).unwrap().1, 1);
    }

    #[test]
    fn unregistering_backs_up_and_does_nothing_when_not_connected() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("hooks.json");
        assert_eq!(unregister_at(&config, Provider::Codex, 1).unwrap(), 0);
        register_at(&config, Provider::Codex, Path::new(CLI), 2).unwrap();
        let connected = std::fs::read_to_string(&config).unwrap();
        assert_eq!(unregister_at(&config, Provider::Codex, 3).unwrap(), 2);
        assert_eq!(std::fs::read_to_string(dir.path().join("hooks.json.qmd-memory-backup-3")).unwrap(), connected);
        assert_eq!(status_at(&config, Provider::Codex, Path::new(CLI)).state, HookState::Missing);
        assert_eq!(unregister_at(&config, Provider::Codex, 4).unwrap(), 0);
        assert!(!dir.path().join("hooks.json.qmd-memory-backup-4").exists());
    }

    #[test]
    fn backups_made_in_the_same_second_do_not_overwrite_each_other() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("settings.json");
        std::fs::write(&config, "{\"model\": \"opus\"}\n").unwrap();
        let first = register_at(&config, Provider::Claude, Path::new(CLI), 9).unwrap().unwrap();
        let connected = std::fs::read_to_string(&config).unwrap();
        unregister_at(&config, Provider::Claude, 9).unwrap();
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "{\"model\": \"opus\"}\n");
        assert_eq!(std::fs::read_to_string(dir.path().join("settings.json.qmd-memory-backup-9-1")).unwrap(), connected);
    }

    #[test]
    fn malformed_files_are_refused_and_registration_keeps_a_backup() {
        assert!(add_hooks(Some("{broken"), Provider::Claude, "x").is_err());
        assert!(add_hooks(Some("[]"), Provider::Claude, "x").is_err());
        assert!(add_hooks(Some(r#"{"hooks": []}"#), Provider::Claude, "x").is_err());

        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("settings.json");
        assert_eq!(register_at(&config, Provider::Codex, Path::new(CLI), 7).unwrap(), None);
        assert_eq!(status_at(&config, Provider::Codex, Path::new(CLI)).state, HookState::Connected);
        let before = std::fs::read_to_string(&config).unwrap();
        let backup = register_at(&config, Provider::Claude, Path::new(CLI), 8).unwrap().unwrap();
        assert_eq!(std::fs::read_to_string(backup).unwrap(), before);
        assert_eq!(status_at(&config, Provider::Claude, Path::new(CLI)).state, HookState::Connected);
        std::fs::write(&config, "{broken").unwrap();
        let status = status_at(&config, Provider::Claude, Path::new(CLI));
        assert!(status.error.is_some());
    }
}
