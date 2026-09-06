# 0002 Workspace boundaries

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
Single crate with modules was considered; separate crates make dependency direction and concurrent ownership explicit.

## Decision
Use domain, engine, adapters, and CLI crates, plus xtask for repository checks. Dependencies flow toward domain; CLI composes concrete adapters.

## Consequences
Cargo manifests and boundary documentation implement the scaffold. No product interfaces are implemented.

## Verification
Inspect the corresponding policy, manifests, and legal files and run workspace checks.
Final execution evidence is reported at initialization; accepted does not imply runtime features exist.
