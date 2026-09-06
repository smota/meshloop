//! `HerdrSessionPort` (ADR 0005 / runtime-design.md §3): CLI-subprocess only in v1,
//! structured arguments, never shell-string interpolation. Argument construction is pure
//! and unit-tested here; a full live round-trip against a running `herdr` session is a
//! follow-up manual/authorized validation step per ADR 0005's verification section — this
//! pass does not spin up a persistent session on the host to verify it automatically.

use std::path::PathBuf;
use std::process::Command;

use meshloop_domain::capability::HarnessError;
use meshloop_engine::agent::AgentSpec;
use meshloop_engine::ports::{HerdrSessionPort, SessionHandle, SessionStatus};

pub struct HerdrCliAdapter {
    herdr_path: PathBuf,
    session_name: String,
}

impl HerdrCliAdapter {
    pub fn new(herdr_path: PathBuf, session_name: impl Into<String>) -> Self {
        Self {
            herdr_path,
            session_name: session_name.into(),
        }
    }

    /// `herdr --session <name> pane run <harness invocation...>` — structured args, no
    /// interpolation into a shell string.
    pub fn spawn_args(&self, harness_invocation: &[String]) -> Vec<String> {
        let mut args = vec![
            "--session".to_string(),
            self.session_name.clone(),
            "pane".to_string(),
            "run".to_string(),
        ];
        args.extend(harness_invocation.iter().cloned());
        args
    }

    pub fn close_pane_args(&self, pane_id: &str) -> Vec<String> {
        vec![
            "--session".to_string(),
            self.session_name.clone(),
            "pane".to_string(),
            "close".to_string(),
            pane_id.to_string(),
        ]
    }

    pub fn list_sessions_args(&self) -> Vec<String> {
        vec!["session".to_string(), "list".to_string()]
    }

    fn run(&self, args: &[String]) -> Result<String, HarnessError> {
        let output = Command::new(&self.herdr_path)
            .args(args)
            .output()
            .map_err(|e| HarnessError::ProcessFault {
                detail: e.to_string(),
            })?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            Err(HarnessError::ProcessFault {
                detail: String::from_utf8_lossy(&output.stderr).to_string(),
            })
        }
    }
}

impl HerdrSessionPort for HerdrCliAdapter {
    fn spawn(&self, spec: &AgentSpec) -> Result<SessionHandle, HarnessError> {
        let invocation = vec![spec.harness.clone()];
        let stdout = self.run(&self.spawn_args(&invocation))?;
        Ok(SessionHandle {
            id: stdout.trim().to_string(),
        })
    }

    fn status(&self, handle: &SessionHandle) -> Result<SessionStatus, HarnessError> {
        let listing = self.run(&self.list_sessions_args())?;
        if listing.contains(&handle.id) {
            Ok(SessionStatus::Running)
        } else {
            Ok(SessionStatus::Exited(0))
        }
    }

    fn cancel(&self, handle: &SessionHandle) -> Result<(), HarnessError> {
        // Idempotent: a pane already gone is not an error the caller needs to react to.
        let _ = self.run(&self.close_pane_args(&handle.id));
        Ok(())
    }

    fn cleanup(&self, handle: &SessionHandle) -> Result<(), HarnessError> {
        self.cancel(handle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_args_never_shell_interpolates_the_harness_command() {
        let adapter = HerdrCliAdapter::new(PathBuf::from("herdr"), "meshloop");
        let args = adapter.spawn_args(&["claude".to_string(), "--dangerous; rm -rf /".to_string()]);
        assert_eq!(
            args,
            vec![
                "--session",
                "meshloop",
                "pane",
                "run",
                "claude",
                "--dangerous; rm -rf /"
            ]
        );
        // The malicious-looking string is one argument element, never concatenated into a
        // shell string that a shell would re-interpret.
        assert_eq!(args.len(), 6);
    }

    #[test]
    fn close_pane_args_target_the_configured_session() {
        let adapter = HerdrCliAdapter::new(PathBuf::from("herdr"), "meshloop");
        assert_eq!(
            adapter.close_pane_args("pane-1"),
            vec!["--session", "meshloop", "pane", "close", "pane-1"]
        );
    }
}
