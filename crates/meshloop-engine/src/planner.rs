//! Decomposition is one more agent dispatch, not a separate code path (ADR 0009 /
//! runtime-design.md §5). This module dispatches it, validates the result structurally,
//! and assigns tiers — it never judges decomposition *quality* (MECE-ness), which this
//! project cannot mechanically verify; that is the `awaiting-plan-review` human gate's job.

use std::path::{Path, PathBuf};

use meshloop_domain::capability::HarnessError;
use meshloop_domain::evidence::AttemptId;
use meshloop_domain::task_graph::{GraphError, TaskGraph, Tier};

use crate::agent::AgentSpec;
use crate::ports::{HarnessCapabilities, HarnessOutcome};

const PLAN_FILE: &str = "meshloop-plan.json";

/// A plan the planner produced but Meshloop rejected, with the text that was rejected.
struct Rejected {
    error: PlanError,
    raw: Option<String>,
}

impl From<PlanError> for Rejected {
    fn from(error: PlanError) -> Self {
        Self { error, raw: None }
    }
}

fn parse_graph(spec: &AgentSpec, outcome: &HarnessOutcome) -> Result<TaskGraph, Rejected> {
    let file_path = spec.worktree_path.join(PLAN_FILE);
    let raw = if file_path.is_file() {
        std::fs::read_to_string(&file_path).map_err(|e| PlanError::Malformed(e.to_string()))?
    } else {
        outcome.output_redacted.clone()
    };
    let reject = |error: PlanError| Rejected {
        error,
        raw: Some(raw.clone()),
    };
    let graph: TaskGraph =
        serde_json::from_str(&raw).map_err(|e| reject(PlanError::Malformed(e.to_string())))?;
    graph
        .validate()
        .map_err(|e| reject(PlanError::Invalid(e)))?;
    Ok(graph)
}

#[derive(Debug)]
pub enum PlanError {
    Dispatch(HarnessError),
    Malformed(String),
    Invalid(GraphError),
    /// Every allowed attempt produced a rejected plan. `saved_plan` is where the last one
    /// was preserved for inspection, when it could be saved.
    Exhausted {
        attempts: u32,
        last: Box<PlanError>,
        saved_plan: Option<PathBuf>,
    },
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanError::Dispatch(e) => write!(f, "planner dispatch failed: {e:?}"),
            PlanError::Malformed(e) => write!(f, "plan is malformed: {e}"),
            PlanError::Invalid(e) => write!(f, "plan is invalid: {e:?}"),
            PlanError::Exhausted {
                attempts,
                last,
                saved_plan,
            } => {
                write!(f, "planner produced a rejected plan on all {attempts} attempt(s); last error: {last}")?;
                match saved_plan {
                    Some(p) => write!(f, "; last rejected plan saved to {}", p.display()),
                    None => write!(f, "; the rejected plan could not be saved"),
                }
            }
        }
    }
}

impl std::error::Error for PlanError {}

/// Dispatches the decomposition agent, retrying a rejected (malformed or structurally
/// invalid) plan as a new attempt up to `max_retries` more times. Each retry prompt carries
/// the previous error. Dispatch errors are not retried here. The last rejected plan is
/// copied under `rejected_dir` and its path reported in the final error.
pub fn decompose_with_retry(
    harness: &dyn HarnessCapabilities,
    spec: &AgentSpec,
    max_retries: u32,
    rejected_dir: &Path,
) -> Result<TaskGraph, PlanError> {
    let total = max_retries.saturating_add(1);
    let mut attempt_spec = spec.clone();
    for n in 0..total {
        attempt_spec.attempt_id = AttemptId(spec.attempt_id.0.saturating_add(n));
        // A stale file from a previous attempt must never be mistaken for this one's output.
        let _ = std::fs::remove_file(spec.worktree_path.join(PLAN_FILE));
        let rejected = match decompose_inner(harness, &attempt_spec) {
            Ok(graph) => return Ok(graph),
            Err(r) => r,
        };
        if !matches!(
            rejected.error,
            PlanError::Malformed(_) | PlanError::Invalid(_)
        ) {
            return Err(rejected.error);
        }
        if n + 1 == total {
            let saved_plan = save_rejected(rejected_dir, rejected.raw.as_deref());
            return Err(PlanError::Exhausted {
                attempts: total,
                last: Box::new(rejected.error),
                saved_plan,
            });
        }
        attempt_spec.prompt = format!(
            "{}\n\nyour previous graph failed: {}\nFix exactly that problem and rewrite {PLAN_FILE}.",
            spec.prompt, rejected.error
        );
    }
    unreachable!("total is at least 1 and the last iteration returns")
}

fn save_rejected(dir: &Path, raw: Option<&str>) -> Option<PathBuf> {
    let raw = raw?;
    std::fs::create_dir_all(dir).ok()?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = dir.join(format!("rejected-plan-{stamp}-{}.json", std::process::id()));
    std::fs::write(&path, raw).ok()?;
    Some(path)
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

    /// Writes each scripted response to the worktree's plan file on invoke and records the
    /// prompt it was given.
    struct ScriptedPlanner {
        responses: Vec<String>,
        prompts: std::cell::RefCell<Vec<String>>,
    }

    impl HarnessCapabilities for ScriptedPlanner {
        fn probe(&self) -> Result<meshloop_domain::capability::HarnessProfile, HarnessError> {
            unimplemented!()
        }
        fn invoke(&self, spec: &AgentSpec) -> Result<HarnessHandle, HarnessError> {
            let mut prompts = self.prompts.borrow_mut();
            let idx = prompts.len().min(self.responses.len() - 1);
            prompts.push(spec.prompt.clone());
            std::fs::write(spec.worktree_path.join(PLAN_FILE), &self.responses[idx]).unwrap();
            Ok(HarnessHandle {
                attempt_id: spec.attempt_id,
                pid: None,
                pane_id: None,
            })
        }
        fn cancel(&self, _handle: &HarnessHandle) -> Result<(), HarnessError> {
            Ok(())
        }
        fn collect(&self, _handle: &HarnessHandle) -> Result<HarnessOutcome, HarnessError> {
            Ok(HarnessOutcome {
                exit_code: 0,
                output_redacted: String::new(),
                worktree_changed: true,
            })
        }
    }

    const VALID: &str = r#"{"graph_id":"g","nodes":[{"id":1,"description":"d","depends_on":[],"tier":"Tier1"}]}"#;
    const BAD_TIER: &str = r#"{"graph_id":"g","nodes":[{"id":1,"description":"d","depends_on":[],"tier":"analysis"}]}"#;

    fn scratch(name: &str) -> (PathBuf, AgentSpec) {
        let dir = std::env::temp_dir().join(format!("meshloop-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let spec = crate::agent::build_planning_spec(
            "objective",
            "scope",
            AttemptId(0),
            "claude-code",
            "m",
            dir.clone(),
            Duration::from_secs(60),
        );
        (dir, spec)
    }

    #[test]
    fn invalid_tier_then_valid_plan_succeeds_on_second_attempt_with_error_in_prompt() {
        let (dir, spec) = scratch("retry-ok");
        let harness = ScriptedPlanner {
            responses: vec![BAD_TIER.into(), VALID.into()],
            prompts: Default::default(),
        };
        let graph = decompose_with_retry(&harness, &spec, 2, &dir.join("rejected"))
            .expect("second attempt is valid");
        assert_eq!(graph.nodes.len(), 1);
        let prompts = harness.prompts.borrow();
        assert_eq!(prompts.len(), 2);
        assert!(!prompts[0].contains("your previous graph failed"));
        assert!(prompts[1].contains("your previous graph failed"));
        assert!(prompts[1].contains("unknown variant `analysis`"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn all_attempts_malformed_reports_saved_rejected_plan_that_exists() {
        let (dir, spec) = scratch("retry-exhausted");
        let harness = ScriptedPlanner {
            responses: vec![BAD_TIER.into()],
            prompts: Default::default(),
        };
        let err = decompose_with_retry(&harness, &spec, 2, &dir.join("rejected"))
            .expect_err("every attempt is rejected");
        assert_eq!(harness.prompts.borrow().len(), 3);
        let PlanError::Exhausted {
            attempts,
            saved_plan: Some(path),
            ..
        } = &err
        else {
            panic!("expected Exhausted with a saved plan, got {err:?}");
        };
        assert_eq!(*attempts, 3);
        assert!(path.is_file());
        assert_eq!(std::fs::read_to_string(path).unwrap(), BAD_TIER);
        assert!(err.to_string().contains(&path.display().to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dispatch_errors_are_not_retried() {
        let (dir, spec) = scratch("retry-dispatch");
        let harness = FakeHarness {
            response: Err(HarnessError::Timeout),
        };
        let err = decompose_with_retry(&harness, &spec, 2, &dir.join("rejected")).unwrap_err();
        assert!(matches!(err, PlanError::Dispatch(_)));
        let _ = std::fs::remove_dir_all(&dir);
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
