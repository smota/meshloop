//! Loads meshloop.toml-shaped configuration. No credentials or per-harness CLI flags
//! are hardcoded here — every value comes from the file.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub selected_harnesses: Vec<String>,
    pub limits: Limits,
    #[serde(default)]
    pub harnesses: HashMap<String, HarnessConfig>,
    #[serde(default)]
    pub verify: VerifyConfig,
}

#[derive(Debug, Deserialize)]
pub struct Limits {
    pub max_concurrent_workers: u32,
    /// Maximum attempts per task (the first dispatch counts). `1` means no fallback.
    pub max_retries: u32,
    pub task_timeout_seconds: u64,
}

#[derive(Debug, Deserialize, Default)]
pub struct VerifyConfig {
    #[serde(default)]
    pub verify_command: Vec<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct HarnessConfig {
    pub executable: String,
    /// Herdr `--kind` for live workers. Ignored for `fixture`.
    #[serde(default)]
    pub kind: Option<String>,
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

#[derive(Debug)]
#[allow(dead_code)]
pub enum ConfigError {
    Io(String),
    Parse(String),
    SelectedButNotConfigured(String),
    NotFound,
}

pub fn discover(explicit: Option<&Path>) -> Result<PathBuf, ConfigError> {
    if let Some(p) = explicit {
        return Ok(p.to_path_buf());
    }
    for candidate in ["meshloop.toml", "config/meshloop.toml"] {
        let p = PathBuf::from(candidate);
        if p.is_file() {
            return Ok(p);
        }
    }
    Err(ConfigError::NotFound)
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

pub fn resolve_executable(configured: &str) -> PathBuf {
    let path = PathBuf::from(configured);
    let has_sep = configured.contains('/') || configured.contains('\\');
    if has_sep || path.exists() {
        if !path.exists() {
            let mut with_ext = path.clone();
            with_ext.set_file_name(format!(
                "{}{}",
                path.file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(configured),
                std::env::consts::EXE_SUFFIX
            ));
            // If the original had a directory, preserve it.
            if path.components().count() > 1 {
                let mut p = path.clone();
                if let Some(name) = path.file_name() {
                    p.set_file_name(format!(
                        "{}{}",
                        name.to_string_lossy(),
                        std::env::consts::EXE_SUFFIX
                    ));
                    if p.exists() {
                        return p;
                    }
                }
            } else if with_ext.exists() {
                return with_ext;
            }
        }
        return path;
    }
    PathBuf::from(configured)
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
        assert!(!config.selected_harnesses.is_empty());
        assert!(
            config
                .selected_harnesses
                .iter()
                .all(|n| n != "fixture" && config.harnesses.contains_key(n))
        );
        for name in &config.selected_harnesses {
            let hc = &config.harnesses[name];
            assert!(hc.kind.as_deref() == Some(name.as_str()) || hc.kind.is_some());
        }
    }

    #[test]
    fn fixture_config_loads_for_ci() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("config/meshloop.fixture.toml");
        let config = load(&path).expect("fixture config should load");
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
