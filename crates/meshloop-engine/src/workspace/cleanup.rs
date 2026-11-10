//! Join inventory entries to read-only safety observations.
//! There is no apply path and this module does not remove anything.

use meshloop_domain::cleanup::{
    CleanupCandidateFacts, CleanupFacts, CleanupPreview, RunOutcome, cleanup_policy,
};
use meshloop_domain::workspace::{DeliveryReceipt, GitRef, InventoryEntry, LifecycleId, ObjectId};

/// One inventory row's filesystem and git facts. Missing rows fail closed.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CleanupObservation {
    pub dirty_tracked: bool,
    pub dirty_untracked: bool,
    pub unknown_ignored: bool,
    pub commit: Option<ObjectId>,
    pub commit_reachable: bool,
    pub commit_unproven: bool,
    pub active_process: bool,
    pub path_reparse: bool,
    pub path_symlink: bool,
    pub path_escape: bool,
    pub path_outside_repository: bool,
    pub missing_dependency: bool,
    pub missing_recovery_object: bool,
    pub receipt: Option<DeliveryReceipt>,
    /// Outcome of the owning run-loop run. Defaults keep older journal tokens readable.
    #[serde(default)]
    pub run_outcome: Option<RunOutcome>,
    #[serde(default)]
    pub run_container: bool,
    #[serde(default)]
    pub attempt_failed: bool,
    #[serde(default)]
    pub base_unknown: bool,
}

/// Retained identity plus one observation per inventory entry, in that order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedCleanup {
    pub repository_id: LifecycleId,
    pub retained_ref: GitRef,
    pub retained_commit: Option<ObjectId>,
    pub observations: Vec<CleanupObservation>,
}

/// Build the preview. `apply` is always false. Observations shorter than
/// `entries` fail closed instead of treating the gap as eligible.
pub fn cleanup_preview(
    repository_id: LifecycleId,
    retained_ref: GitRef,
    retained_commit: Option<ObjectId>,
    entries: &[InventoryEntry],
    observations: &[CleanupObservation],
    bound_digest: Option<&str>,
) -> CleanupPreview {
    let candidates = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let observed = observations.get(index).cloned().unwrap_or_else(fail_closed);
            CleanupCandidateFacts {
                inventory: entry.clone(),
                dirty_tracked: observed.dirty_tracked,
                dirty_untracked: observed.dirty_untracked,
                unknown_ignored: observed.unknown_ignored,
                commit: observed.commit,
                commit_reachable: observed.commit_reachable,
                commit_unproven: observed.commit_unproven,
                active_process: observed.active_process,
                path_reparse: observed.path_reparse,
                path_symlink: observed.path_symlink,
                path_escape: observed.path_escape,
                path_outside_repository: observed.path_outside_repository,
                missing_dependency: observed.missing_dependency,
                missing_recovery_object: observed.missing_recovery_object,
                receipt: observed.receipt,
                run_outcome: observed.run_outcome,
                run_container: observed.run_container,
                attempt_failed: observed.attempt_failed,
                base_unknown: observed.base_unknown,
            }
        })
        .collect();
    cleanup_policy(
        &CleanupFacts {
            repository_id,
            retained_ref,
            retained_commit,
            candidates,
        },
        bound_digest,
    )
}

fn fail_closed() -> CleanupObservation {
    CleanupObservation {
        dirty_tracked: true,
        dirty_untracked: true,
        unknown_ignored: true,
        commit: None,
        commit_reachable: false,
        commit_unproven: true,
        active_process: true,
        path_reparse: false,
        path_symlink: false,
        path_escape: false,
        path_outside_repository: true,
        missing_dependency: true,
        missing_recovery_object: true,
        receipt: None,
        run_outcome: None,
        run_container: false,
        attempt_failed: false,
        base_unknown: false,
    }
}
