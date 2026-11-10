use std::path::{Path, PathBuf};

pub const CLI_FILENAME: &str = if cfg!(windows) { "lore-cli.exe" } else { "lore-cli" };

pub fn home() -> PathBuf {
    let value = if cfg!(windows) {
        std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))
    } else {
        std::env::var_os("HOME")
    };
    value.map(PathBuf::from).unwrap_or_default()
}

pub fn data_root() -> PathBuf {
    if let Some(root) = std::env::var_os("LORE_HOME").filter(|value| !value.is_empty()) {
        return PathBuf::from(root);
    }
    let home = home();
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| home.join("AppData/Local"))
    } else if cfg!(target_os = "macos") {
        home.join("Library/Application Support")
    } else {
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|path| path.is_absolute()).unwrap_or_else(|| home.join(".local/share"))
    };
    base.join(crate::settings::APP_IDENTIFIER)
}

pub fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else { return false };
    if !metadata.is_file() { return false; }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    { true }
}

// Finder and login launchers don't inherit a terminal's PATH. Search known user
// installation directories without evaluating shell startup files.
pub fn search_paths() -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = std::env::var_os("PATH").map(|value| std::env::split_paths(&value).collect()).unwrap_or_default();
    #[cfg(unix)]
    {
        let home = home();
        folders.extend([home.join(".local/bin"), home.join(".bun/bin"), home.join(".cargo/bin"), home.join(".npm-global/bin"), PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/usr/local/bin")]);
        let nvm = std::env::var_os("NVM_DIR").map(PathBuf::from).unwrap_or_else(|| home.join(".nvm"));
        if let Ok(entries) = std::fs::read_dir(nvm.join("versions/node")) {
            let mut versions: Vec<_> = entries.flatten().filter(|entry| entry.path().join("bin").is_dir()).collect();
            versions.sort_by_key(|entry| version_key(&entry.file_name().to_string_lossy()));
            folders.extend(versions.into_iter().rev().map(|entry| entry.path().join("bin")));
        }
        folders.extend([PathBuf::from("/usr/bin"), PathBuf::from("/bin")]);
    }
    let mut seen = std::collections::BTreeSet::new();
    folders.retain(|folder| !folder.as_os_str().is_empty() && seen.insert(folder.clone()));
    folders
}

#[cfg(unix)]
fn version_key(name: &str) -> Vec<u64> {
    name.trim_start_matches('v').split('.').map(|part| part.parse().unwrap_or(0)).collect()
}

pub fn which_in(name: &str, folders: &[PathBuf]) -> Option<PathBuf> {
    let extensions: &[&str] = if cfg!(windows) { &[".exe", ".cmd", ".ps1", ".bat", ""] } else { &[""] };
    folders.iter().flat_map(|folder| extensions.iter().map(move |extension| folder.join(format!("{name}{extension}")))).find(|path| is_executable(path))
}

pub fn which(name: &str) -> Option<PathBuf> {
    which_in(name, &search_paths())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_filename_matches_the_host() {
        assert_eq!(Path::new(CLI_FILENAME).extension().is_some(), cfg!(windows));
    }

    #[test]
    fn executable_lookup_respects_path_order_and_ignores_directories() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        let name = if cfg!(windows) { "test-tool.exe" } else { "test-tool" };
        std::fs::create_dir_all(first.join(name)).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        let executable = second.join(name);
        std::fs::write(&executable, "fixture").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert!(which_in("test-tool", &[first.clone(), second.clone()]).is_none());
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert_eq!(which_in("test-tool", &[first, second]), Some(executable));
    }
}
