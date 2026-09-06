# Implementation status — 2026-09-05 autonomous build

Executor: Claude (Sonnet 5), authorized by the user to implement autonomously without
per-step confirmation. This records what was actually built and verified, not a claim that
Meshloop is feature-complete or that any ADR was formally accepted (ADR `Status` fields are
untouched — the user authorized building, not a line-by-line content sign-off of each
decision; ADR `Implementation` status, which the ADR README defines as independent of
`Status`, is updated below with evidence).

## What exists and is verified

Ran on native Windows only (`rustc 1.98.0`, MSVC linker — confirmed working, unlike at
initialization). `cargo run -p xtask -- check` passes: `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` all
green, 74 tests, 0 failures.

- **meshloop-domain** (ADR 0002, implemented): `task_graph` (cycle/dangling-dependency
  validation, topological order), `state` (the full execution-lifecycle.md transition
  table as a pure function, table-driven-tested against every documented and undocumented
  `(state, event)` pair), `evidence` (the three ADR 0007 evidence kinds and the
  tier-to-required-evidence gate), `policy` (tier-fit, coupling), `capability`
  (HarnessProfile, the ADR 0003 error taxonomy, and a real Closed/Open/HalfOpen circuit
  breaker for quota cooldown). 27 tests.
- **meshloop-engine** (ADR 0003/0005/0009, implemented against fakes): `ports` (all four
  port traits plus canonical in-memory fakes), `agent` (AgentSpec construction, the
  four-slot prompt template, and a passing test that unrelated sibling-task context never
  leaks into a rendered prompt), `router` (QACR as literally described in runtime-design.md
  §5 — filter by configured+capable+available, score via pluggable `RoutingSignal`s,
  tie-break by coupling — with tests proving no unconfigured candidate can ever be
  selected, Tier 3 never resolves to a non-top-tier model, all-candidates-cooled-down
  resolves to an empty result rather than a panic, and load prefers the harness with more
  headroom), `planner` (decomposition-as-agent-dispatch, structural validation, a
  placeholder tier-assignment heuristic explicitly flagged as unvalidated), `orchestrator`
  (the QACR step-5 fallback loop, tested to fall back visibly on `CapacityExhausted` and
  resolve to `Blocked` rather than crash when every candidate is exhausted), `recovery`
  (event-log replay/reconciliation, tested for determinism and for reconciling an
  unconfirmed `Running` attempt to `Blocked`). 24 tests.
- **meshloop-adapters** (ADR 0003/0005/0007, implemented and really exercised): `harness`
  — `probe()` is real and was run against the actually-installed `herdr`, `claude`, `pi`,
  `grok` binaries on this machine during development (version strings genuinely read, not
  assumed); `invoke`/`cancel`/`collect` are real subprocess code, contract-tested against a
  purpose-built fixture binary (`fixture_harness`), never against a real subscription. `git`
  — real `git worktree add`/`remove`, tested against disposable temp repositories, never the
  Meshloop repository itself. `store` — a real SQLite-backed `EvidenceStore`/
  `RoutingFeedbackStore` (bundled `rusqlite`, which did compile cleanly here), with a
  schema-version guard tested to refuse a newer on-disk version rather than guess. `herdr`
  — real, structured (non-shell-interpolated) argument construction for `herdr pane
  run`/`close`/`session list`, unit-tested; **not** exercised against a live `herdr`
  session/server in this pass (see "Not done" below). 8 + 6 unit/contract tests.
- **meshloop-cli** (ADR 0001, implemented): `plan` (loads config, probes configured
  harnesses, routes the decomposition dispatch via QACR, dispatches it, validates the
  result structurally, assigns tiers, writes a reviewable plan file) and `run` (refuses to
  execute without `--accept-plan`, then for each node in dependency order: routes via QACR,
  creates a real disposable Git worktree, dispatches with fallback, records
  `DeterministicEvidence` to SQLite, reports Tier 1/2 as verified vs. Tier 3 as requiring
  human acceptance, removes the worktree). An end-to-end integration test drives the real
  compiled binary through `plan` → `run` against the fixture harness, including the
  `--accept-plan` gate. 5 + 3 tests, plus the pre-existing scaffold acceptance test updated
  to match (it previously asserted `run` was unimplemented, which is no longer true).

## Not done — do not treat these as implemented

- **No real dispatch against Claude Code, Codex, Pi, Grok, or Agy was performed.**
  `probe()` genuinely ran against them; `invoke` (actually assigning them work) was
  deliberately never triggered autonomously — that would consume real subscription quota
  and take real actions on your files without a specific task from you, which is a
  different kind of decision than building infrastructure.
- **No live `herdr` session was spawned, monitored, or torn down.** The CLI-subprocess
  argument construction is real and tested; a full live round-trip against a running herdr
  server is the follow-up ADR 0005's verification section already calls for.
- **No git integration/merge-back (ADR 0006/0005) happens in `run`.** Every worktree is
  created, dispatched into, and then discarded after evidence is recorded — nothing is
  merged into any branch. This was a deliberate scope cut, stated in `run`'s own console
  output, not an oversight.
- **No WSL2/Linux verification.** Everything above ran on native Windows only. WSL2 Ubuntu
  is installed but has no C build toolchain (`gcc` not found), which blocks even a Linux
  build here — this needs `sudo apt-get install build-essential` (or equivalent) inside
  WSL, which was not run: installing system packages via elevated privileges is outside
  what "implement autonomously" was read to authorize (AGENTS.md: no elevated privileges or
  global tool installation without explicit review of that specific action).
- **Tier assignment is a placeholder heuristic** (dependency-count based), explicitly
  flagged in code comments as unvalidated — real tier assignment needs measurement against
  actual task outcomes.
- **Concurrency and retry limits are parsed from config but not enforced.** `run` is
  strictly sequential in this pass; bounded parallelism and retry-on-`ProcessFault` are
  Phase 8 work per implementation-plan.md.
- **No packaging** (Phase 9: release binaries, versioning) was done.

## Uncommitted

All of the above are working-tree changes only — nothing was committed or pushed. Review
via `git status`/`git diff` before deciding whether and how to commit.
