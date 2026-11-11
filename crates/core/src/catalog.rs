//! Model lists reported by the recording CLIs. Neither command is a documented,
//! stable interface, so only a few fields are read and anything else means the
//! list is unavailable; recording never depends on it.
use std::path::Path;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::run;
use crate::settings::Settings;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    pub value: String,
    pub label: String,
    pub resolved: Option<String>,
    pub efforts: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Catalog {
    pub codex: Option<Vec<Choice>>,
    pub claude: Option<Vec<Choice>>,
}

const LIMIT: Duration = Duration::from_secs(30);
const INITIALIZE: &str = "{\"type\":\"control_request\",\"request_id\":\"lore-models\",\"request\":{\"subtype\":\"initialize\"}}\n";

fn strings(levels: Option<&Value>, read: impl Fn(&Value) -> Option<&str>) -> Vec<String> {
    levels.and_then(Value::as_array).map(|levels| levels.iter().filter_map(&read).map(str::to_string).collect()).unwrap_or_default()
}

pub fn codex(stdout: &str) -> Option<Vec<Choice>> {
    let root: Value = serde_json::from_str(stdout.trim()).ok()?;
    let mut listed: Vec<(i64, Choice)> = root
        .get("models")?
        .as_array()?
        .iter()
        .filter(|model| model.get("visibility").and_then(Value::as_str) == Some("list"))
        .filter_map(|model| {
            let value = model.get("slug")?.as_str()?.to_string();
            let label = model.get("display_name").and_then(Value::as_str).unwrap_or(&value).to_string();
            let efforts = strings(model.get("supported_reasoning_levels"), |level| level.get("effort").and_then(Value::as_str));
            Some((model.get("priority").and_then(Value::as_i64).unwrap_or(i64::MAX), Choice { value, label, resolved: None, efforts }))
        })
        .collect();
    listed.sort_by_key(|(priority, _)| *priority);
    let list: Vec<Choice> = listed.into_iter().map(|(_, choice)| choice).collect();
    (!list.is_empty()).then_some(list)
}

pub fn claude(stdout: &str) -> Option<Vec<Choice>> {
    let response = stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|line| line.get("type").and_then(Value::as_str) == Some("control_response"))?;
    let list: Vec<Choice> = response
        .get("response")?
        .get("response")?
        .get("models")?
        .as_array()?
        .iter()
        .filter_map(|model| {
            let value = model.get("value")?.as_str()?.to_string();
            // Claude Code's own default model, not the recording default.
            if value == "default" {
                return None;
            }
            let label = model.get("displayName").and_then(Value::as_str).unwrap_or(&value).to_string();
            let resolved = model.get("resolvedModel").and_then(Value::as_str).map(str::to_string);
            let efforts = strings(model.get("supportedEffortLevels"), Value::as_str);
            Some(Choice { value, label, resolved, efforts })
        })
        .collect();
    (!list.is_empty()).then_some(list)
}

fn with(command: &[String], args: &[&str]) -> Option<Vec<String>> {
    (!command.is_empty()).then(|| command.iter().cloned().chain(args.iter().map(|arg| arg.to_string())).collect())
}

pub fn fetch(settings: &Settings, workdir: &Path) -> Catalog {
    let stdout = |command: Vec<String>, stdin: Option<&str>| run::run(&command, workdir, &[], stdin, &|| LIMIT).ok().map(|output| output.stdout);
    let claude_args = ["--safe-mode", "--print", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose", "--no-session-persistence", "--strict-mcp-config", "--mcp-config", r#"{"mcpServers":{}}"#];
    Catalog {
        codex: with(&settings.commands.codex, &["debug", "models"]).and_then(|command| stdout(command, None)).and_then(|text| codex(&text)),
        claude: with(&settings.commands.claude, &claude_args).and_then(|command| stdout(command, Some(INITIALIZE))).and_then(|text| claude(&text)),
    }
}

impl Catalog {
    pub fn default_model(&self, backend: &str) -> Option<String> {
        if backend != "codex" {
            return None;
        }
        self.codex.as_ref()?.first().map(|choice| choice.value.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CODEX: &str = r#"{"models": [
        {"slug": "gpt-b", "display_name": "GPT-B", "visibility": "list", "priority": 2, "supported_reasoning_levels": [{"effort": "low", "description": "x"}, {"effort": "max", "description": "y"}]},
        {"slug": "gpt-hidden", "display_name": "Hidden", "visibility": "hide", "priority": 0, "supported_reasoning_levels": []},
        {"slug": "gpt-a", "display_name": "GPT-A", "visibility": "list", "priority": 1, "supported_reasoning_levels": [{"effort": "low"}, {"effort": "ultra"}]},
        {"display_name": "No slug", "visibility": "list", "priority": 3}
    ]}"#;

    #[test]
    fn codex_lists_visible_models_in_priority_order_with_their_efforts() {
        let list = codex(CODEX).unwrap();
        assert_eq!(list.iter().map(|choice| choice.value.as_str()).collect::<Vec<_>>(), ["gpt-a", "gpt-b"]);
        assert_eq!(list[0].label, "GPT-A");
        assert_eq!(list[0].efforts, ["low", "ultra"]);
        assert_eq!(list[1].efforts, ["low", "max"]);
        let catalog = Catalog { codex: Some(list), claude: None };
        assert_eq!(catalog.default_model("codex").as_deref(), Some("gpt-a"));
        assert_eq!(catalog.default_model("claude"), None);
    }

    #[test]
    fn claude_reads_the_initialize_response_and_drops_its_own_default_entry() {
        let stdout = concat!(
            r#"{"type":"system","subtype":"ui_invalidate"}"#, "\n",
            r#"{"type":"control_response","response":{"subtype":"success","request_id":"lore-models","response":{"models":["#,
            r#"{"value":"default","resolvedModel":"claude-opus-5-5","displayName":"Default (recommended)","supportedEffortLevels":["low"]},"#,
            r#"{"value":"haiku","resolvedModel":"claude-haiku-5-5","displayName":"Haiku 5.5","supportsEffort":true,"supportedEffortLevels":["low","medium","max"]},"#,
            r#"{"value":"claude-haiku-4-5-20251001","resolvedModel":"claude-haiku-4-5-20251001","displayName":"Haiku 4.5"}"#,
            r#"]}}}"#, "\n"
        );
        let list = claude(stdout).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0], Choice { value: "haiku".into(), label: "Haiku 5.5".into(), resolved: Some("claude-haiku-5-5".into()), efforts: vec!["low".into(), "medium".into(), "max".into()] });
        assert!(list[1].efforts.is_empty());
    }

    #[test]
    fn unreadable_or_empty_output_means_no_list() {
        for text in ["", "not json", r#"{"models": []}"#, r#"{"models": [{"slug": "x", "visibility": "hide"}]}"#, r#"{"data": []}"#] {
            assert_eq!(codex(text), None, "{text}");
        }
        for text in ["", r#"{"type":"result"}"#, r#"{"type":"control_response","response":{"subtype":"error"}}"#] {
            assert_eq!(claude(text), None, "{text}");
        }
    }
}
