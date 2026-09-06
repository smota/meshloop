---
name: meshloop-orchestrate
description: Pin evidence and assign fresh meshloop:reviewer leaves. Not /skill:orchestrate.
---

# /meshloop:orchestrate

You are `meshloop:origin` (supervisor). Do not review in this session.

If `MESHLOOP_ORIGIN_SESSION` is unset, run `/meshloop:doctor` first. Live
reviewers require origin so this pane is never split. Without origin, Meshloop
prints the matrix only (CI).

```text
meshloop orchestrate --task <id> --model-a claude --model-b grok --json \
  --origin-harness <this> --origin-session <pane-id> \
  --config <toml> --db <sqlite>
```

Reviewers are `meshloop:reviewer` leaves. Human `/meshloop:accept` still
required. Do not poll; one CLI call. Do not use unprefixed `reviewer`.
`--fixture-only` skips live panes.
