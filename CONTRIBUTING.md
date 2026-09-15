# Contributing to Meshloop

**Before you start:** read the [docs hub](docs/README.md), then this file.
Contributors and agents must also read [AGENTS.md](AGENTS.md).
Run `cargo run -p xtask -- check` from the repository root.
`xtask live` is the launch gate (verifying daemonless execution and Git worktree isolation).
Intentional contributions are Apache-2.0 (see below).

Meshloop is a multi-agent development optimization and loop engineering runtime on native Windows & Linux.
Subprocess workers execute in ephemeral Git worktrees via direct CLI dispatch (`CliHarness`).
Fixture is the CI double.

---

## How to Extend Meshloop

Meshloop's hexagonal architecture makes adding capabilities straightforward and isolated:

### 1. Adding a New Language for AST Context Pruning
To add a new language to [`meshloop-context`](crates/meshloop-context):
1. Open [`crates/meshloop-context/src/skeleton.rs`](crates/meshloop-context/src/skeleton.rs):
   - Add the language variant to the `Language` enum.
   - Map file extensions in `Language::from_path`.
   - Implement `prune_<language>(source: &str) -> String` to remove function/method bodies while keeping signatures, types, and docstrings.
2. In [`crates/meshloop-context/src/signature.rs`](crates/meshloop-context/src/signature.rs), add signature extraction for the new language.
3. Add unit tests in `skeleton.rs` validating token reduction and idempotence.

### 2. Adding or Customizing Harnesses
Meshloop uses zero hardcoded CLI flags in the engine:
1. Standard CLI harnesses are configured via TOML templates (`invoke_args_template`) in `meshloop.toml`.
2. To create a custom transport (e.g. isolated containers or microservices), implement the [`HarnessCapabilities`](crates/meshloop-engine/src/ports.rs) trait in [`meshloop-adapters`](crates/meshloop-adapters) (`probe`, `invoke`, `try_collect`, `collect`, `cancel`).
3. Compose the adapter in [`meshloop-cli::compose`](crates/meshloop-cli/src/compose.rs).

### 3. Adding New Deterministic Checks
To plug in new linters, type checkers, or test runners:
1. Implement the [`CheckRunner`](crates/meshloop-engine/src/ports.rs) trait in `meshloop-adapters::check`.
2. Ensure exit codes and compiler errors are captured cleanly for the diagnostic lattice.

---

## Development Setup

- **Toolchain:** Rust 1.98+, edition 2024, managed via mise. No global tool installs or elevated privileges.
- **Verification Gates:**
  ```bash
  cargo run -p xtask -- check    # Format, clippy -D warnings, workspace tests
  cargo run -p xtask -- bench    # Benchmark scorecard (SPEC-ML-BENCH-001)
  cargo run -p xtask -- live     # Verifies daemonless direct-cli and worktree isolation
  ```
- **Branch Naming:** `codex/<slug>` for Codex, `work/<slug>` for other harnesses.
- **Privacy:** Never commit credentials, private API keys, prompts, or `.meshloop/` state.

---

## Engineering Governance and Standards

All contributions, regardless of author or tools used, must strictly satisfy the project's architectural invariants, quality standards, and review requirements ([ADR 0012](docs/architecture/adr/0012-engineering-governance.md)).

Every change must clearly document:
- The problem being addressed and scope of change.
- Acceptance criteria and relevant ADRs.
- Validation and verification evidence (`cargo run -p xtask -- check` and `bench`).
- Execution and review attribution.

---

## Pull Requests and Component-Bounded Scope

Use the [PR template](.github/PULL_REQUEST_TEMPLATE.md). To preserve hexagonal boundaries and maintain code quality, follow these non-negotiable rules:

### 1. Bounded Scope per Architectural Layer
Every PR must touch **exactly one architectural layer**:
- `domain`: Pure data types, state machine, evidence contracts (zero I/O).
- `context`: AST extraction, quantized indexing, prompt assembly (zero agent process execution).
- `engine`: Sagas, RunLoop, convergence logic, QACR router (ports/traits only).
- `adapters`: Concrete I/O (Git worktrees, SQLite WAL, OS process trees, Docker).
- `cli`: Command composition, operator skills, protocol servers.

PRs mixing domain entities with adapter I/O or CLI commands will be rejected at triage.  
Naming convention: `feat(domain)/...`, `fix(engine)/...`, `perf(context)/...`.

### 2. Minimum Testing Requirements
- `cargo test --workspace` must pass with 100% success.
- Zero linter or formatter warnings: `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --check`.
- Deterministic test coverage: every state transition, pruning rule, or metric formula must have matching deterministic unit tests covering positive and negative edge cases. No tests may depend on network calls, API keys, or arbitrary timing delays (`sleep`).

### 3. Measurement Framework Results
PRs touching engine, context, or adapters must include the benchmark scorecard from `cargo run -p xtask -- bench` in the PR description:
- Verification of fail-closed isolation invariants (`iso.worktree.leak_count = 0`, `conc.orphan_process_count = 0`).
- Token reduction ratio (`ctx.tokens.reduction_pct`).
- Convergence potential delta ($\Delta \phi$) or retrieval latency (`quant.search.latency_us`).

### 4. Non-Negotiable Design Principles
- **Unsafe Rust**: `#![forbid(unsafe_code)]` across all crates. The only exception is the Win32 Job Object wrapper in `meshloop-adapters::process::job`, isolated with `#![allow(unsafe_code)]` and covered by an accepted ADR.
- **Zero Tokio in the Core Engine**: The coordinator saga is single-threaded and synchronous. Tokio is restricted to optional adapter crates (`rmcp`, `bollard`) behind feature flags.
- **Deterministic Evidence Honesty**: LLMs never judge their own correctness. Only real tool execution exit codes (`DeterministicEvidence`) satisfy acceptance gates.
- **Windows/Linux Boundary**: Native paths are respected per OS; never cross filesystem boundaries awkwardly.

Merge requires maintainer authorization ([@smota](https://github.com/smota)). crates.io 0.1.0 is published. Further releases are maintainer-gated (ADR 0018).

---

## Conduct and Legal

This project follows the [Contributor Covenant](.github/CODE_OF_CONDUCT.md).
Enforcement contact: GitHub [@smota](https://github.com/smota).

Contributions intentionally submitted for inclusion are provided under Apache-2.0 as described in section 5 of [LICENSE](LICENSE). Forks must not imply official endorsement. See [TRADEMARKS.md](TRADEMARKS.md).
