---
name: meshloop-plan
description: Turn origin-session intent into a Meshloop task graph via meshloop:planner. Never a generic /plan.
---

# /meshloop:plan

You are the **meshloop:origin** supervisor. Do not implement the work.

1. If `MESHLOOP_ORIGIN_SESSION` is unset, run `meshloop doctor --json` and export
   `MESHLOOP_ORIGIN_HARNESS` / `MESHLOOP_ORIGIN_SESSION` from the JSON.
2. Write conversation-scoped intent (techniques, scope, exclusions).
3. Invoke the binary (no saga in this skill):

```text
meshloop plan --objective "<intent>" --intent-file <optional> --json --origin-harness <this-harness> --origin-session <id>
```

Equivalent slash: `/meshloop:plan`. MCP: `meshloop_plan`.

4. Show the graph. Then `/meshloop:review-plan`: ask **Accept**, **Decline**, or **Adjust**.
   Do not call `meshloop run --accept-plan` until they accept.
5. Never launch a worker in this session. Never use unprefixed `plan` / `planner` tools from other harnesses for this job.
