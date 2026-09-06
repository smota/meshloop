# Architecture overview

## Problem and solution

Herdr already runs one local, authenticated CLI harness in a terminal pane. Meshloop's
premise, stated concretely: Codex, Claude Code, Pi, Grok, and Agy are each a flat-rate
subscription with its own capability set and its own rate-limit window, not a metered API —
used one at a time, that capacity sits idle or gets exhausted unevenly, and nothing
verifies what a session produced or recovers if it dies mid-task. Meshloop is the
planning-and-orchestration layer on top of Herdr: planning and routing (ADR 0009) turn one
engineering objective into a task graph by dispatching that decomposition itself as a
bounded agent call, not a hardcoded algorithm, then pick which harness and model tier
handles each node via Quota-Aware Capability Routing — extensible, verified capability,
observed subscription quota, never dollar cost, never a hardcoded default; each node
dispatches as a bounded agent (ADR 0003); results are verified and recovered cleanly from
interruption. The compiled CLI (ADR 0001) is the sole operating surface for all of this,
running on Windows natively or on Linux via WSL2 on the same host.

Full product framing: docs/product/brief.md. Acceptance targets: docs/product/requirements.md.

## Architecture

The accepted development structure separates pure domain contracts, engine use cases,
external adapters, and CLI composition. Product runtime choices remain proposed.

```text
CLI -> Engine -> Domain
 |        ^        ^
 +-> Adapters -----+
```

| Layer | Owns |
|---|---|
| meshloop-domain | Task graph, the 10-state execution lifecycle, evidence, policy, and harness-capability/error-taxonomy value types — no I/O |
| meshloop-engine | Planner, router (QACR), agent dispatch, orchestrator, and recovery use cases, defined against ports it does not implement |
| meshloop-adapters | Herdr (CLI-subprocess), per-harness capability probes, Git worktree isolation, SQLite-backed store |
| meshloop-cli | Argument parsing, composing adapters into engine ports, human-readable reports |

Execution moves a task through `pending -> ready -> running -> verifying ->
awaiting-review -> accepted -> integrated` (or `failed`/`cancelled`/`blocked`), one attempt
at a time, with every transition logged as an event carrying executor, revision, and
evidence reference — see execution-lifecycle.md. Isolation is by Git worktree with one
integration owner (ADR 0005); Herdr panes are never treated as a security boundary.
Verification separates deterministic evidence, advisory model review, and mandatory human
acceptance at Tier 3 (ADR 0007), which also owns local SQLite persistence, versioned and
redacted. Routing follows Quota-Aware Capability Routing: verified capability, risk tier,
observed subscription quota, and historical success — never a static model-name table or
dollar-cost budget (ADR 0009). Agent dispatch (ADR 0003) turns a routed task into a bounded
prompt and worktree-scoped execution; the worktree diff, not harness stdout, is the primary
output verification evaluates.

See boundaries.md, execution-lifecycle.md, threat-model.md, the ADR index, and
docs/engineering/implementation-plan.md for the staged build-out toward that architecture,
runtime-design.md for the mechanism behind every ADR above, and
docs/engineering/design-patterns.md for the concrete pattern behind each contract and how
to design, implement, and validate it.
