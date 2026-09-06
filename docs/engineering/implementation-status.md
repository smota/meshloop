# Implementation status — 2026-09-06 Release 1 closed loop

Executor: Grok, authorized by the user to implement Release 1 autonomously on this
workstation. This records what was actually built and verified, not a claim that the
five parent runtime ADRs are accepted.

## What exists and is verified

Native Windows (`rustc 1.98.0`). `cargo run -p xtask -- check` is the acceptance bar:
fmt, clippy `-D warnings`, `cargo test --workspace`.

- **meshloop-domain:** task graph (incl. legal `graph_id`, reserved `TaskId(0)`,
  `allowed_paths` / `empty_diff_ok`), lifecycle table, `satisfies` (presence) plus
  `all_deterministic_passed` (exit codes), policy, `QuotaState::from_parts`.
- **meshloop-engine:** QACR (missing headroom no longer scored as full capacity),
  planner (worktree `meshloop-plan.json` then stdout), prompt envelope, `dispatch_with_fallback`
  kept for fake tests, **`RunLoop` saga** drives `transition()` + event log, per-task
  `replay_tasks`, git-diff verification, human accept at every tier, no auto-retry on resume.
- **meshloop-adapters:** `CliHarness` (`{prompt_file}` and `{model_ref}`, mutex poison →
  `ProcessFault`, capacity-exhausted stderr), git worktree from a start-point, commit/merge
  in a named worktree, SQLite schema v2 (events/runs/attempts/quota, WAL on disk),
  `WindowsProcessView`, `CommandCheckRunner`, fixture `--emit-graph` / `--prompt-file` /
  `--noop`.
- **meshloop-cli:** `plan`, `run --accept-plan`, `status`, `resume [--retry]`, `cancel`,
  `inspect`, `accept --as`, `integrate --into --accept-integrate`. Default store
  `.meshloop/state.sqlite`. Honest no-args banner. Live harness spawn refused without
  `--allow-live-harness`.

End-to-end CLI tests drive the compiled binary against `fixture_harness` on disposable
git repos: canned plan, `--accept-plan` gate, empty-diff failure, accept then resume
to Integrated.

## Session control plane (ADR 0017) — 2026-09-06

- Prefixed ids: `meshloop:plan|roles|doctor|orchestrate|mcp`, slash `/meshloop:…`, MCP `meshloop_…`.
- Unprefixed `reviewer`/`planner`/… rejected at CLI.
- Skills under `skills/meshloop-*`. Local MCP: `meshloop mcp`.
- Herdr 0.8 adapter argv matches `pane split|run|close` (not the old `--session pane run`).
- `doctor` probes `herdr status` read-only (no pane split).
- `meshloop:orchestrate` pins the attempt worktree diff into `.meshloop/reviews/`, writes a two-`meshloop:reviewer` matrix, and with `--allow-live-harness --origin-session` launches Herdr panes (`split --no-focus` from a non-origin pane, `agent start --kind`, `prompt --wait`). Synthesis records `ModelReviewEvidence`. Human accept is still required. Tests never split panes.
- `cargo run -p xtask -- smoke` and `-- bundle`.

## Not done — do not treat these as implemented

- **No real dispatch against Claude Code, Codex, Pi, Grok, or Agy.** Probe may work;
  invoke is gated on `--allow-live-harness` and is not an R1 stamp requirement.
- **No live Herdr session.** Adapter argv exists; CLI does not compose it.
- **`integrate --into` is implemented but not the default of `run`.** Operator branch is
  untouched until that command.
- **No WSL2/Linux verification. No packaging.**
- **Tier assignment remains the unvalidated dependency-count heuristic** (does not
  overwrite a human-supplied tier).
- **Concurrency remains 1.** `max_retries` is maximum *attempts* per task.

## Uncommitted

Working-tree implementation pending human review of whether and how to commit.
