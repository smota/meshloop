# 0023 Measurement, Benchmark, and Architectural Validation Framework

- Status: Proposed
- Implementation: in-progress
- Date: 2026-09-15
- Author/executor: Antigravity / AI Pair programmer & Grok
- Decision owner: Samuel
- Approval evidence: user authorization in chat ("Chame o grok e defina em conjunto o desenho de um framework de medição e benchmark para o meshloop...")
- Supersedes: none (extends 0007, 0009, 0022)
- Superseded by: none
- Technical Specification: [docs/architecture/measurement-and-benchmark-spec.md](../measurement-and-benchmark-spec.md)

## Context and constraints
With the implementation of daemonless standalone execution and 7-language AST context engineering (ADR 0022), Meshloop operates without external daemon dependencies and performs aggressive token compression. However, verifying that context pruning preserves necessary semantic signals without degrading First-Pass Acceptance Rates (FPAR), and ensuring that QACR routing, SQLite WAL durability, and Git worktree isolation do not regress under high throughput requires a formalized, repeatable measurement and benchmark framework.

## Alternatives
1. Rely exclusively on standard `cargo test` unit and scenario tests. (Inadequate: fails to measure structural regressions, token bloat, cache staleness, or stochastic live agent distributions).
2. Ad-hoc Python profiling scripts outside the project lifecycle. (Inadequate: creates divergent tooling, unversioned metric definitions, and untracked environments).
3. Cohesive, in-tree 3-tier measurement framework (`xtask bench`) separating deterministic mock proofs (World D) from stochastic live agent sampling (World S), governed by a canonical JSON schema v1 and living technical specification.

## Decision
1. **Living Specification Contract:** Adopt [`SPEC-ML-BENCH-001`](../measurement-and-benchmark-spec.md) as the canonical architectural and engineering specification for all benchmark tooling.
2. **Dual-World Methodology:** Strictly segregate deterministic mock runs (World D, bit-reproducible, CI gate) from stochastic live agent runs (World S, $N \ge 10$, Wilson 95% confidence intervals).
3. **Fail-Closed Invariants (`iso.*`):** Enforce zero worktree leakage, 100% secret redaction, 100% human gate obedience, and 100% crash recovery fidelity on SQLite WAL as hard CI gates.
4. **Three-Tier Workload Taxonomy:**
   - Tier A: Criterion micro-benchmarks for AST parsing across 7 languages and WAL serialization.
   - Tier B: Parametric synthetic DAG workloads across 7 canonical families (`benches/manifests/`).
   - Tier C: Disposable E2E sandbox repositories under RAII lifecycle.
5. **Unified CLI via `xtask`:** Implement all benchmark commands under `cargo run -p xtask -- bench*`, reading thresholds from `benches/thresholds.toml`.

## Consequences
- Every pull request receives objective regression protection on context size, parse latency, and safety invariants.
- Living specification provides explicit extension points (`RFC-BENCH-01` to `RFC-BENCH-04`) for iterative refinement alongside ongoing engine evolution.
- Transparent scorecards (1-page executive + technical annex) provide auditable evidence of architectural integrity and product efficiency.
