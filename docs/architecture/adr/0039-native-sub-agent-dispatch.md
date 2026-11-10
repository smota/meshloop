# 0039 Native sub-agent dispatch from the origin session

- Status: Accepted
- Accepted date: 2026-11-10
- Implementation: not-started
- Date: 2026-11-10
- Author/executor: Claude (Opus 5.5), builder worker for #146
- Reviewer: fresh-context Claude architecture review (opus), jointly with ADR 0038: round 1 ACCEPT WITH CHANGES (all adopted and applied), round 2 ACCEPT WITH CHANGES (N1 and minors applied, verified by the coordinator)
- Decision owner: Samuel
- Approval evidence: accepted 2026-11-10 by the coordinator (Claude) under authority delegated by the repository owner, who waived human approvals ([#132 comment](https://github.com/smota/meshloop/issues/132#issuecomment-6770755409)); no human read this record before acceptance. The five owner decisions on goal #145 (2026-11-10) are inputs. ADR 0039 decision 6 is an architect decision under delegated owner authority (2026-11-10)
- Supersedes: none. Narrows the scope of [ADR 0003](0003-harness-contracts.md) (dispatch through `HarnessCapabilities`), [0005](0005-execution-recovery.md) (process reconciliation), [0025](0025-process-tree-ownership.md) and [0035](0035-atomic-process-job-assignment.md) to subprocess attempts. [ADR 0007](0007-verification.md) applies to every mode unchanged
- Superseded by: none
- Related: [ADR 0038](0038-nodecode-learning-plane-and-harness.md) (#133), "Nodecode as learning plane and harness; kernel authority unchanged"; reviewed together with this record
- Tracking: [#146](https://github.com/smota/meshloop/issues/146), goal [#145](https://github.com/smota/meshloop/issues/145); implementation #147 to #153
- Inspected source: `45ef6a4`

## Context and constraints

Every worker is a CLI process Meshloop starts and owns (ADR 0003, 0022, 0025, 0035). An
operator inside a client with native sub-agents (Claude Code Task tool, Agy
`invoke_subagent`) pays for a second process, login context and cold start when a node
routes to that same harness. Only the client's own model can start a native sub-agent,
in its own turn, so control inverts: the session pulls work, starts the sub-agent and
reports back. Meshloop keeps the graph, gates, verification and integration.

The engine sizing on #145 was checked against the code; evidence with file and line is
in runtime-design.md §3 "Sizing at 45ef6a4". Effort L for #147 stands, with a wider
surface than the goal listed.

Constraints that are not reopened: kernel authority, observed evidence only, explicit
`meshloop accept --task <id> --as <identity>`, process-tree ownership for subprocess
mode, ADR 0003's rule that no per-harness flags ship, and socket-free offline CI.

## Alternatives

1. **Subprocess only (current).** Keeps every guarantee and pays the duplicate process.
   Remains the default.
2. **Reuse `running` with a mode column.** A PID-less `running` attempt is failed on
   resume, so a missed branch or an older binary silently fails live work. Rejected.
3. **The driver parks and `next` only claims.** The deadline would run while no session
   holds the work. Rejected.
4. **Trust the report's verdict.** Contradicts ADR 0003 and 0007. Rejected.
5. **Session heartbeat.** It is a claim that observes nothing. Rejected.
6. **MCP as the contract.** Rejected by owner decision 2.
7. **Engine calls a vendor sub-agent API.** That is another owned worker. Rejected.
8. **Native reviewers.** The verdict text would be a claim from end to end. Deferred by
   owner decision 5.
9. **Another session adopts a parked attempt.** Rejected for now; the origin binding is
   the only check on a report.

## Decision or proposal

Add a **native** dispatch mode beside **subprocess**. The mechanism is in
runtime-design.md §3 "Native dispatch (ADR 0039)" and execution-lifecycle.md
"Amendment 2026-11-10".

1. **State and events.** A new task state, `awaiting-report`, holds a concurrency slot.
   - Transitions: `AttemptParked` (ready → awaiting-report), `NativeReportReceived`
     (awaiting-report → verifying), `ParkDeadlinePassed` (awaiting-report → failed) and
     `Cancel`.
   - `DispatchModeChanged` is a ready → ready event with a mandatory reason, appended
     whenever a task's new attempt changes mode.
   - `NativeStopAttested` is a per-attempt self-transition on failed, cancelled, accepted
     or integrated; see decision 4.
2. **Eligibility and surface.**
   - **Eligibility.** A node is native-eligible when every condition holds:
     - its routed harness has `dispatch = "native"`;
     - the harness `kind` is set and equals the origin harness;
     - an origin session is set;
     - it is a run-loop worker attempt;
     - the `kind` is in the set proven by #152.

     The proven set is a compiled-in capability table with evidence links, changed only by
     PR; it is not configurable.
   - **`doctor`** refuses a `native` setting for an unproven or unset `kind`, and warns
     when the setting can never apply.
   - **`meshloop next`** is the only command that parks eligible nodes. `run`, `resume`
     and detached drivers leave them `ready`. `next` never starts subprocess workers and
     returns `AwaitingSubprocessDriver` when only subprocess-routed ready nodes remain.
     `--wait-seconds` bounds how long it runs; a verification already running can overrun
     the bound by up to the check timeout.
   - **`meshloop report`** records that an attempt finished. It takes no verdict. It only
     appends, so it is allowed while another driver is live.
   - Both commands are origin-bound. They are the contract; the MCP tools `meshloop_next`
     and `meshloop_report` wrap them.
3. **Evidence rule.** Pass or fail comes only from observation:
   - an out-of-worktree check against the expected state;
   - a snapshot commit, whose SHA becomes the attempt's verified revision;
   - the diff against the recorded base, `allowed_paths`, and the configured checks;
   - a re-read of HEAD and status after the checks, where any change fails the attempt.

   The note, the reported model and any claim of success are data. They are never parsed
   and never count toward a pass. `accept`, `merge_accepted` and `prepare_deliverable`
   use the verified revision for a native attempt and refuse (`StaleBaseDetected` at
   merge) when the branch tip differs. This is mandatory for native attempts and
   recommended, not required here, for subprocess attempts.
4. **Deadline, cancellation, liveness and cleanup.**
   - **Deadline.** `deadline_at = parked_at + [limits] task_timeout_seconds`, persisted at
     park. It is enforced first by any command that touches the attempt. `status` only
     displays it (ADR 0034).
   - **Cancel.** Stops tracking. It does not stop the sub-agent: there is no PID fallback
     and no `cancel_owned`.
   - **Liveness.** None is observed.
   - **Restart.** `resume --restart` refuses while an attempt is `awaiting-report`.
   - **Cleanup.** Every native attempt's worktree is liveness-uncertain. Cleanup proceeds
     only after `meshloop cancel --task <id> --attempt <id> --attest-stopped --as <identity>`
     (`NativeStopAttested`),
     or after the verified revision is landed and the worktree still equals it at cleanup
     preview. This is a dependency on #142's cleanup eligibility.
5. **Status.** `status` and `inspect` show, for each native attempt, its mode, origin,
   deadline, report state, requested and reported model, and verified revision. They show
   `live: NotObservable` and the fixed list of native limits.
6. **No in-attempt repair; deviation from #149.** Native attempts get no in-attempt
   repair, heavy or micro; the orphan-verification path runs none. Native repair rounds as
   a re-park of the same attempt are deferred to a successor decision. #149's requirement
   is amended to read "retries". A retry is native again unless routing or policy says
   otherwise, and a mode change is a `DispatchModeChanged` event. (Recorded architect
   decision under delegated owner authority, 2026-11-10.)
7. **Feedback and learning ([ADR 0038](0038-nodecode-learning-plane-and-harness.md), #133).**
   - **Routing feedback.** The feedback key gains the dispatch mode. Every native outcome
     is recorded under model `unverified`, with the requested model stored beside it for
     display. This lasts until a harness offers a model observation that Meshloop reads
     itself, which needs an amendment.
   - **Learning plane.** Only observed evidence counts: check exit codes and output,
     diffs, and commits Meshloop makes. Worker self-report is data. Native and subprocess
     series stay apart. ADR 0038 priors ignore `unverified` entries. Feedback may
     recommend a mode but cannot change policy.
8. **Reviewers stay subprocess.** `meshloop:reviewer`, the planner and model review are
   never native (owner decision 5).
9. **Unchanged.** The origin stays `meshloop:origin` and never becomes a worker. Human
   acceptance and ADR 0037 gates are unchanged. Subprocess mode keeps ADR 0025 and 0035.
   Harness-specific wording lives in `skills/meshloop-*` (#151).

### Owner decisions on goal #145 (recorded, not reopened)

| # | Question | Answer in this record |
|---|---|---|
| 1 | Default for the native switch | `dispatch = "subprocess"`; native is opt-in per harness and honoured only for kinds proven by #152 |
| 2 | Surface | `meshloop next` / `meshloop report` CLI as the contract; `meshloop_next` / `meshloop_report` MCP tools wrap them |
| 3 | First harnesses | Claude Code and Agy; Codex, Grok and Pi recorded by #152 as supported or unsupported |
| 4 | Deadline | `parked_at + [limits] task_timeout_seconds`, persisted at park; no per-node timeout exists |
| 5 | Native reviewers | No; run-loop worker attempts only |

## Consequences

**Lost against subprocess mode, for native attempts.**
- **No owned process.** There is no kill-on-close job, process group, `cancel_owned` proof
  or termination observation. A cancelled or expired sub-agent can keep writing.
- **No liveness.** A crashed session, closed client or compacted context looks like a slow
  sub-agent until the deadline. That can cost up to one `task_timeout_seconds` per
  attempt.
- **No detached execution and no adoption.** The graph's native nodes advance only while
  the session calls `next`.
- **No exit code, captured output or model proof.** The model is a claim, so feedback
  says `unverified`.
- **Session permissions.** The sub-agent inherits the session's permissions, tools, MCP
  servers, credentials and environment. Worktree confinement is by instruction only and
  unproven until #152.
- **Partial out-of-worktree detection.** Undetected writes include:
  - the home directory, other repositories and temporary files;
  - sibling attempt worktrees, which are attributed to that sibling;
  - untracked files in the origin checkout.

  Operator edits to tracked files in the origin checkout fail the attempt, a false
  positive by design.
- **No in-attempt repair** (decision 6).
- **Invisible cost.** Tokens, time and quota are spent in the origin session, unmetered.
- **Client-bounded parallelism.** `max_concurrent_workers` caps hand-out only.
- **Two drivers for mixed graphs.** Native nodes go through `next`, subprocess nodes
  through `run` or `resume`, and the two must not tick one graph at the same time.
- **Manual cleanup.** Cleanup waits for attestation or landing.
- **No live test in CI.** Offline CI covers state, store and surfaces with fixtures;
  native behaviour is proven live in #152.

**Trust.** The origin binding (`--origin-harness`, `--origin-session`) is an
unauthenticated claim, like `--as`. A forged report can only trigger verification early.
That is safe because verification is observed and integration uses the verified revision.

**Compatibility.** A forward-only migration adds attempt columns (mode, origin,
deadline, reported model, verified revision), a `routing_feedback` mode column and ledger
records of integrate HEADs. Older binaries refuse the new names. Subprocess behaviour is
unchanged.

**Privacy.** The origin session id stays in the local ledger and is never written to
telemetry or exports. Prompts are passed by path. Notes are redacted.

**Stop conditions (#145).**
- If this record is declined, the goal closes.
- If #152 proves no harness, the record stays as a recorded limitation.
- If #153 shows no gain, the default stays `subprocess`.

## Verification and implementation evidence

None yet; this is a proposal. The planned scenarios are listed in runtime-design.md §3
"Native dispatch (ADR 0039)". They include the named test "commit after report is not
integrated". All of them run under `cargo test --workspace --locked --offline`. Live
proof is #152 and the benchmark is #153.
