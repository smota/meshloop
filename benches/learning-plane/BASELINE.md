# Learning-plane baseline (task set lp-3)

Issue [#144](https://github.com/smota/meshloop/issues/144). This is the baseline of
record that the learning-plane comparison in [#140](https://github.com/smota/meshloop/issues/140)
is measured against, recorded before the learning loop ([#139](https://github.com/smota/meshloop/issues/139))
exists. No learning-plane code was involved: none exists at the Meshloop commit used.
Superseded records: [BASELINE-lp-2.md](BASELINE-lp-2.md) and
[BASELINE-lp-1.md](BASELINE-lp-1.md).

The thresholds at the end are **proposals**. The repository owner records the
thresholds that apply on #144.

## History of the task set

- **lp-1** could not tell a learning effect apart: one (harness, model) for all runs and
  no retries.
- **lp-2** added planned tiers per task and the operator retry protocol, and replaced
  py-calc's regex ban with an AST check.
- **lp-3** (integrity hardening after an independent review) changes how results are
  verified, and nothing about what workers see except py-calc's `SPEC.md` wording:
  - `verify.py` tests a tree built from the pristine seed, the worker's version of the
    files listed as `editable` in `task.json`, and the pristine hidden tests. Nothing
    else from the worker is copied. Stray top-level `*.py`, `conftest.py`,
    `sitecustomize.py`, `usercustomize.py` and `*.pth` files are also removed.
  - Python runs under `python3 -I`, with `unittest` imported from the standard library
    before the tested tree is put on `sys.path`. Rust runs `cargo test --offline` with
    the pristine `Cargo.toml`, `Cargo.lock` and tests.
  - `--validate` checks that every seed fails, every reference passes, and every tamper
    case fails. The tamper cases are a shadowing `unittest.py`, `sitecustomize.py`,
    `conftest.py`, a `.pth` file and edited tests (two edited test files for Rust).
    45/45 pass (seven Python tasks with 5 tamper cases each, one Rust task with 2). The lp-2 driver passed an unsolved seed that carried a shadowing
    `unittest.py`; lp-3 fails it.
  - py-calc's ban flags any use of the names `eval`, `exec`, `compile` and `__import__`
    in any context, string constants equal to those names, attribute access to them on
    `builtins`/`__builtins__`, and imports of `builtins`, `importlib` and `ast`. The
    hidden test includes the review's cases (`e = eval`, `from builtins import eval as
    e`, `getattr(builtins, 'eval')`, `__builtins__['eval']`, `import builtins as b;
    b.eval`, `importlib.import_module('ast')`). `re.compile` is still allowed.
  - `common/AGENTS.md` is now tracked. The shared `.git/info/exclude` had kept it out of
    the repository. The earlier runs used the same local file, so their worker
    instructions were the same.
  - The runner aborts a task when `review-plan` fails, removes the disposable repository
    on every early return, and exits non-zero if results cannot be written. Token
    counts are split per arm into fully reported, recovered and not reported.

**Was lp-2 inflated?** No evidence of it.
- In every attempt of lp-2 (and of lp-1), the kept git-diff evidence lists exactly one
  changed file, the task's editable file. A shadowing `unittest.py`,
  `sitecustomize.py`, `conftest.py` or `*.pth` would have shown there; the only paths
  it would not show are ignored ones, which the old driver did not copy.
- The py-calc source itself was not kept (the disposable repositories are deleted), so
  a banned alias in lp-2's py-calc solutions cannot be ruled out from evidence. The
  workers' transcripts show only `evaluate(` and codex's `exec` log markers.

## Run identity

| Item | Value |
|---|---|
| Date | 2026-11-10, 18:32:32Z start, 565 s for all 24 runs |
| Meshloop | `meshloop 0.3.0`, commit `f0691d9`, binary sha256 `cb8a912a…8115b135` (same binary as lp-1 and lp-2) |
| Runner and task set | `20a1277` (lp-3) |
| Configuration | [`meshloop.toml`](meshloop.toml), sha256 `9cf1307c…f559fbcdf4923` (unchanged since lp-1) |
| Host | WSL2 Linux (kernel 6.18), Python 3.14.7, cargo 1.98.0 |
| Harness versions (doctor) | codex-cli 0.167.0, Claude Code 2.1.322, pi 1.5.0 |
| Models actually run | codex `gpt-6-luna` (6 attempts), claude `sonnet`, which claude itself reported as `claude-sonnet-5-5` (12), codex `gpt-6.1-sol` (6). Each is the `model_ref` Meshloop substituted into `--model` and recorded on the attempt row |
| Raw results | [`results/baseline-lp-3-2026-11-10.json`](results/baseline-lp-3-2026-11-10.json). It contains no paths or worker transcripts; those stay in the ignored `.agent-runs/bench-learning/baseline-lp-3/` |
| Command | `xtask bench-learning --live --reps 3 --bin <meshloop-f0691d9> --bin-commit f0691d9 --out .agent-runs/bench-learning/baseline-lp-3 --results benches/learning-plane/results/baseline-lp-3-2026-11-10.json` |

Confound: an unrelated codex session (a separate workload on the same host and ChatGPT
account) was active during all three baselines. No quota or rate-limit signal was seen.

## Task set lp-3

Each task has a seed repository (`SPEC.md`, a stub or buggy module and visible tests),
hidden tests, a reference solution and an `editable` list. The worker is told the
`test_command` in `task.json` as its own check. The result comes only from `verify.py`.

| Task | Lang | Difficulty | Planned tier | Editable | What it tests |
|---|---|---|---|---|---|
| `py-median-bugfix` | Python | easy | Tier1 | `stats.py` | fix even-length mean and input mutation; iterables; empty input |
| `py-wordfreq` | Python | easy | Tier1 | `wordfreq.py` | tokenising with inner apostrophes, `casefold`, tie order |
| `py-lru` | Python | medium | Tier2 | `lru.py` | LRU cache: capacity validation, capacity 0, eviction callback, recency |
| `py-intervals` | Python | medium | Tier2 | `intervals.py` | half-open interval merge (touching intervals merge) and subtraction |
| `py-roman` | Python | medium | Tier2 | `roman.py` | strict canonical Roman numeral parsing |
| `rs-brackets` | Rust | medium | Tier2 | `src/lib.rs` | bracket matching by character position; strings with escapes skipped |
| `py-calc` | Python | hard | Tier3 | `calc.py` | exact rational expression evaluator; precedence; strict syntax; no `eval` in any form |
| `py-semver` | Python | hard | Tier3 | `semver.py` | SemVer 2.0.0 precedence; npm-style ranges with the pre-release rule |

## Configuration and routing

`selected_harnesses = ["codex", "claude", "pi"]`, `max_retries = 3`,
`task_timeout_seconds = 420`. grok is excluded (#162) and agy (#164).

| Harness | Template | Models (tier) |
|---|---|---|
| codex | `exec --model {model_ref} -` (stdin) | `gpt-6-luna` (lightweight), `gpt-6.1-sol` (top) |
| claude | `--permission-mode acceptEdits --output-format json --model {model_ref} --print {prompt}` | `haiku` (lightweight), `sonnet` (mid) |
| pi | `--model {model_ref} -p {prompt}` | `openai-codex/gpt-5.6-luna` (lightweight), `openai-codex/gpt-6.1-terra` (mid) |

With empty feedback, each candidate scores tier-efficiency + historical mean 0.5 +
exploration 0.7. Ties go to the configured order. The table shows the candidates
`doctor` listed for each tier (recorded as `tier_candidates` in the results), and what
follows from `router.rs` at `f0691d9`:

| Tier | Candidates | Baseline first dispatch | Fallback ladder (`resume --retry`) | What a ±0.25 prior can change |
|---|---|---|---|---|
| Tier1 | all six | codex/`gpt-6-luna` | claude/`haiku` → pi/`gpt-5.6-luna` | Reorder the three lightweight arms. A mid arm can only tie at 0.95 and loses on configured order |
| Tier2 | codex/`gpt-6.1-sol` (top), claude/`sonnet` (mid), pi/`gpt-6.1-terra` (mid) | claude/`sonnet` | pi/`gpt-6.1-terra` → codex/`gpt-6.1-sol` | Swap sonnet and terra freely. codex/`gpt-6.1-sol` goes first only at exactly the bound (a tie at 0.95, broken by configured order) |
| Tier3 | codex/`gpt-6.1-sol` only | codex/`gpt-6.1-sol` | none | Nothing |

At `f0691d9` a failed deterministic check does not fall back on its own
(`maybe_fallback(..., &[])` in `run_loop.rs`). That is why the protocol issues `resume
--retry`. A forced-failure check made during lp-2 confirmed the Tier1 ladder above and
the budget stop (see [BASELINE-lp-2.md](BASELINE-lp-2.md)). Tier hints are raise-only
and act through plan review.

## Protocol and carried state

- Fixed automated protocol, no human in the loop:
  - `review-plan --accept --as claude-coordinator` of a one-node plan with the planned
    tier; if it fails, the task is aborted and `run` is never started;
  - `run`, then `resume --retry` while the node is `Failed` and attempts remain;
  - `accept --task 1 --as claude-coordinator` only when the node is `AwaitingReview`,
    `resume`, `integrate --into main --accept-integrate`;
  - `verify.py` on `main`, whose exit code is the run's success.
- **Adjust/Decline = 0 by protocol.** This is not a measurement of a human.
- Each task × repetition uses a new disposable repository and a new store.
  `routing_feedback_before = []` and `store_existed_before = false` in all 24 records.
  Nothing Meshloop stores carries across tasks or repetitions. Not controlled:
  vendor-side state (CLI login, user-level config, caches, account quota windows). All
  24 disposable repositories were removed.
- Order: repetition-major. Outer timeout 1560 s per `run` or `resume --retry`; none was
  reached.

## Results

Success means the hidden tests pass through `verify.py` on `main` after integration.
Rates have 95% Wilson intervals. Wall time is per dispatch: `run` plus any `resume
--retry`.

| Metric | Baseline (lp-3) |
|---|---|
| Success rate | **100%** (24/24), 95% CI [86.2%, 100%] |
| First-attempt success | 100% (24/24) |
| Success by repetition | 8/8, 8/8, 8/8 |
| Retries | 0 in all 24 runs; `resume --retry` never needed; retry budget never exhausted |
| Repair loop | not triggered |
| First-dispatch distribution | Tier1: codex/`gpt-6-luna` 6/6; Tier2: claude/`sonnet` 12/12; Tier3: codex/`gpt-6.1-sol` 6/6 |
| Dispatch wall time | p50 13.3 s, p95 41.8 s (nearest rank) |
| End-to-end wall time | p50 16.5 s, p95 43.6 s |
| Adjust / Decline | 0 / 0 by protocol |
| Timeouts, quota hits, aborted tasks | 0, 0, 0 |

By tier:

| Tier | Runs | Success | Dispatch wall p50 / p95 (s) | Median by repetition (s) |
|---|---|---|---|---|
| Tier1 | 6 | 6/6 (CI [61.0, 100]) | 20.8 / 26.3 | 19.5, 23.0, 22.6 |
| Tier2 | 12 | 12/12 (CI [75.8, 100]) | 11.3 / 13.3 | 11.3, 12.4, 11.3 |
| Tier3 | 6 | 6/6 (CI [61.0, 100]) | 38.8 / 47.0 | 38.2, 40.2, 40.9 |

By (harness, model):

| Harness | model_ref | Reported model | Attempts = first dispatches | Checks passed | Duration p50 / p95 (s) | Token basis | Tokens per attempt, mean (median) | Mean by repetition |
|---|---|---|---|---|---|---|---|---|
| codex | `gpt-6-luna` | — | 6 | 6/6 | 21 / 26 | fully reported, 6/6 | 11,654 (7,559) | 7,404 / 18,606 / 8,951 |
| claude | `sonnet` | `claude-sonnet-5-5` | 12 | 12/12 | 11 / 13 | **recovered only**, 12/12 | 156,806 (156,899) | 156,508 / 156,731 / 157,179 |
| codex | `gpt-6.1-sol` | — | 6 | 6/6 | 38 / 46 | fully reported, 6/6 | 19,349 (18,061) | 24,828 / 17,136 / 16,082 |
| claude | `haiku` | | 0 | | | | | |
| codex | others | | 0 | | | | | |
| pi | both | | 0 | | | | | |

**Token figures.**
- **Recovered-only arm: claude/`sonnet`.** Meshloop keeps only the last 2 KB of worker
  output, and in all 12 claude attempts that cut the `--output-format json` result
  before the `modelUsage` key. The counters were recovered from the per-model fields
  that survived. The total is input + output + cache read + cache creation: on average
  about 134k cache read and 1,175 output tokens.
- **Fully reported arms: both codex arms**, from codex's own `tokens used` line.
- **Comparing:** totals are not comparable across harnesses. The #140 arm must be parsed
  by the same runner, so that recovered counts are compared with recovered counts.
  Fixing the evidence truncation (#175) is out of scope here.

By task (3 repetitions, all 3/3, all first attempt):

| Task | Tier | First dispatch | Dispatch wall (s) | Tokens per attempt |
|---|---|---|---|---|
| `py-median-bugfix` | 1 | codex/`gpt-6-luna` | 18.3 / 24.8 / 19.0 | 5,435 / 5,743 / 5,136 |
| `py-wordfreq` | 1 | codex/`gpt-6-luna` | 20.8 / 21.3 / 26.3 | 9,374 / 31,470 / 12,766 |
| `py-lru` | 2 | claude/`sonnet` | 11.0 / 11.3 / 11.5 | 156,365 / 156,366 / 157,246 |
| `py-intervals` | 2 | claude/`sonnet` | 11.3 / 12.8 / 12.5 | 156,300 / 156,294 / 157,165 |
| `py-roman` | 2 | claude/`sonnet` | 12.0 / 12.0 / 11.0 | 156,429 / 156,861 / 156,938 |
| `rs-brackets` | 2 | claude/`sonnet` | 11.3 / 13.3 / 10.8 | 156,936 / 157,404 / 157,368 |
| `py-calc` | 3 | codex/`gpt-6.1-sol` | 41.8 / 38.8 / 47.0 | 19,147 / 17,730 / 18,392 |
| `py-semver` | 3 | codex/`gpt-6.1-sol` | 34.5 / 41.5 / 34.8 | 30,509 / 16,541 / 13,773 |

### Variation between batches (lp-2 and lp-3)

The lp-2 and lp-3 runs used the same binary, configuration, tiers and worker
instructions, about 20 minutes apart. They differ only in verification, which runs
after the worker. Both reached 24/24 with no retries and the same first dispatches. The
worker-side numbers moved between the two batches:

| Metric | lp-2 | lp-3 | Change |
|---|---|---|---|
| Tier1 dispatch p50 | 18.5 s | 20.8 s | +12% |
| Tier2 dispatch p50 | 11.3 s | 11.3 s | 0% |
| Tier3 dispatch p50 | 45.5 s | 38.8 s | −15% |
| codex/`gpt-6-luna` tokens, mean | 12,996 | 11,654 | −10% |
| claude/`sonnet` tokens, mean (recovered) | 159,756 | 156,806 | −2% |
| codex/`gpt-6.1-sol` tokens, mean | 30,398 | 19,349 | −36% |

#140 will run in a later batch, so the thresholds below use the larger of twice the
spread within lp-3 and this shift between batches.

### What lp-3 can and cannot show

- **Success is at the ceiling** (24/24 in lp-2 and lp-3). #140 can show a loss, but no
  success gain. A learning effect has to show up as a change in first dispatch, time or
  tokens, with success and retries held.
- **Fallback was not exercised** in the baseline. The mechanism was checked separately
  during lp-2.
- **Unused arms:** claude `haiku` and both pi models were never dispatched. pi in text
  mode reports no tokens.
- **Too noisy for a token criterion:** codex/`gpt-6-luna` (a ×2.5 swing between
  repetitions, driven by one 31k `py-wordfreq` attempt) and codex/`gpt-6.1-sol` (±23%
  within lp-3, −36% between batches).
- **The codex model name** is not in the kept evidence tail. The model of record is the
  `model_ref` passed as `--model`.
- **Human Adjust/Decline** is 0 by protocol.

## Proposed thresholds (for the coordinator to record)

The structure is unchanged from the version recorded on #144; the numbers are
re-derived from lp-3. Margins come from the larger of twice the spread between lp-3
repetitions and the lp-2 → lp-3 shift, rounded up to a multiple of 5%. Wall-time
spread within lp-3, from the median of each repetition: Tier1 17%, Tier2 10%, Tier3 7%.
Mean tokens per attempt between repetitions: claude/`sonnet` ±0.2%; the codex arms are
too noisy (see above).

The #140 arm must use:
- the lp-3 task set, the same `meshloop.toml`, protocol and runner (token parsing
  included);
- 3 × 8 runs with a new store per run;
- a recorded Meshloop commit and the accepted priors or hints file.

**Go** (all must hold):

1. Success ≥ 23/24: at most one failure, with no task failing in 2 or more
   repetitions.
2. Retries: at most 2 runs need `resume --retry`, and no run exhausts the retry
   budget.
3. At least one learning effect, attributable to the accepted priors or hints file:
   - **First-dispatch change.** For at least one tier, the first dispatch differs from
     the baseline (Tier1 codex/`gpt-6-luna`, Tier2 claude/`sonnet`) in at least 2 of 3
     repetitions for the same tasks, and every affected run succeeds first time; or
   - **Faster.** Dispatch wall p50 drops for one tier by its margin, with no other tier
     regressing past its no-go: Tier1 ≥ 35% (≤ 13.5 s), Tier2 ≥ 20% (≤ 9.0 s), Tier3 ≥
     15% (≤ 33.0 s); or
   - **Cheaper.** Mean tokens per attempt for claude/`sonnet` drop ≥ 5% (≤ 148,966),
     using recovered counts in both arms. The 5% is a floor: the measured spread is
     ±0.2% within lp-3 and −2% between batches. The codex arms are excluded as too
     noisy.
4. No no-go condition holds.

**No-go** (any one):

- success ≤ 22/24, or any task failing in 2 or more repetitions;
- more than 3 runs need a retry, or any run exhausts the retry budget;
- dispatch wall p50 up by more than its margin, and never less than 25%: Tier1 >
  28.0 s (+35%), Tier2 > 14.1 s (+25%), Tier3 > 48.5 s (+25%); or overall p95 > 52.2 s
  (+25%);
- claude/`sonnet` mean tokens per attempt up more than 25% (> 196,008), or a codex arm's
  mean more than doubling (> 23,308 for `gpt-6-luna`, > 38,698 for `gpt-6.1-sol`);
- a first dispatch to a candidate outside the tier's configured, dispatchable set
  (`tier_candidates`), or **a prior that adds or removes a candidate**, or a tier hint
  that lowers a tier (#139 contract);
- any timeout, an aborted task, or a quota-driven early stop that leaves fewer than 3
  full repetitions.

**Inconclusive** (neither): treat as no-go, per ADR 0038 ("If #140 shows no gain,
learning stops").

Notes for the decision:

- A tier hint can only raise a tier. On lp-3 that moves a task to a slower and costlier
  arm: Tier3 dispatch p50 is about 3.4 times Tier2's. With no failures to prevent, a
  hint is likely to trip the time no-go.
- At Tier3 only codex/`gpt-6.1-sol` fits, so priors and fallback can change nothing
  there. A second top-tier model would be a config change and needs a new task-set
  version.
- Whether a failed check should fall back on its own, without an operator `resume
  --retry`, is a kernel question for a separate issue.

## Reproduce

```bash
cargo run -p xtask -- bench-learning --validate            # offline: seeds fail, references pass, tamper cases fail
cargo run -p xtask -- bench-learning --live --reps 3 \
  --bin <meshloop binary> --bin-commit <sha> \
  --out .agent-runs/bench-learning/<name> \
  --results benches/learning-plane/results/<name>.json     # live; uses harness quota
cargo run -p xtask -- bench-learning --aggregate .agent-runs/bench-learning/<name>/results.json \
  --results benches/learning-plane/results/<name>.json     # re-parse kept evidence
```
