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

/// `SessionEnd` and `PreCompact` put a session in the queue. `UserPromptSubmit` tells the session how to mark.
pub const EVENTS: [&str; 3] = ["SessionEnd", "PreCompact", "UserPromptSubmit"];

pub fn config_path(provider: Provider) -> PathBuf {
    match provider {
        Provider::Claude => home().join(".claude").join("settings.json"),
        Provider::Codex => std::env::var_os("CODEX_HOME").filter(|s| !s.is_empty()).map(PathBuf::from).unwrap_or_else(|| home().join(".codex")).join("hooks.json"),
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
    command.to_ascii_lowercase().contains("lore-cli") && command.trim_end().ends_with(&format!("hook {}", provider.as_str()))
}

/// The prompt hook tells every session to run `lore-cli mark`. Without a rule that allows it,
/// Claude Code asks the user each time, or in auto mode leaves it to a classifier that may refuse.
pub fn claude_rules(cli: &Path) -> Vec<String> {
    let mark = format!("{} mark", crate::mark::shell_command(&cli.to_string_lossy()));
    let mut rules = vec![format!("Bash({mark} *)")];
    if cfg!(windows) {
        rules.push(format!("PowerShell({mark} *)"));
    }
    rules
}

fn is_our_rule(rule: &str) -> bool {
    let lower = rule.to_ascii_lowercase();
    (rule.starts_with("Bash(") || rule.starts_with("PowerShell(")) && lower.contains("lore-cli") && lower.contains(" mark ")
}

/// Codex reads every `.rules` file in the `rules` folder next to its config, so Lore keeps its rule in a file of its own.
pub fn codex_rules_path(config: &Path) -> PathBuf {
    config.parent().unwrap_or_else(|| Path::new(".")).join("rules").join("lore.rules")
}

pub fn codex_rules(cli: &Path) -> String {
    let program = serde_json::to_string(&cli.to_string_lossy().replace('\\', "/")).unwrap_or_default();
    format!(
        "# Written by Lore when it connects to Codex, and removed when it disconnects.\n\
         prefix_rule(\n    pattern = [{program}, \"mark\"],\n    decision = \"allow\",\n    justification = \"lore-cli mark only prints one line, which Lore reads from the session later.\",\n)\n"
    )
}

fn allow_list(root: &mut Value) -> Result<&mut Vec<Value>> {
    let object = root.as_object_mut().ok_or_else(|| Error::permanent("The settings root must be a JSON object."))?;
    let permissions = object.entry("permissions").or_insert_with(|| Value::Object(Map::new()));
    let permissions = permissions.as_object_mut().ok_or_else(|| Error::permanent("The permissions field must be a JSON object."))?;
    permissions.entry("allow").or_insert_with(|| Value::Array(Vec::new())).as_array_mut().ok_or_else(|| Error::permanent("The permissions.allow field must be an array."))
}

pub fn add_rules(text: Option<&str>, rules: &[String]) -> Result<String> {
    let mut root = parse(text)?;
    let allow = allow_list(&mut root)?;
    allow.retain(|rule| rule.as_str().map(|rule| rules.iter().any(|wanted| wanted == rule) || !is_our_rule(rule)).unwrap_or(true));
    for rule in rules {
        if !allow.iter().any(|existing| existing.as_str() == Some(rule)) {
            allow.push(Value::String(rule.clone()));
        }
    }
    Ok(format!("{}\n", serde_json::to_string_pretty(&root)?))
}

pub fn remove_rules(text: Option<&str>) -> Result<(String, usize)> {
    let mut root = parse(text)?;
    let mut removed = 0;
    let mut emptied = false;
    if let Some(permissions) = root.get_mut("permissions").and_then(Value::as_object_mut) {
        if let Some(allow) = permissions.get_mut("allow").and_then(Value::as_array_mut) {
            let before = allow.len();
            allow.retain(|rule| !rule.as_str().map(is_our_rule).unwrap_or(false));
            removed = before - allow.len();
            if removed > 0 && allow.is_empty() {
                permissions.remove("allow");
            }
        }
        emptied = removed > 0 && permissions.is_empty();
    }
    if emptied {
        if let Some(object) = root.as_object_mut() {
            object.remove("permissions");
        }
    }
    Ok((format!("{}\n", serde_json::to_string_pretty(&root)?), removed))
}

fn rules_present(text: Option<&str>, rules: &[String]) -> Result<bool> {
    let root = parse(text)?;
    let allow = root.get("permissions").and_then(|permissions| permissions.get("allow")).and_then(Value::as_array);
    Ok(rules.iter().all(|rule| allow.is_some_and(|allow| allow.iter().any(|existing| existing.as_str() == Some(rule)))))
}

fn parse(text: Option<&str>) -> Result<Value> {
    match text.map(|text| text.trim_start_matches('\u{feff}').trim()).filter(|text| !text.is_empty()) {
        None => Ok(Value::Object(Map::new())),
        Some(text) => {
            let value: Value = serde_json::from_str(text).map_err(|error| Error::permanent(format!("Could not read the hook settings JSON: {error}")))?;
            if value.is_object() {
                Ok(value)
            } else {
                Err(Error::permanent("The hook settings root must be a JSON object."))
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
    let object = root.as_object_mut().ok_or_else(|| Error::permanent("The hook settings root must be a JSON object."))?;
    let hooks = object.entry("hooks").or_insert_with(|| Value::Object(Map::new()));
    let hooks = hooks.as_object_mut().ok_or_else(|| Error::permanent("The hooks field must be a JSON object."))?;
    for event in EVENTS {
        let entries = hooks.entry(event).or_insert_with(|| Value::Array(Vec::new()));
        let entries = entries.as_array_mut().ok_or_else(|| Error::permanent(format!("The {event} hook entry must be an array.")))?;
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
        name.push(if suffix == 0 { format!(".lore-backup-{now}") } else { format!(".lore-backup-{now}-{suffix}") });
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
    let mut removed = 0;
    if provider == Provider::Codex {
        match std::fs::remove_file(codex_rules_path(config)) {
            Ok(()) => removed += 1,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    let Some(original) = read_optional(config)? else { return Ok(removed) };
    let (mut updated, mut changed) = remove_hooks(Some(&original), provider)?;
    if provider == Provider::Claude {
        let (text, rules) = remove_rules(Some(&updated))?;
        updated = text;
        changed += rules;
    }
    if changed == 0 {
        return Ok(removed);
    }
    write_backup(config, &original, now)?;
    atomic_write(config, updated.as_bytes())?;
    Ok(removed + changed)
}

fn read_optional(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Whether a session may run `lore-cli mark` without being asked. A connection without it counts
/// as missing, so connecting again after an update adds it.
fn mark_allowed(config: &Path, text: Option<&str>, provider: Provider, cli: &Path) -> Result<bool> {
    match provider {
        Provider::Claude => rules_present(text, &claude_rules(cli)),
        Provider::Codex => Ok(read_optional(&codex_rules_path(config))?.as_deref() == Some(codex_rules(cli).as_str())),
    }
}

pub fn status_at(config: &Path, provider: Provider, cli: &Path) -> HookStatus {
    let wanted = command(cli, provider);
    let config_text = config.to_string_lossy().replace('\\', "/");
    let checked = read_optional(config).and_then(|text| {
        let (state, found) = inspect(text.as_deref(), provider, &wanted)?;
        if state == HookState::Connected && !mark_allowed(config, text.as_deref(), provider, cli)? {
            return Ok((HookState::Missing, None));
        }
        Ok((state, found))
    });
    match checked {
        Ok((state, found)) => HookStatus { provider, state, config: config_text, command: wanted, found, error: None },
        Err(error) => HookStatus { provider, state: HookState::Missing, config: config_text, command: wanted, found: None, error: Some(error.message) },
    }
}

pub fn register_at(config: &Path, provider: Provider, cli: &Path, now: i64) -> Result<Option<PathBuf>> {
    let original = read_optional(config)?;
    let mut updated = add_hooks(original.as_deref(), provider, &command(cli, provider))?;
    if provider == Provider::Claude {
        updated = add_rules(Some(&updated), &claude_rules(cli))?;
    }
    let backup = match &original {
        Some(text) => Some(write_backup(config, text, now)?),
        None => None,
    };
    atomic_write(config, updated.as_bytes())?;
    if provider == Provider::Codex {
        atomic_write(&codex_rules_path(config), codex_rules(cli).as_bytes())?;
    }
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLI: &str = "C:/Apps/lore/lore-cli.exe";

    fn wanted(provider: Provider) -> String {
        command(Path::new(CLI), provider)
    }

    #[test]
    fn hook_commands_use_platform_appropriate_quoting() {
        if cfg!(windows) {
            assert_eq!(wanted(Provider::Claude), "C:/Apps/lore/lore-cli.exe hook claude");
            assert_eq!(command(Path::new("C:\\Program Files\\q\\lore-cli.exe"), Provider::Codex), "\"C:/Program Files/q/lore-cli.exe\" hook codex");
        } else {
            assert_eq!(wanted(Provider::Claude), "'C:/Apps/lore/lore-cli.exe' hook claude");
            assert_eq!(command(Path::new("/Users/it's $me/`apps`/lore-cli"), Provider::Codex), "'/Users/it'\"'\"'s $me/`apps`/lore-cli' hook codex");
        }
    }

    #[test]
    #[cfg(unix)]
    fn posix_hook_executes_a_literal_path_with_shell_metacharacters() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let cli = dir.path().join("it's $HOME `date`").join("lore-cli");
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
    fn both_tools_are_hooked_on_session_end_before_compaction_and_on_each_prompt() {
        for provider in [Provider::Claude, Provider::Codex] {
            let value: Value = serde_json::from_str(&add_hooks(None, provider, &wanted(provider)).unwrap()).unwrap();
            for event in ["SessionEnd", "PreCompact", "UserPromptSubmit"] {
                assert_eq!(event_commands(&value, event), vec![wanted(provider)], "{event}");
            }
        }
    }

    // A connection made by a version without marks has no prompt hook; reconnecting adds it and keeps the rest.
    #[test]
    fn a_connection_made_before_an_event_was_added_is_completed_by_reconnecting() {
        let entry = format!(r#"[{{"hooks": [{{"type": "command", "command": "{}", "timeout": 10}}]}}]"#, wanted(Provider::Codex));
        let old = format!(r#"{{"hooks": {{"SessionEnd": {entry}, "PreCompact": {entry}}}}}"#);
        assert_eq!(inspect(Some(&old), Provider::Codex, &wanted(Provider::Codex)).unwrap().0, HookState::Missing);
        let updated = add_hooks(Some(&old), Provider::Codex, &wanted(Provider::Codex)).unwrap();
        let value: Value = serde_json::from_str(&updated).unwrap();
        for event in EVENTS {
            assert_eq!(event_commands(&value, event), vec![wanted(Provider::Codex)], "{event}");
        }
        assert_eq!(inspect(Some(&updated), Provider::Codex, &wanted(Provider::Codex)).unwrap().0, HookState::Connected);
    }

    #[test]
    fn stale_install_path_is_replaced_not_duplicated() {
        let stale = r#"{"hooks": {"SessionEnd": [{"hooks": [{"type": "command", "command": "D:/old/lore-cli.exe hook codex", "timeout": 10}]}]}}"#;
        assert_eq!(inspect(Some(stale), Provider::Codex, &wanted(Provider::Codex)).unwrap(), (HookState::OtherPath, Some("D:/old/lore-cli.exe hook codex".into())));
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
        assert_eq!(removed, EVENTS.len());
        let value: Value = serde_json::from_str(&removed_text).unwrap();
        assert_eq!(event_commands(&value, "SessionEnd"), vec!["orca-agent-hook.exe claude".to_string()]);
        assert!(value["hooks"].get("PreCompact").is_none() && value["hooks"].get("UserPromptSubmit").is_none());
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
        assert_eq!(removed, EVENTS.len());
        assert_eq!(inspect(Some(&text), Provider::Claude, &wanted(Provider::Claude)).unwrap().0, HookState::Connected);
        let stale = r#"{"hooks": {"SessionEnd": [{"hooks": [{"type": "command", "command": "D:/old/lore-cli.exe hook codex"}]}]}}"#;
        assert_eq!(remove_hooks(Some(stale), Provider::Codex).unwrap().1, 1);
    }

    #[test]
    fn unregistering_backs_up_and_does_nothing_when_not_connected() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("hooks.json");
        assert_eq!(unregister_at(&config, Provider::Codex, 1).unwrap(), 0);
        register_at(&config, Provider::Codex, Path::new(CLI), 2).unwrap();
        let connected = std::fs::read_to_string(&config).unwrap();
        // The hooks and Codex's rules file.
        assert_eq!(unregister_at(&config, Provider::Codex, 3).unwrap(), EVENTS.len() + 1);
        assert_eq!(std::fs::read_to_string(dir.path().join("hooks.json.lore-backup-3")).unwrap(), connected);
        assert_eq!(status_at(&config, Provider::Codex, Path::new(CLI)).state, HookState::Missing);
        assert_eq!(unregister_at(&config, Provider::Codex, 4).unwrap(), 0);
        assert!(!dir.path().join("hooks.json.lore-backup-4").exists());
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
        assert_eq!(std::fs::read_to_string(dir.path().join("settings.json.lore-backup-9-1")).unwrap(), connected);
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

    fn allow(text: &str) -> Vec<String> {
        let value: Value = serde_json::from_str(text).unwrap();
        value["permissions"]["allow"].as_array().map(|rules| rules.iter().map(|rule| rule.as_str().unwrap().to_string()).collect()).unwrap_or_default()
    }

    #[test]
    fn connecting_claude_allows_the_mark_command_and_leaves_the_users_rules() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("settings.json");
        let original = r#"{"permissions": {"allow": ["Bash(npm test)", "Bash(C:/x/lore-cli.exe status *)"], "deny": ["Bash(rm *)"]}, "model": "opus"}"#;
        std::fs::write(&config, original).unwrap();
        register_at(&config, Provider::Claude, Path::new(CLI), 1).unwrap();
        let connected = std::fs::read_to_string(&config).unwrap();
        let mut expected = vec!["Bash(npm test)".to_string(), "Bash(C:/x/lore-cli.exe status *)".to_string()];
        expected.extend(claude_rules(Path::new(CLI)));
        assert_eq!(allow(&connected), expected);
        assert!(allow(&connected).contains(&"Bash(C:/Apps/lore/lore-cli.exe mark *)".to_string()));
        assert_eq!(status_at(&config, Provider::Claude, Path::new(CLI)).state, HookState::Connected);
        register_at(&config, Provider::Claude, Path::new(CLI), 2).unwrap();
        assert_eq!(std::fs::read_to_string(&config).unwrap(), connected);

        assert_eq!(unregister_at(&config, Provider::Claude, 3).unwrap(), EVENTS.len() + claude_rules(Path::new(CLI)).len());
        let disconnected: Value = serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
        let mut restored: Value = serde_json::from_str(original).unwrap();
        restored["hooks"] = json!({});
        assert_eq!(disconnected, restored);
    }

    #[test]
    fn disconnecting_claude_takes_away_a_permissions_block_only_lore_made() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("settings.json");
        std::fs::write(&config, r#"{"model": "opus"}"#).unwrap();
        register_at(&config, Provider::Claude, Path::new(CLI), 1).unwrap();
        unregister_at(&config, Provider::Claude, 2).unwrap();
        let value: Value = serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
        assert!(value.get("permissions").is_none(), "{value}");
    }

    #[test]
    fn a_connection_without_the_mark_rule_counts_as_missing_and_connecting_again_adds_it() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("settings.json");
        std::fs::write(&config, add_hooks(None, Provider::Claude, &wanted(Provider::Claude)).unwrap()).unwrap();
        assert_eq!(status_at(&config, Provider::Claude, Path::new(CLI)).state, HookState::Missing);
        register_at(&config, Provider::Claude, Path::new(CLI), 1).unwrap();
        assert_eq!(status_at(&config, Provider::Claude, Path::new(CLI)).state, HookState::Connected);
    }

    #[test]
    fn a_mark_rule_for_an_old_install_path_is_replaced() {
        let old = r#"{"permissions": {"allow": ["Bash(D:/old/lore-cli.exe mark *)", "Bash(npm test)"]}}"#;
        let updated = add_rules(Some(old), &claude_rules(Path::new(CLI))).unwrap();
        let mut expected = vec!["Bash(npm test)".to_string()];
        expected.extend(claude_rules(Path::new(CLI)));
        assert_eq!(allow(&updated), expected);
    }

    #[test]
    fn connecting_codex_writes_its_own_rules_file_and_disconnecting_removes_it() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("hooks.json");
        let rules = dir.path().join("rules").join("lore.rules");
        register_at(&config, Provider::Codex, Path::new(CLI), 1).unwrap();
        let written = std::fs::read_to_string(&rules).unwrap();
        assert!(written.contains("pattern = [\"C:/Apps/lore/lore-cli.exe\", \"mark\"]"), "{written}");
        assert!(written.contains("decision = \"allow\""), "{written}");
        assert_eq!(status_at(&config, Provider::Codex, Path::new(CLI)).state, HookState::Connected);

        std::fs::remove_file(&rules).unwrap();
        assert_eq!(status_at(&config, Provider::Codex, Path::new(CLI)).state, HookState::Missing);
        register_at(&config, Provider::Codex, Path::new(CLI), 2).unwrap();
        assert_eq!(status_at(&config, Provider::Codex, Path::new(CLI)).state, HookState::Connected);

        unregister_at(&config, Provider::Codex, 3).unwrap();
        assert!(!rules.exists());
        assert!(dir.path().join("rules").exists());
    }
}
