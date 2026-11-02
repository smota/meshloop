//! Command pattern (design-patterns.md): AgentSpec is an immutable value object describing
//! one unit of work. This module only constructs it — it never dispatches, never chooses a
//! harness or model (ADR 0003/0009's "no routing/dispatch authority" rule).

use std::path::PathBuf;
use std::time::Duration;

use meshloop_domain::decision::DecisionOutcome;
use meshloop_domain::evidence::AttemptId;
use meshloop_domain::task_graph::{AcceptanceGate, TaskGraph, TaskId, TaskNode, Tier};

use crate::ports::{BatchNoulQuery, DecisionPort};

/// `DecisionPort` rejects batches larger than this. Do not raise it.
const MAX_UNPINNED_NOUL: usize = 24;

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
        None,
    )
}

/// Builds an AgentSpec enriched with AST skeletons and a cache-optimized static prefix.
///
/// `noul` of `None` does not query and keeps the FWHT prompt. `Some` reranks only the
/// unpinned suffix with one batched noul query before skeletons are rendered.
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
    noul: Option<&dyn DecisionPort>,
) -> AgentSpec {
    let prompt = if skeletons.is_empty() {
        render_prompt(&PromptEnvelope {
            situation: dependency_context(graph, node),
            complication: node.description.clone(),
            question: "Implement this change and leave evidence of what was done.".into(),
            output_contract: DEFAULT_OUTPUT_CONTRACT.into(),
        })
    } else {
        let selected = meshloop_context::select_context(
            &node.description,
            skeletons,
            &node.allowed_paths,
            meshloop_context::DEFAULT_SKELETON_BUDGET,
        );
        let (ranked, accepted_noul) = match noul {
            Some(port) => {
                rerank_unpinned_suffix(&selected, &node.description, &node.allowed_paths, port)
            }
            None => (selected, false),
        };
        let mut builder = meshloop_context::PromptCacheBuilder::new()
            .with_contract(DEFAULT_OUTPUT_CONTRACT)
            .with_system_rule("Touch only allowed paths. Leave the worktree in a buildable state.");
        if accepted_noul {
            builder = builder.preserve_skeleton_order();
        }

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

/// Permute unpinned slots by one batched noul query. Pinned indices stay fixed.
/// The bool is true only when every returned score was usable.
/// An unusable response, an empty suffix, or a suffix longer than
/// [`MAX_UNPINNED_NOUL`] keeps `selected` and reports false. Nothing is dropped.
pub fn rerank_unpinned_suffix(
    selected: &[(String, String)],
    description: &str,
    pin_prefixes: &[String],
    noul: &dyn DecisionPort,
) -> (Vec<(String, String)>, bool) {
    let mut unpinned_at = Vec::new();
    for (index, (path, _)) in selected.iter().enumerate() {
        if !meshloop_context::path_pinned(path, pin_prefixes) {
            unpinned_at.push(index);
        }
    }
    if unpinned_at.is_empty() || unpinned_at.len() > MAX_UNPINNED_NOUL {
        return (selected.to_vec(), false);
    }
    let propositions: Vec<String> = unpinned_at
        .iter()
        .map(|&index| format!("{description}\n{}", selected[index].0))
        .collect();
    let Ok(outcomes) = noul.evaluate_batch_noul(&BatchNoulQuery { propositions }) else {
        return (selected.to_vec(), false);
    };
    if outcomes.len() != unpinned_at.len() {
        return (selected.to_vec(), false);
    }
    let mut scores = Vec::with_capacity(outcomes.len());
    for outcome in outcomes {
        let DecisionOutcome::Score(score) = outcome else {
            return (selected.to_vec(), false);
        };
        scores.push(score.get());
    }
    // noul descending, then earlier FWHT suffix index, then path ascending.
    let mut suffix_order: Vec<usize> = (0..unpinned_at.len()).collect();
    suffix_order.sort_by(|&left, &right| {
        scores[right]
            .total_cmp(&scores[left])
            .then(left.cmp(&right))
            .then(
                selected[unpinned_at[left]]
                    .0
                    .cmp(&selected[unpinned_at[right]].0),
            )
    });
    let mut result = selected.to_vec();
    for (slot, suffix_index) in unpinned_at.iter().zip(suffix_order) {
        result[*slot] = selected[unpinned_at[suffix_index]].clone();
    }
    (result, true)
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

/// The planner's output contract: every `TaskNode` field with its allowed values, the
/// accepted tier names (taken from the domain type so they cannot drift), and a one-node
/// example serialized from the domain types.
fn planning_output_contract() -> String {
    let tiers = Tier::ALL
        .iter()
        .map(|t| t.name())
        .collect::<Vec<_>>()
        .join("|");
    let example = TaskGraph {
        graph_id: "example-graph".into(),
        nodes: vec![TaskNode {
            id: TaskId(1),
            description: "Add the parse_widget function with unit tests".into(),
            depends_on: vec![],
            tier: Some(Tier::Tier1),
            allowed_paths: vec!["crates/widgets/src/".into()],
            empty_diff_ok: false,
            deliverable: Some("issue-12".into()),
            gate: AcceptanceGate::Human,
            risk_q: None,
            risk_prompt_hash: None,
        }],
    };
    let example = serde_json::to_string_pretty(&example).unwrap_or_default();
    format!(
        "Write exactly one JSON TaskGraph to meshloop-plan.json in this worktree. \
         Unknown values are rejected, so use exactly these fields.\n\
         graph_id: string matching [A-Za-z0-9._-]+ (no slashes or spaces).\n\
         nodes: non-empty array; each node has:\n\
         - id: integer >= 1, unique within the graph.\n\
         - description: string, what this task must accomplish.\n\
         - depends_on: array of ids of other nodes that must finish first ([] if none); no cycles.\n\
         - tier: exactly one of {tiers}, or omit it (null) to let Meshloop assign one. \
         Never a role or phase name such as \"analysis\" or \"implementation\".\n\
         - allowed_paths: array of repo-relative path prefixes the task may change \
         (e.g. \"crates/foo/src/\"); omit or [] for no restriction.\n\
         - empty_diff_ok: boolean, default false. Set true only for a node that legitimately \
         produces no diff (e.g. a pure analysis or verification step); otherwise leave false.\n\
         - deliverable: optional string matching [A-Za-z0-9._-]+ naming what the node ships \
         with, e.g. \"issue-12\" when one graph delivers several issues. Nodes that share it \
         are integrated together. Omit it when the graph has one deliverable.\n\
         - gate: \"human\" (default; omit it) or \"verify\". verify is valid only when tier is \
         explicit Tier1 or Tier2; that node accepts after deterministic checks. Omit gate, or \
         use human, and the node waits for meshloop accept. Tier3 and an omitted tier cannot \
         use verify.\n\
         Example of a valid one-node graph:\n{example}\n\
         No other files unless required to produce that graph. Do not print prose."
    )
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
        output_contract: planning_output_contract(),
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
    use std::cell::{Cell, RefCell};

    use meshloop_domain::decision::DecisionOutcome;
    use meshloop_domain::task_graph::Tier;

    use super::*;
    use crate::ports::{
        BatchNoulQuery, ChoiceQuery, DecisionPort, DecisionPortError, NoulQuery, ScoreQuery,
    };

    fn node(id: u32, desc: &str, deps: &[u32]) -> TaskNode {
        TaskNode {
            id: TaskId(id),
            description: desc.into(),
            depends_on: deps.iter().map(|d| TaskId(*d)).collect(),
            tier: Some(Tier::Tier1),
            allowed_paths: vec![],
            empty_diff_ok: false,
            deliverable: None,
            gate: AcceptanceGate::Human,
            risk_q: None,
            risk_prompt_hash: None,
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
    fn planning_prompt_lists_every_tier_variant_and_task_node_field() {
        let spec = build_planning_spec(
            "o",
            "s",
            AttemptId(1),
            "claude-code",
            "m",
            PathBuf::from("/tmp/wt"),
            Duration::from_secs(300),
        );
        for tier in Tier::ALL {
            assert!(spec.prompt.contains(tier.name()), "missing {tier:?}");
        }
        let value = serde_json::to_value(node(1, "d", &[])).unwrap();
        let fields: Vec<&String> = value.as_object().unwrap().keys().collect();
        assert_eq!(
            fields.len(),
            6,
            "TaskNode gained a field; update the contract"
        );
        for field in fields {
            assert!(
                spec.prompt.contains(&format!("- {field}:")),
                "contract does not describe `{field}`"
            );
        }
        assert!(spec.prompt.contains("[A-Za-z0-9._-]+"));
        assert!(spec.prompt.contains("empty_diff_ok"));
        assert!(spec.prompt.contains("deliverable"));
        assert!(spec.prompt.contains("- gate:"));
        assert!(spec.prompt.contains("\"graph_id\": \"example-graph\""));
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
            None,
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
            None,
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

    #[test]
    fn markdown_architecture_doc_retained_in_context_spec() {
        let graph = TaskGraph {
            graph_id: "g".into(),
            nodes: vec![node(1, "implement markdown ast doc skeleton ADR 0031", &[])],
        };
        let target = &graph.nodes[0];
        let skeletons = vec![
            (
                "docs/architecture/adr/0031-markdown-doc-ast-context-engineering.md".to_string(),
                "# 0031 Markdown AST Context Engineering\n- Status: Accepted\n## Decision\n- Rule 1".to_string(),
            ),
            (
                "crates/meshloop-context/src/doc_skeleton.rs".to_string(),
                "pub fn prune_markdown(source: &str) -> String;".to_string(),
            ),
        ];

        let spec = build_agent_spec_with_context(
            &graph,
            target,
            AttemptId(1),
            "claude-code",
            "configured-model",
            PathBuf::from("/tmp/wt"),
            Duration::from_secs(300),
            &skeletons,
            None,
        );

        assert!(
            spec.prompt
                .contains("0031-markdown-doc-ast-context-engineering.md")
        );
        assert!(
            spec.prompt
                .contains("0031 Markdown AST Context Engineering")
        );
        assert!(spec.prompt.contains("doc_skeleton.rs"));
    }

    fn noul_score(value: f64) -> DecisionOutcome {
        DecisionOutcome::try_score(value).expect("finite test score")
    }

    fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items
            .iter()
            .map(|(path, body)| (path.to_string(), body.to_string()))
            .collect()
    }

    fn apply(
        selected: &[(String, String)],
        prefixes: &[String],
        port: &BatchNoulDouble,
    ) -> (Vec<(String, String)>, bool) {
        super::rerank_unpinned_suffix(selected, "implement the change", prefixes, port)
    }

    fn marker(prompt: &str, path: &str) -> usize {
        let needle = format!("// File: {path}\n");
        match prompt.find(&needle) {
            Some(index) => index,
            None => panic!("prompt missing skeleton for {path}"),
        }
    }

    struct BatchNoulDouble {
        batch: Result<Vec<DecisionOutcome>, DecisionPortError>,
        batch_calls: Cell<usize>,
        noul_calls: Cell<usize>,
        propositions: RefCell<Vec<String>>,
    }

    impl BatchNoulDouble {
        fn new(batch: Result<Vec<DecisionOutcome>, DecisionPortError>) -> Self {
            Self {
                batch,
                batch_calls: Cell::new(0),
                noul_calls: Cell::new(0),
                propositions: RefCell::new(Vec::new()),
            }
        }
    }

    impl DecisionPort for BatchNoulDouble {
        fn evaluate_choice(
            &self,
            _query: &ChoiceQuery,
        ) -> Result<DecisionOutcome, DecisionPortError> {
            Err(DecisionPortError::EmptyQuery)
        }

        fn evaluate_noul(&self, _query: &NoulQuery) -> Result<DecisionOutcome, DecisionPortError> {
            self.noul_calls.set(self.noul_calls.get() + 1);
            Err(DecisionPortError::EmptyQuery)
        }

        fn evaluate_score(
            &self,
            _query: &ScoreQuery,
        ) -> Result<DecisionOutcome, DecisionPortError> {
            Err(DecisionPortError::EmptyQuery)
        }

        fn evaluate_batch_noul(
            &self,
            query: &BatchNoulQuery,
        ) -> Result<Vec<DecisionOutcome>, DecisionPortError> {
            self.batch_calls.set(self.batch_calls.get() + 1);
            *self.propositions.borrow_mut() = query.propositions.clone();
            self.batch.clone()
        }
    }

    #[test]
    fn pinned_index_stays_while_unpinned_suffix_reorders() {
        let prefixes = vec!["pin/".to_string()];
        let selected = pairs(&[("pin/a.rs", "A"), ("u/b.rs", "B"), ("u/c.rs", "C")]);
        let port = BatchNoulDouble::new(Ok(vec![noul_score(0.1), noul_score(0.9)]));
        let (got, accepted) = apply(&selected, &prefixes, &port);
        assert!(accepted);
        assert_eq!(
            got,
            pairs(&[("pin/a.rs", "A"), ("u/c.rs", "C"), ("u/b.rs", "B")])
        );
        assert_eq!(port.batch_calls.get(), 1);
        assert_eq!(port.noul_calls.get(), 0);
        assert_eq!(
            *port.propositions.borrow(),
            vec![
                "implement the change\nu/b.rs".to_string(),
                "implement the change\nu/c.rs".to_string(),
            ]
        );

        let selected = pairs(&[("u/b.rs", "B"), ("pin/a.rs", "A"), ("u/c.rs", "C")]);
        let port = BatchNoulDouble::new(Ok(vec![noul_score(0.1), noul_score(0.9)]));
        let (got, accepted) = apply(&selected, &prefixes, &port);
        assert!(accepted);
        assert_eq!(
            got,
            pairs(&[("u/c.rs", "C"), ("pin/a.rs", "A"), ("u/b.rs", "B")])
        );
        assert_eq!(port.noul_calls.get(), 0);
    }

    #[test]
    fn equal_noul_keeps_fwht_order_by_suffix_index() {
        let prefixes = vec!["pin/".to_string()];
        let selected = pairs(&[("pin/a.rs", "A"), ("u/b.rs", "B"), ("u/c.rs", "C")]);
        let port = BatchNoulDouble::new(Ok(vec![noul_score(0.4), noul_score(0.4)]));
        let (got, accepted) = apply(&selected, &prefixes, &port);
        assert!(accepted);
        assert_eq!(got, selected);

        // Same noul: earlier suffix index wins over path ascending.
        let selected = pairs(&[("u/c.rs", "C"), ("u/b.rs", "B")]);
        let port = BatchNoulDouble::new(Ok(vec![noul_score(0.4), noul_score(0.4)]));
        let (got, accepted) = apply(&selected, &[], &port);
        assert!(accepted);
        assert_eq!(got, selected);
    }

    #[test]
    fn noul_rerank_drops_nothing() {
        let prefixes = vec!["pin/".to_string()];
        let selected = pairs(&[("pin/a.rs", "A"), ("u/b.rs", "B"), ("u/c.rs", "C")]);
        let port = BatchNoulDouble::new(Ok(vec![noul_score(0.1), noul_score(0.9)]));
        let (got, _) = apply(&selected, &prefixes, &port);
        assert_eq!(got.len(), 3);
        assert_eq!(got[0], ("pin/a.rs".to_string(), "A".to_string()));
        assert_eq!(got[1], ("u/c.rs".to_string(), "C".to_string()));
        assert_eq!(got[2], ("u/b.rs".to_string(), "B".to_string()));
    }

    #[test]
    fn unusable_noul_batch_keeps_input_order() {
        let prefixes = vec!["pin/".to_string()];
        let selected = pairs(&[("pin/a.rs", "A"), ("u/b.rs", "B"), ("u/c.rs", "C")]);
        let cases = [
            Err(DecisionPortError::EmptyQuery),
            Ok(vec![noul_score(0.9)]),
            Ok(vec![DecisionOutcome::Boolean(true), noul_score(0.9)]),
            Ok(vec![noul_score(0.1), noul_score(0.9), noul_score(0.5)]),
        ];
        for batch in cases {
            let port = BatchNoulDouble::new(batch);
            let (got, accepted) = apply(&selected, &prefixes, &port);
            assert!(!accepted);
            assert_eq!(got, selected);
            assert_eq!(port.batch_calls.get(), 1);
            assert_eq!(port.noul_calls.get(), 0);
        }
    }

    #[test]
    fn empty_unpinned_suffix_does_not_call_the_port() {
        let prefixes = vec!["pin/".to_string()];
        let selected = pairs(&[("pin/a.rs", "A"), ("pin/b.rs", "B")]);
        let port = BatchNoulDouble::new(Err(DecisionPortError::EmptyQuery));
        let (got, accepted) = apply(&selected, &prefixes, &port);
        assert!(!accepted);
        assert_eq!(got, selected);
        assert_eq!(port.batch_calls.get(), 0);
        assert_eq!(port.noul_calls.get(), 0);
    }

    #[test]
    fn suffix_longer_than_batch_max_does_not_call_the_port() {
        let at_max = (0..super::MAX_UNPINNED_NOUL)
            .map(|i| (format!("u/{i:02}.rs"), format!("body-{i}")))
            .collect::<Vec<_>>();
        let scores = (0..at_max.len()).map(|i| noul_score(i as f64)).collect();
        let port = BatchNoulDouble::new(Ok(scores));
        let (got, accepted) = apply(&at_max, &[], &port);
        assert!(accepted);
        assert_eq!(port.batch_calls.get(), 1);
        assert_eq!(port.noul_calls.get(), 0);
        assert_eq!(
            got[0].0,
            format!("u/{:02}.rs", super::MAX_UNPINNED_NOUL - 1)
        );
        assert_eq!(got.len(), at_max.len());

        let over = (0..=super::MAX_UNPINNED_NOUL)
            .map(|i| (format!("u/{i:02}.rs"), format!("body-{i}")))
            .collect::<Vec<_>>();
        assert_eq!(over.len(), super::MAX_UNPINNED_NOUL + 1);
        let port = BatchNoulDouble::new(Ok(vec![noul_score(1.0); over.len()]));
        let (got, accepted) = apply(&over, &[], &port);
        assert!(!accepted);
        assert_eq!(got, over);
        assert_eq!(port.batch_calls.get(), 0);
        assert_eq!(port.noul_calls.get(), 0);
    }

    #[test]
    fn noul_rerank_changes_prompt_order_and_failure_does_not() {
        let description = "implement Auth trait verify";
        let mut task = node(1, description, &[]);
        task.allowed_paths = vec!["a/pinned.rs".into()];
        let graph = TaskGraph {
            graph_id: "g".into(),
            nodes: vec![task],
        };
        let target = &graph.nodes[0];
        let skeletons = vec![
            (
                "m/auth.rs".into(),
                "pub trait Auth { fn verify(&self); }".into(),
            ),
            (
                "z/noise.rs".into(),
                "pub struct Pool { url: String }".into(),
            ),
            ("a/pinned.rs".into(), "pub struct PinnedKeep;".into()),
        ];
        let ranked = meshloop_context::select_context(
            description,
            &skeletons,
            &target.allowed_paths,
            meshloop_context::DEFAULT_SKELETON_BUDGET,
        );
        assert_eq!(ranked.len(), 3);
        assert_eq!(ranked[0].0, "a/pinned.rs");
        let unpinned: Vec<_> = ranked
            .iter()
            .filter(|(path, _)| !meshloop_context::path_pinned(path, &target.allowed_paths))
            .cloned()
            .collect();
        assert_eq!(unpinned.len(), 2);
        assert_eq!(unpinned[0].0, "m/auth.rs");
        assert_eq!(unpinned[1].0, "z/noise.rs");

        let spec = |noul: Option<&dyn DecisionPort>| {
            build_agent_spec_with_context(
                &graph,
                target,
                AttemptId(1),
                "claude-code",
                "configured-model",
                PathBuf::from("/tmp/wt"),
                Duration::from_secs(300),
                &skeletons,
                noul,
            )
        };
        let none = spec(None);
        let failing = BatchNoulDouble::new(Err(DecisionPortError::EmptyQuery));
        let failed = spec(Some(&failing));
        assert_eq!(failed.prompt, none.prompt);
        assert_eq!(failing.batch_calls.get(), 1);
        assert_eq!(failing.noul_calls.get(), 0);

        let scores = vec![noul_score(0.1), noul_score(0.9)];
        let port = BatchNoulDouble::new(Ok(scores));
        let scored = spec(Some(&port));
        assert_eq!(port.batch_calls.get(), 1);
        assert_eq!(port.noul_calls.get(), 0);
        assert_eq!(
            *port.propositions.borrow(),
            unpinned
                .iter()
                .map(|(path, _)| format!("{description}\n{path}"))
                .collect::<Vec<_>>()
        );
        let later = marker(&scored.prompt, "z/noise.rs");
        let earlier = marker(&scored.prompt, "m/auth.rs");
        let pinned = marker(&scored.prompt, "a/pinned.rs");
        assert!(later < marker(&none.prompt, "z/noise.rs"));
        assert!(pinned < later);
        assert!(pinned < earlier);
        assert!(later < earlier);
        assert!(scored.prompt.contains("pub struct PinnedKeep;"));
        assert!(
            scored
                .prompt
                .contains("pub trait Auth { fn verify(&self); }")
        );
        assert!(scored.prompt.contains("pub struct Pool { url: String }"));
    }
}
