//! CLI contract for the optional nodecode cell (ADR 0038, #135, #136).
//!
//! `cells/nodecode-meshloop/contract.json` names every field the cell reads from
//! `meshloop <verb> --json`: the read-only views and the operator's plan, review-plan,
//! run, resume, cancel, accept and integrate. This test runs those verbs on fixture
//! repositories, in the order the cell's commands run them and with the arguments the
//! cell passes, and asserts each named field is there with its type. A CLI change that
//! breaks the cell's input fails here, with no nodecode installed and no socket opened:
//! git, the fixture harness and this crate's binary only.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

/// The arguments the cell appends to every verb (cells/nodecode-meshloop/cell.lisp, ASK).
const CELL_SUFFIX: [&str; 5] = [
    "--json",
    "--origin-harness",
    "nodecode",
    "--origin-session",
    "s-CONTRACT",
];

fn contract() -> Value {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../cells/nodecode-meshloop/contract.json");
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read cell contract {}: {e}", path.display()));
    serde_json::from_str(&text).expect("cell contract is JSON")
}

fn bounded_output(command: &mut Command) -> Output {
    let mut child = meshloop_adapters::process::spawn_command(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .expect("spawn");
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if child.try_wait().expect("wait").is_some() {
            return child.wait_with_output().expect("output");
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child.wait_with_output().expect("output");
            panic!("subprocess exceeded 60 seconds: {command:?}: {output:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn fixture_harness() -> PathBuf {
    let mut dir = std::env::current_exe().expect("test executable path");
    dir.pop();
    dir.pop();
    let path = dir.join(format!("fixture_harness{}", std::env::consts::EXE_SUFFIX));
    assert!(
        path.exists(),
        "expected fixture harness at {}; build meshloop-adapters first",
        path.display()
    );
    path
}

/// Which harness the fixture repository's config runs.
#[derive(Clone, Copy)]
enum Harness {
    /// Work nodes: the fixture writes a file per prompt.
    Worker,
    /// The planner: the fixture answers a canned two-node graph.
    Planner,
    /// Work nodes that take four seconds, so a driver is seen alive.
    Slow,
    /// A no-op worker whose verify command cancels the graph: `Cancel` lands while
    /// the attempt is `Verifying`, as in the issue #167 race.
    CancelInVerify,
}

struct Fixture {
    base: PathBuf,
    repo: PathBuf,
}

impl Fixture {
    fn new(harness: Harness) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "meshloop-cell-contract-{}-{nonce}",
            std::process::id()
        ));
        let repo = base.join("repo");
        fs::create_dir_all(&repo).expect("fixture dir");
        let fixture = Self { base, repo };
        fixture.git(&["init", "--quiet"]);
        fs::write(fixture.repo.join("README.md"), "seed\n").expect("seed");
        fs::write(
            fixture.repo.join(".gitignore"),
            "/.meshloop/\n/worktrees/\n",
        )
        .expect("ignore");
        fs::write(
            fixture.repo.join("meshloop.toml"),
            format!(
                r#"selected_harnesses = ["fixture"]
[limits]
max_concurrent_workers = 1
max_retries = 2
task_timeout_seconds = 30
[verify]
verify_command = {}
[harnesses.fixture]
executable = "{}"
version_args = ["--version"]
invoke_args_template = {}
model_ref = "fixture-model"
model_tier = "top"
"#,
                match harness {
                    Harness::CancelInVerify => {
                        let path = |p: &Path| format!("{:?}", p.display().to_string());
                        format!(
                            "[{}, \"cancel\", \"--graph\", \"gcell\", \"--config\", {}, \"--db\", {}, \"--worktree-base\", {}]",
                            path(Path::new(env!("CARGO_BIN_EXE_meshloop"))),
                            path(&fixture.repo.join("meshloop.toml")),
                            path(&fixture.repo.join(".meshloop").join("state.sqlite")),
                            path(&fixture.repo.join(".meshloop").join("worktrees")),
                        )
                    }
                    _ => "[]".to_string(),
                },
                fixture_harness()
                    .display()
                    .to_string()
                    .replace('\\', "\\\\"),
                match harness {
                    Harness::Worker => r#"["--prompt-file", "{prompt_file}"]"#,
                    Harness::Planner => r#"["--emit-graph"]"#,
                    Harness::Slow => r#"["--prompt-file", "{prompt_file}", "4000"]"#,
                    Harness::CancelInVerify => r#"["--noop"]"#,
                }
            ),
        )
        .expect("config");
        fs::write(
            fixture.repo.join("plan.json"),
            r#"{"graph_id":"gcell","nodes":[{"id":1,"description":"first","depends_on":[],"tier":null},{"id":2,"description":"second","depends_on":[1],"tier":"Tier2"}]}"#,
        )
        .expect("plan");
        fs::write(
            fixture.repo.join("plan-cancel.json"),
            r#"{"graph_id":"gcancel","nodes":[{"id":1,"description":"to cancel","depends_on":[],"tier":null}]}"#,
        )
        .expect("cancel plan");
        fixture.git(&["add", "."]);
        fixture.git(&[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "seed",
        ]);
        fixture
    }

    fn git(&self, args: &[&str]) {
        let out = bounded_output(Command::new("git").current_dir(&self.repo).args(args));
        assert!(out.status.success(), "git {args:?}: {out:?}");
    }

    fn meshloop(&self, args: &[&str]) -> Output {
        bounded_output(
            Command::new(env!("CARGO_BIN_EXE_meshloop"))
                .current_dir(&self.repo)
                .args(args),
        )
    }

    /// The verb as the cell runs it: argv, then the cell's suffix, in the repository root.
    fn as_cell(&self, argv: &[String]) -> Value {
        let out = self.as_cell_raw(argv);
        serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{argv:?} did not answer JSON ({e}): {out:?}"))
    }

    /// The verb as the cell runs it, for a verb that answers text (accept, integrate).
    fn as_cell_raw(&self, argv: &[String]) -> Output {
        let mut args: Vec<&str> = argv.iter().map(String::as_str).collect();
        args.extend(CELL_SUFFIX);
        self.meshloop(&args)
    }

    /// `status --graph GRAPH --json` once `done` holds of it, polled for 60 seconds.
    fn wait_for(&self, graph: &str, what: &str, done: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let out = self.meshloop(&["status", "--graph", graph, "--json"]);
            let status: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
            if done(&status) {
                return status;
            }
            assert!(Instant::now() < deadline, "{what} within 60 s: {status:#}");
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// Until node `task` of `graph` is in `state` and the graph's driver is no longer alive.
    fn wait_for_node(&self, graph: &str, task: u64, state: &str) -> Value {
        self.wait_for(graph, &format!("{graph} #{task} {state}"), |status| {
            let node_ready = status["data"]["nodes"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|n| n["id"] == task && n["state"] == state);
            let driver_done = status["data"]["driver"]
                .as_str()
                .is_none_or(|d| d != "alive");
            node_ready && driver_done
        })
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        assert!(self.base.starts_with(std::env::temp_dir()));
        let _ = fs::remove_dir_all(&self.base);
    }
}

/// The values a contract path reaches, `None` where a step is missing; a step ending in
/// `[]` reaches every element of that array (cell.lisp, FIELD-VALUES).
fn values_at<'a>(value: &'a Value, steps: &[&str]) -> Vec<Option<&'a Value>> {
    let Some((step, rest)) = steps.split_first() else {
        return vec![Some(value)];
    };
    let (key, each) = match step.strip_suffix("[]") {
        Some(key) => (key, true),
        None => (*step, false),
    };
    match (value.get(key), each) {
        (None, _) => vec![None],
        (Some(Value::Array(items)), true) => items
            .iter()
            .flat_map(|item| values_at(item, rest))
            .collect(),
        (Some(_), true) => vec![None],
        (Some(next), false) => values_at(next, rest),
    }
}

/// Whether `value` is of the contract's `ty`; a type ending in `?` also takes null
/// (cell.lisp, TYPE-FITS-P).
fn type_fits(value: &Value, ty: &str) -> bool {
    let (base, nullable) = match ty.strip_suffix('?') {
        Some(base) => (base, true),
        None => (ty, false),
    };
    (nullable && value.is_null())
        || match base {
            "boolean" => value.is_boolean(),
            "string" => value.is_string(),
            "integer" => value.is_i64() || value.is_u64(),
            "array" => value.is_array(),
            "object" => value.is_object(),
            other => panic!("contract type {other:?} is not one the cell reads"),
        }
}

fn assert_fields(fields: &Value, out: &Value, label: &str) {
    for field in fields.as_array().expect("fields array") {
        let path = field["path"].as_str().expect("path");
        let ty = field["type"].as_str().expect("type");
        let optional = field["optional"].as_bool().unwrap_or(false);
        let steps: Vec<&str> = path.split('.').collect();
        // A `[]` step over an empty array would hold vacuously: the fixture must reach
        // at least one element.
        let reached = values_at(out, &steps);
        assert!(
            !reached.is_empty(),
            "{label}: {path} reached no element; the fixture must populate it\n{out:#}"
        );
        for value in reached {
            if optional && value.is_none() {
                continue;
            }
            let value = value.unwrap_or_else(|| panic!("{label}: missing {path} ({ty})\n{out:#}"));
            assert!(
                type_fits(value, ty),
                "{label}: {path} is {value}, not {ty}\n{out:#}"
            );
        }
    }
}

fn assert_case(contract: &Value, name: &str, argv: &[String], out: &Value) {
    let case = &contract["cases"][name];
    let verb = &argv[0];
    let label = format!("case {name} ({argv:?})");
    assert_fields(&contract["envelope"], out, &label);
    let command = case["command"]
        .as_str()
        .map_or_else(|| format!("meshloop:{verb}"), str::to_string);
    assert_eq!(out["command"], command, "{label}\n{out:#}");
    assert_eq!(out["ok"], case["ok"], "{label}\n{out:#}");
    assert_fields(&case["fields"], out, &label);
}

fn argv_of(contract: &Value, name: &str) -> Vec<String> {
    contract["cases"][name]["argv"]
        .as_array()
        .unwrap_or_else(|| panic!("case {name} has an argv"))
        .iter()
        .map(|a| a.as_str().expect("argv string").to_string())
        .collect()
}

/// The acceptance meshloop recorded for `task`, read back as the cell reads it.
fn acceptance_of(inspect: &Value, task: u64) -> Option<String> {
    let node = inspect["data"]["nodes"]
        .as_array()?
        .iter()
        .find(|n| n["id"] == task)?;
    node["attempts"].as_array()?.last()?["evidence"]
        .as_array()?
        .iter()
        .find(|e| e["kind"] == "human_acceptance")?["summary_redacted"]
        .as_str()
        .map(str::to_string)
}

#[test]
fn cli_json_carries_every_field_the_nodecode_cell_reads() {
    let contract = contract();
    let names: BTreeSet<&str> = contract["cases"]
        .as_object()
        .expect("cases")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        names,
        BTreeSet::from([
            "cancel",
            "doctor",
            "error",
            "events",
            "inspect",
            "inspect_attempts",
            "integrate_preview",
            "plan",
            "resume_detached",
            "resume_driver_alive",
            "review_plan",
            "run_detached",
            "status",
            "status_empty",
            "workspace_inventory",
        ]),
        "every contract case must be exercised below"
    );
    let fixture = Fixture::new(Harness::Worker);
    let case = |name: &str| {
        let argv = argv_of(&contract, name);
        let out = fixture.as_cell(&argv);
        assert_case(&contract, name, &argv, &out);
        out
    };
    let graph_fields = &contract["graph"];
    let plan: Value =
        serde_json::from_str(&fs::read_to_string(fixture.repo.join("plan.json")).expect("plan"))
            .expect("plan JSON");
    assert_fields(graph_fields, &plan, "plan file");

    // Before any run, status answers the empty shape.
    case("status_empty");

    // /meshloop review-plan --accept --as, then /meshloop run (always detached).
    let review = case("review_plan");
    assert_eq!(review["data"]["as"], "contract-person");
    assert_fields(
        graph_fields,
        &review["data"]["graph"],
        "review-plan data.graph",
    );
    case("run_detached");
    fixture.wait_for_node("gcell", 1, "AwaitingReview");

    for name in [
        "doctor",
        "status",
        "inspect",
        "error",
        "inspect_attempts",
        "integrate_preview",
    ] {
        case(name);
    }
    // The cell passes `--graph ID` when the operator names a run.
    for name in ["status", "inspect", "inspect_attempts"] {
        let mut argv = argv_of(&contract, name);
        argv.extend(["--graph".to_string(), "gcell".to_string()]);
        assert_case(&contract, name, &argv, &fixture.as_cell(&argv));
    }

    // /meshloop accept answers text; the cell reads the acceptance back from inspect.
    let accept: Vec<String> = [
        "accept",
        "--task",
        "1",
        "--as",
        "contract-person",
        "--graph",
        "gcell",
    ]
    .map(String::from)
    .to_vec();
    let out = fixture.as_cell_raw(&accept);
    assert!(out.status.success(), "accept: {out:?}");
    let argv: Vec<String> = ["inspect", "--attempts", "--graph", "gcell"]
        .map(String::from)
        .to_vec();
    let inspect = fixture.as_cell(&argv);
    assert_case(&contract, "inspect_attempts", &argv, &inspect);
    let recorded = acceptance_of(&inspect, 1).expect("acceptance evidence for #1");
    assert!(recorded.contains("contract-person"), "{recorded}");

    // /meshloop resume: detached, as run.
    case("resume_detached");
    fixture.wait_for_node("gcell", 2, "AwaitingReview");

    // The Board projection (#137): every transition of gcell, each with its stable id,
    // read after a cursor. The last event per node is the state status reports.
    let events = case("events");
    let rows = events["data"]["events"].as_array().expect("events");
    let seqs: Vec<i64> = rows
        .iter()
        .map(|e| e["seq"].as_i64().expect("seq"))
        .collect();
    assert!(seqs.windows(2).all(|w| w[0] < w[1]), "{seqs:?}");
    assert_eq!(events["data"]["last_seq"], *seqs.last().expect("an event"));
    let status = fixture.as_cell(&["status", "--graph", "gcell"].map(String::from));
    for node in status["data"]["nodes"].as_array().expect("nodes") {
        let last = rows.iter().rev().find(|e| e["task"] == node["id"]);
        assert_eq!(last.map(|e| &e["to"]), Some(&node["state"]), "{events:#}");
    }
    let tail = fixture.as_cell(
        &[
            "events",
            "--graph",
            "gcell",
            "--after",
            &seqs[seqs.len() - 2].to_string(),
        ]
        .map(String::from),
    );
    assert_eq!(
        tail["data"]["events"].as_array().map(Vec::len),
        Some(1),
        "{tail:#}"
    );
    let none = fixture.as_cell(
        &[
            "events",
            "--graph",
            "gcell",
            "--after",
            &seqs[seqs.len() - 1].to_string(),
        ]
        .map(String::from),
    );
    assert_eq!(none["data"]["events"], serde_json::json!([]), "{none:#}");
    assert_eq!(none["data"]["last_seq"], *seqs.last().expect("an event"));
    assert_eq!(none["data"]["more"], false, "{none:#}");
    // --limit is applied in SQL; `more` says the rest wait past last_seq.
    let first = fixture.as_cell(&["events", "--graph", "gcell", "--limit", "1"].map(String::from));
    assert_eq!(
        first["data"]["events"].as_array().map(Vec::len),
        Some(1),
        "{first:#}"
    );
    assert_eq!(
        (&first["data"]["more"], &first["data"]["last_seq"]),
        (&Value::from(true), &Value::from(seqs[0])),
        "{first:#}"
    );
    assert!(
        events["data"]["max_seq"].as_i64() >= seqs.last().copied(),
        "{events:#}"
    );

    // /meshloop cancel on a second run, and the worktrees meshloop keeps for it.
    for argv in [
        vec![
            "review-plan",
            "--plan",
            "plan-cancel.json",
            "--accept",
            "--as",
            "contract-person",
        ],
        vec!["run", "--plan", "plan-cancel.json", "--detach"],
    ] {
        let argv: Vec<String> = argv.into_iter().map(String::from).collect();
        assert_eq!(fixture.as_cell(&argv)["ok"], true, "{argv:?}");
    }
    let cancel = case("cancel");
    assert_eq!(cancel["data"]["status"], "cancelled");
    fixture.wait_for("gcancel", "gcancel's driver to exit", |status| {
        status["data"]["driver"]
            .as_str()
            .is_none_or(|d| d != "alive")
    });
    let inventory = case("workspace_inventory");
    assert!(
        inventory["data"]["entries"]
            .as_array()
            .is_some_and(|e| e.iter().any(|e| e["graph_id"] == "gcancel")),
        "{inventory:#}"
    );

    // A second driver is refused while one is alive: resume and run alike.
    let slow = Fixture::new(Harness::Slow);
    for argv in [
        vec![
            "review-plan",
            "--plan",
            "plan.json",
            "--accept",
            "--as",
            "contract-person",
        ],
        vec!["run", "--plan", "plan.json", "--detach"],
    ] {
        let argv: Vec<String> = argv.into_iter().map(String::from).collect();
        assert_eq!(slow.as_cell(&argv)["ok"], true, "{argv:?}");
    }
    slow.wait_for("gcell", "the slow driver to be alive", |status| {
        status["data"]["driver"] == "alive"
    });
    let argv = argv_of(&contract, "resume_driver_alive");
    let refused = slow.as_cell(&argv);
    assert_case(&contract, "resume_driver_alive", &argv, &refused);
    assert!(
        refused["error"]
            .as_str()
            .is_some_and(|e| e.contains("a driver is running for gcell")),
        "{refused:#}"
    );
    let rerun: Vec<String> = ["run", "--plan", "plan.json", "--detach"]
        .map(String::from)
        .to_vec();
    let refused = slow.as_cell(&rerun);
    assert_eq!(refused["ok"], false, "{refused:#}");
    assert!(
        refused["error"]
            .as_str()
            .is_some_and(|e| e.contains("a driver is running for gcell")),
        "{refused:#}"
    );
    let status = slow.wait_for_node("gcell", 1, "AwaitingReview");
    assert_eq!(
        status["data"]["awaiting_acceptance"],
        serde_json::json!(["gcell"])
    );

    // /meshloop plan, on a repository whose harness plans.
    let planner = Fixture::new(Harness::Planner);
    let argv = argv_of(&contract, "plan");
    let out = planner.as_cell(&argv);
    assert_case(&contract, "plan", &argv, &out);
    assert_fields(graph_fields, &out["data"]["graph"], "plan data.graph");
    let written: Value = serde_json::from_str(
        &fs::read_to_string(planner.repo.join("planned.json")).expect("planned.json"),
    )
    .expect("planned JSON");
    assert_fields(graph_fields, &written, "planned.json");
}

/// #137: a ledger written before #167 holds the cancelled attempt's stale check result
/// after `Cancel`. Every reader's fold skips it, so `events` marks it superseded and
/// its folded `nodes` agree with status: a projection never posts a state status does
/// not show, nor wakes anyone for it.
#[test]
fn events_mark_a_pre_167_stale_row_superseded_and_fold_as_status_does() {
    let contract = contract();
    let fixture = Fixture::new(Harness::CancelInVerify);
    // The run ends when its verify command cancels the graph; its exit is not the point.
    let _ = fixture.meshloop(&["run", "--plan", "plan.json", "--accept-plan"]);
    let argv: Vec<String> = ["events", "--graph", "gcell"].map(String::from).to_vec();
    let before = fixture.as_cell(&argv);
    let cancel = before["data"]["events"]
        .as_array()
        .expect("events")
        .iter()
        .find(|e| e["event"] == "Cancel" && e["task"] == 1)
        .cloned()
        .unwrap_or_else(|| panic!("a Cancel row for #1: {before:#}"));
    assert_eq!(cancel["from"], "Verifying", "{before:#}");
    // The exact row the pre-fix driver appended after the cancel (cancel_race.rs).
    let conn = rusqlite::Connection::open(fixture.repo.join(".meshloop").join("state.sqlite"))
        .expect("open ledger");
    conn.execute(
        "INSERT INTO events (graph_id, task_id, attempt_id, from_state, to_state, event_type, executor, occurred_at)
         VALUES ('gcell', 1, ?1, 'Verifying', 'Failed', 'DeterministicChecksFailed', 'meshloop', '0')",
        [cancel["attempt"].as_i64().expect("attempt")],
    )
    .expect("insert the stale row");
    drop(conn);
    let mut cell_argv = argv.clone();
    cell_argv.extend(["--after", "0", "--limit", "200"].map(String::from));
    let events = fixture.as_cell(&cell_argv);
    assert_case(&contract, "events", &cell_argv, &events);
    let rows = events["data"]["events"].as_array().expect("events");
    let stale = rows.last().expect("the stale row");
    assert_eq!(
        (&stale["to"], &stale["superseded"]),
        (&Value::from("Failed"), &Value::from(true)),
        "{events:#}"
    );
    assert!(
        rows[..rows.len() - 1]
            .iter()
            .all(|e| e["superseded"] == false),
        "{events:#}"
    );
    assert_eq!(events["data"]["nodes"]["1"], "Cancelled", "{events:#}");
    assert_eq!(
        events["data"]["latest"]["1"], cancel["seq"],
        "the newest row the fold counts for #1 is the Cancel"
    );
    let status = fixture.as_cell(&["status", "--graph", "gcell"].map(String::from));
    let node = status["data"]["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .find(|n| n["id"] == 1)
        .cloned()
        .expect("node 1");
    assert_eq!(node["state"], events["data"]["nodes"]["1"], "{status:#}");
    assert_eq!(
        events["data"]["max_seq"], stale["seq"],
        "the stale row is the ledger's newest"
    );
}

#[test]
fn cell_contract_types_are_ones_the_cell_checks() {
    let contract = contract();
    let known = [
        "boolean", "string", "integer", "array", "object", "boolean?", "string?", "integer?",
        "array?", "object?",
    ];
    let mut lists = vec![&contract["envelope"], &contract["graph"]];
    for case in contract["cases"].as_object().expect("cases").values() {
        lists.push(&case["fields"]);
        assert!(case["ok"].is_boolean(), "{case}");
        assert!(
            case["argv"].as_array().is_some_and(|a| !a.is_empty()),
            "{case}"
        );
        assert!(
            case.get("command")
                .is_none_or(|c| c.as_str().is_some_and(|c| c.starts_with("meshloop:"))),
            "{case}"
        );
    }
    for fields in lists {
        for field in fields.as_array().expect("fields") {
            let ty = field["type"].as_str().expect("type");
            assert!(known.contains(&ty), "unknown contract type {ty:?}");
            assert!(field["path"].as_str().is_some_and(|p| !p.is_empty()));
            assert!(
                field.get("optional").is_none_or(Value::is_boolean),
                "{field}"
            );
        }
    }
}
