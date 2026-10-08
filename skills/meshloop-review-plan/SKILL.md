---
name: meshloop-review-plan
description: Ask the origin user to accept, decline, or adjust a Meshloop graph. Slash /meshloop:review-plan.
---

# /meshloop:review-plan

You are `meshloop:origin`. Do not implement. Do not start workers yet.

Pass your origin explicitly (`--origin-harness`, `--origin-session`, or
`MESHLOOP_ORIGIN_HARNESS` / `MESHLOOP_ORIGIN_SESSION`); `/meshloop:doctor` only echoes
them back.

**Ask once, in English**, then wait:

> **Accept** this graph (we can run a worker next), **Decline** it (stop, no
> worker), or **Adjust** (planner runs again with your notes).

Treat Portuguese `aceitar` / `recusar` / `ajustar` (and close synonyms) as the
same three intents. Do **not** show CLI flags until they have chosen. Then run
**exactly one**, with the values at the start of the line replaced and `--as` naming
the person who answered:

```bash
h=claude; s=0f1e2d3c-session-id; p="/home/me/runs/issue-12/meshloop-plan.json"; meshloop review-plan --plan "$p" --accept --as "Sam" --json --origin-harness "$h" --origin-session "$s"

h=claude; s=0f1e2d3c-session-id; p="/home/me/runs/issue-12/meshloop-plan.json"; meshloop review-plan --plan "$p" --decline --reason "Splits issue 12 too finely" --json --origin-harness "$h" --origin-session "$s"

h=claude; s=0f1e2d3c-session-id; p="/home/me/runs/issue-12/meshloop-plan.json"; meshloop review-plan --plan "$p" --adjust --objective "Merge nodes 2 and 3" --json --origin-harness "$h" --origin-session "$s"
```

Accept and Decline start nothing, so you can run them yourself once the user has
answered. **Adjust runs the planner again**, which starts a worker: your harness may
refuse to let you run it, and that refusal is correct. Do not route around it. Hand the
user one line to paste instead (PowerShell form: `$p = "C:\..."; meshloop review-plan
--plan "$p" ...`), with every value substituted and no `<...>` placeholders, which
PowerShell parses as redirection.

| Reply | Stored | `run` |
|---|---|---|
| Accept | `PlanAccepted` | allowed (no `--accept-plan`) |
| Decline | `PlanDeclined` | refuses until a later Accept |
| Adjust | still `AwaitingPlanReview` | refuses; ask again after the new graph |

Do not call `run --accept-plan` to skip this question.

R1 rule: every node stops for `meshloop accept`; the tier selects harness/model only.
The review-plan JSON lists them under `acceptance_required`.
