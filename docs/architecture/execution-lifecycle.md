# Proposed execution lifecycle

This is input to ADR 0005; accepting ADR 0005 accepts this table as part of the decision,
not as a separately-approved document.

## States
pending -> ready -> running -> verifying -> awaiting-review -> accepted -> integrated.
Side states reachable from running/verifying/awaiting-review/accepted: failed, cancelled, blocked.

| From | To | Trigger | Precondition |
|---|---|---|---|
| pending | ready | dependency-satisfied event | all upstream task nodes reached `integrated`; scope authorization still valid |
| pending | blocked | dependency-failed or scope-revoked event | an upstream task reached `failed`/`cancelled`, or authorization was withdrawn |
| ready | running | attempt-started event | concurrency budget available (ADR 0009); harness capability confirmed (ADR 0003) |
| running | verifying | harness-exited event | exit code recorded; harness exit success alone never satisfies the later `accepted` precondition |
| running | failed | harness-crashed or timeout event | owned process tree confirmed terminated |
| running | cancelled | user/operator cancel event | owned process tree confirmed terminated |
| verifying | awaiting-review | deterministic-checks-passed event | DeterministicEvidence bound to exact candidate revision (ADR 0007) |
| verifying | failed | deterministic-checks-failed event | failure evidence retained, redacted (ADR 0007) |
| awaiting-review | accepted | model-review-passed (Tier 1/2, policy-eligible) or human-acceptance-recorded (Tier 3, always) event | evidence contract satisfied (ADR 0007); Tier 3 never transitions on model review alone |
| awaiting-review | failed | review-rejected event | rejection reason retained |
| accepted | integrated | integration-owner-merge event | single integration owner (ADR 0005); base revision still current |
| accepted | failed | stale-base-detected event | integration blocked, new attempt required, never a silent rebase |
| any non-terminal | cancelled | explicit cancel | process tree terminated, no partial integration |
| failed | ready | retry-authorized event, new attempt id | retry budget (`max_retries`) not exhausted |

## Attempts and idempotency
A task can have multiple attempts; each attempt gets its own identifier and its own event
stream. A transition is a pure function of (current persisted state, event, evidence) —
never of wall-clock assumptions about what "must have happened by now." Recovery replays
the persisted event log against actual process/Git/worktree state before deciding to
resume, retry, or mark `blocked`; it never resumes an attempt whose owning process cannot
be reattached to or confirmed dead.

Every transition record carries: event type, reason, executor identity, task id, attempt
id, the relevant revision/evidence reference, and the precondition checked. Retries always
mint a new attempt id — never replay the same attempt id with different evidence, which
would make evidence ambiguous about which run it validates.

Timeout and cancellation terminate the owned process tree, preserve redacted evidence, and
land in `failed`/`cancelled` — never `accepted` or `integrated`, even partially.

## Plan-level gate (ADR 0009)
Before any node in a graph is scheduled, the graph itself passes through
`awaiting-plan-review -> plan-accepted`, parallel to and preceding the per-node states
above — this gates the whole graph on human acceptance for any objective at or above the
lowest risk tier, since decomposition quality is not mechanically checkable the way a
single node's deterministic evidence is. No node reaches `ready` while its graph is still
`awaiting-plan-review`.

## Open questions for ADR 0005 acceptance
Exactly-once integration semantics when the integration owner process itself crashes
mid-merge, and whether `blocked` needs sub-reasons (dependency-failed vs. scope-revoked)
as distinct states rather than one state with a reason field, are unresolved. Publication
is outside task execution unless separately authorized.
