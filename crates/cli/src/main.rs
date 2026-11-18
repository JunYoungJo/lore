use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use lore_core::candidate::Kind;
use lore_core::cleanup;
use lore_core::error::{Error, Result};
use lore_core::hook::{self, Hook};
use lore_core::hooks;
use lore_core::mark::{self, Mark};
use lore_core::qmd::Qmd;
use lore_core::queue::{Queue, Status};
use lore_core::recorder::Recorders;
use lore_core::run::{fixed, RECORDER_MARK};
use lore_core::scan;
use lore_core::settings::{self, Paths};
use lore_core::transcript::Provider;
use lore_core::worker::{self, Discarded, Outcome, Reporter, Work, WorkerLock};

const USAGE: &str = "Usage: lore-cli <command>
  hook <claude|codex>              Handle hook input (JSON on stdin): queue a session that ended or is
                                  about to be compacted, or tell a session how to mark
  mark <decision|correction|lesson> <sentence>
                                  Mark something worth remembering, from inside a session. Prints one
                                  line that Lore reads from the session file; writes nothing
  enqueue <claude|codex> <file>     Queue a transcript file
  scan                            Find sessions missed by hooks and queue them
  work [--once]                   Process queued sessions and approved candidates (tray app must be closed)
  candidates                      List the candidates waiting for review
  candidates approve <id>...|all  Approve candidates; work then writes them into the notes
  candidates discard <id>...      Discard candidates
  status                          Show queue counts and failed jobs
  sync <status|now|compare|resolve|dismiss>
                                  Check upstream, compare, preview, and apply memory changes
  doctor                          Diagnose this PC's paths, CLIs, and qmd setup (read-only)
  index                           Register memory collections and refresh this PC's qmd index
  project                         Show the current Git repository's memory project ID
  retry <job-id>                  Requeue a failed or skipped job
  discard <job-id>                Skip a failed segment, or stop an unreadable session
  hooks                           Show Claude / Codex hook connection status
  connect <claude|codex>          Add this CLI to hook settings (backs up the previous file)
  disconnect <claude|codex>       Remove only Lore hooks (backs up the previous file)
  integrations disconnect <claude|codex>
                                  Remove unchanged Lore-owned MCP, rules, skill, and hooks
  cleanup [<folder>] [--absorb <folder>]... [--dry-run | --undo]
                                  Merge repeated notes and remove outdated ones in a project folder
                                  or in common, with the recording model. Without a folder, lists them
                                  with how many notes changed since their last cleanup.
                                  --absorb merges another project folder into it. Applied and committed
                                  at once unless --dry-run; tray app must be closed to apply.
                                  --undo reverts the folder's last cleanup with one commit
  --help, -h                     Show this help
  --version, -V                  Show the CLI version";

struct PrintReporter;

impl Reporter for PrintReporter {
    fn started(&self, work: &Work, project: &str) {
        let what = if matches!(work, Work::Session(_)) { "Reading a session" } else { "Writing approved candidates" };
        println!("{what}: {project} ({})", work.id());
    }

    fn finished(&self, work: &Work, outcome: &Outcome) {
        println!("{}: {}", work.id(), serde_json::to_string(outcome).unwrap_or_default());
    }

    fn session_ended(&self, project: &str, candidates: usize) {
        println!("{project}: the session ended with {candidates} candidate(s) to review. List them with `lore-cli candidates`.");
    }
}

fn candidate_ids(args: &[String]) -> Result<Vec<i64>> {
    if args.is_empty() {
        return Err(Error::permanent("Name at least one candidate ID."));
    }
    args.iter().map(|arg| arg.parse().map_err(|_| Error::permanent(format!("Not a candidate ID: {arg}")))).collect()
}

fn provider(value: Option<&String>) -> Result<Provider> {
    value.and_then(|value| Provider::parse(value)).ok_or_else(|| Error::permanent("The transcript provider must be claude or codex."))
}

fn open(paths: &Paths) -> Result<(settings::Settings, Queue)> {
    let settings = settings::load_or_init(&paths.settings(), worker::now())?;
    let queue = Queue::open(&paths.database())?;
    Ok((settings, queue))
}

fn run(args: &[String]) -> Result<()> {
    let paths = Paths::standard();
    let command = args.first().map(String::as_str).unwrap_or("");
    if matches!(command, "--help" | "-h" | "help") {
        println!("{USAGE}");
        return Ok(());
    }
    if matches!(command, "--version" | "-V") {
        println!("lore-cli {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    match command {
        "integrations" => {
            if args.get(1).map(String::as_str) != Some("disconnect") { return Err(Error::permanent("Use integrations disconnect <claude|codex>.")); }
            let provider = provider(args.get(2))?;
            let kept = lore_core::setup::disconnect(&lore_core::platform::home(), &paths, provider)?;
            hooks::unregister_at(&hooks::config_path(provider), provider, lore_core::worker::now())?;
            for warning in kept { println!("{warning}"); }
            println!("Removed unchanged Lore-owned integration entries and Lore hooks.");
        }
        "sync" => {
            let mode = args.get(1).map(String::as_str).unwrap_or("status");
            let Some(_lock) = WorkerLock::try_acquire(&paths.lock())? else { return Err(Error::transient("Close the tray app before using CLI sync.")); };
            let settings = settings::load_or_init(&paths.settings(), worker::now())?;
            settings.validate()?;
            let root = settings.knowledge_root();
            let folder = lore_core::sync::folder(&paths, &root);
            match mode {
                "status" => {}
                "now" => {
                    let queue = Queue::open(&paths.database())?;
                    if queue.write_interrupted()? {
                        return Err(Error::permanent("Approved candidates were being written into the notes when Lore stopped. Finish that with work first."));
                    }
                    lore_core::sync::sync(&root, &folder, || {
                        let qmd = Qmd::new(&settings, &paths.recorder_workdir(), fixed(settings.qmd_timeout_secs));
                        if !qmd.available() { return Err(Error::transient("qmd was not found.")); }
                        qmd.ensure_index(&root)?; qmd.refresh()
                    })?;
                }
                "compare" => {
                    let recorders = Recorders::from_settings(&settings, &paths.recorder_workdir(), fixed(settings.model_timeout_secs), |_| None);
                    lore_core::sync::retry_comparison(&folder)?;
                    while let Some(id) = lore_core::sync::pending_comparison(&folder, worker::now()) {
                        lore_core::sync::compare(&folder, &id, &mut |prompt, schema, artifacts| recorders.request_json(prompt, schema, artifacts), &mut || {})?;
                    }
                }
                "resolve" => {
                    let id = args.get(2).ok_or_else(|| Error::permanent("The item ID is required."))?;
                    let option: usize = args.get(3).and_then(|value| value.parse().ok()).ok_or_else(|| Error::permanent("The option number is required."))?;
                    lore_core::sync::resolve_item(&root, &folder, id, option, None)?;
                }
                "dismiss" => {
                    let id = args.get(2).ok_or_else(|| Error::permanent("The item ID is required."))?;
                    lore_core::sync::dismiss_item(&folder, id)?;
                }
                _ => return Err(Error::permanent("Use sync status, now, compare, resolve <item-id> <option>, or dismiss <item-id>.")),
            }
            println!("{}", serde_json::to_string_pretty(&serde_json::json!({"summary": lore_core::sync::summary(&folder), "state": lore_core::sync::state(&folder)}))?);
        }
        "doctor" => {
            let report = lore_core::diagnostics::inspect(&paths, &std::env::current_exe()?);
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        "project" => {
            let repository = lore_core::project::resolve_checked(&std::env::current_dir()?)?.ok_or_else(|| Error::permanent("No Git repository or valid project ID was found in the current folder."))?;
            println!("{}", serde_json::json!({"project": repository.name, "repository": repository.root}));
        }
        "index" => {
            let Some(_lock) = WorkerLock::try_acquire(&paths.lock())? else {
                return Err(Error::transient("The tray app or another worker is running."));
            };
            let settings = settings::load_or_init(&paths.settings(), worker::now())?;
            settings.validate()?;
            let qmd = Qmd::new(&settings, &paths.recorder_workdir(), fixed(settings.qmd_timeout_secs));
            if !qmd.available() { return Err(Error::permanent("qmd was not found. Run doctor to check the executable path.")); }
            qmd.ensure_index(&settings.knowledge_root())?;
            qmd.refresh()?;
            println!("This PC’s qmd index was refreshed.");
        }
        "hook" => {
            if std::env::var_os(RECORDER_MARK).is_some() {
                return Ok(());
            }
            let provider = provider(args.get(1))?;
            let mut input = String::new();
            std::io::stdin().read_to_string(&mut input)?;
            // Only a Codex event without a file path needs the settings, to find the session by its id.
            let sessions_dir = || Ok(PathBuf::from(settings::load_or_init(&paths.settings(), worker::now())?.codex_sessions_dir));
            match hook::read(provider, &input, sessions_dir)? {
                Hook::Session { transcript, ended } => println!("{}", Queue::open(&paths.database())?.enqueue(provider, &transcript, ended, worker::now())?),
                // Runs on every prompt, so it reads no settings and opens no database: it only
                // prints what the session is told.
                Hook::Prompt => println!("{}", hook::prompt_output(&std::env::current_exe()?)),
                Hook::Ignored => {}
            }
        }
        "mark" => {
            let kind = args.get(1).and_then(|value| Kind::parse(value)).ok_or_else(|| Error::permanent("Use mark <decision|correction|lesson> <sentence>."))?;
            let text = args[2..].join(" ").trim().to_string();
            if text.is_empty() {
                return Err(Error::permanent("Use mark <decision|correction|lesson> <sentence>."));
            }
            println!("{}", mark::line(&Mark { kind, text }));
        }
        "enqueue" => {
            let provider = provider(args.get(1))?;
            let transcript = args.get(2).ok_or_else(|| Error::permanent("A transcript path is required."))?;
            let (_, queue) = open(&paths)?;
            println!("{}", queue.enqueue(provider, Path::new(transcript), true, worker::now())?);
        }
        "candidates" => {
            let (_, queue) = open(&paths)?;
            match args.get(1).map(String::as_str) {
                None => {
                    for candidate in queue.waiting_candidates()? {
                        println!("{} [{}] {} ({}) {}", candidate.id, candidate.kind.as_str(), candidate.project, serde_json::to_string(&candidate.relation.kind)?.trim_matches('"'), candidate.text);
                    }
                }
                Some("approve") => {
                    let ids = if args.get(2).map(String::as_str) == Some("all") { queue.waiting_candidates()?.iter().map(|candidate| candidate.id).collect() } else { candidate_ids(&args[2..])? };
                    let batches = queue.approve_candidates(&ids, false, worker::now())?;
                    println!("Approved into {} apply batch(es). Run work, or open the tray app, to write them into the notes.", batches.len());
                }
                Some("discard") => println!("Discarded: {}", queue.discard_candidates(&candidate_ids(&args[2..])?, worker::now())?),
                Some(_) => return Err(Error::permanent("Use candidates, candidates approve <id>...|all, or candidates discard <id>...")),
            }
        }
        "scan" => {
            let (settings, queue) = open(&paths)?;
            println!("Sessions queued: {}", scan::scan(&queue, &settings, worker::now())?);
        }
        "work" => {
            let Some(_lock) = WorkerLock::try_acquire(&paths.lock())? else {
                return Err(Error::transient("The tray app or another worker is running."));
            };
            let (settings, queue) = open(&paths)?;
            settings.validate()?;
            let recorder = Recorders::from_settings(&settings, &paths.recorder_workdir(), fixed(settings.model_timeout_secs), |_| None);
            let qmd = Qmd::new(&settings, &paths.recorder_workdir(), fixed(settings.qmd_timeout_secs));
            let once = args.iter().any(|arg| arg == "--once");
            let memory = settings.knowledge_root();
            let mut ask = |prompt: &str, schema: &serde_json::Value, artifacts: &Path| recorder.request_json_reading(prompt, schema, artifacts, &memory);
            loop {
                worker::approve_overdue(&queue, &settings, worker::now())?;
                match worker::step(&queue, &settings, &paths, &mut ask, &qmd, &PrintReporter)? {
                    None => {
                        println!("No jobs to process.");
                        break;
                    }
                    Some(_) if once => break,
                    Some(_) => {}
                }
            }
        }
        "status" => {
            let (_, queue) = open(&paths)?;
            let counts = queue.counts(worker::now(), None)?;
            println!("{}", serde_json::to_string_pretty(&counts)?);
            for job in queue.jobs_with_status(&[Status::Failed, Status::Pending], 50)? {
                println!("{} {:?} {} {}", job.id, job.status, job.project.as_deref().unwrap_or("-"), job.last_error.as_deref().unwrap_or(""));
            }
        }
        "retry" => {
            let id = args.get(1).ok_or_else(|| Error::permanent("A job ID is required."))?;
            let (_, queue) = open(&paths)?;
            if !queue.retry(id, worker::now())? {
                return Err(Error::permanent("Only failed or skipped jobs can be requeued."));
            }
            println!("Requeued: {id}");
        }
        "discard" => {
            let id = args.get(1).ok_or_else(|| Error::permanent("A job ID is required."))?;
            let (settings, queue) = open(&paths)?;
            match worker::discard(&queue, &settings, &paths, id, worker::now())? {
                None => return Err(Error::permanent("Only failed jobs can be skipped.")),
                Some(Discarded::Batch { after_line, end_line }) => println!("Skipping transcript lines {}–{end_line}: {id}", after_line + 1),
                Some(Discarded::Session { reason }) => println!("Recording stopped for unreadable session: {id} ({reason})"),
            }
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
            println!("Connected {} hooks: {}", provider.as_str(), config.display());
            if let Some(backup) = backup {
                println!("Previous settings backup: {}", backup.display());
            }
        }
        "disconnect" => {
            let provider = provider(args.get(1))?;
            let config = hooks::config_path(provider);
            match hooks::unregister_at(&config, provider, worker::now())? {
                0 => println!("No Lore hooks connected for {}: {}", provider.as_str(), config.display()),
                removed => println!("Disconnected {removed} {} hooks: {} (previous settings backed up in the same folder)", provider.as_str(), config.display()),
            }
        }
        "cleanup" => {
            let (settings, queue) = open(&paths)?;
            settings.validate()?;
            let root = settings.knowledge_root();
            let Some(folder) = args.get(1).filter(|arg| !arg.starts_with("--")) else {
                for folder in cleanup::folders(&root, &queue.cleanup_checks()?)? {
                    let since = if folder.last.is_none() && folder.checked_at.is_none() { "never cleaned up".to_string() } else { format!("{} changed since the last cleanup", folder.fresh) };
                    println!("{} ({} {}, {since}){}", folder.name, folder.notes, if folder.notes == 1 { "note" } else { "notes" }, if folder.due { " *" } else { "" });
                }
                println!("* has {} or more notes to look at.", cleanup::DUE_NOTES);
                return Ok(());
            };
            if args.iter().any(|arg| arg == "--undo") {
                let Some(_lock) = WorkerLock::try_acquire(&paths.lock())? else {
                    return Err(Error::transient("The tray app or another worker is running. Undo the cleanup in its cleanup window."));
                };
                let qmd = Qmd::new(&settings, &paths.recorder_workdir(), fixed(settings.qmd_timeout_secs));
                let stale = worker::undo_cleanup(&queue, &settings, &qmd, folder)?;
                println!("{}", stale.unwrap_or_else(|| format!("Undid the last cleanup of {folder}.")));
                return Ok(());
            }
            let absorb = args.iter().zip(args.iter().skip(1)).filter(|(flag, _)| *flag == "--absorb").map(|(_, folder)| folder.clone()).collect();
            let target = cleanup::Target { folder: folder.clone(), absorb };
            let recorders = Recorders::from_settings(&settings, &paths.recorder_workdir(), fixed(settings.model_timeout_secs), |_| None);
            let mut ask = |prompt: &str, schema: &serde_json::Value, artifacts: &Path| recorders.request_json(prompt, schema, artifacts);
            let outcome = if args.iter().any(|arg| arg == "--dry-run") {
                let artifacts = paths.reviews().join(format!("cleanup-preview-{folder}-{}", worker::now()));
                let outcome = cleanup::run(&root, &target, cleanup::REQUEST_CHARS, &lore_core::review::local_date(), &artifacts, false, &mut ask)?;
                println!("Preview only; no file was changed. The model's answer is in {}", artifacts.display());
                outcome
            } else {
                let Some(_lock) = WorkerLock::try_acquire(&paths.lock())? else {
                    return Err(Error::transient("The tray app or another worker is running."));
                };
                let qmd = Qmd::new(&settings, &paths.recorder_workdir(), fixed(settings.qmd_timeout_secs));
                worker::clean(&queue, &settings, &paths, &qmd, &target, worker::now(), &mut ask)?
            };
            println!("{}", serde_json::to_string_pretty(&outcome)?);
        }
        _ => {
            println!("{USAGE}");
            if !command.is_empty() {
                return Err(Error::permanent(format!("Unknown command: {command}")));
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
            eprintln!("Error: {error}");
            ExitCode::FAILURE
        }
    }
}
