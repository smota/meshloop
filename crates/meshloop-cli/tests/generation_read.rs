//! Bounded active-generation public reads, not the complete S3 lifecycle oracle.
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use meshloop_adapters::store::SqliteStore;
use meshloop_adapters::workspace::GitRepositoryIdentity;
use meshloop_domain::digest::sha256_hex;
use meshloop_domain::evidence::AttemptId;
use meshloop_domain::evidence::{CandidateRef, DeterministicEvidence, Evidence};
use meshloop_domain::state::PlanState;
use meshloop_domain::state::{Event, TaskState};
use meshloop_domain::task_graph::TaskId;
use meshloop_domain::workspace::*;
use meshloop_engine::ports::{RunRow, RunStore, TransitionRecord};
use meshloop_engine::workspace::*;

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

fn id(s: &str) -> LifecycleId {
    LifecycleId::new(s).unwrap()
}
fn graph() -> GraphId {
    GraphId::new("bound-run").unwrap()
}
fn plan(description: &str, extra: bool) -> String {
    let mut nodes =
        vec![serde_json::json!({"id":1,"description":description,"depends_on":[],"tier":"Tier1"})];
    if extra {
        nodes.push(
            serde_json::json!({"id":2,"description":"new task","depends_on":[1],"tier":"Tier1"}),
        );
    }
    serde_json::json!({"graph_id":"bound-run","nodes":nodes}).to_string()
}
fn revision(
    repo: LifecycleId,
    name: &str,
    json: String,
    predecessor: Option<LifecycleId>,
    generation_id: Option<LifecycleId>,
) -> PlanRevisionSnapshot {
    PlanRevisionSnapshot {
        metadata: PlanRevision {
            id: id(name),
            repository_id: repo,
            graph_id: graph(),
            generation_id,
            predecessor,
            plan_hash: PlanHash::new(sha256_hex(json.as_bytes())).unwrap(),
        },
        plan_json: json,
    }
}
fn auth(rev: &str) -> RevisionAuthorization {
    RevisionAuthorization {
        id: id(&format!("auth-{rev}")),
        revision_id: id(rev),
        kind: AuthorizationKind::Human,
        decision: RevisionDecision::Accepted,
        actor: id("fixture-human"),
    }
}
fn setup(f: &Fixture) -> (SqliteStore, meshloop_adapters::workspace::RepositoryLease) {
    setup_binding(f, true)
}
fn setup_binding(
    f: &Fixture,
    active: bool,
) -> (SqliteStore, meshloop_adapters::workspace::RepositoryLease) {
    let mut store = SqliteStore::open(&f.db).unwrap();
    let mut lease = GitRepositoryIdentity::new("git")
        .acquire(&f.repo, &f.db)
        .unwrap();
    let root = f.base.join("owned-root");
    fs::create_dir(&root).unwrap();
    let alias = RootAlias::new("worktrees").unwrap();
    lease.register_root(alias.clone(), &root).unwrap();
    store.bind_repository(&lease).unwrap();
    store.register_root(&lease, &alias).unwrap();
    let json = plan("immutable task", false);
    store
        .save_run(&RunRow {
            graph_id: "bound-run".into(),
            plan_state: PlanState::PlanAccepted,
            run_base: "HEAD".into(),
            integrate_ref: "unused".into(),
            plan_id: sha256_hex(json.as_bytes()),
            plan_json: json,
            created_at: "1".into(),
            review_note: None,
        })
        .unwrap();
    store
        .create_run_binding(
            &lease,
            &RunBinding {
                graph_id: graph(),
                repository_id: lease.repository().id.clone(),
                location: PortableLocation {
                    root_alias: alias,
                    relative_path: RelativePath::new("runs/run-1").unwrap(),
                },
                layout_version: 1,
                active_generation: None,
                current_revision: None,
            },
        )
        .unwrap();
    if active {
        activate(&mut store, &lease, 1);
    }
    (store, lease)
}
fn activate(store: &mut SqliteStore, lease: &dyn RepositoryFence, number: u32) {
    let current_revision = store
        .run_binding(&graph())
        .unwrap()
        .unwrap()
        .current_revision
        .unwrap_or_else(|| id("rev-1"));
    let rev = store
        .revision(&current_revision)
        .unwrap()
        .unwrap_or_else(|| {
            revision(
                lease.repository().id.clone(),
                "rev-1",
                plan("immutable task", false),
                None,
                None,
            )
        });
    store
        .activate_generation(
            lease,
            &GenerationActivation {
                generation: GenerationRecord {
                    id: id(&format!("gen-{number}")),
                    repository_id: lease.repository().id.clone(),
                    graph_id: graph(),
                    ordinal: number,
                    accepted_plan_hash: rev.metadata.plan_hash.clone(),
                    root_alias: RootAlias::new("worktrees").unwrap(),
                    relative_path: RelativePath::new(format!("runs/run-1/gen-{number}")).unwrap(),
                    layout_version: 1,
                },
                authorization: auth(rev.metadata.id.as_str()),
                revision: rev,
                artifacts: vec![],
                operations: vec![],
                events: vec![],
            },
        )
        .unwrap();
}
fn expected() -> ExpectedState {
    ExpectedState {
        source_artifact: None,
        source: None,
        base: None,
        location: None,
        native_identity: None,
        head: None,
        reference: None,
        registered: false,
        exists: false,
    }
}
fn scope(store: &SqliteStore) -> ExecutionScope {
    store
        .resolve_scope(&graph(), &RunScope::ActiveGeneration)
        .unwrap()
        .unwrap()
}
fn append(
    store: &mut SqliteStore,
    lease: &dyn RepositoryFence,
    attempt: Option<AttemptId>,
    from: TaskState,
    event: Event,
) {
    let scope = scope(store);
    let revision_id = store
        .run_binding(&graph())
        .unwrap()
        .unwrap()
        .current_revision
        .unwrap();
    store
        .append_scoped_event(
            lease,
            &ScopedEvent {
                scope,
                revision_id,
                record: TransitionRecord {
                    graph_id: "bound-run".into(),
                    task_id: TaskId(1),
                    attempt_id: attempt,
                    from,
                    to: meshloop_domain::state::transition(from, event).unwrap(),
                    event,
                    reason: None,
                    executor: "fixture".into(),
                    occurred_at: "1".into(),
                },
            },
        )
        .unwrap();
}
fn attempt(store: &mut SqliteStore, lease: &dyn RepositoryFence, name: &str) -> ScopedAttempt {
    let scope = scope(store);
    let artifact = ArtifactRecord {
        id: id(name),
        generation_id: scope.generation_id.clone(),
        kind: ArtifactKind::Attempt,
        root_alias: RootAlias::new("worktrees").unwrap(),
        relative_path: RelativePath::new(format!("runs/{name}")).unwrap(),
        attempt_id: None,
    };
    let spawn_intent = ProducerOperation {
        id: id(&format!("spawn-{name}")),
        repository_id: scope.repository_id.clone(),
        graph_id: Some(graph()),
        generation_id: Some(scope.generation_id.clone()),
        target: ProducerTarget::Artifact(artifact.id.clone()),
        kind: ProducerKind::Spawn,
        before: expected(),
        after: expected(),
        max_steps: 1,
    };
    let revision_id = store
        .run_binding(&graph())
        .unwrap()
        .unwrap()
        .current_revision
        .unwrap();
    let a = store
        .reserve_attempt(
            lease,
            &AttemptReservation {
                scope: scope.clone(),
                revision_id: revision_id.clone(),
                task_id: TaskId(1),
                harness: Some("fixture".into()),
                model_ref: None,
                artifact,
                spawn_intent,
            },
        )
        .unwrap();
    store
        .record_scoped_evidence(
            lease,
            &scope,
            &revision_id,
            &Evidence::Deterministic(DeterministicEvidence {
                candidate: CandidateRef {
                    task_id: a.task_id,
                    attempt_id: a.attempt_id,
                    revision: "a".repeat(40),
                },
                tool: "fixture".into(),
                tool_version: "1".into(),
                exit_code: 0,
                output_redacted: name.into(),
            }),
        )
        .unwrap();
    a
}
fn mutate(store: &mut SqliteStore, lease: &dyn RepositoryFence) {
    let r = revision(
        lease.repository().id.clone(),
        "rev-2",
        plan("current immutable task", true),
        Some(id("rev-1")),
        Some(id("gen-1")),
    );
    store.append_revision(lease, &r).unwrap();
    store.authorize_revision(lease, &auth("rev-2")).unwrap();
    store
        .select_revision(lease, &scope(store), &id("rev-2"))
        .unwrap();
}

#[test]
fn public_reads_select_active_generation_attempts_evidence_and_portable_locations() {
    let f = Fixture::new("generations");
    let (mut store, lease) = setup(&f);
    let old = attempt(&mut store, &lease, "old-attempt");
    append(
        &mut store,
        &lease,
        Some(old.attempt_id),
        TaskState::Ready,
        Event::AttemptStarted,
    );
    append(
        &mut store,
        &lease,
        Some(old.attempt_id),
        TaskState::Running,
        Event::HarnessCrashedOrTimeout,
    );
    activate(&mut store, &lease, 2);
    let current = attempt(&mut store, &lease, "current-attempt");
    append(
        &mut store,
        &lease,
        Some(current.attempt_id),
        TaskState::Ready,
        Event::AttemptStarted,
    );
    drop(lease);
    drop(store);
    let before = snapshot(&f.base);
    let status = f.read(&["status", "--graph", "bound-run"], 0);
    assert_eq!(status["data"]["generation_id"], "gen-2");
    assert_eq!(status["data"]["revision_id"], "rev-1");
    assert_eq!(status["data"]["nodes"][0]["state"], "Running");
    assert_eq!(status["data"]["summary"]["attempts"], 1);
    let inspect = f.read(&["inspect", "--graph", "bound-run", "--attempts"], 0);
    let data = &inspect["data"];
    let attempts = data["nodes"][0]["attempts"].as_array().unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0]["attempt_id"], current.attempt_id.0);
    assert_eq!(attempts[0]["generation_id"], "gen-2");
    assert_eq!(attempts[0]["revision_id"], "rev-1");
    assert_eq!(
        attempts[0]["evidence"][0]["summary_redacted"],
        "current-attempt"
    );
    assert_eq!(data["nodes"][0]["location"]["root_alias"], "worktrees");
    assert!(data["nodes"][0]["worktree"].is_null());
    assert!(data["git_export"].is_null());
    assert!(
        data["git_export_unavailable"]
            .as_str()
            .unwrap()
            .contains("generation-scoped")
    );
    assert!(!inspect.to_string().contains(f.base.to_str().unwrap()));
    assert_eq!(snapshot(&f.base), before);
}

#[test]
fn current_immutable_plan_replays_prior_revision_history_and_ignores_mutable_projection() {
    let f = Fixture::new("revisions");
    let (mut store, lease) = setup(&f);
    let a = attempt(&mut store, &lease, "ancestor-attempt");
    append(
        &mut store,
        &lease,
        Some(a.attempt_id),
        TaskState::Ready,
        Event::AttemptStarted,
    );
    mutate(&mut store, &lease);
    append(
        &mut store,
        &lease,
        None,
        TaskState::Running,
        Event::HarnessExited,
    );
    drop(lease);
    drop(store);
    let c = rusqlite::Connection::open(&f.db).unwrap();
    c.execute(
        "UPDATE runs SET plan_json='invalid mutable projection',plan_state='AwaitingPlanReview'",
        [],
    )
    .unwrap();
    drop(c);
    let out = f.read(&["status", "--graph", "bound-run"], 0);
    assert_eq!(out["data"]["revision_id"], "rev-2");
    assert_eq!(out["data"]["plan_state"], "PlanAccepted");
    assert_eq!(out["data"]["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(
        out["data"]["nodes"][0]["description"],
        "current immutable task"
    );
    assert_eq!(out["data"]["nodes"][0]["state"], "Verifying");
    assert_eq!(out["data"]["summary"]["attempts"], 1);
    let inspected = f.read(&["inspect", "--graph", "bound-run", "--attempts"], 0);
    assert_eq!(inspected["data"]["revision_id"], "rev-2");
    assert_eq!(
        inspected["data"]["nodes"][0]["attempts"][0]["revision_id"],
        "rev-1"
    );
    assert_eq!(
        inspected["data"]["nodes"][0]["attempts"][0]["generation_id"],
        "gen-1"
    );
}

#[test]
fn malformed_binding_or_same_hash_wrong_initial_revision_refuses_without_legacy_fallback() {
    let f = Fixture::new("refusal");
    let (mut store, lease) = setup(&f);
    let unrelated = revision(
        lease.repository().id.clone(),
        "same-hash-unrelated",
        plan("immutable task", false),
        None,
        None,
    );
    store.append_revision(&lease, &unrelated).unwrap();
    drop(lease);
    drop(store);
    let c = rusqlite::Connection::open(&f.db).unwrap();
    c.execute(
        "UPDATE workspace_generation_revisions SET revision_id='same-hash-unrelated'",
        [],
    )
    .unwrap();
    drop(c);
    let before = snapshot(&f.base);
    for command in ["status", "inspect", "watch"] {
        let out = f.read(
            &[command, "--graph", "bound-run"],
            if command == "inspect" { 1 } else { 2 },
        );
        assert!(out.to_string().contains("PlanRevisionMismatch"));
    }
    assert_eq!(snapshot(&f.base), before);
    let c = rusqlite::Connection::open(&f.db).unwrap();
    let payload: String = c
        .query_row("SELECT payload_json FROM workspace_run_bindings", [], |r| {
            r.get(0)
        })
        .unwrap();
    let mut b: RunBinding = serde_json::from_str(&payload).unwrap();
    b.current_revision = None;
    c.execute(
        "UPDATE workspace_run_bindings SET current_revision=NULL,payload_json=?1",
        [serde_json::to_string(&b).unwrap()],
    )
    .unwrap();
    drop(c);
    assert!(
        f.read(&["status", "--graph", "bound-run"], 2)
            .to_string()
            .contains("PlanRevisionMismatch")
    );
}

#[test]
fn watch_pins_scope_and_refuses_restart_or_revision_change() {
    use std::io::{BufRead, BufReader};
    for change in ["restart", "revision", "events"] {
        let f = Fixture::new("watch-scope");
        let (mut store, lease) = setup(&f);
        let a = attempt(&mut store, &lease, "watch-attempt");
        append(
            &mut store,
            &lease,
            Some(a.attempt_id),
            TaskState::Ready,
            Event::AttemptStarted,
        );
        let mut child = f
            .command(&[
                "watch",
                "--graph",
                "bound-run",
                "--interval",
                "1",
                "--timeout",
                "8",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut r = BufReader::new(stdout);
            let mut first = String::new();
            r.read_line(&mut first).unwrap();
            tx.send(first).unwrap();
            let mut rest = String::new();
            std::io::Read::read_to_string(&mut r, &mut rest).unwrap();
            rest
        });
        let first = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(first.contains("gen-1"), "{first}");
        match change {
            "revision" => mutate(&mut store, &lease),
            "restart" => activate(&mut store, &lease, 2),
            _ => {
                append(
                    &mut store,
                    &lease,
                    Some(a.attempt_id),
                    TaskState::Running,
                    Event::HarnessExited,
                );
                append(
                    &mut store,
                    &lease,
                    Some(a.attempt_id),
                    TaskState::Verifying,
                    Event::DeterministicChecksPassed,
                );
            }
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while child.try_wait().unwrap().is_none() {
            if Instant::now() > deadline {
                child.kill().unwrap();
                panic!("watch blocked");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let code = child.wait().unwrap().code();
        let rest = reader.join().unwrap();
        if change == "events" {
            assert_eq!(code, Some(0));
            assert!(rest.contains("AwaitingReview"), "{rest}");
            assert!(rest.contains("awaiting_review"));
        } else {
            assert_eq!(code, Some(1));
            assert!(rest.contains("ReadScopeChanged"), "{rest}");
            assert!(!rest.contains("current immutable task"));
        }
    }
}

#[test]
fn revision_cycles_and_resource_limit_fail_closed() {
    for cycle in [true, false] {
        let f = Fixture::new("chain-bound");
        let (store, lease) = setup(&f);
        let repository = lease.repository().id.clone();
        drop(lease);
        drop(store);
        let mut c = rusqlite::Connection::open(&f.db).unwrap();
        let tx = c.transaction().unwrap();
        let length = if cycle { 1 } else { 4096 };
        for n in 1..=length {
            let name = format!("chain-{n}");
            let predecessor = if cycle {
                id(&name)
            } else if n == 1 {
                id("rev-1")
            } else {
                id(&format!("chain-{}", n - 1))
            };
            let r = revision(
                repository.clone(),
                &name,
                plan("immutable task", false),
                Some(predecessor),
                Some(id("gen-1")),
            );
            tx.execute(
                "INSERT INTO workspace_plan_revisions VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                rusqlite::params![
                    name,
                    repository.as_str(),
                    "bound-run",
                    "gen-1",
                    r.metadata.predecessor.as_ref().unwrap().as_str(),
                    r.metadata.plan_hash.as_str(),
                    r.plan_json,
                    serde_json::to_string(&r.metadata).unwrap()
                ],
            )
            .unwrap();
        }
        let payload: String = tx
            .query_row("SELECT payload_json FROM workspace_run_bindings", [], |r| {
                r.get(0)
            })
            .unwrap();
        let mut b: RunBinding = serde_json::from_str(&payload).unwrap();
        b.current_revision = Some(id(&format!("chain-{length}")));
        tx.execute(
            "UPDATE workspace_run_bindings SET current_revision=?1,payload_json=?2",
            rusqlite::params![
                b.current_revision.as_ref().unwrap().as_str(),
                serde_json::to_string(&b).unwrap()
            ],
        )
        .unwrap();
        tx.commit().unwrap();
        drop(c);
        let out = f.read(&["status", "--graph", "bound-run"], 2);
        assert!(
            out.to_string().contains(if cycle {
                "PlanRevisionMismatch"
            } else {
                "ReadRevisionLimitExceeded"
            }),
            "{out}"
        );
    }
}

fn with_reader<T>(
    store: &mut SqliteStore,
    fixture: &Fixture,
    observe: impl FnOnce(&meshloop_engine::run_loop::RunLoop<'_>) -> T,
) -> T {
    let workspace = meshloop_adapters::git::GitWorktreeAdapter::new(fixture.repo.clone());
    let processes = meshloop_adapters::process::HostProcessView {};
    let checks = meshloop_adapters::check::CommandCheckRunner;
    let saga = meshloop_engine::run_loop::RunLoop {
        harnesses: Default::default(),
        candidates: vec![],
        workspace: &workspace,
        store,
        processes: &processes,
        checks: &checks,
        router: Default::default(),
        limits: meshloop_engine::run_loop::RunLimits {
            max_retries: 1,
            task_timeout: Duration::from_secs(1),
            max_concurrent_workers: 1,
        },
        fixture_only: true,
        verify_command: vec![],
        worktree_base: fixture.base.join("unused"),
        active_graph: None,
        planner: Default::default(),
    };
    observe(&saga)
}

#[test]
fn writer_contract_allows_unassigned_pending_revision_and_exact_restart_anchor_from_prior_generation()
 {
    let f = Fixture::new("valid-revision-contract");
    let (mut store, lease) = setup(&f);
    let r = revision(
        lease.repository().id.clone(),
        "unassigned-revision",
        plan("pending immutable revision", true),
        Some(id("rev-1")),
        None,
    );
    store.append_revision(&lease, &r).unwrap();
    store
        .project_revision(&lease, &graph(), &r.metadata.id)
        .unwrap();
    let pending = f.read(&["status", "--graph", "bound-run"], 0);
    assert_eq!(pending["data"]["revision_id"], "unassigned-revision");
    assert_eq!(pending["data"]["plan_state"], "AwaitingPlanReview");
    store
        .authorize_revision(&lease, &auth("unassigned-revision"))
        .unwrap();
    store
        .project_revision(&lease, &graph(), &r.metadata.id)
        .unwrap();
    assert_eq!(
        f.read(&["status", "--graph", "bound-run"], 0)["data"]["plan_state"],
        "PlanAccepted"
    );
    drop(lease);
    drop(store);

    let f = Fixture::new("prior-generation-anchor");
    let (mut store, lease) = setup(&f);
    mutate(&mut store, &lease);
    activate(&mut store, &lease, 2);
    assert_eq!(
        store
            .revision(&id("rev-2"))
            .unwrap()
            .unwrap()
            .metadata
            .generation_id,
        Some(id("gen-1"))
    );
    let out = f.read(&["status", "--graph", "bound-run"], 0);
    assert_eq!(out["data"]["generation_id"], "gen-2");
    assert_eq!(out["data"]["revision_id"], "rev-2");
}

#[test]
fn executed_revision_requires_historical_acceptance_but_later_decline_keeps_history_and_refreshes_state()
 {
    let f = Fixture::new("authorization-provenance");
    let (mut store, lease) = setup(&f);
    mutate(&mut store, &lease);
    let a = attempt(&mut store, &lease, "accepted-revision-attempt");
    append(
        &mut store,
        &lease,
        Some(a.attempt_id),
        TaskState::Ready,
        Event::AttemptStarted,
    );
    let mut reader = SqliteStore::open_read_only(&f.db).unwrap().unwrap();
    with_reader(&mut reader, &f, |saga| {
        let selection =
            meshloop_engine::workspace::read::ReadSelection::capture(saga.store, "bound-run")
                .unwrap();
        assert_eq!(
            saga.read_frame(&selection).unwrap().status.plan_state,
            PlanState::PlanAccepted
        );
        let mut decline = auth("rev-2");
        decline.id = id("later-decline");
        decline.decision = RevisionDecision::Declined;
        store.authorize_revision(&lease, &decline).unwrap();
        store
            .project_revision(&lease, &graph(), &id("rev-2"))
            .unwrap();
        let refreshed = saga.read_frame(&selection).unwrap();
        assert_eq!(refreshed.status.plan_state, PlanState::AwaitingPlanReview);
        assert_eq!(refreshed.summary.attempts, 1);
        assert_eq!(refreshed.status.nodes[0].state, TaskState::Running);
    });
    let c = rusqlite::Connection::open(&f.db).unwrap();
    c.execute(
        "DELETE FROM workspace_revision_authorizations WHERE id='auth-rev-2'",
        [],
    )
    .unwrap();
    drop(c);
    let out = f.read(&["status", "--graph", "bound-run"], 2);
    assert!(out.to_string().contains("PlanRevisionMismatch"));
}

#[test]
fn sqlite_snapshot_holds_complete_old_frame_across_committed_writer_changes_then_refreshes() {
    for change_revision in [false, true] {
        let f = Fixture::new("snapshot-consistency");
        let (mut writer, lease) = setup(&f);
        let mut reader = SqliteStore::open_read_only(&f.db).unwrap().unwrap();
        with_reader(&mut reader, &f, |saga| {
            let selection =
                meshloop_engine::workspace::read::ReadSelection::capture(saga.store, "bound-run")
                    .unwrap();
            saga.store
                .with_read_snapshot(&mut || {
                    let old = saga.read_frame(&selection).unwrap();
                    assert_eq!(old.summary.attempts, 0);
                    if change_revision {
                        mutate(&mut writer, &lease);
                    } else {
                        let a = attempt(&mut writer, &lease, "new-committed-attempt");
                        append(
                            &mut writer,
                            &lease,
                            Some(a.attempt_id),
                            TaskState::Ready,
                            Event::AttemptStarted,
                        );
                    }
                    // Nested frame reads share the outer reader transaction, even
                    // though another connection has committed newer WAL records.
                    let stable = saga.read_current_frame("bound-run").unwrap();
                    assert_eq!(stable.revision_id, Some(id("rev-1")));
                    assert_eq!(stable.summary.attempts, 0);
                    assert_eq!(stable.status.nodes.len(), 1);
                })
                .unwrap();
            if change_revision {
                assert!(
                    format!("{:?}", saga.read_frame(&selection).err().unwrap())
                        .contains("ReadScopeChanged")
                );
                assert_eq!(
                    saga.read_current_frame("bound-run").unwrap().revision_id,
                    Some(id("rev-2"))
                );
            } else {
                let fresh = saga.read_frame(&selection).unwrap();
                assert_eq!(fresh.summary.attempts, 1);
                assert_eq!(fresh.status.nodes[0].state, TaskState::Running);
                assert_eq!(
                    fresh.attempts[&TaskId(1)][0].evidence[0].summary_redacted,
                    "new-committed-attempt"
                );
            }
        });
    }
}

#[test]
fn bound_start_restart_and_cancel_preserve_dirty_legacy_artifacts_and_history() {
    // Both pending and activated bindings must stop legacy producers. The legacy
    // rows/paths deliberately coexist, exposing restart's former early effects.
    for active in [false, true] {
        let f = Fixture::new(if active {
            "active-mutation"
        } else {
            "pending-mutation"
        });
        let worktrees = f.base.join("legacy-worktrees");
        let integrate = worktrees.join("bound-run/integrate");
        let attempt_path = worktrees.join("bound-run/task-1-attempt-91");
        for (path, branch) in [
            (&integrate, "meshloop/bound-run/integrate"),
            (&attempt_path, "meshloop/bound-run/task-1/attempt-91"),
        ] {
            git(
                &f.repo,
                &[
                    "worktree",
                    "add",
                    "--quiet",
                    "-b",
                    branch,
                    path.to_str().unwrap(),
                    "HEAD",
                ],
            );
            fs::write(
                path.join("tracked.txt"),
                "dirty tracked work must survive\n",
            )
            .unwrap();
            fs::write(path.join("untracked.txt"), "unknown work must survive\n").unwrap();
            fs::write(path.join(".gitignore"), "ignored-output/\n").unwrap();
            fs::create_dir(path.join("ignored-output")).unwrap();
            fs::write(path.join("ignored-output/result.bin"), b"retained output").unwrap();
        }
        let input = f.repo.join("input-plan.json");
        fs::write(&input, plan("immutable task", false)).unwrap();
        // Seed legacy execution before assigning lifecycle ownership.
        let mut legacy = SqliteStore::open(&f.db).unwrap();
        let json = plan("immutable task", false);
        legacy
            .save_run(&RunRow {
                graph_id: "bound-run".into(),
                plan_state: PlanState::PlanAccepted,
                run_base: "HEAD".into(),
                integrate_ref: "meshloop/bound-run/integrate".into(),
                plan_id: sha256_hex(json.as_bytes()),
                plan_json: json,
                created_at: "1".into(),
                review_note: None,
            })
            .unwrap();
        legacy
            .save_attempt(&meshloop_engine::ports::AttemptRow {
                attempt_id: AttemptId(91),
                graph_id: "bound-run".into(),
                task_id: TaskId(1),
                harness: Some("fixture".into()),
                model_ref: None,
                worktree_path: Some(attempt_path.clone()),
                pid: None,
                image_name: None,
                pane_id: Some("retained-pane".into()),
                started_at: Some("1".into()),
                ended_at: None,
                outcome: None,
            })
            .unwrap();
        drop(legacy);
        let (store, lease) = setup_binding(&f, active);
        drop(lease);
        drop(store);
        let artifacts = snapshot(&worktrees);
        let refs = git(
            &f.repo,
            &["for-each-ref", "--format=%(refname) %(objectname)"],
        );
        let database = fs::read(&f.db).unwrap();
        for args in [
            vec!["run", "--plan", input.to_str().unwrap(), "--accept-plan"],
            vec![
                "run",
                "--plan",
                input.to_str().unwrap(),
                "--accept-plan",
                "--reset",
            ],
            vec!["resume", "--graph", "bound-run", "--restart"],
            vec!["cancel", "--graph", "bound-run"],
        ] {
            let mut command = f.command(&args);
            if args[0] != "cancel" {
                command
                    .arg("--fixture-only")
                    .arg("--worktree-base")
                    .arg(&worktrees);
            }
            let out = bounded_output(&mut command);
            assert_eq!(out.status.code(), Some(1), "{args:?}: {out:?}");
            assert!(
                String::from_utf8_lossy(&out.stderr).contains("LegacyOwnershipUnresolved")
                    || String::from_utf8_lossy(&out.stdout).contains("LegacyOwnershipUnresolved"),
                "{args:?}: {out:?}"
            );
            assert_eq!(snapshot(&worktrees), artifacts, "{args:?}");
            assert_eq!(
                git(
                    &f.repo,
                    &["for-each-ref", "--format=%(refname) %(objectname)"]
                ),
                refs,
                "{args:?}"
            );
            assert_eq!(
                fs::read(&f.db).unwrap(),
                database,
                "legacy and lifecycle history changed: {args:?}"
            );
        }
    }
}
