# Design patterns for implementation

This names the concrete pattern behind each proposed contract, so implementation has a
known technique to reach for instead of inventing structure ad hoc. Naming a pattern here
does not itself decide anything — it elaborates an already-proposed ADR, per AGENTS.md's
"routine changes within accepted boundaries need no new ADR." If implementing a pattern
would require deviating from the ADR it serves, that is a change to the ADR, not to this
document.

| Pattern | Serves | Module |
|---|---|---|
| Ports and Adapters (Hexagonal) | ADR 0002 | meshloop-engine::ports, meshloop-adapters::* |
| Strategy | ADR 0009 (QACR) | meshloop-engine::router |
| Table-driven state machine | ADR 0005, execution-lifecycle.md | meshloop-domain::state |
| Circuit breaker | ADR 0003, ADR 0009 | meshloop-domain::capability, meshloop-engine::router |
| Event sourcing | ADR 0005, ADR 0007 | meshloop-adapters::store, meshloop-engine::recovery |
| Repository | ADR 0007 | meshloop-engine::ports (EvidenceStore, RoutingFeedbackStore), meshloop-adapters::store |
| Command | ADR 0003 | meshloop-engine::agent |
| Template method | ADR 0003 (prompt envelope) | meshloop-engine::agent |
| Orchestration saga | ADR 0005 | meshloop-engine::orchestrator |
| Bulkhead | ADR 0001, ADR 0009 (concurrency/worktrees) | meshloop-engine::router, meshloop-adapters::git |
| Test double / fake object | testing.md's port/adapter split | every meshloop-engine unit test |

## Ports and Adapters (Hexagonal)
Serves ADR 0002's crate boundaries.
- **Design**: define every trait (`HarnessCapabilities`, `HerdrSessionPort`,
  `EvidenceStore`, `RoutingFeedbackStore`) in meshloop-engine::ports before any adapter
  exists. The trait signature is the contract; write it against what the engine needs, not
  what one adapter happens to expose.
- **Implement**: adapters depend on engine trait definitions, never the reverse — Cargo
  path dependencies already enforce this direction at the crate level.
- **Validate**: engine unit tests run only against in-memory fakes implementing the same
  trait; adapter contract tests run independently against the real external (or a fixture
  of it) and must pass the identical behavioral assertions the fakes satisfy.

## Strategy
Serves ADR 0009's extensibility requirement.
- **Design**: `trait RoutingSignal { fn score(&self, candidate: &Candidate, context: &RoutingContext) -> SignalScore }`.
  `router` holds an ordered set of signals (tier-fit, historical-success, load-balance,
  coupling) and combines their scores by the weighted sum ADR 0009 specifies.
- **Implement**: each signal is a separate, small, independently constructible type — no
  signal reads another signal's internal state, and adding one never touches `router`'s
  control flow, only its signal list.
- **Validate**: unit test each signal in isolation against synthetic candidates with known
  expected scores; separately test the weighted composition against a fixed signal set and
  known expected ordering, so a future signal addition can be tested the same way.

## Table-driven state machine
Serves ADR 0005 and execution-lifecycle.md's transition table.
- **Design**: a closed `TaskState` enum and one pure function,
  `transition(current: TaskState, event: Event) -> Result<TaskState, IllegalTransition>`,
  are the single source of truth for legality — no state check is ever duplicated as an
  `if`/`match` elsewhere in the codebase.
- **Implement**: the function's match arms mirror execution-lifecycle.md's table exactly;
  rely on Rust's exhaustiveness checking on the `(state, event)` match to catch a missing
  or mistyped row at compile time rather than at runtime.
- **Validate**: a table-driven test iterates every row in execution-lifecycle.md and
  asserts `transition()` returns exactly that result; a second test asserts every
  `(state, event)` pair *not* in the table returns `IllegalTransition`, so an unauthorized
  transition can never silently succeed.

## Circuit breaker
Serves ADR 0003's `CapacityExhausted` classification and ADR 0009's cooldown filtering.
- **Design**: each `(harness, model)` pair's quota state is a breaker — Closed (available)
  -> Open (`CapacityExhausted` observed, cooled down until `retry_after`) -> Half-Open
  (first probe after cooldown expiry) -> Closed on success or back to Open on repeat
  failure. This is the concrete shape of `QuotaState` (meshloop-domain::capability).
- **Implement**: QACR's step 2 filter (ADR 0009) is exactly "exclude candidates whose
  breaker is Open"; no separate quota-tracking logic should exist outside this state.
- **Validate**: drive a fake harness through Closed -> Open -> Half-Open -> Closed and
  assert router behavior at each stage; assert the "every candidate's breaker Open at
  once" case resolves the task to `blocked`, per ADR 0009's verification section, not a crash.

## Event sourcing
Serves ADR 0005's recovery-by-replay and ADR 0007's `events` table.
- **Design**: state is never mutated in place; it is derived by folding the durable event
  log for a task/attempt. The `events` table is the log; "current state" is a projection
  of it, never an independently authoritative column.
- **Implement**: `recovery` replays events against a fresh projection on every startup
  rather than trusting any cached current-state value, which is what makes crash recovery
  possible at all.
- **Validate**: assert replaying the same event log twice yields the same projected state
  (determinism); assert a truncated or corrupted log surfaces as `blocked`, never a guessed resume.

## Repository
Serves ADR 0007's `EvidenceStore`/`RoutingFeedbackStore` ports.
- **Design**: meshloop-engine depends only on the trait signatures — no `rusqlite` (or any
  storage-specific) type may appear in an engine module's public API.
- **Implement**: the SQLite adapter is the only place SQL and the ADR 0007 schema live; a
  future storage swap touches only meshloop-adapters::store.
- **Validate**: run the same repository contract test suite against the real SQLite
  adapter and an in-memory fake, asserting identical externally observable behavior from both.

## Command
Serves ADR 0003's `AgentSpec`.
- **Design**: `AgentSpec` is an immutable value object — task, attempt, harness, model,
  worktree, prompt, timeout, cancellation token — that fully describes one unit of work
  before anything executes it.
- **Implement**: `agent` only constructs this object; it never calls a harness directly.
  Dispatch is `orchestrator`'s job via `HarnessCapabilities::invoke`, keeping "what to do"
  separate from "who executes it," per ADR 0003's "no routing/dispatch authority" rule.
- **Validate**: unit test `AgentSpec` construction as a pure function of `TaskNode` with no
  dispatch side effect; separately test that a given `AgentSpec` reaches `invoke` unmodified.

## Template method
Serves ADR 0003's four-part prompt envelope.
- **Design**: one rendering function with four fixed slots (situation, complication,
  question, output contract) — never a different ad hoc string-builder per harness.
- **Implement**: slot content is populated only from a `TaskNode`'s own fields and its
  declared graph dependencies, per ML-012's isolation requirement.
- **Validate**: golden-output tests per slot, plus ADR 0003's named test that no context
  outside a node's declared dependencies ever appears in its rendered prompt.

## Orchestration saga
Serves ADR 0005's single-integration-owner orchestrator.
- **Design**: `orchestrator` is the one saga coordinator; adapters and agents never decide
  the next step or talk to each other directly — every transition is coordinator-initiated.
- **Implement**: every failure path has an explicit compensating action, not a generic
  catch-all: a failed attempt discards its worktree rather than merging it, a stale base at
  integration starts a new attempt rather than silently rebasing.
- **Validate**: the fault-injection scenarios ADR 0005 already names (kill mid-`running`,
  kill mid-integration, stale base, retry exhaustion) each assert the correct compensation
  ran, not merely that the happy path works when nothing fails.

## Bulkhead
Serves ADR 0001's concurrency cap and ADR 0009's per-harness load balancing.
- **Design**: concurrency limits and worktree isolation are per-harness and per-writer, so
  one overloaded or hung harness/task cannot starve scheduling of unrelated ready nodes.
- **Implement**: a stuck agent in one worktree must never block the orchestrator from
  starting or progressing an independent ready node on a different harness or worktree.
- **Validate**: hang one fake harness indefinitely and assert other ready nodes still
  schedule, run, and complete on their own timeline.

## Test double / fake object
Serves testing.md's port/adapter split and is the mechanism that makes every pattern above
independently testable before Phase 5's real adapters exist (implementation-plan.md).
- **Design**: every port gets one canonical in-memory fake, built alongside the trait in
  Phase 3 — not one ad hoc mock per test.
- **Implement**: fakes must be able to simulate failure modes the real adapters can hit
  (`CapacityExhausted`, cooldown expiry, a hung process), not only the happy path, since
  those are exactly what the circuit-breaker and bulkhead tests above need.
- **Validate**: a fake's behavior is itself covered by the same repository/adapter contract
  tests run against the real implementation (see Repository above), so the fake cannot
  silently drift from what the real adapter actually does.
