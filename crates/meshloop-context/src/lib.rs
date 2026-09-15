//! Context engineering engine for Meshloop.
//! Handles AST skeleton extraction, Tier 1 bulk-reader dynamic resolution,
//! prompt cache normalization, and data-oblivious signature retrieval.

pub mod cache;
pub mod quant;
pub mod signature;
pub mod skeleton;
pub mod tier1;

pub use cache::{CacheOptimizedPrompt, PromptCacheBuilder};
pub use quant::{BitWidth, DEFAULT_SKELETON_BUDGET, PackedCode, SignatureIndex, select_context};
pub use signature::{Signature, SignatureKind, extract_signatures};
pub use skeleton::{Language, SkeletonResult, estimate_tokens, extract_skeleton};
pub use tier1::{
    CliTier1Overrides, SelectionSource, Tier1Config, Tier1ProviderKind,
    build_delegated_reader_prompt, resolve_tier1_provider,
};
