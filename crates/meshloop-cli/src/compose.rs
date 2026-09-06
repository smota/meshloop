//! Wires adapters into engine ports for one run — the only place concrete adapter types
//! are constructed (ADR 0002 / boundaries.md's meshloop-cli role).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use meshloop_adapters::check::CommandCheckRunner;
use meshloop_adapters::git::GitWorktreeAdapter;
use meshloop_adapters::harness::{CliHarness, CliHarnessConfig};
use meshloop_adapters::process::WindowsProcessView;
use meshloop_adapters::store::SqliteStore;
use meshloop_domain::policy::ModelCapabilityTier;
use meshloop_engine::ports::StoreError;
use meshloop_engine::router::Candidate;
use meshloop_engine::run_loop::RunLimits;

use crate::config::{Config, resolve_executable};

pub struct Composed {
    pub harnesses: HashMap<String, CliHarness>,
    pub candidates: Vec<Candidate>,
    pub git: GitWorktreeAdapter,
    pub store: SqliteStore,
    pub processes: WindowsProcessView,
    pub checks: CommandCheckRunner,
    pub limits: RunLimits,
    pub verify_command: Vec<String>,
    pub worktree_base: PathBuf,
}

fn parse_model_tier(s: &str) -> ModelCapabilityTier {
    match s {
        "top" => ModelCapabilityTier::TopTier,
        "mid" => ModelCapabilityTier::MidTier,
        _ => ModelCapabilityTier::Lightweight,
    }
}

pub fn default_db(repo_root: &Path) -> PathBuf {
    repo_root.join(".meshloop").join("state.sqlite")
}

pub fn default_worktree_base(repo_root: &Path) -> PathBuf {
    let name = repo_root
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".into());
    repo_root
        .parent()
        .unwrap_or(repo_root)
        .join(".meshloop-worktrees")
        .join(name)
}

pub fn compose(
    config: &Config,
    repo_root: PathBuf,
    db_path: Option<&Path>,
    worktree_base: Option<PathBuf>,
) -> Result<Composed, String> {
    let mut harnesses = HashMap::new();
    let mut candidates = Vec::new();
    for name in &config.selected_harnesses {
        let hc = config
            .harnesses
            .get(name)
            .ok_or_else(|| format!("harness '{name}' selected but not configured"))?;
        harnesses.insert(
            name.clone(),
            CliHarness::new(CliHarnessConfig {
                name: name.clone(),
                executable: resolve_executable(&hc.executable),
                version_args: hc.version_args.clone(),
                invoke_args_template: hc.invoke_args_template.clone(),
            }),
        );
        candidates.push(Candidate {
            harness: name.clone(),
            model_ref: hc.model_ref.clone(),
            model_tier: parse_model_tier(&hc.model_tier),
        });
    }

    let git = GitWorktreeAdapter::new(repo_root.clone());
    let db = db_path
        .map(Path::to_path_buf)
        .unwrap_or_else(|| default_db(&repo_root));
    let store = SqliteStore::open(&db).map_err(|e: StoreError| format!("{e:?}"))?;
    let wt = worktree_base.unwrap_or_else(|| default_worktree_base(&repo_root));

    Ok(Composed {
        harnesses,
        candidates,
        git,
        store,
        processes: WindowsProcessView,
        checks: CommandCheckRunner,
        limits: RunLimits {
            max_retries: config.limits.max_retries,
            task_timeout: Duration::from_secs(config.limits.task_timeout_seconds),
            max_concurrent_workers: config.limits.max_concurrent_workers,
        }
        .clamped(),
        verify_command: config.verify.verify_command.clone(),
        worktree_base: wt,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load;
    use std::path::Path;

    #[test]
    fn composes_the_example_config_into_real_adapters() {
        let config_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("config/meshloop.example.toml");
        let config = load(&config_path).unwrap();
        let dir = std::env::temp_dir().join(format!("meshloop-compose-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let composed = compose(&config, dir.clone(), Some(&dir.join("t.sqlite")), None).unwrap();
        assert_eq!(composed.harnesses.len(), 1);
        assert_eq!(composed.candidates[0].harness, "fixture");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
