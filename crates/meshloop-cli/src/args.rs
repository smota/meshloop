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
        attempts: bool,
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

/// Flags every subcommand accepts: `(takes_value, name)`.
const GLOBAL_FLAGS: &[(bool, &str)] = &[
    (false, "--json"),
    (true, "--origin-harness"),
    (true, "--origin-session"),
    (false, "--help"),
    (false, "-h"),
];

/// Per-verb flags beyond the global ones: `(takes_value, name)`. `None` for verbs that are
/// not subcommands (help, version) or unknown. Must mirror what `parse_command` reads.
fn verb_flags(verb: &str) -> Option<&'static [(bool, &'static str)]> {
    Some(match verb {
        "mcp" | "roles" => &[],
        "doctor" => &[(true, "--config"), (true, "--db")],
        "plan" => &[
            (true, "--objective"),
            (true, "--config"),
            (true, "--out"),
            (true, "--scope"),
            (true, "--db"),
            (true, "--intent-file"),
        ],
        "review-plan" => &[
            (false, "--accept"),
            (false, "--decline"),
            (false, "--adjust"),
            (true, "--plan"),
            (true, "--reason"),
            (true, "--as"),
            (true, "--objective"),
            (true, "--intent-file"),
            (true, "--config"),
            (true, "--db"),
            (true, "--out"),
            (true, "--scope"),
            (false, "--fixture-only"),
        ],
        "run" => &[
            (true, "--plan"),
            (false, "--accept-plan"),
            (false, "--reset"),
            (true, "--config"),
            (true, "--worktree-base"),
            (true, "--db"),
            (false, "--fixture-only"),
            (false, "--detach"),
        ],
        "status" => &[(true, "--graph"), (true, "--config"), (true, "--db")],
        "resume" => &[
            (true, "--graph"),
            (false, "--retry"),
            (false, "--restart"),
            (true, "--config"),
            (true, "--db"),
            (true, "--worktree-base"),
            (false, "--fixture-only"),
        ],
        "cancel" => &[
            (true, "--session-id"),
            (true, "--graph"),
            (true, "--task"),
            (true, "--config"),
            (true, "--db"),
            (true, "--worktree-base"),
        ],
        "inspect" => &[
            (true, "--session-id"),
            (true, "--graph"),
            (true, "--task"),
            (true, "--config"),
            (true, "--db"),
            (false, "--attempts"),
        ],
        "watch" => &[
            (true, "--session-id"),
            (true, "--graph"),
            (true, "--interval"),
            (true, "--timeout"),
            (true, "--config"),
            (true, "--db"),
        ],
        "accept" => &[
            (true, "--task"),
            (true, "--as"),
            (true, "--graph"),
            (true, "--config"),
            (true, "--db"),
            (true, "--worktree-base"),
        ],
        "integrate" => &[
            (true, "--graph"),
            (true, "--into"),
            (false, "--accept-integrate"),
            (true, "--config"),
            (true, "--db"),
            (true, "--worktree-base"),
        ],
        "orchestrate" => &[
            (true, "--task"),
            (true, "--graph"),
            (true, "--config"),
            (true, "--db"),
            (true, "--worktree-base"),
            (true, "--model-a"),
            (true, "--model-b"),
            (false, "--fixture-only"),
        ],
        "bundle" => &[(true, "--dest"), (false, "--gitignore")],
        "ast-skeleton" => &[(true, "--path")],
        "symbol-lookup" => &[(true, "--query"), (true, "--k"), (true, "--scope")],
        "mutate-plan" => &[
            (true, "--graph"),
            (true, "--mutation-file"),
            (true, "--mutation"),
            (false, "--require-review"),
            (true, "--config"),
            (true, "--db"),
            (true, "--worktree-base"),
        ],
        _ => return None,
    })
}

fn looks_like_flag(token: &str) -> bool {
    token.starts_with("--")
        || token
            .strip_prefix('-')
            .and_then(|r| r.chars().next())
            .is_some_and(|c| c.is_alphabetic())
}

/// Reject any flag-like token the verb does not accept. The token after a value flag is
/// that flag's value and is never inspected, even when it starts with `-`.
fn reject_unknown_flags(verb: &str, rest: &[String]) -> Result<(), String> {
    let Some(own) = verb_flags(verb) else {
        return Ok(());
    };
    let mut tokens = rest.iter();
    while let Some(token) = tokens.next() {
        if !looks_like_flag(token) {
            continue;
        }
        match GLOBAL_FLAGS
            .iter()
            .chain(own.iter())
            .find(|(_, name)| name == token)
        {
            Some((true, _)) => {
                tokens.next();
            }
            Some((false, _)) => {}
            None => return Err(format!("meshloop:{verb} does not accept {token}")),
        }
    }
    Ok(())
}

fn parse_command(args: &[String]) -> Result<Command, String> {
    if let Some(first) = args.first() {
        reject_unknown_flags(&verb(first), &args[1..])?;
    }
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
                attempts: has_flag(rest, "--attempts"),
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

    fn argv(s: &str) -> Vec<String> {
        s.split_whitespace().map(ToString::to_string).collect()
    }

    /// Every documented flag of every verb, with the required ones present.
    const DOCUMENTED: &[&str] = &[
        "doctor --config c --db d",
        "plan --objective o --config c --out x --scope s --db d --intent-file i",
        "review-plan --accept --plan p --reason r --as me --objective o --intent-file i --config c --db d --out x --scope s --fixture-only",
        "review-plan --decline --plan p",
        "review-plan --adjust --reason r",
        "run --plan p --accept-plan --reset --config c --worktree-base w --db d --fixture-only --detach",
        "status --graph g --config c --db d",
        "resume --graph g --retry --config c --db d --worktree-base w --fixture-only",
        "resume --restart",
        "cancel --session-id s --graph g --task 1 --config c --db d --worktree-base w",
        "inspect --session-id s --graph g --task 1 --config c --db d --attempts",
        "watch --session-id s --graph g --interval 2 --timeout 9 --config c --db d",
        "accept --task 1 --as me --graph g --config c --db d --worktree-base w",
        "integrate --graph g --into main --accept-integrate --config c --db d --worktree-base w",
        "orchestrate --task 1 --graph g --config c --db d --worktree-base w --model-a a --model-b b --fixture-only",
        "bundle --dest d --gitignore",
        "ast-skeleton --path p",
        "symbol-lookup --query q --k 3 --scope s",
        "mutate-plan --graph g --mutation-file f --mutation {} --require-review --config c --db d --worktree-base w",
        "mcp",
        "roles",
    ];

    #[test]
    fn documented_flags_still_parse() {
        for line in DOCUMENTED {
            parse(&argv(line)).unwrap_or_else(|e| panic!("`{line}` rejected: {e}"));
        }
    }

    #[test]
    fn global_flags_accepted_on_every_subcommand() {
        for line in DOCUMENTED {
            let with_globals =
                format!("{line} --json --origin-harness h --origin-session s --help -h");
            parse(&argv(&with_globals)).unwrap_or_else(|e| panic!("`{with_globals}`: {e}"));
        }
    }

    #[test]
    fn unknown_flag_is_rejected_on_every_subcommand() {
        for line in DOCUMENTED {
            let verb = line.split_whitespace().next().unwrap();
            let err = parse(&argv(&format!("{line} --bogus"))).expect_err(line);
            assert_eq!(err, format!("meshloop:{verb} does not accept --bogus"));
        }
    }

    #[test]
    fn status_rejects_bogus_flag() {
        let err = parse(&argv("status --bogus")).expect_err("must reject");
        assert!(err.contains("--bogus") && err.contains("status"), "{err}");
        let err = parse(&argv("status -z")).expect_err("must reject short flag");
        assert!(err.contains("-z"), "{err}");
        // A flag valid for another verb is still unknown here.
        assert!(parse(&argv("status --attempts")).is_err());
    }

    #[test]
    fn value_starting_with_dash_is_not_a_flag() {
        let inv = parse(&argv("review-plan --adjust --reason -x")).expect("parse");
        match inv.command {
            Command::ReviewPlan { reason, .. } => assert_eq!(reason.as_deref(), Some("-x")),
            _ => panic!("expected review-plan"),
        }
        assert!(parse(&argv("status --graph --not-a-flag")).is_ok());
        // The skipped value does not shield the token after it.
        assert!(parse(&argv("status --graph g --bogus")).is_err());
    }

    #[test]
    fn prefixed_verb_form_is_validated_too() {
        assert!(parse(&argv("meshloop:status --bogus")).is_err());
    }
}
