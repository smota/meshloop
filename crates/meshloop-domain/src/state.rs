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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IllegalTransition {
    pub from: TaskState,
    pub event: Event,
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
        (state, Cancel) if state != Integrated && state != Cancelled => Cancelled,
        _ => return Err(IllegalTransition { from, event }),
    };
    Ok(to)
}

/// The graph-level gate from runtime-design.md §5: no node may reach `Ready` while its
/// graph is still `AwaitingPlanReview`.
pub fn plan_transition(from: PlanState, accepted: bool) -> Result<PlanState, IllegalTransition> {
    match (from, accepted) {
        (PlanState::AwaitingPlanReview, true) => Ok(PlanState::PlanAccepted),
        (PlanState::AwaitingPlanReview, false) => Err(IllegalTransition {
            from: TaskState::Blocked,
            event: Event::ReviewRejected,
        }),
        (PlanState::PlanAccepted, _) => Ok(PlanState::PlanAccepted),
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
    fn plan_gate_blocks_until_accepted() {
        assert_eq!(
            plan_transition(PlanState::AwaitingPlanReview, true),
            Ok(PlanState::PlanAccepted)
        );
        assert!(plan_transition(PlanState::AwaitingPlanReview, false).is_err());
        assert_eq!(
            plan_transition(PlanState::PlanAccepted, false),
            Ok(PlanState::PlanAccepted)
        );
    }
}
