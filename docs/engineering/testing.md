# Validation contract

Run `cargo run -p xtask -- check` from the root. It checks formatting, compilation,
Clippy with warnings denied, and workspace tests, offline and locked where applicable.
After adding dependencies, fetch approved locked dependencies separately before offline checks.

Classify tests by behavior: pure domain unit tests, engine tests using controlled ports,
adapter contract tests with synthetic fixtures, and live Herdr tests against a running
server. `tests/scenarios/scaffold_cli.rs` covers help, version, missing-argument
rejection, and unprefixed-role rejection. End-to-end CLI tests drive the compiled
binary against `fixture_harness` on disposable git repos (canned plan, `--accept-plan`
gate, empty-diff failure, accept then resume to Integrated). Live tests may split
**non-origin** panes when `herdr status` is running; they skip if the server is down.
`cargo run -p xtask -- live` **fails** if Herdr is down (launch gate). Never split
the origin supervisor pane. `cargo run -p xtask -- publish-dry` packages the four
publishable crates in isolation (ADR 0018); it is not a crates.io upload.

Future critical cases: DAG cycles, dependency failure, concurrency limits, cancellation,
timeout, fallback exhaustion, recovery, stale evidence, path escape, test tampering,
merge conflict, interrupted integration, and secret redaction. Do not fake live success.
No coverage percentage is adopted. Feature combinations and supported platforms require
an explicit matrix; do not assume all-features is valid. Revalidate integrated candidates.
