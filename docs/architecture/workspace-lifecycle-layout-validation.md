# Workspace lifecycle layout validation

S1 evidence for [#72](https://github.com/smota/meshloop/issues/72), parent
[#71](https://github.com/smota/meshloop/issues/71), accepted [ADR 0033](adr/0033-workspace-lifecycle.md).
This is a disposable layout experiment; no product defaults or actual project
worktree registrations were changed.

## Reproduction and environment

From repository root: `pwsh -NoProfile -File scripts/test-workspace-layout.ps1`.
Observed 2026-10-28 on base `2bac352fe3acf0aa0d87adb9ba62035503db7558`:
Windows NT 10.0.26300.0, PowerShell 7.6.5, Git 2.55.0.windows.5,
Cargo 1.98.0 (797e8a9bc 2026-08-05), rustc 1.98.0 (88d9e12ae 2026-08-18).
The script creates a fresh UUID beneath ignored
`.agent-runs/workspace-lifecycle/layout-spike/`. Each case uses its own Git repository,
minimal standalone Rust package with `[workspace]`, committed lockfile, and local
`core.longpaths=true`, `core.autocrlf=false`. There are no network dependencies.

## Results

| Layout | Absolute worktree path characters | Source path characters | Result |
|---|---:|---:|---|
| Nested `.meshloop/worktrees/run-0001` | 160 | 172 | Offline locked Rust build and all controls passed |
| External sibling `external-worktrees/run-0001` | 154 | 166 | Offline locked Rust build and all controls passed |
| Nested representative long root | 298 | Not reached | Git add failed, exit 128 |
| External representative long root | 292 | Not reached | Git add failed, exit 128 |

Both actual long-path attempts emitted `fatal: '$GIT_DIR' too big` from
`git worktree add --detach <path> HEAD`, even with `core.longpaths=true`.
These are measured attempted paths, not successful long-path builds. The experiment
does not isolate a Windows API limit from Git's administrative-path limit, identify
the exact threshold, or prove that all paths below 260 work. No Cargo build occurred
for failed additions. Switching to an external root helps only when it actually
shortens the selected absolute path.

For both ordinary layouts:

- `cargo build --offline --locked --manifest-path <worktree>/Cargo.toml` exited 0.
- Parent `git status --porcelain --untracked-files=all` stayed empty.
- `rg --files --hidden -g '!.git/**'` and `rg -l layout-spike-marker .`
  excluded the nested ignored worktree; the latter found only the parent source.
- Parent `cargo metadata --offline --locked --no-deps --format-version 1` returned
  one package/member, without recursively discovering the nested worktree.
- `git status --porcelain --ignored` recognized generated `target/`. The fixture
  preserved it until the explicit generated-output deletion step.
- Dirty tracked source, an untracked file and an unknown ignored file were all
  detected and preserved through classification. The test restored only its own
  known controls before teardown. This tests fixture classification, not a product
  cleanup implementation or an adversarial filesystem race.
- After checked removal of the fixture's generated `target/`, ordinary
  `git worktree remove <path>` succeeded without force. Worktree-list output exactly
  matched its initial registration and the parent remained clean. Long-path failures
  also left the registration list unchanged. All four disposable repositories were removed.

The final script exited 0 because all observation assertions passed; this **does not
mean universal Windows path support was verified**. JSON evidence is retained under
the invocation's ignored UUID directory (`report.json`); no fixture repository remains
in that successful invocation. Two development runs exposed a line-ending stat
refresh issue and the long Git path failure before the final script recorded these
limits. Their added worktree registrations were removed or never created, but two
development fixture directories remain ignored: `b40ee14e-5603-4c36-b329-29239fa15b60`
and `a4031879-343c-46d3-8347-2fd96a158da0`. Automatic approval review rejected their
recursive directory teardown commands as blocked by policy; no detailed reason was
provided. They are disposable scratch residue, not production registrations.

## Decision boundary

Ordinary nested layout is feasible in this Windows fixture. The representative long
path failed. On 2026-10-28 Samuel explicitly stated "ok, now ADR033 is accepted"
after reviewing the concrete ADR. The placement decision is resolved: nested layout
requires a fail-closed capability probe using the worktree-creating Git binary;
otherwise an explicitly configured shorter external root must also pass the probe.
There is no silent root switch or universal Windows path-length promise. ADR 0033
is Accepted with implementation not-started; acceptance does not change the measured
results or establish runtime verification. No WSL/POSIX experiment, full product workspace build,
runtime cleanup, GC/bundle durability, migration or concurrency verification was run.
