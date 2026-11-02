//! System 1 decision values (ADR 0032). Pure data only: no ports, adapters,
//! sockets, or network types.

use crate::task_graph::Tier;

/// Finite probability in `0.0..=1.0`. Endpoints are valid. NaN, infinities, and
/// values outside that interval are not.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Confidence(f64);

impl Confidence {
    pub fn try_new(value: f64) -> Result<Self, DecisionError> {
        if value.is_finite() && (0.0..=1.0).contains(&value) {
            Ok(Self(value))
        } else {
            Err(DecisionError::ConfidenceOutOfRange)
        }
    }

    pub fn get(self) -> f64 {
        self.0
    }
}

/// Non-empty abstain text. ASCII controls, including newlines, are rejected so a
/// reason cannot be a raw multiline dump.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SanitizedReason(String);

impl SanitizedReason {
    pub fn try_new(value: impl Into<String>) -> Result<Self, DecisionError> {
        let value = value.into();
        if value.is_empty() {
            return Err(DecisionError::EmptyReason);
        }
        if value.chars().any(|c| c.is_ascii_control()) {
            return Err(DecisionError::ControlInReason);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Non-empty selected choice. Empty is not representable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceId(String);

impl ChoiceId {
    pub fn try_new(value: impl Into<String>) -> Result<Self, DecisionError> {
        let value = value.into();
        if value.is_empty() {
            return Err(DecisionError::EmptySelection);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Finite score. Not a unit interval: any finite `f64` is valid. NaN and
/// infinities are not.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FiniteScore(f64);

impl FiniteScore {
    pub fn try_new(value: f64) -> Result<Self, DecisionError> {
        if value.is_finite() {
            Ok(Self(value))
        } else {
            Err(DecisionError::NonFiniteScore)
        }
    }

    pub fn get(self) -> f64 {
        self.0
    }
}

/// Outcome of a decision port. Invalid selections and non-finite scores cannot
/// be built: those variants hold validated newtypes.
///
/// No `Eq`: `Score` holds `f64`, which is only `PartialEq`.
#[derive(Debug, Clone, PartialEq)]
pub enum DecisionOutcome {
    Selected(ChoiceId),
    Boolean(bool),
    Score(FiniteScore),
    Abstain(SanitizedReason),
}

impl DecisionOutcome {
    pub fn try_selected(choice: impl Into<String>) -> Result<Self, DecisionError> {
        Ok(Self::Selected(ChoiceId::try_new(choice)?))
    }

    pub fn try_score(score: f64) -> Result<Self, DecisionError> {
        Ok(Self::Score(FiniteScore::try_new(score)?))
    }

    pub fn abstain(reason: SanitizedReason) -> Self {
        Self::Abstain(reason)
    }
}

/// Raise-only tier floor. `apply` returns `max(declared, floor)` and never a
/// tier below `declared`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RiskFloor(Tier);

impl RiskFloor {
    pub const fn new(floor: Tier) -> Self {
        Self(floor)
    }

    pub const fn floor(self) -> Tier {
        self.0
    }

    pub fn apply(self, declared: Tier) -> Tier {
        std::cmp::max(declared, self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionError {
    ConfidenceOutOfRange,
    EmptyReason,
    ControlInReason,
    EmptySelection,
    NonFiniteScore,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confidence_accepts_endpoints_and_rejects_out_of_range() {
        assert_eq!(Confidence::try_new(0.0).unwrap().get(), 0.0);
        assert_eq!(Confidence::try_new(1.0).unwrap().get(), 1.0);
        assert_eq!(
            Confidence::try_new(-0.1),
            Err(DecisionError::ConfidenceOutOfRange)
        );
        assert_eq!(
            Confidence::try_new(1.1),
            Err(DecisionError::ConfidenceOutOfRange)
        );
        assert_eq!(
            Confidence::try_new(f64::NAN),
            Err(DecisionError::ConfidenceOutOfRange)
        );
        assert_eq!(
            Confidence::try_new(f64::INFINITY),
            Err(DecisionError::ConfidenceOutOfRange)
        );
        assert_eq!(
            Confidence::try_new(f64::NEG_INFINITY),
            Err(DecisionError::ConfidenceOutOfRange)
        );
    }

    #[test]
    fn risk_floor_never_downgrades() {
        assert_eq!(RiskFloor::new(Tier::Tier3).apply(Tier::Tier1), Tier::Tier3);
        assert_eq!(RiskFloor::new(Tier::Tier1).apply(Tier::Tier3), Tier::Tier3);
    }

    #[test]
    fn sanitized_reason_rejects_empty_and_newline() {
        assert_eq!(
            SanitizedReason::try_new(""),
            Err(DecisionError::EmptyReason)
        );
        assert_eq!(
            SanitizedReason::try_new("line\nbreak"),
            Err(DecisionError::ControlInReason)
        );
        assert_eq!(
            SanitizedReason::try_new("line\rbreak"),
            Err(DecisionError::ControlInReason)
        );
        let reason = SanitizedReason::try_new("needs a human").unwrap();
        assert_eq!(reason.as_str(), "needs a human");
    }

    #[test]
    fn decision_outcome_rejects_empty_selection_and_non_finite_score() {
        assert_eq!(
            DecisionOutcome::try_selected(""),
            Err(DecisionError::EmptySelection)
        );
        assert_eq!(
            DecisionOutcome::try_score(f64::NAN),
            Err(DecisionError::NonFiniteScore)
        );
        assert_eq!(
            DecisionOutcome::try_score(f64::INFINITY),
            Err(DecisionError::NonFiniteScore)
        );
        let selected = DecisionOutcome::try_selected("keep").unwrap();
        assert!(matches!(selected, DecisionOutcome::Selected(id) if id.as_str() == "keep"));
        let scored = DecisionOutcome::try_score(-1.5).unwrap();
        assert!(matches!(scored, DecisionOutcome::Score(score) if score.get() == -1.5));
        let reason = SanitizedReason::try_new("needs a human").unwrap();
        let outcome = DecisionOutcome::abstain(reason);
        assert!(matches!(
            outcome,
            DecisionOutcome::Abstain(got) if got.as_str() == "needs a human"
        ));
    }
}
