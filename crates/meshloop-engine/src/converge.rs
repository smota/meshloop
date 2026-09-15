//! Attempt-scoped self-healing convergence.
//!
//! Pure decision procedure over [`DiagnosticLattice`] snapshots. The saga stays
//! in `Running`; this module never emits a new `TaskState`. Rollback is a
//! *decision* — the caller applies `WorkspacePort::reset_hard`.
//!
//! Progress φ is the lattice's lexicographic vector (syntax, type, test, error,
//! blocking count). Continue is legal only when φ strictly decreases. A repeated
//! fingerprint is oscillation. A new syntax error is always a rollback.

use meshloop_domain::diagnostic::{DiagnosticLattice, LatticeOrder, compare, syntax_regressed};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepairBudget {
    pub max_rounds: u32,
}

impl Default for RepairBudget {
    fn default() -> Self {
        Self { max_rounds: 3 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundRecord {
    pub round: u32,
    pub fingerprint: u64,
    pub snapshot_rev: String,
    pub lattice: DiagnosticLattice,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    Converged,
    Oscillation { cycle_at: u32 },
    BudgetExhausted,
    SyntaxRegression,
    Unrecoverable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepairAction {
    /// No blocking diagnostics. Last revision is the candidate.
    Accept,
    /// Inject the negative constraint and run another inner round.
    Continue {
        round: u32,
        negative_constraint: String,
    },
    /// `git reset --hard` to `to_rev`, then continue with the constraint.
    Rollback {
        to_round: u32,
        to_rev: String,
        negative_constraint: String,
    },
    Stop {
        reason: StopReason,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepairSession {
    budget: RepairBudget,
    rounds: Vec<RoundRecord>,
}

impl RepairSession {
    pub fn new(budget: RepairBudget) -> Self {
        Self {
            budget,
            rounds: Vec::new(),
        }
    }

    pub fn rounds(&self) -> &[RoundRecord] {
        &self.rounds
    }

    pub fn round_count(&self) -> u32 {
        self.rounds.len() as u32
    }

    /// Observe the lattice produced by this check. `snapshot_rev` is the worktree
    /// HEAD the caller can pass to `WorkspacePort::reset_hard` on Rollback.
    pub fn observe(&mut self, lattice: DiagnosticLattice, snapshot_rev: String) -> RepairAction {
        let action = decide(&self.rounds, self.budget, &lattice);
        self.rounds.push(RoundRecord {
            round: self.rounds.len() as u32,
            fingerprint: lattice.fingerprint(),
            snapshot_rev,
            lattice,
        });
        action
    }
}

fn decide(
    history: &[RoundRecord],
    budget: RepairBudget,
    lattice: &DiagnosticLattice,
) -> RepairAction {
    let fingerprint = lattice.fingerprint();
    let round = history.len() as u32;

    if let Some(prev) = history.iter().find(|r| r.fingerprint == fingerprint) {
        return RepairAction::Stop {
            reason: StopReason::Oscillation {
                cycle_at: prev.round,
            },
        };
    }

    if lattice.blocking() == 0 {
        return RepairAction::Accept;
    }

    // `max_rounds` is the total number of observations, including the first check.
    if round + 1 >= budget.max_rounds {
        return RepairAction::Stop {
            reason: StopReason::BudgetExhausted,
        };
    }

    let Some(previous) = history.last() else {
        return RepairAction::Continue {
            round: 1,
            negative_constraint: negative_constraint(&DiagnosticLattice::empty(), lattice),
        };
    };

    if syntax_regressed(&previous.lattice, lattice) {
        return RepairAction::Rollback {
            to_round: previous.round,
            to_rev: previous.snapshot_rev.clone(),
            negative_constraint: negative_constraint(&previous.lattice, lattice),
        };
    }

    match compare(&previous.lattice, lattice) {
        LatticeOrder::Improves => RepairAction::Continue {
            round: round + 1,
            negative_constraint: remaining_constraint(lattice),
        },
        LatticeOrder::Equal => RepairAction::Stop {
            reason: StopReason::Oscillation { cycle_at: round },
        },
        LatticeOrder::Regresses => RepairAction::Rollback {
            to_round: previous.round,
            to_rev: previous.snapshot_rev.clone(),
            negative_constraint: negative_constraint(&previous.lattice, lattice),
        },
    }
}

/// Negative constraint injected into the next prompt: codes the model must not
/// reintroduce, plus any syntax ban after a rollback.
pub fn negative_constraint(baseline: &DiagnosticLattice, next: &DiagnosticLattice) -> String {
    let mut lines = Vec::new();
    if syntax_regressed(baseline, next) {
        lines.push(
            "NEGATIVE CONSTRAINT: do not introduce new syntax/parse errors. \
             Restore a parseable tree before attempting other fixes."
                .to_string(),
        );
    }
    let introduced = next.introduced_since(baseline);
    if !introduced.is_empty() {
        lines.push("NEGATIVE CONSTRAINT: do not reintroduce:".into());
        for atom in &introduced {
            lines.push(format!(
                "  - {} {} {}",
                atom.severity.as_str(),
                atom.code,
                atom.path
            ));
        }
    }
    if lines.is_empty() {
        remaining_constraint(next)
    } else {
        lines.join("\n")
    }
}

fn remaining_constraint(lattice: &DiagnosticLattice) -> String {
    let mut lines = vec!["REMAINING DIAGNOSTICS (fix these, introduce nothing new):".into()];
    for atom in lattice.atoms() {
        lines.push(format!(
            "  - {} {} {}",
            atom.severity.as_str(),
            atom.code,
            atom.path
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use meshloop_domain::diagnostic::parse_diagnostics;

    fn lat(s: &str) -> DiagnosticLattice {
        parse_diagnostics(s)
    }

    #[test]
    fn empty_diagnostics_accept() {
        let mut s = RepairSession::new(RepairBudget { max_rounds: 3 });
        let action = s.observe(DiagnosticLattice::empty(), "rev0".into());
        assert_eq!(action, RepairAction::Accept);
    }

    #[test]
    fn first_errors_continue_with_constraint() {
        let mut s = RepairSession::new(RepairBudget { max_rounds: 3 });
        let action = s.observe(
            lat("error[E0308]: mismatched types\n --> src/main.rs:2:5\n"),
            "rev1".into(),
        );
        match action {
            RepairAction::Continue {
                round,
                negative_constraint,
            } => {
                assert_eq!(round, 1);
                assert!(negative_constraint.contains("E0308"));
            }
            other => panic!("expected Continue, got {other:?}"),
        }
    }

    #[test]
    fn syntax_regression_rolls_back_to_previous_rev() {
        let mut s = RepairSession::new(RepairBudget { max_rounds: 3 });
        let _ = s.observe(
            lat("error[E0308]: mismatched types\n --> src/main.rs:2:5\n"),
            "rev-type".into(),
        );
        let action = s.observe(
            lat("error: this file contains an unclosed delimiter\n --> src/lib.rs:1:1\n"),
            "rev-syntax".into(),
        );
        match action {
            RepairAction::Rollback {
                to_round,
                to_rev,
                negative_constraint,
            } => {
                assert_eq!(to_round, 0);
                assert_eq!(to_rev, "rev-type");
                assert!(negative_constraint.contains("syntax"));
            }
            other => panic!("expected Rollback, got {other:?}"),
        }
    }

    #[test]
    fn repeated_fingerprint_is_oscillation() {
        let mut s = RepairSession::new(RepairBudget { max_rounds: 5 });
        let d = lat("error[E0308]: mismatched types\n --> src/main.rs:2:5\n");
        let _ = s.observe(d.clone(), "r0".into());
        let action = s.observe(d, "r1".into());
        match action {
            RepairAction::Stop {
                reason: StopReason::Oscillation { cycle_at },
            } => assert_eq!(cycle_at, 0),
            other => panic!("expected oscillation, got {other:?}"),
        }
    }

    #[test]
    fn budget_exhausted_stops() {
        let mut s = RepairSession::new(RepairBudget { max_rounds: 1 });
        let action = s.observe(
            lat("error[E0308]: mismatched types\n --> src/main.rs:2:5\n"),
            "r0".into(),
        );
        assert_eq!(
            action,
            RepairAction::Stop {
                reason: StopReason::BudgetExhausted
            }
        );
    }

    #[test]
    fn progress_on_fewer_type_errors_continues() {
        let mut s = RepairSession::new(RepairBudget { max_rounds: 3 });
        let _ = s.observe(
            lat("error[E0308]: mismatched types\n --> a.rs:1:1\n\
                 error[E0425]: cannot find value `x` in this scope\n --> b.rs:1:1\n"),
            "r0".into(),
        );
        let action = s.observe(
            lat("error[E0308]: mismatched types\n --> a.rs:1:1\n"),
            "r1".into(),
        );
        assert!(matches!(action, RepairAction::Continue { round: 2, .. }));
    }
}
