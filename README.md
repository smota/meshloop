# Meshloop

A local Rust orchestration engine for AI-assisted software engineering through
authenticated command-line agents and Herdr.

Created and maintained by Samuel. Meshloop is fully AI-coded under human product
direction and architecture governance. Contributor and execution records distinguish
human decisions from generated implementation; they do not establish legal authorship.

## Status

**Release 1** is a fixture-backed closed loop. **Session control plane (ADR 0017):**
operate *from* an agent session (supervisor-only). All skills, slash commands, MCP
tools, and roles are prefixed `meshloop:` (`/meshloop:plan`, MCP `meshloop_plan`,
roles `meshloop:planner` / `meshloop:reviewer`). Live workers use Herdr 0.8 panes;
fixture subprocesses remain CI. `meshloop doctor` / `roles` / `orchestrate` / `mcp`
are implemented. Live pane split is fail-closed and not run by tests.

See [ADR 0016](docs/architecture/adr/0016-r1-closed-loop.md),
[ADR 0017](docs/architecture/adr/0017-session-control-plane.md),
and [skills](skills/README.md).

## Development

Use Rust 1.98.0 with rustfmt and Clippy. The existing workstation uses mise; no global
tool installation is part of this repository. Run from the repository root:

```text
cargo run -p xtask -- check
cargo run -p xtask -- smoke
cargo run -p xtask -- bundle
cargo run -p meshloop-cli -- --help
cargo run -p meshloop-cli -- roles --json
cargo run -p meshloop-cli -- doctor --json
```

Typical fixture loop (from a git repository, with `config/meshloop.example.toml` or a
local `meshloop.toml`):

```text
cargo build -p meshloop-adapters --bin fixture_harness
cargo run -p meshloop-cli -- plan --objective "…" --config config/meshloop.example.toml
cargo run -p meshloop-cli -- run --plan meshloop-plan.json --accept-plan --config config/meshloop.example.toml
cargo run -p meshloop-cli -- accept --task 1 --as you
cargo run -p meshloop-cli -- resume
```

Default store: `.meshloop/state.sqlite`. `run` does not merge onto your current branch;
use `meshloop integrate --graph <id> --into <ref> --accept-integrate` after review.

Read [AGENTS.md](AGENTS.md), [architecture](docs/architecture/overview.md),
[ADRs](docs/architecture/adr/README.md), and [workflow](docs/engineering/agent-workflow.md).
The engine is intended to use existing authenticated CLI sessions, not store credentials.

## Licensing and contributions

Code is licensed under [Apache-2.0](LICENSE). See [NOTICE](NOTICE),
[trademark policy](TRADEMARKS.md), and [contribution policy](CONTRIBUTING.md).
No trademark registration or exclusive copyright over AI-generated output is asserted.
