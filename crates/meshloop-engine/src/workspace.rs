//! Explicit lifecycle persistence and local repository ownership ports.
use crate::ports::StoreError;
use meshloop_domain::workspace::*;
use std::path::Path;

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
        graph: &LifecycleId,
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
