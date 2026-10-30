//! Pinned public observation. Never used by producer replay or recovery.
use std::collections::{HashMap, HashSet};

use meshloop_domain::digest::sha256_hex;
use meshloop_domain::state::{PlanState, TaskState};
use meshloop_domain::task_graph::{TaskGraph, TaskId};
use meshloop_domain::workspace::*;

use super::{ScopedAttempt, StoredAttempt, StoredEvent};
use crate::ports::{RunStore, StoreError, TransitionRecord};
use crate::recovery::replay_tasks;
use crate::run_loop::{
    AttemptView, EvidenceView, NodeStatus, OrchestratorError, RunLoop, RunStatus, RunSummary,
    WaitingFor, duration_secs,
};

pub const BOUND_GIT_EXPORT_UNAVAILABLE: &str =
    "generation-scoped Git export is unavailable until artifact-based export is implemented";

/// Captured once per public command. Watch refuses generation or revision changes.
pub struct ReadSelection {
    graph_id: String,
    bound: Option<BoundSelection>,
}
struct BoundSelection {
    binding: RunBinding,
    scope: ExecutionScope,
    graph: TaskGraph,
    revisions: HashSet<String>,
    initial_revision: LifecycleId,
}
pub struct ReadFrame {
    pub status: RunStatus,
    pub summary: RunSummary,
    pub attempts: HashMap<TaskId, Vec<AttemptView>>,
    pub locations: HashMap<TaskId, PortableLocation>,
    pub attempt_locations: HashMap<u32, PortableLocation>,
    pub attempt_revisions: HashMap<u32, LifecycleId>,
    pub generation_id: Option<LifecycleId>,
    pub revision_id: Option<LifecycleId>,
}
fn mismatch() -> OrchestratorError {
    StoreError::PlanRevisionMismatch.into()
}
/// Preserve typed observation errors while propagating snapshot begin/end errors.
pub(crate) fn read_snapshot<T>(
    store: &dyn RunStore,
    observe: impl FnOnce() -> Result<T, OrchestratorError>,
) -> Result<T, OrchestratorError> {
    let mut observe = Some(observe);
    let mut result = None;
    store.with_read_snapshot(&mut || {
        result = Some(match observe.take() {
            Some(observe) => observe(),
            None => {
                Err(StoreError::Corrupt("snapshot callback invoked more than once".into()).into())
            }
        });
    })?;
    result.ok_or_else(|| {
        OrchestratorError::Store(StoreError::Corrupt(
            "snapshot callback was not invoked".into(),
        ))
    })?
}
impl ReadSelection {
    pub fn capture(store: &dyn RunStore, graph_id: &str) -> Result<Self, OrchestratorError> {
        read_snapshot(store, || Self::capture_snapshot(store, graph_id))
    }
    fn capture_snapshot(store: &dyn RunStore, graph_id: &str) -> Result<Self, OrchestratorError> {
        store
            .load_run(graph_id)?
            .ok_or_else(|| OrchestratorError::MissingGraph(graph_id.into()))?;
        let id = GraphId::new(graph_id).map_err(|_| mismatch())?;
        let Some(binding) = store.run_binding(&id)? else {
            return Ok(Self {
                graph_id: graph_id.into(),
                bound: None,
            });
        };
        if binding.active_generation.is_none() && binding.current_revision.is_none() {
            return Err(OrchestratorError::Illegal(
                "NoAcceptedGeneration: lifecycle-bound run has no accepted execution generation"
                    .into(),
            ));
        }
        let generation_id = binding.active_generation.clone().ok_or_else(mismatch)?;
        let revision_id = binding.current_revision.clone().ok_or_else(mismatch)?;
        let scope = store
            .resolve_scope(&id, &RunScope::Generation(generation_id.clone()))?
            .ok_or_else(mismatch)?;
        let generation = store.generation(&generation_id)?.ok_or_else(mismatch)?;
        if binding.graph_id != id
            || scope.repository_id != binding.repository_id
            || scope.graph_id != id
            || scope.generation_id != generation_id
            || generation.id != generation_id
            || generation.repository_id != binding.repository_id
            || generation.graph_id != id
            || generation.root_alias != binding.location.root_alias
            || generation.layout_version != binding.layout_version
            || store
                .repository(&binding.repository_id)?
                .as_ref()
                .map(|r| &r.id)
                != Some(&binding.repository_id)
        {
            return Err(mismatch());
        }
        let initial = store
            .generation_initial_revision(&scope)?
            .ok_or_else(mismatch)?;
        let mut revisions = HashSet::new();
        let mut accepted_revisions = HashSet::new();
        let mut next = revision_id.clone();
        let mut graph = None;
        loop {
            if revisions.len() >= 4096 {
                return Err(OrchestratorError::Illegal("ReadRevisionLimitExceeded: observation supports at most 4096 revisions per generation".into()));
            }
            if !revisions.insert(next.as_str().to_string()) {
                return Err(mismatch());
            }
            let snapshot = store.revision(&next)?.ok_or_else(mismatch)?;
            let m = &snapshot.metadata;
            if m.id != next
                || m.graph_id != id
                || m.repository_id != binding.repository_id
                || sha256_hex(snapshot.plan_json.as_bytes()) != m.plan_hash.as_str()
            {
                return Err(mismatch());
            }
            let parsed: TaskGraph =
                serde_json::from_str(&snapshot.plan_json).map_err(|_| mismatch())?;
            parsed.validate().map_err(|_| mismatch())?;
            if parsed.graph_id != graph_id {
                return Err(mismatch());
            }
            let authorizations = store.authorizations(&next)?;
            if authorizations.iter().any(|a| a.revision_id != next) {
                return Err(mismatch());
            }
            if authorizations
                .iter()
                .any(|a| a.decision == RevisionDecision::Accepted)
            {
                accepted_revisions.insert(next.as_str().to_string());
            }
            if next == revision_id {
                graph = Some(parsed);
            }
            if next == initial {
                if m.plan_hash != generation.accepted_plan_hash
                    || !accepted_revisions.contains(next.as_str())
                {
                    return Err(mismatch());
                }
                break;
            }
            // Only revisions after activation must belong to this generation.
            if m.generation_id
                .as_ref()
                .is_some_and(|g| g != &generation_id)
            {
                return Err(mismatch());
            }
            next = m.predecessor.clone().ok_or_else(mismatch)?;
        }
        let selected = Self {
            graph_id: graph_id.into(),
            bound: Some(BoundSelection {
                binding,
                scope,
                graph: graph.ok_or_else(mismatch)?,
                revisions,
                initial_revision: initial,
            }),
        };
        selected.validate(store)?;
        Ok(selected)
    }
    pub fn is_bound(&self) -> bool {
        self.bound.is_some()
    }
    fn validate(&self, store: &dyn RunStore) -> Result<(), OrchestratorError> {
        let current = store.run_binding(&GraphId::new(&self.graph_id).map_err(|_| mismatch())?)?;
        let expected = self.bound.as_ref().map(|b| &b.binding);
        if current.as_ref() != expected {
            return Err(OrchestratorError::Illegal(
                "ReadScopeChanged: generation or plan revision changed; start a new observation"
                    .into(),
            ));
        }
        Ok(())
    }
}

impl RunLoop<'_> {
    pub fn read_current_frame(&self, graph_id: &str) -> Result<ReadFrame, OrchestratorError> {
        read_snapshot(self.store, || {
            let selection = ReadSelection::capture(self.store, graph_id)?;
            self.read_frame(&selection)
        })
    }
    pub fn read_frame(&self, selection: &ReadSelection) -> Result<ReadFrame, OrchestratorError> {
        read_snapshot(self.store, || self.read_frame_snapshot(selection))
    }
    fn read_frame_snapshot(
        &self,
        selection: &ReadSelection,
    ) -> Result<ReadFrame, OrchestratorError> {
        selection.validate(self.store)?;
        let Some(b) = &selection.bound else {
            let status = self.status(&selection.graph_id)?;
            let summary = self.run_summary(&selection.graph_id)?;
            let mut attempts = HashMap::new();
            for n in &status.nodes {
                attempts.insert(n.task_id, self.attempts(&selection.graph_id, n.task_id)?);
            }
            selection.validate(self.store)?;
            return Ok(ReadFrame {
                status,
                summary,
                attempts,
                locations: HashMap::new(),
                attempt_locations: HashMap::new(),
                attempt_revisions: HashMap::new(),
                generation_id: None,
                revision_id: None,
            });
        };
        let which = RunScope::Generation(b.scope.generation_id.clone());
        // Authorization is append-only but may change without advancing the
        // binding. Refresh it in every frame's snapshot, retaining historical
        // acceptance as execution provenance even after a later decline.
        let mut accepted_revisions = HashSet::new();
        let mut plan_state = PlanState::AwaitingPlanReview;
        for id in &b.revisions {
            let revision = LifecycleId::new(id).map_err(|_| mismatch())?;
            let authorizations = self.store.authorizations(&revision)?;
            if authorizations.iter().any(|a| a.revision_id != revision) {
                return Err(mismatch());
            }
            if authorizations
                .iter()
                .any(|a| a.decision == RevisionDecision::Accepted)
            {
                accepted_revisions.insert(id.clone());
            }
            if Some(&revision) == b.binding.current_revision.as_ref()
                && authorizations
                    .last()
                    .is_some_and(|a| a.decision == RevisionDecision::Accepted)
            {
                plan_state = PlanState::PlanAccepted;
            }
        }
        if !accepted_revisions.contains(b.initial_revision.as_str()) {
            return Err(mismatch());
        }
        let mut events: Vec<TransitionRecord> = Vec::new();
        for event in self.store.scoped_events(&b.scope.graph_id, &which)? {
            match event {
                StoredEvent::Scoped(e)
                    if e.scope == b.scope
                        && b.revisions.contains(e.revision_id.as_str())
                        && accepted_revisions.contains(e.revision_id.as_str())
                        && e.record.graph_id == selection.graph_id =>
                {
                    events.push(e.record)
                }
                _ => return Err(mismatch()),
            }
        }
        let mut rows: Vec<ScopedAttempt> = Vec::new();
        for attempt in self.store.scoped_attempts(&b.scope.graph_id, &which)? {
            match attempt {
                StoredAttempt::Scoped(a)
                    if a.scope == b.scope
                        && b.revisions.contains(a.revision_id.as_str())
                        && accepted_revisions.contains(a.revision_id.as_str()) =>
                {
                    rows.push(a)
                }
                _ => return Err(mismatch()),
            }
        }
        let task_ids: HashSet<_> = b.graph.nodes.iter().map(|n| n.id).collect();
        for a in &rows {
            let artifact = self.store.artifact(&a.artifact_id)?.ok_or_else(mismatch)?;
            if !task_ids.contains(&a.task_id)
                || artifact.generation_id != b.scope.generation_id
                || artifact.kind != ArtifactKind::Attempt
                || artifact.attempt_id != Some(a.attempt_id)
                || artifact.root_alias != a.location.root_alias
                || artifact.relative_path != a.location.relative_path
            {
                return Err(mismatch());
            }
        }
        for e in &events {
            if !task_ids.contains(&e.task_id)
                || e.attempt_id.is_some_and(|id| {
                    !rows
                        .iter()
                        .any(|a| a.attempt_id == id && a.task_id == e.task_id)
                })
            {
                return Err(mismatch());
            }
        }
        rows.sort_by_key(|a| a.attempt_id.0);
        // The adapter joins evidence to its globally unique attempt and graph/generation.
        // Evidence has a candidate Git revision, not a plan revision; plan membership
        // is established by validating the selected attempt's exact revision above.
        let evidence = self.store.scoped_evidence(&b.scope.graph_id, &which)?;
        for e in &evidence {
            if !rows.iter().any(|a| {
                a.attempt_id == e.candidate().attempt_id && a.task_id == e.candidate().task_id
            }) {
                return Err(mismatch());
            }
        }
        selection.validate(self.store)?;
        let tasks = replay_tasks(&events)?;
        let mut attempts: HashMap<TaskId, Vec<AttemptView>> = HashMap::new();
        let mut attempt_locations = HashMap::new();
        let mut attempt_revisions = HashMap::new();
        let mut wall = 0;
        for a in &rows {
            let duration_s = duration_secs(&a.started_at, &a.ended_at);
            wall += duration_s.unwrap_or(0);
            let base_revision = a
                .outcome
                .as_deref()
                .and_then(|o| o.strip_prefix("base:"))
                .map(String::from);
            let outcome = events
                .iter()
                .rev()
                .find(|e| e.attempt_id == Some(a.attempt_id))
                .map(|e| format!("{:?}", e.event))
                .or_else(|| {
                    if base_revision.is_none() {
                        a.outcome.clone()
                    } else {
                        None
                    }
                });
            attempts.entry(a.task_id).or_default().push(AttemptView {
                attempt_id: a.attempt_id,
                harness: a.harness.clone(),
                model_ref: a.model_ref.clone(),
                started_at: a.started_at.clone(),
                ended_at: a.ended_at.clone(),
                duration_s,
                outcome,
                base_revision,
                evidence: evidence
                    .iter()
                    .filter(|e| {
                        e.candidate().attempt_id == a.attempt_id
                            && e.candidate().task_id == a.task_id
                    })
                    .map(EvidenceView::from_evidence)
                    .collect(),
            });
            attempt_locations.insert(a.attempt_id.0, a.location.clone());
            attempt_revisions.insert(a.attempt_id.0, a.revision_id.clone());
        }
        let mut locations = HashMap::new();
        let nodes = b
            .graph
            .nodes
            .iter()
            .map(|n| {
                let state = tasks.get(&n.id).copied().unwrap_or(TaskState::Pending);
                let attempt = rows.iter().rev().find(|a| a.task_id == n.id);
                if let Some(a) = attempt {
                    locations.insert(n.id, a.location.clone());
                }
                let mut blocked_by: Vec<_> = n
                    .depends_on
                    .iter()
                    .copied()
                    .filter(|id| {
                        !matches!(
                            tasks.get(id),
                            Some(TaskState::Accepted | TaskState::Integrated)
                        )
                    })
                    .collect();
                blocked_by.sort();
                blocked_by.dedup();
                let terminal = matches!(
                    state,
                    TaskState::Accepted | TaskState::Integrated | TaskState::Cancelled
                );
                let waiting_for = if plan_state == PlanState::AwaitingPlanReview && !terminal {
                    Some(WaitingFor::PlanAcceptance)
                } else if state == TaskState::AwaitingReview {
                    Some(WaitingFor::Acceptance)
                } else if matches!(
                    state,
                    TaskState::Pending | TaskState::Ready | TaskState::Blocked
                ) && !blocked_by.is_empty()
                {
                    Some(WaitingFor::Dependency)
                } else if state == TaskState::Failed
                    && attempts.get(&n.id).map_or(0, Vec::len) as u32 >= self.limits.max_retries
                {
                    Some(WaitingFor::RetryBudgetExhausted)
                } else {
                    None
                };
                if waiting_for != Some(WaitingFor::Dependency) {
                    blocked_by.clear();
                }
                NodeStatus {
                    task_id: n.id,
                    waiting_for,
                    blocked_by,
                    description: n.description.clone(),
                    state,
                    note: None,
                    worktree: None,
                    revision: None,
                    pane_id: attempt.and_then(|a| a.pane_id.clone()),
                    // Completion timestamps do not prove OS process/tree liveness.
                    live: attempt.map(|_| "Ambiguous".into()),
                    deliverable: n.deliverable.clone(),
                }
            })
            .collect();
        Ok(ReadFrame {
            status: RunStatus {
                graph_id: selection.graph_id.clone(),
                plan_state,
                nodes,
            },
            summary: RunSummary {
                attempts: rows.len() as u32,
                retries: attempts
                    .values()
                    .map(|a| a.len().saturating_sub(1) as u32)
                    .sum(),
                worker_wall_seconds: wall,
            },
            attempts,
            locations,
            attempt_locations,
            attempt_revisions,
            generation_id: Some(b.scope.generation_id.clone()),
            revision_id: b.binding.current_revision.clone(),
        })
    }
}
