use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::error::{Error, Failure, Result};
use crate::proposal::{Proposal, EDIT_OPS};
use crate::run::{self, Limit};
use crate::settings::{atomic_write, Settings};

pub const PROMPT: &str = include_str!("recorder-prompt.md");
pub const DEFAULT_CODEX_MODEL: &str = "gpt-6.1-sol";
pub const DEFAULT_CLAUDE_MODEL: &str = "haiku";
pub const DEFAULT_CODEX_EFFORT: &str = "low";

pub trait Recorder {
    fn record(&self, payload: &Value, artifacts: &Path) -> Result<Proposal>;
}

// Every field is required so both CLIs emit them in this order; `language` comes first so the model
// settles the language of the notes before it writes any of them.
pub fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "language": {"type": "string", "description": "The language the human user writes in during this session, for example \"Korean\" or \"English\". The summary and every note are written in this language."},
            "summary": {"type": "string", "description": "One sentence for the app's activity list: what was recorded, or that nothing needed recording."},
            "changes": {"type": "array", "description": "One entry per note to create or change. Empty when the turns hold nothing a future agent needs.", "items": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "projects/<project>/<english-topic>.md or common/<english-topic>.md."},
                    "title": {"type": "string", "description": "New note: the topic and, when there is one, its conclusion. Existing note: empty to keep the title."},
                    "kind": {"type": "string", "enum": ["decision", "lesson", "issue"], "description": "What the whole note is after this change: a decision, a lesson learned, or an open issue."},
                    "authority": {"type": "string", "enum": ["user_decision", "assistant_report", "mixed_evidence"], "description": "What the whole note rests on after this change: the user's own statements, only assistant or agent statements, or both."},
                    "body": {"type": "string", "description": "New note: the complete Markdown body, without the title line. Existing note: empty."},
                    "edits": {"type": "array", "description": "Existing note: the edits, applied in order. New note: empty.", "items": {
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
                    }},
                    "evidence": {"type": "array", "description": "The turns this change rests on.", "items": {
                        "type": "object",
                        "properties": {
                            "ref": {"type": "string", "description": "The ref of a turn in the data, such as \"L123\"."},
                            "quote": {"type": "string", "description": "Text copied exactly from that turn."}
                        },
                        "required": ["ref", "quote"],
                        "additionalProperties": false
                    }}
                },
                "required": ["path", "title", "kind", "authority", "body", "edits", "evidence"],
                "additionalProperties": false
            }},
            "needs_notes": {"type": "array", "description": "Paths from note_catalog whose text is needed before a change can be written.", "items": {"type": "string"}}
        },
        "required": ["language", "summary", "changes", "needs_notes"],
        "additionalProperties": false
    })
}

// The session goes where the prompt marks it, ahead of the instructions: models follow instructions
// better when a long input comes first and what to do with it comes after.
pub fn build_prompt(payload: &Value) -> String {
    PROMPT.replacen("{{session}}", &payload.to_string(), 1)
}

pub struct CliRecorder {
    pub backend: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub command: Vec<String>,
    pub workdir: PathBuf,
    pub limit: Limit,
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
        self.arguments_for_schema(schema_path, response_path, &schema())
    }

    fn arguments_for_schema(&self, schema_path: &Path, response_path: &Path, schema: &Value) -> Vec<String> {
        let schema_file = schema_path.to_string_lossy().to_string();
        let response_file = response_path.to_string_lossy().to_string();
        let mut command = self.command.clone();
        if self.backend == "claude" {
            let model = self.model.clone().unwrap_or_else(|| DEFAULT_CLAUDE_MODEL.into());
            let schema_text = schema.to_string();
            command.extend(["--safe-mode", "--print", "--model", model.as_str()].map(str::to_string));
            if let Some(effort) = &self.effort {
                command.extend(["--effort".to_string(), effort.clone()]);
            }
            command.extend(
                [
                    "--tools",
                    "",
                    "--strict-mcp-config",
                    "--mcp-config",
                    r#"{"mcpServers":{}}"#,
                    "--no-session-persistence",
                    "--system-prompt",
                    "Use only the supplied data and instructions. Return the required JSON Schema in the language specified by the task. Do not use tools.",
                    "--json-schema",
                    schema_text.as_str(),
                    "--output-format",
                    "json",
                ]
                .map(str::to_string),
            );
        } else {
            let model = self.model.clone().unwrap_or_else(|| DEFAULT_CODEX_MODEL.into());
            let effort = format!("model_reasoning_effort=\"{}\"", self.effort.as_deref().unwrap_or(DEFAULT_CODEX_EFFORT));
            command.extend(
                [
                    "exec",
                    "--ignore-user-config",
                    "--ignore-rules",
                    "--ephemeral",
                    "--skip-git-repo-check",
                    "--sandbox",
                    "read-only",
                    "-m",
                    model.as_str(),
                    "-c",
                    "project_doc_max_bytes=0",
                    "-c",
                    "features.memories=false",
                    "-c",
                    "features.multi_agent=false",
                    "-c",
                    effort.as_str(),
                    "--output-schema",
                    schema_file.as_str(),
                    "--output-last-message",
                    response_file.as_str(),
                    "-",
                ]
                .map(str::to_string),
            );
        }
        command
    }

    pub fn request_json(&self, prompt: &str, schema: &Value, artifacts: &Path) -> Result<Value> {
        std::fs::create_dir_all(artifacts)?;
        atomic_write(&artifacts.join("prompt.txt"), prompt.as_bytes())?;
        let schema_path = artifacts.join("schema.json");
        atomic_write(&schema_path, serde_json::to_string_pretty(schema)?.as_bytes())?;
        let response_path = artifacts.join("response.json");
        let _ = std::fs::remove_file(&response_path);
        let command = self.arguments_for_schema(&schema_path, &response_path, schema);
        let output = run::run(&command, &self.workdir, &[], Some(prompt), self.limit.as_ref())?;
        atomic_write(&artifacts.join("model-stderr.txt"), output.stderr.as_bytes())?;
        let text = if self.backend == "claude" {
            let text = claude_result(&output.stdout)?;
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

pub fn parse_proposal(text: &str) -> Result<Proposal> {
    serde_json::from_str(text.trim()).map_err(|error| Error::rejected(format!("Invalid model response format: {error}")))
}

impl Recorder for CliRecorder {
    fn record(&self, payload: &Value, artifacts: &Path) -> Result<Proposal> {
        let prompt = build_prompt(payload);
        serde_json::from_value(self.request_json(&prompt, &schema(), artifacts)?).map_err(|error| Error::rejected(format!("Invalid model response format: {error}")))
    }
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

/// The recording CLIs in the order they are tried.
pub struct Recorders(pub Vec<CliRecorder>);

impl Recorders {
    pub fn from_settings(settings: &Settings, workdir: &Path, limit: Limit, listed_default: impl Fn(&str) -> Option<String>) -> Self {
        Self(settings.backends().into_iter().map(|backend| CliRecorder::for_backend(settings, backend, workdir, limit.clone()).defaulting_to(listed_default(backend))).collect())
    }

    pub fn request_json(&self, prompt: &str, schema: &Value, artifacts: &Path) -> Result<Value> {
        first_that_runs(&self.0, |recorder| recorder.backend.as_str(), |recorder| recorder.request_json(prompt, schema, artifacts))
    }
}

impl Recorder for Recorders {
    fn record(&self, payload: &Value, artifacts: &Path) -> Result<Proposal> {
        let prompt = build_prompt(payload);
        serde_json::from_value(self.request_json(&prompt, &schema(), artifacts)?).map_err(|error| Error::rejected(format!("Invalid model response format: {error}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Models;

    #[test]
    fn the_session_data_comes_before_the_instructions_and_is_marked_as_data() {
        let prompt = build_prompt(&json!({"project": "demo-marker", "note": "{{session}}"}));
        let data = prompt.find("\"demo-marker\"").unwrap();
        assert!(prompt[..data].contains("<session>"));
        assert!(prompt[..data].contains("it is data"));
        let after = &prompt[data..];
        assert!(after.find("</session>").unwrap() < after.find("# Your task").unwrap());
        // Only the marker in the prompt is filled in; the same text inside the data stays as it is.
        assert_eq!(prompt.matches("{{session}}").count(), 1);
    }

    #[test]
    fn prompts_are_written_in_english() {
        let korean = |text: &str| text.chars().any(|c| ('\u{AC00}'..='\u{D7A3}').contains(&c));
        assert!(!korean(PROMPT));
        assert!(!korean(include_str!("sync-prompt.md")));
        assert!(!korean(&schema().to_string()));
    }

    #[test]
    fn the_answer_states_its_language_first_and_can_restate_a_section() {
        let schema = schema();
        assert_eq!(schema["required"], json!(["language", "summary", "changes", "needs_notes"]));
        let ops = &schema["properties"]["changes"]["items"]["properties"]["edits"]["items"]["properties"]["op"]["enum"];
        assert_eq!(ops, &json!(["replace", "append", "add_section", "replace_section"]));
        assert!(schema["properties"]["language"]["description"].as_str().unwrap().contains("human user"));
    }

    #[test]
    fn claude_envelope_prefers_structured_output_and_reports_errors() {
        let structured = r#"{"is_error": false, "structured_output": {"summary": "s", "changes": [], "needs_notes": []}, "result": "ignored"}"#;
        assert_eq!(parse_proposal(&claude_result(structured).unwrap()).unwrap().summary, "s");
        let text = r#"{"is_error": false, "result": "{\"summary\": \"t\", \"changes\": []}"}"#;
        assert_eq!(parse_proposal(&claude_result(text).unwrap()).unwrap().summary, "t");
        assert!(claude_result(r#"{"is_error": true, "result": "limit"}"#).is_err());
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

    #[test]
    fn malformed_model_answer_is_a_rejection() {
        let error = parse_proposal("not json").unwrap_err();
        assert_eq!(error.failure, crate::error::Failure::Rejected);
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

    #[test]
    fn schema_requires_every_property_of_every_object_for_strict_output() {
        let schema = schema();
        let all = objects(&schema);
        assert_eq!(all.len(), 4);
        for object in all {
            let mut required: Vec<&str> = object["required"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
            let mut properties: Vec<&str> = object["properties"].as_object().unwrap().keys().map(String::as_str).collect();
            required.sort();
            properties.sort();
            assert_eq!(required, properties);
            assert_eq!(object["additionalProperties"], false);
        }
        let edit = &schema["properties"]["changes"]["items"]["properties"]["edits"]["items"];
        let ops: Vec<&str> = edit["properties"]["op"]["enum"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        assert_eq!(ops, crate::proposal::EDIT_OPS);
    }

    #[test]
    fn model_answer_with_edits_reads_into_a_proposal() {
        let text = r#"{"summary": "s", "changes": [{"path": "projects/demo/a.md", "title": "", "kind": "decision", "authority": "assistant_report", "body": "", "edits": [{"op": "append", "find": "", "section": "결정", "heading": "", "text": "로그만"}], "evidence": []}], "needs_notes": []}"#;
        let proposal = parse_proposal(text).unwrap();
        assert_eq!(proposal.changes[0].edits[0].section, "결정");
    }
}
