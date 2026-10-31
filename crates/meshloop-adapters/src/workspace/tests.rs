use super::schema::{BASE_SCHEMA, migrate_with_fault};
use super::*;
use crate::process::TestCommandExt;
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
        .guarded_output()
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
        graph_id: GraphId::new("graph-alpha").unwrap(),
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
    let canonical = Connection::open_in_memory().unwrap();
    canonical.execute_batch(BASE_SCHEMA).unwrap();
    [
        "runs",
        "events",
        "attempts",
        "evidence",
        "routing_feedback",
        "quota_state",
    ]
    .iter()
    .map(|table| {
        let mut info = canonical
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap();
        let columns = info
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let mut statement = conn
            .prepare(&format!(
                "SELECT {} FROM {table} ORDER BY rowid",
                columns.join(",")
            ))
            .unwrap();
        let count = columns.len();
        statement
            .query_map([], |r| {
                (0..count).map(|i| r.get(i)).collect::<Result<Vec<_>, _>>()
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    })
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
        rusqlite::types::Value::Integer(6)
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
                rusqlite::types::Value::Integer(6)
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
        .guarded_output()
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
            .generations(
                &lease.repository().id,
                &GraphId::new("graph-alpha").unwrap()
            )
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
    bad.graph_id = GraphId::new("missing-graph").unwrap();
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
        let mut c = OwnedSpawnSpec::new(std::env::current_exe().unwrap());
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
            .generations(
                &lease.repository().id,
                &GraphId::new("graph-alpha").unwrap()
            )
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

// B1 persistence fixtures: no public producer or CLI activation is implied.
fn seeded_v5(path: &Path) -> Connection {
    let conn = Connection::open(path).unwrap();
    seed_v4(&conn);
    conn.execute_batch(schema::LIFECYCLE_SCHEMA).unwrap();
    conn.execute("UPDATE schema_meta SET version=5", [])
        .unwrap();
    let repo = RepositoryRecord {
        id: id("retained-repository"),
        common_dir_identity: NativeIdentity::new("a".repeat(64)).unwrap(),
    };
    conn.execute(
        "INSERT INTO workspace_repositories VALUES(?1,?2,?3,?4)",
        params![
            repo.id.as_str(),
            repo.common_dir_identity.as_str(),
            "b".repeat(64),
            serde_json::to_string(&repo).unwrap()
        ],
    )
    .unwrap();
    let root = RootRecord {
        repository_id: repo.id.clone(),
        alias: alias(),
        native_identity: NativeIdentity::new("c".repeat(64)).unwrap(),
    };
    conn.execute(
        "INSERT INTO workspace_roots VALUES(?1,?2,?3)",
        params![
            repo.id.as_str(),
            alias().as_str(),
            serde_json::to_string(&root).unwrap()
        ],
    )
    .unwrap();
    let generation = GenerationRecord {
        id: id("retained-generation"),
        repository_id: repo.id.clone(),
        graph_id: GraphId::new("graph-alpha").unwrap(),
        ordinal: 1,
        accepted_plan_hash: hash(),
        root_alias: alias(),
        relative_path: RelativePath::new("retained/run").unwrap(),
        layout_version: 1,
    };
    conn.execute(
        "INSERT INTO workspace_generations VALUES(?1,?2,?3,1,?4,?5,?6)",
        params![
            generation.id.as_str(),
            repo.id.as_str(),
            generation.graph_id.as_str(),
            alias().as_str(),
            hash().as_str(),
            serde_json::to_string(&generation).unwrap()
        ],
    )
    .unwrap();
    for name in ["retained-plan", "retained-integration"] {
        let artifact = artifact(name, &generation, ArtifactKind::Plan);
        conn.execute(
            "INSERT INTO workspace_artifacts VALUES(?1,?2,?3,?4,NULL,?5)",
            params![
                name,
                generation.id.as_str(),
                repo.id.as_str(),
                alias().as_str(),
                serde_json::to_string(&artifact).unwrap()
            ],
        )
        .unwrap();
    }
    conn.execute_batch("INSERT INTO workspace_dependencies VALUES('retained-integration','retained-plan'); INSERT INTO workspace_facts VALUES('retained-fact','retained-plan','retained exact payload'); INSERT INTO workspace_journal VALUES('retained-journal','retained-plan','retained exact journal');").unwrap();
    conn
}
fn lifecycle_snapshot(c: &Connection) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    [
        "workspace_repositories",
        "workspace_roots",
        "workspace_generations",
        "workspace_artifacts",
        "workspace_dependencies",
        "workspace_facts",
        "workspace_journal",
        "workspace_legacy_unbound",
    ]
    .iter()
    .map(|t| rows(c, t))
    .collect()
}
#[test]
fn lifecycle_fence_v5_v6_additive_migration_preserves_legacy_rows() {
    let f = Fixture::new();
    let path = f.base.join("schema5.sqlite");
    let c = seeded_v5(&path);
    let before = history(&c);
    let lifecycle = lifecycle_snapshot(&c);
    let identity = native_identity(&path).unwrap();
    drop(c);
    let store = SqliteStore::open(&path).unwrap();
    assert_eq!(before, history(&store.conn));
    assert_eq!(lifecycle, lifecycle_snapshot(&store.conn));
    assert_eq!(identity, native_identity(&path).unwrap());
    for table in ["events", "attempts"] {
        let n:i64=store.conn.query_row(&format!("SELECT COUNT(*) FROM {table} WHERE generation_id IS NOT NULL OR revision_id IS NOT NULL"),[],|r|r.get(0)).unwrap();
        assert_eq!(n, 0);
    }
    let schema_after = schema_snapshot(&store.conn);
    drop(store);
    let reopened = SqliteStore::open(&path).unwrap();
    assert_eq!(schema_after, schema_snapshot(&reopened.conn));
    assert_eq!(before, history(&reopened.conn));
    assert_eq!(lifecycle, lifecycle_snapshot(&reopened.conn));
}
#[test]
fn lifecycle_fence_v6_rollback_and_future_schema_refusal() {
    let f = Fixture::new();
    let path = f.base.join("rollback5.sqlite");
    let c = seeded_v5(&path);
    let before = history(&c);
    let before_schema = schema_snapshot(&c);
    let lifecycle = lifecycle_snapshot(&c);
    assert!(schema::migrate_with_fault(&c, true).is_err());
    assert_eq!(before, history(&c));
    assert_eq!(before_schema, schema_snapshot(&c));
    assert_eq!(lifecycle, lifecycle_snapshot(&c));
    assert_eq!(schema::schema_version(&c).unwrap(), Some(5));
    let future = f.base.join("future7.sqlite");
    let c = Connection::open(&future).unwrap();
    c.execute_batch("CREATE TABLE schema_meta(version INTEGER);INSERT INTO schema_meta VALUES(7);CREATE TABLE sentinel(value TEXT);INSERT INTO sentinel VALUES('unchanged');").unwrap();
    let before_schema = schema_snapshot(&c);
    let before_rows = rows(&c, "sentinel");
    let mode: String = c
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    let identity = native_identity(&future).unwrap();
    drop(c);
    assert!(SqliteStore::open(&future).is_err());
    let c = Connection::open(&future).unwrap();
    assert_eq!(before_schema, schema_snapshot(&c));
    assert_eq!(before_rows, rows(&c, "sentinel"));
    assert_eq!(identity, native_identity(&future).unwrap());
    assert_eq!(
        mode,
        c.query_row::<String, _, _>("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap()
    );
}
#[test]
fn lifecycle_fence_readonly_absent_db_and_pre_v6_upgrade_required() {
    use meshloop_engine::workspace::{RuntimeLedger, StoredAttempt, StoredEvent};
    let f = Fixture::new();
    let absent = f.base.join("absent/nested/ledger.sqlite");
    assert!(SqliteStore::open_read_only(&absent).unwrap().is_none());
    assert!(!f.base.join("absent").exists());
    let path = f.base.join("readonly5.sqlite");
    let c = seeded_v5(&path);
    let before_schema = schema_snapshot(&c);
    let before = history(&c);
    let lifecycle = lifecycle_snapshot(&c);
    let identity = native_identity(&path).unwrap();
    let mode: String = c
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    drop(c);
    let bytes = std::fs::read(&path).unwrap();
    assert!(matches!(
        SqliteStore::open_read_only(&path),
        Err(StoreError::SchemaUpgradeRequired)
    ));
    assert_eq!(bytes, std::fs::read(&path).unwrap());
    let c = Connection::open(&path).unwrap();
    assert_eq!(before_schema, schema_snapshot(&c));
    assert_eq!(before, history(&c));
    assert_eq!(lifecycle, lifecycle_snapshot(&c));
    assert_eq!(identity, native_identity(&path).unwrap());
    assert_eq!(
        mode,
        c.query_row::<String, _, _>("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap()
    );
    drop(c);
    let migrated = SqliteStore::open(&path).unwrap();
    let mode: String = migrated
        .conn
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    let before_schema = schema_snapshot(&migrated.conn);
    drop(migrated);
    let bytes = std::fs::read(&path).unwrap();
    let entries_before = observation_entries(&f.base, &path);
    let mut reader = SqliteStore::open_read_only(&path).unwrap().unwrap();
    assert_eq!(before_schema, schema_snapshot(&reader.conn));
    assert_eq!(before, history(&reader.conn));
    assert_eq!(lifecycle, lifecycle_snapshot(&reader.conn));
    let graph = GraphId::new("graph-alpha").unwrap();
    assert!(
        reader
            .scoped_events(&graph, &RunScope::Legacy)
            .unwrap()
            .iter()
            .all(|e| matches!(e, StoredEvent::Legacy(_)))
    );
    let attempts = reader.scoped_attempts(&graph, &RunScope::Legacy).unwrap();
    assert!(
        matches!(&attempts[0],StoredAttempt::Legacy(row)if row.worktree_path.as_ref().unwrap().to_string_lossy()=="C:/legacy/root/attempt")
    );
    assert!(matches!(reader.save_run(&run()), Err(StoreError::ReadOnly)));
    assert_observer_sql_writes_rejected(&reader.conn);
    assert!(reader.run_binding(&graph).unwrap().is_none());
    assert_eq!(before_schema, schema_snapshot(&reader.conn));
    assert_eq!(before, history(&reader.conn));
    assert_eq!(lifecycle, lifecycle_snapshot(&reader.conn));
    assert_eq!(identity, native_identity(&path).unwrap());
    assert_eq!(bytes, std::fs::read(&path).unwrap());
    assert_eq!(
        mode,
        reader
            .conn
            .query_row::<String, _, _>("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap()
    );
    assert_eq!(entries_before, observation_entries(&f.base, &path));
    drop(reader);
    assert_eq!(bytes, std::fs::read(&path).unwrap());
    assert_eq!(entries_before, observation_entries(&f.base, &path));
    assert!(!f.repo.join(".git").join(REGISTRY).exists());
}

// Only exact SQLite sidecar names are exempt from entry preservation. A suffix
// match would accidentally allow other ledgers or application-owned files.
fn observation_entries(
    base: &Path,
    ledger: &Path,
) -> std::collections::BTreeSet<std::ffi::OsString> {
    let sidecar = |suffix| {
        let mut name = ledger.file_name().unwrap().to_os_string();
        name.push(suffix);
        name
    };
    let allowed = [sidecar("-wal"), sidecar("-shm")];
    std::fs::read_dir(base)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| !allowed.contains(name))
        .collect()
}

fn assert_observer_sql_writes_rejected(conn: &Connection) {
    for sql in [
        "UPDATE runs SET review_note='forbidden'",
        "UPDATE workspace_repositories SET payload_json='forbidden'",
        "DELETE FROM workspace_roots",
        "UPDATE schema_meta SET version=5",
        "CREATE TABLE forbidden_observer_write(value TEXT)",
    ] {
        let error = conn.execute_batch(sql).unwrap_err();
        assert_eq!(
            error.sqlite_error_code(),
            Some(rusqlite::ErrorCode::ReadOnly)
        );
    }
}

#[test]
fn lifecycle_observation_live_wal_child() {
    let Some(ledger) = std::env::var_os("MESHLOOP_OBSERVATION_LEDGER") else {
        return;
    };
    let repo = std::env::var_os("MESHLOOP_OBSERVATION_REPO").unwrap();
    assert!(
        GitRepositoryIdentity::new("git")
            .acquire(Path::new(&repo), Path::new(&ledger))
            .is_err()
    );
    let reader = SqliteStore::open_read_only(Path::new(&ledger))
        .unwrap()
        .unwrap();
    assert_eq!(
        reader
            .load_run("graph-alpha")
            .unwrap()
            .unwrap()
            .review_note
            .as_deref(),
        Some("committed-only-in-live-wal")
    );
    assert_observer_sql_writes_rejected(&reader.conn);
}

#[test]
fn lifecycle_observation_independent_process_reads_live_wal_while_fenced() {
    let f = Fixture::new();
    let (mut writer, lease) = bound(&f);
    writer
        .conn
        .pragma_update(None, "wal_autocheckpoint", 0)
        .unwrap();
    writer
        .conn
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
    let bytes = std::fs::read(&f.ledger).unwrap();
    let mut updated = run();
    updated.review_note = Some("committed-only-in-live-wal".into());
    writer.save_run(&updated).unwrap();
    assert_eq!(
        bytes,
        std::fs::read(&f.ledger).unwrap(),
        "fixture commit must reside only in WAL"
    );
    let mut wal = f.ledger.as_os_str().to_os_string();
    wal.push("-wal");
    assert!(std::fs::metadata(Path::new(&wal)).unwrap().len() > 32);
    let schema = schema_snapshot(&writer.conn);
    let domain = history(&writer.conn);
    let lifecycle = lifecycle_snapshot(&writer.conn);
    let identity = native_identity(&f.ledger).unwrap();
    let registry_bytes = locked_registry_bytes(&lease);
    let root = lease.root(&alias()).unwrap();
    let entries = observation_entries(&f.base, &f.ledger);
    let mut command = OwnedSpawnSpec::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "workspace::tests::lifecycle_observation_live_wal_child",
            "--nocapture",
        ])
        .env("MESHLOOP_OBSERVATION_LEDGER", &f.ledger)
        .env("MESHLOOP_OBSERVATION_REPO", &f.repo);
    let output = bounded_stdout(command, Duration::from_secs(10)).unwrap();
    let output = String::from_utf8(output).unwrap();
    assert!(
        output.contains("1 passed"),
        "child did not execute observation proof: {output}"
    );
    lease.validate().unwrap();
    assert_eq!(schema, schema_snapshot(&writer.conn));
    assert_eq!(domain, history(&writer.conn));
    assert_eq!(lifecycle, lifecycle_snapshot(&writer.conn));
    assert_eq!(identity, native_identity(&f.ledger).unwrap());
    assert_eq!(registry_bytes, locked_registry_bytes(&lease));
    assert_eq!(root, lease.root(&alias()).unwrap());
    assert_eq!(bytes, std::fs::read(&f.ledger).unwrap());
    assert_eq!(entries, observation_entries(&f.base, &f.ledger));
}

fn locked_registry_bytes(lease: &RepositoryLease) -> Vec<u8> {
    let mut file = lease.file.try_clone().unwrap();
    let position = file.stream_position().unwrap();
    file.seek(std::io::SeekFrom::Start(0)).unwrap();
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    file.seek(std::io::SeekFrom::Start(position)).unwrap();
    bytes
}
#[test]
#[ignore = "mandatory external acceptance proof: explicit compatible retained S2 binary and SHA256 required"]
fn retained_schema5_binary_refuses_schema6_without_mutation() {
    let executable = std::env::var_os("MESHLOOP_SCHEMA5_BINARY")
        .expect("MESHLOOP_SCHEMA5_BINARY must point to the retained actual S2 CLI");
    let expected =
        std::env::var("MESHLOOP_SCHEMA5_SHA256").expect("MESHLOOP_SCHEMA5_SHA256 is mandatory");
    let bytes = std::fs::read(&executable).expect("retained binary missing; no skip allowed");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        expected.to_ascii_lowercase(),
        "retained binary hash mismatch"
    );
    let f = Fixture::new();
    let path = f.base.join("v6-for-old-binary.sqlite");
    let c = seeded_v5(&path);
    schema::migrate(&c).unwrap();
    let before = history(&c);
    let lifecycle = lifecycle_snapshot(&c);
    let before_schema = schema_snapshot(&c);
    let mode: String = c
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    let identity = native_identity(&path).unwrap();
    drop(c);
    let config = f.base.join("fixture.toml");
    std::fs::write(&config,"selected_harnesses = [\"fixture\"]\n[limits]\nmax_concurrent_workers = 1\nmax_retries = 1\ntask_timeout_seconds = 30\n[verify]\nverify_command = []\n[harnesses.fixture]\nexecutable = \"git\"\nversion_args = [\"--version\"]\ninvoke_args_template = []\nmodel_ref = \"fixture\"\nmodel_tier = \"top\"\n").unwrap();
    let output = Command::new(executable)
        .current_dir(&f.repo)
        .args(["status", "--config"])
        .arg(config)
        .arg("--db")
        .arg(&path)
        .args(["--graph", "graph-alpha", "--json"])
        .guarded_output()
        .unwrap();
    let diagnostic = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.status.success());
    assert!(
        diagnostic.to_ascii_lowercase().contains("schema"),
        "did not reach schema refusal: {diagnostic}"
    );
    let c = Connection::open(&path).unwrap();
    assert_eq!(before, history(&c));
    assert_eq!(lifecycle, lifecycle_snapshot(&c));
    assert_eq!(before_schema, schema_snapshot(&c));
    assert_eq!(identity, native_identity(&path).unwrap());
    assert_eq!(
        mode,
        c.query_row::<String, _, _>("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap()
    );
}
fn runtime_fixture(
    f: &Fixture,
) -> (
    SqliteStore,
    RepositoryLease,
    meshloop_engine::workspace::GenerationActivation,
) {
    use meshloop_engine::workspace::*;
    let (mut store, lease) = bound(f);
    let mut row = run();
    row.plan_json=serde_json::json!({"graph_id":"graph-alpha","nodes":(1..=66).map(|n|serde_json::json!({"id":n,"description":"fixture","depends_on":[],"tier":"Tier1"})).collect::<Vec<_>>()}).to_string();
    row.plan_id = format!("{:x}", Sha256::digest(row.plan_json.as_bytes()));
    store.save_run(&row).unwrap();
    let b = RunBinding {
        graph_id: GraphId::new("graph-alpha").unwrap(),
        repository_id: lease.repository().id.clone(),
        location: PortableLocation {
            root_alias: alias(),
            relative_path: RelativePath::new("runs/opaque-run").unwrap(),
        },
        layout_version: 1,
        active_generation: None,
        current_revision: None,
    };
    store.create_run_binding(&lease, &b).unwrap();
    let mut generation = generation(&lease, 1);
    generation.accepted_plan_hash = PlanHash::new(row.plan_id).unwrap();
    let revision = PlanRevisionSnapshot {
        metadata: PlanRevision {
            id: id("revision-1"),
            repository_id: b.repository_id,
            graph_id: b.graph_id,
            generation_id: None,
            predecessor: None,
            plan_hash: generation.accepted_plan_hash.clone(),
        },
        plan_json: row.plan_json,
    };
    let authorization = RevisionAuthorization {
        id: id("authorization-1"),
        revision_id: revision.metadata.id.clone(),
        kind: AuthorizationKind::Human,
        decision: RevisionDecision::Accepted,
        actor: id("human-fixture"),
    };
    (
        store,
        lease,
        GenerationActivation {
            generation,
            revision,
            authorization,
            artifacts: Vec::new(),
            operations: Vec::new(),
            events: Vec::new(),
        },
    )
}
fn empty_expected() -> ExpectedState {
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
#[test]
fn b1_scoped_writes_reject_foreign_tasks_without_persisting_changes() {
    use meshloop_domain::state::{Event, TaskState};
    use meshloop_domain::task_graph::TaskId;
    use meshloop_engine::workspace::*;

    fn snapshot(store: &SqliteStore) -> Vec<(String, Vec<Vec<rusqlite::types::Value>>)> {
        let mut statement = store
            .conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap();
        statement
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|name| {
                let name = name.unwrap();
                let contents = rows(&store.conn, &name);
                (name, contents)
            })
            .collect()
    }

    let f = Fixture::new();
    let (mut store, lease, activation) = runtime_fixture(&f);
    store.activate_generation(&lease, &activation).unwrap();
    let scope = store
        .resolve_scope(&activation.generation.graph_id, &RunScope::ActiveGeneration)
        .unwrap()
        .unwrap();
    let artifact = artifact(
        "task-owned-attempt",
        &activation.generation,
        ArtifactKind::Attempt,
    );
    let mut reservation = AttemptReservation {
        scope: scope.clone(),
        revision_id: activation.revision.metadata.id.clone(),
        task_id: TaskId(67), // The accepted revision contains only tasks 1 through 66.
        harness: Some("fixture".into()),
        model_ref: None,
        spawn_intent: ProducerOperation {
            id: id("task-owned-spawn"),
            repository_id: scope.repository_id.clone(),
            graph_id: Some(scope.graph_id.clone()),
            generation_id: Some(scope.generation_id.clone()),
            target: ProducerTarget::Artifact(artifact.id.clone()),
            kind: ProducerKind::Spawn,
            before: empty_expected(),
            after: empty_expected(),
            max_steps: 1,
        },
        artifact,
    };
    let before = snapshot(&store);
    assert!(store.reserve_attempt(&lease, &reservation).is_err());
    assert_eq!(before, snapshot(&store));
    reservation.task_id = TaskId(1);
    let attempt = store.reserve_attempt(&lease, &reservation).unwrap();
    let mut event = ScopedEvent {
        scope,
        revision_id: reservation.revision_id.clone(),
        record: meshloop_engine::ports::TransitionRecord {
            graph_id: reservation.scope.graph_id.as_str().into(),
            task_id: TaskId(1),
            attempt_id: Some(attempt.attempt_id),
            from: TaskState::Ready,
            to: TaskState::Running,
            event: Event::AttemptStarted,
            reason: None,
            executor: "fixture".into(),
            occurred_at: "1".into(),
        },
    };
    for (task_id, attempt_id) in [
        (TaskId(2), Some(attempt.attempt_id)), // Existing task, foreign attempt.
        (TaskId(67), Some(attempt.attempt_id)), // Unknown task with an attempt.
        (TaskId(67), None),                    // Unknown task without an attempt.
    ] {
        event.record.task_id = task_id;
        event.record.attempt_id = attempt_id;
        let before = snapshot(&store);
        assert!(store.append_scoped_event(&lease, &event).is_err());
        assert_eq!(before, snapshot(&store));
    }
    event.record.task_id = TaskId(1);
    event.record.attempt_id = Some(attempt.attempt_id);
    store.append_scoped_event(&lease, &event).unwrap();
    event.record.task_id = TaskId(2);
    event.record.attempt_id = None;
    store.append_scoped_event(&lease, &event).unwrap();
    assert_eq!(
        store
            .scoped_events(&reservation.scope.graph_id, &RunScope::ActiveGeneration)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn b1_runtime_atomic_generations_revision_scopes_and_portable_attempts() {
    use meshloop_domain::task_graph::TaskId;
    use meshloop_engine::workspace::*;
    let f = Fixture::new();
    let (mut store, lease, activation) = runtime_fixture(&f);
    let graph = activation.generation.graph_id.clone();
    store
        .conn
        .execute("UPDATE runs SET plan_state='AwaitingPlanReview'", [])
        .unwrap();
    assert!(store.activate_generation(&lease, &activation).is_err());
    assert!(store.revision(&id("revision-1")).unwrap().is_none());
    assert!(
        store
            .generation(&activation.generation.id)
            .unwrap()
            .is_none()
    );
    store
        .conn
        .execute("UPDATE runs SET plan_state='PlanAccepted'", [])
        .unwrap();
    store.activate_generation(&lease, &activation).unwrap();
    let selected = store
        .resolve_scope(&graph, &RunScope::ActiveGeneration)
        .unwrap()
        .unwrap();
    let artifact = artifact(
        "attempt-owned",
        &activation.generation,
        ArtifactKind::Attempt,
    );
    let operation = ProducerOperation {
        id: id("spawn-operation"),
        repository_id: selected.repository_id.clone(),
        graph_id: Some(graph.clone()),
        generation_id: Some(selected.generation_id.clone()),
        target: ProducerTarget::Artifact(artifact.id.clone()),
        kind: ProducerKind::Spawn,
        before: empty_expected(),
        after: empty_expected(),
        max_steps: 1,
    };
    let reservation = AttemptReservation {
        scope: selected.clone(),
        revision_id: id("revision-1"),
        task_id: TaskId(1),
        harness: Some("fixture".into()),
        model_ref: Some("fixture-model".into()),
        artifact,
        spawn_intent: operation,
    };
    let attempt = store.reserve_attempt(&lease, &reservation).unwrap();
    assert!(
        store
            .load_attempt(attempt.attempt_id)
            .unwrap()
            .unwrap()
            .worktree_path
            .is_none()
    );
    assert_eq!(
        vec![StoredAttempt::Scoped(attempt.clone())],
        store
            .scoped_attempts(&graph, &RunScope::ActiveGeneration)
            .unwrap()
    );
    assert!(
        store
            .scoped_attempts(&graph, &RunScope::Legacy)
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .save_attempt(&store.load_attempt(attempt.attempt_id).unwrap().unwrap())
            .is_err()
    );
    let mut unsafe_attempt = attempt.clone();
    unsafe_attempt.outcome = Some("failed C:/private/external".into());
    assert!(
        store
            .update_scoped_attempt(&lease, &unsafe_attempt)
            .is_err()
    );
    let mut event = meshloop_engine::ports::TransitionRecord {
        graph_id: graph.as_str().into(),
        task_id: TaskId(1),
        attempt_id: Some(attempt.attempt_id),
        from: meshloop_domain::state::TaskState::Ready,
        to: meshloop_domain::state::TaskState::Running,
        event: meshloop_domain::state::Event::AttemptStarted,
        reason: None,
        executor: "fixture".into(),
        occurred_at: "1".into(),
    };
    store
        .append_scoped_event(
            &lease,
            &ScopedEvent {
                scope: selected.clone(),
                revision_id: id("revision-1"),
                record: event.clone(),
            },
        )
        .unwrap();
    event.reason = Some("failure C:/external/root".into());
    assert!(
        store
            .append_scoped_event(
                &lease,
                &ScopedEvent {
                    scope: selected.clone(),
                    revision_id: id("revision-1"),
                    record: event
                }
            )
            .is_err()
    );
    let mut second = GenerationRecord {
        id: id("generation-2"),
        ordinal: 2,
        ..activation.generation.clone()
    };
    let restart = GenerationActivation {
        generation: second.clone(),
        revision: store.revision(&id("revision-1")).unwrap().unwrap(),
        authorization: activation.authorization.clone(),
        artifacts: Vec::new(),
        operations: Vec::new(),
        events: Vec::new(),
    };
    store.activate_generation(&lease, &restart).unwrap();
    assert!(
        store
            .scoped_attempts(&graph, &RunScope::ActiveGeneration)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        1,
        store
            .scoped_attempts(&graph, &RunScope::Generation(selected.generation_id))
            .unwrap()
            .len()
    );
    assert_eq!(
        1,
        store
            .scoped_events(
                &graph,
                &RunScope::Generation(activation.generation.id.clone())
            )
            .unwrap()
            .len()
    );
    second.id = id("must-rollback");
    second.accepted_plan_hash = PlanHash::new("c".repeat(64)).unwrap();
    let bad = GenerationActivation {
        generation: second,
        revision: store.revision(&id("revision-1")).unwrap().unwrap(),
        authorization: activation.authorization,
        artifacts: Vec::new(),
        operations: Vec::new(),
        events: Vec::new(),
    };
    assert!(store.activate_generation(&lease, &bad).is_err());
    assert!(store.generation(&id("must-rollback")).unwrap().is_none());
}
#[test]
fn b1_root_bootstrap_and_control_client_are_narrow_and_bounded() {
    use meshloop_engine::workspace::*;
    let f = Fixture::new();
    let (mut store, mut lease, activation) = runtime_fixture(&f);
    store.activate_generation(&lease, &activation).unwrap();
    let target = f.base.join("explicit-new-root");
    let root = lease
        .bootstrap_root(RootAlias::new("bootstrap").unwrap(), &target)
        .unwrap();
    assert!(target.is_dir());
    assert!(lease.manifest.pending_roots.contains_key(&root.alias));
    let encoded = serde_json::to_string(&root).unwrap();
    assert!(!encoded.contains(&f.base.to_string_lossy().to_string()));
    assert!(ReadOnlyLifecycleAccess.fence().is_err());
    let scope = store
        .resolve_scope(&activation.generation.graph_id, &RunScope::ActiveGeneration)
        .unwrap()
        .unwrap();
    let process = ProcessIdentity {
        pid: 123,
        creation_token: id("exact-start-token"),
        tree_token: id("owned-tree"),
    };
    let driver = DriverRecord {
        id: id("driver-1"),
        nonce: id("nonce-1"),
        target: DriverTarget::Execution(scope.clone()),
        process: Some(process.clone()),
        phase: DriverPhase::Running,
        heartbeat: 1,
        error_code: None,
        diagnostic: None,
    };
    store.register_driver(&lease, &driver).unwrap();
    let mut client = CancellationClient::open(Path::new("git"), &f.repo, &f.ledger).unwrap();
    for task in 1..=64 {
        let request = CancellationRequest {
            id: id(&format!("request-{task}")),
            target: CancellationTarget::Execution {
                scope: scope.clone(),
                driver_id: driver.id.clone(),
                task_id: Some(meshloop_domain::task_graph::TaskId(task)),
            },
            process: process.clone(),
            created_at: 1,
        };
        assert_eq!(
            client.enqueue(&request).unwrap().status,
            CancellationStatus::Pending
        );
        assert_eq!(client.enqueue(&request).unwrap().request, request);
    }
    let overflow = CancellationRequest {
        id: id("overflow"),
        target: CancellationTarget::Execution {
            scope,
            driver_id: driver.id.clone(),
            task_id: Some(meshloop_domain::task_graph::TaskId(65)),
        },
        process,
        created_at: 2,
    };
    assert!(matches!(
        client.enqueue(&overflow),
        Err(StoreError::LifecycleBusy)
    ));
    assert_eq!(64, store.cancellation_requests(&driver.id).unwrap().len());
    store
        .acknowledge_cancellation(&lease, &id("request-1"), CancellationStatus::Blocked)
        .unwrap();
    assert_eq!(
        client
            .observation(&id("request-1"))
            .unwrap()
            .unwrap()
            .status,
        CancellationStatus::Blocked
    );
    assert!(client.enqueue(&overflow).is_ok());
    assert!(
        store
            .acknowledge_cancellation(&lease, &id("request-1"), CancellationStatus::Applied)
            .is_err(),
        "a no-op terminal ack cannot claim successful persistence"
    );
}

#[test]
fn provisional_control_is_identity_bound_idempotent_and_pending_is_not_success() {
    use meshloop_engine::workspace::*;
    let f = Fixture::new();
    let (mut store, lease, _activation) = runtime_fixture(&f);
    let owner = ProvisionalOwner {
        id: id("control-owner"),
        repository_id: lease.repository().id.clone(),
        graph_id: None,
        kind: ProvisionalKind::Planning,
        location: PortableLocation {
            root_alias: alias(),
            relative_path: RelativePath::new("control-planning").unwrap(),
        },
        provenance: id("control-origin"),
    };
    store.create_provisional(&lease, &owner).unwrap();
    let nonce = id("real-control-nonce");
    let Some(process) = lease.owner_process_identity(&nonce).unwrap() else {
        return;
    };
    assert_eq!(process.pid, std::process::id());
    assert_eq!(process.tree_token, nonce);
    assert_eq!(
        Some(process.clone()),
        lease.owner_process_identity(&nonce).unwrap()
    );
    let mut driver = DriverRecord {
        id: id("provisional-driver"),
        nonce,
        target: DriverTarget::Provisional {
            repository_id: owner.repository_id.clone(),
            owner_id: owner.id.clone(),
        },
        process: Some(process),
        phase: DriverPhase::Running,
        heartbeat: 1,
        error_code: None,
        diagnostic: None,
    };
    store.register_driver(&lease, &driver).unwrap();
    let mut client = CancellationClient::open(Path::new("git"), &f.repo, &f.ledger).unwrap();
    let observation = client.enqueue_driver(&driver.id).unwrap();
    assert_eq!(
        CancellationStatus::Pending,
        client
            .wait_for_acknowledgement(observation.clone(), Duration::ZERO)
            .unwrap()
            .status
    );
    let mut wrong = observation.request.clone();
    wrong.process.creation_token = id("different-creation");
    assert!(client.enqueue(&wrong).is_err());
    wrong = observation.request.clone();
    wrong.target = CancellationTarget::Provisional {
        repository_id: owner.repository_id.clone(),
        owner_id: id("wrong-owner"),
        driver_id: driver.id.clone(),
    };
    assert!(client.enqueue(&wrong).is_err());
    assert_eq!(1, store.cancellation_requests(&driver.id).unwrap().len());
    store
        .acknowledge_cancellation(&lease, &observation.request.id, CancellationStatus::Applied)
        .unwrap();
    driver.phase = DriverPhase::Exited;
    store.update_driver(&lease, &driver).unwrap();
    let repeat = client.enqueue_driver(&driver.id).unwrap();
    assert_eq!(repeat.request, observation.request);
    assert_eq!(repeat.status, CancellationStatus::Applied);
    assert!(store.cancellation_requests(&driver.id).unwrap().is_empty());
    driver.id = id("already-stale-driver");
    store.register_driver(&lease, &driver).unwrap();
    assert!(client.enqueue_driver(&driver.id).is_err());
}

#[test]
fn provisional_completion_drains_request_enqueued_during_final_collect_as_stale() {
    use meshloop_domain::capability::{Compatibility, HarnessError, HarnessProfile};
    use meshloop_engine::ports::{HarnessCapabilities, HarnessHandle, HarnessOutcome};
    use meshloop_engine::workspace::*;
    struct FinalCollectRequest {
        repo: PathBuf,
        ledger: PathBuf,
        request: std::sync::Mutex<Option<CancellationObservation>>,
    }
    impl HarnessCapabilities for FinalCollectRequest {
        fn probe(&self) -> Result<HarnessProfile, HarnessError> {
            Ok(HarnessProfile {
                harness: "fixture".into(),
                version: "test".into(),
                compatibility: Compatibility::Compatible,
                supports_noninteractive: true,
                supports_structured_output: true,
                supports_cancellation: true,
                reason: None,
            })
        }
        fn supports_nonblocking_collection(&self) -> bool {
            true
        }
        fn invoke(
            &self,
            _: &meshloop_engine::agent::AgentSpec,
        ) -> Result<HarnessHandle, HarnessError> {
            Ok(HarnessHandle {
                attempt_id: AttemptId(0),
                pid: None,
                pane_id: None,
            })
        }
        fn cancel(&self, _: &HarnessHandle) -> Result<(), HarnessError> {
            panic!("legacy cancellation forbidden")
        }
        fn collect(&self, _: &HarnessHandle) -> Result<HarnessOutcome, HarnessError> {
            panic!("blocking collection forbidden")
        }
        fn try_collect(&self, _: &HarnessHandle) -> Result<Option<HarnessOutcome>, HarnessError> {
            let store = SqliteStore::open_read_only(&self.ledger).unwrap().unwrap();
            let driver = store
                .observed_drivers(Path::new("git"), &self.repo)
                .unwrap()
                .pop()
                .unwrap();
            assert_eq!(driver.phase, DriverPhase::Running);
            let mut client =
                CancellationClient::open(Path::new("git"), &self.repo, &self.ledger).unwrap();
            *self.request.lock().unwrap() = Some(client.enqueue_driver(&driver.id).unwrap());
            Ok(Some(HarnessOutcome { exit_code: 0, output_redacted: r#"{"graph_id":"completion-race","nodes":[{"id":1,"description":"task","depends_on":[],"tier":null}]}"#.into(), worktree_changed: false }))
        }
    }
    let f = Fixture::new();
    let (mut store, mut lease) = bound(&f);
    if lease
        .owner_process_identity(&id("supported-check"))
        .unwrap()
        .is_none()
    {
        return;
    }
    let worktrees = f.base.join("planning");
    let alias = RootAlias::new("planning-worktrees").unwrap();
    lease.bootstrap_root(alias.clone(), &worktrees).unwrap();
    store.register_root(&lease, &alias).unwrap();
    let harness = FinalCollectRequest {
        repo: f.repo.clone(),
        ledger: f.ledger.clone(),
        request: std::sync::Mutex::new(None),
    };
    let git = crate::git::GitWorktreeAdapter::new(f.repo.clone());
    let process = crate::process::HostProcessView {};
    let checks = crate::check::CommandCheckRunner;
    let mut loop_ = meshloop_engine::run_loop::RunLoop {
        harnesses: std::collections::HashMap::from([(
            "fixture".into(),
            &harness as &dyn HarnessCapabilities,
        )]),
        candidates: vec![meshloop_engine::router::Candidate {
            harness: "fixture".into(),
            model_ref: "test".into(),
            model_tier: meshloop_domain::policy::ModelCapabilityTier::TopTier,
        }],
        workspace: &git,
        store: &mut store,
        processes: &process,
        checks: &checks,
        router: meshloop_engine::router::Router::default(),
        limits: meshloop_engine::run_loop::RunLimits {
            max_retries: 1,
            task_timeout: Duration::from_secs(10),
            max_concurrent_workers: 1,
        },
        fixture_only: true,
        verify_command: vec![],
        worktree_base: worktrees,
        active_graph: None,
        planner: meshloop_engine::run_loop::PlannerSelection::Harness("fixture".into()),
    };
    assert!(loop_.plan("fixture", "fixture", &mut lease).is_ok());
    let request = harness.request.lock().unwrap().clone().unwrap();
    let mut client = CancellationClient::open(Path::new("git"), &f.repo, &f.ledger).unwrap();
    assert_eq!(
        client
            .observation(&request.request.id)
            .unwrap()
            .unwrap()
            .status,
        CancellationStatus::Stale
    );
    assert_eq!(
        store.drivers(&lease.repository().id).unwrap()[0].phase,
        DriverPhase::Exited
    );
    assert_eq!(
        client.enqueue(&request.request).unwrap().status,
        CancellationStatus::Stale
    );
}

#[test]
fn b1_revision_authorization_is_append_only_and_operations_fail_closed() {
    use meshloop_engine::workspace::*;
    let f = Fixture::new();
    let (mut store, lease, activation) = runtime_fixture(&f);
    store.activate_generation(&lease, &activation).unwrap();
    let scope = store
        .resolve_scope(&activation.generation.graph_id, &RunScope::ActiveGeneration)
        .unwrap()
        .unwrap();
    let mut graph: serde_json::Value =
        serde_json::from_str(&activation.revision.plan_json).unwrap();
    graph["nodes"][0]["description"] = "changed".into();
    let plan_json = serde_json::to_string(&graph).unwrap();
    let revision = PlanRevisionSnapshot {
        metadata: PlanRevision {
            id: id("revision-2"),
            repository_id: scope.repository_id.clone(),
            graph_id: scope.graph_id.clone(),
            generation_id: Some(scope.generation_id.clone()),
            predecessor: Some(id("revision-1")),
            plan_hash: PlanHash::new(format!("{:x}", Sha256::digest(plan_json.as_bytes())))
                .unwrap(),
        },
        plan_json,
    };
    store.append_revision(&lease, &revision).unwrap();
    store
        .select_revision(&lease, &scope, &revision.metadata.id)
        .unwrap();
    assert_eq!(
        store
            .load_run(scope.graph_id.as_str())
            .unwrap()
            .unwrap()
            .plan_state,
        PlanState::AwaitingPlanReview
    );
    let authorization = RevisionAuthorization {
        id: id("authorization-2"),
        revision_id: revision.metadata.id.clone(),
        kind: AuthorizationKind::Human,
        decision: RevisionDecision::Accepted,
        actor: id("explicit-reviewer"),
    };
    store.authorize_revision(&lease, &authorization).unwrap();
    store
        .project_revision(&lease, &scope.graph_id, &revision.metadata.id)
        .unwrap();
    assert_eq!(
        store
            .load_run(scope.graph_id.as_str())
            .unwrap()
            .unwrap()
            .plan_state,
        PlanState::PlanAccepted
    );
    assert!(store.append_revision(&lease, &revision).is_err());
    assert!(store.authorize_revision(&lease, &authorization).is_err());
    assert_eq!(
        activation.revision.metadata,
        store.revision(&id("revision-1")).unwrap().unwrap().metadata
    );
    assert_eq!(
        activation.generation,
        store.generation(&scope.generation_id).unwrap().unwrap()
    );
    let p = ProvisionalOwner {
        id: id("provisional"),
        repository_id: scope.repository_id.clone(),
        graph_id: None,
        kind: ProvisionalKind::Planning,
        location: PortableLocation {
            root_alias: alias(),
            relative_path: RelativePath::new("provisional/opaque-owner").unwrap(),
        },
        provenance: id("planning-request"),
    };
    store.create_provisional(&lease, &p).unwrap();
    let operation = ProducerOperation {
        id: id("plan-intent"),
        repository_id: p.repository_id.clone(),
        graph_id: None,
        generation_id: None,
        target: ProducerTarget::Provisional(p.id.clone()),
        kind: ProducerKind::Plan,
        before: empty_expected(),
        after: empty_expected(),
        max_steps: 1,
    };
    store.append_operation(&lease, &operation).unwrap();
    let duplicate = ProducerOperation {
        id: id("duplicate-pending"),
        ..operation.clone()
    };
    assert!(store.append_operation(&lease, &duplicate).is_err());
    let wrong = OperationStep {
        id: id("bad-step"),
        operation_id: operation.id.clone(),
        step: 2,
        observed: empty_expected(),
        state: OperationState::Completed,
        error_code: None,
    };
    assert!(store.append_operation_step(&lease, &wrong).is_err());
    let step = OperationStep {
        id: id("completed-step"),
        step: 1,
        ..wrong
    };
    store.append_operation_step(&lease, &step).unwrap();
    assert_eq!(vec![step], store.operation_steps(&operation.id).unwrap());
    assert_eq!(vec![p], store.provisionals(&scope.repository_id).unwrap());
}

#[test]
fn provisional_root_bootstrap_records_missing_parent_suffix_and_resolves_native_path() {
    use meshloop_engine::workspace::LifecycleAccess;
    let f = Fixture::new();
    let mut lease = f.lease();
    let target = f.base.join("missing-parent").join("selected-root");
    let root = RootAlias::new("planner-fixture").unwrap();
    lease.bootstrap_root(root.clone(), &target).unwrap();
    assert_eq!(
        lease.manifest.pending_roots[&root].child.as_str(),
        "missing-parent/selected-root"
    );
    let location = PortableLocation {
        root_alias: root,
        relative_path: RelativePath::new("plan-opaque").unwrap(),
    };
    let resolved = lease.resolve(&location).unwrap();
    assert_eq!(
        std::fs::canonicalize(resolved.parent().unwrap()).unwrap(),
        std::fs::canonicalize(&target).unwrap()
    );
    #[cfg(windows)]
    assert!(!resolved.to_string_lossy().starts_with(r"\\?\"));
    git(
        &f.repo,
        &[
            "worktree",
            "add",
            "-b",
            "planner-native-path",
            resolved.to_str().unwrap(),
            "HEAD",
        ],
    );
    assert!(resolved.join(".git").is_file());
}
#[test]
fn provisional_root_partial_creation_is_retained_and_cannot_be_adopted() {
    use meshloop_engine::workspace::LifecycleAccess;
    let f = Fixture::new();
    let mut lease = f.lease();
    let alias = RootAlias::new("partial-planner").unwrap();
    let parent = std::fs::canonicalize(&f.base).unwrap();
    let target = parent.join("partial-parent").join("selected-root");
    lease.manifest.pending_roots.insert(
        alias.clone(),
        PendingRoot {
            canonical_parent: parent.clone(),
            parent_identity: native_identity(&parent).unwrap(),
            child: RelativePath::new("partial-parent/selected-root").unwrap(),
        },
    );
    write_manifest(&mut lease.file, &lease.manifest).unwrap();
    std::fs::create_dir(parent.join("partial-parent")).unwrap();
    std::fs::write(
        parent.join("partial-parent/evidence.txt"),
        "uncertain origin",
    )
    .unwrap();
    assert!(lease.bootstrap_root(alias.clone(), &target).is_err());
    std::fs::create_dir(&target).unwrap();
    assert!(lease.register_root(alias.clone(), &target).is_err());
    assert!(lease.bootstrap_root(alias.clone(), &target).is_err());
    assert!(!lease.manifest.roots.contains_key(&alias));
    assert_eq!(
        std::fs::read_to_string(parent.join("partial-parent/evidence.txt")).unwrap(),
        "uncertain origin"
    );
}
#[test]
fn provisional_root_rejects_reparse_ancestor_before_creating_children() {
    use meshloop_engine::workspace::LifecycleAccess;
    let f = Fixture::new();
    let mut lease = f.lease();
    let real = f.base.join("real");
    let link = f.base.join("linked");
    std::fs::create_dir(&real).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&real, &link).unwrap();
    #[cfg(windows)]
    assert!(
        Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&real)
            .guarded_output()
            .unwrap()
            .status
            .success()
    );
    let alias = RootAlias::new("linked-root").unwrap();
    assert!(
        lease
            .bootstrap_root(alias.clone(), &link.join("child"))
            .is_err()
    );
    assert!(lease.register_root(alias.clone(), &link).is_err());
    assert!(!real.join("child").exists());
    assert!(!lease.manifest.pending_roots.contains_key(&alias));
}
