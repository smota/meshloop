//! Worktree-per-writer isolation (ADR 0005 / runtime-design.md §3). Real `git worktree`
//! subprocess calls with structured arguments — never shell-string interpolation.

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitError {
    CommandFailed { stderr: String },
    Io(String),
}

pub struct GitWorktreeAdapter {
    repo_root: PathBuf,
}

impl GitWorktreeAdapter {
    pub fn new(repo_root: PathBuf) -> Self {
        Self { repo_root }
    }

    fn run(&self, args: &[&str]) -> Result<(), GitError> {
        let output = Command::new("git")
            .current_dir(&self.repo_root)
            .args(args)
            .output()
            .map_err(|e| GitError::Io(e.to_string()))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(GitError::CommandFailed {
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            })
        }
    }

    pub fn add_worktree(&self, path: &Path, branch: &str) -> Result<(), GitError> {
        self.run(&["worktree", "add", "-b", branch, &path.to_string_lossy()])
    }

    /// Idempotent-in-effect: removing a path that is not a live worktree returns the git
    /// error, which the caller treats as "already gone," per the recovery contract's
    /// no-op-on-already-cleaned rule.
    pub fn remove_worktree(&self, path: &Path) -> Result<(), GitError> {
        self.run(&["worktree", "remove", "--force", &path.to_string_lossy()])
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
}
