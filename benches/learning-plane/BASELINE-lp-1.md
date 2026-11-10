# Learning-plane baseline (task set lp-1) — superseded

> **Superseded by lp-2 ([BASELINE.md](BASELINE.md)), 2026-11-10:** lp-1 could not tell a learning effect apart: all 24 runs went to a single (harness, model), none were retried, and priors bounded at ±0.25 cannot reach mid or top models at Tier1. The lp-1 task set and runner are at commit `a2e944e`; in this record, `taskset.json`, `meshloop.toml` and `verify.py` mean their versions at that commit.

Issue [#144](https://github.com/smota/meshloop/issues/144). The baseline that the
learning-plane comparison in [#140](https://github.com/smota/meshloop/issues/140) is
measured against, recorded before the learning loop ([#139](https://github.com/smota/meshloop/issues/139))
exists. No learning-plane code was involved: none exists at the Meshloop commit used.

The thresholds at the end are **proposals**. The repository owner records the
thresholds that apply on #144.

## Run identity

| Item | Value |
|---|---|
| Date | 2026-11-10, 17:48:06Z start, 742 s for all 24 runs |
| Meshloop | `meshloop 0.3.0`, commit `f0691d9` (merge of #166), binary sha256 `cb8a912a…8115b135` |
| Runner | `xtask bench-learning` at `a2e944e` for the live run. Attempts were re-parsed afterwards with `--aggregate` by the final commit of this branch, which adds the repair-loop fields; no worker was dispatched again |
| Task set | `lp-1` ([`taskset.json`](taskset.json)) |
| Configuration | [`meshloop.toml`](meshloop.toml), sha256 `9cf1307c…f559fbcdf4923` |
| Host | WSL2 Linux (kernel 6.18), Python 3.14.7, cargo 1.98.0 |
| Harness versions (doctor) | codex-cli 0.167.0, Claude Code 2.1.322, pi 1.5.0 |
| Model actually run | `codex` / `gpt-6-luna` for all 24 attempts. This is the `model_ref` that Meshloop substituted into `--model {model_ref}` and recorded on each attempt row. claude and pi were never dispatched |
| Raw results | [`results/baseline-lp-1-2026-11-10.json`](results/baseline-lp-1-2026-11-10.json). Paths are redacted. Worker transcripts are not included; they stay in the ignored `.agent-runs/bench-learning/baseline/` |
| Command | `xtask bench-learning --live --reps 3 --bin <meshloop-f0691d9> --bin-commit f0691d9 --out .agent-runs/bench-learning/baseline --results benches/learning-plane/results/baseline-lp-1-2026-11-10.json` |

Not part of the baseline: two pilot runs of `py-median-bugfix` on codex/`gpt-6-luna`,
both passed, made while building the runner. Single-prompt probes also confirmed that
each configured model reference is accepted by its CLI (`gpt-6-luna`, `gpt-6.1-sol`,
`haiku` → `claude-haiku-5-5`, `sonnet` → `claude-sonnet-5-5`,
`openai-codex/gpt-5.6-luna`, `openai-codex/gpt-6.1-terra`).

Confound: during the run an unrelated codex session (a separate workload on the same
host and ChatGPT account) was active. It competed for CPU and shared the account's quota
window. No quota or rate-limit signal was seen.

## Task set

Each task has a seed repository (`seed/`: a `SPEC.md`, a stub or buggy module and a few
visible tests), hidden tests (`hidden/tests/`) and a reference solution (`reference/`).
[`verify.py`](verify.py) copies the candidate tree, replaces its `tests/` with the
pristine hidden tests and runs the task's `test_command`. A worker therefore cannot pass
by editing tests, and the exit code decides the result. `xtask bench-learning
--validate` checks offline that every seed fails and every reference passes.

| Task | Lang | Difficulty | What it tests |
|---|---|---|---|
| `py-median-bugfix` | Python | easy | fix two bugs (even-length mean, input mutation), iterables, empty input |
| `py-wordfreq` | Python | easy | tokenising with inner apostrophes, `casefold`, tie ordering |
| `py-lru` | Python | medium | LRU cache: capacity validation (`bool` rejected), capacity 0, eviction callback, recency rules |
| `py-intervals` | Python | medium | half-open interval merge (touching intervals merge) and subtraction |
| `py-roman` | Python | medium | strict canonical Roman numeral parsing (rejects `IIII`, `IL`, `IXI`, ...) |
| `rs-brackets` | Rust | medium | bracket matching with character (not byte) positions, skipped string literals with escapes |
| `py-calc` | Python | hard | exact rational expression evaluator: precedence, right-associative `^` with unary sign, strict number syntax, no `eval` |
| `py-semver` | Python | hard | SemVer 2.0.0 parsing and precedence, npm-style ranges with `^`, `~` and the pre-release rule |

All eight are single-node plans with `tier = null`. The worker receives the
`description` from `task.json`, and `AGENTS.md`/`CLAUDE.md` in the seed tell it to
work inline and leave `tests/` unchanged.

**Known defect in lp-1 (`py-calc`).** The hidden check that forbids `eval`, `exec` and
`compile` uses the pattern `\b(eval|exec|compile|__import__)\s*\(`. That pattern also
matches `re.compile(`, which the spec does not forbid. In repetition 2 the worker's
solution failed only on this check (evidence: the assertion reports `match='compile('`).
The repetition 1 failure is genuine: the solution accepted `.5`, which the spec rejects.
The task set is left unchanged here, so the committed results match the committed
tests. The defect affects both arms of #140 equally if lp-1 stays frozen. The
coordinator decides whether to keep lp-1 as is, or to fix the pattern (for example to
`(?<![.\w])(eval|exec|compile|__import__)\s*\(`), version it lp-2 and re-run the
baseline. Counted by the spec's intent, success would be 23/24.

## Configuration and what routing does with it

[`meshloop.toml`](meshloop.toml) selects `["codex", "claude", "pi"]`, with
`max_retries = 3` and `task_timeout_seconds = 420`. grok is excluded (#162, quota) and
agy (#164, not dispatchable). Every flag below was checked against the CLI's `--help`,
a `meshloop doctor` run (all three harnesses compatible and dispatchable) and a probe
of each model.

| Harness | Template | Models (tier) |
|---|---|---|
| codex | `exec --model {model_ref} -` (stdin) | `gpt-6-luna` (lightweight), `gpt-6.1-sol` (top) |
| claude | `--permission-mode acceptEdits --output-format json --model {model_ref} --print {prompt}` | `haiku` (lightweight), `sonnet` (mid) |
| pi | `--model {model_ref} -p {prompt}` | `openai-codex/gpt-5.6-luna` (lightweight), `openai-codex/gpt-6.1-terra` (mid) |

From `router.rs` and `planner.rs` at `f0691d9`:

- A single leaf node with `tier = null` is assigned **Tier1** by the placeholder
  `DefaultTierAssigner`. Recorded `routing_feedback` rows confirm `tier = Tier1`.
- With empty feedback, each candidate scores tier-efficiency + historical mean (0.5) +
  exploration bonus (0.7). That is 1.2 for lightweight, 0.7 for mid and 0.2 for top.
  Ties go to the configured order. **The first dispatch is therefore always
  codex/`gpt-6-luna`.** A retry after a failed check would go to claude/`haiku` and then
  pi/`gpt-5.6-luna`.
- The bounded prior (at most ±0.25 on the historical mean, #139) can reorder the three
  lightweight arms. It cannot lift a mid or top model above a lightweight one at Tier1:
  the best case is a tie at 0.95, and the configured order then keeps the lightweight
  arm first. Only a tier hint accepted at plan review (Tier2) reaches the mid models.
- **Observed: no retry happened in any run.** Both failures (`py-calc`, repetitions 1
  and 2) went through Meshloop's in-attempt repair loop: 2 rounds with an unchanged
  revision, then a stop on `Oscillation`. The node then ended `Failed`
  (`FailedTerminal`) without falling back to another candidate, even though attempts
  were left. As a result, claude and pi were never dispatched, and the baseline holds
  evidence for one arm only. For #139 this means that, under this configuration, a
  routing prior can only demote codex/`gpt-6-luna` toward an arm that has no
  observations. Whether an oscillation stop should allow a fallback attempt is a kernel
  question that is out of scope here.

## Protocol and carried state

- Fixed automated protocol, with no human in the loop: `review-plan --accept --as
  claude-coordinator`, `run`, `accept --task 1 --as claude-coordinator` only when the
  node is `AwaitingReview` (Meshloop's own checks passed), `resume`, `integrate --into
  main --accept-integrate`. Then the hidden tests run on `main`, and their exit code is
  the run's success.
- **Adjust/Decline = 0 by protocol.** No plan is ever adjusted or declined. This is not
  a measurement of human behaviour, so #140 cannot compare human plan adjustments
  against it.
- Each task × repetition uses a new disposable repository and a new Meshloop store.
  `routing_feedback` was empty before every run (`routing_feedback_before = []`,
  `store_existed_before = false` in all 24 records). Nothing Meshloop stores carries
  across tasks or repetitions. Not controlled: vendor-side state (CLI login, user-level
  config and instruction files, vendor caches, account quota windows). Each disposable
  repository was deleted after its run, worktrees included.
- Order: repetition-major (all eight tasks, then the next repetition). Per-run outer
  timeout 1560 s; none was reached.

## Results

Success means the hidden tests exit 0 on `main` after integration. Rates have 95%
Wilson intervals.

| Metric | Baseline |
|---|---|
| Success rate | **91.7%** (22/24), 95% CI [74.2%, 97.7%] |
| First-attempt success | 91.7% (22/24), same CI (no run needed a second attempt) |
| Success by repetition | 87.5% (7/8), 87.5% (7/8), 100% (8/8): range 12.5 pp |
| Retries per run | mean 0.00, median 0, max 0 |
| Attempts per run | 1.00 |
| Repair loop | ran in 2 attempts (both `py-calc` failures), 4 rounds in total, both stopped on `Oscillation` |
| Failed runs without fallback | 2 of 2 |
| `run` wall time | p50 24.3 s, p95 48.8 s (nearest rank) |
| End-to-end wall time (review to final verify) | p50 26.0 s, p95 50.6 s |
| Worker wall time (Meshloop's own) | p50 24 s, p95 47 s |
| Tokens (codex `tokens used`) | reported for 24/24 attempts: mean 15,927, median 14,218 per attempt (min 8,460, max 26,852), 382,236 in total. Per successful run: median 14,218, mean 15,622 |
| Tokens by repetition (mean per attempt) | 16,142, 17,285, 14,352 |
| `run` wall p50 by repetition | 24.3 s, 24.0 s, 23.3 s |
| Adjust / Decline | 0 / 0 by protocol |
| Timeouts, quota hits | 0, 0 |

By task (3 repetitions each):

| Task | Difficulty | Success | `run` wall p50 (s) | Tokens per attempt |
|---|---|---|---|---|
| `py-median-bugfix` | easy | 3/3 | 16.8 | 11,976 / 20,719 / 8,460 |
| `py-wordfreq` | easy | 3/3 | 24.3 | 12,259 / 12,069 / 15,821 |
| `py-lru` | medium | 3/3 | 20.8 | 15,043 / 13,392 / 12,350 |
| `py-intervals` | medium | 3/3 | 23.3 | 22,522 / 12,177 / 9,430 |
| `py-roman` | medium | 3/3 | 29.0 | 26,852 / 15,410 / 22,241 |
| `rs-brackets` | medium | 3/3 | 21.5 | 11,893 / 17,904 / 12,026 |
| `py-calc` | hard | **1/3** | 40.0 | 12,929 / 25,619 / 11,504 |
| `py-semver` | hard | 3/3 | 47.6 | 15,663 / 20,993 / 22,984 |

By (harness, model):

| Harness | model_ref | Attempts | First dispatches | Checks passed | Duration p50 / p95 (s) | Tokens reported |
|---|---|---|---|---|---|---|
| codex | `gpt-6-luna` | 24 | 24 | 22/24 (91.7%, CI [74.2, 97.7]) | 24 / 47 | 24/24, median 14,218 |
| codex | `gpt-6.1-sol` | 0 | 0 | — | — | — |
| claude | `haiku` | 0 | 0 | — | — | — |
| claude | `sonnet` | 0 | 0 | — | — | — |
| pi | `openai-codex/gpt-5.6-luna` | 0 | 0 | — | — | — |
| pi | `openai-codex/gpt-6.1-terra` | 0 | 0 | — | — | — |

### What was not measurable

- **Tokens for claude and pi.** Neither was dispatched. The runner parses claude's
  `--output-format json` `modelUsage` (unit-tested with samples of the real output format, including
  when the 2 KB evidence tail is truncated). pi in text mode prints no usage, so pi
  would record "not reported". Codex token totals use codex's own definition and must
  not be summed with or compared against another harness's numbers.
- **The canonical model behind codex.** Codex prints its model only at the start of its
  transcript, which the 2 KB evidence tail drops. The model of record is the
  `model_ref` passed as `--model`.
- **Retries and fallback.** There were 0 retries. The fallback ladder was never
  exercised, because failures ended at the repair loop's oscillation stop.
- **Human Adjust/Decline.** Fixed at 0 by protocol.
- **Statistical power.** n = 24 with 22 successes. Even 24/24 has a Wilson interval of
  [86.2%, 100%], which overlaps the baseline interval, so no success-rate gain on lp-1
  can show non-overlapping intervals.

## Proposed thresholds (for the coordinator to record)

These are based on the observed spread across repetitions: success varied by one task per
repetition (12.5 pp), mean tokens per attempt by about ±10% between repetitions, and the
`run` wall p50 by about 1 s (5%) between repetitions. The #140 arm must use the same task
set, configuration, protocol, runner and repetition count (3 × 8), and must record its
Meshloop commit.

**Go** (all must hold):

1. No success regression: success ≥ 22/24.
2. At least one of:
   - tokens per successful run (median, codex-reported) at least 20% lower (≤ 11,374),
     which is about twice the observed spread between repetitions;
   - `run` wall p50 at least 15% lower (≤ 20.6 s), about three times the observed
     spread;
   - `py-calc` succeeds 3/3, the only task below 3/3. This counts only if lp-1's
     `py-calc` pattern defect is resolved the same way for both arms;
   - a measured routing change: at least one first dispatch moves away from
     codex/`gpt-6-luna` because of an accepted prior or tier hint, with success not
     lower on the tasks where that happens.
3. No no-go condition holds.

**No-go** (any one):

- success ≤ 20/24 (at least 2 fewer than baseline, more than one repetition's
  worth of spread);
- a task that passed 3/3 in the baseline fails in 2 or more repetitions;
- `run` wall p95 up by more than 25% (> 61.0 s), or `run` wall p50 up by more than 15%
  (> 27.9 s);
- tokens per successful run (median) up by more than 20% (> 17,061);
- any run timeout, any quota-driven early stop, or any candidate added or removed by a
  prior (the #139 contract);
- mean retries above 0.25 without a success gain.

**Inconclusive** (neither go nor no-go): treat as no-go. ADR 0038 says learning stops if
#140 shows no gain.

Before #139 starts, the coordinator should also decide two design points this baseline
exposes. First, whether #140 needs a Tier2 arm or an exploration arm. As configured, the
baseline never reaches claude or pi, so routing priors have only one observed arm to
learn from. Second, whether lp-1 is frozen with the `py-calc` defect or re-baselined as
lp-2.

## Reproduce

```bash
cargo run -p xtask -- bench-learning --validate            # offline: seeds fail, references pass
cargo run -p xtask -- bench-learning --live --reps 3 \
  --bin <meshloop binary> --bin-commit <sha> \
  --out .agent-runs/bench-learning/<name> \
  --results benches/learning-plane/results/<name>.json     # live; uses harness quota
cargo run -p xtask -- bench-learning --aggregate .agent-runs/bench-learning/<name>/results.json \
  --results benches/learning-plane/results/<name>.json     # re-parse kept evidence, recompute summary
```
