//! Decomposition is one more agent dispatch, not a separate code path (ADR 0009 /
//! runtime-design.md §5). This module dispatches it, validates the result structurally,
//! and assigns tiers — it never judges decomposition *quality* (MECE-ness), which this
//! project cannot mechanically verify; that is the `awaiting-plan-review` human gate's job.

use std::path::PathBuf;

use meshloop_domain::capability::HarnessError;
use meshloop_domain::decision::{Confidence, DecisionOutcome, RiskFloor};
use meshloop_domain::evidence::AttemptId;
use meshloop_domain::task_graph::{GraphError, TaskGraph, TaskId, TaskNode, Tier};

use crate::agent::AgentSpec;
use crate::ports::{
    DecisionPort, DecisionPortError, HarnessCapabilities, HarnessOutcome, RunStore, ScoreQuery,
    StoreError, WorkspacePort, decision_fingerprint,
};
use crate::workspace::LifecycleAccess;
use meshloop_domain::workspace::*;

const PLAN_FILE: &str = "meshloop-plan.json";

/// Structural rejection; exact output stays in the protected owned file.
struct Rejected {
    error: PlanError,
}

impl From<PlanError> for Rejected {
    fn from(error: PlanError) -> Self {
        Self { error }
    }
}

fn parse_graph(spec: &AgentSpec, outcome: &HarnessOutcome) -> Result<TaskGraph, Rejected> {
    let file_path = spec.worktree_path.join(PLAN_FILE);
    let raw = if file_path.is_file() {
        std::fs::read_to_string(&file_path).map_err(|e| PlanError::Malformed(e.to_string()))?
    } else {
        outcome.output_redacted.clone()
    };
    let reject = |error: PlanError| Rejected { error };
    let graph: TaskGraph =
        serde_json::from_str(&raw).map_err(|e| reject(PlanError::Malformed(e.to_string())))?;
    graph
        .validate()
        .map_err(|e| reject(PlanError::Invalid(e)))?;
    Ok(graph)
}

#[derive(Debug)]
pub enum PlanError {
    Control(&'static str),
    Ownership(StoreError),
    Preservation,
    Dispatch(HarnessError),
    Malformed(String),
    Invalid(GraphError),
    /// Every allowed attempt produced a rejected plan. `saved_plan` identifies the
    /// protected output by root alias and relative path, never an absolute root.
    Exhausted {
        attempts: u32,
        last: Box<PlanError>,
        saved_plan: Option<PathBuf>,
    },
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanError::Control(code) => write!(f, "{code}"),
            PlanError::Ownership(_) => write!(f, "planner-ownership-failed"),
            PlanError::Preservation => write!(f, "planner-preservation-failed"),
            PlanError::Dispatch(_) => write!(f, "planner-dispatch-unresolved"),
            PlanError::Malformed(_) => write!(f, "planner-malformed-output"),
            PlanError::Invalid(_) => write!(f, "planner-invalid-graph"),
            PlanError::Exhausted {
                attempts,
                last,
                saved_plan,
            } => {
                write!(
                    f,
                    "planner produced a rejected plan on all {attempts} attempt(s); last error: {last}"
                )?;
                match saved_plan {
                    Some(p) => write!(
                        f,
                        "; last rejected plan retained at portable locator {}",
                        p.display()
                    ),
                    None => write!(f, "; the rejected plan could not be saved"),
                }
            }
        }
    }
}

impl std::error::Error for PlanError {}

/// Actual planner producer. Every retry gets a distinct owner, worktree and ref.
/// An operation with no step is deliberately unresolved after an ambiguous failure.
pub(crate) fn decompose_owned(
    harness: &dyn HarnessCapabilities,
    spec: &AgentSpec,
    max_retries: u32,
    workspace: &dyn WorkspacePort,
    store: &mut dyn RunStore,
    access: &mut dyn LifecycleAccess,
) -> Result<TaskGraph, PlanError> {
    let owned = |e| PlanError::Ownership(e);
    access.fence().map_err(owned)?.validate().map_err(owned)?;
    let base = workspace
        .current_head()
        .map_err(|_| PlanError::Preservation)?;
    let base = ObjectId::new(base).map_err(|_| PlanError::Preservation)?;
    let mut prompt = spec.prompt.clone();
    for n in 0..max_retries.max(1) {
        let id = access.new_id().map_err(owned)?;
        let owner = ProvisionalOwner {
            id: id.clone(),
            repository_id: access.fence().map_err(owned)?.repository().id.clone(),
            graph_id: None,
            kind: ProvisionalKind::Planning,
            location: PortableLocation {
                root_alias: RootAlias::new("planning-worktrees")
                    .map_err(|_| PlanError::Preservation)?,
                relative_path: RelativePath::new(format!("plan-{}", id.as_str()))
                    .map_err(|_| PlanError::Preservation)?,
            },
            provenance: access.new_id().map_err(owned)?,
        };
        store
            .create_provisional(access.fence().map_err(owned)?, &owner)
            .map_err(owned)?;
        let path = access.resolve(&owner.location).map_err(owned)?;
        let branch = format!("meshloop/planning/{}", id.as_str());
        let before = ExpectedState {
            source_artifact: None,
            source: None,
            base: Some(base.clone()),
            location: Some(owner.location.clone()),
            native_identity: None,
            head: None,
            reference: Some(
                RelativePath::new(branch.clone()).map_err(|_| PlanError::Preservation)?,
            ),
            registered: false,
            exists: false,
        };
        let mut after = before.clone();
        after.exists = true;
        after.registered = true;
        after.head = Some(base.clone());
        let worktree = planning_intent(
            store,
            access,
            &owner,
            ProducerKind::Worktree,
            before,
            after.clone(),
        )?;
        // Refuse collisions. Never reset, adopt, or delete an older attempt.
        match std::fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err(PlanError::Preservation),
        }
        if workspace
            .branch_exists(&branch)
            .map_err(|_| PlanError::Preservation)?
        {
            return Err(PlanError::Preservation);
        }
        workspace
            .add_worktree_from(&path, &branch, base.as_str())
            .map_err(|_| PlanError::Preservation)?;
        if workspace.head(&path).map_err(|_| PlanError::Preservation)? != base.as_str()
            || !workspace
                .branch_exists(&branch)
                .map_err(|_| PlanError::Preservation)?
        {
            return Err(PlanError::Preservation);
        }
        planning_step(store, access, &worktree, OperationState::Completed, None)?;
        let mut attempt_spec = spec.clone();
        attempt_spec.worktree_path = path.clone();
        attempt_spec.attempt_id = AttemptId(n);
        attempt_spec.prompt = prompt;
        let process_state = ExpectedState {
            source_artifact: None,
            source: None,
            base: None,
            location: Some(owner.location.clone()),
            native_identity: None,
            head: None,
            reference: None,
            registered: false,
            exists: true,
        };
        let spawn = planning_intent(
            store,
            access,
            &owner,
            ProducerKind::Spawn,
            process_state.clone(),
            process_state.clone(),
        )?;
        let mut output = process_state;
        output.location = Some(PortableLocation {
            root_alias: owner.location.root_alias.clone(),
            relative_path: RelativePath::new(format!(
                "{}/meshloop-plan.json",
                owner.location.relative_path.as_str()
            ))
            .map_err(|_| PlanError::Preservation)?,
        });
        // Pending claims are exclusive per target. Output ownership is distinct
        // from the process/worktree so both intents exist before invocation.
        let output_owner = ProvisionalOwner {
            id: access.new_id().map_err(owned)?,
            repository_id: owner.repository_id.clone(),
            graph_id: None,
            kind: ProvisionalKind::Planning,
            location: output.location.clone().ok_or(PlanError::Preservation)?,
            provenance: owner.id.clone(),
        };
        store
            .create_provisional(access.fence().map_err(owned)?, &output_owner)
            .map_err(owned)?;
        let mut absent = output.clone();
        absent.exists = false;
        let plan = planning_intent(
            store,
            access,
            &output_owner,
            ProducerKind::Plan,
            absent,
            output,
        )?;
        let output_path = access.resolve(&output_owner.location).map_err(owned)?;
        match std::fs::symlink_metadata(&output_path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err(PlanError::Preservation),
        }
        access.fence().map_err(owned)?.validate().map_err(owned)?;
        // Neither invoke nor collect failures prove the process tree is quiescent.
        // Leave both intents unresolved and do not salvage or retry uncertain effects.
        let outcome = collect_with_driver(harness, &attempt_spec, &owner, store, access)?;
        // Preserve stdout-only output locally before validating. A producer-written
        // file always wins and is never overwritten, including malformed output.
        let output_path = access.resolve(&output_owner.location).map_err(owned)?;
        match std::fs::symlink_metadata(&output_path) {
            Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&output_path)
                    .map_err(|_| PlanError::Preservation)?;
                file.write_all(outcome.output_redacted.as_bytes())
                    .map_err(|_| PlanError::Preservation)?;
                file.sync_all().map_err(|_| PlanError::Preservation)?;
            }
            _ => return Err(PlanError::Preservation),
        }
        planning_step(store, access, &spawn, OperationState::Completed, None)?;
        match parse_graph(&attempt_spec, &outcome) {
            Ok(graph) if outcome.exit_code == 0 => {
                planning_step(store, access, &plan, OperationState::Completed, None)?;
                return Ok(graph);
            }
            result => {
                let error = match result {
                    Err(rejected) => rejected.error,
                    Ok(_) => PlanError::Dispatch(HarnessError::ProcessFault {
                        detail: "planner exited unsuccessfully".into(),
                    }),
                };
                let code = if outcome.exit_code != 0 {
                    "planner-nonzero-exit"
                } else if matches!(error, PlanError::Malformed(_)) {
                    "planner-malformed-output"
                } else {
                    "planner-invalid-graph"
                };
                planning_step(store, access, &plan, OperationState::Blocked, Some(code))?;
                if outcome.exit_code != 0
                    || !matches!(error, PlanError::Malformed(_) | PlanError::Invalid(_))
                {
                    return Err(error);
                }
                if n + 1 == max_retries.max(1) {
                    return Err(PlanError::Exhausted {
                        attempts: n + 1,
                        last: Box::new(error),
                        saved_plan: Some(PathBuf::from(format!(
                            "{}/{}",
                            output_owner.location.root_alias.as_str(),
                            output_owner.location.relative_path.as_str()
                        ))),
                    });
                }
                prompt = format!(
                    "{}\n\nyour previous graph failed: {}\nFix exactly that problem and rewrite {PLAN_FILE}.",
                    spec.prompt, error
                );
            }
        }
    }
    unreachable!("at least one planning attempt")
}

/// A driver belongs to one attempt because its provisional target is immutable.
/// The repository fence remains borrowed throughout invocation, polling and ack.
fn collect_with_driver(
    harness: &dyn HarnessCapabilities,
    spec: &AgentSpec,
    owner: &ProvisionalOwner,
    store: &mut dyn RunStore,
    access: &mut dyn LifecycleAccess,
) -> Result<HarnessOutcome, PlanError> {
    use std::time::Duration;
    let nonce = access.new_id().map_err(PlanError::Ownership)?;
    let process = access
        .owner_process_identity(&nonce)
        .map_err(PlanError::Ownership)?;
    let controlled = process.is_some() && harness.supports_nonblocking_collection();
    let mut driver = DriverRecord {
        id: access.new_id().map_err(PlanError::Ownership)?,
        nonce,
        target: DriverTarget::Provisional {
            repository_id: owner.repository_id.clone(),
            owner_id: owner.id.clone(),
        },
        process,
        phase: if controlled {
            DriverPhase::Starting
        } else {
            DriverPhase::Blocked
        },
        heartbeat: 0,
        error_code: if controlled {
            None
        } else {
            Some(
                LifecycleId::new("driver-control-unavailable")
                    .map_err(|_| PlanError::Preservation)?,
            )
        },
        diagnostic: None,
    };
    store
        .register_driver(access.fence().map_err(PlanError::Ownership)?, &driver)
        .map_err(PlanError::Ownership)?;
    let mut retained_handle = None;
    let mut cancellation_attempted = false;
    let result = (|| {
        let handle = harness.invoke(spec).map_err(PlanError::Dispatch)?;
        retained_handle = Some(handle.clone());
        if !controlled {
            return harness.collect(&handle).map_err(PlanError::Dispatch);
        }
        driver.phase = DriverPhase::Running;
        store
            .update_driver(access.fence().map_err(PlanError::Ownership)?, &driver)
            .map_err(PlanError::Ownership)?;
        let mut blocked = false;
        loop {
            access
                .fence()
                .map_err(PlanError::Ownership)?
                .validate()
                .map_err(PlanError::Ownership)?;
            let requests = store
                .cancellation_requests(&driver.id)
                .map_err(PlanError::Ownership)?;
            for observation in requests
                .into_iter()
                .take(64)
                .filter(|r| r.status == CancellationStatus::Pending)
            {
                let request = observation.request;
                let exact = request.process
                    == driver
                        .process
                        .clone()
                        .ok_or(PlanError::Control("driver-identity-unavailable"))?
                    && request.target
                        == CancellationTarget::Provisional {
                            repository_id: owner.repository_id.clone(),
                            owner_id: owner.id.clone(),
                            driver_id: driver.id.clone(),
                        }
                    && access
                        .owner_process_identity(&driver.nonce)
                        .map_err(PlanError::Ownership)?
                        == driver.process;
                let status = if !exact {
                    CancellationStatus::Stale
                } else {
                    cancellation_attempted = true;
                    if harness
                        .cancel_owned(&handle, Duration::from_secs(2))
                        .is_ok()
                    {
                        CancellationStatus::Applied
                    } else {
                        CancellationStatus::Blocked
                    }
                };
                // Only this successful fenced commit makes a proof an Applied ack.
                store
                    .acknowledge_cancellation(
                        access.fence().map_err(PlanError::Ownership)?,
                        &request.id,
                        status,
                    )
                    .map_err(|_| PlanError::Control("planner-cancellation-ack-failed"))?;
                if status == CancellationStatus::Applied {
                    return Err(PlanError::Control("planner-cancelled"));
                }
                if status == CancellationStatus::Blocked {
                    // Keep the retained owner alive until normal collection or
                    // its existing timeout. In particular, a POSIX Child drop
                    // does not terminate its process group. Never retry this
                    // attempt even if its eventual output is valid.
                    blocked = true;
                    driver.error_code = Some(
                        LifecycleId::new("planner-cancellation-blocked")
                            .map_err(|_| PlanError::Preservation)?,
                    );
                }
            }
            if let Some(outcome) = harness.try_collect(&handle).map_err(PlanError::Dispatch)? {
                return if blocked {
                    Err(PlanError::Control("planner-cancellation-blocked"))
                } else {
                    Ok(outcome)
                };
            }
            driver.heartbeat = driver.heartbeat.saturating_add(1);
            store
                .update_driver(access.fence().map_err(PlanError::Ownership)?, &driver)
                .map_err(PlanError::Ownership)?;
            std::thread::sleep(Duration::from_millis(100));
        }
    })();
    if result.is_err()
        && !cancellation_attempted
        && !matches!(result, Err(PlanError::Control("planner-cancelled")))
        && let Some(handle) = retained_handle.as_ref()
    {
        // Fence/persistence/identity errors can occur after launch too. Only the
        // exact retained-owner port may perform best-effort unwind cleanup;
        // its result is not an acknowledgement, and unresolved intent stays.
        let _cleanup = harness.cancel_owned(handle, Duration::from_secs(2));
    }
    driver.phase = match &result {
        Ok(_) | Err(PlanError::Control("planner-cancelled")) => DriverPhase::Exited,
        Err(_) => DriverPhase::Blocked,
    };
    if let Err(error) = &result {
        driver.error_code = Some(
            LifecycleId::new(match error {
                PlanError::Control(code) => *code,
                _ => "planner-driver-unresolved",
            })
            .map_err(|_| PlanError::Preservation)?,
        );
    }
    // A failed terminal write is also uncertain; never continue validation/retry.
    store
        .update_driver(access.fence().map_err(PlanError::Ownership)?, &driver)
        .map_err(|_| PlanError::Control("planner-driver-update-failed"))?;
    if result.is_ok() {
        // Enqueue and the terminal phase update serialize in SQLite. Requests
        // committed before that transition are drained after it; later enqueue
        // is refused. Normal completion is never retroactively called Applied.
        for observation in store
            .cancellation_requests(&driver.id)
            .map_err(PlanError::Ownership)?
        {
            store
                .acknowledge_cancellation(
                    access.fence().map_err(PlanError::Ownership)?,
                    &observation.request.id,
                    CancellationStatus::Stale,
                )
                .map_err(|_| PlanError::Control("planner-cancellation-ack-failed"))?;
        }
    }
    result
}

fn planning_intent(
    store: &mut dyn RunStore,
    access: &mut dyn LifecycleAccess,
    owner: &ProvisionalOwner,
    kind: ProducerKind,
    before: ExpectedState,
    after: ExpectedState,
) -> Result<ProducerOperation, PlanError> {
    let operation = ProducerOperation {
        id: access.new_id().map_err(PlanError::Ownership)?,
        repository_id: owner.repository_id.clone(),
        graph_id: None,
        generation_id: None,
        target: ProducerTarget::Provisional(owner.id.clone()),
        kind,
        before,
        after,
        max_steps: 1,
    };
    store
        .append_operation(access.fence().map_err(PlanError::Ownership)?, &operation)
        .map_err(PlanError::Ownership)?;
    Ok(operation)
}

fn planning_step(
    store: &mut dyn RunStore,
    access: &mut dyn LifecycleAccess,
    operation: &ProducerOperation,
    state: OperationState,
    error_code: Option<&str>,
) -> Result<(), PlanError> {
    let step = OperationStep {
        id: access.new_id().map_err(PlanError::Ownership)?,
        operation_id: operation.id.clone(),
        step: 1,
        observed: operation.after.clone(),
        state,
        error_code: error_code
            .map(LifecycleId::new)
            .transpose()
            .map_err(|_| PlanError::Preservation)?,
    };
    store
        .append_operation_step(access.fence().map_err(PlanError::Ownership)?, &step)
        .map_err(PlanError::Ownership)
}

/// Dispatches the decomposition agent and returns a structurally-validated graph, or a
/// `PlanError` the caller should turn into a bounded retry (a new attempt, ADR 0005) — a
/// cyclic or schema-invalid graph must never be partially scheduled.
pub fn decompose(
    harness: &dyn HarnessCapabilities,
    spec: &AgentSpec,
) -> Result<TaskGraph, PlanError> {
    decompose_inner(harness, spec).map_err(|r| r.error)
}

fn decompose_inner(
    harness: &dyn HarnessCapabilities,
    spec: &AgentSpec,
) -> Result<TaskGraph, Rejected> {
    match harness.invoke(spec) {
        Ok(handle) => {
            let outcome: HarnessOutcome = harness.collect(&handle).map_err(PlanError::Dispatch)?;
            parse_graph(spec, &outcome)
        }
        Err(e) => {
            let file_path = spec.worktree_path.join("meshloop-plan.json");
            if file_path.is_file() {
                let outcome = HarnessOutcome {
                    exit_code: 0,
                    output_redacted: String::new(),
                    stderr_redacted: String::new(),
                    worktree_changed: true,
                };
                if let Ok(graph) = parse_graph(spec, &outcome) {
                    return Ok(graph);
                }
            }
            Err(PlanError::Dispatch(e).into())
        }
    }
}

pub trait TierAssigner {
    fn assign_tier(&self, graph: &TaskGraph, node_id: meshloop_domain::task_graph::TaskId) -> Tier;
}

/// A placeholder heuristic (leaf nodes with no dependents = Tier1, nodes with dependents =
/// Tier2, nodes with 2+ dependencies = Tier3) — real tier heuristics need measurement
/// against actual task outcomes before being trusted; this exists so the mechanism is
/// exercised, not as a claim of a tuned policy.
pub struct DefaultTierAssigner;

impl TierAssigner for DefaultTierAssigner {
    fn assign_tier(&self, graph: &TaskGraph, node_id: meshloop_domain::task_graph::TaskId) -> Tier {
        let node = graph.nodes.iter().find(|n| n.id == node_id);
        let dep_count = node.map(|n| n.depends_on.len()).unwrap_or(0);
        let has_dependents = graph.nodes.iter().any(|n| n.depends_on.contains(&node_id));
        if dep_count >= 2 {
            Tier::Tier3
        } else if has_dependents {
            Tier::Tier2
        } else {
            Tier::Tier1
        }
    }
}

/// Raise-only risk ratchet. The base tier is the declared tier, or
/// [`DefaultTierAssigner`] when the node omits one. A stored unit-interval
/// `q >= 0.90` lifts that base to [`Tier::Tier3`] and never lowers it.
/// Does not call a port; [`apply_risk_ratchet`] loads and stores the score.
pub struct JevTierAssigner;

const RISK_TIER3_AT: f64 = 0.90;

fn risk_subject(description: &str) -> String {
    format!("risk\n{description}")
}

fn risk_prompt_hash(subject: &str) -> String {
    decision_fingerprint("risk", &[subject])
}

fn stored_risk_q(node: &TaskNode, prompt_hash: &str) -> Option<f64> {
    if node.risk_prompt_hash.as_deref() != Some(prompt_hash) {
        return None;
    }
    let q = node.risk_q?;
    Confidence::try_new(q).ok().map(Confidence::get)
}

fn base_tier(graph: &TaskGraph, node_id: TaskId, declared: Option<Tier>) -> Tier {
    match declared {
        Some(tier) => tier,
        None => DefaultTierAssigner.assign_tier(graph, node_id),
    }
}

fn raised_tier(base: Tier, q: Option<f64>) -> Tier {
    match q {
        Some(q) if Confidence::try_new(q).is_ok() && q >= RISK_TIER3_AT => {
            RiskFloor::new(Tier::Tier3).apply(base)
        }
        _ => base,
    }
}

enum RiskSample {
    Reused(f64),
    Accepted(f64),
    Ignored,
}

impl TierAssigner for JevTierAssigner {
    fn assign_tier(&self, graph: &TaskGraph, node_id: TaskId) -> Tier {
        let Some(node) = graph.nodes.iter().find(|node| node.id == node_id) else {
            return base_tier(graph, node_id, None);
        };
        let subject = risk_subject(&node.description);
        let prompt_hash = risk_prompt_hash(&subject);
        let q = stored_risk_q(node, &prompt_hash);
        let base = base_tier(graph, node_id, node.tier);
        raised_tier(base, q)
    }
}

pub fn assign_tiers(graph: &mut TaskGraph, assigner: &dyn TierAssigner) {
    let ids: Vec<_> = graph.nodes.iter().map(|n| n.id).collect();
    for id in ids {
        let tier = assigner.assign_tier(graph, id);
        if let Some(node) = graph.nodes.iter_mut().find(|n| n.id == id)
            && node.tier.is_none()
        {
            node.tier = Some(tier);
        }
    }
}

/// Score each node once, unless it already stores this description's sample.
/// `q >= 0.90` in `0.0..=1.0` raises the tier to Tier3. Abstain, a non-score
/// outcome, or a score outside that interval does not raise and does not store
/// a sample. Returns the first [`DecisionPortError`] and does not panic.
pub fn apply_risk_ratchet(
    graph: &mut TaskGraph,
    port: &dyn DecisionPort,
) -> Result<(), DecisionPortError> {
    let ids: Vec<_> = graph.nodes.iter().map(|node| node.id).collect();
    for id in ids {
        let Some((subject, prompt_hash, declared, reused)) =
            graph.nodes.iter().find(|node| node.id == id).map(|node| {
                let subject = risk_subject(&node.description);
                let prompt_hash = risk_prompt_hash(&subject);
                let reused = stored_risk_q(node, &prompt_hash);
                (subject, prompt_hash, node.tier, reused)
            })
        else {
            continue;
        };
        let sample = if let Some(q) = reused {
            RiskSample::Reused(q)
        } else {
            match port.evaluate_score(&ScoreQuery { subject })? {
                DecisionOutcome::Score(score) => match Confidence::try_new(score.get()) {
                    Ok(confidence) => RiskSample::Accepted(confidence.get()),
                    Err(_) => RiskSample::Ignored,
                },
                DecisionOutcome::Boolean(_)
                | DecisionOutcome::Selected(_)
                | DecisionOutcome::Abstain(_) => RiskSample::Ignored,
            }
        };
        let q = match sample {
            RiskSample::Reused(q) | RiskSample::Accepted(q) => Some(q),
            RiskSample::Ignored => None,
        };
        let tier = raised_tier(base_tier(graph, id, declared), q);
        if let Some(node) = graph.nodes.iter_mut().find(|node| node.id == id) {
            if let RiskSample::Accepted(q) = sample {
                node.risk_q = Some(q);
                node.risk_prompt_hash = Some(prompt_hash);
            }
            node.tier = Some(tier);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::{
        BatchNoulQuery, ChoiceQuery, DecisionPort, HarnessHandle, NoulQuery, ScriptedDecisionPort,
    };
    use meshloop_domain::decision::DecisionOutcome;
    use meshloop_domain::evidence::AttemptId;
    use meshloop_domain::task_graph::{AcceptanceGate, TaskId, TaskNode};
    use std::cell::Cell;
    use std::path::PathBuf;
    use std::time::Duration;

    struct FakeHarness {
        response: Result<String, HarnessError>,
    }

    impl HarnessCapabilities for FakeHarness {
        fn probe(&self) -> Result<meshloop_domain::capability::HarnessProfile, HarnessError> {
            unimplemented!()
        }
        fn invoke(&self, _spec: &AgentSpec) -> Result<HarnessHandle, HarnessError> {
            Ok(HarnessHandle {
                attempt_id: AttemptId(1),
                pid: None,
                pane_id: None,
            })
        }
        fn cancel(&self, _handle: &HarnessHandle) -> Result<(), HarnessError> {
            Ok(())
        }
        fn collect(&self, _handle: &HarnessHandle) -> Result<HarnessOutcome, HarnessError> {
            match &self.response {
                Ok(json) => Ok(HarnessOutcome {
                    exit_code: 0,
                    output_redacted: json.clone(),
                    stderr_redacted: String::new(),
                    worktree_changed: false,
                }),
                Err(e) => Err(e.clone()),
            }
        }
    }

    fn spec() -> AgentSpec {
        crate::agent::build_planning_spec(
            "objective",
            "scope",
            AttemptId(1),
            "claude-code",
            "m",
            PathBuf::from("/tmp"),
            Duration::from_secs(60),
        )
    }

    #[test]
    fn valid_json_graph_is_accepted() {
        let harness = FakeHarness {
            response: Ok(r#"{"graph_id":"g","nodes":[{"id":1,"description":"d","depends_on":[],"tier":null}]}"#.into()),
        };
        let graph = decompose(&harness, &spec()).expect("should decompose");
        assert_eq!(graph.nodes.len(), 1);
    }

    #[test]
    fn cyclic_graph_is_rejected_not_partially_scheduled() {
        let harness = FakeHarness {
            response: Ok(r#"{"graph_id":"g","nodes":[{"id":1,"description":"d","depends_on":[1],"tier":null}]}"#.into()),
        };
        let result = decompose(&harness, &spec());
        assert!(matches!(
            result,
            Err(PlanError::Invalid(GraphError::Cycle(_)))
        ));
    }

    #[test]
    fn malformed_json_is_rejected() {
        let harness = FakeHarness {
            response: Ok("not json".into()),
        };
        assert!(matches!(
            decompose(&harness, &spec()),
            Err(PlanError::Malformed(_))
        ));
    }

    #[test]
    fn salvage_plan_file_when_invoke_fails() {
        let dir = std::env::temp_dir().join(format!("meshloop-salvage-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let json =
            r#"{"graph_id":"g","nodes":[{"id":1,"description":"d","depends_on":[],"tier":null}]}"#;
        std::fs::write(dir.join("meshloop-plan.json"), json).unwrap();
        struct InvokeFail;
        impl HarnessCapabilities for InvokeFail {
            fn probe(&self) -> Result<meshloop_domain::capability::HarnessProfile, HarnessError> {
                unimplemented!()
            }
            fn invoke(&self, _spec: &AgentSpec) -> Result<HarnessHandle, HarnessError> {
                Err(HarnessError::Timeout)
            }
            fn cancel(&self, _handle: &HarnessHandle) -> Result<(), HarnessError> {
                Ok(())
            }
            fn collect(&self, _handle: &HarnessHandle) -> Result<HarnessOutcome, HarnessError> {
                unimplemented!()
            }
        }
        let spec = crate::agent::build_planning_spec(
            "objective",
            "scope",
            AttemptId(1),
            "claude-code",
            "m",
            dir.clone(),
            Duration::from_secs(60),
        );
        let graph = decompose(&InvokeFail, &spec).expect("should salvage worktree plan");
        assert_eq!(graph.nodes.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn capacity_exhausted_dispatch_surfaces_as_dispatch_error_for_qacr_fallback() {
        let harness = FakeHarness {
            response: Err(HarnessError::CapacityExhausted {
                retry_after: Duration::from_secs(60),
            }),
        };
        assert!(matches!(
            decompose(&harness, &spec()),
            Err(PlanError::Dispatch(_))
        ));
    }

    #[test]
    fn tier_assignment_gives_multi_dependency_nodes_tier3() {
        let mut graph = TaskGraph {
            graph_id: "g".into(),
            nodes: vec![
                meshloop_domain::task_graph::TaskNode {
                    id: TaskId(1),
                    description: "a".into(),
                    depends_on: vec![],
                    tier: None,
                    allowed_paths: vec![],
                    empty_diff_ok: false,
                    deliverable: None,
                    gate: AcceptanceGate::Human,
                    risk_q: None,
                    risk_prompt_hash: None,
                },
                meshloop_domain::task_graph::TaskNode {
                    id: TaskId(2),
                    description: "b".into(),
                    depends_on: vec![],
                    tier: None,
                    allowed_paths: vec![],
                    empty_diff_ok: false,
                    deliverable: None,
                    gate: AcceptanceGate::Human,
                    risk_q: None,
                    risk_prompt_hash: None,
                },
                meshloop_domain::task_graph::TaskNode {
                    id: TaskId(3),
                    description: "c".into(),
                    depends_on: vec![TaskId(1), TaskId(2)],
                    tier: None,
                    allowed_paths: vec![],
                    empty_diff_ok: false,
                    deliverable: None,
                    gate: AcceptanceGate::Human,
                    risk_q: None,
                    risk_prompt_hash: None,
                },
            ],
        };
        assign_tiers(&mut graph, &DefaultTierAssigner);
        assert_eq!(graph.nodes[0].tier, Some(Tier::Tier2));
        assert_eq!(graph.nodes[2].tier, Some(Tier::Tier3));
    }

    #[test]
    fn assign_tiers_does_not_overwrite_a_human_supplied_tier() {
        let mut graph = TaskGraph {
            graph_id: "g".into(),
            nodes: vec![meshloop_domain::task_graph::TaskNode {
                id: TaskId(1),
                description: "a".into(),
                depends_on: vec![],
                tier: Some(Tier::Tier3),
                allowed_paths: vec![],
                empty_diff_ok: false,
                deliverable: None,
                gate: AcceptanceGate::Human,
                risk_q: None,
                risk_prompt_hash: None,
            }],
        };
        assign_tiers(&mut graph, &DefaultTierAssigner);
        assert_eq!(graph.nodes[0].tier, Some(Tier::Tier3));
    }

    fn risk_node(id: u32, description: &str, deps: &[u32], tier: Option<Tier>) -> TaskNode {
        TaskNode {
            id: TaskId(id),
            description: description.into(),
            depends_on: deps.iter().copied().map(TaskId).collect(),
            tier,
            allowed_paths: vec![],
            empty_diff_ok: false,
            deliverable: None,
            gate: AcceptanceGate::Human,
            risk_q: None,
            risk_prompt_hash: None,
        }
    }

    fn risk_graph(nodes: Vec<TaskNode>) -> TaskGraph {
        TaskGraph {
            graph_id: "g".into(),
            nodes,
        }
    }

    fn script_score(description: &str, score: f64) -> ScriptedDecisionPort {
        let subject = format!("risk\n{description}");
        let mut port = ScriptedDecisionPort::default();
        port.insert(
            decision_fingerprint("score", &[subject.as_str()]),
            DecisionOutcome::try_score(score).expect("finite test score"),
        );
        port
    }

    fn script_scores(scores: &[(&str, f64)]) -> ScriptedDecisionPort {
        let mut port = ScriptedDecisionPort::default();
        for (description, score) in scores {
            let subject = format!("risk\n{description}");
            port.insert(
                decision_fingerprint("score", &[subject.as_str()]),
                DecisionOutcome::try_score(*score).expect("finite test score"),
            );
        }
        port
    }

    /// Counts `evaluate_score` and returns `Unscripted` after the first call.
    struct OnceScore {
        score: f64,
        calls: Cell<usize>,
    }

    impl DecisionPort for OnceScore {
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
            let seen = self.calls.get();
            self.calls.set(seen + 1);
            if seen > 0 {
                return Err(DecisionPortError::Unscripted {
                    fingerprint: query.subject.clone(),
                });
            }
            Ok(DecisionOutcome::try_score(self.score).expect("finite test score"))
        }

        fn evaluate_batch_noul(
            &self,
            _query: &BatchNoulQuery,
        ) -> Result<Vec<DecisionOutcome>, DecisionPortError> {
            Err(DecisionPortError::EmptyQuery)
        }
    }

    #[test]
    fn risk_ratchet_low_score_leaf_stays_tier1() {
        let mut graph = risk_graph(vec![risk_node(1, "leaf", &[], None)]);
        let port = script_score("leaf", 0.10);
        apply_risk_ratchet(&mut graph, &port).expect("score");
        assert_eq!(graph.nodes[0].tier, Some(Tier::Tier1));
        assert_eq!(JevTierAssigner.assign_tier(&graph, TaskId(1)), Tier::Tier1);
    }

    #[test]
    fn risk_ratchet_low_score_with_dependent_stays_tier2() {
        let mut graph = risk_graph(vec![
            risk_node(1, "parent", &[], None),
            risk_node(2, "child", &[1], None),
        ]);
        let port = script_scores(&[("parent", 0.10), ("child", 0.10)]);
        apply_risk_ratchet(&mut graph, &port).expect("score");
        assert_eq!(graph.nodes[0].tier, Some(Tier::Tier2));
        assert_eq!(JevTierAssigner.assign_tier(&graph, TaskId(1)), Tier::Tier2);
    }

    #[test]
    fn risk_ratchet_high_score_leaf_becomes_tier3() {
        let mut graph = risk_graph(vec![risk_node(1, "leaf", &[], None)]);
        let port = script_score("leaf", 0.90);
        apply_risk_ratchet(&mut graph, &port).expect("score");
        assert_eq!(graph.nodes[0].tier, Some(Tier::Tier3));
        assert_eq!(graph.nodes[0].risk_q, Some(0.90));
        let subject = "risk\nleaf";
        let expected = decision_fingerprint("risk", &[subject]);
        assert_eq!(
            graph.nodes[0].risk_prompt_hash.as_deref(),
            Some(expected.as_str())
        );
        assert_eq!(JevTierAssigner.assign_tier(&graph, TaskId(1)), Tier::Tier3);
    }

    #[test]
    fn risk_ratchet_low_score_two_dependencies_stays_tier3() {
        let mut graph = risk_graph(vec![
            risk_node(1, "left", &[], None),
            risk_node(2, "right", &[], None),
            risk_node(3, "join", &[1, 2], None),
        ]);
        let port = script_scores(&[("left", 0.0), ("right", 0.0), ("join", 0.0)]);
        apply_risk_ratchet(&mut graph, &port).expect("score");
        assert_eq!(graph.nodes[2].tier, Some(Tier::Tier3));
        assert_eq!(JevTierAssigner.assign_tier(&graph, TaskId(3)), Tier::Tier3);
    }

    #[test]
    fn risk_ratchet_declared_tier1_rises_to_tier3() {
        let mut graph = risk_graph(vec![risk_node(1, "leaf", &[], Some(Tier::Tier1))]);
        let port = script_score("leaf", 0.90);
        apply_risk_ratchet(&mut graph, &port).expect("score");
        assert_eq!(graph.nodes[0].tier, Some(Tier::Tier3));
        assert_eq!(JevTierAssigner.assign_tier(&graph, TaskId(1)), Tier::Tier3);
    }

    #[test]
    fn risk_ratchet_declared_tier1_stays_tier1() {
        let mut graph = risk_graph(vec![
            risk_node(1, "left", &[], None),
            risk_node(2, "right", &[], None),
            risk_node(3, "declared", &[1, 2], Some(Tier::Tier1)),
        ]);
        let port = script_scores(&[("left", 0.10), ("right", 0.10), ("declared", 0.10)]);
        apply_risk_ratchet(&mut graph, &port).expect("score");
        assert_eq!(graph.nodes[2].tier, Some(Tier::Tier1));
        assert_eq!(JevTierAssigner.assign_tier(&graph, TaskId(3)), Tier::Tier1);
    }

    #[test]
    fn risk_ratchet_second_apply_does_not_call_port() {
        let mut graph = risk_graph(vec![risk_node(1, "leaf", &[], None)]);
        let port = OnceScore {
            score: 0.90,
            calls: Cell::new(0),
        };
        apply_risk_ratchet(&mut graph, &port).expect("first score");
        assert_eq!(port.calls.get(), 1);
        assert_eq!(graph.nodes[0].tier, Some(Tier::Tier3));
        assert_eq!(graph.nodes[0].risk_q, Some(0.90));
        let stored_q = graph.nodes[0].risk_q;
        let tier = graph.nodes[0].tier;
        apply_risk_ratchet(&mut graph, &port).expect("reused score");
        assert_eq!(port.calls.get(), 1);
        assert_eq!(graph.nodes[0].tier, tier);
        assert_eq!(graph.nodes[0].risk_q, stored_q);
    }
}
