use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
#[cfg(unix)]
use std::os::unix::process::CommandExt;

use crate::error::{Error, Result};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

pub const RECORDER_MARK: &str = "QMD_MEMORY_RECORDER";

pub type Limit = Arc<dyn Fn() -> Duration + Send + Sync>;

pub fn fixed(seconds: u64) -> Limit {
    Arc::new(move || Duration::from_secs(seconds))
}

pub struct Output {
    pub stdout: String,
    pub stderr: String,
    pub code: Option<i32>,
}

fn tail(text: &str, limit: usize) -> String {
    let count = text.chars().count();
    text.chars().skip(count.saturating_sub(limit)).collect()
}

fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .status();
    }
    #[cfg(unix)]
    // Every command starts its own process group; terminate the group so CLI
    // wrappers cannot leave a model process running after a timeout.
    unsafe { libc::kill(-(pid as i32), libc::SIGKILL); }
}

pub fn run(command: &[String], cwd: &Path, env: &[(&str, String)], stdin: Option<&str>, limit: &dyn Fn() -> Duration) -> Result<Output> {
    run_allow(command, cwd, env, stdin, limit, &[0])
}

pub fn run_allow(command: &[String], cwd: &Path, env: &[(&str, String)], stdin: Option<&str>, limit: &dyn Fn() -> Duration, accepted: &[i32]) -> Result<Output> {
    let (program, args) = command.split_first().ok_or_else(|| Error::transient("실행할 명령이 설정되지 않았습니다."))?;
    std::fs::create_dir_all(cwd)?;
    let mut builder = Command::new(program);
    builder.args(args).current_dir(cwd).stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Ok(path) = std::env::join_paths(crate::platform::search_paths()) {
        builder.env("PATH", path);
    }
    for (name, _) in std::env::vars_os().filter_map(|(name, value)| name.into_string().ok().map(|name| (name, value))) {
        if name.starts_with("ORCA_") || name == "CLAUDECODE" || name == "CLAUDE_CODE_ENTRYPOINT" {
            builder.env_remove(name);
        }
    }
    builder.env(RECORDER_MARK, "1").env("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1");
    for (name, value) in env {
        builder.env(name, value);
    }
    #[cfg(windows)]
    builder.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
    #[cfg(unix)]
    builder.process_group(0);
    let mut child = builder.spawn().map_err(|error| Error::transient(format!("{program} 실행 실패: {error}")))?;
    let writer = stdin.map(|text| {
        let mut pipe = child.stdin.take();
        let text = text.to_string();
        thread::spawn(move || {
            if let Some(pipe) = pipe.as_mut() {
                let _ = pipe.write_all(text.as_bytes());
            }
        })
    });
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    let stdout_reader = thread::spawn(move || {
        let mut buffer = Vec::new();
        if let Some(pipe) = stdout_pipe.as_mut() {
            let _ = pipe.read_to_end(&mut buffer);
        }
        buffer
    });
    let stderr_reader = thread::spawn(move || {
        let mut buffer = Vec::new();
        if let Some(pipe) = stderr_pipe.as_mut() {
            let _ = pipe.read_to_end(&mut buffer);
        }
        buffer
    });
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        let timeout = limit();
        if started.elapsed() >= timeout {
            kill_tree(child.id());
            let _ = child.wait();
            return Err(Error::transient(format!("{program} 실행이 {}초를 넘어 중단했습니다.", timeout.as_secs())));
        }
        thread::sleep(Duration::from_millis(100));
    };
    if let Some(writer) = writer {
        let _ = writer.join();
    }
    let stdout = String::from_utf8_lossy(&stdout_reader.join().unwrap_or_default()).to_string();
    let stderr = String::from_utf8_lossy(&stderr_reader.join().unwrap_or_default()).to_string();
    if !status.code().map(|code| accepted.contains(&code)).unwrap_or(false) {
        let detail = if stderr.trim().is_empty() { &stdout } else { &stderr };
        return Err(Error::transient(format!("{program} 실행 실패(종료 코드 {:?}): {}", status.code(), tail(detail.trim(), 1500))));
    }
    Ok(Output { stdout, stderr, code: status.code() })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn timeout_terminates_the_wrapper_and_its_child_process_group() {
        let dir = tempfile::tempdir().unwrap();
        let child_pid = dir.path().join("pid");
        // The child writes proof of survival only after its parent times out.
        // kill(-pgid) must reach both the shell wrapper and this child shell.
        let marker = dir.path().join("survived");
        let command = vec!["/bin/sh".into(), "-c".into(), "sh -c 'sleep 2; touch \"$1\"' child \"$1\" & echo $! > \"$2\"; wait".into(), "wrapper".into(), marker.to_string_lossy().to_string(), child_pid.to_string_lossy().to_string()];
        let error = run(&command, dir.path(), &[], None, &|| Duration::from_millis(500)).err().unwrap();
        assert!(error.message.contains("중단"));
        let pid: i32 = std::fs::read_to_string(child_pid).unwrap().trim().parse().unwrap();
        // Reap can be asynchronous; a killed child can briefly remain a zombie.
        // Waiting past the marker's deadline checks behavior rather than PID reuse.
        thread::sleep(Duration::from_millis(1800));
        assert!(!marker.exists(), "timed-out child {pid} kept running");
    }
}
