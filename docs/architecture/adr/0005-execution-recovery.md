# 0005 Execution, transport, isolation, and recovery

- Status: Accepted
- Implementation: implemented for R1 — state machine, kept worktrees, integrate worktree, Herdr CLI transport composed for live workers, `pane_id` on attempts. Residual: concurrency remains 1; native Herdr protocol still deferred.
- Date: 2026-09-05
- Accepted: 2026-09-06
- Author/executor: Claude (Sonnet 5), consolidated from prior Codex/user planning; absorbs former ADRs 0004 (herdr transport) and 0006 (isolation and integration)
- Decision owner: Samuel
- Approval evidence: user approved the 2026-09-06 launch plan (Herdr is the live transport)
- Supersedes: none
- Superseded by: none

## Context and constraints
How a session is transported to a worker, how that worker's changes are isolated, and how
the whole run recovers from a crash are one lifecycle, not three separate decisions — a
transport failure, an isolation conflict, and a crash all resolve through the same state
machine. Do not assume Unix sockets work on Windows or WSL. Panes provide no filesystem
isolation. Retry budgets, cancellation, and duplicate side effects must be bounded.

## Alternatives
CLI invocation versus native Herdr transport; worktrees versus patch-only workers; serial
merge versus a staged integration branch; event log versus mutable snapshots; automatic
resumption versus explicit recovery decisions.

## Decision
Herdr sessions are reached only via CLI-subprocess invocation in v1 (structured arguments,
never shell-string interpolation); a native transport is deferred pending a platform spike,
not adopted or rejected. Each writer gets an isolated Git worktree; one integration owner
merges, with candidate-bound validation, blocking on stale bases or conflicts rather than
auto-resolving. Panes are never trusted as the isolation boundary — worktrees are. Every
task moves through the explicit, durable state machine in execution-lifecycle.md
(`pending` through `integrated`, plus `failed`/`cancelled`/`blocked`), with attempt
identities that are never reused and recovery that reconciles the event log against real
process/Git/worktree state before resuming anything. Full detail is in runtime-design.md §3.

## Consequences
Implementation must remain within accepted decisions; unresolved capabilities must be
visible. The bootstrap does not establish runtime readiness. Harness process exit success
is never sufficient for `accepted`; a persisted `running` state with no matching live
process becomes `blocked`, never a silent retry.

## Verification
See runtime-design.md §3's validation scenarios.
