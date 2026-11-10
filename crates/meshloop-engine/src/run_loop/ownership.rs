//! Run-loop ownership (issue #142). A run this version creates registers its
//! repository fence, run binding, generation and one artifact per worktree before
//! that worktree is added, so cleanup can verify ownership later. Runs that already
//! had worktrees before this version keep their legacy state and still need adoption.

use meshloop_domain::digest::sha256_hex;
use meshloop_domain::evidence::AttemptId;
use meshloop_domain::state::PlanState;
use meshloop_domain::workspace::{
    ArtifactKind, ArtifactRecord, GenerationRecord, GraphId, PlanHash, RUN_LOOP_LAYOUT_VERSION,
    RelativePath,
};

use super::{OrchestratorError, RunLoop};
use crate::ports::StoreError;
use crate::workspace::RunOwnershipLease;

/// A run is pinned to the root it was bound under. A lease for another worktree
/// base is refused before anything is recorded or added.
fn pinned(
    lease: &dyn RunOwnershipLease,
    binding: &GenerationRecord,
    graph_id: &str,
) -> Result<(), OrchestratorError> {
    if !lease.is_base_root(&binding.root_alias)? {
        return Err(unpinned(graph_id));
    }
    Ok(())
}

fn unpinned(graph_id: &str) -> OrchestratorError {
    OrchestratorError::Illegal(format!(
        "run '{graph_id}' is pinned to its registered worktree root; \
         use it without changing --worktree-base or [workspace] root"
    ))
}

fn graph_identity(graph_id: &str) -> Result<GraphId, OrchestratorError> {
    GraphId::new(graph_id).map_err(|_| OrchestratorError::Store(StoreError::PlanRevisionMismatch))
}

impl RunLoop<'_> {
    /// Registers durable ownership for a fresh, accepted run before its first
    /// worktree. A run is fresh only when it has no attempt, no integrate worktree
    /// and no integrate branch; anything else predates ownership and stays legacy.
    /// A graph id that is not a portable relative path also stays legacy.
    pub(super) fn own_new_run(&mut self, graph_id: &str) -> Result<(), OrchestratorError> {
        let Some(ownership) = self.ownership else {
            return Ok(());
        };
        let graph = graph_identity(graph_id)?;
        if self.store.run_binding(&graph)?.is_some()
            || self.store.run_loop_generation(&graph)?.is_some()
        {
            return Ok(());
        }
        if self.store.legacy_ownership_unresolved(&graph)? {
            return Err(StoreError::LegacyOwnershipUnresolved.into());
        }
        let Some(row) = self.store.load_run(graph_id)? else {
            return Ok(());
        };
        if row.plan_state != PlanState::PlanAccepted
            || !self.store.attempts_for_graph(graph_id)?.is_empty()
            || self
                .workspace
                .worktree_exists(&self.integrate_path(graph_id))
            || self
                .workspace
                .branch_exists(&self.integrate_branch(graph_id))?
        {
            return Ok(());
        }
        let Ok(relative_path) = RelativePath::new(graph_id) else {
            return Ok(());
        };
        let plan_hash = PlanHash::new(sha256_hex(row.plan_json.as_bytes()))
            .map_err(|_| OrchestratorError::Store(StoreError::PlanRevisionMismatch))?;
        let mut lease = ownership.acquire()?;
        let alias = lease.root_alias().clone();
        let generation_id = lease.new_id()?;
        let fence = lease.fence();
        self.store.bind_repository(fence)?;
        self.store.register_root(fence, &alias)?;
        let binding = GenerationRecord {
            id: generation_id,
            repository_id: fence.repository().id.clone(),
            graph_id: graph,
            ordinal: 1,
            accepted_plan_hash: plan_hash,
            root_alias: alias,
            relative_path,
            layout_version: RUN_LOOP_LAYOUT_VERSION,
        };
        self.store.register_run_loop_run(fence, &binding)?;
        Ok(())
    }

    /// True when the run loop owns `graph_id` and must register its worktrees.
    pub(super) fn run_loop_owned(&self, graph_id: &str) -> Result<bool, OrchestratorError> {
        if self.ownership.is_none() {
            return Ok(false);
        }
        let graph = graph_identity(graph_id)?;
        Ok(self.store.run_loop_generation(&graph)?.is_some())
    }

    /// Registers the worktree `leaf` of an owned run before it is created. The run
    /// is pinned to its registered root: a different worktree base is refused.
    /// Unowned runs are left unchanged.
    pub(super) fn own_artifact(
        &mut self,
        graph_id: &str,
        kind: ArtifactKind,
        leaf: &str,
        attempt_id: Option<AttemptId>,
    ) -> Result<(), OrchestratorError> {
        let Some(ownership) = self.ownership else {
            return Ok(());
        };
        if !self.run_loop_owned(graph_id)? {
            return Ok(());
        }
        let mut lease = ownership.acquire()?;
        self.own_artifact_held(lease.as_mut(), graph_id, kind, leaf, attempt_id)
    }

    /// A run-loop run is pinned to the root it was bound under. Entry points that
    /// derive worktree paths refuse another worktree base before any state changes.
    pub(super) fn require_owned_root(&self, graph_id: &str) -> Result<(), OrchestratorError> {
        let Some(ownership) = self.ownership else {
            return Ok(());
        };
        let Some(binding) = self.store.run_loop_generation(&graph_identity(graph_id)?)? else {
            return Ok(());
        };
        if !ownership.is_base_root(&binding.root_alias)? {
            return Err(unpinned(graph_id));
        }
        Ok(())
    }

    /// Fails when `lease` resolves a different root than the run was bound under.
    pub(super) fn require_pinned_root(
        &self,
        lease: &dyn RunOwnershipLease,
        graph_id: &str,
    ) -> Result<(), OrchestratorError> {
        match self.store.run_loop_generation(&graph_identity(graph_id)?)? {
            Some(binding) => pinned(lease, &binding, graph_id),
            None => Ok(()),
        }
    }

    /// `own_artifact` under a fence the caller already holds, so a busy fence is
    /// reported before the caller records any state.
    pub(super) fn own_artifact_held(
        &mut self,
        lease: &mut dyn RunOwnershipLease,
        graph_id: &str,
        kind: ArtifactKind,
        leaf: &str,
        attempt_id: Option<AttemptId>,
    ) -> Result<(), OrchestratorError> {
        let graph = graph_identity(graph_id)?;
        let Some(binding) = self.store.run_loop_generation(&graph)? else {
            return Ok(());
        };
        let relative_path = RelativePath::new(format!("{}/{leaf}", binding.relative_path.as_str()))
            .map_err(|_| OrchestratorError::Store(StoreError::PlanRevisionMismatch))?;
        pinned(lease, &binding, graph_id)?;
        let record = ArtifactRecord {
            id: lease.new_id()?,
            generation_id: binding.id,
            kind,
            root_alias: binding.root_alias,
            relative_path,
            attempt_id,
        };
        self.store
            .register_run_loop_artifact(lease.fence(), &record)?;
        Ok(())
    }
}
