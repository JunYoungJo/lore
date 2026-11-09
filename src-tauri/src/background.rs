use std::sync::Arc;
use std::time::Duration;

use qmd_memory_core::error::Failure;
use qmd_memory_core::qmd::Qmd;
use qmd_memory_core::queue::{Job, Queue};
use qmd_memory_core::recorder::CliRecorder;
use qmd_memory_core::run::Limit;
use qmd_memory_core::settings::Settings;
use qmd_memory_core::worker::{self, Outcome, Reporter, WorkerLock};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::state::{AppState, Current};
use crate::tray;

const IDLE_WAIT: Duration = Duration::from_secs(5);
const ERROR_WAIT: Duration = Duration::from_secs(30);

fn notify(app: &AppHandle, title: &str, body: &str) {
    if let Err(error) = app.notification().builder().title(title).body(body).show() {
        eprintln!("알림을 띄우지 못했습니다: {error}");
    }
}

fn changed(app: &AppHandle) {
    tray::refresh_tooltip(app);
    let _ = app.emit("state-changed", ());
}

fn live_limit(app: &AppHandle, seconds: fn(&Settings) -> u64) -> Limit {
    let app = app.clone();
    Arc::new(move || Duration::from_secs(app.state::<AppState>().read_settings(seconds)))
}

fn minutes_until(timestamp: i64) -> i64 {
    ((timestamp - worker::now()).max(0) + 59) / 60
}

struct AppReporter {
    app: AppHandle,
}

impl Reporter for AppReporter {
    fn started(&self, job: &Job, project: &str) {
        let state = self.app.state::<AppState>();
        state.set_current(Some(Current { job_id: job.id.clone(), provider: job.provider, project: project.to_string(), started_at: worker::now() }));
        notify(&self.app, "기록 시작", &format!("{project} 세션을 기억 문서로 정리합니다."));
        changed(&self.app);
    }

    fn finished(&self, _job: &Job, outcome: &Outcome) {
        self.app.state::<AppState>().set_current(None);
        match outcome {
            Outcome::Recorded { project, notes, .. } => {
                notify(&self.app, "기록 완료", &format!("{project}: 기억 문서 {}개를 고쳤습니다. 눌러서 확인할 수 있습니다.", notes.len()));
            }
            Outcome::NoChange { project, .. } => notify(&self.app, "기록 완료", &format!("{project}: 새로 기록할 내용이 없습니다.")),
            Outcome::Failed { project, message, gave_up, next_attempt_at } => {
                let target = project.clone().unwrap_or_else(|| "세션".into());
                let next = if *gave_up { "자동으로 다시 시도하지 않습니다.".to_string() } else { format!("약 {}분 뒤 다시 시도합니다.", minutes_until(*next_attempt_at)) };
                notify(&self.app, "기록 실패", &format!("{target}: {message} {next}"));
            }
            Outcome::Skipped { .. } | Outcome::CaughtUp => {}
        }
        changed(&self.app);
    }
}

pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let paths = state.paths.clone();
        let _lock = match WorkerLock::try_acquire(&paths.lock()) {
            Ok(Some(lock)) => lock,
            Ok(None) => {
                state.set_problem(Some("다른 처리 작업(qmd-memory-cli work)이 실행 중이라 기록을 시작하지 못했습니다. 그 작업이 끝난 뒤 앱을 다시 실행하십시오.".into()));
                changed(&app);
                return;
            }
            Err(error) => {
                state.set_problem(Some(format!("처리 잠금 파일을 열지 못했습니다: {error}")));
                changed(&app);
                return;
            }
        };
        let queue = match Queue::open(&paths.database()) {
            Ok(queue) => queue,
            Err(error) => {
                state.set_problem(Some(format!("상태 DB를 열지 못했습니다: {error}")));
                changed(&app);
                return;
            }
        };
        let reporter = AppReporter { app: app.clone() };
        let model_limit = live_limit(&app, |settings| settings.model_timeout_secs);
        let qmd_limit = live_limit(&app, |settings| settings.qmd_timeout_secs);
        loop {
            let settings = state.settings();
            if settings.paused || state.reviewing() {
                state.wait(ERROR_WAIT);
                continue;
            }
            if let Err(error) = settings.validate() {
                state.set_problem(Some(error.to_string()));
                changed(&app);
                state.wait(ERROR_WAIT);
                continue;
            }
            let recorder = CliRecorder::from_settings(&settings, &paths.recorder_workdir(), model_limit.clone());
            let qmd = Qmd::new(&settings, &paths.recorder_workdir(), qmd_limit.clone());
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| worker::step(&queue, &settings, &paths, &recorder, &qmd, &reporter)));
            match result {
                Ok(Ok(Some(_))) => state.set_problem(None),
                Ok(Ok(None)) => state.wait(IDLE_WAIT),
                Ok(Err(error)) => {
                    state.set_current(None);
                    state.set_problem(Some(format!("작업을 처리하지 못했습니다: {error}")));
                    changed(&app);
                    state.wait(ERROR_WAIT);
                }
                Err(_) => {
                    state.set_current(None);
                    if let Ok(Some(job)) = queue.next_job(worker::now()) {
                        let _ = queue.record_failure(&job.id, Failure::Transient, "기록 처리 중 내부 오류가 발생했습니다.", worker::now());
                    }
                    state.set_problem(Some("기록 처리 중 내부 오류가 발생했습니다. 해당 작업은 뒤로 미루고 다음 작업을 처리합니다.".into()));
                    changed(&app);
                    state.wait(ERROR_WAIT);
                }
            }
        }
    });
}
