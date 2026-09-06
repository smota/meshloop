# 0017 Session control plane: supervisor origin, Herdr live transport, meshloop: namespace

- Status: Proposed
- Implementation: implemented — `meshloop:orchestrate` pins attempt evidence, writes a review pack, and on `--allow-live-harness` plus `--origin-session` launches two `meshloop:reviewer` Herdr panes (never the origin pane), waits, parses COMPLETE/INCOMPLETE, synthesizes, and records `ModelReviewEvidence`. Human `meshloop:accept` remains required. CI never splits panes.
- Date: 2026-09-06
- Author/executor: Grok, under human product direction
- Reviewer: pending
- Approval evidence: user decisions 2026-09-06 (supervisor-only; dual planning; skill+MCP; Herdr live transport; prefixed ids)
- Supersedes: none
- Superseded by: none

## Context
Release 1 (ADR 0016) is a CLI that parents fixture subprocesses. Operators work *inside* Codex/Claude/Grok/Pi and must not collide with those products' `plan` / `reviewer` / `orchestrate` vocabulary. [pi-herdr-agents](https://github.com/giuseppecrj/pi-herdr-agents) shows useful *contracts* (bundled visible definitions, pinned-evidence review) but is a Pi-only extension — Meshloop must not become that.

## Decision
1. Origin session is **supervisor-only** (`meshloop:origin`).
2. Origin writes conversation intent; **`meshloop:planner`** (Herdr child when live) produces the executable multi-model graph; the engine validates, routes, verifies, integrates.
3. Wrappers are a **skill pack + local MCP**; slash commands are deterministic (`/meshloop:plan`). No saga in SKILL.md (ML-014).
4. Live workers are **Herdr 0.8 panes** (`pane split --no-focus`, `pane run <id> …`, `agent start --kind --pane`). Fixture subprocess remains CI. Doctor does not split panes.
5. Every skill, command, MCP tool, and role id is prefixed **`meshloop:`**. MCP uses `meshloop_<leaf>` because MCP names cannot contain `:`. Unprefixed harness words are rejected.

Learned from pi-herdr-agents (not copied): listable roles with source; reviewers are leaves; parent pins evidence; two distinct models; INCOMPLETE is real; launch matrix `name | role | model | worktree`; bash is not read-only; do not steal focus.

## Consequences
`meshloop reviewer` is an error. `meshloop meshloop:plan` and `meshloop plan` both work (binary already namespaces). Live fan-out of reviewers is assignment-matrix-first; auto pane split against the operator's current session is forbidden without explicit `--allow-live-harness` and is not performed by `doctor` or tests.

## Verification
`cargo run -p xtask -- check`, `cargo run -p xtask -- smoke`. CLI tests: unprefixed rejection, roles JSON prefix, MCP tool names, doctor JSON, R1 fixture loop still green.
