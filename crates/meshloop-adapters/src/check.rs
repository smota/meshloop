//! Optional configured check command, structured argv, run in the worktree.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use meshloop_domain::evidence::{AttemptId, CandidateRef, DeterministicEvidence};
use meshloop_domain::task_graph::TaskId;
use meshloop_engine::ports::{CheckError, CheckRunner};

pub struct CommandCheckRunner;

impl CheckRunner for CommandCheckRunner {
    fn run(
        &self,
        worktree: &Path,
        argv: &[String],
        timeout: Duration,
    ) -> Result<DeterministicEvidence, CheckError> {
        if argv.is_empty() {
            return Err(CheckError::Io("empty verify_command".into()));
        }
        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..])
            .current_dir(worktree)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child =
            crate::process::spawn_owned(cmd).map_err(|e| CheckError::Io(e.to_string()))?;
        // Drain both pipes concurrently. Waiting for exit before reading deadlocks
        // on Windows once either pipe exceeds the ~4 KiB buffer (rustc/cargo stderr).
        let stdout_handle = spawn_drain(child.stdout.take());
        let stderr_handle = spawn_drain(child.stderr.take());
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let stdout = stdout_handle.join().unwrap_or_default();
                    let stderr = stderr_handle.join().unwrap_or_default();
                    let combined = combine_stdio(&stdout, &stderr);
                    let code = status.code().unwrap_or(-1);
                    return Ok(DeterministicEvidence {
                        candidate: CandidateRef {
                            task_id: TaskId(0),
                            attempt_id: AttemptId(0),
                            revision: String::new(),
                        },
                        tool: argv[0].clone(),
                        tool_version: "n/a".into(),
                        exit_code: code,
                        output_redacted: crate::redact::redact(&combined),
                    });
                }
                Ok(None) => {
                    if start.elapsed() >= timeout {
                        child.kill_tree();
                        let _ = stdout_handle.join();
                        let _ = stderr_handle.join();
                        return Err(CheckError::Timeout);
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => {
                    child.kill_tree();
                    let _ = stdout_handle.join();
                    let _ = stderr_handle.join();
                    return Err(CheckError::Io(e.to_string()));
                }
            }
        }
    }
}

fn spawn_drain<R: Read + Send + 'static>(pipe: Option<R>) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut buf = String::new();
        if let Some(mut out) = pipe {
            let _ = out.read_to_string(&mut buf);
        }
        buf
    })
}

fn combine_stdio(stdout: &str, stderr: &str) -> String {
    match (stdout.is_empty(), stderr.is_empty()) {
        (true, true) => String::new(),
        (false, true) => stdout.to_string(),
        (true, false) => stderr.to_string(),
        (false, false) => {
            let mut out = String::with_capacity(stdout.len() + stderr.len() + 1);
            out.push_str(stdout);
            if !stdout.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(stderr);
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combine_stdio_keeps_stderr_when_stdout_empty() {
        assert_eq!(
            combine_stdio("", "error[E0308]: mismatched types\n"),
            "error[E0308]: mismatched types\n"
        );
    }

    #[test]
    fn combine_stdio_joins_both_streams() {
        assert_eq!(combine_stdio("out\n", "err\n"), "out\nerr\n");
        assert_eq!(combine_stdio("out", "err"), "out\nerr");
    }
}
