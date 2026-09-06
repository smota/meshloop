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
        let mut child = Command::new(&argv[0])
            .args(&argv[1..])
            .current_dir(worktree)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| CheckError::Io(e.to_string()))?;
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let mut stdout = String::new();
                    if let Some(mut out) = child.stdout.take() {
                        let _ = out.read_to_string(&mut stdout);
                    }
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
                        output_redacted: crate::redact::redact(&stdout),
                    });
                }
                Ok(None) => {
                    if start.elapsed() >= timeout {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(CheckError::Timeout);
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => return Err(CheckError::Io(e.to_string())),
            }
        }
    }
}
