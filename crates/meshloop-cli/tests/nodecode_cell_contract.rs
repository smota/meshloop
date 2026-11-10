//! CLI contract for the optional nodecode cell (ADR 0038, #135).
//!
//! `cells/nodecode-meshloop/contract.json` names every field the cell reads from
//! `meshloop doctor|status|inspect --json`. This test runs those verbs on a fixture
//! repository, with the arguments the cell passes, and asserts each named field is there
//! with its type. A CLI change that breaks the cell's input fails here, with no nodecode
//! installed and no socket opened: git, the fixture harness and this crate's binary only.

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

struct Fixture {
    base: PathBuf,
    repo: PathBuf,
}

impl Fixture {
    fn new() -> Self {
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
verify_command = []
[harnesses.fixture]
executable = "{}"
version_args = ["--version"]
invoke_args_template = ["--prompt-file", "{{prompt_file}}"]
model_ref = "fixture-model"
model_tier = "top"
"#,
                fixture_harness()
                    .display()
                    .to_string()
                    .replace('\\', "\\\\")
            ),
        )
        .expect("config");
        fs::write(
            fixture.repo.join("plan.json"),
            r#"{"graph_id":"gcell","nodes":[{"id":1,"description":"first","depends_on":[],"tier":null},{"id":2,"description":"second","depends_on":[1],"tier":null}]}"#,
        )
        .expect("plan");
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
        let mut args: Vec<&str> = argv.iter().map(String::as_str).collect();
        args.extend(CELL_SUFFIX);
        let out = self.meshloop(&args);
        serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{args:?} did not answer JSON ({e}): {out:?}"))
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
        let steps: Vec<&str> = path.split('.').collect();
        // A `[]` step over an empty array would hold vacuously: the fixture must reach
        // at least one element.
        let reached = values_at(out, &steps);
        assert!(
            !reached.is_empty(),
            "{label}: {path} reached no element; the fixture must populate it\n{out:#}"
        );
        for value in reached {
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
    assert_eq!(
        out["command"],
        format!("meshloop:{verb}"),
        "{label}\n{out:#}"
    );
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
        BTreeSet::from(["doctor", "error", "inspect", "status", "status_empty"]),
        "every contract case must be exercised below"
    );
    let fixture = Fixture::new();

    // Before any run, status answers the empty shape.
    let argv = argv_of(&contract, "status_empty");
    assert_case(&contract, "status_empty", &argv, &fixture.as_cell(&argv));

    let run = fixture.meshloop(&[
        "run",
        "--plan",
        "plan.json",
        "--accept-plan",
        "--worktree-base",
        "worktrees",
    ]);
    assert!(run.status.success(), "fixture run: {run:?}");

    for name in ["doctor", "status", "inspect", "error"] {
        let argv = argv_of(&contract, name);
        assert_case(&contract, name, &argv, &fixture.as_cell(&argv));
    }
    // The cell passes `--graph ID` when the operator names a run.
    for name in ["status", "inspect"] {
        let mut argv = argv_of(&contract, name);
        argv.extend(["--graph".to_string(), "gcell".to_string()]);
        assert_case(&contract, name, &argv, &fixture.as_cell(&argv));
    }
}

#[test]
fn cell_contract_types_are_ones_the_cell_checks() {
    let contract = contract();
    let known = [
        "boolean", "string", "integer", "array", "object", "boolean?", "string?", "integer?",
        "array?", "object?",
    ];
    let mut lists = vec![&contract["envelope"]];
    for case in contract["cases"].as_object().expect("cases").values() {
        lists.push(&case["fields"]);
        assert!(case["ok"].is_boolean(), "{case}");
        assert!(
            case["argv"].as_array().is_some_and(|a| !a.is_empty()),
            "{case}"
        );
    }
    for fields in lists {
        for field in fields.as_array().expect("fields") {
            let ty = field["type"].as_str().expect("type");
            assert!(known.contains(&ty), "unknown contract type {ty:?}");
            assert!(field["path"].as_str().is_some_and(|p| !p.is_empty()));
        }
    }
}
