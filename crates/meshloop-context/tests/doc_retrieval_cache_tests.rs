use meshloop_context::quant::signature_fingerprint;
use meshloop_context::{PromptCacheBuilder, select_context};

#[test]
fn cycle2_mixed_code_and_doc_quantized_ranking() {
    let auth_rs = (
        "src/auth.rs".to_string(),
        "pub trait Auth { fn verify(&self); }\npub struct AuthService;".to_string(),
    );
    let adr_0029 = (
        "docs/architecture/adr/0029-deterministic-loop-algorithms.md".to_string(),
        "# 0029 Deterministic Loop Algorithms\n## Context\nQuantized signature retrieval using 1-bit and 2-bit codes.\n## Decision\nUse Walsh-Hadamard rotation."
            .to_string(),
    );
    let store_rs = (
        "src/store.rs".to_string(),
        "pub struct Store { pub db: String }".to_string(),
    );

    let candidates = vec![auth_rs.clone(), adr_0029.clone(), store_rs.clone()];

    // Query targeted at code
    let code_ranked = select_context("implement Auth trait and verify logic", &candidates, &[], 2);
    assert!(!code_ranked.is_empty());
    assert_eq!(
        code_ranked[0].0, "src/auth.rs",
        "Code query must rank auth.rs first"
    );

    // Query targeted at ADR
    let doc_ranked = select_context(
        "quantized signature retrieval and Walsh-Hadamard rotation",
        &candidates,
        &[],
        2,
    );
    assert!(!doc_ranked.is_empty());
    assert_eq!(
        doc_ranked[0].0, "docs/architecture/adr/0029-deterministic-loop-algorithms.md",
        "Doc query must rank the matching ADR first"
    );
}

#[test]
fn cycle2_prompt_cache_builder_byte_identity_under_permutation() {
    let rust_skel = "pub trait Engine { fn run(&self); }";
    let doc_skel = "# ADR 0031\n- Status: Accepted\n## Decision\n- Rule 1";

    let p1 = PromptCacheBuilder::new()
        .with_system_rule("Safe Rust only.")
        .with_contract("Output JSON")
        .with_ast_skeleton("crates/meshloop-engine/src/lib.rs", rust_skel)
        .with_ast_skeleton("docs/adr/0031.md", doc_skel)
        .build("Task A", "Action A");

    let p2 = PromptCacheBuilder::new()
        .with_system_rule("Safe Rust only.")
        .with_contract("Output JSON")
        .with_ast_skeleton("docs/adr/0031.md", doc_skel)
        .with_ast_skeleton("crates/meshloop-engine/src/lib.rs", rust_skel)
        .build("Task B", "Action B");

    // Static prefix must be bitwise identical regardless of insertion order
    assert_eq!(
        p1.static_prefix, p2.static_prefix,
        "Static prefix must be bitwise identical under permutation"
    );
}

#[test]
fn cycle2_signature_fingerprint_doc_body_edit_is_invariant() {
    let doc_v1 = r#"# ADR 0029 Deterministic Loop Algorithms
- Status: Accepted

## Context
First paragraph of historical context explaining why we chose this.
Another long narrative explanation that gets pruned.

## Decision
- Rule A
"#;

    let doc_v2 = r#"# ADR 0029 Deterministic Loop Algorithms
- Status: Accepted

## Context
Completely rewritten narrative paragraph with different wording.
Still within the pruned context body.

## Decision
- Rule A
"#;

    let fp1 = signature_fingerprint("docs/adr/0029.md", doc_v1);
    let fp2 = signature_fingerprint("docs/adr/0029.md", doc_v2);

    assert_eq!(
        fp1, fp2,
        "Editing narrative body in markdown doc must not change signature fingerprint"
    );
}
