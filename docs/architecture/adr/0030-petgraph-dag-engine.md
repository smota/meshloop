# 0030 Internalized petgraph DAG Engine, Parametric Manifests, and Scheduling Latency Gates

- Status: Accepted
- Implementation: implemented
- Date: 2026-09-15
- Author/executor: Antigravity & Grok
- Reviewer: Samuel (User Approval) / Grok CLI
- Approval evidence: Ratified in 3-PR Implementation Plan (De Acordo)
- Supersedes: none
- Superseded by: none
- Technical Specification: [docs/architecture/measurement-and-benchmark-spec.md](../measurement-and-benchmark-spec.md)

## Context and constraints
Task decomposition in Meshloop translates high-level engineering objectives into a directed acyclic graph (`TaskGraph`) of tasks and dependencies. Prior to this decision, topological ordering and cycle detection in `meshloop-domain` relied on a recursive traversal.

This introduced several risks:
- Recursive graph traversals can encounter call-stack exhaustion on deeply sequenced task graphs.
- Cycle detection was not formally verified against complex multi-path or disconnected topologies.
- Orchestration scheduling overhead was not measured against canonical graph topologies, leaving scheduler latency unbounded under fan-out and fan-in spikes.

Constraints:
- Strict `#![forbid(unsafe_code)]` in `meshloop-domain`.
- Zero async runtime (`tokio`) dependencies in domain and engine.
- Bounded scheduling latency: P95 scheduling overhead must not exceed 25.0ms (`orch.schedule.overhead_ms.p95 <= 25.0ms`).

## Alternatives
1. **Maintain hand-rolled recursive or Kahn's algorithm in `meshloop-domain`**:
   Rejected: Re-inventing graph algorithms increases maintenance burden, risks edge cases in cycle reporting, and lacks external verification.
2. **Asynchronous workflow runtimes (Tokio DAG / Ray / Airflow patterns)**:
   Rejected: Violates the core architectural invariant of zero background daemons and zero async runtime bloat in core crates.
3. **Internalize battle-tested `petgraph` (`DiGraphMap`) with parametric manifests and `HdrHistogram`**:
   Chosen: Leverages `petgraph`'s iterative topological sorting, provides 7 canonical manifests covering all standard graph topologies, and measures scheduling overhead with high-dynamic-range histograms.

## Decision or proposal
1. **Adopt `petgraph` in `meshloop-domain`**:
   - Add `petgraph = { version = "0.8.3", default-features = false, features = ["graphmap"] }` to `crates/meshloop-domain/Cargo.toml`.
   - Implement `TaskGraph::try_topological_order` using `petgraph::graphmap::DiGraphMap<TaskId, ()>` and `petgraph::algo::toposort`.
   - Keep cycle and error mapping fully typed via `GraphError::Cycle`, `GraphError::DanglingDependency`, and `GraphError::DuplicateId`.
2. **Tier B Parametric Manifest Suite (`benches/manifests/`)**:
   - Provide 7 standardized JSON manifests:
     - `chain.json`: Linear sequential dependencies ($N=4$).
     - `diamond.json`: Classic diamond fan-out / fan-in ($N=4$).
     - `wide-fanout.json`: 1 root spawning 16 parallel tasks ($N=17$).
     - `wide-fanin.json`: 16 parallel tasks converging into 1 terminal sink ($N=17$).
     - `forest.json`: Disconnected subgraphs / disjoint components ($N=8$).
     - `nested-diamond.json`: Multi-stage layered diamonds ($N=10$).
     - `cyclic-negative-control.json`: Negative control graph containing an intentional cycle.
3. **P95 Scheduling Overhead Gate via `HdrHistogram`**:
   - Add `hdrhistogram = "7.6.0"` to `xtask`.
   - Implement `xtask bench-dag` to parse and schedule all 7 manifests, measuring node readiness to dispatch transition overhead.
   - Enforce `orch.schedule.overhead_ms.p95 <= 25.0ms` as a blocking gate in `benches/thresholds.toml`.

## Consequences
- Topological sorting is iterative and safe against stack overflow on pipelines of arbitrary depth.
- Graph validation correctly rejects cyclic graphs before any execution attempt begins.
- Scheduling overhead across all topologies is empirically measured and enforced (observed P95: 0.006 ms - 0.054 ms, far below the 25.0 ms threshold).
- `#![forbid(unsafe_code)]` remains strictly enforced in `meshloop-domain`.

## Verification and implementation evidence
- `crates/meshloop-domain/src/task_graph.rs`:
  - `deep_pipeline_dag_sorts_without_stack_overflow`: confirms deep linear graphs sort without stack overflow.
  - `try_topological_order_detects_triangle_cycle_safely`: confirms cycle detection on closed loops.
- `benches/manifests/`: All 7 JSON manifests present and valid.
- `cargo run -p xtask -- bench-dag`: All 6 valid DAGs sort correctly; cyclic negative control is rejected with `Cycle detected`; P95 scheduling overhead reports 0.054 ms (PASS).
- `cargo run --release -p xtask -- bench`: Evaluates 7 manifests, reports `orch.schedule.overhead_ms.p95` at 0.006 ms, passing the canonical gate.
