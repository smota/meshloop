# Validation contract

Run `cargo run -p xtask -- check` from the root. It checks formatting, compilation,
Clippy with warnings denied, and workspace tests, offline and locked where applicable.
After adding dependencies, fetch approved locked dependencies separately before offline checks.

Classify tests by behavior: pure domain unit tests, engine tests using controlled ports,
adapter contract tests with synthetic fixtures, and explicitly authorized live integration
tests against disposable repositories. `tests/scenarios/scaffold_cli.rs` covers help,
version, missing-argument rejection, and unprefixed-role rejection. End-to-end CLI tests
drive the compiled binary against `fixture_harness` on disposable git repos (canned plan,
`--accept-plan` gate, empty-diff failure, accept then resume to Integrated). Live pane
splits are not an R1 test requirement.

Future critical cases: DAG cycles, dependency failure, concurrency limits, cancellation,
timeout, fallback exhaustion, recovery, stale evidence, path escape, test tampering,
merge conflict, interrupted integration, and secret redaction. Do not fake live success.
No coverage percentage is adopted. Feature combinations and supported platforms require
an explicit matrix; do not assume all-features is valid. Revalidate integrated candidates.
