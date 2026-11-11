//! User-scoped integrations. Only owned entries are updated or removed; the rest
//! of each client's configuration remains under the client's/user's control.
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use toml_edit::{DocumentMut, Item, Table};

use crate::error::{Error, Result};
use crate::settings::{atomic_write, Paths, Settings};
use crate::transcript::{digest, Provider};

pub const RULES: &str = "<!-- lore-memory:start -->\n## Project memory (qmd)\n\n- Before project work, search relevant project decisions and lessons in qmd, including `common`. Use the project ID from `.lore.json` and explicitly set `collections`.\n- Read relevant document bodies. Check original sources when approval is unclear. Distinguish agent proposals from user decisions, respect their scope, and follow current instructions.\n- Reuse previously retrieved context when it remains relevant. Do not treat a failed lookup as evidence that no records exist.\n- Do not edit memory notes or add session summaries unless requested. Leave automatic recording to Lore.\n<!-- lore-memory:end -->";
pub const SKILL: &str = include_str!("lore-memory/SKILL.md");

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Progress {
    pub step: usize,
    pub completed: bool,
    pub deferred: bool,
    pub providers: Vec<Provider>,
    pub test_started_at: Option<i64>,
    pub recall_confirmed: bool,
}

pub fn progress_path(paths: &Paths) -> PathBuf { paths.root.join("setup.json") }
pub fn progress(paths: &Paths) -> Result<Progress> {
    Ok(read(&progress_path(paths))?.map(|text| serde_json::from_str(&text)).transpose()?.unwrap_or_default())
}
pub fn save_progress(paths: &Paths, progress: &Progress) -> Result<()> {
    atomic_write(&progress_path(paths), serde_json::to_string_pretty(progress)?.as_bytes())
}

fn read(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
fn bad(message: impl Into<String>) -> Error { Error::permanent(message) }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Kind { Mcp, Rules, Skill }

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Owned { kind: Kind, installed: String, #[serde(default)] previous: Option<String> }

fn receipt_path(paths: &Paths, provider: Provider) -> PathBuf {
    paths.root.join("integrations").join(format!("{}.json", provider.as_str()))
}
fn receipts(paths: &Paths, provider: Provider) -> Result<Vec<Owned>> {
    Ok(read(&receipt_path(paths, provider))?.map(|text| serde_json::from_str(&text)).transpose()?.unwrap_or_default())
}
fn save_receipts(paths: &Paths, provider: Provider, owned: &[Owned]) -> Result<()> {
    atomic_write(&receipt_path(paths, provider), serde_json::to_string_pretty(owned)?.as_bytes())
}

fn client_dir(home: &Path, provider: Provider) -> PathBuf {
    // Honor the documented Codex config root without copying it to another PC.
    if provider == Provider::Codex && home == crate::platform::home() {
        if let Some(root) = std::env::var_os("CODEX_HOME").filter(|s| !s.is_empty()) { return PathBuf::from(root); }
    }
    home.join(if provider == Provider::Codex { ".codex" } else { ".claude" })
}
fn resource_path(home: &Path, provider: Provider, kind: &Kind) -> PathBuf {
    let dir = client_dir(home, provider);
    match kind {
        Kind::Mcp => if provider == Provider::Codex { dir.join("config.toml") } else { home.join(".claude.json") },
        Kind::Rules => dir.join(if provider == Provider::Codex { "AGENTS.md" } else { "CLAUDE.md" }),
        Kind::Skill => if provider == Provider::Codex { home.join(".agents/skills/lore-memory/SKILL.md") } else { dir.join("skills/lore-memory/SKILL.md") },
    }
}

fn mcp(settings: &Settings, provider: Provider) -> Result<Value> {
    let (command, rest) = settings.commands.qmd.split_first().ok_or_else(|| bad("Install qmd and check again before connecting."))?;
    let args: Vec<_> = rest.iter().cloned().chain(["mcp".into(), "--index".into(), settings.qmd_index.clone()]).collect();
    let mut entry = json!({"command": command, "args": args, "env": {
        "QMD_CONFIG_DIR": settings.qmd_config_dir().to_string_lossy().replace('\\', "/"),
        "XDG_CACHE_HOME": settings.qmd_cache_dir().to_string_lossy().replace('\\', "/")
    }});
    if provider == Provider::Claude { entry["type"] = json!("stdio"); }
    Ok(entry)
}
fn json_root(text: &str) -> Result<Value> {
    let value: Value = serde_json::from_str(if text.trim().is_empty() { "{}" } else { text.trim_start_matches('\u{feff}') })?;
    if !value.is_object() { return Err(bad("Client settings must contain a JSON object.")); }
    Ok(value)
}
fn toml(text: &str) -> Result<DocumentMut> {
    text.parse().map_err(|e| bad(format!("Could not read Codex config: {e}")))
}
fn toml_json(item: &Item) -> Result<Value> {
    // Convert only the qmd table, leaving comments/other TOML tables untouched.
    fn table(value: &dyn toml_edit::TableLike) -> Value {
        Value::Object(value.iter().filter_map(|(k, v)| convert(v).map(|v| (k.to_string(), v))).collect())
    }
    fn convert(item: &Item) -> Option<Value> {
        if let Some(t) = item.as_table_like() { return Some(table(t)); }
        let v = item.as_value()?;
        if let Some(s) = v.as_str() { Some(json!(s)) }
        else if let Some(b) = v.as_bool() { Some(json!(b)) }
        else if let Some(i) = v.as_integer() { Some(json!(i)) }
        else if let Some(f) = v.as_float() { Some(json!(f)) }
        else if let Some(d) = v.as_datetime() { Some(json!(d.to_string())) }
        else { v.as_array().map(|a| Value::Array(a.iter().filter_map(|v| convert(&Item::Value(v.clone()))).collect())) }
    }
    convert(item).ok_or_else(|| bad("The qmd MCP entry must be a table."))
}
fn rules_range(text: &str) -> Result<Option<std::ops::Range<usize>>> {
    let start = "<!-- lore-memory:start -->";
    let end = "<!-- lore-memory:end -->";
    match (text.matches(start).count(), text.matches(end).count()) {
        (0, 0) => Ok(None),
        (1, 1) => {
            let a = text.find(start).unwrap();
            let b = text.find(end).unwrap() + end.len();
            if b <= a { return Err(bad("Lore memory markers are out of order. Repair them before connecting.")); }
            Ok(Some(a..b))
        }
        _ => Err(bad("Lore memory markers are incomplete or duplicated. Repair them before connecting.")),
    }
}
fn fragment(text: &str, provider: Provider, kind: &Kind) -> Result<Option<String>> {
    match kind {
        Kind::Skill => Ok(if text.is_empty() { None } else { Some(text.to_string()) }),
        Kind::Rules => Ok(rules_range(text)?.map(|r| text[r].to_string())),
        Kind::Mcp => {
            let value = if provider == Provider::Codex {
                let doc = toml(text)?;
                if doc.get("mcp_servers").is_some_and(|v| v.as_table_like().is_none()) { return Err(bad("mcp_servers must be a TOML table.")); }
                doc.get("mcp_servers").and_then(|t| t.get("qmd")).map(toml_json).transpose()?
            } else {
                let root = json_root(text)?;
                if root.get("mcpServers").is_some_and(|v| !v.is_object()) { return Err(bad("mcpServers must be an object.")); }
                root.get("mcpServers").and_then(|t| t.get("qmd")).cloned()
            };
            value.map(|v| serde_json::to_string(&v).map_err(Into::into)).transpose()
        }
    }
}
fn matches(a: &str, b: &str, kind: &Kind) -> bool {
    if *kind == Kind::Mcp { serde_json::from_str::<Value>(a).ok() == serde_json::from_str::<Value>(b).ok() }
    else { a == b }
}
fn patch(text: &str, provider: Provider, kind: &Kind, value: Option<&str>) -> Result<String> {
    match kind {
        Kind::Skill => Ok(value.unwrap_or("").to_string()),
        Kind::Rules => match rules_range(text)? {
            Some(range) => { let mut text = text.to_string(); text.replace_range(range, value.unwrap_or("")); Ok(text) }
            None => Ok(match value { Some(value) if !text.is_empty() => format!("{}\n\n{value}\n", text.trim_end()), Some(value) => format!("{value}\n"), None => text.to_string() }),
        },
        Kind::Mcp if provider == Provider::Claude => {
            let mut root = json_root(text)?;
            if let Some(value) = value {
                if root.get("mcpServers").is_none() { root["mcpServers"] = json!({}); }
                root["mcpServers"].as_object_mut().ok_or_else(|| bad("mcpServers must be an object."))?.insert("qmd".into(), serde_json::from_str(value)?);
            } else if let Some(entries) = root.get_mut("mcpServers").and_then(Value::as_object_mut) { entries.remove("qmd"); }
            Ok(format!("{}\n", serde_json::to_string_pretty(&root)?))
        }
        Kind::Mcp => {
            let mut doc = toml(text)?;
            if let Some(value) = value {
                if doc.get("mcp_servers").is_none() { doc["mcp_servers"] = Item::Table(Table::new()); }
                let value: Value = serde_json::from_str(value)?;
                let mut entry = Table::new();
                entry["command"] = toml_edit::value(value["command"].as_str().unwrap());
                let mut args = toml_edit::Array::new();
                for arg in value["args"].as_array().unwrap() { args.push(arg.as_str().unwrap()); }
                entry["args"] = toml_edit::value(args);
                let mut env = Table::new();
                for (k, v) in value["env"].as_object().unwrap() { env[k] = toml_edit::value(v.as_str().unwrap()); }
                entry["env"] = Item::Table(env);
                if let Some(servers) = doc["mcp_servers"].as_inline_table_mut() {
                    servers.insert("qmd", toml_edit::Value::InlineTable(entry.into_inline_table()));
                } else {
                    doc["mcp_servers"]["qmd"] = Item::Table(entry);
                }
            } else if let Some(entries) = doc.get_mut("mcp_servers").and_then(Item::as_table_like_mut) { entries.remove("qmd"); }
            Ok(doc.to_string())
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Resource {
    pub label: String,
    pub path: PathBuf,
    pub state: String,
    pub detail: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub token: String,
    pub resources: Vec<Resource>,
    pub can_apply: bool,
}

pub fn plan(home: &Path, paths: &Paths, provider: Provider, settings: &Settings) -> Result<Plan> {
    let owned = receipts(paths, provider)?;
    let mut resources = Vec::new();
    let mut hash = serde_json::to_string(&owned)?;
    for kind in [Kind::Mcp, Kind::Rules, Kind::Skill] {
        let path = resource_path(home, provider, &kind);
        let text = read(&path)?.unwrap_or_default();
        let current = fragment(&text, provider, &kind)?;
        let wanted = match kind { Kind::Mcp => serde_json::to_string(&mcp(settings, provider)?)?, Kind::Rules => RULES.into(), Kind::Skill => SKILL.into() };
        let receipt = owned.iter().find(|e| e.kind == kind);
        let state = match current.as_deref() {
            None => "add",
            Some(c) if matches(c, &wanted, &kind) => "ready",
            Some(c) if receipt.is_some_and(|r| matches(c, &r.installed, &kind) || r.previous.as_deref().is_some_and(|p| matches(c, p, &kind))) => "update",
            Some(_) if kind == Kind::Rules && receipt.is_none() => "preserved",
            Some(_) => "conflict",
        };
        hash.push_str(&format!("{:?}:{}:{}:{}", kind, path.display(), text, wanted));
        let (label, detail) = match kind {
            Kind::Mcp => ("qmd MCP", "Local search server, using this PC's qmd command and index paths."),
            Kind::Rules => ("Memory guidance", "A short Lore section; other instructions and installer markers are kept."),
            Kind::Skill => ("Memory search skill", "On-demand instructions for finding project decisions and reading evidence."),
        };
        resources.push(Resource { label: label.into(), path, state: state.into(), detail: detail.into(), content: wanted });
    }
    Ok(Plan { token: digest(hash.as_bytes()), can_apply: resources.iter().all(|r| r.state != "conflict"), resources })
}

// The token binds the review to the exact files and local index configuration.
pub fn connect(home: &Path, paths: &Paths, provider: Provider, settings: &Settings, token: &str) -> Result<()> {
    let plan = plan(home, paths, provider, settings)?;
    if plan.token != token { return Err(bad("Configuration changed after preview. Review the changes again.")); }
    if !plan.can_apply { return Err(bad("An existing qmd entry or skill was changed outside Lore. Resolve the conflict before connecting.")); }
    let mut owned = receipts(paths, provider)?;
    for (kind, resource) in [Kind::Mcp, Kind::Rules, Kind::Skill].into_iter().zip(plan.resources) {
        if !matches!(resource.state.as_str(), "add" | "update") { continue; }
        let text = read(&resource.path)?.unwrap_or_default();
        let wanted = match kind { Kind::Mcp => serde_json::to_string(&mcp(settings, provider)?)?, Kind::Rules => RULES.into(), Kind::Skill => SKILL.into() };
        let updated = patch(&text, provider, &kind, Some(&wanted))?;
        // Save ownership first: a failed write remains repairable on the next run.
        let previous = owned.iter().find(|e| e.kind == kind).map(|e| e.installed.clone());
        owned.retain(|e| e.kind != kind);
        owned.push(Owned { kind: kind.clone(), installed: wanted, previous });
        save_receipts(paths, provider, &owned)?;
        atomic_write(&resource.path, updated.as_bytes())?;
        owned.iter_mut().find(|e| e.kind == kind).unwrap().previous = None;
        save_receipts(paths, provider, &owned)?;
    }
    Ok(())
}

pub fn disconnect(home: &Path, paths: &Paths, provider: Provider) -> Result<Vec<String>> {
    let mut owned = receipts(paths, provider)?;
    let mut kept = Vec::new();
    for entry in owned.clone() {
        let path = resource_path(home, provider, &entry.kind);
        let text = read(&path)?.unwrap_or_default();
        let current = fragment(&text, provider, &entry.kind)?;
        if current.as_deref().is_some_and(|c| !matches(c, &entry.installed, &entry.kind) && !entry.previous.as_deref().is_some_and(|p| matches(c, p, &entry.kind))) {
            kept.push(format!("Kept your edited {}", path.display()));
            continue;
        }
        if current.is_some() {
            let updated = patch(&text, provider, &entry.kind, None)?;
            if entry.kind == Kind::Skill { std::fs::remove_file(&path)?; }
            else { atomic_write(&path, updated.as_bytes())?; }
        }
        owned.retain(|r| r.kind != entry.kind);
        save_receipts(paths, provider, &owned)?;
    }
    Ok(kept)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings;

    fn fixture() -> (tempfile::TempDir, PathBuf, Paths, Settings) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("user");
        let paths = Paths { root: dir.path().join("app") };
        let settings = Settings { knowledge_root: dir.path().join("notes").to_string_lossy().into(), commands: settings::Commands { qmd: vec!["/runtime/node".into(), "/tools/qmd".into()], ..Default::default() }, ..Default::default() };
        (dir, home, paths, settings)
    }
    fn install(home: &Path, paths: &Paths, provider: Provider, settings: &Settings) {
        let preview = plan(home, paths, provider, settings).unwrap();
        connect(home, paths, provider, settings, &preview.token).unwrap();
    }

    #[test]
    fn installing_and_removing_keeps_other_tools_and_managed_markers() {
        for provider in [Provider::Claude, Provider::Codex] {
            let (_dir, home, paths, settings) = fixture();
            let config = resource_path(&home, provider, &Kind::Mcp);
            let original = if provider == Provider::Codex { "# personal config\nmodel = 'my-model'\n[mcp_servers.other]\ncommand = 'other'\n" } else { r#"{"other":"keep","mcpServers":{"other":{"command":"other"}}}"# };
            atomic_write(&config, original.as_bytes()).unwrap();
            let rules = resource_path(&home, provider, &Kind::Rules);
            let instructions = "<!-- codebase-memory-mcp:start -->\nKeep this installer block.\n<!-- codebase-memory-mcp:end -->\n\nDo not use expensive subagents.\n";
            atomic_write(&rules, instructions.as_bytes()).unwrap();
            install(&home, &paths, provider, &settings);
            install(&home, &paths, provider, &settings);
            assert!(plan(&home, &paths, provider, &settings).unwrap().resources.iter().all(|r| r.state == "ready"));
            assert_eq!(read(&rules).unwrap().unwrap().matches("<!-- lore-memory:start -->").count(), 1);
            assert!(disconnect(&home, &paths, provider).unwrap().is_empty());
            let after = read(&config).unwrap().unwrap();
            if provider == Provider::Codex {
                assert!(after.contains("# personal config"));
                assert_eq!(toml(&after).unwrap()["mcp_servers"]["other"]["command"].as_str(), Some("other"));
                assert_eq!(toml(&after).unwrap()["model"].as_str(), Some("my-model"));
            } else { assert_eq!(json_root(&after).unwrap()["mcpServers"]["other"]["command"], "other"); }
            assert_eq!(read(&rules).unwrap().unwrap().trim(), instructions.trim());
            assert!(!resource_path(&home, provider, &Kind::Skill).exists());
        }
    }

    #[test]
    fn a_preexisting_memory_section_is_reused_and_never_claimed() {
        let (_dir, home, paths, settings) = fixture();
        let rules = resource_path(&home, Provider::Codex, &Kind::Rules);
        let original = "Global policy\n<!-- lore-memory:start -->\nMy own short memory rule.\n<!-- lore-memory:end -->\n";
        atomic_write(&rules, original.as_bytes()).unwrap();
        install(&home, &paths, Provider::Codex, &settings);
        assert_eq!(read(&rules).unwrap().unwrap(), original);
        disconnect(&home, &paths, Provider::Codex).unwrap();
        assert_eq!(read(&rules).unwrap().unwrap(), original);
    }

    #[test]
    fn disconnect_preserves_user_edits_and_keeps_ownership_for_later_repair() {
        let (_dir, home, paths, settings) = fixture();
        install(&home, &paths, Provider::Claude, &settings);
        let skill = resource_path(&home, Provider::Claude, &Kind::Skill);
        atomic_write(&skill, b"User edited skill").unwrap();
        assert_eq!(disconnect(&home, &paths, Provider::Claude).unwrap().len(), 1);
        assert_eq!(read(&skill).unwrap().unwrap(), "User edited skill");
        assert!(!plan(&home, &paths, Provider::Claude, &settings).unwrap().can_apply);
    }

    #[test]
    fn preview_expires_if_any_client_file_or_local_index_path_changes() {
        let (_dir, home, paths, settings) = fixture();
        let token = plan(&home, &paths, Provider::Codex, &settings).unwrap().token;
        let path = resource_path(&home, Provider::Codex, &Kind::Mcp);
        atomic_write(&path, b"model = 'changed'\n").unwrap();
        assert!(connect(&home, &paths, Provider::Codex, &settings, &token).unwrap_err().message.contains("changed after preview"));
        assert_eq!(read(&path).unwrap().unwrap(), "model = 'changed'\n");
        let token = plan(&home, &paths, Provider::Codex, &settings).unwrap().token;
        let settings = Settings { knowledge_root: "/another/root".into(), ..settings };
        assert!(connect(&home, &paths, Provider::Codex, &settings, &token).is_err());
    }

    #[test]
    fn owned_mcp_paths_can_be_repaired_but_unowned_entries_are_not_overwritten() {
        let (_dir, home, paths, settings) = fixture();
        install(&home, &paths, Provider::Codex, &settings);
        let new_settings = Settings { knowledge_root: "/another/root".into(), ..settings.clone() };
        assert_eq!(plan(&home, &paths, Provider::Codex, &new_settings).unwrap().resources[0].state, "update");
        install(&home, &paths, Provider::Codex, &new_settings);
        let config = resource_path(&home, Provider::Codex, &Kind::Mcp);
        let root = toml(&read(&config).unwrap().unwrap()).unwrap();
        // MCP environment paths use forward slashes on every platform.
        assert_eq!(root["mcp_servers"]["qmd"]["env"]["QMD_CONFIG_DIR"].as_str().unwrap(), new_settings.qmd_config_dir().to_string_lossy().replace('\\', "/"));
        let custom = "[mcp_servers.qmd]\ncommand = 'my-qmd'\n";
        atomic_write(&config, custom.as_bytes()).unwrap();
        assert!(!plan(&home, &paths, Provider::Codex, &settings).unwrap().can_apply);
        let warnings = disconnect(&home, &paths, Provider::Codex).unwrap();
        assert_eq!(warnings.len(), 1);
        assert_eq!(read(&config).unwrap().unwrap(), custom);
    }

    #[test]
    fn malformed_settings_and_markers_fail_without_writes_or_panics() {
        let (_dir, home, paths, settings) = fixture();
        let config = resource_path(&home, Provider::Codex, &Kind::Mcp);
        for text in ["mcp_servers = 'invalid'", "[not valid", "mcp_servers = 5"] {
            atomic_write(&config, text.as_bytes()).unwrap();
            assert!(plan(&home, &paths, Provider::Codex, &settings).is_err());
            assert_eq!(read(&config).unwrap().unwrap(), text);
        }
        atomic_write(&config, b"").unwrap();
        let rules = resource_path(&home, Provider::Codex, &Kind::Rules);
        for text in ["<!-- lore-memory:start -->", "<!-- lore-memory:end --><!-- lore-memory:start -->", "<!-- lore-memory:start --><!-- lore-memory:start --><!-- lore-memory:end -->"] {
            atomic_write(&rules, text.as_bytes()).unwrap();
            assert!(plan(&home, &paths, Provider::Codex, &settings).is_err());
        }
    }

    #[test]
    fn setup_progress_resumes_without_touching_memory_or_other_settings() {
        let (_dir, _home, paths, settings) = fixture();
        let saved = Progress { step: 3, deferred: true, providers: vec![Provider::Claude], ..Default::default() };
        save_progress(&paths, &saved).unwrap();
        let loaded = progress(&paths).unwrap();
        assert_eq!(loaded.step, 3);
        assert!(loaded.deferred);
        assert_eq!(loaded.providers, vec![Provider::Claude]);
        assert!(!settings.knowledge_root().exists());
        assert!(!paths.settings().exists());
    }

    #[test]
    fn codex_inline_server_tables_remain_valid_after_connecting() {
        let (_dir, home, paths, settings) = fixture();
        let config = resource_path(&home, Provider::Codex, &Kind::Mcp);
        atomic_write(&config, b"mcp_servers = { other = { command = 'keep' } }\n").unwrap();
        install(&home, &paths, Provider::Codex, &settings);
        let parsed = toml(&read(&config).unwrap().unwrap()).unwrap();
        assert_eq!(parsed["mcp_servers"]["other"]["command"].as_str(), Some("keep"));
        assert_eq!(parsed["mcp_servers"]["qmd"]["command"].as_str(), Some("/runtime/node"));
    }
}
