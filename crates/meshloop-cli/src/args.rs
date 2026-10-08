//! Hand-rolled argv parsing. Keep this the only place that names flags.

use std::path::PathBuf;

use meshloop_domain::role::MeshloopId;
use meshloop_domain::state::PlanDecision;
use meshloop_engine::origin::Origin;

#[derive(Debug, Clone)]
pub struct Invocation {
    pub command: Command,
    pub json: bool,
    pub origin: Origin,
}

#[derive(Debug, Clone)]
pub enum Command {
    Help,
    Version,
    Mcp,
    Roles,
    Doctor {
        config: Option<PathBuf>,
        db: Option<PathBuf>,
    },
    Status {
        graph: Option<String>,
        config: Option<PathBuf>,
        db: Option<PathBuf>,
    },
    Plan {
        objective: String,
        config: Option<PathBuf>,
        out: Option<PathBuf>,
        scope: Option<String>,
        db: Option<PathBuf>,
        intent_file: Option<PathBuf>,
    },
    ReviewPlan {
        plan: PathBuf,
        decision: PlanDecision,
        reason: Option<String>,
        identity: Option<String>,
        objective: String,
        intent_file: Option<PathBuf>,
        config: Option<PathBuf>,
        db: Option<PathBuf>,
        out: Option<PathBuf>,
        scope: Option<String>,
        fixture_only: bool,
    },
    Run {
        plan: PathBuf,
        accept_plan: bool,
        reset: bool,
        config: Option<PathBuf>,
        worktree_base: Option<PathBuf>,
        db: Option<PathBuf>,
        fixture_only: bool,
        detach: bool,
    },
    Resume {
        graph: Option<String>,
        retry: bool,
        restart: bool,
        config: Option<PathBuf>,
        db: Option<PathBuf>,
        worktree_base: Option<PathBuf>,
        fixture_only: bool,
    },
    Cancel {
        graph: Option<String>,
        task: Option<u32>,
        session_id: Option<String>,
        config: Option<PathBuf>,
        db: Option<PathBuf>,
        worktree_base: Option<PathBuf>,
    },
    Inspect {
        task: Option<u32>,
        graph: Option<String>,
        session_id: Option<String>,
        config: Option<PathBuf>,
        db: Option<PathBuf>,
    },
    Watch {
        graph: Option<String>,
        session_id: Option<String>,
        config: Option<PathBuf>,
        db: Option<PathBuf>,
        interval_secs: u64,
        timeout_secs: u64,
    },
    Accept {
        task: u32,
        identity: String,
        graph: Option<String>,
        config: Option<PathBuf>,
        db: Option<PathBuf>,
        worktree_base: Option<PathBuf>,
    },
    Integrate {
        graph: String,
        into: String,
        accept_integrate: bool,
        config: Option<PathBuf>,
        db: Option<PathBuf>,
        worktree_base: Option<PathBuf>,
    },
    Orchestrate {
        graph: Option<String>,
        task: u32,
        config: Option<PathBuf>,
        db: Option<PathBuf>,
        worktree_base: Option<PathBuf>,
        model_a: String,
        model_b: String,
        fixture_only: bool,
    },
    Bundle {
        dest: PathBuf,
        gitignore: bool,
    },
    AstSkeleton {
        path: String,
    },
    SymbolLookup {
        query: String,
        k: usize,
        scope: Option<String>,
    },
    MutatePlan {
        graph: Option<String>,
        mutation_file: Option<PathBuf>,
        mutation_json: Option<String>,
        require_review: bool,
        config: Option<PathBuf>,
        db: Option<PathBuf>,
        worktree_base: Option<PathBuf>,
    },
}

pub fn flag_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

pub fn has_flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn verb(raw: &str) -> String {
    MeshloopId::parse(raw)
        .map(|id| id.cli_verb().to_string())
        .unwrap_or_else(|_| raw.to_string())
}

pub fn parse(args: &[String]) -> Result<Invocation, String> {
    let json = has_flag(args, "--json");
    let origin = Origin::from_flags_or_env(
        flag_value(args, "--origin-harness"),
        flag_value(args, "--origin-session"),
    );
    let command = parse_command(args)?;
    Ok(Invocation {
        command,
        json,
        origin,
    })
}

fn parse_command(args: &[String]) -> Result<Command, String> {
    match args.first().map(|s| verb(s)) {
        None => Ok(Command::Status {
            graph: None,
            config: None,
            db: None,
        }),
        Some(v) if v == "--help" || v == "-h" || v == "help" => Ok(Command::Help),
        Some(v) if v == "--version" || v == "-V" || v == "version" => Ok(Command::Version),
        Some(v) if v == "mcp" => Ok(Command::Mcp),
        Some(v) if v == "roles" => Ok(Command::Roles),
        Some(v) if v == "doctor" => Ok(Command::Doctor {
            config: flag_value(&args[1..], "--config").map(PathBuf::from),
            db: flag_value(&args[1..], "--db").map(PathBuf::from),
        }),
        Some(v) if v == "plan" => {
            let rest = &args[1..];
            let objective = flag_value(rest, "--objective").unwrap_or_default();
            if objective.is_empty() && flag_value(rest, "--intent-file").is_none() {
                return Err(
                    "meshloop:plan requires --objective \"<text>\" or --intent-file <path>".into(),
                );
            }
            Ok(Command::Plan {
                objective,
                config: flag_value(rest, "--config").map(PathBuf::from),
                out: flag_value(rest, "--out").map(PathBuf::from),
                scope: flag_value(rest, "--scope"),
                db: flag_value(rest, "--db").map(PathBuf::from),
                intent_file: flag_value(rest, "--intent-file").map(PathBuf::from),
            })
        }
        Some(v) if v == "review-plan" => {
            let rest = &args[1..];
            let accept = has_flag(rest, "--accept");
            let decline = has_flag(rest, "--decline");
            let adjust = has_flag(rest, "--adjust");
            let n = [accept, decline, adjust].into_iter().filter(|b| *b).count();
            if n != 1 {
                return Err(
                    "meshloop:review-plan requires exactly one of --accept, --decline, or --adjust"
                        .into(),
                );
            }
            let decision = if accept {
                PlanDecision::Accept
            } else if decline {
                PlanDecision::Decline
            } else {
                PlanDecision::Adjust
            };
            if decision == PlanDecision::Adjust
                && flag_value(rest, "--objective")
                    .unwrap_or_default()
                    .is_empty()
                && flag_value(rest, "--intent-file").is_none()
                && flag_value(rest, "--reason").is_none()
            {
                return Err(
                    "meshloop:review-plan --adjust requires --objective, --intent-file, or --reason"
                        .into(),
                );
            }
            Ok(Command::ReviewPlan {
                plan: flag_value(rest, "--plan")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("meshloop-plan.json")),
                decision,
                reason: flag_value(rest, "--reason"),
                identity: flag_value(rest, "--as"),
                objective: flag_value(rest, "--objective").unwrap_or_default(),
                intent_file: flag_value(rest, "--intent-file").map(PathBuf::from),
                config: flag_value(rest, "--config").map(PathBuf::from),
                db: flag_value(rest, "--db").map(PathBuf::from),
                out: flag_value(rest, "--out").map(PathBuf::from),
                scope: flag_value(rest, "--scope"),
                fixture_only: has_flag(rest, "--fixture-only"),
            })
        }
        Some(v) if v == "run" => {
            let rest = &args[1..];
            let plan = flag_value(rest, "--plan")
                .map(PathBuf::from)
                .ok_or_else(|| "meshloop:run requires --plan <file>".to_string())?;
            Ok(Command::Run {
                plan,
                accept_plan: has_flag(rest, "--accept-plan"),
                reset: has_flag(rest, "--reset"),
                config: flag_value(rest, "--config").map(PathBuf::from),
                worktree_base: flag_value(rest, "--worktree-base").map(PathBuf::from),
                db: flag_value(rest, "--db").map(PathBuf::from),
                fixture_only: has_flag(rest, "--fixture-only"),
                detach: has_flag(rest, "--detach"),
            })
        }
        Some(v) if v == "status" => Ok(Command::Status {
            graph: flag_value(&args[1..], "--graph"),
            config: flag_value(&args[1..], "--config").map(PathBuf::from),
            db: flag_value(&args[1..], "--db").map(PathBuf::from),
        }),
        Some(v) if v == "resume" => {
            let rest = &args[1..];
            let retry = has_flag(rest, "--retry");
            let restart = has_flag(rest, "--restart");
            if retry && restart {
                return Err("meshloop:resume accepts either --retry or --restart, not both".into());
            }
            Ok(Command::Resume {
                graph: flag_value(rest, "--graph"),
                retry,
                restart,
                config: flag_value(rest, "--config").map(PathBuf::from),
                db: flag_value(rest, "--db").map(PathBuf::from),
                worktree_base: flag_value(rest, "--worktree-base").map(PathBuf::from),
                fixture_only: has_flag(rest, "--fixture-only"),
            })
        }
        Some(v) if v == "cancel" => {
            let rest = &args[1..];
            let session_id = flag_value(rest, "--session-id");
            let graph = flag_value(rest, "--graph").or_else(|| session_id.clone());
            Ok(Command::Cancel {
                graph,
                task: flag_value(rest, "--task").and_then(|s| s.parse().ok()),
                session_id,
                config: flag_value(rest, "--config").map(PathBuf::from),
                db: flag_value(rest, "--db").map(PathBuf::from),
                worktree_base: flag_value(rest, "--worktree-base").map(PathBuf::from),
            })
        }
        Some(v) if v == "inspect" => {
            let rest = &args[1..];
            let session_id = flag_value(rest, "--session-id");
            let graph = flag_value(rest, "--graph").or_else(|| session_id.clone());
            let task = flag_value(rest, "--task").and_then(|s| s.parse().ok());
            Ok(Command::Inspect {
                task,
                graph,
                session_id,
                config: flag_value(rest, "--config").map(PathBuf::from),
                db: flag_value(rest, "--db").map(PathBuf::from),
            })
        }
        Some(v) if v == "watch" => {
            let rest = &args[1..];
            let session_id = flag_value(rest, "--session-id");
            let graph = flag_value(rest, "--graph").or_else(|| session_id.clone());
            let interval_secs = match flag_value(rest, "--interval") {
                Some(raw) => raw.parse::<u64>().ok().filter(|n| *n >= 1).ok_or_else(|| {
                    format!("meshloop:watch --interval expects whole seconds >= 1, got {raw}")
                })?,
                None => 5,
            };
            let timeout_secs = match flag_value(rest, "--timeout") {
                Some(raw) => raw.parse::<u64>().map_err(|_| {
                    format!("meshloop:watch --timeout expects whole seconds (0 = none), got {raw}")
                })?,
                None => 0,
            };
            Ok(Command::Watch {
                graph,
                session_id,
                config: flag_value(rest, "--config").map(PathBuf::from),
                db: flag_value(rest, "--db").map(PathBuf::from),
                interval_secs,
                timeout_secs,
            })
        }
        Some(v) if v == "accept" => {
            let rest = &args[1..];
            let task = flag_value(rest, "--task")
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| "meshloop:accept requires --task <id>".to_string())?;
            let identity = flag_value(rest, "--as")
                .ok_or_else(|| "meshloop:accept requires --as <identity>".to_string())?;
            Ok(Command::Accept {
                task,
                identity,
                graph: flag_value(rest, "--graph"),
                config: flag_value(rest, "--config").map(PathBuf::from),
                db: flag_value(rest, "--db").map(PathBuf::from),
                worktree_base: flag_value(rest, "--worktree-base").map(PathBuf::from),
            })
        }
        Some(v) if v == "integrate" => {
            let rest = &args[1..];
            let graph = flag_value(rest, "--graph")
                .ok_or_else(|| "meshloop:integrate requires --graph <id>".to_string())?;
            let into = flag_value(rest, "--into")
                .ok_or_else(|| "meshloop:integrate requires --into <ref>".to_string())?;
            Ok(Command::Integrate {
                graph,
                into,
                accept_integrate: has_flag(rest, "--accept-integrate"),
                config: flag_value(rest, "--config").map(PathBuf::from),
                db: flag_value(rest, "--db").map(PathBuf::from),
                worktree_base: flag_value(rest, "--worktree-base").map(PathBuf::from),
            })
        }
        Some(v) if v == "orchestrate" => {
            let rest = &args[1..];
            let task = flag_value(rest, "--task")
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| "meshloop:orchestrate requires --task <id>".to_string())?;
            Ok(Command::Orchestrate {
                graph: flag_value(rest, "--graph"),
                task,
                config: flag_value(rest, "--config").map(PathBuf::from),
                db: flag_value(rest, "--db").map(PathBuf::from),
                worktree_base: flag_value(rest, "--worktree-base").map(PathBuf::from),
                model_a: flag_value(rest, "--model-a").unwrap_or_else(|| "model-a".into()),
                model_b: flag_value(rest, "--model-b").unwrap_or_else(|| "model-b".into()),
                fixture_only: has_flag(rest, "--fixture-only"),
            })
        }
        Some(v) if v == "bundle" => {
            let rest = &args[1..];
            Ok(Command::Bundle {
                dest: flag_value(rest, "--dest")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("dist/meshloop-session-bundle")),
                gitignore: has_flag(rest, "--gitignore"),
            })
        }
        Some(v) if v == "ast-skeleton" => {
            let path = flag_value(&args[1..], "--path")
                .ok_or_else(|| "meshloop:ast-skeleton requires --path <file>".to_string())?;
            Ok(Command::AstSkeleton { path })
        }
        Some(v) if v == "symbol-lookup" => {
            let rest = &args[1..];
            let query = flag_value(rest, "--query")
                .ok_or_else(|| "meshloop:symbol-lookup requires --query \"<text>\"".to_string())?;
            let k = match flag_value(rest, "--k") {
                Some(raw) => raw.parse().map_err(|_| {
                    format!("meshloop:symbol-lookup --k expects a number, got {raw}")
                })?,
                None => crate::context_tools::DEFAULT_LOOKUP_K,
            };
            Ok(Command::SymbolLookup {
                query,
                k,
                scope: flag_value(rest, "--scope"),
            })
        }
        Some(v) if v == "mutate-plan" => {
            let rest = &args[1..];
            Ok(Command::MutatePlan {
                graph: flag_value(rest, "--graph"),
                mutation_file: flag_value(rest, "--mutation-file").map(PathBuf::from),
                mutation_json: flag_value(rest, "--mutation"),
                require_review: has_flag(rest, "--require-review"),
                config: flag_value(rest, "--config").map(PathBuf::from),
                db: flag_value(rest, "--db").map(PathBuf::from),
                worktree_base: flag_value(rest, "--worktree-base").map(PathBuf::from),
            })
        }
        Some(v)
            if matches!(
                v.as_str(),
                "reviewer" | "planner" | "scout" | "worker" | "orchestrator"
            ) =>
        {
            Err(format!(
                "unprefixed '{v}' is not a Meshloop command. Use meshloop:{v} / meshloop {v} after the binary name, never a bare harness role."
            ))
        }
        _ => Err("Unsupported command. Run meshloop --help.".into()),
    }
}

pub fn help_text() -> &'static str {
    "Meshloop session control plane (native Windows). All skills/commands/tools are prefixed meshloop:.\n\
     Canonical ids: meshloop:plan | meshloop:review-plan | meshloop:run | meshloop:status | meshloop:accept\n\
     \x20 meshloop:resume | meshloop:cancel | meshloop:inspect | meshloop:integrate | meshloop:roles\n\
     \x20 meshloop:doctor | meshloop:orchestrate | meshloop:mutate-plan | meshloop:mcp | meshloop:bundle\n\
     \x20 meshloop:watch\n\
     CLI verbs (binary already namespaces): meshloop plan|run|status|... or meshloop meshloop:plan\n\
     Slash: /meshloop:plan   MCP tools: meshloop_plan\n\
     Usage:\n\
     \x20 meshloop plan --objective \"<text>\" [--intent-file <path>] [--config] [--out] [--json]\n\
     \x20 meshloop review-plan --plan <path> --accept|--decline|--adjust [--reason] [--as] [--json]\n\
     \x20 meshloop run --plan <path> [--accept-plan] [--reset] [--fixture-only] [--json]\n\
     \x20 meshloop resume [--graph <id>] [--retry|--restart] [--json]\n\
     \x20 meshloop status [--graph <id>] [--config <path>] [--db <path>] [--json]\n\
     \x20 meshloop watch --graph <id> [--session-id <id>] [--interval <secs, default 5>] [--timeout <secs, 0 = none>] [--json]\n\
     \x20 meshloop roles [--json]\n\
     \x20 meshloop doctor [--config <path>] [--db <path>] [--json]\n\
     \x20 meshloop orchestrate --task <id> --model-a <ref> --model-b <ref> [--json]\n\
     \x20 meshloop mutate-plan [--graph <id>] (--mutation <json> | --mutation-file <path>) [--require-review] [--json]\n\
     \x20 meshloop mcp\n\
     \x20 meshloop bundle [--dest <dir>] [--gitignore]\n\
     Origin (supervisor-only): --origin-harness <name> --origin-session <id>\n\
     \x20 or MESHLOOP_ORIGIN_HARNESS / MESHLOOP_ORIGIN_SESSION.\n\
     Live workers are the default (Herdr). --fixture-only forces the CI subprocess double."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_keeps_config_db_and_graph() {
        let args: Vec<String> = ["status", "--config", "x", "--db", "y", "--graph", "g"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let inv = parse(&args).expect("parse");
        match inv.command {
            Command::Status { graph, config, db } => {
                assert_eq!(graph.as_deref(), Some("g"));
                assert_eq!(config, Some(PathBuf::from("x")));
                assert_eq!(db, Some(PathBuf::from("y")));
            }
            _ => panic!("expected status"),
        }
    }
}
