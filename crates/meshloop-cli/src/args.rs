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
        drivers: bool,
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
        /// Start the resume as a detached driver, as `run --detach` does; needs `--graph`.
        detach: bool,
    },
    Cancel {
        driver: Option<String>,
        graph: Option<String>,
        task: Option<u32>,
        session_id: Option<String>,
        config: Option<PathBuf>,
        db: Option<PathBuf>,
        worktree_base: Option<PathBuf>,
    },
    Inspect {
        driver: Option<String>,
        task: Option<u32>,
        graph: Option<String>,
        session_id: Option<String>,
        config: Option<PathBuf>,
        db: Option<PathBuf>,
        attempts: bool,
    },
    /// The graph's run-table events with their stable ids, read-only (#137).
    Events {
        graph: Option<String>,
        after: i64,
        limit: i64,
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
        deliverable: Option<String>,
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
    Workspace {
        report: WorkspaceReport,
        cleanup: bool,
        apply: bool,
        digest: Option<String>,
        /// `apply --cleanup` only: limit removal to this graph's eligible entries.
        graph: Option<String>,
        config: Option<PathBuf>,
        db: Option<PathBuf>,
        worktree_base: Option<PathBuf>,
    },
    /// Advisory routing priors (#139): `accept --file F --as ID`, `revoke --as ID`,
    /// and the read-only `show` and `check --file F`.
    Priors {
        action: PriorsAction,
        file: Option<PathBuf>,
        identity: Option<String>,
        config: Option<PathBuf>,
        db: Option<PathBuf>,
    },
}

/// `meshloop priors <action>` (#139). Only `accept` and `revoke` write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriorsAction {
    Accept,
    Revoke,
    Show,
    Check,
}

impl PriorsAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accept => "accept",
            Self::Revoke => "revoke",
            Self::Show => "show",
            Self::Check => "check",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceReport {
    Inventory,
    Preview,
    Apply,
    Adopt,
    Maintain,
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

fn driver_selector(args: &[String]) -> Result<Option<String>, String> {
    let driver = flag_value(args, "--driver");
    if driver.is_some()
        && ["--graph", "--session-id", "--task"]
            .iter()
            .any(|flag| has_flag(args, flag))
    {
        return Err("--driver is mutually exclusive with --graph, --session-id and --task".into());
    }
    Ok(driver)
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
        "status" => &[
            (false, "--drivers"),
            (true, "--graph"),
            (true, "--config"),
            (true, "--db"),
        ],
        "resume" => &[
            (true, "--graph"),
            (false, "--retry"),
            (false, "--restart"),
            (true, "--config"),
            (true, "--db"),
            (true, "--worktree-base"),
            (false, "--fixture-only"),
            (false, "--detach"),
        ],
        "cancel" => &[
            (true, "--driver"),
            (true, "--session-id"),
            (true, "--graph"),
            (true, "--task"),
            (true, "--config"),
            (true, "--db"),
            (true, "--worktree-base"),
        ],
        "inspect" => &[
            (true, "--driver"),
            (true, "--session-id"),
            (true, "--graph"),
            (true, "--task"),
            (true, "--config"),
            (true, "--db"),
            (false, "--attempts"),
        ],
        "events" => &[
            (true, "--graph"),
            (true, "--after"),
            (true, "--limit"),
            (true, "--config"),
            (true, "--db"),
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
            (true, "--deliverable"),
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
        "priors" => &[
            (true, "--file"),
            (true, "--as"),
            (true, "--config"),
            (true, "--db"),
        ],
        "workspace" => &[
            (false, "--cleanup"),
            (false, "--apply"),
            (true, "--digest"),
            (true, "--graph"),
            (true, "--config"),
            (true, "--db"),
            (true, "--worktree-base"),
        ],
        _ => return None,
    })
}

fn positionals(verb: &str, rest: &[String]) -> Vec<String> {
    let own = verb_flags(verb).unwrap_or(&[]);
    let mut tokens = rest.iter();
    let mut out = Vec::new();
    while let Some(token) = tokens.next() {
        if looks_like_flag(token) {
            if GLOBAL_FLAGS
                .iter()
                .chain(own)
                .any(|(takes, name)| *name == token && *takes)
            {
                tokens.next();
            }
        } else {
            out.push(token.clone());
        }
    }
    out
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
            drivers: false,
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
        Some(v) if v == "status" => {
            if has_flag(&args[1..], "--drivers") && has_flag(&args[1..], "--graph") {
                return Err("--drivers is mutually exclusive with --graph".into());
            }
            Ok(Command::Status {
                drivers: has_flag(&args[1..], "--drivers"),
                graph: flag_value(&args[1..], "--graph"),
                config: flag_value(&args[1..], "--config").map(PathBuf::from),
                db: flag_value(&args[1..], "--db").map(PathBuf::from),
            })
        }
        Some(v) if v == "resume" => {
            let rest = &args[1..];
            let retry = has_flag(rest, "--retry");
            let restart = has_flag(rest, "--restart");
            if retry && restart {
                return Err("meshloop:resume accepts either --retry or --restart, not both".into());
            }
            let detach = has_flag(rest, "--detach");
            if detach && flag_value(rest, "--graph").is_none() {
                return Err("meshloop:resume --detach requires --graph <id>".into());
            }
            Ok(Command::Resume {
                graph: flag_value(rest, "--graph"),
                retry,
                restart,
                config: flag_value(rest, "--config").map(PathBuf::from),
                db: flag_value(rest, "--db").map(PathBuf::from),
                worktree_base: flag_value(rest, "--worktree-base").map(PathBuf::from),
                fixture_only: has_flag(rest, "--fixture-only"),
                detach,
            })
        }
        Some(v) if v == "cancel" => {
            let rest = &args[1..];
            let driver = driver_selector(rest)?;
            let session_id = flag_value(rest, "--session-id");
            let graph = flag_value(rest, "--graph").or_else(|| session_id.clone());
            Ok(Command::Cancel {
                driver,
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
            let driver = driver_selector(rest)?;
            if driver.is_some() && has_flag(rest, "--attempts") {
                return Err("--driver is mutually exclusive with --attempts".into());
            }
            let session_id = flag_value(rest, "--session-id");
            let graph = flag_value(rest, "--graph").or_else(|| session_id.clone());
            let task = flag_value(rest, "--task").and_then(|s| s.parse().ok());
            Ok(Command::Inspect {
                driver,
                task,
                graph,
                session_id,
                config: flag_value(rest, "--config").map(PathBuf::from),
                db: flag_value(rest, "--db").map(PathBuf::from),
                attempts: has_flag(rest, "--attempts"),
            })
        }
        Some(v) if v == "events" => {
            let rest = &args[1..];
            let after = match flag_value(rest, "--after") {
                Some(raw) => raw.parse::<i64>().ok().filter(|n| *n >= 0).ok_or_else(|| {
                    format!("meshloop:events --after expects an event id >= 0, got {raw}")
                })?,
                None => 0,
            };
            let limit = match flag_value(rest, "--limit") {
                Some(raw) => raw
                    .parse::<i64>()
                    .ok()
                    .filter(|n| (1..=10_000).contains(n))
                    .ok_or_else(|| {
                        format!("meshloop:events --limit expects 1 to 10000 events, got {raw}")
                    })?,
                None => 1000,
            };
            Ok(Command::Events {
                graph: flag_value(rest, "--graph"),
                after,
                limit,
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
                deliverable: flag_value(rest, "--deliverable"),
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
        Some(v) if v == "workspace" => {
            let rest = &args[1..];
            let positionals = positionals("workspace", rest);
            let names: Vec<&str> = positionals.iter().map(String::as_str).collect();
            let report = match names.as_slice() {
                ["inventory"] => WorkspaceReport::Inventory,
                ["preview"] => WorkspaceReport::Preview,
                ["apply"] => WorkspaceReport::Apply,
                ["adopt"] => WorkspaceReport::Adopt,
                ["maintain"] => WorkspaceReport::Maintain,
                _ => {
                    return Err(
                        "meshloop:workspace requires inventory, preview, apply, adopt, or maintain"
                            .into(),
                    );
                }
            };
            let cleanup = has_flag(rest, "--cleanup");
            let apply = has_flag(rest, "--apply");
            if cleanup && !matches!(report, WorkspaceReport::Preview | WorkspaceReport::Apply) {
                return Err("meshloop:workspace --cleanup requires preview or apply".into());
            }
            if apply && !matches!(report, WorkspaceReport::Adopt | WorkspaceReport::Maintain) {
                return Err("meshloop:workspace --apply requires adopt or maintain".into());
            }
            if has_flag(rest, "--digest") && !(cleanup || apply) {
                return Err("meshloop:workspace --digest requires --cleanup or --apply".into());
            }
            let digest = flag_value(rest, "--digest");
            if has_flag(rest, "--digest") && digest.is_none() {
                return Err("meshloop:workspace --digest requires a value".into());
            }
            let digest_ok = digest.as_deref().is_some_and(|value| {
                value.len() == 64
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            });
            if report == WorkspaceReport::Apply && (!cleanup || !digest_ok) {
                return Err(
                    "meshloop:workspace apply requires --cleanup --digest <64 lowercase hex>"
                        .into(),
                );
            }
            let graph = flag_value(rest, "--graph");
            if has_flag(rest, "--graph")
                && (report != WorkspaceReport::Apply
                    || graph
                        .as_deref()
                        .is_none_or(|g| !meshloop_domain::task_graph::graph_id_is_legal(g)))
            {
                return Err(
                    "meshloop:workspace --graph <graph-id> requires apply --cleanup".into(),
                );
            }
            if matches!(report, WorkspaceReport::Adopt | WorkspaceReport::Maintain)
                && apply
                && !digest_ok
            {
                return Err(
                    "meshloop:workspace adopt/maintain --apply requires --digest <64 lowercase hex>"
                        .into(),
                );
            }
            Ok(Command::Workspace {
                report,
                cleanup,
                apply,
                digest,
                graph,
                config: flag_value(rest, "--config").map(PathBuf::from),
                db: flag_value(rest, "--db").map(PathBuf::from),
                worktree_base: flag_value(rest, "--worktree-base").map(PathBuf::from),
            })
        }
        Some(v) if v == "priors" => parse_priors(&args[1..]),
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

fn parse_priors(rest: &[String]) -> Result<Command, String> {
    let names = positionals("priors", rest);
    let action = match names
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["accept"] => PriorsAction::Accept,
        ["revoke"] => PriorsAction::Revoke,
        ["show"] => PriorsAction::Show,
        ["check"] => PriorsAction::Check,
        _ => return Err("meshloop:priors requires accept, revoke, show, or check".into()),
    };
    let verb = action.as_str();
    let file = flag_value(rest, "--file").map(PathBuf::from);
    let identity = flag_value(rest, "--as");
    let wants_file = matches!(action, PriorsAction::Accept | PriorsAction::Check);
    let wants_identity = matches!(action, PriorsAction::Accept | PriorsAction::Revoke);
    if wants_file && file.is_none() {
        return Err(format!("meshloop:priors {verb} requires --file <path>"));
    }
    if !wants_file && has_flag(rest, "--file") {
        return Err(format!("meshloop:priors {verb} does not take --file"));
    }
    if wants_identity {
        // Required and never defaulted (ADR 0038 #139 amendment, rule 7).
        let id = identity
            .as_deref()
            .ok_or_else(|| format!("meshloop:priors {verb} requires --as <identity>"))?;
        meshloop_domain::advisory::check_identity(id)
            .map_err(|fault| format!("meshloop:priors {verb}: {}", fault.message()))?;
    } else if has_flag(rest, "--as") {
        return Err(format!("meshloop:priors {verb} does not take --as"));
    }
    Ok(Command::Priors {
        action,
        file,
        identity,
        config: flag_value(rest, "--config").map(PathBuf::from),
        db: flag_value(rest, "--db").map(PathBuf::from),
    })
}

pub fn help_text() -> &'static str {
    "Meshloop session control plane (native Windows). All skills/commands/tools are prefixed meshloop:.\n\
     Canonical ids: meshloop:plan | meshloop:review-plan | meshloop:run | meshloop:status | meshloop:accept\n\
     \x20 meshloop:resume | meshloop:cancel | meshloop:inspect | meshloop:integrate | meshloop:roles\n\
     \x20 meshloop:doctor | meshloop:orchestrate | meshloop:mutate-plan | meshloop:mcp | meshloop:bundle\n\
     \x20 meshloop:watch | meshloop:events | meshloop:workspace | meshloop:priors\n\
     CLI verbs (binary already namespaces): meshloop plan|run|status|... or meshloop meshloop:plan\n\
     Slash: /meshloop:plan   MCP tools: meshloop_plan\n\
     Usage:\n\
     \x20 meshloop plan --objective \"<text>\" [--intent-file <path>] [--config] [--out] [--json]\n\
     \x20 meshloop review-plan --plan <path> --accept|--decline|--adjust [--reason] [--as] [--json]\n\
     \x20 meshloop run --plan <path> [--accept-plan] [--reset] [--fixture-only] [--json]\n\
     \x20 meshloop resume [--graph <id>] [--retry|--restart] [--detach (needs --graph)] [--json]\n\
     \x20 meshloop status [--graph <id>|--drivers] [--config <path>] [--db <path>] [--json]\n\
     \x20 meshloop inspect --driver <id> [--db <path>] [--json]\n\
     \x20 meshloop cancel --driver <id> [--db <path>] [--json]\n\
     \x20 meshloop watch --graph <id> [--session-id <id>] [--interval <secs, default 5>] [--timeout <secs, 0 = none>] [--json]\n\
     \x20 meshloop events [--graph <id>] [--after <event id>] [--limit <n, default 1000>] [--config <path>] [--db <path>] [--json]\n\
     \x20 meshloop workspace inventory [--config <path>] [--db <path>] [--worktree-base <path>] [--json]\n\
     \x20 meshloop workspace preview [--cleanup] [--config <path>] [--db <path>] [--worktree-base <path>] [--json]\n\
     \x20 meshloop workspace apply --cleanup --digest <64 lowercase hex> [--graph <id>] [--config <path>] [--db <path>] [--worktree-base <path>] [--json]\n\
     \x20 meshloop workspace adopt|maintain [--apply --digest <64 lowercase hex>] [--config <path>] [--db <path>] [--worktree-base <path>] [--json]\n\
     \x20 --cleanup is valid only with preview or apply; apply requires --cleanup and a digest and never accepts --apply; --apply with a digest is valid only with adopt or maintain; --graph is valid only with apply --cleanup.\n\
     \x20 meshloop priors show [--json] | priors check --file <path> [--json]   (read-only)\n\
     \x20 meshloop priors accept --file <path> --as <identity> | priors revoke --as <identity> [--json]\n\
     \x20 meshloop roles [--json]\n\
     \x20 meshloop doctor [--config <path>] [--db <path>] [--json]\n\
     \x20 meshloop orchestrate --task <id> --model-a <ref> --model-b <ref> [--json]\n\
     \x20 meshloop mutate-plan [--graph <id>] (--mutation <json> | --mutation-file <path>) [--require-review] [--json]\n\
     \x20 meshloop mcp\n\
     \x20 meshloop bundle [--dest <dir>] [--gitignore]\n\
     Origin (supervisor-only): --origin-harness <name> --origin-session <id>\n\
     \x20 or MESHLOOP_ORIGIN_HARNESS / MESHLOOP_ORIGIN_SESSION.\n\
     Live workers are direct CLI subprocesses in Git worktrees. --fixture-only forces the CI subprocess double."
}

#[cfg(test)]
mod tests {
    #[test]
    fn driver_selectors_are_exclusive_and_discoverable_without_graph() {
        for verb in ["cancel", "inspect"] {
            for selector in ["--graph", "--session-id", "--task"] {
                let args = [verb, "--driver", "driver-id", selector, "1"].map(String::from);
                assert!(
                    super::parse(&args)
                        .unwrap_err()
                        .contains("mutually exclusive")
                );
            }
            assert!(super::parse(&[verb, "--driver", "driver-id"].map(String::from)).is_ok());
        }
        assert!(super::parse(&["status", "--drivers"].map(String::from)).is_ok());
        assert!(super::parse(&["status", "--drivers", "--graph", "g"].map(String::from)).is_err());
    }
    use super::*;

    #[test]
    fn events_takes_graph_and_an_after_cursor() {
        let parsed = parse(&["events", "--graph", "g", "--after", "42"].map(String::from)).unwrap();
        assert!(matches!(
            parsed.command,
            Command::Events { ref graph, after: 42, .. } if graph.as_deref() == Some("g")
        ));
        let parsed = parse(&["events"].map(String::from)).unwrap();
        assert!(matches!(
            parsed.command,
            Command::Events {
                graph: None,
                after: 0,
                ..
            }
        ));
        for bad in ["-1", "x", ""] {
            assert!(
                parse(&["events", "--after", bad].map(String::from)).is_err(),
                "{bad}"
            );
        }
        assert!(parse(&["events", "--task", "1"].map(String::from)).is_err());
        let parsed = parse(&["events", "--limit", "200"].map(String::from)).unwrap();
        assert!(matches!(parsed.command, Command::Events { limit: 200, .. }));
        assert!(matches!(
            parse(&["events"].map(String::from)).unwrap().command,
            Command::Events { limit: 1000, .. }
        ));
        for bad in ["0", "10001", "-5", "x"] {
            assert!(
                parse(&["events", "--limit", bad].map(String::from)).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn status_keeps_config_db_and_graph() {
        let args: Vec<String> = ["status", "--config", "x", "--db", "y", "--graph", "g"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let inv = parse(&args).expect("parse");
        match inv.command {
            Command::Status {
                graph, config, db, ..
            } => {
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
        "resume --graph g --retry --config c --db d --worktree-base w --fixture-only --detach",
        "resume --restart",
        "cancel --session-id s --graph g --task 1 --config c --db d --worktree-base w",
        "inspect --session-id s --graph g --task 1 --config c --db d --attempts",
        "watch --session-id s --graph g --interval 2 --timeout 9 --config c --db d",
        "accept --task 1 --as me --graph g --config c --db d --worktree-base w",
        "integrate --graph g --into main --accept-integrate --deliverable t --config c --db d --worktree-base w",
        "orchestrate --task 1 --graph g --config c --db d --worktree-base w --model-a a --model-b b --fixture-only",
        "bundle --dest d --gitignore",
        "ast-skeleton --path p",
        "symbol-lookup --query q --k 3 --scope s",
        "mutate-plan --graph g --mutation-file f --mutation {} --require-review --config c --db d --worktree-base w",
        "workspace inventory --config c --db d --worktree-base w",
        "workspace preview --config c --db d --worktree-base w",
        "workspace preview --cleanup --digest d --config c --db d --worktree-base w",
        "workspace adopt --config c --db d --worktree-base w",
        "workspace adopt --apply --digest aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa --config c --db d --worktree-base w",
        "workspace maintain --config c --db d --worktree-base w",
        "priors show --config c --db d",
        "priors check --file f --config c --db d",
        "priors accept --file f --as me --config c --db d",
        "priors revoke --as me --config c --db d",
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

    /// LP test 17: `--as` is required for the two writing verbs and each bad form is
    /// rejected before anything is read or written.
    #[test]
    fn priors_as_validation_rejects_each_bad_form() {
        let parse_as = |id: &str| {
            parse(&[
                "priors".to_string(),
                "revoke".into(),
                "--as".into(),
                id.to_string(),
            ])
        };
        assert!(parse_as("sam").is_ok());
        for (bad, why) in [
            ("", "must not be empty"),
            ("-sam", "must not start with '-'"),
            ("--json", "must not start with '-'"),
            ("sam\nroot", "control characters"),
            ("sam\u{7f}", "control characters"),
        ] {
            let err = parse_as(bad).expect_err(bad);
            assert!(err.contains(why), "{bad:?}: {err}");
        }
        let long = "a".repeat(129);
        assert!(parse_as(&long).unwrap_err().contains("128 bytes"));
        assert!(parse_as(&"a".repeat(128)).is_ok());
        for line in [
            "priors revoke",
            "priors accept --file f",
            "priors accept --as me",
            "priors check",
            "priors show --as me",
            "priors check --file f --as me",
            "priors revoke --as me --file f",
            "priors",
            "priors promote --as me",
        ] {
            assert!(parse(&argv(line)).is_err(), "{line}");
        }
        assert!(matches!(
            parse(&argv("priors accept --file f --as me"))
                .unwrap()
                .command,
            Command::Priors {
                action: PriorsAction::Accept,
                ..
            }
        ));
    }

    #[test]
    fn prefixed_verb_form_is_validated_too() {
        assert!(parse(&argv("meshloop:status --bogus")).is_err());
    }

    #[test]
    fn detached_resume_names_its_graph() {
        let err = parse(&argv("resume --detach")).expect_err("must name the graph");
        assert!(err.contains("--detach requires --graph"), "{err}");
        match parse(&argv("resume --detach --graph g"))
            .expect("parse")
            .command
        {
            Command::Resume { detach, graph, .. } => {
                assert!(detach);
                assert_eq!(graph.as_deref(), Some("g"));
            }
            _ => panic!("expected resume"),
        }
    }
}
