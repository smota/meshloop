# 0021 Restart an accepted plan without replanning

- Status: Accepted
- Implementation: implemented
- Date: 2026-09-08
- Accepted: 2026-09-08
- Author/executor: Grok, under human product direction
- Decision owner: Samuel
- Approval evidence: operator could not re-run `awesome-landscape-v1` after FailedTerminal without changing `graph_id`; they asked to start over without replanning
- Supersedes: none
- Superseded by: none

## Context
`graph_id` keys both the accepted plan and the saga attempts. After a failed wave
(`max_retries` spent, dependents Blocked), `run` returns DuplicateGraph (“choose a
new graph_id”) and `resume --retry` is a no-op. That forces a replan to retry
execution.

Worktrees stay beside the repo (`<parent>/.meshloop-worktrees/<repo>/`); this ADR
does not move them.

## Decision
1. `meshloop resume --restart` is the human-gated start-over: keep `PlanAccepted`
   and `plan_json`, delete this graph’s events/attempts/evidence, cancel live
   panes, remove leftover attempt worktrees/branches, reset the integrate
   worktree to `run_base`, then `loop_until_idle` as a first `run`.
2. `meshloop run --plan … --reset` is the same restart when the plan file is
   already on disk.
3. `--retry` and `--restart` are mutually exclusive. Restart is not implicit.
4. DuplicateGraph when the saga is terminal tells the operator to
   `resume --restart`, not to invent a new `graph_id`.

## Consequences
Restart discards the previous attempt ledger for that `graph_id`. It is not an
undo of git on `main`. Attempt ids continue from the global sequence.

## Verification
Store: reset keeps the run row and clears events/attempts. CLI: a FailedTerminal
fixture graph `resume --restart`s into AwaitingReview without changing `graph_id`.
`xtask check`.
