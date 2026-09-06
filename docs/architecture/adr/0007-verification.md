# 0007 Verification and persistence

- Status: Accepted
- Implementation: implemented for R1 — three evidence types, git-diff verification, SQLite WAL schema v4 (`pane_id` on attempts, `review_note` on runs). Residual: optional `verify_command` is empty in the example; no credential storage (by design).
- Date: 2026-09-05
- Accepted: 2026-09-06
- Author/executor: Claude (Sonnet 5), consolidated from prior Codex/user planning; absorbs former ADR 0008 (persistence and privacy)
- Decision owner: Samuel
- Approval evidence: user approved the 2026-09-06 launch plan
- Supersedes: none
- Superseded by: none

## Context and constraints
What counts as evidence that a change is acceptable, and where that evidence durably lives,
are one decision — evidence with nowhere durable to live cannot survive a crash, and
storage with no evidence contract to fill it has nothing meaningful to hold. Risk controls
review depth; model consensus is not a correctness guarantee. No credentials or raw
sensitive sessions may ever be stored.

## Alternatives
Uniform full pipeline versus risk-based gates with mandatory critical-surface checks;
SQLite versus append-only files; minimal metadata versus redacted diagnostic retention.

## Decision
Separate three evidence kinds — deterministic checks (always required, produced by real
local tools), model review (advisory, never a substitute for deterministic evidence), and
human acceptance (mandatory at Tier 3, unconditionally) — each bound to the exact candidate
revision and attempt id it validates. Persist events, evidence summaries, and routing
feedback in embedded SQLite with a versioned, forward-only-migrating schema; redact secrets,
prompts, completions, and file contents before anything is written. Full evidence and
schema detail is in runtime-design.md §4.

## Consequences
Implementation must remain within accepted decisions; unresolved capabilities must be
visible. The bootstrap does not establish runtime readiness. This creates deliberate
friction at Tier 3: no single "LLM-as-a-judge" gate is ever sufficient for high-risk changes.

## Verification
See runtime-design.md §4's validation scenarios.
