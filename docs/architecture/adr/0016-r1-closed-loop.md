# 0016 Release 1: closed-loop orchestration with live Herdr workers

- Status: Accepted
- Implementation: implemented — native Windows; live Herdr workers are the product path; fixture subprocess is the CI double
- Date: 2026-09-06
- Accepted: 2026-09-06
- Author/executor: Grok, under human product direction
- Decision owner: Samuel
- Approval evidence: user approved the 2026-09-06 launch plan; the fixture-as-R1 stamp and `--allow-live-harness` product gate are withdrawn
- Supersedes: none
- Superseded by: none

## Context and constraints
The five runtime ADRs (0001, 0003, 0005, 0007, 0009) describe v1. An earlier draft of this
ADR stamped Release 1 as a fixture-backed CLI with live dispatch opt-in. That stamp was
incorrect: Meshloop's concept is operating *from* an authenticated agent session and
farming live workers through Herdr.

## Alternatives
A: Fixture-only forever (CI as product). B: Fan out five harnesses before verify+integrate.
C: This subset — one writer, kept worktrees, git-diff verify, human accept, **live Herdr
workers as default** for any non-`fixture` harness.

## Decision
R1 ships a foreground engine on native Windows that drives one accepted task graph
sequentially:

- Origin session is supervisor-only (`meshloop:origin`). Never split that pane.
- Dual planning: origin writes conversation intent; `meshloop:planner` produces the
  executable graph (live pane when not fixture); the engine validates, routes, verifies,
  integrates.
- Plan file (human-authored first-class; worktree `meshloop-plan.json` for agents).
  Human plan gate: `meshloop:review-plan` `--accept` / `--decline` / `--adjust`,
  or one-step `run --accept-plan`.
- QACR filter without invented quota numbers.
- One candidate = one attempt = one kept worktree. Live dispatch: Herdr `pane split
  --no-focus` from a **non-origin** pane, `agent start --kind --pane`, `prompt --wait`.
- Git-diff verification (`all_deterministic_passed` ∧ `satisfies(DeterministicOnly)`).
- WAL SQLite event log with a **per-task** fold; attempts persist `pane_id`;
  runs persist `review_note` (schema v4).
- `status` / `review-plan` / `resume` / `cancel` / `inspect` / `accept` /
  `integrate --into`.
- Dead `Running` → `Failed`; `resume` does not auto-retry.
- Integrate merges happen in a dedicated integrate worktree; `repo_root` is only touched by
  `meshloop integrate --into --accept-integrate`.
- **Harness name `fixture` → subprocess (CI).** Any other configured harness → live Herdr.
  `--fixture-only` forces the double. `--allow-live-harness` is withdrawn as a product gate.
- Completing a live dispatch against at least one real kind on this Windows host **is** the
  R1 stamp (`xtask live`). `xtask check` may skip live splits if Herdr is down.
- Concurrency = 1. Packaging and WSL2 are deferred.

This ADR does not replace 0001/0003/0005/0007/0009; they are accepted for R1 with named
residuals.

## Consequences
Operators in Claude/Codex/Grok/Pi/Agy get a usable loop: doctor → plan →
review-plan (Accept / Decline / Adjust) → live worker pane → accept node →
orchestrate reviewers → integrate. Fixture remains honest CI, not the product
demo. Docs that still say “live is not R1” are wrong.

## Verification
`cargo run -p xtask -- check` (fmt, clippy, fixture tests). `cargo run -p xtask -- live`
when Herdr is running (launch gate). Session QA from an origin pane: doctor JSON shows
origin session; plan; visible worker pane; orchestrate never splits origin.
