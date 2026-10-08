---
name: meshloop-plan
description: Turn origin-session intent into a Meshloop task graph via meshloop:planner. Never a generic /plan.
---

# /meshloop:plan

You are the **meshloop:origin** supervisor. Do not implement the work.

1. Know your origin: your harness name (`claude`, `codex`, `pi`, `grok`, `agy`) and this
   session's id. Meshloop takes them from `--origin-harness` / `--origin-session`, or from
   `MESHLOOP_ORIGIN_HARNESS` / `MESHLOOP_ORIGIN_SESSION`. `meshloop doctor --json` only
   echoes them back (`origin_harness`, `origin_session`); it does not detect them, so pass
   them explicitly.
2. Write conversation-scoped intent (techniques, scope, exclusions) to a file.
3. `meshloop plan` starts a planner worker. Follow **Starting workers** below. The command,
   with the values at the start of the line replaced:

```powershell
$h = "claude"; $s = "0f1e2d3c-session-id"; $d = "C:\Users\me\runs\issue-12"; meshloop plan --objective "Deliver issue 12" --intent-file "$d\intent.md" --config "$d\meshloop.toml" --out "$d\meshloop-plan.json" --json --origin-harness $h --origin-session $s
```

```bash
h=claude; s=0f1e2d3c-session-id; d="/home/me/runs/issue-12"; meshloop plan --objective "Deliver issue 12" --intent-file "$d/intent.md" --config "$d/meshloop.toml" --out "$d/meshloop-plan.json" --json --origin-harness "$h" --origin-session "$s"
```

Equivalent slash: `/meshloop:plan`. MCP: `meshloop_plan`.

4. Show the graph. Then `/meshloop:review-plan`: ask **Accept**, **Decline**, or **Adjust**.
   Do not call `meshloop run --accept-plan` until they accept.
5. Never use unprefixed `plan` / `planner` tools from other harnesses for this job.

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
