//! Attempt-scoped self-healing convergence.
//!
//! Repair decisions over [`DiagnosticLattice`] snapshots. The saga stays in
//! `Running`; this module never emits a new `TaskState`. Rollback is a
//! *decision* — the caller applies `WorkspacePort::reset_hard`.
//!
//! Progress φ is the lattice's lexicographic vector (syntax, type, test, error,
//! blocking count). Continue is legal only when φ strictly decreases. A repeated
//! fingerprint is oscillation. A new syntax error is always a rollback.
//!
//! Micro-repair reads two [`DecisionPort`] scores and records one observation.
//! It does not spawn a process or open a socket.

use meshloop_domain::decision::{Confidence, DecisionOutcome};
use meshloop_domain::diagnostic::{
    DiagnosticLattice, DiagnosticSeverity, LatticeOrder, compare, syntax_regressed,
};

use crate::ports::{DecisionPort, ScoreQuery};

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
    /// Redacted check output that produced this lattice. Restored on Rollback
    /// so the next prompt sees the pre-regression diagnostics, not the syntax
    /// failure that triggered `reset_hard`.
    pub diag: String,
    pub action: RepairAction,
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

    pub fn record_at(&self, round: u32) -> Option<&RoundRecord> {
        self.rounds.iter().find(|r| r.round == round)
    }

    pub fn last_action(&self) -> Option<&RepairAction> {
        self.rounds.last().map(|r| &r.action)
    }

    /// Compact, secret-free trajectory for a `DeterministicEvidence` row.
    pub fn trajectory_redacted(&self) -> String {
        let mut lines = Vec::with_capacity(self.rounds.len() + 1);
        lines.push(format!("repair-session rounds={}", self.rounds.len()));
        for r in &self.rounds {
            let p = r.lattice.progress();
            lines.push(format!(
                "round={} rev={} fingerprint={:016x} phi={} syntax={} type={} test={} blocking={} action={}",
                r.round,
                r.snapshot_rev,
                r.fingerprint,
                r.lattice.blocking(),
                p.syntax,
                p.type_errors,
                p.tests,
                r.lattice.blocking(),
                r.action.summary(),
            ));
        }
        lines.join("\n")
    }

    /// Observe the lattice produced by this check. `snapshot_rev` is the worktree
    /// HEAD the caller can pass to `WorkspacePort::reset_hard` on Rollback.
    ///
    /// The caller must not observe a worktree restored by Rollback: that snapshot
    /// is already in `rounds` and a second observe would be classified as
    /// `Stop { Oscillation }`.
    pub fn observe(
        &mut self,
        lattice: DiagnosticLattice,
        snapshot_rev: impl Into<String>,
        diag: impl Into<String>,
    ) -> RepairAction {
        let action = decide(&self.rounds, self.budget, &lattice);
        self.rounds.push(RoundRecord {
            round: self.rounds.len() as u32,
            fingerprint: lattice.fingerprint(),
            snapshot_rev: snapshot_rev.into(),
            lattice,
            diag: diag.into(),
            action: action.clone(),
        });
        action
    }
}

impl RepairAction {
    pub fn summary(&self) -> String {
        match self {
            Self::Accept => "Accept".into(),
            Self::Continue { round, .. } => format!("Continue:{round}"),
            Self::Rollback {
                to_round, to_rev, ..
            } => {
                format!("Rollback:{to_round}:{to_rev}")
            }
            Self::Stop { reason } => format!("Stop:{reason:?}"),
        }
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

/// Blocking codes a micro-repair may close before the heavy harness.
pub const ATOM_ALLOWLIST: &[&str] = &["E0425", "unused_imports"];

const MECHANICAL_MIN: f64 = 0.70;
const SEMANTIC_MAX: f64 = 0.20;

fn unit_interval(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

/// Eligible only for a blocking, syntax-free lattice whose codes are all allowlisted
/// and whose scores sit in the mechanical/semantic window.
pub fn micro_repair_eligible(lattice: &DiagnosticLattice, mechanical: f64, semantic: f64) -> bool {
    if lattice.count(DiagnosticSeverity::Syntax) != 0 || lattice.blocking() == 0 {
        return false;
    }
    let codes_allowed = lattice
        .atoms()
        .filter(|atom| atom.severity < DiagnosticSeverity::Warning)
        .all(|atom| ATOM_ALLOWLIST.contains(&atom.code.as_str()));
    codes_allowed
        && unit_interval(mechanical)
        && mechanical >= MECHANICAL_MIN
        && unit_interval(semantic)
        && semantic <= SEMANTIC_MAX
}

fn unit_score(port: &dyn DecisionPort, subject: &str) -> Option<f64> {
    let outcome = port
        .evaluate_score(&ScoreQuery {
            subject: subject.to_owned(),
        })
        .ok()?;
    match outcome {
        DecisionOutcome::Score(score) => Confidence::try_new(score.get()).ok().map(Confidence::get),
        DecisionOutcome::Boolean(_)
        | DecisionOutcome::Selected(_)
        | DecisionOutcome::Abstain(_) => None,
    }
}

/// Two scores, or `None` when either call is not a unit-interval score.
fn repair_probabilities(port: &dyn DecisionPort, description: &str) -> Option<(f64, f64)> {
    let mechanical = unit_score(port, &format!("mechanical\n{description}"));
    let semantic = unit_score(port, &format!("semantic\n{description}"));
    match (mechanical, semantic) {
        (Some(mechanical), Some(semantic)) => Some((mechanical, semantic)),
        _ => None,
    }
}

/// One closed edit. `true` means the edit was applied. No path and no process.
pub trait MicroRepairActuator {
    fn apply(&mut self, lattice: &DiagnosticLattice) -> bool;
}

/// One observation of `after`. Phi is the caller's [`compare`], not this function.
pub fn micro_observation(
    session: &mut RepairSession,
    before: &DiagnosticLattice,
    after: &DiagnosticLattice,
    snapshot_rev: &str,
    diag: &str,
) -> RepairAction {
    let _ = before;
    session.observe(after.clone(), snapshot_rev, diag)
}

/// What the repair loop should do after a micro-repair attempt.
#[derive(Debug, PartialEq)]
pub(crate) enum MicroRepairReport {
    /// Phi improved. Do not call `dispatch_repair_round` this iteration.
    Improved {
        action: RepairAction,
        lattice: DiagnosticLattice,
        diag: String,
        revision: String,
    },
    /// Heavy dispatch. Any applied edit was reset to `prior_revision`.
    Heavy,
    /// `reset_hard` failed after an edit. Do not dispatch.
    Abort,
}

pub(crate) enum MicroRepairHost {
    Rerun,
    Reset { revision: String },
}

pub(crate) enum MicroRepairHostReply {
    Reran {
        lattice: DiagnosticLattice,
        revision: String,
        diag: String,
    },
    Unavailable,
    ResetDone,
    ResetFailed,
}

fn reset_micro_edit(
    host: &mut impl FnMut(MicroRepairHost) -> MicroRepairHostReply,
    prior_revision: &str,
) -> MicroRepairReport {
    match host(MicroRepairHost::Reset {
        revision: prior_revision.to_string(),
    }) {
        MicroRepairHostReply::ResetFailed => MicroRepairReport::Abort,
        MicroRepairHostReply::ResetDone
        | MicroRepairHostReply::Unavailable
        | MicroRepairHostReply::Reran { .. } => MicroRepairReport::Heavy,
    }
}

/// Try one closed edit. `Heavy` and `Abort` are the only results that do not keep it.
pub(crate) fn consider_micro_repair(
    session: &mut RepairSession,
    before: &DiagnosticLattice,
    prior_revision: &str,
    description: &str,
    port: &dyn DecisionPort,
    actuator: &mut dyn MicroRepairActuator,
    mut host: impl FnMut(MicroRepairHost) -> MicroRepairHostReply,
) -> MicroRepairReport {
    let Some((mechanical, semantic)) = repair_probabilities(port, description) else {
        return MicroRepairReport::Heavy;
    };
    if !micro_repair_eligible(before, mechanical, semantic) || !actuator.apply(before) {
        return MicroRepairReport::Heavy;
    }
    let (after, after_revision, after_diag) = match host(MicroRepairHost::Rerun) {
        MicroRepairHostReply::Reran {
            lattice,
            revision,
            diag,
        } => (lattice, revision, diag),
        MicroRepairHostReply::Unavailable
        | MicroRepairHostReply::ResetDone
        | MicroRepairHostReply::ResetFailed => {
            return reset_micro_edit(&mut host, prior_revision);
        }
    };
    let action = micro_observation(session, before, &after, &after_revision, &after_diag);
    if compare(before, &after) == LatticeOrder::Improves {
        MicroRepairReport::Improved {
            action,
            lattice: after,
            diag: after_diag,
            revision: after_revision,
        }
    } else {
        reset_micro_edit(&mut host, prior_revision)
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
        let action = s.observe(DiagnosticLattice::empty(), "rev0", "");
        assert_eq!(action, RepairAction::Accept);
        assert_eq!(s.rounds().len(), 1);
        assert_eq!(s.rounds()[0].lattice.blocking(), 0);
        assert!(matches!(s.rounds()[0].action, RepairAction::Accept));
    }

    #[test]
    fn first_errors_continue_with_constraint() {
        let mut s = RepairSession::new(RepairBudget { max_rounds: 3 });
        let diag = "error[E0308]: mismatched types\n --> src/main.rs:2:5\n";
        let action = s.observe(lat(diag), "rev1", diag);
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
        assert_eq!(s.rounds()[0].diag, diag);
    }

    #[test]
    fn syntax_regression_rolls_back_to_previous_rev() {
        let mut s = RepairSession::new(RepairBudget { max_rounds: 3 });
        let type_diag = "error[E0308]: mismatched types\n --> src/main.rs:2:5\n";
        let _ = s.observe(lat(type_diag), "rev-type", type_diag);
        let action = s.observe(
            lat("error: this file contains an unclosed delimiter\n --> src/lib.rs:1:1\n"),
            "rev-syntax",
            "syntax-diag",
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
                let restored = s.record_at(to_round).expect("restored round");
                assert_eq!(restored.diag, type_diag);
                assert_eq!(restored.snapshot_rev, "rev-type");
            }
            other => panic!("expected Rollback, got {other:?}"),
        }
    }

    #[test]
    fn repeated_fingerprint_is_oscillation() {
        let mut s = RepairSession::new(RepairBudget { max_rounds: 5 });
        let d = lat("error[E0308]: mismatched types\n --> src/main.rs:2:5\n");
        let _ = s.observe(d.clone(), "r0", "d0");
        let action = s.observe(d, "r1", "d1");
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
            "r0",
            "",
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
        let two = "error[E0308]: mismatched types\n --> a.rs:1:1\n\
                 error[E0425]: cannot find value `x` in this scope\n --> b.rs:1:1\n";
        let one = "error[E0308]: mismatched types\n --> a.rs:1:1\n";
        let _ = s.observe(lat(two), "r0", two);
        let action = s.observe(lat(one), "r1", one);
        assert!(matches!(action, RepairAction::Continue { round: 2, .. }));
    }

    #[test]
    fn third_observation_at_phi_zero_is_recorded_as_accept() {
        let mut s = RepairSession::new(RepairBudget { max_rounds: 3 });
        let two = "error[E0308]: mismatched types\n --> a.rs:1:1\n\
                 error[E0425]: cannot find value `x` in this scope\n --> b.rs:1:1\n";
        let one = "error[E0308]: mismatched types\n --> a.rs:1:1\n";
        let _ = s.observe(lat(two), "r0", two);
        let _ = s.observe(lat(one), "r1", one);
        let action = s.observe(DiagnosticLattice::empty(), "r2", "");
        assert_eq!(action, RepairAction::Accept);
        assert_eq!(s.rounds().len(), 3);
        assert_eq!(s.rounds()[2].lattice.blocking(), 0);
        assert!(matches!(s.rounds()[2].action, RepairAction::Accept));
        let traj = s.trajectory_redacted();
        assert!(traj.contains("action=Accept"));
        assert!(traj.contains("phi=0"));
    }

    #[test]
    fn third_blocking_observation_exhausts_three_round_budget() {
        let mut s = RepairSession::new(RepairBudget { max_rounds: 3 });
        let two = "error[E0308]: mismatched types\n --> a.rs:1:1\n\
                 error[E0425]: cannot find value `x` in this scope\n --> b.rs:1:1\n";
        let one = "error[E0308]: mismatched types\n --> a.rs:1:1\n";
        let other = "error[E0412]: cannot find type `T` in this scope\n --> a.rs:1:1\n";
        let _ = s.observe(lat(two), "r0", two);
        let _ = s.observe(lat(one), "r1", one);
        let action = s.observe(lat(other), "r2", other);
        assert_eq!(
            action,
            RepairAction::Stop {
                reason: StopReason::BudgetExhausted
            }
        );
        assert_eq!(s.round_count(), 3);
    }

    use std::cell::RefCell;

    use meshloop_domain::decision::SanitizedReason;
    use meshloop_domain::diagnostic::DiagnosticAtom;

    use crate::ports::{BatchNoulQuery, ChoiceQuery, DecisionPortError, NoulQuery};

    struct ScoreScript {
        mechanical: Result<DecisionOutcome, DecisionPortError>,
        semantic: Result<DecisionOutcome, DecisionPortError>,
        subjects: RefCell<Vec<String>>,
    }

    impl ScoreScript {
        fn pair(mechanical: f64, semantic: f64) -> Self {
            Self {
                mechanical: Ok(DecisionOutcome::try_score(mechanical).expect("finite")),
                semantic: Ok(DecisionOutcome::try_score(semantic).expect("finite")),
                subjects: RefCell::new(Vec::new()),
            }
        }
    }

    impl DecisionPort for ScoreScript {
        fn evaluate_choice(
            &self,
            _query: &ChoiceQuery,
        ) -> Result<DecisionOutcome, DecisionPortError> {
            Err(DecisionPortError::EmptyQuery)
        }

        fn evaluate_noul(&self, _query: &NoulQuery) -> Result<DecisionOutcome, DecisionPortError> {
            Err(DecisionPortError::EmptyQuery)
        }

        fn evaluate_score(&self, query: &ScoreQuery) -> Result<DecisionOutcome, DecisionPortError> {
            self.subjects.borrow_mut().push(query.subject.clone());
            if query.subject.starts_with("mechanical\n") {
                self.mechanical.clone()
            } else if query.subject.starts_with("semantic\n") {
                self.semantic.clone()
            } else {
                Err(DecisionPortError::Unscripted {
                    fingerprint: query.subject.clone(),
                })
            }
        }

        fn evaluate_batch_noul(
            &self,
            _query: &BatchNoulQuery,
        ) -> Result<Vec<DecisionOutcome>, DecisionPortError> {
            Err(DecisionPortError::EmptyQuery)
        }
    }

    struct CountingActuator {
        calls: u32,
        applied: bool,
    }

    impl MicroRepairActuator for CountingActuator {
        fn apply(&mut self, _lattice: &DiagnosticLattice) -> bool {
            self.calls += 1;
            self.applied
        }
    }

    struct MicroDrive {
        report: MicroRepairReport,
        calls: u32,
        resets: u32,
        /// Incremented only when the helper asks for the heavy path.
        dispatches: u32,
        rounds: u32,
    }

    fn atom(severity: DiagnosticSeverity, code: &str) -> DiagnosticLattice {
        DiagnosticLattice::new([DiagnosticAtom {
            severity,
            code: code.to_string(),
            path: "lib.rs".into(),
            template: 1,
        }])
    }

    fn e0425() -> DiagnosticLattice {
        lat("error[E0425]: cannot find value `x` in this scope\n --> src/lib.rs:1:1\n")
    }

    #[test]
    fn micro_repair_eligible_allowlists_e0425_at_thresholds() {
        let lattice = e0425();
        assert_eq!(lattice.count(DiagnosticSeverity::Syntax), 0);
        assert!(lattice.blocking() > 0);
        assert!(lattice.atoms().any(|atom| atom.code == "E0425"));
        assert!(micro_repair_eligible(&lattice, 0.70, 0.20));
        assert!(micro_repair_eligible(
            &atom(DiagnosticSeverity::Type, "unused_imports"),
            0.70,
            0.20
        ));
        let with_warning = parse_diagnostics(
            "error[E0425]: cannot find value `x` in this scope\n --> src/lib.rs:1:1\n\
             warning: unused variable: `y`\n --> src/lib.rs:2:5\n",
        );
        assert!(micro_repair_eligible(&with_warning, 0.70, 0.20));
    }

    #[test]
    fn micro_repair_rejects_low_scores_syntax_and_e0308() {
        let lattice = e0425();
        assert!(!micro_repair_eligible(&lattice, 0.69, 0.20));
        assert!(!micro_repair_eligible(&lattice, 0.70, 0.21));
        assert!(!micro_repair_eligible(&lattice, f64::NAN, 0.20));
        assert!(!micro_repair_eligible(&lattice, 1.5, 0.20));
        assert!(!micro_repair_eligible(&lattice, 0.70, f64::INFINITY));
        assert!(!micro_repair_eligible(
            &DiagnosticLattice::empty(),
            0.70,
            0.20
        ));
        assert!(!micro_repair_eligible(
            &atom(DiagnosticSeverity::Syntax, "E0425"),
            0.70,
            0.20
        ));
        let syntax = lat("error: this file contains an unclosed delimiter\n --> src/lib.rs:1:1\n");
        assert!(syntax.count(DiagnosticSeverity::Syntax) > 0);
        assert!(!micro_repair_eligible(&syntax, 0.70, 0.20));
        let mismatched = lat("error[E0308]: mismatched types\n --> src/main.rs:2:5\n");
        assert!(mismatched.atoms().any(|atom| atom.code == "E0308"));
        assert!(!micro_repair_eligible(&mismatched, 0.70, 0.20));
        let mixed = parse_diagnostics(
            "error[E0425]: cannot find value `x` in this scope\n --> a.rs:1:1\n\
             error[E0308]: mismatched types\n --> b.rs:1:1\n",
        );
        assert!(!micro_repair_eligible(&mixed, 0.70, 0.20));
    }

    #[test]
    fn micro_repair_probabilities_require_unit_interval_scores() {
        let description = "missing x";
        let mut port = ScoreScript::pair(0.70, 0.20);
        assert_eq!(repair_probabilities(&port, description), Some((0.70, 0.20)));
        assert_eq!(
            port.subjects.borrow().as_slice(),
            [
                "mechanical\nmissing x".to_string(),
                "semantic\nmissing x".to_string()
            ]
        );
        port.mechanical = Ok(DecisionOutcome::Boolean(true));
        assert_eq!(repair_probabilities(&port, description), None);
        port.mechanical = Ok(DecisionOutcome::try_score(0.70).expect("finite"));
        port.semantic = Ok(DecisionOutcome::try_score(1.5).expect("finite"));
        assert_eq!(repair_probabilities(&port, description), None);
        port.semantic = Ok(DecisionOutcome::abstain(
            SanitizedReason::try_new("no score").expect("reason"),
        ));
        assert_eq!(repair_probabilities(&port, description), None);
        port.semantic = Err(DecisionPortError::EmptyQuery);
        assert_eq!(repair_probabilities(&port, description), None);
        port.semantic = Ok(DecisionOutcome::try_selected("keep").expect("choice"));
        assert_eq!(repair_probabilities(&port, description), None);
    }

    fn drive(
        before: &DiagnosticLattice,
        follow_up: DiagnosticLattice,
        applied: bool,
        port: &ScoreScript,
    ) -> MicroDrive {
        let mut session = RepairSession::new(RepairBudget::default());
        let mut actuator = CountingActuator { calls: 0, applied };
        let mut resets = 0u32;
        let mut dispatches = 0u32;
        let report = consider_micro_repair(
            &mut session,
            before,
            "rev-before",
            "missing x",
            port,
            &mut actuator,
            |request| match request {
                MicroRepairHost::Rerun => MicroRepairHostReply::Reran {
                    lattice: follow_up.clone(),
                    revision: "rev-after".into(),
                    diag: "after".into(),
                },
                MicroRepairHost::Reset { revision } => {
                    assert_eq!(revision, "rev-before");
                    resets += 1;
                    MicroRepairHostReply::ResetDone
                }
            },
        );
        if matches!(report, MicroRepairReport::Heavy | MicroRepairReport::Abort) {
            dispatches += 1;
        }
        MicroDrive {
            report,
            calls: actuator.calls,
            resets,
            dispatches,
            rounds: session.round_count(),
        }
    }

    #[test]
    fn micro_repair_clearing_actuator_observes_once_and_skips_dispatch() {
        let before = e0425();
        let driven = drive(
            &before,
            DiagnosticLattice::empty(),
            true,
            &ScoreScript::pair(0.70, 0.20),
        );
        assert_eq!(
            compare(&before, &DiagnosticLattice::empty()),
            LatticeOrder::Improves
        );
        assert!(matches!(
            driven.report,
            MicroRepairReport::Improved {
                action: RepairAction::Accept,
                ..
            }
        ));
        assert_eq!(driven.calls, 1);
        assert_eq!(driven.rounds, 1, "observe once");
        assert_eq!(driven.resets, 0);
        assert_eq!(driven.dispatches, 0);
    }

    #[test]
    fn micro_repair_without_phi_improvement_resets_once_and_requests_heavy_path() {
        let before = e0425();
        let driven = drive(
            &before,
            before.clone(),
            true,
            &ScoreScript::pair(0.70, 0.20),
        );
        assert_ne!(compare(&before, &before), LatticeOrder::Improves);
        assert_eq!(driven.report, MicroRepairReport::Heavy);
        assert_eq!(driven.calls, 1);
        assert_eq!(driven.resets, 1);
        assert_eq!(
            driven.rounds, 1,
            "the non-improving lattice is observed once"
        );
        assert_eq!(driven.dispatches, 1);
    }

    #[test]
    fn micro_repair_syntax_lattice_never_calls_actuator() {
        let syntax = lat("error: this file contains an unclosed delimiter\n --> src/lib.rs:1:1\n");
        assert!(syntax.count(DiagnosticSeverity::Syntax) > 0);
        let driven = drive(
            &syntax,
            DiagnosticLattice::empty(),
            true,
            &ScoreScript::pair(0.70, 0.20),
        );
        assert_eq!(driven.calls, 0);
        assert_eq!(driven.resets, 0);
        assert_eq!(driven.rounds, 0);
        assert_eq!(driven.report, MicroRepairReport::Heavy);
        assert_eq!(driven.dispatches, 1);
    }

    #[test]
    fn micro_repair_false_apply_does_not_observe_or_reset() {
        let driven = drive(
            &e0425(),
            DiagnosticLattice::empty(),
            false,
            &ScoreScript::pair(0.70, 0.20),
        );
        assert_eq!(driven.calls, 1);
        assert_eq!(driven.rounds, 0);
        assert_eq!(driven.resets, 0);
        assert_eq!(driven.report, MicroRepairReport::Heavy);
    }

    #[test]
    fn micro_repair_probability_error_skips_actuator() {
        let mut port = ScoreScript::pair(0.70, 0.20);
        port.mechanical = Err(DecisionPortError::EmptyQuery);
        let driven = drive(&e0425(), DiagnosticLattice::empty(), true, &port);
        assert_eq!(driven.calls, 0);
        assert_eq!(driven.rounds, 0);
        assert_eq!(driven.resets, 0);
        assert_eq!(driven.report, MicroRepairReport::Heavy);
    }
}
