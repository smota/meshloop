# Learning-plane comparison: results (#140)

Issue [#140](https://github.com/smota/meshloop/issues/140) compares runs with and without
accepted advisory routing priors (#139) against the lp-4 baseline of record
([`BASELINE.md`](../../benches/learning-plane/BASELINE.md)). The go/no-go uses the
thresholds fixed in advance in the **final** comment on
[#144](https://github.com/smota/meshloop/issues/144#issuecomment-6776613560), copied
without change into
[`thresholds-lp-4.json`](../../benches/learning-plane/thresholds-lp-4.json). Nothing was
tuned. The verdict below is what the numbers give. Recording go or no-go on #140 is the
repository owner's decision.

**Verdict the numbers give: INCONCLUSIVE, which counts as NO-GO.** All no-go conditions
are clear, but the GO set needs a learning effect traceable to the promoted priors, and
there was none. The priors were in effect in all 24 measured stores and never changed a
routing decision (`changed_top` = 0 of 24). This is the outcome
[`learning-plane.md`](../architecture/learning-plane.md) predicts for a task set at the
success ceiling: every warm-up arm passed every attempt, so every prior clips to the same
fused mean and the order stays the baseline's.

## Arms

| Arm | What | Runs | Where |
|---|---|---|---|
| Baseline (lp-4) | Meshloop `f0691d9`, no learning plane | 3 × 8 | [`baseline-lp-4-2026-11-10.json`](../../benches/learning-plane/results/baseline-lp-4-2026-11-10.json) |
| Warm-up (exploration) | Meshloop `29651a8`, each candidate pinned in turn, one shared store, never measured | 66 | [`warmup-lp-4-2026-11-11.json`](../../benches/learning-plane/results/warmup-lp-4-2026-11-11.json) |
| Priors on (measured) | Meshloop `29651a8`, the cell's proposal promoted into every fresh store | 3 × 8 | [`priors-on-lp-4-2026-11-11.json`](../../benches/learning-plane/results/priors-on-lp-4-2026-11-11.json) |
| Offline baseline / priors on | fixture harness, CI | 3 × 4 each | `artifacts/bench/learning_offline.json` (CI artifact) |
| Board / worker coordination on/off | **not exercised** | — | see Limits |

## Protocol

**Measured arm.** It used the lp-4 protocol and runner without change. The task set and
`verify.py` were unchanged, and so was
[`meshloop.toml`](../../benches/learning-plane/meshloop.toml): sha256 `9cf1307c…cdf4923`,
the same hash as the baseline. Each task × repetition got a new disposable repository and
store (`store_existed_before = false`, `routing_feedback_before = []` in all 24). The run
sequence was:

1. `review-plan --accept`;
2. `run`, then `resume --retry` while the node is `Failed` and attempts remain;
3. `accept`, `resume`, `integrate --into main`;
4. a trusted `verify.py` pass on `main` decides success.

The one addition was a step before `review-plan`:
`meshloop priors accept --file priors-lp-4-warmup.json --as claude-coordinator`. All 24
accepts exited 0. After each run, the runner recorded `priors show` and the per-attempt
`advisory_routing` rows. The arm ran from 2026-11-11T10:20:04Z and took 612 s. The
command was
`xtask bench-learning --live --reps 3 --priors benches/learning-plane/results/priors-lp-4-warmup.json --bin <meshloop-29651a8> --bin-commit 29651a8`.

**Warm-up (exploration by pinning, ADR 0038 decision (c)).** Each run's configuration
allowed one candidate. `selected_harnesses` named only that candidate's harness, and that
harness's `models` list named only that model. The pinned configurations' hashes are in the
results file (`warmup.pinned_config_sha256`). Each candidate ran on every task of each tier
it fits, with enough repetitions for each (harness, model, tier) key to reach the kernel's
evidence floor of n ≥ 5:

| Tier | Candidates | Tasks | Repetitions | Runs |
|---|---|---|---|---|
| Tier1 | 6 | 2 | 3 | 36 |
| Tier2 | 3 | 4 | 2 | 24 |
| Tier3 | 1 | 2 | 3 | 6 |

Meshloop binds a store to one repository, and the cell folds one store. So all 66 runs
shared one repository and store. Before each run, `main` became a single root commit
holding the task's seed, the same tree and commit message a fresh repository gets. A
pinned run has no fallback, so the warm-up skipped `resume --retry`, `accept` and
`integrate`. Its evidence is the attempt's deterministic check, which runs the same
`verify.py` (nonce result line). Warm-up runs are never measured. The warm-up ran from
2026-11-11T09:52:16Z and took 1,640 s.

**Fold.** The real cell ran `/meshloop learn --graph … (66 graphs) --out …` through the
nodecode gateway's `POST /api/gateway/slash`, with no model turn
([`cell_learn.py`](../../benches/learning-plane/cell_learn.py)). It reused the load test's
organism:

- a throwaway `HOME`, an emptied environment and `update.mode = off`;
- a private copy of the pinned release `98dd6dc80`, checked against the load test's
  sha256 list.

The work folder, release copy and home were removed afterwards, and no process was left.

The cell counted 66 attempts and excluded 0 (every reason 0). `meshloop priors check`
with the frozen configuration reported:

- 10 usable entries and 0 ignored;
- no file fault;
- only report-level `provenance_unresolved` diagnostics, expected because the measured
  stores are fresh.

The proposal is
[`priors-lp-4-warmup.json`](../../benches/learning-plane/results/priors-lp-4-warmup.json),
**sha256 `b4ee3e2901b6aeee20eca59cd0a55474a4e9919e5368d1bc9a87ef559727066c`**. Every
measured store reported it in effect with this digest.

### Warm-up evidence per key

π = (s + 1) / (n + 2). On a fresh store (n_local = 0, μ_local = 0.5), the fused mean is π
clipped to [0.25, 0.75]. Token figures come from the same parser as the baseline.

| Harness / model | Tier | n | Passed | π | Fused (fresh store) | Duration p50 (s) | Tokens per attempt, mean (basis) |
|---|---|---|---|---|---|---|---|
| codex / `gpt-6-luna` | Tier1 | 6 | 6 | 0.875 | 0.75 | 19.5 | 14,020 (reported 6/6) |
| codex / `gpt-6.1-sol` | Tier1 | 6 | 6 | 0.875 | 0.75 | 25.5 | 20,899 (reported 6/6) |
| claude / `haiku` | Tier1 | 6 | 6 | 0.875 | 0.75 | 16.5 | not reported (0/6 recovered, #175) |
| claude / `sonnet` | Tier1 | 6 | 6 | 0.875 | 0.75 | 11.5 | 156,330 (recovered 3/6) |
| pi / `openai-codex/gpt-5.6-luna` | Tier1 | 6 | 6 | 0.875 | 0.75 | 30.5 | not reported (pi prints none) |
| pi / `openai-codex/gpt-6.1-terra` | Tier1 | 6 | 6 | 0.875 | 0.75 | 17.0 | not reported |
| codex / `gpt-6.1-sol` | Tier2 | 8 | 8 | 0.900 | 0.75 | 30.0 | 16,097 (reported 8/8) |
| claude / `sonnet` | Tier2 | 8 | 8 | 0.900 | 0.75 | 11.0 | 157,189 (recovered 6/8) |
| pi / `openai-codex/gpt-6.1-terra` | Tier2 | 8 | 8 | 0.900 | 0.75 | 20.5 | not reported |
| codex / `gpt-6.1-sol` | Tier3 | 6 | 6 | 0.875 | 0.75 | 56.5 | 18,744 (reported 6/6) |

All 66 warm-up attempts passed: 95% Wilson CI [94.5%, 100%], every one with
`dispatch_mode = subprocess` and `model_passed = true`. Every candidate's prior clips to
0.75, so the historical signal rises by the same 0.25 for every candidate of a tier. The
score order is unchanged. The ties fall back to configured order, as in the baseline:
codex/`gpt-6-luna` at Tier1 and claude/`sonnet` at Tier2. At Tier2, codex/`gpt-6.1-sol`
stays 0.5 behind because of its tier-efficiency penalty.

## Results: every metric, every arm

Rates carry 95% Wilson intervals. Wall time is per dispatch (`run` plus any `resume
--retry`), with nearest-rank percentiles, the same code for both arms.

| Metric | Baseline lp-4 (`f0691d9`) | Priors on (`29651a8`) |
|---|---|---|
| Runs | 24 | 24 |
| Success | 24/24 (100%, CI [86.2, 100]) | 24/24 (100%, CI [86.2, 100]) |
| First-attempt success | 24/24 (CI [86.2, 100]) | 24/24 (CI [86.2, 100]) |
| Success by repetition | 8/8, 8/8, 8/8 | 8/8, 8/8, 8/8 |
| Runs needing a retry / retries | 0 / 0 | 0 / 0 |
| Retry budget exhausted | 0 | 0 |
| Repair iterations (attempts with the repair loop / rounds) | 0 / 0 | 0 / 0 |
| First dispatch Tier1 | codex/`gpt-6-luna` 6/6 | codex/`gpt-6-luna` 6/6 |
| First dispatch Tier2 | claude/`sonnet` 12/12 | claude/`sonnet` 12/12 |
| First dispatch Tier3 | codex/`gpt-6.1-sol` 6/6 | codex/`gpt-6.1-sol` 6/6 |
| `changed_top` (attempts with an `advisory_routing` row) | — (no learning plane) | **0 of 24** (24 of 24) |
| Dispatch wall p50 / p95 | 13.76 / 49.03 s | 15.01 / 50.52 s |
| End-to-end wall p50 / p95 | 16.79 / 51.05 s | 17.79 / 52.80 s |
| Tier1 dispatch p50 / p95 (median by rep) | 18.51 / 32.27 (20.1, 19.6, 24.6) | 19.52 / 48.02 (31.5, 20.3, 21.1) |
| Tier2 dispatch p50 / p95 (median by rep) | 11.01 / 13.76 (11.5, 11.3, 11.5) | 12.01 / 15.01 (12.8, 11.9, 11.6) |
| Tier3 dispatch p50 / p95 (median by rep) | 41.02 / 74.79 (45.5, 39.6, 56.5) | 39.77 / 55.78 (45.1, 50.3, 30.4) |
| codex/`gpt-6-luna` tokens per attempt, mean (median; by rep) | 11,604 (11,112; 10,082 / 8,034 / 16,696), reported 6/6 | 12,686 (12,029; 12,518 / 16,663 / 8,878), reported 6/6 |
| claude/`sonnet` tokens per attempt, mean (median; by rep) | 157,002 (157,111; 156,944 / 157,130 / 156,909), recovered 11/12 | 157,371 (157,285; 157,419 / 157,419 / 157,257), recovered 10/12 |
| codex/`gpt-6.1-sol` tokens per attempt, mean (median; by rep) | 16,979 (15,631; 17,896 / 15,462 / 17,580), reported 6/6 | 17,449 (16,642; 16,613 / 20,696 / 15,036), reported 6/6 |
| Human plan Adjust / Decline | 0 / 0 by protocol | 0 / 0 by protocol |
| Timeouts, quota hits, aborted tasks | 0, 0, 0 | 0, 0, 0 |

Per task, the first dispatch is the same in all 3 repetitions in both arms. No run in
either arm needed a retry.

### Offline arms (fixture, CI)

From `cargo run -p xtask -- bench-learning --offline`
([`benches/learning-plane/offline/`](../../benches/learning-plane/offline/)). One fixture
harness lists five models:

- `fail-lite` (lightweight) and `fail-mid` (mid) leave no change, so they fail Meshloop's
  git-diff check on every task;
- `lite` (lightweight), `top` (top) and `extra` (lightweight, in the warm-up configuration
  only) pass every task but `t1-never`.

The warm-up ran 40 pinned runs over 7 keys:

| Key | Tier | n | Passed | π |
|---|---|---|---|---|
| `extra` | Tier1 | 6 | 6 | 0.875 |
| `lite` | Tier1 | 6 | 6 | 0.875 |
| `top` | Tier1 | 6 | 6 | 0.875 |
| `fail-lite` | Tier1 | 6 | 0 | 0.125 |
| `fail-mid` | Tier1 | 6 | 0 | 0.125 |
| `top` | Tier2 | 5 | 5 | 0.857 |
| `fail-mid` | Tier2 | 5 | 0 | 0.143 |

No attempts were excluded. Each arm ran 4 tasks × 3 repetitions:

| Metric | Offline baseline | Offline priors on |
|---|---|---|
| Success | 9/12 (CI [46.8, 91.1]) | 9/12 (CI [46.8, 91.1]) |
| First-attempt success | 0/12 (CI [0, 24.2]) | 6/12 (CI [25.4, 74.6]) |
| Retries | 18 | 12 |
| Repair rounds | 39 | 30 |
| First dispatch Tier1 / Tier2 | `fail-lite` 9 / `fail-mid` 3 | `lite` 9 / `fail-mid` 3 |
| `changed_top` attempts | 0 | 9 (the 9 Tier1 first attempts) |
| Dispatch wall p50 / p95 | under 2 s (fixture, not meaningful) | under 2 s |
| Tokens | not reported (fixture) | not reported |
| Adjust / Decline | 0 / 0 by protocol | 0 / 0 by protocol |

Success is 9/12 in both arms because `t1-never` fails for every worker. The 15 mechanism
gates in
[`offline/thresholds.toml`](../../benches/learning-plane/offline/thresholds.toml) all pass:

- the priors are applied (in effect with the promoted digest, and an `advisory_routing`
  row for every attempt, none in the baseline);
- no dispatch falls outside the configured candidates. The prior for `extra` is reported
  `not_a_candidate`, and `extra` is never dispatched;
- `t1-never` walks the same four Tier1 candidates in both arms;
- the bound holds. At Tier2, `fail-mid` (π 0.14) and `top` (π 0.86) differ by 0.71, but the
  clipped means only tie `top`'s tier-efficiency penalty, so `fail-mid` still goes first;
- `changed_top` is true exactly on the first attempts whose dispatch differs from the
  baseline's.

The Rust fold matches the real cell's proposal for the same fixture byte for byte, apart
from `generated_at` and `generator`
([`offline/cell-fold.json`](../../benches/learning-plane/offline/cell-fold.json)). The
offline arm checks this on every run. A unit test, `priors_fold::tests`, checks the same
for the live warm-up's recorded fold input
([`warmup-lp-4-fold-input.json`](../../benches/learning-plane/results/warmup-lp-4-fold-input.json)).
The offline numbers show the mechanism works when the evidence separates arms. They are
not evidence that learning helps on real tasks. The offline file's digest changes on
every run, because `generated_at` changes.

## Go / no-go, condition by condition

Computed by `xtask bench-learning --compare --baseline baseline-lp-4-2026-11-10.json
--measured priors-on-lp-4-2026-11-11.json`. A learning effect counts only when it is
traceable to the promoted file, meaning `advisory_routing.changed_top` is true on the
affected attempts.

| Condition (final #144) | Observed | Holds |
|---|---|---|
| Precondition: same task set, `meshloop.toml` hash, 3 repetitions | lp-4; `9cf1307c…`; 3 | yes |
| Precondition: the promoted file is in effect in every measured store | 24 of 24, sha256 `b4ee3e29…` | yes |
| **GO 1**: success ≥ 23/24 and no task failing in ≥ 2 repetitions | 24/24; none | yes |
| **GO 2**: ≤ 2 runs need a retry; none exhausts the budget | 0; 0 | yes |
| **GO 3a**: first dispatch for some tier differs from baseline in ≥ 2 of 3 repetitions, every affected run first-time | 0 runs differ | **no** |
| **GO 3b**: Tier2 wall p50 ≤ 10.5 s with no tier past its no-go | 12.01 s, and no Tier2 `changed_top` | **no** |
| **GO 3c**: claude/`sonnet` mean ≤ 149,152 or codex/`gpt-6.1-sol` ≤ 10,188 | 157,371; 17,449; `changed_top` 0 | **no** |
| GO 3 (any of 3a, 3b, 3c) | none | **no** |
| NO-GO: success ≤ 22/24, or a task failing in ≥ 2 repetitions | 24/24; none | no |
| NO-GO: > 3 runs need a retry, or any run exhausts the budget | 0; 0 | no |
| NO-GO: wall p50 above Tier1 28.7 s, Tier2 13.8 s, Tier3 75.9 s; overall p95 above 61.3 s | 19.52 / 12.01 / 39.77 s; p95 50.52 s | no |
| NO-GO: tokens above claude/`sonnet` 196,253, codex/`gpt-6-luna` 23,209, codex/`gpt-6.1-sol` 33,958 | 157,371 / 12,686 / 17,449 | no |
| NO-GO: a first dispatch outside the tier's dispatchable set; a prior that adds or removes a candidate; a tier hint that lowers a tier | 0 outside; candidate set equal to the baseline's; no tier hints (v1 carries none) | no |
| NO-GO: any timeout, aborted task, or quota stop before 3 full repetitions | 0, 0, 24/24 runs | no |

No no-go condition holds. GO 1 and GO 2 hold, and GO 3 does not. Neither GO nor NO-GO
holds, so the result is **INCONCLUSIVE, which counts as NO-GO**. Per #132's stop
condition, that means keep WP-2..WP-4 as delivered and close #141 as not planned. That
decision is the repository owner's to record on #140.

The GO 3b and 3c numbers fail even before traceability: the Tier2 p50 rose from 11.01 s to
12.01 s, and the token means moved by under 1% (`sonnet`) and +3% (`gpt-6.1-sol`). Those
moves are within the batch drift recorded for lp-2 to lp-4. With `changed_top` = 0, they
could not have come from the priors anyway. Per-attempt tokens of a fixed arm have no
causal path from a routing prior.

## Limits

- **Ceiling.** lp-4 is at the success ceiling, and so was every warm-up arm (66/66). A
  routing prior encodes success only, so it could not separate any candidate here. The
  architect decided no further task-set iterations before #140 (final #144 comment).
- **Binary.** The baseline ran `f0691d9` and the measured arm ran `29651a8` (it has #139;
  the baseline predates it). The two arms therefore differ by binary as well as priors.
  No no-priors control ran on `29651a8`. Because `changed_top` was 0, no difference
  between the arms can be attributed to the priors.
- **One machine, one day apart.** WSL2, with the same harness versions as the baseline
  (codex-cli 0.167.0, Claude Code 2.1.322, pi 1.5.0). The batches were about 15.5 hours
  apart, and drift between batches is the dominant source of variation (BASELINE.md).
  Vendor-side state (logins, caches, quota windows) is not controlled.
- **Sample sizes.** n = 6 runs at Tier1 and Tier3 and 12 at Tier2 per arm. Tier1 and Tier3
  wall times are dominated by single outliers: 48.0 s `py-wordfreq` in repetition 1 here,
  74.8 s `py-semver` in the baseline.
- **Tokens.** claude counts are recovered from the truncated 2 KB evidence tail (#175):
  10 of 12 `sonnet` attempts in the measured arm and 11 of 12 in the baseline, with the
  same parser in both. In the warm-up, `haiku` recovered 0 of 6. pi prints no token
  counts. Totals are not comparable across harnesses.
- **Warm-up differs from the measured protocol.** It used pinned configurations, one
  shared repository and store, and no retry or integrate. Workers could see the
  branches of earlier warm-up graphs, though not on `main`. Its evidence is Meshloop's
  verify check, not a final verify on `main`.
- **Human Adjust/Decline** is 0 by protocol, not a measurement.
- **Board and worker-coordination arm not exercised.** #137 delivers only a one-way Board
  projection, and #138's workers are not in the lp-4 configuration, so the issue's
  "worker coordination on the Board on and off" arm has nothing to measure. It is left
  out, not reported as neutral.
- **Offline arm.** It runs the fixture harness on synthetic outcomes. It gates the
  mechanism, never "learning wins", and takes about two minutes, so it is a separate CI
  step rather than part of `xtask check`.

## Cost and quota

- The warm-up made 66 live attempts: codex 26, claude 20, pi 20. The measured arm made 24:
  codex 12, claude 12.
- No quota or rate-limit signal appeared in either: no worker output matched, and no
  Meshloop breaker opened. Nothing stopped early.
- Wall time was 1,640 s for the warm-up and 612 s for the measured arm.

## Reproduce

See [`testing.md`](testing.md) §2.4b. The raw evidence (worktrees, transcripts and the
warm-up store) stayed in the ignored `.agent-runs/` and was removed after this record.
