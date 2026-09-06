---
name: meshloop-run
description: Execute an accepted Meshloop graph. Supervisor-only origin. Slash /meshloop:run.
---

# /meshloop:run

```text
meshloop run --plan <file> --accept-plan --json --origin-harness <h> --origin-session <id>
```

Live Herdr workers additionally need `--allow-live-harness`. Fixture CI does not.

Do not poll. Read the JSON idle reason; then `/meshloop:status` or `/meshloop:accept`.
