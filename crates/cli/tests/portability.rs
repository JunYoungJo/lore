use std::process::Command;

#[test]
fn doctor_is_read_only_and_the_cli_uses_the_same_overridden_data_directory() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("isolated-app-data");
    let output = Command::new(env!("CARGO_BIN_EXE_qmd-memory-cli"))
        .arg("doctor").env("QMD_MEMORY_HOME", &app).output().unwrap();
    assert!(output.status.success(), "{:?}", output);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["dataDir"], app.to_string_lossy().as_ref());
    assert!(!app.exists(), "doctor must not initialize settings or a queue");
    let output = Command::new(env!("CARGO_BIN_EXE_qmd-memory-cli"))
        .arg("status").env("QMD_MEMORY_HOME", &app).output().unwrap();
    assert!(output.status.success());
    assert!(app.join("settings.json").is_file() && app.join("state.sqlite").is_file());
}

#[test]
fn a_renamed_checkout_uses_its_versioned_project_identity() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("different-local-folder");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::write(repo.join(".qmd-memory.json"), r#"{"project":"my-project"}"#).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_qmd-memory-cli"))
        .arg("project").current_dir(&repo).env("QMD_MEMORY_HOME", dir.path().join("app")).output().unwrap();
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["project"], "my-project");
    assert!(!dir.path().join("app").exists());
}
