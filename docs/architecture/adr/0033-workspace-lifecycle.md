# 0033 Workspace lifecycle and durable history

- Status: Accepted
- Implementation: not-started
- Date: 2026-10-28
- Accepted: 2026-10-28
- Author/executor: Codex (S1 builder and integrating supervisor)
- Reviewer: two Grok native CLI reviews; final requested/resolved grok-4.7-build-fast, medium effort; first fallback model unverified (requested grok-4.7 unavailable); final finding addressed by parent and builder, no independent pass claimed
- Decision owner: Samuel
- Approval evidence: Samuel explicitly stated "ok, now ADR033 is accepted" on 2026-10-28 after reviewing this concrete decision
- Supersedes: only ADR 0021's worktree placement and destructive history-reset semantics; narrowly amends runtime-design §4's prohibition on persisted paths outside the repository for a protected local operational root registry
- Superseded by: none
- Tracking: [#71](https://github.com/smota/meshloop/issues/71), S1 [#72](https://github.com/smota/meshloop/issues/72)

## Context and constraints

Worktrees, generated build output, run history and recovery currently lack a unified
ownership and safe-retirement contract. Restart must retain its accepted plan while
preserving earlier execution generations. ADRs 0002, 0005, 0007, 0019 and 0025 remain
in force: dependency boundaries, isolated writers, durable evidence, origin cockpit
and process-tree reconciliation are unchanged. This decision does not implement them.

## Alternatives

Keep the external sibling layout of ADR 0021; use repository-local
`.meshloop/worktrees/<run-id>/...`; or require a configured external short root.
Repository-local placement groups operational artifacts and follows existing ignore
rules. A short external root can reduce path pressure but does not itself guarantee
Windows support. The [Windows fixture](../workspace-lifecycle-layout-validation.md)
passed worktree paths of 160/154 characters and failed attempted paths of 298/292
characters. The intervening range is unmeasured. The accepted placement is nested
placement on supported paths, mandatory fail-closed path-capability preflight, and an
explicitly configured short external root when needed; there is no universal Windows
path-length promise.

## Decision

The following is accepted; implementation remains not-started:

1. Default to nested `.meshloop/worktrees/<run-id>/...` only after a fail-closed
   capability probe using the same Git binary that creates the worktree succeeds.
   Otherwise require an explicitly configured shorter external root that also passes
   that probe; never silently switch roots or infer support from a numeric threshold.
   Pin the chosen canonical worktree root per run and record layout/schema version
   and immutable generations. Existing runs keep their original roots. Repository
   identity combines canonical common-directory filesystem identity and a stored
   repository UUID, not path-string equality alone. An exclusive repository fence
   lives there, outside removable artifacts, with one authoritative ledger binding
   shared across configurations. Alternate database paths must attach that same
   authority or refuse writable ownership/lineage; they cannot start independent history.
2. Restart creates a new generation pinned to the accepted plan's content hash;
   a changed hash requires a new plan. Keep prior events,
   attempts, evidence and generation links; never clear the historical ledger.
   Distinguish graph, generation, attempt and artifact identities. This replaces only
   the destructive-history part of ADR 0021, preserving explicit restart authorization.
3. Cleanup is a preview followed by explicit apply, bound to a digest of repository
   identity, generation, ownership, paths, Git/ref state, liveness and classification.
   Journal each step's expected before/after state and explicitly allowed own
   transitions; cleanup's own authorized changes do not invalidate its remaining
   plan. Revalidate before each mutation; unexpected drift stops all remaining steps.
   Recovery verifies every committed step against actual state before continuation.
   Stale state blocks; external-edit
   races remain possible between observation and filesystem mutation and must be
   reported rather than represented as eliminated by a digest.
4. Inventory every untracked/ignored entry, including reparse points. Dirty tracked
   files/index, unknown entries, uncertain ownership, unresolved registration, locks,
   or live/uncertain processes block cleanup. Generated output requires a versioned
   allowlist plus ownership/provenance, not a `target` name alone, and explicit narrow
   deletion authorization. Revalidate canonical containment and path/file identity
   before each deletion; unsafe or uncertain reparse traversal blocks. Re-inventory
   after explicit output removal. Ordinary Git worktree removal is last and requires
   zero remaining untracked or ignored entries, including recognized generated
   output not explicitly removed. Every remaining entry must be a pristine tracked
   regular file/directory or verified Git metadata; any remaining reparse point,
   symlink or uncertain file identity blocks removal. Safe support for those entries
   requires a separate decision. Journal the final action's exact authorized residual
   tree. Never use blanket force, broad
   ignored-file deletion, or harness success as safety evidence.
5. Before any destructive step, preserve every ledger/recovery/artifact-required
   object ID and object closure in retained refs outside retiring artifacts or a local
   bundle independently restored and checked in a disposable repository. Assess
   artifact-specific stash/reflog dependencies; do not copy unrelated global refs or
   alter them. Old generations keep their required closures pinned. Prove reachability
   with non-pruning queries; GC durability experiments run only in isolated fixtures,
   never production `gc`/`prune`. Metadata alone is not proof. Optional retention of
   extra landed artifacts is opt-in; evidence and its required objects never expire
   by age. Dropping cited history requires separate destructive authorization.
6. Narrowly amend runtime-design §4's prohibition on persisting paths outside the
   repository: absolute external roots may exist only in a protected local operational
   root registry needed for reconciliation, including canonical common-dir location.
   Use root aliases and relative paths in
   the ledger, evidence, events and default JSON; never include absolute external
   roots in events, evidence, logs or exports. Root aliases are stable operational
   references, not public identities. Preserve ADR 0007's no-secrets promise: no
   credentials, raw sessions, prompts or source content in durable telemetry.
   Durability bundles may contain source: keep them local and uncommitted, excluded
   from all evidence, telemetry, log and report exports.
7. Migration is forward-only and transactional; existing roots/history are recorded
   without destructive relocation. Older binaries must refuse incompatible schemas.
   Recovery reconciles persisted intent with actual Git and process state before
   resumption or retirement. Path preflight must expose Windows limits and require
   an explicit alternative root when the selected layout cannot be created.

## Consequences

No runtime defaults, cleanup behavior, schema or CLI changes occur in S1. This
acceptance updates the documented placement, history and privacy decisions;
their runtime implementation remains outstanding. The fixture is a
Git/Cargo layout experiment, not product cleanup, migration,
concurrency or durability verification. No cross-platform support claim follows.
Implementation boundaries remain the full #71 plan: repository fencing/identity,
generation history, safe preview/apply, recovery, migration, retention and integration
validation must be implemented and reviewed as separately owned slices.

## Verification and implementation evidence

S1 [layout validation](../workspace-lifecycle-layout-validation.md) records actual
commands, tool versions, path lengths, controls and failures on base `2bac352`.
The human placement decision is resolved by the acceptance recorded above, subject
to the measured Windows limits and mandatory capability preflight. Acceptance is
not runtime verification. S2 begins the identity/fence and durable generation
contract from #71; no S2 or later product implementation is claimed here.
