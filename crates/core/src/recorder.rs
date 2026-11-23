use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::error::{Error, Failure, Result};
use crate::proposal::EDIT_OPS;
use crate::run::{self, Limit};
use crate::settings::{atomic_write, Settings};

pub const DEFAULT_CODEX_MODEL: &str = "gpt-6.1-sol";
pub const DEFAULT_CLAUDE_MODEL: &str = "haiku";
pub const DEFAULT_CODEX_EFFORT: &str = "low";

/// The edits to an existing note, in the form both an apply and a cleanup answer with.
pub fn edits_schema() -> Value {
    json!({"type": "array", "description": "Existing note: the edits, applied in order. New note: empty.", "items": {
        "type": "object",
        "properties": {
            "op": {"type": "string", "enum": EDIT_OPS},
            "find": {"type": "string", "description": "replace: text copied exactly from the note, occurring once. Otherwise empty."},
            "section": {"type": "string", "description": "append, replace_section: the title of an existing \"## \" section, without the \"## \". add_section: the section the new one follows, or empty for the end. replace: empty."},
            "heading": {"type": "string", "description": "add_section: the title of the new section. Otherwise empty."},
            "text": {"type": "string", "description": "The new text. For replace, empty deletes the found text."}
        },
        "required": ["op", "find", "section", "heading", "text"],
        "additionalProperties": false
    }})
}

pub struct CliRecorder {
    pub backend: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub command: Vec<String>,
    pub workdir: PathBuf,
    pub limit: Limit,
    /// The qmd MCP server over the memory folder's index, as setup registers it for the CLIs.
    /// A run that reads the notes searches them with it.
    pub search: Option<Value>,
    /// The MCP servers of the user's Codex config. A run turns each of them off.
    pub user_servers: Vec<String>,
}

/// What a run leaves out of the user's Codex setup: what acts on its own, and the tools and
/// instructions meant for interactive work. The rest of the config, such as the sandbox, the
/// provider and the shell environment, is the user's as it stands.
const CODEX_OFF: [&str; 17] = [
    "features.hooks=false",
    "notify=[]",
    "features.plugins=false",
    "plugins._default.enabled=false",
    "features.apps=false",
    "skills.bundled.enabled=false",
    "skills.include_instructions=false",
    "agents.enabled=false",
    "features.multi_agent=false",
    "features.memories=false",
    "features.computer_use=false",
    "features.browser_use=false",
    "features.in_app_browser=false",
    "features.image_generation=false",
    "features.goals=false",
    "web_search=\"disabled\"",
    "project_doc_max_bytes=0",
];

const SEARCH_SERVER: &str = "lore_qmd";
const SEARCH_TOOLS: &str = "mcp__lore_qmd__query,mcp__lore_qmd__get,mcp__lore_qmd__multi_get,mcp__lore_qmd__status";

fn codex_user_servers() -> Vec<String> {
    let config = crate::hooks::config_path(crate::transcript::Provider::Codex).with_file_name("config.toml");
    let Some(document) = std::fs::read_to_string(config).ok().and_then(|text| text.parse::<toml_edit::DocumentMut>().ok()) else {
        return Vec::new();
    };
    document.get("mcp_servers").and_then(|servers| servers.as_table_like()).map(|servers| servers.iter().map(|(name, _)| name.to_string()).collect()).unwrap_or_default()
}

fn codex_server(name: &str, entry: &Value) -> Vec<String> {
    let mut overrides = vec![format!("mcp_servers.{name}.command={}", entry["command"]), format!("mcp_servers.{name}.args={}", entry["args"])];
    if let Some(env) = entry["env"].as_object() {
        let pairs: Vec<String> = env.iter().map(|(key, value)| format!("{key}={value}")).collect();
        overrides.push(format!("mcp_servers.{name}.env={{{}}}", pairs.join(",")));
    }
    overrides
}

/// The last event of a Claude run with `--output-format stream-json`: the same envelope `json` gives.
fn stream_result(stdout: &str) -> Result<&str> {
    stdout
        .lines()
        .rev()
        .find(|line| serde_json::from_str::<Value>(line).ok().is_some_and(|event| event.get("type").and_then(Value::as_str) == Some("result")))
        .ok_or_else(|| Error::transient("The Claude run ended without a result."))
}

impl CliRecorder {
    pub fn from_settings(settings: &Settings, workdir: &Path, limit: Limit) -> Self {
        Self::for_backend(settings, &settings.backend, workdir, limit)
    }

    pub fn for_backend(settings: &Settings, backend: &str, workdir: &Path, limit: Limit) -> Self {
        Self {
            backend: backend.to_string(),
            model: settings.model_of(backend).map(str::to_string),
            effort: settings.effort_of(backend).map(str::to_string),
            command: settings.command_of(backend).to_vec(),
            workdir: workdir.to_path_buf(),
            limit,
            search: crate::setup::mcp(settings, crate::transcript::Provider::Claude).ok(),
            user_servers: if backend == "codex" { codex_user_servers() } else { Vec::new() },
        }
    }

    pub fn defaulting_to(mut self, model: Option<String>) -> Self {
        if self.model.is_none() {
            self.model = model;
        }
        self
    }

    #[cfg(test)]
    fn arguments(&self, schema_path: &Path, response_path: &Path) -> Vec<String> {
        self.arguments_for_schema(schema_path, response_path, &json!({"type": "object"}), None)
    }

    /// `reading` is a folder the model may read but not change. Without it the model has no tools.
    fn arguments_for_schema(&self, schema_path: &Path, response_path: &Path, schema: &Value, reading: Option<&Path>) -> Vec<String> {
        let schema_file = schema_path.to_string_lossy().to_string();
        let response_file = response_path.to_string_lossy().to_string();
        let folder = reading.map(|folder| folder.to_string_lossy().to_string());
        let mut command = self.command.clone();
        if self.backend == "claude" {
            let model = self.model.clone().unwrap_or_else(|| DEFAULT_CLAUDE_MODEL.into());
            let schema_text = schema.to_string();
            command.extend(["--print", "--model", model.as_str()].map(str::to_string));
            if let Some(effort) = &self.effort {
                command.extend(["--effort".to_string(), effort.clone()]);
            }
            let (tools, system, format) = match folder {
                Some(_) => ("Read,Grep,Glob", "Use only the supplied data and instructions and the notes of the memory folder the data names, which you may search and read but not change. Return the required JSON Schema in the language specified by the task.", "stream-json"),
                None => ("", "Use only the supplied data and instructions. Return the required JSON Schema in the language specified by the task. Do not use tools.", "json"),
            };
            let search = self.search.as_ref().filter(|_| folder.is_some());
            let servers = match search {
                Some(entry) => json!({"mcpServers": {SEARCH_SERVER: entry}}),
                None => json!({"mcpServers": {}}),
            }
            .to_string();
            command.extend(
                [
                    "--disable-slash-commands",
                    "--settings",
                    r#"{"disableAllHooks":true}"#,
                    "--tools",
                    tools,
                    "--strict-mcp-config",
                    "--mcp-config",
                    servers.as_str(),
                    "--no-session-persistence",
                    "--system-prompt",
                    system,
                    "--json-schema",
                    schema_text.as_str(),
                    "--output-format",
                    format,
                ]
                .map(str::to_string),
            );
            if search.is_some() {
                command.extend(["--allowedTools".to_string(), SEARCH_TOOLS.to_string()]);
            }
            if let Some(folder) = folder {
                // stream-json, whose events name every file the model read, requires --verbose.
                command.extend(["--verbose".to_string(), "--add-dir".to_string(), folder]);
            }
        } else {
            let model = self.model.clone().unwrap_or_else(|| DEFAULT_CODEX_MODEL.into());
            let effort = format!("model_reasoning_effort=\"{}\"", self.effort.as_deref().unwrap_or(DEFAULT_CODEX_EFFORT));
            command.extend(["exec", "--ignore-rules", "--ephemeral", "--skip-git-repo-check", "--sandbox", "read-only", "-m", model.as_str(), "-c", effort.as_str()].map(str::to_string));
            let overrides = CODEX_OFF.iter().map(|setting| setting.to_string()).chain(self.user_servers.iter().map(|server| format!("mcp_servers.{server}.enabled=false")));
            for setting in overrides {
                command.extend(["-c".to_string(), setting]);
            }
            command.extend(["--output-schema", schema_file.as_str(), "--output-last-message", response_file.as_str()].map(str::to_string));
            if let Some(folder) = folder {
                command.extend(["-C".to_string(), folder, "--json".to_string()]);
                for setting in self.search.iter().flat_map(|entry| codex_server(SEARCH_SERVER, entry)) {
                    command.extend(["-c".to_string(), setting]);
                }
            }
            command.push("-".to_string());
        }
        command
    }

    pub fn request_json(&self, prompt: &str, schema: &Value, artifacts: &Path) -> Result<Value> {
        self.request(prompt, schema, artifacts, None)
    }

    pub fn request(&self, prompt: &str, schema: &Value, artifacts: &Path, reading: Option<&Path>) -> Result<Value> {
        std::fs::create_dir_all(artifacts)?;
        atomic_write(&artifacts.join("prompt.txt"), prompt.as_bytes())?;
        let schema_path = artifacts.join("schema.json");
        atomic_write(&schema_path, serde_json::to_string_pretty(schema)?.as_bytes())?;
        let response_path = artifacts.join("response.json");
        let _ = std::fs::remove_file(&response_path);
        let command = self.arguments_for_schema(&schema_path, &response_path, schema, reading);
        let output = run::run(&command, &self.workdir, &[], Some(prompt), self.limit.as_ref())?;
        atomic_write(&artifacts.join("model-stderr.txt"), output.stderr.as_bytes())?;
        if reading.is_some() {
            atomic_write(&artifacts.join("events.jsonl"), output.stdout.as_bytes())?;
        }
        let text = if self.backend == "claude" {
            let envelope = if reading.is_some() { stream_result(&output.stdout)? } else { output.stdout.as_str() };
            let text = claude_result(envelope)?;
            atomic_write(&response_path, text.as_bytes())?;
            text
        } else {
            std::fs::read_to_string(&response_path).map_err(|_| Error::rejected("Codex did not write a response file."))?
        };
        serde_json::from_str(text.trim()).map_err(|error| Error::rejected(format!("Invalid model response format: {error}")))
    }
}

fn claude_result(stdout: &str) -> Result<String> {
    let envelope: Value = serde_json::from_str(stdout.trim()).map_err(|error| Error::transient(format!("Could not read the Claude response: {error}")))?;
    if envelope.get("is_error").and_then(Value::as_bool).unwrap_or(false) {
        let message = envelope.get("result").and_then(Value::as_str).unwrap_or("Claude recording model failed");
        return Err(Error::transient(message.to_string()));
    }
    if let Some(structured) = envelope.get("structured_output").filter(|value| !value.is_null()) {
        return Ok(structured.to_string());
    }
    envelope.get("result").and_then(Value::as_str).map(str::to_string).ok_or_else(|| Error::rejected("The Claude response contains no result."))
}

/// Run `attempt` with each entry in order until one can run. Only a failure to run
/// (a usage limit, an expired login, an outage, a timeout) moves on to the next
/// entry; an answer the program rejected is not the CLI's fault and is returned.
pub fn first_that_runs<R, T>(entries: &[R], name: impl Fn(&R) -> &str, mut attempt: impl FnMut(&R) -> Result<T>) -> Result<T> {
    let mut failures: Vec<String> = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        match attempt(entry) {
            Err(error) if error.failure == Failure::Transient && index + 1 < entries.len() => failures.push(format!("{}: {error}", name(entry))),
            Err(error) if error.failure == Failure::Transient && !failures.is_empty() => {
                failures.push(format!("{}: {error}", name(entry)));
                return Err(Error::transient(failures.join("\n")));
            }
            result => return result,
        }
    }
    Err(Error::transient("No recording CLI is configured."))
}

const GET: &str = "mcp__lore_qmd__get";
const MULTI_GET: &str = "mcp__lore_qmd__multi_get";
const RESOURCE: &str = "[Resource from lore_qmd at ";

/// The notes the model read whole as they are now, from the record a reading run leaves
/// (`events.jsonl`: Claude's stream-json events or Codex's JSON events). `notes` holds the
/// current text of each note that can count, and `collections` maps a qmd collection to its
/// folder. `Read` takes a file from disk, so a successful call without a range counts. qmd
/// returns the copy in its index, which can be older than the file, so a note it returned
/// counts only when that copy is the note's current text. Search results, grep and shell
/// commands show parts of notes and never count.
pub fn notes_read(record: &str, memory: &Path, collections: &BTreeMap<String, String>, notes: &BTreeMap<String, String>) -> BTreeSet<String> {
    let mut asked: BTreeMap<String, (String, Value)> = BTreeMap::new();
    let mut read = BTreeSet::new();
    let parts = |value: &Value| value.as_array().cloned().unwrap_or_default();
    for event in record.lines().filter_map(|line| serde_json::from_str::<Value>(line).ok()) {
        match event["type"].as_str() {
            Some("assistant") => {
                for call in parts(&event["message"]["content"]).into_iter().filter(|part| part["type"] == "tool_use") {
                    asked.insert(call["id"].as_str().unwrap_or_default().to_string(), (call["name"].as_str().unwrap_or_default().to_string(), call["input"].clone()));
                }
            }
            Some("user") => {
                for result in parts(&event["message"]["content"]).into_iter().filter(|part| part["type"] == "tool_result") {
                    let Some((name, input)) = asked.remove(result["tool_use_id"].as_str().unwrap_or_default()) else { continue };
                    if result["is_error"].as_bool().unwrap_or(false) {
                        continue;
                    }
                    if name == "Read" && input["offset"].is_null() && input["limit"].is_null() {
                        read.extend(input["file_path"].as_str().and_then(|path| under(memory, path)).filter(|path| notes.contains_key(path)));
                    } else if name == GET || name == MULTI_GET {
                        let texts: Vec<String> = match &result["content"] {
                            Value::String(text) => vec![text.clone()],
                            content => parts(content).iter().filter_map(|part| part["text"].as_str().map(str::to_string)).collect(),
                        };
                        for text in texts {
                            if let Some((uri, body)) = text.strip_prefix(RESOURCE).and_then(|rest| rest.split_once("] ")) {
                                read.extend(returned_whole(uri, body, collections, notes));
                            }
                        }
                    }
                }
            }
            Some("item.completed") => {
                let item = &event["item"];
                let tool = item["tool"].as_str().unwrap_or_default();
                if item["type"] == "mcp_tool_call" && item["server"] == SEARCH_SERVER && item["status"] == "completed" && matches!(tool, "get" | "multi_get") {
                    for part in parts(&item["result"]["content"]).iter().filter(|part| part["type"] == "resource") {
                        let resource = &part["resource"];
                        read.extend(returned_whole(resource["uri"].as_str().unwrap_or_default(), resource["text"].as_str().unwrap_or_default(), collections, notes));
                    }
                }
            }
            _ => {}
        }
    }
    read
}

/// A path under the memory folder, relative to it with forward slashes.
fn under(memory: &Path, path: &str) -> Option<String> {
    let root = memory.to_string_lossy().replace('\\', "/").trim_end_matches('/').to_string();
    let path = path.replace('\\', "/");
    let head = path.get(..root.len())?;
    let same = if cfg!(windows) { head.eq_ignore_ascii_case(&root) } else { head == root };
    same.then(|| path[root.len()..].strip_prefix('/').map(str::to_string)).flatten()
}

/// The note qmd returned as `uri` (`qmd://<collection>/<path>`), when `text` is all of its
/// current text: qmd may put the collection's context and line numbers before it.
fn returned_whole(uri: &str, text: &str, collections: &BTreeMap<String, String>, notes: &BTreeMap<String, String>) -> Option<String> {
    let (collection, rest) = uri.strip_prefix("qmd://")?.split_once('/')?;
    let path = format!("{}{rest}", collections.get(collection)?);
    let current = notes.get(&path)?.replace("\r\n", "\n");
    let text = text.replace("\r\n", "\n");
    let text = match text.strip_prefix("<!-- Context:").and_then(|rest| rest.split_once("-->\n\n")) {
        Some((_, body)) => body.to_string(),
        None => text,
    };
    let plain = without_line_numbers(text.trim_end()).unwrap_or(text);
    (plain.trim_end() == current.trim_end()).then_some(path)
}

/// The text without the `N: ` that qmd puts before each line, when every line has it in order.
fn without_line_numbers(text: &str) -> Option<String> {
    let lines: Option<Vec<&str>> = text
        .split('\n')
        .enumerate()
        .map(|(index, line)| {
            let number = format!("{}:", index + 1);
            line.strip_prefix(&number).map(|rest| rest.strip_prefix(' ').unwrap_or(rest))
        })
        .collect();
    lines.map(|lines| lines.join("\n"))
}

/// The recording CLIs in the order they are tried.
pub struct Recorders(pub Vec<CliRecorder>);

impl Recorders {
    pub fn from_settings(settings: &Settings, workdir: &Path, limit: Limit, listed_default: impl Fn(&str) -> Option<String>) -> Self {
        Self(settings.backends().into_iter().map(|backend| CliRecorder::for_backend(settings, backend, workdir, limit.clone()).defaulting_to(listed_default(backend))).collect())
    }

    pub fn request_json(&self, prompt: &str, schema: &Value, artifacts: &Path) -> Result<Value> {
        first_that_runs(&self.0, |recorder| recorder.backend.as_str(), |recorder| recorder.request_json(prompt, schema, artifacts))
    }

    /// A request whose answer depends on the notes: the model may read the memory folder but not change it.
    pub fn request_json_reading(&self, prompt: &str, schema: &Value, artifacts: &Path, folder: &Path) -> Result<Value> {
        first_that_runs(&self.0, |recorder| recorder.backend.as_str(), |recorder| recorder.request(prompt, schema, artifacts, Some(folder)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Models;

    #[test]
    fn the_sync_prompt_is_written_in_english() {
        assert!(!include_str!("sync-prompt.md").chars().any(|c| ('\u{AC00}'..='\u{D7A3}').contains(&c)));
    }

    #[test]
    fn claude_envelope_prefers_structured_output_and_reports_errors() {
        let summary = |text: String| serde_json::from_str::<Value>(&text).unwrap()["summary"].clone();
        let structured = r#"{"is_error": false, "structured_output": {"summary": "s", "changes": []}, "result": "ignored"}"#;
        assert_eq!(summary(claude_result(structured).unwrap()), "s");
        let text = r#"{"is_error": false, "result": "{\"summary\": \"t\", \"changes\": []}"}"#;
        assert_eq!(summary(claude_result(text).unwrap()), "t");
        assert!(claude_result(r#"{"is_error": true, "result": "limit"}"#).is_err());
    }

    #[test]
    fn a_streamed_claude_run_is_read_from_its_last_result_event() {
        let stdout = "{\"type\":\"system\"}\n{\"type\":\"assistant\"}\n{\"type\":\"result\",\"is_error\":false,\"structured_output\":{\"a\":1}}\n";
        assert_eq!(claude_result(stream_result(stdout).unwrap()).unwrap(), "{\"a\":1}");
        assert!(stream_result("{\"type\":\"assistant\"}\n").is_err());
    }

    #[test]
    fn a_run_keeps_the_users_setup_but_not_what_acts_on_its_own_and_reads_only_the_notes() {
        let mut settings = Settings { backend: "claude".into(), ..Settings::default() };
        settings.commands.claude = vec!["claude.exe".into()];
        settings.commands.codex = vec!["codex.exe".into()];
        let schema = json!({"type": "object"});
        let folder = Path::new("D:/memory");
        let search = json!({"type": "stdio", "command": "C:\\node.exe", "args": ["qmd", "mcp", "--index", "index"], "env": {"QMD_CONFIG_DIR": "D:/q/config", "XDG_CACHE_HOME": "D:/q/cache"}});
        let args = |settings: &Settings, reading: Option<&Path>| {
            let mut recorder = CliRecorder::from_settings(settings, Path::new("w"), run::fixed(1));
            recorder.search = Some(search.clone());
            recorder.user_servers = vec!["atlassian".into(), "qmd".into()];
            recorder.arguments_for_schema(Path::new("s.json"), Path::new("r.json"), &schema, reading)
        };
        let after = |args: &[String], flag: &str| args[args.iter().position(|arg| arg == flag).unwrap() + 1].clone();
        let has = |args: &[String], arg: &str| args.iter().any(|each| each == arg);

        let claude = args(&settings, Some(folder));
        assert!(!has(&claude, "--safe-mode") && has(&claude, "--disable-slash-commands"));
        assert_eq!(after(&claude, "--settings"), r#"{"disableAllHooks":true}"#);
        assert_eq!(after(&claude, "--tools"), "Read,Grep,Glob");
        assert_eq!(after(&claude, "--add-dir"), "D:/memory");
        assert_eq!(after(&claude, "--output-format"), "stream-json");
        assert!(has(&claude, "--strict-mcp-config"));
        assert_eq!(serde_json::from_str::<Value>(&after(&claude, "--mcp-config")).unwrap(), json!({"mcpServers": {"lore_qmd": search}}));
        assert_eq!(after(&claude, "--allowedTools"), SEARCH_TOOLS);
        let without = args(&settings, None);
        assert_eq!((after(&without, "--tools").as_str(), after(&without, "--output-format").as_str()), ("", "json"));
        assert_eq!(after(&without, "--mcp-config"), r#"{"mcpServers":{}}"#);
        assert!(!has(&without, "--add-dir") && !has(&without, "--allowedTools") && has(&without, "--disable-slash-commands"));

        settings.backend = "codex".into();
        let codex = args(&settings, Some(folder));
        assert!(!has(&codex, "--ignore-user-config"));
        assert_eq!(after(&codex, "--sandbox"), "read-only");
        assert_eq!(after(&codex, "-C"), "D:/memory");
        assert!(has(&codex, "--json") && has(&codex, "--ephemeral"));
        for setting in CODEX_OFF.into_iter().chain(["mcp_servers.atlassian.enabled=false", "mcp_servers.qmd.enabled=false"]) {
            assert!(has(&codex, setting), "{setting}");
        }
        assert!(has(&codex, r#"mcp_servers.lore_qmd.command="C:\\node.exe""#));
        assert!(has(&codex, r#"mcp_servers.lore_qmd.args=["qmd","mcp","--index","index"]"#));
        assert!(has(&codex, r#"mcp_servers.lore_qmd.env={QMD_CONFIG_DIR="D:/q/config",XDG_CACHE_HOME="D:/q/cache"}"#));
        assert_eq!(codex.last().map(String::as_str), Some("-"));
        let plain = args(&settings, None);
        assert!(has(&plain, "features.hooks=false") && has(&plain, "mcp_servers.atlassian.enabled=false"));
        assert!(!has(&plain, "-C") && !plain.iter().any(|arg| arg.starts_with("mcp_servers.lore_qmd")));
        assert_eq!(plain.last().map(String::as_str), Some("-"));
    }

    #[test]
    fn recorder_runs_the_model_set_for_the_chosen_cli() {
        let mut settings = Settings { backend: "claude".into(), models: Models { codex: Some("gpt-x".into()), claude: Some("opus".into()) }, ..Settings::default() };
        settings.commands.claude = vec!["claude.exe".into()];
        settings.commands.codex = vec!["codex.exe".into()];
        let model_in = |settings: &Settings| {
            let args = CliRecorder::from_settings(settings, Path::new("w"), run::fixed(1)).arguments(Path::new("s.json"), Path::new("r.json"));
            let flag = if settings.backend == "claude" { "--model" } else { "-m" };
            args[args.iter().position(|arg| arg == flag).unwrap() + 1].clone()
        };
        assert_eq!(model_in(&settings), "opus");
        settings.backend = "codex".into();
        assert_eq!(model_in(&settings), "gpt-x");
        settings.models = Models::default();
        assert_eq!(model_in(&settings), DEFAULT_CODEX_MODEL);
    }

    #[test]
    fn the_next_cli_runs_only_when_the_one_before_it_could_not_run() {
        let run = |replies: [Option<Result<u8>>; 2]| {
            let mut replies = replies;
            let mut called = Vec::new();
            let result = first_that_runs(&["claude", "codex"], |name| *name, |name| {
                called.push(*name);
                replies[called.len() - 1].take().expect("this CLI must not run")
            });
            (result, called)
        };

        let (result, called) = run([Some(Ok(1)), None]);
        assert_eq!((result.unwrap(), called), (1, vec!["claude"]));

        let (result, called) = run([Some(Err(Error::transient("usage limit reached"))), Some(Ok(2))]);
        assert_eq!((result.unwrap(), called), (2, vec!["claude", "codex"]));

        let (result, called) = run([Some(Err(Error::rejected("bad quotation"))), None]);
        let error = result.unwrap_err();
        assert_eq!((error.failure, error.message.as_str(), called), (Failure::Rejected, "bad quotation", vec!["claude"]));

        let (result, _) = run([Some(Err(Error::transient("usage limit reached"))), Some(Err(Error::transient("not signed in")))]);
        let error = result.unwrap_err();
        assert_eq!((error.failure, error.message.as_str()), (Failure::Transient, "claude: usage limit reached\ncodex: not signed in"));

        let (result, _) = run([Some(Err(Error::transient("usage limit reached"))), Some(Err(Error::rejected("bad quotation")))]);
        let error = result.unwrap_err();
        assert_eq!((error.failure, error.message.as_str()), (Failure::Rejected, "bad quotation"));

        let alone = first_that_runs(&["claude"], |name| *name, |_| Err::<u8, _>(Error::transient("usage limit reached"))).unwrap_err();
        assert_eq!((alone.failure, alone.message.as_str()), (Failure::Transient, "usage limit reached"));
    }

    #[test]
    fn each_cli_in_the_order_runs_its_own_model_and_command() {
        let mut settings = Settings { backend: "claude".into(), fallback: true, models: Models { codex: None, claude: Some("opus".into()) }, ..Settings::default() };
        settings.commands.claude = vec!["claude.exe".into()];
        settings.commands.codex = vec!["codex.exe".into()];
        let listed = |backend: &str| (backend == "codex").then(|| "gpt-listed".to_string());
        let described = |settings: &Settings| -> Vec<(String, Option<String>, Vec<String>)> {
            Recorders::from_settings(settings, Path::new("w"), run::fixed(1), listed).0.into_iter().map(|recorder| (recorder.backend, recorder.model, recorder.command)).collect()
        };
        assert_eq!(described(&settings), vec![("claude".into(), Some("opus".into()), vec!["claude.exe".to_string()]), ("codex".into(), Some("gpt-listed".into()), vec!["codex.exe".to_string()])]);
        settings.fallback = false;
        assert_eq!(described(&settings).len(), 1);
    }

    #[test]
    fn a_listed_default_applies_only_when_no_model_is_chosen() {
        let mut settings = Settings { backend: "codex".into(), ..Settings::default() };
        settings.commands.codex = vec!["codex.exe".into()];
        let chosen = |settings: &Settings| CliRecorder::from_settings(settings, Path::new("w"), run::fixed(1)).defaulting_to(Some("gpt-listed".into())).model;
        assert_eq!(chosen(&settings).as_deref(), Some("gpt-listed"));
        settings.models.codex = Some("gpt-mine".into());
        assert_eq!(chosen(&settings).as_deref(), Some("gpt-mine"));
    }

    #[test]
    fn effort_is_passed_to_each_cli_and_codex_keeps_low_by_default() {
        let mut settings = Settings { backend: "codex".into(), ..Settings::default() };
        settings.commands.codex = vec!["codex.exe".into()];
        settings.commands.claude = vec!["claude.exe".into()];
        let args = |settings: &Settings| CliRecorder::from_settings(settings, Path::new("w"), run::fixed(1)).arguments(Path::new("s.json"), Path::new("r.json"));
        assert!(args(&settings).contains(&"model_reasoning_effort=\"low\"".to_string()));
        settings.efforts.codex = Some("high".into());
        assert!(args(&settings).contains(&"model_reasoning_effort=\"high\"".to_string()));

        settings.backend = "claude".into();
        assert!(!args(&settings).contains(&"--effort".to_string()));
        settings.efforts.claude = Some("max".into());
        let claude = args(&settings);
        let at = claude.iter().position(|arg| arg == "--effort").unwrap();
        assert_eq!(claude[at + 1], "max");
    }

    fn objects(schema: &Value) -> Vec<&Value> {
        let mut found = Vec::new();
        if schema["type"] == "object" {
            found.push(schema);
        }
        for child in schema["properties"].as_object().into_iter().flat_map(|properties| properties.values()).chain(schema.get("items")) {
            found.extend(objects(child));
        }
        found
    }

    // Codex accepts a schema only when every object lists all of its properties as required.
    #[test]
    fn every_schema_requires_every_property_of_every_object_for_strict_output() {
        for (schema, count) in [(crate::prepare::schema(), 3), (crate::integrate::schema(), 4)] {
            let all = objects(&schema);
            assert_eq!(all.len(), count);
            for object in all {
                let mut required: Vec<&str> = object["required"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
                let mut properties: Vec<&str> = object["properties"].as_object().unwrap().keys().map(String::as_str).collect();
                required.sort();
                properties.sort();
                assert_eq!(required, properties);
                assert_eq!(object["additionalProperties"], false);
            }
        }
        let edit = &crate::integrate::schema()["properties"]["changes"]["items"]["properties"]["edits"]["items"];
        let ops: Vec<&str> = edit["properties"]["op"]["enum"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        assert_eq!(ops, crate::proposal::EDIT_OPS);
    }

    const ALERTS: &str = "---\nqmd:\n  metadata:\n    project: \"demo\"\n---\n\n# 알림\n\n알림은 띄우지 않는다.\n";
    const QUEUE: &str = "# 큐\n\n하나씩 처리한다.\n";

    fn read_from(record: &[Value]) -> Vec<String> {
        let collections = BTreeMap::from([("common".to_string(), "common/".to_string()), ("project-demo".to_string(), "projects/demo/".to_string())]);
        let notes = BTreeMap::from([("common/a.md", "# A\n"), ("common/b.md", "# B\n"), ("projects/demo/alerts.md", ALERTS), ("projects/demo/queue.md", QUEUE)].map(|(path, text)| (path.to_string(), text.to_string())));
        let text = record.iter().map(Value::to_string).collect::<Vec<_>>().join("\n");
        notes_read(&text, Path::new("D:/memory"), &collections, &notes).into_iter().collect()
    }

    fn claude_call(id: &str, name: &str, input: Value, content: Value, failed: bool) -> [Value; 2] {
        [
            json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": id, "name": name, "input": input}]}}),
            json!({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": id, "content": content, "is_error": failed}]}}),
        ]
    }

    /// How Claude shows what a qmd call returned: one text per note, after the note's address.
    fn returned(texts: &[(&str, &str)]) -> Value {
        json!(texts.iter().map(|(uri, text)| json!({"type": "text", "text": format!("[Resource from lore_qmd at {uri}] {text}")})).collect::<Vec<_>>())
    }

    fn numbered(text: &str) -> String {
        text.split('\n').enumerate().map(|(index, line)| format!("{}: {line}", index + 1)).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn a_claude_run_reads_a_note_with_read_when_it_took_the_whole_file() {
        let record: Vec<Value> = [
            claude_call("1", "Read", json!({"file_path": "D:\\memory\\common\\a.md"}), json!("1→# A"), false),
            claude_call("2", "Read", json!({"file_path": "D:/memory/common/b.md", "offset": 10, "limit": 5}), json!("…"), false),
            claude_call("3", "Read", json!({"file_path": "D:/memory/projects/demo/queue.md"}), json!("File does not exist."), true),
            claude_call("4", "Read", json!({"file_path": "D:/elsewhere/projects/demo/alerts.md"}), json!("…"), false),
            claude_call("5", "Grep", json!({"pattern": "알림", "path": "D:/memory"}), json!("projects/demo/alerts.md:9:알림은 띄우지 않는다."), false),
        ]
        .concat();
        assert_eq!(read_from(&record), vec!["common/a.md"]);
    }

    #[test]
    fn a_note_qmd_returned_counts_only_when_it_returned_the_current_text_whole() {
        let context = format!("<!-- Context: Decisions of the demo project. -->\n\n{}", numbered(ALERTS));
        let record: Vec<Value> = [
            claude_call("1", "mcp__lore_qmd__get", json!({"file": "project-demo/alerts.md"}), returned(&[("qmd://project-demo/alerts.md", &context)]), false),
            claude_call("2", "mcp__lore_qmd__multi_get", json!({"pattern": "common/*.md"}), json!([{"type": "text", "text": "Errors:\nFile not found: common/c.md"}, {"type": "text", "text": "[Resource from lore_qmd at qmd://common/a.md] # A\n"}]), false),
            // An older copy in the index, a cut-off copy and a search result do not count.
            claude_call("3", "mcp__lore_qmd__get", json!({"file": "project-demo/queue.md"}), returned(&[("qmd://project-demo/queue.md", "# 큐\n\n둘까지 처리한다.\n")]), false),
            claude_call("4", "mcp__lore_qmd__multi_get", json!({"pattern": "common/b.md", "maxLines": 1}), returned(&[("qmd://common/b.md", "# B\n\n[... truncated 1 more lines]")]), false),
            claude_call("5", "mcp__lore_qmd__query", json!({"searches": []}), json!("{\"results\":[{\"file\":\"common/b.md\"}]}"), false),
        ]
        .concat();
        assert_eq!(read_from(&record), vec!["common/a.md", "projects/demo/alerts.md"]);
    }

    #[test]
    fn a_codex_run_reads_a_note_through_a_completed_qmd_call_and_not_through_the_shell() {
        let item = |tool: &str, uri: &str, text: &str, status: &str| json!({"type": "item.completed", "item": {"type": "mcp_tool_call", "server": "lore_qmd", "tool": tool, "arguments": {}, "status": status, "result": {"content": [{"type": "resource", "resource": {"uri": uri, "text": text}}]}}});
        let record = vec![
            item("get", "qmd://common/a.md", "# A\n", "completed"),
            item("get", "qmd://common/b.md", "# B\n", "failed"),
            item("multi_get", "qmd://project-demo/queue.md", &numbered(QUEUE), "completed"),
            json!({"type": "item.completed", "item": {"type": "command_execution", "command": "cat D:/memory/projects/demo/alerts.md", "aggregated_output": ALERTS, "status": "completed"}}),
        ];
        assert_eq!(read_from(&record), vec!["common/a.md", "projects/demo/queue.md"]);
        assert!(read_from(&[]).is_empty());
    }
}
