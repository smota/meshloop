//! Composition root. The binary is the engine (ADR 0001); skills/MCP are the operator surface.

mod args;
mod compose;
mod config;
mod json_out;
mod mcp;
mod report;
mod session_bundle;

use std::collections::HashMap;
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use meshloop_domain::state::{PlanDecision, PlanState};
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
        Command::Doctor => cmd_doctor(inv.json, inv.origin),
        Command::Status { graph } => cmd_status(graph, inv.json, inv.origin),
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
        } => cmd_run(
            plan,
            accept_plan,
            reset,
            config,
            worktree_base,
            db,
            fixture_only,
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
            config,
            db,
            worktree_base,
        } => cmd_cancel(graph, task, config, db, worktree_base, inv.origin),
        Command::Inspect {
            task,
            graph,
            config,
            db,
        } => cmd_inspect(task, graph, config, db, inv.origin),
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
        Command::Bundle { dest } => cmd_bundle(dest, inv.json, inv.origin),
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
        Ok(c) => Ok((c, path)),
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
        require_herdr: true,
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
    let graph = match saga.plan(
        &objective,
        scope.as_deref().unwrap_or("scope: this repository only"),
    ) {
        Ok(g) => g,
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
        require_herdr: decision == PlanDecision::Adjust && !fixture_only,
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
                eprintln!("Adjust decomposition failed: {e:?}");
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
        fixture_only,
        require_herdr: !fixture_only,
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
    println!(
        "Note: worktrees are kept. `run` does not merge onto your current branch.\n\
         Use `meshloop accept` then `meshloop resume`, and `meshloop integrate --into` to land.\n"
    );
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
    match saga.loop_until_idle() {
        Ok(reason) => {
            let status = saga.status(&gid).ok();
            if json {
                println!(
                    "{}",
                    json_out::ok(
                        "meshloop:run",
                        origin,
                        serde_json::json!({
                            "idle": format!("{reason:?}"),
                            "graph_id": gid,
                        }),
                    )
                );
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
    require_herdr: bool,
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
        require_herdr,
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
    json: bool,
    origin: meshloop_engine::origin::Origin,
) -> ExitCode {
    if crate::config::discover(None).is_err() {
        if json {
            println!(
                "{}",
                json_out::ok(
                    "meshloop:status",
                    origin,
                    serde_json::json!({ "banner": report::banner(), "runs": [] }),
                )
            );
        } else {
            print!("{}", report::banner());
        }
        return ExitCode::SUCCESS;
    }
    let db = None;
    with_saga(None, db, None, false, false, origin.clone(), |saga| {
        let id = match saga.resolve_graph_id(graph.as_deref()) {
            Ok(id) => id,
            Err(_) => {
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
                eprintln!("{e:?}");
                ExitCode::from(1)
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
    with_saga(
        config,
        db,
        worktree_base,
        fixture_only,
        !fixture_only,
        origin,
        |saga| {
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
        },
    )
}

fn cmd_cancel(
    graph: Option<String>,
    task: Option<u32>,
    config: Option<PathBuf>,
    db: Option<PathBuf>,
    worktree_base: Option<PathBuf>,
    origin: meshloop_engine::origin::Origin,
) -> ExitCode {
    with_saga(config, db, worktree_base, false, false, origin, |saga| {
        let id = match saga.resolve_graph_id(graph.as_deref()) {
            Ok(id) => id,
            Err(e) => {
                eprintln!("{e:?}");
                return ExitCode::from(2);
            }
        };
        let task = task.map(TaskId);
        match saga.cancel(&id, task) {
            Ok(()) => {
                println!("cancelled.");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e:?}");
                ExitCode::from(1)
            }
        }
    })
}

fn cmd_inspect(
    task: u32,
    graph: Option<String>,
    config: Option<PathBuf>,
    db: Option<PathBuf>,
    origin: meshloop_engine::origin::Origin,
) -> ExitCode {
    with_saga(config, db, None, false, false, origin, |saga| {
        let id = match saga.resolve_graph_id(graph.as_deref()) {
            Ok(id) => id,
            Err(e) => {
                eprintln!("{e:?}");
                return ExitCode::from(2);
            }
        };
        match saga.status(&id) {
            Ok(s) => {
                if let Some(n) = s.nodes.iter().find(|n| n.task_id.0 == task) {
                    println!(
                        "task {} state={:?} {} worktree={:?}",
                        n.task_id.0, n.state, n.description, n.worktree
                    );
                    ExitCode::SUCCESS
                } else {
                    eprintln!("no such task");
                    ExitCode::from(2)
                }
            }
            Err(e) => {
                eprintln!("{e:?}");
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
    with_saga(config, db, worktree_base, false, false, origin, |saga| {
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
    with_saga(
        config,
        db,
        worktree_base,
        false,
        false,
        origin,
        |saga| match saga.integrate_into(&graph, &into) {
            Ok(()) => {
                println!("integrated graph {graph} into {into}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e:?}");
                ExitCode::from(1)
            }
        },
    )
}

fn cmd_bundle(dest: PathBuf, json: bool, origin: meshloop_engine::origin::Origin) -> ExitCode {
    match session_bundle::write_to(&dest) {
        Ok(files) => {
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
                        }),
                    )
                );
            } else {
                println!("bundled to {}", dest.display());
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

fn cmd_doctor(json: bool, origin: meshloop_engine::origin::Origin) -> ExitCode {
    let herdr = meshloop_adapters::herdr::HerdrCliAdapter::new(compose::herdr_bin());
    let doctor = herdr.probe_status();
    let (running, version, err) = match &doctor {
        Ok(d) => (d.server_running, d.version.clone(), None),
        Err(e) => (false, None, Some(format!("{e:?}"))),
    };
    let pane = herdr.current_pane_id().ok().flatten();
    let origin_session = origin.session.clone().or(pane);
    let origin_harness = origin.harness.clone();
    let data = serde_json::json!({
        "herdr_server_running": running,
        "herdr_version": version,
        "herdr_error": err,
        "origin_session": origin_session,
        "origin_harness": origin_harness,
        "live_transport": "herdr",
        "live_default": true,
        "fixture_transport": "subprocess",
        "fixture_is": "ci-double",
        "namespace": "meshloop:",
        "supervisor_only": true,
        "kinds": ["claude", "codex", "pi", "grok", "agy"],
        "note": "Doctor does not split panes. Live workers use Herdr; never split origin_session. --fixture-only is the CI double.",
    });
    if json {
        println!("{}", json_out::ok("meshloop:doctor", origin, data));
    } else {
        println!("meshloop:doctor");
        println!("  herdr running: {running}");
        println!("  herdr version: {version:?}");
        println!("  origin session: {origin_session:?}");
        println!("  origin harness: {origin_harness:?}");
        if let Some(e) = err {
            println!("  herdr error: {e}");
        }
        println!("  live transport: herdr (default) | fixture: CI subprocess");
        println!("  namespace: meshloop: | origin: supervisor-only");
    }
    ExitCode::SUCCESS
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
    use meshloop_adapters::herdr::HerdrCliAdapter;
    use meshloop_engine::orchestrate::{
        PinnedEvidence, WaveResult, execute_wave, matrix_only, persist_reviews, pin_attempt,
        plan_review, write_pack,
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
            require_herdr: live,
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
            let wave: WaveResult = if live {
                let herdr = HerdrCliAdapter::new(std::path::PathBuf::from("herdr"));
                match herdr.probe_status() {
                    Ok(d) if d.server_running => {}
                    other => {
                        let msg = format!("herdr not ready for live orchestrate: {other:?}");
                        if json {
                            println!("{}", json_out::err("meshloop:orchestrate", origin, msg));
                        } else {
                            eprintln!("{msg}");
                        }
                        return ExitCode::from(2);
                    }
                }
                let pack = pack_dir.clone().unwrap_or_else(repo_root);
                match execute_wave(&herdr, plan, &pack, &repo_root(), 120_000) {
                    Ok(w) => w,
                    Err(e) => {
                        if json {
                            println!("{}", json_out::err("meshloop:orchestrate", origin, e));
                        } else {
                            eprintln!("{e}");
                        }
                        return ExitCode::from(1);
                    }
                }
            } else {
                matrix_only(plan)
            };
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
