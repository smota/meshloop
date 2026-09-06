//! Decomposition is one more agent dispatch, not a separate code path (ADR 0009 /
//! runtime-design.md §5). This module dispatches it, validates the result structurally,
//! and assigns tiers — it never judges decomposition *quality* (MECE-ness), which this
//! project cannot mechanically verify; that is the `awaiting-plan-review` human gate's job.

use meshloop_domain::capability::HarnessError;
use meshloop_domain::task_graph::{GraphError, TaskGraph, Tier};

use crate::agent::AgentSpec;
use crate::ports::{HarnessCapabilities, HarnessOutcome};

fn parse_graph(spec: &AgentSpec, outcome: &HarnessOutcome) -> Result<TaskGraph, PlanError> {
    let file_path = spec.worktree_path.join("meshloop-plan.json");
    let raw = if file_path.is_file() {
        std::fs::read_to_string(&file_path).map_err(|e| PlanError::Malformed(e.to_string()))?
    } else {
        outcome.output_redacted.clone()
    };
    let graph: TaskGraph =
        serde_json::from_str(&raw).map_err(|e| PlanError::Malformed(e.to_string()))?;
    graph.validate().map_err(PlanError::Invalid)?;
    Ok(graph)
}

#[derive(Debug)]
pub enum PlanError {
    Dispatch(HarnessError),
    Malformed(String),
    Invalid(GraphError),
}

/// Dispatches the decomposition agent and returns a structurally-validated graph, or a
/// `PlanError` the caller should turn into a bounded retry (a new attempt, ADR 0005) — a
/// cyclic or schema-invalid graph must never be partially scheduled.
pub fn decompose(
    harness: &dyn HarnessCapabilities,
    spec: &AgentSpec,
) -> Result<TaskGraph, PlanError> {
    let handle = harness.invoke(spec).map_err(PlanError::Dispatch)?;
    let outcome: HarnessOutcome = harness.collect(&handle).map_err(PlanError::Dispatch)?;
    parse_graph(spec, &outcome)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::HarnessHandle;
    use meshloop_domain::evidence::AttemptId;
    use meshloop_domain::task_graph::TaskId;
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
                },
                meshloop_domain::task_graph::TaskNode {
                    id: TaskId(2),
                    description: "b".into(),
                    depends_on: vec![],
                    tier: None,
                    allowed_paths: vec![],
                    empty_diff_ok: false,
                },
                meshloop_domain::task_graph::TaskNode {
                    id: TaskId(3),
                    description: "c".into(),
                    depends_on: vec![TaskId(1), TaskId(2)],
                    tier: None,
                    allowed_paths: vec![],
                    empty_diff_ok: false,
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
            }],
        };
        assign_tiers(&mut graph, &DefaultTierAssigner);
        assert_eq!(graph.nodes[0].tier, Some(Tier::Tier3));
    }
}
