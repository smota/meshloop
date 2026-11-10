//! Run-loop ownership (issue #142, ADR 0033 amendment of 2026-11-10). A run this
//! version creates registers its repository fence, a binding generation
//! (`Producer::RunLoop`, base branch, accepted plan hash) and one artifact per
//! worktree before that worktree is added, so cleanup can verify ownership later.
//! A re-accepted plan or a restart adds a generation pinned to the new plan hash.
//! Records are never deleted: a worktree that is gone is retired with a fact.
//! Runs that had worktrees before this version keep their legacy state.

use std::path::{Path, PathBuf};

use meshloop_domain::digest::sha256_hex;
use meshloop_domain::evidence::AttemptId;
use meshloop_domain::state::PlanState;
use meshloop_domain::workspace::{
    ArtifactKind, ArtifactRecord, Fact, GenerationRecord, GraphId, LifecycleFact, LifecycleId,
    PlanHash, Producer, RUN_LOOP_LAYOUT_VERSION, Reclamation, RelativePath, run_loop_base_ref,
};

use super::{OrchestratorError, RunLoop};
use crate::ports::StoreError;
use crate::workspace::RunOwnershipLease;

fn graph_identity(graph_id: &str) -> Result<GraphId, OrchestratorError> {
    GraphId::new(graph_id)
        .map_err(|_| OrchestratorError::Illegal(format!("invalid graph identity: {graph_id:?}")))
}

fn plan_hash(plan_json: &str) -> Result<PlanHash, OrchestratorError> {
    PlanHash::new(sha256_hex(plan_json.as_bytes()))
        .map_err(|_| OrchestratorError::Store(StoreError::PlanRevisionMismatch))
}

fn unpinned(graph_id: &str) -> OrchestratorError {
    OrchestratorError::Illegal(format!(
        "run '{graph_id}' is pinned to its registered worktree root; \
         use it without changing --worktree-base or [workspace] root"
    ))
}

/// A path that exists in any form, including a dangling symlink.
fn occupied(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

impl RunLoop<'_> {
    /// Absolute path of a run-loop record under this run loop's worktree base.
    /// Callers check the root pin first, so the base is the registered root.
    fn record_path(&self, relative: &RelativePath) -> PathBuf {
        let mut path = self.worktree_base.clone();
        path.extend(relative.as_str().split('/'));
        path
    }

    /// True when a fresh, accepted run may be bound now: ownership is configured
    /// and nothing of the run exists yet (no binding, generation, marker, attempt,
    /// integrate worktree, branch or directory). Read-only; takes no fence.
    pub(super) fn fresh_for_ownership(&self, graph_id: &str) -> Result<bool, OrchestratorError> {
        if self.ownership.is_none() {
            return Ok(false);
        }
        let graph = graph_identity(graph_id)?;
        if self.store.run_binding(&graph)?.is_some()
            || self.store.run_loop_generation(&graph)?.is_some()
        {
            return Ok(false);
        }
        if self.store.legacy_ownership_unresolved(&graph)? {
            return Err(StoreError::LegacyOwnershipUnresolved.into());
        }
        let Some(row) = self.store.load_run(graph_id)? else {
            return Ok(false);
        };
        let integrate = self.integrate_path(graph_id);
        Ok(row.plan_state == PlanState::PlanAccepted
            && RelativePath::new(graph_id).is_ok()
            && self.store.attempts_for_graph(graph_id)?.is_empty()
            && !self.workspace.worktree_exists(&integrate)
            && !occupied(&integrate)
            && !self
                .workspace
                .branch_exists(&self.integrate_branch(graph_id))?)
    }

    /// Binds a fresh run under `lease`. The base branch is the main checkout's
    /// branch now; a detached HEAD or a Meshloop branch records none.
    pub(super) fn bind_new_run(
        &mut self,
        lease: &mut dyn RunOwnershipLease,
        graph_id: &str,
    ) -> Result<(), OrchestratorError> {
        let graph = graph_identity(graph_id)?;
        let row = self
            .store
            .load_run(graph_id)?
            .ok_or_else(|| OrchestratorError::MissingGraph(graph_id.into()))?;
        let relative_path = RelativePath::new(graph_id).map_err(|_| {
            OrchestratorError::Illegal(format!("invalid run location: {graph_id:?}"))
        })?;
        let base_ref = self
            .workspace
            .current_branch()?
            .and_then(|name| run_loop_base_ref(&name));
        let alias = lease.root_alias()?;
        let id = lease.new_id()?;
        let fence = lease.fence();
        self.store.bind_repository(fence)?;
        self.store.register_root(fence, &alias)?;
        let binding = GenerationRecord {
            id,
            repository_id: fence.repository().id.clone(),
            graph_id: graph,
            ordinal: 1,
            accepted_plan_hash: plan_hash(&row.plan_json)?,
            root_alias: alias,
            relative_path,
            layout_version: RUN_LOOP_LAYOUT_VERSION,
            producer: Some(Producer::RunLoop),
            base_ref,
        };
        self.store.register_run_loop_run(fence, &binding)?;
        Ok(())
    }

    /// True when the run loop owns `graph_id` and must register its worktrees.
    pub(super) fn run_loop_owned(&self, graph_id: &str) -> Result<bool, OrchestratorError> {
        if self.ownership.is_none() {
            return Ok(false);
        }
        Ok(self
            .store
            .run_loop_generation(&graph_identity(graph_id)?)?
            .is_some())
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
            Some(binding) if !lease.is_base_root(&binding.root_alias)? => Err(unpinned(graph_id)),
            _ => Ok(()),
        }
    }

    /// Registers the worktree `leaf` of an owned run under a fence the caller
    /// holds, before the worktree is added. Returns the artifact id, or `None` for
    /// an unowned run. The store refuses a current generation whose plan hash is
    /// not the accepted plan's.
    pub(super) fn own_artifact_held(
        &mut self,
        lease: &mut dyn RunOwnershipLease,
        graph_id: &str,
        kind: ArtifactKind,
        leaf: &str,
        attempt_id: Option<AttemptId>,
    ) -> Result<Option<LifecycleId>, OrchestratorError> {
        let graph = graph_identity(graph_id)?;
        let Some(current) = self.store.run_loop_current_generation(&graph)? else {
            return Ok(None);
        };
        self.require_pinned_root(lease, graph_id)?;
        let relative_path = RelativePath::new(format!("{}/{leaf}", current.relative_path.as_str()))
            .map_err(|_| OrchestratorError::Illegal(format!("invalid run location: {leaf:?}")))?;
        let record = ArtifactRecord {
            id: lease.new_id()?,
            generation_id: current.id,
            kind,
            root_alias: current.root_alias,
            relative_path,
            attempt_id,
        };
        Ok(Some(
            self.store
                .register_run_loop_artifact(lease.fence(), &record)?,
        ))
    }

    /// Retires artifacts whose worktree is gone. Records are kept.
    pub(super) fn retire_held(
        &mut self,
        lease: &mut dyn RunOwnershipLease,
        artifacts: &[LifecycleId],
        how: Reclamation,
    ) -> Result<(), OrchestratorError> {
        let mut facts = Vec::with_capacity(artifacts.len());
        for artifact in artifacts {
            facts.push(LifecycleFact {
                id: lease.new_id()?,
                artifact_id: artifact.clone(),
                fact: Fact::Reclamation(how),
            });
        }
        if !facts.is_empty() {
            self.store
                .retire_run_loop_artifacts(lease.fence(), &facts)?;
        }
        Ok(())
    }

    /// Retires `artifact` as never created when its worktree add failed and nothing
    /// is at `path`. Anything left there stays owned and visible to cleanup.
    pub(super) fn retire_not_created(
        &mut self,
        lease: &mut dyn RunOwnershipLease,
        artifact: Option<LifecycleId>,
        path: &Path,
    ) -> Result<(), OrchestratorError> {
        match artifact {
            Some(id) if !occupied(path) && !self.workspace.worktree_exists(path) => {
                self.retire_held(lease, &[id], Reclamation::NotCreated)
            }
            _ => Ok(()),
        }
    }

    /// Pins a re-accepted plan: when the accepted plan's hash differs from the
    /// current generation's, add the next generation. No-op for unowned runs and
    /// unaccepted plans.
    pub(super) fn advance_generation(&mut self, graph_id: &str) -> Result<(), OrchestratorError> {
        let Some(ownership) = self.ownership else {
            return Ok(());
        };
        let graph = graph_identity(graph_id)?;
        let Some(current) = self.store.run_loop_current_generation(&graph)? else {
            return Ok(());
        };
        let Some(row) = self.store.load_run(graph_id)? else {
            return Ok(());
        };
        let hash = plan_hash(&row.plan_json)?;
        if row.plan_state != PlanState::PlanAccepted || hash == current.accepted_plan_hash {
            return Ok(());
        }
        // Records a plan hash only; no worktree path is derived, so no root pin.
        let mut lease = ownership.acquire()?;
        let next = GenerationRecord {
            id: lease.new_id()?,
            ordinal: current.ordinal + 1,
            accepted_plan_hash: hash,
            ..current
        };
        self.store.register_run_loop_run(lease.fence(), &next)?;
        Ok(())
    }

    /// Restart of a run-loop run. History moves into the current generation, a
    /// new generation is pinned to the accepted plan, and attempt artifacts are
    /// retired only when their worktree is confirmed gone. Returns `false` for an
    /// unowned run, which keeps the legacy reset.
    pub(super) fn restart_owned(&mut self, graph_id: &str) -> Result<bool, OrchestratorError> {
        let Some(ownership) = self.ownership else {
            return Ok(false);
        };
        let graph = graph_identity(graph_id)?;
        let Some(current) = self.store.run_loop_current_generation(&graph)? else {
            return Ok(false);
        };
        let row = self
            .store
            .load_run(graph_id)?
            .ok_or_else(|| OrchestratorError::MissingGraph(graph_id.into()))?;
        let mut lease = ownership.acquire()?;
        self.require_pinned_root(lease.as_ref(), graph_id)?;
        let mut retire = Vec::new();
        for (artifact, retired) in self.store.run_loop_artifacts(&graph)? {
            if retired || artifact.kind != ArtifactKind::Attempt {
                continue;
            }
            let path = self.record_path(&artifact.relative_path);
            if !occupied(&path) && !self.workspace.worktree_exists(&path) {
                retire.push(LifecycleFact {
                    id: lease.new_id()?,
                    artifact_id: artifact.id,
                    fact: Fact::Reclamation(Reclamation::Reclaimed),
                });
            }
        }
        let next = GenerationRecord {
            id: lease.new_id()?,
            ordinal: current.ordinal + 1,
            accepted_plan_hash: plan_hash(&row.plan_json)?,
            ..current
        };
        self.store
            .restart_run_loop_run(lease.fence(), &retire, &next)?;
        Ok(true)
    }

    /// Recovery: retire live artifacts whose worktree neither exists nor is
    /// registered with Git (for example a crash between registration and add).
    /// Takes the fence only when something must be retired.
    pub(super) fn reconcile_owned(&mut self, graph_id: &str) -> Result<(), OrchestratorError> {
        let Some(ownership) = self.ownership else {
            return Ok(());
        };
        if !self.run_loop_owned(graph_id)? {
            return Ok(());
        }
        let graph = graph_identity(graph_id)?;
        let absent = |this: &Self| -> Result<Vec<LifecycleId>, OrchestratorError> {
            Ok(this
                .store
                .run_loop_artifacts(&graph)?
                .into_iter()
                .filter(|(artifact, retired)| {
                    let path = this.record_path(&artifact.relative_path);
                    !retired && !occupied(&path) && !this.workspace.worktree_exists(&path)
                })
                .map(|(artifact, _)| artifact.id)
                .collect())
        };
        if absent(self)?.is_empty() {
            return Ok(());
        }
        let mut lease = ownership.acquire()?;
        self.require_pinned_root(lease.as_ref(), graph_id)?;
        // Re-read under the fence: another writer may have added a worktree.
        let gone = absent(self)?;
        self.retire_held(lease.as_mut(), &gone, Reclamation::Absent)
    }
}
