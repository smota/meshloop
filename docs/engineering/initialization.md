# Initialization evidence

> Audience: maintainers and agents. Bootstrap evidence from 2026-09-05, not the
> product landing page. Public map: [docs/README.md](../README.md).

Date: 2026-09-05. Executor: Codex. User authorized initialization after approving
the structure, ADR workflow, five harnesses, and Apache-2.0 selection.

## Created
- Four-crate Rust workspace and xtask, pinned Rust 1.98.0, edition 2024, local-only dependencies.
- Canonical AGENTS.md, engineering guidance, product requirements and architecture baseline.
- Twelve ADRs: workspace, licensing, and governance accepted; nine runtime decisions proposed.
- Official unmodified Apache-2.0 LICENSE, NOTICE, contribution and trademark policies.
- CLI scaffold and an acceptance scenario; no orchestration behavior implemented.

## Verification
- `cargo fmt --all -- --check`: passed on the target repository.
- `cargo check --workspace --all-targets --locked --offline`: passed.
- `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`: passed.
- `cargo test --workspace --locked --offline`: blocked during linking because
  MSVC `link.exe` is unavailable in the execution environment. Tests did not execute.
- The standard mise shim hit a sandbox configuration-access error. Validation used
  the same existing Rust 1.98.0 toolchain binaries directly with a command-scoped PATH.
  No replacement toolchain or global configuration was installed.

## AFD status
Installed AFD reports 0.6.4. Audit and plan used the complete selected set:
`codex,claude-code,pi,grok,agy`. Canonical policy was found. AFD staged pointers for
CLAUDE.md, PI.md, and AGY.md outside this repository; it generated no Grok pointer.

Non-live readiness under the host identity found Codex, Claude Code, Pi, and Grok
available. Agy reported unsupported: no fail-closed read-only smoke runner is registered.
The audit separately marks Grok instruction discovery unsupported and Agy generated-only.
Readiness is not live verification. No live sessions were started, no adapter apply was
performed, and no passing apply receipt exists. The selected set was not reduced.

AFD JSON audit, plan, staged adapters, and readiness reports are retained in the local
AFD initialization output directory outside Meshloop. Git has no initial commit, so
AFD recorded null base revision and workspace fingerprint. Re-plan from the intended
Git state and review exact evidence before any later apply.

## Remaining prerequisites
1. Make an MSVC linker and Windows SDK available through an approved workstation setup,
   then rerun the full check command. Do not claim runtime validation before tests execute.
2. Resolve Agy smoke-runner and Grok discovery support in a separately scoped AFD change.
3. Re-audit, stage, and live-test all five harnesses, then apply and verify the exact plan.
4. Accept the relevant proposed runtime ADRs before implementing product behavior.

No commit, push, release, service installation, credential access, or publication was performed.
