# Getting started

Install and first-time setup: **[Install](install.md)**
(`cargo install meshloop-cli --locked`, then `meshloop bundle`).

**This pane stays.** You are in Claude Code, Codex, Pi, Grok, or Agy inside
Herdr 0.8. You are `meshloop:origin`. Meshloop creates a **separate Herdr
space** (`meshloop-<repo>`) for planner, worker, and reviewer panes. Origin
is not split and gets no extra tabs. If work starts in *this* pane, stop.

The origin agent asks **Accept / Decline / Adjust in English**. You may answer
in Portuguese (`aceitar` / `recusar` / `ajustar`). You should not have to type
flags.

**Run the loop in the throwaway target repo**, not in the Meshloop product
clone. WSL2 and prebuilt GitHub Release binaries: unverified. Concurrency is
1. Fixture = [CI appendix](#ci-appendix-fixture-double).

## What you should see

| State | What you should see | What you do |
|---|---|---|
| Ready | `herdr_server_running: true`, `origin_session` = **this** pane, no new split | Set `MESHLOOP_ORIGIN_*`, then `/meshloop:plan` |
| Herdr down | `herdr_server_running: false`. Zero new panes | Start Herdr 0.8. Do not plan |
| Plan in flight | Planner in the **Meshloop space**, not this pane; `meshloop-plan.json` | Stay. Wait for the 3-way question |
| Accept | No worker yet. Status `PlanAccepted` | `/meshloop:run` |
| Decline | Status `PlanDeclined`. No worker. `run` refuses | Stop, or Adjust — never `run --accept-plan` |
| Adjust | Planner pane **again**; still awaiting review; question repeats | Answer again |
| Run | **Worker** in the Meshloop space + extra worktree; **this branch unchanged** | Stay. Status overlays Herdr `live`. `/meshloop:accept` then `/meshloop:resume` |
| Origin mistake | Planner/worker/reviewer **in this pane** | Cancel. Fix origin. Do not continue |
| FailedTerminal | Same accepted plan, nothing Ready | `meshloop resume --restart` (not a new `graph_id`) |

`resume` merges into the **integrate worktree**. Only
`meshloop integrate --into --accept-integrate` lands on a branch you name.

## 0. Already installed?

From [Install](install.md) you should have: `meshloop` on `PATH`, a throwaway
git **target** repo with `meshloop.toml`, the session pack (`meshloop bundle`),
Herdr 0.8 running, and this origin pane opened **in that target repo**.

```text
herdr status
meshloop --version
meshloop doctor --json
```

Default store: `.meshloop/state.sqlite`. Worktrees:
`<repo-parent>/.meshloop-worktrees/`. No credentials in config. No Windows
service. Every `/meshloop:*` runs in the target repo.

## 1. Doctor — name this pane

Slash: `/meshloop:doctor`.

You want `herdr_server_running: true`, Herdr 0.8.x, `origin_session` = this
pane. If Herdr is down, tell the user and **stop**.

```text
MESHLOOP_ORIGIN_HARNESS=codex
MESHLOOP_ORIGIN_SESSION=w3:p1
```

Later slash skills inject these. Doctor does not split panes.

## 2. Plan

Stay supervisor. Intent can be conversation text or a `.md` (`--intent-file`).

Slash: `/meshloop:plan`. A **planner** pane appears in the Meshloop space, not
this pane. Meshloop checks
the graph is a DAG, not whether the breakdown is wise. Nothing is scheduled.

## 3. Review-plan — wait for their answer

Slash: `/meshloop:review-plan`. Ask once, in English:

> **Accept** this graph (then we can run), **Decline** it (stop, no worker), or
> **Adjust** (planner runs again).

Wait. Then send **exactly one** flag. Do not show CLI before they choose.

| Reply | Stored | Next |
|---|---|---|
| Accept / aceitar | `PlanAccepted` | `/meshloop:run` |
| Decline / recusar | `PlanDeclined` | `run` refuses until a later Accept |
| Adjust / ajustar | still `AwaitingPlanReview` | planner again; ask again |

CLI shortcut (debug, not the skill default): `meshloop run --plan … --accept-plan`.

## 4. Run

Only after Accept. Slash: `/meshloop:run`.

A **worker** pane + attempt worktree. Your current branch does not move.
Verification is the git diff against the attempt base.

```text
# node gate — not the plan gate
/meshloop:accept   →  meshloop accept --task 1 --as sam
/meshloop:resume
```

`you` is not a magic identity.

## 5. Optional reviewers, then land

`/meshloop:orchestrate` — two `meshloop:reviewer` panes in the Meshloop space,
never origin. Synthesis is advisory. Node accept is still required.

```text
meshloop integrate --graph <id> --into <ref> --accept-integrate
```

Without `--accept-integrate`, refuse. Leftovers: `.meshloop/` and
`.meshloop-worktrees/`.

<details>
<summary>Raw engine CLI (debug)</summary>

Skills inject `--json --origin-harness --origin-session`. Typed by hand:

```text
meshloop doctor --json
meshloop plan --objective "<intent>" --json --origin-harness <this> --origin-session <id> --config meshloop.toml
meshloop review-plan --plan meshloop-plan.json --accept --as sam --json
meshloop run --plan meshloop-plan.json --json --config meshloop.toml
```

</details>

## CI appendix (fixture double)

Offline `xtask check` uses a dummy worker, not Claude or Codex:

```text
cargo build -p meshloop-adapters --bin fixture_harness
meshloop plan --objective "Add a hello.txt file" --config config/meshloop.fixture.toml
meshloop run --plan meshloop-plan.json --accept-plan --config config/meshloop.fixture.toml --fixture-only
```

## Next

- [Install](install.md)
- [Docs hub](README.md)
- [Product brief](product/brief.md)
- [Skills](../skills/README.md)
