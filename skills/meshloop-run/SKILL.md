---
name: meshloop-run
description: Execute an accepted Meshloop graph. Supervisor-only origin. Slash /meshloop:run.
---

# /meshloop:run

If `MESHLOOP_ORIGIN_SESSION` is unset, run `/meshloop:doctor` first.

**Only after** the user Accepted via `/meshloop:review-plan` (`PlanAccepted`).
If the engine says the plan is not accepted, ask `/meshloop:review-plan` — do
**not** add `--accept-plan`.

```text
meshloop run --plan <file> --json --origin-harness <h> --origin-session <id>
```

A **worker pane must open in the Meshloop Herdr space**, plus a git worktree.
This origin pane must not implement the work. Current branch stays put.

Do not poll. Read the JSON idle reason; then `/meshloop:status` or
`/meshloop:accept`. `--fixture-only` is CI only.

`--accept-plan` on `run` is a debug shortcut that skips the 3-way question.
Do not use it from this skill.

If the engine says the graph already exists / FailedTerminal, do **not** invent a
new `graph_id`. The operator can re-run this accepted plan with:

```text
meshloop resume --restart --json --origin-harness <h> --origin-session <id>
```

or `meshloop run --plan <file> --reset`. `--restart` and `--retry` are exclusive.
