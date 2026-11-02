# 0032 System 1 decision port

- Status: Accepted
- Accepted date: 2026-11-02
- Implementation: domain value types in this change. Ports, the adapter, and the actuators are later issues and are not implemented here.
- Date: 2026-11-02
- Author/executor: Grok, from the approved backlog plan
- Reviewer: none
- Decision owner: Samuel
- Approval evidence: Samuel approved the backlog plan on 2026-11-02. That plan accepted these System 1 invariants and said new decision metrics get their own table when #19 lands. World D numbers stay.
- Supersedes: none
- Superseded by: none
- Tracking: [#11](https://github.com/smota/meshloop/issues/11) and epic [#10](https://github.com/smota/meshloop/issues/10)

## Context and constraints

Epic #10 needs a System 1 decision boundary that stays inside the workspace split in [ADR 0002](0002-workspace-boundaries.md). `meshloop-domain` may hold value types only. Engine ports, the HTTP adapter, and the actuators are later issues and are outside this record's implementation.

[system1-benchmark-and-validation-spec.md](../system1-benchmark-and-validation-spec.md) section 6 proposed System 1 rows for `benches/thresholds.toml` upon acceptance of this ADR. This ADR does not append those rows. They wait for #19 and a separate table. World D numbers stay. This record does not edit that spec and does not edit the threshold file.

## Alternatives

1. Leave System 1 untyped in later engine and adapter code, with no domain values and no recorded invariants. Rejected. The raise-only tier rule and the review band need a stable decision before those issues land.
2. Append the section 6 rows to `benches/thresholds.toml` in this change. Rejected. The approved plan keeps new decision metrics for #19 on a separate table. World D numbers stay.
3. Accept the invariants below. Land only pure domain value types now. Ports, the HTTP adapter, and the actuators stay later issues.

## Decision or proposal

Accepted on 2026-11-02. Alternative 3 is the decision. Approving the backlog plan accepted these invariants.

1. Hexagonal purity. `meshloop-domain` holds pure value types. Engine ports and the HTTP adapter come later. No network types in the domain.
2. Raise-only tier ratchet. Effective tier is the max of the declared tier and the risk floor. A probability at or above 0.90 of architectural, security, unsafe/native, or migration risk raises to Tier 3. Never downgrade.
3. Dual-threshold review. `p >= 0.95` and deterministic checks passed can pass. `p <= 0.05` fails. Otherwise abstain and stay `AwaitingReview`. Tier 3 never accepts model review.
4. Repair observation stays pure. A mechanical micro-repair is allowed only when every blocking atom is allowlisted, there are zero syntax errors, `P(Mechanical) >= 0.70`, and `P(Semantic) <= 0.20`. It costs one observation, then the existing rollback.
5. Predictive-success oracle fusion is not a routing arm. The prior is clipped. Exploration uses real pulls only. Spend is a separate integer ledger.
6. Context rerank is one batched query over the unpinned suffix. `quant.rs` stays offline. Nothing is dropped. Any error keeps the existing order.
7. Offline CI uses a scripted decision port and opens no sockets. A live external decision API is manual opt-in and is outside CI.

## Consequences

This change can represent a unit-interval confidence, a single-line abstain reason, a non-empty choice, a finite score, a boolean, and a raise-only tier floor. It does not open a socket, call a model, or decide a review. The 0.90 risk classes, the 0.95/0.05 review band, the repair predicate, the oracle clip and spend ledger, the rerank rule, and the scripted CI port remain later issues. They are not functions here.

No rows are appended to `benches/thresholds.toml`. Section 6 of the System 1 benchmark spec stays as written. New decision metrics get their own table when #19 lands. [ADR 0036](0036-world-d-ci-exit.md) World D numbers stay.

## Verification and implementation evidence

Domain value types and their unit tests are `crates/meshloop-domain/src/decision.rs`. Checked with `--locked --offline` on base `75c30c3`:

- `cargo fmt -p meshloop-domain` left the package rustfmt-clean.
- `cargo test -p meshloop-domain --lib --locked --offline` reported `test result: ok. 74 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`.
- `cargo clippy -p meshloop-domain --lib --locked --offline -- -D warnings` finished with no warnings.

Ports, the adapter, and the actuators are not implemented.
