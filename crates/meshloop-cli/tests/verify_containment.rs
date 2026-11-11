//! Issue #181: contained verification. With `[verify] runner` set to
//! `adapters/sandbox/meshloop-verify-sandbox`, a verify step that executes
//! worker-written repository code runs under bubblewrap: it writes only the attempt
//! worktree, reads no operator secret, reaches no host network, inherits no
//! descriptor, and its exit code is still the check's verdict.
//!
//! `adapters/sandbox/verify-containment-test.sh` holds the wrapper's own checks. The
//! end-to-end tests here drive `meshloop run` with the runner configured, and the
//! runner's timeout kill through the owned-process path. All need bubblewrap and git
//! but no network; each prints `SKIP:` and passes where bubblewrap or unprivileged user
//! namespaces are unavailable, unless `MESHLOOP_REQUIRE_CONTAINMENT=1` (set by CI where
//! a probe shows the sandbox can be built). Linux only.

#![cfg(target_os = "linux")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

fn wrapper() -> PathBuf {
    repo_root().join("adapters/sandbox/meshloop-verify-sandbox")
}

/// Cargo's per-test temporary directory lives under target/, on the build disk (the
/// wrapper refuses tmpfs and Windows drives).
fn base_dir(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("verify-containment")
        .join(format!("{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create base dir");
    dir.canonicalize().expect("canonical base dir")
}

fn run(cmd: &mut Command) -> std::process::Output {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    meshloop_adapters::process::spawn_command(cmd)
        .expect("spawn")
        .wait_with_output()
        .expect("wait")
}

/// Whether bubblewrap can build the wrapper's sandbox here. Without it the test skips,
/// unless CI requires containment.
fn sandbox_available() -> bool {
    let probe = run(Command::new("bwrap").args([
        "--unshare-user",
        "--disable-userns",
        "--unshare-pid",
        "--unshare-ipc",
        "--unshare-uts",
        "--unshare-cgroup-try",
        "--unshare-net",
        "--die-with-parent",
        "--new-session",
        "--ro-bind",
        "/",
        "/",
        "true",
    ]));
    if probe.status.success() {
        return true;
    }
    let why = format!(
        "bwrap cannot build the sandbox here: {}",
        String::from_utf8_lossy(&probe.stderr)
    );
    assert!(
        std::env::var("MESHLOOP_REQUIRE_CONTAINMENT").as_deref() != Ok("1"),
        "containment required (MESHLOOP_REQUIRE_CONTAINMENT=1) but would skip: {why}"
    );
    println!("SKIP: {why}");
    false
}

fn bwrap_installed() -> bool {
    let mut found = Command::new("sh");
    found
        .args(["-c", "command -v bwrap"])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let ok = meshloop_adapters::process::spawn_command(&mut found)
        .and_then(|mut c| c.wait())
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        assert!(
            std::env::var("MESHLOOP_REQUIRE_CONTAINMENT").as_deref() != Ok("1"),
            "containment required (MESHLOOP_REQUIRE_CONTAINMENT=1) but bwrap is not installed"
        );
        println!("SKIP: bwrap (bubblewrap) is not installed");
    }
    ok
}

#[test]
fn verify_sandbox_confines_a_worker_written_check() {
    let script = repo_root().join("adapters/sandbox/verify-containment-test.sh");
    let base = base_dir("script");
    let out = run(Command::new("bash").arg(&script).arg(&base));
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    println!("{text}");
    let _ = fs::remove_dir_all(&base);
    assert!(
        out.status.success(),
        "verify-containment-test.sh failed:\n{text}"
    );
    assert!(
        text.contains("verify-containment-test: all checks passed") || text.contains("SKIP:"),
        "verify-containment-test.sh neither passed nor skipped:\n{text}"
    );
}

fn git(dir: &Path, args: &[&str]) {
    let out = run(Command::new("git").current_dir(dir).args(args));
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn disposable_repo(base: &Path) -> PathBuf {
    let dir = base.join("repo");
    fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q"]);
    git(&dir, &["config", "user.email", "test@test"]);
    git(&dir, &["config", "user.name", "test"]);
    fs::write(dir.join("README.md"), "seed").unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "seed"]);
    dir
}

/// The worker: a bash "harness" that writes `worker-test.sh` into its worktree. The
/// script tries to write a marker into the repository root above the worktree, the
/// operator's HOME and the host's /tmp, then exits with the task's verdict: 1 when the
/// task description says `verdict-fail`, else 0. `verify_command` runs that script, as
/// a repository's own test runner would.
fn worker_script(targets: &[PathBuf]) -> String {
    let mut lines = String::from("for t in");
    for t in targets {
        lines.push_str(&format!(" '{}'", t.display()));
    }
    lines.push_str(
        "; do if ( : >\"$t\" ) 2>/dev/null; then echo \"ESCAPED $t\"; else echo \"REFUSED $t\"; fi; done",
    );
    format!(
        r#"if grep -q verdict-fail "$1"; then v=1; else v=0; fi
cat >worker-test.sh <<'WORKER_EOF'
{lines}
WORKER_EOF
printf 'exit %s\n' "$v" >>worker-test.sh
echo worker-wrote-test"#
    )
}

fn toml_str(s: &str) -> String {
    serde_json::to_string(s).unwrap()
}

fn write_config(dir: &Path, targets: &[PathBuf], runner: Option<&Path>) -> PathBuf {
    let template = serde_json::to_string(&[
        "-c".to_string(),
        worker_script(targets),
        "worker".into(),
        "{prompt_file}".into(),
    ])
    .unwrap();
    let runner_line = match runner {
        Some(w) => format!(
            "runner = [{}, \"--worktree\", \"{{worktree}}\", \"--\"]",
            toml_str(&w.display().to_string())
        ),
        None => String::new(),
    };
    let config = format!(
        r#"
selected_harnesses = ["worker"]

[limits]
max_concurrent_workers = 1
max_retries = 1
task_timeout_seconds = 60

[verify]
verify_command = ["sh", "worker-test.sh"]
{runner_line}

[harnesses.worker]
executable = "/usr/bin/bash"
version_args = ["--version"]
invoke_args_template = {template}
model_ref = "worker-model"
model_tier = "top"
"#
    );
    let path = dir.join("meshloop.toml");
    fs::write(&path, config).unwrap();
    path
}

struct RunResult {
    run: std::process::Output,
    tasks: Vec<serde_json::Value>,
}

/// Plans two independent nodes, one passing and one failing, runs them, and inspects
/// both. `HOME` is the fake operator home for every process meshloop starts.
fn run_two_nodes(dir: &Path, home: &Path, config: &Path) -> RunResult {
    let plan = dir.join("plan.json");
    let db = dir.join(".meshloop").join("state.sqlite");
    fs::create_dir_all(dir.join(".meshloop")).unwrap();
    fs::write(
        &plan,
        r#"{"graph_id":"g-contained","nodes":[{"id":1,"description":"verdict-pass","depends_on":[],"tier":null},{"id":2,"description":"verdict-fail","depends_on":[],"tier":null}]}"#,
    )
    .unwrap();
    let meshloop = || {
        let mut c = Command::new(env!("CARGO_BIN_EXE_meshloop"));
        c.current_dir(dir).env("HOME", home);
        c
    };
    let run_out = run(meshloop()
        .args(["run", "--plan"])
        .arg(&plan)
        .args(["--accept-plan", "--config"])
        .arg(config)
        .arg("--worktree-base")
        .arg(dir.join("worktrees"))
        .arg("--db")
        .arg(&db));
    let tasks = ["1", "2"]
        .iter()
        .map(|task| {
            let out = run(meshloop()
                .args([
                    "inspect",
                    "--graph",
                    "g-contained",
                    "--task",
                    task,
                    "--json",
                ])
                .arg("--config")
                .arg(config)
                .arg("--db")
                .arg(&db));
            serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
                panic!(
                    "inspect {task}: {e}: {}",
                    String::from_utf8_lossy(&out.stdout)
                )
            })
        })
        .collect();
    RunResult {
        run: run_out,
        tasks,
    }
}

fn verify_row(task: &serde_json::Value) -> serde_json::Value {
    task["data"]["attempts"][0]["evidence"]
        .as_array()
        .unwrap_or_else(|| panic!("no evidence: {task}"))
        .iter()
        .find(|row| row["tool"] == "sh")
        .cloned()
        .unwrap_or_else(|| panic!("no verify row: {task}"))
}

fn escape_targets(base: &Path, repo: &Path, home: &Path) -> Vec<PathBuf> {
    let tag = base.file_name().unwrap().to_string_lossy().into_owned();
    vec![
        repo.join("escaped-repo-root"),
        home.join("escaped-home"),
        PathBuf::from(format!("/tmp/meshloop-181-{tag}-escaped")),
    ]
}

#[test]
fn contained_verify_runs_worker_written_checks_in_the_sandbox_end_to_end() {
    if !bwrap_installed() || !sandbox_available() {
        return;
    }
    let base = base_dir("e2e");
    let repo = disposable_repo(&base);
    let home = base.join("home");
    fs::create_dir_all(&home).unwrap();
    fs::write(home.join(".secret"), "operator-secret").unwrap();
    let targets = escape_targets(&base, &repo, &home);
    let config = write_config(&base, &targets, Some(&wrapper()));
    let result = run_two_nodes(&repo, &home, &config);
    let escaped: Vec<&PathBuf> = targets.iter().filter(|t| t.exists()).collect();
    for t in &targets {
        let _ = fs::remove_file(t);
    }
    let run_text = format!(
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&result.run.stdout),
        String::from_utf8_lossy(&result.run.stderr)
    );
    assert!(
        escaped.is_empty(),
        "a worker-written check wrote outside the worktree: {escaped:?}\n{run_text}"
    );
    let (pass, fail) = (&result.tasks[0], &result.tasks[1]);
    assert_eq!(
        pass["data"]["state"], "AwaitingReview",
        "{pass}\n{run_text}"
    );
    assert_eq!(fail["data"]["state"], "Failed", "{fail}\n{run_text}");
    let pass_row = verify_row(pass);
    let fail_row = verify_row(fail);
    assert_eq!(pass_row["exit_code"], 0, "{pass_row}");
    assert_eq!(fail_row["exit_code"], 1, "{fail_row}");
    for row in [&pass_row, &fail_row] {
        let summary = row["summary_redacted"].as_str().unwrap_or_default();
        // /tmp is the sandbox's private tmpfs: a write there may succeed inside and
        // never reaches the host (checked above). Every other target is refused.
        for t in targets.iter().filter(|t| !t.starts_with("/tmp")) {
            assert!(
                summary.contains(&format!("REFUSED {}", t.display())),
                "the check did not run or did not record a refusal for {}: {summary}",
                t.display()
            );
        }
    }
    let _ = fs::remove_dir_all(&base);
}

/// Control for the test above: without `runner` the same worker-written check runs on
/// the host and does write outside the worktree. This is the default behaviour,
/// unchanged by #181, and the reason the opt-in exists.
#[test]
fn host_verify_without_runner_lets_the_same_check_escape() {
    let base = base_dir("host");
    let repo = disposable_repo(&base);
    let home = base.join("home");
    fs::create_dir_all(&home).unwrap();
    // Only paths under the disposable base: the control must not touch the host's /tmp.
    let targets = vec![repo.join("escaped-repo-root"), home.join("escaped-home")];
    let config = write_config(&base, &targets, None);
    let result = run_two_nodes(&repo, &home, &config);
    let escaped = targets.iter().filter(|t| t.exists()).count();
    let states = (
        result.tasks[0]["data"]["state"].clone(),
        result.tasks[1]["data"]["state"].clone(),
    );
    let _ = fs::remove_dir_all(&base);
    assert_eq!(
        escaped,
        targets.len(),
        "the host-mode control did not reproduce the escape"
    );
    assert_eq!(states.0, "AwaitingReview");
    assert_eq!(states.1, "Failed");
}

/// A timed-out contained check is killed through the owned-process path (ADR 0025):
/// nothing inside the sandbox survives, including a child that left its session.
#[test]
fn contained_check_timeout_kills_the_whole_sandbox() {
    use meshloop_adapters::check::{ContainedCheckRunner, VerifyRunner};
    use meshloop_engine::ports::{CheckError, CheckRunner};

    if !bwrap_installed() || !sandbox_available() {
        return;
    }
    let base = base_dir("timeout");
    let repo = disposable_repo(&base);
    git(&repo, &["worktree", "add", "-q", "wt", "-b", "wt"]);
    let wt = repo.join("wt");
    let marker = format!(
        "meshloop-181-timeout-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let runner = VerifyRunner::new(vec![
        wrapper().display().to_string(),
        "--worktree".into(),
        "{worktree}".into(),
        "--".into(),
    ])
    .unwrap();
    let checks = ContainedCheckRunner::new(runner);
    let argv: Vec<String> = vec![
        "sh".into(),
        "-c".into(),
        r#"setsid sh -c "sleep 1000; :" "$0-detached" </dev/null >/dev/null 2>&1 & sh -c "sleep 1000; :" "$0-child" & wait"#.into(),
        marker.clone(),
    ];
    let started = Instant::now();
    let result = checks.run(&wt, &argv, Duration::from_secs(2));
    assert!(
        matches!(result, Err(CheckError::Timeout)),
        "expected a timeout, got {result:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(20));
    let survivors = |marker: &str| -> Vec<String> {
        let mut found = Vec::new();
        for entry in fs::read_dir("/proc").into_iter().flatten().flatten() {
            if let Ok(cmd) = fs::read(entry.path().join("cmdline"))
                && String::from_utf8_lossy(&cmd).contains(marker)
            {
                found.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
        found
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut left = survivors(&marker);
    while !left.is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        left = survivors(&marker);
    }
    let _ = fs::remove_dir_all(&base);
    assert!(left.is_empty(), "sandboxed processes survived: {left:?}");
}

/// `meshloop doctor --json` for a config whose one harness reports containment in its
/// version line, with the given `[verify]` table.
fn doctor_json(tag: &str, verify_toml: &str) -> serde_json::Value {
    let base = base_dir(tag);
    let config = base.join("meshloop.toml");
    fs::write(
        &config,
        format!(
            r#"
selected_harnesses = ["boxed"]

[limits]
max_concurrent_workers = 1
max_retries = 1
task_timeout_seconds = 60

[verify]
{verify_toml}

[harnesses.boxed]
executable = "/usr/bin/bash"
version_args = ["-c", "echo 'boxed 1.0 (contained: test-sandbox)'"]
invoke_args_template = ["-c", "true"]
model_ref = "boxed-model"
model_tier = "top"
"#
        ),
    )
    .unwrap();
    let out = run(Command::new(env!("CARGO_BIN_EXE_meshloop"))
        .current_dir(&base)
        .args(["doctor", "--json", "--config"])
        .arg(&config)
        .arg("--db")
        .arg(base.join("state.sqlite")));
    let _ = fs::remove_dir_all(&base);
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("doctor json: {e}: {}", String::from_utf8_lossy(&out.stdout)))
}

fn warnings(doctor: &serde_json::Value) -> Vec<String> {
    doctor["data"]["warnings"]
        .as_array()
        .unwrap_or_else(|| panic!("warnings: {doctor}"))
        .iter()
        .map(|w| w.as_str().unwrap_or_default().to_string())
        .collect()
}

/// Issue #181: doctor warns when a harness runs under containment but verification runs
/// on the host, and not once a runner is set; a runner whose executable is missing is
/// its own warning.
#[test]
fn doctor_warns_when_a_contained_harness_has_uncontained_verification() {
    let host = doctor_json("doctor-host", r#"verify_command = ["make", "check"]"#);
    assert_eq!(host["data"]["verify"]["contained"], false, "{host}");
    assert!(
        warnings(&host)
            .iter()
            .any(|w| w.contains("runs under containment") && w.contains("[verify] runner")),
        "{host}"
    );

    let contained = doctor_json(
        "doctor-contained",
        &format!(
            "verify_command = [\"make\", \"check\"]\nrunner = [{}, \"--worktree\", \"{{worktree}}\", \"--\"]",
            toml_str(&wrapper().display().to_string())
        ),
    );
    assert_eq!(
        contained["data"]["verify"]["contained"], true,
        "{contained}"
    );
    assert_eq!(
        contained["data"]["verify"]["runner"][2], "{worktree}",
        "{contained}"
    );
    assert!(
        warnings(&contained)
            .iter()
            .all(|w| !w.contains("runs under containment") && !w.contains("verify.runner")),
        "{contained}"
    );

    let missing = doctor_json(
        "doctor-missing",
        "verify_command = [\"make\", \"check\"]\nrunner = [\"/nonexistent/meshloop-verify-sandbox\", \"--\"]",
    );
    assert!(
        warnings(&missing)
            .iter()
            .any(|w| w.contains("verify.runner") && w.contains("missing")),
        "{missing}"
    );
}
