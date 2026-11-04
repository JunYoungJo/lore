use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use qmd_memory_core::error::{Error, Result};
use qmd_memory_core::hook;
use qmd_memory_core::hooks;
use qmd_memory_core::migrate;
use qmd_memory_core::queue::{Job, Queue, Status};
use qmd_memory_core::recorder::CliRecorder;
use qmd_memory_core::run::RECORDER_MARK;
use qmd_memory_core::scan;
use qmd_memory_core::settings::{self, Paths};
use qmd_memory_core::transcript::Provider;
use qmd_memory_core::worker::{self, Outcome, Reporter, WorkerLock};

const USAGE: &str = "사용법: qmd-memory-cli <명령>
  hook <claude|codex>              세션 종료 훅 입력(표준입력 JSON)을 큐에 등록
  enqueue <claude|codex> <원문>     원문 파일을 큐에 등록
  scan                             훅이 놓친 세션을 찾아 큐에 등록
  work [--once]                    큐의 작업을 처리 (트레이 앱이 실행 중이면 거부)
  status                           큐 상태와 실패한 작업 출력
  retry <작업 ID>                   실패하거나 건너뛴 작업을 다시 대기열에 넣음
  hooks                            Claude·Codex 훅 연결 상태 출력
  connect <claude|codex>           전역 훅 설정에 이 CLI를 추가 (이전 설정은 백업)
  disconnect <claude|codex>        전역 훅 설정에서 qmd-memory 훅을 제거 (이전 설정은 백업)
  migrate-links [--apply] [--backup <폴더>]
                                   기억 문서의 원본 세션 링크를 대화 사본으로 옮김 (기본은 미리보기)";

struct PrintReporter;

impl Reporter for PrintReporter {
    fn started(&self, job: &Job, project: &str) {
        println!("기록 시작: {project} ({})", job.id);
    }

    fn finished(&self, job: &Job, outcome: &Outcome) {
        println!("{}: {}", job.id, serde_json::to_string(outcome).unwrap_or_default());
    }
}

fn provider(value: Option<&String>) -> Result<Provider> {
    value.and_then(|value| Provider::parse(value)).ok_or_else(|| Error::permanent("원문 종류는 claude 또는 codex여야 합니다."))
}

fn open(paths: &Paths) -> Result<(settings::Settings, Queue)> {
    let settings = settings::load_or_init(&paths.settings(), worker::now())?;
    let queue = Queue::open(&paths.database())?;
    Ok((settings, queue))
}

fn run(args: &[String]) -> Result<()> {
    let paths = Paths::standard();
    let command = args.first().map(String::as_str).unwrap_or("");
    match command {
        "hook" => {
            if std::env::var_os(RECORDER_MARK).is_some() {
                return Ok(());
            }
            let provider = provider(args.get(1))?;
            let mut input = String::new();
            std::io::stdin().read_to_string(&mut input)?;
            let (settings, queue) = open(&paths)?;
            if let Some(transcript) = hook::transcript_for(provider, &input, Path::new(&settings.codex_sessions_dir))? {
                println!("{}", queue.enqueue(provider, &transcript, worker::now())?);
            }
        }
        "enqueue" => {
            let provider = provider(args.get(1))?;
            let transcript = args.get(2).ok_or_else(|| Error::permanent("원문 경로가 필요합니다."))?;
            let (_, queue) = open(&paths)?;
            println!("{}", queue.enqueue(provider, Path::new(transcript), worker::now())?);
        }
        "scan" => {
            let (settings, queue) = open(&paths)?;
            println!("등록한 세션: {}", scan::scan(&queue, &settings, worker::now())?);
        }
        "work" => {
            let Some(_lock) = WorkerLock::try_acquire(&paths.lock())? else {
                return Err(Error::transient("트레이 앱 또는 다른 처리 작업이 실행 중입니다."));
            };
            let (settings, queue) = open(&paths)?;
            settings.validate()?;
            let recorder = CliRecorder::from_settings(&settings, &paths.recorder_workdir());
            let once = args.iter().any(|arg| arg == "--once");
            loop {
                match worker::step(&queue, &settings, &paths, &recorder, &PrintReporter)? {
                    None => {
                        println!("처리할 작업이 없습니다.");
                        break;
                    }
                    Some(_) if once => break,
                    Some(_) => {}
                }
            }
        }
        "status" => {
            let (_, queue) = open(&paths)?;
            let counts = queue.counts(worker::now())?;
            println!("{}", serde_json::to_string_pretty(&counts)?);
            for job in queue.jobs_with_status(&[Status::Failed, Status::Pending, Status::Applying, Status::IndexPending], 50)? {
                println!("{} {:?} {} {}", job.id, job.status, job.project.as_deref().unwrap_or("-"), job.last_error.as_deref().unwrap_or(""));
            }
        }
        "retry" => {
            let id = args.get(1).ok_or_else(|| Error::permanent("작업 ID가 필요합니다."))?;
            let (_, queue) = open(&paths)?;
            if !queue.retry(id, worker::now())? {
                return Err(Error::permanent("실패했거나 건너뛴 작업만 다시 넣을 수 있습니다."));
            }
            println!("다시 대기열에 넣었습니다: {id}");
        }
        "hooks" => {
            let cli = std::env::current_exe()?;
            let statuses: Vec<_> = [Provider::Claude, Provider::Codex].into_iter().map(|provider| hooks::status_at(&hooks::config_path(provider), provider, &cli)).collect();
            println!("{}", serde_json::to_string_pretty(&statuses)?);
        }
        "connect" => {
            let provider = provider(args.get(1))?;
            let cli = std::env::current_exe()?;
            let config = hooks::config_path(provider);
            let backup = hooks::register_at(&config, provider, &cli, worker::now())?;
            println!("{} 훅을 연결했습니다: {}", provider.as_str(), config.display());
            if let Some(backup) = backup {
                println!("이전 설정 백업: {}", backup.display());
            }
        }
        "disconnect" => {
            let provider = provider(args.get(1))?;
            let config = hooks::config_path(provider);
            match hooks::unregister_at(&config, provider, worker::now())? {
                0 => println!("{}에 연결된 qmd-memory 훅이 없습니다: {}", provider.as_str(), config.display()),
                removed => println!("{} 훅 {removed}개를 해제했습니다: {} (이전 설정은 같은 폴더에 백업)", provider.as_str(), config.display()),
            }
        }
        "migrate-links" => {
            let (settings, _) = open(&paths)?;
            let apply = args.iter().any(|arg| arg == "--apply");
            let backup = args.iter().position(|arg| arg == "--backup").and_then(|index| args.get(index + 1)).map(PathBuf::from);
            let report = migrate::migrate_links(&settings.knowledge_root(), backup.as_deref(), apply)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            if !apply {
                println!("미리보기입니다. 반영하려면 --apply를 붙이십시오.");
            }
        }
        _ => {
            println!("{USAGE}");
            if !command.is_empty() {
                return Err(Error::permanent(format!("알 수 없는 명령입니다: {command}")));
            }
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("오류: {error}");
            ExitCode::FAILURE
        }
    }
}
