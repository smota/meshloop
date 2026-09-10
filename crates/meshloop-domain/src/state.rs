//! Table-driven state machine per docs/architecture/execution-lifecycle.md. This is the
//! single source of transition legality — no state check should be duplicated elsewhere.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskState {
    Pending,
    Ready,
    Running,
    Verifying,
    AwaitingReview,
    Accepted,
    Integrated,
    Failed,
    Cancelled,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlanState {
    AwaitingPlanReview,
    PlanAccepted,
    PlanDeclined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlanDecision {
    Accept,
    Decline,
    Adjust,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Event {
    DependencySatisfied,
    DependencyFailedOrScopeRevoked,
    AttemptStarted,
    HarnessExited,
    HarnessCrashedOrTimeout,
    Cancel,
    DeterministicChecksPassed,
    DeterministicChecksFailed,
    ModelReviewPassed,
    HumanAcceptanceRecorded,
    ReviewRejected,
    IntegrationOwnerMerge,
    StaleBaseDetected,
    RetryAuthorized,
    /// Premature Failed while the Herdr pane was still live; worker has now settled.
    LiveWorkerSettled,
    /// Upstream is no longer Failed/Cancelled/Blocked; dependents may be pending again.
    DependencyCleared,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IllegalTransition {
    pub from: TaskState,
    pub event: Event,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IllegalPlanTransition {
    pub from: PlanState,
    pub decision: PlanDecision,
}

/// Pure function of (current state, event) -> next state, mirroring
/// execution-lifecycle.md's transition table exactly. Any `(state, event)` pair not
/// listed there must fall through to `Err`, never guess a next state.
pub fn transition(from: TaskState, event: Event) -> Result<TaskState, IllegalTransition> {
    use Event::*;
    use TaskState::*;

    let to = match (from, event) {
        (Pending, DependencySatisfied) => Ready,
        (Pending, DependencyFailedOrScopeRevoked) => Blocked,
        (Ready, AttemptStarted) => Running,
        (Running, HarnessExited) => Verifying,
        (Running, HarnessCrashedOrTimeout) => Failed,
        (Running, Cancel) => Cancelled,
        (Verifying, DeterministicChecksPassed) => AwaitingReview,
        (Verifying, DeterministicChecksFailed) => Failed,
        (AwaitingReview, ModelReviewPassed) => Accepted,
        (AwaitingReview, HumanAcceptanceRecorded) => Accepted,
        (AwaitingReview, ReviewRejected) => Failed,
        (Accepted, IntegrationOwnerMerge) => Integrated,
        (Accepted, StaleBaseDetected) => Failed,
        (Failed, RetryAuthorized) => Ready,
        (Failed, LiveWorkerSettled) => Verifying,
        (Blocked, DependencyCleared) => Pending,
        (state, Cancel) if state != Integrated && state != Cancelled => Cancelled,
        _ => return Err(IllegalTransition { from, event }),
    };
    Ok(to)
}

/// The graph-level gate from runtime-design.md §5: no node may reach `Ready` while its
/// graph is still `AwaitingPlanReview`. Decline is terminal until Adjust reopens review.
pub fn plan_transition(
    from: PlanState,
    decision: PlanDecision,
) -> Result<PlanState, IllegalPlanTransition> {
    match (from, decision) {
        (PlanState::AwaitingPlanReview, PlanDecision::Accept) => Ok(PlanState::PlanAccepted),
        (PlanState::AwaitingPlanReview, PlanDecision::Decline) => Ok(PlanState::PlanDeclined),
        (PlanState::AwaitingPlanReview, PlanDecision::Adjust) => Ok(PlanState::AwaitingPlanReview),
        (PlanState::PlanAccepted, PlanDecision::Accept) => Ok(PlanState::PlanAccepted),
        (PlanState::PlanDeclined, PlanDecision::Decline) => Ok(PlanState::PlanDeclined),
        (PlanState::PlanDeclined, PlanDecision::Adjust) => Ok(PlanState::AwaitingPlanReview),
        (from, decision) => Err(IllegalPlanTransition { from, decision }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Event::*;
    use TaskState::*;

    const TABLE: &[(TaskState, Event, TaskState)] = &[
        (Pending, DependencySatisfied, Ready),
        (Pending, DependencyFailedOrScopeRevoked, Blocked),
        (Ready, AttemptStarted, Running),
        (Running, HarnessExited, Verifying),
        (Running, HarnessCrashedOrTimeout, Failed),
        (Running, Cancel, Cancelled),
        (Verifying, DeterministicChecksPassed, AwaitingReview),
        (Verifying, DeterministicChecksFailed, Failed),
        (AwaitingReview, ModelReviewPassed, Accepted),
        (AwaitingReview, HumanAcceptanceRecorded, Accepted),
        (AwaitingReview, ReviewRejected, Failed),
        (Accepted, IntegrationOwnerMerge, Integrated),
        (Accepted, StaleBaseDetected, Failed),
        (Failed, RetryAuthorized, Ready),
        (Failed, LiveWorkerSettled, Verifying),
        (Blocked, DependencyCleared, Pending),
    ];

    #[test]
    fn every_documented_transition_succeeds_exactly_as_specified() {
        for (from, event, expected) in TABLE.iter().copied() {
            assert_eq!(transition(from, event), Ok(expected));
        }
    }

    #[test]
    fn every_undocumented_pair_is_illegal() {
        let all_states = [
            Pending,
            Ready,
            Running,
            Verifying,
            AwaitingReview,
            Accepted,
            Integrated,
            Failed,
            Cancelled,
            Blocked,
        ];
        let all_events = [
            DependencySatisfied,
            DependencyFailedOrScopeRevoked,
            AttemptStarted,
            HarnessExited,
            HarnessCrashedOrTimeout,
            Cancel,
            DeterministicChecksPassed,
            DeterministicChecksFailed,
            ModelReviewPassed,
            HumanAcceptanceRecorded,
            ReviewRejected,
            IntegrationOwnerMerge,
            StaleBaseDetected,
            RetryAuthorized,
            LiveWorkerSettled,
            DependencyCleared,
        ];
        for &state in &all_states {
            for &event in &all_events {
                let documented = TABLE.iter().any(|(f, e, _)| *f == state && *e == event);
                let is_cancel_wildcard =
                    event == Cancel && state != Integrated && state != Cancelled;
                if documented || is_cancel_wildcard {
                    assert!(transition(state, event).is_ok());
                } else {
                    assert!(
                        transition(state, event).is_err(),
                        "expected illegal: {state:?} + {event:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn integrated_and_cancelled_reject_cancel() {
        assert!(transition(Integrated, Cancel).is_err());
        assert!(transition(Cancelled, Cancel).is_err());
    }

    #[test]
    fn retry_never_reuses_the_failed_state_directly() {
        assert_eq!(transition(Failed, RetryAuthorized), Ok(Ready));
    }

    #[test]
    fn plan_gate_accept_decline_adjust() {
        assert_eq!(
            plan_transition(PlanState::AwaitingPlanReview, PlanDecision::Accept),
            Ok(PlanState::PlanAccepted)
        );
        assert_eq!(
            plan_transition(PlanState::AwaitingPlanReview, PlanDecision::Decline),
            Ok(PlanState::PlanDeclined)
        );
        assert_eq!(
            plan_transition(PlanState::AwaitingPlanReview, PlanDecision::Adjust),
            Ok(PlanState::AwaitingPlanReview)
        );
        assert_eq!(
            plan_transition(PlanState::PlanAccepted, PlanDecision::Accept),
            Ok(PlanState::PlanAccepted)
        );
        assert!(plan_transition(PlanState::PlanAccepted, PlanDecision::Decline).is_err());
        assert!(plan_transition(PlanState::PlanAccepted, PlanDecision::Adjust).is_err());
        assert!(plan_transition(PlanState::PlanDeclined, PlanDecision::Accept).is_err());
        assert_eq!(
            plan_transition(PlanState::PlanDeclined, PlanDecision::Adjust),
            Ok(PlanState::AwaitingPlanReview)
        );
    }
}
