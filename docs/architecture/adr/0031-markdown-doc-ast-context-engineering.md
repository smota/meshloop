# 0031 Deterministic Markdown AST Context Engineering and Document Skeleton Extraction

- Status: Accepted
- Implementation: implemented
- Date: 2026-09-15
- Author/executor: Antigravity
- Reviewer: Samuel (User Approval) & Grok CLI (Peer Review Receipt)
- Approval evidence: User instruction in chat ("Faça o planejamento... passe pelo Grok e refine... inicie a execução imediatamente")
- Supersedes: none
- Extends: 0022 and 0029
- Superseded by: none

## Context and constraints
Meshloop coordinates autonomous multi-agent engineering tasks across polyglot repositories. In ADR 0022, `meshloop-context` introduced AST skeleton extraction for 7 programming languages (Rust, TypeScript, Python, Go, C#, PHP, C++), reducing input tokens by 70% to 90% by pruning internal function bodies while preserving public signatures and types.

However, complex engineering tasks in documented projects require ingesting extensive technical specifications: Architecture Decision Records (ADRs), RFCs, PRDs, governance rules, and API design documents. Previously, markdown documents lacked structural skeleton extraction and were either passed verbatim (risking prompt bloat, token exhaustion, and "lost-in-the-middle" attention degradation) or excluded entirely.

Constraints:
- `#![forbid(unsafe_code)]` in `meshloop-context`.
- Zero external runtime dependencies; no heavy markdown AST or YAML parsing crates.
- Bounded linear-time streaming state-machine parser with zero recursion.
- Stable, deterministic sentinel (`<!-- meshloop:pruned -->`) without dynamic line or token counters to prevent cache busting.
- Universal prompt cache hygiene (line endings normalized to LF, per-line trailing whitespace trimmed, BOM stripped).
- Compile-time slice analysis (`meshloop-engine::slice`) must remain unaffected by documentation changes.

## Alternatives
1. **Verbatim Ingestion:** Pass full markdown documentation to worker agents. Rejected due to rapid token budget exhaustion and dilution of task-critical code signatures.
2. **Heavy Markdown/YAML Crates (comrak, pulldown-cmark, serde_yaml):** Introduce external parsing dependencies. Rejected to maintain zero-dependency purity, fast build times, and clean crate boundaries.
3. **Regex-based line truncation:** Naive line cutting without syntax awareness. Rejected because it breaks fenced code blocks, corrupts markdown tables, and drops critical decision metadata.

## Decision
1. **`Language::Markdown` Support:** Add `Language::Markdown` to `meshloop-context::Language`, recognizing `.md`, `.markdown`, `.mdown`, and `.mkd` file extensions.
2. **Dedicated `doc_skeleton` Streaming Parser:** Introduce `meshloop-context::doc_skeleton`:
   - **Heading Spine:** Preserves all ATX and Setext headings (H1–H6) in original document order.
   - **Fenced Code Isolation:** Accurately tracks ` ``` ` and `~~~` fences with info lines, isolating code and preserving structural interfaces without misinterpreting markdown syntax inside code.
   - **Table Preservation:** Preserves table headers and column alignment separators while trimming data payload rows.
   - **GFM Admonitions:** Retains callout markers (`> [!NOTE]`, `> [!IMPORTANT]`, `> [!WARNING]`, etc.).
   - **Governance Metadata Allowlist:** Preserves critical decision metadata (`Status`, `Implementation`, `Date`, `Author`, `Decision`, `Consequences`, `Verification`, `Acceptance Criteria`) and nested bullet items.
   - **Prose Pruning & Sentinel:** Caps long narrative paragraphs at 2 lines, replacing pruned bodies with a fixed sentinel `<!-- meshloop:pruned -->`.
   - **Idempotence:** Guaranteed `extract_skeleton(extract_skeleton(src).skeleton).skeleton == extract_skeleton(src).skeleton`.
3. **Universal Prompt Cache Hygiene:** Update `meshloop-context::cache` to normalize all inputs across all languages: enforce LF line endings, strip per-line trailing spaces, and strip UTF-8 BOM.
4. **Heading Signatures & Unified Quantized Indexing:** Extend `meshloop-context::signature` to extract markdown section headings as searchable signatures, allowing `SignatureIndex` (ADR 0029) to perform data-oblivious 1-bit/2-bit quantized retrieval across mixed code and documentation contexts.
5. **Slice Isolation:** Ensure `meshloop-engine::slice` ignores markdown documentation files during compiler diagnostic slicing.

## Consequences
- Technical specifications, ADRs, and guides can be included in context selection without blowing up the token budget (typically achieving 50% to 75% token reduction on long architectural documents).
- Full prompt cache reproducibility: identical inputs permuted in different order yield bitwise-identical static prefixes.
- Clean crate boundaries preserved with zero new dependencies and strict safe Rust.

## Verification and implementation evidence
- Two test and refinement cycles executed and validated:
  - Cycle 1: Unit dialect suite (fences, unclosed blocks, tables, frontmatter, admonitions, Unicode headings, idempotence) and golden ADR assertions (`cargo test -p meshloop-context`).
  - Cycle 2: Integration tests verifying mixed code/doc retrieval in `select_context`, prompt cache byte identity, and slice isolation (`cargo test -p meshloop-engine`).
- Full workspace test suite verification: `cargo test --workspace`.
- Full validation gate: `cargo run -p xtask -- check`.
