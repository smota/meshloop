//! Explicit lifecycle persistence and local repository ownership ports.
use crate::ports::StoreError;
use meshloop_domain::workspace::*;
use std::path::Path;
pub mod read;

pub trait RepositoryFence {
    fn repository(&self) -> &RepositoryRecord;
    fn validate(&self) -> Result<(), StoreError>;
    fn ledger_identity(&self) -> &NativeIdentity;
    fn root(&self, alias: &RootAlias) -> Result<RootRecord, StoreError>;
}
pub trait RepositoryIdentity {
    type Fence: RepositoryFence;
    fn acquire(&self, checkout: &Path, ledger: &Path) -> Result<Self::Fence, StoreError>;
}
pub trait WorkspaceLedger {
    fn bind_repository(&mut self, fence: &dyn RepositoryFence) -> Result<(), StoreError>;
    fn repository(&self, id: &LifecycleId) -> Result<Option<RepositoryRecord>, StoreError>;
    fn register_root(
        &mut self,
        fence: &dyn RepositoryFence,
        alias: &RootAlias,
    ) -> Result<(), StoreError>;
    fn insert_generation(
        &mut self,
        fence: &dyn RepositoryFence,
        record: &GenerationRecord,
    ) -> Result<(), StoreError>;
    fn generation(&self, id: &LifecycleId) -> Result<Option<GenerationRecord>, StoreError>;
    fn generations(
        &self,
        repository: &LifecycleId,
        graph: &GraphId,
    ) -> Result<Vec<GenerationRecord>, StoreError>;
    fn insert_artifacts(
        &mut self,
        fence: &dyn RepositoryFence,
        records: &[ArtifactRecord],
        dependencies: &[ArtifactDependency],
    ) -> Result<(), StoreError>;
    fn artifact(&self, id: &LifecycleId) -> Result<Option<ArtifactRecord>, StoreError>;
    fn dependencies(&self, artifact: &LifecycleId) -> Result<Vec<ArtifactDependency>, StoreError>;
    fn append_fact(
        &mut self,
        fence: &dyn RepositoryFence,
        fact: &LifecycleFact,
    ) -> Result<(), StoreError>;
    fn facts(&self, artifact: &LifecycleId) -> Result<Vec<LifecycleFact>, StoreError>;
    fn append_journal(
        &mut self,
        fence: &dyn RepositoryFence,
        entry: &JournalEntry,
    ) -> Result<(), StoreError>;
    fn journal(&self, artifact: &LifecycleId) -> Result<Vec<JournalEntry>, StoreError>;
}

/// Exact source is protected plan storage, intentionally not Serialize or Debug.
pub struct PlanRevisionSnapshot {
    pub metadata: PlanRevision,
    pub plan_json: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedEvent {
    pub scope: ExecutionScope,
    pub revision_id: LifecycleId,
    pub record: crate::ports::TransitionRecord,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedAttempt {
    pub scope: ExecutionScope,
    pub revision_id: LifecycleId,
    pub artifact_id: LifecycleId,
    pub attempt_id: meshloop_domain::evidence::AttemptId,
    pub task_id: meshloop_domain::task_graph::TaskId,
    pub harness: Option<String>,
    pub model_ref: Option<String>,
    pub location: PortableLocation,
    pub process: Option<ProcessIdentity>,
    pub pane_id: Option<String>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub outcome: Option<String>,
}
pub struct AttemptReservation {
    pub scope: ExecutionScope,
    pub revision_id: LifecycleId,
    pub task_id: meshloop_domain::task_graph::TaskId,
    pub harness: Option<String>,
    pub model_ref: Option<String>,
    pub artifact: ArtifactRecord,
    pub spawn_intent: ProducerOperation,
}
pub struct GenerationActivation {
    pub generation: GenerationRecord,
    pub revision: PlanRevisionSnapshot,
    pub authorization: RevisionAuthorization,
    pub artifacts: Vec<ArtifactRecord>,
    pub operations: Vec<ProducerOperation>,
    pub events: Vec<crate::ports::TransitionRecord>,
}
/// Required writer capability. Read-only access has no successful mutation path.
pub trait LifecycleAccess {
    /// Actual current owner identity. `tree_token` is the retained driver's nonce
    /// ownership marker, not an OS process-group identifier.
    fn owner_process_identity(
        &self,
        _nonce: &LifecycleId,
    ) -> Result<Option<ProcessIdentity>, StoreError> {
        Ok(None)
    }
    fn new_id(&mut self) -> Result<LifecycleId, StoreError>;
    fn fence(&self) -> Result<&dyn RepositoryFence, StoreError>;
    fn resolve(&self, location: &PortableLocation) -> Result<std::path::PathBuf, StoreError>;
    fn bootstrap_root(&mut self, alias: RootAlias, path: &Path) -> Result<RootRecord, StoreError>;
}
pub struct ReadOnlyLifecycleAccess;
impl LifecycleAccess for ReadOnlyLifecycleAccess {
    fn new_id(&mut self) -> Result<LifecycleId, StoreError> {
        Err(StoreError::ReadOnly)
    }
    fn fence(&self) -> Result<&dyn RepositoryFence, StoreError> {
        Err(StoreError::ReadOnly)
    }
    fn resolve(&self, _: &PortableLocation) -> Result<std::path::PathBuf, StoreError> {
        Err(StoreError::ReadOnly)
    }
    fn bootstrap_root(&mut self, _: RootAlias, _: &Path) -> Result<RootRecord, StoreError> {
        Err(StoreError::ReadOnly)
    }
}
pub trait RuntimeLedger: WorkspaceLedger {
    /// Invoke once within a consistent read snapshot. Nested calls reuse the
    /// caller's transaction; ending an owned snapshot must propagate failures.
    fn with_read_snapshot(&self, observe: &mut dyn FnMut()) -> Result<(), StoreError>;
    /// Exact immutable revision used to activate this generation.
    fn generation_initial_revision(
        &self,
        scope: &ExecutionScope,
    ) -> Result<Option<LifecycleId>, StoreError>;
    fn project_revision(
        &mut self,
        fence: &dyn RepositoryFence,
        graph: &GraphId,
        revision: &LifecycleId,
    ) -> Result<(), StoreError>;
    fn record_scoped_evidence(
        &mut self,
        fence: &dyn RepositoryFence,
        scope: &ExecutionScope,
        revision: &LifecycleId,
        evidence: &meshloop_domain::evidence::Evidence,
    ) -> Result<(), StoreError>;
    fn scoped_evidence(
        &self,
        graph: &GraphId,
        scope: &RunScope,
    ) -> Result<Vec<meshloop_domain::evidence::Evidence>, StoreError>;
    fn artifacts(&self, scope: &ExecutionScope) -> Result<Vec<ArtifactRecord>, StoreError>;
    fn bind_artifact(
        &mut self,
        fence: &dyn RepositoryFence,
        binding: &ArtifactBinding,
    ) -> Result<(), StoreError>;
    fn artifact_bindings(&self, scope: &ExecutionScope)
    -> Result<Vec<ArtifactBinding>, StoreError>;
    fn create_run_binding(
        &mut self,
        fence: &dyn RepositoryFence,
        binding: &RunBinding,
    ) -> Result<(), StoreError>;
    fn run_binding(&self, graph: &GraphId) -> Result<Option<RunBinding>, StoreError>;
    fn create_provisional(
        &mut self,
        fence: &dyn RepositoryFence,
        owner: &ProvisionalOwner,
    ) -> Result<(), StoreError>;
    fn provisionals(&self, repository: &LifecycleId) -> Result<Vec<ProvisionalOwner>, StoreError>;
    fn promote_provisional(
        &mut self,
        fence: &dyn RepositoryFence,
        promotion: &ProvisionalPromotion,
    ) -> Result<(), StoreError>;
    fn append_revision(
        &mut self,
        fence: &dyn RepositoryFence,
        revision: &PlanRevisionSnapshot,
    ) -> Result<(), StoreError>;
    fn revision(&self, id: &LifecycleId) -> Result<Option<PlanRevisionSnapshot>, StoreError>;
    fn authorize_revision(
        &mut self,
        fence: &dyn RepositoryFence,
        authorization: &RevisionAuthorization,
    ) -> Result<(), StoreError>;
    fn authorizations(
        &self,
        revision: &LifecycleId,
    ) -> Result<Vec<RevisionAuthorization>, StoreError>;
    fn activate_generation(
        &mut self,
        fence: &dyn RepositoryFence,
        activation: &GenerationActivation,
    ) -> Result<(), StoreError>;
    fn select_revision(
        &mut self,
        fence: &dyn RepositoryFence,
        scope: &ExecutionScope,
        revision: &LifecycleId,
    ) -> Result<(), StoreError>;
    fn resolve_scope(
        &self,
        graph: &GraphId,
        scope: &RunScope,
    ) -> Result<Option<ExecutionScope>, StoreError>;
    fn scoped_events(
        &self,
        graph: &GraphId,
        scope: &RunScope,
    ) -> Result<Vec<StoredEvent>, StoreError>;
    fn append_scoped_event(
        &mut self,
        fence: &dyn RepositoryFence,
        event: &ScopedEvent,
    ) -> Result<(), StoreError>;
    fn reserve_attempt(
        &mut self,
        fence: &dyn RepositoryFence,
        reservation: &AttemptReservation,
    ) -> Result<ScopedAttempt, StoreError>;
    fn update_scoped_attempt(
        &mut self,
        fence: &dyn RepositoryFence,
        attempt: &ScopedAttempt,
    ) -> Result<(), StoreError>;
    fn scoped_attempts(
        &self,
        graph: &GraphId,
        scope: &RunScope,
    ) -> Result<Vec<StoredAttempt>, StoreError>;
    fn append_operation(
        &mut self,
        fence: &dyn RepositoryFence,
        operation: &ProducerOperation,
    ) -> Result<(), StoreError>;
    fn operations(
        &self,
        repository: &LifecycleId,
        scope: Option<&ExecutionScope>,
    ) -> Result<Vec<ProducerOperation>, StoreError>;
    fn append_operation_step(
        &mut self,
        fence: &dyn RepositoryFence,
        step: &OperationStep,
    ) -> Result<(), StoreError>;
    fn operation_steps(&self, id: &LifecycleId) -> Result<Vec<OperationStep>, StoreError>;
    fn register_driver(
        &mut self,
        fence: &dyn RepositoryFence,
        driver: &DriverRecord,
    ) -> Result<(), StoreError>;
    fn update_driver(
        &mut self,
        fence: &dyn RepositoryFence,
        driver: &DriverRecord,
    ) -> Result<(), StoreError>;
    fn drivers(&self, repository: &LifecycleId) -> Result<Vec<DriverRecord>, StoreError>;
    fn cancellation_requests(
        &self,
        driver: &LifecycleId,
    ) -> Result<Vec<CancellationObservation>, StoreError>;
    fn acknowledge_cancellation(
        &mut self,
        fence: &dyn RepositoryFence,
        id: &LifecycleId,
        status: CancellationStatus,
    ) -> Result<(), StoreError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredEvent {
    Legacy(crate::ports::TransitionRecord),
    Scoped(ScopedEvent),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredAttempt {
    Legacy(crate::ports::AttemptRow),
    Scoped(ScopedAttempt),
}
