//! Pure cleanup preview policy. Eligibility never deletes; `apply` stays false.
//! Facts are caller-supplied. Identical facts share one digest.

use serde::{Deserialize, Serialize};

use crate::digest::sha256_hex;
use crate::workspace::{
    BlockerCode, DeliveryReceipt, GitRef, InventoryClass, InventoryEntry, LifecycleId, ObjectId,
    OwnershipCode,
};

/// Safety bits observed for one inventory entry. No path is opened here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupCandidateFacts {
    pub inventory: InventoryEntry,
    pub dirty_tracked: bool,
    pub dirty_untracked: bool,
    pub unknown_ignored: bool,
    pub commit: Option<ObjectId>,
    pub commit_reachable: bool,
    /// Set when a commit may exist but reachability was not proven.
    pub commit_unproven: bool,
    pub active_process: bool,
    pub path_reparse: bool,
    pub path_symlink: bool,
    pub path_escape: bool,
    pub path_outside_repository: bool,
    pub missing_dependency: bool,
    pub missing_recovery_object: bool,
    pub receipt: Option<DeliveryReceipt>,
}

/// Current facts the digest binds. The bound digest is not part of the facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupFacts {
    pub repository_id: LifecycleId,
    pub retained_ref: GitRef,
    pub retained_commit: Option<ObjectId>,
    pub candidates: Vec<CleanupCandidateFacts>,
}

/// Read-only verdict. `eligible` does not authorize removal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupPreview {
    pub apply: bool,
    pub digest: String,
    pub candidates: Vec<CleanupCandidatePreview>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupCandidatePreview {
    pub lifecycle_id: Option<LifecycleId>,
    pub source: crate::workspace::EntrySource,
    pub location: Option<crate::workspace::InventoryLocation>,
    pub ownership: OwnershipCode,
    pub status: CleanupEligibility,
    pub reasons: Vec<CleanupReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CleanupEligibility {
    Blocked,
    Eligible,
}

impl CleanupEligibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Blocked => "blocked",
            Self::Eligible => "eligible",
        }
    }
}

/// Stable cleanup blockers. Sorted by `as_str` in every preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CleanupReason {
    ActiveProcess,
    AmbiguousOwnership,
    DirtyTracked,
    DirtyUntracked,
    MissingDependency,
    MissingRecoveryObject,
    StaleFacts,
    UniqueCommit,
    UnknownIgnored,
    UnsafePath,
}

impl CleanupReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ActiveProcess => "active-process",
            Self::AmbiguousOwnership => "ambiguous-ownership",
            Self::DirtyTracked => "dirty-tracked",
            Self::DirtyUntracked => "dirty-untracked",
            Self::MissingDependency => "missing-dependency",
            Self::MissingRecoveryObject => "missing-recovery-object",
            Self::StaleFacts => "stale-facts",
            Self::UniqueCommit => "unique-commit",
            Self::UnknownIgnored => "unknown-ignored",
            Self::UnsafePath => "unsafe-path",
        }
    }
}

/// Judge `facts` against an optional previously issued digest.
/// A mismatch blocks every candidate. Nothing is removed.
pub fn cleanup_policy(facts: &CleanupFacts, bound_digest: Option<&str>) -> CleanupPreview {
    let digest = digest_of(facts);
    let stale = bound_digest.is_some_and(|bound| bound != digest);
    let mut ranked: Vec<&CleanupCandidateFacts> = facts.candidates.iter().collect();
    ranked.sort_by_key(|candidate| candidate_block(candidate));
    let candidates = ranked
        .into_iter()
        .map(|candidate| candidate_preview(candidate, stale))
        .collect();
    CleanupPreview {
        apply: false,
        digest,
        candidates,
    }
}

fn candidate_preview(candidate: &CleanupCandidateFacts, stale: bool) -> CleanupCandidatePreview {
    let reasons = reasons_for(candidate, stale);
    let status = if reasons.is_empty() {
        CleanupEligibility::Eligible
    } else {
        CleanupEligibility::Blocked
    };
    CleanupCandidatePreview {
        lifecycle_id: candidate.inventory.lifecycle_id.clone(),
        source: candidate.inventory.source,
        location: candidate.inventory.location.clone(),
        ownership: candidate.inventory.ownership,
        status,
        reasons,
    }
}

fn reasons_for(candidate: &CleanupCandidateFacts, stale: bool) -> Vec<CleanupReason> {
    let mut reasons = Vec::new();
    if candidate.dirty_tracked {
        push(&mut reasons, CleanupReason::DirtyTracked);
    }
    if candidate.dirty_untracked {
        push(&mut reasons, CleanupReason::DirtyUntracked);
    }
    if candidate.unknown_ignored {
        push(&mut reasons, CleanupReason::UnknownIgnored);
    }
    if unique_commit(candidate) {
        push(&mut reasons, CleanupReason::UniqueCommit);
    }
    if candidate.active_process || has(&candidate.inventory, BlockerCode::DriverActive) {
        push(&mut reasons, CleanupReason::ActiveProcess);
    }
    if unsafe_path(candidate) {
        push(&mut reasons, CleanupReason::UnsafePath);
    }
    if ambiguous(&candidate.inventory) {
        push(&mut reasons, CleanupReason::AmbiguousOwnership);
    }
    if missing_dependency(candidate) {
        push(&mut reasons, CleanupReason::MissingDependency);
    }
    if candidate.missing_recovery_object {
        push(&mut reasons, CleanupReason::MissingRecoveryObject);
    }
    if stale {
        push(&mut reasons, CleanupReason::StaleFacts);
    }
    reasons.sort_by_key(|reason| reason.as_str());
    reasons.dedup();
    reasons
}

fn unique_commit(candidate: &CleanupCandidateFacts) -> bool {
    candidate.commit_unproven || (candidate.commit.is_some() && !candidate.commit_reachable)
}

fn unsafe_path(candidate: &CleanupCandidateFacts) -> bool {
    candidate.path_reparse
        || candidate.path_symlink
        || candidate.path_escape
        || candidate.path_outside_repository
        || candidate.inventory.withheld
        || candidate.inventory.class == InventoryClass::Inaccessible
        || has(&candidate.inventory, BlockerCode::ReparseBlocked)
        || has(&candidate.inventory, BlockerCode::ExternalPathWithheld)
        || has(&candidate.inventory, BlockerCode::PathInaccessible)
}

fn ambiguous(entry: &InventoryEntry) -> bool {
    entry.ownership != OwnershipCode::LedgerOwned
        || has(entry, BlockerCode::UnknownOwnership)
        || has(entry, BlockerCode::LegacyOwnershipUnresolved)
        || has(entry, BlockerCode::IdentityConflict)
        || has(entry, BlockerCode::RepositoryUnbound)
}

fn missing_dependency(candidate: &CleanupCandidateFacts) -> bool {
    candidate.missing_dependency
        || candidate.inventory.class == InventoryClass::Missing
        || has(&candidate.inventory, BlockerCode::PathMissing)
}

fn has(entry: &InventoryEntry, code: BlockerCode) -> bool {
    entry.blockers.contains(&code)
}

fn push(reasons: &mut Vec<CleanupReason>, reason: CleanupReason) {
    if !reasons.contains(&reason) {
        reasons.push(reason);
    }
}

fn digest_of(facts: &CleanupFacts) -> String {
    let mut material = String::from("cleanup-preview-v1\n");
    material.push_str("repository=");
    material.push_str(facts.repository_id.as_str());
    material.push('\n');
    material.push_str("retained-ref=");
    material.push_str(facts.retained_ref.as_str());
    material.push('\n');
    material.push_str("retained-commit=");
    material.push_str(object_token(facts.retained_commit.as_ref()));
    material.push('\n');
    let mut blocks: Vec<String> = facts.candidates.iter().map(candidate_block).collect();
    blocks.sort();
    for block in blocks {
        material.push_str(&block);
        material.push('\n');
    }
    sha256_hex(material.as_bytes())
}

fn candidate_block(candidate: &CleanupCandidateFacts) -> String {
    let entry = &candidate.inventory;
    let location = entry.location.as_ref();
    let mut blockers: Vec<&str> = entry.blockers.iter().map(|code| code.as_str()).collect();
    blockers.sort_unstable();
    format!(
        "id={}\nsource={}\ngraph={}\nscope={}\nalias={}\npath={}\nclass={}\nownership={}\nwithheld={}\nblockers={}\ndirty-tracked={}\ndirty-untracked={}\nunknown-ignored={}\ncommit={}\nreachable={}\nunproven={}\nactive={}\nreparse={}\nsymlink={}\nescape={}\noutside={}\nmissing-dependency={}\nmissing-recovery={}\nreceipt={}",
        id_token(entry.lifecycle_id.as_ref()),
        entry.source.as_str(),
        entry
            .graph_id
            .as_ref()
            .map(crate::workspace::GraphId::as_str)
            .unwrap_or("-"),
        location.map(|item| item.scope.as_str()).unwrap_or("-"),
        location
            .and_then(|item| item.root_alias.as_ref())
            .map(crate::workspace::RootAlias::as_str)
            .unwrap_or("-"),
        location
            .map(|item| item.relative_path.as_str())
            .unwrap_or("-"),
        entry.class.as_str(),
        entry.ownership.as_str(),
        bit(entry.withheld),
        blockers.join(","),
        bit(candidate.dirty_tracked),
        bit(candidate.dirty_untracked),
        bit(candidate.unknown_ignored),
        object_token(candidate.commit.as_ref()),
        bit(candidate.commit_reachable),
        bit(candidate.commit_unproven),
        bit(candidate.active_process),
        bit(candidate.path_reparse),
        bit(candidate.path_symlink),
        bit(candidate.path_escape),
        bit(candidate.path_outside_repository),
        bit(candidate.missing_dependency),
        bit(candidate.missing_recovery_object),
        receipt_token(candidate.receipt.as_ref()),
    )
}

fn receipt_token(receipt: Option<&DeliveryReceipt>) -> String {
    let Some(receipt) = receipt else {
        return "-".to_string();
    };
    let mut objects: Vec<&str> = receipt
        .recovery_objects
        .iter()
        .map(ObjectId::as_str)
        .collect();
    objects.sort_unstable();
    format!(
        "{}|{}|{}|{}|{}|{}",
        receipt.strategy.as_str(),
        receipt.accepted_commit.as_str(),
        receipt.destination_ref.as_str(),
        object_token(receipt.recorded_destination.as_ref()),
        object_token(receipt.accepted_tree.as_ref()),
        objects.join(","),
    )
}

fn object_token(object: Option<&ObjectId>) -> &str {
    object.map(ObjectId::as_str).unwrap_or("-")
}

fn id_token(id: Option<&LifecycleId>) -> &str {
    id.map(LifecycleId::as_str).unwrap_or("-")
}

fn bit(value: bool) -> u8 {
    u8::from(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{
        EntrySource, GraphId, InventoryLocation, LandingStrategy, LocationScope, RelativePath,
        RootAlias,
    };

    fn oid(fill: char) -> ObjectId {
        ObjectId::new(fill.to_string().repeat(40)).unwrap()
    }

    fn entry(path: &str) -> InventoryEntry {
        InventoryEntry {
            class: InventoryClass::Active,
            ownership: OwnershipCode::LedgerOwned,
            blockers: Vec::new(),
            source: EntrySource::Generation,
            graph_id: Some(GraphId::new("graph-1").unwrap()),
            lifecycle_id: Some(LifecycleId::new("generation-1").unwrap()),
            location: Some(InventoryLocation {
                scope: LocationScope::LedgerRoot,
                root_alias: Some(RootAlias::new("worktrees").unwrap()),
                relative_path: RelativePath::new(path).unwrap(),
            }),
            withheld: false,
        }
    }

    fn candidate() -> CleanupCandidateFacts {
        CleanupCandidateFacts {
            inventory: entry("run/generation-1"),
            dirty_tracked: false,
            dirty_untracked: false,
            unknown_ignored: false,
            commit: Some(oid('a')),
            commit_reachable: true,
            commit_unproven: false,
            active_process: false,
            path_reparse: false,
            path_symlink: false,
            path_escape: false,
            path_outside_repository: false,
            missing_dependency: false,
            missing_recovery_object: false,
            receipt: None,
        }
    }

    fn facts() -> CleanupFacts {
        CleanupFacts {
            repository_id: LifecycleId::new("repository-1").unwrap(),
            retained_ref: GitRef::new("refs/heads/main").unwrap(),
            retained_commit: Some(oid('b')),
            candidates: vec![candidate()],
        }
    }

    fn codes(preview: &CleanupPreview) -> Vec<&'static str> {
        preview
            .candidates
            .iter()
            .flat_map(|candidate| candidate.reasons.iter().map(|reason| reason.as_str()))
            .collect()
    }

    #[test]
    fn cleanup_policy_blocks_adversarial_facts_and_binds_the_digest() {
        for (value, code) in [
            (CleanupReason::ActiveProcess, "active-process"),
            (CleanupReason::AmbiguousOwnership, "ambiguous-ownership"),
            (CleanupReason::DirtyTracked, "dirty-tracked"),
            (CleanupReason::DirtyUntracked, "dirty-untracked"),
            (CleanupReason::MissingDependency, "missing-dependency"),
            (
                CleanupReason::MissingRecoveryObject,
                "missing-recovery-object",
            ),
            (CleanupReason::StaleFacts, "stale-facts"),
            (CleanupReason::UniqueCommit, "unique-commit"),
            (CleanupReason::UnknownIgnored, "unknown-ignored"),
            (CleanupReason::UnsafePath, "unsafe-path"),
        ] {
            assert_eq!(value.as_str(), code);
            assert_eq!(
                serde_json::to_value(value).unwrap(),
                serde_json::json!(code)
            );
        }

        let baseline = facts();
        let fresh = cleanup_policy(&baseline, None);
        assert!(!fresh.apply);
        assert_eq!(fresh.digest.len(), 64);
        assert_eq!(fresh.candidates.len(), 1);
        assert_eq!(fresh.candidates[0].status, CleanupEligibility::Eligible);
        assert!(fresh.candidates[0].reasons.is_empty());
        assert_eq!(cleanup_policy(&baseline, None).digest, fresh.digest);
        assert_eq!(
            cleanup_policy(&baseline, Some(&fresh.digest)).candidates[0].status,
            CleanupEligibility::Eligible
        );
        let repeated = cleanup_policy(&baseline.clone(), Some(fresh.digest.as_str()));
        assert_eq!(
            repeated,
            cleanup_policy(&baseline, Some(fresh.digest.as_str()))
        );

        let mut adversarial: Vec<(&str, CleanupFacts, Option<&str>)> = Vec::new();
        let mut dirty_tracked = baseline.clone();
        dirty_tracked.candidates[0].dirty_tracked = true;
        adversarial.push(("dirty-tracked", dirty_tracked, None));
        let mut dirty_untracked = baseline.clone();
        dirty_untracked.candidates[0].dirty_untracked = true;
        adversarial.push(("dirty-untracked", dirty_untracked, None));
        let mut unknown_ignored = baseline.clone();
        unknown_ignored.candidates[0].unknown_ignored = true;
        adversarial.push(("unknown-ignored", unknown_ignored, None));
        let mut unique = baseline.clone();
        unique.candidates[0].commit_reachable = false;
        adversarial.push(("unique-commit", unique, None));
        let mut active = baseline.clone();
        active.candidates[0].active_process = true;
        adversarial.push(("active-process", active, None));
        let mut reparse = baseline.clone();
        reparse.candidates[0].path_reparse = true;
        adversarial.push(("unsafe-path", reparse, None));
        let mut symlink = baseline.clone();
        symlink.candidates[0].path_symlink = true;
        adversarial.push(("unsafe-path", symlink, None));
        let mut escape = baseline.clone();
        escape.candidates[0].path_escape = true;
        adversarial.push(("unsafe-path", escape, None));
        let mut outside = baseline.clone();
        outside.candidates[0].path_outside_repository = true;
        adversarial.push(("unsafe-path", outside, None));
        let mut ownership = baseline.clone();
        ownership.candidates[0].inventory.ownership = OwnershipCode::Unresolved;
        adversarial.push(("ambiguous-ownership", ownership, None));
        let mut dependency = baseline.clone();
        dependency.candidates[0].missing_dependency = true;
        adversarial.push(("missing-dependency", dependency, None));
        let mut recovery = baseline.clone();
        recovery.candidates[0].missing_recovery_object = true;
        recovery.candidates[0].receipt = Some(DeliveryReceipt {
            strategy: LandingStrategy::FastForward,
            accepted_commit: oid('c'),
            destination_ref: GitRef::new("refs/heads/main").unwrap(),
            recorded_destination: Some(oid('b')),
            accepted_tree: None,
            recovery_objects: vec![oid('d')],
        });
        adversarial.push(("missing-recovery-object", recovery, None));
        adversarial.push(("stale-facts", baseline.clone(), Some("0")));

        assert_eq!(adversarial.len(), 13, "adversarial cleanup cases");
        for (reason, sample, bound) in &adversarial {
            let preview = cleanup_policy(sample, *bound);
            assert!(!preview.apply, "{reason}");
            assert_eq!(preview.candidates[0].status, CleanupEligibility::Blocked);
            assert_eq!(codes(&preview), vec![*reason], "{reason}");
            assert_ne!(preview.digest, "0");
        }

        let stale = cleanup_policy(&baseline, Some(&"f".repeat(64)));
        assert_eq!(stale.digest, fresh.digest);
        assert_eq!(codes(&stale), vec!["stale-facts"]);
        assert!(
            !serde_json::to_string(&stale)
                .unwrap()
                .contains("\"apply\":true")
        );

        let mut path_changed = baseline.clone();
        path_changed.candidates[0]
            .inventory
            .location
            .as_mut()
            .unwrap()
            .relative_path = RelativePath::new("run/generation-2").unwrap();
        let mut commit_changed = baseline.clone();
        commit_changed.candidates[0].commit = Some(oid('e'));
        let mut retained_changed = baseline.clone();
        retained_changed.retained_commit = Some(oid('f'));
        let mut ref_changed = baseline.clone();
        ref_changed.retained_ref = GitRef::new("refs/heads/kept").unwrap();
        for changed in [
            path_changed,
            dirty_variant(&baseline),
            commit_changed,
            liveness_variant(&baseline),
            ownership_variant(&baseline),
            retained_changed,
            ref_changed,
        ] {
            assert_ne!(cleanup_policy(&changed, None).digest, fresh.digest);
        }

        let mut second = candidate();
        second.inventory.lifecycle_id = Some(LifecycleId::new("generation-2").unwrap());
        second.inventory.location.as_mut().unwrap().relative_path =
            RelativePath::new("run/generation-2").unwrap();
        second.dirty_tracked = true;
        let mut paired = baseline.clone();
        paired.candidates.push(second);
        let mut flipped = paired.clone();
        flipped.candidates.reverse();
        let forward = cleanup_policy(&paired, None);
        let backward = cleanup_policy(&flipped, None);
        assert_eq!(forward.digest, backward.digest);
        assert_eq!(forward.candidates.len(), 2);
        assert!(
            forward
                .candidates
                .iter()
                .any(|item| item.status == CleanupEligibility::Eligible)
        );
        assert!(
            forward
                .candidates
                .iter()
                .any(|item| item.reasons == vec![CleanupReason::DirtyTracked])
        );
        let both_stale = cleanup_policy(&paired, Some("stale"));
        assert!(both_stale.candidates.iter().all(|item| {
            item.status == CleanupEligibility::Blocked
                && item.reasons.contains(&CleanupReason::StaleFacts)
        }));
        assert!(!both_stale.apply);

        let mut driver = baseline.clone();
        driver.candidates[0]
            .inventory
            .blockers
            .push(BlockerCode::DriverActive);
        assert_eq!(
            codes(&cleanup_policy(&driver, None)),
            vec!["active-process"]
        );
        assert_ne!(cleanup_policy(&driver, None).digest, fresh.digest);

        let mut receipt_changed = baseline.clone();
        receipt_changed.candidates[0].receipt = Some(DeliveryReceipt {
            strategy: LandingStrategy::Merge,
            accepted_commit: oid('c'),
            destination_ref: GitRef::new("refs/heads/main").unwrap(),
            recorded_destination: None,
            accepted_tree: None,
            recovery_objects: vec![oid('d'), oid('a')],
        });
        let with_receipt = cleanup_policy(&receipt_changed, None);
        assert_eq!(
            with_receipt.candidates[0].status,
            CleanupEligibility::Eligible
        );
        assert_ne!(with_receipt.digest, fresh.digest);
        let mut moved_object = receipt_changed.clone();
        moved_object.candidates[0]
            .receipt
            .as_mut()
            .unwrap()
            .recovery_objects = vec![oid('e')];
        assert_ne!(
            cleanup_policy(&moved_object, None).digest,
            with_receipt.digest
        );
    }

    fn dirty_variant(facts: &CleanupFacts) -> CleanupFacts {
        let mut changed = facts.clone();
        changed.candidates[0].dirty_untracked = true;
        changed
    }

    fn liveness_variant(facts: &CleanupFacts) -> CleanupFacts {
        let mut changed = facts.clone();
        changed.candidates[0].active_process = true;
        changed
    }

    fn ownership_variant(facts: &CleanupFacts) -> CleanupFacts {
        let mut changed = facts.clone();
        changed.candidates[0].inventory.ownership = OwnershipCode::GitRegistered;
        changed
    }
}
