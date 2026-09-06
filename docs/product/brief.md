# Product brief

Public map: [docs/README.md](../README.md). Release 1 is a fixture-backed closed
loop on native Windows; the full multi-harness vision below remains proposed.

Meshloop is a multi-harness coordinator built on Herdr. It takes one engineering
objective, plans it into a task graph, and uses model selection to spread the work across
whichever subscription-based CLI clients are configured (Codex, Claude Code, Pi, Grok,
Agy) — choosing per task which harness and model tier to use from verified capability,
risk, and observed subscription quota — then verifies and integrates what comes back.
Release 1 (ADR 0016) is a closed-loop single-writer engine on native Windows against a fixture harness; the full multi-harness Herdr vision remains proposed.

The problem this solves: each harness is a flat-rate subscription with its own capability
set and its own rate-limit window, not a metered API. Used one at a time, that capacity
sits idle or gets exhausted unevenly; used through ad hoc scripting, nothing verifies what
came back or recovers cleanly when a session dies mid-task. Meshloop's value is the
planning-and-orchestration layer that turns several separately-authenticated,
separately-rate-limited subscriptions into one coherent, bounded, verified execution loop.

Core concerns: transparent authorization, coherent context, isolated changes, reproducible
verification, bounded resource use, auditable recovery, and useful human status reports.
Selected harnesses: Codex, Claude Code, Pi, Grok, Agy. Capabilities must be verified per
installed runtime and platform. Never substitute Agy with Antigravity by name alone.

No credential manager, cloud service, automatic publication, background service installation,
or skill-pack dependency is included in the initialization. Runtime scope is in proposed
ADRs; see docs/architecture/overview.md for how model selection and agent dispatch
(ADR 0009, ADR 0003) implement this.
