//! Wires adapters into engine ports for one run — the only place concrete adapter types
//! are constructed (ADR 0002 / boundaries.md's meshloop-cli role).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use meshloop_adapters::git::GitWorktreeAdapter;
use meshloop_adapters::harness::{CliHarness, CliHarnessConfig};
use meshloop_adapters::store::SqliteStore;
use meshloop_domain::policy::ModelCapabilityTier;
use meshloop_engine::ports::StoreError;
use meshloop_engine::router::Candidate;

use crate::config::Config;

pub struct Composed {
    pub harnesses: HashMap<String, CliHarness>,
    pub candidates: Vec<Candidate>,
    pub git: GitWorktreeAdapter,
    pub store: SqliteStore,
}

fn parse_model_tier(s: &str) -> ModelCapabilityTier {
    match s {
        "top" => ModelCapabilityTier::TopTier,
        "mid" => ModelCapabilityTier::MidTier,
        _ => ModelCapabilityTier::Lightweight,
    }
}

pub fn compose(
    config: &Config,
    repo_root: PathBuf,
    db_path: Option<&Path>,
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
                executable: PathBuf::from(&hc.executable),
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

    let git = GitWorktreeAdapter::new(repo_root);
    let store = match db_path {
        Some(p) => SqliteStore::open(p),
        None => SqliteStore::open_in_memory(),
    }
    .map_err(|e: StoreError| format!("{e:?}"))?;

    Ok(Composed {
        harnesses,
        candidates,
        git,
        store,
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
        let composed = compose(&config, std::env::temp_dir(), None).unwrap();
        assert_eq!(composed.harnesses.len(), 1);
        assert_eq!(composed.candidates.len(), 1);
        assert_eq!(composed.candidates[0].harness, "fixture");
    }
}
