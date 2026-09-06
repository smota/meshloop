# 0007 Verification and persistence

- Status: Proposed
- Implementation: in-progress — the three evidence types and the tier-based required-evidence gate implemented and tested; SQLite-backed EvidenceStore/RoutingFeedbackStore implemented and tested (schema-version guard included) via bundled rusqlite, which compiled successfully here; `run`'s v1 CLI records only exit-code-based DeterministicEvidence, not real linter/test tool output. See docs/engineering/implementation-status.md.
- Date: 2026-09-05
- Author/executor: Claude (Sonnet 5), consolidated from prior Codex/user planning; absorbs former ADR 0008 (persistence and privacy)
- Reviewer: pending
- Approval evidence: none for this runtime decision
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
