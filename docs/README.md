# Meshloop docs

Meshloop is a local Rust orchestration engine for authenticated CLI agents and
Herdr. It plans one engineering objective into a task graph, routes work,
verifies diffs, and recovers. It does not store credentials.

**Release 1** is a fixture-backed closed loop on native Windows. Commands exist:
`plan`, `run --accept-plan`, `status`, `resume`, `cancel`, `inspect`, `accept`,
`integrate`, `roles`, `doctor`, `orchestrate`, `mcp`. Live workers need Herdr and
`--allow-live-harness`. Tests never split live panes. Real dispatch against
Claude Code, Codex, Pi, Grok, or Agy is **not** an R1 stamp. Concurrency is 1.
No packaging. WSL2 is unverified.

Start here by what you need.

## Try it (operators)

1. [Getting started](start.md) — fixture loop on native Windows
2. Example config: [`config/meshloop.example.toml`](../config/meshloop.example.toml)
3. Session control plane (skills / slash / MCP): [`skills/README.md`](../skills/README.md)

`run` does not merge onto your current branch. Integrate is a separate,
explicit command.

## Understand the product

1. [Product brief](product/brief.md) — problem and value
2. [Requirements](product/requirements.md) — ML-001–014 are *targets*, not a claim that R1 implements all of them
3. [R1 decision](architecture/adr/0016-r1-closed-loop.md) — what Release 1 includes and excludes
4. [Session control plane](architecture/adr/0017-session-control-plane.md) — `meshloop:` prefix, supervisor-only origin

## Contribute

1. [Contributing](../CONTRIBUTING.md)
2. [Code of conduct](../.github/CODE_OF_CONDUCT.md)
3. [Engineering workflow](engineering/agent-workflow.md)
4. [Testing](engineering/testing.md) — `cargo run -p xtask -- check`

## Decisions and architecture (read after the hub)

R1 first: [ADR 0016](architecture/adr/0016-r1-closed-loop.md).
Full v1 runtime ADRs are **Proposed**, not accepted, and not all implemented.

1. [ADR index](architecture/adr/README.md)
2. [Architecture overview](architecture/overview.md) — v1 shape; do not treat every paragraph as shipping
3. [Component boundaries](architecture/boundaries.md)
4. [Execution lifecycle](architecture/execution-lifecycle.md)
5. [Threat model](architecture/threat-model.md) — design requirements, not a guarantee the scaffold enforces them

## Engineering internals (builders and agents)

Do not start here unless you are changing code or running a harness against this repo.

- [Implementation status](engineering/implementation-status.md) — executor log of what was built
- [Rust conventions](engineering/rust.md)
- [Harness contract](engineering/harnesses.md) — developing Meshloop *with* the selected CLIs, not a claim they are R1 workers
- [Design patterns](engineering/design-patterns.md)
- Canonical agent policy: [`AGENTS.md`](../AGENTS.md)

## Help, security, legal

- [Support](../.github/SUPPORT.md) — how to ask; no SLA
- [Security policy](../.github/SECURITY.md) — private reports only
- [License](../LICENSE) (Apache-2.0) · [NOTICE](../NOTICE) · [Trademarks](../TRADEMARKS.md)
