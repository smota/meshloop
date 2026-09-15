use std::fs;
use std::path::Path;

use meshloop_context::doc_skeleton::prune_markdown;
use meshloop_context::{Language, extract_skeleton};

#[test]
fn cycle1_dialect_fixture_table() {
    // 1. Unclosed code fence: linear, doesn't panic or loop
    let unclosed = "# Unclosed Fence\n```rust\nfn main() {\n";
    let p_unclosed = prune_markdown(unclosed);
    assert!(p_unclosed.contains("# Unclosed Fence"));
    assert!(p_unclosed.contains("```rust"));

    // 2. Fence containing # headings: must not parse as Markdown headings
    let fence_hashes = "# Real Heading\n```markdown\n# Not A Heading\n## Nor This\n```\n";
    let p_fence = prune_markdown(fence_hashes);
    assert!(p_fence.contains("# Real Heading"));
    let sigs = meshloop_context::extract_signatures("test.md", &p_fence, Language::Markdown);
    assert_eq!(sigs.len(), 1);
    assert_eq!(sigs[0].name, "Real Heading");

    // 3. --- as frontmatter vs Setext vs hr
    let mixed_dashes = "---\ntitle: Frontmatter\n---\n# Title\nSetext Title\n---\nText\n";
    let p_dashes = prune_markdown(mixed_dashes);
    assert!(p_dashes.contains("title: Frontmatter"));
    assert!(p_dashes.contains("# Title"));
    assert!(p_dashes.contains("Setext Title"));

    // 4. Empty file
    let empty = "";
    assert_eq!(prune_markdown(empty), "");

    // 5. CRLF and BOM
    let crlf_bom = "\u{feff}# BOM Heading\r\n- Status: Accepted\r\n\r\nParagraph\r\n";
    let p_crlf = prune_markdown(crlf_bom);
    assert!(p_crlf.contains("# BOM Heading"));
    assert!(p_crlf.contains("- Status: Accepted"));
    assert!(!p_crlf.contains('\r')); // All normalized to LF

    // 6. ATX with closing hashes
    let closing_hashes = "## Architecture Section ##\nProse line.\n";
    let p_closing = prune_markdown(closing_hashes);
    assert!(p_closing.contains("## Architecture Section ##"));

    // 7. GFM admonition
    let admonition = "> [!WARNING]\n> High risk security operation.\n";
    let p_admon = prune_markdown(admonition);
    assert!(p_admon.contains("> [!WARNING]"));
    assert!(p_admon.contains("> High risk security operation."));

    // 8. Mermaid fence
    let mermaid = "```mermaid\ngraph TD\n  A --> B\n```\n";
    let p_mermaid = prune_markdown(mermaid);
    assert!(p_mermaid.contains("```mermaid"));
    assert!(p_mermaid.contains("graph TD"));

    // 9. Meshloop ADR metadata
    let adr_meta = "- Status: Accepted\n- Implementation: implemented\n- Date: 2026-09-15\n";
    let p_meta = prune_markdown(adr_meta);
    assert!(p_meta.contains("- Status: Accepted"));
    assert!(p_meta.contains("- Implementation: implemented"));
    assert!(p_meta.contains("- Date: 2026-09-15"));

    // 10. Unicode headings
    let unicode = "# 🚀 Introdução ao Motor e Execução\n## Restrições & Métricas\n";
    let p_uni = prune_markdown(unicode);
    assert!(p_uni.contains("# 🚀 Introdução ao Motor e Execução"));
    assert!(p_uni.contains("## Restrições & Métricas"));
    let uni_sigs = meshloop_context::extract_signatures("doc.md", &p_uni, Language::Markdown);
    assert_eq!(uni_sigs.len(), 2);
    assert_eq!(uni_sigs[0].name, "🚀 Introdução ao Motor e Execução");
    assert_eq!(uni_sigs[1].name, "Restrições & Métricas");

    // 11. Nested lists in allowlisted sections
    let nested_list = "## Decision\n- 1. Core runtime\n  - safe Rust\n  - daemonless\n";
    let p_list = prune_markdown(nested_list);
    assert!(p_list.contains("## Decision"));
    assert!(p_list.contains("- 1. Core runtime"));

    // 12. Indented fence
    let indented_fence = "  ```rust\n  let val = 42;\n  ```\n";
    let p_indent = prune_markdown(indented_fence);
    assert!(p_indent.contains("```rust"));
}

#[test]
fn cycle1_golden_real_meshloop_adrs() {
    let adr_files = [
        "docs/architecture/adr/0029-deterministic-loop-algorithms.md",
        "docs/architecture/adr/0030-petgraph-dag-engine.md",
        "docs/architecture/adr/0031-markdown-doc-ast-context-engineering.md",
        "docs/architecture/adr/template.md",
    ];

    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();

    for rel_path in &adr_files {
        let full_path = repo_root.join(rel_path);
        if !full_path.exists() {
            continue;
        }

        let content = fs::read_to_string(&full_path).expect("Failed to read ADR");
        let result = extract_skeleton(&content, Language::Markdown);

        // 1. Structure preservation: All ## headings in the original must exist in the pruned skeleton
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("## ") {
                assert!(
                    result.skeleton.contains(trimmed),
                    "ADR {rel_path} missing heading in skeleton: {trimmed}"
                );
            }
        }

        // 2. Metadata preservation: Status and Decision must be in skeleton
        assert!(
            result.skeleton.to_ascii_lowercase().contains("status:"),
            "ADR {rel_path} must preserve Status"
        );
        assert!(
            result.skeleton.to_ascii_lowercase().contains("decision"),
            "ADR {rel_path} must preserve Decision"
        );

        // 3. Idempotence: pruning twice yields identical result
        let pass2 = prune_markdown(&result.skeleton);
        assert_eq!(
            result.skeleton, pass2,
            "ADR {rel_path} is not idempotent under skeletonization"
        );

        // 4. Stable sentinel: does not contain dynamic counters N or K
        assert!(
            !result.skeleton.contains("lines, ~"),
            "ADR {rel_path} must not contain dynamic counter sentinels"
        );
    }
}

#[test]
fn cycle1_token_reduction_on_long_specs() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();

    let doc_path = repo_root.join("docs/architecture/runtime-design.md");
    if doc_path.exists() {
        let content = fs::read_to_string(&doc_path).unwrap();
        let res = extract_skeleton(&content, Language::Markdown);
        assert!(
            res.savings_percentage() > 30.0,
            "runtime-design.md savings {}% should be > 30%",
            res.savings_percentage()
        );
        assert!(
            res.pruned_lines < res.original_lines,
            "pruned lines should be strictly less than original"
        );
    }
}
