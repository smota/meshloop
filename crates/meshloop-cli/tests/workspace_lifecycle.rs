//! Read-only workspace inventory. Proves reason codes and that the command does not write.
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use meshloop_adapters::store::SqliteStore;
use meshloop_adapters::workspace::GitRepositoryIdentity;
use meshloop_domain::digest::fnv1a64;
use meshloop_domain::state::PlanState;
use meshloop_domain::workspace::{
    DriverPhase, DriverRecord, DriverTarget, ExecutionScope, GenerationRecord, GraphId,
    LifecycleId, PlanHash, ProcessIdentity, RelativePath, RootAlias,
};
use meshloop_engine::ports::{RunRow, RunStore};
use meshloop_engine::workspace::{
    LifecycleAccess, RepositoryFence, RepositoryIdentity, WorkspaceLedger,
};

struct Fixture {
    base: PathBuf,
    repo: PathBuf,
    db: PathBuf,
    config: PathBuf,
}
impl Fixture {
    fn new(tag: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "meshloop-cli-inventory-{tag}-{}-{nonce}",
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
        let config = repo.join("meshloop.toml");
        fs::write(
            &config,
            r#"
selected_harnesses = []
[limits]
max_concurrent_workers = 1
max_retries = 1
task_timeout_seconds = 10
"#,
        )
        .unwrap();
        let db = repo.join(".meshloop").join("state.sqlite");
        Self {
            base,
            repo,
            db,
            config,
        }
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_meshloop"));
        command
            .current_dir(&self.repo)
            .args(args)
            .arg("--json")
            .arg("--config")
            .arg(&self.config)
            .arg("--db")
            .arg(&self.db);
        command
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
                .starts_with("meshloop-cli-inventory-")
        );
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn bounded_output(command: &mut Command) -> Output {
    let mut child = meshloop_adapters::process::spawn_command(
        command.stdout(Stdio::piped()).stderr(Stdio::piped()),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!("subprocess exceeded 20-second deadline: {command:?}: {output:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn git(repo: &Path, args: &[&str]) -> Vec<u8> {
    let out = bounded_output(Command::new("git").current_dir(repo).args(args));
    assert!(out.status.success(), "{out:?}");
    out.stdout
}

fn run(fixture: &Fixture, args: &[&str]) -> (i32, serde_json::Value, String) {
    let out = bounded_output(&mut fixture.command(args));
    let stdout = String::from_utf8(out.stdout).unwrap();
    let code = out.status.code().unwrap_or(127);
    assert!(
        !stdout.contains(&fixture.base.display().to_string()),
        "inventory leaked an absolute path: {stdout}"
    );
    if code == 0 {
        let value = serde_json::from_str(&stdout).unwrap_or_else(|error| {
            panic!(
                "{error}: {stdout} stderr {}",
                String::from_utf8_lossy(&out.stderr)
            )
        });
        (code, value, stdout)
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        (code, serde_json::Value::Null, format!("{stdout}{stderr}"))
    }
}

/// SQLite WAL/SHM sidecars are ADR 0034 coordination, not inventory writes.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn visit(root: &Path, path: &Path, out: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == "state.sqlite-wal" || name == "state.sqlite-shm" {
                continue;
            }
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            if entry.file_type().unwrap().is_dir() {
                out.insert(relative, None);
                visit(root, &path, out);
            } else {
                out.insert(relative, Some(fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}

fn fingerprint(map: &BTreeMap<PathBuf, Option<Vec<u8>>>) -> String {
    let mut bytes = Vec::new();
    for (path, body) in map {
        bytes.extend(path.to_string_lossy().as_bytes());
        bytes.push(0);
        match body {
            None => bytes.push(1),
            Some(body) => {
                bytes.push(2);
                bytes.extend_from_slice(body);
            }
        }
    }
    format!("{:016x}", fnv1a64(&bytes))
}

fn assert_unchanged(root: &Path, before: &BTreeMap<PathBuf, Option<Vec<u8>>>) {
    let after = snapshot(root);
    let before_hash = fingerprint(before);
    let after_hash = fingerprint(&after);
    assert_eq!(
        before_hash, after_hash,
        "filesystem hash changed: before {before_hash} after {after_hash}"
    );
    assert_eq!(before, &after, "inventory changed fixture bytes");
}

fn hash_text() -> String {
    "a".repeat(64)
}

#[test]
fn inventory_and_preview_report_reason_codes_without_writes() {
    let absent = Fixture::new("absent");
    let before = snapshot(&absent.base);
    let (code, report, _) = run(&absent, &["workspace", "inventory"]);
    assert_eq!(code, 0, "{report}");
    let summary = serde_json::json!({
        "mode": "inventory",
        "apply": false,
        "blockers": ["fence-missing", "ledger-missing"],
        "entries": []
    });
    assert_eq!(report["ok"], true);
    assert_eq!(report["command"], "meshloop:workspace-inventory");
    assert_eq!(report["data"], summary, "fixture JSON summary mismatch");
    let (code, preview, _) = run(&absent, &["workspace", "preview"]);
    assert_eq!(code, 0);
    assert_eq!(
        preview["data"],
        serde_json::json!({
            "mode": "preview",
            "apply": false,
            "blockers": ["adoption-preview-only", "fence-missing", "ledger-missing"],
            "entries": []
        })
    );
    let (code, _, _) = run(&absent, &["workspace", "apply"]);
    assert_eq!(code, 2);
    assert_unchanged(&absent.base, &before);
    assert!(!absent.db.exists());
    assert!(
        !absent
            .repo
            .join(".git")
            .join("meshloop-workspace-registry.json")
            .exists()
    );
    assert!(!absent.base.join(".meshloop-worktrees").exists());
    assert!(!absent.repo.join(".meshloop").join("worktrees").exists());

    let legacy = Fixture::new("legacy");
    {
        let mut store = SqliteStore::open(&legacy.db).unwrap();
        store
            .save_run(&RunRow {
                graph_id: "legacy-run".into(),
                plan_state: PlanState::AwaitingPlanReview,
                run_base: "base".into(),
                integrate_ref: "integrate".into(),
                plan_json: "{}".into(),
                plan_id: hash_text(),
                created_at: "1".into(),
                review_note: None,
            })
            .unwrap();
    }
    let before = snapshot(&legacy.base);
    let db_bytes = fs::read(&legacy.db).unwrap();
    let (code, report, _) = run(&legacy, &["workspace", "inventory"]);
    assert_eq!(code, 0, "{report}");
    assert_eq!(report["data"]["apply"], false);
    assert_eq!(
        report["data"]["blockers"],
        serde_json::json!(["fence-missing"])
    );
    assert_eq!(
        report["data"]["entries"],
        serde_json::json!([{
            "class": "legacy",
            "ownership": "legacy-unbound",
            "blockers": ["legacy-ownership-unresolved"],
            "source": "legacy-run",
            "graph_id": "legacy-run",
            "lifecycle_id": null,
            "location": null,
            "withheld": false
        }])
    );
    let (code, preview, _) = run(&legacy, &["workspace", "preview"]);
    assert_eq!(code, 0);
    assert_eq!(preview["data"]["apply"], false);
    assert_eq!(
        preview["data"]["entries"][0]["blockers"],
        serde_json::json!(["adoption-preview-only", "legacy-ownership-unresolved"])
    );
    assert_eq!(fs::read(&legacy.db).unwrap(), db_bytes);
    assert_unchanged(&legacy.base, &before);

    let linked = Fixture::new("linked");
    git(
        &linked.repo,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "linked",
            "linked-wt",
            "HEAD",
        ],
    );
    let before = snapshot(&linked.base);
    let (code, report, _) = run(&linked, &["workspace", "inventory"]);
    assert_eq!(code, 0, "{report}");
    assert_eq!(
        report["data"]["entries"],
        serde_json::json!([{
            "class": "unknown",
            "ownership": "git-registered",
            "blockers": ["unknown-ownership"],
            "source": "git-worktree",
            "graph_id": null,
            "lifecycle_id": null,
            "location": {
                "scope": "repository",
                "root_alias": null,
                "relative_path": "linked-wt"
            },
            "withheld": false
        }])
    );
    assert!(
        !linked
            .repo
            .join(".git")
            .join("meshloop-workspace-registry.json")
            .exists()
    );
    assert_unchanged(&linked.base, &before);

    let upgraded = Fixture::new("schema");
    {
        let _store = SqliteStore::open(&upgraded.db).unwrap();
    }
    let conn = rusqlite::Connection::open(&upgraded.db).unwrap();
    conn.execute("UPDATE schema_meta SET version=5", [])
        .unwrap();
    drop(conn);
    let before = snapshot(&upgraded.base);
    let db_bytes = fs::read(&upgraded.db).unwrap();
    let (code, report, _) = run(&upgraded, &["workspace", "inventory"]);
    assert_eq!(code, 0, "{report}");
    assert_eq!(
        report["data"]["blockers"],
        serde_json::json!(["fence-missing", "schema-upgrade-required"])
    );
    assert_eq!(report["data"]["entries"], serde_json::json!([]));
    assert_eq!(fs::read(&upgraded.db).unwrap(), db_bytes);
    let version: i64 = rusqlite::Connection::open_with_flags(
        &upgraded.db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap()
    .query_row("SELECT version FROM schema_meta", [], |row| row.get(0))
    .unwrap();
    assert_eq!(version, 5);
    assert_unchanged(&upgraded.base, &before);

    let missing = Fixture::new("missing");
    {
        let mut store = SqliteStore::open(&missing.db).unwrap();
        let mut lease = GitRepositoryIdentity::new("git")
            .acquire(&missing.repo, &missing.db)
            .unwrap();
        store.bind_repository(&lease).unwrap();
        let alias = RootAlias::new("worktrees").unwrap();
        let root = missing.repo.join("workspace-root");
        fs::create_dir(&root).unwrap();
        lease.register_root(alias.clone(), &root).unwrap();
        store.register_root(&lease, &alias).unwrap();
        store
            .save_run(&RunRow {
                graph_id: "graph-alpha".into(),
                plan_state: PlanState::PlanAccepted,
                run_base: "base".into(),
                integrate_ref: "integrate".into(),
                plan_json: "{}".into(),
                plan_id: hash_text(),
                created_at: "1".into(),
                review_note: None,
            })
            .unwrap();
        store
            .insert_generation(
                &lease,
                &GenerationRecord {
                    id: LifecycleId::new("generation-1").unwrap(),
                    repository_id: lease.repository().id.clone(),
                    graph_id: GraphId::new("graph-alpha").unwrap(),
                    ordinal: 1,
                    accepted_plan_hash: PlanHash::new(hash_text()).unwrap(),
                    root_alias: alias,
                    relative_path: RelativePath::new("absent-gen").unwrap(),
                    layout_version: 1,
                },
            )
            .unwrap();
    }
    let before = snapshot(&missing.base);
    let registry = missing
        .repo
        .join(".git")
        .join("meshloop-workspace-registry.json");
    let registry_bytes = fs::read(&registry).unwrap();
    let db_bytes = fs::read(&missing.db).unwrap();
    let (code, report, _) = run(
        &missing,
        &[
            "workspace",
            "inventory",
            "--worktree-base",
            missing.base.join("configured-root").to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "{report}");
    assert_eq!(report["data"]["apply"], false);
    assert_eq!(report["data"]["blockers"], serde_json::json!([]));
    assert_eq!(
        report["data"]["entries"],
        serde_json::json!([{
            "class": "missing",
            "ownership": "ledger-owned",
            "blockers": ["path-missing"],
            "source": "generation",
            "graph_id": "graph-alpha",
            "lifecycle_id": "generation-1",
            "location": {
                "scope": "ledger-root",
                "root_alias": "worktrees",
                "relative_path": "absent-gen"
            },
            "withheld": false
        }])
    );
    let (code, preview, _) = run(&missing, &["workspace", "preview"]);
    assert_eq!(code, 0);
    assert_eq!(preview["data"]["apply"], false);
    assert_eq!(preview["data"]["entries"], serde_json::json!([]));
    assert_eq!(
        preview["data"]["blockers"],
        serde_json::json!(["adoption-preview-only"])
    );
    assert!(!missing.repo.join("absent-gen").exists());
    assert!(
        !missing
            .repo
            .join("workspace-root")
            .join("absent-gen")
            .exists()
    );
    assert_eq!(fs::read(&missing.db).unwrap(), db_bytes);
    assert_eq!(fs::read(&registry).unwrap(), registry_bytes);
    assert_unchanged(&missing.base, &before);

    #[cfg(unix)]
    {
        let blocked = Fixture::new("reparse");
        let root = blocked.base.join("configured");
        fs::create_dir(&root).unwrap();
        let sentinel = blocked.base.join("sentinel.txt");
        fs::write(&sentinel, "SENTINEL-DO-NOT-LEAK").unwrap();
        std::os::unix::fs::symlink(&sentinel, root.join("leak")).unwrap();
        let before = snapshot(&blocked.base);
        let (code, report, stdout) = run(
            &blocked,
            &[
                "workspace",
                "inventory",
                "--worktree-base",
                root.to_str().unwrap(),
            ],
        );
        assert_eq!(code, 0, "{report}");
        assert!(!stdout.contains("SENTINEL-DO-NOT-LEAK"));
        assert_eq!(report["data"]["entries"][0]["class"], "inaccessible");
        assert_eq!(
            report["data"]["entries"][0]["blockers"],
            serde_json::json!(["reparse-blocked", "unknown-ownership"])
        );
        assert_eq!(report["data"]["entries"][0]["ownership"], "configured-root");
        assert_eq!(fs::read(&sentinel).unwrap(), b"SENTINEL-DO-NOT-LEAK");
        assert_unchanged(&blocked.base, &before);
    }
}

fn candidate<'a>(report: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    report["data"]["cleanup"]["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["lifecycle_id"] == id)
        .unwrap_or_else(|| panic!("missing {id} in {report}"))
}

#[cfg(unix)]
fn candidate_by_path<'a>(report: &'a serde_json::Value, path: &str) -> &'a serde_json::Value {
    report["data"]["cleanup"]["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["location"]["relative_path"] == path)
        .unwrap_or_else(|| panic!("missing path {path} in {report}"))
}

fn has_reason(candidate: &serde_json::Value, reason: &str) -> bool {
    candidate["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item == reason)
}

#[test]
fn cleanup_preview_blocks_adversarial_candidates_without_writes() {
    cleanup_scenario(false);
}

#[test]
fn cleanup_apply_removes_only_eligible_and_preserves_preview() {
    cleanup_scenario(true);
}

fn cleanup_scenario(apply: bool) {
    let fixture = Fixture::new("cleanup");
    let root = fixture.repo.join("workspace-root");
    fs::create_dir(&root).unwrap();
    {
        let mut store = SqliteStore::open(&fixture.db).unwrap();
        let mut lease = GitRepositoryIdentity::new("git")
            .acquire(&fixture.repo, &fixture.db)
            .unwrap();
        store.bind_repository(&lease).unwrap();
        let alias = RootAlias::new("worktrees").unwrap();
        lease.register_root(alias.clone(), &root).unwrap();
        store.register_root(&lease, &alias).unwrap();
        store
            .save_run(&RunRow {
                graph_id: "graph-alpha".into(),
                plan_state: PlanState::PlanAccepted,
                run_base: "base".into(),
                integrate_ref: "integrate".into(),
                plan_json: "{}".into(),
                plan_id: hash_text(),
                created_at: "1".into(),
                review_note: None,
            })
            .unwrap();
        for (ordinal, id, path) in [
            (1, "generation-clean", "clean-wt"),
            (2, "generation-dirty", "dirty-wt"),
            (3, "generation-unique", "unique-wt"),
            (4, "generation-ignored", "ignored-wt"),
        ] {
            store
                .insert_generation(
                    &lease,
                    &GenerationRecord {
                        id: LifecycleId::new(id).unwrap(),
                        repository_id: lease.repository().id.clone(),
                        graph_id: GraphId::new("graph-alpha").unwrap(),
                        ordinal,
                        accepted_plan_hash: PlanHash::new(hash_text()).unwrap(),
                        root_alias: alias.clone(),
                        relative_path: RelativePath::new(path).unwrap(),
                        layout_version: 1,
                    },
                )
                .unwrap();
        }
        let repo_id = lease.repository().id.clone();
        let driver = DriverRecord {
            id: LifecycleId::new("driver-live").unwrap(),
            nonce: LifecycleId::new("nonce-1").unwrap(),
            target: DriverTarget::Execution(ExecutionScope {
                repository_id: repo_id.clone(),
                graph_id: GraphId::new("graph-alpha").unwrap(),
                generation_id: LifecycleId::new("generation-clean").unwrap(),
            }),
            process: Some(ProcessIdentity {
                pid: 1,
                creation_token: LifecycleId::new("created").unwrap(),
                tree_token: LifecycleId::new("tree").unwrap(),
            }),
            phase: DriverPhase::Running,
            heartbeat: 1,
            error_code: None,
            diagnostic: None,
        };
        let payload = serde_json::to_string(&driver).unwrap();
        rusqlite::Connection::open(&fixture.db)
            .unwrap()
            .execute(
                "INSERT INTO workspace_drivers (id, repo_id, payload_json) VALUES (?1, ?2, ?3)",
                rusqlite::params![driver.id.as_str(), repo_id.as_str(), payload],
            )
            .unwrap();
    }
    for (path, branch) in [
        ("clean-wt", "clean-wt"),
        ("dirty-wt", "dirty-wt"),
        ("unique-wt", "unique-wt"),
        ("ignored-wt", "ignored-wt"),
    ] {
        git(
            &fixture.repo,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                branch,
                root.join(path).to_str().unwrap(),
                "HEAD",
            ],
        );
    }
    fs::write(root.join("dirty-wt").join("tracked.txt"), "dirty tracked\n").unwrap();
    fs::write(root.join("unique-wt").join("unique.txt"), "unique commit\n").unwrap();
    git(&root.join("unique-wt"), &["add", "unique.txt"]);
    git(
        &root.join("unique-wt"),
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "unique",
        ],
    );
    let exclude = fixture.repo.join(".git").join("info").join("exclude");
    let mut ignored = fs::read_to_string(&exclude).unwrap_or_default();
    ignored.push_str("*.log\n");
    fs::write(&exclude, ignored).unwrap();
    fs::write(root.join("ignored-wt").join("notes.log"), "ignored\n").unwrap();
    #[cfg(unix)]
    {
        let sentinel = fixture.base.join("sentinel.txt");
        fs::write(&sentinel, "SENTINEL-DO-NOT-LEAK").unwrap();
        std::os::unix::fs::symlink(&sentinel, root.join("leak")).unwrap();
    }

    let before = snapshot(&fixture.base);
    let before_hash = fingerprint(&before);
    let (code, report, stdout) = run(&fixture, &["workspace", "preview", "--cleanup"]);
    assert_eq!(code, 0, "{report}");
    assert!(!stdout.contains("SENTINEL-DO-NOT-LEAK"));
    assert_eq!(report["data"]["apply"], false);
    assert_eq!(report["data"]["cleanup"]["apply"], false);
    assert_eq!(
        report["data"]["cleanup"]["digest"].as_str().unwrap().len(),
        64
    );
    let digest = report["data"]["cleanup"]["digest"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(candidate(&report, "generation-clean")["status"], "eligible");
    assert_eq!(
        candidate(&report, "generation-clean")["reasons"],
        serde_json::json!([])
    );
    let blocked = [
        ("generation-dirty", "dirty-tracked"),
        ("generation-unique", "unique-commit"),
        ("generation-ignored", "unknown-ignored"),
        ("driver-live", "active-process"),
    ];
    assert_eq!(blocked.len(), 4, "named cleanup candidates");
    for (id, reason) in blocked {
        let item = candidate(&report, id);
        assert_eq!(item["status"], "blocked", "{id} {item}");
        assert!(has_reason(item, reason), "{id} missing {reason}: {item}");
    }
    #[cfg(unix)]
    {
        let leak = candidate_by_path(&report, "leak");
        assert_eq!(leak["status"], "blocked", "{leak}");
        assert!(has_reason(leak, "unsafe-path"), "{leak}");
    }
    assert!(root.join("clean-wt").join("tracked.txt").is_file());
    assert_eq!(
        fs::read(root.join("dirty-wt").join("tracked.txt")).unwrap(),
        b"dirty tracked\n"
    );
    assert!(root.join("unique-wt").join("unique.txt").is_file());
    assert!(root.join("ignored-wt").join("notes.log").is_file());
    #[cfg(unix)]
    {
        assert!(
            root.join("leak")
                .symlink_metadata()
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read(fixture.base.join("sentinel.txt")).unwrap(),
            b"SENTINEL-DO-NOT-LEAK"
        );
    }

    let (code, stale, _) = run(
        &fixture,
        &[
            "workspace",
            "preview",
            "--cleanup",
            "--digest",
            &"0".repeat(64),
        ],
    );
    assert_eq!(code, 0, "{stale}");
    assert_eq!(stale["data"]["cleanup"]["apply"], false);
    assert_eq!(stale["data"]["cleanup"]["digest"], digest);
    let stale_candidates = stale["data"]["cleanup"]["candidates"].as_array().unwrap();
    assert!(
        stale_candidates
            .iter()
            .all(|item| { item["status"] == "blocked" && has_reason(item, "stale-facts") })
    );
    let (code, rebound, _) = run(
        &fixture,
        &["workspace", "preview", "--cleanup", "--digest", &digest],
    );
    assert_eq!(code, 0, "{rebound}");
    assert_eq!(rebound["data"]["cleanup"]["digest"], digest);
    assert_eq!(
        candidate(&rebound, "generation-clean")["status"],
        "eligible"
    );
    let (code, _, _) = run(&fixture, &["workspace", "apply"]);
    assert_eq!(code, 2);
    let after_hash = fingerprint(&snapshot(&fixture.base));
    assert_eq!(
        before_hash, after_hash,
        "filesystem hash changed: before {before_hash} after {after_hash}"
    );
    assert_unchanged(&fixture.base, &before);
    if !apply {
        return;
    }
    let (code, stale, _) = run(
        &fixture,
        &[
            "workspace",
            "apply",
            "--cleanup",
            "--digest",
            &"0".repeat(64),
        ],
    );
    assert_eq!(code, 0, "{stale}");
    assert_eq!(stale["data"]["cleanup"]["apply"], false);
    assert_eq!(
        stale["data"]["cleanup"]["reasons"],
        serde_json::json!(["stale-facts"])
    );
    assert_unchanged(&fixture.base, &before);
    for args in [
        vec!["workspace", "apply", "--cleanup"],
        vec!["workspace", "apply", "--cleanup", "--digest", "bad"],
        vec![
            "workspace",
            "apply",
            "--cleanup",
            "--digest",
            "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF",
        ],
    ] {
        assert_eq!(run(&fixture, &args).0, 2);
    }
    let (code, applied, _) = run(
        &fixture,
        &["workspace", "apply", "--cleanup", "--digest", &digest],
    );
    assert_eq!(code, 0, "{applied}");
    assert_eq!(applied["data"]["cleanup"]["apply"], true, "{applied}");
    assert!(!root.join("clean-wt").exists());
    assert_eq!(
        fs::read(root.join("dirty-wt/tracked.txt")).unwrap(),
        b"dirty tracked\n"
    );
    assert_eq!(
        fs::read(root.join("unique-wt/unique.txt")).unwrap(),
        b"unique commit\n"
    );
    assert_eq!(
        fs::read(root.join("ignored-wt/notes.log")).unwrap(),
        b"ignored\n"
    );
    assert!(fixture.repo.join(".git").is_dir());
    assert!(fixture.db.is_file());
    #[cfg(unix)]
    assert!(
        root.join("leak")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let (code, preview, _) = run(&fixture, &["workspace", "preview", "--cleanup"]);
    assert_eq!(code, 0);
    let empty_digest = preview["data"]["cleanup"]["digest"].as_str().unwrap();
    let before_empty = snapshot(&fixture.base);
    let (code, empty, _) = run(
        &fixture,
        &["workspace", "apply", "--cleanup", "--digest", empty_digest],
    );
    assert_eq!(code, 0, "{empty}");
    assert_eq!(empty["data"]["cleanup"]["apply"], false);
    assert_unchanged(&fixture.base, &before_empty);
}

#[test]
fn compatibility_adopts_verified_legacy_and_keeps_roots() {
    let fixture = Fixture::new("compatibility");
    let mut store = SqliteStore::open(&fixture.db).unwrap();
    for graph_id in ["kept-run", "missing-run"] {
        store
            .save_run(&RunRow {
                graph_id: graph_id.into(),
                plan_state: PlanState::PlanAccepted,
                run_base: "base".into(),
                integrate_ref: "integrate".into(),
                plan_json: "{}".into(),
                plan_id: hash_text(),
                created_at: "1".into(),
                review_note: None,
            })
            .unwrap();
    }
    store
        .save_run(&RunRow {
            graph_id: "name-only".into(),
            plan_state: PlanState::AwaitingPlanReview,
            run_base: "base".into(),
            integrate_ref: "integrate".into(),
            plan_json: "{}".into(),
            plan_id: hash_text(),
            created_at: "1".into(),
            review_note: None,
        })
        .unwrap();
    let mut lease = GitRepositoryIdentity::new("git")
        .acquire(&fixture.repo, &fixture.db)
        .unwrap();
    store.bind_repository(&lease).unwrap();
    let alias = RootAlias::new("legacy-pin").unwrap();
    let root = fixture.repo.join("legacy-pin");
    lease.bootstrap_root(alias.clone(), &root).unwrap();
    store.register_root(&lease, &alias).unwrap();
    fs::write(root.join("kept.txt"), "kept\n").unwrap();
    let plan_hash = PlanHash::new(hash_text()).unwrap();
    let kept = GenerationRecord {
        id: LifecycleId::new("kept-generation").unwrap(),
        repository_id: lease.repository().id.clone(),
        graph_id: GraphId::new("kept-run").unwrap(),
        ordinal: 1,
        accepted_plan_hash: plan_hash.clone(),
        root_alias: alias.clone(),
        relative_path: RelativePath::new("kept.txt").unwrap(),
        layout_version: 1,
    };
    let missing = GenerationRecord {
        id: LifecycleId::new("missing-generation").unwrap(),
        repository_id: lease.repository().id.clone(),
        graph_id: GraphId::new("missing-run").unwrap(),
        ordinal: 1,
        accepted_plan_hash: plan_hash,
        root_alias: alias,
        relative_path: RelativePath::new("missing.txt").unwrap(),
        layout_version: 1,
    };
    store.insert_generation(&lease, &kept).unwrap();
    store.insert_generation(&lease, &missing).unwrap();
    drop(store);
    drop(lease);
    let db = rusqlite::Connection::open(&fixture.db).unwrap();
    db.execute(
        "INSERT INTO workspace_legacy_unbound VALUES('runs','kept-run','Unresolved')",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO workspace_legacy_unbound VALUES('runs','missing-run','Unresolved')",
        [],
    )
    .unwrap();
    drop(db);
    fs::write(fixture.repo.join("unknown-artifact.txt"), "leave me\n").unwrap();
    assert!(!root.join("missing.txt").exists());

    let before = snapshot(&fixture.base);
    let (code, preview, _) = run(&fixture, &["workspace", "adopt"]);
    assert_eq!(code, 0, "{preview}");
    assert_eq!(preview["data"]["apply"], false);
    assert_eq!(preview["data"]["status"], "preview");
    let digest = preview["data"]["digest"].as_str().unwrap().to_string();
    assert_eq!(digest.len(), 64);
    assert!(
        preview["data"]["adopted"]
            .as_array()
            .unwrap()
            .iter()
            .any(|pin| pin["graph_id"] == "kept-run" && pin["relative_path"] == "kept.txt")
    );
    assert!(
        preview["data"]["retained"]
            .as_array()
            .unwrap()
            .iter()
            .any(|hold| hold["graph_id"] == "missing-run" && hold["reason"] == "path-missing")
    );
    assert!(
        preview["data"]["retained"]
            .as_array()
            .unwrap()
            .iter()
            .any(|hold| {
                hold["graph_id"] == "name-only" && hold["reason"] == "unverified-location"
            })
    );
    assert_unchanged(&fixture.base, &before);

    let db_bytes = fs::read(&fixture.db).unwrap();
    let (code, stale, stale_out) = run(
        &fixture,
        &["workspace", "adopt", "--apply", "--digest", &"0".repeat(64)],
    );
    assert_eq!(code, 0, "{stale} {stale_out}");
    assert_eq!(stale["data"]["apply"], false);
    assert_eq!(stale["data"]["status"], "stale");
    assert_eq!(fs::read(&fixture.db).unwrap(), db_bytes);
    assert_unchanged(&fixture.base, &before);

    let (code, applied, _) = run(
        &fixture,
        &["workspace", "adopt", "--apply", "--digest", &digest],
    );
    assert_eq!(code, 0, "{applied}");
    assert_eq!(applied["data"]["apply"], true, "{applied}");
    assert_eq!(applied["data"]["status"], "adopted");
    let db = rusqlite::Connection::open(&fixture.db).unwrap();
    let kept_marker: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM workspace_legacy_unbound WHERE source_table='runs' AND source_id='kept-run'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let missing_marker: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM workspace_legacy_unbound WHERE source_table='runs' AND source_id='missing-run'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let binding: String = db
        .query_row(
            "SELECT root_alias FROM workspace_run_bindings WHERE graph_id='kept-run'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let name_only: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM workspace_run_bindings WHERE graph_id='name-only'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    drop(db);
    assert_eq!(kept_marker, 0);
    assert_eq!(missing_marker, 1);
    assert_eq!(binding, "legacy-pin");
    assert_eq!(name_only, 0);
    assert_eq!(fs::read(root.join("kept.txt")).unwrap(), b"kept\n");
    assert_eq!(
        fs::read(fixture.repo.join("unknown-artifact.txt")).unwrap(),
        b"leave me\n"
    );
    assert!(!root.join("missing.txt").exists());
    assert!(!fixture.repo.join(".meshloop").join("worktrees").exists());
    assert!(!fixture.base.join(".meshloop-worktrees").exists());

    let after_adopt = snapshot(&fixture.base);
    let (code, maintain, _) = run(&fixture, &["workspace", "maintain"]);
    assert_eq!(code, 0, "{maintain}");
    assert_eq!(maintain["data"]["apply"], false);
    assert_unchanged(&fixture.base, &after_adopt);

    let mut configured = fixture.config.clone();
    configured.set_extension("workspace.toml");
    let mut text = fs::read_to_string(&fixture.config).unwrap();
    text.push_str("\n[workspace]\nroot = \"configured-root\"\n");
    fs::write(&configured, text).unwrap();
    let (code, reroot, _) = run(
        &fixture,
        &[
            "workspace",
            "adopt",
            "--config",
            configured.to_str().unwrap(),
            "--worktree-base",
            "other-root",
        ],
    );
    assert_eq!(code, 0, "{reroot}");
    assert!(!fixture.repo.join("configured-root").exists());
    assert!(!fixture.repo.join("other-root").exists());
    assert_eq!(fs::read(root.join("kept.txt")).unwrap(), b"kept\n");
    assert!(!root.join("missing.txt").exists());
}
