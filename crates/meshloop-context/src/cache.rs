//! Prompt cache normalizer.
//! Enforces deterministic ordering of prompt segments so that static context
//! (system instructions, tool contracts, immutable AST skeletons) hits provider
//! prompt caches (Anthropic, Gemini, DeepSeek) with >80% hit ratios.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheOptimizedPrompt {
    pub static_prefix: String,
    pub dynamic_suffix: String,
}

impl CacheOptimizedPrompt {
    /// Renders the complete prompt string with zero whitespace drift.
    pub fn render(&self) -> String {
        format!(
            "{}\n\n{}",
            self.static_prefix.trim(),
            self.dynamic_suffix.trim()
        )
    }

    /// Measures the percentage of the prompt eligible for prompt caching.
    pub fn cache_eligible_ratio(&self) -> f64 {
        let static_len = self.static_prefix.len();
        let total = static_len + self.dynamic_suffix.len();
        if total == 0 {
            return 0.0;
        }
        (static_len as f64 / total as f64) * 100.0
    }
}

pub struct PromptCacheBuilder {
    system_rules: Vec<String>,
    contracts: Vec<String>,
    ast_skeletons: Vec<(String, String)>, // (file_path, skeleton_content)
}

impl PromptCacheBuilder {
    pub fn new() -> Self {
        Self {
            system_rules: Vec::new(),
            contracts: Vec::new(),
            ast_skeletons: Vec::new(),
        }
    }

    pub fn with_system_rule(mut self, rule: impl Into<String>) -> Self {
        self.system_rules.push(rule.into());
        self
    }

    pub fn with_contract(mut self, contract: impl Into<String>) -> Self {
        self.contracts.push(contract.into());
        self
    }

    pub fn with_ast_skeleton(
        mut self,
        file_path: impl Into<String>,
        skeleton: impl Into<String>,
    ) -> Self {
        self.ast_skeletons.push((file_path.into(), skeleton.into()));
        self
    }

    /// Compiles into a cache-optimized prompt envelope.
    pub fn build(mut self, task_description: &str, task_question: &str) -> CacheOptimizedPrompt {
        // Sort skeletons deterministically by file path to prevent cache busting
        self.ast_skeletons.sort_by(|a, b| a.0.cmp(&b.0));

        let mut static_parts = Vec::new();

        if !self.system_rules.is_empty() {
            static_parts.push(format!(
                "### SYSTEM POLICIES\n{}",
                self.system_rules.join("\n")
            ));
        }

        if !self.contracts.is_empty() {
            static_parts.push(format!(
                "### OUTPUT CONTRACTS\n{}",
                self.contracts.join("\n")
            ));
        }

        if !self.ast_skeletons.is_empty() {
            let mut skeletons_text = Vec::new();
            for (path, content) in &self.ast_skeletons {
                skeletons_text.push(format!("// File: {path}\n{content}"));
            }
            static_parts.push(format!(
                "### REPOSITORY CONTRACT SKELETONS\n{}",
                skeletons_text.join("\n\n")
            ));
        }

        let static_prefix = static_parts.join("\n\n");

        let dynamic_suffix = format!(
            "### TASK ASSIGNMENT\n{}\n\n### ACTION REQUIRED\n{}",
            task_description.trim(),
            task_question.trim()
        );

        CacheOptimizedPrompt {
            static_prefix,
            dynamic_suffix,
        }
    }
}

impl Default for PromptCacheBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_sorting_preserves_static_prefix() {
        let p1 = PromptCacheBuilder::new()
            .with_system_rule("Safe Rust only.")
            .with_ast_skeleton("src/b.rs", "pub struct B;")
            .with_ast_skeleton("src/a.rs", "pub struct A;")
            .build("Do task 1", "Implement A");

        let p2 = PromptCacheBuilder::new()
            .with_system_rule("Safe Rust only.")
            .with_ast_skeleton("src/a.rs", "pub struct A;")
            .with_ast_skeleton("src/b.rs", "pub struct B;")
            .build("Do task 2", "Implement B");

        // The static prefix must be identical byte-for-byte to trigger prompt caching
        assert_eq!(p1.static_prefix, p2.static_prefix);
        assert!(p1.cache_eligible_ratio() > 50.0);
    }
}
