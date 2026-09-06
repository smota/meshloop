//! Composition root. This binary is the sole functional surface (ADR 0001).

mod args;
mod compose;
mod config;
mod json_out;
mod mcp;
mod report;

use std::collections::HashMap;
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

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
        Command::Run {
            plan,
            accept_plan,
            config,
            worktree_base,
            db,
            allow_live_harness,
        } => cmd_run(
            plan,
            accept_plan,
            config,
            worktree_base,
            db,
            allow_live_harness,
            inv.json,
            inv.origin,
        ),
        Command::Resume {
            graph,
            retry,
            config,
            db,
            worktree_base,
            allow_live_harness,
        } => cmd_resume(graph, retry, config, db, worktree_base, allow_live_harness),
        Command::Cancel {
            graph,
            task,
            config,
            db,
            worktree_base,
        } => cmd_cancel(graph, task, config, db, worktree_base),
        Command::Inspect {
            task,
            graph,
            config,
            db,
        } => cmd_inspect(task, graph, config, db),
        Command::Accept {
            task,
            identity,
            graph,
            config,
            db,
            worktree_base,
        } => cmd_accept(task, identity, graph, config, db, worktree_base),
        Command::Integrate {
            graph,
            into,
            accept_integrate,
            config,
            db,
            worktree_base,
        } => cmd_integrate(graph, into, accept_integrate, config, db, worktree_base),
        Command::Orchestrate {
            graph,
            task,
            config,
            db,
            worktree_base,
            model_a,
            model_b,
            allow_live_harness,
        } => cmd_orchestrate(
            graph,
            task,
            config,
            db,
            worktree_base,
            model_a,
            model_b,
            allow_live_harness,
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
    let mut composed = match compose::compose(&cfg, root.clone(), db.as_deref(), None) {
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
        allow_live_harness: false,
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
                println!(
                    "{}",
                    json_out::ok(
                        "meshloop:plan",
                        origin,
                        serde_json::json!({
                            "role": "meshloop:planner",
                            "path": out_path,
                            "graph": graph,
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
fn cmd_run(
    plan: PathBuf,
    accept_plan: bool,
    config: Option<PathBuf>,
    worktree_base: Option<PathBuf>,
    db: Option<PathBuf>,
    allow_live_harness: bool,
    json: bool,
    origin: meshloop_engine::origin::Origin,
) -> ExitCode {
    if origin.is_set() && allow_live_harness {
        eprintln!(
            "meshloop:origin is supervisor-only; workers will not use origin session {:?}",
            origin.session
        );
    }
    if !accept_plan {
        eprintln!(
            "Plan requires human acceptance before execution (ADR 0009). \
             Re-run with --accept-plan after reviewing {}",
            plan.display()
        );
        return ExitCode::from(2);
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
    let mut composed = match compose::compose(&cfg, root, db.as_deref(), worktree_base) {
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
        allow_live_harness,
        verify_command: composed.verify_command.clone(),
        worktree_base: composed.worktree_base.clone(),
        active_graph: None,
    };
    println!(
        "Note: worktrees are kept. `run` does not merge onto your current branch.\n\
         Use `meshloop accept` then `meshloop resume`, and `meshloop integrate --into` to land.\n"
    );
    if let Err(e) = saga.start(graph, run_base) {
        match e {
            OrchestratorError::DuplicateGraph { graph_id, resume } if resume => {
                eprintln!(
                    "graph '{graph_id}' already exists and is not terminal; use meshloop resume"
                );
                return ExitCode::from(2);
            }
            OrchestratorError::DuplicateGraph { graph_id, .. } => {
                eprintln!("graph '{graph_id}' already exists; choose a new graph_id");
                return ExitCode::from(2);
            }
            other => {
                eprintln!("start failed: {other:?}");
                return ExitCode::from(1);
            }
        }
    }
    let gid = saga.active_graph.clone().unwrap_or_default();
    if let Err(e) = saga.accept_plan(&gid) {
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
    allow_live: bool,
    f: impl FnOnce(&mut RunLoop<'_>) -> ExitCode,
) -> ExitCode {
    let (cfg, _) = match load_cfg(config) {
        Ok(v) => v,
        Err(c) => return c,
    };
    let mut composed = match compose::compose(&cfg, repo_root(), db.as_deref(), worktree_base) {
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
        allow_live_harness: allow_live,
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
    with_saga(None, db, None, false, |saga| {
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

fn cmd_resume(
    graph: Option<String>,
    retry: bool,
    config: Option<PathBuf>,
    db: Option<PathBuf>,
    worktree_base: Option<PathBuf>,
    allow_live: bool,
) -> ExitCode {
    with_saga(config, db, worktree_base, allow_live, |saga| {
        let id = match saga.resolve_graph_id(graph.as_deref()) {
            Ok(id) => id,
            Err(e) => {
                eprintln!("{e:?}");
                return ExitCode::from(2);
            }
        };
        match saga.resume(&id, retry) {
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

fn cmd_cancel(
    graph: Option<String>,
    task: Option<u32>,
    config: Option<PathBuf>,
    db: Option<PathBuf>,
    worktree_base: Option<PathBuf>,
) -> ExitCode {
    with_saga(config, db, worktree_base, false, |saga| {
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
) -> ExitCode {
    with_saga(config, db, None, false, |saga| {
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
) -> ExitCode {
    with_saga(config, db, worktree_base, false, |saga| {
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
) -> ExitCode {
    if !accept_integrate {
        eprintln!("integrate requires --accept-integrate after reviewing the integrate worktree");
        return ExitCode::from(2);
    }
    with_saga(config, db, worktree_base, false, |saga| {
        match saga.integrate_into(&graph, &into) {
            Ok(()) => {
                println!("integrated graph {graph} into {into}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e:?}");
                ExitCode::from(1)
            }
        }
    })
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
    let herdr = meshloop_adapters::herdr::HerdrCliAdapter::new(std::path::PathBuf::from("herdr"));
    let doctor = herdr.probe_status();
    let (running, version, err) = match &doctor {
        Ok(d) => (d.server_running, d.version.clone(), None),
        Err(e) => (false, None, Some(format!("{e:?}"))),
    };
    let data = serde_json::json!({
        "herdr_server_running": running,
        "herdr_version": version,
        "herdr_error": err,
        "live_transport": "herdr",
        "fixture_transport": "subprocess",
        "namespace": "meshloop:",
        "supervisor_only": true,
        "note": "Live pane split is not performed by doctor. Workers require --allow-live-harness.",
    });
    if json {
        println!("{}", json_out::ok("meshloop:doctor", origin, data));
    } else {
        println!("meshloop:doctor");
        println!("  herdr running: {running}");
        println!("  herdr version: {version:?}");
        if let Some(e) = err {
            println!("  herdr error: {e}");
        }
        println!("  live transport: herdr | fixture: subprocess");
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
    allow_live_harness: bool,
    json: bool,
    origin: meshloop_engine::origin::Origin,
) -> ExitCode {
    use meshloop_adapters::herdr::HerdrCliAdapter;
    use meshloop_engine::orchestrate::{
        PinnedEvidence, WaveResult, execute_wave, matrix_only, persist_reviews, pin_attempt,
        plan_review, write_pack,
    };

    if allow_live_harness && origin.session.is_none() {
        let msg = "live meshloop:orchestrate requires --origin-session so the supervisor pane is never split";
        if json {
            println!("{}", json_out::err("meshloop:orchestrate", origin, msg));
        } else {
            eprintln!("{msg}");
        }
        return ExitCode::from(2);
    }

    let mut composed = match load_cfg(config) {
        Ok((cfg, _)) => compose::compose(&cfg, repo_root(), db.as_deref(), worktree_base).ok(),
        Err(_) => None,
    };

    let pinned = composed.as_mut().and_then(|c| {
        let id = graph
            .clone()
            .or_else(|| c.store.latest_run().ok().flatten().map(|r| r.graph_id))?;
        pin_attempt(&c.store, &c.git, &id, TaskId(task)).ok()
    });

    if allow_live_harness && pinned.is_none() {
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
        allow_live_harness,
        pack_dir.clone(),
    ) {
        Ok(plan) => {
            let wave: WaveResult = if allow_live_harness {
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
