use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use crate::error::{Error, Result};
use crate::notes::collection_path;
use crate::run::{self, Limit};
use crate::settings::Settings;

const SEARCH_LIMIT: Duration = Duration::from_secs(180);
const CONTEXT_LIMIT: Duration = Duration::from_secs(120);

pub const QWEN_EMBED: &str = "hf:Qwen/Qwen3-Embedding-0.6B-GGUF/Qwen3-Embedding-0.6B-Q8_0.gguf";
pub const GEMMA_EMBED: &str = "hf:ggml-org/embeddinggemma-300M-GGUF/embeddinggemma-300M-Q8_0.gguf";

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct EmbedModel {
    pub uri: &'static str,
    pub name: &'static str,
    pub size: &'static str,
    pub note: &'static str,
}

// qmd 2.8.3 formats queries for only these two model families.
pub const EMBED_MODELS: [EmbedModel; 2] = [
    EmbedModel { uri: QWEN_EMBED, name: "Qwen3-Embedding-0.6B", size: "about 640 MB", note: "Multilingual, including Korean." },
    EmbedModel { uri: GEMMA_EMBED, name: "embeddinggemma-300M", size: "under 350 MB", note: "qmd’s default. Smaller, but weaker on Korean, Chinese and Japanese." },
];

fn section_lines<'a>(config_text: &'a str, section: &str) -> impl Iterator<Item = &'a str> {
    let header = format!("{section}:");
    let mut inside = false;
    config_text.lines().filter(move |line| {
        if !line.starts_with(' ') && !line.trim().is_empty() {
            inside = line.trim_end() == header;
            return false;
        }
        inside
    })
}

fn unquoted(value: &str) -> &str {
    let value = value.trim();
    value.strip_prefix('"').and_then(|v| v.strip_suffix('"')).or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\''))).unwrap_or(value)
}

pub fn embed_model(config_text: &str) -> Option<String> {
    section_lines(config_text, "models").find_map(|line| line.strip_prefix("  embed:")).map(unquoted).filter(|value| !value.is_empty()).map(str::to_string)
}

pub fn collection_names(config_text: &str) -> Vec<String> {
    section_lines(config_text, "collections")
        .filter(|line| line.starts_with("  ") && !line.starts_with("   "))
        .filter_map(|line| line.trim().strip_suffix(':'))
        .map(|name| unquoted(name).to_string())
        .collect()
}

pub fn with_embed_model(config_text: &str, model: &str) -> String {
    let entry = format!("  embed: \"{model}\"");
    let mut lines: Vec<String> = config_text.lines().map(str::to_string).collect();
    let header = lines.iter().position(|line| line.trim_end() == "models:");
    match header {
        Some(start) => {
            let end = lines.iter().skip(start + 1).position(|line| !line.starts_with(' ') && !line.trim().is_empty()).map_or(lines.len(), |offset| start + 1 + offset);
            match (start + 1..end).find(|&index| lines[index].starts_with("  embed:")) {
                Some(index) => lines[index] = entry,
                None => lines.insert(start + 1, entry),
            }
        }
        None => lines.extend(["models:".to_string(), entry]),
    }
    format!("{}\n", lines.join("\n"))
}

pub struct Qmd {
    command: Vec<String>,
    config_dir: PathBuf,
    cache_dir: PathBuf,
    index: String,
    workdir: PathBuf,
    limit: Limit,
}

pub fn collection_name(project: &str) -> String {
    format!("project-{project}")
}

fn parse_results(output: &str) -> Result<Value> {
    let mut offset = 0;
    for line in output.split_inclusive('\n') {
        if line.trim_start().starts_with('[') {
            return serde_json::from_str(output[offset..].trim()).map_err(|error| Error::transient(format!("Could not read qmd search results: {error}")));
        }
        offset += line.len();
    }
    Err(Error::transient("qmd search returned no results."))
}

pub fn has_collection(config_text: &str, name: &str) -> bool {
    let wanted = format!("  {name}:");
    config_text.lines().any(|line| line.trim_end() == wanted)
}

impl Qmd {
    pub fn new(settings: &Settings, workdir: &Path, limit: Limit) -> Self {
        Self {
            command: settings.commands.qmd.clone(),
            config_dir: settings.qmd_config_dir(),
            cache_dir: settings.qmd_cache_dir(),
            index: settings.qmd_index.clone(),
            workdir: workdir.to_path_buf(),
            limit,
        }
    }

    fn run(&self, args: &[&str], limit: &dyn Fn() -> Duration) -> Result<String> {
        if self.command.is_empty() {
            return Err(Error::transient("qmd was not found. Set the qmd command in settings."));
        }
        let mut full = self.command.clone();
        full.extend(["--index".to_string(), self.index.clone()]);
        full.extend(args.iter().map(|arg| arg.to_string()));
        let env = [
            ("QMD_CONFIG_DIR", self.config_dir.to_string_lossy().replace('\\', "/")),
            ("XDG_CACHE_HOME", self.cache_dir.to_string_lossy().replace('\\', "/")),
        ];
        Ok(run::run(&full, &self.workdir, &env, None, limit)?.stdout)
    }

    pub fn available(&self) -> bool {
        crate::diagnostics::command_available(&self.command)
    }

    // Each PC has its own config and index. Reconstruct collections from the
    // shared Markdown tree rather than copying another PC's absolute paths.
    pub fn ensure_index(&self, root: &Path) -> Result<()> {
        std::fs::create_dir_all(root.join("common"))?;
        self.ensure_named_collection("common", &root.join("common"), "여러 프로젝트에 공통으로 적용되는 결정과 경험.")?;
        let projects = root.join("projects");
        let entries = match std::fs::read_dir(projects) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        let mut folders = Vec::new();
        for entry in entries {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if crate::project::is_valid_name(&name) { folders.push((name, entry.path())); }
            }
        }
        folders.sort();
        for (name, folder) in folders { self.ensure_collection(&name, &folder)?; }
        Ok(())
    }

    pub fn refresh(&self) -> Result<()> {
        self.run(&["update"], self.limit.as_ref())?;
        self.run(&["embed"], self.limit.as_ref())?;
        Ok(())
    }

    // Vectors from different models cannot be mixed, so a model change re-embeds everything.
    pub fn rebuild_vectors(&self) -> Result<()> {
        self.run(&["update"], self.limit.as_ref())?;
        self.run(&["embed", "-f"], self.limit.as_ref())?;
        Ok(())
    }

    pub fn config_path(&self) -> PathBuf {
        self.config_dir.join(format!("{}.yml", self.index))
    }

    pub fn rank(&self, query: &str, collections: &[String], limit: usize) -> Result<Vec<String>> {
        let limit = limit.to_string();
        let mut args = vec!["vsearch", query, "--format", "json", "-n", limit.as_str()];
        for collection in collections {
            args.push("-c");
            args.push(collection);
        }
        let output = self.run(&args, &|| SEARCH_LIMIT)?;
        let results = parse_results(&output)?;
        Ok(results
            .as_array()
            .map(|items| items.iter().filter_map(|item| item.get("file").and_then(Value::as_str)).filter_map(collection_path).collect())
            .unwrap_or_default())
    }

    pub fn configured_collections(&self) -> Result<String> {
        match std::fs::read_to_string(self.config_path()) {
            Ok(text) => Ok(text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn ensure_collection(&self, project: &str, folder: &Path) -> Result<()> {
        let name = collection_name(project);
        let description = format!("{project} 프로젝트의 결정·문제·해결 및 변경 이력. 사용자 지시와 당시 에이전트 보고를 구분.");
        self.ensure_named_collection(&name, folder, &description)
    }

    fn ensure_named_collection(&self, name: &str, folder: &Path, description: &str) -> Result<()> {
        if has_collection(&self.configured_collections()?, name) {
            return Ok(());
        }
        let folder = folder.to_string_lossy().replace('\\', "/");
        self.run(&["collection", "add", &folder, "--name", name, "--mask", "**/*.md"], self.limit.as_ref())?;
        self.run(&["context", "add", &format!("qmd://{name}"), description], &|| CONTEXT_LIMIT)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_lookup_matches_only_top_level_collection_keys() {
        let config = "collections:\n  common:\n    path: D:/k/common\n  project-demo-app:\n    path: D:/k/projects/demo-app\nmodels:\n  embed: x\n";
        assert!(has_collection(config, "project-demo-app"));
        assert!(has_collection(config, "common"));
        assert!(!has_collection(config, "project-tc"));
        assert!(!has_collection(config, "path"));
        assert_eq!(collection_name("demoengine"), "project-demoengine");
    }

    const CONFIG: &str = "collections:\n  common:\n    path: D:\\k\\common\n    pattern: \"**/*.md\"\n    context:\n      \"\": 공통\n  project-demo:\n    path: D:\\k\\projects\\demo\nmodels:\n  embed: hf:Qwen/Qwen3-Embedding-0.6B-GGUF/Qwen3-Embedding-0.6B-Q8_0.gguf\n  rerank: hf:r\n";

    #[test]
    fn the_embedding_model_and_collections_are_read_from_the_index_config() {
        assert_eq!(embed_model(CONFIG).as_deref(), Some(QWEN_EMBED));
        assert_eq!(embed_model("models:\n  embed: \"hf:a/b/c.gguf\"\n").as_deref(), Some("hf:a/b/c.gguf"));
        assert_eq!(embed_model("collections: {}\n"), None);
        assert_eq!(embed_model("collections:\n  embed:\n    path: x\n"), None);
        assert_eq!(collection_names(CONFIG), ["common", "project-demo"]);
        assert!(collection_names("collections: {}\nmodels:\n  embed: x\n").is_empty());
    }

    #[test]
    fn changing_the_embedding_model_keeps_every_other_line() {
        let changed = with_embed_model(CONFIG, GEMMA_EMBED);
        assert_eq!(embed_model(&changed).as_deref(), Some(GEMMA_EMBED));
        assert_eq!(changed.replace(&format!("  embed: \"{GEMMA_EMBED}\""), &format!("  embed: {QWEN_EMBED}")), CONFIG);

        let without_embed = with_embed_model("collections: {}\nmodels:\n  rerank: hf:r\n", GEMMA_EMBED);
        assert_eq!(without_embed, format!("collections: {{}}\nmodels:\n  embed: \"{GEMMA_EMBED}\"\n  rerank: hf:r\n"));

        let without_models = with_embed_model("collections:\n  common:\n    path: x\n", QWEN_EMBED);
        assert_eq!(without_models, format!("collections:\n  common:\n    path: x\nmodels:\n  embed: \"{QWEN_EMBED}\"\n"));
        assert_eq!(collection_names(&without_models), ["common"]);
    }

    #[test]
    fn search_results_are_read_after_progress_lines_that_echo_the_query() {
        let output = "├─ [경고] 이유\n└─ hyde: x\nSearching 2 vector queries...\n[\n  {\"file\": \"qmd://project-demo/a.md\"}\n]\n";
        let results = parse_results(output).unwrap();
        assert_eq!(results[0]["file"], "qmd://project-demo/a.md");
        assert!(parse_results("no results\n").is_err());
    }

    #[test]
    #[cfg(unix)]
    fn a_fresh_pc_registers_common_and_every_project_in_its_local_index() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("shared knowledge");
        std::fs::create_dir_all(root.join("projects/demo")).unwrap();
        let script = dir.path().join("qmd-fixture.sh");
        let log = dir.path().join("calls.txt");
        std::fs::write(&script, r#"log=$1
shift
printf '%s|%s|%s\n' "$QMD_CONFIG_DIR" "$XDG_CACHE_HOME" "$*" >> "$log"
shift 2
if [ "$1:$2" = 'collection:add' ]; then
  mkdir -p "$QMD_CONFIG_DIR"
  printf '  %s:\n    path: %s\n' "$5" "$3" >> "$QMD_CONFIG_DIR/index.yml"
fi
"#).unwrap();
        let qmd = Qmd {
            command: vec!["/bin/sh".into(), script.to_string_lossy().to_string(), log.to_string_lossy().to_string()],
            config_dir: dir.path().join("local/config"), cache_dir: dir.path().join("local/cache"),
            workdir: dir.path().join("worker"), index: "index".into(), limit: run::fixed(5),
        };
        qmd.ensure_index(&root).unwrap();
        qmd.ensure_index(&root).unwrap();
        qmd.refresh().unwrap();
        let calls = std::fs::read_to_string(log).unwrap();
        assert_eq!(calls.matches("collection add").count(), 2);
        assert!(calls.contains("--name common") && calls.contains("--name project-demo"));
        assert!(calls.contains("--index index update") && calls.contains("--index index embed"));
        assert!(calls.lines().all(|line| line.starts_with(qmd.config_dir.to_str().unwrap())));
        assert!(!root.join(".qmd").exists());
    }
}
