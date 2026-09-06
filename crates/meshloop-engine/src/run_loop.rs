//! R1 saga: CLI loops `tick` until `Idle`. Every state change goes through
//! `state::transition` and the event log. One QACR candidate = one attempt.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use meshloop_domain::capability::HarnessError;
use meshloop_domain::evidence::{
    AttemptId, CandidateRef, DeterministicEvidence, Evidence, HumanAcceptanceEvidence,
};
use meshloop_domain::policy::CouplingPenalty;
use meshloop_domain::state::{
    Event, PlanDecision, PlanState, TaskState, plan_transition, transition,
};
use meshloop_domain::task_graph::{TaskGraph, TaskId, Tier};

use crate::agent::{build_agent_spec, build_planning_spec, dependency_context};
use crate::planner::{DefaultTierAssigner, PlanError, assign_tiers, decompose};
use crate::ports::{
    AttemptRow, CheckRunner, FeedbackKey, HarnessCapabilities, LiveCheck, ProcessHint, ProcessView,
    RunRow, RunStore, StoreError, TransitionRecord, WorkspaceError, WorkspacePort,
};
use crate::recovery::replay_tasks;
use crate::router::{Candidate, Router, RoutingContext};
use crate::verify::{all_paths_allowed, git_diff_exit_code, verification_passed};

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
        if self.max_concurrent_workers != 1 {
            self.max_concurrent_workers = 1;
        }
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

fn digest(json: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    json.hash(&mut h);
    format!("{:016x}", h.finish())
}

fn is_fixture(name: &str) -> bool {
    name == "fixture"
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
            plan_sha256: digest(&json),
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
                    existing.plan_sha256 = digest(&json);
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
                plan_sha256: digest(&json),
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
            return Ok(Tick::Idle {
                reason: IdleReason::FailedTerminal,
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
            Err(HarnessError::Unsupported) => return Err(OrchestratorError::Unsupported),
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
                return self.maybe_fallback(graph_id, task_id, fail, &selected);
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
                match harness.collect(&handle) {
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
                        return self.maybe_fallback(graph_id, task_id, fail, &selected);
                    }
                    Ok(_) => {
                        let rec = self.append(
                            graph_id,
                            task_id,
                            Some(attempt_id),
                            TaskState::Running,
                            Event::HarnessExited,
                            None,
                        )?;
                        let _ = rec;
                    }
                }
            }
        }

        let verify = self.verify_attempt(
            graph_id, graph, node, attempt_id, &wt, &start, tier, &candidate,
        )?;
        Ok(verify)
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
        let revision = self.workspace.commit_all(
            wt,
            &format!("meshloop: attempt {} for task {}", attempt_id.0, node.id.0),
        )?;
        let diff = self.workspace.diff_against(wt, base)?;
        let git_code = git_diff_exit_code(&diff, node.empty_diff_ok);
        let candidate_ref = CandidateRef {
            task_id: node.id,
            attempt_id,
            revision: revision.clone(),
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
        if !self.verify_command.is_empty() {
            match self
                .checks
                .run(wt, &self.verify_command, self.limits.task_timeout)
            {
                Ok(mut ev) => {
                    ev.candidate = candidate_ref.clone();
                    rows.push(Evidence::Deterministic(ev));
                }
                Err(e) => {
                    rows.push(Evidence::Deterministic(DeterministicEvidence {
                        candidate: candidate_ref.clone(),
                        tool: self.verify_command[0].clone(),
                        tool_version: "n/a".into(),
                        exit_code: 1,
                        output_redacted: format!("{e:?}"),
                    }));
                }
            }
        }
        for row in &rows {
            self.store.record(row.clone())?;
        }
        let key = FeedbackKey {
            harness: candidate.harness.clone(),
            model_ref: candidate.model_ref.clone(),
            tier: crate::ports::TierKey::from(tier),
        };
        if verification_passed(&rows) {
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
                Tick::Idle { reason } => return Ok(reason),
                Tick::Transition { .. } => continue,
            }
        }
    }

    pub fn resume(&mut self, graph_id: &str, retry: bool) -> Result<IdleReason, OrchestratorError> {
        self.active_graph = Some(graph_id.into());
        self.crash_fail(graph_id)?;
        if retry {
            self.retry_failed(graph_id)?;
        }
        self.loop_until_idle()
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
            let note = if state == TaskState::Ready {
                Some("Ready (may be waiting on QACR or cancel to unblock dependents)".into())
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
        IdleReason::NoCapableCandidate | IdleReason::FailedTerminal => 1,
    }
}
