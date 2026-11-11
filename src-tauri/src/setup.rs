use std::path::{Path, PathBuf};

use lore_core::{diagnostics, hooks, platform, qmd::{self, Qmd}, queue::Queue, run, settings, setup, transcript::Provider, worker};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_dialog::DialogExt;

use crate::{commands, setup_terminal, state::AppState};

fn err(e: impl std::fmt::Display) -> String { e.to_string() }
fn provider(value: &str) -> Result<Provider, String> { Provider::parse(value).ok_or_else(|| "Unknown AI tool.".into()) }
fn changed(app: &AppHandle) { let _ = app.emit("state-changed", ()); }

pub fn create_window(app: &AppHandle, visible: bool) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("setup") {
        if visible { window.show().map_err(err)?; window.set_focus().map_err(err)?; }
        return Ok(());
    }
    let window = WebviewWindowBuilder::new(app, "setup", WebviewUrl::App("index.html".into()))
        .title("Set up Lore").inner_size(1040.0, 790.0).min_inner_size(760.0, 640.0)
        .decorations(false).center().visible(visible).build().map_err(err)?;
    crate::webview::harden(&window);
    // Closing retains the running command and saved step. Reopen from Settings.
    let handle = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event { api.prevent_close(); let _ = handle.hide(); }
    });
    Ok(())
}

#[tauri::command]
pub async fn open_setup(app: AppHandle) -> Result<(), String> {
    create_window(&app, true)
}

#[tauri::command]
pub async fn setup_pick_folder(app: AppHandle) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog().file().set_title("Choose a Lore memory folder").blocking_pick_folder()
            .map(|file| file.into_path().map(|p| p.to_string_lossy().to_string()).map_err(err)).transpose()
    }).await.map_err(err)?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Client {
    provider: Provider,
    plan: Option<setup::Plan>,
    error: Option<String>,
    hooks: hooks::HookStatus,
}

#[tauri::command]
pub fn get_setup(state: State<'_, AppState>) -> Result<Value, String> {
    let settings = state.settings();
    let cli = commands::cli_path();
    let clients: Vec<_> = [Provider::Codex, Provider::Claude].into_iter().map(|provider| {
        let (plan, error) = match setup::plan(&platform::home(), &state.paths, provider, &settings) {
            Ok(plan) => (Some(plan), None), Err(e) => (None, Some(e.to_string())),
        };
        Client { provider, plan, error, hooks: hooks::status_at(&hooks::config_path(provider), provider, &cli) }
    }).collect();
    let tools: Vec<_> = ["node", "npm", "git", "brew", "sqlite3"].into_iter().map(|name| {
        let found = if name == "sqlite3" && cfg!(target_os = "macos") {
            ["/opt/homebrew/opt/sqlite/bin/sqlite3", "/usr/local/opt/sqlite/bin/sqlite3"].into_iter().map(PathBuf::from).find(|p| platform::is_executable(p))
        } else { platform::which(name) };
        json!({"name": name, "path": found})
    }).collect();
    let recent = Queue::open(&state.paths.database()).map_err(err)?.recent_activity(30).map_err(err)?;
    let index_config = std::fs::read_to_string(settings.qmd_config_dir().join(format!("{}.yml", settings.qmd_index))).unwrap_or_default();
    let root = settings.knowledge_root();
    Ok(json!({
        "progress": setup::progress(&state.paths).map_err(err)?, "settings": settings,
        "diagnostics": diagnostics::inspect(&state.paths, &cli), "tools": tools,
        "clients": clients, "platform": std::env::consts::OS,
        "recorded": recent.iter().filter(|a| a.kind == "recorded").count(),
        "testRecorded": setup::progress(&state.paths).map_err(err)?.test_started_at.is_some_and(|start| recent.iter().any(|a| a.kind == "recorded" && a.at >= start)),
        "rootExists": root.is_dir(),
        "qwenModel": qmd::QWEN_EMBED,
        "embedModel": qmd::embed_model(&index_config),
        "embedModels": qmd::EMBED_MODELS,
        "collections": qmd::collection_names(&index_config),
        "repository": root.join(".git").exists(),
        "tracking": lore_core::sync::tracking(&root),
    }))
}

#[tauri::command]
pub fn save_setup_progress(state: State<'_, AppState>, mut progress: setup::Progress) -> Result<(), String> {
    if progress.step > 5 { return Err("Unknown setup step.".into()); }
    // Starting the test is timestamped by the app, never by a client-supplied clock.
    let saved = setup::progress(&state.paths).map_err(err)?;
    progress.test_started_at = saved.test_started_at;
    if progress.completed && !saved.completed {
        let settings = state.settings();
        if progress.providers.is_empty() || settings.paused || !diagnostics::command_available(&settings.commands.qmd) || !settings.knowledge_root().is_dir() || !settings.qmd_config_dir().join(format!("{}.yml", settings.qmd_index)).is_file() {
            return Err("Finish the required tools, memory folder, connections, and index steps first.".into());
        }
        for provider in &progress.providers {
            let plan = setup::plan(&platform::home(), &state.paths, *provider, &settings).map_err(err)?;
            let hooks = hooks::status_at(&hooks::config_path(*provider), *provider, &commands::cli_path());
            if hooks.state != hooks::HookState::Connected || plan.resources.iter().any(|r| !matches!(r.state.as_str(), "ready" | "preserved")) { return Err("Recheck the selected AI connections before finishing.".into()); }
        }
    }
    setup::save_progress(&state.paths, &progress).map_err(err)
}

#[tauri::command]
pub async fn setup_check_tools(app: AppHandle) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let settings = state.settings();
        let mut result = json!({"nodeVersion": null, "nodeReady": false, "auth": {}});
        let check = |command: Vec<String>| {
            run::run_allow(&command, &state.paths.recorder_workdir(), &[], None, &|| std::time::Duration::from_secs(12), &[0, 1])
        };
        if let Some(node) = platform::which("node") {
            if let Ok(output) = check(vec![node.to_string_lossy().into(), "--version".into()]) {
                let version = output.stdout.trim();
                result["nodeVersion"] = json!(version);
                result["nodeReady"] = json!(version.trim_start_matches('v').split('.').next().and_then(|s| s.parse::<u32>().ok()).is_some_and(|major| major >= 22));
            }
        }
        for provider in [Provider::Claude, Provider::Codex] {
            let mut command = if provider == Provider::Codex { settings.commands.codex.clone() } else { settings.commands.claude.clone() };
            let status = if diagnostics::command_available(&command) {
                command.extend(if provider == Provider::Codex { ["login", "status"] } else { ["auth", "status"] }.map(str::to_string));
                match check(command) {
                    Ok(output) if output.code == Some(0) => "ready",
                    Ok(_) => "signed_out",
                    Err(_) => "unknown",
                }
            } else { "unavailable" };
            result["auth"][provider.as_str()] = json!(status);
        }
        Ok(result)
    }).await.map_err(err)?
}

#[tauri::command]
pub async fn open_integrations(app: AppHandle, section: Option<String>) -> Result<(), String> {
    let section = section.filter(|section| section == "folder");
    if let Some(window) = app.get_webview_window("integrations") {
        window.show().map_err(err)?;
        if let Some(section) = &section { app.emit_to("integrations", "integrations-section", section).map_err(err)?; }
        return window.set_focus().map_err(err);
    }
    let page = section.map_or_else(|| "index.html".to_string(), |section| format!("index.html?section={section}"));
    let window = WebviewWindowBuilder::new(&app, "integrations", WebviewUrl::App(page.into()))
        .title("Lore integrations").inner_size(1040.0, 790.0).min_inner_size(760.0, 640.0)
        .decorations(false).center().build().map_err(err)?;
    crate::webview::harden(&window);
    Ok(())
}

#[tauri::command]
pub fn setup_begin_test(state: State<'_, AppState>) -> Result<(), String> {
    if state.settings().paused { return Err("Resume Lore recording from the tray before starting the test.".into()); }
    let mut progress = setup::progress(&state.paths).map_err(err)?;
    progress.completed = false;
    progress.test_started_at = Some(worker::now());
    progress.recall_confirmed = false;
    setup::save_progress(&state.paths, &progress).map_err(err)
}

// Detect newly installed commands without overwriting a working custom command.
#[tauri::command]
pub fn setup_detect(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let mut settings = state.settings();
    for command in [&mut settings.commands.codex, &mut settings.commands.claude, &mut settings.commands.qmd] {
        if !diagnostics::command_available(command) { command.clear(); }
    }
    settings.fill_missing_commands();
    // A Finder-launched app needs an absolute Node path for qmd's env shebang.
    #[cfg(unix)]
    if settings.commands.qmd.len() == 1 {
        let script = &settings.commands.qmd[0];
        let shebang = std::fs::read(script).ok().map(|bytes| bytes.iter().take(160).copied().collect::<Vec<_>>()).unwrap_or_default();
        if shebang.starts_with(b"#!") && String::from_utf8_lossy(&shebang).lines().next().unwrap_or("").contains("node") {
            if let Some(node) = platform::which("node") { settings.commands.qmd.insert(0, node.to_string_lossy().into()); }
        }
    }
    settings::save(&state.paths.settings(), &settings).map_err(err)?;
    state.replace_settings(settings);
    commands::refresh_catalog(&app);
    changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn setup_repository(app: AppHandle, path: String, create: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        if setup_terminal::running(&app) { return Err("Finish the setup terminal command before changing the memory folder.".into()); }
        let state = app.state::<AppState>();
        let _busy = state.begin_setup().ok_or("Another setup operation is running.")?;
        if state.operation_busy() || state.syncing() || state.reviewing() { return Err("Wait for recording or sync to finish and close review windows.".into()); }
        let _gate = state.operation();
        let root = PathBuf::from(path.trim());
        if !root.is_absolute() { return Err("Choose an absolute folder path on this PC.".into()); }
        if create && root.is_dir() && std::fs::read_dir(&root).map_err(err)?.next().is_some() {
            return Err("The new folder must be empty. Choose Use existing for an existing memory repository.".into());
        }
        if !create && !root.is_dir() { return Err("The folder does not exist. Create it or finish cloning first.".into()); }
        std::fs::create_dir_all(&root).map_err(err)?;
        if create {
            let git = platform::which("git").ok_or("Install Git first.")?;
            run::run(&[git.to_string_lossy().into(), "init".into(), "--".into(), root.to_string_lossy().into()], &root, &[], None, &|| std::time::Duration::from_secs(30)).map_err(err)?;
        }
        for folder in ["common", "projects", "sources"] { std::fs::create_dir_all(root.join(folder)).map_err(err)?; }
        let mut settings = state.settings();
        if settings.knowledge_root() != root {
            let mut progress = setup::progress(&state.paths).map_err(err)?;
            progress.completed = false;
            progress.test_started_at = None;
            progress.recall_confirmed = false;
            setup::save_progress(&state.paths, &progress).map_err(err)?;
        }
        settings.knowledge_root = root.to_string_lossy().replace('\\', "/");
        settings.validate().map_err(err)?;
        settings::save(&state.paths.settings(), &settings).map_err(err)?;
        state.replace_settings(settings);
        changed(&app);
        Ok(())
    }).await.map_err(err)?
}

#[tauri::command]
pub fn setup_connect(app: AppHandle, state: State<'_, AppState>, provider: String, token: String) -> Result<(), String> {
    if setup_terminal::running(&app) { return Err("Finish the setup terminal command before connecting.".into()); }
    let provider = self::provider(&provider)?;
    let cli = commands::cli_path();
    if !platform::is_executable(&cli) { return Err("The bundled Lore CLI is missing. Repair the app installation first.".into()); }
    setup::connect(&platform::home(), &state.paths, provider, &state.settings(), &token).map_err(err)?;
    hooks::register_at(&hooks::config_path(provider), provider, &cli, worker::now()).map_err(err)?;
    changed(&app);
    Ok(())
}

#[tauri::command]
pub fn setup_disconnect(app: AppHandle, state: State<'_, AppState>, provider: String) -> Result<Vec<String>, String> {
    if setup_terminal::running(&app) { return Err("Finish the setup terminal command before disconnecting.".into()); }
    let provider = self::provider(&provider)?;
    let kept = setup::disconnect(&platform::home(), &state.paths, provider).map_err(err)?;
    hooks::unregister_at(&hooks::config_path(provider), provider, worker::now()).map_err(err)?;
    changed(&app);
    Ok(kept)
}

#[tauri::command]
pub async fn setup_index(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        if setup_terminal::running(&app) { return Err("Finish the setup terminal command before indexing.".into()); }
        let state = app.state::<AppState>();
        let _busy = state.begin_setup().ok_or("Another setup operation is running.")?;
        if state.operation_busy() || state.syncing() || state.reviewing() { return Err("Wait for recording or sync to finish and close review windows.".into()); }
        let _gate = state.operation();
        let settings = state.settings();
        settings.validate().map_err(err)?;
        if !settings.knowledge_root().is_dir() { return Err("Set up the memory folder first.".into()); }
        let config = settings.qmd_config_dir().join(format!("{}.yml", settings.qmd_index));
        // Only fresh indexes receive the multilingual model. An existing index's
        // model and embeddings are preserved during setup and repair.
        if !config.exists() {
            settings::atomic_write(&config, format!("models:\n  embed: \"{}\"\ncollections: {{}}\n", qmd::QWEN_EMBED).as_bytes()).map_err(err)?;
        }
        let live = app.clone();
        let limit: run::Limit = std::sync::Arc::new(move || std::time::Duration::from_secs(live.state::<AppState>().read_settings(|s| s.qmd_timeout_secs)));
        let qmd = Qmd::new(&settings, &state.paths.recorder_workdir(), limit);
        qmd.ensure_index(&settings.knowledge_root()).map_err(err)?;
        qmd.refresh().map_err(err)?;
        changed(&app);
        Ok(())
    }).await.map_err(err)?
}

#[tauri::command]
pub async fn setup_embed_model(app: AppHandle, model: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        if setup_terminal::running(&app) { return Err("Finish the setup terminal command before changing the model.".into()); }
        if !qmd::EMBED_MODELS.iter().any(|known| known.uri == model) { return Err("Choose one of the listed embedding models.".into()); }
        let state = app.state::<AppState>();
        let _busy = state.begin_setup().ok_or("Another setup operation is running.")?;
        if state.operation_busy() || state.syncing() || state.reviewing() { return Err("Wait for recording or sync to finish and close review windows.".into()); }
        let _gate = state.operation();
        let settings = state.settings();
        let live = app.clone();
        let limit: run::Limit = std::sync::Arc::new(move || std::time::Duration::from_secs(live.state::<AppState>().read_settings(|s| s.qmd_timeout_secs)));
        let qmd = Qmd::new(&settings, &state.paths.recorder_workdir(), limit);
        let config = std::fs::read_to_string(qmd.config_path()).map_err(|_| "Build the search index first.".to_string())?;
        settings::atomic_write(&qmd.config_path(), qmd::with_embed_model(&config, &model).as_bytes()).map_err(err)?;
        changed(&app);
        let rebuilt = qmd.rebuild_vectors().map_err(err);
        changed(&app);
        rebuilt
    }).await.map_err(err)?
}

fn tool(name: &str) -> Result<Vec<String>, String> {
    let program = platform::which(name).ok_or_else(|| format!("{name} was not found. Install it, then check again."))?;
    #[cfg(windows)]
    if program.extension().is_some_and(|e| e == "cmd" || e == "ps1" || e == "bat") {
        // Predefined installer commands only; PowerShell quotes every argument.
        return Ok(vec!["powershell.exe".into(), "-NoProfile".into(), "-Command".into(), format!("& '{}'", program.to_string_lossy().replace('\'', "''"))]);
    }
    Ok(vec![program.to_string_lossy().into()])
}
fn with_args(mut command: Vec<String>, args: &[&str]) -> Vec<String> {
    #[cfg(windows)]
    if command.first().is_some_and(|s| s == "powershell.exe") {
        let script = command.last_mut().unwrap();
        for arg in args { script.push_str(&format!(" '{}'", arg.replace('\'', "''"))); }
        return command;
    }
    command.extend(args.iter().map(|s| s.to_string())); command
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Prepared { pub command: String, pub argv: Vec<String>, pub cwd: PathBuf, pub env: Vec<(String, String)>, pub token: String }

fn prepare(settings: &settings::Settings, task: &str, remote: &str, path: &str, query: &str) -> Result<Prepared, String> {
    let mut cwd = platform::home();
    let mut env = Vec::new();
    let argv = match task {
        "install_qmd" => with_args(tool("npm")?, &["install", "-g", "@tobilu/qmd@2.8.3"]),
        "install_codex" => with_args(tool("npm")?, &["install", "-g", "@openai/codex"]),
        "install_claude" => with_args(tool("npm")?, &["install", "-g", "@anthropic-ai/claude-code"]),
        "install_sqlite" if cfg!(target_os = "macos") => with_args(tool("brew")?, &["install", "sqlite"]),
        "install_node" if cfg!(target_os = "macos") => with_args(tool("brew")?, &["install", "node@24"]),
        "install_node" if cfg!(windows) => with_args(tool("winget")?, &["install", "--id", "OpenJS.NodeJS.LTS", "--exact", "--source", "winget"]),
        "install_git" if cfg!(target_os = "macos") => with_args(tool("brew")?, &["install", "git"]),
        "install_git" if cfg!(windows) => with_args(tool("winget")?, &["install", "--id", "Git.Git", "--exact", "--source", "winget"]),
        "node_version" => with_args(tool("node")?, &["--version"]),
        "login_codex" => with_args(settings.commands.codex.clone(), &["login"]),
        "login_claude" => with_args(settings.commands.claude.clone(), &["auth", "login"]),
        "trust_codex" => settings.commands.codex.clone(),
        "open_claude" => settings.commands.claude.clone(),
        "clone" => {
            if remote.trim().is_empty() || remote.starts_with('-') || remote.contains(['\n', '\r']) { return Err("Enter a Git remote URL.".into()); }
            let root = Path::new(path.trim());
            if !root.is_absolute() { return Err("Choose an absolute destination path.".into()); }
            if root.exists() && std::fs::read_dir(root).map_err(err)?.next().is_some() { return Err("Clone into an empty or new folder.".into()); }
            with_args(tool("git")?, &["clone", "--", remote.trim(), root.to_str().ok_or("Invalid destination path.")?])
        }
        "search" | "qmd_status" => {
            cwd = settings.knowledge_root();
            env = vec![("QMD_CONFIG_DIR".into(), settings.qmd_config_dir().to_string_lossy().into()), ("XDG_CACHE_HOME".into(), settings.qmd_cache_dir().to_string_lossy().into())];
            let mut command = settings.commands.qmd.clone();
            command.extend(["--index".into(), settings.qmd_index.clone()]);
            if task == "qmd_status" { command.push("status".into()); }
            else {
                if query.trim().is_empty() { return Err("Enter a search term from a memory note.".into()); }
                command.extend(["search".into(), query.into(), "-c".into(), "common".into(), "--format".into(), "json".into(), "-n".into(), "5".into()]);
            }
            command
        }
        _ => return Err("This installer is not available on this platform. Use the official download link, then check again.".into()),
    };
    if argv.is_empty() { return Err("Install this CLI and check again first.".into()); }
    // Display only: argv is executed directly, never interpolated into a shell.
    let command = argv.iter().map(|s| if s.contains([' ', '\'', '"']) { format!("{:?}", s) } else { s.clone() }).collect::<Vec<_>>().join(" ");
    let token = lore_core::transcript::digest(serde_json::to_string(&json!([argv, cwd, env])).map_err(err)?.as_bytes());
    Ok(Prepared { command, argv, cwd, env, token })
}

#[tauri::command]
pub fn setup_prepare(state: State<'_, AppState>, task: String, remote: String, path: String, query: String) -> Result<Prepared, String> {
    prepare(&state.settings(), &task, &remote, &path, &query)
}

#[tauri::command]
pub fn setup_run(app: AppHandle, state: State<'_, AppState>, request: RunRequest) -> Result<(), String> {
    if state.setup_busy() { return Err("Wait for memory preparation to finish before running another command.".into()); }
    let prepared = prepare(&state.settings(), &request.task, &request.remote, &request.path, &request.query)?;
    if prepared.token != request.token { return Err("The command changed after preview. Prepare it again before running.".into()); }
    setup_terminal::start(&app, &prepared.argv, &prepared.cwd, &prepared.env, request.cols, request.rows)
}

#[derive(Deserialize)]
pub struct RunRequest { task: String, remote: String, path: String, query: String, token: String, cols: u16, rows: u16 }

#[tauri::command]
pub fn setup_open_docs(app: AppHandle, topic: String) -> Result<(), String> {
    let url = match topic.as_str() {
        "node" => "https://nodejs.org/en/download",
        "git" => "https://git-scm.com/downloads",
        "brew" => "https://brew.sh",
        "qmd" => "https://github.com/tobi/qmd",
        _ => return Err("Unknown documentation link.".into()),
    };
    app.opener().open_url(url, None::<&str>).map_err(err)
}
