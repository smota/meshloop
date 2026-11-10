//! Loads meshloop.toml-shaped configuration. No credentials or per-harness CLI flags
//! are hardcoded here — every value comes from the file.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use meshloop_adapters::harness::PromptVia;
use meshloop_domain::task_graph::Tier;
use meshloop_engine::run_loop::PlannerSelection;
use serde::Deserialize;

#[derive(Debug, Deserialize, Default, Clone)]
#[allow(dead_code)]
pub struct Tier1FileConfig {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub harness: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Config {
    pub selected_harnesses: Vec<String>,
    pub limits: Limits,
    #[serde(default)]
    pub harnesses: HashMap<String, HarnessConfig>,
    #[serde(default)]
    pub verify: VerifyConfig,
    #[serde(default)]
    pub planner: PlannerConfig,
    /// Optional `[workspace]`: `root` and `auto_reclaim`. Missing tables keep the
    /// nested default root and automatic reclamation on.
    #[serde(default)]
    pub workspace: WorkspaceConfig,
    /// When true, Meshloop runs standalone without Herdr daemon, dispatching direct CLI harnesses into Git worktrees.
    #[serde(default)]
    #[allow(dead_code)]
    pub standalone: bool,
    #[serde(default)]
    #[allow(dead_code)]
    pub tier1: Option<Tier1FileConfig>,
}

#[derive(Debug, Deserialize)]
pub struct Limits {
    pub max_concurrent_workers: u32,
    /// Maximum attempts per task (the first dispatch counts). `1` means no fallback.
    pub max_retries: u32,
    pub task_timeout_seconds: u64,
    /// Deadline for each harness's version probe. A probe that does not exit in time
    /// (an interactive UI, an auth prompt) marks that harness unsupported.
    #[serde(default = "default_probe_timeout_seconds")]
    pub probe_timeout_seconds: u64,
}

fn default_probe_timeout_seconds() -> u64 {
    15
}

#[derive(Debug, Deserialize, Default)]
pub struct VerifyConfig {
    #[serde(default)]
    pub verify_command: Vec<String>,
}

/// `[workspace]`. `root` is optional. A relative root is resolved from the repository.
/// An absolute root is used as given. Absent means `<repo>/.meshloop/worktrees`.
/// `auto_reclaim` (default `true`) reclaims a landed run that this version owns
/// through digest-bound cleanup once it is integrated and reachable from the base
/// branch (ADR 0033 amendment, issue #142). `false` prints the cleanup command instead.
#[derive(Debug, Deserialize, Clone)]
pub struct WorkspaceConfig {
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default = "default_auto_reclaim")]
    pub auto_reclaim: bool,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            root: None,
            auto_reclaim: default_auto_reclaim(),
        }
    }
}

fn default_auto_reclaim() -> bool {
    true
}

/// `[planner]`: which harness runs `meshloop:planner`. Without either key, planning is
/// routed as a Tier3 task, so it needs a harness with `model_tier = "top"`.
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct PlannerConfig {
    /// A selected harness to plan with, whatever its `model_tier`.
    #[serde(default)]
    pub harness: Option<String>,
    /// The tier planning is routed at: "Tier1", "Tier2" or "Tier3" (default).
    #[serde(default)]
    pub tier: Option<String>,
}

impl PlannerConfig {
    pub fn selection(&self) -> Result<PlannerSelection, ConfigError> {
        if let Some(name) = &self.harness {
            if self.tier.is_some() {
                return Err(ConfigError::Parse(
                    "planner: set either harness or tier, not both".into(),
                ));
            }
            return Ok(PlannerSelection::Harness(name.clone()));
        }
        match self.tier.as_deref() {
            None => Ok(PlannerSelection::default()),
            Some(t) => Tier::ALL
                .into_iter()
                .find(|v| v.name() == t)
                .map(PlannerSelection::Route)
                .ok_or_else(|| {
                    ConfigError::Parse(format!(
                        "planner.tier '{t}' must be one of Tier1, Tier2, Tier3"
                    ))
                }),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct HarnessConfig {
    pub executable: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub kind: Option<String>,
    /// Arguments for the read-only version probe. Defaults to `["--version"]`; never empty,
    /// since a bare invocation of most harness CLIs opens an interactive session.
    #[serde(default = "default_version_args")]
    pub version_args: Vec<String>,
    #[serde(default)]
    pub invoke_args_template: Vec<String>,
    /// `file` (default), `arg`, or `stdin`. `load` rejects any other value and names the harness.
    #[serde(default = "default_prompt_via")]
    pub prompt_via: String,
    /// Single-model form: the one model this harness runs. Use either this pair or
    /// `models`, never both. `load` rejects an entry with neither.
    #[serde(default)]
    pub model_ref: Option<String>,
    /// Tier of `model_ref`; `"mid"` when omitted. Only with `model_ref`.
    #[serde(default)]
    pub model_tier: Option<String>,
    /// List form (issue #134, ADR 0038): one harness entry, several models. Routing
    /// expands each into its own (harness, model_ref, tier) candidate and the chosen
    /// `model_ref` is substituted into `{model_ref}` in `invoke_args_template`.
    #[serde(default)]
    pub models: Option<Vec<ModelEntry>>,
}

/// One model of a harness entry's `models` list.
#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModelEntry {
    pub model_ref: String,
    /// `"top"`, `"mid"` (default) or anything else for lightweight, as `model_tier`.
    #[serde(default = "default_model_tier")]
    pub model_tier: String,
}

/// Upper bound on one harness entry's `models` list. Each model is a routing arm with
/// its own feedback row, so the list stays small and bounded.
pub const MAX_MODELS_PER_HARNESS: usize = 16;
/// Upper bound on one `model_ref` in a `models` list, in bytes.
const MAX_MODEL_REF_BYTES: usize = 256;

impl HarnessConfig {
    /// The models this entry routes, in configuration order: the `models` list, or the
    /// single `model_ref` with its `model_tier`. Empty only for an entry `load` rejects.
    pub fn model_entries(&self) -> Vec<ModelEntry> {
        match (&self.models, &self.model_ref) {
            (Some(models), _) => models.clone(),
            (None, Some(model_ref)) => vec![ModelEntry {
                model_ref: model_ref.clone(),
                model_tier: self.model_tier.clone().unwrap_or_else(default_model_tier),
            }],
            (None, None) => Vec::new(),
        }
    }
}

/// Checks the model form of one harness entry. The single `model_ref` form keeps its
/// pre-#134 behaviour (no new checks); the `models` list is validated in full.
fn validate_models(name: &str, hc: &HarnessConfig) -> Result<(), ConfigError> {
    let err = |msg: String| Err(ConfigError::Parse(format!("harness '{name}': {msg}")));
    let Some(models) = &hc.models else {
        if hc.model_ref.is_none() {
            return err("model_ref or models is required".into());
        }
        return Ok(());
    };
    if hc.model_ref.is_some() || hc.model_tier.is_some() {
        return err("set either model_ref/model_tier or models, not both".into());
    }
    if models.is_empty() {
        return err("models must not be empty".into());
    }
    if models.len() > MAX_MODELS_PER_HARNESS {
        return err(format!(
            "models lists {} entries; at most {MAX_MODELS_PER_HARNESS} are allowed",
            models.len()
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for (i, m) in models.iter().enumerate() {
        let r = &m.model_ref;
        if r.trim().is_empty() {
            return err(format!("models[{i}].model_ref must not be empty"));
        }
        if r.len() > MAX_MODEL_REF_BYTES {
            return err(format!(
                "models[{i}].model_ref is longer than {MAX_MODEL_REF_BYTES} bytes"
            ));
        }
        // The ref becomes one argv element; a leading '-' would read as a harness flag.
        if r.starts_with('-') {
            return err(format!(
                "models[{i}].model_ref '{r}' must not start with '-'"
            ));
        }
        if r.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return err(format!(
                "models[{i}].model_ref must not contain whitespace or control characters"
            ));
        }
        if !seen.insert(r.as_str()) {
            return err(format!("duplicate model_ref '{r}' in models"));
        }
    }
    Ok(())
}

fn default_model_tier() -> String {
    "mid".into()
}

fn default_version_args() -> Vec<String> {
    vec!["--version".into()]
}

fn default_prompt_via() -> String {
    "file".into()
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
    for (name, hc) in &config.harnesses {
        if hc.version_args.is_empty() {
            return Err(ConfigError::Parse(format!(
                "harness '{name}': version_args must not be empty (omit it to use [\"--version\"])"
            )));
        }
        if PromptVia::parse(&hc.prompt_via).is_none() {
            return Err(ConfigError::Parse(format!(
                "harness '{name}': prompt_via '{}' must be one of file, arg, stdin",
                hc.prompt_via
            )));
        }
        validate_models(name, hc)?;
    }
    if let Some(name) = &config.planner.harness
        && !config.selected_harnesses.contains(name)
    {
        return Err(ConfigError::Parse(format!(
            "planner.harness '{name}' is not in selected_harnesses"
        )));
    }
    config.planner.selection()?;
    if config.limits.probe_timeout_seconds == 0 {
        return Err(ConfigError::Parse(
            "limits.probe_timeout_seconds must be greater than 0".into(),
        ));
    }
    Ok(config)
}

/// Selected harnesses that can never be dispatched as configured, with the reason. These
/// load fine (doctor and probes still report on them) but routing will always skip them.
pub fn dispatch_warnings(config: &Config) -> Vec<(String, String)> {
    config
        .selected_harnesses
        .iter()
        .filter_map(|name| {
            let hc = config.harnesses.get(name)?;
            hc.invoke_args_template.is_empty().then(|| {
                (
                    name.clone(),
                    "no invoke_args_template configured; it will be probed but never dispatched"
                        .to_string(),
                )
            })
        })
        .collect()
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
            assert!(!hc.version_args.is_empty(), "{name}: version_args");
        }
        assert_eq!(config.harnesses["grok"].prompt_via, "file");
        assert_eq!(
            config.harnesses["grok"].invoke_args_template,
            vec!["--prompt-file", "{prompt_file}"]
        );
        assert_eq!(config.harnesses["claude"].prompt_via, "arg");
        assert_eq!(
            config.harnesses["claude"].invoke_args_template,
            vec!["--print", "{prompt}"]
        );
        assert_eq!(config.harnesses["codex"].prompt_via, "stdin");
        assert_eq!(
            config.harnesses["codex"].invoke_args_template,
            vec!["exec", "-"]
        );
        assert_eq!(config.harnesses["pi"].prompt_via, "arg");
        assert_eq!(
            config.harnesses["pi"].invoke_args_template,
            vec!["-p", "{prompt}"]
        );
        assert_eq!(config.harnesses["agy"].prompt_via, "file");
        assert!(config.harnesses["agy"].invoke_args_template.is_empty());
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

    fn write_temp(tag: &str, toml: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("meshloop-cfg-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("meshloop.toml");
        std::fs::write(&path, toml).unwrap();
        path
    }

    const MINIMAL: &str = r#"
        selected_harnesses = ["grok"]
        [limits]
        max_concurrent_workers = 1
        max_retries = 1
        task_timeout_seconds = 60
        [harnesses.grok]
        kind = "grok"
        executable = "grok"
        model_ref = "grok"
        model_tier = "top"
    "#;

    fn load_with(tag: &str, extra: &str) -> Result<Config, ConfigError> {
        load(&write_temp(
            tag,
            &format!(
                "{MINIMAL}
{extra}"
            ),
        ))
    }

    #[test]
    fn planner_section_selects_harness_or_tier_and_defaults_to_tier3() {
        // Issue #54: planning was hardcoded to Tier3, which only model_tier = "top" fits.
        let c = load_with("planner-none", "").expect("no [planner]");
        assert_eq!(
            c.planner.selection().unwrap(),
            PlannerSelection::Route(Tier::Tier3)
        );
        let c = load_with(
            "planner-harness",
            "[planner]
harness = \"grok\"",
        )
        .expect("harness");
        assert_eq!(
            c.planner.selection().unwrap(),
            PlannerSelection::Harness("grok".into())
        );
        let c = load_with(
            "planner-tier",
            "[planner]
tier = \"Tier2\"",
        )
        .expect("tier");
        assert_eq!(
            c.planner.selection().unwrap(),
            PlannerSelection::Route(Tier::Tier2)
        );
    }

    #[test]
    fn planner_section_rejects_bad_values() {
        for (tag, extra, needle) in [
            (
                "p-unsel",
                "[planner]
harness = \"codex\"",
                "not in selected_harnesses",
            ),
            (
                "p-tier",
                "[planner]
tier = \"top\"",
                "must be one of Tier1, Tier2, Tier3",
            ),
            (
                "p-both",
                "[planner]
harness = \"grok\"
tier = \"Tier1\"",
                "either harness or tier",
            ),
            (
                "p-key",
                "[planner]
model = \"x\"",
                "unknown field",
            ),
        ] {
            match load_with(tag, extra) {
                Err(ConfigError::Parse(msg)) => assert!(msg.contains(needle), "{tag}: {msg}"),
                other => panic!("{tag}: expected a parse error, got {other:?}"),
            }
        }
    }

    #[test]
    fn workspace_auto_reclaim_defaults_on_and_can_be_switched_off() {
        let omitted = load_with("reclaim-omitted", "").unwrap();
        assert!(omitted.workspace.auto_reclaim);
        assert!(omitted.workspace.root.is_none());
        let table = load_with("reclaim-table", "[workspace]\nroot = \"short\"").unwrap();
        assert!(table.workspace.auto_reclaim);
        let off = load_with("reclaim-off", "[workspace]\nauto_reclaim = false").unwrap();
        assert!(!off.workspace.auto_reclaim);
        assert!(matches!(
            load_with("reclaim-bad", "[workspace]\nauto_reclaim = \"yes\""),
            Err(ConfigError::Parse(_))
        ));
        for tag in [
            "reclaim-omitted",
            "reclaim-table",
            "reclaim-off",
            "reclaim-bad",
        ] {
            let dir =
                std::env::temp_dir().join(format!("meshloop-cfg-{tag}-{}", std::process::id()));
            std::fs::remove_dir_all(dir).ok();
        }
    }

    #[test]
    fn omitted_version_args_default_to_version_flag_and_probe_timeout_is_bounded() {
        // Issue #9: the minimal repro config used to probe the bare executable forever.
        let path = write_temp("defaults", MINIMAL);
        let config = load(&path).expect("minimal config loads");
        assert_eq!(config.harnesses["grok"].version_args, vec!["--version"]);
        assert_eq!(config.limits.probe_timeout_seconds, 15);
        assert_eq!(config.harnesses["grok"].prompt_via, "file");
        let warnings = dispatch_warnings(&config);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].0, "grok");
        assert!(warnings[0].1.contains("invoke_args_template"));
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn prompt_via_accepts_only_file_arg_or_stdin_and_names_the_harness() {
        for value in ["file", "arg", "stdin"] {
            let toml = MINIMAL.replace(
                "model_tier = \"top\"",
                &format!("model_tier = \"top\"\nprompt_via = \"{value}\""),
            );
            let path = write_temp(&format!("prompt-via-{value}"), &toml);
            let config = load(&path).expect(value);
            assert_eq!(config.harnesses["grok"].prompt_via, value);
            std::fs::remove_dir_all(path.parent().unwrap()).ok();
        }
        let toml = MINIMAL.replace(
            "model_tier = \"top\"",
            "model_tier = \"top\"\nprompt_via = \"pipe\"",
        );
        let path = write_temp("prompt-via-bad", &toml);
        let result = load(&path);
        assert!(
            matches!(&result, Err(ConfigError::Parse(m)) if m.contains("grok") && m.contains("prompt_via") && m.contains("pipe")),
            "{result:?}"
        );
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn explicitly_empty_version_args_are_rejected() {
        let toml = MINIMAL.replace(
            "model_tier = \"top\"",
            "model_tier = \"top\"
version_args = []",
        );
        let path = write_temp("empty-version", &toml);
        let result = load(&path);
        assert!(
            matches!(&result, Err(ConfigError::Parse(m)) if m.contains("grok") && m.contains("version_args")),
            "{result:?}"
        );
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn zero_probe_timeout_is_rejected() {
        let toml = MINIMAL.replace(
            "task_timeout_seconds = 60",
            "task_timeout_seconds = 60
probe_timeout_seconds = 0",
        );
        let path = write_temp("zero-probe", &toml);
        assert!(matches!(load(&path), Err(ConfigError::Parse(_))));
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    /// MINIMAL with its single `model_ref`/`model_tier` replaced by `models_toml`.
    fn with_models(tag: &str, models_toml: &str) -> Result<Config, ConfigError> {
        let toml = MINIMAL.replace(
            "model_ref = \"grok\"\n        model_tier = \"top\"",
            models_toml,
        );
        assert_ne!(toml, MINIMAL, "{tag}: MINIMAL model lines not found");
        let path = write_temp(tag, &toml);
        let result = load(&path);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
        result
    }

    #[test]
    fn legacy_single_model_ref_loads_unchanged_as_one_model() {
        // Issue #134: configs written before `models` existed keep loading as they did.
        let config = load_with("legacy-model", "").expect("legacy config");
        let hc = &config.harnesses["grok"];
        assert_eq!(hc.model_ref.as_deref(), Some("grok"));
        assert_eq!(hc.model_tier.as_deref(), Some("top"));
        assert!(hc.models.is_none());
        assert_eq!(
            hc.model_entries(),
            vec![ModelEntry {
                model_ref: "grok".into(),
                model_tier: "top".into()
            }]
        );
        // An omitted legacy model_tier still defaults to "mid".
        let c = with_models("legacy-default-tier", "model_ref = \"grok\"").expect("loads");
        assert_eq!(c.harnesses["grok"].model_entries()[0].model_tier, "mid");
    }

    #[test]
    fn models_list_expands_in_configuration_order_with_per_model_tier() {
        let config = with_models(
            "models-list",
            r#"models = [
                { model_ref = "grok-4-fast", model_tier = "mid" },
                { model_ref = "grok-4" , model_tier = "top" },
                { model_ref = "grok-mini" },
            ]"#,
        )
        .expect("models list loads");
        let entries = config.harnesses["grok"].model_entries();
        let pairs: Vec<(&str, &str)> = entries
            .iter()
            .map(|m| (m.model_ref.as_str(), m.model_tier.as_str()))
            .collect();
        assert_eq!(
            pairs,
            [
                ("grok-4-fast", "mid"),
                ("grok-4", "top"),
                ("grok-mini", "mid")
            ]
        );
    }

    #[test]
    fn models_list_rejects_bad_shapes_and_names_the_harness() {
        let many: String = (0..=MAX_MODELS_PER_HARNESS)
            .map(|i| format!("{{ model_ref = \"m{i}\" }},"))
            .collect();
        let too_many = format!("models = [{many}]");
        for (tag, models, needle) in [
            (
                "m-both",
                "model_ref = \"grok\"\nmodels = [{ model_ref = \"a\" }]",
                "not both",
            ),
            (
                "m-both-tier",
                "model_tier = \"top\"\nmodels = [{ model_ref = \"a\" }]",
                "not both",
            ),
            ("m-neither", "", "model_ref or models"),
            ("m-empty", "models = []", "must not be empty"),
            (
                "m-blank",
                "models = [{ model_ref = \"  \" }]",
                "must not be empty",
            ),
            (
                "m-dup",
                "models = [{ model_ref = \"a\", model_tier = \"top\" }, { model_ref = \"a\", model_tier = \"mid\" }]",
                "duplicate model_ref 'a'",
            ),
            (
                "m-flag",
                "models = [{ model_ref = \"--dangerously-skip\" }]",
                "must not start with '-'",
            ),
            (
                "m-space",
                "models = [{ model_ref = \"a b\" }]",
                "whitespace or control",
            ),
            ("m-many", too_many.as_str(), "at most"),
        ] {
            match with_models(tag, models) {
                Err(ConfigError::Parse(msg)) => {
                    assert!(msg.contains(needle), "{tag}: {msg}");
                    assert!(msg.contains("grok"), "{tag} must name the harness: {msg}");
                }
                other => panic!("{tag}: expected a parse error, got {other:?}"),
            }
        }
        // Unknown keys inside a model entry are rejected, not ignored.
        match with_models("m-key", "models = [{ model_ref = \"a\", tier = \"top\" }]") {
            Err(ConfigError::Parse(msg)) => assert!(msg.contains("unknown field"), "{msg}"),
            other => panic!("m-key: expected a parse error, got {other:?}"),
        }
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
