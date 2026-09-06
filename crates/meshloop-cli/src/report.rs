//! Human-readable status/evidence output. No orchestration logic — see boundaries.md.

use meshloop_domain::task_graph::TaskGraph;

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
    out += "Re-run with `meshloop run --plan <file> --accept-plan` after reviewing the above.\n";
    out
}

#[derive(Debug)]
pub enum NodeOutcome {
    Verified { harness: String, exit_code: i32 },
    Blocked,
    RequiresHumanAcceptance { harness: String },
}

pub fn format_node_result(task_id: u32, outcome: &NodeOutcome) -> String {
    match outcome {
        NodeOutcome::Verified { harness, exit_code } => format!(
            "  [{task_id}] verified via {harness} (exit code {exit_code}) — DeterministicEvidence recorded.\n"
        ),
        NodeOutcome::Blocked => format!(
            "  [{task_id}] BLOCKED — no configured, capable, available candidate (ADR 0009). Not a crash; retry later.\n"
        ),
        NodeOutcome::RequiresHumanAcceptance { harness } => format!(
            "  [{task_id}] verified via {harness} — Tier 3: requires HumanAcceptanceEvidence before `accepted` (ADR 0007). Not yet integrated.\n"
        ),
    }
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
            }],
        };
        let rendered = format_plan(&graph);
        assert!(rendered.contains("awaiting-plan-review"));
        assert!(rendered.contains("do the thing"));
        assert!(rendered.contains("--accept-plan"));
    }

    #[test]
    fn blocked_message_names_the_cause_not_a_stack_trace() {
        let rendered = format_node_result(1, &NodeOutcome::Blocked);
        assert!(rendered.contains("BLOCKED"));
        assert!(rendered.contains("Not a crash"));
    }
}
