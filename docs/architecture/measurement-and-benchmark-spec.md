# Technical Specification: Measurement, Benchmark, and Architectural Validation Framework

| Metadata | Details |
|---|---|
| **Identifier** | `SPEC-ML-BENCH-001` |
| **Title** | Continuous Measurement and Benchmark Framework for Meshloop |
| **Status** | **Living Specification** |
| **Version** | `0.1.0` |
| **Related Decisions** | ADR 0003, ADR 0005, ADR 0007, ADR 0009, ADR 0016, ADR 0017, ADR 0022, ADR 0024, ADR 0025, ADR 0026, ADR 0029 |
| **Impacted Crates** | `meshloop-context`, `meshloop-engine`, `meshloop-domain`, `meshloop-adapters`, `xtask` |

---

## 1. Context and Motivation

Meshloop is a local orchestration engine in Rust designed for CLI coding agents (Codex, Claude Code, Pi, Grok, Agy). The system provides:
1. Sequential and concurrent agent coordination guided by QACR (Quality, Affinities, Cost, Reliability) routing policies.
2. Code mutation isolation inside ephemeral Git worktrees, preserving developer working branches.
3. Transactional persistence with strict durability via SQLite WAL.
4. Multi-language context engineering (`meshloop-context`), performing AST skeleton extraction across 7 languages and prompt prefix normalization for high cache hit rates (>80%).

This specification details the interfaces, metrics, and thresholds that ensure continuous validation of system performance and invariants.

---

## 2. Core Objectives

- **O1 - Continuous Architectural Validation:** Prevent non-functional regressions in critical paths (AST parsing latency, RunLoop scheduling overhead, SQLite WAL fsync times) and deterministically validate isolation invariants in CI.
- **O2 - Evolutionary Product Validation:** Quantify the impact of architectural improvements across context reduction, prompt cache reuse, and routing precision.
- **O3 - Technical Auditability:** Produce standardized scorecards (structured JSON and Markdown reports) with clear uncertainty bounds and complete provenance.

---

## 3. Metric Taxonomy

### 3.1 Context & Token Metrics (`ctx.*`)

$$\text{Token Reduction Rate: } R = 100 \times \left(1 - \frac{T_{\text{pruned}}}{T_{\text{raw}}}\right)$$

- `ctx.tokens.raw`: Total candidate tokens before pruning.
- `ctx.tokens.pruned`: Total tokens delivered to the agent after AST skeleton extraction.
- `ctx.tokens.reduction_pct`: Token reduction percentage (target: 70% to 90% on structured code).
- `ctx.parse.latency_ms`: Parsing latency across supported languages (`rust`, `ts`, `py`, `go`, etc.).
- `ctx.cache.hit_rate_pct`: Prompt cache prefix hit rate (>80%).
- `ctx.cache.stale_hit_rate_pct`: Cache hits returning stale content (**Invariant: 0.0%**).

### 3.2 Orchestration Metrics (`orch.*`)
- `orch.route.accuracy_pct`: Percentage of tasks routed to the policy-dictated model tier.
- `orch.schedule.overhead_ms`: Scheduler overhead from node readiness to process dispatch.
- `orch.cooldown.violation_count`: Invocations attempted against harnesses under active cooldown (**Invariant: 0**).
- `orch.txn.wal_commit_ms`: RunLoop transaction persistence latency in SQLite WAL.

### 3.3 Isolation and Security Metrics (`iso.*`)
- `iso.worktree.leak_count`: Untracked files, uncommitted changes, or dangling locks left in the host repo (**CI Gate: 0**).
- `iso.crash.recovery_fidelity`: Recovery success rate following crash injection across WAL barriers (**CI Gate: 1.0 / 100%**).
- `iso.redact.pass_rate_pct`: Scrubber pass rate for sensitive tokens across logs, stdout, and diffs (**CI Gate: 100%**).
- `iso.gate.obedience_rate_pct`: Adherence to mandatory human approval gates (`review-plan`, `accept`, `integrate`) (**CI Gate: 100%**).

### 3.4 Developer Cycle Metrics (`dev.*`)
- `dev.fpar`: *First-Pass Acceptance Rate* — Proportion of tasks accepted without retry loops.
- `dev.diff_valid_rate`: Percentage of generated patches that compile and apply cleanly.
- `dev.wall_clock_s`: End-to-end task cycle duration, decomposed by execution phase.

### 3.5 Convergence and Self-Repair Metrics (`conv.*`)
- `conv.lattice.reduction_rate`: Average Lyapunov energy variation between repair rounds ($\Delta \phi / \text{round}$).
- `conv.self_repair.success_rate`: Percentage of tasks failing initial checks that reach `Accepted` within retry limits.
- `conv.oscillation.detected_count`: Identical error cycles caught and terminated by FNV-1a error fingerprints.
- `conv.rollback.count`: Frequency of `git reset --hard` rollbacks triggered by syntax regressions.

### 3.6 Quantized Indexing Metrics (`quant.*`)
- `quant.index.compression_ratio`: Memory compression ratio for indexed AST signatures ($\ge 6\times$).
- `quant.search.latency_us`: Symbol search and ranking latency in `SignatureIndex` ($< 500\mu\text{s}$).
- `quant.recall_at_k`: Top-$k$ recall rate compared against unquantized float32 dot-product search.

### 3.7 Concurrency and Host Governance (`conc.*`)
- `conc.throughput_gain`: Speedup factor ($S_N$) for non-dependent tasks executed concurrently ($N \in [1, 16]$).
- `conc.git_admin.lock_contention_ms`: Wait time in `GitAdminMutex` during concurrent worktree management.
- `conc.orphan_process_count`: Residual child processes remaining after harness cancellation (**CI Gate: 0**).

---

## 4. Canonical JSON Schema (`run-record.v1`)

Benchmark runs generate a standardized JSON record in `artifacts/bench/run.json`:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "schema_version": "1.0.0",
  "run_id": "synthetic-run-001",
  "suite": {
    "name": "full-algorithmic-suite",
    "kind": "synthetic",
    "world": "deterministic",
    "suite_version": "1.0.0"
  },
  "subject": {
    "crate_versions": {
      "meshloop-domain": "0.1.0",
      "meshloop-context": "0.1.0",
      "meshloop-engine": "0.1.0",
      "meshloop-adapters": "0.1.0",
      "meshloop-cli": "0.1.0"
    },
    "os": "windows"
  },
  "metrics": [
    { "name": "conv.lattice.reduction_rate", "value": 1.5, "status": "pass" },
    { "name": "conv.self_repair.success_rate", "value": 100.0, "status": "pass" },
    { "name": "quant.index.compression_ratio", "value": 8.0, "status": "pass" },
    { "name": "quant.search.latency_us", "value": 12.5, "status": "pass" },
    { "name": "quant.recall_at_k", "value": 98.5, "status": "pass" },
    { "name": "slice.build_avoidance_rate", "value": 66.7, "status": "pass" },
    { "name": "conc.orphan_process_count", "value": 0, "status": "pass" },
    { "name": "conc.throughput_gain", "value": 1.85, "status": "pass" }
  ],
  "status": {
    "overall": "pass",
    "invariants": "pass",
    "thresholds": "pass"
  }
}
```

---

## 5. Verification Commands

```bash
# Execute benchmark suite and generate run.json artifact
cargo run -p xtask -- bench

# Check workspace formatting, lints, and unit tests
cargo run -p xtask -- check

# Verify daemonless mode and worktree isolation
cargo run -p xtask -- live
```
