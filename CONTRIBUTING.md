# Contributing to Meshloop

**Before you start:** read the [docs hub](docs/README.md), then this file.
If an agent will author the diff, it must also read [AGENTS.md](AGENTS.md).
Run `cargo run -p xtask -- check` from the repository root. Live tests may
split **non-origin** Herdr panes; they must never split the supervisor pane.
`xtask live` is the launch gate. Intentional contributions are Apache-2.0
(see below).

Release 1 is a live-Herdr closed loop on native Windows. Fixture is the CI
double. Do not reintroduce `--allow-live-harness` as a product gate.

## Before you write

1. Read [Install](docs/install.md) and [Getting started](docs/start.md) if you
   have not run the in-session loop.
2. Open an issue with a template ([bug](.github/ISSUE_TEMPLATE/bug.yml),
   [docs](.github/ISSUE_TEMPLATE/docs.yml),
   [R1 limitation](.github/ISSUE_TEMPLATE/limitation.yml),
   [proposal](.github/ISSUE_TEMPLATE/proposal.yml),
   [question](.github/ISSUE_TEMPLATE/question.yml)). Small patches may
   reference an issue; do not invent GitHub issue numbers.
3. Consequential architecture → draft an ADR
   (`docs/architecture/adr/template.md`). Routine work inside accepted
   boundaries needs no new ADR.
4. Do not open a public issue for vulnerabilities. See
   [SECURITY.md](.github/SECURITY.md).

Human-authored pull requests are welcome. The model is “AI may author; humans
govern,” not “humans may not code.” A human patch that adds a regression test
is first-class.

Good first surfaces that do not require rewriting the engine: fixture
scenarios, doctor probes, docs/status contradictions, skills that wrap the
compiled CLI only, redaction tests, Windows process-view edge cases. You do
not need a five-harness local setup. Live workers need Herdr; fixture covers CI.

## Development setup

- Rust 1.98, edition 2024, via the existing mise-managed toolchain on this
  workstation. No global tool installs and no privilege elevation as part of
  contributing.
- Checks: `cargo run -p xtask -- check` (fmt, clippy `-D warnings`, workspace
  tests). Optional: `smoke`, `bundle`, `publish-dry`.
- Details: [testing](docs/engineering/testing.md), [Rust](docs/engineering/rust.md),
  [workflow](docs/engineering/agent-workflow.md).

Branch naming already in use: `codex/<slug>` for Codex, `work/<slug>` for
other harnesses. Do not invent a public GitFlow.

Keep scratch output in ignored `.agent-runs/`. Do not commit credentials,
private prompts, live session logs, or `.meshloop/` state.

## How work is authored

AI authors implementation and tests; humans direct product intent and accept
consequential decisions ([ADR 0012](docs/architecture/adr/0012-engineering-governance.md)).
Record the actual executor and reviewer, including self-review honestly. Do
not claim independent review for a self-review. Tool usage is not proof of
copyright ownership.

Changes must identify the problem, acceptance criteria, relevant ADRs,
validation, and AI executor/reviewer. Do not include credentials, private
prompts, or third-party code of unknown provenance. Do not weaken tests or
acceptance criteria merely to obtain a passing result.

## Pull requests

Use the [PR template](.github/PULL_REQUEST_TEMPLATE.md). State scope,
exclusions, linked requirements/ADRs, tests run and skipped (with reasons),
review identity, and any compatibility or security consequences.

Merge requires maintainer authorization ([@smota](https://github.com/smota)).
AI review is supporting evidence, not proof of correctness. High-risk
security, unsafe Rust (currently forbidden), destructive data changes, and
release acceptance need human review of a concrete result. Local green is not
remote success. crates.io 0.1.0 is already uploaded. Further versions and GitHub Releases are
maintainer-gated (ADR 0018). Do not `cargo publish` without that
authorization.

## Conduct and legal

This project follows the [Contributor Covenant](.github/CODE_OF_CONDUCT.md).
Enforcement contact: GitHub [@smota](https://github.com/smota).

Unless explicitly stated otherwise, contributions intentionally submitted for
inclusion are provided under Apache-2.0 as described in section 5 of
[LICENSE](LICENSE). Contributors retain their applicable rights; this policy
is not a copyright assignment or a separate CLA. Submit only material you are
authorized to contribute and disclose third-party notices, AI tooling used
when known, and relevant provenance limitations. A future commercial
dual-license model would require separate rights analysis; it is not
established here.

Forks must not imply official endorsement. See [TRADEMARKS.md](TRADEMARKS.md).
Do not merge or publish without the authorization applicable to the task.
