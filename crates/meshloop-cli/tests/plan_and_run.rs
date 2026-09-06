//! End-to-end CLI test against the fixture harness: `plan` produces a reviewable graph,
//! `run` refuses to execute it without `--accept-plan`, then executes it once accepted.
//! Never touches a real subscription-backed harness.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// `CARGO_BIN_EXE_*` only covers binaries within the same package; `fixture_harness` lives
/// in meshloop-adapters, so its path is derived the same way Cargo would have placed it —
/// the target dir sits alongside this test binary's own executable.
fn fixture_path() -> String {
    let exe = format!("fixture_harness{}", std::env::consts::EXE_SUFFIX);
    let mut dir = std::env::current_exe().expect("current test executable path");
    // .../target/<profile>/deps/plan_and_run-<hash>.exe -> .../target/<profile>/
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
fn plan_produces_a_structurally_valid_reviewable_graph() {
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
    assert!(plan_json.contains("fixture-plan"));
    assert!(plan_json.contains("first task"));
    assert!(plan_json.contains("second task"));

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
fn run_dispatches_every_node_and_records_evidence_once_accepted() {
    let dir = disposable_repo("run");
    let config_path = write_config(&dir, r#"["--prompt-file", "{prompt_file}"]"#);
    let plan_path = dir.join("plan.json");
    let db_path = dir.join("meshloop.db");
    let worktree_base = dir.join("worktrees");
    fs::write(
        &plan_path,
        r#"{"graph_id":"g","nodes":[
            {"id":1,"description":"first","depends_on":[],"tier":null},
            {"id":2,"description":"second","depends_on":[1],"tier":null}
        ]}"#,
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
    assert!(stdout.contains("[1] verified via fixture"));
    assert!(stdout.contains("[2] verified via fixture"));
    assert!(db_path.exists());

    fs::remove_dir_all(&dir).ok();
}
