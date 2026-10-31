//! S3 guarded transactional persistence; no producer or CLI activation.
use super::{native_identity, refused, verify_handle};
use crate::store::SqliteStore;
use meshloop_domain::evidence::AttemptId;
use meshloop_domain::workspace::*;
use meshloop_engine::ports::{EventLog, RunStore, StoreError};
use meshloop_engine::workspace::*;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
fn error(_: impl std::fmt::Display) -> StoreError {
    StoreError::Corrupt("lifecycle persistence constraint or operation failed".into())
}
fn json<T: Serialize>(value: &T) -> Result<String, StoreError> {
    serde_json::to_string(value).map_err(error)
}
fn parse<T: DeserializeOwned>(value: String) -> Result<T, StoreError> {
    serde_json::from_str(&value).map_err(error)
}
fn get<T: DeserializeOwned>(
    c: &Connection,
    table: &str,
    key: &str,
    id: &str,
) -> Result<Option<T>, StoreError> {
    c.query_row(
        &format!("SELECT payload_json FROM {table} WHERE {key}=?1"),
        [id],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .map_err(error)?
    .map(parse)
    .transpose()
}
fn list<T: DeserializeOwned>(c: &Connection, sql: &str, id: &str) -> Result<Vec<T>, StoreError> {
    let mut s = c.prepare(sql).map_err(error)?;
    s.query_map([id], |r| r.get::<_, String>(0))
        .map_err(error)?
        .map(|r| parse(r.map_err(error)?))
        .collect()
}
fn binding(c: &Connection, graph: &GraphId) -> Result<RunBinding, StoreError> {
    get(c, "workspace_run_bindings", "graph_id", graph.as_str())?
        .ok_or(StoreError::LegacyOwnershipUnresolved)
}
fn scope(
    c: &Connection,
    f: &dyn RepositoryFence,
    s: &ExecutionScope,
    revision: Option<&LifecycleId>,
) -> Result<(), StoreError> {
    let b = binding(c, &s.graph_id)?;
    if s.repository_id != f.repository().id
        || b.repository_id != s.repository_id
        || b.active_generation.as_ref() != Some(&s.generation_id)
    {
        return Err(refused());
    }
    if let Some(revision) = revision {
        if b.current_revision.as_ref() != Some(revision) {
            return Err(StoreError::PlanRevisionMismatch);
        }
        let snapshot = revision_snapshot(c, revision)?.ok_or(StoreError::PlanRevisionMismatch)?;
        let current: (String, String, String) = c
            .query_row(
                "SELECT plan_state,plan_sha256,plan_json FROM runs WHERE graph_id=?1",
                [s.graph_id.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(error)?;
        if current.0 != "PlanAccepted"
            || current.1 != snapshot.metadata.plan_hash.as_str()
            || current.2 != snapshot.plan_json
            || !authorized(c, revision)?
        {
            return Err(StoreError::PlanRevisionMismatch);
        }
    }
    Ok(())
}
fn revision_snapshot(
    c: &Connection,
    id: &LifecycleId,
) -> Result<Option<PlanRevisionSnapshot>, StoreError> {
    let row: Option<(String, String)> = c
        .query_row(
            "SELECT payload_json,plan_json FROM workspace_plan_revisions WHERE id=?1",
            [id.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(error)?;
    row.map(|(metadata, plan_json)| {
        Ok(PlanRevisionSnapshot {
            metadata: parse(metadata)?,
            plan_json,
        })
    })
    .transpose()
}
fn insert_revision(
    c: &Connection,
    f: &dyn RepositoryFence,
    r: &PlanRevisionSnapshot,
) -> Result<(), StoreError> {
    let m = &r.metadata;
    let b = binding(c, &m.graph_id)?;
    if b.repository_id != f.repository().id
        || m.repository_id != b.repository_id
        || format!("{:x}", Sha256::digest(r.plan_json.as_bytes())) != m.plan_hash.as_str()
    {
        return Err(StoreError::PlanRevisionMismatch);
    }
    let graph: meshloop_domain::task_graph::TaskGraph =
        serde_json::from_str(&r.plan_json).map_err(|_| StoreError::PlanRevisionMismatch)?;
    if graph.graph_id != m.graph_id.as_str() || graph.validate().is_err() {
        return Err(StoreError::PlanRevisionMismatch);
    }
    if let Some(old) = &m.predecessor {
        let previous = revision_snapshot(c, old)?.ok_or(StoreError::PlanRevisionMismatch)?;
        if previous.metadata.graph_id != m.graph_id
            || previous.metadata.repository_id != m.repository_id
        {
            return Err(StoreError::PlanRevisionMismatch);
        }
    }
    c.execute(
        "INSERT INTO workspace_plan_revisions VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            m.id.as_str(),
            m.repository_id.as_str(),
            m.graph_id.as_str(),
            m.generation_id.as_ref().map(|x| x.as_str()),
            m.predecessor.as_ref().map(|x| x.as_str()),
            m.plan_hash.as_str(),
            r.plan_json,
            json(m)?
        ],
    )
    .map_err(error)?;
    Ok(())
}
fn authorized(c: &Connection, id: &LifecycleId) -> Result<bool, StoreError> {
    let mut records: Vec<RevisionAuthorization> = list(
        c,
        "SELECT payload_json FROM workspace_revision_authorizations WHERE revision_id=?1 ORDER BY rowid",
        id.as_str(),
    )?;
    Ok(records
        .pop()
        .is_some_and(|r| r.decision == RevisionDecision::Accepted))
}
fn insert_authorization(
    c: &Connection,
    f: &dyn RepositoryFence,
    a: &RevisionAuthorization,
) -> Result<(), StoreError> {
    let revision = revision_snapshot(c, &a.revision_id)?.ok_or(StoreError::PlanRevisionMismatch)?;
    if revision.metadata.repository_id != f.repository().id {
        return Err(refused());
    }
    if a.kind == AuthorizationKind::InheritedAdr0028 {
        let previous = revision
            .metadata
            .predecessor
            .as_ref()
            .ok_or(StoreError::PlanRevisionMismatch)?;
        if a.decision != RevisionDecision::Accepted || !authorized(c, previous)? {
            return Err(StoreError::PlanRevisionMismatch);
        }
    }
    c.execute(
        "INSERT INTO workspace_revision_authorizations VALUES(?1,?2,?3)",
        params![a.id.as_str(), a.revision_id.as_str(), json(a)?],
    )
    .map_err(error)?;
    Ok(())
}
fn operation(
    c: &Connection,
    f: &dyn RepositoryFence,
    o: &ProducerOperation,
) -> Result<(), StoreError> {
    if o.repository_id != f.repository().id || o.max_steps == 0 || o.max_steps > 256 {
        return Err(refused());
    }
    if let Some(generation_record) = &o.generation_id {
        let g: GenerationRecord =
            get(c, "workspace_generations", "id", generation_record.as_str())?
                .ok_or_else(refused)?;
        if g.repository_id != o.repository_id || Some(&g.graph_id) != o.graph_id.as_ref() {
            return Err(refused());
        }
    }
    match &o.target {
        ProducerTarget::RootBootstrap(alias) => {
            if o.kind != ProducerKind::BootstrapRoot
                || o.generation_id.is_some()
                || o.graph_id.is_some()
                || o.after.location.as_ref().map(|p| &p.root_alias) != Some(alias)
            {
                return Err(refused());
            }
        }
        ProducerTarget::Provisional(id) => {
            let p: ProvisionalOwner =
                get(c, "workspace_provisional_owners", "id", id.as_str())?.ok_or_else(refused)?;
            if p.repository_id != o.repository_id
                || p.graph_id != o.graph_id
                || o.generation_id.is_some()
            {
                return Err(refused());
            }
        }
        ProducerTarget::Artifact(id) => {
            let a: ArtifactRecord =
                get(c, "workspace_artifacts", "id", id.as_str())?.ok_or_else(refused)?;
            if Some(&a.generation_id) != o.generation_id.as_ref() {
                return Err(refused());
            }
        }
        ProducerTarget::Driver(id) => {
            let d: DriverRecord =
                get(c, "workspace_drivers", "id", id.as_str())?.ok_or_else(refused)?;
            if driver_repo(&d.target) != &o.repository_id {
                return Err(refused());
            }
        }
    }
    c.execute(
        "INSERT INTO workspace_operations VALUES(?1,?2,?3,?4,?5,1,?6)",
        params![
            o.id.as_str(),
            o.repository_id.as_str(),
            o.graph_id.as_ref().map(|x| x.as_str()),
            o.generation_id.as_ref().map(|x| x.as_str()),
            json(&o.target)?,
            json(o)?
        ],
    )
    .map_err(error)?;
    Ok(())
}
fn event(c: &Connection, f: &dyn RepositoryFence, e: &ScopedEvent) -> Result<(), StoreError> {
    scope(c, f, &e.scope, Some(&e.revision_id))?;
    if e.record.reason.as_ref().is_some_and(|r| !public_text(r)) || !public_text(&e.record.executor)
    {
        return Err(refused());
    }
    if e.record.graph_id != e.scope.graph_id.as_str() {
        return Err(refused());
    }
    if let Some(id) = e.record.attempt_id {
        let valid:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM attempts WHERE attempt_id=?1 AND generation_id=?2 AND revision_id=?3)",params![id.0,e.scope.generation_id.as_str(),e.revision_id.as_str()],|r|r.get(0)).map_err(error)?;
        if !valid {
            return Err(refused());
        }
    }
    c.execute("INSERT INTO events(graph_id,task_id,attempt_id,from_state,to_state,event_type,reason,executor,occurred_at,generation_id,revision_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![e.record.graph_id,e.record.task_id.0,e.record.attempt_id.map(|a|a.0),format!("{:?}",e.record.from),format!("{:?}",e.record.to),format!("{:?}",e.record.event),e.record.reason,e.record.executor,e.record.occurred_at,e.scope.generation_id.as_str(),e.revision_id.as_str()]).map_err(error)?;
    Ok(())
}
fn driver_repo(t: &DriverTarget) -> &LifecycleId {
    match t {
        DriverTarget::Execution(s) => &s.repository_id,
        DriverTarget::Provisional { repository_id, .. } => repository_id,
    }
}
fn validate_driver(
    c: &Connection,
    f: &dyn RepositoryFence,
    d: &DriverRecord,
) -> Result<(), StoreError> {
    if driver_repo(&d.target) != &f.repository().id {
        return Err(refused());
    }
    match &d.target {
        DriverTarget::Execution(s) => scope(c, f, s, None)?,
        DriverTarget::Provisional { owner_id, .. } => {
            let p: ProvisionalOwner =
                get(c, "workspace_provisional_owners", "id", owner_id.as_str())?
                    .ok_or_else(refused)?;
            if p.repository_id != f.repository().id {
                return Err(refused());
            }
        }
    }
    if matches!(d.phase, DriverPhase::Running | DriverPhase::Starting)
        && d.process.as_ref().is_none_or(|p| p.pid == 0)
    {
        return Err(refused());
    }
    Ok(())
}
impl SqliteStore {
    /// Observes an existing v6 ledger without application writes or migration.
    /// SQLite may create, update, or remove its WAL/SHM coordination sidecars
    /// (ADR 0034); an absent ledger creates no files or directories.
    pub fn open_read_only(path: &std::path::Path) -> Result<Option<Self>, StoreError> {
        match std::fs::metadata(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(refused()),
            Ok(_) => {}
        }
        let file = std::fs::File::open(path).map_err(error)?;
        let handle = same_file::Handle::from_file(file).map_err(error)?;
        verify_handle(&handle, path)?;
        let identity = native_identity(path)?;
        let conn = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(error)?;
        conn.pragma_update(None, "query_only", true)
            .map_err(error)?;

        match super::schema::schema_version(&conn)? {
            Some(6) => {}
            Some(1 | 2 | 4 | 5) => return Err(StoreError::SchemaUpgradeRequired),
            _ => return Err(refused()),
        }
        super::schema::validate6(&conn)?;
        verify_handle(&handle, path)?;
        Ok(Some(Self {
            conn,
            read_only: true,
            database_handle: Some(handle),
            database_identity: Some(identity),
        }))
    }
    pub(crate) fn reject_bound_graph(&self, graph: &str) -> Result<(), StoreError> {
        if self.read_only {
            return Err(StoreError::ReadOnly);
        }
        let exists: bool = self
            .conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM workspace_run_bindings WHERE graph_id=?1)",
                [graph],
                |r| r.get(0),
            )
            .map_err(error)?;
        if exists {
            return Err(StoreError::LegacyOwnershipUnresolved);
        }
        Ok(())
    }
}
impl RuntimeLedger for SqliteStore {
    fn with_read_snapshot(&self, observe: &mut dyn FnMut()) -> Result<(), StoreError> {
        if !self.conn.is_autocommit() {
            observe();
            return Ok(());
        }
        let tx = self.conn.unchecked_transaction().map_err(error)?;
        observe();
        // Observation never commits. Explicit rollback reports snapshot-close
        // failures instead of relying on Transaction's best-effort Drop.
        tx.rollback().map_err(error)
    }
    fn generation_initial_revision(
        &self,
        s: &ExecutionScope,
    ) -> Result<Option<LifecycleId>, StoreError> {
        if self
            .resolve_scope(&s.graph_id, &RunScope::Generation(s.generation_id.clone()))?
            .as_ref()
            != Some(s)
        {
            return Err(refused());
        }
        self.conn
            .query_row(
                "SELECT revision_id FROM workspace_generation_revisions WHERE generation_id=?1",
                [s.generation_id.as_str()],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(error)?
            .map(|id| LifecycleId::new(id).map_err(error))
            .transpose()
    }
    fn project_revision(
        &mut self,
        f: &dyn RepositoryFence,
        graph: &GraphId,
        id: &LifecycleId,
    ) -> Result<(), StoreError> {
        self.validate_fence(f, true)?;
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(error)?;
        let mut b = binding(&tx, graph)?;
        let r = revision_snapshot(&tx, id)?.ok_or(StoreError::PlanRevisionMismatch)?;
        if b.repository_id != f.repository().id
            || r.metadata.graph_id != *graph
            || r.metadata.repository_id != b.repository_id
            || (r.metadata.predecessor.as_ref() != b.current_revision.as_ref()
                && b.current_revision.as_ref() != Some(id))
            || r.metadata
                .generation_id
                .as_ref()
                .is_some_and(|generation| b.active_generation.as_ref() != Some(generation))
        {
            return Err(StoreError::PlanRevisionMismatch);
        }
        tx.execute(
            "UPDATE runs SET plan_json=?1,plan_sha256=?2,plan_state=?3 WHERE graph_id=?4",
            params![
                r.plan_json,
                r.metadata.plan_hash.as_str(),
                if authorized(&tx, id)? {
                    "PlanAccepted"
                } else {
                    "AwaitingPlanReview"
                },
                graph.as_str()
            ],
        )
        .map_err(error)?;
        b.current_revision = Some(id.clone());
        tx.execute("UPDATE workspace_run_bindings SET current_revision=?1,payload_json=?2 WHERE graph_id=?3",params![id.as_str(),json(&b)?,graph.as_str()]).map_err(error)?;
        tx.commit().map_err(error)
    }
    fn record_scoped_evidence(
        &mut self,
        f: &dyn RepositoryFence,
        s: &ExecutionScope,
        revision: &LifecycleId,
        evidence: &meshloop_domain::evidence::Evidence,
    ) -> Result<(), StoreError> {
        self.validate_fence(f, true)?;
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(error)?;
        scope(&tx, f, s, Some(revision))?;
        let candidate = evidence.candidate();
        let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM attempts WHERE attempt_id=?1 AND task_id=?2 AND graph_id=?3 AND generation_id=?4 AND revision_id=?5)",params![candidate.attempt_id.0,candidate.task_id.0,s.graph_id.as_str(),s.generation_id.as_str(),revision.as_str()],|r|r.get(0)).map_err(error)?;
        if !valid {
            return Err(refused());
        }
        let kind = match evidence {
            meshloop_domain::evidence::Evidence::Deterministic(_) => "deterministic",
            meshloop_domain::evidence::Evidence::ModelReview(_) => "model_review",
            meshloop_domain::evidence::Evidence::HumanAcceptance(_) => "human_acceptance",
        };
        let payload = json(evidence)?;
        // Existing evidence redaction contract remains the caller's obligation; reject
        // recognizable absolute operational paths rather than persisting a private root.
        let decoded: serde_json::Value = serde_json::from_str(&payload).map_err(error)?;
        if !json_strings_safe(&decoded) {
            return Err(refused());
        }
        tx.execute(
            "INSERT INTO evidence VALUES(?1,?2,?3,?4,?5)",
            params![
                candidate.task_id.0,
                candidate.attempt_id.0,
                candidate.revision,
                kind,
                payload
            ],
        )
        .map_err(error)?;
        tx.commit().map_err(error)
    }
    fn scoped_evidence(
        &self,
        g: &GraphId,
        which: &RunScope,
    ) -> Result<Vec<meshloop_domain::evidence::Evidence>, StoreError> {
        let selected = self.resolve_scope(g, which)?;
        let key = selected.as_ref().map(|s| s.generation_id.as_str());
        let mut stmt=self.conn.prepare("SELECT e.payload_json FROM evidence e JOIN attempts a ON a.attempt_id=e.attempt_id AND a.task_id=e.task_id WHERE a.graph_id=?1 AND a.generation_id IS ?2 ORDER BY e.rowid").map_err(error)?;
        stmt.query_map(params![g.as_str(), key], |r| r.get::<_, String>(0))
            .map_err(error)?
            .map(|row| parse(row.map_err(error)?))
            .collect()
    }
    fn create_run_binding(
        &mut self,
        f: &dyn RepositoryFence,
        b: &RunBinding,
    ) -> Result<(), StoreError> {
        self.validate_fence(f, true)?;
        let legacy:bool=self.conn.query_row("SELECT EXISTS(SELECT 1 FROM workspace_legacy_unbound WHERE source_table='runs' AND source_id=?1)",[b.graph_id.as_str()],|r|r.get(0)).map_err(error)?;
        if legacy {
            return Err(StoreError::LegacyOwnershipUnresolved);
        }
        f.root(&b.location.root_alias)?;
        if b.repository_id != f.repository().id
            || b.active_generation.is_some()
            || b.current_revision.is_some()
            || b.layout_version == 0
        {
            return Err(refused());
        }
        self.conn
            .execute(
                "INSERT INTO workspace_run_bindings VALUES(?1,?2,?3,NULL,NULL,?4)",
                params![
                    b.graph_id.as_str(),
                    b.repository_id.as_str(),
                    b.location.root_alias.as_str(),
                    json(b)?
                ],
            )
            .map_err(error)?;
        Ok(())
    }
    fn run_binding(&self, g: &GraphId) -> Result<Option<RunBinding>, StoreError> {
        get(&self.conn, "workspace_run_bindings", "graph_id", g.as_str())
    }
    fn create_provisional(
        &mut self,
        f: &dyn RepositoryFence,
        p: &ProvisionalOwner,
    ) -> Result<(), StoreError> {
        self.validate_fence(f, true)?;
        f.root(&p.location.root_alias)?;
        if p.repository_id != f.repository().id {
            return Err(refused());
        }
        self.conn
            .execute(
                "INSERT INTO workspace_provisional_owners VALUES(?1,?2,?3,?4,?5)",
                params![
                    p.id.as_str(),
                    p.repository_id.as_str(),
                    p.graph_id.as_ref().map(|x| x.as_str()),
                    p.location.root_alias.as_str(),
                    json(p)?
                ],
            )
            .map_err(error)?;
        Ok(())
    }
    fn provisionals(&self, r: &LifecycleId) -> Result<Vec<ProvisionalOwner>, StoreError> {
        list(
            &self.conn,
            "SELECT payload_json FROM workspace_provisional_owners WHERE repo_id=?1 ORDER BY rowid",
            r.as_str(),
        )
    }
    fn promote_provisional(
        &mut self,
        f: &dyn RepositoryFence,
        p: &ProvisionalPromotion,
    ) -> Result<(), StoreError> {
        self.validate_fence(f, true)?;
        let owner: ProvisionalOwner = get(
            &self.conn,
            "workspace_provisional_owners",
            "id",
            p.owner_id.as_str(),
        )?
        .ok_or_else(refused)?;
        let generation_record: GenerationRecord =
            self.generation(&p.generation_id)?.ok_or_else(refused)?;
        let artifact = self.artifact(&p.artifact_id)?.ok_or_else(refused)?;
        if owner.repository_id != f.repository().id
            || generation_record.repository_id != owner.repository_id
            || artifact.generation_id != generation_record.id
            || owner
                .graph_id
                .as_ref()
                .is_some_and(|g| g != &generation_record.graph_id)
        {
            return Err(refused());
        }
        self.conn
            .execute(
                "INSERT INTO workspace_provisional_promotions VALUES(?1,?2,?3)",
                params![
                    p.owner_id.as_str(),
                    p.generation_id.as_str(),
                    p.artifact_id.as_str()
                ],
            )
            .map_err(error)?;
        Ok(())
    }
    fn append_revision(
        &mut self,
        f: &dyn RepositoryFence,
        r: &PlanRevisionSnapshot,
    ) -> Result<(), StoreError> {
        self.validate_fence(f, true)?;
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(error)?;
        insert_revision(&tx, f, r)?;
        tx.commit().map_err(error)
    }
    fn revision(&self, id: &LifecycleId) -> Result<Option<PlanRevisionSnapshot>, StoreError> {
        revision_snapshot(&self.conn, id)
    }
    fn authorize_revision(
        &mut self,
        f: &dyn RepositoryFence,
        a: &RevisionAuthorization,
    ) -> Result<(), StoreError> {
        self.validate_fence(f, true)?;
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(error)?;
        insert_authorization(&tx, f, a)?;
        tx.commit().map_err(error)
    }
    fn authorizations(&self, id: &LifecycleId) -> Result<Vec<RevisionAuthorization>, StoreError> {
        list(
            &self.conn,
            "SELECT payload_json FROM workspace_revision_authorizations WHERE revision_id=?1 ORDER BY rowid",
            id.as_str(),
        )
    }
    fn activate_generation(
        &mut self,
        f: &dyn RepositoryFence,
        a: &GenerationActivation,
    ) -> Result<(), StoreError> {
        self.validate_fence(f, true)?;
        let g = &a.generation;
        let mut b = binding(&self.conn, &g.graph_id)?;
        f.root(&g.root_alias)?;
        if b.current_revision
            .as_ref()
            .is_some_and(|id| id != &a.revision.metadata.id)
        {
            return Err(StoreError::PlanRevisionMismatch);
        }
        if b.repository_id != f.repository().id
            || g.repository_id != b.repository_id
            || g.root_alias != b.location.root_alias
            || g.layout_version != b.layout_version
            || g.ordinal == 0
            || a.revision.metadata.graph_id != g.graph_id
            || a.revision.metadata.plan_hash != g.accepted_plan_hash
            || a.authorization.revision_id != a.revision.metadata.id
            || a.authorization.decision != RevisionDecision::Accepted
        {
            return Err(StoreError::PlanRevisionMismatch);
        }
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(error)?;
        let current: (String, String, String) = tx
            .query_row(
                "SELECT plan_state,plan_sha256,plan_json FROM runs WHERE graph_id=?1",
                [g.graph_id.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(error)?;
        if current.0 != "PlanAccepted"
            || current.1 != g.accepted_plan_hash.as_str()
            || current.2 != a.revision.plan_json
        {
            return Err(StoreError::PlanRevisionMismatch);
        }
        if let Some(existing) = revision_snapshot(&tx, &a.revision.metadata.id)? {
            if existing.metadata != a.revision.metadata
                || existing.plan_json != a.revision.plan_json
            {
                return Err(StoreError::PlanRevisionMismatch);
            }
        } else {
            insert_revision(&tx, f, &a.revision)?;
        }
        let auth: Option<RevisionAuthorization> = get(
            &tx,
            "workspace_revision_authorizations",
            "id",
            a.authorization.id.as_str(),
        )?;
        if let Some(auth) = auth {
            if auth != a.authorization {
                return Err(StoreError::PlanRevisionMismatch);
            }
        } else {
            insert_authorization(&tx, f, &a.authorization)?;
        }
        if !authorized(&tx, &a.revision.metadata.id)? {
            return Err(StoreError::PlanRevisionMismatch);
        }
        tx.execute(
            "INSERT INTO workspace_generations VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                g.id.as_str(),
                g.repository_id.as_str(),
                g.graph_id.as_str(),
                g.ordinal,
                g.root_alias.as_str(),
                g.accepted_plan_hash.as_str(),
                json(g)?
            ],
        )
        .map_err(error)?;
        tx.execute(
            "INSERT INTO workspace_generation_revisions VALUES(?1,?2)",
            params![g.id.as_str(), a.revision.metadata.id.as_str()],
        )
        .map_err(error)?;
        b.active_generation = Some(g.id.clone());
        b.current_revision = Some(a.revision.metadata.id.clone());
        tx.execute("UPDATE workspace_run_bindings SET active_generation=?1,current_revision=?2,payload_json=?3 WHERE graph_id=?4",params![g.id.as_str(),a.revision.metadata.id.as_str(),json(&b)?,g.graph_id.as_str()]).map_err(error)?;
        for artifact in &a.artifacts {
            if artifact.generation_id != g.id
                || artifact.root_alias != g.root_alias
                || artifact.attempt_id.is_some()
            {
                return Err(refused());
            }
            tx.execute(
                "INSERT INTO workspace_artifacts VALUES(?1,?2,?3,?4,NULL,?5)",
                params![
                    artifact.id.as_str(),
                    g.id.as_str(),
                    g.repository_id.as_str(),
                    artifact.root_alias.as_str(),
                    json(artifact)?
                ],
            )
            .map_err(error)?;
        }
        for o in &a.operations {
            operation(&tx, f, o)?;
        }
        for r in &a.events {
            event(
                &tx,
                f,
                &ScopedEvent {
                    scope: ExecutionScope {
                        repository_id: g.repository_id.clone(),
                        graph_id: g.graph_id.clone(),
                        generation_id: g.id.clone(),
                    },
                    revision_id: a.revision.metadata.id.clone(),
                    record: r.clone(),
                },
            )?;
        }
        tx.commit().map_err(error)
    }
    fn select_revision(
        &mut self,
        f: &dyn RepositoryFence,
        s: &ExecutionScope,
        id: &LifecycleId,
    ) -> Result<(), StoreError> {
        self.validate_fence(f, true)?;
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(error)?;
        scope(&tx, f, s, None)?;
        let r = revision_snapshot(&tx, id)?.ok_or(StoreError::PlanRevisionMismatch)?;
        if r.metadata.graph_id != s.graph_id
            || r.metadata.repository_id != s.repository_id
            || r.metadata
                .generation_id
                .as_ref()
                .is_some_and(|g| g != &s.generation_id)
        {
            return Err(StoreError::PlanRevisionMismatch);
        }
        let mut b = binding(&tx, &s.graph_id)?;
        if r.metadata.predecessor.as_ref() != b.current_revision.as_ref()
            && b.current_revision.as_ref() != Some(id)
        {
            return Err(StoreError::PlanRevisionMismatch);
        }
        let accepted = authorized(&tx, id)?;
        tx.execute(
            "UPDATE runs SET plan_json=?1,plan_sha256=?2,plan_state=?3 WHERE graph_id=?4",
            params![
                r.plan_json,
                r.metadata.plan_hash.as_str(),
                if accepted {
                    "PlanAccepted"
                } else {
                    "AwaitingPlanReview"
                },
                s.graph_id.as_str()
            ],
        )
        .map_err(error)?;
        b.current_revision = Some(id.clone());
        tx.execute("UPDATE workspace_run_bindings SET current_revision=?1,payload_json=?2 WHERE graph_id=?3",params![id.as_str(),json(&b)?,s.graph_id.as_str()]).map_err(error)?;
        tx.commit().map_err(error)
    }
    fn resolve_scope(
        &self,
        g: &GraphId,
        which: &RunScope,
    ) -> Result<Option<ExecutionScope>, StoreError> {
        if *which == RunScope::Legacy {
            return Ok(None);
        }
        let b = binding(&self.conn, g)?;
        let id = match which {
            RunScope::ActiveGeneration => b.active_generation.ok_or_else(refused)?,
            RunScope::Generation(id) => id.clone(),
            RunScope::Legacy => unreachable!(),
        };
        let generation = self.generation(&id)?.ok_or_else(refused)?;
        if generation.graph_id != *g || generation.repository_id != b.repository_id {
            return Err(refused());
        }
        Ok(Some(ExecutionScope {
            repository_id: b.repository_id,
            graph_id: g.clone(),
            generation_id: id,
        }))
    }
    fn scoped_events(&self, g: &GraphId, which: &RunScope) -> Result<Vec<StoredEvent>, StoreError> {
        let selected = self.resolve_scope(g, which)?;
        let all = EventLog::records_for_graph(self, g.as_str())?;
        let mut stmt = self
            .conn
            .prepare(
                "SELECT generation_id,revision_id FROM events WHERE graph_id=?1 ORDER BY event_id",
            )
            .map_err(error)?;
        let keys = stmt
            .query_map([g.as_str()], |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                ))
            })
            .map_err(error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(error)?;
        let mut result = Vec::new();
        for (record, (generation, revision)) in all.into_iter().zip(keys) {
            match (&selected, generation, revision) {
                (None, None, None) => result.push(StoredEvent::Legacy(record)),
                (Some(s), Some(generation), Some(revision))
                    if generation == s.generation_id.as_str() =>
                {
                    result.push(StoredEvent::Scoped(ScopedEvent {
                        scope: s.clone(),
                        revision_id: LifecycleId::new(revision).map_err(error)?,
                        record,
                    }))
                }
                _ => {}
            }
        }
        Ok(result)
    }
    fn append_scoped_event(
        &mut self,
        f: &dyn RepositoryFence,
        e: &ScopedEvent,
    ) -> Result<(), StoreError> {
        self.validate_fence(f, true)?;
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(error)?;
        event(&tx, f, e)?;
        tx.commit().map_err(error)
    }
    fn reserve_attempt(
        &mut self,
        f: &dyn RepositoryFence,
        r: &AttemptReservation,
    ) -> Result<ScopedAttempt, StoreError> {
        self.validate_fence(f, true)?;
        f.root(&r.artifact.root_alias)?;
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(error)?;
        scope(&tx, f, &r.scope, Some(&r.revision_id))?;
        let g: GenerationRecord = get(
            &tx,
            "workspace_generations",
            "id",
            r.scope.generation_id.as_str(),
        )?
        .ok_or_else(refused)?;
        if r.artifact.generation_id != g.id
            || r.artifact.root_alias != g.root_alias
            || r.artifact.kind != ArtifactKind::Attempt
            || r.artifact.attempt_id.is_some()
            || r.spawn_intent.target != ProducerTarget::Artifact(r.artifact.id.clone())
            || r.spawn_intent.generation_id.as_ref() != Some(&r.scope.generation_id)
            || r.spawn_intent.graph_id.as_ref() != Some(&r.scope.graph_id)
            || r.spawn_intent.kind != ProducerKind::Spawn
        {
            return Err(refused());
        }
        let id: i64 = tx
            .query_row(
                "SELECT MAX((SELECT next_id FROM workspace_attempt_sequence WHERE singleton=1),COALESCE(MAX(attempt_id),0)+1) FROM attempts",
                [],
                |x| x.get(0),
            )
            .map_err(error)?;
        let id = AttemptId(u32::try_from(id).map_err(error)?);
        tx.execute(
            "UPDATE workspace_attempt_sequence SET next_id=?1 WHERE singleton=1",
            [u64::from(id.0) + 1],
        )
        .map_err(error)?;
        let mut artifact = r.artifact.clone();
        artifact.attempt_id = Some(id);
        tx.execute("INSERT INTO attempts(attempt_id,graph_id,task_id,harness,model_ref,generation_id,revision_id) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![id.0,r.scope.graph_id.as_str(),r.task_id.0,r.harness,r.model_ref,r.scope.generation_id.as_str(),r.revision_id.as_str()]).map_err(error)?;
        tx.execute(
            "INSERT INTO workspace_artifacts VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                artifact.id.as_str(),
                g.id.as_str(),
                g.repository_id.as_str(),
                artifact.root_alias.as_str(),
                id.0,
                json(&artifact)?
            ],
        )
        .map_err(error)?;
        tx.execute(
            "INSERT INTO workspace_attempt_metadata VALUES(?1,?2,?3,?4,NULL)",
            params![
                id.0,
                artifact.id.as_str(),
                artifact.root_alias.as_str(),
                artifact.relative_path.as_str()
            ],
        )
        .map_err(error)?;
        operation(&tx, f, &r.spawn_intent)?;
        tx.commit().map_err(error)?;
        Ok(ScopedAttempt {
            scope: r.scope.clone(),
            revision_id: r.revision_id.clone(),
            artifact_id: artifact.id,
            attempt_id: id,
            task_id: r.task_id,
            harness: r.harness.clone(),
            model_ref: r.model_ref.clone(),
            location: PortableLocation {
                root_alias: artifact.root_alias,
                relative_path: artifact.relative_path,
            },
            process: None,
            pane_id: None,
            started_at: None,
            ended_at: None,
            outcome: None,
        })
    }
    fn update_scoped_attempt(
        &mut self,
        f: &dyn RepositoryFence,
        a: &ScopedAttempt,
    ) -> Result<(), StoreError> {
        for value in [
            &a.harness,
            &a.model_ref,
            &a.pane_id,
            &a.started_at,
            &a.ended_at,
            &a.outcome,
        ]
        .into_iter()
        .flatten()
        {
            if !public_text(value) {
                return Err(refused());
            }
        }
        self.validate_fence(f, true)?;
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(error)?;
        scope(&tx, f, &a.scope, Some(&a.revision_id))?;
        let current:Option<(String,String,String)>=tx.query_row("SELECT m.artifact_id,m.root_alias,m.relative_path FROM workspace_attempt_metadata m JOIN attempts a USING(attempt_id) WHERE a.attempt_id=?1 AND a.graph_id=?2 AND a.generation_id=?3 AND a.revision_id=?4 AND a.task_id=?5",params![a.attempt_id.0,a.scope.graph_id.as_str(),a.scope.generation_id.as_str(),a.revision_id.as_str(),a.task_id.0],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(error)?;
        if current
            != Some((
                a.artifact_id.as_str().into(),
                a.location.root_alias.as_str().into(),
                a.location.relative_path.as_str().into(),
            ))
        {
            return Err(refused());
        }
        tx.execute("UPDATE attempts SET pid=?1,pane_id=?2,started_at=?3,ended_at=?4,outcome=?5 WHERE attempt_id=?6",params![a.process.as_ref().map(|p|p.pid),a.pane_id,a.started_at,a.ended_at,a.outcome,a.attempt_id.0]).map_err(error)?;
        tx.execute(
            "UPDATE workspace_attempt_metadata SET process_json=?1 WHERE attempt_id=?2",
            params![a.process.as_ref().map(json).transpose()?, a.attempt_id.0],
        )
        .map_err(error)?;
        tx.commit().map_err(error)
    }
    fn scoped_attempts(
        &self,
        g: &GraphId,
        which: &RunScope,
    ) -> Result<Vec<StoredAttempt>, StoreError> {
        let selected = self.resolve_scope(g, which)?;
        let mut stmt=self.conn.prepare("SELECT attempt_id,generation_id,revision_id FROM attempts WHERE graph_id=?1 ORDER BY attempt_id").map_err(error)?;
        let keys = stmt
            .query_map([g.as_str()], |r| {
                Ok((
                    r.get::<_, u32>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            })
            .map_err(error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(error)?;
        let mut result = Vec::new();
        for (id, generation, revision) in keys {
            let row = self.load_attempt(AttemptId(id))?.ok_or_else(refused)?;
            match (&selected, generation, revision) {
                (None, None, None) => result.push(StoredAttempt::Legacy(row)),
                (Some(s), Some(generation), Some(revision))
                    if generation == s.generation_id.as_str() =>
                {
                    let m:(String,String,String,Option<String>)=self.conn.query_row("SELECT artifact_id,root_alias,relative_path,process_json FROM workspace_attempt_metadata WHERE attempt_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(error)?;
                    if row.worktree_path.is_some() {
                        return Err(refused());
                    }
                    result.push(StoredAttempt::Scoped(ScopedAttempt {
                        scope: s.clone(),
                        revision_id: LifecycleId::new(revision).map_err(error)?,
                        artifact_id: LifecycleId::new(m.0).map_err(error)?,
                        attempt_id: row.attempt_id,
                        task_id: row.task_id,
                        harness: row.harness,
                        model_ref: row.model_ref,
                        location: PortableLocation {
                            root_alias: RootAlias::new(m.1).map_err(error)?,
                            relative_path: RelativePath::new(m.2).map_err(error)?,
                        },
                        process: m.3.map(parse).transpose()?,
                        pane_id: row.pane_id,
                        started_at: row.started_at,
                        ended_at: row.ended_at,
                        outcome: row.outcome,
                    }));
                }
                _ => {}
            }
        }
        Ok(result)
    }
    fn append_operation(
        &mut self,
        f: &dyn RepositoryFence,
        o: &ProducerOperation,
    ) -> Result<(), StoreError> {
        self.validate_fence(f, true)?;
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(error)?;
        operation(&tx, f, o)?;
        tx.commit().map_err(error)
    }
    fn operations(
        &self,
        repo: &LifecycleId,
        s: Option<&ExecutionScope>,
    ) -> Result<Vec<ProducerOperation>, StoreError> {
        let records: Vec<ProducerOperation> = list(
            &self.conn,
            "SELECT payload_json FROM workspace_operations WHERE repo_id=?1 ORDER BY rowid",
            repo.as_str(),
        )?;
        Ok(records
            .into_iter()
            .filter(|o| {
                s.is_none_or(|s| {
                    o.repository_id == s.repository_id
                        && o.graph_id.as_ref() == Some(&s.graph_id)
                        && o.generation_id.as_ref() == Some(&s.generation_id)
                })
            })
            .collect())
    }
    fn append_operation_step(
        &mut self,
        f: &dyn RepositoryFence,
        s: &OperationStep,
    ) -> Result<(), StoreError> {
        self.validate_fence(f, true)?;
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(error)?;
        let o: ProducerOperation =
            get(&tx, "workspace_operations", "id", s.operation_id.as_str())?.ok_or_else(refused)?;
        let pending: bool = tx
            .query_row(
                "SELECT pending FROM workspace_operations WHERE id=?1",
                [s.operation_id.as_str()],
                |r| r.get(0),
            )
            .map_err(error)?;
        let n: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM workspace_operation_steps WHERE operation_id=?1",
                [s.operation_id.as_str()],
                |r| r.get(0),
            )
            .map_err(error)?;
        if o.repository_id != f.repository().id
            || !pending
            || s.step != u32::try_from(n + 1).map_err(error)?
            || s.step > o.max_steps
            || (s.state == OperationState::Completed && s.observed != o.after)
        {
            return Err(refused());
        }
        tx.execute(
            "INSERT INTO workspace_operation_steps VALUES(?1,?2,?3,?4)",
            params![s.id.as_str(), s.operation_id.as_str(), s.step, json(s)?],
        )
        .map_err(error)?;
        if s.state == OperationState::Blocked || s.step == o.max_steps {
            tx.execute(
                "UPDATE workspace_operations SET pending=0 WHERE id=?1",
                [s.operation_id.as_str()],
            )
            .map_err(error)?;
        }
        tx.commit().map_err(error)
    }
    fn operation_steps(&self, id: &LifecycleId) -> Result<Vec<OperationStep>, StoreError> {
        list(
            &self.conn,
            "SELECT payload_json FROM workspace_operation_steps WHERE operation_id=?1 ORDER BY step",
            id.as_str(),
        )
    }
    fn register_driver(
        &mut self,
        f: &dyn RepositoryFence,
        d: &DriverRecord,
    ) -> Result<(), StoreError> {
        self.validate_fence(f, true)?;
        validate_driver(&self.conn, f, d)?;
        self.conn
            .execute(
                "INSERT INTO workspace_drivers VALUES(?1,?2,?3)",
                params![d.id.as_str(), f.repository().id.as_str(), json(d)?],
            )
            .map_err(error)?;
        Ok(())
    }
    fn update_driver(
        &mut self,
        f: &dyn RepositoryFence,
        d: &DriverRecord,
    ) -> Result<(), StoreError> {
        self.validate_fence(f, true)?;
        validate_driver(&self.conn, f, d)?;
        let old: DriverRecord =
            get(&self.conn, "workspace_drivers", "id", d.id.as_str())?.ok_or_else(refused)?;
        if old.target != d.target
            || old.nonce != d.nonce
            || old
                .process
                .as_ref()
                .is_some_and(|p| Some(p) != d.process.as_ref())
            || d.heartbeat < old.heartbeat
        {
            return Err(refused());
        }
        self.conn
            .execute(
                "UPDATE workspace_drivers SET payload_json=?1 WHERE id=?2",
                params![json(d)?, d.id.as_str()],
            )
            .map_err(error)?;
        Ok(())
    }
    fn drivers(&self, repo: &LifecycleId) -> Result<Vec<DriverRecord>, StoreError> {
        list(
            &self.conn,
            "SELECT payload_json FROM workspace_drivers WHERE repo_id=?1 ORDER BY rowid",
            repo.as_str(),
        )
    }
    fn cancellation_requests(
        &self,
        id: &LifecycleId,
    ) -> Result<Vec<CancellationObservation>, StoreError> {
        let mut stmt=self.conn.prepare("SELECT payload_json,status FROM workspace_control_requests WHERE driver_id=?1 ORDER BY rowid").map_err(error)?;
        stmt.query_map([id.as_str()], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(error)?
        .map(|r| {
            let (j, s) = r.map_err(error)?;
            Ok(CancellationObservation {
                request: parse(j)?,
                status: parse(format!("\"{s}\""))?,
            })
        })
        .collect()
    }
    fn acknowledge_cancellation(
        &mut self,
        f: &dyn RepositoryFence,
        id: &LifecycleId,
        status: CancellationStatus,
    ) -> Result<(), StoreError> {
        self.validate_fence(f, true)?;
        let request: CancellationRequest =
            get(&self.conn, "workspace_control_requests", "id", id.as_str())?
                .ok_or_else(refused)?;
        let repo = match &request.target {
            CancellationTarget::Execution { scope, .. } => &scope.repository_id,
            CancellationTarget::Provisional { repository_id, .. } => repository_id,
        };
        if repo != &f.repository().id || status == CancellationStatus::Pending {
            return Err(refused());
        }
        self.conn
            .execute(
                "UPDATE workspace_control_requests SET status=?1 WHERE id=?2 AND status='Pending'",
                params![format!("{status:?}"), id.as_str()],
            )
            .map_err(error)?;
        Ok(())
    }
    fn artifacts(&self, s: &ExecutionScope) -> Result<Vec<ArtifactRecord>, StoreError> {
        let g = self.generation(&s.generation_id)?.ok_or_else(refused)?;
        if g.repository_id != s.repository_id || g.graph_id != s.graph_id {
            return Err(refused());
        }
        list(
            &self.conn,
            "SELECT payload_json FROM workspace_artifacts WHERE generation_id=?1 ORDER BY rowid",
            s.generation_id.as_str(),
        )
    }
    fn bind_artifact(
        &mut self,
        f: &dyn RepositoryFence,
        b: &ArtifactBinding,
    ) -> Result<(), StoreError> {
        self.validate_fence(f, true)?;
        let a = self.artifact(&b.artifact_id)?.ok_or_else(refused)?;
        let g = self.generation(&a.generation_id)?.ok_or_else(refused)?;
        let r = revision_snapshot(&self.conn, &b.revision_id)?.ok_or_else(refused)?;
        if g.repository_id != f.repository().id
            || r.metadata.graph_id != g.graph_id
            || r.metadata.repository_id != g.repository_id
            || (b.logical_tag.is_some() && a.kind != ArtifactKind::Deliverable)
        {
            return Err(refused());
        }
        self.conn
            .execute(
                "INSERT INTO workspace_artifact_bindings VALUES(?1,?2,?3)",
                params![b.artifact_id.as_str(), b.revision_id.as_str(), json(b)?],
            )
            .map_err(error)?;
        Ok(())
    }
    fn artifact_bindings(&self, s: &ExecutionScope) -> Result<Vec<ArtifactBinding>, StoreError> {
        list(
            &self.conn,
            "SELECT b.payload_json FROM workspace_artifact_bindings b JOIN workspace_artifacts a ON a.id=b.artifact_id WHERE a.generation_id=?1 ORDER BY b.rowid",
            s.generation_id.as_str(),
        )
    }
}

/// Narrow concurrent cancellation-only connection. It never reads the locked registry,
/// migrates a database, initializes ownership, or exposes a mutable run store.
pub struct CancellationClient {
    conn: Connection,
    handle: same_file::Handle,
    database_id: NativeIdentity,
    common: std::path::PathBuf,
    common_id: NativeIdentity,
}
impl CancellationClient {
    pub fn open(
        git: &std::path::Path,
        checkout: &std::path::Path,
        ledger: &std::path::Path,
    ) -> Result<Self, StoreError> {
        let mut command = crate::process::OwnedSpawnSpec::new(git);
        command.arg("-C").arg(checkout).args([
            "rev-parse",
            "--path-format=absolute",
            "--git-common-dir",
        ]);
        let output = super::bounded_stdout(command, std::time::Duration::from_secs(10))?;
        let common = std::fs::canonicalize(std::path::PathBuf::from(
            std::str::from_utf8(&output).map_err(error)?.trim(),
        ))
        .map_err(error)?;
        let common_id = native_identity(&common)?;
        let reader = SqliteStore::open_read_only(ledger)?.ok_or_else(refused)?;
        let record: Option<String> = reader
            .conn
            .query_row(
                "SELECT common_native_id FROM workspace_repositories LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(error)?;
        if record.as_deref() != Some(common_id.as_str()) {
            return Err(refused());
        }
        let handle = same_file::Handle::from_file(std::fs::File::open(ledger).map_err(error)?)
            .map_err(error)?;
        let database_id = native_identity(ledger)?;
        verify_handle(&handle, ledger)?;
        let conn = Connection::open_with_flags(
            ledger,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(error)?;
        conn.busy_timeout(std::time::Duration::from_millis(500))
            .map_err(error)?;
        conn.pragma_update(None, "foreign_keys", true)
            .map_err(error)?;
        let client = Self {
            conn,
            handle,
            database_id,
            common,
            common_id,
        };
        client.validate()?;
        Ok(client)
    }
    fn validate(&self) -> Result<(), StoreError> {
        let path = std::path::Path::new(self.conn.path().ok_or_else(refused)?);
        verify_handle(&self.handle, path)?;
        if native_identity(path)? != self.database_id
            || native_identity(&self.common)? != self.common_id
        {
            return Err(refused());
        }
        Ok(())
    }
    pub fn enqueue(
        &mut self,
        r: &CancellationRequest,
    ) -> Result<CancellationObservation, StoreError> {
        self.validate()?;
        let (driver, repo) = match &r.target {
            CancellationTarget::Execution {
                scope, driver_id, ..
            } => (driver_id, &scope.repository_id),
            CancellationTarget::Provisional {
                repository_id,
                driver_id,
                ..
            } => (driver_id, repository_id),
        };
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(error)?;
        let d: DriverRecord =
            get(&tx, "workspace_drivers", "id", driver.as_str())?.ok_or_else(refused)?;
        if driver_repo(&d.target) != repo
            || d.process.as_ref() != Some(&r.process)
            || !matches!(d.phase, DriverPhase::Starting | DriverPhase::Running)
        {
            return Err(refused());
        }
        match (&r.target, &d.target) {
            (
                CancellationTarget::Execution { scope, task_id, .. },
                DriverTarget::Execution(actual),
            ) if scope == actual => {
                let b = binding(&tx, &scope.graph_id)?;
                if b.active_generation.as_ref() != Some(&scope.generation_id) {
                    return Err(refused());
                }
                if let Some(task) = task_id {
                    let plan: String = tx
                        .query_row(
                            "SELECT plan_json FROM runs WHERE graph_id=?1",
                            [scope.graph_id.as_str()],
                            |r| r.get(0),
                        )
                        .map_err(error)?;
                    let graph: meshloop_domain::task_graph::TaskGraph =
                        serde_json::from_str(&plan).map_err(error)?;
                    if !graph.nodes.iter().any(|node| node.id == *task) {
                        return Err(refused());
                    }
                }
            }
            (
                CancellationTarget::Provisional {
                    repository_id,
                    owner_id,
                    ..
                },
                DriverTarget::Provisional {
                    repository_id: actual_repo,
                    owner_id: actual_owner,
                },
            ) if repository_id == actual_repo && owner_id == actual_owner => {}
            _ => return Err(refused()),
        }
        let selector = json(&r.target)?;
        let existing:Option<(String,String)>=tx.query_row("SELECT payload_json,status FROM workspace_control_requests WHERE id=?1 OR (driver_id=?2 AND selector=?3)",params![r.id.as_str(),driver.as_str(),selector],|row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(error)?;
        if let Some((payload, status)) = existing {
            let old: CancellationRequest = parse(payload)?;
            if old.target != r.target || old.process != r.process {
                return Err(refused());
            }
            return Ok(CancellationObservation {
                request: old,
                status: parse(format!("\"{status}\""))?,
            });
        }
        let count:i64=tx.query_row("SELECT COUNT(*) FROM workspace_control_requests WHERE driver_id=?1 AND status='Pending'",[driver.as_str()],|r|r.get(0)).map_err(error)?;
        if count >= 64 {
            return Err(StoreError::LifecycleBusy);
        }
        tx.execute(
            "INSERT INTO workspace_control_requests VALUES(?1,?2,?3,'Pending',?4)",
            params![r.id.as_str(), driver.as_str(), selector, json(r)?],
        )
        .map_err(error)?;
        tx.commit().map_err(error)?;
        Ok(CancellationObservation {
            request: r.clone(),
            status: CancellationStatus::Pending,
        })
    }
    pub fn observation(
        &self,
        id: &LifecycleId,
    ) -> Result<Option<CancellationObservation>, StoreError> {
        self.validate()?;
        let row: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT payload_json,status FROM workspace_control_requests WHERE id=?1",
                [id.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(error)?;
        row.map(|(payload, status)| {
            Ok(CancellationObservation {
                request: parse(payload)?,
                status: parse(format!("\"{status}\""))?,
            })
        })
        .transpose()
    }
}

fn public_text(value: &str) -> bool {
    value.len() <= 4096
        && !value.chars().any(char::is_control)
        && !value.contains('\\')
        && !value
            .as_bytes()
            .windows(3)
            .any(|w| w[0].is_ascii_alphabetic() && w[1] == b':' && w[2] == b'/')
        && !value
            .split_whitespace()
            .any(|part| part.trim_start_matches(['\'', '"', '(']).starts_with('/'))
}

fn json_strings_safe(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(s) => {
            !s.contains('\\')
                && !s
                    .as_bytes()
                    .windows(3)
                    .any(|w| w[0].is_ascii_alphabetic() && w[1] == b':' && w[2] == b'/')
                && !s
                    .split_whitespace()
                    .any(|p| p.trim_start_matches(['\'', '"', '(']).starts_with('/'))
        }
        serde_json::Value::Array(values) => values.iter().all(json_strings_safe),
        serde_json::Value::Object(values) => values.values().all(json_strings_safe),
        _ => true,
    }
}
