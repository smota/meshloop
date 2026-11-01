# 0036 World D CI exit

- Status: Accepted
- Accepted date: 2026-11-01
- Implementation: implemented in this working tree; verification is `cargo run -p xtask -- check`; no new benchmark campaign
- Date: 2026-11-01
- Author/executor: Grok, from the approved program plan in this checkout
- Reviewer: proposal review passed 2026-11-01; implementation review on 2026-11-01 required this decision section to match the Accepted status
- Decision owner: Samuel
- Approval evidence: Samuel's "approved" reply in the implementation session on 2026-11-01 for issue #90
- Supersedes: the CI exit-code consequence of [ADR 0023](0023-measurement-and-benchmark-framework.md) only; ADR 0023 remains Accepted for the measurement framework, the threshold file, World S, and the `iso.*` invariants
- Superseded by: none
- Tracking: [#90](https://github.com/smota/meshloop/issues/90), [#40](https://github.com/smota/meshloop/issues/40)

## Context and constraints

`xtask bench` already records `invariants_pass` and `thresholds_pass` separately in `xtask/src/main.rs`, then exits failure when `all_pass` is false. `benches/thresholds.toml` has 10 equality invariants and 18 performance targets.

[ADR 0023](0023-measurement-and-benchmark-framework.md) decision 2 requires World D, the CI gate, to be bit-reproducible. Decision 3 names `iso.*` as hard CI gates. The consequence sentence that every pull request protects parse latency is the part this record proposes to narrow.

Recorded evidence: PR #89 tree `b8229289` (SHAs `30daf97`, `2645acd`, `c49c5eb`) passed invariants on all six CI jobs and passed all 28 metrics on Ubuntu. Windows missed different performance targets each time: throughput `0.942454`; WAL p95 `14.9502` ms, throughput `0.822956`, mutation p95 `37.055` ms; WAL p95 `11.2744` ms. Check and smoke passed. Issue #82 retained worse misses: Windows WAL `521.3522` ms and repair `2691.2` ms, and a local canonical self-repair of `1604.2` ms. Those stay evidence. They are not erased.

## Alternatives

1. Keep one-run absolute timing as a merge veto. That was the behavior when this record was opened. The same tree fails differently across runners.
2. Lower targets or shrink samples until the historical misses pass. Rejected. Issue #40, issue #90, and ADR 0023's threshold contract forbid that.
3. Invariants plus check and smoke block CI. Performance targets stay at the same ops and values, still evaluate, and still upload. A performance-only miss does not fail the process. Feature-branch push triggers (`work/**`, `codex/**`) are removed. `pull_request` to `main` and push to `main` stay. Windows stays.

## Decision or proposal

Accepted on 2026-11-01 by the decision owner's "approved" reply, recorded on issue #90. Alternative 3 is the decision. The header status is the acceptance. The earlier draft text that said this file was only a proposal is superseded by that reply.

`xtask bench` exits through `world_d_exit`: failure when an invariant fails, success when invariants pass. `status.thresholds` is still `fail` on a performance miss, and the scorecard overall status still reports that miss. `status.overall` is `pass` only when invariants and performance targets both pass. Missing `artifacts/bench/smoke-phases.json` still exits failure. `benches/thresholds.toml` is unchanged.

## Consequences

A real Windows latency regression can merge while remaining visible on the scorecard. Release stays human-gated by [ADR 0018](0018-publishing.md). Invariant failures still block. Severe historical misses stay attached to issues #40 and #90. No `bench-system1` job. The removed diagnostic job `issue82-wal-storage-diagnostic` stays removed. Consumed issue-82 comparison batches stay unrepeated. Computation of bench stays one function in the first implementation slice because invariant measurements are interleaved with timing.

## Verification and implementation evidence

`xtask` unit tests cover a performance-only miss, an invariant-only miss, both missing, and both passing. They call `world_d_exit` and `gate_status`. The 50-sample WAL p95 test still expects index 47. `cargo run -p xtask -- check` is the local verification. No new benchmark campaign was run. `thresholds.toml` is not edited.
