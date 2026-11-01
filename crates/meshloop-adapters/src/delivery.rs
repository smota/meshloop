//! Read-only git reachability for delivery receipts.
//! Uses structured `GitWorktreeAdapter` arguments. No shell, force-push, reset, or prune.

use std::path::Path;

use meshloop_engine::workspace::delivery::{DeliveryGit, DeliveryGitError};

use crate::git::{GitError, GitWorktreeAdapter};

pub struct GitDeliveryProbe<'a> {
    git: &'a GitWorktreeAdapter,
}

impl<'a> GitDeliveryProbe<'a> {
    pub fn new(git: &'a GitWorktreeAdapter) -> Self {
        Self { git }
    }
}

impl DeliveryGit for GitDeliveryProbe<'_> {
    fn resolve_commit(&self, git_ref: &str) -> Result<String, DeliveryGitError> {
        self.git.resolve_commit(git_ref).map_err(unread)
    }

    fn is_ancestor(&self, ancestor: &str, descendant: &str) -> Result<bool, DeliveryGitError> {
        self.git
            .commit_is_ancestor(ancestor, descendant)
            .map_err(unread)
    }

    fn parents(&self, commit: &str) -> Result<Vec<String>, DeliveryGitError> {
        self.git.commit_parents(commit).map_err(unread)
    }

    fn tree_reachable(&self, tree: &str, from_commit: &str) -> Result<bool, DeliveryGitError> {
        self.git
            .tree_reachable_from(tree, from_commit)
            .map_err(unread)
    }

    fn object_exists(&self, object: &str) -> Result<bool, DeliveryGitError> {
        self.git.object_exists(object).map_err(unread)
    }

    fn planner_dirty(&self, worktree: &Path) -> Result<bool, DeliveryGitError> {
        self.git.planner_is_dirty(worktree).map_err(unread)
    }
}

fn unread(_: GitError) -> DeliveryGitError {
    DeliveryGitError
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::TestCommandExt;
    use meshloop_domain::workspace::{
        DeliveryReceipt, DeliveryReceiptReport, GitRef, LandingStrategy, ObjectId,
    };
    use meshloop_engine::workspace::delivery::prove_delivery_receipt;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct Fixture {
        root: PathBuf,
        repo: PathBuf,
        ledger: PathBuf,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.root).ok();
        }
    }

    fn git(repo: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(repo)
            .args(args)
            .env("GIT_AUTHOR_NAME", "meshloop")
            .env("GIT_AUTHOR_EMAIL", "meshloop@localhost")
            .env("GIT_COMMITTER_NAME", "meshloop")
            .env("GIT_COMMITTER_EMAIL", "meshloop@localhost")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .guarded_output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn fixture() -> Fixture {
        let root = std::env::temp_dir().join(format!(
            "meshloop-delivery-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let repo = root.join("repo");
        fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.email", "test@test"]);
        git(&repo, &["config", "user.name", "test"]);
        fs::write(repo.join("README.md"), "seed").unwrap();
        git(&repo, &["add", "README.md"]);
        git(&repo, &["commit", "-q", "-m", "seed"]);
        let ledger = root.join("ledger.sqlite");
        fs::write(&ledger, b"ledger-sentinel").unwrap();
        Fixture { root, repo, ledger }
    }

    fn commit_file(repo: &Path, name: &str, body: &str, message: &str) -> String {
        fs::write(repo.join(name), body).unwrap();
        git(repo, &["add", name]);
        git(repo, &["commit", "-q", "-m", message]);
        git(repo, &["rev-parse", "HEAD"])
    }

    fn oid(value: &str) -> ObjectId {
        ObjectId::new(value).unwrap()
    }

    fn receipt(
        strategy: LandingStrategy,
        accepted: &str,
        recorded: Option<&str>,
        tree: Option<&str>,
        recovery: &[&str],
    ) -> DeliveryReceipt {
        DeliveryReceipt {
            strategy,
            accepted_commit: oid(accepted),
            destination_ref: GitRef::new("refs/heads/main").unwrap(),
            recorded_destination: recorded.map(oid),
            accepted_tree: tree.map(oid),
            recovery_objects: recovery.iter().copied().map(oid).collect(),
        }
    }

    fn codes(report: &DeliveryReceiptReport) -> Vec<&'static str> {
        report
            .reasons
            .iter()
            .map(|reason| reason.as_str())
            .collect()
    }

    /// The check may only read. Ledger bytes, HEAD, and porcelain stay put.
    fn check(fix: &Fixture, claim: &DeliveryReceipt) -> DeliveryReceiptReport {
        let head = git(&fix.repo, &["rev-parse", "HEAD"]);
        let ledger = fs::read(&fix.ledger).unwrap();
        let status = git(&fix.repo, &["status", "--porcelain"]);
        let adapter = GitWorktreeAdapter::new(fix.repo.clone());
        let probe = GitDeliveryProbe::new(&adapter);
        let report = prove_delivery_receipt(&probe, claim, &fix.repo);
        assert!(!report.apply);
        assert_eq!(git(&fix.repo, &["rev-parse", "HEAD"]), head);
        assert_eq!(fs::read(&fix.ledger).unwrap(), ledger);
        assert_eq!(git(&fix.repo, &["status", "--porcelain"]), status);
        report
    }

    fn fast_forward(fix: &Fixture) -> (String, String) {
        git(&fix.repo, &["checkout", "-q", "-b", "feature"]);
        let accepted = commit_file(&fix.repo, "landed.txt", "landed", "accepted");
        git(&fix.repo, &["checkout", "-q", "main"]);
        git(&fix.repo, &["merge", "--ff-only", "feature"]);
        let tip = commit_file(&fix.repo, "follow.txt", "follow", "follow");
        (accepted, tip)
    }

    #[test]
    fn delivery_reachability_fast_forward() {
        let fix = fixture();
        let (accepted, tip) = fast_forward(&fix);
        let kept = root_blob(&fix);
        let report = check(
            &fix,
            &receipt(
                LandingStrategy::FastForward,
                &accepted,
                Some(&tip),
                None,
                &[&kept],
            ),
        );
        assert!(report.passed, "{report:?}");
        assert_eq!(codes(&report), Vec::<&str>::new());
        assert_eq!(
            report.observed_destination.as_ref().map(ObjectId::as_str),
            Some(tip.as_str())
        );
        assert!(git_ok(&fix.repo, &["cat-file", "-e", &kept]));
        let not_merge = check(
            &fix,
            &receipt(LandingStrategy::Merge, &accepted, Some(&tip), None, &[]),
        );
        assert!(!not_merge.passed);
        assert_eq!(codes(&not_merge), ["accepted-not-landed"]);
    }

    #[test]
    fn delivery_reachability_merge() {
        let fix = fixture();
        git(&fix.repo, &["checkout", "-q", "-b", "feature"]);
        let accepted = commit_file(&fix.repo, "feature.txt", "feature", "accepted");
        git(&fix.repo, &["checkout", "-q", "main"]);
        git(&fix.repo, &["merge", "--no-ff", "--no-edit", "feature"]);
        let tip = git(&fix.repo, &["rev-parse", "HEAD"]);
        let report = check(
            &fix,
            &receipt(LandingStrategy::Merge, &accepted, Some(&tip), None, &[]),
        );
        assert!(report.passed, "{report:?}");
        assert_eq!(codes(&report), Vec::<&str>::new());
    }

    #[test]
    fn delivery_reachability_rewrite_reachable() {
        let fix = fixture();
        let base = git(&fix.repo, &["rev-parse", "HEAD"]);
        git(&fix.repo, &["checkout", "-q", "-b", "feature"]);
        let accepted = commit_file(&fix.repo, "landed.txt", "landed", "accepted");
        let tree = git(&fix.repo, &["rev-parse", &format!("{accepted}^{{tree}}")]);
        git(&fix.repo, &["checkout", "-q", "--detach"]);
        let rewritten = git(
            &fix.repo,
            &["commit-tree", &tree, "-p", &base, "-m", "rewrite"],
        );
        git(&fix.repo, &["update-ref", "refs/heads/main", &rewritten]);
        git(&fix.repo, &["checkout", "-q", "main"]);
        assert_ne!(accepted, rewritten);
        assert!(!git_ok(
            &fix.repo,
            &["merge-base", "--is-ancestor", &accepted, &rewritten]
        ));
        let report = check(
            &fix,
            &receipt(
                LandingStrategy::Rewrite,
                &accepted,
                Some(&rewritten),
                Some(&tree),
                &[&accepted],
            ),
        );
        assert!(report.passed, "{report:?}");
        assert!(git_ok(&fix.repo, &["cat-file", "-e", &accepted]));
        assert_eq!(fs::read(fix.repo.join("landed.txt")).unwrap(), b"landed");
    }

    #[test]
    fn delivery_reachability_rewritten_destination() {
        let fix = fixture();
        git(&fix.repo, &["checkout", "-q", "-b", "feature"]);
        let accepted = commit_file(&fix.repo, "landed.txt", "landed", "accepted");
        let tree = git(&fix.repo, &["rev-parse", &format!("{accepted}^{{tree}}")]);
        git(&fix.repo, &["checkout", "-q", "main"]);
        let tip = commit_file(&fix.repo, "other.txt", "other", "unrelated");
        let report = check(
            &fix,
            &receipt(
                LandingStrategy::Rewrite,
                &accepted,
                Some(&tip),
                Some(&tree),
                &[],
            ),
        );
        assert!(!report.passed);
        assert_eq!(codes(&report), ["rewritten-destination"]);
        assert!(git_ok(&fix.repo, &["cat-file", "-e", &accepted]));
    }

    #[test]
    fn delivery_reachability_stale_target() {
        let fix = fixture();
        let (accepted, old) = fast_forward(&fix);
        let _moved = commit_file(&fix.repo, "later.txt", "later", "later");
        let report = check(
            &fix,
            &receipt(
                LandingStrategy::FastForward,
                &accepted,
                Some(&old),
                None,
                &[],
            ),
        );
        assert!(!report.passed);
        assert_eq!(codes(&report), ["stale-destination"]);
    }

    #[test]
    fn delivery_reachability_accepted_only() {
        let fix = fixture();
        let destination = git(&fix.repo, &["rev-parse", "HEAD"]);
        git(&fix.repo, &["checkout", "-q", "-b", "feature"]);
        let accepted = commit_file(&fix.repo, "only.txt", "only", "accepted");
        git(&fix.repo, &["checkout", "-q", "main"]);
        let report = check(
            &fix,
            &receipt(
                LandingStrategy::FastForward,
                &accepted,
                Some(&destination),
                None,
                &[],
            ),
        );
        assert!(!report.passed);
        assert_eq!(codes(&report), ["accepted-not-landed"]);
        assert_eq!(fs::read(fix.repo.join("README.md")).unwrap(), b"seed");
    }

    #[test]
    fn delivery_reachability_planner_dirt() {
        let fix = fixture();
        let (accepted, tip) = fast_forward(&fix);
        fs::write(fix.repo.join("planner-note.txt"), b"do-not-delete").unwrap();
        let report = check(
            &fix,
            &receipt(
                LandingStrategy::FastForward,
                &accepted,
                Some(&tip),
                None,
                &[],
            ),
        );
        assert!(!report.passed);
        assert_eq!(codes(&report), ["planner-dirty"]);
        assert_eq!(
            fs::read(fix.repo.join("planner-note.txt")).unwrap(),
            b"do-not-delete"
        );
    }

    #[test]
    fn delivery_reachability_missing_recovery_object() {
        let fix = fixture();
        let (accepted, tip) = fast_forward(&fix);
        let kept = root_blob(&fix);
        let missing = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let report = check(
            &fix,
            &receipt(
                LandingStrategy::FastForward,
                &accepted,
                Some(&tip),
                None,
                &[&kept, missing],
            ),
        );
        assert!(!report.passed);
        assert_eq!(codes(&report), ["missing-recovery-object"]);
        assert!(git_ok(&fix.repo, &["cat-file", "-e", &kept]));
        assert_eq!(fs::read(fix.root.join("keep.txt")).unwrap(), b"keep");
    }

    #[test]
    fn delivery_reachability_unreadable_ref() {
        let fix = fixture();
        let accepted = git(&fix.repo, &["rev-parse", "HEAD"]);
        let mut claim = receipt(LandingStrategy::FastForward, &accepted, None, None, &[]);
        claim.destination_ref = GitRef::new("refs/heads/missing").unwrap();
        let report = check(&fix, &claim);
        assert!(!report.passed);
        assert_eq!(codes(&report), ["unreadable-ref"]);
        assert!(report.observed_destination.is_none());
    }

    fn root_blob(fix: &Fixture) -> String {
        let path = fix.root.join("keep.txt");
        fs::write(&path, b"keep").unwrap();
        git(&fix.repo, &["hash-object", "-w", path.to_str().unwrap()])
    }

    fn git_ok(repo: &Path, args: &[&str]) -> bool {
        Command::new("git")
            .current_dir(repo)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .guarded_status()
            .unwrap()
            .success()
    }
}
