# Workspace lifecycle

`meshloop workspace` inspects ownership and, only with a matching digest, pins a verified legacy run or deletes eligible worktrees. It does not start workers, relocate an existing run, or accept a release.

Command reference:

```text
meshloop workspace inventory|preview|apply|adopt|maintain [--cleanup] [--apply] [--digest <hex>] [--graph <id>] [--config <path>] [--db <path>] [--worktree-base <path>] [--json]
```

`--graph` applies only to `apply --cleanup`. It limits removal to that graph's eligible entries; the digest still binds every current fact.

`--json` output uses root aliases and relative paths. Passing `--apply` without `--digest`, or a digest that is not 64 lowercase hex characters, is refused before any write.

## New runs

A run created by this version owns its worktrees (issue #142, ADR 0033 amendment of 2026-11-10). Before its first worktree is added, the run loop writes, under the repository fence:

- the fence file `meshloop-workspace-registry.json` in the Git common directory, and the worktree root under an alias (`worktrees`, or the alias already registered for that directory);
- the run's binding: its first generation, marked `producer = "run-loop"`, at `<root>/<graph-id>` (layout version 2), pinned to the accepted plan's SHA-256 and to the base branch the main checkout was on (`refs/heads/<name>`);
- one artifact record each for `integrate`, every attempt (`task-<n>-attempt-<m>`) and every deliverable (`integrate-<tag>`), before that worktree exists and only when nothing is at its path yet.

A run is bound only when its plan is accepted and nothing of it exists yet: no attempt, no `integrate` worktree, branch or directory, no generation, no run binding and no migration marker. `run --accept-plan` records the acceptance before it starts the run for that reason, and still refuses leftover `integrate` state first. A graph id that is not a portable relative path stays unowned. A run started on a detached HEAD or on a `meshloop/...` branch records no base branch and is never reclaimed (`base-ref-unknown`).

The fence is held across each registration and its worktree add, not for the whole run. When another command holds it, the error is `fence-busy`; a run stops with "another Meshloop command holds the repository fence" before it records an attempt. Resume it later.

`run`, `resume`, `cancel`, `accept` and `integrate` work on a new run as before. Its history stays in the run tables, and its binding is the generation, not a `workspace_run_bindings` row, so the ledger stays at schema version 8 and installed 0.3.0 binaries keep working. A run is pinned to its registered root: `run`, `resume`, `integrate` and deliverable builds with a different `--worktree-base` or `[workspace] root` are refused before they change anything.

A re-accepted plan (review, or an accepted `mutate-plan`) and every restart add a generation pinned to the new plan hash; a worktree is registered only under a generation that holds the accepted plan. Restart (`resume --restart`, `run --reset`) of a new run deletes nothing: it moves the run's events and attempts into the generation they ran in, so attempt ids are never reused.

Records are never deleted. A worktree that is gone retires its record with a reclamation fact: after cleanup or restart removed it, after a failed add, or when `resume` or `integrate --into` find neither the directory nor a Git registration (for example after a crash between registration and add). Retired records are not reported, selected or reused.

Inventory reports a new run's generation and worktrees as `ledger-owned` with lifecycle ids. Its Git worktree, configured-root entry and attempt row are reported once, under the ledger location. An attempt path with no live artifact record is reported unresolved and never selected. A fully reclaimed run is not reported.

## Reclaiming a landed run

A new run is reclaimable when every task is `Integrated` and each worktree's HEAD is an ancestor of the base branch recorded for the run, for example after `meshloop integrate --into main --accept-integrate`, or `git merge --ff-only meshloop/<graph>/integrate` on that branch. The checkout's current HEAD does not count: a detached HEAD or another branch that contains the work is not a landing. Each worktree must also pass every other cleanup rule below.

One digest-bound call reclaims the whole run:

```text
meshloop workspace preview --cleanup --json
meshloop workspace apply --cleanup --graph <graph-id> --digest <digest>
```

Apply removes each eligible worktree with `git worktree remove` (no `--force`) and retires its record. It then deletes that worktree's `meshloop/<graph>/...` branch with `git branch -d` (never `-D`), only when the branch still names the commit the digest bound and a fresh check finds that commit on the base branch. `git branch -d` also deletes that branch's reflog; the commits stay reachable from the base branch. When the run directory is then empty, it is removed with `remove_dir`. The run directory is reported as `run-container` and is never a `git worktree remove` target. A branch that is kept, or an empty run directory that cannot be removed, makes the result `partial` (reasons `branch-retained`, `run-dir-retained`), not `completed`.

`[workspace] auto_reclaim` (default `true`) does this automatically. After a `run` or `resume` that completes the graph, and after `integrate --into`, Meshloop runs the same preview and digest-bound apply for that run. It applies only when the run is owned, has a base branch, is landed and every entry of the run is eligible; otherwise it removes nothing and prints the reasons and the cleanup command. With `auto_reclaim = false`, the same point prints the exact command, including the digest once the run is landed.

Failed, cancelled and interrupted runs are retained with `run-failed`, `run-cancelled` or `run-incomplete`, even when their worktrees are clean. Inside a landed run, a failed attempt whose task was integrated through another attempt is reclaimed only when its worktree is clean and its commits are on the base branch; otherwise it is retained as `attempt-failed`. An unreadable ledger retains every entry.

### Known limitations

- A run whose worktree root is outside the repository is not reclaimed, because cleanup treats paths outside the checkout as unsafe. On Windows this includes an explicitly configured short external root.
- If cleanup stops between `git worktree remove` and `git branch -d`, the branch is left and no cleanup candidate covers it yet.
- Runs created before this version are never bound later; they still need `workspace adopt`, which needs an existing fence.

## Preview and apply

| Command | Writes |
|---|---|
| `workspace inventory` | No. Classifies the checkout. |
| `workspace preview` | No. Adoption preview (`apply` is false). |
| `workspace preview --cleanup` | No. Adds a cleanup preview to that report. |
| `workspace apply --cleanup --digest <hex>` | Only when the digest matches the current eligible set. Removes those eligible worktrees and the merged `meshloop/<graph>/` branch of each removed run-loop worktree, and retires their records. Status `partial` when a branch or the run directory is kept. `--graph <id>` limits it to one run. |
| `workspace adopt` | No. Reports what a later apply would pin or retain. |
| `workspace adopt --apply --digest <hex>` | Only when the digest matches and at least one generation is verified. Pins those generations in place. |
| `workspace maintain` | No. Prints the selection plan. |
| `workspace maintain --apply --digest <hex>` | Only when the plan's own digest matches and one candidate is selected. Deletes that one candidate through cleanup. |

A digest that does not match the current report writes nothing. Adoption then reports `status` `stale`. Cleanup apply reports `blocked` with reason `stale-facts` when there is no recovery journal for that digest.

`workspace maintain` uses its own digest. That digest is not the cleanup digest. When the operator opts in, the command recomputes the cleanup digest and passes only the one selected id.

## Configuration

`[workspace] auto_reclaim = true|false` (default `true`) controls automatic reclamation of landed new runs. See [Reclaiming a landed run](#reclaiming-a-landed-run).

New runs use `<repo>/.meshloop/worktrees/<run-id>` after a fail-closed Git layout probe that uses the same Git binary. The probe runs when a new worktree is added. Inventory, preview, adopt, and maintain do not run it.

Precedence for the new-run root is `--worktree-base`, then `[workspace] root` in the selected config (relative to the repository, or absolute), then the nested default. Omitting `[workspace]` keeps the nested default. The resolver does not create the directory.

Neither flag nor config moves a run that already has a persisted path. A failed probe does not fall back to `.meshloop-worktrees`. The operator sets an explicit shorter root instead. A leftover `.meshloop-layout-probe` directory in the parent fails later adds closed and is not deleted by the probe.

## Legacy compatibility

Adoption never trusts a directory name or a branch prefix. A generation is verified only when all of the following hold:

- the run has a SQL legacy-unbound marker
- the fence repository id matches the generation
- the registered root's native identity matches the live directory
- the relative path is inside that root and is not a reparse point, symlink escape, or withheld path

The binding keeps the existing alias and relative path. Files are not moved. Name-only runs, with no marker and no generation, stay `unverified-location` and are not bound. A run that already has a generation and no legacy marker is left alone; that includes every run created by this version.

Runs created before this version have no ownership records. They keep today's behaviour: the run loop never binds them later, inventory reports them `legacy-unbound`, and cleanup does not select them. Graphs with a `workspace_run_bindings` row (adopted or scoped runs) still refuse legacy `run`, `resume`, and `cancel`.

## Blockers

Adoption `status` values:

| Status | Meaning |
|---|---|
| `preview` | Report only. Nothing was written. |
| `adopted` | The matching digest pinned every verified generation in the report. |
| `stale` | The digest did not match, or a re-check inside the write failed. Nothing was committed. |
| `git-unavailable` | The checkout has no Git common directory. |
| `fence-missing` | `meshloop-workspace-registry.json` is absent. Adopt does not create it. |
| `ledger-missing` | The state database is absent. |

Retain reasons: `identity-unverified`, `unverified-location`, `path-missing`, `unsafe-path`, `path-inaccessible`, `ambiguous-root`. One failed generation retains the whole graph. Generations that disagree on the root alias are `ambiguous-root`.

## Recovery

Missing paths are reported and not recreated. Unknown files are reported and not deleted.

Cleanup removal is `git worktree remove` without `--force`. It does not run `git clean`, `git gc`, `git prune`, or `git reset`, and it does not delete the directory tree when Git removal fails. A matching apply journals the intent, then the Git command. If Git fails, apply journals a `blocked` row for that candidate, stops before any later candidate, and returns status `git-remove-failed` with reason `git-remove`. The command prints that report and exits 2. The public error code is `git-remove-failed`.

## Stable limits

These limits stay as they are.

- Once apply acquires the repository fence, it holds that fence until the call returns.
- A digest mismatch returns before the fence is acquired and before a journal row is inserted. An empty journal reports `blocked` with `stale-facts`. A journal that already exists reports `recovery-required`.
- Eligibility is per candidate. A generation can be eligible while a separate driver row is blocked because its process is active.
- The journal column `expected_after_token` is the constant `absent-unregistered`.
- `migrate_v6_to_v7` commits the version 7 schema, then `migrate_v7_to_v8` opens a second transaction. Each cleanup journal insert is its own autocommit statement.
- Workspace command failures use stable codes (`config-io`, `config-parse`, `config-harness`, `config-not-found`, `current-directory`, `io`, `corrupt`, `read-only`, `schema-upgrade-required`, `legacy-ownership-unresolved`, `plan-revision-mismatch`, `lifecycle-busy`, `fence-busy`, `transition-refused`). The codes omit local paths.

After a deletion, recovery completes the stored journal row. It does not recompute the candidate hash. There is no automatic repair that rebuilds a removed worktree or a missing ledger.

Adoption of an empty verified set, or a stale digest, returns the preview and does not open a write transaction.

## Retention

Maintenance is opt-in and daemonless. The planner considers at most 20 candidates, stops at 5 seconds of selection, and starts at most one deletion. It selects only a landed, clean, owned artifact with a known outcome and no dependency.

Failed and cancelled runs are retained. For new runs, cleanup itself also blocks every entry of a failed, cancelled or interrupted run (`run-failed`, `run-cancelled`, `run-incomplete`). Evidence is retained and does not expire. Age of seven days or more is listed as advisory and never selects or excludes a candidate. The CLI does not supply a timestamp, so age stays unset in `workspace maintain`. The CLI also does not mark worktree candidates as evidence; evidence retention is still the planner rule.

A cursor reports work left beyond the window or the deadline. The plan writes nothing unless `--apply` is set and the maintenance digest matches.

## Irreversible effects

`adopt --apply` with a matching digest deletes that run's legacy-unbound marker and inserts a run binding for the existing alias, relative path, and layout version. `active_generation` and `current_revision` stay unset. The files on disk stay where they are.

`apply --cleanup` can remove every eligible worktree, and `maintain --apply` one. For a new run, each removal also deletes that worktree's merged `meshloop/<graph>/` branch and, once empty, the run directory. Automatic reclamation does the same for one landed run. These removals are not undone by another workspace command. The commits stay reachable from the branch they landed on.

Unknown files, missing paths, evidence, failed runs, cancelled runs and interrupted new runs are not deletion candidates.

## Rollback limits

Refusing `--apply`, or passing a digest that does not match, is the operator's cancellation. It writes nothing.

This slice has no command that removes a binding created by adoption, restores a removed worktree, or moves a persisted run back to another root. Changing `--worktree-base` or `[workspace] root` affects only later new runs.

Local `xtask check` and `xtask smoke` do not accept a merge, a GitHub release, or a crates.io publish. Windows evidence for this scenario is the CI Gate `windows-latest` job. A Linux workstation does not count as a Windows run.

## Protected data

The disposable fixture used by `xtask smoke` and the CLI compatibility test plants `protected-data.txt` and an unknown artifact next to a verified generation (`kept.txt`), a missing generation path, and a name-only run. Preview, a stale digest, verified adoption, cleanup preview, and maintenance must leave the protected bytes in place and must not create the missing path or a new worktree root.
