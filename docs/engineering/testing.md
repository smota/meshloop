# Validation contract

Run `cargo run -p xtask -- check` from the root. It checks formatting, compilation,
Clippy with warnings denied, and workspace tests, offline and locked where applicable.
After adding dependencies, fetch approved locked dependencies separately before offline checks.

Classify tests by behavior: pure domain unit tests, context engineering AST skeleton
and cache normalization unit tests (7 languages), engine tests using controlled ports,
adapter contract tests with synthetic fixtures, and live direct-CLI subprocess tests in
isolated Git worktrees. `tests/scenarios/scaffold_cli.rs` covers help, version, missing-argument
rejection, and unprefixed-role rejection. End-to-end CLI tests drive the compiled
binary against `fixture_harness` on disposable git repos (canned plan, `--accept-plan`
gate, empty-diff failure, accept then resume to Integrated). `cargo run -p xtask -- live`
verifies daemonless operation (`doctor` reporting `daemonless: true` and `live_transport: direct-cli`)
and git worktree isolation operational state (launch gate). `cargo run --release -p xtask -- bench`
evaluates the 19 canonical SPEC-ML-BENCH-001 architectural metrics against `benches/thresholds.toml`.
`cargo run -p xtask -- bench-dag` evaluates 7 canonical DAG manifests against the P95 scheduling overhead
gate (`orch.schedule.overhead_ms.p95 <= 25.0ms`). `cargo run -p xtask -- bench-world-s` runs the opt-in
World S benchmark across 8 polyglot exercises with Wilson 95% confidence intervals.
`cargo run -p xtask -- publish-dry` packages all five publishable crates in isolation (ADR 0018);
it is not a crates.io upload.

Future critical cases: DAG cycles, dependency failure, concurrency limits, cancellation,
timeout, fallback exhaustion, recovery, stale evidence, path escape, test tampering,
merge conflict, interrupted integration, and secret redaction. Do not fake live success.
No coverage percentage is adopted. Feature combinations and supported platforms require
an explicit matrix; do not assume all-features is valid. Revalidate integrated candidates.
