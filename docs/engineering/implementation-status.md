# Implementation status — 2026-09-06

> Maintainers and agents. Operators: [Getting started](../start.md).
> Hub: [docs/README.md](../README.md).

Executor: Grok, under human product direction (live Herdr is the product path;
fixture is the CI double; `meshloop:review-plan` is the plan gate).

## What exists and is verified

Native Windows (`rustc 1.98.0`).

| Gate | Meaning |
|---|---|
| `cargo run -p xtask -- check` | fmt, clippy `-D warnings`, workspace tests (live Herdr **skips** if down) |
| `cargo run -p xtask -- live` | Launch gate — **fails** if Herdr is down |
| `cargo run -p xtask -- smoke` | Prefixed CLI / doctor / review-plan in `--help` |
| `cargo run -p xtask -- bundle` | `dist/meshloop-session-bundle/` (also `meshloop bundle`) |
| `cargo run -p xtask -- publish-dry` | `cargo package` isolation for the four publishable crates |

- **meshloop-domain:** task graph, lifecycle including `PlanDeclined`,
  `PlanDecision` (Accept / Decline / Adjust), evidence, policy, prefixed ids
  (`meshloop:review-plan` → MCP `meshloop_review_plan`).
- **meshloop-engine:** QACR, planner, **`RunLoop`** (`stage_plan` /
  `decide_plan` / live or fixture dispatch), git-diff verify, no auto-retry on
  resume. Default is live; `fixture_only` refuses non-fixture.
- **meshloop-adapters:** `CliHarness` (CI), `HerdrWorkerHarness` (live panes),
  Git worktrees, SQLite **schema v4** (`pane_id` on attempts, `review_note` on
  runs, WAL).
- **meshloop-cli:** `plan`, **`review-plan`**, `run`, `status`, `resume`,
  `cancel`, `inspect`, `accept`, `integrate`, `roles`, `doctor` (origin pane),
  `orchestrate`, `mcp`, `bundle`. Store: `.meshloop/state.sqlite`.

Fixture e2e covers canned plan, `--accept-plan`, empty-diff failure, node
accept+resume, review-plan accept/decline/adjust. Live tests may split
**non-origin** panes; they never split the supervisor pane.

## Session control plane (ADR 0017)

- Prefixed ids; unprefixed `reviewer` / `planner` rejected.
- Skills under `skills/meshloop-*` (including `meshloop-review-plan`).
- Doctor: `herdr status` + `pane current`; JSON `origin_session`.
- Orchestrate: pin pack, two reviewer panes from a non-origin pane, synthesis.
- Origin flags from argv or `MESHLOOP_ORIGIN_*`.

## Residuals — not product claims

- WSL2 / prebuilt GitHub Release binaries
- crates.io enablement (ADR 0018); no registry upload yet
- Vendor quota numbers not queried
- Tier assignment = dependency-count heuristic
- Concurrency = 1
- `integrate --into` is explicit, not the default of `run`

## Commit

Live-Herdr flip and `meshloop:review-plan` land on the working tree after
`64430f2`. This file is the executor log, not a public roadmap.
