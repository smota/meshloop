# 0003 Harness capability and agent dispatch contract

- Status: Accepted
- Implementation: implemented for R1 — `HarnessCapabilities` (fixture subprocess + live Herdr worker); worktree diff is the deliverable. Residual: per-harness CLI argv templates are unused on the live path (`Herdr --kind` is).
- Date: 2026-09-05
- Accepted: 2026-09-06
- Author/executor: Claude (Sonnet 5), consolidated from prior Codex/user planning; absorbs former ADR 0013 (agent dispatch)
- Decision owner: Samuel
- Approval evidence: user approved the 2026-09-06 launch plan (live Herdr workers are R1)
- Supersedes: none
- Superseded by: none

> **Scope narrowed by ADR 0039 (proposed), 2026-11-10.** If [ADR 0039](0039-native-sub-agent-dispatch.md) is accepted, dispatch through the `HarnessCapabilities` port (probe/invoke/cancel/collect) applies to subprocess attempts only. Native worker attempts are parked and reported by the origin session without `invoke`, `collect` or `cancel`. The worktree diff stays the verified deliverable for every mode, and ADR 0007 is unchanged. This note does not change this record's decision for subprocess attempts.

## Context and constraints
Verifying what a harness can do, and turning a task into a concrete dispatched unit of
work, are one contract, not two: nothing can be routed or dispatched without both.
Executable identity, instruction discovery, permissions, output, cancellation, model
selection, and version compatibility must all be verified, never assumed.

## Alternatives
Direct CLI adapters versus normalized wrapper protocols; automatic discovery versus
explicit configured capabilities; a rich per-harness request/response protocol versus a
minimal, harness-agnostic dispatch envelope; trusting harness stdout versus treating the
worktree diff as the real deliverable.

## Decision
Define a versioned `HarnessCapabilities` port (probe/invoke/cancel/collect) for Codex,
Claude Code, Pi, Grok, and Agy, rejecting unsupported operations rather than guessing. Every
failure classifies under one error taxonomy (`CapacityExhausted`/`Unsupported`/`Timeout`/
`ProcessFault`) so routing and recovery react correctly instead of treating all failures
alike. A task node becomes a dispatched `AgentSpec` — task, chosen harness/model, worktree,
a deterministically-rendered prompt — built by an agent module with no routing authority of
its own. The worktree diff, not harness stdout, is the verified deliverable. Full contract
detail is in runtime-design.md §2.

## Consequences
Implementation must remain within accepted decisions; unresolved capabilities must be
visible. The bootstrap does not establish runtime readiness. A harness that cannot be
probed, or whose only output channel is free-text chat with no file-system access, is
`Unsupported` and never a routing candidate.

## Verification
See runtime-design.md §2's validation scenarios.
