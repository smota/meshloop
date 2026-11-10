# Learning-plane baseline (task set lp-2) — superseded

> **Superseded by lp-3 ([BASELINE.md](BASELINE.md)), 2026-11-10:** an independent review found that lp-2's `verify.py` could be passed by a shadowing `unittest.py` and that py-calc's ban missed aliases. The lp-2 task set and runner are at commit `3525ea0`, and `verify.py` and `taskset.json` in this record mean those versions. No lp-2 attempt added any file besides its editable file (checked against the kept git-diff evidence), so the lp-2 numbers show no sign of that inflation.

Issue [#144](https://github.com/smota/meshloop/issues/144). This was the baseline of
record that the learning-plane comparison in [#140](https://github.com/smota/meshloop/issues/140)
is measured against, recorded before the learning loop ([#139](https://github.com/smota/meshloop/issues/139))
exists. No learning-plane code was involved: none exists at the Meshloop commit used.
The superseded lp-1 record is in [BASELINE-lp-1.md](BASELINE-lp-1.md).

The thresholds at the end are **proposals**. The repository owner records the
thresholds that apply on #144.

## What changed from lp-1, and why

The architect decided on #144 that lp-1 could not tell a learning effect apart: all 24
runs went to one (harness, model), none were retried, and at Tier1 a prior bounded at
±0.25 cannot reach a mid or top model. lp-2 changes three things and keeps everything
else: the eight tasks, seeds and references, the `meshloop.toml` (same sha256), the
binary, and a new store per task × repetition.

1. **`py-calc` no-eval check.** It is now an AST walk over calls to the built-ins
   `eval`, `exec`, `compile` and `__import__` (also as `builtins.<name>`) and imports of
   `ast`. `re.compile(...)` is allowed. lp-1's regex rejected `re.compile`, which caused
   one of its two failures. `SPEC.md` now says this explicitly. `--validate`: every seed
   fails, every reference passes. A `re.compile` variant of the reference passes and an
   `eval` variant fails.
2. **Planned tiers.** `taskset.json` records the tier a plan reviewer would set, and
   the runner writes it into the plan: easy → `Tier1`, medium → `Tier2`, hard →
   `Tier3`. The plan schema takes `"Tier1"`/`"Tier2"`/`"Tier3"`. Before dispatch, the
   runner checks with `doctor` that each planned tier has at least one dispatchable
   candidate (`tier_fits`: Tier1 any, Tier2 mid or top, Tier3 top only).
3. **Operator retry protocol.** At `f0691d9` a failed deterministic check does not fall
   back on its own: `run_loop.rs` passes an empty candidate list to `maybe_fallback`,
   so the node ends `Failed`. This is why lp-1 had 0 retries. While a node is `Failed`
   and attempts remain (`max_retries = 3`), the runner now issues `resume --retry`.
   `resume` runs to idle, and the retry goes to the first candidate not yet used. No
   newer build was needed: the baseline uses the same `meshloop-f0691d9`.

## Run identity

| Item | Value |
|---|---|
| Date | 2026-11-10, 18:10:43Z start, 616 s for all 24 runs |
| Meshloop | `meshloop 0.3.0`, commit `f0691d9`, binary sha256 `cb8a912a…8115b135` |
| Runner and task set | `3525ea0` (lp-2). Token parsing was improved afterwards; the attempts were re-parsed from the kept evidence with `--aggregate` (no new dispatch) |
| Configuration | [`meshloop.toml`](meshloop.toml), sha256 `9cf1307c…f559fbcdf4923` (unchanged from lp-1) |
| Host | WSL2 Linux (kernel 6.18), Python 3.14.7, cargo 1.98.0 |
| Harness versions (doctor) | codex-cli 0.167.0, Claude Code 2.1.322, pi 1.5.0 |
| Models actually run | codex `gpt-6-luna` (6 attempts), claude `sonnet`, which claude itself reported as `claude-sonnet-5-5` (12), codex `gpt-6.1-sol` (6). Each is the `model_ref` Meshloop substituted into `--model` and recorded on the attempt row |
| Raw results | [`results/baseline-lp-2-2026-11-10.json`](results/baseline-lp-2-2026-11-10.json). It contains no paths or worker transcripts; those stay in the ignored `.agent-runs/bench-learning/baseline-lp-2/` |
| Command | `xtask bench-learning --live --reps 3 --bin <meshloop-f0691d9> --bin-commit f0691d9 --out .agent-runs/bench-learning/baseline-lp-2 --results benches/learning-plane/results/baseline-lp-2-2026-11-10.json` |

Confound: an unrelated codex session (a separate workload on the same host and ChatGPT
account) was active throughout. No quota or rate-limit signal was seen.

## Task set lp-2

Each task has a seed repository (`SPEC.md`, a stub or buggy module and visible tests),
hidden tests and a reference solution. [`verify.py`](verify.py) runs the pristine hidden
tests on a copy of the candidate tree, so edits to `tests/` cannot pass a task, and the
exit code decides the result.

| Task | Lang | Difficulty | Planned tier | What it tests |
|---|---|---|---|---|
| `py-median-bugfix` | Python | easy | Tier1 | fix even-length mean and input mutation; iterables; empty input |
| `py-wordfreq` | Python | easy | Tier1 | tokenising with inner apostrophes, `casefold`, tie order |
| `py-lru` | Python | medium | Tier2 | LRU cache: capacity validation, capacity 0, eviction callback, recency |
| `py-intervals` | Python | medium | Tier2 | half-open interval merge (touching intervals merge) and subtraction |
| `py-roman` | Python | medium | Tier2 | strict canonical Roman numeral parsing |
| `rs-brackets` | Rust | medium | Tier2 | bracket matching by character position; strings with escapes skipped |
| `py-calc` | Python | hard | Tier3 | exact rational expression evaluator; precedence; strict syntax; no `eval` |
| `py-semver` | Python | hard | Tier3 | SemVer 2.0.0 precedence; npm-style ranges with the pre-release rule |

## Configuration and what routing does with it

`selected_harnesses = ["codex", "claude", "pi"]`, `max_retries = 3`,
`task_timeout_seconds = 420`. grok is excluded (#162) and agy (#164).

| Harness | Template | Models (tier) |
|---|---|---|
| codex | `exec --model {model_ref} -` (stdin) | `gpt-6-luna` (lightweight), `gpt-6.1-sol` (top) |
| claude | `--permission-mode acceptEdits --output-format json --model {model_ref} --print {prompt}` | `haiku` (lightweight), `sonnet` (mid) |
| pi | `--model {model_ref} -p {prompt}` | `openai-codex/gpt-5.6-luna` (lightweight), `openai-codex/gpt-6.1-terra` (mid) |

With empty feedback, each candidate scores tier-efficiency + historical mean 0.5 +
exploration 0.7. Ties go to the configured order. The table shows the candidates
`doctor` listed for each tier, and what follows from `router.rs`:

| Tier | Candidates (configured order) | Baseline first dispatch | Fallback ladder | What a ±0.25 prior can change |
|---|---|---|---|---|
| Tier1 | all six | codex/`gpt-6-luna` (lightweight 1.2) | claude/`haiku` → pi/`gpt-5.6-luna` | Reorder the three lightweight arms. A mid arm can only tie at 0.95 and loses on configured order, so a lightweight arm always goes first |
| Tier2 | codex/`gpt-6.1-sol` (top 0.7), claude/`sonnet` (mid 1.2), pi/`gpt-6.1-terra` (mid 1.2) | claude/`sonnet` | pi/`gpt-6.1-terra` → codex/`gpt-6.1-sol` | Swap sonnet and terra freely. codex/`gpt-6.1-sol` goes first only at exactly the bound (a tie at 0.95, which configured order breaks in its favour) |
| Tier3 | codex/`gpt-6.1-sol` only | codex/`gpt-6.1-sol` | none: a failure ends after one attempt | Nothing. Only one candidate fits |

Tier hints are raise-only and act through plan review, so a hint can only move a task
to a costlier tier.

**Retry mechanism check (not part of the baseline).** This was a single disposable run
of `py-median-bugfix` at Tier1, with `verify_command = ["false"]` so that every attempt
fails. Attempt 1 went to codex/`gpt-6-luna`; `resume --retry` gave claude/`haiku`;
`resume --retry` gave pi/`openai-codex/gpt-5.6-luna`. The node then reported
`retry_budget_exhausted` and a further `resume --retry` dispatched nothing. This
confirms the ladder above and the runner's stop condition. The repository was deleted
afterwards; evidence is in `.agent-runs/bench-learning/retry-mechanism/`.

## Protocol and carried state

- Fixed automated protocol, no human in the loop:
  - `review-plan --accept --as claude-coordinator` of a one-node plan with the planned
    tier, then `run`;
  - `resume --retry` while the node is `Failed` and attempts remain;
  - `accept --task 1 --as claude-coordinator` only when the node is `AwaitingReview`
    (Meshloop's checks passed), `resume`, `integrate --into main --accept-integrate`;
  - the hidden tests then run on `main`, and their exit code is the run's success.
- **Adjust/Decline = 0 by protocol.** This is not a measurement of a human.
- Each task × repetition uses a new disposable repository and a new store.
  `routing_feedback` was empty before every run (`routing_feedback_before = []`,
  `store_existed_before = false` in all 24 records). Nothing Meshloop stores carries
  across tasks or repetitions. Not controlled: vendor-side state (CLI login, user-level
  config, caches, account quota windows). Every disposable repository was deleted.
- Order: repetition-major. Outer timeout 1560 s per `run` or `resume --retry`; none was
  reached.

## Results

Success means the hidden tests exit 0 on `main` after integration. Rates have 95%
Wilson intervals. Wall time is per dispatch: `run` plus any `resume --retry`.

| Metric | Baseline (lp-2) |
|---|---|
| Success rate | **100%** (24/24), 95% CI [86.2%, 100%] |
| First-attempt success | 100% (24/24) |
| Success by repetition | 8/8, 8/8, 8/8 |
| Retries | 0 in all 24 runs (mean 0, max 0); `resume --retry` was never needed |
| Retry budget exhausted | 0 runs |
| Repair loop | not triggered |
| First-dispatch distribution | Tier1: codex/`gpt-6-luna` 6/6; Tier2: claude/`sonnet` 12/12; Tier3: codex/`gpt-6.1-sol` 6/6 |
| Dispatch wall time | p50 13.5 s, p95 58.0 s (nearest rank); by repetition p50 13.5 / 13.3 / 13.5 s |
| End-to-end wall time | p50 16.3 s, p95 59.8 s |
| Adjust / Decline | 0 / 0 by protocol |
| Timeouts, quota hits | 0, 0 |

By tier:

| Tier | Runs | Success | Dispatch wall p50 / p95 (s) | p50 by repetition (s) |
|---|---|---|---|---|
| Tier1 | 6 | 6/6 (CI [61.0, 100]) | 18.5 / 23.0 | 18.6, 18.1, 16.4 |
| Tier2 | 12 | 12/12 (CI [75.8, 100]) | 11.3 / 13.8 | 12.3, 11.1, 11.8 |
| Tier3 | 6 | 6/6 (CI [61.0, 100]) | 45.5 / 58.8 | 51.8, 51.5, 49.9 |

By (harness, model):

| Harness | model_ref | Reported model | Attempts | First dispatches | Checks passed | Duration p50 / p95 (s) | Tokens reported | Tokens per attempt, mean (median) | Mean by repetition |
|---|---|---|---|---|---|---|---|---|---|
| codex | `gpt-6-luna` | — | 6 | 6 | 6/6 | 18 / 22 | 6/6 | 12,996 (13,909) | 13,830 / 10,254 / 14,903 |
| claude | `sonnet` | `claude-sonnet-5-5` | 12 | 12 | 12/12 | 11 / 14 | 12/12 † | 159,756 (156,733) | 157,503 / 165,238 / 156,528 |
| codex | `gpt-6.1-sol` | — | 6 | 6 | 6/6 | 45 / 58 | 6/6 | 30,398 (32,035) | 27,890 / 32,034 / 31,270 |
| codex | — others | | 0 | | | | | | |
| claude | `haiku` | | 0 | | | | | | |
| pi | both | | 0 | | | | | | |

† The claude total is input + output + cache read + cache creation from `modelUsage`.
About 134k of it is cache reads and about 1.2k is output (mean 1,186 output tokens).
Meshloop keeps only the last 2 KB of worker output, and in all 12 attempts that cut
claude's JSON just before the `modelUsage` key. The counters were recovered from the
per-model fields that survived, and are marked `reported (recovered from truncated
evidence)`. Codex counts follow codex's own `tokens used` definition. **Totals are not
comparable across harnesses**; compare only within one (harness, model).

By task (3 repetitions, all 3/3):

| Task | Tier | First dispatch | Dispatch wall (s) | Tokens per attempt |
|---|---|---|---|---|
| `py-median-bugfix` | 1 | codex/`gpt-6-luna` | 18.5 / 13.3 / 18.8 | 15,534 / 7,395 / 15,100 |
| `py-wordfreq` | 1 | codex/`gpt-6-luna` | 18.8 / 23.0 / 14.0 | 12,126 / 13,112 / 14,706 |
| `py-lru` | 2 | claude/`sonnet` | 13.5 / 10.8 / 10.5 | 159,733 / 156,395 / 156,371 |
| `py-intervals` | 2 | claude/`sonnet` | 11.8 / 13.8 / 12.3 | 156,244 / 190,562 / 156,273 |
| `py-roman` | 2 | claude/`sonnet` | 12.8 / 11.3 / 13.5 | 157,078 / 156,202 / 156,522 |
| `rs-brackets` | 2 | claude/`sonnet` | 11.3 / 11.0 / 11.3 | 156,956 / 157,795 / 156,944 |
| `py-calc` | 3 | codex/`gpt-6.1-sol` | 45.5 / 44.3 / 44.0 | 22,195 / 31,177 / 43,212 |
| `py-semver` | 3 | codex/`gpt-6.1-sol` | 58.0 / 58.8 / 55.8 | 33,585 / 32,892 / 19,328 |

### What lp-2 can and cannot show

- **Success is at the ceiling.** With tiers set by a reviewer, every task passed first
  time, so #140 cannot show a success gain on lp-2, only a loss. A learning effect has
  to show up as a change in routing (first dispatch), time or tokens, with success and
  retries held.
- **Fallback was not exercised in the baseline.** No run failed. The mechanism works
  (see the check above), but the baseline has no retry variance to compare against.
- **Unused arms:** claude `haiku` and both pi models were never dispatched, so their
  tokens and pass rates are unknown. pi in text mode would report no tokens.
- **The codex model name** is not in the kept evidence tail. The model of record is the
  `model_ref` passed as `--model`.
- **Human Adjust/Decline** is 0 by protocol.

## Proposed thresholds (for the coordinator to record)

These are based on the observed spread between repetitions: success 8/8 in every
repetition, retries 0. Dispatch p50 varied by 13% for Tier1 (16.4–18.6 s), 10% for
Tier2 (11.1–12.3 s) and 4% for Tier3 (49.9–51.8 s). Mean tokens per attempt varied by
about ±19% for codex/`gpt-6-luna` (only 2 tasks per repetition), ±3% for claude/`sonnet`
and ±7% for codex/`gpt-6.1-sol`. The #140 arm must use the same lp-2 task set, the same
`meshloop.toml` and protocol, 3 × 8 runs and a new store per run, and must record its
Meshloop commit and the accepted priors or hints file.

**Go** (all must hold):

1. Success ≥ 23/24: at most one failure, with no task failing in 2 or more
   repetitions.
2. Retries: at most 2 runs need `resume --retry`, and no run exhausts the retry
   budget.
3. At least one learning effect, attributable to the accepted priors or hints file:
   - **First-dispatch change.** For at least one tier, the first dispatch differs from
     the baseline (Tier1 codex/`gpt-6-luna`, Tier2 claude/`sonnet`) in at least 2 of 3
     repetitions for the same tasks, and every affected run succeeds first time; or
   - **Faster.** Dispatch wall p50 drops by at least twice the baseline spread for one
     tier, with no other tier regressing: Tier1 ≥ 25% (≤ 13.9 s), Tier2 ≥ 20%
     (≤ 9.0 s), Tier3 ≥ 10% (≤ 41.0 s); or
   - **Cheaper.** For an arm that both runs use, mean tokens per attempt drop by at
     least twice its spread: claude/`sonnet` ≥ 6% (≤ 150,170) or codex/`gpt-6.1-sol` ≥
     15% (≤ 25,840). codex/`gpt-6-luna` varies too much to count.
4. No no-go condition holds.

**No-go** (any one):

- success ≤ 22/24, or any task failing in 2 or more repetitions;
- more than 3 runs need a retry, or any run exhausts the retry budget;
- dispatch wall p50 up by more than 25% in any tier (Tier1 > 23.1 s, Tier2 > 14.1 s,
  Tier3 > 56.9 s), or overall p95 > 72.5 s;
- for an arm both runs use, mean tokens per attempt up by more than 25%;
- a first dispatch to a candidate outside the tier's configured, dispatchable set
  (`tier_candidates` in the results), or **a prior that adds or removes a candidate**,
  or a tier hint that lowers a tier (#139 contract);
- any timeout, or a quota-driven early stop that leaves fewer than 3 full repetitions.

**Inconclusive** (neither): treat as no-go, per ADR 0038 ("If #140 shows no gain,
learning stops").

Notes for the decision:

- A tier hint can only raise a tier. On lp-2 that moves a task to a slower and costlier
  arm (Tier3 dispatch p50 is about 3–4 times Tier2), so hints are likely to trip the
  time or token no-go unless they also prevent failures. On lp-2 there are no failures
  to prevent.
- At Tier3 only codex/`gpt-6.1-sol` fits, so neither priors nor fallback can change
  anything there. Adding a second top-tier model would be a config change and needs a
  new task-set version and baseline.
- Whether a failed check should fall back on its own, rather than by operator `resume
  --retry`, is a kernel question for a separate issue (`maybe_fallback(..., &[])` in
  `run_loop.rs`).

## Reproduce

```bash
cargo run -p xtask -- bench-learning --validate            # offline: seeds fail, references pass
cargo run -p xtask -- bench-learning --live --reps 3 \
  --bin <meshloop binary> --bin-commit <sha> \
  --out .agent-runs/bench-learning/<name> \
  --results benches/learning-plane/results/<name>.json     # live; uses harness quota
cargo run -p xtask -- bench-learning --aggregate .agent-runs/bench-learning/<name>/results.json \
  --results benches/learning-plane/results/<name>.json     # re-parse kept evidence
```
