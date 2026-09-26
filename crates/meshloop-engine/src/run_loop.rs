//! R1 saga: CLI loops `tick` until `Idle`. Every state change goes through
//! `state::transition` and the event log. One QACR candidate = one attempt.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use meshloop_domain::capability::HarnessError;
use meshloop_domain::diagnostic::DiagnosticLattice;
use meshloop_domain::digest::compute_plan_id;
use meshloop_domain::evidence::{
    AttemptId, CandidateRef, DeterministicEvidence, Evidence, HumanAcceptanceEvidence,
};
use meshloop_domain::policy::CouplingPenalty;
use meshloop_domain::state::{
    Event, PlanDecision, PlanState, TaskState, plan_transition, transition,
};
use meshloop_domain::task_graph::{GraphMutation, TaskGraph, TaskId, Tier};

use crate::agent::{build_agent_spec, build_planning_spec, dependency_context};
use crate::converge::{RepairAction, RepairBudget, RepairSession};
use crate::planner::{DefaultTierAssigner, PlanError, assign_tiers, decompose};
use crate::ports::{
    AttemptRow, CheckRunner, FeedbackKey, HarnessCapabilities, HarnessHandle, LiveCheck,
    ProcessHint, ProcessView, RunRow, RunStore, StoreError, TransitionRecord, WorkspaceError,
    WorkspacePort,
};
use crate::recovery::replay_tasks;
use crate::router::{Candidate, Router, RoutingContext};
use crate::verify::{
    all_paths_allowed, annotate_with_lattice, git_diff_exit_code, verification_passed,
};

#[derive(Debug)]
pub enum OrchestratorError {
    UnknownHarness(String),
    Unsupported,
    LiveHarnessRefused(String),
    Store(StoreError),
    Workspace(WorkspaceError),
    Plan(PlanError),
    Illegal(String),
    DuplicateGraph { graph_id: String, resume: bool },
    MissingGraph(String),
    PlanDeclined(String),
    PlanAlreadyAccepted(String),
    NotAwaitingReview(TaskId),
    AlreadyAccepted(TaskId),
    EmptyIdentity,
    SnapshotMismatch,
    NoCandidate,
}

impl From<StoreError> for OrchestratorError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

impl From<WorkspaceError> for OrchestratorError {
    fn from(e: WorkspaceError) -> Self {
        Self::Workspace(e)
    }
}

#[derive(Debug, Clone)]
pub struct RunLimits {
    pub max_retries: u32,
    pub task_timeout: Duration,
    pub max_concurrent_workers: u32,
}

impl RunLimits {
    pub fn clamped(mut self) -> Self {
        self.max_concurrent_workers = self.max_concurrent_workers.clamp(1, 16);
        if self.max_retries == 0 {
            self.max_retries = 1;
        }
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleReason {
    GraphComplete,
    AwaitingHumanAcceptance,
    NoCapableCandidate,
    FailedTerminal,
    WaitingOnLiveWorker,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Tick {
    Transition {
        task: TaskId,
        attempt: Option<AttemptId>,
        from: TaskState,
        to: TaskState,
        event: Event,
    },
    Idle {
        reason: IdleReason,
    },
}

pub struct RunLoop<'a> {
    pub harnesses: HashMap<String, &'a dyn HarnessCapabilities>,
    pub candidates: Vec<Candidate>,
    pub workspace: &'a dyn WorkspacePort,
    pub store: &'a mut dyn RunStore,
    pub processes: &'a dyn ProcessView,
    pub checks: &'a dyn CheckRunner,
    pub router: Router,
    pub limits: RunLimits,
    /// When true, refuse non-`fixture` harnesses (CI double). Default is live Herdr.
    pub fixture_only: bool,
    pub verify_command: Vec<String>,
    pub worktree_base: PathBuf,
    pub active_graph: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NodeStatus {
    pub task_id: TaskId,
    pub description: String,
    pub state: TaskState,
    pub note: Option<String>,
    pub worktree: Option<PathBuf>,
    pub revision: Option<String>,
    pub pane_id: Option<String>,
    pub live: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RunStatus {
    pub graph_id: String,
    pub plan_state: PlanState,
    pub nodes: Vec<NodeStatus>,
}

fn stamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "0".into())
}

fn is_fixture(name: &str) -> bool {
    name == "fixture"
}

struct RepairTick {
    rows: Vec<Evidence>,
    lattice: Option<DiagnosticLattice>,
    diag: String,
    revision: String,
}

fn terminal(state: TaskState) -> bool {
    matches!(
        state,
        TaskState::Integrated | TaskState::Failed | TaskState::Cancelled | TaskState::Blocked
    )
}

impl<'a> RunLoop<'a> {
    fn graph_dir(&self, graph_id: &str) -> PathBuf {
        self.worktree_base.join(graph_id)
    }

    fn integrate_path(&self, graph_id: &str) -> PathBuf {
        self.graph_dir(graph_id).join("integrate")
    }

    fn plan_path(&self, graph_id: &str) -> PathBuf {
        self.graph_dir(graph_id).join("plan")
    }

    fn attempt_path(&self, graph_id: &str, task: TaskId, attempt: AttemptId) -> PathBuf {
        self.graph_dir(graph_id)
            .join(format!("task-{}-attempt-{}", task.0, attempt.0))
    }

    fn integrate_branch(&self, graph_id: &str) -> String {
        format!("meshloop/{graph_id}/integrate")
    }

    fn attempt_branch(&self, graph_id: &str, task: TaskId, attempt: AttemptId) -> String {
        format!("meshloop/{graph_id}/task-{}/attempt-{}", task.0, attempt.0)
    }

    fn plan_branch(&self, graph_id: &str) -> String {
        format!("meshloop/{graph_id}/plan")
    }

    fn require_dispatch_ok(&self, harness: &str) -> Result<(), OrchestratorError> {
        if self.fixture_only && !is_fixture(harness) {
            return Err(OrchestratorError::LiveHarnessRefused(harness.into()));
        }
        Ok(())
    }

    fn append(
        &mut self,
        graph_id: &str,
        task: TaskId,
        attempt: Option<AttemptId>,
        from: TaskState,
        event: Event,
        reason: Option<String>,
    ) -> Result<TransitionRecord, OrchestratorError> {
        let to = transition(from, event)
            .map_err(|_| OrchestratorError::Illegal(format!("{from:?} + {event:?} is illegal")))?;
        let rec = TransitionRecord {
            graph_id: graph_id.into(),
            task_id: task,
            attempt_id: attempt,
            from,
            to,
            event,
            reason,
            executor: "meshloop".into(),
            occurred_at: stamp(),
        };
        self.store.append(rec.clone())?;
        Ok(rec)
    }

    fn tasks(&self, graph_id: &str) -> Result<HashMap<TaskId, TaskState>, OrchestratorError> {
        let recs = self.store.records_for_graph(graph_id)?;
        replay_tasks(&recs).map_err(Into::into)
    }

    fn graph(&self, graph_id: &str) -> Result<TaskGraph, OrchestratorError> {
        self.store.graph_from_run(graph_id).map_err(Into::into)
    }

    pub fn plan(&mut self, objective: &str, scope: &str) -> Result<TaskGraph, OrchestratorError> {
        let scratch = "planning";
        let profiles = self.probe_all();
        let quotas = self.load_quotas();
        let feedback_store = &*self.store;
        let ctx = RoutingContext {
            task_tier: Tier::Tier3,
            coupling_penalty: CouplingPenalty(0),
            preferred_harness: None,
            headroom: &HashMap::new(),
            feedback: feedback_store,
        };
        let selected = self.router.select(
            &self.candidates,
            &profiles,
            &quotas,
            SystemTime::now(),
            &ctx,
        );
        let top = selected.first().ok_or(OrchestratorError::NoCandidate)?;
        self.require_dispatch_ok(&top.harness)?;
        let harness = *self
            .harnesses
            .get(&top.harness)
            .ok_or_else(|| OrchestratorError::UnknownHarness(top.harness.clone()))?;

        let wt = self.plan_path(scratch);
        let branch = self.plan_branch(scratch);
        let head = self.workspace.current_head()?;
        if self.workspace.worktree_exists(&wt) {
            self.workspace.reset_hard(&wt, &head)?;
        } else if self.workspace.branch_exists(&branch)? {
            return Err(OrchestratorError::Illegal(
                "planning branch exists without a worktree; remove meshloop-worktrees/.../plan and retry"
                    .into(),
            ));
        } else {
            let _ = std::fs::create_dir_all(self.graph_dir(scratch));
            self.workspace.add_worktree_from(&wt, &branch, &head)?;
        }

        let spec = build_planning_spec(
            objective,
            scope,
            AttemptId(0),
            &top.harness,
            &top.model_ref,
            wt.clone(),
            self.limits.task_timeout,
        );
        let graph = decompose(harness, &spec).map_err(OrchestratorError::Plan)?;
        let mut graph = graph;
        assign_tiers(&mut graph, &DefaultTierAssigner);
        Ok(graph)
    }

    pub fn start(
        &mut self,
        mut graph: TaskGraph,
        run_base: String,
    ) -> Result<(), OrchestratorError> {
        graph
            .validate()
            .map_err(|e| OrchestratorError::Illegal(format!("{e:?}")))?;
        assign_tiers(&mut graph, &DefaultTierAssigner);
        let graph_id = graph.graph_id.clone();
        if let Some(existing) = self.store.load_run(&graph_id)? {
            match existing.plan_state {
                PlanState::PlanDeclined => {
                    return Err(OrchestratorError::PlanDeclined(graph_id));
                }
                PlanState::PlanAccepted | PlanState::AwaitingPlanReview => {
                    let tasks = self.tasks(&graph_id).unwrap_or_default();
                    if tasks.is_empty() {
                        self.ensure_integrate(&graph_id, &existing.run_base)?;
                        self.active_graph = Some(graph_id);
                        return Ok(());
                    }
                    let live = graph
                        .nodes
                        .iter()
                        .any(|n| tasks.get(&n.id).map(|s| !terminal(*s)).unwrap_or(true));
                    return Err(OrchestratorError::DuplicateGraph {
                        graph_id,
                        resume: live,
                    });
                }
            }
        }
        let integrate = self.integrate_path(&graph_id);
        let ibranch = self.integrate_branch(&graph_id);
        if self.workspace.worktree_exists(&integrate) || self.workspace.branch_exists(&ibranch)? {
            return Err(OrchestratorError::Illegal(
                "integrate worktree/branch exists without a runs row".into(),
            ));
        }
        let _ = std::fs::create_dir_all(self.graph_dir(&graph_id));
        self.workspace
            .add_worktree_from(&integrate, &ibranch, &run_base)?;
        let json = serde_json::to_string_pretty(&graph)
            .map_err(|e| OrchestratorError::Illegal(e.to_string()))?;
        let row = RunRow {
            graph_id: graph_id.clone(),
            plan_state: PlanState::AwaitingPlanReview,
            run_base,
            integrate_ref: ibranch,
            plan_json: json.clone(),
            plan_id: compute_plan_id(&json).to_string(),
            created_at: stamp(),
            review_note: None,
        };
        self.store.save_run(&row)?;
        let mesh = self.workspace.repo_root().join(".meshloop");
        let _ = std::fs::create_dir_all(&mesh);
        let _ = std::fs::write(mesh.join("plan.json"), json);
        self.active_graph = Some(graph_id);
        Ok(())
    }

    pub fn accept_plan(&mut self, graph_id: &str) -> Result<(), OrchestratorError> {
        self.decide_plan(graph_id, PlanDecision::Accept, None)
            .map(|_| ())
    }

    fn ensure_integrate(&self, graph_id: &str, run_base: &str) -> Result<(), OrchestratorError> {
        let integrate = self.integrate_path(graph_id);
        let ibranch = self.integrate_branch(graph_id);
        if self.workspace.worktree_exists(&integrate) {
            return Ok(());
        }
        if self.workspace.branch_exists(&ibranch)? {
            return Err(OrchestratorError::Illegal(
                "integrate branch exists without a worktree; remove leftover meshloop worktrees and retry"
                    .into(),
            ));
        }
        let _ = std::fs::create_dir_all(self.graph_dir(graph_id));
        self.workspace
            .add_worktree_from(&integrate, &ibranch, run_base)?;
        Ok(())
    }

    /// Persist a graph as awaiting review without starting workers.
    pub fn stage_plan(
        &mut self,
        mut graph: TaskGraph,
        run_base: String,
    ) -> Result<String, OrchestratorError> {
        graph
            .validate()
            .map_err(|e| OrchestratorError::Illegal(format!("{e:?}")))?;
        assign_tiers(&mut graph, &DefaultTierAssigner);
        let graph_id = graph.graph_id.clone();
        let json = serde_json::to_string_pretty(&graph)
            .map_err(|e| OrchestratorError::Illegal(e.to_string()))?;
        let ibranch = self.integrate_branch(&graph_id);
        if let Some(mut existing) = self.store.load_run(&graph_id)? {
            match existing.plan_state {
                PlanState::PlanAccepted => {
                    return Err(OrchestratorError::PlanAlreadyAccepted(graph_id));
                }
                PlanState::AwaitingPlanReview | PlanState::PlanDeclined => {
                    existing.plan_json = json.clone();
                    existing.plan_id = compute_plan_id(&json).to_string();
                    existing.plan_state = PlanState::AwaitingPlanReview;
                    existing.run_base = run_base;
                    self.store.save_run(&existing)?;
                }
            }
        } else {
            self.store.save_run(&RunRow {
                graph_id: graph_id.clone(),
                plan_state: PlanState::AwaitingPlanReview,
                run_base,
                integrate_ref: ibranch,
                plan_json: json.clone(),
                plan_id: compute_plan_id(&json).to_string(),
                created_at: stamp(),
                review_note: None,
            })?;
        }
        let mesh = self.workspace.repo_root().join(".meshloop");
        let _ = std::fs::create_dir_all(&mesh);
        let _ = std::fs::write(mesh.join("plan.json"), json);
        self.active_graph = Some(graph_id.clone());
        Ok(graph_id)
    }

    pub fn decide_plan(
        &mut self,
        graph_id: &str,
        decision: PlanDecision,
        note: Option<String>,
    ) -> Result<PlanState, OrchestratorError> {
        let mut row = self
            .store
            .load_run(graph_id)?
            .ok_or_else(|| OrchestratorError::MissingGraph(graph_id.into()))?;
        let next = plan_transition(row.plan_state, decision).map_err(|_| {
            OrchestratorError::Illegal(format!(
                "illegal plan decision {decision:?} from {:?}",
                row.plan_state
            ))
        })?;
        row.plan_state = next;
        if note.is_some() {
            row.review_note = note;
        }
        self.store.save_run(&row)?;
        self.active_graph = Some(graph_id.into());
        Ok(next)
    }

    /// Dynamically mutates an in-flight plan graph (ADR 0028).
    /// Enforces state immutability, petgraph acyclicity, and updates persistence.
    /// If require_review is true, pauses the run by transitioning to AwaitingPlanReview.
    pub fn mutate_plan(
        &mut self,
        graph_id: &str,
        mutation: GraphMutation,
        require_review: bool,
    ) -> Result<TaskGraph, OrchestratorError> {
        let mut row = self
            .store
            .load_run(graph_id)?
            .ok_or_else(|| OrchestratorError::MissingGraph(graph_id.into()))?;

        let mut graph = self.store.graph_from_run(graph_id)?;
        let tasks = self.tasks(graph_id)?;

        // Validate state immutability per ADR 0028
        match &mutation {
            GraphMutation::InsertPrerequisite { target_task, .. } => {
                if let Some(st) = tasks.get(target_task)
                    && matches!(
                        st,
                        TaskState::Running
                            | TaskState::Verifying
                            | TaskState::AwaitingReview
                            | TaskState::Accepted
                            | TaskState::Integrated
                    )
                {
                    return Err(OrchestratorError::Illegal(format!(
                        "cannot insert prerequisite for task {} in active state {:?}",
                        target_task.0, st
                    )));
                }
            }
            GraphMutation::AppendFollowup { source_task, .. } => {
                if !graph.nodes.iter().any(|n| n.id == *source_task) {
                    return Err(OrchestratorError::Illegal(format!(
                        "source task {} not found in graph",
                        source_task.0
                    )));
                }
            }
        }

        // Apply domain mutation (validates petgraph DAG acyclicity atomically)
        graph
            .apply_mutation(&mutation)
            .map_err(|e| OrchestratorError::Illegal(format!("mutation rejected: {e:?}")))?;

        // Assign tiers to any newly added nodes
        assign_tiers(&mut graph, &DefaultTierAssigner);

        // Revert target_task from Ready to Pending if prerequisites were added
        let mut mutation_events = Vec::new();
        if let GraphMutation::InsertPrerequisite { target_task, .. } = &mutation
            && let Some(TaskState::Ready) = tasks.get(target_task)
        {
            let to = transition(TaskState::Ready, Event::GraphMutated).map_err(|_| {
                OrchestratorError::Illegal("Ready + GraphMutated is illegal".into())
            })?;
            mutation_events.push(TransitionRecord {
                graph_id: graph_id.into(),
                task_id: *target_task,
                attempt_id: None,
                from: TaskState::Ready,
                to,
                event: Event::GraphMutated,
                reason: Some("prerequisite inserted; reverting to pending".into()),
                executor: "meshloop".into(),
                occurred_at: stamp(),
            });
        }

        // Persist updated graph JSON
        let json = serde_json::to_string_pretty(&graph)
            .map_err(|e| OrchestratorError::Illegal(e.to_string()))?;
        row.plan_json = json.clone();
        row.plan_id = compute_plan_id(&json).to_string();

        if require_review {
            row.plan_state = PlanState::AwaitingPlanReview;
        }

        self.store.save_run_and_events(&row, &mutation_events)?;

        let mesh = self.workspace.repo_root().join(".meshloop");
        let _ = std::fs::create_dir_all(&mesh);
        let _ = std::fs::write(mesh.join("plan.json"), &json);

        Ok(graph)
    }

    pub fn tick(&mut self) -> Result<Tick, OrchestratorError> {
        let graph_id = self
            .active_graph
            .clone()
            .ok_or_else(|| OrchestratorError::MissingGraph("no active graph".into()))?;
        let row = self
            .store
            .load_run(&graph_id)?
            .ok_or_else(|| OrchestratorError::MissingGraph(graph_id.clone()))?;
        if row.plan_state != PlanState::PlanAccepted {
            return Err(OrchestratorError::Illegal(
                "plan has not been accepted".into(),
            ));
        }
        let graph = self.graph(&graph_id)?;
        let tasks = self.tasks(&graph_id)?;

        if let Some(tick) = self.reattach_running(&graph_id, &graph, &tasks)? {
            return Ok(tick);
        }
        if let Some(tick) = self.harvest_live_failures(&graph_id, &graph, &tasks)? {
            return Ok(tick);
        }
        if let Some(tick) = self.clear_blocked(&graph_id, &graph, &tasks)? {
            return Ok(tick);
        }

        if let Some(tick) = self.promote_pending(&graph_id, &graph, &tasks)? {
            return Ok(tick);
        }

        let any_awaiting = graph
            .nodes
            .iter()
            .any(|n| tasks.get(&n.id) == Some(&TaskState::AwaitingReview));
        let any_ready = graph
            .nodes
            .iter()
            .any(|n| tasks.get(&n.id) == Some(&TaskState::Ready));
        let any_accepted = graph
            .nodes
            .iter()
            .any(|n| tasks.get(&n.id) == Some(&TaskState::Accepted));
        let any_running = graph
            .nodes
            .iter()
            .any(|n| tasks.get(&n.id) == Some(&TaskState::Running));

        if any_awaiting && !any_ready && !any_accepted && !any_running {
            return Ok(Tick::Idle {
                reason: IdleReason::AwaitingHumanAcceptance,
            });
        }

        if let Some(tick) = self.merge_accepted(&graph_id, &graph, &tasks)? {
            return Ok(tick);
        }

        let integrated: HashSet<TaskId> = graph
            .nodes
            .iter()
            .filter(|n| tasks.get(&n.id) == Some(&TaskState::Integrated))
            .map(|n| n.id)
            .collect();
        let ready: Vec<TaskId> = graph
            .ready_nodes(&integrated)
            .into_iter()
            .filter(|n| tasks.get(&n.id) == Some(&TaskState::Ready))
            .map(|n| n.id)
            .collect();

        if ready.is_empty() {
            if graph
                .nodes
                .iter()
                .all(|n| tasks.get(&n.id) == Some(&TaskState::Integrated))
            {
                return Ok(Tick::Idle {
                    reason: IdleReason::GraphComplete,
                });
            }
            if any_running {
                return Ok(Tick::Idle {
                    reason: IdleReason::WaitingOnLiveWorker,
                });
            }
            return Ok(Tick::Idle {
                reason: IdleReason::FailedTerminal,
            });
        }

        let running_count = graph
            .nodes
            .iter()
            .filter(|n| tasks.get(&n.id) == Some(&TaskState::Running))
            .count();
        if running_count >= self.limits.max_concurrent_workers as usize {
            return Ok(Tick::Idle {
                reason: IdleReason::WaitingOnLiveWorker,
            });
        }

        for id in ready {
            match self.dispatch_ready(&graph_id, &graph, id)? {
                Tick::Idle {
                    reason: IdleReason::NoCapableCandidate,
                } => continue,
                other => return Ok(other),
            }
        }
        Ok(Tick::Idle {
            reason: IdleReason::NoCapableCandidate,
        })
    }

    fn resolve_pane(&self, attempt: &AttemptRow) -> Option<String> {
        if let Some(id) = &attempt.pane_id {
            return Some(id.clone());
        }
        let wt = attempt.worktree_path.as_ref()?;
        let name = attempt.harness.as_deref()?;
        self.harnesses.get(name)?.pane_for_worktree(wt)
    }

    fn attempt_handle(attempt: &AttemptRow, pane_id: Option<String>) -> HarnessHandle {
        HarnessHandle {
            attempt_id: attempt.attempt_id,
            pid: attempt.pid,
            pane_id,
        }
    }

    fn attempt_live(&self, attempt: &AttemptRow) -> LiveCheck {
        let Some(name) = attempt.harness.as_deref() else {
            return LiveCheck::Dead;
        };
        let Some(h) = self.harnesses.get(name) else {
            return LiveCheck::Ambiguous;
        };
        h.session_live(&Self::attempt_handle(attempt, self.resolve_pane(attempt)))
    }

    fn reattach_running(
        &mut self,
        graph_id: &str,
        graph: &TaskGraph,
        tasks: &HashMap<TaskId, TaskState>,
    ) -> Result<Option<Tick>, OrchestratorError> {
        for node in &graph.nodes {
            if tasks.get(&node.id) != Some(&TaskState::Running) {
                continue;
            }
            let Some(attempt) = self.store.latest_attempt_for_task(graph_id, node.id)? else {
                continue;
            };
            match self.attempt_live(&attempt) {
                LiveCheck::Dead => {
                    let rec = self.append(
                        graph_id,
                        node.id,
                        Some(attempt.attempt_id),
                        TaskState::Running,
                        Event::HarnessCrashedOrTimeout,
                        Some("reattach: pane/process gone".into()),
                    )?;
                    return Ok(Some(Tick::Transition {
                        task: node.id,
                        attempt: Some(attempt.attempt_id),
                        from: rec.from,
                        to: rec.to,
                        event: rec.event,
                    }));
                }
                LiveCheck::Live | LiveCheck::Ambiguous => {
                    let Some(name) = attempt.harness.clone() else {
                        continue;
                    };
                    let Some(h) = self.harnesses.get(&name).copied() else {
                        continue;
                    };
                    let handle = Self::attempt_handle(&attempt, self.resolve_pane(&attempt));
                    match h.try_collect(&handle) {
                        Ok(Some(_)) => {
                            let rec = self.append(
                                graph_id,
                                node.id,
                                Some(attempt.attempt_id),
                                TaskState::Running,
                                Event::HarnessExited,
                                Some("reattach: live pane settled".into()),
                            )?;
                            let _ = rec;
                            let tier = node.tier.unwrap_or(Tier::Tier2);
                            let candidate = Candidate {
                                harness: name,
                                model_ref: attempt.model_ref.clone().unwrap_or_default(),
                                model_tier: meshloop_domain::policy::ModelCapabilityTier::TopTier,
                            };
                            let wt = attempt.worktree_path.clone().unwrap_or_default();
                            let start = attempt
                                .outcome
                                .as_deref()
                                .and_then(|s| s.strip_prefix("base:"))
                                .unwrap_or("")
                                .to_string();
                            let tick = self.verify_attempt(
                                graph_id,
                                graph,
                                node,
                                attempt.attempt_id,
                                &wt,
                                &start,
                                tier,
                                &candidate,
                            )?;
                            return Ok(Some(tick));
                        }
                        Ok(None) => {
                            continue;
                        }
                        Err(e) => {
                            if matches!(self.attempt_live(&attempt), LiveCheck::Live) {
                                continue;
                            }
                            let rec = self.append(
                                graph_id,
                                node.id,
                                Some(attempt.attempt_id),
                                TaskState::Running,
                                Event::HarnessCrashedOrTimeout,
                                Some(format!("reattach: {e:?}")),
                            )?;
                            return Ok(Some(Tick::Transition {
                                task: node.id,
                                attempt: Some(attempt.attempt_id),
                                from: rec.from,
                                to: rec.to,
                                event: rec.event,
                            }));
                        }
                    }
                }
            }
        }
        Ok(None)
    }

    fn harvest_live_failures(
        &mut self,
        graph_id: &str,
        graph: &TaskGraph,
        tasks: &HashMap<TaskId, TaskState>,
    ) -> Result<Option<Tick>, OrchestratorError> {
        for node in &graph.nodes {
            if tasks.get(&node.id) != Some(&TaskState::Failed) {
                continue;
            }
            let Some(attempt) = self.store.latest_attempt_for_task(graph_id, node.id)? else {
                continue;
            };
            if self.resolve_pane(&attempt).is_none() {
                continue;
            }
            if self.attempt_live(&attempt) == LiveCheck::Dead {
                continue;
            }
            // Harvest is only for a premature crash/stall. A completed collect that
            // already failed git-diff must not be re-harvested while the idle pane
            // still looks Live (that looped thousands of times on awesome-landscape-v1).
            let recs = self.store.records_for_graph(graph_id)?;
            let last = recs.iter().rev().find(|r| r.task_id == node.id);
            if matches!(
                last.map(|r| r.event),
                Some(Event::DeterministicChecksFailed | Event::LiveWorkerSettled)
            ) {
                continue;
            }
            if last.map(|r| r.event) != Some(Event::HarnessCrashedOrTimeout) {
                continue;
            }
            let Some(name) = attempt.harness.clone() else {
                continue;
            };
            let Some(h) = self.harnesses.get(&name).copied() else {
                continue;
            };
            let handle = Self::attempt_handle(&attempt, self.resolve_pane(&attempt));
            if !matches!(h.try_collect(&handle), Ok(Some(_))) {
                continue;
            }
            let rec = self.append(
                graph_id,
                node.id,
                Some(attempt.attempt_id),
                TaskState::Failed,
                Event::LiveWorkerSettled,
                Some("harvest: live pane settled after premature fail".into()),
            )?;
            let _ = rec;
            let tier = node.tier.unwrap_or(Tier::Tier2);
            let candidate = Candidate {
                harness: name,
                model_ref: attempt.model_ref.clone().unwrap_or_default(),
                model_tier: meshloop_domain::policy::ModelCapabilityTier::TopTier,
            };
            let wt = attempt.worktree_path.clone().unwrap_or_default();
            let start = attempt
                .outcome
                .as_deref()
                .and_then(|s| s.strip_prefix("base:"))
                .unwrap_or("")
                .to_string();
            let tick = self.verify_attempt(
                graph_id,
                graph,
                node,
                attempt.attempt_id,
                &wt,
                &start,
                tier,
                &candidate,
            )?;
            return Ok(Some(tick));
        }
        Ok(None)
    }

    fn clear_blocked(
        &mut self,
        graph_id: &str,
        graph: &TaskGraph,
        tasks: &HashMap<TaskId, TaskState>,
    ) -> Result<Option<Tick>, OrchestratorError> {
        for node in &graph.nodes {
            if tasks.get(&node.id) != Some(&TaskState::Blocked) {
                continue;
            }
            let unsat = node.depends_on.iter().any(|d| {
                matches!(
                    tasks.get(d),
                    Some(TaskState::Failed | TaskState::Cancelled | TaskState::Blocked)
                )
            });
            if unsat {
                continue;
            }
            let rec = self.append(
                graph_id,
                node.id,
                None,
                TaskState::Blocked,
                Event::DependencyCleared,
                Some("upstream no longer failed".into()),
            )?;
            return Ok(Some(Tick::Transition {
                task: node.id,
                attempt: None,
                from: rec.from,
                to: rec.to,
                event: rec.event,
            }));
        }
        Ok(None)
    }

    fn promote_pending(
        &mut self,
        graph_id: &str,
        graph: &TaskGraph,
        tasks: &HashMap<TaskId, TaskState>,
    ) -> Result<Option<Tick>, OrchestratorError> {
        let integrated: HashSet<TaskId> = graph
            .nodes
            .iter()
            .filter(|n| tasks.get(&n.id) == Some(&TaskState::Integrated))
            .map(|n| n.id)
            .collect();
        for node in &graph.nodes {
            let state = tasks.get(&node.id).copied().unwrap_or(TaskState::Pending);
            if state != TaskState::Pending {
                continue;
            }
            let unsat = node.depends_on.iter().any(|d| {
                matches!(
                    tasks.get(d),
                    Some(TaskState::Failed | TaskState::Cancelled | TaskState::Blocked)
                )
            });
            if unsat {
                let rec = self.append(
                    graph_id,
                    node.id,
                    None,
                    TaskState::Pending,
                    Event::DependencyFailedOrScopeRevoked,
                    None,
                )?;
                return Ok(Some(Tick::Transition {
                    task: node.id,
                    attempt: None,
                    from: rec.from,
                    to: rec.to,
                    event: rec.event,
                }));
            }
            if node.depends_on.iter().all(|d| integrated.contains(d)) {
                let rec = self.append(
                    graph_id,
                    node.id,
                    None,
                    TaskState::Pending,
                    Event::DependencySatisfied,
                    None,
                )?;
                return Ok(Some(Tick::Transition {
                    task: node.id,
                    attempt: None,
                    from: rec.from,
                    to: rec.to,
                    event: rec.event,
                }));
            }
        }
        Ok(None)
    }

    fn merge_accepted(
        &mut self,
        graph_id: &str,
        graph: &TaskGraph,
        tasks: &HashMap<TaskId, TaskState>,
    ) -> Result<Option<Tick>, OrchestratorError> {
        let Some(node) = graph
            .nodes
            .iter()
            .find(|n| tasks.get(&n.id) == Some(&TaskState::Accepted))
        else {
            return Ok(None);
        };
        let attempt = self
            .store
            .latest_attempt_for_task(graph_id, node.id)?
            .ok_or_else(|| OrchestratorError::Illegal("accepted without attempt".into()))?;
        let from_ref = self.attempt_branch(graph_id, node.id, attempt.attempt_id);
        let integrate = self.integrate_path(graph_id);
        match self.workspace.merge_in_worktree(&integrate, &from_ref) {
            Ok(()) => {
                let rec = self.append(
                    graph_id,
                    node.id,
                    Some(attempt.attempt_id),
                    TaskState::Accepted,
                    Event::IntegrationOwnerMerge,
                    None,
                )?;
                Ok(Some(Tick::Transition {
                    task: node.id,
                    attempt: Some(attempt.attempt_id),
                    from: rec.from,
                    to: rec.to,
                    event: rec.event,
                }))
            }
            Err(_) => {
                let rec = self.append(
                    graph_id,
                    node.id,
                    Some(attempt.attempt_id),
                    TaskState::Accepted,
                    Event::StaleBaseDetected,
                    Some("merge conflict or dirty integrate worktree".into()),
                )?;
                Ok(Some(Tick::Transition {
                    task: node.id,
                    attempt: Some(attempt.attempt_id),
                    from: rec.from,
                    to: rec.to,
                    event: rec.event,
                }))
            }
        }
    }

    fn dispatch_ready(
        &mut self,
        graph_id: &str,
        graph: &TaskGraph,
        task_id: TaskId,
    ) -> Result<Tick, OrchestratorError> {
        let node = graph
            .nodes
            .iter()
            .find(|n| n.id == task_id)
            .ok_or_else(|| OrchestratorError::Illegal("missing node".into()))?;
        let tier = node.tier.unwrap_or(Tier::Tier2);
        let profiles = self.probe_all();
        let quotas = self.load_quotas();
        let preferred = self.last_success_harness(graph_id);
        let penalty = CouplingPenalty(0);
        let selected: Vec<crate::router::Candidate> = {
            let ctx = RoutingContext {
                task_tier: tier,
                coupling_penalty: penalty,
                preferred_harness: preferred.as_deref(),
                headroom: &HashMap::new(),
                feedback: &*self.store,
            };
            self.router
                .select(
                    &self.candidates,
                    &profiles,
                    &quotas,
                    SystemTime::now(),
                    &ctx,
                )
                .into_iter()
                .cloned()
                .collect()
        };
        if selected.is_empty() {
            return Ok(Tick::Idle {
                reason: IdleReason::NoCapableCandidate,
            });
        }
        let used: HashSet<String> = self
            .store
            .attempts_for_task(graph_id, task_id)?
            .into_iter()
            .filter_map(|a| a.harness)
            .collect();
        let candidate: crate::router::Candidate = selected
            .iter()
            .find(|c| !used.contains(&c.harness))
            .cloned()
            .unwrap_or_else(|| selected[0].clone());
        self.require_dispatch_ok(&candidate.harness)?;

        let attempt_id = self.store.next_attempt_id()?;
        let wt = self.attempt_path(graph_id, task_id, attempt_id);
        let branch = self.attempt_branch(graph_id, task_id, attempt_id);
        if self.workspace.worktree_exists(&wt) || self.workspace.branch_exists(&branch)? {
            return Err(OrchestratorError::Illegal(format!(
                "leftover worktree or branch for attempt {}",
                attempt_id.0
            )));
        }
        let _started = self.append(
            graph_id,
            task_id,
            Some(attempt_id),
            TaskState::Ready,
            Event::AttemptStarted,
            None,
        )?;

        let start = {
            let integrate = self.integrate_path(graph_id);
            if self.workspace.worktree_exists(&integrate) {
                self.workspace.head(&integrate)?
            } else {
                self.store
                    .load_run(graph_id)?
                    .map(|r| r.run_base)
                    .unwrap_or_default()
            }
        };
        if let Err(e) = self.workspace.add_worktree_from(&wt, &branch, &start) {
            let fail = self.append(
                graph_id,
                task_id,
                Some(attempt_id),
                TaskState::Running,
                Event::HarnessCrashedOrTimeout,
                Some(format!("{e:?}")),
            )?;
            return self.maybe_fallback(graph_id, task_id, fail, &selected);
        }

        self.store.save_attempt(&AttemptRow {
            attempt_id,
            graph_id: graph_id.into(),
            task_id,
            harness: Some(candidate.harness.clone()),
            model_ref: Some(candidate.model_ref.clone()),
            worktree_path: Some(wt.clone()),
            pid: None,
            image_name: None,
            pane_id: None,
            started_at: Some(stamp()),
            ended_at: None,
            outcome: Some(format!("base:{start}")),
        })?;

        let spec = build_agent_spec(
            graph,
            node,
            attempt_id,
            &candidate.harness,
            &candidate.model_ref,
            wt.clone(),
            self.limits.task_timeout,
        );
        // situation slot already has descriptions; append note about integrate tree
        let _ = dependency_context(graph, node);
        let harness = *self
            .harnesses
            .get(&candidate.harness)
            .ok_or_else(|| OrchestratorError::UnknownHarness(candidate.harness.clone()))?;

        let invoke = harness.invoke(&spec);
        match invoke {
            Err(HarnessError::Unsupported) => Err(OrchestratorError::Unsupported),
            Err(e) => {
                self.record_quota(&candidate.harness, &e)?;
                let _ = self.store.record_outcome(
                    &FeedbackKey {
                        harness: candidate.harness.clone(),
                        model_ref: candidate.model_ref.clone(),
                        tier: crate::ports::TierKey::from(tier),
                    },
                    false,
                );
                let fail = self.append(
                    graph_id,
                    task_id,
                    Some(attempt_id),
                    TaskState::Running,
                    Event::HarnessCrashedOrTimeout,
                    Some(format!("{e:?}")),
                )?;
                self.maybe_fallback(graph_id, task_id, fail, &selected)
            }
            Ok(handle) => {
                let image = Path::new(
                    &self
                        .candidates
                        .iter()
                        .find(|c| c.harness == candidate.harness)
                        .map(|c| c.harness.clone())
                        .unwrap_or_default(),
                )
                .file_name()
                .map(|s| s.to_string_lossy().into_owned());
                self.store
                    .update_attempt_pid(attempt_id, handle.pid, image.as_deref())?;
                self.store
                    .update_attempt_pane(attempt_id, handle.pane_id.as_deref())?;
                match harness.try_collect(&handle) {
                    Err(e) => {
                        self.record_quota(&candidate.harness, &e)?;
                        let fail = self.append(
                            graph_id,
                            task_id,
                            Some(attempt_id),
                            TaskState::Running,
                            Event::HarnessCrashedOrTimeout,
                            Some(format!("{e:?}")),
                        )?;
                        self.maybe_fallback(graph_id, task_id, fail, &selected)
                    }
                    Ok(Some(_)) => {
                        let rec = self.append(
                            graph_id,
                            task_id,
                            Some(attempt_id),
                            TaskState::Running,
                            Event::HarnessExited,
                            None,
                        )?;
                        let _ = rec;
                        self.verify_attempt(
                            graph_id, graph, node, attempt_id, &wt, &start, tier, &candidate,
                        )
                    }
                    Ok(None) => Ok(Tick::Transition {
                        task: task_id,
                        attempt: Some(attempt_id),
                        from: TaskState::Ready,
                        to: TaskState::Running,
                        event: Event::AttemptStarted,
                    }),
                }
            }
        }
    }

    fn run_deterministic_checks(
        &self,
        node: &meshloop_domain::task_graph::TaskNode,
        attempt_id: AttemptId,
        wt: &Path,
        base: &str,
        revision: &str,
    ) -> Result<
        (
            Vec<Evidence>,
            Option<meshloop_domain::diagnostic::DiagnosticLattice>,
            String,
        ),
        OrchestratorError,
    > {
        let diff = self.workspace.diff_against(wt, base)?;
        let git_code = git_diff_exit_code(&diff, node.empty_diff_ok);
        let candidate_ref = CandidateRef {
            task_id: node.id,
            attempt_id,
            revision: revision.to_string(),
        };
        let mut rows = Vec::new();
        rows.push(Evidence::Deterministic(DeterministicEvidence {
            candidate: candidate_ref.clone(),
            tool: "git-diff".into(),
            tool_version: "n/a".into(),
            exit_code: git_code,
            output_redacted: diff.stat_redacted.clone(),
        }));
        if !all_paths_allowed(&diff.files, &node.allowed_paths) {
            rows.push(Evidence::Deterministic(DeterministicEvidence {
                candidate: candidate_ref.clone(),
                tool: "allowed-paths".into(),
                tool_version: "n/a".into(),
                exit_code: 1,
                output_redacted: format!("changed {:?}", diff.files),
            }));
        }
        let mut lattice_out = None;
        let mut diag_out = String::new();
        if !self.verify_command.is_empty() {
            match self
                .checks
                .run(wt, &self.verify_command, self.limits.task_timeout)
            {
                Ok(mut ev) => {
                    ev.candidate = candidate_ref.clone();
                    let (lat, annotated) = annotate_with_lattice(&ev.output_redacted);
                    diag_out = ev.output_redacted.clone();
                    lattice_out = Some(lat);
                    ev.output_redacted = annotated;
                    rows.push(Evidence::Deterministic(ev));
                }
                Err(e) => {
                    diag_out = format!("{e:?}");
                    rows.push(Evidence::Deterministic(DeterministicEvidence {
                        candidate: candidate_ref.clone(),
                        tool: self.verify_command[0].clone(),
                        tool_version: "n/a".into(),
                        exit_code: 1,
                        output_redacted: diag_out.clone(),
                    }));
                }
            }
        }
        Ok((rows, lattice_out, diag_out))
    }

    #[allow(clippy::too_many_arguments)]
    fn dispatch_repair_round(
        &mut self,
        node: &meshloop_domain::task_graph::TaskNode,
        attempt_id: AttemptId,
        wt: &Path,
        base: &str,
        candidate: &Candidate,
        prompt: &str,
        commit_message: &str,
        diagnostics: &str,
        negative_constraint: &str,
    ) -> Result<Option<RepairTick>, OrchestratorError> {
        let repair_spec = crate::agent::build_repair_spec(
            node,
            attempt_id,
            &candidate.harness,
            &candidate.model_ref,
            wt.to_path_buf(),
            self.limits.task_timeout,
            diagnostics,
            negative_constraint,
        );
        let harness = match self.harnesses.get(&candidate.harness) {
            Some(h) => *h,
            None => return Ok(None),
        };
        let handle = match harness.invoke(&repair_spec) {
            Ok(h) => h,
            Err(_) => return Ok(None),
        };
        if harness.collect(&handle).is_err() {
            return Ok(None);
        }
        let _ = self.workspace.remove_file(wt, prompt);
        let revision = match self.workspace.commit_all(wt, commit_message) {
            Ok(rev) => rev,
            Err(_) => return Ok(None),
        };
        let (rows, lattice, diag) =
            match self.run_deterministic_checks(node, attempt_id, wt, base, &revision) {
                Ok(res) => res,
                Err(_) => return Ok(None),
            };
        for row in &rows {
            let _ = self.store.record(row.clone());
        }
        Ok(Some(RepairTick {
            rows,
            lattice,
            diag,
            revision,
        }))
    }

    fn persist_repair_session(
        &mut self,
        node: &meshloop_domain::task_graph::TaskNode,
        attempt_id: AttemptId,
        revision: &str,
        session: &RepairSession,
        passed: bool,
        elapsed_ms: f64,
    ) {
        if session.round_count() == 0 {
            return;
        }
        let last = session.last_action();
        let accepted = matches!(last, Some(RepairAction::Accept)) && passed;
        let stop = last
            .map(RepairAction::summary)
            .unwrap_or_else(|| "none".into());
        let output_redacted = format!(
            "stop={stop} passed={passed} elapsed_ms={elapsed_ms:.1}\n{}",
            session.trajectory_redacted()
        );
        let _ = self
            .store
            .record(Evidence::Deterministic(DeterministicEvidence {
                candidate: CandidateRef {
                    task_id: node.id,
                    attempt_id,
                    revision: revision.to_string(),
                },
                tool: "repair-session".into(),
                tool_version: "n/a".into(),
                exit_code: if accepted { 0 } else { 1 },
                output_redacted,
            }));
    }

    fn persist_rollback_fidelity(
        &mut self,
        node: &meshloop_domain::task_graph::TaskNode,
        attempt_id: AttemptId,
        wt: &Path,
        to_rev: &str,
    ) {
        let head = self.workspace.head(wt).unwrap_or_default();
        let porcelain = self.workspace.status_porcelain(wt).unwrap_or_default();
        let sha_match = !head.is_empty()
            && (head == to_rev || head.starts_with(to_rev) || to_rev.starts_with(&head));
        let porcelain_clean = porcelain.trim().is_empty();
        let fidelity_ok = sha_match && porcelain_clean;
        let _ = self
            .store
            .record(Evidence::Deterministic(DeterministicEvidence {
                candidate: CandidateRef {
                    task_id: node.id,
                    attempt_id,
                    revision: to_rev.to_string(),
                },
                tool: "repair-rollback".into(),
                tool_version: "n/a".into(),
                exit_code: if fidelity_ok { 0 } else { 1 },
                output_redacted: format!(
                    "to_rev={to_rev} head={head} sha_match={} porcelain_clean={} fidelity={}",
                    u8::from(sha_match),
                    u8::from(porcelain_clean),
                    u8::from(fidelity_ok)
                ),
            }));
    }

    #[allow(clippy::too_many_arguments)]
    fn verify_attempt(
        &mut self,
        graph_id: &str,
        _graph: &TaskGraph,
        node: &meshloop_domain::task_graph::TaskNode,
        attempt_id: AttemptId,
        wt: &Path,
        base: &str,
        tier: Tier,
        candidate: &Candidate,
    ) -> Result<Tick, OrchestratorError> {
        let prompt = format!(".meshloop-prompt-{}", attempt_id.0);
        let _ = self.workspace.remove_file(wt, &prompt);
        let mut current_revision = self.workspace.commit_all(
            wt,
            &format!("meshloop: attempt {} for task {}", attempt_id.0, node.id.0),
        )?;

        let (mut rows, current_lattice, mut current_diag) =
            self.run_deterministic_checks(node, attempt_id, wt, base, &current_revision)?;

        for row in &rows {
            self.store.record(row.clone())?;
        }

        let mut passed = verification_passed(&rows);

        // ADR 0026: observe after every check. Accept records the terminal Φ=0 round.
        // Rollback restores the prior round's diagnostics and must not re-observe the
        // reset tree (that fingerprint is already in history → false Oscillation).
        if !passed && let Some(initial_lat) = current_lattice {
            let mut session = RepairSession::new(RepairBudget::default());
            let mut lat = initial_lat;
            let repair_started = std::time::Instant::now();

            loop {
                let action =
                    session.observe(lat.clone(), current_revision.clone(), current_diag.clone());
                match action {
                    RepairAction::Accept => {
                        passed = verification_passed(&rows);
                        break;
                    }
                    RepairAction::Continue {
                        round,
                        negative_constraint,
                    } => {
                        let commit_msg =
                            format!("meshloop: repair round {} for task {}", round, node.id.0);
                        match self.dispatch_repair_round(
                            node,
                            attempt_id,
                            wt,
                            base,
                            candidate,
                            &prompt,
                            &commit_msg,
                            &current_diag,
                            &negative_constraint,
                        )? {
                            Some(tick) => {
                                rows = tick.rows;
                                current_diag = tick.diag;
                                current_revision = tick.revision;
                                match tick.lattice {
                                    Some(l) => lat = l,
                                    None => break,
                                }
                            }
                            None => break,
                        }
                    }
                    RepairAction::Rollback {
                        to_round,
                        to_rev,
                        negative_constraint,
                    } => {
                        if self.workspace.reset_hard(wt, &to_rev).is_err() {
                            break;
                        }
                        self.persist_rollback_fidelity(node, attempt_id, wt, &to_rev);
                        current_diag = session
                            .record_at(to_round)
                            .map(|r| r.diag.clone())
                            .unwrap_or(current_diag);
                        current_revision = to_rev;
                        let commit_msg =
                            format!("meshloop: repair rollback retry for task {}", node.id.0);
                        match self.dispatch_repair_round(
                            node,
                            attempt_id,
                            wt,
                            base,
                            candidate,
                            &prompt,
                            &commit_msg,
                            &current_diag,
                            &negative_constraint,
                        )? {
                            Some(tick) => {
                                rows = tick.rows;
                                current_diag = tick.diag;
                                current_revision = tick.revision;
                                match tick.lattice {
                                    Some(l) => lat = l,
                                    None => break,
                                }
                            }
                            None => break,
                        }
                    }
                    RepairAction::Stop { .. } => break,
                }
            }
            let elapsed_ms = repair_started.elapsed().as_secs_f64() * 1000.0;
            self.persist_repair_session(
                node,
                attempt_id,
                &current_revision,
                &session,
                passed,
                elapsed_ms,
            );
        }

        let key = FeedbackKey {
            harness: candidate.harness.clone(),
            model_ref: candidate.model_ref.clone(),
            tier: crate::ports::TierKey::from(tier),
        };
        if passed {
            let _ = self.store.record_outcome(&key, true);
            let rec = self.append(
                graph_id,
                node.id,
                Some(attempt_id),
                TaskState::Verifying,
                Event::DeterministicChecksPassed,
                None,
            )?;
            Ok(Tick::Transition {
                task: node.id,
                attempt: Some(attempt_id),
                from: rec.from,
                to: rec.to,
                event: rec.event,
            })
        } else {
            let _ = self.store.record_outcome(&key, false);
            let rec = self.append(
                graph_id,
                node.id,
                Some(attempt_id),
                TaskState::Verifying,
                Event::DeterministicChecksFailed,
                None,
            )?;
            self.maybe_fallback(graph_id, node.id, rec, &[])
        }
    }

    fn maybe_fallback(
        &mut self,
        graph_id: &str,
        task_id: TaskId,
        fail: TransitionRecord,
        selected: &[Candidate],
    ) -> Result<Tick, OrchestratorError> {
        let attempts = self.store.attempts_for_task(graph_id, task_id)?;
        let live_worker = attempts.iter().any(|a| {
            self.resolve_pane(a).is_some()
                && matches!(self.attempt_live(a), LiveCheck::Live | LiveCheck::Ambiguous)
        });
        if live_worker {
            return Ok(Tick::Transition {
                task: task_id,
                attempt: fail.attempt_id,
                from: fail.from,
                to: fail.to,
                event: fail.event,
            });
        }
        let retry_count = attempts.len() as u32;
        let used: HashSet<String> = attempts.into_iter().filter_map(|a| a.harness).collect();
        let unused = selected.iter().any(|c| !used.contains(&c.harness));
        if fail.to == TaskState::Failed && retry_count < self.limits.max_retries && unused {
            let rec = self.append(
                graph_id,
                task_id,
                fail.attempt_id,
                TaskState::Failed,
                Event::RetryAuthorized,
                None,
            )?;
            return Ok(Tick::Transition {
                task: task_id,
                attempt: rec.attempt_id,
                from: rec.from,
                to: rec.to,
                event: rec.event,
            });
        }
        Ok(Tick::Transition {
            task: task_id,
            attempt: fail.attempt_id,
            from: fail.from,
            to: fail.to,
            event: fail.event,
        })
    }

    pub fn loop_until_idle(&mut self) -> Result<IdleReason, OrchestratorError> {
        loop {
            match self.tick()? {
                Tick::Idle {
                    reason: IdleReason::WaitingOnLiveWorker,
                } => {
                    std::thread::sleep(Duration::from_millis(20));
                    continue;
                }
                Tick::Idle { reason } => return Ok(reason),
                Tick::Transition { .. } => continue,
            }
        }
    }

    pub fn resume(
        &mut self,
        graph_id: &str,
        retry: bool,
        restart: bool,
    ) -> Result<IdleReason, OrchestratorError> {
        if retry && restart {
            return Err(OrchestratorError::Illegal(
                "use either --retry or --restart, not both".into(),
            ));
        }
        self.active_graph = Some(graph_id.into());
        if restart {
            self.restart(graph_id)?;
        } else {
            self.crash_fail(graph_id)?;
            if retry {
                self.retry_failed(graph_id)?;
            }
        }
        self.loop_until_idle()
    }

    /// Keep the accepted plan; wipe this saga's attempts/events and leftover
    /// worktrees so the same graph_id can execute again (no replan).
    pub fn restart(&mut self, graph_id: &str) -> Result<(), OrchestratorError> {
        let row = self
            .store
            .load_run(graph_id)?
            .ok_or_else(|| OrchestratorError::MissingGraph(graph_id.into()))?;
        if row.plan_state != PlanState::PlanAccepted {
            return Err(OrchestratorError::Illegal(
                "resume --restart requires a PlanAccepted graph".into(),
            ));
        }
        let attempts = self.store.attempts_for_graph(graph_id)?;
        for attempt in &attempts {
            if let Some(name) = &attempt.harness
                && let Some(h) = self.harnesses.get(name)
            {
                let handle = Self::attempt_handle(attempt, self.resolve_pane(attempt));
                if matches!(
                    h.session_live(&handle),
                    LiveCheck::Live | LiveCheck::Ambiguous
                ) {
                    let _ = h.cancel(&handle);
                }
            }
            if let Some(path) = &attempt.worktree_path {
                let _ = self.workspace.remove_worktree(path);
            }
            let branch = self.attempt_branch(graph_id, attempt.task_id, attempt.attempt_id);
            let _ = self.workspace.delete_branch(&branch);
        }
        let integrate = self.integrate_path(graph_id);
        if self.workspace.worktree_exists(&integrate) {
            let _ = self.workspace.reset_hard(&integrate, &row.run_base);
        }
        self.store.reset_graph_execution(graph_id)?;
        self.active_graph = Some(graph_id.into());
        Ok(())
    }

    fn crash_fail(&mut self, graph_id: &str) -> Result<(), OrchestratorError> {
        let recs = self.store.records_for_graph(graph_id)?;
        let tasks = replay_tasks(&recs)?;
        let graph = self.graph(graph_id)?;
        for node in &graph.nodes {
            if tasks.get(&node.id) != Some(&TaskState::Running) {
                continue;
            }
            let attempt = self.store.latest_attempt_for_task(graph_id, node.id)?;
            let dead = match &attempt {
                Some(a) if a.pane_id.is_some() => {
                    matches!(self.attempt_live(a), LiveCheck::Dead)
                }
                Some(a) => match a.pid {
                    None => true,
                    Some(pid) => {
                        let hint = ProcessHint {
                            pid,
                            image_name: a.image_name.clone(),
                        };
                        !matches!(self.processes.is_live(&hint), LiveCheck::Live)
                    }
                },
                None => true,
            };
            if dead {
                let _ = self.append(
                    graph_id,
                    node.id,
                    attempt.map(|a| a.attempt_id),
                    TaskState::Running,
                    Event::HarnessCrashedOrTimeout,
                    Some("resume: no live process".into()),
                )?;
            }
        }
        Ok(())
    }

    fn retry_failed(&mut self, graph_id: &str) -> Result<(), OrchestratorError> {
        let graph = self.graph(graph_id)?;
        let tasks = self.tasks(graph_id)?;
        for node in &graph.nodes {
            if tasks.get(&node.id) != Some(&TaskState::Failed) {
                continue;
            }
            let count = self.store.attempts_for_task(graph_id, node.id)?.len() as u32;
            if count < self.limits.max_retries {
                let _ = self.append(
                    graph_id,
                    node.id,
                    None,
                    TaskState::Failed,
                    Event::RetryAuthorized,
                    Some("resume --retry".into()),
                )?;
            }
        }
        Ok(())
    }

    pub fn cancel(
        &mut self,
        graph_id: &str,
        task: Option<TaskId>,
    ) -> Result<(), OrchestratorError> {
        self.active_graph = Some(graph_id.into());
        let graph = self.graph(graph_id)?;
        let tasks = self.tasks(graph_id)?;
        let ids: Vec<TaskId> = match task {
            Some(id) => vec![id],
            None => graph.nodes.iter().map(|n| n.id).collect(),
        };
        for id in ids {
            let state = tasks.get(&id).copied().unwrap_or(TaskState::Pending);
            if matches!(state, TaskState::Cancelled | TaskState::Integrated) {
                continue;
            }
            if transition(state, Event::Cancel).is_err() {
                continue;
            }
            if state == TaskState::Running
                && let Some(a) = self.store.latest_attempt_for_task(graph_id, id)?
                && let Some(name) = &a.harness
                && let Some(h) = self.harnesses.get(name)
            {
                let live_pid = a.pid.map(|pid| ProcessHint {
                    pid,
                    image_name: a.image_name.clone(),
                });
                let should_cancel = a.pane_id.is_some()
                    || live_pid
                        .as_ref()
                        .is_some_and(|hint| self.processes.is_live(hint) == LiveCheck::Live);
                if should_cancel {
                    let _ = h.cancel(&crate::ports::HarnessHandle {
                        attempt_id: a.attempt_id,
                        pid: a.pid,
                        pane_id: a.pane_id.clone(),
                    });
                }
            }
            let _ = self.append(graph_id, id, None, state, Event::Cancel, None)?;
        }
        Ok(())
    }

    pub fn accept_human(&mut self, task: TaskId, who: &str) -> Result<(), OrchestratorError> {
        if who.trim().is_empty() {
            return Err(OrchestratorError::EmptyIdentity);
        }
        let graph_id = self
            .active_graph
            .clone()
            .ok_or_else(|| OrchestratorError::MissingGraph("no active graph".into()))?;
        let tasks = self.tasks(&graph_id)?;
        let state = tasks.get(&task).copied().unwrap_or(TaskState::Pending);
        if state == TaskState::Accepted || state == TaskState::Integrated {
            return Err(OrchestratorError::AlreadyAccepted(task));
        }
        if state != TaskState::AwaitingReview {
            return Err(OrchestratorError::NotAwaitingReview(task));
        }
        let attempt = self
            .store
            .latest_attempt_for_task(&graph_id, task)?
            .ok_or_else(|| OrchestratorError::Illegal("no attempt to accept".into()))?;
        let revision = attempt
            .worktree_path
            .as_ref()
            .and_then(|p| self.workspace.head(p).ok())
            .unwrap_or_else(|| "unknown".into());
        self.store
            .record(Evidence::HumanAcceptance(HumanAcceptanceEvidence {
                candidate: CandidateRef {
                    task_id: task,
                    attempt_id: attempt.attempt_id,
                    revision,
                },
                accepted_by: who.into(),
                accepted_at: stamp(),
            }))?;
        let _ = self.append(
            &graph_id,
            task,
            Some(attempt.attempt_id),
            TaskState::AwaitingReview,
            Event::HumanAcceptanceRecorded,
            Some(format!("accepted-by:{who}")),
        )?;
        Ok(())
    }

    pub fn integrate_into(
        &mut self,
        graph_id: &str,
        git_ref: &str,
    ) -> Result<(), OrchestratorError> {
        let dirty = self
            .workspace
            .status_porcelain(self.workspace.repo_root())?;
        if !dirty.trim().is_empty() {
            return Err(OrchestratorError::Workspace(WorkspaceError::Dirty));
        }
        let integrate = self.integrate_path(graph_id);
        let integrate_head = self.workspace.head(&integrate)?;
        self.workspace.checkout_ref(git_ref)?;
        if self.workspace.is_ancestor(git_ref, &integrate_head)? {
            self.workspace.merge_ff_only(&integrate_head)?;
        } else {
            self.workspace.merge_no_ff(&integrate_head)?;
        }
        Ok(())
    }

    pub fn status(&mut self, graph_id: &str) -> Result<RunStatus, OrchestratorError> {
        let row = self
            .store
            .load_run(graph_id)?
            .ok_or_else(|| OrchestratorError::MissingGraph(graph_id.into()))?;
        let graph = self.graph(graph_id)?;
        let tasks = self.tasks(graph_id)?;
        let mut nodes = Vec::new();
        for n in &graph.nodes {
            let state = tasks.get(&n.id).copied().unwrap_or(TaskState::Pending);
            let attempt = self.store.latest_attempt_for_task(graph_id, n.id)?;
            let live = attempt
                .as_ref()
                .map(|a| format!("{:?}", self.attempt_live(a)));
            let pane_id = attempt.as_ref().and_then(|a| a.pane_id.clone());
            let note = if state == TaskState::Ready {
                Some("Ready (may be waiting on QACR or cancel to unblock dependents)".into())
            } else if state == TaskState::Failed && live.as_deref() == Some("Live") {
                Some(
                    "ledger Failed; Herdr pane still live — meshloop resume waits and harvests"
                        .into(),
                )
            } else {
                None
            };
            nodes.push(NodeStatus {
                task_id: n.id,
                description: n.description.clone(),
                state,
                note,
                worktree: attempt.as_ref().and_then(|a| a.worktree_path.clone()),
                revision: None,
                pane_id,
                live,
            });
        }
        Ok(RunStatus {
            graph_id: graph_id.into(),
            plan_state: row.plan_state,
            nodes,
        })
    }

    pub fn resolve_graph_id(&self, requested: Option<&str>) -> Result<String, OrchestratorError> {
        if let Some(id) = requested {
            return Ok(id.into());
        }
        let runs = self.store.list_runs()?;
        let live: Vec<_> = runs
            .iter()
            .filter(|r| r.plan_state == PlanState::PlanAccepted)
            .collect();
        if let Some(row) = self.store.latest_run()? {
            return Ok(row.graph_id);
        }
        let _ = live;
        Err(OrchestratorError::MissingGraph("no runs".into()))
    }

    fn probe_all(&self) -> HashMap<String, meshloop_domain::capability::HarnessProfile> {
        let mut profiles = HashMap::new();
        for (name, h) in &self.harnesses {
            if let Ok(p) = h.probe() {
                profiles.insert(name.clone(), p);
            }
        }
        profiles
    }

    fn load_quotas(&self) -> HashMap<String, meshloop_domain::capability::QuotaState> {
        let mut q = HashMap::new();
        for name in self.harnesses.keys() {
            if let Ok(s) = self.store.load_quota(name) {
                q.insert(name.clone(), s);
            }
        }
        q
    }

    fn last_success_harness(&self, graph_id: &str) -> Option<String> {
        let recs = self.store.records_for_graph(graph_id).ok()?;
        recs.iter()
            .rev()
            .find(|r| r.event == Event::DeterministicChecksPassed)
            .and_then(|r| r.attempt_id)
            .and_then(|id| self.store.load_attempt(id).ok().flatten())
            .and_then(|a| a.harness)
    }

    fn record_quota(&mut self, harness: &str, err: &HarnessError) -> Result<(), OrchestratorError> {
        if let HarnessError::CapacityExhausted { retry_after } = err {
            let mut q = self.store.load_quota(harness)?;
            q.on_capacity_exhausted(SystemTime::now(), *retry_after);
            self.store.save_quota(harness, &q)?;
        }
        Ok(())
    }
}

pub fn idle_exit_code(reason: IdleReason, status: &RunStatus) -> i32 {
    match reason {
        IdleReason::GraphComplete => 0,
        IdleReason::AwaitingHumanAcceptance => {
            if status
                .nodes
                .iter()
                .any(|n| matches!(n.state, TaskState::Failed | TaskState::Blocked))
            {
                1
            } else {
                0
            }
        }
        IdleReason::WaitingOnLiveWorker => 0,
        IdleReason::NoCapableCandidate | IdleReason::FailedTerminal => 1,
    }
}
