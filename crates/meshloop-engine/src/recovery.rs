//! Event sourcing (design-patterns.md): state is never mutated in place; it is derived by
//! folding the durable event log. Tick/status/resume use the **per-task** fold. The
//! per-attempt index is only for PID, worktree, and inspect. `reconcile` remains a view
//! annotation, never a second source of truth.

use std::collections::{HashMap, HashSet};

use meshloop_domain::evidence::AttemptId;
use meshloop_domain::state::{Event, TaskState, transition};
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
/// `status`, and `resume` read, and what the store checks every append against.
///
/// One tolerance, for ledgers written before compare-and-append (issue #167): a driver
/// working from a stale view could append the cancelled attempt's own exit, check,
/// verify-gate or retry events after `Cancelled`. Such rows are skipped and the task
/// stays `Cancelled` only as one unbroken chain (see [`StaleChain`]): each row names the
/// attempt the `Cancel` row named, is one a driver records for a running attempt, is
/// itself a legal transition, and starts from the state the previous row of the chain
/// left, the first one from the state the ledger held when `Cancel` was folded. Any
/// other row after `Cancelled`, including a second contradictory result, is still
/// `Corrupt`. No legal event leaves `Cancelled`, so a ledger that folded before folds
/// to exactly the same states now.
pub fn replay_tasks(
    records: &[TransitionRecord],
) -> Result<HashMap<TaskId, TaskState>, StoreError> {
    replay_tasks_marked(records).map(|(projection, _)| projection)
}

/// [`replay_tasks`], and for each record whether the fold skipped it: `true` for a row
/// of a pre-#167 stale chain after `Cancel`, which no reader counts as a transition
/// (#137, so a projection posts only the transitions status shows).
pub fn replay_tasks_marked(
    records: &[TransitionRecord],
) -> Result<(HashMap<TaskId, TaskState>, Vec<bool>), StoreError> {
    let mut projection: HashMap<TaskId, TaskState> = HashMap::new();
    let mut stale: HashMap<TaskId, StaleChain> = HashMap::new();
    let mut skipped = vec![false; records.len()];
    for (index, record) in records.iter().enumerate() {
        let from = projection
            .get(&record.task_id)
            .copied()
            .unwrap_or(record.from);
        if from == TaskState::Cancelled
            && let Some(chain) = stale.get_mut(&record.task_id)
            && chain.continues_with(record)
        {
            skipped[index] = true;
            continue;
        }
        match transition(from, record.event) {
            Ok(to) if to == record.to => {
                projection.insert(record.task_id, to);
                if record.event == Event::Cancel
                    && let Some(attempt) = record.attempt_id
                {
                    stale.insert(
                        record.task_id,
                        StaleChain {
                            attempt,
                            state: from,
                        },
                    );
                }
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
    Ok((projection, skipped))
}

/// The issue #167 signature after a `Cancel`: the driver's own view of the cancelled
/// attempt, carried on as if the cancel had not happened. See [`replay_tasks`].
struct StaleChain {
    /// The attempt the `Cancel` row named.
    attempt: AttemptId,
    /// Where the driver's view stands: the state the ledger held when `Cancel` was
    /// folded, then the `to` of each skipped row.
    state: TaskState,
}

impl StaleChain {
    /// Skips `record` (and advances the chain) when it is the chain's next row.
    fn continues_with(&mut self, record: &TransitionRecord) -> bool {
        let driver_event = matches!(
            record.event,
            Event::HarnessExited
                | Event::HarnessCrashedOrTimeout
                | Event::DeterministicChecksPassed
                | Event::DeterministicChecksFailed
                | Event::VerifyGatePassed
                | Event::LiveWorkerSettled
                | Event::RetryAuthorized
        );
        let next = driver_event
            && record.attempt_id == Some(self.attempt)
            && record.from == self.state
            && transition(record.from, record.event) == Ok(record.to);
        if next {
            self.state = record.to;
        }
        next
    }
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
    fn replay_of_crash_then_harness_exited_stays_failed() {
        let records = vec![
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
                TaskState::Failed,
                E::HarnessExited,
            ),
        ];
        let tasks = replay_tasks(&records).expect("late exit after failure stays readable");
        assert_eq!(tasks[&TaskId(1)], TaskState::Failed);
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

    #[test]
    fn per_task_fold_replays_dynamically_mutated_tasks() {
        let records = vec![
            record(
                1,
                None,
                TaskState::Pending,
                TaskState::Ready,
                E::DependencySatisfied,
            ),
            // Dynamic mutation: task 1 reverts from Ready to Pending, task 2 is inserted as Pending
            record(
                1,
                None,
                TaskState::Ready,
                TaskState::Pending,
                E::GraphMutated,
            ),
            record(
                2,
                None,
                TaskState::Pending,
                TaskState::Ready,
                E::DependencySatisfied,
            ),
        ];
        let tasks = replay_tasks(&records).expect("legal fold");
        assert_eq!(tasks[&TaskId(1)], TaskState::Pending);
        assert_eq!(tasks[&TaskId(2)], TaskState::Ready);
    }

    /// The exact ledger issue #167 left behind, plus the retry the same tick appended.
    fn cancelled_then_stale_driver_rows() -> Vec<TransitionRecord> {
        vec![
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
                TaskState::Verifying,
                E::HarnessExited,
            ),
            record(
                1,
                Some(1),
                TaskState::Verifying,
                TaskState::Cancelled,
                E::Cancel,
            ),
            record(
                1,
                Some(1),
                TaskState::Verifying,
                TaskState::Failed,
                E::DeterministicChecksFailed,
            ),
            record(
                1,
                Some(1),
                TaskState::Failed,
                TaskState::Ready,
                E::RetryAuthorized,
            ),
        ]
    }

    #[test]
    fn issue_167_ledger_folds_to_cancelled() {
        let tasks = replay_tasks(&cancelled_then_stale_driver_rows()).expect("readable");
        assert_eq!(tasks[&TaskId(1)], TaskState::Cancelled);
    }

    #[test]
    fn issue_167_tolerates_an_exit_recorded_after_cancel_while_running() {
        let records = vec![
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
                TaskState::Cancelled,
                E::Cancel,
            ),
            record(
                1,
                Some(1),
                TaskState::Running,
                TaskState::Verifying,
                E::HarnessExited,
            ),
            record(
                1,
                Some(1),
                TaskState::Verifying,
                TaskState::AwaitingReview,
                E::DeterministicChecksPassed,
            ),
        ];
        let tasks = replay_tasks(&records).expect("readable");
        assert_eq!(tasks[&TaskId(1)], TaskState::Cancelled);
    }

    #[test]
    fn rows_after_cancel_outside_the_issue_167_signature_stay_corrupt() {
        let base = || cancelled_then_stale_driver_rows()[..4].to_vec();
        let tails = [
            // Another attempt.
            record(
                1,
                Some(2),
                TaskState::Verifying,
                TaskState::Failed,
                E::DeterministicChecksFailed,
            ),
            // No attempt.
            record(
                1,
                None,
                TaskState::Verifying,
                TaskState::Failed,
                E::DeterministicChecksFailed,
            ),
            // Not a driver event for a running attempt.
            record(
                1,
                Some(1),
                TaskState::AwaitingReview,
                TaskState::Accepted,
                E::HumanAcceptanceRecorded,
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
                TaskState::Verifying,
                TaskState::Cancelled,
                E::Cancel,
            ),
            // Written from Cancelled itself.
            record(
                1,
                Some(1),
                TaskState::Cancelled,
                TaskState::Failed,
                E::DeterministicChecksFailed,
            ),
            // Internally inconsistent row.
            record(
                1,
                Some(1),
                TaskState::Verifying,
                TaskState::Accepted,
                E::DeterministicChecksFailed,
            ),
        ];
        for tail in tails {
            let mut records = base();
            records.push(tail.clone());
            assert!(
                matches!(replay_tasks(&records), Err(StoreError::Corrupt(_))),
                "must stay corrupt: {tail:?}"
            );
        }
    }

    #[test]
    fn issue_167_rows_are_tolerated_only_as_one_chain() {
        // A second, contradictory result for the same attempt does not continue the
        // chain the first one left (Failed), so it stays corrupt.
        let mut records = cancelled_then_stale_driver_rows()[..5].to_vec();
        records.push(record(
            1,
            Some(1),
            TaskState::Verifying,
            TaskState::AwaitingReview,
            E::DeterministicChecksPassed,
        ));
        assert!(matches!(
            replay_tasks(&records),
            Err(StoreError::Corrupt(_))
        ));
        // A chain that skips a state is not the driver's view either.
        let mut records = cancelled_then_stale_driver_rows()[..4].to_vec();
        records.push(record(
            1,
            Some(1),
            TaskState::Failed,
            TaskState::Ready,
            E::RetryAuthorized,
        ));
        assert!(replay_tasks(&records).is_err());
    }

    #[test]
    fn issue_167_chain_starts_from_the_ledger_state_not_a_stale_cancel_from() {
        // `cancel` read Running; the driver recorded the exit before the Cancel landed.
        // The pre-fix fold accepted that Cancel (it reads the ledger state, Verifying),
        // and the driver went on from Verifying.
        let records = vec![
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
                TaskState::Verifying,
                E::HarnessExited,
            ),
            record(
                1,
                Some(1),
                TaskState::Running,
                TaskState::Cancelled,
                E::Cancel,
            ),
            record(
                1,
                Some(1),
                TaskState::Verifying,
                TaskState::Failed,
                E::DeterministicChecksFailed,
            ),
        ];
        let tasks = replay_tasks(&records).expect("readable");
        assert_eq!(tasks[&TaskId(1)], TaskState::Cancelled);
    }

    #[test]
    fn a_cancel_without_an_attempt_tolerates_nothing_after_it() {
        let records = vec![
            record(1, None, TaskState::Pending, TaskState::Cancelled, E::Cancel),
            record(
                1,
                None,
                TaskState::Verifying,
                TaskState::Failed,
                E::DeterministicChecksFailed,
            ),
        ];
        assert!(replay_tasks(&records).is_err());
    }
}
