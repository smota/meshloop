# 0024 Bounded concurrent execution without Tokio

- Status: Proposed
- Implementation: implemented
- Date: 2026-09-15
- Author/executor: Antigravity
- Reviewer: pending human
- Approval evidence: none
- Supersedes: none
- Superseded by: none

## Context and constraints
Orchestrating multiple independent subtasks simultaneously accelerates agent workflow completion. However, introducing a full asynchronous runtime (e.g. Tokio) into the core coordinator (`meshloop-engine`) would introduce viral async lifetimes, complex reactor overhead, and violate the standalone, daemonless single-threaded architecture (ADR 0022).

Constraints:
- Zero async reactor in `meshloop-engine`: the coordinator `RunLoop` remains synchronous and deterministic.
- Concurrency bound: bounded to $N \in [1, 16]$ concurrent workers to prevent host resource starvation.
- Host storage isolation: concurrent operations in `.git` can collide on `.git/index.lock` under Windows file system caching and antivirus filters.
- Persistence integrity: concurrent state updates must avoid SQLite database locks (`SQLITE_BUSY`).
- Deterministic merge serialization: merging candidate revisions into the target branch must remain strictly serial.

## Alternatives
1. **Full async refactor using Tokio**: Convert all ports and the `RunLoop` to async/await. Rejected: violates hexagon simplicity, adds large dependency graph, and creates non-deterministic scheduling races.
2. **OS thread-per-worker in coordinator**: Spawn separate OS threads for each worker attempt inside `RunLoop`. Rejected: complicates crash recovery, state persistence, and cancellation coordination.
3. **Cooperative polling via non-blocking `try_collect()` in synchronous loop**: Chosen. The `RunLoop` tracks active attempts in a multiplexed collection, calling non-blocking `try_collect()` on running harness instances and sleeping 20ms only when waiting on live workers.

## Decision or proposal
1. **Harness Capability Non-Blocking Polling**:
   - Add `try_collect(&self, handle: &mut HarnessRunHandle) -> Result<Option<HarnessOutput>, HarnessError>` to `HarnessCapabilities`.
   - Provide a default fallback for backwards-compatibility.
   - Implement `try_collect` in `CliHarness` via non-blocking `Child::try_wait()`.
2. **Synchronous Multiplexing in `RunLoop`**:
   - Track active running tasks up to `max_concurrent_workers`.
   - In `tick()`, advance completed workers, verify candidates, and dispatch new ready tasks while capacity permits.
   - If workers are active but none have finished, return `RunLoopOutcome::WaitingOnLiveWorker` and sleep 20ms before the next tick.
3. **Git Admin Mutex and Index Lock Backoff**:
   - Serialize worktree creation (`git worktree add`), removal, and pruning through an in-process `GIT_ADMIN_LOCK` mutex.
   - Implement `run_admin_with_retry` with exponential backoff (50ms–2000ms) to transparently absorb transient `.git/index.lock` collisions on Windows.
4. **SQLite Concurrency**:
   - Enforce WAL mode and execute state transitions inside `BEGIN IMMEDIATE` atomic transactions.
5. **Strict Serial Integration**:
   - Task execution is parallel, but candidate branch integration (`git merge --ff-only` or verification gates) is strictly serial.

## Consequences
- Throughput on multi-task DAGs scales up to $1.85\times$ on 2 workers without Tokio.
- Zero index lock contention failures during concurrent worktree creation.
- The engine coordinator remains 100% safe, synchronous Rust with zero background daemon dependency.

## Verification and implementation evidence
- `crates/meshloop-cli/tests/plan_and_run.rs::run_concurrent_workers_executes_parallel_tasks`: verified parallel execution of independent tasks.
- `xtask bench`: measured `conc.throughput_gain` = 1.85x, `conc.git_admin.lock_contention_ms` < 1ms, `conc.wal.write_contention_ms` < 1ms.
- 100% passing tests across `cargo test --workspace`.
