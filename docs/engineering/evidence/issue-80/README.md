# Issue 80 workspace lifecycle evidence

This records the local gates for the integrated workspace lifecycle. It does not accept a merge, a GitHub release, or a crates.io publish. ADR 0033 stays `Implementation: not-started`. The schema stays 8. `benches/thresholds.toml` was not edited.

## Local Unix

Host OS: Linux. Commands ran from the `work/issue-80-lifecycle` worktree with `--locked --offline`.

| Command | Result |
|---|---|
| `cargo run -p xtask -- check` | exit 0. fmt, clippy `-D warnings`, and workspace tests passed. |
| `cargo run -p xtask -- smoke` | exit 0. Printed `smoke: workspace lifecycle ok`. |
| CLI `compatibility_adopts_verified_legacy_and_keeps_roots` | passed inside `xtask check` (workspace_lifecycle: 4 passed, 0 failed). |

The smoke product-phase sum printed `product wall-clock: 0.44s, gated <= 5.0s`. The workspace scenario runs after that sum and does not change `smoke.wall_clock_s`. The 1-node repair smoke printed monotonic convergence in 133.4ms.

Protected-data controls in that scenario: `protected-data.txt` and `unknown-artifact.txt` stayed byte-for-byte, `missing.txt` was not created, the name-only run was not bound, a stale adoption digest wrote nothing, a stale cleanup digest stayed `blocked`, and maintenance without a matching digest wrote nothing.

## Applicable benchmark

`smoke.wall_clock_s` remains `lte 5.0` in `benches/thresholds.toml`. The local smoke observation above is 0.44s for the product-phase sum. ADR 0036 still treats a threshold miss as a scorecard result, not a process failure by itself. This slice did not run `xtask bench` and did not repeat the issue 82 comparison batches.

CI Gate runs `xtask bench` on ubuntu-latest and windows-latest for the pull request. Those jobs are the benchmark execution. This file does not copy their numbers.

## Not run

| Check | Result | Reason |
|---|---|---|
| Local Windows | not-run | This workstation is Linux. The scenario has no `cfg` for one OS. CI Gate `windows-latest` runs the same `xtask check` and `xtask smoke`. |
| `xtask bench` locally | not-run | The issue 80 test plan is check and smoke. Thresholds were not changed. |
| Crate publish and release binaries | not-run | Human release acceptance stays separate. |
| `retained_schema5_binary_refuses_schema6_without_mutation` | ignored | Pre-existing ignored adapter test. This slice does not change it. |

No local check or smoke step failed.
