//! Markdown and document AST skeleton extraction for technical documentation.
//!
//! Prunes long narrative prose and large data payloads while preserving:
//! - Heading spine (H1 to H6, ATX and Setext)
//! - Code block fences with info strings and structural stubs
//! - Table headers and column separators
//! - GFM callouts / admonitions (`> [!NOTE]`, `> [!IMPORTANT]`, etc.)
//! - Specification and ADR metadata (Status, Decision, Consequences, Verification)
//! - Stable sentinel `<!-- meshloop:pruned -->` without dynamic counters to prevent cache busting
//!
//! Safe Rust only (`#![forbid(unsafe_code)]`), linear time, zero external dependencies.

pub const PRUNED_SENTINEL: &str = "<!-- meshloop:pruned -->";

/// Allowlisted metadata prefixes preserved in technical documents and ADRs.
const ALLOWLISTED_METADATA: &[&str] = &[
    "- status:",
    "status:",
    "- implementation:",
    "implementation:",
    "- date:",
    "date:",
    "- author:",
    "author:",
    "- author/executor:",
    "author/executor:",
    "- reviewer:",
    "reviewer:",
    "- approval evidence:",
    "approval evidence:",
    "- supersedes:",
    "supersedes:",
    "- superseded by:",
    "superseded by:",
    "- technical specification:",
    "technical specification:",
    "decision:",
    "decision or proposal:",
    "consequences:",
    "verification:",
    "verification and implementation evidence:",
    "acceptance criteria:",
    "requirements:",
    "constraints:",
];

/// Checks if a line matches allowlisted document metadata.
fn is_metadata_line(trimmed_lower: &str) -> bool {
    ALLOWLISTED_METADATA
        .iter()
        .any(|prefix| trimmed_lower.starts_with(prefix))
}

/// Checks if a line is an ATX heading (`# Title` through `###### Title`).
fn is_atx_heading(trimmed: &str) -> bool {
    if !trimmed.starts_with('#') {
        return false;
    }
    let hashes = trimmed.bytes().take_while(|&b| b == b'#').count();
    if hashes > 6 {
        return false;
    }
    trimmed[hashes..].starts_with(' ') || trimmed.len() == hashes
}

/// Checks if a line is a GFM admonition / callout (`> [!NOTE]`, `> [!WARNING]`, etc.).
fn is_gfm_admonition(trimmed: &str) -> bool {
    let lower = trimmed.to_ascii_lowercase();
    lower.starts_with("> [!note]")
        || lower.starts_with("> [!tip]")
        || lower.starts_with("> [!important]")
        || lower.starts_with("> [!warning]")
        || lower.starts_with("> [!caution]")
}

/// Checks if a line is a table header separator (e.g. `|---|---|` or `|:---|---:|`).
fn is_table_separator(trimmed: &str) -> bool {
    if !trimmed.contains('|') || !trimmed.contains('-') {
        return false;
    }
    // Check if characters are only '|', '-', ':', and spaces
    trimmed
        .chars()
        .all(|c| c == '|' || c == '-' || c == ':' || c.is_ascii_whitespace())
}

/// Checks if a line is a badge or image-only line (e.g. `[![...](...)](...)`).
fn is_badge_or_image(trimmed: &str) -> bool {
    trimmed.starts_with("![") || trimmed.starts_with("[![")
}

/// Strips UTF-8 Byte Order Mark (BOM) if present.
fn strip_bom(s: &str) -> &str {
    s.strip_prefix('\u{feff}').unwrap_or(s)
}

/// Prunes a Markdown document into its structural AST skeleton.
pub fn prune_markdown(source: &str) -> String {
    let clean_source = strip_bom(source);
    let mut out: Vec<String> = Vec::new();

    let mut in_frontmatter = false;
    let mut frontmatter_started = false;
    let mut in_code_fence = false;
    let mut fence_char = ' ';
    let mut fence_len = 0;
    let mut code_lines_kept = 0;
    let mut has_pruned_code = false;

    let mut consecutive_blanks = 0;
    let mut prose_lines_kept = 0;
    let mut has_pruned_in_block = false;
    let mut in_allowlisted_section = false;

    let mut prev_line = String::new();

    let raw_lines: Vec<&str> = clean_source.lines().collect();
    let num_lines = raw_lines.len();

    for i in 0..num_lines {
        let line = raw_lines[i];
        let trimmed_end = line.trim_end();
        let trimmed = trimmed_end.trim();
        let trimmed_lower = trimmed.to_ascii_lowercase();

        // 1. Code Fence Detection
        let fence_prefix = trimmed
            .bytes()
            .take_while(|&b| b == b'`' || b == b'~')
            .count();
        let is_fence_delimiter = fence_prefix >= 3
            && (fence_prefix == trimmed.len()
                || trimmed.as_bytes()[0] == trimmed.as_bytes()[fence_prefix - 1]);

        if in_code_fence {
            if is_fence_delimiter && trimmed.starts_with(fence_char) && fence_prefix >= fence_len {
                // Closing fence
                if has_pruned_code {
                    out.push("    /* ... */".to_string());
                }
                out.push(trimmed.to_string());
                in_code_fence = false;
                has_pruned_code = false;
                consecutive_blanks = 0;
                prev_line = line.to_string();
                continue;
            } else {
                // Inside code fence: keep first 2 lines only, then stub
                if code_lines_kept < 2 {
                    out.push(trimmed_end.to_string());
                    code_lines_kept += 1;
                } else {
                    has_pruned_code = true;
                }
                prev_line = line.to_string();
                continue;
            }
        } else if is_fence_delimiter {
            in_code_fence = true;
            fence_char = trimmed.chars().next().unwrap_or('`');
            fence_len = fence_prefix;
            code_lines_kept = 0;
            has_pruned_code = false;
            out.push(trimmed_end.to_string());
            consecutive_blanks = 0;
            prev_line = line.to_string();
            continue;
        }

        // 2. Frontmatter Detection (`---` or `+++` at beginning of document)
        if !frontmatter_started && (trimmed == "---" || trimmed == "+++") {
            in_frontmatter = true;
            frontmatter_started = true;
            out.push(trimmed.to_string());
            consecutive_blanks = 0;
            prev_line = line.to_string();
            continue;
        } else if in_frontmatter {
            if trimmed == "---" || trimmed == "+++" {
                in_frontmatter = false;
                out.push(trimmed.to_string());
                consecutive_blanks = 0;
                prev_line = line.to_string();
                continue;
            }
            // Keep frontmatter metadata
            out.push(trimmed_end.to_string());
            prev_line = line.to_string();
            continue;
        }
        if !trimmed.is_empty() {
            frontmatter_started = true;
        }

        // 3. Blank Line Handling (collapse consecutive blank lines)
        if trimmed.is_empty() {
            consecutive_blanks += 1;
            if consecutive_blanks <= 1 && !out.is_empty() {
                out.push(String::new());
            }
            prose_lines_kept = 0;
            has_pruned_in_block = false;
            prev_line.clear();
            continue;
        }
        consecutive_blanks = 0;

        // 4. Skip Badges and HTML comments (unless it is our stable sentinel)
        if trimmed == PRUNED_SENTINEL {
            if !has_pruned_in_block {
                out.push(PRUNED_SENTINEL.to_string());
                has_pruned_in_block = true;
            }
            continue;
        }
        if is_badge_or_image(trimmed) {
            continue;
        }
        if trimmed.starts_with("<!--") && trimmed.ends_with("-->") {
            continue;
        }

        // 5. ATX Headings (`# Heading`)
        if is_atx_heading(trimmed) {
            out.push(trimmed.to_string());
            prose_lines_kept = 0;
            has_pruned_in_block = false;
            in_allowlisted_section = is_metadata_line(&trimmed_lower);
            prev_line = line.to_string();
            continue;
        }

        // 6. Setext Headings (`===` for H1 or `---` for H2 following text)
        if !prev_line.trim().is_empty() {
            let is_setext_h1 = trimmed.bytes().all(|b| b == b'=') && trimmed.len() >= 2;
            let is_setext_h2 = trimmed.bytes().all(|b| b == b'-') && trimmed.len() >= 2;
            if is_setext_h1 || is_setext_h2 {
                // Ensure previous line was emitted
                if out.last().map(|s| s.trim()) != Some(prev_line.trim()) {
                    out.push(prev_line.trim().to_string());
                }
                out.push(trimmed.to_string());
                prose_lines_kept = 0;
                has_pruned_in_block = false;
                in_allowlisted_section = is_metadata_line(&prev_line.trim().to_ascii_lowercase());
                prev_line.clear();
                continue;
            }
        }

        // 7. Table handling
        if trimmed.contains('|') {
            // If it's a table header or separator, always keep
            let is_separator = is_table_separator(trimmed);
            let next_is_separator =
                i + 1 < num_lines && is_table_separator(raw_lines[i + 1].trim());

            if is_separator || next_is_separator {
                out.push(trimmed_end.to_string());
                prev_line = line.to_string();
                continue;
            }
            // Table data row: keep first row or prune
            if prose_lines_kept < 1 {
                out.push(trimmed_end.to_string());
                prose_lines_kept += 1;
            } else if !has_pruned_in_block {
                out.push(format!("| {} |", PRUNED_SENTINEL));
                has_pruned_in_block = true;
            }
            prev_line = line.to_string();
            continue;
        }

        // 8. GFM Callouts / Admonitions
        if is_gfm_admonition(trimmed) {
            out.push(trimmed_end.to_string());
            prose_lines_kept = 0;
            has_pruned_in_block = false;
            prev_line = line.to_string();
            continue;
        }

        // 9. Allowlisted Metadata Lines (ADR headers, Status, Decision, etc.)
        if is_metadata_line(&trimmed_lower) {
            out.push(trimmed_end.to_string());
            prose_lines_kept = 0;
            has_pruned_in_block = false;
            in_allowlisted_section = true;
            prev_line = line.to_string();
            continue;
        }

        // 10. List items under allowlisted sections or top-level lists
        let is_list_item = trimmed.starts_with("- ")
            || trimmed.starts_with("* ")
            || trimmed.starts_with("+ ")
            || (trimmed.chars().next().is_some_and(|c| c.is_ascii_digit())
                && trimmed.contains(". "));

        if in_allowlisted_section && is_list_item {
            out.push(trimmed_end.to_string());
            prev_line = line.to_string();
            continue;
        }

        // 11. Narrative Prose / Paragraphs: cap at 2 lines, then sentinel
        if prose_lines_kept < 2 {
            out.push(trimmed_end.to_string());
            prose_lines_kept += 1;
        } else if !has_pruned_in_block {
            out.push(PRUNED_SENTINEL.to_string());
            has_pruned_in_block = true;
        }

        prev_line = line.to_string();
    }

    // Clean up trailing empty lines
    while out.last().is_some_and(|s| s.trim().is_empty()) {
        out.pop();
    }

    // Join with deterministic LF endings
    let mut result = out.join("\n");
    if !result.is_empty() {
        result.push('\n');
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_spine_and_metadata_preserved() {
        let doc = r#"# Architecture Overview

Status: Accepted
Author: Samuel
Date: 2026-09-15

## Context and constraints
This is line one of context.
This is line two of context.
This is line three of context which should be pruned.
This is line four of context which should also be pruned.
This is line five of context.

## Decision
- Decision item 1
- Decision item 2
- Decision item 3

## Consequences
- Consequence 1
- Consequence 2
"#;

        let pruned = prune_markdown(doc);
        assert!(pruned.contains("# Architecture Overview"));
        assert!(pruned.contains("Status: Accepted"));
        assert!(pruned.contains("Author: Samuel"));
        assert!(pruned.contains("## Context and constraints"));
        assert!(pruned.contains("This is line one of context."));
        assert!(pruned.contains("This is line two of context."));
        assert!(!pruned.contains("This is line three of context which should be pruned."));
        assert!(pruned.contains(PRUNED_SENTINEL));
        assert!(pruned.contains("## Decision"));
        assert!(pruned.contains("- Decision item 1"));
        assert!(pruned.contains("- Decision item 2"));
        assert!(pruned.contains("## Consequences"));
        assert!(pruned.contains("- Consequence 1"));
    }

    #[test]
    fn code_fence_isolation_preserves_hashes() {
        let doc = r#"# Document

Here is code:

```rust
// In rust, this is a comment, not a markdown heading
# fn hidden_hash() {}
pub fn real_func() {
    let x = 1;
    let y = 2;
    let z = 3;
}
```

Paragraph after code.
"#;

        let pruned = prune_markdown(doc);
        assert!(pruned.contains("# Document"));
        assert!(pruned.contains("```rust"));
        assert!(pruned.contains("// In rust, this is a comment, not a markdown heading"));
        assert!(pruned.contains("# fn hidden_hash() {}"));
        assert!(pruned.contains("/* ... */"));
        assert!(pruned.contains("```"));
        assert!(pruned.contains("Paragraph after code."));
    }

    #[test]
    fn table_header_preserved_and_rows_compacted() {
        let doc = r#"# Metrics

| Metric | Target | Actual |
|:-------|:------:|-------:|
| Latency| <25ms  | 12ms   |
| Memory | <50MB  | 30MB   |
| Tokens | >65%   | 72%    |
"#;

        let pruned = prune_markdown(doc);
        assert!(pruned.contains("| Metric | Target | Actual |"));
        assert!(pruned.contains("|:-------|:------:|-------:|"));
        assert!(pruned.contains("| Latency| <25ms  | 12ms   |"));
        assert!(pruned.contains(PRUNED_SENTINEL));
    }

    #[test]
    fn gfm_admonition_preserved() {
        let doc = r#"# Guide

> [!IMPORTANT]
> Critical safety rule.
"#;

        let pruned = prune_markdown(doc);
        assert!(pruned.contains("> [!IMPORTANT]"));
        assert!(pruned.contains("> Critical safety rule."));
    }

    #[test]
    fn frontmatter_handled() {
        let doc = r#"---
title: ADR Document
status: accepted
tags: [architecture, loop]
---

# Title
Body text.
"#;

        let pruned = prune_markdown(doc);
        assert!(pruned.contains("---"));
        assert!(pruned.contains("title: ADR Document"));
        assert!(pruned.contains("status: accepted"));
        assert!(pruned.contains("# Title"));
    }

    #[test]
    fn idempotence_guarantee() {
        let doc = r#"# Architecture

- Status: Accepted

## Context
First line of context.
Second line of context.
Third line that gets pruned.
Fourth line that gets pruned.

## Decision
- Rule A
- Rule B
"#;

        let pass1 = prune_markdown(doc);
        let pass2 = prune_markdown(&pass1);
        assert_eq!(pass1, pass2);
    }
}
