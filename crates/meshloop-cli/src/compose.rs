//! Wires adapters into engine ports for one run — the only place concrete adapter types
//! are constructed (ADR 0002 / boundaries.md's meshloop-cli role).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use meshloop_adapters::check::CommandCheckRunner;
use meshloop_adapters::git::GitWorktreeAdapter;
use meshloop_adapters::harness::{CliHarness, CliHarnessConfig};
use meshloop_adapters::herdr::{HerdrCliAdapter, HerdrWorkerHarness};
use meshloop_adapters::process::WindowsProcessView;
use meshloop_adapters::store::SqliteStore;
use meshloop_domain::capability::{HarnessError, HarnessProfile};
use meshloop_domain::policy::ModelCapabilityTier;
use meshloop_engine::agent::AgentSpec;
use meshloop_engine::orchestrate::herdr_kind;
use meshloop_engine::origin::Origin;
use meshloop_engine::ports::{HarnessCapabilities, HarnessHandle, HarnessOutcome, StoreError};
use meshloop_engine::router::Candidate;
use meshloop_engine::run_loop::RunLimits;

use crate::config::{Config, resolve_executable};

pub enum DispatchHarness {
    Fixture(CliHarness),
    Live(HerdrWorkerHarness),
}

impl HarnessCapabilities for DispatchHarness {
    fn probe(&self) -> Result<HarnessProfile, HarnessError> {
        match self {
            Self::Fixture(h) => h.probe(),
            Self::Live(h) => h.probe(),
        }
    }

    fn invoke(&self, spec: &AgentSpec) -> Result<HarnessHandle, HarnessError> {
        match self {
            Self::Fixture(h) => h.invoke(spec),
            Self::Live(h) => h.invoke(spec),
        }
    }

    fn cancel(&self, handle: &HarnessHandle) -> Result<(), HarnessError> {
        match self {
            Self::Fixture(h) => h.cancel(handle),
            Self::Live(h) => h.cancel(handle),
        }
    }

    fn collect(&self, handle: &HarnessHandle) -> Result<HarnessOutcome, HarnessError> {
        match self {
            Self::Fixture(h) => h.collect(handle),
            Self::Live(h) => h.collect(handle),
        }
    }
}

pub struct Composed {
    pub harnesses: HashMap<String, DispatchHarness>,
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

pub fn herdr_bin() -> PathBuf {
    PathBuf::from("herdr")
}

pub struct ComposeRequest<'a> {
    pub config: &'a Config,
    pub repo_root: PathBuf,
    pub db_path: Option<&'a Path>,
    pub worktree_base: Option<PathBuf>,
    pub origin: Origin,
    pub fixture_only: bool,
    /// Fail closed when a live harness is selected and Herdr is down (plan/run/resume).
    pub require_herdr: bool,
}

pub fn compose(req: ComposeRequest<'_>) -> Result<Composed, String> {
    let needs_live =
        req.config.selected_harnesses.iter().any(|n| n != "fixture") && !req.fixture_only;
    if needs_live && req.require_herdr {
        let herdr = HerdrCliAdapter::new(herdr_bin());
        match herdr.probe_status() {
            Ok(d) if d.server_running => {}
            other => {
                return Err(format!(
                    "live workers require Herdr 0.8 with a running server; got {other:?}. \
                     Start Herdr, or pass --fixture-only for the CI double."
                ));
            }
        }
    }

    let mut harnesses = HashMap::new();
    let mut candidates = Vec::new();
    for name in &req.config.selected_harnesses {
        let hc = req
            .config
            .harnesses
            .get(name)
            .ok_or_else(|| format!("harness '{name}' selected but not configured"))?;
        let live = name != "fixture" && !req.fixture_only;
        if live {
            let kind = hc
                .kind
                .clone()
                .or_else(|| herdr_kind(name).map(str::to_string))
                .or_else(|| herdr_kind(&hc.model_ref).map(str::to_string))
                .ok_or_else(|| {
                    format!(
                        "harness '{name}' is not fixture and has no Herdr --kind (set kind = \"claude\"|\"codex\"|\"pi\"|\"grok\"|\"agy\")"
                    )
                })?;
            harnesses.insert(
                name.clone(),
                DispatchHarness::Live(HerdrWorkerHarness::new(
                    herdr_bin(),
                    name.clone(),
                    kind,
                    req.origin.session.clone(),
                )),
            );
        } else {
            harnesses.insert(
                name.clone(),
                DispatchHarness::Fixture(CliHarness::new(CliHarnessConfig {
                    name: name.clone(),
                    executable: resolve_executable(&hc.executable),
                    version_args: hc.version_args.clone(),
                    invoke_args_template: hc.invoke_args_template.clone(),
                })),
            );
        }
        candidates.push(Candidate {
            harness: name.clone(),
            model_ref: hc.model_ref.clone(),
            model_tier: parse_model_tier(&hc.model_tier),
        });
    }

    let git = GitWorktreeAdapter::new(req.repo_root.clone());
    let db = req
        .db_path
        .map(Path::to_path_buf)
        .unwrap_or_else(|| default_db(&req.repo_root));
    let store = SqliteStore::open(&db).map_err(|e: StoreError| format!("{e:?}"))?;
    let wt = req
        .worktree_base
        .unwrap_or_else(|| default_worktree_base(&req.repo_root));

    Ok(Composed {
        harnesses,
        candidates,
        git,
        store,
        processes: WindowsProcessView,
        checks: CommandCheckRunner,
        limits: RunLimits {
            max_retries: req.config.limits.max_retries,
            task_timeout: Duration::from_secs(req.config.limits.task_timeout_seconds),
            max_concurrent_workers: req.config.limits.max_concurrent_workers,
        }
        .clamped(),
        verify_command: req.config.verify.verify_command.clone(),
        worktree_base: wt,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load;
    use std::path::Path;

    #[test]
    fn composes_the_fixture_config_into_subprocess_adapters() {
        let config_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("config/meshloop.fixture.toml");
        let config = load(&config_path).unwrap();
        let dir = std::env::temp_dir().join(format!("meshloop-compose-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let composed = compose(ComposeRequest {
            config: &config,
            repo_root: dir.clone(),
            db_path: Some(&dir.join("t.sqlite")),
            worktree_base: None,
            origin: Origin::default(),
            fixture_only: true,
            require_herdr: false,
        })
        .unwrap();
        assert_eq!(composed.harnesses.len(), 1);
        assert_eq!(composed.candidates[0].harness, "fixture");
        assert!(matches!(
            composed.harnesses.get("fixture"),
            Some(DispatchHarness::Fixture(_))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
