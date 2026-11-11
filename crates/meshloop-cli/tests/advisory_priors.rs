//! Advisory routing priors end to end (ADR 0038 #139 amendment; learning-plane.md
//! "Required regression tests"). Fixture harness, git and this crate's binary only:
//! no socket, no nodecode.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

fn bounded_output(command: &mut Command) -> Output {
    let mut child = meshloop_adapters::process::spawn_command(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .expect("spawn");
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        if child.try_wait().expect("wait").is_some() {
            return child.wait_with_output().expect("output");
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child.wait_with_output().expect("output");
            panic!("subprocess exceeded 90 seconds: {command:?}: {output:?}");
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
        "build meshloop-adapters first: {}",
        path.display()
    );
    path
}

struct Fixture {
    base: PathBuf,
    repo: PathBuf,
}

/// Two models of one fixture harness, so routing order decides which one runs.
const TWO_MODELS: &str = r#"models = [
  { model_ref = "model-a", model_tier = "top" },
  { model_ref = "model-b", model_tier = "top" },
]"#;
const ONE_MODEL: &str = r#"model_ref = "fixture-model"
model_tier = "top""#;
const PASSES_MODEL: &str = r#"["--model", "{model_ref}", "--prompt-file", "{prompt_file}"]"#;

impl Fixture {
    fn new(template: &str, models: &str, verify: &str, timeout_s: u32) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let base =
            std::env::temp_dir().join(format!("meshloop-priors-{}-{nonce}", std::process::id()));
        let repo = base.join("repo");
        fs::create_dir_all(&repo).expect("fixture dir");
        let f = Self { base, repo };
        f.git(&["init", "--quiet"]);
        fs::write(f.repo.join("README.md"), "seed\n").expect("seed");
        fs::write(f.repo.join(".gitignore"), "/.meshloop/\n*.json.proposal\n").expect("ignore");
        fs::write(
            f.repo.join("meshloop.toml"),
            format!(
                r#"selected_harnesses = ["fixture"]
[limits]
max_concurrent_workers = 1
max_retries = 2
task_timeout_seconds = {timeout_s}
[verify]
verify_command = {verify}
[harnesses.fixture]
executable = "{}"
version_args = ["--version"]
invoke_args_template = {template}
{models}
"#,
                fixture_harness()
                    .display()
                    .to_string()
                    .replace('\\', "\\\\"),
            ),
        )
        .expect("config");
        for (graph, tier) in [("g1", "Tier2"), ("g2", "Tier2"), ("g3", "Tier2")] {
            fs::write(
                f.repo.join(format!("{graph}.json")),
                format!(
                    r#"{{"graph_id":"{graph}","nodes":[{{"id":1,"description":"work","depends_on":[],"tier":"{tier}"}}]}}"#
                ),
            )
            .expect("plan");
        }
        f.git(&["add", "."]);
        f.git(&[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "seed",
        ]);
        f
    }

    fn git(&self, args: &[&str]) {
        // Fixed dates: the same tree gives the same commit in every fixture.
        let out = bounded_output(
            Command::new("git")
                .current_dir(&self.repo)
                .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
                .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z")
                .args(args),
        );
        assert!(out.status.success(), "git {args:?}: {out:?}");
    }

    fn meshloop(&self, args: &[&str]) -> Output {
        bounded_output(
            Command::new(env!("CARGO_BIN_EXE_meshloop"))
                .current_dir(&self.repo)
                .args(args),
        )
    }

    fn json(&self, args: &[&str]) -> Value {
        let out = self.meshloop(args);
        serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{args:?} did not answer JSON ({e}): {out:?}"))
    }

    fn run(&self, graph: &str) -> Output {
        let plan = format!("{graph}.json");
        self.meshloop(&[
            "run",
            "--plan",
            &plan,
            "--accept-plan",
            "--fixture-only",
            "--json",
        ])
    }

    fn attempts(&self, graph: &str) -> Vec<Value> {
        let v = self.json(&["inspect", "--graph", graph, "--attempts", "--json"]);
        v["data"]["nodes"][0]["attempts"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    fn write_proposal(&self, name: &str, value: &Value) -> PathBuf {
        let path = self.repo.join(name);
        fs::write(&path, serde_json::to_vec_pretty(value).expect("json")).expect("proposal");
        path
    }

    fn db(&self) -> PathBuf {
        self.repo.join(".meshloop").join("state.sqlite")
    }

    fn routing_rows(&self) -> Vec<(u32, String, bool)> {
        let conn = rusqlite::Connection::open(self.db()).expect("db");
        let mut stmt = conn
            .prepare("SELECT attempt_id, priors_sha256, changed_top FROM advisory_routing ORDER BY attempt_id")
            .expect("prepare");
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("rows")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        assert!(self.base.starts_with(std::env::temp_dir()));
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn entry(model: &str, tier: &str, passed: &[bool], graph: &str) -> Value {
    json!({
        "harness": "fixture",
        "model_ref": model,
        "tier": tier,
        "dispatch_mode": "subprocess",
        "attempts": passed.len(),
        "successes": passed.iter().filter(|p| **p).count(),
        "provenance": passed.iter().enumerate().map(|(i, p)| json!({
            "graph_id": graph,
            "task_id": 1,
            "attempt_id": 900 + i,
            "passed": p,
        })).collect::<Vec<_>>(),
    })
}

fn proposal(entries: Vec<Value>) -> Value {
    json!({
        "schema": "meshloop/advisory-priors",
        "version": 1,
        "generated_at": "2026-11-11T00:00:00Z",
        "generator": {"name": "test", "version": "1"},
        "priors": entries,
    })
}

/// LP tests 5 and 7: the read-only attempt fields on each terminal path, and the node
/// tier the fold keys on.
#[test]
fn attempt_fields_report_mode_check_result_and_model_passed() {
    // Checks pass; the template passes the model.
    let f = Fixture::new(PASSES_MODEL, ONE_MODEL, "[]", 30);
    f.run("g1");
    let a = f.attempts("g1");
    assert_eq!(a.len(), 1, "{a:#?}");
    assert_eq!(a[0]["dispatch_mode"], "subprocess");
    assert_eq!(a[0]["check_result"], "passed");
    assert_eq!(a[0]["model_passed"], true);
    let inspect = f.json(&["inspect", "--graph", "g1", "--attempts", "--json"]);
    assert_eq!(inspect["data"]["nodes"][0]["tier"], "Tier2");
    let one = f.json(&["inspect", "--graph", "g1", "--task", "1", "--json"]);
    assert_eq!(one["data"]["tier"], "Tier2");
    assert_eq!(one["data"]["attempts"][0]["check_result"], "passed");
    drop(f);

    // A failed check counts as a failure; the template does not pass the model.
    let f = Fixture::new(
        r#"["--prompt-file", "{prompt_file}"]"#,
        ONE_MODEL,
        r#"["git", "no-such-subcommand"]"#,
        30,
    );
    f.run("g1");
    let a = f.attempts("g1");
    assert_eq!(a[0]["outcome"], "DeterministicChecksFailed");
    assert_eq!(a[0]["check_result"], "failed");
    assert_eq!(a[0]["model_passed"], false);
    drop(f);

    // A timeout has no check result (`no_check_result`).
    let f = Fixture::new(r#"["--hang"]"#, ONE_MODEL, "[]", 1);
    f.run("g1");
    let a = f.attempts("g1");
    assert_eq!(a[0]["outcome"], "HarnessCrashedOrTimeout");
    assert_eq!(a[0]["check_result"], Value::Null);
    assert_eq!(a[0]["dispatch_mode"], "subprocess");
}

/// The run outputs the cell and operators read, with the fixture's path removed.
fn routed_outputs(f: &Fixture) -> (String, String, Vec<(String, String)>) {
    let normalize = |out: &Output| {
        String::from_utf8_lossy(&out.stdout).replace(&f.base.display().to_string(), "<BASE>")
    };
    let review = f.meshloop(&["review-plan", "--plan", "g1.json", "--accept", "--json"]);
    let run = f.meshloop(&["run", "--plan", "g1.json", "--fixture-only", "--json"]);
    let arms = f
        .attempts("g1")
        .iter()
        .map(|a| {
            (
                a["harness"].as_str().unwrap_or_default().to_string(),
                a["model_ref"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    (normalize(&review), normalize(&run), arms)
}

/// LP test 3: without a promoted row, routing order and the review-plan and run
/// outputs are byte-identical, with no row, a revoked row, or an unpromoted proposal.
#[test]
fn no_promoted_row_leaves_outputs_byte_identical() {
    let favour_b = proposal(vec![
        entry("model-a", "Tier2", &[false; 8], "far"),
        entry("model-b", "Tier2", &[true; 8], "far"),
    ]);

    let baseline = Fixture::new(PASSES_MODEL, TWO_MODELS, "[]", 30);
    let expected = routed_outputs(&baseline);
    assert_eq!(expected.2, [("fixture".to_string(), "model-a".to_string())]);
    assert!(expected.1.contains("\"ok\": true"), "{}", expected.1);

    let revoked = Fixture::new(PASSES_MODEL, TWO_MODELS, "[]", 30);
    let path = revoked.write_proposal("p.json.proposal", &favour_b);
    let path = path.to_str().expect("utf-8 path");
    assert!(
        revoked
            .meshloop(&["priors", "accept", "--file", path, "--as", "sam"])
            .status
            .success()
    );
    assert!(
        revoked
            .meshloop(&["priors", "revoke", "--as", "sam"])
            .status
            .success()
    );
    assert_eq!(routed_outputs(&revoked), expected);
    assert!(revoked.routing_rows().is_empty());

    let unpromoted = Fixture::new(PASSES_MODEL, TWO_MODELS, "[]", 30);
    unpromoted.write_proposal("p.json.proposal", &favour_b);
    assert_eq!(routed_outputs(&unpromoted), expected);
    assert!(unpromoted.routing_rows().is_empty());
}

/// LP tests 13, 16 and 18, revoke, and doctor: a promoted file moves the pick and
/// records `advisory_routing`; editing the file afterwards changes nothing; revoke
/// stops the priors at once.
#[test]
fn promotion_routes_records_and_revokes() {
    let f = Fixture::new(PASSES_MODEL, TWO_MODELS, "[]", 30);
    f.run("g1");
    let g1 = f.attempts("g1");
    assert_eq!(g1[0]["model_ref"], "model-a");
    assert_eq!(g1[0]["model_passed"], true);

    // Entry 0 favours model-b. Entry 1 claims g1's real attempt, which ran model-a,
    // for model-b on Tier1: `provenance_contradicts`, recorded in the row.
    let mut contradicted = entry("model-b", "Tier1", &[true; 5], "far");
    contradicted["provenance"][0] =
        json!({"graph_id": "g1", "task_id": 1, "attempt_id": 1, "passed": true});
    let file = proposal(vec![
        entry("model-a", "Tier2", &[false; 8], "far"),
        entry("model-b", "Tier2", &[true; 8], "far"),
        contradicted,
    ]);
    let path = f.write_proposal("p.json.proposal", &file);
    let path_s = path.to_str().expect("utf-8 path");

    let check = f.json(&["priors", "check", "--file", path_s, "--json"]);
    assert_eq!(check["command"], "meshloop:priors-check");
    assert_eq!(check["data"]["file_fault"], Value::Null);
    assert_eq!(check["data"]["usable"].as_array().map(Vec::len), Some(2));
    assert_eq!(
        check["data"]["ignored"][0]["code"],
        "provenance_contradicts"
    );
    assert_eq!(check["data"]["resolved_refs"], 1);

    let accepted = f.json(&[
        "priors", "accept", "--file", path_s, "--as", "sam", "--json",
    ]);
    assert_eq!(accepted["ok"], true, "{accepted:#}");
    let digest = accepted["data"]["priors_sha256"]
        .as_str()
        .expect("digest")
        .to_string();

    // LP test 13: the ledger holds the bytes read at accept; the file no longer matters.
    fs::write(&path, b"{ not json").expect("overwrite");
    let show = f.json(&["priors", "show", "--json"]);
    assert_eq!(show["command"], "meshloop:priors-show");
    assert_eq!(show["data"]["in_effect"], true);
    assert_eq!(show["data"]["priors_sha256"], digest.as_str());
    assert_eq!(show["data"]["row"]["identity"], "sam");
    assert_eq!(
        show["data"]["row"]["ignored"][0]["code"],
        "provenance_contradicts"
    );
    let in_effect: Vec<&str> = show["data"]["priors"]
        .as_array()
        .expect("priors")
        .iter()
        .filter_map(|p| p["model_ref"].as_str())
        .collect();
    assert_eq!(in_effect, ["model-a", "model-b"]);

    // LP test 18: routed while priors are in effect, the pick moved to model-b.
    f.run("g2");
    let g2 = f.attempts("g2");
    assert_eq!(g2[0]["model_ref"], "model-b");
    let rows = f.routing_rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, g2[0]["attempt_id"].as_u64().unwrap() as u32);
    assert_eq!(rows[0].1, digest);
    assert!(rows[0].2, "changed_top");
    let show = f.json(&["priors", "show", "--json"]);
    assert_eq!(show["data"]["routing"]["attempts"], 1);
    assert_eq!(show["data"]["routing"]["changed_top"], 1);

    // Revoke takes effect on the next read: g3 routes as the baseline and writes no row.
    let revoked = f.json(&["priors", "revoke", "--as", "sam", "--json"]);
    assert_eq!(revoked["command"], "meshloop:priors-revoke");
    assert_eq!(revoked["data"]["revoked_sha256"], digest.as_str());
    f.run("g3");
    assert_eq!(f.attempts("g3")[0]["model_ref"], "model-a");
    assert_eq!(f.routing_rows().len(), 1);
    let show = f.json(&["priors", "show", "--json"]);
    assert_eq!(show["data"]["in_effect"], false);
    assert_eq!(show["data"]["diagnostics"][0]["code"], "revoked");
    let doctor = f.json(&["doctor", "--json"]);
    assert!(
        doctor["data"]["warnings"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|w| w
                .as_str()
                .is_some_and(|w| w.contains("no priors are in effect"))),
        "{doctor:#}"
    );
}

/// LP tests 8 to 12 at the CLI: every loader case is a diagnostic and `check` exits 0;
/// `accept` refuses a file-level fault and records nothing.
#[test]
fn loader_cases_are_diagnostics_and_accept_refuses_file_faults() {
    let f = Fixture::new(PASSES_MODEL, TWO_MODELS, "[]", 30);
    let mut dup_ref = entry("model-a", "Tier1", &[true; 5], "far");
    dup_ref["provenance"][1] = dup_ref["provenance"][0].clone();
    let mut mismatch = entry("model-b", "Tier1", &[true; 5], "far");
    mismatch["successes"] = 4.into();
    let mut extra = entry("model-a", "Tier3", &[true; 5], "far");
    extra["pi"] = 0.99.into();
    let mut file = proposal(vec![
        entry("model-b", "Tier3", &[true; 4], "far"),
        entry("model-a", "Tier2", &[true; 5], "far"),
        entry("model-a", "Tier2", &[true; 6], "far"),
        dup_ref,
        mismatch,
        extra,
        entry("ghost-model", "Tier2", &[true; 5], "far"),
    ]);
    file["tier_hints"] = json!([]);
    let path = f.write_proposal("p.json.proposal", &file);
    let out = f.meshloop(&[
        "priors",
        "check",
        "--file",
        path.to_str().unwrap(),
        "--json",
    ]);
    assert!(out.status.success(), "{out:?}");
    let check: Value = serde_json::from_slice(&out.stdout).expect("json");
    let codes = |index: u64| -> Vec<String> {
        check["data"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| d["entry"] == index)
            .filter_map(|d| d["code"].as_str().map(String::from))
            .collect()
    };
    assert!(codes(0).contains(&"below_evidence_floor".to_string()));
    assert!(codes(1).contains(&"duplicate_key".to_string()));
    assert!(codes(2).contains(&"duplicate_key".to_string()));
    assert!(codes(3).contains(&"duplicate_provenance".to_string()));
    assert!(codes(4).contains(&"successes_mismatch".to_string()));
    assert!(codes(5).contains(&"unknown_entry_field".to_string()));
    assert!(codes(6).contains(&"not_a_candidate".to_string()));
    let usable: Vec<&str> = check["data"]["usable"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|u| u["model_ref"].as_str())
        .collect();
    assert_eq!(usable, ["ghost-model"]);
    assert!(
        check["data"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "unknown_field")
    );

    // File-level faults: check exits 0 with the fault; accept refuses and writes nothing.
    for (bytes, code) in [
        (b"{ nope".to_vec(), "not_json"),
        (
            serde_json::to_vec(&json!({"schema": "meshloop/advisory-priors", "version": 2}))
                .unwrap(),
            "wrong_version",
        ),
    ] {
        let path = f.repo.join("bad.json.proposal");
        fs::write(&path, bytes).expect("bad");
        let path = path.to_str().unwrap();
        let check = f.meshloop(&["priors", "check", "--file", path, "--json"]);
        assert!(check.status.success());
        let v: Value = serde_json::from_slice(&check.stdout).unwrap();
        assert_eq!(v["data"]["file_fault"], code);
        let accept = f.meshloop(&["priors", "accept", "--file", path, "--as", "sam", "--json"]);
        assert_eq!(accept.status.code(), Some(1), "{accept:?}");
        let v: Value = serde_json::from_slice(&accept.stdout).unwrap();
        assert_eq!(v["ok"], false);
        assert_eq!(v["data"]["file_fault"], code);
    }
    let show = f.json(&["priors", "show", "--json"]);
    assert_eq!(show["data"]["row"], Value::Null);
    assert_eq!(show["data"]["diagnostics"][0]["code"], "not_promoted");
}

/// LP test 17 at the CLI: a bad `--as` is a usage error and nothing is written.
#[test]
fn bad_identities_are_refused_before_anything_is_written() {
    let f = Fixture::new(PASSES_MODEL, ONE_MODEL, "[]", 30);
    let path = f.write_proposal("p.json.proposal", &proposal(vec![]));
    let path = path.to_str().unwrap();
    let long = "x".repeat(129);
    for bad in ["", "-sam", "sam\u{1b}", long.as_str()] {
        let out = f.meshloop(&["priors", "accept", "--file", path, "--as", bad]);
        assert_eq!(out.status.code(), Some(2), "{bad:?}: {out:?}");
        let out = f.meshloop(&["priors", "revoke", "--as", bad]);
        assert_eq!(out.status.code(), Some(2), "{bad:?}: {out:?}");
    }
    assert!(!Path::new(&f.db()).exists(), "no ledger was created");
}
