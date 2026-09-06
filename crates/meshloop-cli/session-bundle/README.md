This directory is the crates.io embed of repo-root `skills/` (ADR 0018).
Edit `skills/` at the workspace root, then `cargo run -p xtask -- bundle`.
`xtask check` fails if the two trees drift.
