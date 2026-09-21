# Implementation Plan v3: System 1 Decision Engine & Predictive Success Oracle (EPIC-ML-016)

- **Status:** Registered in Active Backlog (Target: Milestone 0.2.0)
- **Date:** 2026-09-21
- **Linked Specifications:** [EPIC-ML-016](epic-system1-jev-orchestration.md), [`SPEC-ML-BENCH-002`](../architecture/system1-benchmark-and-validation-spec.md)
- **Linked ADRs:** ADR 0003, ADR 0007, ADR 0009, ADR 0026, ADR 0029, **ADR 0032 (Proposed)**

---

## 1. Scope of Capabilities

| # | Capability | Module / Crate | ADR / Spec | Implementation Status |
|---|---|---|---|---|
| **1** | **Domain Value Types (`DecisionOutcome`, `Confidence`, `RiskFloor`)** | `meshloop-domain` | ADR 0032 / WP-1 | **Backlog (Phase 1)** |
| **2** | **Engine Ports (`DecisionPort`, `PredictiveSuccessOracle`) & Test Doubles** | `meshloop-engine::ports` | ADR 0032 / WP-2 | **Backlog (Phase 1)** |
| **3** | **Safe `ureq` Adapter & Socket-Denied Transport Fakes** | `meshloop-adapters::jev` | ADR 0032 / WP-3 | **Backlog (Phase 2)** |
| **4** | **Planning Risk Ratchet (`JevTierAssigner`)** | `meshloop-engine::planner` | ADR 0009, 0032 / WP-4 | **Backlog (Phase 3)** |
| **5** | **Dual-Threshold Candidate Review Gating ($0.95 / 0.05$)** | `meshloop-engine::verify` | ADR 0007, 0032 / WP-5 | **Backlog (Phase 3)** |
| **6** | **Attempt-Scoped Mechanical Self-Repair Micro-Actuator** | `meshloop-engine::converge` | ADR 0026, 0029 / WP-6 | **Backlog (Phase 3)** |
| **7** | **QACR Oracle Prior Fusion ($\mu_h = \text{clip}((s+4\pi)/(n+4))$)** | `meshloop-engine::router` | ADR 0009, 0032 / WP-7 | **Backlog (Phase 3)** |
| **8** | **Context Suffix Reranking (Batched 24-Noul)** | `meshloop-context`, `engine` | ADR 0029, 0032 / WP-8 | **Backlog (Phase 3)** |
| **9** | **End-to-End Orchestration & Benchmarks (`xtask bench-system1`)** | `xtask` | SPEC-ML-BENCH-002 / WP-9 | **Backlog (Phase 4)** |

---

## 2. Phased Roadmap

```mermaid
flowchart LR
    subgraph Phase1 ["Phase 1: Observation-Only Core"]
        P1_1["WP-1: ADR 0032 & Domain Types"]
        P1_2["WP-2: DecisionPort & Scripted Doubles"]
    end

    subgraph Phase2 ["Phase 2: Transport & Isolation"]
        P2_1["WP-3: ureq Adapter & Pinned TLS"]
        P2_2["Socket-Denied Offline CI Verification"]
    end

    subgraph Phase3 ["Phase 3: Actuator Activation"]
        P3_1["WP-4: JevTierAssigner (Raise-Only Ratchet)"]
        P3_2["WP-5: Model Review Gating (0.95 / 0.05)"]
        P3_3["WP-6: Self-Repair Micro-Actuator"]
        P3_4["WP-7: QACR Prior Fusion & Dollar Ledger"]
        P3_5["WP-8: Context Suffix Reranker"]
    end

    subgraph Phase4 ["Phase 4: Release Hardening"]
        P4_1["WP-9: xtask bench-system1 Scorecard"]
        P4_2["Outbound Data Policy Reconciled"]
    end

    Phase1 --> Phase2 --> Phase3 --> Phase4
```

### Phase 1: Observation-Only Core
- Author **ADR 0032** defining the abstract `DecisionPort` and `PredictiveSuccessOracle`.
- Introduce pure value types in `meshloop-domain`: `Confidence`, `DecisionOutcome`, `DecisionVerdict`, `RiskFloor`.
- Implement `ScriptedDecisionPort` in `meshloop-engine::ports` for zero-I/O in-memory unit tests.
- Record observation-only decision logs in runs without modifying state machine transitions.

### Phase 2: Transport & Isolation
- Implement `JevClient` in `meshloop-adapters::jev` utilizing synchronous `ureq` with explicitly pinned `rustls` (audited for `unsafe_code = "deny"`).
- Implement canned `JevTransportFake` delivering deterministic JSON fixtures across 70ms–500ms simulated latencies.
- Enforce socket-denied test execution under `cargo test -p meshloop-adapters --locked --offline`.

### Phase 3: Actuator Activation
- **Planner Ratchet:** Wire `JevTierAssigner` in `planner.rs`. One-shot Bernoulli union $q \ge 0.90 \implies \text{Tier3}$; lower tiers are preserved. Heuristic Tier 3 is an absolute floor.
- **Review Gating:** Wire dual-threshold gating in `verify.rs`. $p \ge 0.95 \implies \text{Pass}$; $p \le 0.05 \implies \text{Fail}$; $(0.05, 0.95) \implies \text{Abstain}$ (task remains in `AwaitingReview` for human review). Tier 3 remains human-only.
- **Self-Repair Micro-Actuation:** Keep `RepairSession::observe` pure. Outside `observe()`, dispatch allowlisted errors to the micro-actuator on $P(\text{Mech}) \ge 0.70$. Apply closed compiler suggestions. One observation cost before Lyapunov rollback.
- **QACR Bayesian Fusion:** Integrate `PredictiveSuccessOracle` into candidate scoring in `router.rs`. Clip prior shift to $|\mu_h - \mu_{\text{local}}| \le 0.25$. Preserve exploration bonus on real pulls. Manage cumulative API spend in integer micro-dollars ($\mu\$$).
- **Context Reranking:** Send one batched 24-Noul query over unpinned FWHT suffix. Sort key: `(pinned desc, noul desc, quant desc, path asc)`. Fall back instantly to pure FWHT on error.

### Phase 4: Release Hardening & SPEC-ML-BENCH-002
- Implement `xtask bench-system1` evaluating `artifacts/bench/system1_bench.json`.
- Enforce fail-closed CI gates in `benches/thresholds.toml`: 0 downward tier violations, 0 Tier 3 model review bypasses, 0 unpinned file drops, 100% Lyapunov descent.
- Reconcile outbound data policy in `docs/product/brief.md` and `docs/architecture/threat-model.md`.

---

## 3. Contribution Rules and Engineering Invariants

1. **Hexagonal Purity:** No HTTP, vendor, or socket types in `meshloop-domain`.
2. **Raise-Only Ratchet:** Jev can only raise risk tiers, never lower them. Heuristic Tier 3 and human tiers are immutable floors.
3. **Mandatory Human Gate on Tier 3:** Tier 3 tasks can **never** be accepted by model review; human acceptance remains strictly required.
4. **Purity of Lyapunov Convergence:** `RepairSession::observe` remains 100% deterministic over `DiagnosticLattice`. Micro-repairs are regular attempts that must strictly decrease $\phi$.
5. **Freeze Invariant:** Oracle samples are drawn once per task definition / diff revision and frozen; repeated draws on identical text are prohibited.
6. **Zero-Socket Offline CI:** Default workspace test suite runs with zero network dependencies.

---

## 4. Verification Commands

```bash
# Verify workspace formatting, lints, and unit tests
cargo run -p xtask -- check

# Run canonical SPEC-ML-BENCH-001 benchmark scorecard
cargo run -p xtask -- bench

# Run new System 1 benchmark scorecard (SPEC-ML-BENCH-002)
cargo run -p xtask -- bench-system1

# Run full smoke lifecycle test
cargo run -p xtask -- smoke
```
