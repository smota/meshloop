# 0012 ADR and AI engineering governance

- Status: Accepted
- Implementation: implemented
- Date: 2026-09-05
- Author/executor: Codex
- Decision owner: Samuel
- Approval evidence: user approved the preceding structure/governance plan by authorizing initialization; Apache-2.0 was separately explicitly selected.
- Supersedes: none
- Superseded by: none

## Context
Initialize a Rust project for fully AI-coded development across five selected harnesses.

## Alternatives
Implicit decisions and vendor-fixed roles were considered insufficient for cross-harness continuity and evidence.

## Decision
Use the documented ADR lifecycle and separate implementation status. AI authors code and tests; humans retain product direction and consequential acceptance. Record actual executors and candidate-specific validation.

## Consequences
Routine bounded changes can proceed within accepted policy. Proposed runtime ADRs do not become accepted merely because initialization was authorized.

## Verification
Inspect the corresponding policy, manifests, and legal files and run workspace checks.
Final execution evidence is reported at initialization; accepted does not imply runtime features exist.
