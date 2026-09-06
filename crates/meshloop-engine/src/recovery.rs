//! Event sourcing (design-patterns.md): state is never mutated in place; it is derived by
//! folding the durable event log. `reconcile` is what makes crash recovery possible —
//! a persisted `Running` state with no matching live process becomes `Blocked`, never a
//! silent retry (ADR 0005).

use std::collections::{HashMap, HashSet};

use meshloop_domain::evidence::AttemptId;
use meshloop_domain::state::{Event, TaskState};
use meshloop_domain::task_graph::TaskId;

pub type AttemptKey = (TaskId, AttemptId);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionRecord {
    pub task_id: TaskId,
    pub attempt_id: AttemptId,
    pub from: TaskState,
    pub to: TaskState,
    pub event: Event,
}

/// Folds the event log into a current-state projection per attempt. This is the only
/// legitimate source of "current state" — never an independently mutated column.
pub fn replay(records: &[TransitionRecord]) -> HashMap<AttemptKey, TaskState> {
    let mut projection = HashMap::new();
    for record in records {
        projection.insert((record.task_id, record.attempt_id), record.to);
    }
    projection
}

/// Reconciles the replayed projection against which attempts have a confirmed-live
/// process. An attempt frozen in `Running` with no matching live process is `Blocked`,
/// never resumed on a guess.
pub fn reconcile(
    records: &[TransitionRecord],
    live_processes: &HashSet<AttemptKey>,
) -> HashMap<AttemptKey, TaskState> {
    let mut projection = replay(records);
    for (key, state) in projection.iter_mut() {
        if *state == TaskState::Running && !live_processes.contains(key) {
            *state = TaskState::Blocked;
        }
    }
    projection
}

#[cfg(test)]
mod tests {
    use super::*;
    use meshloop_domain::state::Event as E;

    fn record(
        task: u32,
        attempt: u32,
        from: TaskState,
        to: TaskState,
        event: E,
    ) -> TransitionRecord {
        TransitionRecord {
            task_id: TaskId(task),
            attempt_id: AttemptId(attempt),
            from,
            to,
            event,
        }
    }

    #[test]
    fn replay_is_deterministic_across_repeated_folds() {
        let records = vec![
            record(
                1,
                1,
                TaskState::Pending,
                TaskState::Ready,
                E::DependencySatisfied,
            ),
            record(
                1,
                1,
                TaskState::Ready,
                TaskState::Running,
                E::AttemptStarted,
            ),
        ];
        assert_eq!(replay(&records), replay(&records));
    }

    #[test]
    fn running_with_no_live_process_reconciles_to_blocked() {
        let records = vec![record(
            1,
            1,
            TaskState::Ready,
            TaskState::Running,
            E::AttemptStarted,
        )];
        let live = HashSet::new();
        let projection = reconcile(&records, &live);
        assert_eq!(projection[&(TaskId(1), AttemptId(1))], TaskState::Blocked);
    }

    #[test]
    fn running_with_a_confirmed_live_process_stays_running() {
        let records = vec![record(
            1,
            1,
            TaskState::Ready,
            TaskState::Running,
            E::AttemptStarted,
        )];
        let mut live = HashSet::new();
        live.insert((TaskId(1), AttemptId(1)));
        let projection = reconcile(&records, &live);
        assert_eq!(projection[&(TaskId(1), AttemptId(1))], TaskState::Running);
    }

    #[test]
    fn non_running_terminal_states_are_never_reconciled_away() {
        let records = vec![record(
            1,
            1,
            TaskState::Accepted,
            TaskState::Integrated,
            E::IntegrationOwnerMerge,
        )];
        let live = HashSet::new();
        let projection = reconcile(&records, &live);
        assert_eq!(
            projection[&(TaskId(1), AttemptId(1))],
            TaskState::Integrated
        );
    }
}
