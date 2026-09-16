# 0028 Upstream graph mutation and dynamic replanning

- Status: Accepted
- Implementation: implemented
- Date: 2026-09-15
- Author/executor: Antigravity / Claude Code
- Reviewer: Grok 4.5 & Human Pair Programmer
- Approval evidence: Grok critique incorporated; xtask check & E2E suite passed
- Supersedes: none
- Superseded by: none

## Context and constraints
In complex multi-stage tasks, an agent executing a planned subtask may discover unforeseen prerequisites (e.g., an uninstalled crate dependency, a missing database migration, or an unwritten interface contract). If the task graph is strictly static after initial acceptance, the execution either fails completely or forces an out-of-band manual intervention.

Constraints:
- DAG Invariance: Mutated graphs must remain strictly directed acyclic graphs; cycle introductions and dangling dependencies must be rejected with atomic rollback before mutation commitment.
- State Immutability: Nodes in `Running`, `Verifying`, `AwaitingReview`, `Accepted`, or `Integrated` cannot have prerequisites inserted in-place.
- Readiness Consistency: Prepending a new upstream node to a `Ready` or `Pending` node must transition its readiness back to `Pending` via `Event::GraphMutated` until all upstream nodes reach `Integrated`.
- Deterministic Event Sourcing: All graph mutations emit `Event::GraphMutated` persisted in SQLite runs and records for exact crash recovery and replay.
- Operator Governance: Optional `--require-review` transitions `plan_state` back to `AwaitingPlanReview`, requiring `meshloop review-plan --accept` before execution can resume.

## Alternatives
1. **Full Graph Replacement**: Abort the entire run and trigger a new `meshloop plan` from scratch. Rejected: destroys completed work and inflates token/time costs.
2. **Unconstrained In-Place Task Modification**: Allow agents to freely add/remove arbitrary edges and edit running nodes. Rejected: introduces race conditions with active worktrees and breaks serial commit history.
3. **Bounded Upstream Prepending with Petgraph Validation**: Chosen. Agents may propose new prerequisite or followup nodes for pending tasks. The engine validates acyclicity via petgraph, updates tier assignments, inserts the nodes, records `Event::GraphMutated`, and persists the updated plan.

## Decision or proposal
1. **Domain Mutation Primitive**:
   - In `meshloop_domain::task_graph`, define `GraphMutation` enum with serde tagging:
     - `InsertPrerequisite { target_task: TaskId, new_tasks: Vec<TaskNode> }`:
       Terminal nodes (nodes in `new_tasks` with no dependents within `new_tasks`) are appended to `target_task.depends_on`. Existing dependencies of `target_task` are preserved.
     - `AppendFollowup { source_task: TaskId, new_tasks: Vec<TaskNode> }`:
       `source_task` is appended to the dependencies of root nodes in `new_tasks` (nodes with empty `depends_on`).
   - Atomic rollback and Petgraph DAG validation: cycles, duplicates, dangling dependencies, and reserved task ID 0 are rejected atomically.
2. **Readiness Recalculation and Transitions**:
   - In `meshloop_domain::state`, added `Event::GraphMutated`.
   - Transitions:
     - `(Ready, GraphMutated) => Pending` (reverses premature readiness if new unsatisfied prerequisites exist).
     - `(Pending, GraphMutated) => Pending`.
     - `(Blocked, GraphMutated) => Blocked`.
   - Engine immutability check: `target_task` cannot be in `Running`, `Verifying`, `AwaitingReview`, `Accepted`, or `Integrated`.
3. **Audit and Persistence**:
   - Updated `plan_json`, `plan_sha256`, and `Event::GraphMutated` records are persisted atomically via `RunStore::save_run_and_events(&row, &mutation_events)` within a single `BEGIN IMMEDIATE` transaction in SQLite.
   - `.meshloop/plan.json` derived cache is updated on disk only after successful transaction commitment.
4. **Operator Governance**:
   - `--require-review` flag sets `plan_state` to `AwaitingPlanReview`.
   - `meshloop resume` refuses to tick when `plan_state != PlanAccepted`.
   - Operator accepts via `meshloop review-plan --plan .meshloop/plan.json --accept --as <identity>`.
5. **Tool Surface**:
   - CLI command: `meshloop mutate-plan [--graph <id>] (--mutation <json> | --mutation-file <path>) [--require-review] [--json]`.
   - MCP tool: automatically exposed as `meshloop_mutate_plan` from `bundled_commands()`.

## Consequences
- Multi-agent workflows dynamically adapt to unexpected barriers without re-running completed tasks.
- 100% deterministic reproducibility from SQLite event replay (`replay_tasks`).
- Strict human oversight remains possible through `--require-review`.

## Verification and implementation evidence
- Domain unit tests in `crates/meshloop-domain/src/task_graph.rs`:
  - `test_apply_mutation_insert_prerequisite_rewires_correctly`
  - `test_apply_mutation_append_followup_rewires_correctly`
  - `test_apply_mutation_rejects_cycle_with_rollback`
  - `test_apply_mutation_rejects_missing_target`
  - `test_apply_mutation_rejects_duplicate_id`
  - `test_apply_mutation_rejects_reserved_task_id_0`
- State transition unit tests in `crates/meshloop-domain/src/state.rs`.
- Store atomic persistence in `crates/meshloop-adapters/src/store.rs` (`save_run_and_events`).
- Recovery replay unit test in `crates/meshloop-engine/src/recovery.rs`: `per_task_fold_replays_dynamically_mutated_tasks`.
- Engine integration test in `crates/meshloop-engine/src/run_loop.rs`.
- End-to-end integration tests in `crates/meshloop-cli/tests/plan_and_run.rs`:
  - `mutate_plan_inserts_prerequisite_and_resumes_to_completion`
  - `mutate_plan_with_require_review_gates_resume`
- Algorithmic benchmark: `orch.mutation.overhead_ms.p95 <= 15.0ms` across 6 canonical manifests evaluated in `xtask bench-mutation`.
- Rollback invariant: `iso.mutation_rollback.fidelity = 1.0` (100% snapshot equality across 7 negative control injections).
- Full `cargo run -p xtask -- check` and `smoke` validation.
