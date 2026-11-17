use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Serialize;

use crate::error::{Error, Result};
use crate::settings::atomic_write;

/// A note a model is told exists without being shown its text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogEntry {
    pub path: String,
    pub title: String,
}

fn read_folder(root: &Path, relative: &str, notes: &mut BTreeMap<String, String>) -> Result<()> {
    let folder = root.join(relative);
    let metadata = match std::fs::symlink_metadata(&folder) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() {
        return Err(Error::permanent(format!("Symlinks are not allowed in memory folders: {relative}")));
    }
    for entry in std::fs::read_dir(&folder)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        let kind = entry.file_type()?;
        if !kind.is_file() || !name.ends_with(".md") {
            if kind.is_symlink() && name.ends_with(".md") {
                return Err(Error::permanent(format!("Symlinks are not allowed for memory notes: {relative}/{name}")));
            }
            continue;
        }
        notes.insert(format!("{relative}/{name}"), std::fs::read_to_string(entry.path())?);
    }
    Ok(())
}

/// The notes of one folder, such as "common" or "projects/demo".
pub fn load_folder(knowledge_root: &Path, relative: &str) -> Result<BTreeMap<String, String>> {
    let mut notes = BTreeMap::new();
    read_folder(knowledge_root, relative, &mut notes)?;
    Ok(notes)
}

pub fn load_scope(knowledge_root: &Path, project: &str) -> Result<BTreeMap<String, String>> {
    let mut notes = BTreeMap::new();
    read_folder(knowledge_root, "common", &mut notes)?;
    read_folder(knowledge_root, &format!("projects/{project}"), &mut notes)?;
    Ok(notes)
}

pub fn load_all(knowledge_root: &Path) -> Result<BTreeMap<String, String>> {
    let mut notes = BTreeMap::new();
    read_folder(knowledge_root, "common", &mut notes)?;
    let projects = knowledge_root.join("projects");
    match std::fs::read_dir(&projects) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    read_folder(knowledge_root, &format!("projects/{}", entry.file_name().to_string_lossy()), &mut notes)?;
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(notes)
}

pub fn title_of(content: &str) -> String {
    content.lines().find_map(|line| line.strip_prefix("# ")).map(|title| title.trim().to_string()).unwrap_or_default()
}

pub fn collection_path(file: &str) -> Option<String> {
    let rest = file.strip_prefix("qmd://")?;
    let (collection, path) = rest.split_once('/')?;
    if collection == "common" {
        Some(format!("common/{path}"))
    } else {
        collection.strip_prefix("project-").map(|project| format!("projects/{project}/{path}"))
    }
}

/// Write the changed notes, then delete the removed ones. A note that took in another is on
/// disk before the other goes, so an interruption leaves a duplicate rather than a loss.
pub fn write_notes(knowledge_root: &Path, rendered: &BTreeMap<String, String>, removed: &BTreeSet<String>) -> Result<()> {
    for (note, content) in rendered {
        let target = knowledge_root.join(note);
        if let Ok(metadata) = std::fs::symlink_metadata(&target) {
            if metadata.file_type().is_symlink() {
                return Err(Error::permanent(format!("Symlinks are not allowed for memory notes: {note}")));
            }
        }
        atomic_write(&target, content.as_bytes())?;
    }
    for note in removed {
        match std::fs::remove_file(knowledge_root.join(note)) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qmd_file_uris_map_to_knowledge_paths() {
        assert_eq!(collection_path("qmd://common/review.md").as_deref(), Some("common/review.md"));
        assert_eq!(collection_path("qmd://project-demo-app/flow.md").as_deref(), Some("projects/demo-app/flow.md"));
        assert_eq!(collection_path("qmd://other/x.md"), None);
        assert_eq!(collection_path("projects/x.md"), None);
    }

    #[test]
    fn scope_reads_common_and_the_project_only() {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in [("common/a.md", "# A"), ("projects/p/b.md", "# B"), ("projects/q/c.md", "# C"), ("projects/p/skip.txt", "x")] {
            let file = dir.path().join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        }
        let scope = load_scope(dir.path(), "p").unwrap();
        assert_eq!(scope.keys().cloned().collect::<Vec<_>>(), vec!["common/a.md", "projects/p/b.md"]);
        assert!(load_scope(dir.path(), "new").unwrap().contains_key("common/a.md"));
        assert_eq!(load_all(dir.path()).unwrap().keys().cloned().collect::<Vec<_>>(), vec!["common/a.md", "projects/p/b.md", "projects/q/c.md"]);
    }
}
