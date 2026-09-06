//! Event sourcing (design-patterns.md): state is never mutated in place; it is derived by
//! folding the durable event log. Tick/status/resume use the **per-task** fold. The
//! per-attempt index is only for PID, worktree, and inspect. `reconcile` remains a view
//! annotation, never a second source of truth.

use std::collections::{HashMap, HashSet};

use meshloop_domain::evidence::AttemptId;
use meshloop_domain::state::{TaskState, transition};
use meshloop_domain::task_graph::TaskId;

use crate::ports::{StoreError, TransitionRecord};

pub type AttemptKey = (TaskId, AttemptId);

/// Last-write-wins per `(task, attempt)` for inspect / PID liveness. Does not validate
/// legality; the per-task fold is the source of truth for `tick`.
pub fn replay(records: &[TransitionRecord]) -> HashMap<AttemptKey, TaskState> {
    let mut projection = HashMap::new();
    for record in records {
        if let Some(attempt) = record.attempt_id {
            projection.insert((record.task_id, attempt), record.to);
        }
    }
    projection
}

/// Legal per-task fold: events in order, grouped by `task_id`, through `transition()`.
/// `attempt_id` is metadata on the row, not the projection key. This is what `tick`,
/// `status`, and `resume` read.
pub fn replay_tasks(
    records: &[TransitionRecord],
) -> Result<HashMap<TaskId, TaskState>, StoreError> {
    let mut projection: HashMap<TaskId, TaskState> = HashMap::new();
    for record in records {
        let from = projection
            .get(&record.task_id)
            .copied()
            .unwrap_or(record.from);
        match transition(from, record.event) {
            Ok(to) if to == record.to => {
                projection.insert(record.task_id, to);
            }
            Ok(other) => {
                return Err(StoreError::Corrupt(format!(
                    "illegal fold for task {}: {:?} + {:?} => {:?} (recorded {:?})",
                    record.task_id.0, from, record.event, other, record.to
                )));
            }
            Err(_) => {
                return Err(StoreError::Corrupt(format!(
                    "illegal transition for task {}: {:?} + {:?}",
                    record.task_id.0, from, record.event
                )));
            }
        }
    }
    Ok(projection)
}

/// View only: a persisted `Running` attempt with no matching live process is annotated
/// `Blocked`. Never written to the event log; `resume` appends `HarnessCrashedOrTimeout`.
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
        attempt: Option<u32>,
        from: TaskState,
        to: TaskState,
        event: E,
    ) -> TransitionRecord {
        TransitionRecord {
            graph_id: "g".into(),
            task_id: TaskId(task),
            attempt_id: attempt.map(AttemptId),
            from,
            to,
            event,
            reason: None,
            executor: "meshloop".into(),
            occurred_at: "0".into(),
        }
    }

    #[test]
    fn replay_is_deterministic_across_repeated_folds() {
        let records = vec![
            record(
                1,
                None,
                TaskState::Pending,
                TaskState::Ready,
                E::DependencySatisfied,
            ),
            record(
                1,
                Some(1),
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
            Some(1),
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
            Some(1),
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
            Some(1),
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

    #[test]
    fn per_task_fold_uses_latest_attempt_not_a_failed_predecessor() {
        let records = vec![
            record(
                1,
                None,
                TaskState::Pending,
                TaskState::Ready,
                E::DependencySatisfied,
            ),
            record(
                1,
                Some(1),
                TaskState::Ready,
                TaskState::Running,
                E::AttemptStarted,
            ),
            record(
                1,
                Some(1),
                TaskState::Running,
                TaskState::Failed,
                E::HarnessCrashedOrTimeout,
            ),
            record(
                1,
                Some(1),
                TaskState::Failed,
                TaskState::Ready,
                E::RetryAuthorized,
            ),
            record(
                1,
                Some(2),
                TaskState::Ready,
                TaskState::Running,
                E::AttemptStarted,
            ),
        ];
        let tasks = replay_tasks(&records).expect("legal fold");
        assert_eq!(tasks[&TaskId(1)], TaskState::Running);
    }

    #[test]
    fn per_task_fold_rejects_illegal_triples() {
        let records = vec![record(
            1,
            None,
            TaskState::Pending,
            TaskState::Running,
            E::AttemptStarted,
        )];
        assert!(replay_tasks(&records).is_err());
    }
}
