---
name: meshloop-doctor
description: Probe Meshloop + Herdr readiness and origin pane. Slash /meshloop:doctor. Does not split panes.
---

# /meshloop:doctor

```text
meshloop doctor --json
```

Does **not** split panes.

- If `herdr_server_running` is not true: tell the user Herdr 0.8 is down and
  **stop**. Do not invent a pane id. Do not plan.
- Otherwise read `origin_session` and `origin_harness`. If
  `MESHLOOP_ORIGIN_SESSION` is unset, set it from that JSON (and
  `MESHLOOP_ORIGIN_HARNESS` from this session's kind).
- `origin_session` must be **this** pane. Pass those flags on every later
  `meshloop:*` command.
