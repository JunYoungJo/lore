use crate::state::AppState;
use qmd_memory_core::error::{Error, Result};
use qmd_memory_core::qmd::Qmd;
use qmd_memory_core::queue::{Queue, Status};
use qmd_memory_core::recorder::CliRecorder;
use qmd_memory_core::run;
use qmd_memory_core::settings::{self, Paths, Settings};
use qmd_memory_core::sync::{self, Choice, Plan, Summary};
use serde::Serialize;
use std::collections::BTreeMap;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_notification::NotificationExt;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    root: String,
    enabled: bool,
    busy: bool,
    summary: Summary,
    plan: Option<Plan>,
}

fn changed(app: &AppHandle) {
    crate::tray::refresh_tooltip(app);
    let _ = app.emit("state-changed", ());
}

#[tauri::command]
pub async fn open_sync(app: AppHandle) -> std::result::Result<(), String> {
    if let Some(window) = app.get_webview_window("sync") {
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }
    WebviewWindowBuilder::new(&app, "sync", WebviewUrl::App("index.html".into()))
        .title("기억 동기화 검토")
        .inner_size(1120.0, 820.0)
        .min_inner_size(740.0, 620.0)
        .center()
        .build()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn get_sync(state: State<'_, AppState>) -> std::result::Result<Snapshot, String> {
    let settings = state.settings();
    let root = settings.knowledge_root();
    let path = sync::journal(&state.paths, &root);
    Ok(Snapshot {
        root: root.to_string_lossy().into(),
        enabled: settings.sync_enabled,
        busy: state.syncing(),
        summary: sync::summary(&path),
        plan: sync::load(&path).map_err(|e| e.to_string())?,
    })
}

#[tauri::command]
pub fn set_sync_enabled(app: AppHandle, enabled: bool) -> std::result::Result<(), String> {
    let state = app.state::<AppState>();
    let mut settings = state.settings();
    settings.sync_enabled = enabled;
    settings::save(&state.paths.settings(), &settings).map_err(|e| e.to_string())?;
    state.replace_settings(settings);
    changed(&app);
    Ok(())
}

#[tauri::command]
pub fn defer_sync(app: AppHandle) -> std::result::Result<(), String> {
    if app.state::<AppState>().syncing() {
        return Err("동기화 작업이 끝난 뒤 닫으십시오.".into());
    }
    if let Some(window) = app.get_webview_window("sync") {
        window.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn run_exclusive(
    app: &AppHandle,
    operation: impl FnOnce(&Settings, &Paths) -> Result<()>,
) -> Result<()> {
    let state = app.state::<AppState>();
    let busy = state
        .begin_sync()
        .ok_or_else(|| Error::transient("다른 동기화 작업이 진행 중입니다."))?;
    changed(app);
    let result = {
        let _operation = state.operation();
        let settings = state.settings();
        settings.validate().and_then(|_| {
            if state.reviewing() { return Err(Error::permanent("기억 문서 검토 창을 닫은 뒤 동기화하십시오.")); }
            let queue = Queue::open(&state.paths.database())?;
            if !queue.jobs_with_status(&[Status::Applying, Status::IndexPending], 1)?.is_empty() {
                return Err(Error::permanent("중단된 기록 반영·검색 갱신을 먼저 완료해야 합니다. 해당 작업을 다시 시도하십시오."));
            }
            operation(&settings, &state.paths)
        })
    };
    if result.is_ok()
        && state
            .problem()
            .is_some_and(|message| message.starts_with("자동 동기화 대기:"))
    {
        state.set_problem(None);
    }
    drop(busy);
    changed(app);
    result
}

fn finish(settings: &Settings, paths: &Paths) -> Result<()> {
    let root = settings.knowledge_root();
    let path = sync::journal(paths, &root);
    sync::finish(&root, &path, || {
        let qmd = Qmd::new(
            settings,
            &paths.recorder_workdir(),
            run::fixed(settings.qmd_timeout_secs),
        );
        if !qmd.available() {
            return Err(Error::transient(
                "qmd를 찾지 못했습니다. doctor로 설치와 실행 경로를 확인하십시오.",
            ));
        }
        qmd.ensure_index(&root)?;
        qmd.refresh()
    })?;
    Ok(())
}

fn prepare(settings: &Settings, paths: &Paths) -> Result<()> {
    let root = settings.knowledge_root();
    let path = sync::journal(paths, &root);
    let plan = sync::prepare(&root, &path)?;
    if plan.phase != "review" {
        finish(settings, paths)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn prepare_sync(app: AppHandle) -> std::result::Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || run_exclusive(&app, prepare))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn analyze_sync(app: AppHandle) -> std::result::Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        run_exclusive(&app, |settings, paths| {
            let root = settings.knowledge_root();
            let path = sync::journal(paths, &root);
            let recorder = CliRecorder::from_settings(
                settings,
                &paths.recorder_workdir(),
                run::fixed(settings.model_timeout_secs),
            );
            let plan = sync::load(&path)?
                .ok_or_else(|| Error::permanent("먼저 공유 저장소의 변경을 확인하십시오."))?;
            sync::analyze(
                &root,
                &path,
                &recorder,
                &path.parent().unwrap().join(format!("analysis-{}", plan.id)),
            )?;
            Ok(())
        })
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    files: BTreeMap<String, String>,
    token: String,
}
fn preview_token(id: &str, files: &BTreeMap<String, String>) -> Result<String> {
    Ok(qmd_memory_core::transcript::digest(
        format!("{id}:{}", serde_json::to_string(files)?).as_bytes(),
    ))
}

#[tauri::command]
pub async fn preview_sync(
    app: AppHandle,
    id: String,
    choices: Vec<Choice>,
) -> std::result::Result<Preview, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let path = sync::journal(&state.paths, &state.settings().knowledge_root());
        let plan = sync::load(&path)
            .map_err(|e| e.to_string())?
            .ok_or("검토 자료가 없습니다.")?;
        if plan.id != id || plan.phase != "review" {
            return Err("비교 자료가 바뀌었습니다. 다시 선택하십시오.".into());
        }
        let files = sync::preview(&plan, &choices).map_err(|e| e.to_string())?;
        let token = preview_token(&id, &files).map_err(|e| e.to_string())?;
        Ok(Preview { files, token })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn apply_sync(
    app: AppHandle,
    id: String,
    choices: Vec<Choice>,
    token: String,
) -> std::result::Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        run_exclusive(&app, |settings, paths| {
            let root = settings.knowledge_root();
            let path = sync::journal(paths, &root);
            let plan =
                sync::load(&path)?.ok_or_else(|| Error::permanent("검토할 계획이 없습니다."))?;
            let preview = sync::preview(&plan, &choices)?;
            if plan.id != id || preview_token(&id, &preview)? != token {
                return Err(Error::permanent(
                    "미리보기와 선택한 내용이 다릅니다. 최종 문구를 다시 확인하십시오.",
                ));
            }
            sync::approve(&root, &path, &id, &choices)?;
            finish(settings, paths)
        })
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn resume_sync(app: AppHandle) -> std::result::Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || run_exclusive(&app, finish))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

pub fn automatic(app: &AppHandle) {
    let before = {
        let state = app.state::<AppState>();
        sync::summary(&sync::journal(
            &state.paths,
            &state.settings().knowledge_root(),
        ))
    };
    let result = run_exclusive(app, |settings, paths| {
        let path = sync::journal(paths, &settings.knowledge_root());
        match sync::load(&path)? {
            Some(plan) if plan.phase == "review" => Ok(()),
            Some(_) => finish(settings, paths),
            None => prepare(settings, paths),
        }
    });
    let state = app.state::<AppState>();
    if let Err(error) = result {
        state.set_problem(Some(format!("자동 동기화 대기: {error}")));
    }
    let after = sync::summary(&sync::journal(
        &state.paths,
        &state.settings().knowledge_root(),
    ));
    if after.phase == "review" && before.phase != "review" {
        let _ = app.notification().builder().title("기억 동기화 확인 필요").body("양쪽 변경을 확인해야 합니다. 앱의 동기화에서 모델 비교와 최종 문구를 확인하십시오.").show();
    }
    changed(app);
}
