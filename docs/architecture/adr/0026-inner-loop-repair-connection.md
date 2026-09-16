# 0026 Attempt-scoped inner-loop self-repair with session resumption

- Status: Proposed
- Implementation: implemented
- Date: 2026-09-15
- Author/executor: Antigravity
- Reviewer: pending human
- Approval evidence: none
- Supersedes: none
- Superseded by: none

## Context and constraints
When an agent completes an attempt, deterministic checks (`CheckRunner`) frequently surface minor syntax, type, or test errors (e.g. mismatched types, unused imports, or off-by-one test expectations). In the initial baseline, any check failure immediately marked the attempt as `DeterministicChecksFailed` and discarded the worktree or consumed a full external QACR retry.

Constraints:
- No new `TaskState`: the task remains in `TaskState::Running` throughout inner-loop repair.
- Deterministic convergence: repairs must be guided by the Lyapunov potential $\phi$ defined in ADR 0029 over `DiagnosticLattice`.
- Strict bounded rounds: repair attempts are capped at `RepairBudget::default()` (3 rounds maximum).
- Oscillation and regression protection: cyclic fingerprints trigger `Stop { Oscillation }`; syntax regressions trigger `Rollback` via `WorkspacePort::reset_hard`.
- Final gate: acceptance strictly evaluates `verification_passed(&rows)` on the *last* candidate revision.

## Alternatives
1. **Emit a new `TaskState::Repairing` state**: Requires altering the state transition table, SQLite migrations, and event projections. Rejected: inner-loop healing is an execution detail of an ongoing attempt, not an orchestration state change.
2. **Immediate fallback on first error**: Discards partial progress and forces full cold-start re-dispatch on other harnesses. Rejected: highly inefficient and expensive.
3. **Connect `RepairSession::observe` to `RunLoop::verify_attempt`**: Chosen. Automatically drives repair rounds in the existing attempt worktree using negative constraints and git rollback when needed.

## Decision or proposal
1. Add `build_repair_spec` to `meshloop-engine::agent` to construct structured repair prompts containing normalized diagnostics and negative constraints.
2. In `RunLoop::verify_attempt`:
   - When deterministic checks fail, initialize `RepairSession::new(RepairBudget::default())`.
   - After every check, call `session.observe(lattice, revision, diag)`. `max_rounds` is a strict observation budget (default 3).
   - On `RepairAction::Continue`: invoke the harness on the same worktree with `build_repair_spec`, commit the round revision, and re-run deterministic checks.
   - On `RepairAction::Rollback`: `reset_hard` to `to_rev`, restore diagnostics from `history[to_round].diag`, and re-invoke with the negative constraint. Do **not** observe the restored tree (that fingerprint is already in history and would be a false `Oscillation`).
   - On `RepairAction::Stop`: terminate the repair loop and proceed to `DeterministicChecksFailed` and `maybe_fallback`.
   - On `RepairAction::Accept`: record the terminal $\phi=0$ round in `session.rounds()`, then evaluate `verification_passed` on the last revision.
   - Persist the trajectory (`repair-session`) and rollback fidelity (`repair-rollback`) as `DeterministicEvidence` rows.
3. `CommandCheckRunner` drains stdout and stderr concurrently and combines them into `output_redacted` so rustc/cargo diagnostics on stderr reach `annotate_with_lattice`.

## Consequences
- Single-attempt yield increases substantially for minor compiler/test errors without consuming global retry budgets.
- Oscillating edits and syntax regressions are instantly arrested and rolled back.
- No schema changes or state machine bloat.

## Verification and implementation evidence
- Unit and integration tests in `crates/meshloop-engine` and `crates/meshloop-cli`.
- `cargo test --workspace` and `cargo run -p xtask -- check` pass cleanly.
