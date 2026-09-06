//! End-to-end CLI test against the fixture harness. The fixture graph is **canned**
//! (two tasks); tests must not claim the objective was decomposed.
//! Never touches a real subscription-backed harness.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture_path() -> String {
    let exe = format!("fixture_harness{}", std::env::consts::EXE_SUFFIX);
    let mut dir = std::env::current_exe().expect("current test executable path");
    dir.pop();
    dir.pop();
    let path = dir.join(&exe);
    assert!(
        path.exists(),
        "expected fixture harness at {}; build meshloop-adapters first",
        path.display()
    );
    path.to_string_lossy().to_string()
}

fn disposable_repo(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("meshloop-cli-test-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    Command::new("git")
        .current_dir(&dir)
        .args(["init", "-q"])
        .status()
        .unwrap();
    Command::new("git")
        .current_dir(&dir)
        .args(["config", "user.email", "test@test"])
        .status()
        .unwrap();
    Command::new("git")
        .current_dir(&dir)
        .args(["config", "user.name", "test"])
        .status()
        .unwrap();
    fs::write(dir.join("README.md"), "seed").unwrap();
    Command::new("git")
        .current_dir(&dir)
        .args(["add", "."])
        .status()
        .unwrap();
    Command::new("git")
        .current_dir(&dir)
        .args(["commit", "-q", "-m", "seed"])
        .status()
        .unwrap();
    dir
}

fn write_config(dir: &Path, invoke_args_template: &str) -> PathBuf {
    let config_path = dir.join("meshloop.toml");
    let contents = format!(
        r#"
selected_harnesses = ["fixture"]

[limits]
max_concurrent_workers = 1
max_retries = 2
task_timeout_seconds = 30

[verify]
verify_command = []

[harnesses.fixture]
executable = "{}"
version_args = ["--version"]
invoke_args_template = {}
model_ref = "fixture-model"
model_tier = "top"
"#,
        fixture_path().replace('\\', "\\\\"),
        invoke_args_template
    );
    fs::write(&config_path, contents).unwrap();
    config_path
}

fn meshloop() -> Command {
    Command::new(env!("CARGO_BIN_EXE_meshloop"))
}

#[test]
fn plan_writes_the_canned_fixture_graph() {
    let dir = disposable_repo("plan");
    let config_path = write_config(&dir, r#"["--emit-graph"]"#);
    let out_path = dir.join("plan.json");

    let output = meshloop()
        .current_dir(&dir)
        .args(["plan", "--objective", "build a widget", "--config"])
        .arg(&config_path)
        .arg("--out")
        .arg(&out_path)
        .output()
        .expect("run meshloop plan");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "stdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("awaiting-plan-review"));
    assert!(stdout.contains("--accept-plan"));

    let plan_json = fs::read_to_string(&out_path).expect("plan file should exist");
    assert!(plan_json.contains("first task"));
    assert!(plan_json.contains("second task"));
    assert!(
        !plan_json.contains("build a widget"),
        "fixture graph is canned; do not treat it as a decomposition of the objective"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn run_refuses_without_accept_plan_flag() {
    let dir = disposable_repo("gate");
    let config_path = write_config(&dir, r#"["--prompt-file", "{prompt_file}"]"#);
    let plan_path = dir.join("plan.json");
    fs::write(
        &plan_path,
        r#"{"graph_id":"g","nodes":[{"id":1,"description":"do it","depends_on":[],"tier":null}]}"#,
    )
    .unwrap();

    let output = meshloop()
        .current_dir(&dir)
        .args(["run", "--plan"])
        .arg(&plan_path)
        .args(["--config"])
        .arg(&config_path)
        .output()
        .expect("run meshloop run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--accept-plan"));

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn run_pauses_for_human_accept_then_resume_integrates() {
    let dir = disposable_repo("run");
    let config_path = write_config(&dir, r#"["--prompt-file", "{prompt_file}"]"#);
    let plan_path = dir.join("plan.json");
    let db_path = dir.join(".meshloop").join("state.sqlite");
    let worktree_base = dir.join("worktrees");
    fs::create_dir_all(dir.join(".meshloop")).unwrap();
    fs::write(
        &plan_path,
        r#"{"graph_id":"g1","nodes":[{"id":1,"description":"first","depends_on":[],"tier":null}]}"#,
    )
    .unwrap();

    let output = meshloop()
        .current_dir(&dir)
        .args(["run", "--plan"])
        .arg(&plan_path)
        .args(["--accept-plan", "--config"])
        .arg(&config_path)
        .args(["--worktree-base"])
        .arg(&worktree_base)
        .args(["--db"])
        .arg(&db_path)
        .output()
        .expect("run meshloop run");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "stdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("AwaitingReview") || stdout.contains("await `meshloop accept`"),
        "expected pause for human accept, got: {stdout}"
    );

    let accept = meshloop()
        .current_dir(&dir)
        .args(["accept", "--task", "1", "--as", "tester", "--config"])
        .arg(&config_path)
        .args(["--db"])
        .arg(&db_path)
        .args(["--worktree-base"])
        .arg(&worktree_base)
        .output()
        .expect("accept");
    assert!(
        accept.status.success(),
        "accept stderr: {}",
        String::from_utf8_lossy(&accept.stderr)
    );

    let resume = meshloop()
        .current_dir(&dir)
        .args(["resume", "--config"])
        .arg(&config_path)
        .args(["--db"])
        .arg(&db_path)
        .args(["--worktree-base"])
        .arg(&worktree_base)
        .output()
        .expect("resume");
    let resume_out = String::from_utf8_lossy(&resume.stdout);
    assert!(
        resume.status.success(),
        "resume stdout: {resume_out}\nstderr: {}",
        String::from_utf8_lossy(&resume.stderr)
    );
    assert!(
        resume_out.contains("Integrated") || resume_out.contains("complete"),
        "expected integrated graph, got: {resume_out}"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn empty_diff_fails_verification() {
    let dir = disposable_repo("empty");
    let config_path = write_config(&dir, r#"["--noop"]"#);
    let plan_path = dir.join("plan.json");
    let db_path = dir.join("meshloop.db");
    let worktree_base = dir.join("worktrees");
    fs::write(
        &plan_path,
        r#"{"graph_id":"empty1","nodes":[{"id":1,"description":"noop","depends_on":[],"tier":null}]}"#,
    )
    .unwrap();

    let output = meshloop()
        .current_dir(&dir)
        .args(["run", "--plan"])
        .arg(&plan_path)
        .args(["--accept-plan", "--config"])
        .arg(&config_path)
        .args(["--worktree-base"])
        .arg(&worktree_base)
        .args(["--db"])
        .arg(&db_path)
        .output()
        .expect("run");
    let stdout = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("Failed") || stdout.contains("FailedTerminal") || !output.status.success(),
        "empty diff should fail, got: {stdout}"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn roles_json_is_prefixed_and_namespaced_alias_works() {
    let output = meshloop()
        .args(["roles", "--json"])
        .output()
        .expect("roles");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}");
    assert!(stdout.contains("meshloop:reviewer"));
    assert!(stdout.contains("meshloop:planner"));
    assert!(stdout.contains("\"command\": \"meshloop:roles\""));
    assert!(!stdout.contains("\"id\": \"reviewer\""));

    let aliased = meshloop()
        .args(["meshloop:roles", "--json"])
        .output()
        .expect("alias");
    assert!(String::from_utf8_lossy(&aliased.stdout).contains("meshloop:origin"));
}

#[test]
fn doctor_json_includes_origin_and_live_default() {
    let output = meshloop()
        .args(["doctor", "--json"])
        .output()
        .expect("doctor");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}");
    assert!(stdout.contains("meshloop:doctor"));
    assert!(stdout.contains("origin_session"));
    assert!(stdout.contains("live_default"));
    assert!(stdout.contains("ci-double"));
}

#[test]
fn unprefixed_reviewer_is_rejected() {
    let output = meshloop().args(["reviewer"]).output().expect("reviewer");
    assert!(!output.status.success());
    let err = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(err.contains("unprefixed"));
}

#[test]
fn orchestrate_prints_prefixed_reviewer_matrix() {
    let output = meshloop()
        .args([
            "orchestrate",
            "--task",
            "1",
            "--model-a",
            "claude",
            "--model-b",
            "codex",
            "--json",
        ])
        .output()
        .expect("orchestrate");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "stdout={stdout} stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("meshloop:orchestrate"));
    assert!(stdout.contains("meshloop:reviewer"));
    assert!(stdout.contains("untrusted"));
    assert!(stdout.contains("\"live_executed\": false"));
}

#[test]
fn orchestrate_without_origin_session_is_matrix_only() {
    let output = meshloop()
        .args([
            "orchestrate",
            "--task",
            "1",
            "--model-a",
            "claude",
            "--model-b",
            "codex",
            "--json",
        ])
        .output()
        .expect("orchestrate without origin");
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"live_executed\": false"));
}

#[test]
fn orchestrate_pins_fixture_attempt_diff() {
    let dir = disposable_repo("orch-pin");
    let config_path = write_config(&dir, r#"["--prompt-file", "{prompt_file}"]"#);
    let plan_path = dir.join("plan.json");
    let db_path = dir.join(".meshloop").join("state.sqlite");
    let worktree_base = dir.join("worktrees");
    fs::create_dir_all(dir.join(".meshloop")).unwrap();
    fs::write(
        &plan_path,
        r#"{"graph_id":"orch1","nodes":[{"id":1,"description":"first","depends_on":[],"tier":null}]}"#,
    )
    .unwrap();

    let run = meshloop()
        .current_dir(&dir)
        .args(["run", "--plan"])
        .arg(&plan_path)
        .args(["--accept-plan", "--config"])
        .arg(&config_path)
        .args(["--worktree-base"])
        .arg(&worktree_base)
        .args(["--db"])
        .arg(&db_path)
        .output()
        .expect("run");
    assert!(
        run.status.success(),
        "run failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    let orch = meshloop()
        .current_dir(&dir)
        .args([
            "orchestrate",
            "--task",
            "1",
            "--graph",
            "orch1",
            "--model-a",
            "claude",
            "--model-b",
            "codex",
            "--json",
            "--config",
        ])
        .arg(&config_path)
        .args(["--db"])
        .arg(&db_path)
        .args(["--worktree-base"])
        .arg(&worktree_base)
        .output()
        .expect("orchestrate pin");
    let stdout = String::from_utf8_lossy(&orch.stdout);
    assert!(
        orch.status.success(),
        "stdout={stdout} stderr={}",
        String::from_utf8_lossy(&orch.stderr)
    );
    assert!(!stdout.contains("\"base\": \"unresolved\""));
    assert!(stdout.contains("fixture-touched.txt") || stdout.contains("files"));
    assert!(stdout.contains("meshloop:reviewer"));
    assert!(stdout.contains("\"live_executed\": false"));
    let pack = dir
        .join(".meshloop")
        .join("reviews")
        .join("orch1")
        .join("1");
    assert!(
        pack.exists(),
        "expected evidence pack under {}",
        pack.display()
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn review_plan_requires_exactly_one_decision() {
    let output = meshloop()
        .args(["review-plan", "--plan", "meshloop-plan.json"])
        .output()
        .expect("review-plan");
    assert!(!output.status.success());
    let err = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(err.contains("--accept"));
    assert!(err.contains("--decline"));
    assert!(err.contains("--adjust"));
}

#[test]
fn review_plan_accept_then_run_without_accept_plan_flag() {
    let dir = disposable_repo("review-accept");
    let config_path = write_config(&dir, r#"["--prompt-file", "{prompt_file}"]"#);
    let plan_path = dir.join("plan.json");
    let db_path = dir.join(".meshloop").join("state.sqlite");
    fs::create_dir_all(dir.join(".meshloop")).unwrap();
    fs::write(
        &plan_path,
        r#"{"graph_id":"revacc","nodes":[{"id":1,"description":"first","depends_on":[],"tier":null}]}"#,
    )
    .unwrap();

    let reviewed = meshloop()
        .current_dir(&dir)
        .args(["review-plan", "--plan"])
        .arg(&plan_path)
        .args(["--accept", "--as", "sam", "--json", "--config"])
        .arg(&config_path)
        .args(["--db"])
        .arg(&db_path)
        .output()
        .expect("review-plan accept");
    let stdout = String::from_utf8_lossy(&reviewed.stdout);
    assert!(
        reviewed.status.success(),
        "stdout={stdout} stderr={}",
        String::from_utf8_lossy(&reviewed.stderr)
    );
    assert!(stdout.contains("meshloop:review-plan"));
    assert!(stdout.contains("PlanAccepted"));

    let run = meshloop()
        .current_dir(&dir)
        .args(["run", "--plan"])
        .arg(&plan_path)
        .args(["--config"])
        .arg(&config_path)
        .args(["--db"])
        .arg(&db_path)
        .args(["--worktree-base"])
        .arg(dir.join("worktrees"))
        .output()
        .expect("run after review-plan accept");
    assert!(
        run.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn review_plan_decline_blocks_run() {
    let dir = disposable_repo("review-decline");
    let config_path = write_config(&dir, r#"["--prompt-file", "{prompt_file}"]"#);
    let plan_path = dir.join("plan.json");
    let db_path = dir.join(".meshloop").join("state.sqlite");
    fs::create_dir_all(dir.join(".meshloop")).unwrap();
    fs::write(
        &plan_path,
        r#"{"graph_id":"revdec","nodes":[{"id":1,"description":"first","depends_on":[],"tier":null}]}"#,
    )
    .unwrap();

    let reviewed = meshloop()
        .current_dir(&dir)
        .args(["review-plan", "--plan"])
        .arg(&plan_path)
        .args([
            "--decline",
            "--reason",
            "scope too wide",
            "--json",
            "--config",
        ])
        .arg(&config_path)
        .args(["--db"])
        .arg(&db_path)
        .output()
        .expect("review-plan decline");
    assert!(reviewed.status.success());
    let stdout = String::from_utf8_lossy(&reviewed.stdout);
    assert!(stdout.contains("PlanDeclined"));

    let run = meshloop()
        .current_dir(&dir)
        .args(["run", "--plan"])
        .arg(&plan_path)
        .args(["--accept-plan", "--config"])
        .arg(&config_path)
        .args(["--db"])
        .arg(&db_path)
        .output()
        .expect("run after decline");
    assert!(!run.status.success());
    let err = format!(
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(err.contains("declined"));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn review_plan_adjust_rewrites_and_stays_awaiting_review() {
    let dir = disposable_repo("review-adjust");
    let config_path = write_config(&dir, r#"["--emit-graph"]"#);
    let plan_path = dir.join("plan.json");
    let db_path = dir.join(".meshloop").join("state.sqlite");
    fs::create_dir_all(dir.join(".meshloop")).unwrap();
    fs::write(
        &plan_path,
        r#"{"graph_id":"revadj","nodes":[{"id":1,"description":"first","depends_on":[],"tier":null}]}"#,
    )
    .unwrap();

    let reviewed = meshloop()
        .current_dir(&dir)
        .args(["review-plan", "--plan"])
        .arg(&plan_path)
        .args([
            "--adjust",
            "--objective",
            "split into smaller tasks",
            "--json",
            "--config",
        ])
        .arg(&config_path)
        .args(["--db"])
        .arg(&db_path)
        .output()
        .expect("review-plan adjust");
    let stdout = String::from_utf8_lossy(&reviewed.stdout);
    assert!(
        reviewed.status.success(),
        "stdout={stdout} stderr={}",
        String::from_utf8_lossy(&reviewed.stderr)
    );
    assert!(stdout.contains("AwaitingPlanReview"));
    assert!(stdout.contains("adjust"));
    fs::remove_dir_all(&dir).ok();
}
