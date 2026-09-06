---
name: meshloop-orchestrate
description: Pin evidence and assign fresh meshloop:reviewer leaves. Not /skill:orchestrate.
---

# /meshloop:orchestrate

You are `meshloop:origin` (supervisor). Do not review in this session.

Without `--allow-live-harness`: Meshloop pins the attempt pack and prints the matrix (CI).

With live Herdr:

```text
meshloop orchestrate --task <id> --model-a claude --model-b codex --json \
  --allow-live-harness --origin-harness <this> --origin-session <pane-id> \
  --config <toml> --db <sqlite>
```

`--origin-session` is required when live so this pane is never split. Reviewers are `meshloop:reviewer` leaves. Human `/meshloop:accept` still required. Do not poll; one CLI call. Do not use unprefixed `reviewer`.
