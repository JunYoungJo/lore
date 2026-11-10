use std::path::{Component, Path, PathBuf};
use crate::error::{Error, Result};

pub const ID_FILE: &str = ".qmd-memory.json";

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    project: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repository {
    pub root: PathBuf,
    pub name: String,
}

pub fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_lowercase() || first.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

pub fn project_name(folder: &str) -> Option<String> {
    let mapped: String = folder
        .chars()
        .map(|c| {
            let lower = c.to_ascii_lowercase();
            if lower.is_ascii_lowercase() || lower.is_ascii_digit() || lower == '_' || lower == '-' {
                lower
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = mapped.trim_matches(|c: char| c == '-' || c == '_').to_string();
    is_valid_name(&trimmed).then_some(trimmed)
}

fn normalize(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                result.pop();
            }
            Component::CurDir => {}
            other => result.push(other.as_os_str()),
        }
    }
    result
}

fn repository_root(worktree: &Path) -> Option<PathBuf> {
    let marker = worktree.join(".git");
    if marker.is_dir() {
        return Some(worktree.to_path_buf());
    }
    if !marker.is_file() {
        return None;
    }
    let content = std::fs::read_to_string(&marker).ok()?;
    let target = content.lines().find_map(|line| line.trim().strip_prefix("gitdir:")).map(str::trim)?;
    let gitdir = if Path::new(target).is_absolute() { PathBuf::from(target) } else { worktree.join(target) };
    let gitdir = normalize(&gitdir);
    let Ok(common) = std::fs::read_to_string(gitdir.join("commondir")) else {
        return Some(worktree.to_path_buf());
    };
    let common = common.trim();
    let common = if Path::new(common).is_absolute() { PathBuf::from(common) } else { gitdir.join(common) };
    let common = normalize(&common);
    match common.file_name().and_then(|name| name.to_str()) {
        Some(".git") => common.parent().map(Path::to_path_buf),
        _ => Some(worktree.to_path_buf()),
    }
}

pub fn resolve(cwd: &Path) -> Option<Repository> {
    resolve_checked(cwd).ok().flatten()
}

pub fn resolve_checked(cwd: &Path) -> Result<Option<Repository>> {
    if cwd.as_os_str().is_empty() {
        return Ok(None);
    }
    for candidate in cwd.ancestors() {
        if let Some(root) = repository_root(candidate) {
            let identity_path = root.join(ID_FILE);
            let name = match std::fs::read_to_string(&identity_path) {
                Ok(text) => {
                    let identity: Identity = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|error| Error::permanent(format!("{}: could not read the project ID: {error}", identity_path.display())))?;
                    if !is_valid_name(&identity.project) || identity.project.len() > 100 {
                        return Err(Error::permanent(format!("{}: project must be at most 100 lowercase letters, digits, hyphens, or underscores, starting with a letter or digit.", identity_path.display())));
                    }
                    identity.project
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let Some(name) = root.file_name().and_then(|name| name.to_str()).and_then(project_name) else { return Ok(None) };
                    name
                }
                Err(error) => return Err(Error::permanent(format!("{}: {error}", identity_path.display()))),
            };
            return Ok(Some(Repository { root, name }));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn folder_names_become_lowercase_project_names() {
        assert_eq!(project_name("demoengine").as_deref(), Some("demoengine"));
        assert_eq!(project_name("demo-app").as_deref(), Some("demo-app"));
        assert_eq!(project_name("my project").as_deref(), Some("my-project"));
        assert_eq!(project_name("_hidden").as_deref(), Some("hidden"));
        assert_eq!(project_name("한글"), None);
    }

    #[test]
    fn explicit_identity_survives_a_different_clone_folder_and_invalid_ids_fail() {
        let dir = tempfile::tempdir().unwrap();
        for folder in ["demoengine", "renamed-clone"] {
            let repo = dir.path().join(folder);
            fs::create_dir_all(repo.join(".git")).unwrap();
            fs::write(repo.join(ID_FILE), r#"{"project":"demoengine"}"#).unwrap();
            assert_eq!(resolve_checked(&repo).unwrap().unwrap().name, "demoengine");
        }
        let repo = dir.path().join("demoengine");
        fs::write(repo.join(ID_FILE), r#"{"project":"../other"}"#).unwrap();
        assert_eq!(resolve_checked(&repo).unwrap_err().failure, crate::error::Failure::Permanent);
        fs::write(repo.join(ID_FILE), "not json").unwrap();
        assert!(resolve_checked(&repo).is_err());
    }

    #[test]
    fn subfolder_of_a_repository_resolves_to_the_repository() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("sample-app");
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::create_dir_all(repo.join("src").join("core")).unwrap();
        let found = resolve(&repo.join("src").join("core")).unwrap();
        assert_eq!(found, Repository { root: repo, name: "sample-app".into() });
    }

    #[test]
    fn worktree_resolves_to_its_main_repository() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("src").join("demoengine");
        let gitdir = main.join(".git").join("worktrees").join("refactor");
        fs::create_dir_all(&gitdir).unwrap();
        fs::write(gitdir.join("commondir"), "../..\n").unwrap();
        let worktree = dir.path().join("tmp").join("worktrees").join("demoengine-refactor");
        fs::create_dir_all(&worktree).unwrap();
        fs::write(worktree.join(".git"), format!("gitdir: {}\n", gitdir.display())).unwrap();
        let found = resolve(&worktree).unwrap();
        assert_eq!(found.name, "demoengine");
        assert_eq!(found.root, normalize(&main));
        fs::write(main.join(ID_FILE), r#"{"project":"shared-project"}"#).unwrap();
        assert_eq!(resolve_checked(&worktree).unwrap().unwrap().name, "shared-project");
    }

    #[test]
    fn relative_gitdir_is_resolved_from_the_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("ctf-lab");
        let gitdir = main.join(".git").join("worktrees").join("w1");
        fs::create_dir_all(&gitdir).unwrap();
        fs::write(gitdir.join("commondir"), "../..").unwrap();
        let worktree = dir.path().join("w1");
        fs::create_dir_all(&worktree).unwrap();
        fs::write(worktree.join(".git"), "gitdir: ../ctf-lab/.git/worktrees/w1").unwrap();
        assert_eq!(resolve(&worktree).unwrap().name, "ctf-lab");
    }

    #[test]
    fn submodule_resolves_to_its_own_folder() {
        let dir = tempfile::tempdir().unwrap();
        let outer = dir.path().join("host");
        let modules = outer.join(".git").join("modules").join("common");
        fs::create_dir_all(&modules).unwrap();
        let inner = outer.join("common");
        fs::create_dir_all(&inner).unwrap();
        fs::write(inner.join(".git"), "gitdir: ../.git/modules/common").unwrap();
        assert_eq!(resolve(&inner).unwrap().name, "common");
    }

    #[test]
    fn folder_outside_any_repository_has_no_project() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("notes");
        fs::create_dir_all(&plain).unwrap();
        assert!(resolve(&plain).map(|repo| !repo.root.starts_with(dir.path())).unwrap_or(true));
        assert_eq!(resolve(Path::new("")), None);
    }
}
