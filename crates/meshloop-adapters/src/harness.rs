//! CLI-subprocess `HarnessCapabilities` (runtime-design.md §2). `probe` shells out to the
//! configured executable's own version flag — real signal, never an assumed version string.
//! The probe is bounded by `probe_timeout` and runs in an owned process tree (ADR 0025), so
//! a harness whose version call opens an interactive UI is rejected instead of hanging.
//! `invoke`'s argument shape is entirely config-supplied (`invoke_args_template`,
//! `prompt_via`); Meshloop ships no hardcoded per-harness CLI flag, per ADR 0003.

use crate::process::{OwnedChild, OwnedSpawnSpec, OwnedStdio};
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use meshloop_domain::capability::{Compatibility, HarnessError, HarnessProfile};
use meshloop_engine::agent::AgentSpec;
use meshloop_engine::ports::{HarnessCapabilities, HarnessHandle, HarnessOutcome, LiveCheck};

/// How the rendered prompt is delivered. The argv shape stays in config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PromptVia {
    /// Prompt text is only in the prompt file. Stdin is null. `{prompt}` stays literal.
    #[default]
    File,
    /// Also replace `{prompt}` with the prompt text. Stdin stays null.
    Arg,
    /// Write the prompt to the child stdin. `{prompt}` is replaced only if the template has it.
    Stdin,
}

impl PromptVia {
    /// `file`, `arg`, or `stdin`. Any other string is rejected by config load.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "file" => Some(Self::File),
            "arg" => Some(Self::Arg),
            "stdin" => Some(Self::Stdin),
            _ => None,
        }
    }
}

pub struct CliHarnessConfig {
    pub name: String,
    pub executable: PathBuf,
    pub version_args: Vec<String>,
    /// Deadline for the version call; on expiry the process tree is killed and the harness
    /// is reported `Unsupported` with a timeout reason.
    pub probe_timeout: Duration,
    /// Argument template for a dispatch. `{prompt_file}` and `{model_ref}` are replaced
    /// inside each argument. `{prompt}` is replaced for [`PromptVia::Arg`], and for
    /// [`PromptVia::Stdin`] when the template contains it. Unknown `{...}` tokens stay
    /// literal. Supplied by configuration, never hardcoded per harness.
    pub invoke_args_template: Vec<String>,
    pub prompt_via: PromptVia,
}

/// Joined executable plus argv, with one space before each argument.
const MAX_ARG_INVOCATION_BYTES: usize = 30_000;
const ARG_TOO_LONG: &str = "the prompt is too long for an argument";

fn render_invoke_args(
    template: &[String],
    via: PromptVia,
    prompt_file: &Path,
    model_ref: &str,
    prompt: &str,
) -> Vec<String> {
    let prompt_path = prompt_file.to_string_lossy();
    // File mode must not copy the prompt text into argv. Arg always expands
    // `{prompt}`. Stdin expands it only when the template already contains it.
    let prompt_text = match via {
        PromptVia::File => None,
        PromptVia::Arg | PromptVia::Stdin => Some(prompt),
    };
    template
        .iter()
        .map(|arg| substitute_arg(arg, &prompt_path, model_ref, prompt_text))
        .collect()
}

/// Replaces known placeholders inside one argument. The replacement is not scanned again.
fn substitute_arg(
    template: &str,
    prompt_file: &str,
    model_ref: &str,
    prompt: Option<&str>,
) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        if let Some(end) = after.find('}') {
            let token = &after[..=end];
            let replacement = match token {
                "{prompt_file}" => Some(prompt_file),
                "{model_ref}" => Some(model_ref),
                "{prompt}" => prompt,
                _ => None,
            };
            match replacement {
                Some(value) => out.push_str(value),
                None => out.push_str(token),
            }
            rest = &after[end + 1..];
        } else {
            out.push_str(after);
            return out;
        }
    }
    out.push_str(rest);
    out
}

fn joined_invocation_len(executable: &Path, args: &[String]) -> usize {
    let mut len = executable.to_string_lossy().len();
    for arg in args {
        len = len.saturating_add(1 + arg.len());
    }
    len
}

fn write_prompt_stdin(child: &mut OwnedChild, prompt: &[u8]) -> Result<(), HarnessError> {
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| HarnessError::ProcessFault {
            detail: "harness stdin was not piped".into(),
        })?;
    stdin
        .write_all(prompt)
        .map_err(|e| HarnessError::ProcessFault {
            detail: e.to_string(),
        })?;
    // Dropping the writer closes the pipe so the child observes EOF.
    drop(stdin);
    Ok(())
}

const CAPTURE_SLOTS: usize = 16;
const STREAM_LIMIT: usize = 4 * 1024 * 1024;
static CAPTURES: AtomicUsize = AtomicUsize::new(0);

fn capture_fault() -> HarnessError {
    HarnessError::ProcessFault {
        detail: "harness-output-capture-failed".into(),
    }
}
struct CapturePermit;
impl CapturePermit {
    fn reserve() -> Result<Arc<Self>, HarnessError> {
        CAPTURES
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < CAPTURE_SLOTS).then_some(n + 1)
            })
            .map_err(|_| HarnessError::ProcessFault {
                detail: "harness-output-capacity-blocked".into(),
            })?;
        Ok(Arc::new(Self))
    }
}
impl Drop for CapturePermit {
    fn drop(&mut self) {
        CAPTURES.fetch_sub(1, Ordering::AcqRel);
    }
}
#[derive(Clone, Copy)]
#[repr(usize)]
enum CaptureFailure {
    Overflow = 1,
    Read = 2,
    Utf8 = 3,
}
impl CaptureFailure {
    fn error(self) -> HarnessError {
        HarnessError::ProcessFault {
            detail: match self {
                Self::Overflow => "harness-output-overflow",
                Self::Read => "harness-output-read-failed",
                Self::Utf8 => "harness-output-invalid-utf8",
            }
            .into(),
        }
    }
}
fn recorded_failure(reason: &AtomicUsize) -> Option<HarnessError> {
    match reason.load(Ordering::Acquire) {
        1 => Some(CaptureFailure::Overflow.error()),
        2 => Some(CaptureFailure::Read.error()),
        3 => Some(CaptureFailure::Utf8.error()),
        _ => None,
    }
}
type Reader = std::thread::JoinHandle<Result<String, CaptureFailure>>;
struct Capture {
    _permit: Arc<CapturePermit>,
    stdout: Reader,
    stderr: Reader,
    failed: Arc<AtomicUsize>,
}
fn capture_reader<R: Read + Send + 'static>(
    pipe: Option<R>,
    permit: Arc<CapturePermit>,
    failed: Arc<AtomicUsize>,
) -> Result<Reader, HarnessError> {
    std::thread::Builder::new()
        .name("meshloop-output".into())
        .spawn(move || {
            let _permit = permit;
            let result = (|| {
                let mut bytes = Vec::new();
                let mut pipe = pipe.ok_or(CaptureFailure::Read)?;
                let mut chunk = [0u8; 8192];
                loop {
                    let n = pipe.read(&mut chunk).map_err(|_| CaptureFailure::Read)?;
                    if n == 0 {
                        break;
                    }
                    if bytes.len() + n > STREAM_LIMIT {
                        return Err(CaptureFailure::Overflow);
                    }
                    bytes.extend_from_slice(&chunk[..n]);
                }
                String::from_utf8(bytes).map_err(|_| CaptureFailure::Utf8)
            })();
            if let Err(reason) = &result {
                failed.store(*reason as usize, Ordering::Release);
            }
            result
        })
        .map_err(|_| capture_fault())
}
impl Capture {
    fn start(
        child: &mut crate::process::OwnedChild,
        permit: Arc<CapturePermit>,
    ) -> Result<Self, HarnessError> {
        let failed = Arc::new(AtomicUsize::new(0));
        let stdout = capture_reader(child.stdout.take(), permit.clone(), failed.clone());
        let stderr = capture_reader(child.stderr.take(), permit.clone(), failed.clone());
        match (stdout, stderr) {
            (Ok(stdout), Ok(stderr)) => Ok(Self {
                _permit: permit,
                stdout,
                stderr,
                failed,
            }),
            _ => {
                child.kill_tree();
                Err(capture_fault())
            }
        }
    }
    fn finished(&self) -> bool {
        self.stdout.is_finished() && self.stderr.is_finished()
    }
    fn outcome(self) -> Result<(String, String), HarnessError> {
        // Call only after finished(); no pipe read or unfinished join on the caller.
        let stdout = self
            .stdout
            .join()
            .map_err(|_| HarnessError::ProcessFault {
                detail: "harness-output-reader-failed".into(),
            })?
            .map_err(CaptureFailure::error)?;
        let stderr = self
            .stderr
            .join()
            .map_err(|_| HarnessError::ProcessFault {
                detail: "harness-output-reader-failed".into(),
            })?
            .map_err(CaptureFailure::error)?;
        Ok((stdout, stderr))
    }
}
struct Running {
    cancellation_attempted: bool,
    child: Option<crate::process::OwnedChild>,
    capture: Capture,
    timeout: Duration,
    start: Instant,
    status: Option<std::process::ExitStatus>,
}

struct Slot {
    permit: Arc<CapturePermit>,
    running: Option<Running>,
}

pub struct CliHarness {
    config: CliHarnessConfig,
    running: Mutex<HashMap<u32, Slot>>,
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
    fn with_retained_attempt(
        &self,
        handle: &HarnessHandle,
        operation: impl FnOnce(&mut Running) -> Result<(), HarnessError>,
    ) -> Result<(), HarnessError> {
        let key = Self::attempt_key(handle);
        let (permit, mut running) = {
            let mut registry = self.running.lock().map_err(|_| capture_fault())?;
            let slot = registry.get_mut(&key).ok_or(HarnessError::Unsupported)?;
            let running = slot.running.as_ref().ok_or(HarnessError::Unsupported)?;
            if handle.pid != running.child.as_ref().map(|child| child.id())
                || handle.pane_id.is_some()
            {
                return Err(HarnessError::Unsupported);
            }
            (
                slot.permit.clone(),
                slot.running.take().ok_or(HarnessError::Unsupported)?,
            )
        };
        // Ownership remains reserved by the identical Arc token, while native
        // waiting never holds the registry mutex shared by unrelated attempts.
        let result = operation(&mut running);
        let mut registry = self.running.lock().map_err(|_| capture_fault())?;
        if let Some(slot) = registry.get_mut(&key)
            && slot.running.is_none()
            && Arc::ptr_eq(&slot.permit, &permit)
        {
            slot.running = Some(running);
            return result;
        }
        // Another caller removed/replaced the reservation. Never resurrect it.
        // The operation already attempted POSIX cleanup and may have reaped its
        // leader: do not issue another PID/group signal after possible reuse.
        // Windows still owns its private kill-on-close job until running drops.
        drop(registry);
        drop(running);
        Err(capture_fault())
    }
}

impl CliHarness {
    /// Whether a dispatch substitutes `{model_ref}` into its arguments: rendering the
    /// template with two different refs gives different arguments. Exact for the
    /// renderer, including tokens it leaves literal.
    pub fn template_passes_model_ref(template: &[String]) -> bool {
        let render = |model_ref: &str| {
            render_invoke_args(template, PromptVia::File, Path::new(""), model_ref, "")
        };
        render("a") != render("b")
    }
}

impl HarnessCapabilities for CliHarness {
    fn supports_nonblocking_collection(&self) -> bool {
        true
    }

    fn passes_model_ref(&self) -> bool {
        Self::template_passes_model_ref(&self.config.invoke_args_template)
    }

    fn cancel_owned(&self, handle: &HarnessHandle, timeout: Duration) -> Result<(), HarnessError> {
        self.with_retained_attempt(handle, |running| {
            if running.cancellation_attempted {
                return Err(HarnessError::Unsupported);
            }
            running.cancellation_attempted = true;
            let child = running.child.as_mut().ok_or(HarnessError::Unsupported)?;
            #[cfg(not(windows))]
            {
                let _ = timeout;
                // Retained, unreaped owner only. Best effort is never Applied.
                if running.status.is_none() {
                    child.kill_tree();
                    running.status = child.try_wait().map_err(|_| capture_fault())?;
                }
                if running.status.is_some() {
                    // Once reaped, its numeric PID/group may be reused. No
                    // later capture timeout may signal that identity again.
                    running.child.take();
                }
                Err(HarnessError::Unsupported)
            }
            #[cfg(windows)]
            {
                let proof = child.terminate_and_observe(timeout).map_err(|_| {
                    HarnessError::ProcessFault {
                        detail: "owned-tree-quiescence-unproved".into(),
                    }
                })?;
                if !proof.job_empty {
                    return Err(HarnessError::Unsupported);
                }
                running.status = Some(proof.direct_child);
                Ok(())
            }
        })
    }

    /// Real, read-only: resolves the configured executable and reads its actual reported
    /// version. Never invoked with a task prompt — see module docs.
    fn probe(&self) -> Result<HarnessProfile, HarnessError> {
        let permit = CapturePermit::reserve()?;
        let mut cmd = OwnedSpawnSpec::new(&self.config.executable);
        cmd.args(&self.config.version_args)
            .stdin(OwnedStdio::Null)
            .stdout(OwnedStdio::Piped)
            .stderr(OwnedStdio::Piped);
        let mut child = crate::process::spawn_owned(cmd).map_err(|_| capture_fault())?;
        let capture = Capture::start(&mut child, permit)?;
        let start = Instant::now();
        let status = loop {
            if let Some(error) = recorded_failure(&capture.failed) {
                child.kill_tree();
                return Err(error);
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if start.elapsed() >= self.config.probe_timeout => {
                    child.kill_tree();
                    return Ok(HarnessProfile::unsupported(
                        self.config.name.clone(),
                        "probe timed out",
                    ));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(_) => {
                    child.kill_tree();
                    return Err(capture_fault());
                }
            }
        };
        drop(child);
        while !capture.finished() {
            if start.elapsed() >= self.config.probe_timeout {
                return Err(HarnessError::ProcessFault {
                    detail: "harness-output-eof-timeout".into(),
                });
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let (stdout, stderr) = capture.outcome()?;

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
        let args = render_invoke_args(
            &self.config.invoke_args_template,
            self.config.prompt_via,
            &prompt_file,
            &spec.model_ref,
            &spec.prompt,
        );
        if self.config.prompt_via == PromptVia::Arg
            && joined_invocation_len(&self.config.executable, &args) > MAX_ARG_INVOCATION_BYTES
        {
            return Err(HarnessError::ProcessFault {
                detail: ARG_TOO_LONG.into(),
            });
        }
        let permit = CapturePermit::reserve()?;
        {
            let mut registry = self.running.lock().map_err(|_| capture_fault())?;
            match registry.entry(spec.attempt_id.0) {
                std::collections::hash_map::Entry::Vacant(slot) => {
                    slot.insert(Slot {
                        permit: permit.clone(),
                        running: None,
                    });
                }
                std::collections::hash_map::Entry::Occupied(_) => return Err(capture_fault()),
            }
        }
        let launch = (|| {
            fs::write(&prompt_file, &spec.prompt).map_err(|e| HarnessError::ProcessFault {
                detail: e.to_string(),
            })?;

            let mut cmd = OwnedSpawnSpec::new(&self.config.executable);
            cmd.args(&args)
                .current_dir(&spec.worktree_path)
                .stdin(if self.config.prompt_via == PromptVia::Stdin {
                    OwnedStdio::Piped
                } else {
                    OwnedStdio::Null
                })
                .stdout(OwnedStdio::Piped)
                .stderr(OwnedStdio::Piped);
            let mut child =
                crate::process::spawn_owned(cmd).map_err(|e| HarnessError::ProcessFault {
                    detail: e.to_string(),
                })?;
            let capture = Capture::start(&mut child, permit.clone())?;
            if self.config.prompt_via == PromptVia::Stdin
                && let Err(error) = write_prompt_stdin(&mut child, spec.prompt.as_bytes())
            {
                child.kill_tree();
                return Err(error);
            }

            Ok(Running {
                cancellation_attempted: false,
                child: Some(child),
                capture,
                timeout: spec.timeout,
                start: Instant::now(),
                status: None,
            })
        })();
        let mut entry: Running = match launch {
            Ok(entry) => entry,
            Err(error) => {
                if let Ok(mut registry) = self.running.lock()
                    && registry.get(&spec.attempt_id.0).is_some_and(|slot| {
                        slot.running.is_none() && Arc::ptr_eq(&slot.permit, &permit)
                    })
                {
                    registry.remove(&spec.attempt_id.0);
                }
                return Err(error);
            }
        };
        let pid = entry.child.as_ref().map(|child| child.id());
        let mut registry = match self.running.lock() {
            Ok(registry) => registry,
            Err(_) => {
                if let Some(mut child) = entry.child.take() {
                    child.kill_tree();
                }
                return Err(capture_fault());
            }
        };
        if !registry
            .get(&spec.attempt_id.0)
            .is_some_and(|slot| slot.running.is_none() && Arc::ptr_eq(&slot.permit, &permit))
        {
            drop(registry);
            if let Some(mut child) = entry.child.take() {
                child.kill_tree();
            }
            return Err(capture_fault());
        }
        registry.insert(
            spec.attempt_id.0,
            Slot {
                permit,
                running: Some(entry),
            },
        );

        Ok(HarnessHandle {
            attempt_id: spec.attempt_id,
            pid,
            pane_id: None,
        })
    }

    fn session_live(&self, handle: &HarnessHandle) -> LiveCheck {
        match self.running.lock() {
            Ok(reg)
                if reg
                    .get(&Self::attempt_key(handle))
                    .is_some_and(|slot| slot.running.is_some()) =>
            {
                LiveCheck::Live
            }
            Ok(reg) if reg.contains_key(&Self::attempt_key(handle)) => LiveCheck::Ambiguous,
            Ok(_) => LiveCheck::Dead,
            Err(_) => LiveCheck::Ambiguous,
        }
    }

    fn process_image(&self) -> Option<String> {
        crate::process::process_image_name(&self.config.executable)
    }

    fn cancel(&self, handle: &HarnessHandle) -> Result<(), HarnessError> {
        let entry = self
            .running
            .lock()
            .map_err(|_| capture_fault())?
            .remove(&Self::attempt_key(handle));
        // Best effort legacy cancellation: this is not a quiescence acknowledgement.
        if let Some(entry) = entry {
            let Some(mut entry) = entry.running else {
                return Ok(());
            };
            if let Some(mut child) = entry.child.take() {
                child.kill_tree();
            }
            // Unfinished readers retain their process-wide permit even after this state drops.
        } else if let Some(pid) = handle.pid {
            crate::process::kill_orphaned_tree(pid, &self.config.executable);
        }
        Ok(())
    }

    fn try_collect(&self, handle: &HarnessHandle) -> Result<Option<HarnessOutcome>, HarnessError> {
        let key = Self::attempt_key(handle);
        let mut registry = self.running.lock().map_err(|_| capture_fault())?;
        let entry = registry.get_mut(&key).ok_or_else(capture_fault)?;
        let Some(entry) = entry.running.as_mut() else {
            return Ok(None);
        };
        if let Some(error) = recorded_failure(&entry.capture.failed) {
            let mut entry = registry
                .remove(&key)
                .and_then(|slot| slot.running)
                .ok_or_else(capture_fault)?;
            drop(registry);
            if let Some(mut child) = entry.child.take() {
                child.kill_tree();
            }
            return Err(error);
        }
        if entry.status.is_none()
            && let Some(child) = entry.child.as_mut()
        {
            match child.try_wait() {
                Ok(Some(status)) => {
                    entry.status = Some(status);
                }
                Ok(None) => {}
                Err(_) => {
                    let mut entry = registry
                        .remove(&key)
                        .and_then(|slot| slot.running)
                        .ok_or_else(capture_fault)?;
                    drop(registry);
                    if let Some(mut child) = entry.child.take() {
                        child.kill_tree();
                    }
                    return Err(capture_fault());
                }
            }
        }

        if entry.status.is_some() && entry.capture.finished() {
            let entry = registry
                .remove(&key)
                .and_then(|slot| slot.running)
                .ok_or_else(capture_fault)?;
            drop(registry);
            let status = entry.status.ok_or_else(capture_fault)?;
            drop(entry.child);
            let (stdout, stderr) = entry.capture.outcome()?;
            if stderr.to_ascii_lowercase().contains("capacity exhausted") {
                return Err(HarnessError::CapacityExhausted {
                    retry_after: Duration::from_secs(60),
                });
            }
            return Ok(Some(HarnessOutcome {
                exit_code: status.code().unwrap_or(-1),
                output_redacted: crate::redact::redact(&stdout),
                stderr_redacted: crate::redact::redact(&stderr),
                worktree_changed: false,
            }));
        }
        if entry.start.elapsed() >= entry.timeout {
            let mut entry = registry
                .remove(&key)
                .and_then(|slot| slot.running)
                .ok_or_else(capture_fault)?;
            drop(registry);
            if let Some(mut child) = entry.child.take() {
                child.kill_tree();
            }
            return Err(if entry.status.is_some() {
                HarnessError::ProcessFault {
                    detail: "harness-output-eof-timeout".into(),
                }
            } else {
                HarnessError::Timeout
            });
        }
        Ok(None)
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

// Contract tests against the fixture harness binary live in tests/harness_contract.rs —
// `CARGO_BIN_EXE_*` is only set for integration test targets, not unit tests in src/.

#[cfg(test)]
mod capture_tests {
    use super::*;
    use crate::process::TestCommandExt;

    /// #139: `model_passed` is true exactly when a dispatch substitutes `{model_ref}`.
    #[test]
    fn model_ref_is_passed_only_when_the_template_substitutes_it() {
        let passes = |args: &[&str]| {
            CliHarness::template_passes_model_ref(
                &args.iter().map(|a| a.to_string()).collect::<Vec<_>>(),
            )
        };
        assert!(passes(&["exec", "--model", "{model_ref}", "-"]));
        assert!(passes(&["--model={model_ref}"]));
        assert!(!passes(&["--prompt-file", "{prompt_file}"]));
        assert!(!passes(&[]));
        // Tokens the renderer leaves literal do not pass the model.
        assert!(!passes(&["{model_ref"]));
        assert!(!passes(&["{x{model_ref}"]));
        assert!(!passes(&["{model}"]));
        let harness = CliHarness::new(CliHarnessConfig {
            name: "test".into(),
            executable: PathBuf::from("missing-fixture"),
            version_args: vec![],
            probe_timeout: Duration::from_secs(1),
            invoke_args_template: vec!["-m".into(), "{model_ref}".into()],
            prompt_via: PromptVia::Stdin,
        });
        assert!(harness.passes_model_ref());
        assert_eq!(
            harness.dispatch_mode(),
            meshloop_domain::advisory::DispatchMode::Subprocess
        );
    }
    #[test]
    fn inflight_owned_cancel_releases_registry_and_never_resurrects_removed_token() {
        let harness = CliHarness::new(CliHarnessConfig {
            name: "test".into(),
            executable: PathBuf::from("missing-fixture"),
            version_args: vec![],
            probe_timeout: Duration::from_secs(1),
            invoke_args_template: vec!["--noop".into()],
            prompt_via: PromptVia::File,
        });
        let permit = CapturePermit::reserve().unwrap();
        let failed = Arc::new(AtomicUsize::new(0));
        let capture = Capture {
            _permit: permit.clone(),
            stdout: capture_reader(
                Some(std::io::Cursor::new(Vec::<u8>::new())),
                permit.clone(),
                failed.clone(),
            )
            .unwrap(),
            stderr: capture_reader(
                Some(std::io::Cursor::new(Vec::<u8>::new())),
                permit.clone(),
                failed.clone(),
            )
            .unwrap(),
            failed,
        };
        harness.running.lock().unwrap().insert(
            1,
            Slot {
                permit,
                running: Some(Running {
                    cancellation_attempted: false,
                    child: None,
                    capture,
                    timeout: Duration::from_secs(1),
                    start: Instant::now(),
                    status: None,
                }),
            },
        );
        let handle = HarnessHandle {
            attempt_id: meshloop_domain::evidence::AttemptId(1),
            pid: None,
            pane_id: None,
        };
        let operations = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            let (started_tx, started_rx) = std::sync::mpsc::channel();
            let (finish_tx, finish_rx) = std::sync::mpsc::channel();
            let harness_ref = &harness;
            let handle_ref = &handle;
            let operations_ref = &operations;
            let cancel = scope.spawn(move || {
                harness_ref.with_retained_attempt(handle_ref, |_| {
                    operations_ref.fetch_add(1, Ordering::SeqCst);
                    started_tx.send(()).unwrap();
                    finish_rx.recv().unwrap();
                    Ok(())
                })
            });
            started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            let start = Instant::now();
            assert!(harness.try_collect(&handle).unwrap().is_none());
            // A new attempt reaches admission/launch while cancellation waits.
            let dir = std::env::temp_dir()
                .join(format!("meshloop-cancel-inflight-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let spec = AgentSpec {
                task_id: meshloop_domain::task_graph::TaskId(2),
                attempt_id: meshloop_domain::evidence::AttemptId(2),
                harness: "test".into(),
                model_ref: "test".into(),
                worktree_path: dir.clone(),
                prompt: "fixture".into(),
                timeout: Duration::from_secs(1),
            };
            assert!(harness.invoke(&spec).is_err());
            assert!(start.elapsed() < Duration::from_secs(1));
            harness.cancel(&handle).unwrap(); // remove in-flight reservation
            let replacement = CapturePermit::reserve().unwrap();
            harness.running.lock().unwrap().insert(
                1,
                Slot {
                    permit: replacement.clone(),
                    running: None,
                },
            );
            finish_tx.send(()).unwrap();
            assert!(cancel.join().unwrap().is_err());
            let registry = harness.running.lock().unwrap();
            assert!(Arc::ptr_eq(&registry[&1].permit, &replacement));
            assert!(registry[&1].running.is_none());
            assert_eq!(operations.load(Ordering::SeqCst), 1);
            std::fs::remove_dir_all(dir).unwrap();
        });
    }
    #[test]
    fn bounded_capture_global_permits_survive_drop_and_recreation() {
        const CHILD: &str = "MESHLOOP_CAPTURE_POOL_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "harness::capture_tests::bounded_capture_global_permits_survive_drop_and_recreation", "--nocapture"])
                .env(CHILD, "1").guarded_output().unwrap();
            assert!(
                result.status.success(),
                "{}{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            return;
        }
        struct Blocked(std::sync::mpsc::Receiver<()>);
        impl Read for Blocked {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                self.0.recv().unwrap();
                Ok(0)
            }
        }
        let mut release = Vec::new();
        for _ in 0..CAPTURE_SLOTS {
            let permit = CapturePermit::reserve().unwrap();
            let (tx, rx) = std::sync::mpsc::channel();
            let reader = capture_reader(
                Some(Blocked(rx)),
                permit.clone(),
                Arc::new(AtomicUsize::new(0)),
            )
            .unwrap();
            release.push(tx);
            drop(reader); // Captures/harnesses may drop; worker still owns admission.
            drop(permit);
            drop(CliHarness::new(CliHarnessConfig {
                name: "test".into(),
                executable: PathBuf::new(),
                version_args: vec![],
                probe_timeout: Duration::from_secs(1),
                invoke_args_template: vec![],
                prompt_via: PromptVia::File,
            }));
        }
        assert!(CapturePermit::reserve().is_err());
        let dir =
            std::env::temp_dir().join(format!("meshloop-capture-admission-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let harness = CliHarness::new(CliHarnessConfig {
            name: "blocked".into(),
            executable: PathBuf::from("must-not-spawn"),
            version_args: vec![],
            probe_timeout: Duration::from_secs(1),
            invoke_args_template: vec!["--noop".into()],
            prompt_via: PromptVia::File,
        });
        let spec = AgentSpec {
            task_id: meshloop_domain::task_graph::TaskId(1),
            attempt_id: meshloop_domain::evidence::AttemptId(1),
            harness: "blocked".into(),
            model_ref: "test".into(),
            worktree_path: dir.clone(),
            prompt: "must not write".into(),
            timeout: Duration::from_secs(1),
        };
        assert!(
            matches!(harness.invoke(&spec), Err(HarnessError::ProcessFault { detail }) if detail == "harness-output-capacity-blocked")
        );
        assert!(!dir.join(".meshloop-prompt-1").exists());
        std::fs::remove_dir_all(dir).unwrap();
        for tx in release {
            tx.send(()).unwrap();
        }
        let start = Instant::now();
        while CAPTURES.load(Ordering::Acquire) != 0 {
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::yield_now();
        }
        assert!(CapturePermit::reserve().is_ok());
    }
    #[test]
    fn bounded_capture_read_errors_are_explicit_faults() {
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("private diagnostic"))
            }
        }
        let permit = CapturePermit::reserve().unwrap();
        let failed = Arc::new(AtomicUsize::new(0));
        let reader = capture_reader(Some(Broken), permit, failed.clone()).unwrap();
        assert!(reader.join().unwrap().is_err());
        assert_eq!(
            failed.load(Ordering::Acquire),
            CaptureFailure::Read as usize
        );
        assert!(
            matches!(recorded_failure(&failed), Some(HarnessError::ProcessFault { detail }) if detail == "harness-output-read-failed")
        );
    }
    #[test]
    fn bounded_capture_invalid_utf8_is_explicit_fault() {
        let permit = CapturePermit::reserve().unwrap();
        let failed = Arc::new(AtomicUsize::new(0));
        let reader = capture_reader(
            Some(std::io::Cursor::new(vec![0xff])),
            permit,
            failed.clone(),
        )
        .unwrap();
        assert!(matches!(reader.join().unwrap(), Err(CaptureFailure::Utf8)));
        assert!(
            matches!(recorded_failure(&failed), Some(HarnessError::ProcessFault { detail }) if detail == "harness-output-invalid-utf8")
        );
    }
}
