# Implementation plan

This is proposed sequencing, not an accepted ADR. For what has actually been built and
verified as of 2026-09-05, see docs/engineering/implementation-status.md — Phases 0-7 below
are substantially implemented on native Windows (Phase 0's WSL2 half is not: no C toolchain
in the WSL distribution here); Phase 8 (remaining harnesses, parallelism) and Phase 9
(packaging) are not started. Re-derive current status from the actual repository before
trusting either document if time has passed. See design-patterns.md for the concrete
pattern each phase's module should be built and tested around — it maps directly onto
Phases 2-6 below. ADR numbers below reflect the 2026-09-05 consolidation
(docs/architecture/adr/README.md): five active runtime ADRs (0001, 0003, 0005, 0007, 0009)
plus the three accepted ones (0002, 0011, 0012).

## Phase 0: unblock the toolchain
`cargo test` did not execute at initialization: MSVC `link.exe` was unavailable on native
Windows (initialization.md). Confirm the same for the WSL2 side of Tier A (ADR 0001): a
standard Linux build toolchain (gcc/build-essential or equivalent) must be present inside
the WSL distribution used for development. No later phase can be verified — only compiled —
until `cargo run -p xtask -- check` passes with tests actually executing on both sides.

## Phase 1: accept the ADRs that gate everything else
Suggested order: 0003 (harness capability and agent dispatch, including the error
taxonomy) first, since nothing else can be implemented without it; 0005 (execution,
transport, isolation, recovery) next, since it consumes 0003's taxonomy; 0007
(verification and persistence) alongside 0005, since recovery and evidence both write to
the same event log; 0009 (planning and routing) last among the runtime ADRs, since QACR
depends on 0003's capability contracts and 0007's persisted routing feedback. 0001
(product scope, platforms, operating surface) can be accepted any time before Phase 4 — it
only bounds platform/scenario/naming claims.

## Phase 2: domain, no I/O
Implement meshloop-domain: `task_graph` (node/edge/tier types, cycle detection), `state`
(the execution-lifecycle.md enum and transition preconditions as pure functions),
`evidence` (the three ADR 0007 value types), `policy` (tier/coupling types), `capability`
(HarnessProfile, quota/cooldown state, and the ADR 0003 error taxonomy). Pure unit tests
only, per testing.md's "pure domain unit tests" category — no adapter or engine logic yet.

## Phase 3: ports and fakes
Define the four ports in meshloop-engine (`HarnessCapabilities`, `HerdrSessionPort`,
`EvidenceStore`, `RoutingFeedbackStore`) as traits only, plus in-memory fake
implementations that can simulate `CapacityExhausted` and cooldown expiry, since QACR's
fallback logic is untestable without a fake that can go in and out of cooldown. No real
subprocess, Herdr, or SQLite code yet.

## Phase 4: agent, router, and planner against fakes
Implement `agent` (ADR 0003's AgentSpec construction and prompt rendering) and `router`
(the ADR 0009 QACR algorithm, as pluggable `RoutingSignal`s) first, since `planner` depends
on both: decomposition is itself an agent dispatch routed through QACR, not a separate code
path. Then implement `planner`'s three jobs — dispatch the decomposition agent, validate
its output structurally (cycles, dangling dependencies, schema), assign tiers. Required
before this phase counts as implemented: no candidate the user didn't configure can ever be
selected; feedback can reorder candidates but never add one; a Tier 3 node's routing
decision cannot skip the downstream human-acceptance requirement; a fake harness's
`CapacityExhausted` response correctly triggers fallback, including when the failing
dispatch is the decomposition call itself; a prompt built for one task node never contains
another node's context; a structurally invalid decomposition is rejected and re-dispatched,
never partially scheduled.

## Phase 5: adapters, contract-tested before live
Order matters — each adapter's own contract tests must pass before it is wired into the
orchestrator, and each must be exercised on both native Windows and Linux-under-WSL2 (ADR
0001) before being called cross-platform-ready:
1. Harness adapter, first against a fixture/stub CLI that misreports its version or hangs
   on cancel, then real Claude Code/Codex probes.
2. Herdr adapter as CLI-subprocess only (ADR 0005), structured arguments, tested against a
   disposable herdr instance or a stub — separately on native Windows and inside WSL2,
   never crossing the filesystem boundary between them.
3. Git adapter for worktree-per-writer isolation (ADR 0005).
4. SQLite store adapter with the ADR 0007 schema, migration, and redaction-bypass tests.

## Phase 6: orchestrator and recovery, end-to-end on one platform
Wire the state machine driver against real adapters for one bounded, low-risk task on a
disposable Git repository, on whichever Tier A environment (native Windows or
Linux-under-WSL2) is available first. Add the fault-injection scenarios from ADR 0005: kill
mid-`running`, kill mid-`accepted`-to-`integrated`, stale base at integration, retry
exhaustion. This phase is not implemented until recovery reaches `blocked` or a correctly
new attempt in each case — never a duplicate integration.

## Phase 7: CLI, second Tier A environment, Tier 3 and plan-review acceptance paths
Implement `args`/`compose`/`report` (ADR 0001: the CLI is the sole functional entry point —
every capability must be reachable through it before anything else is proposed for
ergonomics); repeat Phase 6's scenario on the other Tier A environment (native Windows if
Phase 6 used WSL2, or vice versa) — the same repository and task graph must not be assumed
to behave identically across that boundary without this check. Add one Tier 3 scenario
proving `HumanAcceptanceEvidence` is actually required before `accepted`, and one scenario
proving a produced task graph cannot reach its first `ready` node without recorded
plan-level acceptance (ADR 0009's `awaiting-plan-review` gate). Any optional operating-skill
wrapper (`mesh-loop-planner`/`mesh-loop-executor`, ADR 0001) is explicitly deferred past
this phase, not part of it.

## Phase 8: remaining harnesses and bounded parallelism
Extend the harness adapter to Pi, Grok, and Agy behind the contract established in Phase 5,
resolving the Agy smoke-runner and Grok discovery gaps noted in initialization.md. Only
after the single-worker path is verified, enable bounded parallel execution across
low-coupling nodes per ADR 0009.

## Phase 9: packaging
Produce one static release binary per Tier A target — `meshloop-windows-x86_64.exe` via a
locked `cargo build --release` on native Windows, `meshloop-linux-x86_64` the same way
inside WSL2 — with no installer and no service registration. Verify each binary has no
undeclared runtime dependency (the MSVC/build-toolchain requirement from Phase 0 is
build-time only) and runs standalone on a clean instance of its own environment. Version
from the workspace `Cargo.toml` plus a git tag. Confirm a WSL user is documented to run the
Linux binary inside their WSL distribution, never the Windows `.exe` reaching across the
filesystem boundary.

Record each phase's implementation status (not-started -> in-progress -> implemented ->
verified) against the ADR(s) it completes, per docs/architecture/adr/README.md — not as an
update to this document.
