# 0035 Atomic Windows process assignment to owned jobs

- Status: Accepted
- Implementation: not-started
- Date: 2026-10-31
- Author/executor: Codex bounded driver-control intake worker (draft); parent owns integration
- Reviewer: independent GPT-6-sol safety reviewer; clear as a proposal, not acceptance or runtime verification
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

## Verification and implementation evidence

No implementation or runtime validation is claimed. Before dependent S3 acceptance,
require real Windows fixtures for first-instruction descendant creation, cancellation
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
Human acceptance of this proposal precedes implementation. Concrete unsafe-code
review and the remaining #74 evidence/human gates precede full S3 acceptance.
