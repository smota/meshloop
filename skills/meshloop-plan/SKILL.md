---
name: meshloop-plan
description: Turn origin-session intent into a Meshloop task graph via meshloop:planner. Never a generic /plan.
---

# /meshloop:plan

You are the **meshloop:origin** supervisor. Do not implement the work.

1. Write conversation-scoped intent (techniques, scope, exclusions).
2. Invoke the binary (no saga in this skill):

```text
meshloop plan --objective "<intent>" --intent-file <optional> --json --origin-harness <this-harness> --origin-session <id>
```

Equivalent slash: `/meshloop:plan`. MCP: `meshloop_plan`.

3. Show the graph to the user. Do not call `meshloop run --accept-plan` until they agree.
4. Never launch a worker in this session. Never use unprefixed `plan` / `planner` tools from other harnesses for this job.
