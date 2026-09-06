# 0009 Planning and routing

- Status: Accepted
- Implementation: implemented for R1 — decomposition-as-dispatch, `--accept-plan` gate, QACR without invented quota numbers. Residual: quota windows are not queried from vendors; tier assignment remains the unvalidated dependency-count heuristic.
- Date: 2026-09-05
- Accepted: 2026-09-06
- Author/executor: Claude (Sonnet 5), consolidated from prior Codex/user planning; absorbs former ADR 0014 (planning and decomposition)
- Decision owner: Samuel
- Approval evidence: user approved the 2026-09-06 launch plan
- Supersedes: none
- Superseded by: none

## Context and constraints
Deciding what the work is, and deciding where each piece of it goes, are one workflow, not
two: decomposition is itself dispatched and routed like any other task. No hardcoded model
availability or blind vendor swaps; feedback cannot change authorization; decomposition
quality is not mechanically verifiable the way a test suite is.

## Alternatives
A dedicated hardcoded decomposition algorithm versus decomposition as one more agent
dispatch; mechanically verifying MECE coverage versus treating it as a review question;
static user rules versus measured adaptive routing; sequential versus bounded parallel execution.

## Decision
Decomposition is one agent dispatch (ADR 0003), routed like any task node, whose output
contract is a task graph rather than code; because decomposition quality can't be
mechanically checked, a produced graph requires human plan-level acceptance before its
first node schedules. Routing follows a named framework, Quota-Aware Capability Routing
(QACR): filter to configured and capability-confirmed candidates, filter out those in
observed rate-limit cooldown, score by tier fit / historical success / subscription-quota
headroom, break ties by coupling, dispatch with visible fallback on failure — "budget"
means subscription rate-limit headroom, never dollar cost. Scoring is extensible via
pluggable `RoutingSignal` implementations, so adding a signal, harness, or model tier never
requires changing routing's control flow. Full algorithm and planning detail is in
runtime-design.md §5.

## Consequences
Implementation must remain within accepted decisions; unresolved capabilities must be
visible. The bootstrap does not establish runtime readiness. Historical feedback can only
reorder already-configured, already-capable candidates — it can never add one or bypass a
capability rejection.

## Verification
See runtime-design.md §5's validation scenarios.
