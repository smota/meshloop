---
name: meshloop-run
description: Execute an accepted Meshloop graph. Supervisor-only origin. Slash /meshloop:run.
---

# /meshloop:run

Pass your origin explicitly (`--origin-harness`, `--origin-session`, or
`MESHLOOP_ORIGIN_HARNESS` / `MESHLOOP_ORIGIN_SESSION`); `/meshloop:doctor` only echoes
them back.

**Only after** the user Accepted via `/meshloop:review-plan` (`PlanAccepted`).
If the engine says the plan is not accepted, ask `/meshloop:review-plan`. Do
**not** add `--accept-plan`.

`run` starts workers. Follow **Starting workers** below. The command, with the values at
the start of the line replaced (`--detach` returns at once and keeps workers running):

```powershell
$h = "claude"; $s = "0f1e2d3c-session-id"; $d = "C:\Users\me\runs\issue-12"; meshloop run --plan "$d\meshloop-plan.json" --config "$d\meshloop.toml" --detach --json --origin-harness $h --origin-session $s
```

```bash
h=claude; s=0f1e2d3c-session-id; d="/home/me/runs/issue-12"; meshloop run --plan "$d/meshloop-plan.json" --config "$d/meshloop.toml" --detach --json --origin-harness "$h" --origin-session "$s"
```

A **worker executes via direct CLI in an ephemeral git worktree**.
This origin pane must not implement the work. Current branch stays put.

Do not poll `inspect`. Block with `watch`, which starts nothing, so you can run it
yourself (NDJSON state changes; last line such as `{"exit":"awaiting_review","nodes":[2]}`; exit 0 =
needs a human or completed, 1 = failed/cancelled, 3 = `--timeout`):

```bash
meshloop watch --graph issue-12 --config "/home/me/runs/issue-12/meshloop.toml" --json
```

Then `/meshloop:status` or `/meshloop:accept`. `--fixture-only` is CI only.

`--accept-plan` on `run` is a debug shortcut that skips the 3-way question.
Do not use it from this skill.

If the engine says the graph already exists / FailedTerminal, do **not** invent a
new `graph_id`. Re-run the accepted plan with `resume --restart` (it starts workers,
so the same rules apply), or `run --plan ... --reset`. `--restart` and `--retry` are
exclusive:

```bash
h=claude; s=0f1e2d3c-session-id; d="/home/me/runs/issue-12"; meshloop resume --restart --config "$d/meshloop.toml" --json --origin-harness "$h" --origin-session "$s"
```

## Starting workers

`plan`, `run`, `resume` and `review-plan --adjust` start unattended agents that write
files. Your harness's permission check may refuse to let you run them. That refusal is
correct: do not retry through another command, a script, or a different tool. Either:

- the user runs the command in their own terminal, or
- the user adds a permission rule that allows these Meshloop commands, then asks you to
  continue.

When the user runs it, hand over **one line they can paste**:

- Substitute every value. Use absolute paths in double quotes.
- No `<...>` placeholders: PowerShell parses `<` as a redirection operator.
- Match their shell. PowerShell sets values with `$d = "C:\..."; meshloop ...`; POSIX
  shells with `d="/..."; meshloop ...`.

You can always run the commands that start nothing: `doctor`, `status`, `inspect`,
`watch` and `roles`. `review-plan --accept|--decline` and `accept` record a person's
decision: run them only after that person answers, and name them with `--as`.
