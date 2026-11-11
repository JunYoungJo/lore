//! One interactive terminal for setup. Output stays in memory and is bounded;
//! credentials/terminal transcripts are never written to Lore's state files.
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

const CAPACITY: usize = 512 * 1024;

pub struct Session {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    pid: Option<u32>,
    output: VecDeque<u8>,
    offset: u64,
    running: bool,
    exit_code: Option<u32>,
}

#[derive(Default)]
pub struct Terminal(pub Mutex<Option<Arc<Mutex<Session>>>>);

pub fn running(app: &AppHandle) -> bool {
    app.state::<Terminal>().0.lock().map(|slot| slot.as_ref().is_some_and(|session| session.lock().map(|s| s.running).unwrap_or(true))).unwrap_or(true)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Output {
    pub bytes: Vec<u8>,
    pub offset: u64,
    pub running: bool,
    pub exit_code: Option<u32>,
}

fn err(e: impl std::fmt::Display) -> String { e.to_string() }
fn size(cols: u16, rows: u16) -> PtySize {
    PtySize { cols: cols.clamp(20, 300), rows: rows.clamp(5, 100), pixel_width: 0, pixel_height: 0 }
}

pub fn start(app: &AppHandle, argv: &[String], cwd: &Path, env: &[(String, String)], cols: u16, rows: u16) -> Result<(), String> {
    let terminal = app.state::<Terminal>();
    let mut slot = terminal.0.lock().map_err(err)?;
    if app.state::<crate::state::AppState>().setup_busy() {
        return Err("Wait for memory preparation or the app update to finish before running a command.".into());
    }
    if slot.as_ref().is_some_and(|s| s.lock().map(|s| s.running).unwrap_or(true)) {
        return Err("A setup command is still running. Finish or stop it first.".into());
    }
    *slot = Some(spawn(argv, cwd, env, cols, rows)?);
    Ok(())
}

fn spawn(argv: &[String], cwd: &Path, env: &[(String, String)], cols: u16, rows: u16) -> Result<Arc<Mutex<Session>>, String> {
    let (program, args) = argv.split_first().ok_or("No command was prepared.")?;
    let pair = native_pty_system().openpty(size(cols, rows)).map_err(err)?;
    let mut command = CommandBuilder::new(program);
    command.args(args);
    command.cwd(cwd);
    command.env("TERM", "xterm-256color");
    command.env("LORE_RECORDER", "1");
    command.env("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1");
    command.env_remove("CLAUDECODE");
    for (key, _) in std::env::vars_os().filter(|(key, _)| key.to_string_lossy().starts_with("ORCA_")) { command.env_remove(key); }
    if let Ok(path) = std::env::join_paths(lore_core::platform::search_paths()) { command.env("PATH", path); }
    for (key, value) in env { command.env(key, value); }
    let mut child = pair.slave.spawn_command(command).map_err(err)?;
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().map_err(err)?;
    let session = Arc::new(Mutex::new(Session {
        writer: pair.master.take_writer().map_err(err)?, killer: child.clone_killer(), pid: child.process_id(),
        master: pair.master, output: VecDeque::new(), offset: 0, running: true, exit_code: None,
    }));
    let returned = session.clone();
    std::thread::spawn(move || {
        let mut chunk = [0; 8192];
        while let Ok(count) = reader.read(&mut chunk) {
            if count == 0 { break; }
            if let Ok(mut session) = session.lock() {
                session.output.extend(&chunk[..count]);
                let excess = session.output.len().saturating_sub(CAPACITY);
                session.output.drain(..excess);
                session.offset += excess as u64;
            }
        }
        let status = child.wait();
        if let Ok(mut session) = session.lock() {
            session.running = false;
            session.exit_code = status.ok().map(|s| s.exit_code());
        }
    });
    Ok(returned)
}

#[tauri::command]
pub fn setup_terminal_read(terminal: State<'_, Terminal>, after: u64) -> Result<Output, String> {
    let slot = terminal.0.lock().map_err(err)?;
    let Some(session) = slot.as_ref() else { return Ok(Output { bytes: vec![], offset: 0, running: false, exit_code: None }); };
    let session = session.lock().map_err(err)?;
    let skip = after.saturating_sub(session.offset).min(session.output.len() as u64) as usize;
    Ok(Output { bytes: session.output.iter().skip(skip).copied().collect(), offset: session.offset + session.output.len() as u64, running: session.running, exit_code: session.exit_code })
}

#[tauri::command]
pub fn setup_terminal_input(terminal: State<'_, Terminal>, data: String) -> Result<(), String> {
    if data.len() > 64 * 1024 { return Err("Terminal input is too large.".into()); }
    let slot = terminal.0.lock().map_err(err)?;
    let mut session = slot.as_ref().ok_or("No setup command is running.")?.lock().map_err(err)?;
    if !session.running { return Err("The setup command has finished.".into()); }
    session.writer.write_all(data.as_bytes()).map_err(err)?;
    session.writer.flush().map_err(err)
}

#[tauri::command]
pub fn setup_terminal_resize(terminal: State<'_, Terminal>, cols: u16, rows: u16) -> Result<(), String> {
    let slot = terminal.0.lock().map_err(err)?;
    if let Some(session) = slot.as_ref() { session.lock().map_err(err)?.master.resize(size(cols, rows)).map_err(err)?; }
    Ok(())
}

#[tauri::command]
pub fn setup_terminal_stop(terminal: State<'_, Terminal>) -> Result<(), String> {
    let slot = terminal.0.lock().map_err(err)?;
    if let Some(session) = slot.as_ref() {
        let mut session = session.lock().map_err(err)?;
        if session.running { session.stop().map_err(err)?; }
    }
    Ok(())
}

impl Drop for Terminal {
    fn drop(&mut self) {
        if let Ok(slot) = self.0.lock() {
            if let Some(session) = slot.as_ref() {
                if let Ok(mut session) = session.lock() { if session.running { let _ = session.stop(); } }
            }
        }
    }
}

impl Session {
    fn stop(&mut self) -> std::io::Result<()> {
        if let Some(pid) = self.pid { lore_core::run::kill_tree(pid); Ok(()) }
        else { self.killer.kill() }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait_for(session: &Arc<Mutex<Session>>, check: impl Fn(&Session) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if check(&session.lock().unwrap()) { return; }
            std::thread::sleep(Duration::from_millis(10));
        }
        session.lock().unwrap().stop().unwrap();
        panic!("PTY did not reach the expected state");
    }

    #[test]
    fn real_pty_accepts_input_and_preserves_unicode_output() {
        let session = spawn(&["/bin/sh".into(), "-c".into(), "test -t 0 || exit 5; printf 'ready'; IFS= read -r answer; printf '\\nreceived:%s\\n' \"$answer\"".into()], Path::new("/tmp"), &[], 90, 16).unwrap();
        wait_for(&session, |s| s.output.len() >= 5);
        {
            let mut session = session.lock().unwrap();
            session.writer.write_all("한글 memory\r".as_bytes()).unwrap();
            session.writer.flush().unwrap();
        }
        wait_for(&session, |s| !s.running);
        let session = session.lock().unwrap();
        assert_eq!(session.exit_code, Some(0));
        assert!(String::from_utf8(session.output.iter().copied().collect()).unwrap().contains("received:한글 memory"));
    }

    #[test]
    fn stop_terminates_a_running_command() {
        let session = spawn(&["/bin/sh".into(), "-c".into(), "printf ready; sleep 100".into()], Path::new("/tmp"), &[], 90, 16).unwrap();
        wait_for(&session, |s| s.output.len() >= 5);
        session.lock().unwrap().stop().unwrap();
        wait_for(&session, |s| !s.running);
        assert_ne!(session.lock().unwrap().exit_code, Some(0));
    }
}
