use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
pub use crate::platform::{home, which};

pub const APP_IDENTIFIER: &str = "io.github.junyoungjo.lore";

#[derive(Debug, Clone)]
pub struct Paths {
    pub root: PathBuf,
}

impl Paths {
    pub fn standard() -> Self {
        Self { root: crate::platform::data_root() }
    }

    pub fn database(&self) -> PathBuf {
        self.root.join("state.sqlite")
    }

    pub fn settings(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    pub fn reviews(&self) -> PathBuf {
        self.root.join("reviews")
    }

    pub fn recorder_workdir(&self) -> PathBuf {
        self.root.join("recorder")
    }

    pub fn lock(&self) -> PathBuf {
        self.root.join("worker.lock")
    }

    pub fn log(&self) -> PathBuf {
        self.root.join("lore.log")
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Commands {
    pub codex: Vec<String>,
    pub claude: Vec<String>,
    pub qmd: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Models {
    pub codex: Option<String>,
    pub claude: Option<String>,
}

fn clean(value: Option<String>) -> Option<String> {
    value.map(|value| value.trim().to_string()).filter(|value| !value.is_empty())
}

impl Models {
    fn trimmed(self) -> Self {
        Self { codex: clean(self.codex), claude: clean(self.claude) }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Efforts {
    pub codex: Option<String>,
    pub claude: Option<String>,
}

impl Efforts {
    fn trimmed(self) -> Self {
        Self { codex: clean(self.codex), claude: clean(self.claude) }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub knowledge_root: String,
    pub backend: String,
    /// Use the other CLI when `backend` cannot run.
    pub fallback: bool,
    pub models: Models,
    pub efforts: Efforts,
    pub model_timeout_secs: u64,
    pub qmd_timeout_secs: u64,
    pub claude_projects_dir: String,
    pub codex_sessions_dir: String,
    pub idle_secs: u64,
    pub since: i64,
    pub paused: bool,
    /// How much new conversation one step of a session reads.
    pub max_chars: usize,
    pub qmd_index: String,
    pub commands: Commands,
    pub sync_enabled: bool,
    /// Clean up a folder of notes without being asked once enough of its notes have changed.
    pub auto_cleanup: bool,
    /// Approve a candidate as it stands once it has waited `auto_approve_days`. Off: it waits for the user.
    pub auto_approve: bool,
    pub auto_approve_days: u64,
    /// When automatic approval was turned on; 0 while it is off. The days count from the later
    /// of this and the time a candidate entered the list, so that turning the setting on never
    /// approves at once what has already waited longer.
    pub auto_approve_since: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditableSettings {
    pub knowledge_root: String,
    pub backend: String,
    pub fallback: bool,
    pub models: Models,
    pub efforts: Efforts,
    pub model_timeout_secs: u64,
    pub qmd_timeout_secs: u64,
    pub max_chars: usize,
}

fn slash(path: PathBuf) -> String {
    path.to_string_lossy().replace('\\', "/")
}

impl Default for Settings {
    fn default() -> Self {
        let home = home();
        Self {
            knowledge_root: slash(home.join("Documents").join("lore-memory")),
            backend: "codex".into(),
            fallback: false,
            models: Models::default(),
            efforts: Efforts::default(),
            model_timeout_secs: 600,
            qmd_timeout_secs: 900,
            claude_projects_dir: slash(home.join(".claude").join("projects")),
            codex_sessions_dir: slash(home.join(".codex").join("sessions")),
            idle_secs: 600,
            since: 0,
            paused: false,
            max_chars: 200_000,
            qmd_index: "index".into(),
            commands: Commands::default(),
            sync_enabled: false,
            auto_cleanup: false,
            auto_approve: false,
            auto_approve_days: 3,
            auto_approve_since: 0,
        }
    }
}

impl Settings {
    /// Turn automatic approval on or off, or change its days. Its days start to count when it is
    /// turned on.
    pub fn set_auto_approve(&mut self, enabled: bool, days: u64, now: i64) {
        let was = self.clone();
        self.auto_approve = enabled;
        self.auto_approve_days = days;
        self.auto_approve_since = self.auto_approve_start(Some(&was), now);
    }

    /// When the days of automatic approval start to count. `was` is the setting before this
    /// change, when that is known; a settings file edited by hand does not say when it was turned on.
    fn auto_approve_start(&self, was: Option<&Settings>, now: i64) -> i64 {
        if !self.auto_approve {
            return 0;
        }
        match was {
            Some(was) if !was.auto_approve => now,
            Some(was) if self.auto_approve_since == 0 && was.auto_approve_since != 0 => was.auto_approve_since,
            _ if self.auto_approve_since == 0 => now,
            _ => self.auto_approve_since,
        }
    }

    pub fn knowledge_root(&self) -> PathBuf {
        PathBuf::from(&self.knowledge_root)
    }

    pub fn qmd_config_dir(&self) -> PathBuf {
        self.qmd_local_dir().join("config")
    }

    pub fn qmd_cache_dir(&self) -> PathBuf {
        self.qmd_local_dir().join("cache")
    }

    fn qmd_local_dir(&self) -> PathBuf {
        let mut key = slash(self.knowledge_root()).trim_end_matches('/').to_string();
        if cfg!(windows) { key = key.to_lowercase(); }
        Paths::standard().root.join("qmd").join(&crate::transcript::digest(key.as_bytes())[..16])
    }

    pub fn validate(&self) -> Result<()> {
        if !matches!(self.backend.as_str(), "codex" | "claude") {
            return Err(Error::permanent("The recording backend must be codex or claude."));
        }
        if self.knowledge_root.trim().is_empty() {
            return Err(Error::permanent("The memory folder path is empty."));
        }
        if !self.knowledge_root().is_absolute() {
            return Err(Error::permanent("The memory folder must be an absolute path on this PC."));
        }
        if self.max_chars < 1_000 {
            return Err(Error::permanent("The input limit is too small."));
        }
        if !(1..=365).contains(&self.auto_approve_days) {
            return Err(Error::permanent("Candidates are approved automatically after 1 to 365 days."));
        }
        if self.model_timeout_secs == 0 || self.qmd_timeout_secs == 0 {
            return Err(Error::permanent("Timeouts must be at least one second."));
        }
        // Codex receives the effort inside a TOML string.
        if [&self.efforts.codex, &self.efforts.claude].into_iter().flatten().any(|effort| !effort.bytes().all(|byte| byte.is_ascii_lowercase())) {
            return Err(Error::permanent("The effort level must be a single lowercase word."));
        }
        Ok(())
    }

    /// The recording CLIs in the order they are tried.
    pub fn backends(&self) -> Vec<&str> {
        let other = if self.backend == "claude" { "codex" } else { "claude" };
        if self.fallback { vec![self.backend.as_str(), other] } else { vec![self.backend.as_str()] }
    }

    pub fn model(&self) -> Option<&str> {
        self.model_of(&self.backend)
    }

    pub fn effort(&self) -> Option<&str> {
        self.effort_of(&self.backend)
    }

    pub fn model_of(&self, backend: &str) -> Option<&str> {
        let model = if backend == "claude" { &self.models.claude } else { &self.models.codex };
        model.as_deref()
    }

    pub fn effort_of(&self, backend: &str) -> Option<&str> {
        let effort = if backend == "claude" { &self.efforts.claude } else { &self.efforts.codex };
        effort.as_deref()
    }

    pub fn command_of(&self, backend: &str) -> &[String] {
        if backend == "claude" { &self.commands.claude } else { &self.commands.codex }
    }

    pub fn edited(&self, change: EditableSettings) -> Settings {
        Settings {
            knowledge_root: change.knowledge_root,
            backend: change.backend,
            fallback: change.fallback,
            models: change.models.trimmed(),
            efforts: change.efforts.trimmed(),
            model_timeout_secs: change.model_timeout_secs,
            qmd_timeout_secs: change.qmd_timeout_secs,
            max_chars: change.max_chars,
            ..self.clone()
        }
    }

    pub fn fill_missing_commands(&mut self) {
        if self.commands.codex.is_empty() {
            self.commands.codex = detect_command("codex", "@openai/codex/bin/codex.js");
        }
        if self.commands.claude.is_empty() {
            self.commands.claude = detect_command("claude", "@anthropic-ai/claude-code/cli.js");
        }
        if self.commands.qmd.is_empty() {
            self.commands.qmd = detect_command("qmd", "@tobilu/qmd/bin/qmd");
        }
    }
}

fn detect_command(name: &str, package_script: &str) -> Vec<String> {
    let Some(found) = which(name) else { return Vec::new() };
    #[cfg(unix)]
    {
        use std::io::Read;
        let mut header = Vec::new();
        if let Ok(file) = std::fs::File::open(&found) { let _ = file.take(160).read_to_end(&mut header); }
        if header.starts_with(b"#!") && String::from_utf8_lossy(&header).lines().next().is_some_and(|line| line.contains("node")) {
            if let Some(node) = which("node") { return vec![node.to_string_lossy().into(), found.to_string_lossy().into()]; }
        }
    }
    let is_exe = found.extension().map(|ext| ext.eq_ignore_ascii_case("exe")).unwrap_or(false);
    if !cfg!(windows) || is_exe {
        return vec![found.to_string_lossy().to_string()];
    }
    let Some(node) = which("node") else { return Vec::new() };
    let folder = found.parent().unwrap_or(Path::new(""));
    for base in [folder.to_path_buf(), folder.join(name)] {
        let script = base.join("node_modules").join(package_script);
        if script.is_file() {
            return vec![node.to_string_lossy().to_string(), script.to_string_lossy().to_string()];
        }
    }
    Vec::new()
}

pub fn atomic_write(path: &Path, content: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    std::fs::write(&temporary, content)?;
    std::fs::rename(&temporary, path)?;
    Ok(())
}

pub fn save(path: &Path, settings: &Settings) -> Result<()> {
    atomic_write(path, format!("{}\n", serde_json::to_string_pretty(settings)?).as_bytes())
}

pub fn reloaded(text: &str, running: &Settings, now: i64) -> Result<Settings> {
    let mut settings: Settings = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|error| Error::permanent(format!("Could not read the settings file: {error}")))?;
    if settings.since == 0 {
        settings.since = running.since;
    }
    settings.auto_approve_since = settings.auto_approve_start(Some(running), now);
    settings.fill_missing_commands();
    Ok(settings)
}

pub fn load_or_init(path: &Path, now: i64) -> Result<Settings> {
    let mut settings = match std::fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<Settings>(text.trim_start_matches('\u{feff}')) {
            Ok(settings) => settings,
            Err(_) => {
                let mut broken = path.as_os_str().to_owned();
                broken.push(".broken");
                std::fs::rename(path, PathBuf::from(broken))?;
                Settings::default()
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Settings::default(),
        Err(error) => return Err(error.into()),
    };
    let before = settings.clone();
    if settings.since == 0 {
        settings.since = now;
    }
    settings.auto_approve_since = settings.auto_approve_start(None, now);
    settings.fill_missing_commands();
    if settings != before || !path.exists() {
        save(path, &settings)?;
    }
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_load_records_the_start_time_and_keeps_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let first = load_or_init(&path, 1_000).unwrap();
        assert_eq!(first.since, 1_000);
        let second = load_or_init(&path, 2_000).unwrap();
        assert_eq!(second.since, 1_000);
    }

    #[test]
    fn missing_fields_take_defaults_and_unknown_backend_is_invalid() {
        let parsed: Settings = serde_json::from_str(r#"{"knowledgeRoot": "E:/notes", "backend": "claude"}"#).unwrap();
        assert_eq!(parsed.knowledge_root, "E:/notes");
        assert_eq!((parsed.max_chars, parsed.auto_approve, parsed.auto_approve_days), (200_000, false, 3));
        assert!(Settings { auto_approve_days: 0, ..Settings::default() }.validate().is_err());
        Settings { knowledge_root: home().join("notes").to_string_lossy().to_string(), ..parsed }.validate().unwrap();
        let wrong = Settings { backend: "gpt".into(), ..Settings::default() };
        assert!(wrong.validate().is_err());
    }

    fn running() -> Settings {
        Settings {
            since: 77,
            paused: true,
            idle_secs: 120,
            qmd_index: "work".into(),
            claude_projects_dir: "C:/u/.claude/projects".into(),
            commands: Commands { codex: vec!["codex.exe".into()], claude: vec!["claude.exe".into()], qmd: vec!["qmd.exe".into()] },
            ..Settings::default()
        }
    }

    fn screen() -> EditableSettings {
        EditableSettings {
            knowledge_root: "E:/notes".into(),
            backend: "claude".into(),
            fallback: true,
            models: Models { codex: Some(" gpt-6.1-sol ".into()), claude: Some("opus".into()) },
            efforts: Efforts { codex: Some(" high ".into()), claude: Some(" ".into()) },
            model_timeout_secs: 1_200,
            qmd_timeout_secs: 300,
            max_chars: 20_000,
        }
    }

    #[test]
    fn screen_changes_replace_only_the_values_the_screen_shows() {
        let edited = running().edited(screen());
        let expected = Settings {
            knowledge_root: "E:/notes".into(),
            backend: "claude".into(),
            fallback: true,
            models: Models { codex: Some("gpt-6.1-sol".into()), claude: Some("opus".into()) },
            efforts: Efforts { codex: Some("high".into()), claude: None },
            model_timeout_secs: 1_200,
            qmd_timeout_secs: 300,
            max_chars: 20_000,
            ..running()
        };
        assert_eq!(edited, expected);
        let blank_model = running().edited(EditableSettings { models: Models { codex: None, claude: Some("  ".into()) }, ..screen() });
        assert_eq!(blank_model.models, Models::default());
    }

    #[test]
    fn each_recorder_cli_keeps_its_own_model() {
        let settings = Settings { backend: "claude".into(), models: Models { codex: Some("gpt-6.1-sol".into()), claude: Some("opus".into()) }, ..Settings::default() };
        assert_eq!(settings.model(), Some("opus"));
        assert_eq!(Settings { backend: "codex".into(), ..settings.clone() }.model(), Some("gpt-6.1-sol"));
        assert_eq!(Settings { backend: "codex".into(), models: Models::default(), ..settings }.model(), None);
    }

    #[test]
    fn the_other_cli_is_tried_second_only_when_fallback_is_on() {
        let claude = Settings { backend: "claude".into(), models: Models { codex: Some("gpt-x".into()), claude: Some("opus".into()) }, efforts: Efforts { codex: Some("low".into()), claude: None }, ..Settings::default() };
        assert_eq!(claude.backends(), vec!["claude"]);
        let both = Settings { fallback: true, ..claude.clone() };
        assert_eq!(both.backends(), vec!["claude", "codex"]);
        assert_eq!(Settings { backend: "codex".into(), ..both.clone() }.backends(), vec!["codex", "claude"]);
        assert_eq!((both.model_of("codex"), both.effort_of("codex"), both.effort_of("claude")), (Some("gpt-x"), Some("low"), None));
        let older: Settings = serde_json::from_str(r#"{"backend": "claude"}"#).unwrap();
        assert!(!older.fallback);
    }

    #[test]
    fn each_recorder_cli_keeps_its_own_effort_and_only_plain_words_are_accepted() {
        let settings = Settings { backend: "claude".into(), efforts: Efforts { codex: Some("low".into()), claude: Some("max".into()) }, ..Settings::default() };
        assert_eq!(settings.effort(), Some("max"));
        assert_eq!(Settings { backend: "codex".into(), ..settings.clone() }.effort(), Some("low"));
        settings.validate().unwrap();
        for effort in ["high\"", "x high", "High", "low\n"] {
            let invalid = Settings { efforts: Efforts { codex: Some(effort.into()), claude: None }, ..Settings::default() };
            assert!(invalid.validate().is_err(), "{effort:?}");
        }
    }

    #[test]
    fn time_limits_must_be_positive() {
        for settings in [Settings { model_timeout_secs: 0, ..Settings::default() }, Settings { qmd_timeout_secs: 0, ..Settings::default() }] {
            assert!(settings.validate().is_err());
        }
        Settings { model_timeout_secs: 1, qmd_timeout_secs: 1, ..Settings::default() }.validate().unwrap();
    }

    #[test]
    fn new_defaults_are_native_and_existing_locations_remain_explicit() {
        let defaults = Settings::default();
        assert!(defaults.knowledge_root().is_absolute());
        assert!(!defaults.qmd_config_dir().starts_with(defaults.knowledge_root()));
        let relative = Settings { knowledge_root: "relative/notes".into(), ..defaults.clone() };
        assert!(relative.validate().is_err());
        let kept: Settings = serde_json::from_str(r#"{"knowledgeRoot":"D:/doc/knowledge"}"#).unwrap();
        assert_eq!(kept.knowledge_root, "D:/doc/knowledge");
        let other = Settings { knowledge_root: home().join("other-knowledge").to_string_lossy().to_string(), ..defaults.clone() };
        assert_ne!(defaults.qmd_config_dir(), other.qmd_config_dir());
    }

    #[test]
    fn file_edited_while_running_is_read_with_the_running_start_time_kept() {
        let text = "\u{feff}{\"knowledgeRoot\": \"E:/notes\", \"modelTimeoutSecs\": 1500, \"commands\": {\"codex\": [\"c.exe\"], \"claude\": [\"k.exe\"], \"qmd\": [\"q.exe\"]}}";
        let edited = reloaded(text, &running(), 900).unwrap();
        assert_eq!(edited.model_timeout_secs, 1_500);
        assert_eq!(edited.knowledge_root, "E:/notes");
        assert_eq!(edited.since, 77);
        assert_eq!(edited.commands.claude, vec!["k.exe".to_string()]);
        let own_start = reloaded_with_since(text);
        assert_eq!(own_start.since, 5);
        assert!(reloaded("{not json", &running(), 900).is_err());
    }

    fn reloaded_with_since(text: &str) -> Settings {
        let text = text.replacen('{', "{\"since\": 5, ", 1);
        reloaded(&text, &running(), 900).unwrap()
    }

    #[test]
    fn broken_settings_file_is_set_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{not json").unwrap();
        let settings = load_or_init(&path, 5).unwrap();
        assert_eq!(settings.since, 5);
        assert!(dir.path().join("settings.json.broken").exists());
    }
}
