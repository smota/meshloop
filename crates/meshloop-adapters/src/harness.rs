//! CLI-subprocess `HarnessCapabilities` (runtime-design.md §2). `probe` shells out to the
//! configured executable's own version flag — real signal, never an assumed version string.
//! `invoke`'s argument shape is entirely config-supplied (`invoke_args_template`); Meshloop
//! ships no hardcoded per-harness CLI flag, per ADR 0003.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use meshloop_domain::capability::{Compatibility, HarnessError, HarnessProfile};
use meshloop_engine::agent::AgentSpec;
use meshloop_engine::ports::{HarnessCapabilities, HarnessHandle, HarnessOutcome};

pub struct CliHarnessConfig {
    pub name: String,
    pub executable: PathBuf,
    pub version_args: Vec<String>,
    /// Argument template for a dispatch; `{prompt_file}` is substituted with the path to a
    /// temp file holding the rendered prompt. Supplied by the user's configuration, never
    /// hardcoded per harness in this project.
    pub invoke_args_template: Vec<String>,
}

pub struct CliHarness {
    config: CliHarnessConfig,
    running: Mutex<HashMap<u32, (Child, Duration)>>,
}

impl CliHarness {
    pub fn new(config: CliHarnessConfig) -> Self {
        Self {
            config,
            running: Mutex::new(HashMap::new()),
        }
    }

    fn attempt_key(handle: &HarnessHandle) -> u32 {
        handle.attempt_id.0
    }
}

impl HarnessCapabilities for CliHarness {
    /// Real, read-only: resolves the configured executable and reads its actual reported
    /// version. Never invoked with a task prompt — see module docs.
    fn probe(&self) -> Result<HarnessProfile, HarnessError> {
        let output = Command::new(&self.config.executable)
            .args(&self.config.version_args)
            .stdin(Stdio::null())
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
                Ok(HarnessProfile {
                    harness: self.config.name.clone(),
                    version,
                    compatibility: Compatibility::Compatible,
                    supports_noninteractive: !self.config.invoke_args_template.is_empty(),
                    supports_structured_output: false,
                    supports_cancellation: true,
                })
            }
            Ok(out) => Ok(HarnessProfile {
                harness: self.config.name.clone(),
                version: String::from_utf8_lossy(&out.stderr).trim().to_string(),
                compatibility: Compatibility::Unsupported,
                supports_noninteractive: false,
                supports_structured_output: false,
                supports_cancellation: false,
            }),
            Err(e) => Err(HarnessError::ProcessFault {
                detail: e.to_string(),
            }),
        }
    }

    fn invoke(&self, spec: &AgentSpec) -> Result<HarnessHandle, HarnessError> {
        if self.config.invoke_args_template.is_empty() {
            return Err(HarnessError::Unsupported);
        }
        let prompt_file = spec
            .worktree_path
            .join(format!(".meshloop-prompt-{}", spec.attempt_id.0));
        fs::write(&prompt_file, &spec.prompt).map_err(|e| HarnessError::ProcessFault {
            detail: e.to_string(),
        })?;

        let args: Vec<String> = self
            .config
            .invoke_args_template
            .iter()
            .map(|a| {
                if a == "{prompt_file}" {
                    prompt_file.to_string_lossy().to_string()
                } else {
                    a.clone()
                }
            })
            .collect();

        let child = Command::new(&self.config.executable)
            .args(&args)
            .current_dir(&spec.worktree_path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| HarnessError::ProcessFault {
                detail: e.to_string(),
            })?;

        self.running
            .lock()
            .expect("harness registry mutex")
            .insert(spec.attempt_id.0, (child, spec.timeout));

        Ok(HarnessHandle {
            attempt_id: spec.attempt_id,
        })
    }

    fn cancel(&self, handle: &HarnessHandle) -> Result<(), HarnessError> {
        let mut registry = self.running.lock().expect("harness registry mutex");
        // Idempotent against an already-exited/never-tracked process, per ADR 0003.
        if let Some((mut child, _)) = registry.remove(&Self::attempt_key(handle)) {
            let _ = child.kill();
            let _ = child.wait();
        }
        Ok(())
    }

    fn collect(&self, handle: &HarnessHandle) -> Result<HarnessOutcome, HarnessError> {
        let (mut child, timeout) = self
            .running
            .lock()
            .expect("harness registry mutex")
            .remove(&Self::attempt_key(handle))
            .ok_or(HarnessError::Unsupported)?;

        // std::process has no built-in wait-with-timeout; poll bounded by the spec's own
        // timeout (captured at invoke time) rather than blocking indefinitely on a hung process.
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    use std::io::Read;
                    let mut stdout = String::new();
                    if let Some(mut out) = child.stdout.take() {
                        let _ = out.read_to_string(&mut stdout);
                    }
                    return Ok(HarnessOutcome {
                        exit_code: status.code().unwrap_or(-1),
                        output_redacted: stdout,
                        worktree_changed: status.success(),
                    });
                }
                Ok(None) => {
                    if start.elapsed() >= timeout {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(HarnessError::Timeout);
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => {
                    return Err(HarnessError::ProcessFault {
                        detail: e.to_string(),
                    });
                }
            }
        }
    }
}

// Contract tests against the fixture harness binary live in tests/harness_contract.rs —
// `CARGO_BIN_EXE_*` is only set for integration test targets, not unit tests in src/.
