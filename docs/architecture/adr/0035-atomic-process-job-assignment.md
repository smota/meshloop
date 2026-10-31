# 0035 Atomic Windows process assignment to owned jobs

- Status: Accepted
- Implementation: native adapter implemented and focused Windows validation passed; concrete unsafe-code human acceptance pending
- Date: 2026-10-31
- Author/executor: proposal by Codex bounded driver-control intake worker; native implementation by Codex `/root/atomic_builder`; parent Codex `/root` owns governance, integration and combined validation
- Reviewer: proposal reviewed by independent GPT-6-sol safety reviewer; concrete native diff reviewed by independent Codex `/root/driver_safety_review`, with no code blockers reported before integration-test launch rework; human unsafe-code acceptance pending
- Decision owner: Samuel
- Accepted date: 2026-10-31
- Approval evidence: Samuel explicitly replied "Accepted" to concrete ADR0035 proposal at checkpoint 2019951; this accepts the architecture decision, not runtime implementation or full S3
- Supersedes: ADR 0025 Windows post-launch assignment mechanism only; other ADR 0025 decisions remain unchanged
- Superseded by: none
- Tracking: [#74](https://github.com/smota/meshloop/issues/74), within [#71](https://github.com/smota/meshloop/issues/71)
- Inspected source: `1e6a831e84c6b74528ecc7a4d88d7238e579e90f`

## Context and constraints

At the inspected revision, `process/job.rs::spawn_owned` calls `Command::spawn`
before `JobObject::assign_process`. A child can run and create descendants in that
interval. Later observing an empty job cannot establish termination of descendants
that escaped assignment. `OwnedChild::kill_tree` also discards termination errors;
its return is not evidence for an Applied cancellation acknowledgement.

[ADR 0025](0025-process-tree-ownership.md) requires Windows process-tree
ownership and cleanup on owner crash. Starting suspended before assignment closes
the execution interval but leaves a crash interval containing an unowned suspended
process. Microsoft documents both this failure and atomic job-list creation in
[its process-creation explanation](https://devblogs.microsoft.com/oldnewthing/20230209-00/?p=107812).
The S3 driver-control milestone cannot weaken those guarantees to acknowledge only
the subset of descendants that happened to enter a job.

This proposal concerns owned Windows subprocess creation and proof of termination.
It does not change detached driver handoff, POSIX recovery policy, accepted-plan
gates, workspace placement, schemas, adoption, cleanup authorization or telemetry.

## Alternatives

1. Keep post-launch assignment: smallest change, but retains the escape interval
   and cannot support the required cancellation proof.
2. Create suspended, assign, then resume: prevents early execution but fails the
   owner-crash guarantee before assignment; thread enumeration adds complexity
   without closing that interval.
3. Use stable `std::process::Command` with creation attributes: desirable when
   available, but `spawn_with_attributes` is currently
   [unstable in Rust's Windows API](https://doc.rust-lang.org/stable/src/std/os/windows/process.rs.html).
   Changing the project to nightly is outside this proposal.
4. Use an atomic native spawn with a private ownership wrapper: larger native
   boundary and compatibility burden, but addresses both intervals. Recommended.
   A helper anchor process merely moves the first-assignment problem elsewhere.
   PID/image matching and a weaker Applied meaning do not provide ownership proof.

## Decision or proposal

Propose Windows creation through `CreateProcessW`, `STARTUPINFOEXW` and
`PROC_THREAD_ATTRIBUTE_JOB_LIST`, with `EXTENDED_STARTUPINFO_PRESENT`. Create and
configure a private job with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` first; place that
job in the creation attributes so membership precedes child execution. Keep its
handle non-inheritable and disable breakaway. Refuse unsupported job attributes,
incompatible enclosing jobs, or failed membership establishment explicitly; never
fall back to post-launch assignment. The documented Windows 10+ attribute is an
explicit requirement for this owned-spawn path, subject to actual host validation.
See [creation attributes](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-updateprocthreadattribute)
and [CreateProcessW](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessw).

Introduce a private typed owned-spawn specification at the adapter boundary for
executable, separate arguments, environment changes, working directory and explicit
stdin/stdout/stderr modes. A private process/pipe wrapper owns native handles and
provides the operations current harness/check callers need. Do not transmute or
construct `std::process::Child` from undocumented layout, or attempt to recover all
stdio configuration from an opaque `Command`. Keep engine/domain ports independent
of Win32 and keep the existing concurrent bounded output-capture behavior.

Preserve structured argument semantics, Unicode, empty arguments, quotes and
trailing backslashes using a tested Windows quoting contract. Preserve current
executable resolution, environment inheritance/removal, cwd and supported stdio
behavior; explicitly reject an unsupported launch form before creation. Do not
introduce shell interpolation or implicit batch execution. Restrict inheritance to
the intended standard handles using a handle allowlist; never inherit the job,
ledger, fence or unrelated handles. Preserve hidden-console behavior and redact
arguments, environment values, prompts and absolute private paths from diagnostics.

Owned cancellation uses the retained job/process handles, bounded termination and
observation. Applied requires the exact driver's authorized request, observed direct
child termination, a successful job query reporting zero active processes, and a
successful fenced acknowledgement write. A termination return, parent exit, pipe
EOF, heartbeat expiry or missing PID alone is insufficient. Query/termination
failure or unresolved descendants yields Blocked; an unwritten acknowledgement
remains Pending and surfaces the persistence failure. Keep the job handle until
observation finishes. Account for Windows process-reference lifetimes rather than
assuming immediate counter updates; see [job accounting](https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-jobobject_basic_accounting_information).

## Consequences

This expands the existing unsafe adapter boundary. Implementation requires explicit
SAFETY invariants for buffer/attribute lifetimes, handle ownership and inheritance,
partial-failure teardown and concurrent spawning, plus focused independent review
and human review of the concrete native diff before acceptance. Prefer existing
`windows-sys`; justify feature additions and any dependency separately. This
proposal does not authorize an unreviewed native implementation or claim support
for unsupported Windows hosts, arbitrary external process attachment or escaped
processes. POSIX cannot report Applied from this Windows proof.

## Required verification

Before dependent S3 acceptance, require real Windows fixtures for first-instruction
descendant creation, cancellation
of child and grandchild with an unrelated process still alive, and owner termination
at instrumented wrapper checkpoints after `CreateProcessW` returns but before the
owned-spawn wrapper returns. These fixtures do not claim deterministic interruption
inside the Windows API call. Verify handle
inheritance isolation and concurrent spawns, Unicode/quoting/executable/env/cwd/stdio
compatibility, large simultaneous output, failure teardown and bounded error paths.
Use negative controls that reintroduce post-launch assignment and suspended-before-
assignment crash gaps; demonstrate the fixtures detect escaped or abandoned children.
Inject denied/unsupported attributes, termination/query failures and acknowledgement
write failure: none may report Applied. Run focused process/harness/CLI proofs and
combined workspace check/smoke on the exact candidate; record actual counts and OS.
Samuel accepted this architecture proposal on 2026-10-31 before implementation.
Review of the concrete unsafe implementation and the remaining #74 evidence/human
gates precede full S3 acceptance.

## Implementation evidence, 2026-10-31

The adapter now consumes `OwnedSpawnSpec`, creates a non-inheritable kill-on-close
job, and uses `CreateProcessW` with both `JOB_LIST` and `HANDLE_LIST` creation
attributes. Only private duplicated standard handles are inheritable. The native
child/process/pipe wrapper owns its handles; there is no `Child` layout conversion,
post-launch assignment, suspended fallback, or owned-path PID kill fallback.
`terminate_and_observe` requires observed direct-child exit and a successful empty
job query within its deadline. It caches exit status and releases its process
reference before observing emptiness. Legacy `kill_tree` still discards cleanup
errors and is explicitly not an acknowledgement.

The shared launch guard encloses the entire private inheritable-duplicate lifetime;
controlled ordinary and detached launches use the same guard, release it after
creation, and wait outside it. This coordinates Meshloop's own host process. An
embedding caller launching through raw `Command::spawn` instead of `spawn_command`
does not participate in that guard. Detached launch policy is unchanged.

Focused native Windows validation passed: 11 unit tests in
`process::job::windows::tests`, 6 `owned_spawn` integration tests, 3 `process_tree`,
19 `harness_contract`, and 2 `check_runner` tests. The native unit tests include
owner termination after creation and before wrapper return; both rejected-startup
negative controls; actual event-object inheritance isolation with an unfiltered
ordinary-spawn control; forced concurrent owned/ordinary/detached launch ordering;
denied assignment rights, unsupported attributes, native termination/query errors,
bounded observation and repeated failed-creation handle cleanup. Compatibility
tests exercise Unicode/quoting, case-insensitive environment changes, cwd,
stdin/stdout/stderr and simultaneous large output. Resolver regressions compare
actual executable selection with stable Rust `Command`.

On this tested host, an extra duplicated reference to an exited process did not
prevent an empty active-process query. The implementation conservatively releases
its own reference and does not assume that result for other hosts. The bounded
observation test uses an intentionally live job instead of depending on reference
count behavior. Checkpoint fixtures do not interrupt inside the Windows API call.

`cargo check -p meshloop-cli --tests` and focused all-targets adapter/CLI Clippy
passed. Parent-governed workspace check subsequently passed before the final
four-file integration-test launch hardening (observation
`ac2fd05c71a196002218ce270a0dd58282df067d3d0e4213ebf4212f1d00903f`). Independent
native review reported no code blockers. The integration binaries also embed native
adapters, so their ordinary test-host launches now participate in the shared guard;
assertions, deadlines, stream policies and parallelism are preserved. Fresh combined
validation of this final candidate, smoke, review of the test-only rework and human
review of the concrete unsafe implementation remain acceptance gates. No driver
acknowledgement, persistence-failure proof, full S3 acceptance, POSIX runtime proof,
commit, merge, release, or deployment is established by this focused evidence.
