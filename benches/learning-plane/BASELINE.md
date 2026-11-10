# Learning-plane baseline (task set lp-4)

Issue [#144](https://github.com/smota/meshloop/issues/144). This is the baseline of
record that the learning-plane comparison in [#140](https://github.com/smota/meshloop/issues/140)
is measured against, recorded before the learning loop ([#139](https://github.com/smota/meshloop/issues/139))
exists. No learning-plane code was involved: none exists at the Meshloop commit used.
Superseded records: [BASELINE-lp-3.md](BASELINE-lp-3.md),
[BASELINE-lp-2.md](BASELINE-lp-2.md) and [BASELINE-lp-1.md](BASELINE-lp-1.md).

The thresholds at the end are **proposals**. The repository owner records the
thresholds that apply on #144.

## History of the task set

- **lp-1:** a single (harness, model) for all runs and no retries, so it could not tell
  a learning effect apart.
- **lp-2:** planned tiers per task, the operator retry protocol, and an AST ban in
  py-calc.
- **lp-3** (first review): `verify.py` tests only the pristine seed, the worker's
  `editable` files and the hidden tests, with Python isolated. Tamper cases were added
  to `--validate`. py-calc's ban covers aliases. `common/AGENTS.md` is tracked.
- **lp-4** (second review, this record) adds a trusted result channel. The editable
  code runs in the test process, so lp-3 trusted a bare exit 0 that `os._exit(0)` at
  import, or Rust's `process::exit(0)` inside `check()`, could produce.
  - **Python.** The parent sends a random nonce on the runner's stdin. The runner reads
    it into a function local and closes fd 0 before importing tested code. Only after a
    successful run does it print `RESULT ok tests=<n> nonce=<nonce>`. A pass needs exit
    0, that exact line, and `<n>` equal to `expected_tests` in `task.json`.
  - **Rust.** A pass needs exit 0, every `test result:` line ok, and passed counts adding
    up to `expected_tests` (13). Cargo runs with `CARGO_*`/`RUST*` variables removed, an
    explicit `RUSTUP_HOME` and a fresh `CARGO_HOME` (review item N3).
  - **New tamper cases** in `--validate`, written into the editable module: exit at
    import, a forged result line, reading stdin for the nonce, and a Rust exit with a
    forged summary. All 67 checks pass: 7 Python tasks × (seed/reference + 8 tamper
    cases), and Rust's seed/reference + 3 tamper cases. Both exit cases passed under
    the lp-3 driver and fail under lp-4.

**Threat model** (review item N2). This is a benchmark of cooperative workers, not a
security boundary. The worker's editable code runs inside the test process. The checks
stop accidental and simple passes: extra files, edited tests, an early exit, a forged
result line. They do not stop code that deliberately patches the test framework from
inside the process (for example replacing `unittest.TestResult.wasSuccessful`), reads
the nonce out of the runner's memory, or prints a Rust summary with the right count and
then exits.

**Were earlier baselines inflated?** No evidence of it. Every kept success of lp-1 (22),
lp-2 (24) and lp-3 (24) has a final-verify output with a complete test summary: Python
`Ran <n> tests … OK` with that version's test count, or Rust `test result: ok.` lines
totalling 13. An early exit would have left no summary. The attempt-level verify
evidence agrees. The kept git-diffs show that no attempt changed a file besides its
editable one.

## Run identity

| Item | Value |
|---|---|
| Date | 2026-11-10, 18:50:56Z start, 608 s for all 24 runs |
| Meshloop | `meshloop 0.3.0`, commit `f0691d9`, binary sha256 `cb8a912a…8115b135` (same binary as all earlier baselines) |
| Runner and task set | `f397d37` (lp-4) |
| Configuration | [`meshloop.toml`](meshloop.toml), sha256 `9cf1307c…f559fbcdf4923` (unchanged since lp-1) |
| Host | WSL2 Linux (kernel 6.18), Python 3.14.7, cargo 1.98.0 |
| Harness versions (doctor) | codex-cli 0.167.0, Claude Code 2.1.322, pi 1.5.0 |
| Models actually run | codex `gpt-6-luna` (6 attempts), claude `sonnet`, which claude itself reported as `claude-sonnet-5-5` (12), codex `gpt-6.1-sol` (6). Each is the `model_ref` Meshloop substituted into `--model` and recorded on the attempt row |
| Raw results | [`results/baseline-lp-4-2026-11-10.json`](results/baseline-lp-4-2026-11-10.json). It contains no paths or worker transcripts; those stay in the ignored `.agent-runs/bench-learning/baseline-lp-4/` |
| Command | `xtask bench-learning --live --reps 3 --bin <meshloop-f0691d9> --bin-commit f0691d9 --out .agent-runs/bench-learning/baseline-lp-4 --results benches/learning-plane/results/baseline-lp-4-2026-11-10.json` |

Confound: an unrelated codex session (a separate workload on the same host and ChatGPT
account) was active during all baselines. No quota or rate-limit signal was seen.

## Task set lp-4

Each task has a seed repository (`SPEC.md`, a stub or buggy module and visible tests),
hidden tests, a reference solution, an `editable` list and `expected_tests`. The worker
is told the `test_command` in `task.json` as its own check. The result comes only from
`verify.py`. Workers see the same files as in lp-3.

| Task | Lang | Difficulty | Planned tier | Editable | Hidden tests | What it tests |
|---|---|---|---|---|---|---|
| `py-median-bugfix` | Python | easy | Tier1 | `stats.py` | 11 | fix even-length mean and input mutation; iterables; empty input |
| `py-wordfreq` | Python | easy | Tier1 | `wordfreq.py` | 12 | tokenising with inner apostrophes, `casefold`, tie order |
| `py-lru` | Python | medium | Tier2 | `lru.py` | 10 | LRU cache: capacity validation, capacity 0, eviction callback, recency |
| `py-intervals` | Python | medium | Tier2 | `intervals.py` | 20 | half-open interval merge and subtraction |
| `py-roman` | Python | medium | Tier2 | `roman.py` | 7 | strict canonical Roman numeral parsing |
| `rs-brackets` | Rust | medium | Tier2 | `src/lib.rs` | 13 | bracket matching by character position; strings with escapes skipped |
| `py-calc` | Python | hard | Tier3 | `calc.py` | 13 | exact rational expression evaluator; no `eval` in any form |
| `py-semver` | Python | hard | Tier3 | `semver.py` | 15 | SemVer 2.0.0 precedence; npm-style ranges with the pre-release rule |

## Configuration and routing

`selected_harnesses = ["codex", "claude", "pi"]`, `max_retries = 3`,
`task_timeout_seconds = 420`. grok is excluded (#162) and agy (#164).

| Harness | Template | Models (tier) |
|---|---|---|
| codex | `exec --model {model_ref} -` (stdin) | `gpt-6-luna` (lightweight), `gpt-6.1-sol` (top) |
| claude | `--permission-mode acceptEdits --output-format json --model {model_ref} --print {prompt}` | `haiku` (lightweight), `sonnet` (mid) |
| pi | `--model {model_ref} -p {prompt}` | `openai-codex/gpt-5.6-luna` (lightweight), `openai-codex/gpt-6.1-terra` (mid) |

| Tier | Candidates (`tier_candidates` in the results) | Baseline first dispatch | Fallback ladder (`resume --retry`) | What a ±0.25 prior can change |
|---|---|---|---|---|
| Tier1 | all six | codex/`gpt-6-luna` | claude/`haiku` → pi/`gpt-5.6-luna` | Reorder the three lightweight arms. A mid arm can only tie at 0.95 and loses on configured order |
| Tier2 | codex/`gpt-6.1-sol` (top), claude/`sonnet` (mid), pi/`gpt-6.1-terra` (mid) | claude/`sonnet` | pi/`gpt-6.1-terra` → codex/`gpt-6.1-sol` | Swap sonnet and terra freely. codex/`gpt-6.1-sol` goes first only at exactly the bound (a tie at 0.95, broken by configured order) |
| Tier3 | codex/`gpt-6.1-sol` only | codex/`gpt-6.1-sol` | none | Nothing |

At `f0691d9` a failed deterministic check does not fall back on its own
(`maybe_fallback(..., &[])` in `run_loop.rs`), so the protocol issues `resume --retry`.
A forced-failure check during lp-2 confirmed the Tier1 ladder and the budget stop
([BASELINE-lp-2.md](BASELINE-lp-2.md)).

## Protocol and carried state

- Fixed automated protocol, no human in the loop:
  - `review-plan --accept --as claude-coordinator` of a one-node plan with the planned
    tier; if it fails, the task is aborted and `run` is never started;
  - `run`, then `resume --retry` while the node is `Failed` and attempts remain;
  - `accept --task 1 --as claude-coordinator` only when the node is `AwaitingReview`,
    `resume`, `integrate --into main --accept-integrate`;
  - `verify.py` on `main`, whose trusted pass decides the run's success.
- **Adjust/Decline = 0 by protocol.** This is not a measurement of a human.
- Each task × repetition uses a new disposable repository and a new store.
  `routing_feedback_before = []` and `store_existed_before = false` in all 24 records.
  Nothing Meshloop stores carries across tasks or repetitions. Not controlled:
  vendor-side state (CLI login, user-level config, caches, account quota windows). All
  24 disposable repositories were removed.
- Order: repetition-major. Outer timeout 1560 s per `run` or `resume --retry`; none was
  reached.

## Results

Success means a trusted `verify.py` pass on `main` after integration: the 21 Python
final verifications carry the nonce line, and the 3 Rust ones add up to 13 passed. Rates
have 95% Wilson intervals. Wall time is per dispatch: `run` plus any `resume --retry`.

| Metric | Baseline (lp-4) |
|---|---|
| Success rate | **100%** (24/24), 95% CI [86.2%, 100%] |
| First-attempt success | 100% (24/24) |
| Success by repetition | 8/8, 8/8, 8/8 |
| Retries | 0 in all 24 runs; `resume --retry` never needed; retry budget never exhausted |
| Repair loop | not triggered |
| First-dispatch distribution | Tier1: codex/`gpt-6-luna` 6/6; Tier2: claude/`sonnet` 12/12; Tier3: codex/`gpt-6.1-sol` 6/6 |
| Dispatch wall time | p50 13.8 s, p95 49.0 s (nearest rank) |
| End-to-end wall time | p50 16.8 s, p95 51.1 s |
| Adjust / Decline | 0 / 0 by protocol |
| Timeouts, quota hits, aborted tasks | 0, 0, 0 |

By tier:

| Tier | Runs | Success | Dispatch wall p50 / p95 (s) | Median by repetition (s) |
|---|---|---|---|---|
| Tier1 | 6 | 6/6 (CI [61.0, 100]) | 18.5 / 32.3 | 20.1, 19.6, 24.6 |
| Tier2 | 12 | 12/12 (CI [75.8, 100]) | 11.0 / 13.8 | 11.5, 11.3, 11.5 |
| Tier3 | 6 | 6/6 (CI [61.0, 100]) | 41.0 / 74.8 | 45.5, 39.7, 56.5 |

By (harness, model):

| Harness | model_ref | Reported model | Attempts = first dispatches | Checks passed | Duration p50 / p95 (s) | Token basis | Tokens per attempt, mean (median) | Mean by repetition |
|---|---|---|---|---|---|---|---|---|
| codex | `gpt-6-luna` | — | 6 | 6/6 | 18 / 32 | fully reported, 6/6 | 11,604 (11,112) | 10,082 / 8,034 / 16,696 |
| claude | `sonnet` | `claude-sonnet-5-5` | 12 | 12/12 | 11 / 13 | **recovered only**, 11/12; 1 not reported | 157,002 (157,111) | 156,944 / 157,130 / 156,909 |
| codex | `gpt-6.1-sol` | — | 6 | 6/6 | 41 / 74 | fully reported, 6/6 | 16,979 (15,631) | 17,896 / 15,462 / 17,580 |
| claude | `haiku` | | 0 | | | | | |
| codex | others | | 0 | | | | | |
| pi | both | | 0 | | | | | |

**Token figures.**
- **Recovered-only arm: claude/`sonnet`.** Meshloop keeps only the last 2 KB of worker
  output, which cuts claude's `--output-format json` result before `modelUsage`. 11
  attempts were recovered from the per-model counters that survived. In one attempt
  (repetition 3, `rs-brackets`) the cut also removed the input and output counters, so
  it is recorded as not reported, never estimated. The total is input + output + cache
  read + cache creation: on average about 134k cache read and 1,186 output tokens.
- **Fully reported arms: both codex arms**, from codex's `tokens used` line.
- **Comparing:** totals are not comparable across harnesses, and #140 must use the same
  parser. #175 (the evidence truncation) is out of scope.

By task (3 repetitions, all 3/3, all first attempt):

| Task | Tier | First dispatch | Dispatch wall (s) | Tokens per attempt |
|---|---|---|---|---|
| `py-median-bugfix` | 1 | codex/`gpt-6-luna` | 18.5 / 18.3 / 32.3 | 8,099 / 11,066 / 22,235 |
| `py-wordfreq` | 1 | codex/`gpt-6-luna` | 21.8 / 21.0 / 17.0 | 12,066 / 5,002 / 11,158 |
| `py-lru` | 2 | claude/`sonnet` | 12.0 / 10.8 / 11.0 | 156,382 / 157,162 / 156,426 |
| `py-intervals` | 2 | claude/`sonnet` | 10.8 / 10.8 / 11.0 | 157,155 / 157,111 / 157,033 |
| `py-roman` | 2 | claude/`sonnet` | 12.0 / 11.8 / 12.0 | 157,322 / 156,450 / 157,269 |
| `rs-brackets` | 2 | claude/`sonnet` | 11.0 / 12.8 / 13.8 | 156,916 / 157,798 / not reported |
| `py-calc` | 3 | codex/`gpt-6.1-sol` | 49.0 / 41.0 / 38.3 | 20,213 / 15,357 / 15,683 |
| `py-semver` | 3 | codex/`gpt-6.1-sol` | 42.0 / 38.3 / 74.8 | 15,578 / 15,567 / 19,477 |

### Variation between batches (lp-2, lp-3, lp-4)

All three batches used the same binary, configuration, tiers and worker instructions,
about 20 minutes apart. They differ only in verification, which runs after the worker.
All three reached 24/24 with no retries and identical first dispatches.

| Metric | lp-2 | lp-3 | lp-4 |
|---|---|---|---|
| Tier1 dispatch p50 | 18.5 s | 20.8 s | 18.5 s |
| Tier2 dispatch p50 | 11.3 s | 11.3 s | 11.0 s |
| Tier3 dispatch p50 | 45.5 s | 38.8 s | 41.0 s |
| codex/`gpt-6-luna` tokens, mean | 12,996 | 11,654 | 11,604 |
| claude/`sonnet` tokens, mean (recovered) | 159,756 | 156,806 | 157,002 |
| codex/`gpt-6.1-sol` tokens, mean | 30,398 | 19,349 | 16,979 |

### What lp-4 can and cannot show

- **Success is at the ceiling** (24/24 in three batches). #140 can show a loss, but no
  success gain. A learning effect has to show up as a change in first dispatch, time or
  tokens, with success and retries held.
- **Fallback was not exercised** in the baseline; the mechanism was checked during lp-2.
- **Unused arms:** claude `haiku` and both pi models were never dispatched. pi reports
  no tokens.
- **Only a few metrics are steady enough for a go criterion:** Tier2 wall time, and the
  tokens of claude/`sonnet` and codex/`gpt-6.1-sol`. Tier1 and Tier3 wall times, and
  codex/`gpt-6-luna` tokens, swing too much between repetitions. Each tier has only two
  Tier1 or Tier3 tasks, and single outliers dominate: 32.3 s `py-median-bugfix`, 74.8 s
  `py-semver`, 22k `py-median-bugfix` tokens.
- **The codex model name** is not in the kept evidence tail. The model of record is the
  `model_ref` passed as `--model`.
- **Human Adjust/Decline** is 0 by protocol.

## Proposed thresholds (for the coordinator to record)

Same structure and method as lp-3. A margin is the larger of:
- twice the spread between lp-4 repetitions (range of the per-repetition medians or
  means, divided by the lp-4 value);
- each shift between consecutive batches (lp-2 → lp-3, lp-3 → lp-4);

rounded up to a multiple of 5%, with a 5% floor. A wall-time no-go margin is never
below 25%.

| Metric | lp-4 value | Spread ×2 | lp-2→lp-3 | lp-3→lp-4 | Margin | Change vs lp-3 proposal |
|---|---|---|---|---|---|---|
| Tier1 dispatch p50 | 18.5 s | 54% | +12% | −11% | 55% | was 35% |
| Tier2 dispatch p50 | 11.0 s | 4.5% | 0% | −2% | 5% | was 20% |
| Tier3 dispatch p50 | 41.0 s | 82% | −15% | +6% | 85% | was 15% |
| claude/`sonnet` tokens | 157,002 | 0.3% | −2% | +0.1% | 5% (floor) | unchanged |
| codex/`gpt-6.1-sol` tokens | 16,979 | 29% | −36% | −12% | 40% | was excluded |
| codex/`gpt-6-luna` tokens | 11,604 | 149% | −10% | −0.4% | 150% | excluded, as before |

The #140 arm must use:
- the lp-4 task set, the same `meshloop.toml`, protocol and runner (verify and token
  parsing included);
- 3 × 8 runs with a new store per run;
- a recorded Meshloop commit and the accepted priors or hints file.

**Go** (all must hold):

1. Success ≥ 23/24: at most one failure, with no task failing in 2 or more
   repetitions. *(Unchanged.)*
2. Retries: at most 2 runs need `resume --retry`, and no run exhausts the retry
   budget. *(Unchanged.)*
3. At least one learning effect, attributable to the accepted priors or hints file:
   - **First-dispatch change.** For at least one tier, the first dispatch differs from
     the baseline (Tier1 codex/`gpt-6-luna`, Tier2 claude/`sonnet`) in at least 2 of 3
     repetitions for the same tasks, and every affected run succeeds first time.
     *(Unchanged.)*
   - **Faster.** Tier2 dispatch p50 ≤ 10.5 s (−5%), with no other tier past its no-go.
     *(Was: any tier, with Tier1 ≤ 13.5 s, Tier2 ≤ 9.0 s, Tier3 ≤ 33.0 s.)* Tier1 (≤ 8.3
     s at −55%) and Tier3 (≤ 6.2 s at −85%) come out of the method as practically
     unreachable, so only Tier2 is proposed as a criterion.
   - **Cheaper.** claude/`sonnet` mean tokens per attempt ≤ 149,152 (−5%), recovered
     counts in both arms *(was ≤ 148,966)*; or codex/`gpt-6.1-sol` ≤ 10,188 (−40%)
     *(new; was excluded)*. codex/`gpt-6-luna` stays excluded.
4. No no-go condition holds.

**No-go** (any one):

- success ≤ 22/24, or any task failing in 2 or more repetitions *(unchanged)*;
- more than 3 runs need a retry, or any run exhausts the retry budget *(unchanged)*;
- dispatch wall p50 above Tier1 28.7 s (+55%; was 28.0 s), Tier2 13.8 s (+25%; was
  14.1 s) or Tier3 75.9 s (+85%; was 48.5 s); or overall p95 above 61.3 s (+25% of
  49.0 s; was 52.2 s);
- claude/`sonnet` mean tokens above 196,253 (+25%; was 196,008), or a codex model's
  mean more than doubling: above 23,209 for `gpt-6-luna` (was 23,308) or 33,958 for
  `gpt-6.1-sol` (was 38,698);
- a first dispatch to a candidate outside the tier's configured, dispatchable set
  (`tier_candidates`), or **a prior that adds or removes a candidate**, or a tier hint
  that lowers a tier (#139 contract) *(unchanged)*;
- any timeout, an aborted task, or a quota-driven early stop that leaves fewer than 3
  full repetitions *(unchanged)*.

**Inconclusive** (neither): treat as no-go, per ADR 0038 ("If #140 shows no gain,
learning stops").

Notes for the decision:

- The wide Tier1 and Tier3 margins follow from only two tasks per tier and three
  repetitions. With them, the Tier3 no-go (75.9 s) is loose, since one slow run could
  hide a real regression. If tighter time gates are wanted for those tiers, the options
  are more repetitions or more tasks per tier; either needs a new task-set version or a
  new baseline.
- A tier hint can only raise a tier. On lp-4 that moves a task to a slower and costlier
  arm (Tier3 dispatch p50 is about 3.7 times Tier2's). With no failures to prevent, a
  hint mainly risks the Tier2 time and token gates.
- At Tier3 only codex/`gpt-6.1-sol` fits, so priors and fallback can change nothing
  there.
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
