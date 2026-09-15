# External contracts

Add versioned execution graph, event, evidence, and configuration schemas only after the relevant ADR is accepted. Validate external data at boundaries.

## Implemented Schemas

- [`run-record.v1.json`](run-record.v1.json): Canonical schema for benchmark and scorecard telemetry runs (`SPEC-ML-BENCH-001`, `ADR 0023`). Enforces metric structures, provenance tracking (Git SHA, rustc, OS, crate versions), execution phases breakdown, and pass/fail gate status.
