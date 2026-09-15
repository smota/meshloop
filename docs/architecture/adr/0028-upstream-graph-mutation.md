# 0028 Upstream graph mutation and dynamic replanning

- Status: Proposed
- Implementation: proposed
- Date: 2026-09-15
- Author/executor: Claude Code
- Reviewer: pending human
- Approval evidence: none
- Supersedes: none
- Superseded by: none

## Context and constraints
In complex multi-stage tasks, an agent executing a planned subtask may discover unforeseen prerequisites (e.g., an uninstalled crate dependency, an missing database migration, or an unwritten interface contract). If the task graph is strictly static after initial acceptance, the execution either fails completely or forces an out-of-band manual intervention.

Constraints:
- DAG Invariance: Mutated graphs must remain strictly directed acyclic graphs; cycle introductions must be rejected before mutation commitment.
- State Immutability: Nodes in `TaskState::Running` or `TaskState::Accepted` cannot be mutated, deleted, or swapped in-place.
- Readiness Consistency: Prepending a new upstream node to a `TaskState::Pending` node must transition its readiness back to blocked until the upstream node reaches `Accepted`.
- Deterministic Event Sourcing: All graph mutations must emit `GraphMutated` domain events persisted in SQLite for exact crash recovery and replay.
- Quota and Boundary Checks: Dynamic additions must respect overall session budgets and quota headroom.

## Alternatives
1. **Full Graph Replacement**: Abort the entire run and trigger a new `meshloop plan` from scratch. Rejected: destroys all completed work and inflates token/time costs.
2. **Unconstrained In-Place Task Modification**: Allow agents to freely add/remove arbitrary edges and edit running nodes. Rejected: introduces race conditions with active worktrees and breaks serial commit history.
3. **Bounded Upstream Prepending with DAG Validation**: Chosen. Agents may propose new prerequisite nodes for pending tasks. The engine validates acyclicity, inserts the nodes in `Pending` status, and records the mutation event.

## Decision or proposal
1. **Domain Mutation Primitive**:
   - In `meshloop-domain::task`, define `GraphMutation` enum:
     - `InsertPrerequisite { target_task: TaskId, new_tasks: Vec<TaskSpec> }`
     - `AppendFollowup { source_task: TaskId, new_tasks: Vec<TaskSpec> }`
   - Validate acyclicity on every proposed mutation via Kahn's algorithm before acceptance.
2. **Readiness Recalculation**:
   - When new prerequisites are added to pending tasks, the coordinator recalculates `is_ready(task)` based on all dependencies (including newly added ones).
3. **Audit and Persistence**:
   - Append `TaskEvent::GraphMutated { mutation, timestamp }` to the SQLite event log within an atomic `BEGIN IMMEDIATE` transaction.
4. **Operator Governance**:
   - If policy requires human sign-off on graph expansions, the run pauses at `PlanAdjusted` / `AwaitingReview`, requiring `meshloop:review-plan` before dispatching newly injected nodes.

## Consequences
- Allows multi-agent workflows to dynamically self-structure around unexpected codebase barriers without human re-planning.
- Preserves 100% deterministic reproducibility from the SQLite event log.
- Keeps engine scheduling simple by leveraging existing readiness checks.

## Verification and implementation evidence
- Domain graph acyclicity validation tests in `meshloop-domain`.
- Event persistence verification in `meshloop-adapters::store`.
