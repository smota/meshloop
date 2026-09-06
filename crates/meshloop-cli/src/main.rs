//! Composition root. `plan` and `run` are the two natural entry points ADR 0015's optional
//! `mesh-loop-planner`/`mesh-loop-executor` skills would wrap — this binary is the sole
//! functional surface either way (ADR 0001).

mod compose;
mod config;
mod report;

use std::collections::HashMap;
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, SystemTime};

use meshloop_domain::evidence::{AttemptId, CandidateRef, DeterministicEvidence, Evidence};
use meshloop_domain::policy::CouplingPenalty;
use meshloop_domain::task_graph::{TaskGraph, Tier};
use meshloop_engine::agent::{build_agent_spec, build_planning_spec};
use meshloop_engine::orchestrator::dispatch_with_fallback;
use meshloop_engine::planner::{DefaultTierAssigner, assign_tiers, decompose};
use meshloop_engine::ports::{EvidenceStore, HarnessCapabilities};
use meshloop_engine::router::{Router, RoutingContext};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => {
            println!("Meshloop: workspace initialized; orchestration is not implemented.");
            ExitCode::SUCCESS
        }
        Some("--help") | Some("-h") => {
            println!(
                "Meshloop\nUsage:\n  meshloop plan --objective \"<text>\" [--config <path>] [--out <path>]\n  meshloop run --plan <path> --accept-plan [--config <path>] [--worktree-base <dir>] [--db <path>]\n  meshloop [--help | --version]"
            );
            ExitCode::SUCCESS
        }
        Some("--version") | Some("-V") => {
            println!("meshloop {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("plan") => cmd_plan(&args[1..]),
        Some("run") => cmd_run(&args[1..]),
        _ => {
            eprintln!("Unsupported command. Run meshloop --help.");
            ExitCode::from(2)
        }
    }
}

fn flag_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn has_flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn default_config_path() -> PathBuf {
    PathBuf::from("config/meshloop.example.toml")
}

fn cmd_plan(args: &[String]) -> ExitCode {
    let Some(objective) = flag_value(args, "--objective") else {
        eprintln!("plan requires --objective \"<text>\"");
        return ExitCode::from(2);
    };
    let config_path = flag_value(args, "--config")
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);
    let Ok(cfg) = config::load(&config_path) else {
        eprintln!("failed to load config at {}", config_path.display());
        return ExitCode::from(2);
    };
    let repo_root = env::current_dir().expect("current directory");
    let composed = match compose::compose(&cfg, repo_root.clone(), None) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };

    let mut profiles = HashMap::new();
    for (name, h) in &composed.harnesses {
        match h.probe() {
            Ok(p) => {
                profiles.insert(name.clone(), p);
            }
            Err(e) => eprintln!("warning: probe failed for {name}: {e:?}"),
        }
    }

    let headroom = HashMap::new();
    let quotas = HashMap::new();
    // Decomposition is always dispatched at a fixed high tier: a bad decomposition is
    // higher-risk than almost any single generated change (ADR 0009/runtime-design.md §5).
    let ctx = RoutingContext {
        task_tier: Tier::Tier3,
        coupling_penalty: CouplingPenalty(0),
        preferred_harness: None,
        headroom: &headroom,
        feedback: &composed.store,
    };
    let selected = Router::default().select(
        &composed.candidates,
        &profiles,
        &quotas,
        SystemTime::now(),
        &ctx,
    );
    let Some(top) = selected.first() else {
        eprintln!("No configured harness is capable of a decomposition dispatch.");
        return ExitCode::from(1);
    };

    let harness_impl = &composed.harnesses[&top.harness];
    let spec = build_planning_spec(
        &objective,
        "scope: this repository only",
        AttemptId(0),
        &top.harness,
        &top.model_ref,
        repo_root,
        Duration::from_secs(cfg.limits.task_timeout_seconds),
    );

    let mut graph: TaskGraph = match decompose(harness_impl, &spec) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("Decomposition failed: {e:?}");
            return ExitCode::from(1);
        }
    };
    assign_tiers(&mut graph, &DefaultTierAssigner);

    println!("{}", report::format_plan(&graph));

    let out_path = flag_value(args, "--out")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("meshloop-plan.json"));
    match serde_json::to_string_pretty(&graph) {
        Ok(json) => {
            if std::fs::write(&out_path, json).is_ok() {
                println!("Plan written to {}", out_path.display());
            }
        }
        Err(e) => eprintln!("failed to serialize plan: {e}"),
    }

    ExitCode::SUCCESS
}

fn cmd_run(args: &[String]) -> ExitCode {
    let Some(plan_path) = flag_value(args, "--plan").map(PathBuf::from) else {
        eprintln!("run requires --plan <file>");
        return ExitCode::from(2);
    };
    if !has_flag(args, "--accept-plan") {
        eprintln!(
            "Plan requires human acceptance before execution (ADR 0009's awaiting-plan-review \
             gate). Re-run with --accept-plan after reviewing {}",
            plan_path.display()
        );
        return ExitCode::from(2);
    }

    let Ok(text) = std::fs::read_to_string(&plan_path) else {
        eprintln!("failed to read plan at {}", plan_path.display());
        return ExitCode::from(2);
    };
    let graph: TaskGraph = match serde_json::from_str(&text) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("malformed plan: {e}");
            return ExitCode::from(2);
        }
    };
    if let Err(e) = graph.validate() {
        eprintln!("plan failed structural validation: {e:?}");
        return ExitCode::from(2);
    }

    let config_path = flag_value(args, "--config")
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);
    let Ok(cfg) = config::load(&config_path) else {
        eprintln!("failed to load config at {}", config_path.display());
        return ExitCode::from(2);
    };
    let repo_root = env::current_dir().expect("current directory");
    let db_path = flag_value(args, "--db").map(PathBuf::from);
    let mut composed = match compose::compose(&cfg, repo_root, db_path.as_deref()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };

    let mut profiles = HashMap::new();
    for (name, h) in &composed.harnesses {
        if let Ok(p) = h.probe() {
            profiles.insert(name.clone(), p);
        }
    }
    let harness_refs: HashMap<String, &dyn HarnessCapabilities> = composed
        .harnesses
        .iter()
        .map(|(k, v)| (k.clone(), v as &dyn HarnessCapabilities))
        .collect();

    let worktree_base = flag_value(args, "--worktree-base")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let quotas = HashMap::new();
    let headroom = HashMap::new();

    println!(
        "Note: this v1 runner dispatches and verifies each node but does not perform git \
         integration/merge-back (ADR 0006) — every worktree is disposed after evidence is \
         recorded, for review, not auto-merged.\n"
    );

    let mut all_ok = true;
    for id in graph.topological_order() {
        let node = graph
            .nodes
            .iter()
            .find(|n| n.id == id)
            .expect("id came from this graph");
        let tier = node.tier.unwrap_or(Tier::Tier2);

        let ctx = RoutingContext {
            task_tier: tier,
            coupling_penalty: CouplingPenalty(0),
            preferred_harness: None,
            headroom: &headroom,
            feedback: &composed.store,
        };
        let selected = Router::default().select(
            &composed.candidates,
            &profiles,
            &quotas,
            SystemTime::now(),
            &ctx,
        );
        if selected.is_empty() {
            print!(
                "{}",
                report::format_node_result(id.0, &report::NodeOutcome::Blocked)
            );
            all_ok = false;
            continue;
        }

        let wt_path = worktree_base.join(format!("meshloop-task-{}", id.0));
        if let Err(e) = composed
            .git
            .add_worktree(&wt_path, &format!("meshloop/task-{}", id.0))
        {
            eprintln!("  [{}] failed to create worktree: {e:?}", id.0);
            all_ok = false;
            continue;
        }

        let attempt_id = AttemptId(id.0);
        let graph_ref = &graph;
        let timeout = Duration::from_secs(cfg.limits.task_timeout_seconds);
        let dispatch = dispatch_with_fallback(
            &harness_refs,
            &selected,
            tier,
            |c| {
                build_agent_spec(
                    graph_ref,
                    node,
                    attempt_id,
                    &c.harness,
                    &c.model_ref,
                    wt_path.clone(),
                    timeout,
                )
            },
            &mut composed.store,
        );

        match dispatch {
            Ok(result) => match result.outcome {
                Some(outcome) => {
                    let candidate_ref = CandidateRef {
                        task_id: id,
                        attempt_id,
                        revision: "worktree-uncommitted".into(),
                    };
                    let _ = composed
                        .store
                        .record(Evidence::Deterministic(DeterministicEvidence {
                            candidate: candidate_ref,
                            tool: "harness-exit-code".into(),
                            tool_version: "n/a".into(),
                            exit_code: outcome.exit_code,
                            output_redacted: outcome.output_redacted.chars().take(500).collect(),
                        }));
                    let harness_used = result.used_harness.clone().unwrap_or_default();
                    let outcome_report = if tier == Tier::Tier3 {
                        report::NodeOutcome::RequiresHumanAcceptance {
                            harness: harness_used,
                        }
                    } else {
                        report::NodeOutcome::Verified {
                            harness: harness_used,
                            exit_code: outcome.exit_code,
                        }
                    };
                    print!("{}", report::format_node_result(id.0, &outcome_report));
                    if outcome.exit_code != 0 {
                        all_ok = false;
                    }
                }
                None => {
                    print!(
                        "{}",
                        report::format_node_result(id.0, &report::NodeOutcome::Blocked)
                    );
                    all_ok = false;
                }
            },
            Err(e) => {
                eprintln!("  [{}] dispatch error: {e:?}", id.0);
                all_ok = false;
            }
        }

        let _ = composed.git.remove_worktree(&wt_path);
    }

    if all_ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
