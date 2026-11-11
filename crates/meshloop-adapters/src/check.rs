//! Optional configured check command, structured argv, run in the worktree.
//!
//! [`CommandCheckRunner`] runs `verify_command` directly on the host with the operator's
//! authority. [`ContainedCheckRunner`] runs it behind an operator-configured argv prefix
//! (`[verify] runner`, issue #181), such as `adapters/sandbox/meshloop-verify-sandbox`,
//! so a check that executes worker-written repository code runs inside a sandbox.
//! [`VerifyChecks`] is the one the CLI composes: host unless a runner is configured.

use crate::process::{OwnedSpawnSpec, OwnedStdio};
use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::Path;
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
        let args: Vec<OsString> = argv[1..].iter().map(OsString::from).collect();
        run_owned(worktree, OsStr::new(&argv[0]), &args, timeout, &argv[0])
    }
}

/// The placeholder a runner argument may be, replaced by the attempt worktree's path.
pub const WORKTREE_PLACEHOLDER: &str = "{worktree}";
/// Upper bound on the runner prefix's length, in arguments.
pub const MAX_RUNNER_ARGS: usize = 64;

/// A validated `[verify] runner` prefix: an absolute executable and its arguments, put
/// in front of every `verify_command`. No shell is involved. An argument that is exactly
/// `{worktree}` becomes the worktree path as one argument; the placeholder is never
/// substituted inside a longer argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyRunner {
    prefix: Vec<String>,
}

impl VerifyRunner {
    /// Validates a configured prefix. Errors name the offending argument.
    pub fn new(prefix: Vec<String>) -> Result<Self, String> {
        let Some(executable) = prefix.first() else {
            return Err("runner must name an executable".into());
        };
        if prefix.len() > MAX_RUNNER_ARGS {
            return Err(format!(
                "runner has {} arguments; at most {MAX_RUNNER_ARGS} are allowed",
                prefix.len()
            ));
        }
        for (i, arg) in prefix.iter().enumerate() {
            if arg.is_empty() {
                return Err(format!("runner[{i}] must not be empty"));
            }
            if arg.contains('\0') {
                return Err(format!("runner[{i}] must not contain a NUL character"));
            }
            if arg != WORKTREE_PLACEHOLDER && arg.contains(WORKTREE_PLACEHOLDER) {
                return Err(format!(
                    "runner[{i}] '{arg}': {WORKTREE_PLACEHOLDER} must be a whole argument"
                ));
            }
            if arg != WORKTREE_PLACEHOLDER && arg.starts_with('{') && arg.ends_with('}') {
                return Err(format!(
                    "runner[{i}] '{arg}' is not a known placeholder (only {WORKTREE_PLACEHOLDER})"
                ));
            }
        }
        if executable == WORKTREE_PLACEHOLDER || !Path::new(executable).is_absolute() {
            return Err(format!(
                "runner[0] '{executable}' must be an absolute path to the executable"
            ));
        }
        Ok(Self { prefix })
    }

    pub fn executable(&self) -> &str {
        &self.prefix[0]
    }

    pub fn prefix(&self) -> &[String] {
        &self.prefix
    }

    /// The full argv after the executable: the prefix's arguments with `{worktree}`
    /// replaced, then the verify command itself.
    pub fn wrapped_args(&self, worktree: &Path, argv: &[String]) -> Vec<OsString> {
        let mut args = Vec::with_capacity(self.prefix.len() - 1 + argv.len());
        for arg in &self.prefix[1..] {
            if arg == WORKTREE_PLACEHOLDER {
                args.push(worktree.as_os_str().to_os_string());
            } else {
                args.push(OsString::from(arg));
            }
        }
        args.extend(argv.iter().map(OsString::from));
        args
    }
}

/// Runs each check through a [`VerifyRunner`] prefix. The prefix's process is the one
/// meshloop owns (ADR 0025/0035): a timeout or cancel kills its whole process tree, and
/// the shipped sandbox wrapper `exec`s bwrap with `--die-with-parent` in a new PID
/// namespace, so nothing inside outlives it. Evidence is labelled with the verify
/// command's own program, exactly as on the host.
pub struct ContainedCheckRunner {
    runner: VerifyRunner,
}

impl ContainedCheckRunner {
    pub fn new(runner: VerifyRunner) -> Self {
        Self { runner }
    }

    pub fn runner(&self) -> &VerifyRunner {
        &self.runner
    }
}

impl CheckRunner for ContainedCheckRunner {
    fn run(
        &self,
        worktree: &Path,
        argv: &[String],
        timeout: Duration,
    ) -> Result<DeterministicEvidence, CheckError> {
        if argv.is_empty() {
            return Err(CheckError::Io("empty verify_command".into()));
        }
        // The wrapper compares its --worktree argument with its working directory, so
        // both must name the same absolute path.
        let worktree = std::path::absolute(worktree).map_err(|e| CheckError::Io(e.to_string()))?;
        let args = self.runner.wrapped_args(&worktree, argv);
        run_owned(
            &worktree,
            OsStr::new(self.runner.executable()),
            &args,
            timeout,
            &argv[0],
        )
    }
}

/// The verification runner the CLI composes from `[verify]`.
pub enum VerifyChecks {
    /// No `runner`: `verify_command` runs on the host (the default, unchanged).
    Host(CommandCheckRunner),
    /// `runner` set: every check runs behind the operator's prefix.
    Contained(ContainedCheckRunner),
}

impl VerifyChecks {
    pub fn from_runner(runner: Option<VerifyRunner>) -> Self {
        match runner {
            None => Self::Host(CommandCheckRunner),
            Some(r) => Self::Contained(ContainedCheckRunner::new(r)),
        }
    }

    pub fn is_contained(&self) -> bool {
        matches!(self, Self::Contained(_))
    }
}

impl CheckRunner for VerifyChecks {
    fn run(
        &self,
        worktree: &Path,
        argv: &[String],
        timeout: Duration,
    ) -> Result<DeterministicEvidence, CheckError> {
        match self {
            Self::Host(r) => r.run(worktree, argv, timeout),
            Self::Contained(r) => r.run(worktree, argv, timeout),
        }
    }
}

/// Spawns `program args` as an owned process tree in `worktree` and collects evidence.
fn run_owned(
    worktree: &Path,
    program: &OsStr,
    args: &[OsString],
    timeout: Duration,
    tool: &str,
) -> Result<DeterministicEvidence, CheckError> {
    let mut cmd = OwnedSpawnSpec::new(program);
    cmd.args(args)
        .current_dir(worktree)
        .stdin(OwnedStdio::Null)
        .stdout(OwnedStdio::Piped)
        .stderr(OwnedStdio::Piped);
    let mut child = crate::process::spawn_owned(cmd).map_err(|e| CheckError::Io(e.to_string()))?;
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
                    tool: tool.to_string(),
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

pub(crate) fn spawn_drain<R: Read + Send + 'static>(
    pipe: Option<R>,
) -> std::thread::JoinHandle<String> {
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

    fn abs_exe() -> String {
        if cfg!(windows) {
            r"C:\tools\sandbox.exe".into()
        } else {
            "/opt/meshloop/meshloop-verify-sandbox".into()
        }
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// Issue #181: the prefix is structured argv. `{worktree}` is replaced only as a
    /// whole argument, and the verify command follows the prefix unchanged.
    #[test]
    fn runner_prefix_substitutes_worktree_as_one_argument() {
        let mut prefix = vec![abs_exe()];
        prefix.extend(strings(&["--worktree", "{worktree}", "--"]));
        let runner = VerifyRunner::new(prefix).expect("valid prefix");
        let wt = Path::new("/work/a tree/with {braces}");
        let args = runner.wrapped_args(wt, &strings(&["cargo", "test", "{worktree}"]));
        let args: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            vec![
                "--worktree",
                "/work/a tree/with {braces}",
                "--",
                "cargo",
                "test",
                // The verify command is the operator's own argv: never substituted.
                "{worktree}",
            ]
        );
        assert_eq!(runner.executable(), abs_exe());
    }

    #[test]
    fn runner_prefix_validation_rejects_bad_shapes() {
        let bad: Vec<(Vec<String>, &str)> = vec![
            (vec![], "must name an executable"),
            (strings(&["sandbox", "--"]), "absolute path"),
            (strings(&["./sandbox"]), "absolute path"),
            (strings(&["{worktree}"]), "absolute path"),
            (
                vec![abs_exe(), String::new()],
                "runner[1] must not be empty",
            ),
            (vec![abs_exe(), "a\0b".into()], "NUL"),
            (
                vec![abs_exe(), "--worktree={worktree}".into()],
                "must be a whole argument",
            ),
            (
                vec![abs_exe(), "{prompt_file}".into()],
                "not a known placeholder",
            ),
            (
                std::iter::once(abs_exe())
                    .chain((0..MAX_RUNNER_ARGS).map(|_| "x".to_string()))
                    .collect(),
                "at most",
            ),
        ];
        for (prefix, frag) in bad {
            let err = VerifyRunner::new(prefix.clone()).expect_err("must be rejected");
            assert!(err.contains(frag), "{prefix:?}: {err}");
        }
        VerifyRunner::new(vec![abs_exe()]).expect("a bare absolute executable is valid");
    }

    #[test]
    fn verify_checks_default_to_the_host_runner() {
        assert!(!VerifyChecks::from_runner(None).is_contained());
        let r = VerifyRunner::new(vec![abs_exe()]).unwrap();
        assert!(VerifyChecks::from_runner(Some(r)).is_contained());
    }

    /// The prefix's process gets the worktree as its working directory and the verify
    /// command after the prefix; evidence keeps the verify command's program as `tool`.
    #[cfg(unix)]
    #[test]
    fn contained_runner_runs_the_prefix_in_the_worktree_and_propagates_exit() {
        let dir = std::env::temp_dir().join(format!(
            "meshloop-check-prefix-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // /bin/sh -c 'SCRIPT' wrapper WT -- PROGRAM ARGS: prints its cwd and argv, then
        // runs the command so its exit code is the check's.
        let script = r#"printf 'cwd=%s wt=%s sep=%s\n' "$(pwd -P)" "$1" "$2"; shift 2; exec "$@""#;
        let runner = VerifyRunner::new(strings(&[
            "/bin/sh",
            "-c",
            script,
            "wrapper",
            "{worktree}",
            "--",
        ]))
        .unwrap();
        let checks = ContainedCheckRunner::new(runner);
        let pass = checks
            .run(
                &dir,
                &strings(&["sh", "-c", "exit 0"]),
                Duration::from_secs(10),
            )
            .expect("evidence");
        let fail = checks
            .run(
                &dir,
                &strings(&["sh", "-c", "exit 3"]),
                Duration::from_secs(10),
            )
            .expect("evidence");
        let real = std::fs::canonicalize(&dir).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(pass.exit_code, 0, "{}", pass.output_redacted);
        assert_eq!(fail.exit_code, 3, "{}", fail.output_redacted);
        assert_eq!(pass.tool, "sh");
        assert!(
            pass.output_redacted.contains(&format!(
                "cwd={} wt={} sep=--",
                real.display(),
                dir.display()
            )),
            "{}",
            pass.output_redacted
        );
    }
}
