//! Human-readable status/evidence output. No orchestration logic — see boundaries.md.

use meshloop_domain::state::TaskState;
use meshloop_domain::task_graph::TaskGraph;
use meshloop_engine::run_loop::{AttemptView, IdleReason, RunStatus};

pub const ACCEPTANCE_RULE: &str = "R1: a verify gate on tier 1 or 2 accepts after deterministic checks; every other node stops for meshloop accept; tier selects harness/model only";

/// Shown when routing finds no planner and `[planner]` names no harness.
pub const PLANNER_ROUTING_HINT: &str = "The planner is routed as a Tier3 task by default, which needs a harness with      model_tier = \"top\". To plan with another harness, set `[planner] harness = \"<name>\"`      (or `[planner] tier`) in meshloop.toml.";

pub fn format_plan(graph: &TaskGraph) -> String {
    let mut out = format!(
        "Plan '{}': {} task(s) — awaiting-plan-review (ADR 0009).\n\
         This plan has NOT been reviewed. Decomposition quality (MECE coverage) cannot be\n\
         mechanically verified; only its structure (no cycles, no dangling dependencies) was.\n",
        graph.graph_id,
        graph.nodes.len()
    );
    for node in &graph.nodes {
        let deps: Vec<String> = node.depends_on.iter().map(|d| d.0.to_string()).collect();
        out += &format!(
            "  [{}] {} (tier: {:?}, depends_on: [{}]{})\n",
            node.id.0,
            node.description,
            node.tier,
            deps.join(", "),
            node.deliverable
                .as_deref()
                .map(|d| format!(", deliverable: {d}"))
                .unwrap_or_default()
        );
    }
    out += "Next: `meshloop review-plan --plan <file> --accept|--decline|--adjust`.\n\
            Or start in one step with `meshloop run --plan <file> --accept-plan`.\n";
    out
}

pub fn format_idle(reason: IdleReason, status: Option<&RunStatus>) -> String {
    match reason {
        IdleReason::GraphComplete => "Graph complete: every node is Integrated.\n".into(),
        IdleReason::AwaitingHumanAcceptance => {
            "Paused: one or more nodes await `meshloop accept --task <id> --as <you>`.\n\
             Then `meshloop resume` to merge into the integrate worktree.\n"
                .into()
        }
        IdleReason::NoCapableCandidate => {
            "No configured, capable, available candidate (ADR 0009). Nodes left Ready.\n\
             Use `meshloop cancel --task <id>` to unblock dependents.\n"
                .into()
        }
        IdleReason::FailedTerminal => format_failed_terminal(status),
        IdleReason::WaitingOnLiveWorker => {
            "A direct-CLI worker is still live. Meshloop is waiting on that worker, not the 5s stall gate.\n\
             `meshloop status` shows pane_id/live. Do not retry while it is working.\n"
                .into()
        }
    }
}

const FAILED_TERMINAL_GUIDANCE: &str = "`resume --retry` if attempts remain; `resume --restart` re-runs this accepted plan without replanning.";

/// Failed/Blocked/Cancelled/Integrated match the engine idle scan. Anything else still
/// needs a driver or an operator; do not call that "nothing runnable".
fn is_terminal_task(state: TaskState) -> bool {
    matches!(
        state,
        TaskState::Integrated | TaskState::Failed | TaskState::Cancelled | TaskState::Blocked
    )
}

fn undriven_task_ids(status: &RunStatus) -> Vec<u32> {
    let mut ids: Vec<u32> = status
        .nodes
        .iter()
        .filter(|node| !is_terminal_task(node.state) && node.live.as_deref() != Some("Live"))
        .map(|node| node.task_id.0)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

fn format_failed_terminal(status: Option<&RunStatus>) -> String {
    let undriven = status.map(undriven_task_ids).unwrap_or_default();
    if undriven.is_empty() {
        format!(
            "Graph stopped: Failed/Blocked/Cancelled with nothing runnable. {FAILED_TERMINAL_GUIDANCE}\n"
        )
    } else {
        let ids = undriven
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "Graph stopped: non-terminal task(s) {ids} have no live driver. {FAILED_TERMINAL_GUIDANCE}\n"
        )
    }
}

pub fn format_status(status: &RunStatus) -> String {
    let mut out = format!(
        "Run '{}': plan_state={:?}\n",
        status.graph_id, status.plan_state
    );
    for n in &status.nodes {
        let wt = n
            .worktree
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "-".into());
        out += &format!(
            "  [{}] {:?} {}  worktree={} pane={} live={}\n",
            n.task_id.0,
            n.state,
            n.description,
            wt,
            n.pane_id.as_deref().unwrap_or("-"),
            n.live.as_deref().unwrap_or("-")
        );
        if let Some(d) = &n.deliverable {
            out += &format!(
                "      deliverable: {d}
"
            );
        }
        if let Some(w) = n.waiting_for {
            if n.blocked_by.is_empty() {
                out += &format!("      waiting: {}\n", w.as_str());
            } else {
                let ids: Vec<String> = n.blocked_by.iter().map(|d| d.0.to_string()).collect();
                out += &format!("      waiting: {} {}\n", w.as_str(), ids.join(","));
            }
        }
        if let Some(note) = &n.note {
            out += &format!("      note: {note}\n");
        }
        if n.state == TaskState::AwaitingReview {
            out += &format!(
                "      next: meshloop accept --task {} --as <identity>\n",
                n.task_id.0
            );
        }
    }
    out
}

pub fn attempt_json(a: &AttemptView) -> serde_json::Value {
    serde_json::json!({
        "attempt_id": a.attempt_id.0,
        "harness": a.harness,
        "model_ref": a.model_ref,
        "started_at": a.started_at,
        "ended_at": a.ended_at,
        "duration_s": a.duration_s,
        "outcome": a.outcome,
        "base_revision": a.base_revision,
        "evidence": a.evidence.iter().map(|e| serde_json::json!({
            "kind": e.kind,
            "tool": e.tool,
            "exit_code": e.exit_code,
            "summary_redacted": e.summary_redacted,
        })).collect::<Vec<_>>(),
    })
}

pub fn scoped_attempt_json(
    a: &AttemptView,
    frame: &meshloop_engine::workspace::read::ReadFrame,
) -> serde_json::Value {
    let mut value = attempt_json(a);
    if let Some(location) = frame.attempt_locations.get(&a.attempt_id.0) {
        value["location"] = serde_json::to_value(location).unwrap_or(serde_json::Value::Null);
        value["generation_id"] =
            serde_json::to_value(&frame.generation_id).unwrap_or(serde_json::Value::Null);
        value["revision_id"] = serde_json::to_value(frame.attempt_revisions.get(&a.attempt_id.0))
            .unwrap_or(serde_json::Value::Null);
    }
    value
}

pub fn format_read_scope(frame: &meshloop_engine::workspace::read::ReadFrame) -> String {
    let (Some(generation), Some(revision)) = (&frame.generation_id, &frame.revision_id) else {
        return String::new();
    };
    let mut text = format!(
        "generation={} revision={}\n",
        generation.as_str(),
        revision.as_str()
    );
    for node in &frame.status.nodes {
        if let Some(location) = frame.locations.get(&node.task_id) {
            text += &format!(
                "  [{}] location={}:{}\n",
                node.task_id.0,
                location.root_alias.as_str(),
                location.relative_path.as_str()
            );
        }
    }
    text += &format!(
        "git_export unavailable: {}\n",
        meshloop_engine::workspace::read::BOUND_GIT_EXPORT_UNAVAILABLE
    );
    text
}

pub fn format_attempt(a: &AttemptView) -> String {
    format!(
        "  attempt {} harness={} model={} duration={} outcome={} evidence={}",
        a.attempt_id.0,
        a.harness.as_deref().unwrap_or("-"),
        a.model_ref.as_deref().unwrap_or("-"),
        a.duration_s
            .map(|d| format!("{d}s"))
            .unwrap_or_else(|| "-".into()),
        a.outcome.as_deref().unwrap_or("-"),
        a.evidence.len()
    )
}

pub fn banner() -> String {
    "Meshloop: agent-session control plane (native Windows). Binary is the engine; skills/MCP are the UX.\n\
     Commands: plan, review-plan, run --accept-plan, status, resume, cancel, inspect, accept, integrate, orchestrate, doctor, roles, watch, mutate-plan, workspace, mcp, bundle.\n\
     Default store: .meshloop/state.sqlite. Worktrees are kept; `run` does not merge to your branch.\n\
     Live workers are direct CLI subprocesses in Git worktrees. --fixture-only is the CI subprocess double.\n"
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use meshloop_domain::state::PlanState;
    use meshloop_domain::task_graph::{AcceptanceGate, TaskId, TaskNode, Tier};
    use meshloop_engine::run_loop::NodeStatus;

    fn node(id: u32, state: TaskState, live: Option<&str>) -> NodeStatus {
        NodeStatus {
            task_id: TaskId(id),
            waiting_for: None,
            blocked_by: vec![],
            description: "task".into(),
            state,
            note: None,
            worktree: None,
            revision: None,
            pane_id: None,
            live: live.map(str::to_string),
            deliverable: None,
        }
    }

    fn status(nodes: Vec<NodeStatus>) -> RunStatus {
        RunStatus {
            graph_id: "g".into(),
            plan_state: PlanState::PlanAccepted,
            nodes,
        }
    }

    #[test]
    fn format_plan_lists_every_node_and_flags_review_requirement() {
        let graph = TaskGraph {
            graph_id: "g1".into(),
            nodes: vec![TaskNode {
                id: TaskId(1),
                description: "do the thing".into(),
                depends_on: vec![],
                tier: Some(Tier::Tier1),
                allowed_paths: vec![],
                empty_diff_ok: false,
                deliverable: None,
                gate: AcceptanceGate::Human,
            }],
        };
        let rendered = format_plan(&graph);
        assert!(rendered.contains("awaiting-plan-review"));
        assert!(rendered.contains("do the thing"));
        assert!(rendered.contains("--accept-plan"));
    }

    #[test]
    fn failed_terminal_keeps_retry_and_restart_guidance() {
        let expected = "Graph stopped: Failed/Blocked/Cancelled with nothing runnable. \
             `resume --retry` if attempts remain; `resume --restart` re-runs this accepted plan without replanning.\n";
        assert_eq!(format_idle(IdleReason::FailedTerminal, None), expected);
        let failed = status(vec![node(1, TaskState::Failed, Some("Dead"))]);
        assert_eq!(
            format_idle(IdleReason::FailedTerminal, Some(&failed)),
            expected
        );
        assert!(expected.contains("`resume --retry` if attempts remain"));
        assert!(
            expected.contains("`resume --restart` re-runs this accepted plan without replanning")
        );
    }

    #[test]
    fn failed_terminal_names_undriven_nonterminal_tasks() {
        let stuck = status(vec![
            node(2, TaskState::Ready, None),
            node(1, TaskState::Failed, Some("Dead")),
            node(3, TaskState::Verifying, Some("Live")),
        ]);
        let text = format_idle(IdleReason::FailedTerminal, Some(&stuck));
        assert!(text.contains("non-terminal task(s) 2"));
        assert!(!text.contains("nothing runnable"));
        assert!(text.contains("`resume --retry` if attempts remain"));
        assert!(text.contains("`resume --restart` re-runs this accepted plan without replanning"));
        assert!(!text.contains("task(s) 1"));
        assert!(!text.contains("task(s) 3"));
    }
}
