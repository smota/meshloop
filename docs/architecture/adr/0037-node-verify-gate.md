# 0037 Node verify gate

- Status: Accepted
- Accepted date: 2026-11-02
- Implementation: implemented in this working tree
- Date: 2026-11-02
- Author/executor: Grok, from the approved backlog plan
- Reviewer: none
- Decision owner: Samuel
- Approval evidence: Samuel approved the backlog plan on 2026-11-02; that plan specified this gate, executor `meshloop:verify-gate`, Tier 3 rejection, and the unchanged plan-level gate
- Supersedes: none
- Superseded by: none
- Tracking: [#53](https://github.com/smota/meshloop/issues/53)

## Context and constraints

[ADR 0016](0016-r1-closed-loop.md) requires every node to stop at `AwaitingReview` until `meshloop accept`. This record amends that every-node accept rule for one case only.

An optional node field `gate` is `human` (default) or `verify`. A verify node whose tier is explicit Tier 1 or Tier 2 reaches `Accepted` when deterministic checks pass. The accepting event is `VerifyGatePassed`. Its executor is exactly `meshloop:verify-gate`. There is no `meshloop accept` call on that path.

`gate = verify` on Tier 3 fails plan validation. `gate = verify` with an omitted tier also fails validation, because `TaskGraph::validate` runs before `assign_tiers`. Omitted tier is not treated as Tier 2. Omitted `gate` stays human. The runtime still refuses auto-accept when tier is omitted or Tier 3 if validation was bypassed.

[ADR 0007](0007-verification.md) Tier 3 human acceptance stays unconditional. [ADR 0009](0009-routing-budgets.md) plan gate (`accept_plan`, `decide_plan`, `plan_transition`, `meshloop review-plan`) is unchanged. `ModelReviewPassed` stays a separate transition and is not this path. External gates such as waiting for a PR merge are out of scope.

## Alternatives

1. Keep every node on `meshloop accept`. That remains the default when `gate` is omitted or `human`.
2. Auto-accept every Tier 1 and Tier 2 node, or route the verify gate through `ModelReviewPassed`. Rejected. The plan must opt in per node, and Tier 3 must not pass through model review or an omitted tier.
3. This gate. Explicit `verify` on Tier 1 or Tier 2 accepts after deterministic checks, under executor `meshloop:verify-gate`.

## Decision or proposal

Accepted on 2026-11-02. Alternative 3 is the decision. This amends the R1 every-node accept rule in ADR 0016 for this case only.

## Consequences

Existing plans and snapshots omit `gate` and stay human; the field is not serialized for the default. A failed check does not append the gate. A resume that already stored `DeterministicChecksPassed` and is `AwaitingReview` still appends the gate event once. An already `Accepted` node is not appended again. Plan-level acceptance is unchanged.

## Verification and implementation evidence

Domain tests cover Tier 3 verify, omitted tier plus verify, Tier 1 verify, Tier 2 verify, and omitted gate deserializing as human. Engine tests cover Tier 1 verify ending `Accepted` with executor `meshloop:verify-gate`, a human gate staying `AwaitingReview`, and a Tier 3 verify node built without validation staying `AwaitingReview`. No schema bump. Implementation is in this working tree.
