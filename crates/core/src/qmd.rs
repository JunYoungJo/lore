use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use crate::error::{Error, Result};
use crate::notes::collection_path;
use crate::run;
use crate::settings::Settings;

pub struct Qmd {
    command: Vec<String>,
    config_dir: PathBuf,
    cache_dir: PathBuf,
    index: String,
    workdir: PathBuf,
    timeout: Duration,
}

pub fn collection_name(project: &str) -> String {
    format!("project-{project}")
}

fn parse_results(output: &str) -> Result<Value> {
    let mut offset = 0;
    for line in output.split_inclusive('\n') {
        if line.trim_start().starts_with('[') {
            return serde_json::from_str(output[offset..].trim()).map_err(|error| Error::transient(format!("qmd 검색 결과를 읽지 못했습니다: {error}")));
        }
        offset += line.len();
    }
    Err(Error::transient("qmd 검색 결과가 비어 있습니다."))
}

pub fn has_collection(config_text: &str, name: &str) -> bool {
    let wanted = format!("  {name}:");
    config_text.lines().any(|line| line.trim_end() == wanted)
}

impl Qmd {
    pub fn new(settings: &Settings, workdir: &Path) -> Self {
        Self {
            command: settings.commands.qmd.clone(),
            config_dir: settings.qmd_config_dir(),
            cache_dir: settings.qmd_cache_dir(),
            index: settings.qmd_index.clone(),
            workdir: workdir.to_path_buf(),
            timeout: Duration::from_secs(settings.qmd_timeout_secs),
        }
    }

    fn run(&self, args: &[&str], timeout: Duration) -> Result<String> {
        if self.command.is_empty() {
            return Err(Error::transient("qmd 실행 경로를 찾지 못했습니다. 설정에서 qmd 명령을 지정하십시오."));
        }
        let mut full = self.command.clone();
        full.extend(["--index".to_string(), self.index.clone()]);
        full.extend(args.iter().map(|arg| arg.to_string()));
        let env = [
            ("QMD_CONFIG_DIR", self.config_dir.to_string_lossy().replace('\\', "/")),
            ("XDG_CACHE_HOME", self.cache_dir.to_string_lossy().replace('\\', "/")),
        ];
        Ok(run::run(&full, &self.workdir, &env, None, timeout)?.stdout)
    }

    pub fn refresh(&self) -> Result<()> {
        self.run(&["update"], self.timeout)?;
        self.run(&["embed"], self.timeout)?;
        Ok(())
    }

    pub fn rank(&self, query: &str, collections: &[String], limit: usize) -> Result<Vec<String>> {
        let limit = limit.to_string();
        let mut args = vec!["vsearch", query, "--format", "json", "-n", limit.as_str()];
        for collection in collections {
            args.push("-c");
            args.push(collection);
        }
        let output = self.run(&args, Duration::from_secs(180))?;
        let results = parse_results(&output)?;
        Ok(results
            .as_array()
            .map(|items| items.iter().filter_map(|item| item.get("file").and_then(Value::as_str)).filter_map(collection_path).collect())
            .unwrap_or_default())
    }

    pub fn configured_collections(&self) -> Result<String> {
        let path = self.config_dir.join(format!("{}.yml", self.index));
        match std::fs::read_to_string(&path) {
            Ok(text) => Ok(text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn ensure_collection(&self, project: &str, folder: &Path) -> Result<()> {
        let name = collection_name(project);
        if has_collection(&self.configured_collections()?, &name) {
            return Ok(());
        }
        let folder = folder.to_string_lossy().replace('\\', "/");
        self.run(&["collection", "add", &folder, "--name", &name, "--mask", "**/*.md"], self.timeout)?;
        let description = format!("{project} 프로젝트의 결정·문제·해결 및 변경 이력. 사용자 지시와 당시 에이전트 보고를 구분.");
        self.run(&["context", "add", &format!("qmd://{name}"), &description], Duration::from_secs(120))?;
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

    #[test]
    fn search_results_are_read_after_progress_lines_that_echo_the_query() {
        let output = "├─ [경고] 이유\n└─ hyde: x\nSearching 2 vector queries...\n[\n  {\"file\": \"qmd://project-demo/a.md\"}\n]\n";
        let results = parse_results(output).unwrap();
        assert_eq!(results[0]["file"], "qmd://project-demo/a.md");
        assert!(parse_results("no results\n").is_err());
    }
}
