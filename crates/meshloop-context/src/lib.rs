//! Context engineering engine for Meshloop.
//! Handles AST skeleton extraction, Tier 1 bulk-reader dynamic resolution,
//! and prompt cache normalization.

pub mod cache;
pub mod skeleton;
pub mod tier1;

pub use cache::{CacheOptimizedPrompt, PromptCacheBuilder};
pub use skeleton::{Language, SkeletonResult, estimate_tokens, extract_skeleton};
pub use tier1::{
    CliTier1Overrides, SelectionSource, Tier1Config, Tier1ProviderKind,
    build_delegated_reader_prompt, resolve_tier1_provider,
};
