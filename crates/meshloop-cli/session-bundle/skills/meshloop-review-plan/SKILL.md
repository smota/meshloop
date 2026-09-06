---
name: meshloop-review-plan
description: Ask the origin user to accept, decline, or adjust a Meshloop graph. Slash /meshloop:review-plan.
---

# /meshloop:review-plan

You are `meshloop:origin`. Do not implement. Do not start workers yet.

If `MESHLOOP_ORIGIN_SESSION` is unset, run `/meshloop:doctor` first.

**Ask once, in English**, then wait:

> **Accept** this graph (we can run a worker next), **Decline** it (stop, no
> worker), or **Adjust** (planner runs again with your notes).

Treat Portuguese `aceitar` / `recusar` / `ajustar` (and close synonyms) as the
same three intents. Do **not** show CLI flags until they have chosen. Then run
**exactly one**:

```text
meshloop review-plan --plan meshloop-plan.json --accept --as <identity> --json \
  --origin-harness <h> --origin-session <id>

meshloop review-plan --plan meshloop-plan.json --decline --reason "<why>" --json \
  --origin-harness <h> --origin-session <id>

meshloop review-plan --plan meshloop-plan.json --adjust --objective "<changes>" --json \
  --origin-harness <h> --origin-session <id>
```

| Reply | Stored | `run` |
|---|---|---|
| Accept | `PlanAccepted` | allowed (no `--accept-plan`) |
| Decline | `PlanDeclined` | refuses until a later Accept |
| Adjust | still `AwaitingPlanReview` | refuses; ask again after the new graph |

Do not call `run --accept-plan` to skip this question.
