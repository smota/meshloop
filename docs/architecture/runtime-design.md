# Runtime design reference

This holds the concrete mechanism ("how") behind the five proposed runtime ADRs. ADRs
record the decision, its alternatives, and its consequences; this file records the detail,
so ADR bodies stay short and this file stays the one place to look for "how exactly."
Update this file when implementation reveals a detail was wrong or incomplete, without a
new ADR — unless the change alters the decision itself, in which case it goes through the
owning ADR as a successor, per docs/architecture/adr/README.md.

The full mechanism below is the v1 *target*. Release 1 implements the subset in
ADR 0016 (native Windows, one sequential writer, live Herdr workers, fixture as
CI double, `meshloop:review-plan` as the plan gate). Residuals (WSL2, concurrency
> 1, packaging, queried quota) are tracked as GitHub `limitation` issues. Do not read
unverified paragraphs as product. Operator path: [Getting started](../start.md).

### Bounded S3 public observation milestone (issue #74)

For lifecycle-bound runs, status, inspect and watch capture the active generation and
immutable current plan revision once per command. Events, attempts and evidence are
selected within that generation; earlier revisions within it remain part of replay,
validated through the exact activation revision and predecessor chain (maximum 4096
revisions). Inconsistent bindings or revision linkage refuse observation. Only runs
without a lifecycle binding retain the legacy read path. Bound output labels generation
and revision and uses root aliases plus relative locations; it never resolves external
roots for display. Bound Git export is explicitly unavailable until artifact-based
export is implemented.
Bound attempt process liveness remains `Ambiguous` until process-identity observation
is implemented; an attempt completion timestamp does not establish OS process death.
Attempt JSON carries its own generation and plan revision, including retained attempts
from earlier revisions. The exact activation association permits restart to reuse an
immutable revision originating in a prior generation. Revision metadata with no
generation remains valid under the existing projection contract; an explicit conflicting
generation after the activation revision refuses observation.

Each command's selection and frame share one consistent SQLite read transaction;
nested observations reuse that transaction and never close a caller-owned transaction.
Observation ends its own transaction through explicit rollback, propagating failures.
The activation revision and every revision with executed events or attempts must have
historical accepted authorization. Later declines preserve that execution provenance.
Current plan state follows the latest authorization, refreshed per frame: accepted means
`PlanAccepted`; missing or declined authorization means `AwaitingPlanReview`, matching
the existing revision projection contract.

Watch refreshes events in its captured scope. A generation or revision change returns
`ReadScopeChanged` and requires a new observation, so newer-revision events are never
folded against the captured older plan. This milestone does not implement generation
producers, restart preservation, historical CLI selectors, or the complete S3 oracle.
SQLite observation sidecars follow accepted ADR 0034.
Every watch poll opens a fresh read snapshot, allowing committed events and same-revision
authorization changes to appear without mixing records from different database states.

### Bounded RunLoop mutation admission (issue #74)

Graph-targeted `RunLoop` mutation entry points check both lifecycle bindings and
exact `workspace_legacy_unbound` migration markers before filesystem, Git, process
or store effects. A marker for the original run or any persisted attempt refuses
with `LegacyOwnershipUnresolved`; query errors propagate without legacy fallback.
Direct legacy store writes also guard incoming and persisted AttemptIds, including
mixed-graph event batches, so changing a supplied graph cannot bypass protection.
CLI mutation entry points perform a read-only preflight before writer composition
(including gitignore maintenance). Plan adjustment, cancellation and orchestration
admit before planner ownership, process termination, review pack creation or reviewer
invocation. Pre-v7 ledgers return `SchemaUpgradeRequired` from this preflight: an
explicit writer schema migration is required first; mutation commands do not perform
that migration implicitly. Migration does not adopt ownership. Refusals identify
the original graph. `meshloop workspace adopt` pins a verified legacy generation
in place and does not move that run. Unverified, missing, and unknown artifacts
stay reported and untouched. Status, inspect and export remain readable; Git export retains its
existing bound-run restriction and does not reject unresolved legacy markers.
Fresh schema7 unmarked, unbound runs keep the existing legacy path.
This is a snapshot admission check, not a cross-process fence or complete exclusion
of ownership changes after the check. It does not activate generations, implement
successful bound producers, or complete S3. Digest-bound cleanup and verified
adoption are separate workspace commands. Operator steps, blockers, recovery,
retention, and rollback limits are in [the workspace lifecycle runbook](../workspace-lifecycle.md).

### Bounded provisional planner producer (issue #74)

`RunLoop::plan` requires writable `LifecycleAccess`. Both `plan` and
`review-plan --adjust` acquire the repository fence, bind the same ledger, and
retain the lease through dispatch and collection. This covers root and planner
production; initial SQLite opening/creation/migration still precedes acquisition.
The requested CLI `--out` export retains its existing behavior and is outside this
managed attempt ownership journal.
It does not extend the fence to legacy graph execution or other producers.

Each invocation and bounded validation retry creates a fresh opaque worktree/ref.
A provisional worktree owner and a distinct protected output owner permit typed
Worktree, Spawn and Plan intents before effects under the existing exclusive
pending-target rule. No accepted hash, generation or execution binding is created.
Existing bound-run mutation refusal and the acceptance gate remain in force.

The selected existing root is registered without adopting artifacts. Missing root
parents use one retained registry intent containing the full relative suffix before
any directory creation. Reparse ancestors and ambiguous partially created trees
are refused and retained for reconciliation; automatic recovery is not implemented.
New runs use `<repo>/.meshloop/worktrees/<run-id>` after a fail-closed Git layout probe. Existing runs stay on their registered roots. `--worktree-base` and `[workspace] root` select an explicit root and do not relocate an existing run.
Windows canonical drive/UNC prefixes are converted only after identity and
containment validation so native Git can consume equivalent absolute paths.

Planner-written output is preserved in its owned attempt. If no file was produced,
collected stdout is written there before validation. Preexisting tracked output is
refused before dispatch. Rejected plans, failed worktrees and old attempts are never
reset, overwritten or deleted. Operation records contain aliases, relative locations
and stable error codes only; output, source and prompts stay outside durable
telemetry. Rejection diagnostics identify portable locators rather than external
absolute paths. Filesystem failures propagate as stable public errors.

Successful collection completes Spawn; only a validated successful output completes
Plan. Rejected output blocks Plan with a stable classification and may use the
bounded validation retry budget. Invoke/collect failures leave their intents
unresolved and stop retries. Recovery and crash reconciliation remain outstanding;
these records are not proof that an uncertain process tree has stopped. This
milestone is not full S3 acceptance.

Each real provisional planning attempt registers its own immutable driver before
invocation, then retains the repository fence while polling control requests and
explicitly nonblocking harness collection every 100 ms. The process identity is
the current Meshloop owner PID and OS creation identity: Windows creation FILETIME,
or Linux boot ID plus `/proc/self/stat` start ticks. The tree token is the driver's
nonce ownership marker, never a fabricated OS process group. Other platforms, or
adapters without explicit nonblocking collection, expose `driver-control-unavailable`
and preserve existing owned planning without offering cancellation control.
Actual identity-probe errors on supported platforms fail closed before invoke.

`status --drivers` and `inspect --driver <id>` observe the existing ledger without
requiring a graph or acquiring its writer registry fence. They expose portable
driver identity/phase and at most the latest 64 cancellation observations; SQLite
sidecar effects follow ADR 0034. Absent ledgers stay absent. `cancel --driver <id>`
is exclusive with graph, session and task selectors. It opens only the narrow
`CancellationClient`, enqueues an exact driver/owner/process request and polls the
acknowledgement at 100 ms intervals for at most five seconds. Its returned Pending
status is not success or proof of termination. At most 64 requests may be pending
per driver. Repeating an exact request returns its existing durable status,
including after terminal Applied; new requests for terminal/stale drivers refuse.

Only the driver owning the retained harness can act. Windows Applied requires
that exact retained job's successful termination, observed direct-child exit,
zero active job processes and a successful fenced acknowledgement write. Registry
ownership is reserved by the same token while native waiting occurs outside its
mutex; concurrent removal cannot resurrect an old attempt. No recorded-PID or
legacy cancellation fallback participates. Unavailable or failed proof returns
Blocked. POSIX attempts best-effort cleanup only of the retained unreaped owner
and returns Blocked; it makes no complete-tree proof claim. The driver retains
ownership through subsequent collection/timeout, and never retries that attempt.
Post-invoke errors also attempt exact retained-owner cleanup, preserving unresolved
intents. A failed acknowledgement leaves Pending and reports the stable
`planner-cancellation-ack-failed` error. Normal completion first publishes its
terminal phase, then acknowledges requests committed before that transition as
Stale; it never infers Applied from completion. Worktrees and output are retained.
Other producer control, accepted-generation activation and full S3 remain excluded.

## 1. Product scope, platforms, and operating surface (ADR 0001)

**Platforms.** R1 stamp: native Windows 10/11 only. v1 target Tier A also includes
Linux via WSL2 on that same host — not a separate bare-metal Linux machine.
**WSL2 is unverified in R1** ([#20](https://github.com/smota/meshloop/issues/20)). For a given run, the
Meshloop binary, the target repository's worktrees, the Herdr instance, and the harness
CLIs must all stay on one side of the Windows/WSL boundary: crossing it (a worktree on the
Windows filesystem accessed from inside WSL via `/mnt/c`, or the reverse via `\\wsl$`) hits
the well-known path-translation, permission, and symlink hazards of that boundary and is
not tested or supported in v1. Tier B (best-effort, not blocking v1): macOS. Every
acceptance scenario must run on native Windows and on Linux-under-WSL2 before a Tier A
runtime claim is made; Tier B can lag without blocking the rest.

**Toolchain.** Rust 1.98.0 pinned, edition 2024, mise-managed. Building requires a working
linker per platform (MSVC/Windows SDK on native Windows; a standard build toolchain —
gcc/build-essential or equivalent — inside WSL/Linux). This is a build-time requirement
only and does not contradict the shipped binary having zero runtime dependencies.

**Packaging (ADR 0018).** Source-install: `meshloop-domain`,
`meshloop-engine`, `meshloop-adapters`, and `meshloop-cli` are on crates.io
(0.1.0). `xtask` stays unpublished. Operator path:
`cargo install meshloop-cli --locked` then `meshloop bundle` (see
[install.md](../install.md)). Library crates are implementation crates, not a
stable Rust API. No installer, no service registration, no global
configuration writes. Versioned by the workspace `Cargo.toml` version. Further
crates.io versions and git tags `v<version>` are human-gated.

A manual workflow, Release binary dry-run, builds `meshloop-linux-x86_64` and
`meshloop-windows-x86_64.exe` with `--locked` and records SHA-256 digests as
workflow artifacts. It does not create a GitHub Release and does not publish
crates. Attaching those binaries to a Release stays a separate maintainer
action (ADR 0018). A WSL user would run a Linux binary inside their WSL
distribution, never the Windows `.exe` reaching across the boundary.
Tier B (macOS) packaging is decided when Tier B moves toward support.

**Operating surface (ADR 0001, Accepted).** The compiled `meshloop` binary is the
sole **engine**: every capability must be reachable through it, with no saga in
wrappers. R1 **operator UX** is the `meshloop:` skill pack plus local MCP
(`meshloop mcp`, slash `/meshloop:plan`). Skills add zero engine logic (ML-014).
Unprefixed harness words (`plan`, `reviewer`) are rejected. Live workers are
Herdr panes in a Meshloop-owned workspace (`meshloop-<repo>`, ADR 0019);
origin space topology is not mutated. Fixture is the CI double.

**Scope.** A single local Git repository, already checked out, known working-tree state,
one Meshloop instance at a time. Multi-repository orchestration, remote repositories, and
concurrent instances against the same repository are out of scope until a follow-on
decision. Daemon/service mode is deferred; v1 runs in the foreground for one task graph and exits.

**Naming.** "Meshloop" is the product and binary name. "herdr-loop" must never appear in
any Meshloop artifact — it wrongly ties Meshloop's identity to Herdr, the separate tool it
coordinates. Kebab-case `mesh-loop-*` is the correct derived naming for sub-artifacts
(e.g. the two skills above) — a package-naming convention, not a revival of "mesh-loop" as
an alternate product name.

**Validation scenarios.** Demonstrate one bounded task on native Windows and on
Linux-under-WSL2 before claiming Tier A support; record Tier B separately, never implying
parity. Every product capability needs a meshloop-cli acceptance scenario before any skill
wrapper is proposed for it; a wrapper, if built, is verified only by asserting it produces
the exact CLI invocation a human would have typed.

## 2. Harness capability and agent dispatch contract (ADR 0003)

**HarnessCapabilities port** (meshloop-engine::ports, implemented per harness in
meshloop-adapters): four operations, none assuming undiscovered behavior.
1. `probe` resolves the configured executable path, reads its reported version/identity,
   and returns a `HarnessProfile` of capabilities actually observed: non-interactive/
   scripted invocation, structured output, exit-code semantics, cancellation (signal vs.
   explicit command), and any model-selection mechanism exposed. Configuration supplies
   only the executable path and any model identifiers the user entered — no hardcoded
   model name or CLI flag ships for any harness. `HarnessProfile` also records a
   compatibility flag (compatible / degraded / unsupported) from comparing observed
   version output against a small per-harness allow-list; "unsupported" refuses to
   schedule work on that harness and says why.
2. `invoke` starts one bounded unit of work using only capabilities the profile confirmed;
   requesting an unconfirmed capability is rejected before the process is spawned.
3. `cancel` terminates the owned process tree for a handle; idempotent against an
   already-exited process.
4. `collect` retrieves exit status and captured, redacted (see §4) output once a handle
   reports done.

**Error taxonomy** on any failed `invoke`/`collect`, exactly one of:
`CapacityExhausted{retry_after}` (rate limit or subscription quota hit — the harness is
fine, just full), `Unsupported` (operation never confirmed by `probe` — a caller bug caught
before dispatch), `Timeout`, or `ProcessFault` (the harness ran and produced a result, but
the result is the failure). `CapacityExhausted`/`Timeout` return a task to `ready` for
re-routing (§5); `ProcessFault` counts against the retry budget (§3).

**Agent dispatch.** An `AgentSpec` is the unit meshloop-engine hands to `invoke`:
`{ task_id, attempt_id, harness, model_ref (chosen by routing, §5 — never by this module),
worktree_path, prompt, timeout, cancellation_token }`. The agent module
(meshloop-engine::agent) builds it from a `TaskNode` plus only the context that node's
graph edges declare it depends on — never the whole repository or unrelated task history —
and has no routing/policy authority of its own.

**Prompt construction** is a deterministic, testable function, not a model call: a fixed
four-part envelope — shared situation, the specific change requested, the question the
agent must answer, and the expected output contract (what paths it may touch, that it must
leave the worktree buildable) — rendered from `TaskNode` fields. This is a narrow
prompt-construction convention carried from the source docx's SCQA framing, not adopted as
a governing theory.

**Worktree diff is the primary output.** A harness process's exit code and stdout are
secondary, redacted evidence of what it believed it did; verification (§4) evaluates the
worktree diff regardless of what the process reported — harness exit success is never
verification success. A harness with no file-system output channel is `Unsupported`, not a candidate.

**Validation scenarios.** Contract tests plus live disposable read-only sessions for every
claimed capability and selected harness; a fixture harness that misreports its version or
hangs on cancel proves profile and cancel-idempotency without needing all five real CLIs in
CI. Prompt-construction tests assert no context outside a node's declared dependencies
leaks into its prompt. A fixture harness returning a clean exit code with a broken worktree
(or the reverse) must prove the worktree diff, not exit code, gates `verifying`.

> **Amendment 2026-11-10 (ADR 0039, accepted 2026-11-10; implementation pending #147–#153).** Native
> attempts do not pass through `HarnessCapabilities`. Meshloop does not call `invoke`,
> `try_collect` or `cancel` for them, and the port is unchanged. The worktree-diff rule
> above applies to native attempts without exception. See §3 "Native dispatch (ADR 0039)".

## 3. Execution, transport, isolation, and recovery (ADR 0005)

> **Amendment 2026-11-10 (ADR 0039, accepted 2026-11-10; implementation pending #147–#153).** This
> section's subprocess-only rule stays in force for every attempt except the native worker
> attempts described in "Native dispatch (ADR 0039)" at the end of this section. The
> subprocess-only rule is the v1 "CLI-subprocess only" statement, which became direct CLI
> workers under ADR 0022. Herdr itself was removed by ADR 0022. Ownership, cancellation
> and liveness commitments below apply to subprocess attempts only. The text below is
> otherwise unchanged.

**Herdr transport.** `HerdrSessionPort` (meshloop-engine) exposes `spawn`, `status`,
`cancel`, `cleanup`. v1 ships exactly one implementation: CLI-subprocess invocation of the
`herdr` binary, using structured arguments — never shell-string interpolation — to create,
list, and terminate panes. A native-socket transport is explicitly deferred, not adopted or
rejected: it needs a platform spike confirming what Herdr actually exposes on Windows and
under WSL before any port method can assume it exists. Until then, `HerdrSessionPort` is
implementable entirely via subprocess calls with signatures that don't leak
subprocess-specific types (PIDs, stdio handles) into the engine, so a socket-backed
implementation later is additive, not a rewrite. Panes are never a security or filesystem
boundary — isolation (below) lives in the worktree, not the pane; a pane crash, kill, or
Herdr restart must never leave a worker's changes unrecoverable or misattributed.

**Isolation and integration.** One Git worktree per writer; one integration owner merges,
with candidate-bound validation (§4's evidence, checked against the exact base revision).
Stale bases and conflicts block integration rather than auto-resolving.

**Execution lifecycle.** The full state set and transition table live in
execution-lifecycle.md (`pending -> ready -> running -> verifying -> awaiting-review ->
accepted -> integrated`, plus `failed`/`cancelled`/`blocked`, plus the `awaiting-plan-review`
gate from §5) — accepting this ADR accepts that table as part of the decision, not a
separately-approved document. Key commitments: harness exit success is necessary but never
sufficient for `accepted`; every transition is logged as an event (executor, task/attempt
id, evidence reference) before it takes effect, so recovery replays rather than guesses;
retries always mint a new attempt id, never reuse one; recovery reconciles the event log
against real process/Git/worktree state before resuming anything — a persisted `running`
state with no matching live process becomes `blocked`, not a silent retry.

**Validation scenarios.** Spawn, monitor, cancel, and clean up owned sessions on each Tier A
platform (native Windows, Linux-under-WSL2) without orphan processes; a
kill-the-herdr-cli-mid-session fault test; a duplicate-cleanup-is-a-no-op test. Concurrent
writes, conflicting changes, stale evidence, and interrupted integration in disposable
repositories. Fault-injection for crash, timeout, stale state, partial completion, and
exhausted retries, including killing the Meshloop process itself mid-`running` and
mid-`accepted`-to-`integrated`, asserting recovery reaches `blocked` or a correctly new
attempt — never a duplicate integration.

### Native dispatch (ADR 0039)

Accepted with ADR 0039 on 2026-11-10. This is the mechanism for that record; it is not yet
implemented, and #147 to #150 implement it.

**Eligibility.** Routing (§5) runs unchanged. A run-loop worker node is native-eligible
only when all of the following hold:
- the routed harness has `dispatch = "native"`;
- its `kind` is set and equals the origin harness;
- an origin session is set;
- that `kind` is in the proven set (below).

`kind` is optional in config (`HarnessConfig::kind`, config.rs:117-118), and a harness
without one is never eligible. `doctor` refuses `native` for an unproven or unset kind, and
`dispatch_warnings` reports a setting that can never apply. Planner, reviewer and
model-review dispatch are never native.

**Proven set.** The proven set is a compiled-in, read-only capability table in the
composition root, for example `crates/meshloop-cli/src/native_support.rs`. Each entry has:
- the harness `kind`;
- the version range observed in the live proof;
- a link to the #152 evidence and its date.

How the table is maintained and used:
- It is mirrored in docs/engineering/harnesses.md "Native dispatch support".
- `doctor` and `dispatch_warnings` read it. `doctor` warns when a probed version is
  outside the observed range.
- It is not configurable through `meshloop.toml`. An operator cannot certify a harness.
- It changes only by pull request with new live evidence.
- Nodecode additionally needs an ADR 0038 amendment.
- The table records observed capability. It is not a per-harness CLI flag, so ADR 0003's
  rule that Meshloop ships no per-harness flags is unaffected.

Which command acts on an eligible node:
- **`meshloop next`** parks eligible nodes. It is the only command that does.
- **`run`, `resume` and detached drivers** leave eligible nodes `ready`. They return
  `AwaitingNativePull`, or `AwaitingNativeReport` when only parked attempts remain, and
  `loop_until_idle` returns on both instead of re-ticking.
- **`next`** returns `AwaitingSubprocessDriver` when the only ready nodes left are routed
  to subprocess dispatch.

**`meshloop next [--graph <id>] [--wait-seconds <n>] --json`.** It is origin-bound and
never starts a subprocess worker, so it never holds an owned process when it returns.
Subprocess-routed ready nodes are left for `run` or `resume`. `next` refuses while another
driver is live on the graph: one ticking writer at a time. `report` only appends, so it is
allowed while another driver is live. Steps, in order:
1. For every parked attempt past `deadline_at`, append `ParkDeadlinePassed`.
2. Run the loop steps that need no subprocess worker: verify reported attempts, merge,
   promote. Stop between ticks when `--wait-seconds` expires. The default is finite and
   set by #148. One verification already running is not interrupted, so `next` can
   overrun `--wait-seconds` by up to the check timeout (`task_timeout`, run_loop.rs:1616).
3. For each eligible ready node within `max_concurrent_workers` (parked attempts occupy
   slots):
   1. If the task's previous attempt used the other mode, append `DispatchModeChanged`
      (`ready → ready`, reason required).
   2. Create the attempt id, worktree, branch and `.meshloop-prompt-<attempt>` file.
   3. Persist on the attempt: mode, origin harness and session, requested harness and
      model, base revision, `parked_at`, and
      `deadline_at = parked_at + [limits] task_timeout_seconds`.
   4. Record the expected-state baseline: the integrate HEAD, the origin checkout HEAD,
      and the origin checkout's tracked-file porcelain digest.
   5. Append `AttemptParked`. The attempt row and the event are written in one store
      transaction.

Output: every parked, unreported attempt bound to this origin, with `task_id`,
`attempt_id`, `role` (`meshloop:worker`), `prompt_path`, `worktree_path`,
`base_revision`, `harness`, `model_ref` and `deadline_at`. It also carries the idle
reason, or `WaitBoundReached` when the bound expired. Repeating `next` returns the same
items and never re-parks them.

**`meshloop report --task <id> --attempt <id> [--note <text> | --note-file <path>] [--model <name>] --json`.**
1. Refuse, appending nothing, when any of these holds:
   - the origin differs from the attempt's recorded origin;
   - the attempt is not the task's latest;
   - a `NativeReportReceived` already exists for the attempt.
2. If the attempt is overdue, append `ParkDeadlinePassed` first.
3. Append `NativeReportReceived` and set `ended_at`.
4. Store the note redacted and truncated to the 2048-byte outcome limit, as a
   `native-report` row that never counts toward a pass. Store the reported model beside
   the requested one.
5. On `failed` or `cancelled`, the event is stored, the state is unchanged, and nothing is
   verified. `LiveWorkerSettled` is never used for native attempts.

The origin binding is an unauthenticated claim, like `--as`.

**Verification.** It runs on the next tick through the no-in-memory-outcome path
(`recover_verifying` / `finish_orphaned_verification`). That path gets a native branch:
1. **Expected-state check.**
   - The integrate worktree's HEAD must equal the last ledger-recorded
     `IntegrationOwnerMerge` result, or the park baseline if there is none, and its
     porcelain must be clean.
   - The origin checkout's HEAD must equal the park baseline, or the result of a
     ledger-recorded Meshloop integrate event since park. Its tracked-file porcelain digest
     must equal the park baseline.
   - Any other difference fails the attempt and lists the paths.
   - Today neither `merge_accepted` nor `integrate_into` records the resulting HEAD
     (run_loop.rs:1347-1355, 2644-2652). #149 adds it.
2. **Snapshot.** `commit_all` in the attempt worktree. The resulting SHA is stored as the
   attempt's `verified_revision`.
3. **Checks.** `git-diff` against the recorded base (honouring `empty_diff_ok`),
   `allowed-paths`, and the configured verify command, all bound to `verified_revision`.
4. **Re-read.** Re-read the attempt worktree's HEAD and porcelain. Any change since step 2
   fails the attempt with reason `worktree changed during verification`.
5. **No repair.** No repair runs, heavy or micro (ADR 0039 decision 6). A failure goes to
   the retry budget.
6. **Feedback.** Record feedback under `(harness, "unverified", tier, native)`.
7. **Gate.** The unchanged gate path follows: ADR 0037 verify gate or
   `meshloop accept --as`.

**Integration of a native attempt.** Integration uses `verified_revision`, never the
branch tip.
- `accept` refuses when the attempt branch tip differs from `verified_revision`.
- `merge_accepted` merges `verified_revision`. If the tip differs, it appends
  `StaleBaseDetected` (accepted → failed). Today it merges the branch ref
  (run_loop.rs:1344-1346).
- `prepare_deliverable` cherry-picks `base..verified_revision`. If the tip differs, it
  refuses. Today it picks up to the branch tip (run_loop.rs:2767-2768).

Required test: "commit after report is not integrated". The same binding is recommended,
but not required by ADR 0039, for subprocess attempts.

**Cancel, attestation, restart, recovery.**
- **Cancel.** `meshloop cancel` on `awaiting-report` appends `Cancel` with reason
  `native: tracking stopped; sub-agent not stopped by Meshloop`. It calls no harness
  method and uses no recorded PID. Its JSON names the attempt and the origin.
- **Attestation.** `meshloop cancel --task <id> --attempt <id> --attest-stopped --as <identity>`
  appends `NativeStopAttested` for that native attempt.
  - It is recorded as a self-transition on `failed`, `cancelled`, `accepted` or
    `integrated`. When the attempt is still parked, `Cancel` is appended first.
  - The executor is the identity.
  - It covers only the named attempt.
- **Cleanup.** A native attempt's worktree is eligible for cleanup (ADR 0033, #142) only
  in one of two cases:
  - a `NativeStopAttested` event exists;
  - its `verified_revision` is landed and the worktree still equals it at cleanup
    preview.

  Otherwise it is liveness-uncertain, and cleanup is blocked.
- **Restart.** `resume --restart` refuses while any attempt is `awaiting-report`.
- **Recovery.** `crash_fail` and `reattach_running` consider only `running`. A parked
  attempt is reconciled only by report, deadline or cancel.

**Status.** Read-only (ADR 0034). For each native attempt, `status` and `inspect` show:
- `dispatch_mode`, state `awaiting-report`, `waiting_for: native_report`;
- origin harness and session, `parked_at`, `deadline_at`, `overdue`;
- `report` (`pending`, `received` or `late`);
- `model_ref`, `model_reported` and `verified_revision`;
- `live: NotObservable`;
- `limits`: `no_owned_process`, `no_cancel_proof`, `no_liveness`,
  `no_detached_execution`, `inherits_session_permissions`.

**Validation scenarios.** All are fixture-only and offline:
- park, report, deadline, cancel, attestation;
- late report on `failed` or `cancelled`;
- wrong-origin refusal and a duplicate report;
- crash between park and report, then `resume`;
- a report claiming success while checks fail;
- an out-of-worktree change, versus a Meshloop merge that is not flagged;
- a commit in the origin checkout during park that fails the attempt, versus a
  ledger-recorded Meshloop integrate that does not;
- a commit after report that is not integrated;
- an empty diff;
- a mode change event;
- the `--wait-seconds` bound;
- an unproven or unset kind refused by `doctor`, and a probed version outside the
  observed range warned about;
- `AwaitingSubprocessDriver` from `next`, and `report` accepted while another driver is
  live;
- refusal of a ledger with an unknown state or event.

Live proof per harness is #152, not CI.

### Sizing at 45ef6a4 (ADR 0039)

This evidence checks the engine estimate on goal #145. "Confirmed" means the code matched
the estimate, and "correction" means it did not.

| Fact | Evidence | Effect |
|---|---|---|
| The tick does not block: dispatch returns `Running`, and later ticks poll | run_loop.rs:1560 (`Ok(None)` arm), 1055-1144 (`try_collect` at 1105) | Confirmed: a new state plus arms, no loop restructuring |
| `orchestrator.rs` is not on the live path | `dispatch_with_fallback` (orchestrator.rs:35) is called only by its tests (161, 192, 216); live fallback is `maybe_fallback` (run_loop.rs:2151) | Correction: no change there |
| A PID-less `Running` attempt is failed on resume | `attempt_driver_dead` returns true for `pid: None` (run_loop.rs:2300); `crash_fail` (2322-2331) | Correction: a new task state is needed in domain, store and both status projections (`status_snapshot` run_loop.rs:2894, `workspace/read.rs`) |
| The foreground loop spins on live workers | run_loop.rs:2218 | New idle reasons are needed |
| Heavy repair blocks inside the tick | `dispatch_repair_round` invoke then blocking `collect` (run_loop.rs:1670-1675) | No in-attempt heavy repair for native attempts |
| The orphan verification path verifies from the worktree alone | `verify_deferred` → `recover_verifying` / `finish_orphaned_verification` (run_loop.rs:1797-1850, 2340-2474) | Reusable, but it runs no repair, heavy or micro, and records feedback under the requested `model_ref` with no mode (run_loop.rs:2456); it needs a native branch |
| Integration uses the branch tip, not the verified revision | `merge_accepted` (run_loop.rs:1344-1346); `prepare_deliverable` (2767-2768); `integrate_into` records no event (2644-2652) | Verified-revision binding is needed for native attempts |
| No per-node timeout exists | `TaskNode` (task_graph.rs:50-73); `[limits] task_timeout_seconds` (config.rs:50) → `RunLimits::task_timeout` (run_loop.rs:85, compose.rs:265) | Correction: the deadline uses this value, persisted at park |
| The origin is not persisted | `Origin` (origin.rs:6-9); `attempts` columns (workspace/schema.rs:42-55) | Confirmed: a schema migration is needed |
| Unknown names fail closed | `parse_state` / `parse_event` (store.rs:199-237) | Older binaries refuse new ledgers |
| Late harness exit is legal only on `failed` | state.rs:96; `LiveWorkerSettled` state.rs:94 | Native needs its own late-report arms |
| Nodes have no role field | `TaskNode` (task_graph.rs:50-73); reviewers are `meshloop orchestrate` leaves (orchestrate.rs:1-3) | Correction: "builder nodes only" means run-loop worker attempts |
| MCP wraps the CLI with an unbounded wait | `dispatch_tool` (mcp.rs:202-235; `wait_with_output` at 235) | Confirmed: the CLI is the contract; `next` needs `--wait-seconds` |
| Harness `kind` is optional | config.rs:117-118 | An unset kind is never eligible |
| `Origin::blocks_worker` is unused on the live path | origin.rs:33, referenced only by its own test | No conflicting guard |
| The run loop mutates only unbound runs | `require_legacy_mutation` (run_loop.rs:356-372) | Native mode follows the same producer path |

Effort L for #147 stands. The surface also includes `state.rs`, store parse and migration
(attempt columns plus a `routing_feedback` mode column), and both status projections.

## 4. Verification and persistence (ADR 0007)

**Evidence kinds**, each bound to the exact candidate revision hash and attempt id it validates:
- `DeterministicEvidence`: linter/build/test tool, exit code, captured redacted output,
  tool version. Produced by meshloop-adapters running real local tools; never fabricated or
  inferred from a model's claim.
- `ModelReviewEvidence`: which harness/model reviewed, its verdict, its stated rationale.
  Advisory only — never substitutes for `DeterministicEvidence`, never treated as a
  correctness guarantee.
- `HumanAcceptanceEvidence`: which human, when, against which exact revision. Required
  unconditionally for Tier 3 tasks before `accepted`. Tier 1/2 may reach `accepted` on
  `DeterministicEvidence` plus, where routing policy requires it, `ModelReviewEvidence` — a
  configurable threshold that can only raise the bar toward more human involvement, never
  remove `DeterministicEvidence` as a precondition.

A candidate missing any evidence kind required for its tier cannot reach `accepted`,
regardless of what the harness reported. Evidence against a stale base revision is
rejected, never silently rebased.

**Persistence.** Embedded SQLite, versioned schema (`schema_version` from row one,
forward-only migrations — a build refuses to run against a newer on-disk schema than it
knows), local only. Proposed table shape (an actual schema file belongs in `schemas/` only
once this ADR is accepted, per `schemas/README.md`):
- `tasks(task_id, graph_id, tier, created_at, ...)` — node metadata, no free-text prompt content.
- `attempts(attempt_id, task_id, harness, model_ref, started_at, ended_at, outcome)`.
- `events(event_id, task_id, attempt_id, from_state, to_state, event_type, reason, executor, evidence_ref, occurred_at)` —
  the durable transition log §3's recovery depends on.
- `evidence(evidence_ref, kind, candidate_revision, summary_redacted, tool_version, created_at)` —
  summaries only; large raw output goes to redacted on-disk log files referenced by path, never inlined.
- `routing_feedback(harness, model_ref, tier, success_count, failure_count, updated_at)` —
  aggregate counters only, feeding §5's routing scores.

**Existing-ledger observation (ADR 0034).** A read-only SQLite connection may let
SQLite create, update, or remove its WAL/SHM coordination sidecars so readers see
committed live WAL data while a writer holds the repository fence. Observation
does not migrate schemas, set journal mode, write application schema/data, initialize
identity, register roots, or adopt runs. An absent ledger creates no files or
directories. Permission and compatibility errors remain explicit; immutable live-source
fallbacks are forbidden. This contract does not claim full S3 implementation.

**Redaction.** No raw prompt text, no full model completions, no file contents, no
credentials or tokens are persisted. [ADR 0033](adr/0033-workspace-lifecycle.md),
accepted 2026-10-28 with implementation not-started, narrowly permits absolute
external roots only in a protected local operational root registry needed for
reconciliation, including the canonical common-directory location. Use stable root
aliases and relative paths in the ledger, evidence, events and default JSON; never
include absolute external roots in events, evidence, logs or exports. Root aliases
are operational references, not public identities. Durability bundles may contain
source; keep them local and uncommitted, excluded from evidence, telemetry, log and
report exports. This accepted exception does not claim a runtime implementation.
`summary_redacted` fields pass a redaction step (secret-pattern scan plus a length cap)
before being written. Retention: `events`/`evidence` retained indefinitely by default
(small, audit-relevant); time-based pruning is a future decision if the store grows
unexpectedly, but v1 never silently expires recovery-relevant history.

**Validation scenarios.** Reject tampered tests, missing evidence, stale candidates, false
success, unauthorized acceptance — a candidate with `DeterministicEvidence` and
`ModelReviewEvidence` but no `HumanAcceptanceEvidence` at Tier 3 must be rejected at
`awaiting-review -> accepted`, not merely flagged. Interrupted writes, schema upgrades,
corruption, redaction, retention, recovery consistency; a redaction-bypass test asserting a
synthetic secret pattern never reaches `evidence.summary_redacted` or any log file.

## 5. Planning and routing (ADR 0009)

**Planning.** Decomposition is not a separate code path: it is one `AgentSpec` (§2)
dispatched before any other task exists, routed like any other node (below) at a fixed high
tier — a bad decomposition is higher-risk than almost any single generated change, since
every downstream task inherits its errors. Its prompt is built from the user's authorized
objective plus explicit scope/exclusions; its declared output contract is a task graph
conforming to meshloop-domain::task_graph's schema, not a worktree diff. The planner module
(meshloop-engine::planner) dispatches the decomposition agent, validates its output
structurally (schema conformance, cycle detection, no dangling dependencies — rejecting and
re-dispatching on failure), and assigns each node a risk tier — it does not judge whether
the decomposition is a *good* one; MECE-ness is a semantic property this project cannot
mechanically verify. Because of that, a produced task graph enters
`awaiting-plan-review -> plan-accepted` (parallel to, not replacing, per-node
`awaiting-review` in execution-lifecycle.md) and requires human acceptance before its first
node schedules, for any objective at or above the lowest risk tier. Re-planning (a node's
repeated failure implying the decomposition was wrong, or scope changing mid-run) is out of
scope for v1; a rejected or irrecoverably failed plan restarts as a new planning dispatch
from scratch, not a live graph edit.

**Routing: Quota-Aware Capability Routing (QACR).** "Budget" means subscription
rate-limit/quota headroom, not dollar cost — all harnesses are flat-rate subscriptions, so
the resource allocated is turns-per-reset-window per harness (and per model tier where one
reports separate limits), not spend. Runs per ready task node:
1. Filter to harness/model combinations the user explicitly configured and that `probe`
   (§2) confirmed compatible. No candidate outside this set is ever considered.
2. Filter out any candidate currently in `CapacityExhausted` cooldown (§2), using its most
   recent observed `retry_after`. Quota is tracked reactively from observed exhaustion
   signals, not a proactively queried number most CLIs don't expose.
3. Score remaining candidates on: tier-to-model fit, `historical_success_rate` for this
   exact (harness, model, tier) triple (from `routing_feedback`, §4, decayed toward neutral
   for sparse data), and a load-balancing term favoring a harness with more headroom
   relative to its own reset window, so configured subscriptions get used roughly
   proportional to available capacity.
4. Break ties with `coupling_penalty`: prefer keeping tightly-coupled nodes on the same
   harness/session context, independent nodes on different harnesses.
5. Dispatch to the top-scored candidate. On `CapacityExhausted`/`Timeout`/`ProcessFault`
   (§2), record `routing_feedback` and re-run QACR excluding the failed candidate —
   visibly, as a logged fallback event, never silently.

`historical_success_rate` can only reorder among already-configured, already-capable
candidates; it can never add a candidate the user didn't configure or bypass a capability
rejection. Concurrency: low-coupling nodes with headroom on distinct harnesses run in
bounded parallel; high-coupling nodes run sequentially even when capacity would allow
parallel dispatch, since merge-conflict risk (§3) outweighs throughput gain. Concrete
numeric thresholds (what counts as "hot," concurrency caps, retry budgets) are session
configuration with documented defaults, not constants here — no measurement yet justifies a
fixed default confidently.

**Extensibility.** Step 3's scoring is a weighted sum over independently pluggable
`RoutingSignal` implementations (`fn score(&self, candidate, context) -> SignalScore`), one
per term (tier-fit, historical success, load-balance, coupling, or a future signal such as
latency or a user-supplied preference weight). Adding a signal, harness, or model tier
never requires changing `router`'s control flow — only registering a new `RoutingSignal` or
`HarnessCapabilities` implementation and configuration entry. The harnesses named elsewhere
in these documents are the initial configured set, not an architectural ceiling.

**Validation scenarios.** A decomposition agent returning a cyclic or schema-invalid graph
must be rejected and retried, never partially scheduled; an objective at or above the
lowest risk tier must not reach its first `ready` node without recorded plan-level
acceptance; a `CapacityExhausted` decomposition dispatch falls back through QACR like any
other dispatch. Unavailable models, quota cooldown expiry (not just permanent exhaustion),
fallback exhaustion (all candidates cooled down at once resolves to `blocked`, not a
crash), high coupling, policy-preserving feedback; no test can make the router select a
harness/model the user never configured; load actually spreads across two configured
harnesses with equal headroom rather than always picking the first.

## 6. Deterministic loop algorithms (ADR 0029)

Mechanism only. The decision is ADR 0029. This section is the v1 *algorithm*; auto-reinvoke
of a harness after `Rollback`/`Continue` is ADR 0026 and is not in the R1 saga.

**Diagnostic lattice.** `parse_diagnostics` walks rustc/cargo/tsc/python/go/generic stderr
into a `BTreeSet` of atoms `(severity, code, basename, template_hash)`. Templates replace
backtick-quoted idents with `` `_` `` and digits with `#`, so `cannot find value \`foo\``
and `cannot find value \`bar\`` are the same atom. Worktree prefixes and `line:col` are
dropped. `fingerprint` is portable FNV-1a over the ordered set. Progress φ is lexicographic
`(syntax, type, test, error, blocking)`. Warnings do not block Accept.

**Convergence.** `RepairSession::observe(lattice, snapshot_rev) -> RepairAction`:

1. Repeated fingerprint → `Stop { Oscillation { cycle_at } }` (no-oscillation guarantee).
2. `blocking == 0` → `Accept`.
3. Observation count ≥ `max_rounds` → `Stop { BudgetExhausted }`.
4. First failing observation → `Continue` with a negative constraint listing current codes.
5. `syntax(next) > syntax(prev)` → `Rollback { to_rev: prev.snapshot_rev }` plus a syntax ban.
6. φ strictly decreases → `Continue` with remaining codes; otherwise `Rollback`.

The caller applies `WorkspacePort::reset_hard(worktree, to_rev)` on Rollback. The engine
does not emit a new `TaskState`; repair stays inside the attempt.

**Signature index.** Texts are CountSketch-embedded into R^64, rotated by a normalized
Walsh–Hadamard transform (the data-oblivious stand-in for TurboQuant's random rotation),
then 1-bit (sign) or 2-bit (Lloyd-Max N(0,1/d)) packed. `scale = 1/⟨u, x̂⟩` unbiases the
inner product. `add` never rewrites an existing code. Search is a linear scan — correct
at repository scale. `select_context` pins `allowed_paths`, then fills a 24-file budget
from `rank_files`. `turbovec` is not a dependency; see ADR 0029 alternatives.

**Syntactic slice.** `SignatureSnapshot` hashes each file's sorted public signatures.
`impact(before, after)` is `BodyOnly`, `SignatureChanged { files }`, or `MembershipChanged`.
`skip_full_compile` is true only for `BodyOnly` with no syntax/type diagnostics — git-diff
and tests still run. `check_slice` unions changed files, diagnostic basenames, and index
neighbors.

**QACR restless bandit.** `RestlessBanditSignal` scores `c / sqrt(n+1) + 0.5 * headroom`
(`c = 0.7`). Cooldown remains a hard filter (the arm is not pullable). Missing headroom
is 0, never 1. Historical success remains a separate signal so adding this one does not
retune exploitation.

## 7. Dynamic graph mutation and replanning (ADR 0028)

**Graph mutation primitive.** Multi-agent loops can encounter unforeseen prerequisites
during task execution. Rather than aborting and re-planning from scratch, `RunLoop::mutate_plan`
applies atomic graph mutations (`GraphMutation`, serialized with `#[serde(tag = "type", rename_all = "snake_case")]`):

- `InsertPrerequisite { target_task, new_tasks }`: New sub-graph tasks are inserted. Terminal nodes
  of `new_tasks` (nodes without dependents inside `new_tasks`) are rewired into `target_task.depends_on`.
  Existing dependencies of `target_task` are strictly preserved.
- `AppendFollowup { source_task, new_tasks }`: `source_task` is appended as a dependency to the root
  nodes of `new_tasks` (nodes with empty `depends_on`).

**DAG invariants & validation.** Every mutation is checked via petgraph before committing:
- Rejects cycles (`petgraph::algo::is_cyclic_directed`).
- Rejects dangling dependencies (all referenced IDs must exist in the resulting graph).
- Rejects duplicate IDs and reserved task ID 0.
- On failure, the mutation rolls back atomically to the prior graph state.

**Active-state immutability.** A target task cannot have prerequisites inserted if it has already
entered execution (`Running`, `Verifying`, `AwaitingReview`, `Accepted`, `Integrated`).
Attempting to mutate an active task yields `OrchestratorError::Illegal`.

**Readiness reversal.** Prepending prerequisites to a `Ready` task transitions it back to `Pending`
via domain event `Event::GraphMutated`:
- `(Ready, GraphMutated) => Pending`
- `(Pending, GraphMutated) => Pending`
- `(Blocked, GraphMutated) => Blocked`

**Persistence & replay.**
- Mutated graph tiers are recomputed via `assign_tiers`.
- Updated plan JSON and SHA-256 are persisted in SQLite `runs` table and `.meshloop/plan.json`.
- `Event::GraphMutated` transition is appended to SQLite event log.
- `replay_tasks` cleanly folds `Event::GraphMutated` and dynamic task rows during crash recovery.

**Operator governance.** If invoked with `--require-review`, `RunLoop::mutate_plan` transitions the
run to `PlanState::AwaitingPlanReview`. `meshloop resume` is refused until the operator accepts the
modified plan via `meshloop review-plan --plan .meshloop/plan.json --accept --as <identity>`.

### Bounded CLI harness output collection

CLI invocation and version probes drain stdout and stderr concurrently. Each stream
retains at most 4 MiB; overflow, read failure, invalid UTF-8 or reader failure is a
capture fault. The existing 2048-byte redacted outcome limit remains in effect;
large planner JSON uses its protected owned plan file. Sixteen capture pairs are
admitted process-wide before spawn (at most 32 reader threads and 128 MiB of retained
stream bytes, excluding ordinary transient parsing/redaction allocations). Both
workers and capture state retain admission until released. Escaped external writers
holding a pipe can consume a slot even after harness destruction; subsequent harness
instances refuse new admission when capacity is exhausted. This bounds accumulation,
without promising termination of an escaped writer. Probes share the admission budget.

Polling never reads a pipe or joins an unfinished reader under the harness registry
mutex. The CLI dispatch wrapper delegates nonblocking collection. Owned handles remain
available through output collection so deadline failure can terminate owned descendants.
Pending pipe EOF at the deadline reports an explicit EOF timeout; a still-running leader
reports the existing attempt Timeout. Existing cancellation remains best effort and does
not prove process tree quiescence. The separate strict provisional-driver contract
is described above; full S3 producer/recovery integration remains outstanding.

### Atomic owned Windows startup (ADR 0035)

Owned harnesses, checks and bounded Git probes use an explicit adapter spawn
specification. Windows creates the child directly inside a private kill-on-close
job with `CreateProcessW` job-list attributes; its explicit standard-handle allowlist
excludes the job and unrelated handles. Private inheritable duplicates are closed
before the shared launch guard releases. Meshloop-controlled ordinary and detached
launches share this guard; embedders must use `process::spawn_command` for other
launches in the same host process. The guard covers launch rather than process wait.
Detached handoff policy and POSIX process-group/recovery policy remain unchanged.

The native wrapper supports structured arguments, UTF-16 environment changes,
working directories and null/piped/inherited standard streams. Batch launch forms
are explicitly rejected. Executable search is checked against stable Rust's
Windows behavior, including changed child PATH precedence and parent-relative
explicit executable paths. Native failures expose operation names and error codes,
without arguments, environment values or private absolute paths.

`OwnedChild::terminate_and_observe` retains the job while it observes direct-child
exit, caches that status, releases its own process reference and queries for zero
active job processes within a deadline. Failure and unresolved membership return
errors. This adapter observation is not a driver acknowledgement. Legacy cleanup
still provides no quiescence proof. Focused native fixtures and their negative
controls are recorded in ADR 0035; independent review, concrete unsafe-code human
acceptance and the remaining driver-control evidence are still pending.
