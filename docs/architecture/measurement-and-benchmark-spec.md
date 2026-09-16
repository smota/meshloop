# Technical Specification: Measurement, Benchmark, and Architectural Validation Framework

| Metadata | Details |
|---|---|
| **Identifier** | `SPEC-ML-BENCH-001` |
| **Title** | Continuous Measurement and Benchmark Framework for Meshloop |
| **Status** | **Living Specification** |
| **Version** | `0.1.0` |
| **Related Decisions** | ADR 0003, ADR 0005, ADR 0007, ADR 0009, ADR 0016, ADR 0017, ADR 0022, ADR 0024, ADR 0025, ADR 0026, ADR 0029, ADR 0030 |
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
- `orch.schedule.overhead_ms.p95`: 95th percentile scheduling overhead across 7 canonical manifests using `HdrHistogram` (**Target: $\le 25.0\text{ms}$**).
- `orch.cooldown.violation_count`: Invocations attempted against harnesses under active cooldown (**Invariant: 0**).
- `orch.txn.wal_commit_ms`: RunLoop transaction persistence latency in SQLite WAL.
- `orch.wave.dispatch_overhead_ms`: Scheduler overhead to integrate ancestor tasks, evaluate dependency satisfaction, and dispatch dependent downstream tasks (**Target: $\le 1500.0\text{ms}$**).

### 3.3 Isolation and Security Metrics (`iso.*`)
- `iso.worktree.leak_count`: Untracked files, uncommitted changes, or dangling locks left in the host repo (**CI Gate: 0**).
- `iso.ancestor_propagation.pass_rate_pct`: Fidelity of ancestor artifact inheritance and sibling isolation across multi-wave execution (**CI Gate: 100%**).
- `iso.crash.recovery_fidelity`: Recovery success rate following crash injection across WAL barriers (**CI Gate: 1.0 / 100%**).
- `iso.redact.pass_rate_pct`: Scrubber pass rate for sensitive tokens across logs, stdout, and diffs (**CI Gate: 100%**).
- `iso.gate.obedience_rate_pct`: Adherence to mandatory human approval gates (`review-plan`, `accept`, `integrate`) (**CI Gate: 100%**).

### 3.4 Developer Cycle Metrics (`dev.*`)
- `dev.fpar`: *First-Pass Acceptance Rate* — Proportion of tasks accepted on initial attempt without retry loops. Measured in World S with pure Rust analytical Wilson score 95% confidence intervals ($z = 1.95996$):
  $$w = \frac{\hat{p} + \frac{z^2}{2n} \pm z \sqrt{\frac{\hat{p}(1-\hat{p})}{n} + \frac{z^2}{4n^2}}}{1 + \frac{z^2}{n}}$$
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
    { "name": "orch.schedule.overhead_ms.p95", "value": 0.054, "status": "pass" },
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

### 4.1 Tier B: Parametric Manifest Suite (`benches/manifests/`)

Standardized synthetic DAG workloads validating topological sorting, cycle detection, and scheduling latency across 7 canonical graph structures:
- `chain.json`: Linear sequential dependencies ($N=4$).
- `diamond.json`: Classic diamond fan-out / fan-in ($N=4$).
- `wide-fanout.json`: 1 root spawning 16 parallel tasks ($N=17$).
- `wide-fanin.json`: 16 parallel tasks converging into 1 terminal sink ($N=17$).
- `forest.json`: Disconnected subgraphs / disjoint components ($N=8$).
- `nested-diamond.json`: Multi-stage layered diamonds ($N=10$).
- `cyclic-negative-control.json`: Negative control containing an intentional cycle.

### 4.2 Tier S: World S Autonomy Benchmark (`benches/world-s/`)

Opt-in polyglot evaluation suite executing real-world agent tasks across 8 curated exercises:
- `rust-two-fer` (Rust)
- `rust-clock` (Rust)
- `ts-bob` (TypeScript)
- `py-luhn` (Python)
- `go-hamming` (Go)
- `cs-nucleotide-count` (C#)
- `php-gigasecond` (PHP)
- `cpp-reverse-string` (C++)

Generates `artifacts/bench/world_s.json` reporting task resolution rate and First-Pass Acceptance Rate (FPAR) bounded by Wilson score 95% confidence intervals.

---

## 5. Verification Commands

```bash
# Execute canonical SPEC-ML-BENCH-001 scorecard against thresholds.toml
cargo run --release -p xtask -- bench

# Benchmark 7 canonical DAG manifests against P95 scheduling gate
cargo run -p xtask -- bench-dag

# Run World S opt-in autonomy benchmark suite (8 polyglot exercises)
cargo run -p xtask -- bench-world-s

# Check workspace formatting, lints, and unit tests
cargo run -p xtask -- check

# Verify daemonless mode and worktree isolation launch gate
cargo run -p xtask -- live
```
