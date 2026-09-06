# 0017 Session control plane: supervisor origin, Herdr live transport, meshloop: namespace

- Status: Accepted
- Implementation: implemented — live Herdr is default for non-fixture harnesses; `meshloop:orchestrate` pins attempt evidence, writes a review pack, and with `--origin-session` launches two `meshloop:reviewer` Herdr panes (never the origin pane), waits, parses COMPLETE/INCOMPLETE, synthesizes, and records `ModelReviewEvidence`. Human `meshloop:accept` remains required. Fixture tests do not require Herdr.
- Date: 2026-09-06
- Accepted: 2026-09-06
- Author/executor: Grok, under human product direction
- Decision owner: Samuel
- Approval evidence: user decisions 2026-09-06 (supervisor-only; dual planning; skill+MCP; Herdr live transport; prefixed ids; live is product default)
- Supersedes: none
- Superseded by: none

## Context
Operators work *inside* Codex/Claude/Grok/Pi and must not collide with those products'
`plan` / `reviewer` / `orchestrate` vocabulary. [pi-herdr-agents](https://github.com/giuseppecrj/pi-herdr-agents)
shows useful *contracts* (bundled visible definitions, pinned-evidence review) but is a
Pi-only extension — Meshloop must not become that.

## Decision
1. Origin session is **supervisor-only** (`meshloop:origin`).
2. Origin writes conversation intent; **`meshloop:planner`** (Herdr child when live)
   produces the executable multi-model graph; the engine validates, routes, verifies,
   integrates.
3. Wrappers are a **skill pack + local MCP**; slash commands are deterministic
   (`/meshloop:plan`, `/meshloop:review-plan`). No saga in SKILL.md (ML-014).
   Skills inject `--origin-harness` / `--origin-session` or `MESHLOOP_ORIGIN_*`.
4. Live workers are **Herdr 0.8 panes** (`pane split --no-focus`, `pane run <id> …`,
   `agent start --kind --pane`). Fixture subprocess is CI only. Doctor does not split
   panes; it reports `origin_session` from `herdr pane current` or env.
5. Every skill, command, MCP tool, and role id is prefixed **`meshloop:`**. MCP uses
   `meshloop_<leaf>` because MCP names cannot contain `:`. Unprefixed harness words are
   rejected.

Learned from pi-herdr-agents (not copied): listable roles with source; reviewers are
leaves; parent pins evidence; two distinct models; INCOMPLETE is real; launch matrix
`name | role | model | worktree`; bash is not read-only; do not steal focus.

## Consequences
`meshloop reviewer` is an error. `meshloop meshloop:plan` and `meshloop plan` both work
(binary already namespaces). Live fan-out of reviewers requires `--origin-session` so the
supervisor pane is never split. Tests **may** split non-origin panes when Herdr is up;
they must pass origin pane id. `xtask live` fails if Herdr is down.

## Verification
`cargo run -p xtask -- check`, `cargo run -p xtask -- smoke`, `cargo run -p xtask -- live`.
CLI tests: unprefixed rejection, roles JSON prefix, MCP tool names, doctor JSON includes
`origin_session`, `review-plan` accept/decline/adjust, fixture loop still green without Herdr.
