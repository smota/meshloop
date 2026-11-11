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

/// Error detail when a check's output pipes stayed open past the drain grace: a
/// process outside the check's owned tree still holds them (issue #186).
pub const CHECK_OUTPUT_ABANDONED: &str = "check-output-abandoned";

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
            // The sandbox wrapper's dry run prints its argv instead of running the
            // check; in a prefix it would turn every check into a no-op.
            if arg == "--print-bwrap" {
                return Err(format!(
                    "runner[{i}] '--print-bwrap' only prints the sandbox; remove it"
                ));
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

/// Error detail when the check's direct child exited but its owned tree could not be
/// observed ended within [`TREE_END_GRACE`] (ADR 0025/0035, issue #186).
pub const CHECK_TREE_SURVIVED: &str = "check-process-tree-survived";
/// How long ending a check's tree may take after its direct child exits or is killed.
const TREE_END_GRACE: Duration = Duration::from_secs(5);
/// How long output readers may take to reach end-of-file once the tree is ended.
const DRAIN_GRACE: Duration = Duration::from_secs(2);

/// Spawns `program args` as an owned process tree in `worktree` and collects evidence.
///
/// Bounded (issue #186): once the direct child exits or the timeout kills it, the whole
/// owned tree is ended (the harness's post-exit mechanism, #180) and the output is
/// drained for at most [`DRAIN_GRACE`], so a check returns within
/// `timeout + TREE_END_GRACE + DRAIN_GRACE`. A pipe still open after that is held by a
/// process outside the owned tree; its reader is abandoned and the check fails closed.
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
    let drain = Drain::start(&mut child);
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let ended = child.end_tree_after_exit(TREE_END_GRACE);
                let output = drain.finish(DRAIN_GRACE);
                if !ended {
                    return Err(CheckError::Io(CHECK_TREE_SURVIVED.into()));
                }
                let Some((stdout, stderr)) = output else {
                    return Err(CheckError::Io(format!(
                        "{CHECK_OUTPUT_ABANDONED}: output still held open {}s after the \
                         check exited; a process outside its tree holds it",
                        DRAIN_GRACE.as_secs()
                    )));
                };
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
                    // The timeout is the verdict; ending the tree and draining are
                    // bounded so it is reported, never a pass and never a hang.
                    end_killed_tree(&mut child);
                    let _ = drain.finish(DRAIN_GRACE);
                    return Err(CheckError::Timeout);
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => {
                end_killed_tree(&mut child);
                let _ = drain.finish(DRAIN_GRACE);
                return Err(CheckError::Io(e.to_string()));
            }
        }
    }
}

/// Kills a check that has not exited and ends the rest of its owned tree, bounded by
/// [`TREE_END_GRACE`]. The caller already fails the check, so a survivor adds nothing
/// to its verdict.
fn end_killed_tree(child: &mut crate::process::OwnedChild) {
    child.kill_tree();
    let _ = child.end_tree_after_exit(TREE_END_GRACE);
}

/// Reader threads for a check's stdout and stderr.
struct Drain {
    stdout: std::thread::JoinHandle<String>,
    stderr: std::thread::JoinHandle<String>,
}

impl Drain {
    fn start(child: &mut crate::process::OwnedChild) -> Self {
        Self {
            stdout: spawn_drain(child.stdout.take()),
            stderr: spawn_drain(child.stderr.take()),
        }
    }

    /// Both streams once both readers reach end-of-file within `grace`. `None` when a
    /// pipe is still held open after `grace`: the readers are then abandoned (detached;
    /// each ends when the last writer closes its pipe) and their output discarded.
    fn finish(self, grace: Duration) -> Option<(String, String)> {
        let deadline = Instant::now() + grace;
        while !(self.stdout.is_finished() && self.stderr.is_finished()) {
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        // Both readers have finished: these joins cannot block. A reader that panicked
        // lost output; treat it like abandoned output (fail closed), never as empty.
        let (Ok(stdout), Ok(stderr)) = (self.stdout.join(), self.stderr.join()) else {
            return None;
        };
        Some((stdout, stderr))
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
                vec![abs_exe(), "--print-bwrap".into(), "--".into()],
                "only prints the sandbox",
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

    /// A fresh directory for one fixture's pid files.
    #[cfg(unix)]
    fn fixture_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "meshloop-check-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Runs `f` on its own thread and gives up after `limit`, so a hanging check fails
    /// the test instead of hanging the suite.
    #[cfg(unix)]
    fn within<T: Send + 'static>(
        limit: Duration,
        f: impl FnOnce() -> T + Send + 'static,
    ) -> Option<T> {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        rx.recv_timeout(limit).ok()
    }

    /// The pid a fixture wrote to `dir/name`, waiting briefly for the write.
    #[cfg(unix)]
    fn fixture_pid(dir: &Path, name: &str) -> u32 {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(text) = std::fs::read_to_string(dir.join(name))
                && let Ok(pid) = text.trim().parse()
            {
                return pid;
            }
            assert!(Instant::now() < deadline, "fixture never wrote {name}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Ends a process that left the check's tree (it is not ours to own), so a test
    /// leaves nothing behind.
    #[cfg(unix)]
    fn reap_escaped(dir: &Path) {
        if let Ok(text) = std::fs::read_to_string(dir.join("escaped"))
            && let Ok(pid) = text.trim().parse::<u32>()
        {
            crate::process::kill_process_tree(pid);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A background process in the check's group that keeps stdout open.
    #[cfg(unix)]
    const BACKGROUND_HOLDER: &str = r#"echo $$ > "$1/group"; sleep 30 & echo done"#;
    /// A process that left the check's group (`setsid`) and keeps stdout open.
    #[cfg(target_os = "linux")]
    const ESCAPED_HOLDER: &str =
        r#"echo $$ > "$1/group"; setsid sh -c 'echo $$ > "$0/escaped"; exec sleep 30' "$1" &"#;

    /// Issue #186: a check whose direct child exits while a background process still
    /// holds its stdout returns at once, and the check's process group is ended.
    #[cfg(unix)]
    #[test]
    fn check_exit_ends_the_tree_and_does_not_wait_on_held_pipes() {
        let dir = fixture_dir("exit-held");
        let argv = strings(&["sh", "-c", BACKGROUND_HOLDER, "sh"]);
        let wt = dir.clone();
        let started = Instant::now();
        let result = within(Duration::from_secs(15), move || {
            let mut argv = argv;
            argv.push(wt.display().to_string());
            CommandCheckRunner.run(&wt, &argv, Duration::from_secs(20))
        });
        let group = fixture_pid(&dir, "group");
        let alive = crate::process::process_group_alive(group);
        crate::process::kill_process_tree(group);
        let _ = std::fs::remove_dir_all(&dir);
        let evidence = result
            .expect("the check must not wait on a pipe its background process holds")
            .expect("evidence");
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(evidence.exit_code, 0, "{}", evidence.output_redacted);
        assert!(evidence.output_redacted.contains("done"));
        assert_eq!(
            alive,
            Some(false),
            "the check's process group must be empty"
        );
    }

    /// Issue #186, #181: the contained runner shares the same bounded drain.
    #[cfg(unix)]
    #[test]
    fn contained_check_exit_ends_the_tree_and_does_not_wait_on_held_pipes() {
        let dir = fixture_dir("contained-held");
        let runner =
            VerifyRunner::new(strings(&["/bin/sh", "-c", r#"exec "$@""#, "wrapper"])).unwrap();
        let checks = ContainedCheckRunner::new(runner);
        let wt = dir.clone();
        let result = within(Duration::from_secs(15), move || {
            let mut argv = strings(&["sh", "-c", BACKGROUND_HOLDER, "sh"]);
            argv.push(wt.display().to_string());
            checks.run(&wt, &argv, Duration::from_secs(20))
        });
        let group = fixture_pid(&dir, "group");
        let alive = crate::process::process_group_alive(group);
        crate::process::kill_process_tree(group);
        let _ = std::fs::remove_dir_all(&dir);
        let evidence = result
            .expect("the contained check must not wait on a held pipe")
            .expect("evidence");
        assert_eq!(evidence.exit_code, 0, "{}", evidence.output_redacted);
        assert_eq!(
            alive,
            Some(false),
            "the check's process group must be empty"
        );
    }

    /// Issue #186: after a timeout, a process that escaped the check's group and holds
    /// its pipes cannot hold verification: the check reports the timeout within the
    /// timeout plus a fixed grace, and its own process group is empty.
    #[cfg(target_os = "linux")]
    #[test]
    fn check_timeout_is_bounded_when_an_escaped_process_holds_the_pipes() {
        let dir = fixture_dir("timeout-escaped");
        let script = format!("{ESCAPED_HOLDER} sleep 30");
        let wt = dir.clone();
        let started = Instant::now();
        let result = within(Duration::from_secs(15), move || {
            let argv = strings(&["sh", "-c", &script, "sh", &wt.display().to_string()]);
            CommandCheckRunner.run(&wt, &argv, Duration::from_secs(1))
        });
        let group = fixture_pid(&dir, "group");
        let alive = crate::process::process_group_alive(group);
        crate::process::kill_process_tree(group);
        reap_escaped(&dir);
        let result = result.expect("a timed-out check must return within its grace");
        assert!(
            matches!(result, Err(CheckError::Timeout)),
            "expected a timeout, got {result:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(
            alive,
            Some(false),
            "the check's process group must be empty"
        );
    }

    /// Issue #186: a check that exits 0 while an escaped process still holds its output
    /// is not a pass; the abandoned output is reported as an error within the grace.
    #[cfg(target_os = "linux")]
    #[test]
    fn check_exit_with_an_escaped_pipe_holder_fails_closed_within_the_grace() {
        let dir = fixture_dir("exit-escaped");
        let script = format!("{ESCAPED_HOLDER} echo done");
        let wt = dir.clone();
        let started = Instant::now();
        let result = within(Duration::from_secs(15), move || {
            let argv = strings(&["sh", "-c", &script, "sh", &wt.display().to_string()]);
            CommandCheckRunner.run(&wt, &argv, Duration::from_secs(20))
        });
        let _ = fixture_pid(&dir, "escaped");
        reap_escaped(&dir);
        let result = result.expect("the check must return within its grace");
        match result {
            Err(CheckError::Io(detail)) => assert!(
                detail.contains(CHECK_OUTPUT_ABANDONED),
                "unexpected detail: {detail}"
            ),
            other => panic!("expected an abandoned-output error, got {other:?}"),
        }
        assert!(started.elapsed() < Duration::from_secs(10));
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
