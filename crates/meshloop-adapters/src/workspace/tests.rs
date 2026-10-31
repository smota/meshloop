use super::schema::{BASE_SCHEMA, migrate_with_fault};
use super::*;
use crate::store::SqliteStore;
use meshloop_domain::evidence::AttemptId;
use meshloop_domain::state::PlanState;
use meshloop_engine::ports::{RunRow, RunStore};
use meshloop_engine::workspace::WorkspaceLedger;
use rusqlite::{Connection, params};

struct Fixture {
    base: PathBuf,
    repo: PathBuf,
    ledger: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let base =
            std::env::temp_dir().join(format!("meshloop-workspace-s2-{}", uuid::Uuid::new_v4()));
        let repo = base.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "--quiet"]);
        git(
            &repo,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "--allow-empty",
                "-m",
                "fixture",
            ],
        );
        let ledger = base.join("ledger.sqlite");
        SqliteStore::open(&ledger).unwrap();
        Self { base, repo, ledger }
    }
    fn lease(&self) -> RepositoryLease {
        GitRepositoryIdentity::new("git")
            .acquire(&self.repo, &self.ledger)
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // Only a unique disposable test fixture beneath OS temp may be removed.
        assert!(self.base.starts_with(std::env::temp_dir()));
        assert!(
            self.base
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("meshloop-workspace-s2-")
        );
        let _ = std::fs::remove_dir_all(&self.base);
    }
}
fn git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Git fixture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn id(s: &str) -> LifecycleId {
    LifecycleId::new(s).unwrap()
}
fn alias() -> RootAlias {
    RootAlias::new("worktrees").unwrap()
}
fn hash() -> PlanHash {
    PlanHash::new("a".repeat(64)).unwrap()
}
fn run() -> RunRow {
    RunRow {
        graph_id: "graph-alpha".into(),
        plan_state: PlanState::PlanAccepted,
        run_base: "base".into(),
        integrate_ref: "integrate".into(),
        plan_json: "{}".into(),
        plan_id: hash().as_str().into(),
        created_at: "1".into(),
        review_note: None,
    }
}
fn generation(lease: &RepositoryLease, n: u32) -> GenerationRecord {
    GenerationRecord {
        id: id(&format!("generation-{n}")),
        repository_id: lease.repository().id.clone(),
        graph_id: id("graph-alpha"),
        ordinal: n,
        accepted_plan_hash: hash(),
        root_alias: alias(),
        relative_path: RelativePath::new(format!("run/generation-{n}")).unwrap(),
        layout_version: 1,
    }
}
fn artifact(name: &str, generation: &GenerationRecord, kind: ArtifactKind) -> ArtifactRecord {
    ArtifactRecord {
        id: id(name),
        generation_id: generation.id.clone(),
        kind,
        root_alias: alias(),
        relative_path: RelativePath::new(format!("run/{name}")).unwrap(),
        attempt_id: None,
    }
}
fn bound(f: &Fixture) -> (SqliteStore, RepositoryLease) {
    let mut store = SqliteStore::open(&f.ledger).unwrap();
    let mut lease = f.lease();
    store.bind_repository(&lease).unwrap();
    lease.register_root(alias(), &f.repo).unwrap();
    store.register_root(&lease, &alias()).unwrap();
    store.save_run(&run()).unwrap();
    (store, lease)
}
fn seed_v4(conn: &Connection) {
    conn.execute_batch(BASE_SCHEMA).unwrap();
    conn.execute("INSERT INTO schema_meta VALUES(4)", [])
        .unwrap();
    for (graph, attempt) in [("graph-alpha", 7), ("graph-beta", 8)] {
        conn.execute("INSERT INTO runs VALUES(?1,'PlanAccepted','base','integrate','{}',?2,'10','retained review')",params![graph,hash().as_str()]).unwrap();
        conn.execute("INSERT INTO attempts VALUES(?1,?2,1,'fixture','model','C:/legacy/root/attempt',NULL,NULL,'10','20','done','pane')",params![attempt,graph]).unwrap();
        conn.execute("INSERT INTO events(graph_id,task_id,attempt_id,from_state,to_state,event_type,reason,executor,evidence_ref,occurred_at) VALUES(?1,1,?2,'Ready','Running','AttemptStarted','retained reason','fixture','retained evidence','10')",params![graph,attempt]).unwrap();
        conn.execute(
            "INSERT INTO evidence VALUES(1,?1,'revision','deterministic',?2)",
            params![attempt, "{\"retained\":\"unicode Ω and exact bytes\"}"],
        )
        .unwrap();
    }
    conn.execute_batch("INSERT INTO routing_feedback VALUES('fixture','model','Tier2',3,4); INSERT INTO quota_state VALUES('fixture','Closed',NULL,100);").unwrap();
}
fn rows(conn: &Connection, table: &str) -> Vec<Vec<rusqlite::types::Value>> {
    let mut statement = conn
        .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
        .unwrap();
    let columns = statement.column_count();
    statement
        .query_map([], |r| (0..columns).map(|i| r.get(i)).collect())
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}
fn history(conn: &Connection) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    [
        "runs",
        "events",
        "attempts",
        "evidence",
        "routing_feedback",
        "quota_state",
    ]
    .iter()
    .map(|t| rows(conn, t))
    .collect()
}
fn schema_snapshot(conn: &Connection) -> Vec<(String, Option<String>)> {
    let mut stmt = conn
        .prepare("SELECT name,sql FROM sqlite_master ORDER BY name")
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

#[test]
fn migration_v4_preserves_legacy_unbound() {
    let f = Fixture::new();
    let path = f.base.join("v4.sqlite");
    let conn = Connection::open(&path).unwrap();
    seed_v4(&conn);
    let before = history(&conn);
    let original_id = native_identity(&path).unwrap();
    drop(conn);
    let store = SqliteStore::open(&path).unwrap();
    assert_eq!(before, history(&store.conn));
    assert_eq!(original_id, native_identity(&path).unwrap());
    assert_eq!(
        rows(&store.conn, "schema_meta")[0][0],
        rusqlite::types::Value::Integer(5)
    );
    assert_eq!(rows(&store.conn, "workspace_legacy_unbound").len(), 4);
    assert!(rows(&store.conn, "workspace_repositories").is_empty());
    assert!(rows(&store.conn, "workspace_generations").is_empty());
    assert!(
        rows(&store.conn, "workspace_legacy_unbound")
            .iter()
            .all(|row| row[2] == rusqlite::types::Value::Text("Unresolved".into()))
    );
}
#[test]
fn migration_v1_v3_explicit_compatibility() {
    for version in [1, 2, 3] {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(if version == 1 { V1_SCHEMA } else { V2_SCHEMA })
            .unwrap();
        conn.execute("INSERT INTO schema_meta VALUES(?1)", [version])
            .unwrap();
        for attempt in [7, 8] {
            conn.execute(
                "INSERT INTO evidence VALUES(1,?1,'old','deterministic','exact legacy payload Ω')",
                [attempt],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO routing_feedback VALUES('fixture','model','Tier1',2,3)",
            [],
        )
        .unwrap();
        let mut tables = vec!["evidence", "routing_feedback"];
        if version != 1 {
            for (graph, attempt) in [("graph-alpha", 7), ("graph-beta", 8)] {
                conn.execute(
                    "INSERT INTO runs VALUES(?1,'PlanAccepted','base','integrate','{}',?2,'10')",
                    params![graph, hash().as_str()],
                )
                .unwrap();
                conn.execute("INSERT INTO attempts VALUES(?1,?2,1,'fixture','model','C:/retained/legacy-root',NULL,NULL,'10','20','done')",params![attempt,graph]).unwrap();
                conn.execute("INSERT INTO events(graph_id,task_id,attempt_id,from_state,to_state,event_type,reason,executor,evidence_ref,occurred_at) VALUES(?1,1,?2,'Ready','Running','AttemptStarted','exact reason','fixture','exact evidence','10')",params![graph,attempt]).unwrap();
            }
            conn.execute(
                "INSERT INTO quota_state VALUES('fixture','Closed',NULL,100)",
                [],
            )
            .unwrap();
            tables.extend(["runs", "attempts", "events", "quota_state"]);
        }
        let before_schema = schema_snapshot(&conn);
        let before: Vec<_> = tables
            .iter()
            .map(|table| {
                let mut stmt = conn
                    .prepare(&format!("PRAGMA table_info({table})"))
                    .unwrap();
                let columns = stmt
                    .query_map([], |row| row.get::<_, String>(1))
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap();
                (*table, columns, rows(&conn, table))
            })
            .collect();
        if version == 3 {
            // Synthetic unknown-version sentinel, not an invented historical v3 contract.
            assert!(schema::migrate(&conn).is_err());
            assert_eq!(before_schema, schema_snapshot(&conn));
            assert_eq!(
                rows(&conn, "schema_meta")[0][0],
                rusqlite::types::Value::Integer(3)
            );
        } else {
            schema::migrate(&conn).unwrap();
            assert_eq!(
                rows(&conn, "schema_meta")[0][0],
                rusqlite::types::Value::Integer(5)
            );
        }
        for (table, columns, before_rows) in before {
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT {} FROM {table} ORDER BY rowid",
                    columns.join(",")
                ))
                .unwrap();
            let count = columns.len();
            let after = stmt
                .query_map([], |row| {
                    (0..count)
                        .map(|i| row.get::<_, rusqlite::types::Value>(i))
                        .collect::<Result<Vec<_>, _>>()
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert_eq!(before_rows, after, "source v{version}, table {table}");
        }
    }
}
#[test]
fn migration_newer_schema_no_write() {
    let f = Fixture::new();
    let path = f.base.join("future.sqlite");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TABLE schema_meta(version INTEGER NOT NULL); INSERT INTO schema_meta VALUES(99); CREATE TABLE sentinel(value TEXT); INSERT INTO sentinel VALUES('untouched');").unwrap();
    let before_schema = schema_snapshot(&conn);
    let before = rows(&conn, "sentinel");
    let metadata = rows(&conn, "schema_meta");
    let mode: String = conn
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    let identity = native_identity(&path).unwrap();
    drop(conn);
    assert!(SqliteStore::open(&path).is_err());
    let conn = Connection::open(&path).unwrap();
    assert_eq!(before_schema, schema_snapshot(&conn));
    assert_eq!(before, rows(&conn, "sentinel"));
    assert_eq!(metadata, rows(&conn, "schema_meta"));
    assert_eq!(
        mode,
        conn.query_row::<String, _, _>("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap()
    );
    assert_eq!(identity, native_identity(&path).unwrap());
    assert!(!path.with_extension("sqlite-wal").exists());
}
#[test]
fn migration_transaction_rollback() {
    let f = Fixture::new();
    let path = f.base.join("rollback.sqlite");
    let conn = Connection::open(&path).unwrap();
    seed_v4(&conn);
    let before = history(&conn);
    let before_schema = schema_snapshot(&conn);
    let identity = native_identity(&path).unwrap();
    assert!(migrate_with_fault(&conn, true).is_err());
    assert_eq!(before, history(&conn));
    assert_eq!(before_schema, schema_snapshot(&conn));
    assert_eq!(identity, native_identity(&path).unwrap());
    assert_eq!(
        rows(&conn, "schema_meta")[0][0],
        rusqlite::types::Value::Integer(4)
    );
    schema::migrate(&conn).unwrap();
}
#[test]
fn identity_common_dir_and_linked_worktree() {
    let f = Fixture::new();
    let linked = f.base.join("linked");
    git(
        &f.repo,
        &["worktree", "add", "--detach", linked.to_str().unwrap()],
    );
    let lease = f.lease();
    let identity = lease.repository().clone();
    assert!(
        GitRepositoryIdentity::new("git")
            .acquire(&linked, &f.ledger)
            .is_err()
    );
    drop(lease);
    let lease = GitRepositoryIdentity::new("git")
        .acquire(&linked, &f.ledger)
        .unwrap();
    assert_eq!(identity, *lease.repository());
    let mut store = SqliteStore::open(&f.ledger).unwrap();
    store.bind_repository(&lease).unwrap();
    drop(lease);
    let main = f.lease();
    store.bind_repository(&main).unwrap();
    assert_eq!(identity, *main.repository());
}
#[test]
fn identity_same_basename_distinct_filesystems() {
    let a = Fixture::new();
    let b = Fixture::new();
    assert_eq!(a.repo.file_name(), b.repo.file_name());
    let first = a.lease();
    let second = b.lease();
    assert_ne!(first.repository(), second.repository());
    let mut store_a = SqliteStore::open(&a.ledger).unwrap();
    store_a.bind_repository(&first).unwrap();
    let mut store_b = SqliteStore::open(&b.ledger).unwrap();
    store_b.bind_repository(&second).unwrap();
    assert!(store_a.bind_repository(&second).is_err());
    // Even a legitimate lease targeting the same DB cannot bind a second repository.
    drop(second);
    drop(first);
    let c = Fixture::new();
    let unrelated = GitRepositoryIdentity::new("git")
        .acquire(&c.repo, &a.ledger)
        .unwrap();
    assert!(store_a.bind_repository(&unrelated).is_err());
}
#[test]
fn identity_copied_uuid_conflict() {
    let original = Fixture::new();
    let copy = Fixture::new();
    let lease = original.lease();
    let identity = lease.repository().clone();
    drop(lease);
    let manifest = std::fs::read(original.repo.join(".git").join(REGISTRY)).unwrap();
    std::fs::write(copy.repo.join(".git").join(REGISTRY), &manifest).unwrap();
    assert!(
        GitRepositoryIdentity::new("git")
            .acquire(&copy.repo, &original.ledger)
            .is_err()
    );
    assert_eq!(
        manifest,
        std::fs::read(copy.repo.join(".git").join(REGISTRY)).unwrap()
    );
    let lease = original.lease();
    assert_eq!(identity, *lease.repository());
    let mut store = SqliteStore::open(&original.ledger).unwrap();
    store.bind_repository(&lease).unwrap();
}
#[test]
fn ledger_alternate_database_and_replacement() {
    let f = Fixture::new();
    let lease = f.lease();
    let identity = lease.repository().clone();
    drop(lease);
    let registry = f.repo.join(".git").join(REGISTRY);
    let before = std::fs::read(&registry).unwrap();
    let alternate = f.base.join("alternate.sqlite");
    std::fs::copy(&f.ledger, &alternate).unwrap();
    assert!(
        GitRepositoryIdentity::new("git")
            .acquire(&f.repo, &alternate)
            .is_err()
    );
    let original = f.base.join("original-retained.sqlite");
    std::fs::rename(&f.ledger, &original).unwrap();
    std::fs::copy(&original, &f.ledger).unwrap();
    assert_ne!(
        native_identity(&original).unwrap(),
        native_identity(&f.ledger).unwrap()
    );
    assert!(
        GitRepositoryIdentity::new("git")
            .acquire(&f.repo, &f.ledger)
            .is_err()
    );
    assert_eq!(before, std::fs::read(&registry).unwrap());
    // Restore the exact original file, proving refused replacement did not transfer authority.
    let refused_replacement = f.base.join("refused-replacement.sqlite");
    std::fs::rename(&f.ledger, &refused_replacement).unwrap();
    std::fs::rename(&original, &f.ledger).unwrap();
    assert_eq!(identity, *f.lease().repository());
}
#[test]
fn workspace_fence_child() {
    let Ok(repo) = std::env::var("MESHLOOP_S2_FENCE_REPO") else {
        return;
    };
    let ledger = std::env::var("MESHLOOP_S2_FENCE_LEDGER").unwrap();
    let expected = std::env::var("MESHLOOP_S2_FENCE_EXPECT").unwrap();
    let result = GitRepositoryIdentity::new("git").acquire(Path::new(&repo), Path::new(&ledger));
    assert_eq!(result.is_ok(), expected == "acquire");
}
fn contender(f: &Fixture, expected: &str) {
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "workspace::tests::workspace_fence_child",
            "--nocapture",
        ])
        .env("MESHLOOP_S2_FENCE_REPO", &f.repo)
        .env("MESHLOOP_S2_FENCE_LEDGER", &f.ledger)
        .env("MESHLOOP_S2_FENCE_EXPECT", expected)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}
#[test]
fn fence_cross_process_and_unknown_filesystem() {
    let f = Fixture::new();
    let lease = f.lease();
    contender(&f, "refuse");
    drop(lease);
    contender(&f, "acquire");
    // Inject the actual acquisition failure boundary, never a file-exists fallback.
    let unsupported = |_: &File| Err(refused());
    assert!(
        GitRepositoryIdentity::new("git")
            .acquire_with_hooks(
                &f.repo,
                &f.ledger,
                &native_identity,
                &|_| Ok(()),
                &unsupported
            )
            .is_err()
    );
}
#[test]
fn identity_unavailable_common_or_ledger_file_id_refuses_without_mutation() {
    use std::cell::Cell;
    let f = Fixture::new();
    let registry = f.repo.join(".git").join(REGISTRY);
    let before = std::fs::read(&f.ledger).unwrap();
    for fail_at in [1, 2] {
        let calls = Cell::new(0);
        let probe = |path: &Path| {
            let n = calls.get() + 1;
            calls.set(n);
            if n == fail_at {
                Err(refused())
            } else {
                native_identity(path)
            }
        };
        assert!(
            GitRepositoryIdentity::new("git")
                .acquire_with(&f.repo, &f.ledger, &probe)
                .is_err()
        );
        assert!(!registry.exists());
        assert_eq!(before, std::fs::read(&f.ledger).unwrap());
        assert_eq!(calls.get(), fail_at);
    }
    let lease = f.lease();
    let mut bound_store = SqliteStore::open(&f.ledger).unwrap();
    bound_store.bind_repository(&lease).unwrap();
    drop(bound_store);
    drop(lease);
    let manifest = std::fs::read(&registry).unwrap();
    let bound_bytes = std::fs::read(&f.ledger).unwrap();
    let bound_connection = Connection::open(&f.ledger).unwrap();
    let bound_history = history(&bound_connection);
    let bound_repository = rows(&bound_connection, "workspace_repositories");
    assert_eq!(bound_repository.len(), 1);
    for fail_at in [1, 2] {
        let calls = Cell::new(0);
        let probe = |path: &Path| {
            let n = calls.get() + 1;
            calls.set(n);
            if n == fail_at {
                Err(refused())
            } else {
                native_identity(path)
            }
        };
        assert!(
            GitRepositoryIdentity::new("git")
                .acquire_with(&f.repo, &f.ledger, &probe)
                .is_err()
        );
        assert_eq!(manifest, std::fs::read(&registry).unwrap());
        assert_eq!(bound_bytes, std::fs::read(&f.ledger).unwrap());
        assert_eq!(bound_history, history(&bound_connection));
        assert_eq!(
            bound_repository,
            rows(&bound_connection, "workspace_repositories")
        );
    }
}
#[test]
fn ledger_generation_records_pin_plan_hash_and_preserve_history() {
    let f = Fixture::new();
    let (mut store, lease) = bound(&f);
    let first = generation(&lease, 1);
    store.insert_generation(&lease, &first).unwrap();
    let artifact = artifact("plan-artifact", &first, ArtifactKind::Plan);
    store
        .insert_artifacts(&lease, std::slice::from_ref(&artifact), &[])
        .unwrap();
    let before = history(&store.conn);
    let second = generation(&lease, 2);
    store.insert_generation(&lease, &second).unwrap();
    let mut rejected = generation(&lease, 3);
    rejected.accepted_plan_hash = PlanHash::new("b".repeat(64)).unwrap();
    assert!(store.insert_generation(&lease, &rejected).is_err());
    assert_eq!(
        vec![first.clone(), second],
        store
            .generations(&lease.repository().id, &id("graph-alpha"))
            .unwrap()
    );
    assert_eq!(Some(first), store.generation(&id("generation-1")).unwrap());
    assert_eq!(
        Some(artifact),
        store.artifact(&id("plan-artifact")).unwrap()
    );
    assert_eq!(before, history(&store.conn));
    assert!(
        store
            .insert_generation(&lease, &generation(&lease, 1))
            .is_err()
    );
    let mut new_acceptance = run();
    new_acceptance.plan_id = rejected.accepted_plan_hash.as_str().into();
    store.save_run(&new_acceptance).unwrap();
    store.insert_generation(&lease, &rejected).unwrap();
}
#[test]
fn ledger_artifact_dependency_roundtrip_and_foreign_keys() {
    let f = Fixture::new();
    let (mut store, mut lease) = bound(&f);
    let alternate_alias = RootAlias::new("alternate").unwrap();
    lease
        .register_root(alternate_alias.clone(), &f.base)
        .unwrap();
    store.register_root(&lease, &alternate_alias).unwrap();
    let first = generation(&lease, 1);
    let second = generation(&lease, 2);
    store.insert_generation(&lease, &first).unwrap();
    store.insert_generation(&lease, &second).unwrap();
    let plan = artifact("plan", &first, ArtifactKind::Plan);
    let deliverable = artifact("delivered", &second, ArtifactKind::Deliverable);
    let integration = artifact("integrated", &second, ArtifactKind::Integration);
    let edges = vec![
        ArtifactDependency {
            artifact_id: deliverable.id.clone(),
            requires_artifact_id: plan.id.clone(),
        },
        ArtifactDependency {
            artifact_id: integration.id.clone(),
            requires_artifact_id: plan.id.clone(),
        },
    ];
    store
        .insert_artifacts(
            &lease,
            &[plan.clone(), deliverable.clone(), integration.clone()],
            &edges,
        )
        .unwrap();
    let facts = vec![
        LifecycleFact {
            id: id("delivery"),
            artifact_id: deliverable.id.clone(),
            fact: Fact::Delivery(Delivery::Delivered),
        },
        LifecycleFact {
            id: id("retention"),
            artifact_id: deliverable.id.clone(),
            fact: Fact::Retention(Retention::Required),
        },
        LifecycleFact {
            id: id("reclamation"),
            artifact_id: deliverable.id.clone(),
            fact: Fact::Reclamation(Reclamation::Pending),
        },
    ];
    for fact in &facts {
        store.append_fact(&lease, fact).unwrap();
    }
    let journal = JournalEntry {
        id: id("step-intent"),
        artifact_id: deliverable.id.clone(),
        operation_id: id("operation"),
        step: 1,
        state: JournalState::Intent,
    };
    store.append_journal(&lease, &journal).unwrap();
    assert!(
        store
            .register_root(&lease, &RootAlias::new("unknown").unwrap())
            .is_err()
    );
    let mut bad = generation(&lease, 3);
    bad.graph_id = id("missing-graph");
    assert!(store.insert_generation(&lease, &bad).is_err());
    let mut wrong_root = artifact("wrong-root", &second, ArtifactKind::Plan);
    wrong_root.root_alias = alternate_alias;
    assert!(
        store
            .insert_artifacts(&lease, std::slice::from_ref(&wrong_root), &[])
            .is_err()
    );
    assert!(store.artifact(&wrong_root.id).unwrap().is_none());
    let orphan = artifact(
        "orphan",
        &GenerationRecord {
            id: id("missing-generation"),
            ..first.clone()
        },
        ArtifactKind::Plan,
    );
    assert!(store.insert_artifacts(&lease, &[orphan], &[]).is_err());
    let fresh = artifact("transaction-must-rollback", &second, ArtifactKind::Plan);
    let dangling = ArtifactDependency {
        artifact_id: fresh.id.clone(),
        requires_artifact_id: id("missing-artifact"),
    };
    assert!(
        store
            .insert_artifacts(&lease, std::slice::from_ref(&fresh), &[dangling])
            .is_err()
    );
    assert!(store.artifact(&fresh.id).unwrap().is_none());
    let existing_root = lease.root(&alias()).unwrap();
    // Root alias identity is immutable even when another directory is legitimate.
    // This lease is immutable here; a separate fixture below tests mutable registration.
    assert_eq!(existing_root.repository_id, lease.repository().id);
    store
        .conn
        .execute(
            "INSERT INTO attempts(attempt_id,graph_id,task_id) VALUES(55,'other-graph',1)",
            [],
        )
        .unwrap();
    let mut wrong_graph = artifact("wrong-graph-attempt", &second, ArtifactKind::Attempt);
    wrong_graph.attempt_id = Some(AttemptId(55));
    assert!(store.insert_artifacts(&lease, &[wrong_graph], &[]).is_err());
    let mut attempt = artifact("attempt", &second, ArtifactKind::Attempt);
    attempt.attempt_id = Some(AttemptId(999));
    assert!(store.insert_artifacts(&lease, &[attempt], &[]).is_err());
    // Constraint enforcement also survives callers bypassing typed insert prechecks.
    assert!(
        store
            .conn
            .execute(
                "INSERT INTO workspace_dependencies VALUES('missing','plan')",
                []
            )
            .is_err()
    );
    assert!(store.conn.execute("INSERT INTO workspace_generations VALUES('missing-repo','none','graph-alpha',99,'worktrees','hash','{}')",[]).is_err());
    drop(store);
    let mut store = SqliteStore::open(&f.ledger).unwrap();
    store.bind_repository(&lease).unwrap();
    for record in [plan, deliverable.clone(), integration.clone()] {
        assert_eq!(Some(record.clone()), store.artifact(&record.id).unwrap());
        let json = serde_json::to_string(&record).unwrap();
        assert!(!json.contains(&f.base.to_string_lossy().to_string()));
    }
    assert_eq!(
        vec![edges[0].clone()],
        store.dependencies(&deliverable.id).unwrap()
    );
    assert_eq!(
        vec![edges[1].clone()],
        store.dependencies(&integration.id).unwrap()
    );
    assert_eq!(facts, store.facts(&deliverable.id).unwrap());
    assert_eq!(vec![journal], store.journal(&deliverable.id).unwrap());
    assert!(
        store
            .append_fact(
                &lease,
                &LifecycleFact {
                    id: id("dangling-fact"),
                    artifact_id: id("missing"),
                    fact: Fact::Retention(Retention::Required)
                }
            )
            .is_err()
    );
}

#[test]
fn workspace_fence_replaced_registry_handle_refuses() {
    let f = Fixture::new();
    let lease = f.lease();
    drop(lease);
    let registry = f.repo.join(".git").join(REGISTRY);
    let before = std::fs::read(&registry).unwrap();
    let retained = f.base.join("original-registry.json");
    let replace = |path: &Path| {
        std::fs::rename(path, &retained).map_err(io_refused)?;
        std::fs::write(path, &before).map_err(io_refused)?;
        Ok(())
    };
    assert!(
        GitRepositoryIdentity::new("git")
            .acquire_with_hooks(&f.repo, &f.ledger, &native_identity, &replace, &lock)
            .is_err()
    );
    assert_eq!(before, std::fs::read(&registry).unwrap());
    let replacement = f.base.join("replacement-registry.json");
    std::fs::rename(&registry, &replacement).unwrap();
    std::fs::rename(&retained, &registry).unwrap();
    let lease = f.lease();
    std::fs::rename(&registry, &retained).unwrap();
    std::fs::write(&registry, &before).unwrap();
    assert!(lease.validate().is_err());
}
#[test]
fn workspace_git_process_child() {
    let Ok(mode) = std::env::var("MESHLOOP_S2_PROCESS_CHILD") else {
        return;
    };
    if mode == "sleep" {
        std::thread::sleep(Duration::from_secs(10));
    } else {
        print!("{}", "x".repeat(20 * 1024));
    }
}
#[test]
fn workspace_git_process_is_bounded() {
    let command = |mode: &str| {
        let mut c = Command::new(std::env::current_exe().unwrap());
        c.args([
            "--exact",
            "workspace::tests::workspace_git_process_child",
            "--nocapture",
        ])
        .env("MESHLOOP_S2_PROCESS_CHILD", mode);
        c
    };
    let started = Instant::now();
    assert!(bounded_stdout(command("sleep"), Duration::from_millis(100)).is_err());
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(bounded_stdout(command("large"), Duration::from_secs(2)).is_err());
}

#[test]
fn workspace_root_alias_rebinding_refuses() {
    let f = Fixture::new();
    let (mut store, mut lease) = bound(&f);
    let before = lease.root(&alias()).unwrap();
    assert!(lease.register_root(alias(), &f.base).is_err());
    assert_eq!(before, lease.root(&alias()).unwrap());
    store.register_root(&lease, &alias()).unwrap();
}

#[test]
fn workspace_ledger_refuses_mismatched_database_handle() {
    let f = Fixture::new();
    let (mut store, lease) = bound(&f);
    let alternate = f.base.join("different-database.sqlite");
    std::fs::copy(&f.ledger, &alternate).unwrap();
    let (foreign_handle, _) = pin_database(&alternate).unwrap();
    store.database_handle = Some(foreign_handle);
    assert!(store.bind_repository(&lease).is_err());
    assert!(
        store
            .insert_generation(&lease, &generation(&lease, 1))
            .is_err()
    );
    assert!(
        store
            .generations(&lease.repository().id, &id("graph-alpha"))
            .unwrap()
            .is_empty()
    );
}

// Exact historical DDL, git c610ef3 (v1) and 64430f2 (v2).
const V1_SCHEMA: &str = r#"CREATE TABLE IF NOT EXISTS schema_meta (version INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS evidence (
                 task_id INTEGER NOT NULL,
                 attempt_id INTEGER NOT NULL,
                 revision TEXT NOT NULL,
                 kind TEXT NOT NULL,
                 payload_json TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS routing_feedback (
                 harness TEXT NOT NULL,
                 model_ref TEXT NOT NULL,
                 tier TEXT NOT NULL,
                 success_count INTEGER NOT NULL DEFAULT 0,
                 failure_count INTEGER NOT NULL DEFAULT 0,
                 PRIMARY KEY (harness, model_ref, tier)
             );"#;
const V2_SCHEMA: &str = r#"CREATE TABLE IF NOT EXISTS schema_meta (version INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS evidence (
                 task_id INTEGER NOT NULL,
                 attempt_id INTEGER NOT NULL,
                 revision TEXT NOT NULL,
                 kind TEXT NOT NULL,
                 payload_json TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS routing_feedback (
                 harness TEXT NOT NULL,
                 model_ref TEXT NOT NULL,
                 tier TEXT NOT NULL,
                 success_count INTEGER NOT NULL DEFAULT 0,
                 failure_count INTEGER NOT NULL DEFAULT 0,
                 PRIMARY KEY (harness, model_ref, tier)
             );
             CREATE TABLE IF NOT EXISTS events (
                 event_id INTEGER PRIMARY KEY AUTOINCREMENT,
                 graph_id TEXT NOT NULL,
                 task_id INTEGER NOT NULL,
                 attempt_id INTEGER,
                 from_state TEXT NOT NULL,
                 to_state TEXT NOT NULL,
                 event_type TEXT NOT NULL,
                 reason TEXT,
                 executor TEXT NOT NULL,
                 evidence_ref TEXT,
                 occurred_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS runs (
                 graph_id TEXT PRIMARY KEY,
                 plan_state TEXT NOT NULL,
                 run_base TEXT NOT NULL,
                 integrate_ref TEXT NOT NULL,
                 plan_json TEXT NOT NULL,
                 plan_sha256 TEXT NOT NULL,
                 created_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS attempts (
                 attempt_id INTEGER PRIMARY KEY,
                 graph_id TEXT NOT NULL,
                 task_id INTEGER NOT NULL,
                 harness TEXT,
                 model_ref TEXT,
                 worktree_path TEXT,
                 pid INTEGER,
                 image_name TEXT,
                 started_at TEXT,
                 ended_at TEXT,
                 outcome TEXT
             );
             CREATE TABLE IF NOT EXISTS quota_state (
                 harness TEXT PRIMARY KEY,
                 breaker TEXT NOT NULL,
                 opened_at TEXT,
                 cooldown_ms INTEGER NOT NULL
             );"#;

#[test]
fn workspace_v5_missing_constraints_refuse_before_wal() {
    let f = Fixture::new();
    let variants = [
        (
            "missing-fk",
            "artifact_id TEXT NOT NULL REFERENCES workspace_artifacts(id), requires_id",
            "artifact_id TEXT NOT NULL, requires_id",
        ),
        (
            "missing-pk",
            "CREATE TABLE workspace_facts (id TEXT PRIMARY KEY,",
            "CREATE TABLE workspace_facts (id TEXT,",
        ),
        (
            "missing-unique",
            "common_native_id TEXT UNIQUE NOT NULL",
            "common_native_id TEXT NOT NULL",
        ),
        (
            "missing-check",
            "ordinal INTEGER NOT NULL CHECK(ordinal>0)",
            "ordinal INTEGER NOT NULL",
        ),
    ];
    for (name, expected, replacement) in variants {
        let path = f.base.join(format!("{name}.sqlite"));
        let conn = Connection::open(&path).unwrap();
        seed_v4(&conn);
        let altered = schema::LIFECYCLE_SCHEMA.replace(expected, replacement);
        assert_ne!(altered, schema::LIFECYCLE_SCHEMA);
        conn.execute_batch(&altered).unwrap();
        conn.execute("UPDATE schema_meta SET version=5", [])
            .unwrap();
        let before_schema = schema_snapshot(&conn);
        let before_history = history(&conn);
        let before_legacy = rows(&conn, "workspace_legacy_unbound");
        let before_mode: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        let identity = native_identity(&path).unwrap();
        drop(conn);
        assert!(SqliteStore::open(&path).is_err(), "accepted {name}");
        let conn = Connection::open(&path).unwrap();
        assert_eq!(before_schema, schema_snapshot(&conn));
        assert_eq!(before_history, history(&conn));
        assert_eq!(before_legacy, rows(&conn, "workspace_legacy_unbound"));
        assert_eq!(identity, native_identity(&path).unwrap());
        assert_eq!(
            before_mode,
            conn.query_row::<String, _, _>("PRAGMA journal_mode", [], |r| r.get(0))
                .unwrap()
        );
    }
}
