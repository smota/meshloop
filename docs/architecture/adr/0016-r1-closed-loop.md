# 0016 Release 1 subset: closed-loop single-writer orchestration

- Status: Proposed
- Implementation: implemented (native Windows, fixture-backed closed loop)
- Date: 2026-09-06
- Author/executor: Grok (R1 autonomous implementation under human product direction)
- Reviewer: pending
- Approval evidence: none
- Supersedes: none
- Superseded by: none

## Context and constraints
The five proposed runtime ADRs (0001, 0003, 0005, 0007, 0009) describe a full v1. The tree at `c610ef3` had a fixture lab that discarded worktrees and treated harness exit as verification. Release 1 needs a shippable subset that closes one sequential loop without claiming unused adapters as product.

## Alternatives
A: Fixture-only forever. B: Fan out across five harnesses before verify+integrate. C: This subset (one writer, kept worktrees, git-diff verify, human accept at every tier, live invoke opt-in).

## Decision or proposal
R1 ships a foreground CLI on native Windows that drives one accepted task graph sequentially: plan file (human-authored first-class; worktree `meshloop-plan.json` for agents), persisted `--accept-plan`, QACR filter without invented quota, one candidate = one attempt = one kept worktree, git-diff verification (`all_deterministic_passed` ∧ `satisfies(DeterministicOnly)`), WAL SQLite event log with a **per-task** fold, `status`/`resume`/`cancel`/`inspect`/`accept`/`integrate --into`. Dead `Running` → `Failed`; `resume` does not auto-retry. Integrate merges happen in a dedicated integrate worktree; `repo_root` is only touched by `meshloop integrate --into --accept-integrate`. Non-fixture harnesses require `--allow-live-harness`. Herdr stays uncomposed. Concurrency = 1. Packaging and WSL2 are deferred.

This ADR does not supersede 0001/0003/0005/0007/0009; they remain Proposed.

## Consequences
Operators get an honest product path against `fixture_harness`. Completing a live Claude/Codex task is not required to stamp R1. Parents stay Proposed until a human accepts them.

## Verification and implementation evidence
`cargo run -p xtask -- check` on native Windows after this implementation. CLI tests: canned fixture plan, `--accept-plan` gate, empty-diff failure, accept+resume integrate. See `docs/engineering/implementation-status.md`.
