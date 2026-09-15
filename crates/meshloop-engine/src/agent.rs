//! Command pattern (design-patterns.md): AgentSpec is an immutable value object describing
//! one unit of work. This module only constructs it — it never dispatches, never chooses a
//! harness or model (ADR 0003/0009's "no routing/dispatch authority" rule).

use std::path::PathBuf;
use std::time::Duration;

use meshloop_domain::evidence::AttemptId;
use meshloop_domain::task_graph::{TaskGraph, TaskId, TaskNode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSpec {
    pub task_id: TaskId,
    pub attempt_id: AttemptId,
    pub harness: String,
    pub model_ref: String,
    pub worktree_path: PathBuf,
    pub prompt: String,
    pub timeout: Duration,
}

/// Template method (design-patterns.md): a fixed four-slot envelope, never a per-harness
/// ad hoc string-builder.
pub struct PromptEnvelope {
    pub situation: String,
    pub complication: String,
    pub question: String,
    pub output_contract: String,
}

pub fn render_prompt(envelope: &PromptEnvelope) -> String {
    format!(
        "SITUATION:\n{}\n\nCOMPLICATION:\n{}\n\nQUESTION:\n{}\n\nOUTPUT CONTRACT:\n{}\n",
        envelope.situation, envelope.complication, envelope.question, envelope.output_contract
    )
}

/// Assembles context from only a node's declared dependencies — never the whole graph —
/// per ML-012 and ADR 0003's isolation requirement.
pub fn dependency_context(graph: &TaskGraph, node: &TaskNode) -> String {
    node.depends_on
        .iter()
        .filter_map(|dep_id| graph.nodes.iter().find(|n| n.id == *dep_id))
        .map(|dep| dep.description.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

pub const DEFAULT_OUTPUT_CONTRACT: &str =
    "Touch only paths relevant to this task. Leave the worktree in a buildable state.";

pub fn build_agent_spec(
    graph: &TaskGraph,
    node: &TaskNode,
    attempt_id: AttemptId,
    harness: &str,
    model_ref: &str,
    worktree_path: PathBuf,
    timeout: Duration,
) -> AgentSpec {
    build_agent_spec_with_context(
        graph,
        node,
        attempt_id,
        harness,
        model_ref,
        worktree_path,
        timeout,
        &[],
    )
}

/// Builds an AgentSpec enriched with AST skeletons and cache-optimized static prefix.
#[allow(clippy::too_many_arguments)]
pub fn build_agent_spec_with_context(
    graph: &TaskGraph,
    node: &TaskNode,
    attempt_id: AttemptId,
    harness: &str,
    model_ref: &str,
    worktree_path: PathBuf,
    timeout: Duration,
    skeletons: &[(String, String)],
) -> AgentSpec {
    let prompt = if skeletons.is_empty() {
        render_prompt(&PromptEnvelope {
            situation: dependency_context(graph, node),
            complication: node.description.clone(),
            question: "Implement this change and leave evidence of what was done.".into(),
            output_contract: DEFAULT_OUTPUT_CONTRACT.into(),
        })
    } else {
        let ranked = meshloop_context::select_context(
            &node.description,
            skeletons,
            &node.allowed_paths,
            meshloop_context::DEFAULT_SKELETON_BUDGET,
        );
        let mut builder = meshloop_context::PromptCacheBuilder::new()
            .with_contract(DEFAULT_OUTPUT_CONTRACT)
            .with_system_rule("Touch only allowed paths. Leave the worktree in a buildable state.");

        for (path, skel) in &ranked {
            builder = builder.with_ast_skeleton(path, skel);
        }

        let dep_ctx = dependency_context(graph, node);
        let task_desc = if dep_ctx.is_empty() {
            node.description.clone()
        } else {
            format!("DEPENDENCIES:\n{}\n\nTASK:\n{}", dep_ctx, node.description)
        };

        builder
            .build(
                &task_desc,
                "Implement this change and leave evidence of what was done.",
            )
            .render()
    };

    AgentSpec {
        task_id: node.id,
        attempt_id,
        harness: harness.into(),
        model_ref: model_ref.into(),
        worktree_path,
        prompt,
        timeout,
    }
}

/// Builds an AgentSpec for an inner-loop repair round, including normalized failure diagnostics
/// and negative constraints derived from the diagnostic lattice (ADR 0026).
#[allow(clippy::too_many_arguments)]
pub fn build_repair_spec(
    node: &TaskNode,
    attempt_id: AttemptId,
    harness: &str,
    model_ref: &str,
    worktree_path: PathBuf,
    timeout: Duration,
    diagnostics: &str,
    negative_constraint: &str,
) -> AgentSpec {
    let complication = if negative_constraint.is_empty() {
        format!(
            "{}\n\n[PREVIOUS ATTEMPT FAILED CHECKS. DIAGNOSTICS:]\n{}",
            node.description, diagnostics
        )
    } else {
        format!(
            "{}\n\n[PREVIOUS ATTEMPT FAILED CHECKS. DIAGNOSTICS:]\n{}\n\n[NEGATIVE CONSTRAINTS - DO NOT INTRODUCE THESE CODES:]\n{}",
            node.description, diagnostics, negative_constraint
        )
    };
    let prompt = render_prompt(&PromptEnvelope {
        situation:
            "A previous attempt introduced errors or failed deterministic checks. Fix the errors."
                .into(),
        complication,
        question:
            "Fix the diagnostics while preserving existing behavior. Touch only allowed paths."
                .into(),
        output_contract: DEFAULT_OUTPUT_CONTRACT.into(),
    });
    AgentSpec {
        task_id: node.id,
        attempt_id,
        harness: harness.into(),
        model_ref: model_ref.into(),
        worktree_path,
        prompt,
        timeout,
    }
}

/// The decomposition dispatch itself (ADR 0009/runtime-design.md §5) is an AgentSpec whose
/// declared output contract is a task graph, not a worktree diff.
pub fn build_planning_spec(
    objective: &str,
    scope_and_exclusions: &str,
    attempt_id: AttemptId,
    harness: &str,
    model_ref: &str,
    worktree_path: PathBuf,
    timeout: Duration,
) -> AgentSpec {
    let prompt = render_prompt(&PromptEnvelope {
        situation: scope_and_exclusions.to_string(),
        complication: objective.to_string(),
        question: "Decompose this objective into a versioned task graph.".into(),
        output_contract: "Write exactly one JSON TaskGraph to meshloop-plan.json in this worktree \
             (graph_id matching [A-Za-z0-9._-]+, nodes[] with id >= 1, description, \
             depends_on, optional tier, optional allowed_paths, optional empty_diff_ok). \
             No other files unless required to produce that graph. Do not print prose."
            .into(),
    });
    AgentSpec {
        task_id: TaskId(0),
        attempt_id,
        harness: harness.into(),
        model_ref: model_ref.into(),
        worktree_path,
        prompt,
        timeout,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use meshloop_domain::task_graph::Tier;

    fn node(id: u32, desc: &str, deps: &[u32]) -> TaskNode {
        TaskNode {
            id: TaskId(id),
            description: desc.into(),
            depends_on: deps.iter().map(|d| TaskId(*d)).collect(),
            tier: Some(Tier::Tier1),
            allowed_paths: vec![],
            empty_diff_ok: false,
        }
    }

    #[test]
    fn prompt_contains_all_four_slots() {
        let rendered = render_prompt(&PromptEnvelope {
            situation: "S".into(),
            complication: "C".into(),
            question: "Q".into(),
            output_contract: "O".into(),
        });
        for marker in [
            "SITUATION",
            "COMPLICATION",
            "QUESTION",
            "OUTPUT CONTRACT",
            "S",
            "C",
            "Q",
            "O",
        ] {
            assert!(rendered.contains(marker));
        }
    }

    #[test]
    fn dependency_context_excludes_unrelated_sibling_nodes() {
        let graph = TaskGraph {
            graph_id: "g".into(),
            nodes: vec![
                node(1, "SECRET_UNRELATED_TASK_TEXT", &[]),
                node(2, "the real dependency", &[]),
                node(3, "depends only on 2", &[2]),
            ],
        };
        let target = &graph.nodes[2];
        let ctx = dependency_context(&graph, target);
        assert!(ctx.contains("the real dependency"));
        assert!(!ctx.contains("SECRET_UNRELATED_TASK_TEXT"));
    }

    #[test]
    fn built_spec_prompt_never_leaks_unrelated_task_context() {
        let graph = TaskGraph {
            graph_id: "g".into(),
            nodes: vec![
                node(1, "SECRET_UNRELATED_TASK_TEXT", &[]),
                node(2, "depends on nothing", &[]),
            ],
        };
        let target = &graph.nodes[1];
        let spec = build_agent_spec(
            &graph,
            target,
            AttemptId(1),
            "claude-code",
            "configured-model",
            PathBuf::from("/tmp/wt"),
            Duration::from_secs(300),
        );
        assert!(!spec.prompt.contains("SECRET_UNRELATED_TASK_TEXT"));
        assert!(spec.prompt.contains("depends on nothing"));
    }

    #[test]
    fn planning_spec_declares_task_graph_output_contract_not_a_diff() {
        let spec = build_planning_spec(
            "build a widget",
            "scope: this repo only",
            AttemptId(1),
            "claude-code",
            "configured-model",
            PathBuf::from("/tmp/wt"),
            Duration::from_secs(300),
        );
        assert!(spec.prompt.contains("meshloop-plan.json"));
    }

    #[test]
    fn built_spec_with_context_includes_ast_skeletons() {
        let graph = TaskGraph {
            graph_id: "g".into(),
            nodes: vec![node(1, "implement authentication", &[])],
        };
        let target = &graph.nodes[0];
        let skeletons = vec![(
            "src/auth.rs".to_string(),
            "pub trait Auth { fn verify(&self); }".to_string(),
        )];
        let spec = build_agent_spec_with_context(
            &graph,
            target,
            AttemptId(1),
            "claude-code",
            "configured-model",
            PathBuf::from("/tmp/wt"),
            Duration::from_secs(300),
            &skeletons,
        );
        assert!(spec.prompt.contains("pub trait Auth"));
        assert!(spec.prompt.contains("src/auth.rs"));
        assert!(spec.prompt.contains("SYSTEM POLICIES"));
        assert!(spec.prompt.contains("TASK ASSIGNMENT"));
    }

    #[test]
    fn repair_spec_contains_diagnostics_and_negative_constraints() {
        let n = node(1, "implement authentication", &[]);
        let spec = build_repair_spec(
            &n,
            AttemptId(1),
            "claude-code",
            "configured-model",
            PathBuf::from("/tmp/wt"),
            Duration::from_secs(300),
            "error[E0308]: mismatched types",
            "E0308, E0425",
        );
        assert!(spec.prompt.contains("DIAGNOSTICS:"));
        assert!(spec.prompt.contains("error[E0308]: mismatched types"));
        assert!(spec.prompt.contains("NEGATIVE CONSTRAINTS"));
        assert!(spec.prompt.contains("E0308, E0425"));
    }

    #[test]
    fn context_budget_keeps_the_query_relevant_file() {
        let graph = TaskGraph {
            graph_id: "g".into(),
            nodes: vec![node(1, "implement Auth trait verify", &[])],
        };
        let target = &graph.nodes[0];
        let mut skeletons = Vec::new();
        for i in 0..40 {
            skeletons.push((
                format!("src/n{i}.rs"),
                format!("pub fn n{i}() {{ /* ... */ }}"),
            ));
        }
        skeletons.push((
            "src/auth.rs".into(),
            "pub trait Auth { fn verify(&self); }".into(),
        ));
        let spec = build_agent_spec_with_context(
            &graph,
            target,
            AttemptId(1),
            "claude-code",
            "configured-model",
            PathBuf::from("/tmp/wt"),
            Duration::from_secs(300),
            &skeletons,
        );
        assert!(spec.prompt.contains("src/auth.rs"));
        assert!(spec.prompt.contains("pub trait Auth"));
        let mentioned = (0..40)
            .filter(|i| spec.prompt.contains(&format!("src/n{i}.rs")))
            .count();
        assert!(
            mentioned < 40,
            "budget should drop noise files, kept {mentioned}"
        );
    }
}
