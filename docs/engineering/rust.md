# Rust conventions

Rust 1.98.0 and edition 2024 are the bootstrap baseline, using the existing mise-managed
toolchain. An MSRV below 1.98 is not promised. Cargo.lock is committed when publication
of the bootstrap is authorized. External crates are not needed for this scaffold.

Use rustfmt, idiomatic naming, explicit domain types, Result for recoverable failures,
and documentation for public APIs and invariants. Workspace policy forbids unsafe code;
changing it requires a reviewed ADR. Avoid arbitrary line limits and premature abstractions.
Use feature/platform matrices only once real requirements are defined. Dependencies such
as Tokio, SQLite bindings, serialization, and argument parsers require selection evidence.
