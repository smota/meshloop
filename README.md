# Meshloop

A local Rust orchestration engine for AI-assisted software engineering through
authenticated command-line agents and Herdr.

Created and maintained by Samuel. Meshloop is fully AI-coded under human product
direction and architecture governance. Contributor and execution records distinguish
human decisions from generated implementation; they do not establish legal authorship.

## Status
Initial workspace and engineering baseline only. The CLI reports this scaffold status;
planning, execution, verification, persistence, and integration are not implemented.
The selected harnesses are Codex, Claude Code, Pi, Grok, and Agy. Full AFD harness
verification is pending; see [harness status](docs/engineering/harnesses.md).

## Development
Use Rust 1.98.0 with rustfmt and Clippy. The existing workstation uses mise; no global
tool installation is part of this repository. Run from the repository root:

```text
cargo run -p xtask -- check
cargo run -p meshloop-cli -- --help
```

Read [AGENTS.md](AGENTS.md), [architecture](docs/architecture/overview.md),
[ADRs](docs/architecture/adr/README.md), and [workflow](docs/engineering/agent-workflow.md).
The engine is intended to use existing authenticated CLI sessions, not store credentials.
Herdr and selected CLIs will be external prerequisites; native distribution does not
mean that those prerequisites disappear. Supported runtime platforms remain proposed.

## Licensing and contributions
Code is licensed under [Apache-2.0](LICENSE). See [NOTICE](NOTICE),
[trademark policy](TRADEMARKS.md), and [contribution policy](CONTRIBUTING.md).
No trademark registration or exclusive copyright over AI-generated output is asserted.
