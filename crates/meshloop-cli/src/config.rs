//! Loads config/meshloop.example.toml-shaped configuration. No credentials, real model
//! IDs, or per-harness CLI flags are hardcoded here — every value comes from the file.

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub selected_harnesses: Vec<String>,
    pub limits: Limits,
    #[serde(default)]
    pub harnesses: HashMap<String, HarnessConfig>,
}

// Concurrency and retry bounds are read and validated by config parsing now; `run`'s v1
// sequential dispatcher does not yet consume them (implementation-plan.md Phase 8).
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct Limits {
    pub max_concurrent_workers: u32,
    pub max_retries: u32,
    pub task_timeout_seconds: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct HarnessConfig {
    pub executable: String,
    #[serde(default)]
    pub version_args: Vec<String>,
    #[serde(default)]
    pub invoke_args_template: Vec<String>,
    pub model_ref: String,
    #[serde(default = "default_model_tier")]
    pub model_tier: String,
}

fn default_model_tier() -> String {
    "mid".into()
}

// Only read via {:?} at call sites for diagnostics — rustc's dead-code lint does not
// count Debug-only usage as a read, hence the explicit allow.
#[derive(Debug)]
#[allow(dead_code)]
pub enum ConfigError {
    Io(String),
    Parse(String),
    SelectedButNotConfigured(String),
}

pub fn load(path: &Path) -> Result<Config, ConfigError> {
    let text = std::fs::read_to_string(path).map_err(|e| ConfigError::Io(e.to_string()))?;
    let config: Config = toml::from_str(&text).map_err(|e| ConfigError::Parse(e.to_string()))?;
    for name in &config.selected_harnesses {
        if !config.harnesses.contains_key(name) {
            return Err(ConfigError::SelectedButNotConfigured(name.clone()));
        }
    }
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_config_loads_and_is_internally_consistent() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("config/meshloop.example.toml");
        let config = load(&path).expect("example config should load");
        assert_eq!(config.selected_harnesses, vec!["fixture"]);
        assert!(config.harnesses.contains_key("fixture"));
    }

    #[test]
    fn selecting_an_unconfigured_harness_is_rejected() {
        let toml = r#"
            selected_harnesses = ["ghost"]
            [limits]
            max_concurrent_workers = 1
            max_retries = 1
            task_timeout_seconds = 60
        "#;
        let dir = std::env::temp_dir().join(format!("meshloop-cfg-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bad.toml");
        std::fs::write(&path, toml).unwrap();
        let result = load(&path);
        assert!(matches!(
            result,
            Err(ConfigError::SelectedButNotConfigured(_))
        ));
        std::fs::remove_dir_all(&dir).ok();
    }
}
