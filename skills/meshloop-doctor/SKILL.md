---
name: meshloop-doctor
description: Probe Meshloop readiness, daemonless state, and origin pane. Slash /meshloop:doctor. Does not split panes.
---

# /meshloop:doctor

```text
meshloop doctor --json
```

Does **not** split panes.

- Verify `daemonless: true` and `git_available: true`. If not ready, tell the user and stop.
- Otherwise read `origin_session` and `origin_harness`. If
  `MESHLOOP_ORIGIN_SESSION` is unset, set it from that JSON (and
  `MESHLOOP_ORIGIN_HARNESS` from this session's kind).
- `origin_session` must be **this** pane. Pass those flags on every later
  `meshloop:*` command.
