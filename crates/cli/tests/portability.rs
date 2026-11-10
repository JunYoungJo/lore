use std::process::Command;

#[test]
fn help_and_version_identify_lore_without_creating_app_state() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("app");
    for flag in ["--help", "-h", "--version", "-V"] {
        let output = Command::new(env!("CARGO_BIN_EXE_lore-cli"))
            .arg(flag).env("LORE_HOME", &home).output().unwrap();
        assert!(output.status.success(), "{flag}: {:?}", output);
        assert!(String::from_utf8(output.stdout).unwrap().contains("lore-cli"));
        assert!(!home.exists(), "{flag} must not initialize application state");
    }
}

#[test]
fn bundle_configuration_and_cli_agree_on_identity_and_default_storage() {
    let config: serde_json::Value = serde_json::from_str(include_str!("../../../src-tauri/tauri.conf.json")).unwrap();
    assert_eq!(config["identifier"], lore_core::settings::APP_IDENTIFIER);
    let sidecar = config["bundle"]["externalBin"][0].as_str().unwrap();
    let stem = std::path::Path::new(sidecar).file_name().unwrap().to_str().unwrap();
    let expected_cli = if cfg!(windows) { format!("{stem}.exe") } else { stem.to_string() };
    assert_eq!(expected_cli, lore_core::platform::CLI_FILENAME);

    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_lore-cli"))
        .arg("doctor")
        .env_remove("LORE_HOME")
        .env("HOME", dir.path())
        .env("USERPROFILE", dir.path())
        .env("LOCALAPPDATA", dir.path().join("local"))
        .env("XDG_DATA_HOME", dir.path().join("data"))
        .output().unwrap();
    assert!(output.status.success(), "{:?}", output);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let data = std::path::Path::new(report["dataDir"].as_str().unwrap());
    assert_eq!(data.file_name().unwrap().to_str().unwrap(), config["identifier"]);
    let memory = std::path::Path::new(report["knowledgeRoot"].as_str().unwrap());
    assert_eq!(memory, dir.path().join("Documents/lore-memory"));
    assert!(!data.exists(), "diagnostics must not initialize application state");
}

#[test]
fn doctor_is_read_only_and_the_cli_uses_the_same_overridden_data_directory() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("isolated-app-data");
    let output = Command::new(env!("CARGO_BIN_EXE_lore-cli"))
        .arg("doctor").env("LORE_HOME", &app).output().unwrap();
    assert!(output.status.success(), "{:?}", output);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["dataDir"], app.to_string_lossy().as_ref());
    assert!(!app.exists(), "doctor must not initialize settings or a queue");
    let output = Command::new(env!("CARGO_BIN_EXE_lore-cli"))
        .arg("status").env("LORE_HOME", &app).output().unwrap();
    assert!(output.status.success());
    assert!(app.join("settings.json").is_file() && app.join("state.sqlite").is_file());
}

#[test]
fn a_renamed_checkout_uses_its_versioned_project_identity() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("different-local-folder");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::write(repo.join(".lore.json"), r#"{"project":"my-project"}"#).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_lore-cli"))
        .arg("project").current_dir(&repo).env("LORE_HOME", dir.path().join("app")).output().unwrap();
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["project"], "my-project");
    assert!(!dir.path().join("app").exists());
}
