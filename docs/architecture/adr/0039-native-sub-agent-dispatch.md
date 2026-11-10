# 0039 Native sub-agent dispatch from the origin session

- Status: Proposed
- Implementation: not-started
- Date: 2026-11-10
- Author/executor: Claude (Opus 5.5), builder worker for #146
- Reviewer: none yet; Claude architecture review from a clean context, then the repository owner
- Decision owner: Samuel
- Approval evidence: none. The five owner decisions recorded on goal #145 (2026-11-10) are inputs and are not reopened here; acceptance of this record is still pending
- Supersedes: none. Amends runtime-design.md §3 ("CLI-subprocess only in v1") and execution-lifecycle.md by dated amendment notes. ADR 0003, 0005, 0007, 0025 and 0035 stay in force for subprocess mode
- Superseded by: none
- Related: ADR 0038 (#133, proposed), "Nodecode as learning plane and harness; kernel unchanged". It is reviewed together with this record
- Tracking: [#146](https://github.com/smota/meshloop/issues/146), goal [#145](https://github.com/smota/meshloop/issues/145); implementation in #147 to #153
- Inspected source: `45ef6a4`

## Context and constraints

Every worker today is a CLI process that Meshloop starts and owns (`HarnessCapabilities`
`invoke`/`try_collect`/`cancel_owned`, ports.rs:51-92; ADR 0022 direct CLI; ADR 0025 and
0035 process-tree ownership). An operator working inside a client that has native
sub-agents (Claude Code Task tool, Agy `invoke_subagent`) pays for a second process,
login context and cold start when a node routes to that same harness. Only the client's
own model can start a native sub-agent, during its own turn. The engine cannot call it.
Control has to invert for those attempts: the session pulls work, starts the sub-agent and
reports back. Meshloop keeps the graph, the gates, verification and integration.

Engine facts at `45ef6a4` that size the work (this confirms and corrects the estimate on #145):

| Fact | Evidence | Effect on sizing |
|---|---|---|
| The tick is already non-blocking: dispatch returns `Running` and later ticks poll | run_loop.rs:1560 (`Ok(None)` arm of `dispatch_ready`), 1055-1144 (`reattach_running` polls `try_collect` at 1105) | Confirmed: no loop restructuring; parking is a new state plus arms |
| `orchestrator.rs` is not on the live dispatch path | `dispatch_with_fallback` (orchestrator.rs:35) is called only by its own tests (161, 192, 216); live fallback is `maybe_fallback` (run_loop.rs:2151) | Correction: no change to `orchestrator.rs` |
| A `Running` attempt with no PID is treated as dead and failed on resume | `attempt_driver_dead` returns true for `pid: None` (run_loop.rs:2300); `crash_fail` appends `HarnessCrashedOrTimeout` "resume: no live process" (2322-2331) | Correction: a parked attempt cannot reuse `Running`; a new task state is needed in domain, store and every projection |
| The foreground loop spins on live workers | `loop_until_idle` sleeps 20 ms and re-ticks on `WaitingOnLiveWorker` (run_loop.rs:2218) | New idle reasons so a command returns instead of spinning until a deadline |
| Heavy repair rounds block inside the tick | `dispatch_repair_round` calls `invoke` then blocking `collect` (run_loop.rs:1670-1675) | Correction: ADR 0026 in-attempt heavy repair is not available to native attempts |
| A verifying attempt with no in-memory outcome is already verified from the worktree alone | `verify_deferred` falls through to `recover_verifying` / `finish_orphaned_verification` (run_loop.rs:1797-1850, 2340-2474) | Reusable for native verification after a report |
| No per-node timeout exists | `TaskNode` has no timeout field (task_graph.rs:50-73); the timeout is run-wide `[limits] task_timeout_seconds` (config.rs:50) carried as `RunLimits::task_timeout` (run_loop.rs:85, compose.rs:265) | Correction: owner decision 4 means this per-attempt value, which must be persisted at park |
| The origin is not persisted | `Origin` is a per-invocation struct (origin.rs:6-9); the `attempts` table has no origin, mode or deadline column (workspace/schema.rs:42-55) | Confirmed: store schema change with forward-only migration (ADR 0033 §7) |
| Unknown states and events fail closed in older binaries | `parse_state` / `parse_event` return `Corrupt` on unknown names (store.rs:199-237) | A new state makes older binaries refuse the ledger rather than misread it |
| Nodes have no role; run-loop nodes are `meshloop:worker` leaves | `TaskNode` (task_graph.rs:50-73); reviewers are `meshloop:reviewer` leaves launched by `meshloop orchestrate` (orchestrate.rs:1-3) | "Builder nodes only" means run-loop worker attempts; no role field is needed |
| MCP tools already wrap the CLI | `dispatch_tool` spawns the binary with `--json` and origin flags (mcp.rs:202-235) | Confirmed: CLI is the contract (owner decision 2) |
| No live guard keeps a worker out of the origin harness | `Origin::blocks_worker` (origin.rs:33) is referenced only by its own test | No existing guard conflicts; the origin itself still never becomes a worker |
| The run loop mutates only unbound (legacy) runs | `require_legacy_mutation` (run_loop.rs:356-372) | Native mode follows the same producer path; it adds no separate path for bound runs |

Effort L for #147 stands. Its surface is wider than the goal's table: domain
(`state.rs` transition table), store parse and migration, engine run loop, recovery and
both status projections (legacy `status_snapshot`, run_loop.rs:2894, and the bound-run
read frame in `workspace/read.rs`).

Constraints that are not reopened: kernel authority (the event log is the only source of
truth), observed evidence only, explicit human acceptance through
`meshloop accept --task <id> --as <identity>`, process-tree ownership for subprocess
mode, ADR 0003 (no shipped per-harness flags), and socket-free offline CI.

## Alternatives

1. **Subprocess only (current).** Keeps every ownership and evidence guarantee. Pays the
   duplicate process and cold start. Remains the default.
2. **Reuse `Running` with a dispatch-mode column.** Smaller diff. Rejected: a PID-less
   `Running` attempt is "dead" (run_loop.rs:2300, 2322), so any missed branch, and any
   older binary, silently crash-fails a live native attempt. A distinct state fails closed.
3. **The driver parks; `next` only claims.** Rejected: the deadline would run while no
   session has pulled the work, and a detached driver with no session would park work that
   nobody can execute.
4. **Trust the report's verdict.** Rejected: contradicts ADR 0003 (worktree diff is the
   deliverable) and ADR 0007 (deterministic evidence).
5. **Session heartbeat as liveness.** Rejected: a heartbeat is one more claim from the same
   session and observes nothing. The deadline is the only bound Meshloop can enforce.
6. **MCP tools as the contract.** Rejected by owner decision 2; the CLI is the contract,
   consistent with ADR 0017 and the existing `mcp.rs` wrapping.
7. **Engine calls a vendor sub-agent API directly.** Rejected: that is another
   Meshloop-owned worker with its own credentials, not the client's native sub-agent, and
   it saves none of the duplicate context.
8. **Native reviewers now.** Deferred by owner decision 5. A reviewer's deliverable is its
   verdict text, so a native review would be a claim end to end, with no diff or check for
   Meshloop to observe.
9. **Another session may adopt a parked attempt.** Rejected for now: the origin binding is
   the only identity check on a report. Adoption would need its own decision.

## Decision or proposal

Proposed. Add a second dispatch mode, **native**, beside **subprocess**. Mechanism detail
is in runtime-design.md §3 "Native dispatch (ADR 0039)" and execution-lifecycle.md
"Amendment 2026-11-10".

1. **Parked attempt state and events.** New task state `awaiting-report`
   (`TaskState::AwaitingReport`). Events:
   - `AttemptParked`: `ready → awaiting-report`. The attempt id, worktree, branch, base
     revision and prompt file are created as for a subprocess attempt. The attempt
     persists dispatch mode, origin harness and session, requested harness and model,
     `parked_at` and `deadline_at`.
   - `NativeReportReceived`: `awaiting-report → verifying`. Stored late on
     `failed`/`cancelled` without changing state, like a late `HarnessExited`.
   - `ParkDeadlinePassed`: `awaiting-report → failed`. Distinct from
     `HarnessCrashedOrTimeout`, so "no report" is never recorded as a process fault.
   - `Cancel`: `awaiting-report → cancelled` through the existing generic arm.
   - `DispatchModeChanged`: `ready → ready`, with a mandatory reason. It is appended before
     dispatch whenever a task's new attempt uses a different mode from its previous
     attempt, in either direction.
   A parked attempt holds a concurrency slot like `running` (ADR 0024).
2. **Pull and report surface.** CLI commands are the contract. MCP tools `meshloop_next`
   and `meshloop_report` wrap them.
   - `meshloop next [--graph <id>] --json` drives the loop in the foreground until it is
     idle or waits only on native work. It then parks every ready node whose routed
     harness is native-eligible, within the concurrency budget. It returns one item per
     parked attempt for this origin: task id, attempt id, role (`meshloop:worker`), prompt
     path, worktree path, base revision, harness, requested model and deadline. Attempts
     already parked for the same origin and not yet reported are returned again, so the
     call is idempotent. It never re-parks them. With nothing to hand out it returns the
     idle reason.
   - `meshloop report --task <id> --attempt <id> [--note <text> | --note-file <path>]
     [--model <reported>] --json` records that the sub-agent finished. It takes no pass
     or fail field. The note and the reported model are stored redacted as data, within
     the existing 2048-byte outcome limit.
   - Both require `--origin-harness` and `--origin-session` (or the environment
     variables). A report whose origin differs from the parked attempt's origin is refused
     and appends nothing. Neither command authorizes acceptance or integration.
   - `run`, `resume` and a detached driver never park. They leave native-eligible nodes
     `ready`. They return the new idle reason `AwaitingNativePull`, or
     `AwaitingNativeReport` when only parked attempts remain.
3. **Evidence rule.** On a report, Meshloop computes the outcome itself from observed
   evidence only: the worktree diff against the recorded base revision, `allowed_paths`,
   an out-of-worktree check, and the configured verify command. Then the normal gate path
   runs unchanged. The out-of-worktree check compares HEAD and porcelain status of the
   origin checkout and the integrate worktree with the values recorded at park; any
   difference fails the attempt and lists the paths. The note, the reported model and any
   claim of success are data. They are never parsed for a verdict and never count toward
   any pass condition. Every evidence row and the attempt record name the dispatch mode.
   An empty diff is the existing no-change case (`empty_diff_ok`). #149 implements this
   rule.
4. **Deadline, cancellation and liveness.**
   - Deadline: `deadline_at = parked_at + [limits] task_timeout_seconds`, persisted at
     park (owner decision 4). A command that ticks the graph (`next`, `run`,
     `resume`), and `report` for its own attempt, appends `ParkDeadlinePassed` for an overdue attempt before it does
     anything else. The existing retry budget and routing then decide a new attempt.
     `status` shows overdue but appends nothing (ADR 0034). A report after the deadline is
     stored as a late event and is never verified.
   - Cancellation: `meshloop cancel` on an `awaiting-report` task appends `Cancel` and
     stops tracking. It does not stop the sub-agent. There is no PID, no recorded-PID
     fallback and no `cancel_owned` call. The command output names the attempt and origin
     so the operator skill can stop the sub-agent in the session.
   - Liveness: none is observed. A dead session and a slow sub-agent look the same until
     the deadline. Recovery leaves `awaiting-report` untouched except for the deadline;
     `crash_fail` and `reattach_running` never see it. `resume --restart` refuses while an
     attempt is `awaiting-report`.
   - Cleanup: a cancelled or expired native attempt's worktree has uncertain liveness, and
     ADR 0033 blocks cleanup of it.
5. **Status.** For a native attempt, `status` and `inspect` (text and `--json`) show:
   `dispatch_mode: native`, state `awaiting-report`, `waiting_for: native_report`, origin
   harness and session, `parked_at`, `deadline_at`, overdue when past it, report pending,
   received or late, requested and reported model, and a fixed `limits` list:
   `no_owned_process`, `no_cancel_proof`, `no_liveness`, `no_detached_execution` and
   `inherits_session_permissions`. `live` is `NotObservable`, never `Live` or `Dead`.
6. **Mode policy.** Per-harness operator config
   `[harnesses.<name>] dispatch = "subprocess" | "native"`, default `subprocess` (owner
   decision 1). Mode is chosen after routing and never changes which harness or model is
   routed. A node is native only when all of these hold: the routed harness is set to
   `native`, the harness's `kind` equals the invocation's origin harness, an origin
   session is present, the attempt is a run-loop worker attempt, and the command is
   `next`. In every other case the node uses subprocess dispatch. A mode change between
   attempts of the same task is a `DispatchModeChanged` event. `doctor` and
   `dispatch_warnings` report a `native` setting that can never apply. Routing feedback
   is keyed by dispatch mode as well as harness, model and tier. A native outcome whose
   reported model is missing or differs from the requested model is recorded under model
   `unverified`. Feedback may recommend a mode; it cannot change the policy.
7. **Reviewers stay subprocess.** `meshloop:reviewer` leaves (`meshloop orchestrate`), the
   planner and model-review producers are never native (owner decision 5). Only run-loop
   `meshloop:worker` attempts may be. Native reviewers need a successor decision.
8. **Learning plane (ADR 0038, #133, proposed).** Only observed evidence counts: exit
   codes and output of checks Meshloop runs, files and diffs it reads, commits it makes.
   Worker self-report (the note, the reported model, claimed success) is data. It may be
   displayed but is never a learning, ranking or routing signal. Native and subprocess
   series stay apart. Pooling them requires a separate human-accepted decision. Native
   evidence must land with its mode marker before #139 consumes it.
9. **Unchanged.** The origin stays `meshloop:origin`, which is never a worker; the native
   sub-agent is the worker leaf. Human acceptance stays explicit
   `meshloop accept --task <id> --as <identity>`, and ADR 0037 verify gates are
   unchanged. Subprocess mode keeps ADR 0025 and 0035 ownership. Meshloop ships no
   per-harness native flags; harness wording lives in `skills/meshloop-*` (#151).

### Owner decisions on goal #145 (recorded, not reopened)

| # | Question | Answer in this record |
|---|---|---|
| 1 | Default for the native switch | `dispatch = "subprocess"`; native is opt-in per harness (decision 6) |
| 2 | Surface | `meshloop next` / `meshloop report` CLI as contract; `meshloop_next` / `meshloop_report` MCP tools wrap them (decision 2) |
| 3 | First harnesses | Claude Code and Agy; Codex, Grok and Pi recorded as supported or unsupported by the #152 live proof. Nothing here is harness-specific (decision 9) |
| 4 | Deadline | `parked_at + [limits] task_timeout_seconds`, persisted at park; no per-node timeout exists today (decision 4) |
| 5 | Native reviewers | No; run-loop worker attempts only (decision 7) |

## Consequences

**Lost against subprocess mode, for native attempts.** Each item is a real loss.
- **No owned process.** There is no kill-on-close job, no process group, no
  `cancel_owned` proof and no termination observation. Cancel and deadline stop tracking
  only. A cancelled or expired sub-agent can keep writing to its worktree and anywhere
  else the session can write.
- **No liveness.** There is no PID, pane or heartbeat. A crashed session, a closed client
  or a compacted context looks like a slow sub-agent until the deadline. At worst one full
  `task_timeout_seconds` is lost per attempt.
- **No detached execution and no adoption.** The sub-agent lives and dies with the origin
  session. Another session cannot report for it. The graph advances only while the session
  keeps calling `next`; `run --detach` does not drive native nodes.
- **No exit code or captured output.** The only "output" is a claimed note.
- **No model proof.** The model is whatever the session says. A harness may ignore or
  substitute a per-call model, and Meshloop cannot tell.
- **Permissions are the session's.** The sub-agent inherits the session's permission mode,
  tools, MCP servers, credentials and environment. Meshloop chooses no argv, environment
  or permission flags. Confinement to the worktree is by instruction only. Whether a
  harness can confine a native sub-agent to the worktree at all is unproven until #152.
- **Partial out-of-worktree detection.** Only the Git checkouts Meshloop knows are
  checked. Writes elsewhere (home directory, other repositories, temp files) go undetected.
  Concurrent operator edits in the origin checkout fail the attempt, a false positive by
  design.
- **No in-attempt heavy repair (ADR 0026).** A failed check fails the attempt. A retry is a
  new attempt, native again by default; a mode change is an event. This narrows #149's
  "repair rounds are native again": that applies to retries, not to session-resumed rounds
  inside one attempt.
- **Cost is invisible.** The sub-agent's tokens, time and quota are spent in the origin
  session. Meshloop does not meter them, and the sub-agent's result consumes the origin's
  context.
- **Parallelism is the client's.** `max_concurrent_workers` caps hand-out only.
- **Mixed graphs serialize the session.** `next` waits on subprocess workers it drives,
  bounded by their timeout.
- **Cleanup blocks.** Worktrees of cancelled or expired native attempts stay
  liveness-uncertain and block ADR 0033 cleanup until an operator resolves them; #142 must
  account for this.
- **No live test in CI.** Offline CI covers the state machine, store and surfaces with
  fixtures only. Native behaviour is proven live per harness (#152), not in CI.

**Compatibility.** The new state and events need a forward-only schema migration
(attempt columns for mode, origin, deadline, reported model). Older binaries refuse the
ledger (`parse_state`/`parse_event` return `Corrupt`) rather than misreading it.
Subprocess behaviour and its tests are unchanged.

**Security and privacy.** The origin session id is an opaque identifier stored in the
local ledger. It is never written to telemetry, evidence exports or logs beyond what
`status` already shows. Prompt text is returned by path, not inline. The note is redacted
before storage.

**Stop conditions (from #145).** If this record is declined, the goal closes. If #152
shows no selected harness can start a native sub-agent in an assigned worktree with a
chosen model, this record stays as a recorded limitation. If #153 shows no gain, the
default stays `subprocess`.

## Verification and implementation evidence

None yet; this is a proposal. Planned evidence: domain transition-table tests for every
new arm and late-event case; engine tests for park, report, deadline, cancel, wrong-origin
refusal, resume after a crash between park and report, and a report that claims success
while checks fail (#147, #149); CLI and MCP contract tests (#148); policy tests for switch
off, matching and non-matching origin, reviewer refusal and mode-change events (#150);
`cargo test --workspace --locked --offline` socket-free; live proof per harness (#152);
benchmark in both modes on the same graph (#153).
