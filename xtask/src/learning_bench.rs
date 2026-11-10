//! Learning-plane benchmark (issue #144): a fixed task set run through the full
//! Meshloop lifecycle with live harnesses, to record the baseline that #140 compares
//! against.
//!
//! Live dispatch happens only with `--live`. Nothing here runs from `xtask check`,
//! `cargo test` or CI: offline CI stays socket-free. `--validate` is offline (each seed
//! fails its hidden tests and passes them with the reference solution). `--aggregate`
//! recomputes the summary of an existing results file.
//!
//! Protocol per task and repetition: fresh disposable repository from the seed,
//! `review-plan --accept` -> `run` -> `accept` (only when Meshloop's own checks passed)
//! -> `resume` -> `integrate --into main`, then the hidden tests on `main` decide
//! success by exit code. The disposable repository is deleted afterwards.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const SCHEMA: &str = "meshloop.bench.learning-plane.v1";
const VERIFY_PLACEHOLDER: &str = "verify_command = []";
const GIT_IDENTITY: [&str; 4] = [
    "-c",
    "user.name=meshloop-bench",
    "-c",
    "user.email=bench@localhost",
];
/// Lower-cased fragments that mark a vendor usage or rate limit in worker output.
const QUOTA_MARKERS: [&str; 7] = [
    "usage limit",
    "rate limit",
    "rate_limit",
    "quota",
    "hit your limit",
    "limit reached",
    "credit balance",
];

const USAGE: &str = "Usage: cargo run -p xtask -- bench-learning \
(--live [--reps N] [--out DIR] [--bin PATH] [--bin-commit SHA] [--tasks a,b] \
[--run-timeout SECS] [--results PATH] [--as IDENTITY] | --validate | --aggregate RESULTS.json [--results PATH])";

const PROTOCOL: &str = "Fixed automated protocol, no human in the loop: review-plan --accept \
of a one-node plan with the task's planned tier, run, then resume --retry while the node is \
Failed and attempts remain (up to max_retries), accept --task 1 only when Meshloop's own \
checks passed (node AwaitingReview), resume, integrate --into main --accept-integrate; success \
is the exit code of the task's hidden tests run on main after integration. The plan is never adjusted or declined, so Adjust/Decline = 0 \
by protocol, not by measurement of a human.";

const STATE_CARRIED: &str = "Each task x repetition runs in a new disposable Git repository \
with a new Meshloop store, so routing_feedback, quota_state, attempts and events start empty \
for every task (checked and recorded per run as routing_feedback_before). Nothing Meshloop \
stores carries across tasks or repetitions. Not controlled: vendor-side state outside the \
repository (harness CLI login, user-level config and instruction files, vendor caches and \
account quota windows).";

#[derive(Debug, Clone, Deserialize)]
struct TaskSet {
    version: String,
    tasks: Vec<String>,
    /// The tier a plan reviewer sets per task (lp-2). Absent: `tier = null` (lp-1).
    #[serde(default)]
    plan_tiers: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
struct TaskMeta {
    id: String,
    language: String,
    difficulty: String,
    kind: String,
    description: String,
    #[allow(dead_code)]
    test_command: Vec<String>,
    /// Files SPEC.md lets the worker change (lp-3).
    #[serde(default)]
    editable: Vec<String>,
    /// Number of hidden tests verify.py requires to pass (lp-4).
    #[serde(default)]
    expected_tests: u32,
}

#[derive(Debug, Default)]
struct Options {
    live: bool,
    validate: bool,
    aggregate: Option<PathBuf>,
    reps: u32,
    out: Option<PathBuf>,
    bin: Option<PathBuf>,
    bin_commit: Option<String>,
    tasks: Option<Vec<String>>,
    run_timeout_s: u64,
    results: Option<PathBuf>,
    accept_as: String,
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        reps: 1,
        run_timeout_s: 1560,
        accept_as: "claude-coordinator".into(),
        ..Options::default()
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = |name: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match a.as_str() {
            "--live" => o.live = true,
            "--validate" => o.validate = true,
            "--aggregate" => o.aggregate = Some(PathBuf::from(value(a)?)),
            "--reps" => {
                o.reps = value(a)?
                    .parse()
                    .map_err(|_| "--reps must be a positive integer".to_string())?;
                if o.reps == 0 || o.reps > 10 {
                    return Err("--reps must be 1..=10".into());
                }
            }
            "--out" => o.out = Some(PathBuf::from(value(a)?)),
            "--bin" => o.bin = Some(PathBuf::from(value(a)?)),
            "--bin-commit" => o.bin_commit = Some(value(a)?),
            "--tasks" => {
                o.tasks = Some(value(a)?.split(',').map(|s| s.trim().to_string()).collect())
            }
            "--run-timeout" => {
                o.run_timeout_s = value(a)?
                    .parse()
                    .map_err(|_| "--run-timeout must be seconds".to_string())?
            }
            "--results" => o.results = Some(PathBuf::from(value(a)?)),
            "--as" => o.accept_as = value(a)?,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let modes = usize::from(o.live) + usize::from(o.validate) + usize::from(o.aggregate.is_some());
    if modes != 1 {
        return Err("choose exactly one of --live, --validate, --aggregate".into());
    }
    Ok(o)
}

pub fn bench_learning(root: &Path, args: &[String]) -> ExitCode {
    let opts = match parse_args(args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("bench-learning: {e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let lp = root.join("benches").join("learning-plane");
    if let Some(path) = &opts.aggregate {
        return aggregate_file(root, path, opts.results.as_deref());
    }
    let (set, tasks) = match load_tasks(&lp, opts.tasks.as_deref()) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("bench-learning: {e}");
            return ExitCode::FAILURE;
        }
    };
    if opts.validate {
        return validate(&lp, &tasks);
    }
    live(root, &lp, &set, &tasks, &opts)
}

fn load_tasks(lp: &Path, only: Option<&[String]>) -> Result<(TaskSet, Vec<TaskMeta>), String> {
    let set: TaskSet = read_json(&lp.join("taskset.json"))?;
    let mut tasks = Vec::new();
    for id in &set.tasks {
        if only.is_some_and(|o| !o.iter().any(|x| x == id)) {
            continue;
        }
        let meta: TaskMeta = read_json(&lp.join("tasks").join(id).join("task.json"))?;
        if &meta.id != id {
            return Err(format!("task.json id {} does not match {id}", meta.id));
        }
        tasks.push(meta);
    }
    if let Some(o) = only {
        for id in o {
            if !set.tasks.contains(id) {
                return Err(format!("unknown task {id}"));
            }
        }
    }
    for (id, tier) in &set.plan_tiers {
        if !set.tasks.contains(id) {
            return Err(format!("plan_tiers names unknown task {id}"));
        }
        if !matches!(tier.as_str(), "Tier1" | "Tier2" | "Tier3") {
            return Err(format!(
                "plan_tiers.{id}: {tier} is not Tier1, Tier2 or Tier3"
            ));
        }
    }
    if tasks.is_empty() {
        return Err("no tasks selected".into());
    }
    Ok((set, tasks))
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// Offline validation
// ---------------------------------------------------------------------------

fn validate(lp: &Path, tasks: &[TaskMeta]) -> ExitCode {
    let mut ok = true;
    for t in tasks {
        let task_dir = lp.join("tasks").join(&t.id);
        let tmp =
            std::env::temp_dir().join(format!("ml-lp-validate-{}-{}", std::process::id(), t.id));
        let _ = fs::remove_dir_all(&tmp);
        if let Err(e) = copy_dir(&task_dir.join("seed"), &tmp) {
            eprintln!("{}: cannot copy seed: {e}", t.id);
            ok = false;
            continue;
        }
        let verify = lp.join("verify.py");
        let seed_exit = run_verify(&verify, &task_dir, &tmp, false);
        let reference_exit = run_verify(&verify, &task_dir, &tmp, true);
        let mut pass = seed_exit.is_some_and(|c| c == 1) && reference_exit == Some(0);
        println!(
            "  [{}] {:<18} seed exit {:?} (want 1), reference exit {:?} (want 0)",
            if seed_exit == Some(1) && reference_exit == Some(0) {
                "PASS"
            } else {
                "FAIL"
            },
            t.id,
            seed_exit,
            reference_exit
        );
        // Tamper cases: an unsolved seed plus a file that would fake a pass under a naive
        // driver must still fail verification (lp-3).
        for (lang, name, file, body) in TAMPER_CASES {
            if t.language != *lang {
                continue;
            }
            let file = if *file == "<editable>" {
                match t.editable.first() {
                    Some(f) => f.as_str(),
                    None => {
                        println!("  [FAIL] {:<18} tamper {name}: no editable file", t.id);
                        pass = false;
                        continue;
                    }
                }
            } else {
                file
            };
            let body = body.replace("<expected>", &t.expected_tests.to_string());
            let case = tmp.with_extension(name);
            let _ = fs::remove_dir_all(&case);
            let exit = copy_dir(&tmp, &case)
                .and_then(|_| {
                    if let Some(parent) = case.join(file).parent() {
                        fs::create_dir_all(parent)?;
                    }
                    fs::write(case.join(file), &body)
                })
                .ok()
                .and_then(|_| run_verify(&verify, &task_dir, &case, false));
            let _ = fs::remove_dir_all(&case);
            let case_pass = exit == Some(1);
            pass &= case_pass;
            println!(
                "  [{}] {:<18} tamper {name}: exit {exit:?} (want 1)",
                if case_pass { "PASS" } else { "FAIL" },
                t.id
            );
        }
        let _ = fs::remove_dir_all(&tmp);
        ok &= pass;
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// (case name, file written into an unsolved seed, content). Each would make a naive
/// `python3 -m unittest` in the candidate tree report success.
const TAMPER_CASES: &[(&str, &str, &str, &str)] = &[
    (
        "python",
        "shadow-unittest",
        "unittest.py",
        "import sys\nprint('OK')\nsys.exit(0)\n",
    ),
    (
        "python",
        "sitecustomize",
        "sitecustomize.py",
        "import os\nos._exit(0)\n",
    ),
    (
        "python",
        "conftest",
        "conftest.py",
        "import os\nos._exit(0)\n",
    ),
    ("python", "pth", "zz-hook.pth", "import os; os._exit(0)\n"),
    (
        "python",
        "edited-tests",
        "tests/test_visible.py",
        "import unittest\n\nclass T(unittest.TestCase):\n    def test_ok(self):\n        pass\n",
    ),
    (
        "rust",
        "edited-tests",
        "tests/hidden.rs",
        "#[test]\nfn ok() {}\n",
    ),
    (
        "rust",
        "edited-tests-visible",
        "tests/visible.rs",
        "#[test]\nfn ok() {}\n",
    ),
    // lp-4: the editable module runs inside the test process. `<editable>` is the task's
    // editable file and `<expected>` its expected_tests count.
    (
        "python",
        "exit-at-import",
        "<editable>",
        "import os\nos._exit(0)\n",
    ),
    (
        "python",
        "forged-result",
        "<editable>",
        "import os, sys\nsys.stdout.write('RESULT ok tests=<expected> nonce=x\\n')\nsys.stdout.flush()\nos._exit(0)\n",
    ),
    (
        "python",
        "read-stdin",
        "<editable>",
        "import os, sys\nn = ''\ntry:\n    n = sys.stdin.read()\nexcept Exception:\n    pass\ntry:\n    n = n or os.read(0, 64).decode()\nexcept Exception:\n    pass\nsys.stdout.write('RESULT ok tests=<expected> nonce=%s\\n' % n.strip())\nsys.stdout.flush()\nos._exit(0)\n",
    ),
    (
        "rust",
        "exit-with-forged-summary",
        "<editable>",
        "#[derive(Debug, Clone, Copy, PartialEq, Eq)]\npub enum Mismatch {\n    Unexpected { at: usize, found: char },\n    Unclosed { at: usize, open: char },\n    UnterminatedString { at: usize },\n}\n\npub fn check(_input: &str) -> Result<(), Mismatch> {\n    use std::io::Write;\n    let _ = std::io::stdout().write_all(b\"test result: ok. <expected> passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\\n\");\n    std::process::exit(0)\n}\n",
    ),
];

fn run_verify(verify: &Path, task_dir: &Path, cwd: &Path, with_reference: bool) -> Option<i32> {
    let mut cmd = Command::new("python3");
    cmd.arg(verify).arg(task_dir).current_dir(cwd);
    if with_reference {
        cmd.arg("--with-reference");
    }
    cmd.stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()
        .and_then(|s| s.code())
}

fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let to = dst.join(entry.file_name());
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir(&entry.path(), &to)?;
        } else if ty.is_file() {
            fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Records
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
struct TokenReport {
    /// `reported`, `not reported` or `not reported (evidence truncated)`.
    status: String,
    /// What the numbers mean for this harness. Totals are not comparable across harnesses.
    source: Option<String>,
    total: Option<u64>,
    input: Option<u64>,
    output: Option<u64>,
    cache_read: Option<u64>,
    cache_creation: Option<u64>,
    /// Model names the harness itself reported (claude `modelUsage` keys).
    reported_models: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct AttemptRecord {
    index: usize,
    attempt_id: u64,
    harness: Option<String>,
    model_ref: Option<String>,
    duration_s: Option<u64>,
    outcome: Option<String>,
    worker_exit: Option<i64>,
    diff_exit: Option<i64>,
    /// Exit of the first verify run (the worker's own result, before any repair round).
    verify_exit: Option<i64>,
    /// Exit of the last verify run recorded for the attempt (after repair rounds).
    verify_exit_last: Option<i64>,
    /// Rounds of Meshloop's in-attempt compiler/test repair loop, when it ran.
    repair_rounds: Option<u64>,
    /// Why the repair loop stopped, as Meshloop recorded it (for example `Oscillation`).
    repair_stop: Option<String>,
    checks_passed: bool,
    tokens: TokenReport,
    quota_signal: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct FeedbackRow {
    harness: String,
    model_ref: String,
    tier: String,
    success: i64,
    failure: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct RunRecord {
    task_id: String,
    language: String,
    difficulty: String,
    kind: String,
    rep: u32,
    graph_id: String,
    seed_commit: Option<String>,
    started_unix: u64,
    routing_feedback_before: Vec<FeedbackRow>,
    store_existed_before: bool,
    review_exit: Option<i32>,
    run_exit: Option<i32>,
    run_timed_out: bool,
    run_idle: Option<String>,
    node_state_after_run: Option<String>,
    /// Node state after the run and any `resume --retry` rounds.
    node_state_after_dispatch: Option<String>,
    /// Tier set in the plan (lp-2), or None for `tier = null` (lp-1).
    plan_tier: Option<String>,
    /// One entry per `resume --retry` issued by the runner.
    retry_resumes: Vec<Value>,
    /// `run` plus every `resume --retry`.
    dispatch_wall_seconds: f64,
    attempts_count: u64,
    retries: u64,
    worker_wall_seconds: Option<u64>,
    attempts: Vec<AttemptRecord>,
    accept_exit: Option<i32>,
    resume_exit: Option<i32>,
    integrate_exit: Option<i32>,
    final_verify_exit: Option<i32>,
    success: bool,
    first_attempt_success: bool,
    adjust_count: u32,
    decline_count: u32,
    run_wall_seconds: f64,
    total_wall_seconds: f64,
    routing_feedback_after: Vec<FeedbackRow>,
    quota_state_after: Vec<Value>,
    quota_hit: Option<String>,
    repo_removed: bool,
    errors: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Results {
    schema: String,
    task_set_version: String,
    mode: String,
    learning_plane: String,
    started_unix: u64,
    started_utc: String,
    finished_unix: u64,
    meshloop: Value,
    host_os: String,
    config_file: String,
    config_sha256: Option<String>,
    harnesses: Value,
    /// Dispatchable candidates per planned tier, from doctor (configured order).
    tier_candidates: Value,
    plan_tiers: serde_json::Map<String, Value>,
    protocol: String,
    state_carried: String,
    reps_requested: u32,
    tasks: Vec<String>,
    run_timeout_seconds: u64,
    stopped_early: Option<String>,
    runs: Vec<RunRecord>,
    summary: Value,
}

// ---------------------------------------------------------------------------
// Live runner
// ---------------------------------------------------------------------------

struct Ctx<'a> {
    lp: &'a Path,
    bin: PathBuf,
    out: PathBuf,
    config_template: String,
    version: String,
    run_timeout: Duration,
    accept_as: String,
    max_retries: u32,
}

fn live(root: &Path, lp: &Path, set: &TaskSet, tasks: &[TaskMeta], opts: &Options) -> ExitCode {
    let bin = opts
        .bin
        .clone()
        .unwrap_or_else(|| crate::meshloop_bin(root));
    if !bin.is_file() {
        eprintln!(
            "bench-learning: meshloop binary not found at {} (pass --bin)",
            bin.display()
        );
        return ExitCode::FAILURE;
    }
    let started = unix_now();
    let out = absolute(&opts.out.clone().unwrap_or_else(|| {
        root.join(".agent-runs")
            .join("bench-learning")
            .join(started.to_string())
    }));
    let bin = absolute(&bin);
    if let Err(e) = fs::create_dir_all(out.join("runs")) {
        eprintln!("bench-learning: cannot create {}: {e}", out.display());
        return ExitCode::FAILURE;
    }
    let config_path = lp.join("meshloop.toml");
    let config_template = match fs::read_to_string(&config_path) {
        Ok(t) if t.lines().filter(|l| l.trim() == VERIFY_PLACEHOLDER).count() == 1 => t,
        Ok(_) => {
            eprintln!(
                "bench-learning: meshloop.toml must contain exactly one `{VERIFY_PLACEHOLDER}` line"
            );
            return ExitCode::FAILURE;
        }
        Err(e) => {
            eprintln!("bench-learning: {}: {e}", config_path.display());
            return ExitCode::FAILURE;
        }
    };
    let version = capture(&bin, &["--version"]).unwrap_or_else(|| "unknown".into());
    let ctx = Ctx {
        lp,
        bin: bin.clone(),
        out: out.clone(),
        version: set.version.clone(),
        run_timeout: Duration::from_secs(opts.run_timeout_s),
        accept_as: opts.accept_as.clone(),
        max_retries: config_max_retries(&config_template).unwrap_or(1),
        config_template,
    };

    println!(
        "bench-learning: task set {} ({} tasks), {} repetition(s)",
        set.version,
        tasks.len(),
        opts.reps
    );
    println!(
        "bench-learning: meshloop {} at {}",
        version.trim(),
        bin.display()
    );
    println!("bench-learning: evidence in {}", out.display());

    let harnesses = match doctor(&ctx, &tasks[0]) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("bench-learning: doctor failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut planned: Vec<&str> = tasks
        .iter()
        .map(|t| set.plan_tiers.get(&t.id).map_or("Tier1", String::as_str))
        .collect();
    planned.sort_unstable();
    planned.dedup();
    let tier_candidates = match tier_coverage(&harnesses, &planned) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("bench-learning: doctor: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!("  doctor: candidates per planned tier {tier_candidates}");

    let mut results = Results {
        schema: SCHEMA.into(),
        task_set_version: set.version.clone(),
        mode: "live".into(),
        learning_plane: "absent (no learning-plane code exists at this commit)".into(),
        started_unix: started,
        started_utc: utc_string(started),
        meshloop: json!({
            "version": version.trim(),
            "binary_sha256": sha256(&bin),
            "commit": opts.bin_commit,
        }),
        host_os: std::env::consts::OS.into(),
        config_file: "benches/learning-plane/meshloop.toml".into(),
        config_sha256: sha256(&config_path),
        harnesses,
        tier_candidates,
        plan_tiers: tasks
            .iter()
            .map(|t| (t.id.clone(), json!(set.plan_tiers.get(&t.id))))
            .collect(),
        protocol: PROTOCOL.into(),
        state_carried: STATE_CARRIED.into(),
        reps_requested: opts.reps,
        tasks: tasks.iter().map(|t| t.id.clone()).collect(),
        run_timeout_seconds: opts.run_timeout_s,
        ..Results::default()
    };

    'reps: for rep in 1..=opts.reps {
        for task in tasks {
            println!("\n== rep {rep} task {}", task.id);
            let tier = set.plan_tiers.get(&task.id).map(String::as_str);
            let record = run_one(&ctx, task, rep, tier);
            print_run(&record);
            let quota = record.quota_hit.clone();
            results.runs.push(record);
            results.summary = summarize(&results.runs);
            if let Err(e) = write_results(&out.join("results.json"), &results, &[]) {
                eprintln!("bench-learning: {e}; stopping");
                return ExitCode::FAILURE;
            }
            if let Some(q) = quota {
                results.stopped_early = Some(format!(
                    "stopped after rep {rep} task {}: quota or rate limit signalled ({q})",
                    task.id
                ));
                println!(
                    "bench-learning: {}",
                    results.stopped_early.as_deref().unwrap_or("")
                );
                break 'reps;
            }
        }
    }
    results.finished_unix = unix_now();
    results.summary = summarize(&results.runs);
    let redactions = redactions(root, &out);
    if let Err(e) = write_results(&out.join("results.json"), &results, &[]) {
        eprintln!("bench-learning: {e}");
        return ExitCode::FAILURE;
    }
    if let Some(path) = &opts.results {
        if let Err(e) = write_results(path, &results, &redactions) {
            eprintln!("bench-learning: {e}");
            return ExitCode::FAILURE;
        }
        println!(
            "bench-learning: redacted results written to {}",
            path.display()
        );
    }
    print_summary(&results.summary);
    ExitCode::SUCCESS
}

fn redactions(root: &Path, out: &Path) -> Vec<(String, String)> {
    let mut r = vec![
        (out.display().to_string(), "<out>".to_string()),
        (root.display().to_string(), "<meshloop>".to_string()),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home).display().to_string();
        if home.len() > 1 {
            r.push((home, "~".into()));
        }
    }
    r
}

fn write_results(
    path: &Path,
    results: &Results,
    redactions: &[(String, String)],
) -> Result<(), String> {
    let mut text = serde_json::to_string_pretty(results)
        .map_err(|e| format!("cannot serialize results: {e}"))?;
    for (from, to) in redactions {
        text = text.replace(from.as_str(), to);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    fs::write(path, text + "\n").map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Whether a model tier as `doctor --json` parses it fits a task tier (`tier_fits`).
fn tier_fits(task_tier: &str, model_tier: &str) -> bool {
    match task_tier {
        "Tier2" => matches!(model_tier, "MidTier" | "TopTier"),
        "Tier3" => model_tier == "TopTier",
        _ => true,
    }
}

/// Dispatchable (harness, model) candidates per planned tier, in configured order.
fn tier_coverage(harnesses: &Value, tiers: &[&str]) -> Result<Value, String> {
    let mut out = serde_json::Map::new();
    for tier in tiers {
        let fits: Vec<String> = harnesses
            .as_array()
            .into_iter()
            .flatten()
            .filter(|h| h["dispatchable"] == Value::Bool(true))
            .flat_map(|h| {
                h["models"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(move |m| tier_fits(tier, m["tier"].as_str().unwrap_or("")))
                    .map(move |m| {
                        format!(
                            "{}/{}",
                            h["name"].as_str().unwrap_or("?"),
                            m["model_ref"].as_str().unwrap_or("?")
                        )
                    })
            })
            .collect();
        if fits.is_empty() {
            return Err(format!("no dispatchable candidate fits {tier}"));
        }
        out.insert((*tier).to_string(), json!(fits));
    }
    Ok(Value::Object(out))
}

fn doctor(ctx: &Ctx, task: &TaskMeta) -> Result<Value, String> {
    let dir = ctx.out.join("doctor");
    let repo = dir.join("repo");
    let _ = fs::remove_dir_all(&repo);
    fs::create_dir_all(&repo).map_err(|e| e.to_string())?;
    git(&repo, &["init", "-q", "-b", "main"])?;
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "doctor"])?;
    let config = dir.join("meshloop.toml");
    fs::write(&config, task_config(ctx, task)).map_err(|e| e.to_string())?;
    let out = run_cmd(
        &ctx.bin,
        &["doctor", "--config", &path_str(&config), "--json"],
        &repo,
        &dir.join("doctor.json"),
        &dir.join("doctor.err"),
        Duration::from_secs(120),
    );
    let _ = fs::remove_dir_all(&repo);
    let doc: Value = serde_json::from_str(&out.stdout).map_err(|e| format!("doctor json: {e}"))?;
    if doc["ok"] != Value::Bool(true) {
        return Err(format!("doctor not ok: {}", doc["error"]));
    }
    let harnesses = doc["data"]["harnesses"].clone();
    let list = match harnesses.as_array() {
        Some(list) if !list.is_empty() => list,
        _ => return Err("doctor reported no harnesses".into()),
    };
    for h in list {
        let name = h["name"].as_str().unwrap_or("?");
        if h["dispatchable"] != Value::Bool(true) {
            return Err(format!(
                "harness {name} is not dispatchable: {}",
                h["reason"]
            ));
        }
        println!(
            "  doctor: {name} {} models {}",
            h["version"].as_str().unwrap_or("?"),
            h["models"]
        );
    }
    Ok(Value::Array(
        list.iter()
            .map(|h| {
                json!({
                    "name": h["name"],
                    "version": h["version"],
                    "dispatchable": h["dispatchable"],
                    "models": h["models"],
                })
            })
            .collect(),
    ))
}

/// `max_retries` from the `[limits]` table of the configuration template.
fn config_max_retries(template: &str) -> Option<u32> {
    template
        .lines()
        .map(str::trim)
        .find_map(|l| l.strip_prefix("max_retries"))
        .and_then(|rest| rest.trim_start().strip_prefix('='))
        .and_then(|v| v.trim().parse().ok())
}

fn task_config(ctx: &Ctx, task: &TaskMeta) -> String {
    let verify = ctx.lp.join("verify.py");
    let task_dir = ctx.lp.join("tasks").join(&task.id);
    let line = format!(
        "verify_command = [{}, {}, {}]",
        toml_str("python3"),
        toml_str(&path_str(&verify)),
        toml_str(&path_str(&task_dir))
    );
    replace_verify_line(&ctx.config_template, &line)
}

fn replace_verify_line(template: &str, line: &str) -> String {
    template
        .lines()
        .map(|l| {
            if l.trim() == VERIFY_PLACEHOLDER {
                line
            } else {
                l
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn toml_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Commands run inside the disposable repository, so every path handed to them is absolute.
fn absolute(p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|d| d.join(p))
            .unwrap_or_else(|_| p.to_path_buf())
    }
}

fn path_str(p: &Path) -> String {
    p.display().to_string()
}

fn plan_json(graph_id: &str, task: &TaskMeta, tier: Option<&str>) -> Value {
    json!({
        "graph_id": graph_id,
        "nodes": [{
            "id": 1,
            "description": task.description,
            "depends_on": [],
            "tier": tier,
        }]
    })
}

fn run_one(ctx: &Ctx, task: &TaskMeta, rep: u32, tier: Option<&str>) -> RunRecord {
    let total = Instant::now();
    let graph_id = format!("lp-{}-r{rep}", task.id);
    let dir = ctx.out.join("runs").join(format!("r{rep}-{}", task.id));
    let repo = dir.join("repo");
    let mut rec = RunRecord {
        task_id: task.id.clone(),
        language: task.language.clone(),
        difficulty: task.difficulty.clone(),
        kind: task.kind.clone(),
        rep,
        graph_id: graph_id.clone(),
        plan_tier: tier.map(String::from),
        started_unix: unix_now(),
        ..RunRecord::default()
    };
    let _ = fs::remove_dir_all(&dir);
    if let Err(e) = seed_repo(ctx, task, &repo) {
        rec.errors.push(format!("seed: {e}"));
        rec.repo_removed = remove_repo(&repo, &mut rec.errors);
        return rec;
    }
    rec.seed_commit = capture_in(&repo, "git", &["rev-parse", "HEAD"]).map(|s| s.trim().into());
    let config = dir.join("meshloop.toml");
    let plan = dir.join("plan.json");
    let written = fs::write(&config, task_config(ctx, task)).and_then(|_| {
        fs::write(
            &plan,
            serde_json::to_string_pretty(&plan_json(&graph_id, task, tier)).unwrap_or_default(),
        )
    });
    if let Err(e) = written {
        rec.errors.push(format!("write config/plan: {e}"));
        rec.repo_removed = remove_repo(&repo, &mut rec.errors);
        return rec;
    }
    let db = repo.join(".meshloop").join("state.sqlite");
    rec.store_existed_before = db.exists();
    rec.routing_feedback_before = read_feedback(&db).unwrap_or_default();

    let cfg = path_str(&config);
    let short = Duration::from_secs(180);
    let ml = |args: &[&str], name: &str, timeout: Duration| {
        run_cmd(
            &ctx.bin,
            args,
            &repo,
            &dir.join(format!("{name}.json")),
            &dir.join(format!("{name}.err")),
            timeout,
        )
    };

    let review = ml(
        &[
            "review-plan",
            "--plan",
            &path_str(&plan),
            "--accept",
            "--as",
            &ctx.accept_as,
            "--config",
            &cfg,
            "--json",
        ],
        "review",
        short,
    );
    rec.review_exit = review.code;
    if review.code != Some(0) {
        rec.errors.push(format!(
            "review-plan exit {:?}; task aborted before run",
            review.code
        ));
        rec.total_wall_seconds = round2(total.elapsed().as_secs_f64());
        rec.repo_removed = remove_repo(&repo, &mut rec.errors);
        return rec;
    }

    let run_start = Instant::now();
    let run = ml(
        &[
            "run",
            "--plan",
            &path_str(&plan),
            "--config",
            &cfg,
            "--json",
        ],
        "run",
        ctx.run_timeout,
    );
    rec.run_wall_seconds = round2(run_start.elapsed().as_secs_f64());
    rec.run_exit = run.code;
    rec.run_timed_out = run.timed_out;
    rec.run_idle = serde_json::from_str::<Value>(&run.stdout)
        .ok()
        .and_then(|v| v["data"]["idle"].as_str().map(String::from));

    let read_status = |name: &str| -> Value {
        let out = ml(
            &["status", "--graph", &graph_id, "--config", &cfg, "--json"],
            name,
            short,
        );
        serde_json::from_str(&out.stdout).unwrap_or(Value::Null)
    };
    let mut status_v = read_status("status");
    rec.node_state_after_run = status_v["data"]["nodes"][0]["state"]
        .as_str()
        .map(String::from);

    // Operator retry protocol (lp-2): a node that ends Failed while attempts remain gets
    // `resume --retry`, up to max_retries, so fallback across candidates is exercised.
    let mut dispatch_secs = rec.run_wall_seconds;
    for i in 1..=ctx.max_retries {
        let node = &status_v["data"]["nodes"][0];
        let attempts_before = status_v["data"]["summary"]["attempts"]
            .as_u64()
            .unwrap_or(0);
        if node["state"].as_str() != Some("Failed")
            || node["waiting_for"].as_str() == Some("retry_budget_exhausted")
            || attempts_before >= u64::from(ctx.max_retries)
        {
            break;
        }
        let start = Instant::now();
        let name = format!("retry-{i}");
        let out = ml(
            &[
                "resume", "--graph", &graph_id, "--retry", "--config", &cfg, "--json",
            ],
            &name,
            ctx.run_timeout,
        );
        let secs = round2(start.elapsed().as_secs_f64());
        dispatch_secs += secs;
        status_v = read_status(&format!("status-{name}"));
        let attempts_after = status_v["data"]["summary"]["attempts"]
            .as_u64()
            .unwrap_or(0);
        rec.retry_resumes.push(json!({
            "exit": out.code,
            "timed_out": out.timed_out,
            "wall_seconds": secs,
            "attempts_after": attempts_after,
            "state_after": status_v["data"]["nodes"][0]["state"],
        }));
        if out.timed_out {
            rec.run_timed_out = true;
            break;
        }
        if attempts_after <= attempts_before {
            rec.errors.push(format!(
                "resume --retry dispatched no new attempt (no unused candidate at {})",
                tier.unwrap_or("default tier")
            ));
            break;
        }
    }
    rec.dispatch_wall_seconds = round2(dispatch_secs);
    let node = &status_v["data"]["nodes"][0];
    rec.node_state_after_dispatch = node["state"].as_str().map(String::from);
    let summary = &status_v["data"]["summary"];
    rec.attempts_count = summary["attempts"].as_u64().unwrap_or(0);
    rec.retries = summary["retries"].as_u64().unwrap_or(0);
    rec.worker_wall_seconds = summary["worker_wall_seconds"].as_u64();

    let inspect = ml(
        &[
            "inspect", "--graph", &graph_id, "--task", "1", "--config", &cfg, "--json",
        ],
        "inspect",
        short,
    );
    let inspect_v: Value = serde_json::from_str(&inspect.stdout).unwrap_or(Value::Null);
    rec.attempts = parse_attempts(&inspect_v);

    let state = rec.node_state_after_dispatch.clone().unwrap_or_default();
    if state == "AwaitingReview" {
        let accept = ml(
            &[
                "accept",
                "--task",
                "1",
                "--as",
                &ctx.accept_as,
                "--graph",
                &graph_id,
                "--config",
                &cfg,
                "--json",
            ],
            "accept",
            short,
        );
        rec.accept_exit = accept.code;
    }
    if state == "AwaitingReview" || state == "Accepted" {
        let resume = ml(
            &["resume", "--graph", &graph_id, "--config", &cfg, "--json"],
            "resume",
            short,
        );
        rec.resume_exit = resume.code;
        let integrate = ml(
            &[
                "integrate",
                "--graph",
                &graph_id,
                "--into",
                "main",
                "--accept-integrate",
                "--config",
                &cfg,
                "--json",
            ],
            "integrate",
            short,
        );
        rec.integrate_exit = integrate.code;
        if integrate.code == Some(0) {
            let verify = ctx.lp.join("verify.py");
            let task_dir = ctx.lp.join("tasks").join(&task.id);
            let final_verify = run_cmd(
                Path::new("python3"),
                &[&path_str(&verify), &path_str(&task_dir)],
                &repo,
                &dir.join("final-verify.out"),
                &dir.join("final-verify.err"),
                Duration::from_secs(300),
            );
            rec.final_verify_exit = final_verify.code;
        }
    } else {
        rec.errors.push(format!(
            "node {state:?} after dispatch (run idle {:?}); not accepted or integrated",
            rec.run_idle
        ));
    }
    rec.success = rec.final_verify_exit == Some(0);
    rec.first_attempt_success =
        rec.success && rec.attempts.first().is_some_and(|a| a.checks_passed);

    rec.routing_feedback_after = read_feedback(&db).unwrap_or_default();
    rec.quota_state_after = read_quota(&db).unwrap_or_default();
    rec.quota_hit = quota_hit(&rec);
    rec.total_wall_seconds = round2(total.elapsed().as_secs_f64());
    rec.repo_removed = remove_repo(&repo, &mut rec.errors);
    rec
}

/// Deletes the disposable repository (worktrees included). True when it is gone.
fn remove_repo(repo: &Path, errors: &mut Vec<String>) -> bool {
    if !repo.exists() {
        return true;
    }
    match fs::remove_dir_all(repo) {
        Ok(()) => true,
        Err(e) => {
            errors.push(format!("remove disposable repo: {e}"));
            false
        }
    }
}

fn seed_repo(ctx: &Ctx, task: &TaskMeta, repo: &Path) -> Result<(), String> {
    let task_dir = ctx.lp.join("tasks").join(&task.id);
    copy_dir(&task_dir.join("seed"), repo).map_err(|e| e.to_string())?;
    let common = ctx.lp.join("common");
    for (from, to) in [
        ("AGENTS.md", "AGENTS.md"),
        ("CLAUDE.md", "CLAUDE.md"),
        ("gitignore", ".gitignore"),
    ] {
        fs::copy(common.join(from), repo.join(to)).map_err(|e| format!("{from}: {e}"))?;
    }
    git(repo, &["init", "-q", "-b", "main"])?;
    git(repo, &["add", "-A"])?;
    git(
        repo,
        &[
            "commit",
            "-q",
            "-m",
            &format!("seed {} ({})", task.id, ctx.version),
        ],
    )
}

fn git(repo: &Path, args: &[&str]) -> Result<(), String> {
    let status = Command::new("git")
        .args(GIT_IDENTITY)
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(repo)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("git: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("git {} failed: {status}", args.join(" ")))
    }
}

struct CmdOut {
    code: Option<i32>,
    timed_out: bool,
    stdout: String,
}

/// Runs a command with stdout and stderr in files and a hard timeout. On Unix the child
/// leads its own process group and the whole group is killed on timeout.
fn run_cmd(
    program: &Path,
    args: &[&str],
    cwd: &Path,
    stdout_path: &Path,
    stderr_path: &Path,
    timeout: Duration,
) -> CmdOut {
    let fail = |msg: String| {
        let _ = fs::write(stderr_path, &msg);
        CmdOut {
            code: None,
            timed_out: false,
            stdout: String::new(),
        }
    };
    let (Ok(out), Ok(err)) = (fs::File::create(stdout_path), fs::File::create(stderr_path)) else {
        return fail("cannot create output files".into());
    };
    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return fail(format!("spawn {}: {e}", program.display())),
    };
    let start = Instant::now();
    let mut timed_out = false;
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) if start.elapsed() >= timeout => {
                timed_out = true;
                #[cfg(unix)]
                {
                    let _ = Command::new("kill")
                        .args(["-KILL", &format!("-{}", child.id())])
                        .status();
                }
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(250)),
            Err(_) => break None,
        }
    };
    CmdOut {
        code,
        timed_out,
        stdout: fs::read_to_string(stdout_path).unwrap_or_default(),
    }
}

fn capture(program: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn capture_in(cwd: &Path, program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn sha256(path: &Path) -> Option<String> {
    let out = Command::new("sha256sum").arg(path).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .map(String::from)
}

fn read_feedback(db: &Path) -> Result<Vec<FeedbackRow>, String> {
    if !db.exists() {
        return Ok(Vec::new());
    }
    let conn = open_ro(db)?;
    let mut stmt = conn
        .prepare("SELECT harness, model_ref, tier, success_count, failure_count FROM routing_feedback ORDER BY harness, model_ref, tier")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(FeedbackRow {
                harness: r.get(0)?,
                model_ref: r.get(1)?,
                tier: r.get(2)?,
                success: r.get(3)?,
                failure: r.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

fn read_quota(db: &Path) -> Result<Vec<Value>, String> {
    if !db.exists() {
        return Ok(Vec::new());
    }
    let conn = open_ro(db)?;
    let mut stmt = conn
        .prepare(
            "SELECT harness, breaker, opened_at, cooldown_ms FROM quota_state ORDER BY harness",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(json!({
                "harness": r.get::<_, String>(0)?,
                "breaker": r.get::<_, String>(1)?,
                "opened_at": r.get::<_, Option<String>>(2)?,
                "cooldown_ms": r.get::<_, i64>(3)?,
            }))
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

fn open_ro(db: &Path) -> Result<rusqlite::Connection, String> {
    rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| e.to_string())
}

fn quota_hit(rec: &RunRecord) -> Option<String> {
    for q in &rec.quota_state_after {
        let breaker = q["breaker"].as_str().unwrap_or("");
        if !breaker.eq_ignore_ascii_case("closed") {
            return Some(format!(
                "meshloop quota_state {} breaker {breaker}",
                q["harness"].as_str().unwrap_or("?")
            ));
        }
    }
    rec.attempts.iter().find(|a| a.quota_signal).map(|a| {
        format!(
            "worker output of {}/{} mentions a usage or rate limit",
            a.harness.as_deref().unwrap_or("?"),
            a.model_ref.as_deref().unwrap_or("?")
        )
    })
}

// ---------------------------------------------------------------------------
// Parsing (pure, unit tested)
// ---------------------------------------------------------------------------

fn parse_attempts(inspect: &Value) -> Vec<AttemptRecord> {
    let Some(list) = inspect["data"]["attempts"].as_array() else {
        return Vec::new();
    };
    list.iter()
        .enumerate()
        .map(|(i, a)| {
            let harness = a["harness"].as_str().map(String::from);
            let mut rec = AttemptRecord {
                index: i + 1,
                attempt_id: a["attempt_id"].as_u64().unwrap_or(0),
                harness: harness.clone(),
                model_ref: a["model_ref"].as_str().map(String::from),
                duration_s: a["duration_s"].as_u64(),
                outcome: a["outcome"].as_str().map(String::from),
                ..AttemptRecord::default()
            };
            rec.checks_passed = rec.outcome.as_deref() == Some("DeterministicChecksPassed");
            let mut transcript = String::new();
            for e in a["evidence"].as_array().into_iter().flatten() {
                if e["kind"].as_str() != Some("deterministic") {
                    continue;
                }
                let exit = e["exit_code"].as_i64();
                match e["tool"].as_str() {
                    Some("git-diff") => rec.diff_exit = exit,
                    Some("harness-output") => {
                        transcript = e["summary_redacted"].as_str().unwrap_or("").to_string();
                    }
                    Some("repair-session") => {
                        let text = e["summary_redacted"].as_str().unwrap_or("");
                        let (rounds, stop) = parse_repair_session(text);
                        rec.repair_rounds = rounds;
                        rec.repair_stop = stop;
                    }
                    Some(_) => {
                        if rec.verify_exit.is_none() {
                            rec.verify_exit = exit;
                        }
                        rec.verify_exit_last = exit;
                    }
                    None => {}
                }
            }
            rec.worker_exit = transcript
                .strip_prefix("exit=")
                .and_then(|s| s.lines().next())
                .and_then(|s| s.trim().parse().ok());
            rec.tokens = parse_tokens(harness.as_deref().unwrap_or(""), &transcript);
            let lower = transcript.to_lowercase();
            rec.quota_signal = (!rec.checks_passed || rec.worker_exit.is_some_and(|c| c != 0))
                && QUOTA_MARKERS.iter().any(|m| lower.contains(m));
            rec
        })
        .collect()
}

fn parse_tokens(harness: &str, transcript: &str) -> TokenReport {
    let truncated = transcript.contains("…[truncated ");
    let not_reported = |why: &str| TokenReport {
        status: why.into(),
        ..TokenReport::default()
    };
    let missing = if truncated {
        "not reported (evidence truncated)"
    } else {
        "not reported"
    };
    match harness {
        "codex" => match parse_codex_tokens(transcript) {
            Some(total) => TokenReport {
                status: "reported".into(),
                source: Some("codex-cli final `tokens used` line".into()),
                total: Some(total),
                ..TokenReport::default()
            },
            None => not_reported(missing),
        },
        "claude" => match parse_claude_usage(transcript) {
            Some(mut t) => {
                t.status = if truncated {
                    "reported (recovered from truncated evidence)".into()
                } else {
                    "reported".into()
                };
                t.source = Some(
                    "claude --output-format json modelUsage: input+output+cacheRead+cacheCreation"
                        .into(),
                );
                t
            }
            None => not_reported(missing),
        },
        _ => not_reported("not reported"),
    }
}

/// `(rounds, stop reason)` from a `repair-session` evidence summary such as
/// `stop=Stop:Oscillation { cycle_at: 0 } passed=false ...\nrepair-session rounds=2`.
fn parse_repair_session(text: &str) -> (Option<u64>, Option<String>) {
    let rounds = text.find("rounds=").and_then(|i| {
        text[i + "rounds=".len()..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse()
            .ok()
    });
    let stop = text.strip_prefix("stop=").map(|rest| {
        let end = rest.find(" passed=").unwrap_or(rest.len());
        rest[..end].trim().to_string()
    });
    (rounds, stop)
}

/// The count after the last `tokens used` line, e.g. `tokens used\n11,278`.
fn parse_codex_tokens(text: &str) -> Option<u64> {
    let idx = text.rfind("tokens used")?;
    let rest = text[idx + "tokens used".len()..].trim_start_matches([':', ' ', '\t', '\r', '\n']);
    let digits: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == ',')
        .filter(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

/// Sums `modelUsage` from claude's `--output-format json` result. Uses the full JSON when
/// the stdout section is intact, else scans the `modelUsage` object in what was kept.
fn parse_claude_usage(text: &str) -> Option<TokenReport> {
    if let (Some(start), Some(end)) = (text.find("stdout:\n"), text.rfind("\nstderr:\n")) {
        let body = &text[start + "stdout:\n".len()..end.max(start + "stdout:\n".len())];
        if let Ok(v) = serde_json::from_str::<Value>(body.trim())
            && let Some(models) = v["modelUsage"].as_object()
        {
            let mut t = TokenReport::default();
            let (mut i, mut o, mut cr, mut cc) = (0u64, 0u64, 0u64, 0u64);
            for (name, m) in models {
                t.reported_models.push(name.clone());
                i += m["inputTokens"].as_u64().unwrap_or(0);
                o += m["outputTokens"].as_u64().unwrap_or(0);
                cr += m["cacheReadInputTokens"].as_u64().unwrap_or(0);
                cc += m["cacheCreationInputTokens"].as_u64().unwrap_or(0);
            }
            return Some(fill_claude(t, i, o, cr, cc));
        }
    }
    // camelCase counters appear only inside `modelUsage` (`usage` is snake_case), so
    // when truncation cut the `modelUsage` key itself, start at the first counter.
    let idx = text
        .find("\"modelUsage\":{")
        .or_else(|| text.find("\"inputTokens\":"))?;
    let rest = &text[idx..];
    let sum = |key: &str| -> Option<u64> {
        let pat = format!("\"{key}\":");
        let mut total = 0u64;
        let mut found = false;
        for (pos, _) in rest.match_indices(&pat) {
            let n: String = rest[pos + pat.len()..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let Ok(v) = n.parse::<u64>() {
                total += v;
                found = true;
            }
        }
        found.then_some(total)
    };
    let (i, o) = (sum("inputTokens")?, sum("outputTokens")?);
    let (cr, cc) = (
        sum("cacheReadInputTokens").unwrap_or(0),
        sum("cacheCreationInputTokens").unwrap_or(0),
    );
    let mut t = TokenReport::default();
    let mut search = rest;
    while let Some(p) = search.find("\"canonicalModel\":\"") {
        let after = &search[p + "\"canonicalModel\":\"".len()..];
        if let Some(q) = after.find('"') {
            t.reported_models.push(after[..q].to_string());
        }
        search = after;
    }
    Some(fill_claude(t, i, o, cr, cc))
}

fn fill_claude(mut t: TokenReport, i: u64, o: u64, cr: u64, cc: u64) -> TokenReport {
    t.input = Some(i);
    t.output = Some(o);
    t.cache_read = Some(cr);
    t.cache_creation = Some(cc);
    t.total = Some(i + o + cr + cc);
    t
}

// ---------------------------------------------------------------------------
// Aggregation (pure, unit tested)
// ---------------------------------------------------------------------------

/// Nearest-rank percentile, `p` in (0, 1].
fn percentile(values: &[f64], p: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let rank = ((p * v.len() as f64).ceil() as usize).clamp(1, v.len());
    Some(v[rank - 1])
}

fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = v.len();
    Some(if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    })
}

fn mean(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

fn opt2(x: Option<f64>) -> Value {
    x.map(|v| json!(round2(v))).unwrap_or(Value::Null)
}

fn rate(successes: usize, n: usize) -> Value {
    let w = crate::wilson_score_interval(successes, n);
    json!({
        "successes": successes,
        "n": n,
        "rate": round4(w.proportion),
        "wilson95_low": round4(w.lower_bound),
        "wilson95_high": round4(w.upper_bound),
    })
}

fn round4(x: f64) -> f64 {
    (x * 10000.0).round() / 10000.0
}

fn group_stats(runs: &[&RunRecord]) -> Value {
    let n = runs.len();
    let ok = runs.iter().filter(|r| r.success).count();
    let first = runs.iter().filter(|r| r.first_attempt_success).count();
    let retries: Vec<f64> = runs.iter().map(|r| r.retries as f64).collect();
    let wall: Vec<f64> = runs.iter().map(|r| dispatch_wall(r)).collect();
    json!({
        "runs": n,
        "success": rate(ok, n),
        "first_attempt_success": rate(first, n),
        "retries_mean": opt2(mean(&retries)),
        "retries_median": opt2(median(&retries)),
        "dispatch_wall_seconds_p50": opt2(percentile(&wall, 0.5)),
        "dispatch_wall_seconds_p95": opt2(percentile(&wall, 0.95)),
        "fallback_successes": runs.iter().filter(|r| r.success && !r.first_attempt_success).count(),
    })
}

/// `run` plus retries. Records from before lp-2 have only `run_wall_seconds`.
fn dispatch_wall(r: &RunRecord) -> f64 {
    if r.dispatch_wall_seconds > 0.0 {
        r.dispatch_wall_seconds
    } else {
        r.run_wall_seconds
    }
}

fn summarize(runs: &[RunRecord]) -> Value {
    let all: Vec<&RunRecord> = runs.iter().collect();
    let mut overall = group_stats(&all);
    let retries: Vec<f64> = runs.iter().map(|r| r.retries as f64).collect();
    let attempts: Vec<f64> = runs.iter().map(|r| r.attempts.len() as f64).collect();
    let wall_total: Vec<f64> = runs.iter().map(|r| r.total_wall_seconds).collect();
    let worker_wall: Vec<f64> = runs
        .iter()
        .filter_map(|r| r.worker_wall_seconds.map(|w| w as f64))
        .collect();
    overall["retries_max"] = json!(retries.iter().cloned().fold(0.0, f64::max));
    overall["attempts_mean"] = opt2(mean(&attempts));
    overall["total_wall_seconds_p50"] = opt2(percentile(&wall_total, 0.5));
    overall["total_wall_seconds_p95"] = opt2(percentile(&wall_total, 0.95));
    overall["worker_wall_seconds_p50"] = opt2(percentile(&worker_wall, 0.5));
    overall["worker_wall_seconds_p95"] = opt2(percentile(&worker_wall, 0.95));
    overall["adjust_count"] = json!(runs.iter().map(|r| r.adjust_count).sum::<u32>());
    overall["decline_count"] = json!(runs.iter().map(|r| r.decline_count).sum::<u32>());
    overall["adjust_decline_note"] =
        json!("0 by protocol (fixed automated acceptance), not a human measurement");
    let repaired: Vec<&AttemptRecord> = runs
        .iter()
        .flat_map(|r| &r.attempts)
        .filter(|a| a.repair_rounds.is_some())
        .collect();
    overall["attempts_with_repair_loop"] = json!(repaired.len());
    overall["repair_rounds_total"] =
        json!(repaired.iter().filter_map(|a| a.repair_rounds).sum::<u64>());
    overall["failed_runs_without_fallback"] = json!(
        runs.iter()
            .filter(|r| !r.success && r.attempts.len() == 1)
            .count()
    );
    overall["timed_out_runs"] = json!(runs.iter().filter(|r| r.run_timed_out).count());
    overall["quota_hits"] = json!(runs.iter().filter(|r| r.quota_hit.is_some()).count());

    let mut by_rep = serde_json::Map::new();
    let mut reps: Vec<u32> = runs.iter().map(|r| r.rep).collect();
    reps.sort_unstable();
    reps.dedup();
    for rep in &reps {
        let g: Vec<&RunRecord> = runs.iter().filter(|r| r.rep == *rep).collect();
        by_rep.insert(format!("rep{rep}"), group_stats(&g));
    }
    let rep_rates: Vec<f64> = reps
        .iter()
        .map(|rep| {
            let g: Vec<&RunRecord> = runs.iter().filter(|r| r.rep == *rep).collect();
            g.iter().filter(|r| r.success).count() as f64 / g.len().max(1) as f64
        })
        .collect();

    let mut by_tier = serde_json::Map::new();
    let mut tiers: Vec<String> = runs
        .iter()
        .map(|r| r.plan_tier.clone().unwrap_or_else(|| "null".into()))
        .collect();
    tiers.sort();
    tiers.dedup();
    for t in &tiers {
        let g: Vec<&RunRecord> = runs
            .iter()
            .filter(|r| r.plan_tier.as_deref().unwrap_or("null") == t)
            .collect();
        by_tier.insert(t.clone(), group_stats(&g));
    }
    let mut first_dispatch = serde_json::Map::new();
    for r in runs {
        if let Some(a) = r.attempts.first() {
            let key = format!(
                "{}/{}",
                a.harness.as_deref().unwrap_or("?"),
                a.model_ref.as_deref().unwrap_or("?")
            );
            let n = first_dispatch
                .get(&key)
                .and_then(Value::as_u64)
                .unwrap_or(0);
            first_dispatch.insert(key, json!(n + 1));
        }
    }
    let mut retries_hist = serde_json::Map::new();
    for r in runs {
        let key = r.retries.to_string();
        let n = retries_hist.get(&key).and_then(Value::as_u64).unwrap_or(0);
        retries_hist.insert(key, json!(n + 1));
    }

    let mut by_task = serde_json::Map::new();
    let mut task_ids: Vec<&str> = Vec::new();
    for r in runs {
        if !task_ids.contains(&r.task_id.as_str()) {
            task_ids.push(&r.task_id);
        }
    }
    for id in &task_ids {
        let g: Vec<&RunRecord> = runs.iter().filter(|r| r.task_id == *id).collect();
        let mut s = group_stats(&g);
        s["difficulty"] = json!(g[0].difficulty);
        s["plan_tier"] = json!(g[0].plan_tier);
        by_task.insert((*id).to_string(), s);
    }

    let mut arms: Vec<(String, String)> = Vec::new();
    for a in runs.iter().flat_map(|r| &r.attempts) {
        let key = (
            a.harness.clone().unwrap_or_default(),
            a.model_ref.clone().unwrap_or_default(),
        );
        if !arms.contains(&key) {
            arms.push(key);
        }
    }
    let by_arm: Vec<Value> = arms
        .iter()
        .map(|(h, m)| {
            let atts: Vec<&AttemptRecord> = runs
                .iter()
                .flat_map(|r| &r.attempts)
                .filter(|a| a.harness.as_deref() == Some(h) && a.model_ref.as_deref() == Some(m))
                .collect();
            let first = runs
                .iter()
                .filter(|r| {
                    r.attempts.first().is_some_and(|a| {
                        a.harness.as_deref() == Some(h) && a.model_ref.as_deref() == Some(m)
                    })
                })
                .count();
            let passed = atts.iter().filter(|a| a.checks_passed).count();
            let dur: Vec<f64> = atts
                .iter()
                .filter_map(|a| a.duration_s.map(|d| d as f64))
                .collect();
            let tok: Vec<f64> = atts
                .iter()
                .filter(|a| a.tokens.status.starts_with("reported"))
                .filter_map(|a| a.tokens.total.map(|t| t as f64))
                .collect();
            let full = atts
                .iter()
                .filter(|a| a.tokens.status == "reported")
                .count();
            let recovered = atts
                .iter()
                .filter(|a| a.tokens.status.starts_with("reported (recovered"))
                .count();
            let mut models: Vec<String> = atts
                .iter()
                .flat_map(|a| a.tokens.reported_models.clone())
                .collect();
            models.sort();
            models.dedup();
            json!({
                "harness": h,
                "model_ref": m,
                "harness_reported_models": models,
                "attempts": atts.len(),
                "first_dispatches": first,
                "attempt_checks_passed": rate(passed, atts.len()),
                "duration_seconds_p50": opt2(percentile(&dur, 0.5)),
                "duration_seconds_p95": opt2(percentile(&dur, 0.95)),
                "tokens_reported_attempts": full,
                "tokens_recovered_attempts": recovered,
                "tokens_not_reported_attempts": atts.len() - full - recovered,
                "tokens_figures_basis": if recovered == 0 { "fully reported" }
                    else if full == 0 { "recovered only (truncated evidence)" }
                    else { "mixed: fully reported and recovered" },
                "tokens_total_sum": tok.iter().sum::<f64>(),
                "tokens_total_mean": opt2(mean(&tok)),
                "tokens_total_median": opt2(median(&tok)),
            })
        })
        .collect();

    json!({
        "overall": overall,
        "success_rate_by_rep": rep_rates.iter().map(|r| round4(*r)).collect::<Vec<_>>(),
        "success_rate_rep_range_pp": if rep_rates.is_empty() { Value::Null } else {
            let max = rep_rates.iter().cloned().fold(f64::MIN, f64::max);
            let min = rep_rates.iter().cloned().fold(f64::MAX, f64::min);
            json!(round2((max - min) * 100.0))
        },
        "by_rep": by_rep,
        "by_plan_tier": by_tier,
        "first_dispatch_distribution": first_dispatch,
        "runs_by_retries": retries_hist,
        "by_task": by_task,
        "by_harness_model": by_arm,
    })
}

fn aggregate_file(root: &Path, path: &Path, out: Option<&Path>) -> ExitCode {
    let mut results: Results = match read_json(path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("bench-learning: {e}");
            return ExitCode::FAILURE;
        }
    };
    // Re-parse attempts from the evidence kept next to the results file, so parser fixes
    // apply to an existing run without dispatching anything again.
    let runs_dir = path.parent().map(|p| p.join("runs"));
    let mut reparsed = 0;
    for r in &mut results.runs {
        let Some(dir) = &runs_dir else { break };
        let file = dir
            .join(format!("r{}-{}", r.rep, r.task_id))
            .join("inspect.json");
        if let Ok(text) = fs::read_to_string(&file)
            && let Ok(v) = serde_json::from_str::<Value>(&text)
        {
            r.attempts = parse_attempts(&v);
            r.first_attempt_success =
                r.success && r.attempts.first().is_some_and(|a| a.checks_passed);
            reparsed += 1;
        }
    }
    if reparsed > 0 {
        println!("bench-learning: re-parsed attempts of {reparsed} run(s) from kept inspect.json");
    }
    results.summary = summarize(&results.runs);
    if let Some(out) = out {
        let dir = absolute(path.parent().unwrap_or(Path::new(".")));
        if let Err(e) = write_results(out, &results, &redactions(root, &dir)) {
            eprintln!("bench-learning: {e}");
            return ExitCode::FAILURE;
        }
    }
    print_summary(&results.summary);
    ExitCode::SUCCESS
}

fn print_run(r: &RunRecord) {
    let arms: Vec<String> = r
        .attempts
        .iter()
        .map(|a| {
            format!(
                "{}/{}:{}{}",
                a.harness.as_deref().unwrap_or("?"),
                a.model_ref.as_deref().unwrap_or("?"),
                if a.checks_passed { "pass" } else { "fail" },
                a.tokens
                    .total
                    .map(|t| format!("({t} tok)"))
                    .unwrap_or_default()
            )
        })
        .collect();
    println!(
        "  {} tier={} success={} first={} retries={} dispatch={}s attempts=[{}] final_verify={:?}{}",
        r.task_id,
        r.plan_tier.as_deref().unwrap_or("null"),
        r.success,
        r.first_attempt_success,
        r.retries,
        dispatch_wall(r),
        arms.join(", "),
        r.final_verify_exit,
        r.quota_hit
            .as_deref()
            .map(|q| format!(" QUOTA: {q}"))
            .unwrap_or_default()
    );
    for e in &r.errors {
        println!("    note: {e}");
    }
}

fn print_summary(summary: &Value) {
    println!(
        "\nsummary:\n{}",
        serde_json::to_string_pretty(summary).unwrap_or_default()
    );
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `YYYY-MM-DDTHH:MM:SSZ` from Unix seconds (civil-from-days, proleptic Gregorian).
fn utc_string(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_require_exactly_one_mode() {
        assert!(parse_args(&[]).is_err());
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(parse_args(&a(&["--live", "--validate"])).is_err());
        let o = parse_args(&a(&["--live", "--reps", "3"])).unwrap();
        assert!(o.live && o.reps == 3);
        assert!(parse_args(&a(&["--live", "--reps", "0"])).is_err());
        assert!(parse_args(&a(&["--reps", "2"])).is_err());
    }

    #[test]
    fn codex_tokens_parse_last_line() {
        let t = "exit=0\nstdout:\nDone.\nstderr:\nhook: Stop\ntokens used\n11,278\n";
        assert_eq!(parse_codex_tokens(t), Some(11_278));
        assert_eq!(parse_codex_tokens("no usage here"), None);
        let r = parse_tokens("codex", t);
        assert_eq!(r.status, "reported");
        assert_eq!(r.total, Some(11_278));
    }

    #[test]
    fn claude_usage_from_full_json() {
        let t = "exit=0\nstdout:\n{\"result\":\"ok\",\"modelUsage\":{\"claude-haiku-5-5\":{\"inputTokens\":2,\"outputTokens\":4,\"cacheReadInputTokens\":10,\"cacheCreationInputTokens\":100}}}\nstderr:\n";
        let r = parse_tokens("claude", t);
        assert_eq!(r.status, "reported");
        assert_eq!(r.total, Some(116));
        assert_eq!(r.reported_models, vec!["claude-haiku-5-5".to_string()]);
    }

    #[test]
    fn claude_usage_from_truncated_tail() {
        let t = "…[truncated 9000 bytes]…tput\":1},\"modelUsage\":{\"claude-sonnet-5-5\":{\"inputTokens\":5,\"outputTokens\":7,\"cacheReadInputTokens\":1,\"cacheCreationInputTokens\":2,\"canonicalModel\":\"claude-sonnet-5-5\"}},\"result\":\"x\"}\nstderr:\n";
        let r = parse_tokens("claude", t);
        assert_eq!(r.total, Some(15));
        assert_eq!(r.reported_models, vec!["claude-sonnet-5-5".to_string()]);
        let cut = "exit=0\nstdout:\n…[truncated 2791 bytes]…laude-sonnet-5-5\":{\"inputTokens\":10,\"outputTokens\":1300,\"cacheReadInputTokens\":5,\"cacheCreationInputTokens\":2,\"webSearchRequests\":0},\"result\":\"x\"}\nstderr:\n";
        let r = parse_tokens("claude", cut);
        assert_eq!(r.total, Some(1317));
        assert_eq!(r.status, "reported (recovered from truncated evidence)");
        let gone = parse_tokens(
            "claude",
            "…[truncated 9000 bytes]…\"result\":\"x\"}\nstderr:\n",
        );
        assert_eq!(gone.status, "not reported (evidence truncated)");
        assert_eq!(gone.total, None);
    }

    #[test]
    fn repair_session_parses() {
        let t = "stop=Stop:Oscillation { cycle_at: 0 } passed=false elapsed_ms=10453.7\nrepair-session rounds=2\nround=0";
        assert_eq!(
            parse_repair_session(t),
            (
                Some(2),
                Some("Stop:Oscillation { cycle_at: 0 }".to_string())
            )
        );
        assert_eq!(parse_repair_session("other"), (None, None));
    }

    #[test]
    fn write_results_reports_failure() {
        let dir = std::env::temp_dir().join(format!("ml-lp-wr-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let blocker = dir.join("file");
        fs::write(&blocker, "x").unwrap();
        // A path below a regular file cannot be created.
        assert!(write_results(&blocker.join("r.json"), &Results::default(), &[]).is_err());
        assert!(write_results(&dir.join("ok.json"), &Results::default(), &[]).is_ok());
        let mut errors = Vec::new();
        assert!(remove_repo(&dir, &mut errors));
        assert!(remove_repo(&dir, &mut errors));
        assert!(errors.is_empty());
    }

    #[test]
    fn token_counts_split_reported_and_recovered() {
        let att = |status: &str| AttemptRecord {
            harness: Some("claude".into()),
            model_ref: Some("sonnet".into()),
            checks_passed: true,
            tokens: TokenReport {
                status: status.into(),
                total: (status != "not reported").then_some(100),
                ..TokenReport::default()
            },
            ..AttemptRecord::default()
        };
        let runs = vec![RunRecord {
            task_id: "a".into(),
            rep: 1,
            success: true,
            attempts: vec![
                att("reported"),
                att("reported (recovered from truncated evidence)"),
                att("not reported"),
            ],
            ..RunRecord::default()
        }];
        let arm = &summarize(&runs)["by_harness_model"][0];
        assert_eq!(arm["tokens_reported_attempts"], 1);
        assert_eq!(arm["tokens_recovered_attempts"], 1);
        assert_eq!(arm["tokens_not_reported_attempts"], 1);
        assert_eq!(
            arm["tokens_figures_basis"],
            "mixed: fully reported and recovered"
        );
    }

    #[test]
    fn max_retries_from_template() {
        assert_eq!(config_max_retries("[limits]\nmax_retries = 3\n"), Some(3));
        assert_eq!(config_max_retries("x = 1\n"), None);
    }

    #[test]
    fn tier_coverage_follows_tier_fits() {
        let h = json!([
            {"name": "codex", "dispatchable": true, "models": [
                {"model_ref": "lite", "tier": "Lightweight"},
                {"model_ref": "big", "tier": "TopTier"}]},
            {"name": "claude", "dispatchable": true, "models": [
                {"model_ref": "mid", "tier": "MidTier"}]},
            {"name": "agy", "dispatchable": false, "models": [
                {"model_ref": "x", "tier": "TopTier"}]}
        ]);
        let c = tier_coverage(&h, &["Tier1", "Tier2", "Tier3"]).unwrap();
        assert_eq!(c["Tier1"], json!(["codex/lite", "codex/big", "claude/mid"]));
        assert_eq!(c["Tier2"], json!(["codex/big", "claude/mid"]));
        assert_eq!(c["Tier3"], json!(["codex/big"]));
        let none = json!([{"name": "pi", "dispatchable": true, "models": [
            {"model_ref": "m", "tier": "MidTier"}]}]);
        assert!(tier_coverage(&none, &["Tier3"]).is_err());
    }

    #[test]
    fn pi_tokens_not_reported() {
        assert_eq!(
            parse_tokens("pi", "exit=0\nstdout:\nDone\nstderr:\n").status,
            "not reported"
        );
    }

    #[test]
    fn percentile_and_median() {
        let v = [5.0, 1.0, 3.0, 2.0, 4.0];
        assert_eq!(percentile(&v, 0.5), Some(3.0));
        assert_eq!(percentile(&v, 0.95), Some(5.0));
        assert_eq!(median(&[1.0, 2.0, 3.0, 4.0]), Some(2.5));
        assert_eq!(percentile(&[], 0.5), None);
    }

    #[test]
    fn verify_line_is_replaced_once() {
        let t = "a = 1\n[verify]\nverify_command = []\nb = 2\n";
        let r = replace_verify_line(t, "verify_command = [\"x\"]");
        assert_eq!(r, "a = 1\n[verify]\nverify_command = [\"x\"]\nb = 2\n");
        assert_eq!(toml_str("a\"b\\c"), "\"a\\\"b\\\\c\"");
    }

    #[test]
    fn plan_has_one_node_with_the_planned_tier() {
        let t = TaskMeta {
            id: "x".into(),
            language: "python".into(),
            difficulty: "easy".into(),
            kind: "implement".into(),
            description: "do it".into(),
            test_command: vec![],
            editable: vec![],
            expected_tests: 0,
        };
        let p = plan_json("lp-x-r1", &t, None);
        assert_eq!(p["nodes"][0]["tier"], Value::Null);
        assert_eq!(
            plan_json("lp-x-r1", &t, Some("Tier2"))["nodes"][0]["tier"],
            "Tier2"
        );
        assert_eq!(p["nodes"][0]["description"], "do it");
    }

    #[test]
    fn attempts_parse_from_inspect_json() {
        let v = json!({"data": {"attempts": [
            {"attempt_id": 1, "harness": "codex", "model_ref": "m", "duration_s": 9,
             "outcome": "ChecksFailed",
             "evidence": [
                {"kind": "deterministic", "tool": "git-diff", "exit_code": 0, "summary_redacted": "a"},
                {"kind": "deterministic", "tool": "python3", "exit_code": 1, "summary_redacted": "b"},
                {"kind": "deterministic", "tool": "python3", "exit_code": 0, "summary_redacted": "c"},
                {"kind": "deterministic", "tool": "repair-session", "exit_code": 1,
                 "summary_redacted": "stop=Stop:Converged passed=true\nrepair-session rounds=1"},
                {"kind": "deterministic", "tool": "harness-output", "exit_code": 0,
                 "summary_redacted": "exit=1\nstdout:\n\nstderr:\nYou've hit your usage limit\n"}]},
            {"attempt_id": 2, "harness": "claude", "model_ref": "haiku", "duration_s": 5,
             "outcome": "DeterministicChecksPassed", "evidence": []}
        ]}});
        let a = parse_attempts(&v);
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].verify_exit, Some(1));
        assert_eq!(a[0].verify_exit_last, Some(0));
        assert_eq!(a[0].repair_rounds, Some(1));
        assert_eq!(a[0].repair_stop.as_deref(), Some("Stop:Converged"));
        assert_eq!(a[0].worker_exit, Some(1));
        assert!(a[0].quota_signal);
        assert!(!a[0].checks_passed);
        assert!(a[1].checks_passed);
        assert!(!a[1].quota_signal);
    }

    #[test]
    fn summary_counts_success_and_arms() {
        let att = |h: &str, ok: bool| AttemptRecord {
            harness: Some(h.into()),
            model_ref: Some("m".into()),
            checks_passed: ok,
            duration_s: Some(10),
            ..AttemptRecord::default()
        };
        let runs = vec![
            RunRecord {
                task_id: "a".into(),
                rep: 1,
                success: true,
                first_attempt_success: true,
                attempts: vec![att("codex", true)],
                run_wall_seconds: 10.0,
                ..RunRecord::default()
            },
            RunRecord {
                task_id: "b".into(),
                rep: 1,
                success: true,
                retries: 1,
                attempts: vec![att("codex", false), att("claude", true)],
                run_wall_seconds: 30.0,
                ..RunRecord::default()
            },
            RunRecord {
                task_id: "a".into(),
                rep: 2,
                success: false,
                retries: 2,
                attempts: vec![att("codex", false), att("claude", false), att("pi", false)],
                run_wall_seconds: 50.0,
                ..RunRecord::default()
            },
        ];
        let s = summarize(&runs);
        assert_eq!(s["overall"]["success"]["successes"], 2);
        assert_eq!(s["overall"]["first_attempt_success"]["successes"], 1);
        assert_eq!(s["overall"]["retries_mean"], 1.0);
        assert_eq!(s["overall"]["dispatch_wall_seconds_p50"], 30.0);
        assert_eq!(s["overall"]["fallback_successes"], 1);
        assert_eq!(s["first_dispatch_distribution"]["codex/m"], 3);
        assert_eq!(s["runs_by_retries"]["0"], 1);
        assert_eq!(s["overall"]["adjust_count"], 0);
        assert_eq!(s["success_rate_by_rep"], json!([1.0, 0.0]));
        assert_eq!(s["by_harness_model"][0]["first_dispatches"], 3);
        assert_eq!(s["by_harness_model"][0]["attempts"], 3);
        assert_eq!(s["by_task"]["a"]["runs"], 2);
    }

    #[test]
    fn utc_string_known_dates() {
        assert_eq!(utc_string(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc_string(1_794_332_378), "2026-11-10T17:39:38Z");
    }
}
