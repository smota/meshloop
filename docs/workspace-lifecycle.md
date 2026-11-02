# Workspace lifecycle

`meshloop workspace` inspects ownership and, only with a matching digest, pins a verified legacy run or deletes one eligible worktree. It does not start workers, relocate an existing run, or accept a release.

Command reference:

```text
meshloop workspace inventory|preview|apply|adopt|maintain [--cleanup] [--apply] [--digest <hex>] [--config <path>] [--db <path>] [--worktree-base <path>] [--json]
```

`--json` output uses root aliases and relative paths. Passing `--apply` without `--digest`, or a digest that is not 64 lowercase hex characters, is refused before any write.

## Preview and apply

| Command | Writes |
|---|---|
| `workspace inventory` | No. Classifies the checkout. |
| `workspace preview` | No. Adoption preview (`apply` is false). |
| `workspace preview --cleanup` | No. Adds a cleanup preview to that report. |
| `workspace apply --cleanup --digest <hex>` | Only when the digest matches the current eligible set. Removes those eligible worktrees. |
| `workspace adopt` | No. Reports what a later apply would pin or retain. |
| `workspace adopt --apply --digest <hex>` | Only when the digest matches and at least one generation is verified. Pins those generations in place. |
| `workspace maintain` | No. Prints the selection plan. |
| `workspace maintain --apply --digest <hex>` | Only when the plan's own digest matches and one candidate is selected. Deletes that one candidate through cleanup. |

A digest that does not match the current report writes nothing. Adoption then reports `status` `stale`. Cleanup apply reports `blocked` with reason `stale-facts` when there is no recovery journal for that digest.

`workspace maintain` uses its own digest. That digest is not the cleanup digest. When the operator opts in, the command recomputes the cleanup digest and passes only the one selected id.

## Configuration

New runs use `<repo>/.meshloop/worktrees/<run-id>` after a fail-closed Git layout probe that uses the same Git binary. The probe runs when a new worktree is added. Inventory, preview, adopt, and maintain do not run it.

Precedence for the new-run root is `--worktree-base`, then `[workspace] root` in the selected config (relative to the repository, or absolute), then the nested default. Omitting `[workspace]` keeps the nested default. The resolver does not create the directory.

Neither flag nor config moves a run that already has a persisted path. A failed probe does not fall back to `.meshloop-worktrees`. The operator sets an explicit shorter root instead. A leftover `.meshloop-layout-probe` directory in the parent fails later adds closed and is not deleted by the probe.

## Legacy compatibility

Adoption never trusts a directory name or a branch prefix. A generation is verified only when all of the following hold:

- the run has a SQL legacy-unbound marker
- the fence repository id matches the generation
- the registered root's native identity matches the live directory
- the relative path is inside that root and is not a reparse point, symlink escape, or withheld path

The binding keeps the existing alias and relative path. Files are not moved. Name-only runs, with no marker and no generation, stay `unverified-location` and are not bound. A run that already has a generation and no legacy marker is left alone.

Bound graphs still refuse legacy `run`, `resume`, and `cancel`.

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

Cleanup removal is `git worktree remove` without `--force`. It does not run `git clean`, `git gc`, `git prune`, or `git reset`, and it does not delete the directory tree when Git removal fails. A matching apply can journal a blocked candidate before that Git command. If Git then fails, the process exits with an error and the partial report may not be printed.

After a deletion, recovery completes the stored journal row. It does not recompute the candidate hash. There is no automatic repair that rebuilds a removed worktree or a missing ledger.

Adoption of an empty verified set, or a stale digest, returns the preview and does not open a write transaction.

## Retention

Maintenance is opt-in and daemonless. The planner considers at most 20 candidates, stops at 5 seconds of selection, and starts at most one deletion. It selects only a landed, clean, owned artifact with a known outcome and no dependency.

Failed and cancelled runs are retained. Evidence is retained and does not expire. Age of seven days or more is listed as advisory and never selects or excludes a candidate. The CLI does not supply a timestamp, so age stays unset in `workspace maintain`. The CLI also does not mark worktree candidates as evidence; evidence retention is still the planner rule.

A cursor reports work left beyond the window or the deadline. The plan writes nothing unless `--apply` is set and the maintenance digest matches.

## Irreversible effects

`adopt --apply` with a matching digest deletes that run's legacy-unbound marker and inserts a run binding for the existing alias, relative path, and layout version. `active_generation` and `current_revision` stay unset. The files on disk stay where they are.

`apply --cleanup` and `maintain --apply` can remove one eligible worktree. That removal is not undone by another workspace command.

Unknown files, missing paths, evidence, failed runs, and cancelled runs are not deletion candidates.

## Rollback limits

Refusing `--apply`, or passing a digest that does not match, is the operator's cancellation. It writes nothing.

This slice has no command that removes a binding created by adoption, restores a removed worktree, or moves a persisted run back to another root. Changing `--worktree-base` or `[workspace] root` affects only later new runs.

Local `xtask check` and `xtask smoke` do not accept a merge, a GitHub release, or a crates.io publish. Windows evidence for this scenario is the CI Gate `windows-latest` job. A Linux workstation does not count as a Windows run.

## Protected data

The disposable fixture used by `xtask smoke` and the CLI compatibility test plants `protected-data.txt` and an unknown artifact next to a verified generation (`kept.txt`), a missing generation path, and a name-only run. Preview, a stale digest, verified adoption, cleanup preview, and maintenance must leave the protected bytes in place and must not create the missing path or a new worktree root.
