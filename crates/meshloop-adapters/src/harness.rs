//! CLI-subprocess `HarnessCapabilities` (runtime-design.md §2). `probe` shells out to the
//! configured executable's own version flag — real signal, never an assumed version string.
//! The probe is bounded by `probe_timeout` and runs in an owned process tree (ADR 0025), so
//! a harness whose version call opens an interactive UI is rejected instead of hanging.
//! `invoke`'s argument shape is entirely config-supplied (`invoke_args_template`); Meshloop
//! ships no hardcoded per-harness CLI flag, per ADR 0003.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use meshloop_domain::capability::{Compatibility, HarnessError, HarnessProfile};
use meshloop_engine::agent::AgentSpec;
use meshloop_engine::ports::{HarnessCapabilities, HarnessHandle, HarnessOutcome, LiveCheck};

pub struct CliHarnessConfig {
    pub name: String,
    pub executable: PathBuf,
    pub version_args: Vec<String>,
    /// Deadline for the version call; on expiry the process tree is killed and the harness
    /// is reported `Unsupported` with a timeout reason.
    pub probe_timeout: Duration,
    /// Argument template for a dispatch; `{prompt_file}` is substituted with the path to a
    /// temp file holding the rendered prompt. Supplied by the user's configuration, never
    /// hardcoded per harness in this project.
    pub invoke_args_template: Vec<String>,
}

pub struct CliHarness {
    config: CliHarnessConfig,
    running: Mutex<HashMap<u32, (crate::process::OwnedChild, Duration, Instant)>>,
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
        let mut cmd = Command::new(&self.config.executable);
        cmd.args(&self.config.version_args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child =
            crate::process::spawn_owned(cmd).map_err(|e| HarnessError::ProcessFault {
                detail: e.to_string(),
            })?;
        let stdout_handle = crate::check::spawn_drain(child.stdout.take());
        let stderr_handle = crate::check::spawn_drain(child.stderr.take());
        let start = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if start.elapsed() >= self.config.probe_timeout => {
                    child.kill_tree();
                    // Readers are not joined: a descendant outside the job could still hold
                    // a pipe open, and the probe must stay bounded.
                    return Ok(HarnessProfile::unsupported(
                        self.config.name.clone(),
                        format!(
                            "probe timed out after {}s running {} {}; the version call must exit without interaction",
                            self.config.probe_timeout.as_secs(),
                            self.config.executable.display(),
                            self.config.version_args.join(" ")
                        ),
                    ));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(e) => {
                    child.kill_tree();
                    return Err(HarnessError::ProcessFault {
                        detail: e.to_string(),
                    });
                }
            }
        };
        // Closing the job reaps any descendant on Windows; the bounded join covers a
        // descendant elsewhere that still holds a pipe open.
        drop(child);
        let deadline = start + self.config.probe_timeout;
        let stdout = join_until(stdout_handle, deadline);
        let stderr = join_until(stderr_handle, deadline);

        if !status.success() {
            let detail = crate::redact::redact(stderr.trim());
            let mut profile = HarnessProfile::unsupported(
                self.config.name.clone(),
                format!(
                    "version call exited with {}{}",
                    status.code().unwrap_or(-1),
                    if detail.is_empty() {
                        String::new()
                    } else {
                        format!(": {detail}")
                    }
                ),
            );
            profile.version = detail;
            return Ok(profile);
        }

        let supports_noninteractive = !self.config.invoke_args_template.is_empty();
        Ok(HarnessProfile {
            harness: self.config.name.clone(),
            version: stdout.trim().to_string(),
            compatibility: Compatibility::Compatible,
            supports_noninteractive,
            supports_structured_output: false,
            supports_cancellation: true,
            reason: (!supports_noninteractive).then(|| {
                "no invoke_args_template configured; cannot dispatch non-interactively".into()
            }),
        })
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
                } else if a == "{model_ref}" {
                    spec.model_ref.clone()
                } else {
                    a.clone()
                }
            })
            .collect();

        let mut cmd = Command::new(&self.config.executable);
        cmd.args(&args)
            .current_dir(&spec.worktree_path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = crate::process::spawn_owned(cmd).map_err(|e| HarnessError::ProcessFault {
            detail: e.to_string(),
        })?;
        let pid = child.id();

        self.running
            .lock()
            .map_err(|_| HarnessError::ProcessFault {
                detail: "harness registry mutex poisoned".into(),
            })?
            .insert(spec.attempt_id.0, (child, spec.timeout, Instant::now()));

        Ok(HarnessHandle {
            attempt_id: spec.attempt_id,
            pid: Some(pid),
            pane_id: None,
        })
    }

    fn session_live(&self, handle: &HarnessHandle) -> LiveCheck {
        match self.running.lock() {
            Ok(reg) if reg.contains_key(&Self::attempt_key(handle)) => LiveCheck::Live,
            Ok(_) => LiveCheck::Dead,
            Err(_) => LiveCheck::Ambiguous,
        }
    }

    fn process_image(&self) -> Option<String> {
        crate::process::process_image_name(&self.config.executable)
    }

    fn cancel(&self, handle: &HarnessHandle) -> Result<(), HarnessError> {
        let mut registry = self
            .running
            .lock()
            .map_err(|_| HarnessError::ProcessFault {
                detail: "harness registry mutex poisoned".into(),
            })?;
        // Idempotent against an already-exited/never-tracked process, per ADR 0003.
        if let Some((mut child, _, _)) = registry.remove(&Self::attempt_key(handle)) {
            child.kill_tree();
        } else if let Some(pid) = handle.pid {
            // Not ours: a previous Meshloop process started it and is gone (#30).
            crate::process::kill_orphaned_tree(pid, &self.config.executable);
        }
        Ok(())
    }

    fn try_collect(&self, handle: &HarnessHandle) -> Result<Option<HarnessOutcome>, HarnessError> {
        let key = Self::attempt_key(handle);
        let mut registry = self
            .running
            .lock()
            .map_err(|_| HarnessError::ProcessFault {
                detail: "harness registry mutex poisoned".into(),
            })?;

        let entry = registry.get_mut(&key).ok_or(HarnessError::ProcessFault {
            detail: "unknown harness handle".into(),
        })?;

        match entry.0.try_wait() {
            Ok(Some(status)) => {
                let (mut child, _, _) = registry.remove(&key).unwrap();
                use std::io::Read;
                let mut stdout = String::new();
                if let Some(mut out) = child.stdout.take() {
                    let _ = out.read_to_string(&mut stdout);
                }
                let mut stderr = String::new();
                if let Some(mut err) = child.stderr.take() {
                    let _ = err.read_to_string(&mut stderr);
                }
                if stderr.to_ascii_lowercase().contains("capacity exhausted") {
                    return Err(HarnessError::CapacityExhausted {
                        retry_after: Duration::from_secs(60),
                    });
                }
                Ok(Some(HarnessOutcome {
                    exit_code: status.code().unwrap_or(-1),
                    output_redacted: crate::redact::redact(&stdout),
                    worktree_changed: false,
                }))
            }
            Ok(None) => {
                let (_, timeout, start) = entry;
                if start.elapsed() >= *timeout {
                    let (mut child, _, _) = registry.remove(&key).unwrap();
                    child.kill_tree();
                    Err(HarnessError::Timeout)
                } else {
                    Ok(None)
                }
            }
            Err(e) => {
                if let Some((mut child, _, _)) = registry.remove(&key) {
                    child.kill_tree();
                }
                Err(HarnessError::ProcessFault {
                    detail: e.to_string(),
                })
            }
        }
    }

    fn collect(&self, handle: &HarnessHandle) -> Result<HarnessOutcome, HarnessError> {
        loop {
            match self.try_collect(handle)? {
                Some(outcome) => return Ok(outcome),
                None => std::thread::sleep(Duration::from_millis(20)),
            }
        }
    }
}

/// Joins a pipe reader, giving up with empty output once `deadline` passes.
fn join_until(handle: std::thread::JoinHandle<String>, deadline: Instant) -> String {
    while !handle.is_finished() {
        if Instant::now() >= deadline {
            return String::new();
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    handle.join().unwrap_or_default()
}

// Contract tests against the fixture harness binary live in tests/harness_contract.rs —
// `CARGO_BIN_EXE_*` is only set for integration test targets, not unit tests in src/.
