use std::path::Path;

use serde_json::{json, Value};

use crate::platform;
use crate::settings::{self, Paths, Settings};

pub fn command_available(command: &[String]) -> bool {
    command.first().map(|program| {
        let path = Path::new(program);
        if path.components().count() > 1 { platform::is_executable(path) } else { platform::which(program).is_some() }
    }).unwrap_or(false)
}

// Diagnostics must not initialize a database, register hooks, run a model, or
// change settings. This also makes it safe to run before the first app launch.
pub fn inspect(paths: &Paths, cli: &Path) -> Value {
    let mut problems = Vec::new();
    let mut settings = match std::fs::read_to_string(paths.settings()) {
        Ok(text) => settings::reloaded(&text, &Settings::default()).unwrap_or_else(|error| {
            problems.push(error.to_string());
            Settings::default()
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Settings::default(),
        Err(error) => { problems.push(error.to_string()); Settings::default() }
    };
    settings.fill_missing_commands();
    if let Err(error) = settings.validate() { problems.push(error.to_string()); }
    let mut commands = serde_json::Map::new();
    for (name, command) in [("codex", &settings.commands.codex), ("claude", &settings.commands.claude), ("qmd", &settings.commands.qmd)] {
        let available = command_available(command);
        if !available && (name == "qmd" || name == settings.backend) {
            problems.push(format!("The {name} executable was not found. Install it or set commands.{name} in settings.json."));
        }
        commands.insert(name.into(), json!({"command": command, "available": available}));
    }
    if !platform::is_executable(cli) { problems.push(format!("Hook CLI not found: {}", cli.display())); }
    json!({
        "platform": std::env::consts::OS,
        "architecture": std::env::consts::ARCH,
        "dataDir": paths.root,
        "settingsFile": paths.settings(),
        "knowledgeRoot": settings.knowledge_root,
        "qmdConfigDir": settings.qmd_config_dir(),
        "qmdCacheDir": settings.qmd_cache_dir(),
        "qmdIndex": settings.qmd_index,
        "indexConfigured": settings.qmd_config_dir().join(format!("{}.yml", settings.qmd_index)).is_file(),
        "hookCli": cli,
        "commands": commands,
        "problems": problems,
    })
}
