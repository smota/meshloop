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

fn write_config_with_concurrency(
    dir: &Path,
    invoke_args_template: &str,
    concurrency: u32,
) -> PathBuf {
    let config_path = dir.join("meshloop.toml");
    let contents = format!(
        r#"
selected_harnesses = ["fixture"]

[limits]
max_concurrent_workers = {concurrency}
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
fn resume_restart_reruns_accepted_plan_without_new_graph_id() {
    let dir = disposable_repo("restart");
    let fail_config = write_config(&dir, r#"["--noop"]"#);
    let plan_path = dir.join("plan.json");
    let db_path = dir.join(".meshloop").join("state.sqlite");
    let worktree_base = dir.join("worktrees");
    fs::create_dir_all(dir.join(".meshloop")).unwrap();
    fs::write(
        &plan_path,
        r#"{"graph_id":"g-restart","nodes":[{"id":1,"description":"first","depends_on":[],"tier":null}]}"#,
    )
    .unwrap();

    let fail = meshloop()
        .current_dir(&dir)
        .args(["run", "--plan"])
        .arg(&plan_path)
        .args(["--accept-plan", "--config"])
        .arg(&fail_config)
        .args(["--worktree-base"])
        .arg(&worktree_base)
        .args(["--db"])
        .arg(&db_path)
        .output()
        .expect("first run");
    let fail_out = format!(
        "{}{}",
        String::from_utf8_lossy(&fail.stdout),
        String::from_utf8_lossy(&fail.stderr)
    );
    assert!(
        fail_out.contains("Failed")
            || fail_out.contains("FailedTerminal")
            || !fail.status.success(),
        "expected failed first wave, got: {fail_out}"
    );

    let again = meshloop()
        .current_dir(&dir)
        .args(["run", "--plan"])
        .arg(&plan_path)
        .args(["--accept-plan", "--config"])
        .arg(&fail_config)
        .args(["--worktree-base"])
        .arg(&worktree_base)
        .args(["--db"])
        .arg(&db_path)
        .output()
        .expect("duplicate run");
    let again_err = String::from_utf8_lossy(&again.stderr);
    assert!(
        !again.status.success(),
        "second run without --reset must refuse"
    );
    assert!(
        again_err.contains("resume --restart") || again_err.contains("--reset"),
        "duplicate graph should point at restart, got: {again_err}"
    );

    let ok_config = write_config(&dir, r#"["--prompt-file", "{prompt_file}"]"#);
    let restart = meshloop()
        .current_dir(&dir)
        .args(["resume", "--restart", "--config"])
        .arg(&ok_config)
        .args(["--db"])
        .arg(&db_path)
        .args(["--worktree-base"])
        .arg(&worktree_base)
        .output()
        .expect("restart");
    let restart_out = format!(
        "{}{}",
        String::from_utf8_lossy(&restart.stdout),
        String::from_utf8_lossy(&restart.stderr)
    );
    assert!(
        restart.status.success(),
        "restart should re-run the accepted plan, got: {restart_out}"
    );
    assert!(
        restart_out.contains("AwaitingReview") || restart_out.contains("await `meshloop accept`"),
        "expected a fresh dispatch after restart, got: {restart_out}"
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

#[test]
fn bundle_emits_version_locked_session_pack() {
    let dest = std::env::temp_dir().join(format!(
        "meshloop-cli-bundle-{}-{}",
        std::process::id(),
        "pack"
    ));
    let _ = fs::remove_dir_all(&dest);
    let output = meshloop()
        .args(["bundle", "--dest"])
        .arg(&dest)
        .output()
        .expect("meshloop bundle");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "stdout={stdout} stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("bundled to"));
    let plan = dest.join("skills/meshloop-plan/SKILL.md");
    assert!(plan.is_file(), "missing {}", plan.display());
    let catalog = fs::read_to_string(dest.join("meshloop-mcp-tools.json")).expect("catalog");
    assert!(catalog.contains("\"namespace\": \"meshloop:\""));
    assert!(catalog.contains(env!("CARGO_PKG_VERSION")));
    assert!(catalog.contains("meshloop mcp"));
    let skill = fs::read_to_string(plan).expect("skill");
    assert!(skill.contains("meshloop:plan"));
    assert!(!skill.contains("unprefixed plan should be used"));
    let pack = fs::read_to_string(dest.join("README.md")).expect("pack readme");
    assert!(pack.contains("docs/install.md"));
    fs::remove_dir_all(&dest).ok();
}

#[test]
fn run_concurrent_workers_executes_parallel_tasks() {
    let dir = disposable_repo("concurrent");
    let config_path =
        write_config_with_concurrency(&dir, r#"["--prompt-file", "{prompt_file}"]"#, 2);
    let plan_path = dir.join("plan.json");
    let db_path = dir.join(".meshloop").join("state.sqlite");
    let worktree_base = dir.join("worktrees");
    fs::create_dir_all(dir.join(".meshloop")).unwrap();
    fs::write(
        &plan_path,
        r#"{"graph_id":"g_conc","nodes":[{"id":1,"description":"first","depends_on":[],"tier":null},{"id":2,"description":"second","depends_on":[],"tier":null}]}"#,
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
        .expect("run meshloop run with concurrency=2");

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

    let status_out = meshloop()
        .current_dir(&dir)
        .args(["status", "--config"])
        .arg(&config_path)
        .args(["--db"])
        .arg(&db_path)
        .output()
        .expect("status");
    let status_str = String::from_utf8_lossy(&status_out.stdout);
    assert!(
        status_str.contains("first")
            || status_str.contains("task_id: 1")
            || status_str.contains("1")
    );
    assert!(
        status_str.contains("second")
            || status_str.contains("task_id: 2")
            || status_str.contains("2")
    );

    let accept1 = meshloop()
        .current_dir(&dir)
        .args(["accept", "--task", "1", "--as", "tester", "--config"])
        .arg(&config_path)
        .args(["--db"])
        .arg(&db_path)
        .args(["--worktree-base"])
        .arg(&worktree_base)
        .output()
        .expect("accept 1");
    assert!(
        accept1.status.success(),
        "accept 1: {}",
        String::from_utf8_lossy(&accept1.stderr)
    );

    let accept2 = meshloop()
        .current_dir(&dir)
        .args(["accept", "--task", "2", "--as", "tester", "--config"])
        .arg(&config_path)
        .args(["--db"])
        .arg(&db_path)
        .args(["--worktree-base"])
        .arg(&worktree_base)
        .output()
        .expect("accept 2");
    assert!(
        accept2.status.success(),
        "accept 2: {}",
        String::from_utf8_lossy(&accept2.stderr)
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
    let resume_err = String::from_utf8_lossy(&resume.stderr);
    assert!(
        resume.status.success(),
        "resume stdout: {resume_out}\nstderr: {resume_err}"
    );
    assert!(
        resume_out.contains("Integrated") || resume_out.contains("complete"),
        "expected integrated graph, got stdout:\n{resume_out}\nstderr:\n{resume_err}"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn mutate_plan_inserts_prerequisite_and_resumes_to_completion() {
    let dir = disposable_repo("mutate_insert");
    let config_path = write_config(&dir, r#"["--prompt-file", "{prompt_file}"]"#);
    let plan_path = dir.join("plan.json");
    let db_path = dir.join(".meshloop").join("state.sqlite");
    let worktree_base = dir.join("worktrees");
    fs::create_dir_all(dir.join(".meshloop")).unwrap();
    fs::write(
        &plan_path,
        r#"{"graph_id":"g_mutate_prereq","nodes":[{"id":1,"description":"initial root","depends_on":[],"tier":null},{"id":2,"description":"downstream task","depends_on":[1],"tier":null}]}"#,
    )
    .unwrap();

    // 1. Run initial plan with auto-accept
    let run_out = meshloop()
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
        .expect("run initial plan");
    assert!(
        run_out.status.success(),
        "run failed: {}",
        String::from_utf8_lossy(&run_out.stderr)
    );

    // 2. Accept task 1 (task 2 is still pending downstream)
    let accept1 = meshloop()
        .current_dir(&dir)
        .args(["accept", "--task", "1", "--as", "tester", "--config"])
        .arg(&config_path)
        .args(["--db"])
        .arg(&db_path)
        .args(["--worktree-base"])
        .arg(&worktree_base)
        .output()
        .expect("accept 1");
    assert!(
        accept1.status.success(),
        "accept 1: {}",
        String::from_utf8_lossy(&accept1.stderr)
    );

    // 3. Mutate plan before resume: inject task 3 as prerequisite to task 2
    let mutation_file = dir.join("mutation.json");
    fs::write(
        &mutation_file,
        r#"{"type":"insert_prerequisite","target_task_id":2,"new_tasks":[{"id":3,"description":"injected prerequisite","depends_on":[],"tier":null}]}"#,
    )
    .unwrap();

    let mutate_out = meshloop()
        .current_dir(&dir)
        .args([
            "mutate-plan",
            "--graph",
            "g_mutate_prereq",
            "--mutation-file",
        ])
        .arg(&mutation_file)
        .args(["--config"])
        .arg(&config_path)
        .args(["--db"])
        .arg(&db_path)
        .args(["--worktree-base"])
        .arg(&worktree_base)
        .output()
        .expect("mutate-plan");
    let mutate_str = String::from_utf8_lossy(&mutate_out.stdout);
    assert!(
        mutate_out.status.success(),
        "mutate-plan failed: stdout: {mutate_str}\nstderr: {}",
        String::from_utf8_lossy(&mutate_out.stderr)
    );
    assert!(
        mutate_str.contains("3 nodes"),
        "expected 3 nodes, got: {mutate_str}"
    );

    // 4. Resume 1: integrates task 1, sees task 2 blocked on task 3, and runs task 3
    let resume1 = meshloop()
        .current_dir(&dir)
        .args(["resume", "--config"])
        .arg(&config_path)
        .args(["--db"])
        .arg(&db_path)
        .args(["--worktree-base"])
        .arg(&worktree_base)
        .output()
        .expect("resume 1");
    let r1_out = String::from_utf8_lossy(&resume1.stdout);
    assert!(resume1.status.success(), "resume 1 failed: {r1_out}");
    assert!(
        r1_out.contains("AwaitingReview") || r1_out.contains("await `meshloop accept`"),
        "expected pause for task 3 review, got: {r1_out}"
    );

    // 5. Accept task 3
    let accept3 = meshloop()
        .current_dir(&dir)
        .args(["accept", "--task", "3", "--as", "tester", "--config"])
        .arg(&config_path)
        .args(["--db"])
        .arg(&db_path)
        .args(["--worktree-base"])
        .arg(&worktree_base)
        .output()
        .expect("accept 3");
    assert!(
        accept3.status.success(),
        "accept 3: {}",
        String::from_utf8_lossy(&accept3.stderr)
    );

    // 6. Resume 2: integrates task 3, then task 2 (now unblocked) runs and pauses at AwaitingReview
    let resume2 = meshloop()
        .current_dir(&dir)
        .args(["resume", "--config"])
        .arg(&config_path)
        .args(["--db"])
        .arg(&db_path)
        .args(["--worktree-base"])
        .arg(&worktree_base)
        .output()
        .expect("resume 2");
    let r2_out = String::from_utf8_lossy(&resume2.stdout);
    assert!(resume2.status.success(), "resume 2 failed: {r2_out}");
    assert!(
        r2_out.contains("AwaitingReview") || r2_out.contains("await `meshloop accept`"),
        "expected pause for task 2 review, got: {r2_out}"
    );

    // 7. Accept task 2
    let accept2 = meshloop()
        .current_dir(&dir)
        .args(["accept", "--task", "2", "--as", "tester", "--config"])
        .arg(&config_path)
        .args(["--db"])
        .arg(&db_path)
        .args(["--worktree-base"])
        .arg(&worktree_base)
        .output()
        .expect("accept 2");
    assert!(
        accept2.status.success(),
        "accept 2: {}",
        String::from_utf8_lossy(&accept2.stderr)
    );

    // 8. Final resume: completes all tasks
    let resume4 = meshloop()
        .current_dir(&dir)
        .args(["resume", "--config"])
        .arg(&config_path)
        .args(["--db"])
        .arg(&db_path)
        .args(["--worktree-base"])
        .arg(&worktree_base)
        .output()
        .expect("resume 4");
    let r4_out = String::from_utf8_lossy(&resume4.stdout);
    assert!(resume4.status.success(), "resume 4 failed: {r4_out}");
    assert!(
        r4_out.contains("Integrated") || r4_out.contains("complete"),
        "expected all tasks integrated, got: {r4_out}"
    );

    // 9. Verify status shows all 3 tasks
    let status_out = meshloop()
        .current_dir(&dir)
        .args(["status", "--config"])
        .arg(&config_path)
        .args(["--db"])
        .arg(&db_path)
        .output()
        .expect("status");
    let status_str = String::from_utf8_lossy(&status_out.stdout);
    assert!(
        status_str.contains("injected prerequisite")
            || status_str.contains("task_id: 3")
            || status_str.contains("3")
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn mutate_plan_with_require_review_gates_resume() {
    let dir = disposable_repo("mutate_review");
    let config_path = write_config(&dir, r#"["--prompt-file", "{prompt_file}"]"#);
    let plan_path = dir.join("plan.json");
    let db_path = dir.join(".meshloop").join("state.sqlite");
    let worktree_base = dir.join("worktrees");
    fs::create_dir_all(dir.join(".meshloop")).unwrap();
    fs::write(
        &plan_path,
        r#"{"graph_id":"g_mutate_rev","nodes":[{"id":1,"description":"root node","depends_on":[],"tier":null}]}"#,
    )
    .unwrap();

    // 1. Run and accept task 1
    let run_out = meshloop()
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
        .expect("run initial");
    assert!(run_out.status.success());

    let accept1 = meshloop()
        .current_dir(&dir)
        .args(["accept", "--task", "1", "--as", "tester", "--config"])
        .arg(&config_path)
        .arg(&db_path)
        .args(["--worktree-base"])
        .arg(&worktree_base)
        .output()
        .expect("accept 1");
    assert!(accept1.status.success());

    let resume1 = meshloop()
        .current_dir(&dir)
        .args(["resume", "--config"])
        .arg(&config_path)
        .args(["--db"])
        .arg(&db_path)
        .args(["--worktree-base"])
        .arg(&worktree_base)
        .output()
        .expect("resume 1");
    assert!(resume1.status.success());

    // 2. Mutate with --require-review
    let mutation_file = dir.join("mutation.json");
    fs::write(
        &mutation_file,
        r#"{"type":"append_followup","source_task_id":1,"new_tasks":[{"id":2,"description":"review-gated followup","depends_on":[],"tier":null}]}"#,
    )
    .unwrap();

    let mutate_out = meshloop()
        .current_dir(&dir)
        .args(["mutate-plan", "--graph", "g_mutate_rev", "--mutation-file"])
        .arg(&mutation_file)
        .args(["--require-review", "--config"])
        .arg(&config_path)
        .args(["--db"])
        .arg(&db_path)
        .args(["--worktree-base"])
        .arg(&worktree_base)
        .output()
        .expect("mutate-plan");
    assert!(mutate_out.status.success());

    // 3. Resume must be rejected while graph is in AwaitingPlanReview
    let resume_blocked = meshloop()
        .current_dir(&dir)
        .args(["resume", "--config"])
        .arg(&config_path)
        .args(["--db"])
        .arg(&db_path)
        .args(["--worktree-base"])
        .arg(&worktree_base)
        .output()
        .expect("resume blocked");
    assert!(
        !resume_blocked.status.success(),
        "resume should fail when plan review is required"
    );
    let blocked_err = String::from_utf8_lossy(&resume_blocked.stderr);
    assert!(
        blocked_err.contains("not been accepted")
            || blocked_err.contains("not accepted")
            || blocked_err.contains("AwaitingPlanReview"),
        "expected error regarding plan acceptance, got: {blocked_err}"
    );

    // 4. Accept updated plan via review-plan
    let updated_plan_path = dir.join(".meshloop").join("plan.json");
    let review_out = meshloop()
        .current_dir(&dir)
        .args(["review-plan", "--plan"])
        .arg(&updated_plan_path)
        .args(["--accept", "--as", "lead_reviewer", "--config"])
        .arg(&config_path)
        .args(["--db"])
        .arg(&db_path)
        .output()
        .expect("review-plan accept");
    assert!(
        review_out.status.success(),
        "review-plan accept failed: {}",
        String::from_utf8_lossy(&review_out.stderr)
    );

    // 5. Now resume succeeds and executes task 2
    let resume2 = meshloop()
        .current_dir(&dir)
        .args(["resume", "--config"])
        .arg(&config_path)
        .args(["--db"])
        .arg(&db_path)
        .args(["--worktree-base"])
        .arg(&worktree_base)
        .output()
        .expect("resume 2");
    assert!(resume2.status.success());

    let accept2 = meshloop()
        .current_dir(&dir)
        .args(["accept", "--task", "2", "--as", "tester", "--config"])
        .arg(&config_path)
        .args(["--db"])
        .arg(&db_path)
        .args(["--worktree-base"])
        .arg(&worktree_base)
        .output()
        .expect("accept 2");
    assert!(accept2.status.success());

    let resume3 = meshloop()
        .current_dir(&dir)
        .args(["resume", "--config"])
        .arg(&config_path)
        .args(["--db"])
        .arg(&db_path)
        .args(["--worktree-base"])
        .arg(&worktree_base)
        .output()
        .expect("resume 3");
    assert!(resume3.status.success());

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn run_with_json_outputs_clean_rfc8259_and_authentic_sha256() {
    let dir = disposable_repo("json-clean");
    let config_path = write_config(&dir, r#"["--prompt-file", "{prompt_file}"]"#);
    let plan_path = dir.join("plan.json");
    let plan_text = r#"{"graph_id":"g_clean_json","nodes":[{"id":1,"description":"first task","depends_on":[],"tier":null}]}"#;
    fs::write(&plan_path, plan_text).unwrap();

    let db_path = dir.join("state.sqlite");
    let worktree_base = dir.join("worktrees");

    let run = meshloop()
        .current_dir(&dir)
        .args([
            "run",
            "--plan",
        ])
        .arg(&plan_path)
        .args([
            "--accept-plan",
            "--fixture-only",
            "--json",
            "--config",
        ])
        .arg(&config_path)
        .args(["--db"])
        .arg(&db_path)
        .args(["--worktree-base"])
        .arg(&worktree_base)
        .output()
        .expect("run CLI");

    let stdout_str = String::from_utf8_lossy(&run.stdout);
    let stderr_str = String::from_utf8_lossy(&run.stderr);

    // Issue #3: Notice must be routed to stderr, NOT stdout
    assert!(
        stderr_str.contains("Note: worktrees are kept."),
        "stderr should contain the human notice, got: {stderr_str}"
    );
    assert!(
        !stdout_str.contains("Note: worktrees are kept."),
        "stdout must NOT contain non-JSON preambles, got: {stdout_str}"
    );

    // Issue #3: stdout must start with '{' and end with '}'
    let trimmed = stdout_str.trim();
    assert!(trimmed.starts_with('{'), "stdout must start with '{{': {trimmed}");
    assert!(trimmed.ends_with('}'), "stdout must end with '}}': {trimmed}");

    // Issue #3: stdout must parse cleanly as RFC-8259 JSON
    let parsed: serde_json::Value = serde_json::from_str(trimmed)
        .expect("stdout must be valid RFC-8259 JSON without preprocessing");
    assert_eq!(parsed["ok"], true);
    assert_eq!(parsed["command"], "meshloop:run");

    // Issue #4: plan_id must be present as opaque correlation token (16-hex)
    let plan_id = parsed["data"]["plan_id"].as_str().expect("plan_id present");
    assert_eq!(plan_id.len(), 16);

    // Issue #4: artifact_digest must contain algorithm-tagged authentic sha256
    let artifact = &parsed["data"]["artifact_digest"];
    assert_eq!(artifact["digest_algo"], "sha256");
    let digest = artifact["digest"].as_str().expect("digest string present");
    assert_eq!(digest.len(), 64);
    let persisted_bytes = fs::read(dir.join(".meshloop").join("plan.json")).unwrap();
    assert_eq!(digest, meshloop_domain::digest::sha256_hex(&persisted_bytes));

    // Must NOT have legacy plan_sha256 key in data
    assert!(parsed["data"].get("plan_sha256").is_none());

    fs::remove_dir_all(&dir).ok();
}
