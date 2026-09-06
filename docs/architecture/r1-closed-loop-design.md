# Meshloop Release 1 — Closed-Loop Single-Writer Orchestration

> **Do not implement from this file.** Historical design input only.
> **Current product:** [Getting started](../start.md) · [ADR 0016](adr/0016-r1-closed-loop.md)
> · [implementation status](../engineering/implementation-status.md).
>
> The body below still describes a fixture-first lab and `--allow-live-harness`.
> Those claims are **superseded**: live Herdr is the product path; fixture is CI;
> `meshloop:review-plan` is the plan gate. Audience: archaeology, not operators.

| Field | Value |
|---|---|
| **Document title** | Meshloop Release 1: closed-loop single-writer orchestration |
| **Author** | Meshloop design (AI-authored under human product direction) |
| **Date** | 2026-09-06 |
| **Status** | Draft (revised after design review) |
| **Base revision** | `c610ef3480f8b9a378a53af066ab6ca40919b9c9` (`main`) |
| **Audience** | Human decision owner; implementers of `meshloop-domain`, `meshloop-engine`, `meshloop-adapters`, `meshloop-cli` |
| **Does not authorize** | Commit, push, merge, ADR acceptance, live harness invoke, packaging, or privilege elevation |

---

## Overview

Meshloop today is a layered Rust workspace whose domain, engine, and adapters are real, but whose product path is not. `meshloop plan` / `meshloop run --accept-plan` work against `fixture_harness` in a lab: they create a Git worktree, spawn a subprocess, treat a zero exit code as "verified", record that claim to SQLite only if `--db` is passed, then `--force` delete the worktree. `state::transition()`, `evidence::satisfies()`, `recovery::reconcile()`, predecessor diffs, `model_ref` argv substitution, and Herdr composition are not on that path. The README and no-args banner still say orchestration is unimplemented. That is not a shippable product, and claiming the unused pieces are capabilities would violate `AGENTS.md`.

Release 1 is the smallest architecture a human operator on native Windows (Tier A, this host) can use to run **one** engineering objective through Meshloop with honest status, durable state, isolated work, verification that inspects the worktree, crash recovery that reconciles before resuming, and no silent overclaim. R1 closes **one real loop** — plan (human-authored or agent-file), accept the plan, dispatch sequentially (one QACR candidate = one attempt), keep the worktree, verify the diff, persist the event log, recover without auto-retry, require human `meshloop accept` at every tier before `Accepted`, then merge onto an isolated integrate worktree — rather than spreading work across five harnesses without shared artifacts. Spreading without verify-and-integrate is worse than one writer.

---

## Background & Motivation

### Product intent (unchanged)

From `docs/product/brief.md` and `AGENTS.md`: Meshloop takes one engineering objective, plans it into a task graph, routes work across configured subscription CLI clients via Quota-Aware Capability Routing, verifies what comes back, and recovers cleanly. Humans direct product intent and accept consequential architecture; AI authors code. Domain stays free of process/storage/harness dependencies; engine consumes ports; adapters implement; CLI composes. Selected harnesses: Codex, Claude Code, Pi, Grok, Agy. The engine uses existing authenticated CLI sessions; it does not store credentials. Never substitute Agy with Antigravity by name.

Requirements ML-001 through ML-014 in `docs/product/requirements.md` remain the acceptance targets. R1 does not satisfy all of them. It must satisfy a named subset honestly, and name the rest as deferred.

### What actually works (verified 2026-09-06, `c610ef3`)

Layered crates exist and tests pass on native Windows (`rustc 1.98.0`, MSVC). This is a fixture-backed lab, not a product:

| Layer | What is real |
|---|---|
| `meshloop-domain` | Task graph validation (`TaskGraph::validate`, cycle/dangling/duplicate), 10-state lifecycle as pure `state::transition()`, three evidence kinds + `evidence::satisfies()` (presence-only; does not read `exit_code`), `policy::tier_fits` / `coupling_penalty`, `QuotaState` circuit breaker (fields private; no persist constructor) |
| `meshloop-engine` | QACR `Router::select` against fakes, `planner::decompose` (stdout JSON → `TaskGraph`), prompt envelope with sibling isolation (`agent::dependency_context`), `orchestrator::dispatch_with_fallback` (sets `TaskState` without `transition()`, records routing success on any `collect` Ok), `recovery::replay` / `reconcile` (unwired from CLI; `replay` last-write-wins, does not fold `transition()`) |
| `meshloop-adapters` | `CliHarness` subprocess (probe/invoke/cancel/collect), `GitWorktreeAdapter` add/remove, `SqliteStore` evidence + routing_feedback (no WAL, swallows read errors), `HerdrCliAdapter` argv construction (not composed) |
| `meshloop-cli` | `plan` / `run --accept-plan` against `fixture_harness` only |

### What is missing or lying (must be fixed or explicitly cut)

Cited from the current tree, not from vision docs:

1. **Banner lie.** `crates/meshloop-cli/src/main.rs:28` prints "orchestration is not implemented"; `README.md` Status says the same. `plan`/`run` exist.
2. **Real harness invoke never done.** `probe()` has been run against installed `herdr`/`claude`/`pi`/`grok`; `invoke` has not. `docs/engineering/implementation-status.md` records this.
3. **Herdr not composed.** `HerdrCliAdapter` is never constructed in `compose.rs`.
4. **Worktrees discarded.** `cmd_run` calls `git.remove_worktree` (`git.rs:44` uses `worktree remove --force`). No merge. Dependents only see predecessor *descriptions* via `agent::dependency_context`, not diffs.
5. **False verification.** CLI reports "verified" from harness exit code (`report.rs:36-37`, `main.rs:302` `tool: "harness-exit-code"`). `evidence::satisfies()` and `state::transition()` are never called from CLI. `CandidateRef.revision` is the literal `"worktree-uncommitted"` (`main.rs:295`). `CliHarness::collect` sets `worktree_changed: status.success()` (`harness.rs:157`) without inspecting git. `satisfies()` itself only tests *presence* of a `Deterministic(_)` row — a failed `exit_code` would still pass.
6. **QACR production path is hollow.** `cmd_plan`/`cmd_run` pass empty `quotas` and `headroom` HashMaps (`main.rs:96-97`, `218-219`). `CouplingPenalty(0)` always (`main.rs:238`). `LoadBalanceSignal` treats missing headroom as `1.0` (`router.rs:74`) — unknown is scored as "full capacity". `dispatch_with_fallback` records `record_outcome(..., true)` on any `Ok(outcome)` (`orchestrator.rs:60-61`).
7. **`model_ref` never reaches argv.** `CliHarness::invoke` substitutes only `{prompt_file}` (`harness.rs:96-101`).
8. **Recovery unwired; default store is volatile.** `recovery.rs` is unused by CLI. `compose.rs:60-63` opens in-memory SQLite unless `--db` is passed. Store has `evidence` and `routing_feedback` tables; **no `events` table**. Default rusqlite connection has no WAL.
9. **Fixture `--emit-graph` ignores the prompt** (`fixture_harness.rs:14-18` canned JSON). Real CLIs will not emit a `TaskGraph` on stdout. `planner::decompose` parses `outcome.output_redacted` as JSON (`planner.rs:28`).
10. **Concurrency/retries parsed, unused.** `config.rs:17-18` documents this; `Limits` is `#[allow(dead_code)]`.
11. **Example config is not Windows-safe.** `config/meshloop.example.toml:14` uses `target/debug/fixture_harness` without `EXE_SUFFIX`.
12. **No `status` / `resume` / `cancel` / `inspect`.**
13. **Runtime ADRs 0001/0003/0005/0007/0009 are still Proposed.** Implementation does not equal acceptance (`docs/architecture/adr/README.md`, ADR 0012).
14. **`implementation-status.md` still says "Uncommitted"**; `c610ef3` is on `main`.
15. **Attempt identity is wrong.** `AttemptId(id.0)` (`main.rs:269`) reuses the task id, so retries cannot mint a new attempt as ADR 0005 requires. `dispatch_with_fallback` reuses the same attempt/worktree for every QACR candidate.
16. **CLI owns the saga.** `cmd_run` inlines routing, worktree lifecycle, dispatch, evidence, and cleanup — violating `docs/architecture/boundaries.md` ("CLI must not own orchestration business logic"). `cmd_run` uses `topological_order()` and still dispatches dependents after a predecessor failure.
17. **Poisoned mutex panics** on the production invoke path (`harness.rs:117, 126, 139` `.expect("harness registry mutex")`).
18. **Store swallows read errors.** `SqliteStore::evidence_for` returns `vec![]` on prepare/query failure; `record_outcome` uses `let _ = self.conn.execute(...)`. `RoutingFeedbackStore::record_outcome` returns `()`.

A three-expert review (harness engineer, applied-AI, operator) concluded R1 should close one real path rather than fan out. This document adopts that conclusion as product direction for R1.

### Why now

The lab proves the crate boundaries and the pure domain/engine algorithms. It does not prove Meshloop can survive a crash, inspect a diff, or tell an operator the truth. Those are the product. Everything else (Herdr panes, five-harness parallelism, packaging, WSL2 parity) is deferred until this loop is closed.

---

## Goals & Non-Goals

### Goals (R1 ships these)

1. **Honest operator surface** on native Windows: help, version, no-args banner, README, and `implementation-status.md` match the binary.
2. **Durable run state** by default: on-disk SQLite (WAL) under `.meshloop/`, including an event log that recovery can replay, plus a snapshot of the accepted plan.
3. **Lifecycle actually driven:** every task-state change goes through `meshloop_domain::state::transition` and is appended to the event log *before* side effects that depend on the new state. `replay` errors on an illegal `(from, event, to)` triple.
4. **Worktrees kept** under a stable path, one per attempt, not force-removed on success. The integrate ref lives in its **own** worktree; `repo_root` is never checked out except by explicit `meshloop integrate --into`.
5. **Verification inspects git:** `DeterministicEvidence` is produced by git diff (and an optional configured check command) bound to a real revision hash. A node reaches `AwaitingReview` only if **every** deterministic row has `exit_code == 0` **and** `satisfies(DeterministicOnly)` is true. Human acceptance is a later command, never part of this gate. Harness exit code is never sufficient for "verified". Failed evidence is still recorded.
6. **Dependent tasks see predecessor commits**, not only descriptions. Ready-set is `TaskGraph::ready_nodes(integrated)`, never `topological_order()` for dispatch.
7. **Crash recovery from the CLI:** `status` / `resume` / `cancel`. A persisted `Running` attempt with no collectable process is failed via `HarnessCrashedOrTimeout` → `Failed`. **`resume` does not auto-retry.** The operator must `resume --retry` to emit `RetryAuthorized`. `recovery::reconcile` remains a *view annotation*, never a second source of truth.
8. **Retries and timeout enforced.** Each QACR candidate is its own attempt with a new `AttemptId` and worktree. Config key `max_retries` is **maximum attempts per task** (first dispatch counts). `max_concurrent_workers` is clamped to 1.
9. **QACR used honestly:** configured+probed+dispatchable filter, cooldown filter when exhaustion is observed, visible fallback. Missing headroom does not invent capacity. Routing success is recorded only after `DeterministicChecksPassed`. No claim that subscription quota is queried.
10. **Plan gate persisted:** `--accept-plan` records `PlanState::PlanAccepted` and snapshots the graph; nodes cannot reach `Ready` otherwise.
11. **Human accept at every tier.** T1/T2/T3 all stop at `AwaitingReview` until `meshloop accept --task --as`. No synthetic `HumanAcceptanceEvidence`.
12. **Live-harness refusal is required.** Non-fixture config without `--allow-live-harness` must refuse to spawn. Completing a live Claude/Codex task is **not** required to stamp R1.

### Non-goals (explicitly deferred past R1)

| Deferred | Why |
|---|---|
| Herdr composition / live pane lifecycle | Adapter argv is real; live session is unproven. Worktrees, not panes, are the isolation boundary (ADR 0005). Direct `CliHarness` invoke is sufficient for R1. |
| Multi-harness parallelism / `max_concurrent_workers > 1` | Spreading without verify+integrate is the failure mode R1 is designed to avoid. |
| Pi / Grok / Agy invoke | Probe ≠ invoke. Agy smoke-runner and Grok discovery remain AFD gaps (`docs/engineering/harnesses.md`). |
| Completing a live Claude/Codex objective | Gated path is designed; CI truth is the fixture. |
| WSL2 / Linux-under-WSL2 acceptance | No C toolchain in the WSL distribution here; installing it requires privilege elevation not authorized. R1 claims **native Windows only**. |
| Packaging / release binaries / installers (Phase 9) | Not required to call a local `cargo build --release` slice "Release 1". |
| Optional skills `mesh-loop-planner` / `mesh-loop-executor` | ADR 0001 defers them; CLI remains the sole entry point. |
| Daemon / service mode | v1 is foreground-for-one-graph; R1 even more so. |
| Re-planning / live graph edits | ADR 0009: rejected or irrecoverable plans restart from scratch. |
| Model-review as a required gate | Advisory only (ADR 0007). R1 does not dispatch a reviewer agent. |
| Auto-merge to the operator's current branch / publication | ML-009: local success ≠ review acceptance ≠ integration ≠ publication. |
| Credential storage, cloud service, dollar-cost budgets | Out of product scope. |
| Claiming Tier A complete, five-harness readiness, or ADR acceptance by virtue of this draft | Human-owned. |
| OS sandbox / catching writes outside the worktree | `allowed_paths` is a post-hoc git-diff check only. |

### Requirements coverage

| ID | R1 stance |
|---|---|
| ML-001 | **Ship.** `TaskGraph::validate` already rejects cycles/dangling/empty/duplicates; CLI already re-validates on `run`. `graph_id` is sanitized at load. |
| ML-002 | **Ship, narrowed.** Probe + configured set only. No hardcoded model flags. Compatibility allow-lists stay "Compatible if `--version` exits 0" until a real allow-list is measured. |
| ML-003 | **Ship, narrowed.** Timeout already applied at `collect`. R1 **enforces** `max_retries` as maximum attempts per task (first dispatch counts), **forces** concurrency = 1, and requires `resume --retry` after a crash. Cancellation via `meshloop cancel`. |
| ML-004 | **Ship, narrowed.** One worktree per attempt; integrate worktree is the single integration owner for per-node merges. No checkout of `repo_root` except `meshloop integrate --into`. |
| ML-005 | **Ship.** Evidence bound to `git rev-parse HEAD` (or the attempt commit), not `"worktree-uncommitted"`. Failed evidence is retained. |
| ML-006 | **Ship.** `resume` reconciles events vs PID *hints* vs worktree paths; does not auto-retry a crash-failed attempt. |
| ML-007 | **Partial.** Prompt still built only from declared deps. `allowed_paths` is a post-hoc prefix check on `git diff --name-only` paths. Out-of-worktree writes are out of scope for this check. |
| ML-008 | **Ship, narrowed.** No credentials in config. Secret-pattern + 2 KiB cap redaction before persist. Prompt file deleted before `commit_all`. |
| ML-009 | **Ship as reporting.** Status distinguishes dispatched / verified / awaiting-review / accepted / integrated. Publication remains out of scope. |
| ML-010 | **Ship.** Probe failures, empty QACR result, unsupported invoke, live-harness refusal, and empty-diff failures all surface explicitly. |
| ML-011 | **Partial.** Selection from configured+probed+tier-fit. Quota is reactive (observed `CapacityExhausted`), never a queried remaining-quota number. |
| ML-012 | **Ship.** Prompt from declared context; verify the worktree diff directly. |
| ML-013 | **Ship.** `--accept-plan` plus a persisted plan-acceptance event and graph snapshot. Node-level `meshloop accept` is additional R1 friction, not a substitute for the plan gate. |
| ML-014 | **Ship.** CLI is the sole entry point; no skill wrappers. |

---

## Proposed Design

### R1 in one sentence

A foreground `meshloop` process, on native Windows, drives one accepted `TaskGraph` sequentially through the documented state machine, against one configured harness (fixture by default; one live CLI if the operator authorizes it), isolating each attempt in a kept Git worktree, verifying the diff (not the exit code), appending every transition to SQLite, reconciling that log with real processes and worktrees before any resume, and never auto-retrying a crash.

### Architecture (unchanged crate direction; tighter CLI)

```text
CLI (args, compose, report)
  -> Engine (planner, router, agent, orchestrator saga, recovery)
       -> Domain (graph, state, evidence, policy, capability)
  Adapters implement engine ports (harness, git, store, process view, check runner)
  Herdr adapter exists and stays uncomposed in R1
```

This is ADR 0002. The defect is that `cmd_run` currently *is* the saga. R1 moves the saga into `meshloop-engine::orchestrator::RunLoop` and leaves CLI as composition + flags + printing.

```mermaid
flowchart TB
  subgraph cli [meshloop-cli]
    Args[args: plan/run/status/resume/cancel/inspect/accept/integrate]
    Compose[compose.rs]
    Report[report.rs]
  end

  subgraph engine [meshloop-engine]
    Planner[planner::decompose]
    Router[router::Router::select]
    Agent[agent::build_agent_spec]
    Saga[orchestrator::RunLoop]
    Recovery[recovery::replay / reconcile]
    Verify[verify::verify_attempt]
    Ports[ports: HarnessCapabilities, WorkspacePort, RunStore, ProcessView, CheckRunner]
  end

  subgraph domain [meshloop-domain]
    Graph[task_graph]
    State[state::transition]
    Evidence[satisfies + all_deterministic_passed]
    Policy[policy]
    Cap[capability::QuotaState]
  end

  subgraph adapters [meshloop-adapters]
    Harness[CliHarness]
    Git[GitWorktreeAdapter]
    Store[SqliteStore WAL]
    Proc[WindowsProcessView]
    Check[CommandCheckRunner]
    Herdr[HerdrCliAdapter - uncomposed]
  end

  Args --> Compose
  Args --> Saga
  Compose --> Harness
  Compose --> Git
  Compose --> Store
  Compose --> Proc
  Compose --> Check
  Saga --> Planner
  Saga --> Router
  Saga --> Agent
  Saga --> Recovery
  Saga --> Verify
  Saga --> State
  Saga --> Evidence
  Planner --> Graph
  Router --> Policy
  Router --> Cap
  Saga --> Ports
  Harness -.-> Ports
  Git -.-> Ports
  Store -.-> Ports
  Proc -.-> Ports
  Check -.-> Ports
```

### Scope matrix

| Capability | R1 | Notes |
|---|---|---|
| `plan` | Yes | Prefers worktree file `meshloop-plan.json`; stdout JSON is fixture fallback |
| Human-authored `--plan` | Yes | **Primary happy path** — real CLIs are not graph printers |
| `--accept-plan` | Yes | Persisted + graph snapshot, not just a flag check |
| QACR filter (configured, probed, dispatchable, tier-fit, cooldown) | Yes | |
| QACR score (history, load, coupling) | Wired, honest | History from store; load signal omitted unless every remaining candidate has observed headroom |
| Per-candidate dispatch | Yes | One QACR candidate = one attempt = one worktree. `dispatch_with_fallback` does **not** set `TaskState` |
| Worktree isolation | Yes | Kept; unique branch per attempt; integrate worktree separate |
| Verify via git diff + optional check | Yes | `all_deterministic_passed` + `satisfies`; failed rows recorded |
| Recover | Yes | CLI `status`/`resume`/`cancel`; `resume --retry` for RetryAuthorized |
| Per-node integrate | Yes | Merge inside the integrate worktree only |
| Operator `integrate --into` | Yes | Never implied by `run` |
| Human `accept` | Yes | **All tiers** |
| Herdr | Adapter only | Not composed |
| Parallelism | No | Concurrency = 1 |
| Live harness spawn | Refused unless `--allow-live-harness` | Completing a live task is not an R1 stamp requirement |
| Packaging | No | `cargo build --release -p meshloop-cli` locally |

### 1. Decomposition when real CLIs do not print JSON

ADR 0003 says the worktree is the primary output channel; stdout is secondary. ADR 0009 says decomposition is one `AgentSpec` whose output contract is a task graph. The current implementation contradicts both: `planner::decompose` trusts stdout (`planner.rs:28-29`), and `fixture_harness --emit-graph` ignores the prompt.

**R1 collection order for `decompose`:**

1. `RunLoop::plan(objective, scope)` (not `plan(&AgentSpec)`). The loop owns QACR for the decomposition dispatch, builds the planning spec, and uses a dedicated planning worktree (not the operator's dirty checkout).
2. `collect` the process (timeout still applies).
3. **Read `{worktree}/meshloop-plan.json` first.** If present, parse and `TaskGraph::validate`.
4. Else parse stdout JSON (fixture fallback).
5. If neither is a valid graph: `PlanError::Malformed` — never partially schedule.
6. Sanitize `graph_id` to `[A-Za-z0-9._-]+`. Reject otherwise (do not silently rewrite a human-supplied id).
7. Reject any node with `TaskId(0)` — that id is reserved for the planning `AgentSpec` (`agent.rs:97`).
8. `assign_tiers` remains the unvalidated `DefaultTierAssigner` heuristic, still flagged as such. Operator may set `tier` in a human-authored plan; assigned values **do not overwrite non-null tiers**: `if node.tier.is_none() { node.tier = Some(tier); }`. Test: a human plan with `tier: Tier3` survives `assign_tiers`. Assigned in PR 5.

**R1 prompt contract change** (`agent::build_planning_spec`): the output-contract slot becomes "Write exactly one JSON TaskGraph to `meshloop-plan.json` in this worktree (`graph_id` matching `[A-Za-z0-9._-]+`, `nodes[]` with `id` ≥ 1, `description`, `depends_on`, optional `tier`, optional `allowed_paths`, optional `empty_diff_ok`). No other files unless required to produce that graph. Do not print prose."

**Human-authored plans are first-class.** An operator can write `plan.json` and `meshloop run --plan plan.json --accept-plan` without ever calling `plan`. This is the realistic path for a real engineering objective in R1.

**Fixture `--emit-graph`:** writes `meshloop-plan.json` in `cwd` *and* prints JSON. The **node set is constant** (two canned tasks). `graph_id` is `fixture-<first 8 hex of sha256(prompt)>` so two different objectives do not produce identical plan files, but tests **must not** claim the fixture decomposed the objective. `crates/meshloop-cli/tests/plan_and_run.rs` is renamed in comment to `plan_writes_the_canned_fixture_graph`; it asserts structure and the `awaiting-plan-review` marker, not that "build a widget" appears as a node.

**No new planner algorithm.** We change the **artifact channel** to match ADR 0003.

`meshloop plan` opens the on-disk store (same default as `run`) so a later `run --accept-plan` can see probe/quota observations; it does **not** mark the graph `PlanAccepted`.

```mermaid
sequenceDiagram
  participant Op as Operator
  participant CLI as meshloop plan
  participant Saga as RunLoop
  participant H as CliHarness
  participant WT as Planning worktree
  participant V as TaskGraph::validate

  Op->>CLI: --objective "..."
  CLI->>Saga: plan(objective, scope)
  Note over Saga: QACR + build_planning_spec inside RunLoop
  Saga->>H: invoke(planning spec)
  H->>WT: write .meshloop-prompt-N
  H->>H: spawn configured argv
  H-->>Saga: collect(exit, stdout)
  Saga->>WT: read meshloop-plan.json
  alt file present
    WT-->>Saga: JSON
  else fallback
    Note over Saga: parse stdout (fixture)
  end
  Saga->>V: validate + sanitize graph_id
  V-->>Op: awaiting-plan-review (printed + file)
```

### 2. Verification inspects git, not process exit

ADR 0007 / ML-005 / ML-012: evidence is bound to the exact candidate revision; the worktree diff is what verification evaluates.

**`satisfies()` is not enough.** Current `evidence::satisfies` (`crates/meshloop-domain/src/evidence.rs`) returns true if *any* `Deterministic(_)` row exists. It never reads `exit_code`. R1 adds a second pure function and uses both:

```rust
/// True iff every DeterministicEvidence row has exit_code == 0.
/// Vacuous-false: no deterministic rows → false (same as satisfies(DeterministicOnly, &[])).
pub fn all_deterministic_passed(evidence: &[Evidence]) -> bool {
    let rows: Vec<_> = evidence.iter().filter_map(|e| match e {
        Evidence::Deterministic(d) => Some(d),
        _ => None,
    }).collect();
    !rows.is_empty() && rows.iter().all(|d| d.exit_code == 0)
}
```

**Orchestrator gate for `Verifying → AwaitingReview`:**

At this transition, `required` is **always** `RequiredEvidence::DeterministicOnly` in R1, for every tier including Tier 3. `HumanAcceptanceEvidence` is **never** required here; `accept_human` records it later and emits `HumanAcceptanceRecorded` without re-running `satisfies(DeterministicAndHumanAcceptance)` as a verify gate. The precondition for accept is simply that the node is already `AwaitingReview`.

1. Record **every** produced deterministic row (pass or fail) for the candidate — audit first.
2. If `!all_deterministic_passed(rows)` → emit `DeterministicChecksFailed` → `Failed`.
3. Else if `!satisfies(RequiredEvidence::DeterministicOnly, rows)` → emit `DeterministicChecksFailed` → `Failed` (no deterministic kind present).
4. Else emit `DeterministicChecksPassed` → `AwaitingReview`.

Domain/engine tests (required):

- `DeterministicEvidence { exit_code: 1 }` ⇒ `all_deterministic_passed` false; `satisfies(DeterministicOnly)` may still be true — the orchestrator must not treat that as a pass.
- Two rows, git-diff 0 and `verify_command` 1 ⇒ failed.
- Empty slice ⇒ both functions false.
- **Tier 3 with only passing git-diff evidence reaches `AwaitingReview`, not `Failed`.** Human accept is a later command.

**R1 verification pipeline** (after `HarnessExited`, state `Verifying`):

1. **Refuse to treat harness exit code as evidence of correctness.** `collect` returning `Ok(HarnessOutcome)` means the process exited (including exit 1) and **always** appends `HarnessExited` → `Verifying`. Git-diff / `verify_command` then pass or fail. Exit 0 with an empty diff is a failed git-diff check (the ADR 0003 fixture case). Reserve `ProcessFault` / `HarnessCrashedOrTimeout` for spawn failure, I/O on pipes, timeout, and kill — never for a waitable non-zero exit.
2. **Delete `.meshloop-prompt-<attempt>`** from the worktree before any commit (that file is written by `CliHarness::invoke` at `harness.rs:84-88`).
3. **Materialize a candidate revision.** If the harness left uncommitted changes, `WorkspacePort::commit_all` creates **one Meshloop-owned attempt commit** on the attempt branch (`meshloop/<sanitized-graph-id>/task-<id>/attempt-<n>`), message `meshloop: attempt <id> for task <task_id>` with no prompt text. Author/committer are set via process env (`GIT_AUTHOR_NAME=meshloop`, `GIT_AUTHOR_EMAIL=meshloop@localhost`, and the matching `GIT_COMMITTER_*`) — **not** by rewriting the operator repo’s `user.name`. If the repo rejects the commit for other reasons, verification fails (recorded). If the harness already committed, use `HEAD`. `CandidateRef.revision` is that hash. Never `"worktree-uncommitted"`. If the tree is clean and `empty_diff_ok` is false, still record git-diff evidence with `exit_code = 1` and fail the gate.
4. **Produce `DeterministicEvidence` from git** (tool `git-diff`, version from `git --version`):
   - `exit_code = 0` iff the tree differs from the attempt’s base revision **or** the node is explicitly marked `empty_diff_ok` (default false).
   - `output_redacted` = diffstat + path list, length-capped and secret-scanned. No file contents, no prompt.
5. **Optional configured check.** `verify_command` lives only under `[verify]` in `meshloop.toml`:

   ```toml
   [verify]
   verify_command = ["cargo", "test", "--offline", "--locked"]
   ```

   Deserialize default is an empty vec (git-diff only). Structured argv, run in the worktree via `CheckRunner`, timeout = `task_timeout_seconds`. Example is operator-supplied, not hardcoded. A second `DeterministicEvidence` row (tool = argv[0]). Per-node override is **not** in R1 (`TaskNode` stays small). `[limits]` stays timeouts/retries/concurrency only.
6. **Path containment (ML-007, partial).** See matching rules below. A violation is an additional deterministic row (tool `allowed-paths`, `exit_code = 1`) or a failed git-diff row — either way `all_deterministic_passed` is false.
7. **Gate** as specified above. The orchestrator is the only caller of `state::transition`.
8. **All tiers stop at `AwaitingReview`.** `meshloop accept --task <id> --as <identity>` records `HumanAcceptanceEvidence` and emits `HumanAcceptanceRecorded`. `--as` rejects the empty string. There is **no** `policy:tier-1-2-auto` marker and no `ModelReviewPassed` (that would lie). Model review is not dispatched.

**`allowed_paths` matching (when the vec is non-empty):**

- Paths are **relative to the worktree root**.
- Compared against `git diff --name-only <base>` output, normalized to `/` separators, **case-sensitive as git reports them** on that repo (no extra Windows folding).
- A rule ending in `/` is a **prefix** (directory); otherwise exact match.
- Deleted paths are still "changed" and must match.
- Symlink escape and writes **outside** the worktree are **out of scope** for this check (threat-model residual: harness can write anywhere the user can). Do not claim "any path under the worktree" as sandboxing.
- Empty `allowed_paths`: skip the check; log one warning per node that no path policy was declared.

Fixture test: node with `allowed_paths = ["src/"]` whose worktree change is `README.md` → `DeterministicChecksFailed`.

```mermaid
stateDiagram-v2
  [*] --> Pending: plan accepted
  Pending --> Ready: deps integrated
  Ready --> Running: attempt started
  Running --> Verifying: harness exited
  Running --> Failed: crash/timeout/cancel
  Verifying --> AwaitingReview: all_deterministic_passed and satisfies
  Verifying --> Failed: any deterministic exit_code != 0 or missing kinds
  AwaitingReview --> Accepted: meshloop accept --as
  AwaitingReview --> Failed: review rejected
  Accepted --> Integrated: merge in integrate worktree
  Accepted --> Failed: stale base / merge conflict
  Failed --> Ready: resume --retry only
```

### 3. Worktrees kept, stacked, integrated

**Layout:**

```text
<repo>/                          # operator checkout — never auto-checked-out
  .meshloop/
    state.sqlite                 # default db (+ -wal/-shm)
    logs/                        # redacted attempt stdout
    plan.json                    # snapshot written at PlanAccepted
```

Worktrees live **beside** the repo, namespaced by the repo directory so two clones sharing a parent do not collide:

```text
<repo-parent>/
  <repo>/
  .meshloop-worktrees/
    <repo-dir-name>/
      <sanitized-graph-id>/
        plan/
        integrate/               # THE integrate worktree — only merge target for per-node integrate
        task-<id>-attempt-<n>/
```

Override: `--worktree-base` replaces `<repo-parent>/.meshloop-worktrees/<repo-dir-name>`. `--force` remove is **not** used on success. Failed attempts keep the worktree for `inspect`.

This repository’s `.gitignore` adds `.meshloop/` and `.meshloop-worktrees/` (covers `state.sqlite`, `-wal`, `-shm`, `plan.json`). R1 **does not** rewrite a target repo’s gitignore.

**`GitWorktreeAdapter` R1 methods** (structured argv, `current_dir` is the *named* worktree or `repo_root` only for `worktree add` / `worktree list`):

| Method | Git | Notes |
|---|---|---|
| `add_worktree_from(path, branch, start_point)` | `worktree add -b <branch> <path> <start_point>` | Fails if path or branch exists — caller must not treat that as a retry |
| `head(worktree)` | `-C <worktree> rev-parse HEAD` | |
| `diff_against(worktree, base)` | `-C <worktree> diff --stat/--name-only <base>` | |
| `status_porcelain(worktree)` | `-C <worktree> status --porcelain` | |
| `commit_all(worktree, message)` | `-C <worktree> add -A` then `commit` | Env identity above; skip if clean; never interpolates prompt |
| `merge_in_worktree(worktree, from_ref)` | `-C <worktree> merge --no-ff --no-edit <from_ref>` | On conflict: `merge --abort`; worktree HEAD SHA unchanged |
| `branch_exists` | `rev-parse --verify` | |
| `worktree_exists(path)` | path is a directory and `rev-parse` succeeds | |
| `prune` | `worktree prune` | Cleanup of registered-but-missing paths |

**Per-node integrate never checks out `repo_root`.** At `RunLoop::start`:

1. Capture `run_base = git -C repo_root rev-parse HEAD`.
2. `add_worktree_from(integrate_path, meshloop/<gid>/integrate, run_base)`.

When a task reaches `Accepted`, merge **in the integrate worktree**:

```text
git -C <integrate-wt> merge --no-ff --no-edit meshloop/<gid>/task-<id>/attempt-<n>
```

Success → `IntegrationOwnerMerge` → `Integrated`. Conflict or dirty integrate tree → `merge --abort`, `StaleBaseDetected` → `Failed`. The operator’s `HEAD` and index are untouched.

**Dependent composition.** `execution-lifecycle.md` requires upstream nodes to reach **`integrated`** before `pending → ready`. R1 honors that.

Next ready node’s attempt worktree is created **from `meshloop/<gid>/integrate` HEAD** (the integrate worktree’s current commit), so the dependent sees the real predecessor tree. Prompt situation slot still includes predecessor descriptions; R1 also appends a redacted diffstat (`dependency_context_with_diffs`). Sibling isolation tests still pass.

**Graph-level `meshloop integrate --into <ref> --accept-integrate`** is the **only** path that may operate on `repo_root`:

- Refuse if `repo_root` is dirty (`status --porcelain` non-empty).
- Refuse without `--accept-integrate`.
- Fast-forward iff **integrate HEAD is a descendant of `<ref>`**, including equality (already up to date). Predicate: `git merge-base --is-ancestor <ref> <integrate-HEAD>`. After a normal run this is true (`run_base` is an ancestor of integrate HEAD). Commands: `merge --ff-only` from `repo_root` after the dirty check, or `update-ref` to move `<ref>` to integrate HEAD. **Never** `reset --hard` (would rewind a ref that is actually ahead of integrate).
- Else (`<ref>` is not an ancestor of integrate HEAD): checkout `<ref>` in `repo_root` (operator asked) and `merge --no-ff` from integrate; on conflict `merge --abort` and leave both SHAs **byte-identical** to before the call.
- Never force-push. Never `branch -f` / `reset --hard`.

```mermaid
flowchart LR
  Main[run_base captured]
  IW[integrate worktree]
  T1[task-1 attempt worktree]
  T2[task-2 attempt worktree]
  Op[operator repo_root]

  Main --> IW
  IW --> T1
  T1 -->|Accepted: merge --no-ff in integrate wt| IW
  IW --> T2
  T2 -->|Accepted: merge --no-ff in integrate wt| IW
  IW -.->|meshloop integrate --into --accept-integrate| Op
```

**One writer.** At most one `Running` attempt. `max_concurrent_workers > 1` logs a warning and is clamped to 1.

**Leftover Git state after crash:**

- If the attempt is still `Running`/`Failed` and `worktree_exists(path)`: **do not** `worktree add` again. Inspect/reuse that path for `cancel`/`inspect`. Never `worktree add` as an implicit retry.
- If `path` exists but is not a worktree, or `branch` exists without a worktree: fail the new attempt start with `ProcessFault` / `OrchestratorError` — do not delete operator data.
- `worktree prune` is allowed for git’s stale administrative entries, not for deleting on-disk attempt directories.
- **Planning worktree (`plan/`):** `plan()` is not a `resume` path. If `plan/` already exists as a Meshloop worktree, **reuse it**: `git -C <plan-wt> reset --hard` to `repo_root` HEAD (this does **not** touch `repo_root`). If `plan/` exists but is not a worktree, or the plan branch exists without a worktree: fail with an explicit “remove `.meshloop-worktrees/<repo>/<gid>/plan` and retry” — do not delete it. Same branch-exists rule as attempts. Contract test in PR 3.
- Contract tests (PR 3): add when path exists (error), add when branch exists (error), prune is a no-op on a clean repo, reuse existing `plan/` worktree.

### 4. Persistence, event log, crash recovery

**Default store:** `<repo>/.meshloop/state.sqlite`. `--db` still overrides. In-memory is **test-only**. `SqliteStore::open` (on-disk only) sets **WAL** and `busy_timeout = 5000` ms so `status` / `inspect` / `cancel` can run concurrently with `run`. In-memory tests may skip WAL. Contract test: two connections, append on one, `records_for_graph` on the other.

**Schema v1 → v2** (forward-only). Existing `schema_meta`, `evidence`, `routing_feedback` remain. New:

```sql
CREATE TABLE IF NOT EXISTS events (
  event_id INTEGER PRIMARY KEY AUTOINCREMENT,
  graph_id TEXT NOT NULL,
  task_id INTEGER NOT NULL,
  attempt_id INTEGER NOT NULL,
  from_state TEXT NOT NULL,
  to_state TEXT NOT NULL,
  event_type TEXT NOT NULL,
  reason TEXT,
  executor TEXT NOT NULL,
  evidence_ref TEXT,
  occurred_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS runs (
  graph_id TEXT PRIMARY KEY,
  plan_state TEXT NOT NULL,
  run_base TEXT NOT NULL,
  integrate_ref TEXT NOT NULL,
  plan_json TEXT NOT NULL,
  plan_sha256 TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS attempts (
  attempt_id INTEGER PRIMARY KEY,           -- database-global monotonic
  graph_id TEXT NOT NULL,
  task_id INTEGER NOT NULL,
  harness TEXT,
  model_ref TEXT,
  worktree_path TEXT,
  pid INTEGER,
  image_name TEXT,                         -- executable file name for PID-reuse checks
  started_at TEXT,
  ended_at TEXT,
  outcome TEXT
);
CREATE TABLE IF NOT EXISTS quota_state (
  harness TEXT PRIMARY KEY,
  breaker TEXT NOT NULL,
  opened_at TEXT,                          -- RFC3339 or NULL
  cooldown_ms INTEGER NOT NULL
);
```

**Graph snapshot.** At `PlanAccepted`, write `.meshloop/plan.json` **and** `runs.plan_json` + `runs.plan_sha256`. `resume` loads `plan_json` from SQL (authoritative). If the file is missing, continue from SQL. If the file exists and its hash ≠ `plan_sha256`, **refuse** (operator edited the snapshot). There is no live graph-edit path.

**Attempt ids are database-global** (`INTEGER PRIMARY KEY AUTOINCREMENT` / `MAX(attempt_id)+1` across the file). They are **not** per-graph. Two graphs in one db cannot both start at 1; that is fine — `AttemptId` only needs uniqueness for evidence binding. Never `AttemptId(task_id)`.

**Retry count** is derived, not a column: `retry_count(task) = COUNT(*) FROM attempts WHERE graph_id=? AND task_id=?`. `RetryAuthorized` is legal iff `retry_count < limits.max_retries`. Planning uses `TaskId(0)` only in the in-memory `AgentSpec`; it is **not** inserted as a task row. Graph nodes must have `id >= 1`.

**Event serialization.** `event_type` / `from_state` / `to_state` are the `Debug` names of `Event` / `TaskState` (`AttemptStarted`, `Ready`, …) — stable because they are domain enums. `occurred_at` is RFC3339 UTC. `executor` is `meshloop` or the `--as` identity.

**`EventLog::append` is transactional** with the matching `attempts` / `evidence` rows. Kill-mid-append either commits the whole transition or none of it.

**`replay` validates the fold.** Reuse `TransitionRecord` (do not invent `PersistedTransition` as a second type). Add optional `graph_id`, `reason`, `executor`, `occurred_at` fields with serde/SQL defaults, or wrap as `PersistedTransition { record: TransitionRecord, ... }` with `record` reused. `replay`:

1. For each row, `transition(from, event)` must be `Ok(to)`; otherwise `StoreError::Corrupt` (or debug-assert in tests + error in prod).
2. Projection is last legal `to` per `(task_id, attempt_id)`.

**QuotaState persistence.** Domain additive API:

```rust
impl QuotaState {
    pub fn from_parts(breaker: Breaker, opened_at: Option<SystemTime>, cooldown: Duration) -> Self;
    pub fn opened_at(&self) -> Option<SystemTime>;
    pub fn cooldown(&self) -> Duration;
}
```

On-disk `opened_at` is RFC3339. `QuotaStore::{load,save}` return `Result<_, StoreError>`. Missing row → `Ok(QuotaState::default())` (Closed). IO failure must **not** look like “no exhaustion observed.”

Redaction: before any INSERT of `reason`, `output_redacted`, `plan_json` is the accepted graph (no prompts). Scan secret patterns (PEM headers, `sk-`, `ghp_`, `Bearer `, `api_key=`) and cap at 2 KiB for evidence/reason/logs. Do not persist raw prompts or full diffs.

#### Recovery policy (resolved; not an open question)

| Observed at `resume` | Legal event appended | Next | Auto-retry? |
|---|---|---|---|
| `Running`, PID hint Dead (or Ambiguous), or worktree missing | `HarnessCrashedOrTimeout` | `Failed` | **No** |
| `Running`, PID hint Live **and** image name matches **and** worktree exists | none | stays `Running` | No collect reattach. Operator `cancel` |
| `Verifying` with no evidence (crash mid-verify) | `DeterministicChecksFailed` | `Failed` | **No** |
| `Accepted` with dirty/incomplete integrate merge (`MERGE_HEAD` or lock) | `StaleBaseDetected` | `Failed` | **No** (do not retry merge) |

- **`resume` without `--retry`:** appends the legal fail events above, then **loops `tick` until `Idle`** (same stop set as `run`). That loop **includes** tick step 4 (`Accepted` → merge in the integrate worktree). It does **not** emit `RetryAuthorized`. It is not “Ready work only.”
- **`resume --retry`:** first, for each `Failed` task with `retry_count < max_retries`, emit `RetryAuthorized` → `Ready`; then **loop `tick` until `Idle`** as above.
- **Ctrl+C / console close:** on Windows this typically kills the process group, including the harness child. Treat as orchestrator crash: children are usually Dead. Same table as `taskkill` of `meshloop.exe`. Orphans (Live child after orchestrator death) are the exception and require `cancel`.
- **`recovery::reconcile`:** remains a **view** used by `status` to annotate “effectively blocked / orphan”. It is **not** written to the event log and is **not** what `resume` uses to pick the next state. Status prints `state=Failed` (log) plus `note=reconcile-view: was Running with no live pid` when useful — never a second source of truth.

**ProcessView (PID reuse):**

```rust
pub struct ProcessHint { pub pid: u32, pub image_name: Option<String> }
pub enum LiveCheck { Live, Dead, Ambiguous }
pub trait ProcessView {
    fn is_live(&self, hint: &ProcessHint) -> LiveCheck;
}
```

Windows implementation: `tasklist /FO CSV /NH /FI PID eq <n>` (CSV, not English column headers). `Live` only if the PID is listed **and** the image name matches `image_name` when that is known. PID listed but image differs → `Ambiguous`. `cancel` **must not** `taskkill` on `Ambiguous` (wrong process). Document PID reuse as a residual.

`HarnessHandle` gains `pid: Option<u32>` (source break for test struct literals).

**Stdio reattach is impossible** after the orchestrator dies. R1 does not claim it.

### 5. Operator surface

**Commands (R1):**

| Command | Role |
|---|---|
| `meshloop` (no args) | Short status of the product + last run if `.meshloop/state.sqlite` exists. **Not** "orchestration is not implemented". |
| `meshloop --help` / `--version` | Accurate usage. |
| `meshloop plan --objective <text> [--config] [--out] [--scope]` | Produce a reviewable graph file. Opens the store; does not accept the plan. |
| `meshloop run --plan <path> --accept-plan [--config] [--worktree-base] [--db] [--allow-live-harness]` | Snapshot+accept plan, then loop `tick` until `Idle`. Exit 0 only for `GraphComplete` or `AwaitingHumanAcceptance` with no Failed/Blocked; 1 otherwise. If `graph_id` already exists: not terminal → refuse, tell operator to `resume`; terminal → refuse (new `graph_id` required). |
| `meshloop status [--graph <id>]` | Replay event log; print per-node **log** state; annotate reconcile-view; worktree path, HEAD, evidence. Supported **during** `run` (WAL). |
| `meshloop resume [--graph <id>] [--retry]` | Append crash-fail events (and `RetryAuthorized` iff `--retry`), then **loop `tick` until `Idle`**: Accepted merge, Pending promotion, Ready dispatch — not only Ready nodes. Same exit-code map as `run`. |
| `meshloop cancel [--graph <id>] [--task <id>]` | Legal `Cancel`; kill only if `LiveCheck::Live`; no merge. Idempotent. |
| `meshloop inspect --task <id> [--graph <id>]` | Diffstat, evidence (including failed rows), attempt list, worktree path. |
| `meshloop accept --task <id> --as <identity>` | Any tier at `AwaitingReview`. `--as` must be non-empty. |
| `meshloop integrate --graph <id> --into <ref> --accept-integrate` | Only command that touches `repo_root`. |

Keep hand-rolled argv parsing. Extract `crates/meshloop-cli/src/args.rs`.

**Config discovery order:**

1. `--config <path>`
2. `./meshloop.toml`
3. `./config/meshloop.toml`
4. **Stop.** Do **not** silently fall back to `config/meshloop.example.toml`. Tests pass `--config`.

**`[verify]`:** optional. `verify_command = []` by default (git-diff only). Not under `[limits]`.

**Example config:**

- Show `executable = "target/debug/fixture_harness.exe"` on Windows.
- `{prompt_file}` and `{model_ref}` placeholders.
- `max_retries` is **maximum attempts per task** (the first dispatch counts). `max_retries = 1` ⇒ one attempt, no fallback. Example value `2` means two attempts, not “two retries after the first.”
- No real model ids, no permission-bypass flags.

**EXE_SUFFIX rule:** if the configured `executable` contains a path separator (`/` or `\`) **or** is a relative file that exists, and that path is missing, retry `path + EXE_SUFFIX`. **Bare names** (`claude`, `git`) are passed to `Command::new` for PATH lookup — never existence-checked at compose time, never suffixed.

**Live-harness refusal (required, not optional):** if any selected harness name is not `fixture` and `--allow-live-harness` is absent, `run`/`plan` exit non-zero **before spawn**. This test lives in the required PR set.

**Windows EXE.** `cargo build --release -p meshloop-cli` → `target/release/meshloop.exe`. No installer.

### 6. Live harness vs fixture

**Default and CI path: fixture only.** Completing a live task is not an R1 stamp requirement. **Refusing** a live spawn without the flag **is** an R1 stamp requirement.

Optional live path (not a rewrite later):

- Operator-authored `meshloop.toml`, one harness section. Recommended first candidate if authorized: Claude Code (probe already succeeded here). **No hardcoded flags.**
- `--allow-live-harness` required.
- Warn once: consumes subscription quota; Meshloop stores no credentials.
- Do not recommend permission-bypass flags in shipped docs.
- `{model_ref}` substituted only when the template contains `{model_ref}`.
- `capacity_exhausted_patterns` is optional config (default empty). Empty means we do **not** pretend to detect quota. Patterns may land in the optional live PR; **engine/fixture tests must not wait for that PR** — the fixture gains `--exhausted` (stderr contains a stable token, `CliHarness` maps it in tests via a scripted/fake harness, or the fixture exits with a dedicated code that a test `CliHarnessConfig` does not need for production). Engine tests use a fake that returns `CapacityExhausted`.

**Herdr:** uncomposed. Direct subprocess is the R1 transport.

### 7. QACR in R1 without fake quota

Keep `Router::select`. **Rewrite** `dispatch_with_fallback` so it does not set `TaskState` and does not share an `AttemptId`/worktree across candidates.

| Signal / filter | R1 behavior |
|---|---|
| Configured set | Unchanged; never add a candidate |
| `probe` + `is_dispatchable` | Unchanged |
| `tier_fits` | Unchanged |
| `QuotaState` | Persisted via `QuotaStore`. Missing row = Closed. IO error ≠ Closed |
| `LoadBalanceSignal` | If **any** remaining candidate lacks a `headroom` entry, **omit the signal for every candidate** (all get contribution `0.0` from load). Only when *every* remaining candidate has an observed headroom value is the numeric score applied. Mixed known/unknown therefore does not let `0.1` beat “unknown”. Test this. Print “no quota observations” / “load signal omitted (incomplete headroom)” |
| `HistoricalSuccessSignal` | Unchanged (`0.5` when no rows) |
| `CouplingSignal` | Compute `coupling_penalty` from `allowed_paths` intersection when both nodes have them; else 0. Set `preferred_harness` to the last harness that reached `DeterministicChecksPassed` **in this graph** when that penalty is `> 0`; else `None`. (`CouplingSignal` scores 0 whenever `preferred_harness` is `None` — `router.rs:85` — so omitting this assignment would make the penalty a no-op.) Engine test: overlapping `allowed_paths` prefers the last successful harness |
| Fallback | Each candidate is a new attempt (see §8). Routing **success** (`record_outcome(..., true)`) only after `DeterministicChecksPassed`. `collect` Ok with empty diff is a verification failure, recorded as routing **failure** |

Replacement primitive:

```rust
/// Invoke+collect one already-chosen candidate. Does not touch TaskState.
pub fn invoke_one(
    harness: &dyn HarnessCapabilities,
    spec: &AgentSpec,
) -> Result<HarnessOutcome, HarnessError>;
```

Existing `dispatch_with_fallback` is either deleted or kept as a test-only helper that loops `invoke_one` **without** returning a state. Engine tests that assert `TaskState::Verifying` from it are rewritten onto `RunLoop`.

**`max_retries` vs fallback:**

- The TOML key stays `max_retries` for compatibility. Its meaning in R1 is **maximum attempts per task**, not “retries after the first.” The first dispatch counts: `max_retries = 1` ⇒ one attempt and no in-run fallback; `max_retries = 2` ⇒ at most two attempts. `RunLimits.max_retries` is that same integer. `retry_count(task) = COUNT(*) FROM attempts WHERE graph_id=? AND task_id=?`. `RetryAuthorized` is legal iff `retry_count < limits.max_retries`.
- Each QACR candidate dispatch is one attempt (new id, new worktree, `AttemptStarted`).
- On `CapacityExhausted` / `Timeout` / spawn-or-IO `ProcessFault`: append `HarnessCrashedOrTimeout` (`CapacityExhausted` reason `capacity-exhausted` — not a new `Event` variant). A waitable non-zero harness exit is **not** this path; it is `HarnessExited` → `Verifying`.
- If **remaining candidates** exist **and** `retry_count < max_retries`, `RunLoop` appends `RetryAuthorized` **in the same `run` process** (this is fallback, not crash recovery) and the next tick starts the next candidate.
- Empty QACR at `Ready` has no legal event to `Blocked`. **Chosen:** leave the node `Ready` and return `Idle { NoCapableCandidate }` (no state change). Status prints `Ready (no capable candidate)` and lists dependents as “waiting on unsatisfiable ready node.” The operator must `meshloop cancel --task <id>` to move that node `Ready → Cancelled`; tick step 2 then promotes dependents `Pending → Blocked`. Do **not** auto-`Cancel` (wrong semantics). `run`/`resume` exit 1.

Empty select at fallback time (all remaining cooled down, last attempt already `Failed`): do not invent `Blocked` in the log; dependents see `Failed` and promote via step 2.

### 8. Orchestrator saga — numbered tick algorithm

```rust
pub struct RunLimits {
    pub max_retries: u32, // maximum attempts per task; first dispatch counts
    pub task_timeout: Duration,
    pub max_concurrent_workers: u32, // clamped to 1
}

pub struct RunLoop<'a> {
    pub harnesses: HashMap<String, &'a dyn HarnessCapabilities>,
    pub workspace: &'a dyn WorkspacePort,
    pub store: &'a mut dyn RunStore,
    pub processes: &'a dyn ProcessView,
    pub checks: &'a dyn CheckRunner,
    pub router: Router,
    pub limits: RunLimits,
    pub allow_live_harness: bool,
}

/// Closed set the CLI matches exhaustively. `run` and `resume` loop until `Idle`.
pub enum Tick {
    Transition { task: TaskId, attempt: AttemptId, from: TaskState, to: TaskState, event: Event },
    Idle { reason: IdleReason },
}

pub enum IdleReason {
    GraphComplete,             // every node Integrated
    AwaitingHumanAcceptance,   // ≥1 AwaitingReview; nothing else runnable
    NoCapableCandidate,        // ≥1 Ready with empty QACR; nothing else runnable
    FailedTerminal,            // ≥1 Failed/Blocked/Cancelled and nothing runnable
}

impl RunLoop<'_> {
    pub fn plan(&mut self, objective: &str, scope: &str) -> Result<TaskGraph, PlanError>;
    pub fn start(&mut self, graph: TaskGraph, run_base: String) -> Result<(), OrchestratorError>;
    pub fn accept_plan(&mut self, graph_id: &str) -> Result<(), OrchestratorError>;
    pub fn tick(&mut self) -> Result<Tick, OrchestratorError>;
    /// Append crash-fail events (and RetryAuthorized if `retry`), then loop tick until Idle.
    pub fn resume(&mut self, graph_id: &str, retry: bool) -> Result<IdleReason, OrchestratorError>;
    pub fn cancel(&mut self, graph_id: &str, task: Option<TaskId>) -> Result<(), OrchestratorError>;
    pub fn accept_human(&mut self, task: TaskId, who: &str) -> Result<(), OrchestratorError>;
    pub fn integrate_into(&mut self, graph_id: &str, git_ref: &str) -> Result<(), OrchestratorError>;
    pub fn status(&mut self, graph_id: &str) -> Result<RunStatus, OrchestratorError>;
}
```

**CLI stop set (one return per situation, no slash):** `run` and `resume` loop `tick` until `Tick::Idle`. Then map `IdleReason` to process exit:

| `IdleReason` | Exit | Condition |
|---|---|---|
| `GraphComplete` | 0 | all nodes `Integrated` |
| `AwaitingHumanAcceptance` | 0 | ≥1 `AwaitingReview`, nothing else runnable, **and** no `Failed`/`Blocked` |
| `AwaitingHumanAcceptance` | 1 | same, but some node is `Failed`/`Blocked` (stdout still tells the operator to `accept` the reviewable nodes) |
| `NoCapableCandidate` | 1 | |
| `FailedTerminal` | 1 | |

`run --plan --accept-plan` calls `start` then `accept_plan` then loops `tick` until `Idle`. `plan` does not call `accept_plan`.

**`start` when `graph_id` already exists:**

- If a `runs` row exists and the graph is **not terminal** (any node still `Pending`/`Ready`/`Running`/`Verifying`/`AwaitingReview`/`Accepted`): return error, CLI exits non-zero with “use `meshloop resume`”. Do not INSERT, do not `worktree add`.
- If a `runs` row exists and the graph **is terminal** (every node `Integrated`/`Failed`/`Cancelled`/`Blocked`): refuse overwrite; operator must use a new `graph_id`. No silent reset.
- If the integrate path/branch exists **without** a `runs` row (crash between `worktree add` and INSERT): fail `start` before any `AttemptStarted` per leftover-Git rules; do not `worktree add` again. Test both collisions.

**Event-before-effect:** `append` (transaction) then spawn/merge/kill. If `append` fails, do not spawn.

**If `append(AttemptStarted)` succeeds and `spawn` fails:** the log says `Running` with no PID. That is the resume path: `HarnessCrashedOrTimeout` → `Failed`. Do not silently delete the event.

**`tick` algorithm** (exactly one of these per call; `collect` is blocking and is part of the `Ready → Running → Verifying|Failed` *cycle*, which is **one** `tick` from the CLI’s point of view because concurrency is 1 and we cannot interleave):

Preconditions: `runs.plan_state == PlanAccepted`. If not, return `Idle` (caller bug). At most one task in `Running` in the projection.

1. **Load** snapshot `TaskGraph` from `runs.plan_json`. Replay events; abort on illegal fold.
2. **Promote pending.** `integrated = { nodes in Integrated }`. For each `Pending` node:
   - If `node.depends_on` all in `integrated`: append `DependencySatisfied` (`Pending → Ready`). Return that `Tick::Transition` (one promotion per tick).
   - Else if any dependency’s **current projected state** is `Failed`, `Cancelled`, or `Blocked`: append `DependencyFailedOrScopeRevoked` (`Pending → Blocked`). Return that transition. A three-node chain 1→2→3 with node 1 `Failed` therefore blocks node 2, then node 3 (dependency `Blocked` is unsatisfiable). Empty-QACR `Ready` is **not** unsatisfiable until the operator `cancel --task`s it.
3. **Human gate.** If any node is `AwaitingReview` and none are `Ready`/`Running`/`Accepted`: return `Idle { AwaitingHumanAcceptance }` **only** (not a second variant). CLI maps exit code per the table above.
4. **Per-node merge.** If any node is `Accepted`: merge that attempt ref **in the integrate worktree** (`merge_in_worktree`). Success: `IntegrationOwnerMerge` → `Integrated`. Conflict: `merge --abort`, `StaleBaseDetected` → `Failed`. Return that `Tick::Transition`. Do not start another attempt in the same tick. `resume` after `accept` **must** hit this step (it loops until Idle, not “Ready only”).
5. **Pick one Ready node** via `graph.ready_nodes(integrated)` intersect `state == Ready`. If none:
   - if any `AwaitingReview` remains, that was step 3;
   - if any `Accepted` remains, that was step 4;
   - if any `Failed`/`Blocked`/`Cancelled` and nothing runnable: `Idle { FailedTerminal }`;
   - if every node `Integrated`: `Idle { GraphComplete }`.
6. **QACR.** `Router::select(...)`. If empty: return `Idle { NoCapableCandidate }` (node stays `Ready`; no state change). Dependents stay `Pending` until `cancel --task` of that Ready node.
7. **Dispatch cycle (blocking):**
   1. Mint global `AttemptId`. Compute worktree path. **If path or branch exists, do not add; fail the start** (`OrchestratorError` / map to `HarnessCrashedOrTimeout` after a synthetic start only if we already appended — prefer failing *before* `AttemptStarted` if `worktree_exists`).
   2. Append `AttemptStarted` (`Ready → Running`) with harness/model/worktree/pid=NULL.
   3. `add_worktree_from` from integrate HEAD (or `run_base` if nothing integrated yet). On add failure: append `HarnessCrashedOrTimeout` → `Failed`; return.
   4. `invoke_one`. On invoke error: map `CapacityExhausted`/`Timeout`/`ProcessFault` → `HarnessCrashedOrTimeout` → `Failed`; persist quota on `CapacityExhausted`; record routing **failure**. On `Unsupported`: `OrchestratorError` (caller bug).
   5. Record pid + image_name on the attempts row (same transaction as a follow-up update; if the process dies here, resume uses Dead).
   6. Blocking `collect`. Timeout / kill / pipe I/O error → `HarnessCrashedOrTimeout` → `Failed`. `Ok(HarnessOutcome)` (any exit code, including 1) → `HarnessExited` → `Verifying`. Verification decides success; harness exit is never the gate.
   7. **Verify** (`verify_attempt`). Record all deterministic rows in the same or immediately subsequent transaction. Gate with `all_deterministic_passed` ∧ `satisfies(DeterministicOnly)`. Pass → `DeterministicChecksPassed` → `AwaitingReview` and `record_outcome(..., true)`. Fail → `DeterministicChecksFailed` → `Failed` and `record_outcome(..., false)`.
   8. **In-run fallback:** if the attempt `Failed` from steps 4/6/7 **and** `retry_count < max_retries` **and** unused QACR candidates remain, append `RetryAuthorized` (`Failed → Ready`) **before returning from this tick** (still one CLI-visible cycle: the next `tick` will dispatch the next candidate). If budget exhausted, leave `Failed`.
8. **Idle.** If nothing remains from steps 2–7, return `Idle` with the reason from step 5 (`GraphComplete` or `FailedTerminal`). Never return a non-Idle variant as a stop signal.

`tick` does **not** use `topological_order()` to dispatch. A predecessor `Failed`/`Cancelled`/`Blocked` never starts dependents (step 2). Empty-QACR `Ready` does not, until `cancel --task`.

**`status(&mut self)`.** The `store: &'a mut dyn RunStore` field makes `&self` status uncompilable. R1 uses `status(&mut self)` only. Do not split `EventLog` reads in R1. CLI is single-threaded.

### 9. Domain extensions (additive)

```rust
pub struct TaskNode {
    pub id: TaskId,
    pub description: String,
    pub depends_on: Vec<TaskId>,
    pub tier: Option<Tier>,
    #[serde(default)]
    pub allowed_paths: Vec<String>,
    #[serde(default)]
    pub empty_diff_ok: bool,
}
```

Old plan JSON remains valid.

`HarnessOutcome.worktree_changed` is **not used as a gate**.

`evidence::satisfies` **stays** the kind-check (do not silently change its meaning). New `all_deterministic_passed` sits beside it.

`QuotaState::from_parts` + accessors as above.

### 10. ADRs for R1

Do **not** silently mark 0001/0003/0005/0007/0009 Accepted.

**ADR 0016 — "Release 1 subset: closed-loop single-writer orchestration"** is landed **first** (Draft → Proposed). It does not supersede the five parents.

R1 claim boundary in 0016:

- Native Windows only; Herdr uncomposed; concurrency = 1; packaging deferred; live invoke opt-in (refusal is required).
- Planning artifact = worktree file; human-authored plans first-class.
- T1/T2/T3 all require `meshloop accept` (no policy auto-accept, no fake `HumanAcceptanceEvidence`).
- Dead `Running` → `Failed` via `HarnessCrashedOrTimeout`; `resume` does not auto-retry.
- Integrate worktree isolated from `repo_root`.
- Additive ports listed in the API section.
- `Ready` + empty QACR stays `Ready` with `Idle { NoCapableCandidate }` (no illegal `Blocked` transition). Operator `cancel --task` to propagate to dependents.

Human sequence: review this design → accept ADR 0016 → then saga PR.

`runtime-design.md` line 10 is stale; the ADR 0016 PR updates it to target vs R1 subset.

---

## API / Interface Changes

### Ports and DTOs (complete list)

| Item | Kind | Notes |
|---|---|---|
| `WorkspacePort` | New trait | Methods in §3. `merge_in_worktree`, never checkout `repo_root` |
| `EventLog` | New trait | `append(&mut self, TransitionRecord) -> Result`; `records_for_graph(&self, ...) -> Result<Vec<TransitionRecord>, _>` |
| `QuotaStore` | New trait | `load(&self, harness) -> Result<QuotaState, StoreError>`; `save(&mut self, ...) -> Result<(), _>` |
| `ProcessView` | New trait | `is_live(&self, &ProcessHint) -> LiveCheck` |
| `CheckRunner` | New trait | `run(&self, worktree: &Path, argv: &[String], timeout: Duration) -> Result<DeterministicEvidence, StoreError>` (or a small `CheckError`) |
| `RunStore` | Supertrait alias | `EvidenceStore + RoutingFeedbackStore + EventLog + QuotaStore` plus `save_run` / `load_run` for `runs` rows. **Not** a fourth SQLite type — `SqliteStore` impls all |
| `TransitionRecord` | Reused | Optional extra fields via wrapper if SQL needs `graph_id`; do not fork a parallel struct |
| `DiffSummary` | New DTO | `base: String, head: String, files: Vec<String>, stat_redacted: String` |
| `ProcessHint` / `LiveCheck` | New DTOs | |
| `HarnessHandle.pid` | New field | Source break for test literals |
| `HarnessCapabilities` | Unchanged methods | `probe/invoke/cancel/collect` |
| `HerdrSessionPort` | Unchanged, uncomposed | |
| `EvidenceStore::evidence_for` | **Signature change** | `-> Result<Vec<Evidence>, StoreError>` (defect 18). Source break |
| `RoutingFeedbackStore::record_outcome` | **Signature change** | `-> Result<(), StoreError>`. Source break. In-memory fake updated |
| `QuotaState::from_parts` | New | Domain |

`invoke_one` replaces `dispatch_with_fallback` as the production primitive.

### Domain

| Change | Kind |
|---|---|
| `TaskNode.allowed_paths`, `empty_diff_ok` | Additive, serde default |
| `all_deterministic_passed` | New function |
| `evidence::satisfies` | **Unchanged** (kind-check only) |
| `state::transition` table | **Unchanged** |
| `QuotaState` accessors / `from_parts` | Additive |
| `AttemptId` allocation | Behavioral: db-global |

### Agent / planner / harness / git

| Function | Change |
|---|---|
| `RunLoop::plan(objective, scope)` | Owns QACR + spec construction |
| `build_planning_spec` | Output contract → `meshloop-plan.json` |
| `decompose` | Read file then stdout |
| `dependency_context_with_diffs` | Additive |
| `CliHarness` template | `{prompt_file}` and `{model_ref}` |
| `CliHarness` mutex poison | `ProcessFault`, not `expect` |
| `GitWorktreeAdapter` | Methods in §3 |

### CLI

Saga out of `main.rs`. Default on-disk store. No `HerdrCliAdapter`. Live-harness refusal before spawn.

---

## Data Model Changes

Schema v1 → v2 as above. Migration: if version == 1, create new tables, set version = 2. If version > 2, refuse. Test: open a v1 file, read it as v2.

**Retention:** events and evidence indefinite. Worktrees kept.

**`.gitignore` (this repo):** `.meshloop/` and `.meshloop-worktrees/`. Never rewrite a target repo’s gitignore.

---

## Alternatives Considered

### A. Fixture-only R1 with no live-harness gate

- **Pros:** No subscription risk; CI is the whole claim.
- **Cons:** Operator cannot try a real CLI without a rewrite; criterion “refuse live spawn” would not exist.
- **Decision:** Fixture is the *default and the completion bar*. **Refusal** of ungated live spawn **is** required. Completing a live task is not.

### B. Spread across two harnesses in parallel in R1

- **Rejected.** Sequential single-writer.

### C. Keep stdout-JSON planning; tell operators to wrap CLIs

- **Rejected.** File artifact + human-authored plans.

### D. Skip integrate entirely; keep worktrees for the operator to merge by hand

- **Rejected.** Per-node merge in the **integrate worktree** is in R1. Operator `--into` stays explicit.

### E. Accept ADRs 0001/0003/0005/0007/0009 as written and implement "all of v1"

- **Rejected.** ADR 0016 subset; parents stay Proposed.

### F. Add clap + tokio + a Git crate

- **Rejected.** No new heavy deps.

### G. Reattach orphans via job objects / unsafe

- **Rejected.** PID hint + explicit cancel.

### H. T1/T2 policy auto-accept via fake `HumanAcceptanceEvidence`

- **Rejected.** Stops at `AwaitingReview` until `meshloop accept`. More friction; no fake evidence kind; `inspect` cannot confuse a policy marker with a human.

### I. Score missing headroom as `0.0` per candidate

- **Rejected.** Mixed known/unknown would prefer a nearly-exhausted observed harness over an unknown one. Omit the load signal for **all** candidates unless every remaining candidate has an observation.

### J. `resume` auto-`RetryAuthorized` after crash-fail

- **Rejected.** Contradicts ADR 0005 “never a silent retry.” Operator passes `--retry`.

---

## Security & Privacy Considerations

Threat model remains `docs/architecture/threat-model.md`. R1 control mapping:

| Threat | R1 control | Residual |
|---|---|---|
| Shell argument injection | Structured `Command.args` only | Operator-supplied template still becomes argv |
| Path traversal / writes outside repo | Post-hoc `allowed_paths` on `git diff --name-only` | **No OS sandbox.** Out-of-worktree writes are invisible to this check. Severity: **high** |
| Symlink escape | Out of scope for R1 path check | Same residual as threat-model |
| Prompt injection via repo files | Prompts built from graph fields | Harness may still read the repo. **Medium** |
| Secrets in logs/store | Secret-pattern scan + 2 KiB cap; no prompt persist; prompt file deleted before commit | Scanner is not DLP. **Medium** |
| Credential storage | None | Live invoke uses harness login. Gate: `--allow-live-harness` |
| Stale evidence / test tampering | Evidence bound to revision hash | Operator could modify worktree during `Verifying`. **Low** |
| Poisoned routing feedback | Feedback cannot add candidates; success only after deterministic pass | Can reorder. Acceptable |
| Orphan workers / PID reuse | `LiveCheck`; no `taskkill` on Ambiguous; `cancel` required | Cannot reattach. **Medium** |
| Partial integration | Merge in integrate worktree; `--abort` on conflict; SHA unchanged | Crash mid-merge → `StaleBaseDetected`, no auto-retry merge |
| Checkout of operator `HEAD` | Per-node merge never uses `repo_root` | Only `--into` may checkout, and only if clean |
| Herdr pane as fake isolation | Herdr uncomposed | N/A |

Unsafe code remains `forbid`.

---

## Observability

| Signal | Where |
|---|---|
| Every transition | `events` table + stdout one-liner |
| Routing decision | stdout (`format_routing`) + `reason` on `AttemptStarted` |
| Verification | `evidence` rows including **failed** `exit_code`; `inspect` prints them |
| Recovery | `status` prints **log state** first; reconcile-view as annotation only |
| Failures | stderr, non-zero exit; ML-010 no silent skip |

`status` during `run` is supported (WAL + busy_timeout).

No telemetry leaves the machine.

---

## Rollout Plan

1. Land PRs in the **PR Plan** order. ADR 0016 text is PR 1, **before** the saga.
2. Human accepts ADR 0016 before the saga PR merges.
3. Default path remains fixture. Live spawn refused without `--allow-live-harness`.
4. Stay on `0.1.x`. A git tag is separately authorized.
5. Rollback: revert the saga PR if the loop is unsafe; keep v2 migrate readable.
6. WSL2 / packaging / Herdr live are later releases.

No feature-flag crate.

---

## Risks

| Risk | Severity | Mitigation |
|---|---|---|
| Real CLIs ignore `meshloop-plan.json` | High | Human-authored plans are the primary path |
| Fixture tests currently assert "verified" on exit 0 with no diff | High | Fixture `--prompt-file` writes a file; assertions use diff-based wording |
| Operator friction: accept every node | Medium | Chosen over fake evidence; `run` pauses cleanly at `AwaitingReview` |
| Crash mid-`git merge` | Medium | `--abort`; SHA unchanged; no auto-retry merge |
| Worktree disk use | Low | Document; unique paths |
| PID reuse / `tasklist` | Medium | Image-name match; CSV format; no kill on Ambiguous |
| Operator template includes dangerous flags | High | Do not ship them; require `--allow-live-harness` |
| ADR 0016 not accepted | High | Design does not authorize implementation |
| Mutex poison panics | Low | Map to `ProcessFault` in the harness PR |
| Store swallows | Low | `Result` on evidence/feedback (source-breaking, assigned) |
| Two clones sharing a parent collide worktrees | Low | Namespace by `<repo-dir-name>` |

---

## Open Questions

Resolved in this revision (no longer open): T1/T2 gate (all tiers need `meshloop accept`); dead-Running mapping (`Failed` + no auto-retry); schema v1→v2 migration test (yes).

Still open, **not blocking PRs 1–4**; resolve before or with the saga PR if they affect it:

1. **Exactly-once merge** (already open in `execution-lifecycle.md`). R1: crash mid-merge → `StaleBaseDetected` → `Failed`; leave the integrate worktree for `inspect`; do not retry merge automatically. Confirm this is acceptable to the ADR 0005 owner.
2. **Whether `Blocked` needs sub-reasons.** R1 stores `reason` on the event; empty-QACR stays `Ready` + `Idle { NoCapableCandidate }` until `cancel --task`. Confirm we do not need a new `Event` for “no capable candidate.”
3. **Live harness choice** if the optional path is authorized: Claude Code vs Codex. Recommendation: Claude Code first on this host; not hardcoded.
4. **Whether `.meshloop/` belongs in the target repo.** This design splits: sqlite/logs/plan snapshot in-repo (gitignored), worktrees beside repo, namespaced by repo dir.

---

## Success criteria / acceptance tests for calling it "Release 1"

All on **native Windows**, `cargo run -p xtask -- check` green. Do not claim WSL2.

### Behavioral (must pass)

1. **Honesty.** No-args banner and README describe the real commands. `tests/scenarios/scaffold_cli.rs` asserts the new banner marker.
2. **Plan gate.** `run` without `--accept-plan` exits 2 and writes no worktrees and no events. With the flag, `PlanAccepted` exists before any `AttemptStarted`, and `runs.plan_json` matches the file hash.
3. **Structural validation.** Cyclic / dangling / empty plans rejected. `graph_id` with `/` rejected. `TaskId(0)` rejected.
4. **Diff-gated verification.** Fixture `--noop` (exit 0, no writes) or `invoke_args_template = ["--version"]` → node `Failed`; report does **not** say "verified"; a `DeterministicEvidence { exit_code: 1, tool: "git-diff" }` row exists. Fixture `--prompt-file` writes `fixture-touched.txt` → `exit_code = 0`, `revision` is a 40-char hash, `all_deterministic_passed` true. Domain unit test: `exit_code: 1` does not pass the orchestrator gate. T3 with only passing git-diff reaches `AwaitingReview`.
5. **Worktrees kept.** After a successful dispatch, `git worktree list` still shows the attempt path; no `--force` remove on success. `repo_root` `HEAD` unchanged after per-node integrate.
6. **Dependents see predecessor trees.** Two-node graph: after node 1 is accepted **and** integrated (via `accept --as` then `resume`, which merges), node 2’s worktree `HEAD` contains node 1’s file; prompt/inspect shows node 1 diffstat; sibling isolation test still passes. Node 1 `Failed` ⇒ node 2 never `Running`. Three-node chain: node 1 `Failed` ⇒ node 2 `Blocked` ⇒ node 3 `Blocked`. Empty-QACR `Ready` does not auto-block dependents until `cancel --task`.
7. **Durable default.** `run` without `--db` creates `<repo>/.meshloop/state.sqlite` with `events` rows. WAL sidecars may exist. Second process `status` succeeds while `run` holds the db.
8. **Recovery.** Start fixture `--hang`; kill `meshloop.exe` (and document Ctrl+C as the same Dead-child path). `meshloop resume` (no `--retry`) appends `HarnessCrashedOrTimeout`, leaves the task `Failed`, does **not** spawn a second hang. `resume --retry` is required to start a new attempt. After a T1 node reaches `AwaitingReview`, `accept --as` then `resume` without `--retry` performs `IntegrationOwnerMerge`; `repo_root` HEAD unchanged; integrate worktree contains the file. `cancel` is idempotent and does not `taskkill` on `Ambiguous`.
9. **Retries / attempts.** `max_retries` is maximum **attempts** per task (first dispatch counts). `max_retries = 1`; two QACR candidates: first `--fail` (or `--noop` empty diff) consumes the only attempt; second candidate is **not** started. `max_retries = 2`: first fail, second candidate gets attempt 2 with a **new** worktree. Never `AttemptId(task_id)`.
10. **Concurrency clamp.** Config `max_concurrent_workers = 8` logs a warning and never has two `Running` attempts.
11. **QACR honesty.** Routing report contains “no quota observations” or “load signal omitted” when headroom is incomplete. Mixed known/unknown does not prefer the known `0.1`. Fake/fixture `CapacityExhausted` persists breaker Open. Routing success is recorded only after deterministic pass.
12. **`{model_ref}`.** Contract test: template `["--model", "{model_ref}", "{prompt_file}"]` passes the configured model string.
13. **Windows fixture path.** Example config uses `fixture_harness.exe`. Bare `claude` is not existence-checked at compose.
14. **Human accept, all tiers.** A Tier 1 node does **not** reach `Accepted` without `meshloop accept --as ...`. Same for Tier 3.
15. **Integrate `--into`.** Without `--accept-integrate`, refused. With it, fast-forward iff `merge-base --is-ancestor <ref> <integrate-HEAD>` (integrate HEAD is a descendant of `<ref>`). Happy-path test: operator branch still at `run_base`, integrate has one merge commit → FF, operator ref == integrate HEAD, **no** merge commit on the operator branch. Never `reset --hard`. Otherwise merge commit. On conflict: `merge --abort`, both SHAs unchanged. `repo_root` dirty → refuse. Second `run` with the same `graph_id` while not terminal → refuse, message names `resume`. Terminal graph → refuse overwrite.
16. **Live harness default-off.** Config pointing at `claude` without `--allow-live-harness` exits non-zero and does not spawn.
17. **Redaction.** `-----BEGIN PRIVATE KEY-----` never appears in `evidence.payload_json` or event `reason`. Prompt file is not in the attempt commit.
18. **`allowed_paths`.** Node `["src/"]` writing `README.md` fails deterministic checks.
19. **Illegal log.** Hand-crafted event row with `Ready + HarnessExited` → `replay` returns `Corrupt`, not a guessed state.

### Explicitly not required to stamp R1

- Any live Claude/Codex/Pi/Grok/Agy task completing.
- Herdr pane spawn.
- WSL2 `cargo test`.
- `meshloop-windows-x86_64.exe` distribution artifact.
- Parallel two-harness success.
- Model-review agent.

### Honest status sentence after R1

> Meshloop R1 on native Windows can plan (or load a human graph), require plan acceptance, run one harness at a time in kept worktrees, verify git diffs, persist an event log, pause for human accept at every tier, and resume after a crash without auto-retry or duplicate integration. It does not orchestrate Herdr, does not parallelize across subscriptions, and does not claim five-harness production invoke.

If that sentence is true in the README and in the binary, it is Release 1.

---

## References

- `AGENTS.md` — work contract, no overclaim, crate direction
- `docs/product/brief.md`, `docs/product/requirements.md` (ML-001–ML-014)
- `docs/architecture/overview.md`, `boundaries.md`, `runtime-design.md`, `execution-lifecycle.md`, `threat-model.md`
- `docs/engineering/implementation-plan.md`, `implementation-status.md`, `testing.md`, `design-patterns.md`, `harnesses.md`, `rust.md`
- ADRs: 0001, 0002 (Accepted), 0003, 0005, 0007, 0009, 0011 (Accepted), 0012 (Accepted)
- Code: `crates/meshloop-{domain,engine,adapters,cli}/src/**`, `config/meshloop.example.toml`, `crates/meshloop-adapters/src/bin/fixture_harness.rs`
- Base: `c610ef3480f8b9a378a53af066ab6ca40919b9c9`

---

## Key Decisions

1. **R1 is one closed loop, not a thinner restatement of v1.** Plan/route/dispatch/verify/recover/integrate ship; Herdr composition, parallelism, five-harness invoke, WSL2 claim, and packaging do not.

2. **Human-authored task graphs are the primary planning path; agent planning writes `meshloop-plan.json`.** Stdout parse remains a fixture fallback. Fixture node set is canned; `graph_id` may hash the prompt; tests must not claim objective-sensitive decomposition.

3. **Verification records every deterministic row, then requires `all_deterministic_passed` (every `exit_code == 0`) and `satisfies(DeterministicOnly)`.** `satisfies` is not changed to read exit codes. At `Verifying → AwaitingReview`, `required` is always `DeterministicOnly` (including Tier 3). `HumanAcceptanceEvidence` is recorded only by `accept_human`. `collect` Ok (any exit code) is `HarnessExited` → `Verifying`; `ProcessFault` is spawn/IO/timeout/kill only. Harness exit is never “verified.”

4. **The integrate branch has its own worktree. Per-node merge is `git -C <integrate-wt> merge --no-ff`. `repo_root` is never checked out except explicit `meshloop integrate --into`.** Meshloop-owned commits use env identity `meshloop@localhost`, not `git config` on the operator repo.

5. **Operator-branch merge is explicit (`integrate --into` + `--accept-integrate`).** Fast-forward iff **integrate HEAD is a descendant of `<ref>`** (`merge-base --is-ancestor <ref> <integrate-HEAD>`), including equality. Never `reset --hard`. Otherwise `--no-ff`. Conflict → `--abort`, SHAs unchanged.

6. **Default persistence is on-disk SQLite with WAL, an `events` table, and `runs.plan_json`.** In-memory is test-only. `resume` refuses a mismatched snapshot hash.

7. **Dead `Running` with no collect → `HarnessCrashedOrTimeout` → `Failed`. `resume` does not auto-retry. `resume --retry` emits `RetryAuthorized`.** Then both forms **loop `tick` until `Idle`**, including `Accepted` merge. Orphans with `LiveCheck::Live` stay `Running` until `cancel`. Reconcile is a status annotation only. Ctrl+C ≡ orchestrator crash (children usually die).

8. **QACR stays; incomplete headroom omits the load signal for every remaining candidate.** Routing success is recorded only after `DeterministicChecksPassed`. Quota IO errors do not look like Closed.

9. **Each QACR candidate is its own attempt (new `AttemptId`, new worktree). `invoke_one` does not set `TaskState`. The TOML key `max_retries` is maximum attempts per task (first dispatch counts).** In-run fallback may `RetryAuthorized`; crash recovery may not. Empty QACR leaves the node `Ready` and returns `Idle { NoCapableCandidate }`; operator `cancel --task` to propagate.

10. **Concurrency is 1.** Dispatch uses `ready_nodes(integrated)`, never `topological_order()`.

11. **T1/T2/T3 all stop at `AwaitingReview` until `meshloop accept --as`.** No `policy:tier-1-2-auto` row. `run`/`resume` loop until `Idle { AwaitingHumanAcceptance | GraphComplete | NoCapableCandidate | FailedTerminal }`. After accept, `resume` (no `--retry`) performs the per-node integrate merge.

12. **Live harness invoke is opt-in (`--allow-live-harness`). Ungated non-fixture spawn must refuse (R1 stamp). Completing a live task is not an R1 stamp.**

13. **CLI surrenders the saga to `RunLoop`; ports are additive except the two `Result` signature fixes on store traits.**

14. **ADR 0016 defines the R1 subset and lands before the saga; 0001/0003/0005/0007/0009 stay Proposed.**

15. **Packaging (Phase 9) is deferred.** Version 0.1.x.

16. **No new heavy dependencies (clap/tokio/libgit2).**

17. **`allowed_paths` are worktree-relative, git `--name-only`, `/`-normalized, case-sensitive as git, prefix if the rule ends in `/`. Out-of-worktree writes are out of scope.**

18. **Attempt ids are database-global. `TaskId(0)` is reserved. `graph_id` must match `[A-Za-z0-9._-]+`. Default worktree base is namespaced by repo directory name.** Second `run` for an existing `graph_id`: not terminal → use `resume`; terminal → refuse overwrite.

19. **EXE_SUFFIX retry only for paths with a separator (or an existing relative file). Bare PATH names are not existence-checked at compose.**

20. **`Tick` is `Transition | Idle { GraphComplete | AwaitingHumanAcceptance | NoCapableCandidate | FailedTerminal }`.** CLI loops until `Idle`. Exit 0 only for `GraphComplete` or `AwaitingHumanAcceptance` with no Failed/Blocked. `status` is `&mut self`. `verify_command` lives under `[verify]` only. `preferred_harness` is the last `DeterministicChecksPassed` harness when coupling penalty > 0.

---

## PR Plan

Each PR is independently reviewable and must leave `cargo run -p xtask -- check` green. No PR claims “Release 1 complete” until PR 10. Do not commit unless authorized.

**Ordering rule:** ADR 0016 before saga. `{model_ref}` and load-signal omission before saga. Live-harness *refusal* in PR 1. Saga (PR 7) includes stacking, `ready_nodes(integrated)`, **`accept --as`**, and **`resume` without `--retry`**. Operator `--into` is PR 9. `resume --retry` is PR 8.

### PR 1 — Honesty, ADR 0016, gitignore, config discovery, live-harness refusal

- **Title:** `docs+cli: ADR 0016, honest banner, config discovery, refuse ungated live spawn`
- **Files/components:** `docs/architecture/adr/0016-release-1-subset.md` (Draft/Proposed); `docs/architecture/adr/README.md`; `README.md`; `crates/meshloop-cli/src/main.rs` (banner, `--help`, refuse non-fixture without `--allow-live-harness` even while `run` is still the old loop); `crates/meshloop-cli/src/args.rs` (extract); `crates/meshloop-cli/src/config.rs` (discovery order; `[verify] verify_command` default empty; comment that `max_retries` is max attempts); `config/meshloop.example.toml` (`fixture_harness.exe`, `{model_ref}`, `max_retries` comment); `.gitignore` (`.meshloop/`, `.meshloop-worktrees/`); `docs/engineering/implementation-status.md`; `docs/architecture/runtime-design.md` (stale “none of this is implemented”); `tests/scenarios/scaffold_cli.rs`; CLI test: config `executable = "claude"` without the flag does not spawn
- **Depends on:** none
- **Description:** Authorization + stop lying. EXE_SUFFIX retry **only** when the path contains a separator; not in compose existence-checks for bare names. Example config is not a silent default.

### PR 2 — `{model_ref}` substitution and honest load signal

- **Title:** `harness+router: substitute {model_ref}; omit load signal when headroom is incomplete`
- **Files/components:** `crates/meshloop-adapters/src/harness.rs`; `crates/meshloop-adapters/tests/harness_contract.rs`; `crates/meshloop-engine/src/router.rs`; router tests for mixed known/unknown headroom **and** overlapping `allowed_paths` with `preferred_harness` = last successful harness; `crates/meshloop-adapters/src/bin/fixture_harness.rs` (`--exhausted` stderr token for later quota tests)
- **Depends on:** none (parallel with PR 1)
- **Description:** One-line-class production-path honesty. Does not need `RunLoop`. Fixture `--exhausted` exists so later PRs are not blocked on live patterns. `preferred_harness` wiring lands here at the signal level; RunLoop supplies the value in PR 7.

### PR 3 — WorkspacePort, integrate worktree, leftover git state

- **Title:** `engine+adapters: WorkspacePort; merge in a dedicated worktree; never checkout repo_root`
- **Files/components:** `crates/meshloop-engine/src/ports.rs` (`WorkspacePort`, `DiffSummary`, `WorkspaceError`); `crates/meshloop-adapters/src/git.rs`; disposable-repo tests: `add_worktree_from`, `head`, `diff_against`, `commit_all` with env identity, `merge_in_worktree` success, conflict `--abort` SHA unchanged, add when path exists (err), add when branch exists (err), `prune`, reuse existing `plan/` worktree via reset-in-worktree; fake workspace for engine tests
- **Depends on:** none
- **Description:** CLI not yet wired. No `--force` on the new merge path. `remove_worktree` may keep a `force: bool` default false.

### PR 4 — Schema v2, WAL, EventLog, QuotaStore, Result-returning store

- **Title:** `adapters: SQLite v2 events/runs/attempts/quota_state with WAL`
- **Files/components:** `crates/meshloop-adapters/src/store.rs`; `crates/meshloop-engine/src/ports.rs` (`EventLog`, `QuotaStore`, `RunStore` supertrait); `crates/meshloop-engine/src/recovery.rs` (`replay` errors on illegal triples; reuse `TransitionRecord`); `crates/meshloop-domain/src/capability.rs` (`from_parts`, accessors); redaction helper; tests: v1 migrates; newer-than-v2 refused; two connections append/read; IO failure on load ≠ default Closed
- **Depends on:** none (parallel with PR 2–3)
- **Description:** `EvidenceStore::evidence_for` and `RoutingFeedbackStore::record_outcome` return `Result` (source break; update fakes and call sites). CLI may still default to memory until PR 7.

### PR 5 — Domain verification helpers and TaskNode fields

- **Title:** `domain: all_deterministic_passed, allowed_paths, reserve TaskId(0)`
- **Files/components:** `crates/meshloop-domain/src/evidence.rs`; `task_graph.rs` (`allowed_paths`, `empty_diff_ok`; validate `id != 0`; optional `graph_id` charset helper used by planner/CLI); `crates/meshloop-engine/src/planner.rs` (`assign_tiers` only writes `if node.tier.is_none()`); tests including `exit_code: 1`, two-row mixed pass/fail, and human plan `tier: Tier3` surviving `assign_tiers`
- **Depends on:** none
- **Description:** Does **not** change `satisfies()` semantics. `state::transition` untouched. Overwrite guard is this PR so PR 7 cannot ship the old always-write `assign_tiers`.

### PR 6 — verify_attempt + CheckRunner + fixture writes a file

- **Title:** `engine: verify_attempt from git diff and optional CheckRunner`
- **Files/components:** `crates/meshloop-engine/src/verify.rs`; `ports.rs` (`CheckRunner`); `CommandCheckRunner` in adapters; `fixture_harness.rs` (`--prompt-file` writes `fixture-touched.txt`; **`--noop` exits 0 and writes nothing** for the empty-diff scenario); allowed_paths unit test with a fake workspace
- **Depends on:** PR 3, PR 5
- **Description:** No CLI reporting change yet. Mutex poison → `ProcessFault` in `CliHarness` in this PR or PR 2 (assign here if not already). Empty-diff CLI tests use `--noop` (or `invoke_args_template = ["--version"]`), never `--prompt-file`.

### PR 7 — RunLoop closed loop (single-writer, stacking, attempt-per-candidate, `accept --as`)

- **Title:** `engine+cli: RunLoop tick algorithm; ready_nodes; integrate worktree; invoke_one; accept --as`
- **Files/components:** `crates/meshloop-engine/src/orchestrator.rs` (`RunLoop`, `invoke_one`, numbered tick, `accept_human`, `resume` looping until Idle); `crates/meshloop-cli/src/main.rs` (`plan`/`run`/`accept`/`resume` without `--retry`); `compose.rs`; `report.rs`; `agent.rs`; `planner.rs` (file-first decompose); CLI tests listed below
- **Depends on:** PR 1 (ADR 0016 + refusal), PR 2, PR 3, PR 4, PR 5, PR 6, **human acceptance of ADR 0016**
- **Description:** Saga PR. Includes stacking, `ready_nodes(integrated)`, **`accept --as` CLI**, and **`resume` without `--retry`** so accept is not a dead-end (second `run` for the same `graph_id` is refused). Does **not** include operator `--into` (PR 9) or `resume --retry`/ProcessView hang-kill (PR 8). CLI tests: one-node diff-pass → `Idle { AwaitingHumanAcceptance }` exit 0; `accept --as` then `resume` performs `IntegrationOwnerMerge`; two-node predecessor file visible after that; predecessor fail blocks dependent; three-node `Blocked` propagation; `--noop` empty diff fails; `repo_root` HEAD unchanged; duplicate `run` same `graph_id` refuses. In-run QACR fallback uses `RetryAuthorized` within budget. Do not force-remove worktrees.

### PR 8 — status / cancel / inspect; `resume --retry`; ProcessView

- **Title:** `cli: status/cancel/inspect; resume --retry; ProcessView`
- **Files/components:** CLI args/report; `WindowsProcessView` (`tasklist /FO CSV`); hang-fixture + `taskkill` of meshloop: `resume` without `--retry` fails the attempt and does not spawn; `resume --retry` starts attempt 2; Ctrl+C documented as Dead-child path
- **Depends on:** PR 7
- **Description:** ML-006. Reconcile-view is annotation only. Post-accept `resume` (no `--retry`) already in PR 7.

### PR 9 — `meshloop integrate --into`

- **Title:** `cli: integrate --into with FF-or-merge-abort`
- **Files/components:** CLI; `RunLoop::integrate_into`; tests: dirty `repo_root` refused; `merge-base --is-ancestor <ref> <integrate-HEAD>` ⇒ FF, operator ref == integrate HEAD, no merge commit on the operator branch; conflict abort SHA unchanged; never `reset --hard`
- **Depends on:** PR 7
- **Description:** Operator `--into` only. `accept --as` and stacking already landed in PR 7.

### PR 10 — R1 acceptance scenarios

- **Title:** `test+docs: R1 acceptance scenarios`
- **Files/components:** `tests/scenarios/` covering success criteria 1–19 not already owned; `docs/engineering/implementation-status.md` update; `schemas/README.md` snapshot of v2 SQL (0016 is Proposed)
- **Depends on:** PR 7–9
- **Description:** Does not self-accept parent ADRs. Stamping R1 is this PR plus human confirmation of the honest status sentence.

### PR 11 (optional, not required to stamp R1) — live-harness documentation and `capacity_exhausted_patterns`

- **Title:** `docs+cli: optional live invoke patterns`
- **Files/components:** config comments (still no hardcoded flags); docs warning; map `capacity_exhausted_patterns` in `CliHarness` (production heuristic)
- **Depends on:** PR 7
- **Description:** Refusal already shipped in PR 1. This PR only makes a *successful* live spawn configurable. No live CI test.

### PR 12 (deferred) — Herdr compose, parallelism, WSL2, packaging

- Not R1. Existing Phases 8–9 of `implementation-plan.md`.
