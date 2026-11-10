use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
pub use crate::platform::{home, which};

pub const APP_IDENTIFIER: &str = "dev.jyjo.qmd-memory";

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
        self.root.join("qmd-memory.log")
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

impl Models {
    fn trimmed(self) -> Self {
        let clean = |model: Option<String>| model.map(|model| model.trim().to_string()).filter(|model| !model.is_empty());
        Self { codex: clean(self.codex), claude: clean(self.claude) }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub knowledge_root: String,
    pub backend: String,
    pub models: Models,
    #[serde(rename = "model", skip_serializing)]
    pub single_model: Option<String>,
    pub model_timeout_secs: u64,
    pub qmd_timeout_secs: u64,
    pub claude_projects_dir: String,
    pub codex_sessions_dir: String,
    pub idle_secs: u64,
    pub since: i64,
    pub paused: bool,
    pub max_chars: usize,
    pub notes_budget: usize,
    pub qmd_index: String,
    pub commands: Commands,
    pub sync_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditableSettings {
    pub knowledge_root: String,
    pub backend: String,
    pub models: Models,
    pub model_timeout_secs: u64,
    pub qmd_timeout_secs: u64,
    pub max_chars: usize,
    pub notes_budget: usize,
}

fn slash(path: PathBuf) -> String {
    path.to_string_lossy().replace('\\', "/")
}

impl Default for Settings {
    fn default() -> Self {
        let home = home();
        Self {
            knowledge_root: slash(home.join("Documents").join("knowledge")),
            backend: "codex".into(),
            models: Models::default(),
            single_model: None,
            model_timeout_secs: 600,
            qmd_timeout_secs: 900,
            claude_projects_dir: slash(home.join(".claude").join("projects")),
            codex_sessions_dir: slash(home.join(".codex").join("sessions")),
            idle_secs: 600,
            since: 0,
            paused: false,
            max_chars: 30_000,
            notes_budget: 60_000,
            qmd_index: "index".into(),
            commands: Commands::default(),
            sync_enabled: false,
        }
    }
}

impl Settings {
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
            return Err(Error::permanent("기록 모델은 codex 또는 claude여야 합니다."));
        }
        if self.knowledge_root.trim().is_empty() {
            return Err(Error::permanent("기억 저장소 위치가 비어 있습니다."));
        }
        if !self.knowledge_root().is_absolute() {
            return Err(Error::permanent("기억 저장소 위치는 이 PC의 절대 경로여야 합니다."));
        }
        if self.max_chars < 1_000 || self.notes_budget < 1_000 {
            return Err(Error::permanent("입력 한도가 너무 작습니다."));
        }
        if self.model_timeout_secs == 0 || self.qmd_timeout_secs == 0 {
            return Err(Error::permanent("시간 제한은 1초 이상이어야 합니다."));
        }
        Ok(())
    }

    pub fn model(&self) -> Option<&str> {
        let model = if self.backend == "claude" { &self.models.claude } else { &self.models.codex };
        model.as_deref()
    }

    fn move_single_model(&mut self) {
        let Some(model) = self.single_model.take() else { return };
        let slot = if self.backend == "claude" { &mut self.models.claude } else { &mut self.models.codex };
        if slot.is_none() {
            *slot = Some(model);
        }
        self.models = std::mem::take(&mut self.models).trimmed();
    }

    pub fn edited(&self, change: EditableSettings) -> Settings {
        Settings {
            knowledge_root: change.knowledge_root,
            backend: change.backend,
            models: change.models.trimmed(),
            model_timeout_secs: change.model_timeout_secs,
            qmd_timeout_secs: change.qmd_timeout_secs,
            max_chars: change.max_chars,
            notes_budget: change.notes_budget,
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

pub fn reloaded(text: &str, running: &Settings) -> Result<Settings> {
    let mut settings: Settings = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|error| Error::permanent(format!("설정 파일을 읽지 못했습니다: {error}")))?;
    settings.move_single_model();
    if settings.since == 0 {
        settings.since = running.since;
    }
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
    settings.move_single_model();
    if settings.since == 0 {
        settings.since = now;
    }
    settings.fill_missing_commands();
    if settings != before || !path.exists() {
        save(path, &settings)?;
    }
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

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
        assert_eq!(parsed.max_chars, 30_000);
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
            models: Models { codex: Some(" gpt-6.1-sol ".into()), claude: Some("opus".into()) },
            model_timeout_secs: 1_200,
            qmd_timeout_secs: 300,
            max_chars: 20_000,
            notes_budget: 40_000,
        }
    }

    #[test]
    fn screen_changes_replace_only_the_values_the_screen_shows() {
        let edited = running().edited(screen());
        let expected = Settings {
            knowledge_root: "E:/notes".into(),
            backend: "claude".into(),
            models: Models { codex: Some("gpt-6.1-sol".into()), claude: Some("opus".into()) },
            model_timeout_secs: 1_200,
            qmd_timeout_secs: 300,
            max_chars: 20_000,
            notes_budget: 40_000,
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
    fn a_single_model_from_an_older_file_moves_to_the_cli_it_was_set_for() {
        let commands = r#""commands": {"codex": ["c.exe"], "claude": ["k.exe"], "qmd": ["q.exe"]}"#;
        let old = format!(r#"{{"backend": "claude", "model": "opus", "since": 3, {commands}}}"#);
        let moved = reloaded(&old, &running()).unwrap();
        assert_eq!(moved.models, Models { codex: None, claude: Some("opus".into()) });
        let written = serde_json::to_value(&moved).unwrap();
        assert!(written.get("model").is_none(), "{written}");
        assert_eq!(written["models"]["claude"], "opus");

        let both = format!(r#"{{"backend": "claude", "model": "haiku", "models": {{"claude": "opus"}}, {commands}}}"#);
        assert_eq!(reloaded(&both, &running()).unwrap().models.claude.as_deref(), Some("opus"));

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, &old).unwrap();
        assert_eq!(load_or_init(&path, 9).unwrap().models.claude.as_deref(), Some("opus"));
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(saved.get("model").is_none() && saved["models"]["claude"] == "opus", "{saved}");
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
        let edited = reloaded(text, &running()).unwrap();
        assert_eq!(edited.model_timeout_secs, 1_500);
        assert_eq!(edited.knowledge_root, "E:/notes");
        assert_eq!(edited.since, 77);
        assert_eq!(edited.commands.claude, vec!["k.exe".to_string()]);
        let own_start = reloaded_with_since(text);
        assert_eq!(own_start.since, 5);
        assert!(reloaded("{not json", &running()).is_err());
    }

    fn reloaded_with_since(text: &str) -> Settings {
        let text = text.replacen('{', "{\"since\": 5, ", 1);
        reloaded(&text, &running()).unwrap()
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
