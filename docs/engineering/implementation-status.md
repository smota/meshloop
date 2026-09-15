# Implementation status — 2026-09-15

> Maintainers and agents. Operators: [Getting started](../start.md).
> Hub: [docs/README.md](../README.md).

Executor: Antigravity / AI Pair programmer, under human product direction (daemonless direct-CLI dispatch in Git worktrees is the product path; ADR 0022; `meshloop:review-plan` is the plan gate).

## What exists and is verified

Native Windows (`rustc 1.98.0`).

| Gate | Meaning |
|---|---|
| `cargo run -p xtask -- check` | fmt, clippy `-D warnings`, workspace tests (106 tests) |
| `cargo run -p xtask -- live` | Launch gate — verifies doctor `daemonless: true` and git worktree isolation |
| `cargo run -p xtask -- smoke` | Prefixed CLI / doctor / review-plan in `--help` |
| `cargo run -p xtask -- bundle` | `dist/meshloop-session-bundle/` (also `meshloop bundle`) |
| `cargo run -p xtask -- publish-dry` | `cargo package` isolation for all five publishable crates |

- **meshloop-domain:** task graph, lifecycle including `PlanDeclined`,
  `PlanDecision` (Accept / Decline / Adjust), evidence, policy, prefixed ids
  (`meshloop:review-plan` → MCP `meshloop_review_plan`).
- **meshloop-context:** multi-language AST skeleton extraction across **7 languages**
  (Rust, TypeScript/JS, Python, Go, C#, PHP, C++), Tier 1 dynamic 4-level provider resolution
  (CLI flag > Env > TOML > Auto-detect / local Ollama fallback), deterministic prompt cache normalizer.
- **meshloop-engine:** QACR, planner, **`RunLoop`** (`stage_plan` /
  `decide_plan` / live direct or fixture dispatch), context-enriched prompt builder
  (`build_agent_spec_with_context`), git-diff verify, no auto-retry on resume.
- **meshloop-adapters:** `CliHarness` (direct CLI subprocess execution & CI double),
  Git worktree adapter, SQLite **schema v4** (`pane_id` / attempt tracking, `review_note` on
  runs, WAL). Herdr dependency completely purged.
- **meshloop-cli:** `plan`, **`review-plan`**, `run`, `status`, `resume`,
  `cancel`, `inspect`, `accept`, `integrate`, `roles`, `doctor` (reports `daemonless: true`),
  `orchestrate`, `mcp`, `bundle`. Store: `.meshloop/state.sqlite`.
- **ADRs:** 0001–0022 accepted. (ADR 0022: Daemonless Standalone Execution & Context Engineering).

Fixture e2e covers canned plan, `--accept-plan`, empty-diff failure, node
accept+resume, review-plan accept/decline/adjust. Live tests execute direct CLI
processes in isolated worktrees with zero background daemon dependency.

## Session control plane (ADR 0017)

- Prefixed ids; unprefixed `reviewer` / `planner` rejected.
- Skills under `skills/meshloop-*` (including `meshloop-review-plan`).
- Doctor: `herdr status` + `pane current`; JSON `origin_session`.
- Orchestrate: pin pack, two reviewer panes from a non-origin pane, synthesis.
- Origin flags from argv or `MESHLOOP_ORIGIN_*`.

## Residuals — not product claims

- WSL2 / prebuilt GitHub Release binaries
- Further crates.io versions still maintainer-gated (0.1.0 is uploaded)
- Vendor quota numbers not queried
- Tier assignment = dependency-count heuristic
- Concurrency = 1
- `integrate --into` is explicit, not the default of `run`

## Commit

Live-Herdr flip and `meshloop:review-plan` land after `64430f2`. crates.io
0.1.0 upload is on `main` after `28e467b`. This file is the executor log, not
a public roadmap.
