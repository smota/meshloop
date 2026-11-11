//! `meshloop watch` on a stopped run: NDJSON state changes and a non-zero exit.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture_path() -> String {
    let exe = format!("fixture_harness{}", std::env::consts::EXE_SUFFIX);
    let mut dir = std::env::current_exe().expect("current test executable path");
    dir.pop();
    dir.pop();
    let path = dir.join(&exe);
    assert!(
        path.exists(),
        "expected fixture harness at {}",
        path.display()
    );
    path.to_string_lossy().to_string()
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "{args:?} {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn temp_repo(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("meshloop-watch-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q"]);
    git(&dir, &["config", "user.email", "test@test"]);
    git(&dir, &["config", "user.name", "test"]);
    fs::write(dir.join("README.md"), "seed").unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "seed"]);
    dir
}

fn write_config(dir: &Path) -> PathBuf {
    let path = dir.join("meshloop.toml");
    let contents = format!(
        r#"
selected_harnesses = ["fixture"]

[limits]
max_concurrent_workers = 1
max_retries = 1
task_timeout_seconds = 30

[verify]
verify_command = []

[harnesses.fixture]
executable = "{}"
version_args = ["--version"]
invoke_args_template = ["--fail"]
model_ref = "fixture-model"
model_tier = "top"
"#,
        fixture_path().replace('\\', "\\\\")
    );
    fs::write(&path, contents).unwrap();
    path
}

fn meshloop(dir: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_meshloop"));
    cmd.current_dir(dir);
    cmd
}

fn ndjson(out: &std::process::Output) -> Vec<serde_json::Value> {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("NDJSON {line:?}: {e}")))
        .collect()
}

#[test]
fn watch_failed_run_exits_nonzero_and_json_names_the_change() {
    let dir = temp_repo("fail");
    let config = write_config(&dir);
    let db = dir.join("state.sqlite");
    let plan = dir.join("plan.json");
    fs::write(
        &plan,
        r#"{"graph_id":"gfail","nodes":[{"id":1,"description":"first","depends_on":[],"tier":null}]}"#,
    )
    .unwrap();
    let run = meshloop(&dir)
        .args(["run", "--plan"])
        .arg(&plan)
        .args(["--accept-plan", "--fixture-only", "--config"])
        .arg(&config)
        .arg("--db")
        .arg(&db)
        .arg("--worktree-base")
        .arg(dir.join("worktrees"))
        .output()
        .expect("run");
    let run_out = format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );

    let json = meshloop(&dir)
        .args([
            "watch",
            "--graph",
            "gfail",
            "--json",
            "--timeout",
            "2",
            "--config",
        ])
        .arg(&config)
        .arg("--db")
        .arg(&db)
        .output()
        .expect("watch --json");
    assert_eq!(json.status.code(), Some(1), "{run_out}\n{json:?}");
    let lines = ndjson(&json);
    let last = lines.last().expect("exit line");
    assert_eq!(last["exit"], "failed", "{lines:?}");
    assert_eq!(last["nodes"], serde_json::json!([1]), "{lines:?}");
    let changes: Vec<_> = lines
        .iter()
        .filter(|line| line.get("node").is_some())
        .collect();
    assert_eq!(changes.len(), 1, "{lines:?}");
    assert_eq!(changes[0]["node"], 1);
    assert_eq!(changes[0]["to"], "Failed");
    let plans: Vec<_> = lines
        .iter()
        .filter(|line| line["change"] == "plan")
        .collect();
    assert_eq!(plans.len(), 1, "{lines:?}");
    assert_eq!(plans[0]["to"], "PlanAccepted");

    let text = meshloop(&dir)
        .args(["watch", "--graph", "gfail", "--timeout", "2", "--config"])
        .arg(&config)
        .arg("--db")
        .arg(&db)
        .output()
        .expect("watch text");
    assert_eq!(text.status.code(), Some(1), "{text:?}");
    let text_out = String::from_utf8_lossy(&text.stdout);
    let last_line = text_out.lines().next_back().unwrap_or("");
    assert!(
        last_line.contains("failed"),
        "last line should name the reason, got {text_out:?}"
    );
    assert!(
        text_out.contains("-> Failed"),
        "text mode should show the node change: {text_out:?}"
    );
    assert!(
        text_out.contains("plan:"),
        "text mode should show the plan state: {text_out:?}"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// Kills every recorded driver and attempt process when a test ends, even on panic.
struct Reaper {
    db: PathBuf,
}

impl Drop for Reaper {
    fn drop(&mut self) {
        let Ok(conn) = rusqlite::Connection::open(&self.db) else {
            return;
        };
        let _ = conn.busy_timeout(std::time::Duration::from_secs(2));
        for sql in [
            "SELECT pid FROM run_driver_liveness",
            "SELECT pid FROM attempts WHERE pid IS NOT NULL",
        ] {
            let Ok(mut stmt) = conn.prepare(sql) else {
                continue;
            };
            let Ok(rows) = stmt.query_map([], |row| row.get::<_, i64>(0)) else {
                continue;
            };
            for pid in rows.filter_map(Result::ok).filter(|pid| *pid > 0) {
                meshloop_adapters::process::kill_process_tree(pid as u32);
            }
        }
    }
}

fn gated_config(dir: &Path, gate: &Path) -> PathBuf {
    let path = dir.join("meshloop.toml");
    let esc = |s: &str| s.replace('\\', "\\\\");
    let contents = format!(
        r#"
selected_harnesses = ["fixture"]

[limits]
max_concurrent_workers = 2
max_retries = 1
task_timeout_seconds = 150

[verify]
verify_command = []

[harnesses.fixture]
executable = "{}"
version_args = ["--version"]
invoke_args_template = ["--gated-prompt-file", "{{prompt_file}}", "{}"]
model_ref = "fixture-model"
model_tier = "top"
"#,
        esc(&fixture_path()),
        esc(&gate.to_string_lossy()),
    );
    fs::write(&path, contents).unwrap();
    path
}

/// Runs `cmd`, killing it if it outlives `limit`, so a stalled `watch` fails the
/// test instead of hanging the suite.
fn bounded_output(cmd: &mut Command, limit: std::time::Duration) -> (std::process::Output, bool) {
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = meshloop_adapters::process::spawn_command(cmd).expect("spawn");
    let deadline = std::time::Instant::now() + limit;
    let mut killed = false;
    while child.try_wait().expect("try_wait").is_none() {
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            killed = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    (child.wait_with_output().expect("watch output"), killed)
}

/// A `watch` whose NDJSON stdout is read line by line as it is written, so a
/// test can act on what the watch has already seen. Killed on drop.
struct Streamed {
    child: std::process::Child,
    lines: std::sync::mpsc::Receiver<serde_json::Value>,
    seen: Vec<serde_json::Value>,
}

impl Streamed {
    fn spawn(cmd: &mut Command) -> Self {
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        let mut child = meshloop_adapters::process::spawn_command(cmd).expect("spawn watch");
        let stdout = child.stdout.take().expect("watch stdout");
        let (send, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            use std::io::BufRead;
            for line in std::io::BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if line.is_empty() {
                    continue;
                }
                let value = serde_json::from_str(&line)
                    .unwrap_or_else(|_| serde_json::json!({ "unparsed": line }));
                if send.send(value).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            lines,
            seen: Vec::new(),
        }
    }

    /// Waits for a line matching `pred`. Fails on the exit line, EOF or `limit`.
    fn until(
        &mut self,
        pred: impl Fn(&serde_json::Value) -> bool,
        limit: std::time::Duration,
    ) -> Result<(), String> {
        let deadline = std::time::Instant::now() + limit;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(line) => {
                    let hit = pred(&line);
                    let exit = line.get("exit").is_some();
                    self.seen.push(line);
                    if hit {
                        return Ok(());
                    }
                    if exit {
                        return Err(format!("watch exited first: {:?}", self.seen));
                    }
                }
                Err(_) => return Err(format!("no matching line: {:?}", self.seen)),
            }
        }
    }

    /// Reads to the exit line and the process exit, all within `limit`.
    fn finish(
        mut self,
        limit: std::time::Duration,
    ) -> Result<(Vec<serde_json::Value>, Option<i32>), String> {
        self.until(|l| l.get("exit").is_some(), limit)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if let Some(status) = self.child.try_wait().map_err(|e| e.to_string())? {
                return Ok((std::mem::take(&mut self.seen), status.code()));
            }
            if std::time::Instant::now() >= deadline {
                return Err(format!(
                    "watch printed exit but kept running: {:?}",
                    self.seen
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}

impl Drop for Streamed {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn watch_cmd(dir: &Path, config: &Path, db: &Path, graph: &str) -> Command {
    let mut cmd = meshloop(dir);
    cmd.args([
        "watch",
        "--graph",
        graph,
        "--json",
        "--interval",
        "1",
        "--config",
    ])
    .arg(config)
    .arg("--db")
    .arg(db);
    cmd
}

/// #195: a `watch` started after `accept`, while the detached driver is still
/// working on node 2, must report node 2's AwaitingReview and exit, including
/// after the driver exits. Before the fix the watch process dropped its SQLite
/// WAL lock while opening the ledger, the exiting driver unlinked the WAL under
/// it, and the watch read that orphaned snapshot until killed.
#[test]
fn watch_started_after_accept_reports_the_next_review_after_the_driver_exits() {
    let dir = temp_repo("after-accept");
    let gate = dir.join("gate-open");
    let config = gated_config(&dir, &gate);
    let db = dir.join("state.sqlite");
    let _reaper = Reaper { db: db.clone() };
    let plan = dir.join("plan.json");
    fs::write(
        &plan,
        r#"{"graph_id":"gaccept","nodes":[{"id":1,"description":"fast first","depends_on":[],"tier":null},{"id":2,"description":"GATED second","depends_on":[],"tier":null}]}"#,
    )
    .unwrap();
    let run = meshloop(&dir)
        .args(["run", "--plan"])
        .arg(&plan)
        .args([
            "--accept-plan",
            "--fixture-only",
            "--detach",
            "--json",
            "--config",
        ])
        .arg(&config)
        .arg("--db")
        .arg(&db)
        .arg("--worktree-base")
        .arg(dir.join("worktrees"))
        .output()
        .expect("run --detach");
    assert!(run.status.success(), "{run:?}");

    let limit = std::time::Duration::from_secs(90);
    let (first, killed) = bounded_output(&mut watch_cmd(&dir, &config, &db, "gaccept"), limit);
    assert!(!killed, "first watch stalled: {first:?}");
    let lines = ndjson(&first);
    let exit = lines.last().expect("exit line");
    assert_eq!(exit["exit"], "awaiting_review", "{lines:?}");
    assert_eq!(exit["nodes"], serde_json::json!([1]), "{lines:?}");

    let accept = meshloop(&dir)
        .args([
            "accept", "--task", "1", "--as", "tester", "--graph", "gaccept",
        ])
        .arg("--config")
        .arg(&config)
        .arg("--db")
        .arg(&db)
        .output()
        .expect("accept");
    assert!(accept.status.success(), "{accept:?}");

    // Open the gate only once the second watch has streamed node 2 as Running, so
    // node 2's review and the driver's exit both land while it is watching.
    let mut second = Streamed::spawn(&mut watch_cmd(&dir, &config, &db, "gaccept"));
    second
        .until(
            |l| l["node"] == 2 && l["to"] == "Running" && l["from"].is_null(),
            limit,
        )
        .unwrap_or_else(|e| panic!("second watch must start while node 2 runs: {e}"));
    fs::write(&gate, "open").unwrap();
    let (lines, code) = second
        .finish(limit)
        .unwrap_or_else(|e| panic!("second watch stalled after node 2 left Running: {e}"));
    let exit = lines.last().expect("exit line");
    assert_eq!(exit["exit"], "awaiting_review", "{lines:?}");
    assert_eq!(exit["nodes"], serde_json::json!([2]), "{lines:?}");
    assert_eq!(code, Some(0), "{lines:?}");

    let _ = fs::remove_dir_all(&dir);
}

/// A detached driver that is gone while a node still needs it leaves nothing to
/// wait for: `watch` exits with `driver_gone` instead of blocking forever.
#[test]
fn watch_exits_when_the_detached_driver_is_gone_and_no_node_can_progress() {
    let dir = temp_repo("driver-gone");
    let gate = dir.join("gate-never");
    let config = gated_config(&dir, &gate);
    let db = dir.join("state.sqlite");
    let _reaper = Reaper { db: db.clone() };
    let plan = dir.join("plan.json");
    fs::write(
        &plan,
        r#"{"graph_id":"ggone","nodes":[{"id":1,"description":"never runs","depends_on":[],"tier":null}]}"#,
    )
    .unwrap();
    let run = meshloop(&dir)
        .env("MESHLOOP_DRIVER_PANIC", "1")
        .args(["run", "--plan"])
        .arg(&plan)
        .args([
            "--accept-plan",
            "--fixture-only",
            "--detach",
            "--json",
            "--config",
        ])
        .arg(&config)
        .arg("--db")
        .arg(&db)
        .arg("--worktree-base")
        .arg(dir.join("worktrees"))
        .output()
        .expect("run --detach");
    assert!(run.status.success(), "{run:?}");

    let (out, killed) = bounded_output(
        &mut watch_cmd(&dir, &config, &db, "ggone"),
        std::time::Duration::from_secs(60),
    );
    let lines = ndjson(&out);
    assert!(!killed, "watch never noticed the gone driver: {lines:?}");
    let exit = lines.last().expect("exit line");
    assert_eq!(exit["exit"], "driver_gone", "{lines:?}");
    assert_eq!(exit["nodes"], serde_json::json!([1]), "{lines:?}");
    let driver = exit["driver"].as_str().unwrap_or("");
    assert!(
        driver == "lost" || driver.starts_with("exited("),
        "{lines:?}"
    );
    assert!(exit["driver_log"].as_str().is_some(), "{lines:?}");
    assert_eq!(out.status.code(), Some(4), "{out:?}");

    let _ = fs::remove_dir_all(&dir);
}

/// `meshloop <args> --config <c> --db <d>` as parsed JSON stdout, or `Null`.
fn cli_json(dir: &Path, config: &Path, db: &Path, args: &[&str]) -> serde_json::Value {
    let out = meshloop(dir)
        .args(args)
        .arg("--config")
        .arg(config)
        .arg("--db")
        .arg(db)
        .output()
        .expect("meshloop");
    serde_json::from_slice(&out.stdout).unwrap_or(serde_json::Value::Null)
}

/// Polls `status` until `pred` holds, within 60s.
fn wait_status(
    dir: &Path,
    config: &Path,
    db: &Path,
    graph: &str,
    what: &str,
    pred: impl Fn(&serde_json::Value) -> bool,
) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let mut status = serde_json::Value::Null;
    while std::time::Instant::now() < deadline {
        status = cli_json(dir, config, db, &["status", "--graph", graph, "--json"]);
        if pred(&status["data"]) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("status never showed {what}: {status}");
}

fn node_state(data: &serde_json::Value, id: u32) -> String {
    data["nodes"]
        .as_array()
        .and_then(|nodes| nodes.iter().find(|n| n["id"] == id))
        .and_then(|n| n["state"].as_str())
        .unwrap_or("")
        .to_string()
}

/// Detached run of `graph`: node 1 is fast, node 2 is GATED and depends on it.
/// Returns once node 1 awaits review, the detached driver has exited, and node 1
/// is accepted: the run then needs a new driver to merge node 1 and run node 2.
fn paused_after_accept(dir: &Path, config: &Path, db: &Path, graph: &str) {
    let plan = dir.join("plan.json");
    fs::write(
        &plan,
        format!(
            r#"{{"graph_id":"{graph}","nodes":[{{"id":1,"description":"fast first","depends_on":[],"tier":null}},{{"id":2,"description":"GATED second","depends_on":[1],"tier":null}}]}}"#
        ),
    )
    .unwrap();
    let run = meshloop(dir)
        .args(["run", "--plan"])
        .arg(&plan)
        .args([
            "--accept-plan",
            "--fixture-only",
            "--detach",
            "--json",
            "--config",
        ])
        .arg(config)
        .arg("--db")
        .arg(db)
        .arg("--worktree-base")
        .arg(dir.join("worktrees"))
        .output()
        .expect("run --detach");
    assert!(run.status.success(), "{run:?}");
    wait_status(dir, config, db, graph, "an exited driver", |d| {
        node_state(d, 1) == "AwaitingReview"
            && d["driver"]
                .as_str()
                .is_some_and(|l| l.starts_with("exited("))
    });
    let accept = meshloop(dir)
        .args(["accept", "--task", "1", "--as", "tester", "--graph", graph])
        .arg("--config")
        .arg(config)
        .arg("--db")
        .arg(db)
        .output()
        .expect("accept");
    assert!(accept.status.success(), "{accept:?}");
    // Nothing drives the run now: watch says so instead of blocking.
    let (out, killed) = bounded_output(
        &mut watch_cmd(dir, config, db, graph),
        std::time::Duration::from_secs(30),
    );
    let lines = ndjson(&out);
    assert!(!killed, "{lines:?}");
    assert_eq!(lines.last().unwrap()["exit"], "driver_gone", "{lines:?}");
}

/// Review #195-1: a foreground `resume` drives the graph after the detached
/// driver exited. Its old `exited(0)` row must not make `watch` exit
/// `driver_gone` while the foreground resume is making progress.
#[test]
fn watch_does_not_report_driver_gone_while_a_foreground_resume_drives() {
    let dir = temp_repo("foreground-resume");
    let gate = dir.join("gate-open");
    let config = gated_config(&dir, &gate);
    let db = dir.join("state.sqlite");
    let _reaper = Reaper { db: db.clone() };
    paused_after_accept(&dir, &config, &db, "gfore");

    let mut resume = meshloop(&dir);
    resume
        .args(["resume", "--graph", "gfore", "--fixture-only", "--config"])
        .arg(&config)
        .arg("--db")
        .arg(&db)
        .arg("--worktree-base")
        .arg(dir.join("worktrees"))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let mut resume = meshloop_adapters::process::spawn_command(&mut resume).expect("resume");
    wait_status(&dir, &config, &db, "gfore", "node 2 running", |d| {
        node_state(d, 2) == "Running"
    });

    let limit = std::time::Duration::from_secs(60);
    let mut watch = Streamed::spawn(&mut watch_cmd(&dir, &config, &db, "gfore"));
    let started = watch.until(|l| l["node"] == 2 && l["to"] == "Running", limit);
    fs::write(&gate, "open").unwrap();
    started.unwrap_or_else(|e| panic!("watch must follow the foreground resume: {e}"));
    let (lines, code) = watch
        .finish(limit)
        .unwrap_or_else(|e| panic!("watch never saw node 2's review: {e}"));
    let exit = lines.last().expect("exit line");
    assert_eq!(exit["exit"], "awaiting_review", "{lines:?}");
    assert_eq!(exit["nodes"], serde_json::json!([2]), "{lines:?}");
    assert_eq!(code, Some(0), "{lines:?}");

    let deadline = std::time::Instant::now() + limit;
    while resume.try_wait().expect("resume wait").is_none() {
        assert!(
            std::time::Instant::now() < deadline,
            "foreground resume never paused"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let _ = fs::remove_dir_all(&dir);
}

/// Review #195-2: `resume --detach` returns before its driver registers. Until
/// it does, the previous driver's `exited(0)` row must not read as
/// `driver_gone`, however long registration takes.
#[test]
fn watch_waits_for_a_resume_detach_driver_that_has_not_registered_yet() {
    let dir = temp_repo("resume-detach");
    let gate = dir.join("gate-open");
    let config = gated_config(&dir, &gate);
    let db = dir.join("state.sqlite");
    let _reaper = Reaper { db: db.clone() };
    paused_after_accept(&dir, &config, &db, "gredetach");

    let resume = meshloop(&dir)
        .env("MESHLOOP_DRIVER_REGISTER_DELAY_MS", "4000")
        .args([
            "resume",
            "--detach",
            "--graph",
            "gredetach",
            "--fixture-only",
            "--json",
            "--config",
        ])
        .arg(&config)
        .arg("--db")
        .arg(&db)
        .arg("--worktree-base")
        .arg(dir.join("worktrees"))
        .output()
        .expect("resume --detach");
    assert!(resume.status.success(), "{resume:?}");

    let limit = std::time::Duration::from_secs(60);
    let mut watch = Streamed::spawn(&mut watch_cmd(&dir, &config, &db, "gredetach"));
    let started = watch.until(|l| l["node"] == 2 && l["to"] == "Running", limit);
    fs::write(&gate, "open").unwrap();
    started.unwrap_or_else(|e| panic!("watch must wait for the new driver: {e}"));
    let (lines, code) = watch
        .finish(limit)
        .unwrap_or_else(|e| panic!("watch never saw node 2's review: {e}"));
    let exit = lines.last().expect("exit line");
    assert_eq!(exit["exit"], "awaiting_review", "{lines:?}");
    assert_eq!(exit["nodes"], serde_json::json!([2]), "{lines:?}");
    assert_eq!(code, Some(0), "{lines:?}");
    let _ = fs::remove_dir_all(&dir);
}
