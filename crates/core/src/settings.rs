use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

pub const APP_IDENTIFIER: &str = "dev.jyjo.qmd-memory";

#[derive(Debug, Clone)]
pub struct Paths {
    pub root: PathBuf,
}

impl Paths {
    pub fn standard() -> Self {
        let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| home().join("AppData").join("Local"));
        Self { root: base.join(APP_IDENTIFIER) }
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

pub fn home() -> PathBuf {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_default()
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Commands {
    pub codex: Vec<String>,
    pub claude: Vec<String>,
    pub qmd: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub knowledge_root: String,
    pub backend: String,
    pub model: Option<String>,
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
}

fn slash(path: PathBuf) -> String {
    path.to_string_lossy().replace('\\', "/")
}

impl Default for Settings {
    fn default() -> Self {
        let home = home();
        Self {
            knowledge_root: "D:/doc/knowledge".into(),
            backend: "codex".into(),
            model: None,
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
        }
    }
}

impl Settings {
    pub fn knowledge_root(&self) -> PathBuf {
        PathBuf::from(&self.knowledge_root)
    }

    pub fn qmd_config_dir(&self) -> PathBuf {
        self.knowledge_root().join(".qmd").join("config")
    }

    pub fn qmd_cache_dir(&self) -> PathBuf {
        self.knowledge_root().join(".qmd").join("cache")
    }

    pub fn validate(&self) -> Result<()> {
        if !matches!(self.backend.as_str(), "codex" | "claude") {
            return Err(Error::permanent("기록 모델은 codex 또는 claude여야 합니다."));
        }
        if self.knowledge_root.trim().is_empty() {
            return Err(Error::permanent("기억 저장소 위치가 비어 있습니다."));
        }
        if self.max_chars < 1_000 || self.notes_budget < 1_000 {
            return Err(Error::permanent("입력 한도가 너무 작습니다."));
        }
        Ok(())
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

pub fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for folder in std::env::split_paths(&path) {
        for extension in [".exe", ".cmd", ".ps1", ".bat"] {
            let candidate = folder.join(format!("{name}{extension}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn detect_command(name: &str, package_script: &str) -> Vec<String> {
    let Some(found) = which(name) else { return Vec::new() };
    let is_exe = found.extension().map(|ext| ext.eq_ignore_ascii_case("exe")).unwrap_or(false);
    if is_exe {
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
        assert_eq!(parsed.max_chars, 30_000);
        parsed.validate().unwrap();
        let wrong = Settings { backend: "gpt".into(), ..Settings::default() };
        assert!(wrong.validate().is_err());
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
