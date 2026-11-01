//! Read-only delivery receipts. Task status is not an input.
//! A pass does not close tasks, publish, delete recovery objects, or clear a ledger.
//! Reachability uses non-pruning git reads supplied by the caller.

use std::path::Path;

use meshloop_domain::workspace::{
    DeliveryReason, DeliveryReceipt, DeliveryReceiptReport, LandingStrategy, ObjectId,
};

/// Git read failure. Callers must fail closed; the detail is not a reason code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryGitError;

/// Structured git reads. Implementations must not prune, rewrite, or delete.
pub trait DeliveryGit {
    fn resolve_commit(&self, git_ref: &str) -> Result<String, DeliveryGitError>;
    fn is_ancestor(&self, ancestor: &str, descendant: &str) -> Result<bool, DeliveryGitError>;
    fn parents(&self, commit: &str) -> Result<Vec<String>, DeliveryGitError>;
    fn tree_reachable(&self, tree: &str, from_commit: &str) -> Result<bool, DeliveryGitError>;
    fn object_exists(&self, object: &str) -> Result<bool, DeliveryGitError>;
    fn planner_dirty(&self, worktree: &Path) -> Result<bool, DeliveryGitError>;
}

/// Prove `receipt` against current git state. `apply` is always false.
pub fn prove_delivery_receipt(
    git: &impl DeliveryGit,
    receipt: &DeliveryReceipt,
    planner_worktree: &Path,
) -> DeliveryReceiptReport {
    let mut reasons = Vec::new();
    let observed = match git.resolve_commit(receipt.destination_ref.as_str()) {
        Ok(oid) => match ObjectId::new(oid) {
            Ok(oid) => Some(oid),
            Err(_) => {
                push(&mut reasons, DeliveryReason::UnreadableRef);
                None
            }
        },
        Err(_) => {
            push(&mut reasons, DeliveryReason::UnreadableRef);
            None
        }
    };
    let recorded_mismatch = match (&receipt.recorded_destination, &observed) {
        (Some(recorded), Some(tip)) => recorded != tip,
        _ => false,
    };
    if recorded_mismatch {
        push(&mut reasons, DeliveryReason::StaleDestination);
    }
    if let Some(tip) = &observed {
        match receipt.strategy {
            LandingStrategy::FastForward => {
                note_fast_forward(git, receipt, tip.as_str(), recorded_mismatch, &mut reasons);
            }
            LandingStrategy::Merge => {
                note_merge(git, receipt, tip.as_str(), recorded_mismatch, &mut reasons);
            }
            LandingStrategy::Rewrite => note_rewrite(git, receipt, &mut reasons),
        }
    }
    note_recovery(git, receipt, &mut reasons);
    match git.planner_dirty(planner_worktree) {
        Ok(true) => push(&mut reasons, DeliveryReason::PlannerDirty),
        Ok(false) => {}
        Err(_) => push(&mut reasons, DeliveryReason::UnreadableRef),
    }
    finish(&mut reasons);
    DeliveryReceiptReport {
        passed: reasons.is_empty(),
        apply: false,
        reasons,
        observed_destination: observed,
    }
}

fn note_fast_forward(
    git: &impl DeliveryGit,
    receipt: &DeliveryReceipt,
    tip: &str,
    recorded_mismatch: bool,
    reasons: &mut Vec<DeliveryReason>,
) {
    match git.is_ancestor(receipt.accepted_commit.as_str(), tip) {
        Ok(true) => {}
        Ok(false) => push(
            reasons,
            if recorded_mismatch {
                DeliveryReason::RewrittenDestination
            } else {
                DeliveryReason::AcceptedNotLanded
            },
        ),
        Err(_) => push(reasons, DeliveryReason::UnreadableRef),
    }
}

fn note_merge(
    git: &impl DeliveryGit,
    receipt: &DeliveryReceipt,
    tip: &str,
    recorded_mismatch: bool,
    reasons: &mut Vec<DeliveryReason>,
) {
    let parents = match git.parents(tip) {
        Ok(parents) => parents,
        Err(_) => {
            push(reasons, DeliveryReason::UnreadableRef);
            return;
        }
    };
    let proved = parents.len() >= 2
        && parents
            .iter()
            .any(|parent| parent == receipt.accepted_commit.as_str());
    if proved {
        return;
    }
    if !recorded_mismatch {
        push(reasons, DeliveryReason::AcceptedNotLanded);
        return;
    }
    match git.is_ancestor(receipt.accepted_commit.as_str(), tip) {
        Ok(true) => {}
        Ok(false) => push(reasons, DeliveryReason::RewrittenDestination),
        Err(_) => push(reasons, DeliveryReason::UnreadableRef),
    }
}

fn note_rewrite(
    git: &impl DeliveryGit,
    receipt: &DeliveryReceipt,
    reasons: &mut Vec<DeliveryReason>,
) {
    let Some(recorded) = &receipt.recorded_destination else {
        push(reasons, DeliveryReason::RewrittenDestination);
        return;
    };
    match contained(git, receipt, recorded.as_str()) {
        Ok(true) => {}
        Ok(false) => push(reasons, DeliveryReason::RewrittenDestination),
        Err(_) => push(reasons, DeliveryReason::UnreadableRef),
    }
}

/// Accepted commit, or its tree when a rewrite changed the commit id.
fn contained(
    git: &impl DeliveryGit,
    receipt: &DeliveryReceipt,
    from_commit: &str,
) -> Result<bool, DeliveryGitError> {
    if git.is_ancestor(receipt.accepted_commit.as_str(), from_commit)? {
        return Ok(true);
    }
    match &receipt.accepted_tree {
        Some(tree) => git.tree_reachable(tree.as_str(), from_commit),
        None => Ok(false),
    }
}

fn note_recovery(
    git: &impl DeliveryGit,
    receipt: &DeliveryReceipt,
    reasons: &mut Vec<DeliveryReason>,
) {
    let mut missing = false;
    for object in &receipt.recovery_objects {
        match git.object_exists(object.as_str()) {
            Ok(true) => {}
            Ok(false) => missing = true,
            Err(_) => push(reasons, DeliveryReason::UnreadableRef),
        }
    }
    if missing {
        push(reasons, DeliveryReason::MissingRecoveryObject);
    }
}

fn push(reasons: &mut Vec<DeliveryReason>, reason: DeliveryReason) {
    if !reasons.contains(&reason) {
        reasons.push(reason);
    }
}

fn finish(reasons: &mut Vec<DeliveryReason>) {
    reasons.sort_by_key(|reason| reason.as_str());
    reasons.dedup();
}

#[cfg(test)]
mod tests {
    use super::*;
    use meshloop_domain::workspace::GitRef;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    const TIP: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const OLD: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const ACCEPTED: &str = "cccccccccccccccccccccccccccccccccccccccc";
    const TREE: &str = "dddddddddddddddddddddddddddddddddddddddd";
    const BASE: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
    const KEPT: &str = "ffffffffffffffffffffffffffffffffffffffff";
    const MISSING: &str = "1111111111111111111111111111111111111111";

    struct Script {
        tip: Result<String, DeliveryGitError>,
        ancestor: Result<bool, DeliveryGitError>,
        parents: Result<Vec<String>, DeliveryGitError>,
        tree: Result<bool, DeliveryGitError>,
        present: Vec<String>,
        object_error: bool,
        dirty: Result<bool, DeliveryGitError>,
    }

    impl DeliveryGit for Script {
        fn resolve_commit(&self, git_ref: &str) -> Result<String, DeliveryGitError> {
            GitRef::new(git_ref).map_err(|_| DeliveryGitError)?;
            self.tip.clone()
        }
        fn is_ancestor(&self, _: &str, _: &str) -> Result<bool, DeliveryGitError> {
            self.ancestor.clone()
        }
        fn parents(&self, _: &str) -> Result<Vec<String>, DeliveryGitError> {
            self.parents.clone()
        }
        fn tree_reachable(&self, _: &str, _: &str) -> Result<bool, DeliveryGitError> {
            self.tree.clone()
        }
        fn object_exists(&self, object: &str) -> Result<bool, DeliveryGitError> {
            if self.object_error {
                return Err(DeliveryGitError);
            }
            Ok(self.present.iter().any(|oid| oid == object))
        }
        fn planner_dirty(&self, _: &Path) -> Result<bool, DeliveryGitError> {
            self.dirty.clone()
        }
    }

    fn oid(value: &str) -> ObjectId {
        ObjectId::new(value).unwrap()
    }

    fn script() -> Script {
        Script {
            tip: Ok(TIP.to_string()),
            ancestor: Ok(true),
            parents: Ok(vec![BASE.to_string(), ACCEPTED.to_string()]),
            tree: Ok(false),
            present: vec![KEPT.to_string()],
            object_error: false,
            dirty: Ok(false),
        }
    }

    fn receipt(strategy: LandingStrategy) -> DeliveryReceipt {
        DeliveryReceipt {
            strategy,
            accepted_commit: oid(ACCEPTED),
            destination_ref: GitRef::new("refs/heads/main").unwrap(),
            recorded_destination: Some(oid(TIP)),
            accepted_tree: None,
            recovery_objects: Vec::new(),
        }
    }

    fn codes(report: &DeliveryReceiptReport) -> Vec<&'static str> {
        report
            .reasons
            .iter()
            .map(|reason| reason.as_str())
            .collect()
    }

    fn prove(git: &Script, receipt: &DeliveryReceipt) -> DeliveryReceiptReport {
        let report = prove_delivery_receipt(git, receipt, Path::new("."));
        assert!(!report.apply);
        report
    }

    #[test]
    fn delivery_receipt_fast_forward_requires_ancestry() {
        let report = prove(&script(), &receipt(LandingStrategy::FastForward));
        assert!(report.passed, "{report:?}");
        assert_eq!(codes(&report), Vec::<&str>::new());
        assert_eq!(
            report.observed_destination.as_ref().map(ObjectId::as_str),
            Some(TIP)
        );

        let mut behind = script();
        behind.ancestor = Ok(false);
        let report = prove(&behind, &receipt(LandingStrategy::FastForward));
        assert!(!report.passed);
        assert_eq!(codes(&report), ["accepted-not-landed"]);
    }

    #[test]
    fn delivery_receipt_merge_requires_a_parent() {
        let report = prove(&script(), &receipt(LandingStrategy::Merge));
        assert!(report.passed, "{report:?}");

        let mut fast_forward = script();
        fast_forward.parents = Ok(vec![BASE.to_string()]);
        fast_forward.ancestor = Ok(true);
        let report = prove(&fast_forward, &receipt(LandingStrategy::Merge));
        assert!(!report.passed);
        assert_eq!(codes(&report), ["accepted-not-landed"]);
    }

    #[test]
    fn delivery_receipt_rewrite_reachable_tree_passes() {
        let mut git = script();
        git.ancestor = Ok(false);
        git.tree = Ok(true);
        let mut claim = receipt(LandingStrategy::Rewrite);
        claim.accepted_tree = Some(oid(TREE));
        let report = prove(&git, &claim);
        assert!(report.passed, "{report:?}");

        git.tree = Ok(false);
        let report = prove(&git, &claim);
        assert!(!report.passed);
        assert_eq!(codes(&report), ["rewritten-destination"]);

        claim.recorded_destination = None;
        git.ancestor = Ok(true);
        let report = prove(&git, &claim);
        assert!(!report.passed);
        assert_eq!(codes(&report), ["rewritten-destination"]);
    }

    #[test]
    fn delivery_receipt_stale_target_fails_closed() {
        let mut claim = receipt(LandingStrategy::FastForward);
        claim.recorded_destination = Some(oid(OLD));
        let report = prove(&script(), &claim);
        assert!(!report.passed);
        assert_eq!(codes(&report), ["stale-destination"]);
    }

    #[test]
    fn delivery_receipt_accepted_only_is_not_landed() {
        // Task acceptance is not an argument. A commit that exists only off
        // the destination ref does not pass.
        let mut git = script();
        git.ancestor = Ok(false);
        let report = prove(&git, &receipt(LandingStrategy::FastForward));
        assert!(!report.passed);
        assert_eq!(codes(&report), ["accepted-not-landed"]);
    }

    #[test]
    fn delivery_receipt_planner_dirt_is_not_a_landing() {
        let path = std::env::temp_dir().join(format!(
            "meshloop-receipt-dirt-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&path, b"planner-dirt-stays").unwrap();
        let mut git = script();
        git.dirty = Ok(true);
        let report = prove_delivery_receipt(&git, &receipt(LandingStrategy::FastForward), &path);
        assert!(!report.apply);
        assert!(!report.passed);
        assert_eq!(codes(&report), ["planner-dirty"]);
        assert_eq!(fs::read(&path).unwrap(), b"planner-dirt-stays");
        fs::remove_file(&path).ok();
    }

    #[test]
    fn delivery_receipt_missing_recovery_and_unreadable_ref_fail_closed() {
        let mut claim = receipt(LandingStrategy::FastForward);
        claim.recovery_objects = vec![oid(KEPT), oid(MISSING)];
        let report = prove(&script(), &claim);
        assert!(!report.passed);
        assert_eq!(codes(&report), ["missing-recovery-object"]);

        let mut unreadable = script();
        unreadable.tip = Err(DeliveryGitError);
        let report = prove(&unreadable, &receipt(LandingStrategy::Merge));
        assert!(!report.passed);
        assert_eq!(codes(&report), ["unreadable-ref"]);
        assert!(report.observed_destination.is_none());
    }

    #[test]
    fn delivery_receipt_replaced_history_without_the_commit_is_rewritten() {
        let mut git = script();
        git.tip = Ok(OLD.to_string());
        git.ancestor = Ok(false);
        let report = prove(&git, &receipt(LandingStrategy::FastForward));
        assert!(!report.passed);
        assert_eq!(
            codes(&report),
            ["rewritten-destination", "stale-destination"]
        );
    }
}
