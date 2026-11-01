//! `meshloop watch` on a stopped run: NDJSON state changes and a non-zero exit.

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
        "expected fixture harness at {}",
        path.display()
    );
    path.to_string_lossy().to_string()
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "{args:?} {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn temp_repo(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("meshloop-watch-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q"]);
    git(&dir, &["config", "user.email", "test@test"]);
    git(&dir, &["config", "user.name", "test"]);
    fs::write(dir.join("README.md"), "seed").unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "seed"]);
    dir
}

fn write_config(dir: &Path) -> PathBuf {
    let path = dir.join("meshloop.toml");
    let contents = format!(
        r#"
selected_harnesses = ["fixture"]

[limits]
max_concurrent_workers = 1
max_retries = 1
task_timeout_seconds = 30

[verify]
verify_command = []

[harnesses.fixture]
executable = "{}"
version_args = ["--version"]
invoke_args_template = ["--fail"]
model_ref = "fixture-model"
model_tier = "top"
"#,
        fixture_path().replace('\\', "\\\\")
    );
    fs::write(&path, contents).unwrap();
    path
}

fn meshloop(dir: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_meshloop"));
    cmd.current_dir(dir);
    cmd
}

fn ndjson(out: &std::process::Output) -> Vec<serde_json::Value> {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("NDJSON {line:?}: {e}")))
        .collect()
}

#[test]
fn watch_failed_run_exits_nonzero_and_json_names_the_change() {
    let dir = temp_repo("fail");
    let config = write_config(&dir);
    let db = dir.join("state.sqlite");
    let plan = dir.join("plan.json");
    fs::write(
        &plan,
        r#"{"graph_id":"gfail","nodes":[{"id":1,"description":"first","depends_on":[],"tier":null}]}"#,
    )
    .unwrap();
    let run = meshloop(&dir)
        .args(["run", "--plan"])
        .arg(&plan)
        .args(["--accept-plan", "--fixture-only", "--config"])
        .arg(&config)
        .arg("--db")
        .arg(&db)
        .arg("--worktree-base")
        .arg(dir.join("worktrees"))
        .output()
        .expect("run");
    let run_out = format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );

    let json = meshloop(&dir)
        .args([
            "watch",
            "--graph",
            "gfail",
            "--json",
            "--timeout",
            "2",
            "--config",
        ])
        .arg(&config)
        .arg("--db")
        .arg(&db)
        .output()
        .expect("watch --json");
    assert_eq!(json.status.code(), Some(1), "{run_out}\n{json:?}");
    let lines = ndjson(&json);
    let last = lines.last().expect("exit line");
    assert_eq!(last["exit"], "failed", "{lines:?}");
    assert_eq!(last["nodes"], serde_json::json!([1]), "{lines:?}");
    let changes: Vec<_> = lines
        .iter()
        .filter(|line| line.get("node").is_some())
        .collect();
    assert_eq!(changes.len(), 1, "{lines:?}");
    assert_eq!(changes[0]["node"], 1);
    assert_eq!(changes[0]["to"], "Failed");
    let plans: Vec<_> = lines
        .iter()
        .filter(|line| line["change"] == "plan")
        .collect();
    assert_eq!(plans.len(), 1, "{lines:?}");
    assert_eq!(plans[0]["to"], "PlanAccepted");

    let text = meshloop(&dir)
        .args(["watch", "--graph", "gfail", "--timeout", "2", "--config"])
        .arg(&config)
        .arg("--db")
        .arg(&db)
        .output()
        .expect("watch text");
    assert_eq!(text.status.code(), Some(1), "{text:?}");
    let text_out = String::from_utf8_lossy(&text.stdout);
    let last_line = text_out.lines().next_back().unwrap_or("");
    assert!(
        last_line.contains("failed"),
        "last line should name the reason, got {text_out:?}"
    );
    assert!(
        text_out.contains("-> Failed"),
        "text mode should show the node change: {text_out:?}"
    );
    assert!(
        text_out.contains("plan:"),
        "text mode should show the plan state: {text_out:?}"
    );

    let _ = fs::remove_dir_all(&dir);
}
