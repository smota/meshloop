//! Tier 1 (Bulk Reader) dynamic resolution and delegated I/O reader prompt construction.
//! Supports 4 levels of precedence: CLI flag > Env var > Config TOML > Auto-detection.
//! Supports both subscription harnesses (agy, claude, codex) and direct/local models.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tier1ProviderKind {
    /// Authenticated CLI harness consuming existing user subscription (agy, claude, codex, etc.)
    Harness,
    /// Gemini API (1M+ token window, lowest cost)
    Gemini,
    /// Anthropic API (Haiku)
    Anthropic,
    /// DeepSeek API (V3)
    DeepSeek,
    /// Local Ollama instance (free, unlimited, offline)
    Ollama,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tier1Config {
    pub kind: Tier1ProviderKind,
    pub model: String,
    pub harness_name: Option<String>,
    pub endpoint: Option<String>,
    pub source: SelectionSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectionSource {
    CliFlag,
    EnvironmentVariable,
    ConfigFile,
    AutoDetected,
    DefaultFallback,
}

/// Options passed from runtime CLI invocation.
#[derive(Debug, Clone, Default)]
pub struct CliTier1Overrides {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub harness: Option<String>,
}

/// Resolves the Tier 1 provider based on the 4-tier precedence rule.
pub fn resolve_tier1_provider(
    cli: &CliTier1Overrides,
    config_provider: Option<&str>,
    config_model: Option<&str>,
    config_harness: Option<&str>,
    env_lookup: impl Fn(&str) -> Option<String>,
) -> Tier1Config {
    // 1. Check CLI flags
    if let Some(harness) = &cli.harness {
        return Tier1Config {
            kind: Tier1ProviderKind::Harness,
            model: cli.model.clone().unwrap_or_else(|| "default".into()),
            harness_name: Some(harness.clone()),
            endpoint: None,
            source: SelectionSource::CliFlag,
        };
    }
    if let Some(provider) = &cli.provider {
        let (kind, default_model) = parse_provider_kind(provider);
        return Tier1Config {
            kind,
            model: cli.model.clone().unwrap_or(default_model),
            harness_name: None,
            endpoint: None,
            source: SelectionSource::CliFlag,
        };
    }

    // 2. Check Environment Variables
    if let Some(harness) = env_lookup("MESHLOOP_TIER1_HARNESS") {
        let model = env_lookup("MESHLOOP_TIER1_MODEL").unwrap_or_else(|| "default".into());
        return Tier1Config {
            kind: Tier1ProviderKind::Harness,
            model,
            harness_name: Some(harness),
            endpoint: None,
            source: SelectionSource::EnvironmentVariable,
        };
    }
    if let Some(provider) = env_lookup("MESHLOOP_TIER1_PROVIDER") {
        let (kind, default_model) = parse_provider_kind(&provider);
        let model = env_lookup("MESHLOOP_TIER1_MODEL").unwrap_or(default_model);
        return Tier1Config {
            kind,
            model,
            harness_name: None,
            endpoint: None,
            source: SelectionSource::EnvironmentVariable,
        };
    }

    // 3. Check Configuration File (TOML)
    if let Some(harness) = config_harness {
        return Tier1Config {
            kind: Tier1ProviderKind::Harness,
            model: config_model.unwrap_or("default").into(),
            harness_name: Some(harness.into()),
            endpoint: None,
            source: SelectionSource::ConfigFile,
        };
    }
    if let Some(provider) = config_provider {
        let (kind, default_model) = parse_provider_kind(provider);
        return Tier1Config {
            kind,
            model: config_model.map(str::to_string).unwrap_or(default_model),
            harness_name: None,
            endpoint: None,
            source: SelectionSource::ConfigFile,
        };
    }

    // 4. Auto-detection from environment API keys
    if env_lookup("GEMINI_API_KEY").is_some() {
        return Tier1Config {
            kind: Tier1ProviderKind::Gemini,
            model: "gemini-2.5-flash".into(),
            harness_name: None,
            endpoint: None,
            source: SelectionSource::AutoDetected,
        };
    }
    if env_lookup("ANTHROPIC_API_KEY").is_some() {
        return Tier1Config {
            kind: Tier1ProviderKind::Anthropic,
            model: "claude-3-5-haiku".into(),
            harness_name: None,
            endpoint: None,
            source: SelectionSource::AutoDetected,
        };
    }
    if env_lookup("DEEPSEEK_API_KEY").is_some() {
        return Tier1Config {
            kind: Tier1ProviderKind::DeepSeek,
            model: "deepseek-chat".into(),
            harness_name: None,
            endpoint: None,
            source: SelectionSource::AutoDetected,
        };
    }

    // 5. Default Fallback: Local Ollama (zero-cost, private)
    Tier1Config {
        kind: Tier1ProviderKind::Ollama,
        model: "qwen2.5-coder".into(),
        harness_name: None,
        endpoint: Some("http://localhost:11434".into()),
        source: SelectionSource::DefaultFallback,
    }
}

fn parse_provider_kind(raw: &str) -> (Tier1ProviderKind, String) {
    match raw.to_lowercase().as_str() {
        "gemini" | "google" => (Tier1ProviderKind::Gemini, "gemini-2.5-flash".into()),
        "anthropic" | "claude" | "haiku" => {
            (Tier1ProviderKind::Anthropic, "claude-3-5-haiku".into())
        }
        "deepseek" => (Tier1ProviderKind::DeepSeek, "deepseek-chat".into()),
        "ollama" | "local" => (Tier1ProviderKind::Ollama, "qwen2.5-coder".into()),
        harness => (Tier1ProviderKind::Harness, format!("harness:{harness}")),
    }
}

/// Builds the delegated reader prompt for the bulk-reader model (Tier 1).
pub fn build_delegated_reader_prompt(
    objective: &str,
    raw_content: &str,
    target_summary_tokens: usize,
) -> String {
    format!(
        "OBJECTIVE:\nExtract and condense the essential architectural contracts, types, and schema dependencies needed to satisfy:\n{}\n\n\
         CONSTRAINTS:\n- Do not include implementation logic, internal bodies, or chatty descriptions.\n\
         - Output strict, machine-readable specifications and signatures only.\n\
         - Target max output size: ~{} tokens.\n\n\
         RAW REPOSITORY CONTENT:\n{}\n",
        objective.trim(),
        target_summary_tokens,
        raw_content.trim()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_flag_has_highest_precedence() {
        let cli = CliTier1Overrides {
            provider: Some("gemini".into()),
            model: Some("gemini-custom".into()),
            harness: None,
        };
        let cfg = resolve_tier1_provider(&cli, Some("anthropic"), Some("haiku"), None, |k| {
            if k == "MESHLOOP_TIER1_PROVIDER" {
                Some("deepseek".into())
            } else {
                None
            }
        });
        assert_eq!(cfg.kind, Tier1ProviderKind::Gemini);
        assert_eq!(cfg.model, "gemini-custom");
        assert_eq!(cfg.source, SelectionSource::CliFlag);
    }

    #[test]
    fn harness_subscription_selected_via_cli() {
        let cli = CliTier1Overrides {
            provider: None,
            model: None,
            harness: Some("agy".into()),
        };
        let cfg = resolve_tier1_provider(&cli, None, None, None, |_| None);
        assert_eq!(cfg.kind, Tier1ProviderKind::Harness);
        assert_eq!(cfg.harness_name.as_deref(), Some("agy"));
        assert_eq!(cfg.source, SelectionSource::CliFlag);
    }

    #[test]
    fn env_var_precedence_over_config_file() {
        let cli = CliTier1Overrides::default();
        let cfg = resolve_tier1_provider(&cli, Some("ollama"), Some("llama3"), None, |k| {
            if k == "MESHLOOP_TIER1_PROVIDER" {
                Some("deepseek".into())
            } else {
                None
            }
        });
        assert_eq!(cfg.kind, Tier1ProviderKind::DeepSeek);
        assert_eq!(cfg.source, SelectionSource::EnvironmentVariable);
    }

    #[test]
    fn autodetects_gemini_when_api_key_present() {
        let cli = CliTier1Overrides::default();
        let cfg = resolve_tier1_provider(&cli, None, None, None, |k| {
            if k == "GEMINI_API_KEY" {
                Some("secret".into())
            } else {
                None
            }
        });
        assert_eq!(cfg.kind, Tier1ProviderKind::Gemini);
        assert_eq!(cfg.model, "gemini-2.5-flash");
        assert_eq!(cfg.source, SelectionSource::AutoDetected);
    }

    #[test]
    fn falls_back_to_local_ollama_when_no_credentials() {
        let cli = CliTier1Overrides::default();
        let cfg = resolve_tier1_provider(&cli, None, None, None, |_| None);
        assert_eq!(cfg.kind, Tier1ProviderKind::Ollama);
        assert_eq!(cfg.source, SelectionSource::DefaultFallback);
    }
}
