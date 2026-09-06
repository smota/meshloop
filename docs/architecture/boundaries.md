# Component boundaries

| Crate | Owns | Must not own |
|---|---|---|
| meshloop-domain | Task, state, policy, evidence value types | I/O, concrete harnesses, storage |
| meshloop-engine | Use cases, scheduling, ports, recovery and integration coordination | Concrete adapter construction |
| meshloop-adapters | Herdr, harness, process, Git, storage implementations | Product policy decisions |
| meshloop-cli | Arguments, composition, reports | Orchestration business logic |

Declared path dependencies enforce direction at crate level. Module folders will be added
when behavior is implemented; empty product modules do not imply supported functionality.
Crate-local integration tests own component contracts; root scenarios are wired through xtask.

## Module breakdown

Named here so implementation has a concrete target; each module is still added only when
its owning ADR is accepted and its behavior is actually implemented, per the rule above.

| Crate | Modules |
|---|---|
| meshloop-domain | `task_graph` (node/edge/tier types, cycle validation), `state` (the execution-lifecycle.md state enum and transition preconditions as pure functions), `evidence` (the three ADR 0007 evidence-kind value types), `policy` (tier/coupling value types consumed by routing, no scoring logic), `capability` (HarnessProfile, quota/cooldown state, and the ADR 0003 error taxonomy — pure data, no probing logic) |
| meshloop-engine | `ports` (HarnessCapabilities, HerdrSessionPort, EvidenceStore, RoutingFeedbackStore trait definitions only), `planner` (ADR 0009: dispatches the decomposition agent, validates its output structurally, assigns tiers — does not judge decomposition quality), `router` (the ADR 0009 QACR scoring function, composed of pluggable `RoutingSignal` implementations, over injected ports), `agent` (ADR 0003 AgentSpec construction and prompt rendering, no routing authority), `orchestrator` (state machine driver consuming ports, no concrete process/IO), `recovery` (event-log reconciliation) |
| meshloop-adapters | `herdr` (CLI-subprocess HerdrSessionPort per ADR 0005), `harness` (per-harness HarnessCapabilities impls plus the fixture/stub harness used in contract tests), `git` (worktree isolation per ADR 0005), `store` (SQLite-backed EvidenceStore/RoutingFeedbackStore per ADR 0007) |
| meshloop-cli | `args` (argument-parsing surface), `compose` (wires adapters into engine ports for a run), `report` (human-readable status/evidence output) |

This replaces the source docx's flat `src/core/{planner,orchestrator,judge,synthesizer}.rs`
layout: those four concerns are split across domain/engine/adapters so engine logic can be
unit-tested against fake ports without a real Herdr, harness, or SQLite present, per
docs/engineering/testing.md's port/adapter test split.
