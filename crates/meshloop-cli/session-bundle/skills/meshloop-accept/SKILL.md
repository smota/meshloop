---
name: meshloop-accept
description: Record human acceptance of a Meshloop node. Slash /meshloop:accept.
---

# /meshloop:accept

R1 rule: every node stops for `meshloop accept`; the tier selects harness/model only.
`status`/`inspect` report `waiting_for: acceptance` for nodes awaiting this step.

Only after the user agrees in this supervisor session:

```text
meshloop accept --task <id> --as <identity> --json --origin-harness <h> --origin-session <id>
meshloop resume --json --origin-harness <h> --origin-session <id>
```
