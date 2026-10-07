//! Composition root. The binary is the engine (ADR 0001); skills/MCP are the operator surface.

#![forbid(unsafe_code)]

mod args;
mod compose;
mod config;
mod context_tools;
mod gitignore;
mod json_out;
mod mcp;
mod report;
mod session_bundle;

use std::collections::HashMap;
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use meshloop_domain::state::{PlanDecision, PlanState, TaskState};
use meshloop_domain::task_graph::{TaskGraph, TaskId};
use meshloop_engine::ports::{HarnessCapabilities, RunStore, WorkspacePort};
use meshloop_engine::router::Router;
use meshloop_engine::run_loop::{OrchestratorError, RunLoop, idle_exit_code};

use crate::args::{Command, Invocation};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match args::parse(&args) {
        Ok(inv) => dispatch(inv),
        Err(e) => {
            if args.iter().any(|a| a == "--json") {
                println!(
                    "{}",
                    json_out::err(
                        "meshloop:error",
                        meshloop_engine::origin::Origin::default(),
                        e
                    )
                );
            } else {
                eprintln!("{e}");
            }
            ExitCode::from(2)
        }
    }
}

fn dispatch(inv: Invocation) -> ExitCode {
    match inv.command {
        Command::Help => {
            if inv.json {
                println!(
                    "{}",
                    json_out::ok(
                        "meshloop:help",
                        inv.origin,
                        serde_json::json!({ "text": args::help_text() }),
                    )
                );
            } else {
                println!("{}", args::help_text());
            }
            ExitCode::SUCCESS
        }
        Command::Version => {
            let v = format!("meshloop {}", env!("CARGO_PKG_VERSION"));
            if inv.json {
                println!(
                    "{}",
                    json_out::ok(
                        "meshloop:version",
                        inv.origin,
                        serde_json::json!({ "version": env!("CARGO_PKG_VERSION") }),
                    )
                );
            } else {
                println!("{v}");
            }
            ExitCode::SUCCESS
        }
        Command::Mcp => ExitCode::from(mcp::run_stdio() as u8),
        Command::Roles => cmd_roles(inv.json, inv.origin),
        Command::Doctor { config, db } => cmd_doctor(config, db, inv.json, inv.origin),
        Command::Status { graph, config, db } => {
            cmd_status(graph, config, db, inv.json, inv.origin)
        }
        Command::Plan {
            objective,
            config,
            out,
            scope,
            db,
            intent_file,
        } => cmd_plan(
            objective,
            config,
            out,
            scope,
            db,
            intent_file,
            inv.json,
            inv.origin,
        ),
        Command::ReviewPlan {
            plan,
            decision,
            reason,
            identity,
            objective,
            intent_file,
            config,
            db,
            out,
            scope,
            fixture_only,
        } => cmd_review_plan(
            plan,
            decision,
            reason,
            identity,
            objective,
            intent_file,
            config,
            db,
            out,
            scope,
            fixture_only,
            inv.json,
            inv.origin,
        ),
        Command::Run {
            plan,
            accept_plan,
            reset,
            config,
            worktree_base,
            db,
            fixture_only,
            detach,
        } => cmd_run(
            plan,
            accept_plan,
            reset,
            config,
            worktree_base,
            db,
            fixture_only,
            detach,
            inv.json,
            inv.origin,
        ),
        Command::Resume {
            graph,
            retry,
            restart,
            config,
            db,
            worktree_base,
            fixture_only,
        } => cmd_resume(
            graph,
            retry,
            restart,
            config,
            db,
            worktree_base,
            fixture_only,
            inv.origin,
        ),
        Command::Cancel {
            graph,
            task,
            session_id,
            config,
            db,
            worktree_base,
        } => cmd_cancel(
            graph,
            task,
            session_id,
            config,
            db,
            worktree_base,
            inv.json,
            inv.origin,
        ),
        Command::Inspect {
            task,
            graph,
            session_id,
            config,
            db,
        } => cmd_inspect(task, graph, session_id, config, db, inv.json, inv.origin),
        Command::Accept {
            task,
            identity,
            graph,
            config,
            db,
            worktree_base,
        } => cmd_accept(task, identity, graph, config, db, worktree_base, inv.origin),
        Command::Integrate {
            graph,
            into,
            accept_integrate,
            config,
            db,
            worktree_base,
        } => cmd_integrate(
            graph,
            into,
            accept_integrate,
            config,
            db,
            worktree_base,
            inv.origin,
        ),
        Command::Orchestrate {
            graph,
            task,
            config,
            db,
            worktree_base,
            model_a,
            model_b,
            fixture_only,
        } => cmd_orchestrate(
            graph,
            task,
            config,
            db,
            worktree_base,
            model_a,
            model_b,
            fixture_only,
            inv.json,
            inv.origin,
        ),
        Command::Bundle { dest, gitignore } => cmd_bundle(dest, gitignore, inv.json, inv.origin),
        Command::AstSkeleton { path } => cmd_ast_skeleton(&path, inv.json, inv.origin),
        Command::SymbolLookup { query, k, scope } => {
            cmd_symbol_lookup(&query, k, scope.as_deref(), inv.json, inv.origin)
        }
        Command::MutatePlan {
            graph,
            mutation_file,
            mutation_json,
            require_review,
            config,
            db,
            worktree_base,
        } => cmd_mutate_plan(
            graph,
            mutation_file,
            mutation_json,
            require_review,
            config,
            db,
            worktree_base,
            inv.json,
            inv.origin,
        ),
    }
}

fn load_cfg(config: Option<PathBuf>) -> Result<(crate::config::Config, PathBuf), ExitCode> {
    let path = match crate::config::discover(config.as_deref()) {
        Ok(p) => p,
        Err(_) => {
            eprintln!("no config found; pass --config or add meshloop.toml");
            return Err(ExitCode::from(2));
        }
    };
    match crate::config::load(&path) {
        Ok(c) => {
            for (name, why) in crate::config::dispatch_warnings(&c) {
                eprintln!("warning: harness '{name}': {why}");
            }
            Ok((c, path))
        }
        Err(e) => {
            eprintln!("failed to load config: {e:?}");
            Err(ExitCode::from(2))
        }
    }
}

fn repo_root() -> PathBuf {
    env::current_dir().expect("current directory")
}

#[allow(clippy::too_many_arguments)]
fn cmd_plan(
    mut objective: String,
    config: Option<PathBuf>,
    out: Option<PathBuf>,
    scope: Option<String>,
    db: Option<PathBuf>,
    intent_file: Option<PathBuf>,
    json: bool,
    origin: meshloop_engine::origin::Origin,
) -> ExitCode {
    if let Some(path) = intent_file {
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                if objective.is_empty() {
                    objective = text;
                } else {
                    objective = format!("{objective}\n\nORIGIN INTENT:\n{text}");
                }
            }
            Err(e) => {
                eprintln!("failed to read --intent-file: {e}");
                return ExitCode::from(2);
            }
        }
    }
    let (cfg, _) = match load_cfg(config) {
        Ok(v) => v,
        Err(c) => return c,
    };
    let root = repo_root();
    let mut composed = match compose::compose(compose::ComposeRequest {
        config: &cfg,
        repo_root: root.clone(),
        db_path: db.as_deref(),
        worktree_base: None,
        origin: origin.clone(),
        fixture_only: false,
    }) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let harness_refs: HashMap<String, &dyn HarnessCapabilities> = composed
        .harnesses
        .iter()
        .map(|(k, v)| (k.clone(), v as &dyn HarnessCapabilities))
        .collect();
    let mut saga = RunLoop {
        harnesses: harness_refs,
        candidates: composed.candidates.clone(),
        workspace: &composed.git,
        store: &mut composed.store,
        processes: &composed.processes,
        checks: &composed.checks,
        router: Router::default(),
        limits: composed.limits,
        fixture_only: false,
        verify_command: composed.verify_command.clone(),
        worktree_base: composed.worktree_base.clone(),
        active_graph: None,
    };
    eprintln!(
        "meshloop:plan probing {} harness(es) in parallel (timeout {}s)",
        cfg.selected_harnesses.len(),
        cfg.limits.probe_timeout_seconds
    );
    let graph = match saga.plan(
        &objective,
        scope.as_deref().unwrap_or("scope: this repository only"),
    ) {
        Ok(g) => g,
        Err(OrchestratorError::NoCandidate(rejections)) => {
            eprintln!("Decomposition failed: no harness can run the planner.");
            for r in &rejections {
                eprintln!("  {}: {}", r.harness, r.reason);
            }
            eprintln!("Run `meshloop doctor --config <path>` for per-harness readiness.");
            return ExitCode::from(1);
        }
        Err(OrchestratorError::Plan(e)) => {
            eprintln!("Decomposition failed: {e}");
            return ExitCode::from(1);
        }
        Err(e) => {
            eprintln!("Decomposition failed: {e:?}");
            return ExitCode::from(1);
        }
    };
    if !json {
        println!("{}", report::format_plan(&graph));
    }
    let out_path = out.unwrap_or_else(|| PathBuf::from("meshloop-plan.json"));
    match serde_json::to_string_pretty(&graph) {
        Ok(plan_json) => {
            if let Err(e) = std::fs::write(&out_path, &plan_json) {
                eprintln!("failed to write plan: {e}");
                return ExitCode::from(1);
            }
            if json {
                let loop_space =
                    std::fs::read_to_string(root.join(".meshloop").join("loop-space.json"))
                        .ok()
                        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                        .unwrap_or(serde_json::Value::Null);
                println!(
                    "{}",
                    json_out::ok(
                        "meshloop:plan",
                        origin,
                        serde_json::json!({
                            "role": "meshloop:planner",
                            "path": out_path,
                            "graph": graph,
                            "loop_space": loop_space,
                            "next": "meshloop:review-plan --accept|--decline|--adjust",
                        }),
                    )
                );
            } else {
                println!("Plan written to {}", out_path.display());
            }
        }
        Err(e) => {
            eprintln!("failed to serialize plan: {e}");
            return ExitCode::from(1);
        }
    }
    ExitCode::SUCCESS
}

#[allow(clippy::too_many_arguments)]
fn cmd_review_plan(
    plan: PathBuf,
    decision: PlanDecision,
    reason: Option<String>,
    identity: Option<String>,
    mut objective: String,
    intent_file: Option<PathBuf>,
    config: Option<PathBuf>,
    db: Option<PathBuf>,
    out: Option<PathBuf>,
    scope: Option<String>,
    fixture_only: bool,
    json: bool,
    origin: meshloop_engine::origin::Origin,
) -> ExitCode {
    if let Some(path) = &intent_file {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                if objective.is_empty() {
                    objective = text;
                } else {
                    objective = format!("{objective}\n\nORIGIN INTENT:\n{text}");
                }
            }
            Err(e) => {
                eprintln!("failed to read --intent-file: {e}");
                return ExitCode::from(2);
            }
        }
    }
    let text = match std::fs::read_to_string(&plan) {
        Ok(t) => t,
        Err(_) => {
            eprintln!("failed to read plan at {}", plan.display());
            return ExitCode::from(2);
        }
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
    let (cfg, _) = match load_cfg(config) {
        Ok(v) => v,
        Err(c) => return c,
    };
    let root = repo_root();
    let mut composed = match compose::compose(compose::ComposeRequest {
        config: &cfg,
        repo_root: root,
        db_path: db.as_deref(),
        worktree_base: None,
        origin: origin.clone(),
        fixture_only,
    }) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let harness_refs: HashMap<String, &dyn HarnessCapabilities> = composed
        .harnesses
        .iter()
        .map(|(k, v)| (k.clone(), v as &dyn HarnessCapabilities))
        .collect();
    let run_base = match composed.git.current_head() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("failed to read HEAD: {e:?}");
            return ExitCode::from(1);
        }
    };
    let mut saga = RunLoop {
        harnesses: harness_refs,
        candidates: composed.candidates.clone(),
        workspace: &composed.git,
        store: &mut composed.store,
        processes: &composed.processes,
        checks: &composed.checks,
        router: Router::default(),
        limits: composed.limits,
        fixture_only,
        verify_command: composed.verify_command.clone(),
        worktree_base: composed.worktree_base.clone(),
        active_graph: None,
    };

    let mut graph = graph;
    if decision == PlanDecision::Adjust {
        let mut prompt = objective;
        if let Some(r) = &reason {
            if prompt.is_empty() {
                prompt = r.clone();
            } else {
                prompt = format!("{prompt}\n\nADJUSTMENT:\n{r}");
            }
        }
        prompt = format!(
            "Revise this Meshloop task graph.\n\
             Current graph_id: {}\n\
             Current nodes: {}\n\
             Adjustment from origin:\n{prompt}\n\
             Write a replacement TaskGraph to meshloop-plan.json.",
            graph.graph_id,
            graph
                .nodes
                .iter()
                .map(|n| format!("[{}] {}", n.id.0, n.description))
                .collect::<Vec<_>>()
                .join("; ")
        );
        match saga.plan(
            &prompt,
            scope.as_deref().unwrap_or("scope: this repository only"),
        ) {
            Ok(g) => graph = g,
            Err(e) => {
                match &e {
                    OrchestratorError::Plan(p) => {
                        eprintln!("Adjust decomposition failed: {p}")
                    }
                    _ => eprintln!("Adjust decomposition failed: {e:?}"),
                }
                return ExitCode::from(1);
            }
        }
        let out_path = out.unwrap_or(plan.clone());
        match serde_json::to_string_pretty(&graph) {
            Ok(plan_json) => {
                if let Err(e) = std::fs::write(&out_path, &plan_json) {
                    eprintln!("failed to write adjusted plan: {e}");
                    return ExitCode::from(1);
                }
            }
            Err(e) => {
                eprintln!("failed to serialize adjusted plan: {e}");
                return ExitCode::from(1);
            }
        }
    }

    let note = match (&identity, &reason) {
        (Some(who), Some(why)) => Some(format!("{who}: {why}")),
        (Some(who), None) => Some(who.clone()),
        (None, Some(why)) => Some(why.clone()),
        (None, None) => None,
    };

    let gid = match saga.stage_plan(graph.clone(), run_base) {
        Ok(id) => id,
        Err(OrchestratorError::PlanAlreadyAccepted(id)) if decision == PlanDecision::Accept => id,
        Err(e) => {
            eprintln!("review-plan stage failed: {e:?}");
            return ExitCode::from(1);
        }
    };
    let state = match saga.decide_plan(&gid, decision, note.clone()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("review-plan decide failed: {e:?}");
            return ExitCode::from(2);
        }
    };

    let next = match state {
        PlanState::PlanAccepted => "meshloop run --plan <file> (already accepted)",
        PlanState::PlanDeclined => "stopped; meshloop:review-plan --adjust to reopen",
        PlanState::AwaitingPlanReview => "meshloop:review-plan --accept|--decline|--adjust",
    };
    if json {
        println!(
            "{}",
            json_out::ok(
                "meshloop:review-plan",
                origin,
                serde_json::json!({
                    "decision": format!("{decision:?}").to_ascii_lowercase(),
                    "plan_state": format!("{state:?}"),
                    "graph_id": gid,
                    "reason": reason,
                    "as": identity,
                    "graph": graph,
                    "next": next,
                }),
            )
        );
    } else {
        println!("meshloop:review-plan {decision:?} → {state:?} (graph {gid})");
        if let Some(n) = &note {
            println!("  note: {n}");
        }
        println!("  next: {next}");
        if decision == PlanDecision::Adjust {
            print!("{}", report::format_plan(&graph));
        }
    }
    ExitCode::SUCCESS
}

#[allow(clippy::too_many_arguments)]
fn cmd_run(
    plan: PathBuf,
    accept_plan: bool,
    reset: bool,
    config: Option<PathBuf>,
    worktree_base: Option<PathBuf>,
    db: Option<PathBuf>,
    fixture_only: bool,
    detach: bool,
    json: bool,
    origin: meshloop_engine::origin::Origin,
) -> ExitCode {
    if origin.is_set() && !fixture_only {
        eprintln!(
            "meshloop:origin is supervisor-only; workers will not use origin session {:?}",
            origin.session
        );
    }
    let text = match std::fs::read_to_string(&plan) {
        Ok(t) => t,
        Err(_) => {
            eprintln!("failed to read plan at {}", plan.display());
            return ExitCode::from(2);
        }
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
    let (cfg, _) = match load_cfg(config.clone()) {
        Ok(v) => v,
        Err(c) => return c,
    };
    let root = repo_root();
    let mut composed = match compose::compose(compose::ComposeRequest {
        config: &cfg,
        repo_root: root,
        db_path: db.as_deref(),
        worktree_base: worktree_base.clone(),
        origin: origin.clone(),
        fixture_only,
    }) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let harness_refs: HashMap<String, &dyn HarnessCapabilities> = composed
        .harnesses
        .iter()
        .map(|(k, v)| (k.clone(), v as &dyn HarnessCapabilities))
        .collect();
    let run_base = match composed.git.current_head() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("failed to read HEAD: {e:?}");
            return ExitCode::from(1);
        }
    };
    let mut saga = RunLoop {
        harnesses: harness_refs,
        candidates: composed.candidates.clone(),
        workspace: &composed.git,
        store: &mut composed.store,
        processes: &composed.processes,
        checks: &composed.checks,
        router: Router::default(),
        limits: composed.limits,
        fixture_only,
        verify_command: composed.verify_command.clone(),
        worktree_base: composed.worktree_base.clone(),
        active_graph: None,
    };
    if json {
        eprintln!(
            "Note: worktrees are kept. `run` does not merge onto your current branch.\n\
             Use `meshloop accept` then `meshloop resume`, and `meshloop integrate --into` to land."
        );
    } else {
        println!(
            "Note: worktrees are kept. `run` does not merge onto your current branch.\n\
             Use `meshloop accept` then `meshloop resume`, and `meshloop integrate --into` to land.\n"
        );
    }
    let graph_id = graph.graph_id.clone();
    let existing_state = saga
        .store
        .load_run(&graph_id)
        .ok()
        .flatten()
        .map(|r| r.plan_state);
    if existing_state == Some(PlanState::PlanDeclined) {
        eprintln!("plan '{graph_id}' was declined; meshloop:review-plan --adjust, then --accept");
        return ExitCode::from(2);
    }
    if !accept_plan && existing_state != Some(PlanState::PlanAccepted) {
        eprintln!(
            "Plan requires human acceptance before execution (ADR 0009). \
             Use meshloop review-plan --accept, or re-run with --accept-plan after reviewing {}",
            plan.display()
        );
        return ExitCode::from(2);
    }
    if let Err(e) = saga.start(graph, run_base) {
        match e {
            OrchestratorError::PlanDeclined(id) => {
                eprintln!("plan '{id}' was declined; meshloop:review-plan --adjust, then --accept");
                return ExitCode::from(2);
            }
            OrchestratorError::DuplicateGraph { graph_id, .. } if reset => {
                if let Err(err) = saga.restart(&graph_id) {
                    eprintln!("run --reset failed: {err:?}");
                    return ExitCode::from(1);
                }
            }
            OrchestratorError::DuplicateGraph { graph_id, resume } if resume => {
                eprintln!(
                    "graph '{graph_id}' already exists and is not terminal; use meshloop resume \
                     (or resume --restart to wipe attempts and re-run this accepted plan)"
                );
                return ExitCode::from(2);
            }
            OrchestratorError::DuplicateGraph { graph_id, .. } => {
                eprintln!(
                    "graph '{graph_id}' already exists (FailedTerminal). \
                     Re-run the accepted plan without replanning: meshloop resume --restart \
                     (or meshloop run --plan … --reset). To start a different plan, change graph_id."
                );
                return ExitCode::from(2);
            }
            other => {
                eprintln!("start failed: {other:?}");
                return ExitCode::from(1);
            }
        }
    }
    let gid = saga
        .active_graph
        .clone()
        .unwrap_or_else(|| graph_id.clone());
    if existing_state != Some(PlanState::PlanAccepted)
        && let Err(e) = saga.accept_plan(&gid)
    {
        eprintln!("accept-plan failed: {e:?}");
        return ExitCode::from(1);
    }
    if detach {
        let current_exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("meshloop"));
        let mut cmd = std::process::Command::new(current_exe);
        cmd.arg("resume").arg("--graph").arg(&gid);
        if let Some(cfg) = &config {
            cmd.arg("--config").arg(cfg);
        }
        if let Some(d) = &db {
            cmd.arg("--db").arg(d);
        }
        if let Some(wb) = &worktree_base {
            cmd.arg("--worktree-base").arg(wb);
        }
        if fixture_only {
            cmd.arg("--fixture-only");
        }
        match meshloop_adapters::process::spawn_detached(cmd) {
            Ok(_) => {
                if json {
                    println!(
                        "{}",
                        json_out::ok(
                            "meshloop:run",
                            origin,
                            serde_json::json!({
                                "session_id": gid,
                                "graph_id": gid,
                                "status": "running",
                                "detached": true,
                            }),
                        )
                    );
                } else {
                    println!("Detached session started: {gid}");
                }
                return ExitCode::SUCCESS;
            }
            Err(e) => {
                if json {
                    println!(
                        "{}",
                        json_out::err(
                            "meshloop:run",
                            origin,
                            format!("Failed to spawn detached process: {e}")
                        )
                    );
                } else {
                    eprintln!("Failed to spawn detached process: {e}");
                }
                return ExitCode::from(1);
            }
        }
    }
    match saga.loop_until_idle() {
        Ok(reason) => {
            let status = saga.status(&gid).ok();
            let git_export = saga.export_git_metadata(&gid).ok().flatten();
            if json {
                let row = saga.store.load_run(&gid).ok().flatten();
                let plan_id = row.as_ref().map(|r| r.plan_id.clone());
                let artifact_digest = row.as_ref().map(|r| {
                    meshloop_domain::digest::ArtifactDigest::sha256(r.plan_json.as_bytes())
                });
                let mut data = serde_json::json!({
                    "idle": format!("{reason:?}"),
                    "graph_id": gid,
                    "plan_id": plan_id,
                    "artifact_digest": artifact_digest,
                });
                if let Some(export) = git_export {
                    data["git_export"] =
                        serde_json::to_value(export).unwrap_or(serde_json::Value::Null);
                }
                println!("{}", json_out::ok("meshloop:run", origin, data,));
            } else {
                println!("{}", report::format_idle(reason));
                if let Some(s) = &status {
                    print!("{}", report::format_status(s));
                }
            }
            let code = status
                .as_ref()
                .map(|s| idle_exit_code(reason, s))
                .unwrap_or(1);
            ExitCode::from(code as u8)
        }
        Err(e) => {
            if json {
                println!(
                    "{}",
                    json_out::err("meshloop:run", origin, format!("{e:?}"))
                );
            } else {
                eprintln!("run failed: {e:?}");
            }
            ExitCode::from(1)
        }
    }
}

fn with_saga(
    config: Option<PathBuf>,
    db: Option<PathBuf>,
    worktree_base: Option<PathBuf>,
    fixture_only: bool,
    origin: meshloop_engine::origin::Origin,
    f: impl FnOnce(&mut RunLoop<'_>) -> ExitCode,
) -> ExitCode {
    let (cfg, _) = match load_cfg(config) {
        Ok(v) => v,
        Err(c) => return c,
    };
    let mut composed = match compose::compose(compose::ComposeRequest {
        config: &cfg,
        repo_root: repo_root(),
        db_path: db.as_deref(),
        worktree_base,
        origin,
        fixture_only,
    }) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let harness_refs: HashMap<String, &dyn HarnessCapabilities> = composed
        .harnesses
        .iter()
        .map(|(k, v)| (k.clone(), v as &dyn HarnessCapabilities))
        .collect();
    let mut saga = RunLoop {
        harnesses: harness_refs,
        candidates: composed.candidates.clone(),
        workspace: &composed.git,
        store: &mut composed.store,
        processes: &composed.processes,
        checks: &composed.checks,
        router: Router::default(),
        limits: composed.limits,
        fixture_only,
        verify_command: composed.verify_command.clone(),
        worktree_base: composed.worktree_base.clone(),
        active_graph: None,
    };
    f(&mut saga)
}

fn cmd_status(
    graph: Option<String>,
    config: Option<PathBuf>,
    db: Option<PathBuf>,
    json: bool,
    origin: meshloop_engine::origin::Origin,
) -> ExitCode {
    if crate::config::discover(config.as_deref()).is_err() {
        let msg = "no config found; pass --config or add meshloop.toml";
        if json {
            println!("{}", json_out::err("meshloop:status", origin, msg));
        } else {
            eprintln!("{msg}");
        }
        return ExitCode::from(2);
    }
    with_saga(config, db, None, false, origin.clone(), |saga| {
        let id = match saga.resolve_graph_id(graph.as_deref()) {
            Ok(id) => id,
            Err(e) => {
                if let Some(g) = graph.as_deref() {
                    let msg = format!("graph '{g}' not found: {e:?}");
                    if json {
                        println!("{}", json_out::err("meshloop:status", origin.clone(), msg));
                    } else {
                        eprintln!("{msg}");
                    }
                    return ExitCode::from(2);
                }
                if json {
                    println!(
                        "{}",
                        json_out::ok(
                            "meshloop:status",
                            origin.clone(),
                            serde_json::json!({ "banner": report::banner(), "runs": [] }),
                        )
                    );
                } else {
                    print!("{}", report::banner());
                }
                return ExitCode::SUCCESS;
            }
        };
        match saga.status(&id) {
            Ok(s) => {
                if json {
                    println!(
                        "{}",
                        json_out::ok(
                            "meshloop:status",
                            origin.clone(),
                            serde_json::json!({
                                "graph_id": s.graph_id,
                                "plan_state": format!("{:?}", s.plan_state),
                                "nodes": s.nodes.iter().map(|n| serde_json::json!({
                                    "id": n.task_id.0,
                                    "state": format!("{:?}", n.state),
                                    "description": n.description,
                                    "pane_id": n.pane_id,
                                    "live": n.live,
                                    "worktree": n.worktree.as_ref().map(|p| p.display().to_string()),
                                    "note": n.note,
                                })).collect::<Vec<_>>(),
                            }),
                        )
                    );
                } else {
                    print!("{}", report::format_status(&s));
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                if json {
                    println!(
                        "{}",
                        json_out::err("meshloop:status", origin.clone(), format!("{e:?}"))
                    );
                } else {
                    eprintln!("{e:?}");
                }
                ExitCode::from(2)
            }
        }
    })
}

#[allow(clippy::too_many_arguments)]
fn cmd_resume(
    graph: Option<String>,
    retry: bool,
    restart: bool,
    config: Option<PathBuf>,
    db: Option<PathBuf>,
    worktree_base: Option<PathBuf>,
    fixture_only: bool,
    origin: meshloop_engine::origin::Origin,
) -> ExitCode {
    with_saga(config, db, worktree_base, fixture_only, origin, |saga| {
        let id = match saga.resolve_graph_id(graph.as_deref()) {
            Ok(id) => id,
            Err(e) => {
                eprintln!("{e:?}");
                return ExitCode::from(2);
            }
        };
        match saga.resume(&id, retry, restart) {
            Ok(reason) => {
                println!("{}", report::format_idle(reason));
                if let Ok(s) = saga.status(&id) {
                    print!("{}", report::format_status(&s));
                    ExitCode::from(idle_exit_code(reason, &s) as u8)
                } else {
                    ExitCode::SUCCESS
                }
            }
            Err(e) => {
                eprintln!("resume failed: {e:?}");
                ExitCode::from(1)
            }
        }
    })
}

#[allow(clippy::too_many_arguments)]
fn cmd_cancel(
    graph: Option<String>,
    task: Option<u32>,
    session_id: Option<String>,
    config: Option<PathBuf>,
    db: Option<PathBuf>,
    worktree_base: Option<PathBuf>,
    json: bool,
    origin: meshloop_engine::origin::Origin,
) -> ExitCode {
    let resolved_id = graph.or(session_id);
    with_saga(config, db, worktree_base, false, origin.clone(), |saga| {
        let id = match saga.resolve_graph_id(resolved_id.as_deref()) {
            Ok(id) => id,
            Err(e) => {
                if json {
                    println!(
                        "{}",
                        json_out::err("meshloop:cancel", origin, format!("{e:?}"))
                    );
                } else {
                    eprintln!("{e:?}");
                }
                return ExitCode::from(2);
            }
        };
        // Kill process trees of any live attempts to prevent orphaned processes
        if let Ok(graph) = saga.graph(&id) {
            for node in &graph.nodes {
                if let Ok(Some(attempt)) = saga.store.latest_attempt_for_task(&id, node.id)
                    && let Some(pid) = attempt.pid
                {
                    meshloop_adapters::process::kill_process_tree(pid);
                }
            }
        }
        let task = task.map(TaskId);
        match saga.cancel(&id, task) {
            Ok(()) => {
                if json {
                    println!(
                        "{}",
                        json_out::ok(
                            "meshloop:cancel",
                            origin,
                            serde_json::json!({
                                "session_id": id,
                                "status": "cancelled",
                            }),
                        )
                    );
                } else {
                    println!("cancelled.");
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                if json {
                    println!(
                        "{}",
                        json_out::err("meshloop:cancel", origin, format!("{e:?}"))
                    );
                } else {
                    eprintln!("{e:?}");
                }
                ExitCode::from(1)
            }
        }
    })
}

fn compute_overall_status(s: &meshloop_engine::run_loop::RunStatus) -> &'static str {
    if s.nodes.is_empty() {
        return "idle";
    }
    if s.nodes.iter().all(|n| n.state == TaskState::Integrated) {
        return "completed";
    }
    if s.nodes.iter().any(|n| n.state == TaskState::Failed) {
        return "failed";
    }
    if s.nodes.iter().any(|n| n.state == TaskState::Cancelled) {
        return "cancelled";
    }
    if s.nodes.iter().any(|n| n.state == TaskState::AwaitingReview) {
        return "awaiting_acceptance";
    }
    "running"
}

#[allow(clippy::too_many_arguments)]
fn cmd_inspect(
    task: Option<u32>,
    graph: Option<String>,
    session_id: Option<String>,
    config: Option<PathBuf>,
    db: Option<PathBuf>,
    json: bool,
    origin: meshloop_engine::origin::Origin,
) -> ExitCode {
    let resolved_id = graph.or(session_id);
    with_saga(config, db, None, false, origin.clone(), |saga| {
        let id = match saga.resolve_graph_id(resolved_id.as_deref()) {
            Ok(id) => id,
            Err(e) => {
                if json {
                    println!(
                        "{}",
                        json_out::err("meshloop:inspect", origin, format!("{e:?}"))
                    );
                } else {
                    eprintln!("{e:?}");
                }
                return ExitCode::from(2);
            }
        };
        match saga.status(&id) {
            Ok(s) => {
                if let Some(t) = task {
                    if let Some(n) = s.nodes.iter().find(|n| n.task_id.0 == t) {
                        if json {
                            println!(
                                "{}",
                                json_out::ok(
                                    "meshloop:inspect",
                                    origin.clone(),
                                    serde_json::json!({
                                        "session_id": id,
                                        "task_id": n.task_id.0,
                                        "state": format!("{:?}", n.state),
                                        "description": n.description,
                                        "worktree": n.worktree.as_ref().map(|p| p.display().to_string()),
                                        "pane_id": n.pane_id,
                                        "live": n.live,
                                        "note": n.note,
                                    }),
                                )
                            );
                        } else {
                            println!(
                                "task {} state={:?} {} worktree={:?}",
                                n.task_id.0, n.state, n.description, n.worktree
                            );
                        }
                        ExitCode::SUCCESS
                    } else {
                        if json {
                            println!(
                                "{}",
                                json_out::err("meshloop:inspect", origin.clone(), "no such task")
                            );
                        } else {
                            eprintln!("no such task");
                        }
                        ExitCode::from(2)
                    }
                } else {
                    let git_export = saga.export_git_metadata(&id).ok().flatten();
                    let overall_status = compute_overall_status(&s);
                    if json {
                        let mut data = serde_json::json!({
                            "session_id": id,
                            "graph_id": s.graph_id,
                            "status": overall_status,
                            "plan_state": format!("{:?}", s.plan_state),
                            "nodes": s.nodes.iter().map(|n| serde_json::json!({
                                "id": n.task_id.0,
                                "state": format!("{:?}", n.state),
                                "description": n.description,
                                "pane_id": n.pane_id,
                                "live": n.live,
                                "worktree": n.worktree.as_ref().map(|p| p.display().to_string()),
                                "note": n.note,
                            })).collect::<Vec<_>>(),
                        });
                        if let Some(export) = git_export {
                            data["git_export"] =
                                serde_json::to_value(export).unwrap_or(serde_json::Value::Null);
                        }
                        println!("{}", json_out::ok("meshloop:inspect", origin.clone(), data));
                    } else {
                        print!("{}", report::format_status(&s));
                    }
                    ExitCode::SUCCESS
                }
            }
            Err(e) => {
                if json {
                    println!(
                        "{}",
                        json_out::err("meshloop:inspect", origin, format!("{e:?}"))
                    );
                } else {
                    eprintln!("{e:?}");
                }
                ExitCode::from(1)
            }
        }
    })
}

fn cmd_accept(
    task: u32,
    identity: String,
    graph: Option<String>,
    config: Option<PathBuf>,
    db: Option<PathBuf>,
    worktree_base: Option<PathBuf>,
    origin: meshloop_engine::origin::Origin,
) -> ExitCode {
    with_saga(config, db, worktree_base, false, origin, |saga| {
        let id = match saga.resolve_graph_id(graph.as_deref()) {
            Ok(id) => id,
            Err(e) => {
                eprintln!("{e:?}");
                return ExitCode::from(2);
            }
        };
        saga.active_graph = Some(id);
        match saga.accept_human(TaskId(task), &identity) {
            Ok(()) => {
                println!("accepted task {task} as {identity}. Run `meshloop resume` to merge.");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e:?}");
                ExitCode::from(1)
            }
        }
    })
}

fn cmd_integrate(
    graph: String,
    into: String,
    accept_integrate: bool,
    config: Option<PathBuf>,
    db: Option<PathBuf>,
    worktree_base: Option<PathBuf>,
    origin: meshloop_engine::origin::Origin,
) -> ExitCode {
    if !accept_integrate {
        eprintln!("integrate requires --accept-integrate after reviewing the integrate worktree");
        return ExitCode::from(2);
    }
    with_saga(config, db, worktree_base, false, origin, |saga| match saga
        .integrate_into(&graph, &into)
    {
        Ok(()) => {
            println!("integrated graph {graph} into {into}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e:?}");
            ExitCode::from(1)
        }
    })
}

fn workspace_root() -> Result<PathBuf, String> {
    env::current_dir().map_err(|e| format!("cannot read current directory: {e}"))
}

fn cmd_ast_skeleton(path: &str, json: bool, origin: meshloop_engine::origin::Origin) -> ExitCode {
    match workspace_root().and_then(|root| context_tools::ast_skeleton(&root, path)) {
        Ok(data) => {
            if json {
                println!("{}", json_out::ok("meshloop:ast-skeleton", origin, data));
            } else {
                eprintln!(
                    "{} ({}): {} -> {} approx tokens, {}% saved",
                    data["path"].as_str().unwrap_or_default(),
                    data["language"].as_str().unwrap_or_default(),
                    data["original_approx_tokens"],
                    data["pruned_approx_tokens"],
                    data["savings_percentage"],
                );
                println!("{}", data["skeleton"].as_str().unwrap_or_default());
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            if json {
                println!("{}", json_out::err("meshloop:ast-skeleton", origin, e));
            } else {
                eprintln!("{e}");
            }
            ExitCode::from(2)
        }
    }
}

fn cmd_symbol_lookup(
    query: &str,
    k: usize,
    scope: Option<&str>,
    json: bool,
    origin: meshloop_engine::origin::Origin,
) -> ExitCode {
    match workspace_root().and_then(|root| context_tools::symbol_lookup(&root, query, k, scope)) {
        Ok(data) => {
            if json {
                println!("{}", json_out::ok("meshloop:symbol-lookup", origin, data));
            } else {
                println!(
                    "indexed {} files ({} entries) in {} us; search {} us",
                    data["files_indexed"],
                    data["entries_indexed"],
                    data["index_build_us"],
                    data["search_us"],
                );
                for hit in data["hits"].as_array().into_iter().flatten() {
                    println!(
                        "{:>8} {} {} [{}]",
                        hit["score"],
                        hit["path"].as_str().unwrap_or_default(),
                        hit["name"].as_str().unwrap_or_default(),
                        hit["kind"].as_str().unwrap_or_default(),
                    );
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            if json {
                println!("{}", json_out::err("meshloop:symbol-lookup", origin, e));
            } else {
                eprintln!("{e}");
            }
            ExitCode::from(2)
        }
    }
}

fn cmd_bundle(
    dest: PathBuf,
    gitignore: bool,
    json: bool,
    origin: meshloop_engine::origin::Origin,
) -> ExitCode {
    let written = session_bundle::write_to(&dest)
        .and_then(|files| gitignore::bundle_check(&dest, gitignore).map(|g| (files, g)));
    match written {
        Ok((files, ignore)) => {
            if json {
                let paths: Vec<String> = files
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                println!(
                    "{}",
                    json_out::ok(
                        "meshloop:bundle",
                        origin,
                        serde_json::json!({
                            "dest": dest.to_string_lossy(),
                            "version": env!("CARGO_PKG_VERSION"),
                            "files": paths,
                            "gitignore": ignore.as_str(),
                            "gitignore_line": gitignore::IGNORE_LINE,
                        }),
                    )
                );
            } else {
                println!("bundled to {}", dest.display());
                println!("gitignore: {}", ignore.as_str());
                if ignore == gitignore::BundleIgnore::Missing {
                    eprintln!(
                        "add this line to {}/.gitignore (or rerun with --gitignore): {}",
                        dest.display(),
                        gitignore::IGNORE_LINE
                    );
                }
            }
            if json && ignore == gitignore::BundleIgnore::Missing {
                eprintln!(
                    "gitignore: missing; add `{}` to .gitignore (or rerun with --gitignore)",
                    gitignore::IGNORE_LINE
                );
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            if json {
                println!("{}", json_out::err("meshloop:bundle", origin, e));
            } else {
                eprintln!("{e}");
            }
            ExitCode::from(1)
        }
    }
}

fn cmd_roles(json: bool, origin: meshloop_engine::origin::Origin) -> ExitCode {
    let roles = meshloop_domain::role::bundled_roles();
    if json {
        println!(
            "{}",
            json_out::ok(
                "meshloop:roles",
                origin,
                serde_json::to_value(&roles).unwrap_or_default(),
            )
        );
    } else {
        println!("Bundled visible definitions (project > user > bundled):");
        println!("name | kind | spawn | source | responsibility");
        for r in &roles {
            println!(
                "{} | {:?} | {:?} | {} | {}",
                r.id.as_str(),
                r.kind,
                r.spawn,
                r.source,
                r.responsibility
            );
        }
        println!("Slash: /meshloop:plan  MCP: meshloop_plan");
    }
    ExitCode::SUCCESS
}

fn cmd_doctor(
    config: Option<PathBuf>,
    db: Option<PathBuf>,
    json: bool,
    origin: meshloop_engine::origin::Origin,
) -> ExitCode {
    let origin_session = origin.session.clone();
    let origin_harness = origin.harness.clone();

    let (config_path, harnesses, error) = match crate::config::discover(config.as_deref()) {
        Err(_) => (
            None,
            Vec::new(),
            Some("no config found; pass --config or add meshloop.toml".to_string()),
        ),
        Ok(path) => match crate::config::load(&path) {
            Err(e) => (
                Some(path),
                Vec::new(),
                Some(format!("failed to load config: {e:?}")),
            ),
            Ok(cfg) => {
                let rows = doctor_probe(&cfg);
                let error = (!rows.iter().any(|r| r["dispatchable"] == true)).then(|| {
                    "no selected harness is dispatchable; see data.harnesses[].reason".to_string()
                });
                (Some(path), rows, error)
            }
        },
    };
    let harnesses_ready = error.is_none();
    let store_ignored =
        gitignore::store_ignored(&db.unwrap_or_else(|| compose::default_db(&repo_root())));

    let data = serde_json::json!({
        "daemonless": true,
        "origin_session": origin_session,
        "origin_harness": origin_harness,
        "live_transport": "direct-cli",
        "live_default": true,
        "fixture_transport": "subprocess",
        "fixture_is": "ci-double",
        "namespace": "meshloop:",
        "supervisor_only": true,
        "kinds": ["claude", "codex", "pi", "grok", "agy"],
        "config": config_path.as_ref().map(|p| p.display().to_string()),
        "harnesses_ready": harnesses_ready,
        "harnesses": harnesses,
        "store_ignored": store_ignored,
        "note": "Doctor loads the config and runs each selected harness's bounded version probe. Live workers run as direct CLI subprocesses in Git worktrees.",
    });
    if json {
        println!(
            "{}",
            json_out::report("meshloop:doctor", origin, error.clone(), data)
        );
    } else {
        println!("meshloop:doctor");
        println!("  daemonless: true");
        println!("  origin session: {origin_session:?}");
        println!("  origin harness: {origin_harness:?}");
        println!("  live transport: direct-cli (default) | fixture: CI subprocess");
        println!("  namespace: meshloop: | origin: supervisor-only");
        match &config_path {
            Some(p) => println!("  config: {}", p.display()),
            None => println!("  config: (none)"),
        }
        for h in &harnesses {
            let status = if h["dispatchable"] == true {
                "ready".to_string()
            } else {
                format!("not ready: {}", h["reason"].as_str().unwrap_or("unknown"))
            };
            println!("  harness {}: {status}", h["name"].as_str().unwrap_or("?"));
        }
        println!("  store_ignored: {store_ignored}");
        if !store_ignored {
            println!(
                "  fix: add `{}` to .gitignore, or run `meshloop bundle --dest . --gitignore`",
                gitignore::IGNORE_LINE
            );
        }
        if let Some(e) = &error {
            println!("  error: {e}");
        }
    }
    if harnesses_ready {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

/// Runs every selected harness's bounded probe concurrently and reports per-harness
/// readiness in configuration order. Opens no store and dispatches nothing.
fn doctor_probe(cfg: &crate::config::Config) -> Vec<serde_json::Value> {
    use meshloop_domain::capability::Compatibility;
    let built: Vec<(String, meshloop_adapters::harness::CliHarness)> = cfg
        .selected_harnesses
        .iter()
        .filter_map(|name| {
            let hc = cfg.harnesses.get(name)?;
            Some((name.clone(), compose::cli_harness(name, hc, &cfg.limits)))
        })
        .collect();
    std::thread::scope(|scope| {
        let handles: Vec<_> = built
            .iter()
            .map(|(name, h)| (name, scope.spawn(move || h.probe())))
            .collect();
        handles
            .into_iter()
            .map(|(name, handle)| {
                let executable = &cfg.harnesses[name].executable;
                match handle.join() {
                    Ok(Ok(p)) => serde_json::json!({
                        "name": name,
                        "executable": executable,
                        "version": p.version,
                        "compatible": p.compatibility != Compatibility::Unsupported,
                        "dispatchable": p.is_dispatchable(),
                        "reason": p.rejection_reason(),
                    }),
                    Ok(Err(e)) => serde_json::json!({
                        "name": name,
                        "executable": executable,
                        "compatible": false,
                        "dispatchable": false,
                        "reason": format!("probe failed: {e:?}"),
                    }),
                    Err(_) => serde_json::json!({
                        "name": name,
                        "executable": executable,
                        "compatible": false,
                        "dispatchable": false,
                        "reason": "probe panicked",
                    }),
                }
            })
            .collect()
    })
}

#[allow(clippy::too_many_arguments)]
fn cmd_orchestrate(
    graph: Option<String>,
    task: u32,
    config: Option<PathBuf>,
    db: Option<PathBuf>,
    worktree_base: Option<PathBuf>,
    model_a: String,
    model_b: String,
    fixture_only: bool,
    json: bool,
    origin: meshloop_engine::origin::Origin,
) -> ExitCode {
    use meshloop_engine::orchestrate::{
        PinnedEvidence, WaveResult, matrix_only, persist_reviews, pin_attempt, plan_review,
        write_pack,
    };

    let live = !fixture_only && origin.session.is_some();
    if !fixture_only && origin.session.is_none() {
        eprintln!(
            "note: meshloop:orchestrate is matrix-only without --origin-session (or MESHLOOP_ORIGIN_SESSION); live reviewers will not start"
        );
    }

    let mut composed = match load_cfg(config) {
        Ok((cfg, _)) => compose::compose(compose::ComposeRequest {
            config: &cfg,
            repo_root: repo_root(),
            db_path: db.as_deref(),
            worktree_base,
            origin: origin.clone(),
            fixture_only,
        })
        .ok(),
        Err(_) => None,
    };

    let pinned = composed.as_mut().and_then(|c| {
        let id = graph
            .clone()
            .or_else(|| c.store.latest_run().ok().flatten().map(|r| r.graph_id))?;
        pin_attempt(&c.store, &c.git, &id, TaskId(task)).ok()
    });

    if live && pinned.is_none() {
        let msg =
            "live meshloop:orchestrate needs a pinned attempt (node must reach AwaitingReview)";
        if json {
            println!("{}", json_out::err("meshloop:orchestrate", origin, msg));
        } else {
            eprintln!("{msg}");
        }
        return ExitCode::from(2);
    }

    let evidence = pinned.unwrap_or(PinnedEvidence {
        graph_id: graph.clone().unwrap_or_else(|| "unknown".into()),
        task_id: TaskId(task),
        attempt_id: meshloop_domain::evidence::AttemptId(0),
        revision: "unresolved".into(),
        base: "unresolved".into(),
        head: "unresolved".into(),
        files: vec![],
        stat_redacted: String::new(),
        description: String::new(),
        worktree: None,
        unified_diff: String::new(),
    });

    let pack_dir = write_pack(&repo_root(), &evidence).ok();
    match plan_review(
        origin.clone(),
        evidence.clone(),
        &model_a,
        &model_b,
        live,
        pack_dir.clone(),
    ) {
        Ok(plan) => {
            let wave: WaveResult = matrix_only(plan);
            if let Some(c) = composed.as_mut()
                && wave.live_executed
            {
                let _ = persist_reviews(
                    &mut c.store,
                    &evidence,
                    &wave.reports,
                    wave.synthesis_status,
                    &wave.synthesis,
                );
            }
            if json {
                println!(
                    "{}",
                    json_out::ok(
                        "meshloop:orchestrate",
                        origin,
                        serde_json::to_value(&wave).unwrap_or_default(),
                    )
                );
            } else {
                println!("meshloop:orchestrate");
                if let Some(p) = &wave.plan.pack_dir {
                    println!("  pack: {}", p.display());
                }
                println!("name | role | model | kind | pane | lens");
                for a in &wave.plan.assignments {
                    println!(
                        "{} | {} | {} | {:?} | {:?} | {}",
                        a.name, a.role, a.model_ref, a.kind, a.pane_id, a.lens
                    );
                }
                println!(
                    "synthesis ({:?}): {}",
                    wave.synthesis_status, wave.synthesis
                );
                println!("{}", wave.plan.trust_boundary);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            if json {
                println!("{}", json_out::err("meshloop:orchestrate", origin, e));
            } else {
                eprintln!("{e}");
            }
            ExitCode::from(2)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn cmd_mutate_plan(
    graph_arg: Option<String>,
    mutation_file: Option<PathBuf>,
    mutation_json: Option<String>,
    require_review: bool,
    config: Option<PathBuf>,
    db: Option<PathBuf>,
    worktree_base: Option<PathBuf>,
    json: bool,
    origin: meshloop_engine::origin::Origin,
) -> ExitCode {
    let payload = match (mutation_json, mutation_file) {
        (Some(s), _) => s,
        (None, Some(f)) => match std::fs::read_to_string(&f) {
            Ok(s) => s,
            Err(e) => {
                let err = format!("failed to read mutation file {}: {e}", f.display());
                if json {
                    println!("{}", json_out::err("meshloop:mutate-plan", origin, err));
                } else {
                    eprintln!("{err}");
                }
                return ExitCode::from(2);
            }
        },
        (None, None) => {
            let err = "meshloop:mutate-plan requires --mutation <json> or --mutation-file <path>";
            if json {
                println!("{}", json_out::err("meshloop:mutate-plan", origin, err));
            } else {
                eprintln!("{err}");
            }
            return ExitCode::from(2);
        }
    };

    let mutation: meshloop_domain::task_graph::GraphMutation = match serde_json::from_str(&payload)
    {
        Ok(m) => m,
        Err(e) => {
            let err = format!("invalid mutation JSON: {e}");
            if json {
                println!("{}", json_out::err("meshloop:mutate-plan", origin, err));
            } else {
                eprintln!("{err}");
            }
            return ExitCode::from(2);
        }
    };

    let (cfg, _) = match load_cfg(config) {
        Ok(v) => v,
        Err(c) => return c,
    };
    let root = repo_root();
    let mut composed = match compose::compose(compose::ComposeRequest {
        config: &cfg,
        repo_root: root,
        db_path: db.as_deref(),
        worktree_base,
        origin: origin.clone(),
        fixture_only: false,
    }) {
        Ok(c) => c,
        Err(e) => {
            if json {
                println!("{}", json_out::err("meshloop:mutate-plan", origin, e));
            } else {
                eprintln!("{e}");
            }
            return ExitCode::from(2);
        }
    };

    let harness_refs: HashMap<String, &dyn HarnessCapabilities> = composed
        .harnesses
        .iter()
        .map(|(k, v)| (k.clone(), v as &dyn HarnessCapabilities))
        .collect();
    let mut saga = RunLoop {
        harnesses: harness_refs,
        candidates: composed.candidates.clone(),
        workspace: &composed.git,
        store: &mut composed.store,
        processes: &composed.processes,
        checks: &composed.checks,
        router: Router::default(),
        limits: composed.limits,
        fixture_only: false,
        verify_command: composed.verify_command.clone(),
        worktree_base: composed.worktree_base.clone(),
        active_graph: None,
    };

    let graph_id = match graph_arg {
        Some(g) => g,
        None => match saga.store.latest_run() {
            Ok(Some(r)) => r.graph_id,
            Ok(None) => {
                let err = "no runs found; specify --graph <id>";
                if json {
                    println!("{}", json_out::err("meshloop:mutate-plan", origin, err));
                } else {
                    eprintln!("{err}");
                }
                return ExitCode::from(2);
            }
            Err(e) => {
                let err = format!("store error: {e:?}");
                if json {
                    println!("{}", json_out::err("meshloop:mutate-plan", origin, err));
                } else {
                    eprintln!("{err}");
                }
                return ExitCode::from(2);
            }
        },
    };

    let updated_graph = match saga.mutate_plan(&graph_id, mutation, require_review) {
        Ok(g) => g,
        Err(e) => {
            let err = format!("mutation failed: {e:?}");
            if json {
                println!("{}", json_out::err("meshloop:mutate-plan", origin, err));
            } else {
                eprintln!("{err}");
            }
            return ExitCode::from(1);
        }
    };

    let topo_order = updated_graph
        .try_topological_order()
        .map(|order| order.into_iter().map(|t| t.0).collect::<Vec<_>>())
        .unwrap_or_default();
    let row = saga.store.load_run(&graph_id).ok().flatten();
    let plan_state = row.as_ref().map(|r| format!("{:?}", r.plan_state));
    let plan_id = row.as_ref().map(|r| r.plan_id.clone());
    let artifact_digest = row
        .as_ref()
        .map(|r| meshloop_domain::digest::ArtifactDigest::sha256(r.plan_json.as_bytes()));

    if json {
        println!(
            "{}",
            json_out::ok(
                "meshloop:mutate-plan",
                origin,
                serde_json::json!({
                    "graph_id": graph_id,
                    "require_review": require_review,
                    "nodes_count": updated_graph.nodes.len(),
                    "topological_order": topo_order,
                    "plan_state": plan_state,
                    "plan_id": plan_id,
                    "artifact_digest": artifact_digest,
                    "graph": updated_graph,
                }),
            )
        );
    } else {
        println!(
            "meshloop:mutate-plan succeeded for graph {graph_id} ({} nodes, require_review={require_review})",
            updated_graph.nodes.len()
        );
    }
    ExitCode::SUCCESS
}
