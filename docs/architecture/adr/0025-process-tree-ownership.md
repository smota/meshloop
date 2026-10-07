# 0025 Host process-tree ownership via Windows Job Objects

- Status: Accepted
- Implementation: implemented
- Date: 2026-09-15
- Author/executor: Antigravity
- Reviewer: Grok CLI (Architecture Reviewer)
- Approval evidence: Ratified in 3-PR Implementation Plan (De Acordo)
- Supersedes: none
- Superseded by: none

## Context and constraints
CLI harnesses (Codex, Claude Code, Pi, Agy, Grok) and deterministic checks (`cargo check`, `npm test`) routinely spawn child and grandchild processes (`rustc.exe`, `cargo.exe`, `node.exe`, `esbuild.exe`). When a run times out, is aborted by user cancellation, or is terminated due to an invariant violation, calling standard Rust `Child::kill()` terminates only the parent process.

Grandchild processes are orphaned:
- They continue consuming CPU and memory.
- On Windows, they keep open file handles to binaries and `.git/index.lock`, preventing worktree cleanup (`git worktree remove`) and causing subsequent runs to fail with access denied errors (`ERROR_SHARING_VIOLATION`).
- In extreme cases, orphaned build processes compile stale artifacts or mutate shared caches.

Constraints:
- Complete eradication: zero orphaned processes (`conc.orphan_process_count = 0`).
- Cross-platform semantics: robust behavior on native Windows hosts and POSIX systems.
- Crash resilience: process cleanup must occur even if the orchestrator process crashes abruptly.

## Alternatives
1. **Rely on standard `std::process::Child::kill()`**: Rejected: leaves grandchildren alive on both Windows and POSIX.
2. **PID file tracking**: Write PIDs to a ledger and kill them sequentially. Rejected: racily misses rapidly spawned sub-processes.
3. **Windows Process-Tree Kill (`taskkill /F /T`) and Job Objects (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`)**: Chosen. On Windows, child processes are terminated with recursive tree termination (`taskkill /F /T /PID`), complemented by kernel-managed Job Objects with `KILL_ON_JOB_CLOSE`. On POSIX, process groups (`setpgid` and `kill(-pgid, SIGKILL)`) are used.

## Decision or proposal
1. **Tree Termination in `meshloop-adapters::process`**:
   - Provide `kill_process_tree(pid: u32) -> Result<(), ProcessError>`:
     - Windows: Executes `taskkill /F /T /PID <pid>`, recursively eliminating the root process and all descendants.
     - POSIX: Sends `SIGKILL` to the negative process group ID `-pid`.
2. **Integration into Harness & Runner Lifecycle**:
   - In `CliHarness::try_collect` and `CliHarness::invoke`: on timeout or error, invoke `kill_process_tree(child.id())` before reaping the parent.
   - In `CheckRunner::run`: on execution timeout, invoke `kill_process_tree(child.id())`.
3. **Windows Job Object Kernel Limit**:
   - Assign subprocesses to an anonymous Win32 Job Object configured with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` where supported, ensuring that even unexpected orchestrator termination triggers immediate OS-level cleanup.

## Consequences
- Guarantees `conc.orphan_process_count == 0` across all test suites and production executions.
- Worktrees can be removed immediately after task completion without encountering file locking conflicts.
- Zero residual background processes after test suite abort or Ctrl+C interruption.

## Verification and implementation evidence
- `crates/meshloop-adapters/src/process/job.rs`: Win32 Job Object abstraction (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`) and POSIX `setpgid(0, 0)` via `spawn_owned`.
- `crates/meshloop-adapters/tests/process_tree.rs`:
  - `grandchild_dies_on_timeout`: confirms grandchild termination on timeout/kill_tree.
  - `grandchild_dies_on_parent_abort` (Windows only): confirms OS kernel terminates grandchildren when parent abruptly aborts (`std::process::abort()`). POSIX: see the 2026-10-07 recovery amendment.
- `xtask bench`: metric `conc.orphan_process_count` reports `0` (PASS).
- 100% passing tests in `cargo test -p meshloop-adapters` (24 passed) and `cargo test --workspace` (131 passed).

## Amendment 2026-10-07: POSIX orphans are reclaimed during recovery

- Status: Accepted
- Approval evidence: Samuel (human) chose option 1 ("recovery reconciliation") for issue #30 on 2026-10-07, in the AgentFlow/Meshloop setup session.
- Executor: Claude Code (Opus 5.5)

**Context.** The guarantee that grandchildren die when Meshloop aborts holds only on Windows, where the Job Object closes with the process. A POSIX process group is torn down only when Meshloop calls `kill_tree`. If Meshloop itself aborts or is SIGKILLed, the harness group keeps running; on WSL2 Ubuntu 24.04 the abort fixture left two processes alive for more than 11 minutes. `grandchild_dies_on_parent_abort` is therefore limited to Windows.

**Decision.** Reclaim orphans during recovery, in safe Rust. When `RunLoop` reattaches a `Running` attempt that the current harness instance does not track, it calls `HarnessCapabilities::cancel` before recording `HarnessCrashedOrTimeout` and retrying. For an untracked attempt with a recorded pid, `CliHarness::cancel` calls `process::kill_orphaned_tree`. That function kills the tree only when the pid is alive and runs the harness executable (image name on Windows; `comm` on POSIX, which Linux truncates to 15 bytes), and, on POSIX, still leads its own process group. This guards against pid reuse.

**Alternatives rejected.** `prctl(PR_SET_PDEATHSIG)` is Linux-only, covers only the direct child, and needs `unsafe`. A subreaper supervisor process adds moving parts.

**Consequences.** Orphans keep running between the crash and the next `resume` or `status` of that graph; they are not reclaimed immediately. A harness launched through a shim whose process image differs from the configured executable is not matched, so it is left alone rather than risk killing an unrelated process.

**Verification.** `crates/meshloop-adapters/tests/harness_contract.rs`: `cancel_kills_an_orphaned_tree_left_by_a_previous_process` fails without the untracked-pid branch ("orphaned leader survived cancel") and passes with it. `cancel_leaves_an_unrelated_process_with_a_recorded_pid_alone` covers the pid-reuse guard.
## Amendment 2026-10-07: detached spawns do not inherit caller handles

- Status: Accepted
- Approval evidence: accepted by Samuel (human) on 2026-10-07 in the AgentFlow/Meshloop setup session, as the decision requested on issue #5.
- Executor: Claude Code (Opus 5.5)

**Context.** `meshloop run --detach` must return to its caller promptly (#5: session ID in < 500 ms). On Windows, `CreateProcess` with handle inheritance gives the detached `resume` worker every inheritable handle of the CLI, including the stdout/stderr pipes of a caller that captures output. The worker's own stdio being null does not prevent this. Callers using `Command::output()` (tests, AgentFlow's CLI adapter) therefore blocked until the background session finished: the CLI exited in 0.18-0.59 s, but stdout reached EOF only after 2.3-2.8 s locally, and after more than 5 s on hosted CI.

**Decision.** `meshloop_adapters::process::spawn_detached` owns detached spawning. On Windows it clears `HANDLE_FLAG_INHERIT` on the caller's three standard handles for the duration of the spawn, starts the worker with `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP` and null stdio, and then restores the flags. This adds `unsafe` Win32 calls (`GetStdHandle`, `GetHandleInformation`, `SetHandleInformation`) to `meshloop-adapters`, next to the existing Job Object code, with `SAFETY` comments; the CLI keeps `deny(unsafe_code)`. On POSIX, `exec` passes on only the fds that were explicitly set up, so no handle change is needed; the worker gets its own process group (`process_group(0)`).

**Alternatives.** `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` would restrict inheritance precisely, but std's `Command` cannot pass process attributes on stable Rust, so this would mean reimplementing process creation. A trampoline process would inherit the same handles. Leaving it as is breaks the detach contract.

**Consequences.** Only the three standard handles are covered. Other inheritable handles that the CLI itself inherited from its parent are still passed on, which is acceptable because the CLI opens none of its own as inheritable. Concurrent spawns during the window are unaffected, because std duplicates inherited stdio into fresh inheritable handles.

**Verification.** `crates/meshloop-adapters/tests/process_tree.rs::detached_spawn_does_not_hold_the_callers_output_pipe` launches a fixture that detaches a 3 s sleeper and asserts that capturing the fixture's output returns in under 2 s. With the inherit-clearing disabled it fails after 3.25 s; with it, it passes.
