# Rust conventions

Rust 1.98.0 and edition 2024 are the bootstrap baseline, using the existing mise-managed
toolchain. An MSRV below 1.98 is not promised. Cargo.lock is committed. crates.io
0.1.0 is published for the four product crates (ADR 0018); `xtask` is unpublished.
Do not `cargo publish` a new version without maintainer authorization.

Use rustfmt, idiomatic naming, explicit domain types, Result for recoverable failures,
and documentation for public APIs and invariants. Workspace policy forbids unsafe code;
changing it requires a reviewed ADR. Avoid arbitrary line limits and premature abstractions.
Use feature/platform matrices only once real requirements are defined. Dependencies such
as Tokio, SQLite bindings, serialization, and argument parsers require selection evidence.
