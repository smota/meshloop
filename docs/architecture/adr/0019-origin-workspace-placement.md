# 0019 Origin cockpit and Meshloop-owned Herdr space

- Status: Accepted
- Implementation: implemented
- Date: 2026-09-08
- Accepted: 2026-09-08
- Author/executor: Grok, under human product direction
- Decision owner: Samuel
- Approval evidence: user direction 2026-09-08 to implement a dedicated Herdr space for live panes and keep origin as the monitor; follows the 2026-09-08 finding that `split_pane` stole a pane from another project workspace
- Supersedes: none
- Superseded by: none

## Context and constraints
Live workers are Herdr 0.8 panes (ADR 0017). `agent start` needs an empty shell pane;
origin already hosts the supervisor agent, so a second live agent cannot run *in*
`origin_session`. Isolation remains git worktrees (ADR 0005), not panes.

R1 `ReviewTransport::split_pane` listed every pane on the server and split the first
id that was not origin. With several Herdr spaces, that stole a pane from another
project (`w6` / movetheneedle) while origin (`w7` / agentflow-demo) was the only pane
in its space. Docs said “a planner pane appears elsewhere.” Elsewhere must not mean
another product space, and it must not mean splitting origin.

Operators asked to keep this pane as the cockpit (no splits or extra tabs in the
origin space) and to put planner / worker / reviewer panes in a space Meshloop owns.

`--current` is not an authority: Meshloop is a subprocess and is not the focused pane.
`--origin-session` / `MESHLOOP_ORIGIN_SESSION` names the supervisor only.

## Alternatives
A: Split a sibling pane in origin’s workspace; `tab create --workspace <origin>` when
origin is the only pane. Rejected — it still mutates the operator’s space.
B: Split origin when it is the only pane. Rejected — conflicts with OriginPane.
C: Document “elsewhere = any non-origin pane on the server.” Rejected — the incident.
D: Dedicated Meshloop Herdr workspace (`workspace create --no-focus`), origin
untouched, origin monitors via CLI/JSON. Selected.
E: Headless subprocess workers with no Herdr panes. Deferred — loses `blocked` TUI
and recoverable Herdr sessions; fixture remains the CI double.

## Decision
1. Origin space topology is immutable. Never `pane split` origin, never
   `tab create` in origin’s workspace, never `--current`.
2. Live planner, worker, and reviewer panes live in a Meshloop-owned Herdr
   workspace labeled `meshloop-<repo>`, created with
   `herdr workspace create --cwd <worktree> --label meshloop-<repo> --no-focus`.
   Reuse that workspace for later agents in the same target repo (cache
   `.meshloop/loop-space.json`, or look up by label). Git worktrees stay Meshloop’s;
   do not call `herdr worktree create`.
3. Additional agents in that space use
   `herdr tab create --workspace <loop-ws> --cwd <worktree> --no-focus`, then
   `agent start --pane <new>`. A freshly created workspace’s root pane may host the
   first agent.
4. Fail closed if `origin_session` is missing on a live path. The loop workspace id
   must differ from origin’s workspace id.
5. Origin is the monitor: `--no-focus` stays; plan/status JSON include `workspace_id`
   and `pane_id`. Operators visit the Meshloop space only for a blocked TUI.
6. `herdr agent prompt` delivers the prompt without `--wait`. See ADR 0020:
   Meshloop waits while the pane is `working`/`blocked`; the 5s stall gate is
   not a crash. If wait fails but the planning worktree already has a valid
   `meshloop-plan.json`, salvage it and still write origin `--out`.

## Consequences
`with_workspace` as an unused hint on the origin adapter is not the mechanism; the
loop workspace is created or reused explicitly. Global `herdr pane list` is not a
placement source. Closing the Meshloop workspace on cancel/integrate is layout-only
and is not required in this change (git worktrees remain). Mechanism detail:
runtime-design.md §3 and `crates/meshloop-adapters/src/herdr.rs`.

## Verification
Unit: pane id → workspace id; create/tab args include `--no-focus` and never
`--current`; label `meshloop-<repo>`; create JSON parse; origin pane is not a split
source. Live (when Herdr is up): origin workspace pane_count may be 1; after
`split_pane`, the new pane’s `workspace_id` differs from origin’s and equals the
cached loop space; origin pane id is unchanged. `xtask check`; `xtask live` still
fails if Herdr is down.
