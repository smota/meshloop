use meshloop_engine::ports::StoreError;
use rusqlite::{Connection, OptionalExtension};
pub(super) const BASE_SCHEMA: &str = r#"CREATE TABLE IF NOT EXISTS schema_meta (version INTEGER NOT NULL);
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
                 created_at TEXT NOT NULL,
                 review_note TEXT
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
                 outcome TEXT,
                 pane_id TEXT
             );
             CREATE TABLE IF NOT EXISTS quota_state (
                 harness TEXT PRIMARY KEY,
                 breaker TEXT NOT NULL,
                 opened_at TEXT,
                 cooldown_ms INTEGER NOT NULL
             );"#;

pub(super) const LIFECYCLE_SCHEMA: &str = "
CREATE TABLE workspace_repositories (repo_id TEXT PRIMARY KEY, common_native_id TEXT UNIQUE NOT NULL, ledger_native_id TEXT NOT NULL, payload_json TEXT NOT NULL);
CREATE TABLE workspace_roots (repo_id TEXT NOT NULL REFERENCES workspace_repositories(repo_id), alias TEXT NOT NULL, payload_json TEXT NOT NULL, PRIMARY KEY(repo_id,alias));
CREATE TABLE workspace_generations (id TEXT PRIMARY KEY, repo_id TEXT NOT NULL REFERENCES workspace_repositories(repo_id), graph_id TEXT NOT NULL REFERENCES runs(graph_id), ordinal INTEGER NOT NULL CHECK(ordinal>0), root_alias TEXT NOT NULL, plan_hash TEXT NOT NULL, payload_json TEXT NOT NULL, UNIQUE(repo_id,graph_id,ordinal), UNIQUE(id,repo_id), FOREIGN KEY(repo_id,root_alias) REFERENCES workspace_roots(repo_id,alias));
CREATE TABLE workspace_artifacts (id TEXT PRIMARY KEY, generation_id TEXT NOT NULL, repo_id TEXT NOT NULL, root_alias TEXT NOT NULL, attempt_id INTEGER REFERENCES attempts(attempt_id), payload_json TEXT NOT NULL, FOREIGN KEY(generation_id,repo_id) REFERENCES workspace_generations(id,repo_id), FOREIGN KEY(repo_id,root_alias) REFERENCES workspace_roots(repo_id,alias));
CREATE TABLE workspace_dependencies (artifact_id TEXT NOT NULL REFERENCES workspace_artifacts(id), requires_id TEXT NOT NULL REFERENCES workspace_artifacts(id), CHECK(artifact_id<>requires_id), PRIMARY KEY(artifact_id,requires_id));
CREATE TABLE workspace_facts (id TEXT PRIMARY KEY, artifact_id TEXT NOT NULL REFERENCES workspace_artifacts(id), payload_json TEXT NOT NULL);
CREATE TABLE workspace_journal (id TEXT PRIMARY KEY, artifact_id TEXT NOT NULL REFERENCES workspace_artifacts(id), payload_json TEXT NOT NULL);
CREATE TABLE workspace_legacy_unbound (source_table TEXT NOT NULL CHECK(source_table IN ('runs','attempts')), source_id TEXT NOT NULL, binding TEXT NOT NULL CHECK(binding='Unresolved'), PRIMARY KEY(source_table,source_id));
INSERT INTO workspace_legacy_unbound SELECT 'runs',graph_id,'Unresolved' FROM runs;
INSERT INTO workspace_legacy_unbound SELECT 'attempts',CAST(attempt_id AS TEXT),'Unresolved' FROM attempts;
";
fn err(_: impl std::fmt::Display) -> StoreError {
    StoreError::Corrupt("workspace schema is incompatible or migration failed".into())
}
fn columns(conn: &Connection, table: &str, required: &[&str]) -> Result<(), StoreError> {
    let mut statement = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(err)?;
    let found = statement
        .query_map([], |r| r.get::<_, String>(1))
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    if required
        .iter()
        .any(|column| !found.iter().any(|actual| actual == column))
    {
        return Err(err("missing column"));
    }
    Ok(())
}
fn validate_base(conn: &Connection, version: i64) -> Result<(), StoreError> {
    columns(
        conn,
        "evidence",
        &["task_id", "attempt_id", "revision", "kind", "payload_json"],
    )?;
    columns(
        conn,
        "routing_feedback",
        &[
            "harness",
            "model_ref",
            "tier",
            "success_count",
            "failure_count",
        ],
    )?;
    if version == 1 {
        return Ok(());
    }
    columns(
        conn,
        "runs",
        &[
            "graph_id",
            "plan_state",
            "run_base",
            "integrate_ref",
            "plan_json",
            "plan_sha256",
            "created_at",
        ],
    )?;
    columns(
        conn,
        "events",
        &[
            "event_id",
            "graph_id",
            "task_id",
            "attempt_id",
            "from_state",
            "to_state",
            "event_type",
            "reason",
            "executor",
            "evidence_ref",
            "occurred_at",
        ],
    )?;
    columns(
        conn,
        "attempts",
        &[
            "attempt_id",
            "graph_id",
            "task_id",
            "harness",
            "model_ref",
            "worktree_path",
            "pid",
            "image_name",
            "started_at",
            "ended_at",
            "outcome",
        ],
    )?;
    columns(
        conn,
        "quota_state",
        &["harness", "breaker", "opened_at", "cooldown_ms"],
    )?;
    if version >= 4 {
        columns(conn, "runs", &["review_note"])?;
        columns(conn, "attempts", &["pane_id"])?;
    }
    Ok(())
}
pub(crate) fn migrate(conn: &Connection) -> Result<(), StoreError> {
    migrate_with_fault(conn, false)
}
fn migrate_to5(conn: &Connection, fail_after_ddl: bool) -> Result<(), StoreError> {
    let has_meta: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='schema_meta')",
            [],
            |r| r.get(0),
        )
        .map_err(err)?;
    let version = if has_meta {
        let mut stmt = conn
            .prepare("SELECT version FROM schema_meta")
            .map_err(err)?;
        let versions = stmt
            .query_map([], |r| r.get::<_, i64>(0))
            .map_err(err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err)?;
        if versions.len() != 1 {
            return Err(err("ambiguous version"));
        }
        let version = versions[0];
        // Historical commits c610ef3/v1, 64430f2/v2, 173e26a/v4. No evidenced v3 contract.
        if !matches!(version, 1 | 2 | 4 | 5) {
            return Err(err("unsupported version"));
        }
        validate_base(conn, version)?;
        Some(version)
    } else {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
                [],
                |r| r.get(0),
            )
            .map_err(err)?;
        if count != 0 {
            return Err(err("unversioned nonempty database"));
        }
        None
    };
    if version == Some(5) {
        validate_lifecycle_schema(conn)?;
        return Ok(());
    }
    let tx = conn;
    tx.execute_batch(BASE_SCHEMA).map_err(err)?;
    if version == Some(2) {
        tx.execute_batch("ALTER TABLE attempts ADD COLUMN pane_id TEXT; ALTER TABLE runs ADD COLUMN review_note TEXT;").map_err(err)?;
    }
    // Failure is deliberately after actual v5 DDL, before any commit.
    if fail_after_ddl {
        tx.execute_batch("CREATE TABLE workspace_repositories (repo_id TEXT PRIMARY KEY);")
            .map_err(err)?;
        return Err(err("injected mid-DDL failure"));
    }
    tx.execute_batch(LIFECYCLE_SCHEMA).map_err(err)?;
    if version.is_none() {
        tx.execute("INSERT INTO schema_meta VALUES(5)", [])
            .map_err(err)?;
    } else {
        tx.execute("UPDATE schema_meta SET version=5", [])
            .map_err(err)?;
    }
    let violation: Option<String> = tx
        .query_row(
            "SELECT \"table\" FROM pragma_foreign_key_check LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(err)?;
    if violation.is_some() {
        return Err(err("foreign key violation"));
    }
    Ok(())
}

/// This binary owns canonical v5 DDL. Arbitrary third-party equivalent schemas
/// are not supported: accepting a weakened CHECK, key or FK would lose lineage.
fn normalize_ddl(sql: &str) -> String {
    let mut normalized = String::new();
    let mut quote = None;
    let mut whitespace = false;
    for ch in sql.chars() {
        if let Some(delimiter) = quote {
            normalized.push(ch);
            if ch == delimiter {
                quote = None;
            }
        } else if ch.is_whitespace() {
            whitespace = true;
        } else {
            if whitespace && !normalized.is_empty() {
                normalized.push(' ');
            }
            whitespace = false;
            normalized.push(ch);
            if matches!(ch, '\'' | '"') {
                quote = Some(ch);
            }
        }
    }
    normalized.trim_end_matches(';').to_owned()
}
fn owned_objects(
    conn: &Connection,
    table: &str,
) -> Result<Vec<(String, String, Option<String>)>, StoreError> {
    let mut statement = conn
        .prepare("SELECT type,name,sql FROM sqlite_master WHERE tbl_name=?1 ORDER BY type,name")
        .map_err(err)?;
    statement
        .query_map([table], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get::<_, Option<String>>(2)?
                    .map(|sql| normalize_ddl(&sql)),
            ))
        })
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)
}
fn validate_lifecycle_schema(conn: &Connection) -> Result<(), StoreError> {
    // Build the known schema in an ephemeral database, never modifying the candidate.
    let canonical = Connection::open_in_memory().map_err(err)?;
    canonical.execute_batch(BASE_SCHEMA).map_err(err)?;
    canonical.execute_batch(LIFECYCLE_SCHEMA).map_err(err)?;
    for table in [
        "workspace_repositories",
        "workspace_roots",
        "workspace_generations",
        "workspace_artifacts",
        "workspace_dependencies",
        "workspace_facts",
        "workspace_journal",
        "workspace_legacy_unbound",
    ] {
        // Includes canonical table DDL and all constraint-created indexes; unexpected
        // triggers or altered index objects on owned tables also fail closed.
        if owned_objects(conn, table)? != owned_objects(&canonical, table)? {
            return Err(err("noncanonical lifecycle constraints"));
        }
    }
    // Lifecycle foreign keys also rely on these existing legacy parent keys.
    for (table, column) in [("runs", "graph_id"), ("attempts", "attempt_id")] {
        let key: Option<i64> = conn
            .query_row(
                "SELECT pk FROM pragma_table_info(?1) WHERE name=?2",
                [table, column],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        if key != Some(1) {
            return Err(err("missing legacy parent key"));
        }
    }
    let violation: Option<String> = conn
        .query_row(
            "SELECT \"table\" FROM pragma_foreign_key_check LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(err)?;
    if violation.is_some() {
        return Err(err("existing lifecycle foreign key violation"));
    }
    Ok(())
}

pub(super) const RUNTIME_SCHEMA:&str="
CREATE TABLE workspace_attempt_sequence(singleton INTEGER PRIMARY KEY CHECK(singleton=1), next_id INTEGER NOT NULL CHECK(next_id>0));
INSERT INTO workspace_attempt_sequence SELECT 1,COALESCE(MAX(attempt_id),0)+1 FROM attempts;
CREATE TABLE workspace_run_bindings(graph_id TEXT PRIMARY KEY REFERENCES runs(graph_id), repo_id TEXT NOT NULL REFERENCES workspace_repositories(repo_id), root_alias TEXT NOT NULL, active_generation TEXT REFERENCES workspace_generations(id), current_revision TEXT REFERENCES workspace_plan_revisions(id), payload_json TEXT NOT NULL, FOREIGN KEY(repo_id,root_alias) REFERENCES workspace_roots(repo_id,alias));
CREATE TABLE workspace_provisional_owners(id TEXT PRIMARY KEY, repo_id TEXT NOT NULL REFERENCES workspace_repositories(repo_id), graph_id TEXT, root_alias TEXT NOT NULL, payload_json TEXT NOT NULL, FOREIGN KEY(repo_id,root_alias) REFERENCES workspace_roots(repo_id,alias));
CREATE TABLE workspace_plan_revisions(id TEXT PRIMARY KEY, repo_id TEXT NOT NULL REFERENCES workspace_repositories(repo_id), graph_id TEXT NOT NULL REFERENCES workspace_run_bindings(graph_id), generation_id TEXT REFERENCES workspace_generations(id) DEFERRABLE INITIALLY DEFERRED, predecessor TEXT REFERENCES workspace_plan_revisions(id), plan_hash TEXT NOT NULL, plan_json TEXT NOT NULL, payload_json TEXT NOT NULL);
CREATE TABLE workspace_revision_authorizations(id TEXT PRIMARY KEY, revision_id TEXT NOT NULL REFERENCES workspace_plan_revisions(id), payload_json TEXT NOT NULL);
CREATE TABLE workspace_generation_revisions(generation_id TEXT PRIMARY KEY REFERENCES workspace_generations(id), revision_id TEXT NOT NULL REFERENCES workspace_plan_revisions(id));
CREATE TABLE workspace_provisional_promotions(owner_id TEXT NOT NULL REFERENCES workspace_provisional_owners(id), generation_id TEXT NOT NULL REFERENCES workspace_generations(id), artifact_id TEXT NOT NULL REFERENCES workspace_artifacts(id), PRIMARY KEY(owner_id,artifact_id));
CREATE TABLE workspace_operations(id TEXT PRIMARY KEY, repo_id TEXT NOT NULL REFERENCES workspace_repositories(repo_id), graph_id TEXT, generation_id TEXT REFERENCES workspace_generations(id), claim_key TEXT NOT NULL, pending INTEGER NOT NULL CHECK(pending IN (0,1)), payload_json TEXT NOT NULL);
CREATE UNIQUE INDEX workspace_pending_operation ON workspace_operations(repo_id,claim_key) WHERE pending=1;
CREATE TABLE workspace_operation_steps(id TEXT PRIMARY KEY, operation_id TEXT NOT NULL REFERENCES workspace_operations(id), step INTEGER NOT NULL CHECK(step>0), payload_json TEXT NOT NULL, UNIQUE(operation_id,step));
CREATE TABLE workspace_drivers(id TEXT PRIMARY KEY, repo_id TEXT NOT NULL REFERENCES workspace_repositories(repo_id), payload_json TEXT NOT NULL);
CREATE TABLE workspace_control_requests(id TEXT PRIMARY KEY, driver_id TEXT NOT NULL REFERENCES workspace_drivers(id), selector TEXT NOT NULL, status TEXT NOT NULL CHECK(status IN ('Pending','Applied','Blocked','Stale','OwnerLost')), payload_json TEXT NOT NULL, UNIQUE(driver_id,selector));
CREATE TABLE workspace_artifact_bindings(artifact_id TEXT PRIMARY KEY REFERENCES workspace_artifacts(id), revision_id TEXT NOT NULL REFERENCES workspace_plan_revisions(id), payload_json TEXT NOT NULL);
CREATE TABLE workspace_attempt_metadata(attempt_id INTEGER PRIMARY KEY REFERENCES attempts(attempt_id), artifact_id TEXT NOT NULL UNIQUE REFERENCES workspace_artifacts(id), root_alias TEXT NOT NULL, relative_path TEXT NOT NULL, process_json TEXT);
ALTER TABLE events ADD COLUMN generation_id TEXT REFERENCES workspace_generations(id);
ALTER TABLE events ADD COLUMN revision_id TEXT REFERENCES workspace_plan_revisions(id);
ALTER TABLE attempts ADD COLUMN generation_id TEXT REFERENCES workspace_generations(id);
ALTER TABLE attempts ADD COLUMN revision_id TEXT REFERENCES workspace_plan_revisions(id);
CREATE INDEX workspace_events_scope ON events(graph_id,generation_id,event_id);
CREATE INDEX workspace_attempts_scope ON attempts(graph_id,generation_id,attempt_id);
CREATE TRIGGER workspace_bound_event_insert BEFORE INSERT ON events WHEN EXISTS(SELECT 1 FROM workspace_run_bindings WHERE graph_id=NEW.graph_id) AND (NEW.generation_id IS NULL OR NEW.revision_id IS NULL) BEGIN SELECT RAISE(ABORT,'bound event requires scope'); END;
CREATE TRIGGER workspace_bound_attempt_insert BEFORE INSERT ON attempts WHEN EXISTS(SELECT 1 FROM workspace_run_bindings WHERE graph_id=NEW.graph_id) AND (NEW.generation_id IS NULL OR NEW.revision_id IS NULL OR NEW.worktree_path IS NOT NULL) BEGIN SELECT RAISE(ABORT,'bound attempt requires portable scope'); END;
";
pub(super) fn schema_version(conn: &Connection) -> Result<Option<i64>, StoreError> {
    let exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='schema_meta' AND type='table')",
            [],
            |r| r.get(0),
        )
        .map_err(err)?;
    if !exists {
        return Ok(None);
    }
    let mut stmt = conn
        .prepare("SELECT version FROM schema_meta")
        .map_err(err)?;
    let rows = stmt
        .query_map([], |r| r.get::<_, i64>(0))
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    if rows.len() != 1 {
        return Err(err("ambiguous version"));
    }
    Ok(Some(rows[0]))
}
pub(super) fn validate6(conn: &Connection) -> Result<(), StoreError> {
    validate_lifecycle_schema(conn)?;
    let canonical = Connection::open_in_memory().map_err(err)?;
    canonical.execute_batch(BASE_SCHEMA).map_err(err)?;
    canonical.execute_batch(LIFECYCLE_SCHEMA).map_err(err)?;
    canonical.execute_batch(RUNTIME_SCHEMA).map_err(err)?;
    for table in [
        "workspace_attempt_sequence",
        "workspace_run_bindings",
        "workspace_provisional_owners",
        "workspace_plan_revisions",
        "workspace_revision_authorizations",
        "workspace_generation_revisions",
        "workspace_provisional_promotions",
        "workspace_operations",
        "workspace_operation_steps",
        "workspace_drivers",
        "workspace_control_requests",
        "workspace_attempt_metadata",
        "workspace_artifact_bindings",
    ] {
        if owned_objects(conn, table)? != owned_objects(&canonical, table)? {
            return Err(err("noncanonical v6 schema"));
        }
    }
    for table in ["events", "attempts"] {
        for column in ["generation_id", "revision_id"] {
            columns(conn, table, &[column])?;
        }
        let expected = owned_objects(&canonical, table)?;
        let actual = owned_objects(conn, table)?;
        // Legacy CREATE SQL may differ after historically supported ALTERs; require
        // the additive scope indexes/triggers and actual FK endpoints explicitly.
        for object in expected.into_iter().filter(|o| o.0 != "table") {
            if !actual.contains(&object) {
                return Err(err("missing scope constraints"));
            }
        }
        let mut statement = conn
            .prepare(&format!("PRAGMA foreign_key_list({table})"))
            .map_err(err)?;
        let keys = statement
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })
            .map_err(err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err)?;
        for key in [
            (
                "workspace_generations".to_owned(),
                "generation_id".to_owned(),
                "id".to_owned(),
            ),
            (
                "workspace_plan_revisions".to_owned(),
                "revision_id".to_owned(),
                "id".to_owned(),
            ),
        ] {
            if !keys.contains(&key) {
                return Err(err("missing scope foreign key"));
            }
        }
    }
    Ok(())
}
pub(super) fn migrate_with_fault(
    conn: &Connection,
    fail_after_ddl: bool,
) -> Result<(), StoreError> {
    let version = schema_version(conn)?;
    if version == Some(6) {
        return validate6(conn);
    }
    if matches!(version,Some(v) if !matches!(v,1|2|4|5)) {
        return Err(err("unsupported schema"));
    }
    if version == Some(5) {
        validate_lifecycle_schema(conn)?;
    }
    let tx = rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    migrate_to5(&tx, false)?;
    if fail_after_ddl {
        tx.execute_batch("CREATE TABLE workspace_attempt_sequence(singleton INTEGER PRIMARY KEY CHECK(singleton=1), next_id INTEGER NOT NULL CHECK(next_id>0));
INSERT INTO workspace_attempt_sequence SELECT 1,COALESCE(MAX(attempt_id),0)+1 FROM attempts;
CREATE TABLE workspace_run_bindings(graph_id TEXT PRIMARY KEY);")
            .map_err(err)?;
        return Err(err("injected v6 DDL failure"));
    }
    tx.execute_batch(RUNTIME_SCHEMA).map_err(err)?;
    tx.execute("UPDATE schema_meta SET version=6", [])
        .map_err(err)?;
    validate6(&tx)?;
    tx.commit().map_err(err)
}
