---
name: meshloop-status
description: Show Meshloop run state as JSON. Slash /meshloop:status.
---

# /meshloop:status

If `MESHLOOP_ORIGIN_SESSION` is unset, run `/meshloop:doctor` first.

```text
meshloop status --json [--graph <id>] --origin-harness <h> --origin-session <id>
```

To wait for a run instead of polling, use
`meshloop watch --graph <id> --json [--interval <s>] [--timeout <s>]`. It reads
the store only (works on `run --detach`), prints one NDJSON line per node state
change, and ends with `{"exit":<reason>,"nodes":[…]}`: `awaiting_review`,
`awaiting_plan_acceptance`, `completed` (exit 0), `failed`/`cancelled` (exit 1),
`timeout` (exit 3).
