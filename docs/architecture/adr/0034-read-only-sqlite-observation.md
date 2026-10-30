# 0034 Read-only SQLite observation

- Status: Proposed
- Implementation: not-started
- Date: 2026-10-30
- Author/executor: Codex (bounded investigation and proposal)
- Reviewer: pending parent review and human decision
- Approval evidence: none; this proposal does not change the current contract
- Supersedes: none
- Related: [0033](0033-workspace-lifecycle.md), [issue #74](https://github.com/smota/meshloop/issues/74)
- Superseded by: none

## Context and constraints

The S3 design requires status/inspect/watch to open an existing ledger without any
source filesystem writes, including WAL/SHM creation. An absent ledger must return
no managed runs without creating directories. Readers must also see committed live
WAL data while another process owns the repository fence. Readers cannot migrate
schemas, initialize database/repository identity, register roots, or adopt runs.

At source revision `c114418432263a1c4968d8230be65b52083fffaf`,
`workspace/runtime.rs:296` opens SQLite with `SQLITE_OPEN_READ_ONLY`, then queries
the schema. That flag protects database writes but does not ensure zero sidecar
effects. Bundled SQLite (`libsqlite3-sys 0.28.0`, `sqlite3/sqlite3.c:66198-66205`)
opens the WAL with `READWRITE | CREATE`; its default shared-memory implementation
also opens SHM with `READWRITE | CREATE` (`50219-50232` on Windows,
`42720-42731` on Unix). These are inspected source facts, not a new behavioral test.

The URI parameter `readonly_shm=1` selects read-only SHM handles in those VFS
implementations, but does not eliminate WAL creation and cannot open missing SHM
universally. It is not a complete fix. Backup/serialize still require opening and
reading the source connection. `immutable=1` on a live source disables required
change detection and is unsuitable. Unlocked file copying does not establish a
consistent database/WAL snapshot.

## Alternatives

1. **Retain strict zero source writes.** Design and prove a portable read-only VFS
   or SQLite-compatible locking/snapshot mechanism. Merely rejecting VFS writes may
   prevent WAL recovery or wal-index initialization; a private wal-index or other
   mechanism may be necessary. No complete solution has been demonstrated here.
   Any new dependency requires concrete review. Unsafe Rust/FFI requires an accepted
   decision, documented safety invariants, focused tests, and explicit review under
   project policy. This option needs further design and cross-platform validation.
2. **Permit SQLite-managed observation sidecars.** Refine the read-only contract
   to allow SQLite's normal WAL/SHM creation and coordination solely for observation.
   Retain a read-only database connection, no migration or journal-mode setup, and
   no application schema/data, identity, repository registry, root, or adoption
   writes. Preserve SQLite's normal live WAL visibility and locking. This option
   explicitly gives up byte-for-byte/entry-for-entry source filesystem immutability;
   SQLite may update coordination state or remove sidecars as connections close.

## Decision or proposal

Recommend option 2 for human consideration: it uses the existing persistence stack
and keeps the remaining S3 work bounded given the user's capacity concern. This is
a proposed contract refinement, not evidence that the current reader meets S3.
Option 1 remains available if zero source writes is a required product property.

The current strict contract and frozen oracle remain authoritative until the human
accepts a concrete choice and the owning issue, design, oracle, and implementation
are updated through the governed workflow. Neither this document nor a commit of
it authorizes implementation, a new dependency, or unsafe code.

## Consequences

Option 2 would require precise operator wording about SQLite sidecars and revised
tests that still reject all application mutations. Filesystem permission failures
must remain explicit; readers must never fall back to immutable live-source reads
or silently omit committed WAL data. Option 1 retains stronger filesystem semantics
but adds unresolved design, review, portability, and maintenance costs. Neither
option permits lifecycle authority initialization during observation.

## Verification and implementation evidence

Required proofs after a decision: absent DB creates nothing; supported existing
schemas remain readable; old schemas return `SchemaUpgradeRequired` unchanged;
closed WAL databases work; independent-process readers see committed live WAL data
while a writer owns the fence; forbidden schema/domain/identity/root writes fail.
Option 1 additionally needs exact source filesystem immutability under races;
option 2 needs assertions distinguishing allowed sidecars from forbidden writes.

On 2026-10-30, the parent reproduced the current blocker on Windows:
`cargo test --locked -p meshloop-adapters lifecycle_fence_readonly_absent_db_and_pre_v6_upgrade_required -- --exact workspace::tests::lifecycle_fence_readonly_absent_db_and_pre_v6_upgrade_required`.
Result: **0 passed, 1 failed, 0 ignored**, at `workspace/tests.rs:1199`;
the reader added `readonly5.sqlite-shm` and `readonly5.sqlite-wal`.
Separately, `cargo test --locked -p meshloop-adapters --lib -- --skip lifecycle_fence_readonly_absent_db_and_pre_v6_upgrade_required`
reported **44 passed, 0 failed, 1 ignored, 1 filtered**. This excludes the blocker.

Separately, the parent executed on Windows at the exact source revision above:

```text
cargo test --locked -p meshloop-adapters retained_schema5_binary_refuses_schema6_without_mutation -- --ignored
```

Actual result: **1 passed, 0 failed, 0 ignored**. Retained v5 fixture SHA-256:
`e6a81cb444e5714e142620995de0efc9768fb5981339f02defe3097cf4d3b233`.
This proves the retained old binary refusal regression only; it does not prove
read-only observation or S3 completion. No VFS, snapshot, or contract refinement
has been implemented by this proposal.
