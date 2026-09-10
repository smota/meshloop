//! Worktree-per-writer isolation (ADR 0005 / runtime-design.md §3). Real `git worktree`
//! subprocess calls with structured arguments — never shell-string interpolation.

use std::path::{Path, PathBuf};
use std::process::Command;

use meshloop_engine::ports::{DiffSummary, WorkspaceError, WorkspacePort};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitError {
    CommandFailed { stderr: String },
    Io(String),
    Conflict,
    Dirty,
}

impl From<GitError> for WorkspaceError {
    fn from(e: GitError) -> Self {
        match e {
            GitError::CommandFailed { stderr } => WorkspaceError::CommandFailed { stderr },
            GitError::Io(s) => WorkspaceError::Io(s),
            GitError::Conflict => WorkspaceError::Conflict,
            GitError::Dirty => WorkspaceError::Dirty,
        }
    }
}

pub struct GitWorktreeAdapter {
    repo_root: PathBuf,
}

impl GitWorktreeAdapter {
    pub fn new(repo_root: PathBuf) -> Self {
        Self { repo_root }
    }

    fn run_in(&self, cwd: &Path, args: &[&str]) -> Result<String, GitError> {
        let mut cmd = Command::new("git");
        cmd.current_dir(cwd).args(args);
        cmd.env("GIT_AUTHOR_NAME", "meshloop");
        cmd.env("GIT_AUTHOR_EMAIL", "meshloop@localhost");
        cmd.env("GIT_COMMITTER_NAME", "meshloop");
        cmd.env("GIT_COMMITTER_EMAIL", "meshloop@localhost");
        let output = cmd.output().map_err(|e| GitError::Io(e.to_string()))?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            Err(GitError::CommandFailed {
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            })
        }
    }

    fn run(&self, args: &[&str]) -> Result<String, GitError> {
        self.run_in(&self.repo_root, args)
    }

    pub fn add_worktree(&self, path: &Path, branch: &str) -> Result<(), GitError> {
        self.run(&["worktree", "add", "-b", branch, &path.to_string_lossy()])
            .map(|_| ())
    }

    pub fn remove_worktree(&self, path: &Path) -> Result<(), GitError> {
        self.run(&["worktree", "remove", "--force", &path.to_string_lossy()])
            .map(|_| ())
    }
}

impl WorkspacePort for GitWorktreeAdapter {
    fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    fn add_worktree_from(
        &self,
        path: &Path,
        branch: &str,
        start_point: &str,
    ) -> Result<(), WorkspaceError> {
        self.run(&[
            "worktree",
            "add",
            "-b",
            branch,
            &path.to_string_lossy(),
            start_point,
        ])
        .map(|_| ())
        .map_err(Into::into)
    }

    fn head(&self, worktree: &Path) -> Result<String, WorkspaceError> {
        self.run_in(worktree, &["rev-parse", "HEAD"])
            .map_err(Into::into)
    }

    fn diff_against(&self, worktree: &Path, base: &str) -> Result<DiffSummary, WorkspaceError> {
        let head = self.head(worktree)?;
        let files_raw = self
            .run_in(worktree, &["diff", "--name-only", base, "HEAD"])
            .or_else(|_| self.run_in(worktree, &["diff", "--name-only", base]))
            .map_err(WorkspaceError::from)?;
        let files: Vec<String> = files_raw
            .lines()
            .map(|l| l.trim().replace('\\', "/"))
            .filter(|l| !l.is_empty())
            .collect();
        let stat = self
            .run_in(worktree, &["diff", "--stat", base, "HEAD"])
            .or_else(|_| self.run_in(worktree, &["diff", "--stat", base]))
            .unwrap_or_default();
        Ok(DiffSummary {
            base: base.into(),
            head,
            files,
            stat_redacted: crate::redact::redact(&stat),
        })
    }

    fn status_porcelain(&self, worktree: &Path) -> Result<String, WorkspaceError> {
        self.run_in(worktree, &["status", "--porcelain"])
            .map_err(Into::into)
    }

    fn commit_all(&self, worktree: &Path, message: &str) -> Result<String, WorkspaceError> {
        let porcelain = self.status_porcelain(worktree)?;
        if porcelain.trim().is_empty() {
            return self.head(worktree);
        }
        self.run_in(worktree, &["add", "-A"])
            .map_err(WorkspaceError::from)?;
        self.run_in(worktree, &["commit", "-q", "-m", message])
            .map_err(WorkspaceError::from)?;
        self.head(worktree)
    }

    fn merge_in_worktree(&self, worktree: &Path, from_ref: &str) -> Result<(), WorkspaceError> {
        match self.run_in(worktree, &["merge", "--no-ff", "--no-edit", from_ref]) {
            Ok(_) => Ok(()),
            Err(_) => {
                let _ = self.run_in(worktree, &["merge", "--abort"]);
                Err(WorkspaceError::Conflict)
            }
        }
    }

    fn branch_exists(&self, branch: &str) -> Result<bool, WorkspaceError> {
        match self.run(&["rev-parse", "--verify", branch]) {
            Ok(_) => Ok(true),
            Err(GitError::CommandFailed { .. }) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    fn worktree_exists(&self, path: &Path) -> bool {
        path.is_dir()
            && self
                .run_in(path, &["rev-parse", "--is-inside-work-tree"])
                .is_ok()
    }

    fn prune(&self) -> Result<(), WorkspaceError> {
        self.run(&["worktree", "prune"])
            .map(|_| ())
            .map_err(Into::into)
    }

    fn is_ancestor(&self, ancestor: &str, descendant: &str) -> Result<bool, WorkspaceError> {
        match self.run(&["merge-base", "--is-ancestor", ancestor, descendant]) {
            Ok(_) => Ok(true),
            Err(GitError::CommandFailed { .. }) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    fn checkout_ref(&self, git_ref: &str) -> Result<(), WorkspaceError> {
        let dirty = self.status_porcelain(&self.repo_root)?;
        if !dirty.trim().is_empty() {
            return Err(WorkspaceError::Dirty);
        }
        self.run(&["checkout", git_ref])
            .map(|_| ())
            .map_err(Into::into)
    }

    fn merge_ff_only(&self, from: &str) -> Result<(), WorkspaceError> {
        self.run(&["merge", "--ff-only", from])
            .map(|_| ())
            .map_err(Into::into)
    }

    fn merge_no_ff(&self, from: &str) -> Result<(), WorkspaceError> {
        match self.run(&["merge", "--no-ff", "--no-edit", from]) {
            Ok(_) => Ok(()),
            Err(_) => {
                let _ = self.run(&["merge", "--abort"]);
                Err(WorkspaceError::Conflict)
            }
        }
    }

    fn current_head(&self) -> Result<String, WorkspaceError> {
        self.head(&self.repo_root)
    }

    fn reset_hard(&self, worktree: &Path, rev: &str) -> Result<(), WorkspaceError> {
        self.run_in(worktree, &["reset", "--hard", rev])
            .map(|_| ())
            .map_err(Into::into)
    }

    fn remove_file(&self, worktree: &Path, rel: &str) -> Result<(), WorkspaceError> {
        let path = worktree.join(rel);
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| WorkspaceError::Io(e.to_string()))?;
        }
        Ok(())
    }

    fn unified_diff(&self, worktree: &Path, base: &str) -> Result<String, WorkspaceError> {
        self.run_in(worktree, &["diff", "--no-color", base, "HEAD"])
            .or_else(|_| self.run_in(worktree, &["diff", "--no-color", base]))
            .map_err(Into::into)
    }

    fn remove_worktree(&self, path: &Path) -> Result<(), WorkspaceError> {
        if !self.worktree_exists(path) {
            return Ok(());
        }
        self.run(&["worktree", "remove", "--force", &path.to_string_lossy()])
            .map(|_| ())
            .map_err(Into::into)
    }

    fn delete_branch(&self, branch: &str) -> Result<(), WorkspaceError> {
        match self.run(&["branch", "-D", branch]) {
            Ok(_) => Ok(()),
            Err(GitError::CommandFailed { stderr })
                if stderr.contains("not found") || stderr.contains("not exist") =>
            {
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command as StdCommand;

    fn disposable_repo() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("meshloop-git-test-{}", uuid_ish()));
        fs::create_dir_all(&dir).unwrap();
        StdCommand::new("git")
            .current_dir(&dir)
            .args(["init", "-q"])
            .status()
            .unwrap();
        StdCommand::new("git")
            .current_dir(&dir)
            .args(["config", "user.email", "test@test"])
            .status()
            .unwrap();
        StdCommand::new("git")
            .current_dir(&dir)
            .args(["config", "user.name", "test"])
            .status()
            .unwrap();
        fs::write(dir.join("README.md"), "seed").unwrap();
        StdCommand::new("git")
            .current_dir(&dir)
            .args(["add", "."])
            .status()
            .unwrap();
        StdCommand::new("git")
            .current_dir(&dir)
            .args(["commit", "-q", "-m", "seed"])
            .status()
            .unwrap();
        dir
    }

    fn uuid_ish() -> u128 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }

    #[test]
    fn add_and_remove_worktree_round_trips_on_a_disposable_repo() {
        let repo = disposable_repo();
        let adapter = GitWorktreeAdapter::new(repo.clone());
        let wt_path = repo.parent().unwrap().join(format!("wt-{}", uuid_ish()));

        adapter
            .add_worktree(&wt_path, "meshloop/test")
            .expect("add should succeed");
        assert!(wt_path.join(".git").exists());

        adapter
            .remove_worktree(&wt_path)
            .expect("remove should succeed");
        assert!(!wt_path.exists());

        fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn removing_a_nonexistent_worktree_reports_a_git_error_not_a_panic() {
        let repo = disposable_repo();
        let adapter = GitWorktreeAdapter::new(repo.clone());
        let missing = repo.parent().unwrap().join("never-existed");
        assert!(adapter.remove_worktree(&missing).is_err());
        fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn add_worktree_from_fails_when_path_exists() {
        let repo = disposable_repo();
        let adapter = GitWorktreeAdapter::new(repo.clone());
        let wt_path = repo
            .parent()
            .unwrap()
            .join(format!("wt-exists-{}", uuid_ish()));
        fs::create_dir_all(&wt_path).unwrap();
        fs::write(wt_path.join("blocker.txt"), "nope").unwrap();
        let head = adapter.current_head().unwrap();
        assert!(
            adapter
                .add_worktree_from(&wt_path, "meshloop/exists", &head)
                .is_err()
        );
        fs::remove_dir_all(&wt_path).ok();
        fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn commit_all_creates_a_meshloop_owned_commit() {
        let repo = disposable_repo();
        let adapter = GitWorktreeAdapter::new(repo.clone());
        let wt_path = repo
            .parent()
            .unwrap()
            .join(format!("wt-commit-{}", uuid_ish()));
        let head = adapter.current_head().unwrap();
        adapter
            .add_worktree_from(&wt_path, "meshloop/commit-test", &head)
            .unwrap();
        fs::write(wt_path.join("new.txt"), "hello").unwrap();
        let new_head = adapter.commit_all(&wt_path, "meshloop: attempt").unwrap();
        assert_ne!(new_head, head);
        adapter.remove_worktree(&wt_path).ok();
        fs::remove_dir_all(&repo).ok();
    }
}
