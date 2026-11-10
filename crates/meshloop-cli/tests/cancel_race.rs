//! Issue #167: `cancel` raced the driver. The driver recorded the killed worker's
//! check result after `cancel` had recorded `Cancelled`, which left the illegal pair
//! `Cancelled` + `DeterministicChecksFailed` in the ledger and made `status` fail with
//! `Store(Corrupt(..))`.
//!
//! The window is opened deterministically, without timing: the verify command is
//! `meshloop cancel` itself. It runs after the driver recorded `HarnessExited` and
//! before the driver records the check result, so the cancel always lands between
//! the worker's exit and the driver's check append.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn guarded_output(cmd: &mut Command) -> Output {
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    meshloop_adapters::process::spawn_command(cmd)
        .expect("spawn")
        .wait_with_output()
        .expect("wait")
}

fn fixture_path() -> String {
    let exe = format!("fixture_harness{}", std::env::consts::EXE_SUFFIX);
    let mut dir = std::env::current_exe().expect("current test executable path");
    dir.pop();
    dir.pop();
    let path = dir.join(&exe);
    assert!(
        path.exists(),
        "expected fixture harness at {}; build meshloop-adapters first",
        path.display()
    );
    path.to_string_lossy().to_string()
}

fn git(dir: &Path, args: &[&str]) {
    let out = guarded_output(Command::new("git").current_dir(dir).args(args));
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

fn disposable_repo(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("meshloop-cancel-race-{tag}-{}", std::process::id()));
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

fn toml_str(value: &Path) -> String {
    value.to_string_lossy().replace('\\', "\\\\")
}

struct Run {
    dir: PathBuf,
    config: PathBuf,
    db: PathBuf,
    worktrees: PathBuf,
    graph_id: String,
}

impl Run {
    fn new(tag: &str) -> Self {
        let dir = disposable_repo(tag);
        let config = dir.join("meshloop.toml");
        let db = dir.join("meshloop.db");
        let worktrees = dir.join("worktrees");
        let graph_id = format!("g167{tag}");
        let fixture = fixture_path().replace('\\', "\\\\");
        let meshloop = toml_str(Path::new(env!("CARGO_BIN_EXE_meshloop")));
        // The verify command is the cancel. `--noop` leaves an empty diff, so the
        // checks fail and the driver goes on to record `DeterministicChecksFailed`.
        let contents = format!(
            r#"
selected_harnesses = ["fixture"]

[limits]
max_concurrent_workers = 1
max_retries = 2
task_timeout_seconds = 60

[verify]
verify_command = ["{meshloop}", "cancel", "--graph", "{graph_id}", "--config", "{config}", "--db", "{db}", "--worktree-base", "{worktrees}"]

[harnesses.fixture]
executable = "{fixture}"
version_args = ["--version"]
invoke_args_template = ["--noop"]
model_ref = "fixture-model"
model_tier = "top"
"#,
            config = toml_str(&config),
            db = toml_str(&db),
            worktrees = toml_str(&worktrees),
        );
        fs::write(&config, contents).unwrap();
        Self {
            dir,
            config,
            db,
            worktrees,
            graph_id,
        }
    }

    fn meshloop(&self, args: &[&str]) -> Output {
        guarded_output(
            Command::new(env!("CARGO_BIN_EXE_meshloop"))
                .current_dir(&self.dir)
                .args(args)
                .arg("--config")
                .arg(&self.config)
                .arg("--db")
                .arg(&self.db),
        )
    }

    fn events(&self) -> Vec<(String, String, String)> {
        let conn = rusqlite::Connection::open(&self.db).expect("open db");
        let mut stmt = conn
            .prepare(
                "SELECT from_state, to_state, event_type FROM events
                 WHERE graph_id = ?1 AND task_id = 1 ORDER BY event_id",
            )
            .unwrap();
        stmt.query_map([&self.graph_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    }

    fn attempt_pids(&self) -> Vec<Option<i64>> {
        let conn = rusqlite::Connection::open(&self.db).expect("open db");
        let mut stmt = conn
            .prepare("SELECT pid FROM attempts WHERE graph_id = ?1")
            .unwrap();
        stmt.query_map([&self.graph_id], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn cancel_between_worker_exit_and_check_append_leaves_a_legal_readable_ledger() {
    let run = Run::new("window");
    let plan = run.dir.join("plan.json");
    fs::write(
        &plan,
        format!(
            r#"{{"graph_id":"{}","nodes":[{{"id":1,"description":"noop","depends_on":[],"tier":null}}]}}"#,
            run.graph_id
        ),
    )
    .unwrap();

    let driver = run.meshloop(&[
        "run",
        "--plan",
        plan.to_str().unwrap(),
        "--accept-plan",
        "--worktree-base",
        run.worktrees.to_str().unwrap(),
    ]);
    let driver_out = format!(
        "{}{}",
        String::from_utf8_lossy(&driver.stdout),
        String::from_utf8_lossy(&driver.stderr)
    );
    let events = run.events();

    // The cancel really ran inside the window: it is recorded after the exit.
    let kinds: Vec<&str> = events.iter().map(|(_, _, e)| e.as_str()).collect();
    let exited = kinds
        .iter()
        .position(|e| *e == "HarnessExited")
        .unwrap_or_else(|| panic!("no HarnessExited: {events:?}\n{driver_out}"));
    let cancel = kinds
        .iter()
        .position(|e| *e == "Cancel")
        .unwrap_or_else(|| panic!("cancel did not run in the window: {events:?}\n{driver_out}"));
    assert!(exited < cancel, "{events:?}");

    // Legal ledger: `Cancelled` is the last state and nothing follows it.
    assert_eq!(
        events.last().map(|(_, to, e)| (to.as_str(), e.as_str())),
        Some(("Cancelled", "Cancel")),
        "nothing may be appended after Cancelled: {events:?}\n{driver_out}"
    );
    // Every row continues from the state the previous row left.
    for pair in events.windows(2) {
        assert_eq!(pair[0].1, pair[1].0, "broken chain: {events:?}");
    }
    // The driver treats the refused append as "cancelled, stop": it ends idle on a
    // terminal graph (exit 1 is `FailedTerminal`), not with a store error.
    assert!(
        !driver_out.contains("Corrupt") && !driver_out.contains("run failed"),
        "the driver must stop cleanly: {driver_out}"
    );
    assert_eq!(driver.status.code(), Some(1), "{driver_out}");

    // `status` reads the graph.
    let status = run.meshloop(&["status", "--json", "--graph", &run.graph_id]);
    let stdout = String::from_utf8_lossy(&status.stdout);
    assert!(
        status.status.success(),
        "status failed: {stdout}{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).expect("status json");
    assert_eq!(json["ok"], true, "{json}");
    let text = json.to_string();
    assert!(!text.contains("Corrupt"), "{text}");
    assert!(text.contains("Cancelled"), "{text}");

    // No worker is left: every recorded worker process is gone.
    let processes = meshloop_adapters::process::HostProcessView {};
    for pid in run.attempt_pids().into_iter().flatten() {
        let live = meshloop_engine::ports::ProcessView::is_live(
            &processes,
            &meshloop_engine::ports::ProcessHint {
                pid: pid as u32,
                image_name: None,
            },
        );
        assert_ne!(
            live,
            meshloop_engine::ports::LiveCheck::Live,
            "worker {pid} still running"
        );
    }
}

fn plan_file(run: &Run) -> PathBuf {
    let plan = run.dir.join("plan.json");
    fs::write(
        &plan,
        format!(
            r#"{{"graph_id":"{}","nodes":[{{"id":1,"description":"noop","depends_on":[],"tier":null}}]}}"#,
            run.graph_id
        ),
    )
    .unwrap();
    plan
}

#[test]
fn a_ledger_written_before_the_fix_reads_as_cancelled() {
    // Issue #167 remedy: graphs already holding `Cancelled` + the cancelled attempt's
    // stale check result stay readable, and the task stays cancelled.
    let run = Run::new("legacy");
    let plan = plan_file(&run);
    run.meshloop(&[
        "run",
        "--plan",
        plan.to_str().unwrap(),
        "--accept-plan",
        "--worktree-base",
        run.worktrees.to_str().unwrap(),
    ]);
    let conn = rusqlite::Connection::open(&run.db).unwrap();
    // The exact rows the pre-fix driver appended after the cancel.
    conn.execute(
        "INSERT INTO events (graph_id, task_id, attempt_id, from_state, to_state, event_type, executor, occurred_at)
         VALUES (?1, 1, 1, 'Verifying', 'Failed', 'DeterministicChecksFailed', 'meshloop', '0')",
        [&run.graph_id],
    )
    .unwrap();
    drop(conn);
    let last = run.events().last().cloned().unwrap();
    assert_eq!(last.2, "DeterministicChecksFailed");

    let status = run.meshloop(&["status", "--json", "--graph", &run.graph_id]);
    let stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status.status.success(), "status failed: {stdout}");
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).expect("status json");
    let text = json.to_string();
    assert!(!text.contains("Corrupt"), "{text}");
    assert!(text.contains("Cancelled"), "{text}");

    // Resuming does not drive the cancelled task again and does not fail.
    let resume = run.meshloop(&[
        "resume",
        "--graph",
        &run.graph_id,
        "--worktree-base",
        run.worktrees.to_str().unwrap(),
    ]);
    let out = format!(
        "{}{}",
        String::from_utf8_lossy(&resume.stdout),
        String::from_utf8_lossy(&resume.stderr)
    );
    assert!(
        !out.contains("Corrupt") && !out.contains("resume failed"),
        "{out}"
    );
    assert_eq!(run.events().last().cloned().unwrap(), last, "{out}");
}

mod stale_cancel {
    //! `cancel` read `Running`; the driver recorded the worker's exit before the
    //! `Cancel` committed. The store refuses the stale `Cancel` and `cancel` re-reads,
    //! so it cancels from `Verifying` and never writes over a state it did not see.

    use std::cell::Cell;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::time::Duration;

    use meshloop_adapters::store::SqliteStore;
    use meshloop_domain::capability::{Compatibility, HarnessError, HarnessProfile};
    use meshloop_domain::evidence::AttemptId;
    use meshloop_domain::state::{Event, PlanState, TaskState};
    use meshloop_domain::task_graph::TaskId;
    use meshloop_engine::agent::AgentSpec;
    use meshloop_engine::ports::{
        AttemptRow, EventLog, HarnessCapabilities, HarnessHandle, HarnessOutcome, LiveCheck,
        ProcessHint, ProcessView, RunRow, RunStore, TransitionRecord,
    };
    use meshloop_engine::run_loop::{RunLimits, RunLoop};

    struct Worker {
        stopped: Mutex<Vec<HarnessHandle>>,
    }

    impl HarnessCapabilities for Worker {
        fn probe(&self) -> Result<HarnessProfile, HarnessError> {
            Ok(HarnessProfile {
                harness: "h".into(),
                version: "0".into(),
                compatibility: Compatibility::Compatible,
                supports_noninteractive: true,
                supports_structured_output: true,
                supports_cancellation: true,
                reason: None,
            })
        }
        fn invoke(&self, _spec: &AgentSpec) -> Result<HarnessHandle, HarnessError> {
            Err(HarnessError::Unsupported)
        }
        fn cancel(&self, handle: &HarnessHandle) -> Result<(), HarnessError> {
            self.stopped.lock().unwrap().push(handle.clone());
            Ok(())
        }
        fn collect(&self, _handle: &HarnessHandle) -> Result<HarnessOutcome, HarnessError> {
            Err(HarnessError::Unsupported)
        }
    }

    /// Reports the worker live once; on that probe, the driver (its own connection)
    /// records the exit the kill caused, before `cancel` appends.
    struct DriverRecordsExit {
        db: PathBuf,
        graph_id: String,
        fired: Cell<bool>,
    }

    impl ProcessView for DriverRecordsExit {
        fn is_live(&self, _hint: &ProcessHint) -> LiveCheck {
            if self.fired.replace(true) {
                return LiveCheck::Dead;
            }
            let mut driver = SqliteStore::open(&self.db).expect("driver connection");
            driver
                .append(record(
                    &self.graph_id,
                    TaskState::Running,
                    Event::HarnessExited,
                    TaskState::Verifying,
                ))
                .expect("driver records the exit");
            LiveCheck::Live
        }
    }

    fn record(graph_id: &str, from: TaskState, event: Event, to: TaskState) -> TransitionRecord {
        TransitionRecord {
            graph_id: graph_id.into(),
            task_id: TaskId(1),
            attempt_id: Some(AttemptId(1)),
            from,
            to,
            event,
            reason: None,
            executor: "meshloop".into(),
            occurred_at: "1".into(),
        }
    }

    #[test]
    fn cancel_refused_as_stale_rereads_and_cancels_from_the_new_state() {
        let dir =
            std::env::temp_dir().join(format!("meshloop-cancel-race-stale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("state.sqlite");
        let graph_id = "g167stale".to_string();
        let mut store = SqliteStore::open(&db).unwrap();
        store
            .save_run_and_events(
                &RunRow {
                    graph_id: graph_id.clone(),
                    plan_state: PlanState::PlanAccepted,
                    run_base: "base".into(),
                    integrate_ref: format!("meshloop/{graph_id}/integrate"),
                    plan_json: format!(
                        r#"{{"graph_id":"{graph_id}","nodes":[{{"id":1,"description":"x","depends_on":[],"tier":null}}]}}"#
                    ),
                    plan_id: "p".into(),
                    created_at: "1".into(),
                    review_note: None,
                },
                &[
                    record(&graph_id, TaskState::Pending, Event::DependencySatisfied, TaskState::Ready),
                    record(&graph_id, TaskState::Ready, Event::AttemptStarted, TaskState::Running),
                ],
            )
            .unwrap();
        store
            .save_attempt(&AttemptRow {
                attempt_id: AttemptId(1),
                graph_id: graph_id.clone(),
                task_id: TaskId(1),
                harness: Some("h".into()),
                model_ref: Some("m".into()),
                worktree_path: None,
                pid: Some(4_000_000),
                image_name: None,
                pane_id: None,
                started_at: Some("1".into()),
                ended_at: None,
                outcome: None,
            })
            .unwrap();

        let worker = Worker {
            stopped: Mutex::new(Vec::new()),
        };
        let processes = DriverRecordsExit {
            db: db.clone(),
            graph_id: graph_id.clone(),
            fired: Cell::new(false),
        };
        let workspace = meshloop_adapters::git::GitWorktreeAdapter::new(dir.clone());
        let checks = meshloop_adapters::check::CommandCheckRunner;
        let mut saga = RunLoop {
            harnesses: HashMap::from([("h".to_string(), &worker as &dyn HarnessCapabilities)]),
            candidates: vec![],
            workspace: &workspace,
            store: &mut store,
            processes: &processes,
            checks: &checks,
            router: Default::default(),
            limits: RunLimits {
                max_retries: 1,
                task_timeout: Duration::from_secs(1),
                max_concurrent_workers: 1,
            },
            fixture_only: false,
            verify_command: vec![],
            worktree_base: dir.join("worktrees"),
            active_graph: None,
            planner: Default::default(),
            pending_outcomes: HashMap::new(),
            micro_repair: None,
            ownership: None,
        };
        saga.cancel(&graph_id, None).expect("cancel");
        let status = saga.status(&graph_id).expect("status reads the graph");
        drop(saga);

        assert_eq!(status.nodes[0].state, TaskState::Cancelled);
        assert_eq!(
            worker.stopped.lock().unwrap().len(),
            1,
            "worker stopped once"
        );
        let rows: Vec<(TaskState, TaskState, Event)> = store
            .records_for_graph(&graph_id)
            .unwrap()
            .into_iter()
            .map(|r| (r.from, r.to, r.event))
            .collect();
        assert_eq!(
            rows,
            vec![
                (
                    TaskState::Pending,
                    TaskState::Ready,
                    Event::DependencySatisfied
                ),
                (TaskState::Ready, TaskState::Running, Event::AttemptStarted),
                (
                    TaskState::Running,
                    TaskState::Verifying,
                    Event::HarnessExited
                ),
                (TaskState::Verifying, TaskState::Cancelled, Event::Cancel),
            ]
        );
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
