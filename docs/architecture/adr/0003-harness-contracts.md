# 0003 Harness capability and agent dispatch contract

- Status: Proposed
- Implementation: in-progress — HarnessCapabilities, the error taxonomy, and AgentSpec/prompt dispatch implemented and contract-tested against a fixture binary; `probe` also run for real against installed herdr/claude/pi/grok binaries; `invoke` never dispatched against a real subscription-backed harness. See docs/engineering/implementation-status.md.
- Date: 2026-09-05
- Author/executor: Claude (Sonnet 5), consolidated from prior Codex/user planning; absorbs former ADR 0013 (agent dispatch)
- Reviewer: pending
- Approval evidence: none for this runtime decision
- Supersedes: none
- Superseded by: none

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
