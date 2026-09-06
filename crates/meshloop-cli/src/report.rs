//! Human-readable status/evidence output. No orchestration logic — see boundaries.md.

use meshloop_domain::state::TaskState;
use meshloop_domain::task_graph::TaskGraph;
use meshloop_engine::run_loop::{IdleReason, RunStatus};

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
            "  [{}] {} (tier: {:?}, depends_on: [{}])\n",
            node.id.0,
            node.description,
            node.tier,
            deps.join(", ")
        );
    }
    out += "Next: `meshloop review-plan --plan <file> --accept|--decline|--adjust`.\n\
            Or start in one step with `meshloop run --plan <file> --accept-plan`.\n";
    out
}

pub fn format_idle(reason: IdleReason) -> String {
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
        IdleReason::FailedTerminal => {
            "Graph stopped: Failed/Blocked/Cancelled with nothing runnable. `resume --retry` may apply.\n"
                .into()
        }
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
            "  [{}] {:?} {}  worktree={}\n",
            n.task_id.0, n.state, n.description, wt
        );
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

pub fn banner() -> String {
    "Meshloop: agent-session control plane (native Windows). Binary is the engine; skills/MCP are the UX.\n\
     Commands: plan, review-plan, run --accept-plan, status, resume, cancel, inspect, accept, integrate, orchestrate, doctor.\n\
     Default store: .meshloop/state.sqlite. Worktrees are kept; `run` does not merge to your branch.\n\
     Live workers use Herdr. --fixture-only is the CI subprocess double.\n"
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use meshloop_domain::task_graph::{TaskId, TaskNode, Tier};

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
            }],
        };
        let rendered = format_plan(&graph);
        assert!(rendered.contains("awaiting-plan-review"));
        assert!(rendered.contains("do the thing"));
        assert!(rendered.contains("--accept-plan"));
    }
}
