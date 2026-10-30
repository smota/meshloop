//! Bounded ADR 0034 CLI observation proofs, not the complete S3 lifecycle oracle.
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use meshloop_adapters::store::SqliteStore;
use meshloop_adapters::workspace::GitRepositoryIdentity;
use meshloop_domain::evidence::AttemptId;
use meshloop_domain::state::PlanState;
use meshloop_domain::task_graph::TaskId;
use meshloop_domain::workspace::RootAlias;
use meshloop_engine::ports::{AttemptRow, RunRow, RunStore};
use meshloop_engine::workspace::{RepositoryFence, RepositoryIdentity};

struct Fixture {
    base: PathBuf,
    repo: PathBuf,
    db: PathBuf,
}
impl Fixture {
    fn new(tag: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "meshloop-cli-read-{tag}-{}-{nonce}",
            std::process::id()
        ));
        let repo = base.join("repo");
        fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "--quiet"]);
        fs::write(repo.join("tracked.txt"), "committed seed\n").unwrap();
        git(&repo, &["add", "tracked.txt"]);
        git(
            &repo,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "--quiet",
                "-m",
                "seed",
            ],
        );
        fs::write(
            repo.join("meshloop.toml"),
            r#"
selected_harnesses = []
[limits]
max_concurrent_workers = 1
max_retries = 1
task_timeout_seconds = 10
"#,
        )
        .unwrap();
        let db = repo.join(".meshloop/state.sqlite");
        Self { base, repo, db }
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_meshloop"));
        command
            .current_dir(&self.repo)
            .args(args)
            .arg("--json")
            .arg("--config")
            .arg(self.repo.join("meshloop.toml"))
            .arg("--db")
            .arg(&self.db);
        command
    }
    fn read(&self, args: &[&str], code: i32) -> serde_json::Value {
        let out = bounded_output(&mut self.command(args));
        assert_eq!(out.status.code(), Some(code), "{out:?}");
        serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("{e}: {out:?}"))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        assert!(self.base.starts_with(std::env::temp_dir()));
        assert!(
            self.base
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("meshloop-cli-read-")
        );
        let _ = fs::remove_dir_all(&self.base);
    }
}
// These fixtures emit small JSON responses. A blocked reader (for example one
// accidentally trying to acquire the writer fence) must fail within a deadline.
fn bounded_output(command: &mut Command) -> Output {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!("subprocess exceeded 10-second deadline: {command:?}: {output:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn git(repo: &Path, args: &[&str]) -> Vec<u8> {
    let out = bounded_output(Command::new("git").current_dir(repo).args(args));
    assert!(out.status.success(), "{out:?}");
    out.stdout
}
// SQLite owns sidecar coordination under ADR 0034. Every other file byte and
// directory entry, including DB, Git index/refs, registry and roots, must match.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    snapshot_except_locked_registry(root, None)
}
fn snapshot_except_locked_registry(
    root: &Path,
    locked_registry: Option<&Path>,
) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn visit(
        root: &Path,
        path: &Path,
        locked_registry: Option<&Path>,
        out: &mut BTreeMap<PathBuf, Option<Vec<u8>>>,
    ) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == "state.sqlite-wal" || name == "state.sqlite-shm" {
                continue;
            }
            // Windows denies reading the exclusively held registry; compare its
            // bytes separately across the held-fence observation window.
            if locked_registry == Some(path.as_path()) {
                // Preserve entry presence; only its locked bytes are exempt.
                out.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    Some(Vec::new()),
                );
                continue;
            }
            if entry.file_type().unwrap().is_dir() {
                out.insert(path.strip_prefix(root).unwrap().to_path_buf(), None);
                visit(root, &path, locked_registry, out);
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    Some(fs::read(&path).unwrap()),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, locked_registry, &mut out);
    out
}
fn row(repo: &Path) -> RunRow {
    RunRow {
        graph_id: "only-in-live-wal".into(),
        plan_state: PlanState::AwaitingPlanReview,
        run_base: String::from_utf8(git(repo, &["rev-parse", "HEAD"])).unwrap().trim().into(),
        integrate_ref: "unused".into(),
        plan_json: r#"{"graph_id":"only-in-live-wal","nodes":[{"id":1,"description":"visible from committed WAL","depends_on":[],"tier":null}]}"#.into(),
        plan_id: "test-plan".into(),
        created_at: "1".into(),
        review_note: None,
    }
}
fn seed(store: &mut SqliteStore, repo: &Path, worktree: &Path) {
    store.save_run(&row(repo)).unwrap();
    store
        .save_attempt(&AttemptRow {
            attempt_id: AttemptId(1),
            graph_id: "only-in-live-wal".into(),
            task_id: TaskId(1),
            harness: None,
            model_ref: None,
            worktree_path: Some(worktree.into()),
            pid: None,
            image_name: None,
            pane_id: None,
            started_at: None,
            ended_at: None,
            outcome: None,
        })
        .unwrap();
}
fn assert_watch(out: Output) {
    assert!(out.status.success(), "{out:?}");
    let lines: Vec<serde_json::Value> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(
        lines
            .iter()
            .any(|line| line["graph_id"] == "only-in-live-wal"),
        "{lines:?}"
    );
    assert_eq!(lines.last().unwrap()["exit"], "awaiting_plan_acceptance");
}

#[test]
fn absent_ledger_reads_create_nothing_and_targeted_reads_exit_two() {
    let mut fixture = Fixture::new("absent");
    // Exercise both the default location and a custom path with missing parents.
    for db in [
        fixture.db.clone(),
        fixture.base.join("missing/nested/custom.sqlite"),
    ] {
        fixture.db = db;
        let before = snapshot(&fixture.base);
        let status = fixture.read(&["status"], 0);
        assert_eq!(status["data"]["runs"], serde_json::json!([]));
        for command in ["status", "inspect", "watch"] {
            fixture.read(&[command, "--graph", "missing"], 2);
        }
        for command in ["inspect", "watch"] {
            fixture.read(&[command], 2);
        }
        assert_eq!(snapshot(&fixture.base), before);
    }
}

#[test]
fn existing_empty_ledger_targeted_reads_exit_two_without_maintenance() {
    let fixture = Fixture::new("empty");
    let store = SqliteStore::open(&fixture.db).unwrap();
    let before = snapshot(&fixture.base);
    assert_eq!(
        fixture.read(&["status"], 0)["data"]["runs"],
        serde_json::json!([])
    );
    for command in ["status", "inspect", "watch"] {
        fixture.read(&[command, "--graph", "missing"], 2);
    }
    assert_eq!(snapshot(&fixture.base), before);
    drop(store);
}

#[test]
fn public_cli_observes_live_wal_while_fence_held_without_application_or_git_writes() {
    let fixture = Fixture::new("live-wal");
    let mut store = SqliteStore::open(&fixture.db).unwrap();
    let mut fence = GitRepositoryIdentity::new("git")
        .acquire(&fixture.repo, &fixture.db)
        .unwrap();
    let root = fixture.base.join("owned-root");
    fs::create_dir(&root).unwrap();
    fence
        .register_root(RootAlias::new("worktrees").unwrap(), &root)
        .unwrap();
    fs::write(root.join("keep.txt"), "root contents retained").unwrap();
    let wt = root.join("attempt");
    git(
        &fixture.repo,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "attempt-read-proof",
            wt.to_str().unwrap(),
        ],
    );
    fs::write(wt.join("tracked.txt"), "staged change\n").unwrap();
    git(&wt, &["add", "tracked.txt"]);
    fs::write(wt.join("tracked.txt"), "staged and unstaged change\n").unwrap();
    fs::write(wt.join("untracked.txt"), "retain untracked\n").unwrap();
    let head = git(&wt, &["rev-parse", "HEAD"]);
    let dirty = git(
        &wt,
        &["--no-optional-locks", "status", "--porcelain", "-uall"],
    );
    seed(&mut store, &fixture.repo, &wt);
    drop(fence);
    let registry = fixture.repo.join(".git/meshloop-workspace-registry.json");
    let registry_before = fs::read(&registry).unwrap();
    let fence = GitRepositoryIdentity::new("git")
        .acquire(&fixture.repo, &fixture.db)
        .unwrap();
    // Keep the writer alive so the newly committed run has not checkpointed to DB.
    assert!(
        !fs::read(&fixture.db)
            .unwrap()
            .windows(b"only-in-live-wal".len())
            .any(|w| w == b"only-in-live-wal")
    );
    assert!(PathBuf::from(format!("{}-wal", fixture.db.display())).exists());
    let before = snapshot_except_locked_registry(&fixture.base, Some(&registry));
    let status = fixture.read(&["status", "--graph", "only-in-live-wal"], 0);
    assert_eq!(
        status["data"]["nodes"][0]["description"],
        "visible from committed WAL"
    );
    let inspect = fixture.read(&["inspect", "--graph", "only-in-live-wal", "--attempts"], 0);
    assert_eq!(
        inspect["data"]["git_export"]["commit_sha"],
        String::from_utf8(head.clone())
            .unwrap()
            .trim()
            .to_lowercase()
    );
    assert_eq!(
        fixture.read(
            &["inspect", "--graph", "only-in-live-wal", "--task", "1"],
            0
        )["data"]["task_id"],
        1
    );
    assert_watch(bounded_output(&mut fixture.command(&[
        "watch",
        "--graph",
        "only-in-live-wal",
        "--timeout",
        "2",
        "--interval",
        "1",
    ])));
    assert_eq!(git(&wt, &["rev-parse", "HEAD"]), head);
    assert_eq!(
        git(
            &wt,
            &["--no-optional-locks", "status", "--porcelain", "-uall"]
        ),
        dirty
    );
    assert_eq!(
        snapshot_except_locked_registry(&fixture.base, Some(&registry)),
        before
    );
    fence.validate().unwrap();
    drop(fence);
    assert_eq!(fs::read(&registry).unwrap(), registry_before);
    drop(store);
}

#[test]
fn closed_wal_ledger_is_readable_without_identity_or_gitignore_creation() {
    let fixture = Fixture::new("closed");
    {
        let mut store = SqliteStore::open(&fixture.db).unwrap();
        seed(&mut store, &fixture.repo, &fixture.repo);
    }
    let before = snapshot(&fixture.base);
    assert_eq!(
        fixture.read(&["status"], 0)["data"]["graph_id"],
        "only-in-live-wal"
    );
    fixture.read(&["inspect", "--graph", "only-in-live-wal"], 0);
    assert_watch(bounded_output(&mut fixture.command(&[
        "watch",
        "--graph",
        "only-in-live-wal",
        "--timeout",
        "2",
    ])));
    assert_eq!(snapshot(&fixture.base), before);
}

#[test]
fn pre_v6_public_reads_require_upgrade_without_migrating_application_state() {
    let fixture = Fixture::new("pre-v6");
    fs::create_dir_all(fixture.db.parent().unwrap()).unwrap();
    let conn = rusqlite::Connection::open(&fixture.db).unwrap();
    conn.execute_batch("CREATE TABLE schema_meta(version INTEGER NOT NULL); INSERT INTO schema_meta VALUES(5); CREATE TABLE sentinel(value TEXT); INSERT INTO sentinel VALUES('retained');").unwrap();
    drop(conn);
    let before = snapshot(&fixture.base);
    for command in ["status", "inspect", "watch"] {
        let value = fixture.read(&[command, "--graph", "old"], 2);
        assert!(
            value.to_string().contains("SchemaUpgradeRequired"),
            "{value}"
        );
    }
    assert_eq!(snapshot(&fixture.base), before);
    let conn = rusqlite::Connection::open_with_flags(
        &fixture.db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        conn.query_row("SELECT version FROM schema_meta", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        5
    );
    assert_eq!(
        conn.query_row("SELECT value FROM sentinel", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "retained"
    );
}
