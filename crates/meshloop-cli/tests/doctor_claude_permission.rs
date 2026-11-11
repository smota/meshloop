//! Issue #163: doctor warns when a `claude` harness uses print mode with no permission
//! mode, since such a worker cannot write files. The warning comes from config alone.

use std::fs;
use std::path::Path;
use std::process::Command;

fn doctor_warnings(tag: &str, executable: &str, template: &str) -> Vec<String> {
    let base = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("doctor-claude-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let config = base.join("meshloop.toml");
    fs::write(
        &config,
        format!(
            r#"
selected_harnesses = ["c"]

[limits]
max_concurrent_workers = 1
max_retries = 1
task_timeout_seconds = 60

[harnesses.c]
executable = "{executable}"
version_args = ["--version"]
invoke_args_template = {template}
prompt_via = "arg"
model_ref = "m"
model_tier = "top"
"#
        ),
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_meshloop"))
        .current_dir(&base)
        .args(["doctor", "--json", "--config"])
        .arg(&config)
        .arg("--db")
        .arg(base.join("state.sqlite"))
        .output()
        .unwrap();
    let text = Command::new(env!("CARGO_BIN_EXE_meshloop"))
        .current_dir(&base)
        .args(["doctor", "--config"])
        .arg(&config)
        .arg("--db")
        .arg(base.join("state.sqlite"))
        .output()
        .unwrap();
    let _ = fs::remove_dir_all(&base);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("doctor json");
    let w: Vec<String> = v["data"]["warnings"]
        .as_array()
        .expect("warnings")
        .iter()
        .map(|w| w.as_str().unwrap_or_default().to_string())
        .collect();
    let text = String::from_utf8_lossy(&text.stdout);
    assert_eq!(
        w.iter().any(|x| x.contains("print mode")),
        text.contains("warning: claude in print mode"),
        "text and json agree: {text}"
    );
    w
}

fn warned(w: &[String]) -> bool {
    w.iter().any(|x| {
        x == "claude in print mode cannot write files without a permission mode; see harnesses.md"
    })
}

#[test]
fn warns_on_claude_print_without_permission_mode() {
    assert!(warned(&doctor_warnings(
        "a",
        "/opt/bin/claude",
        r#"["--print", "{prompt}"]"#
    )));
    assert!(warned(&doctor_warnings(
        "b",
        "claude.exe",
        r#"["-p", "{prompt}"]"#
    )));
}

#[test]
fn quiet_with_permission_mode_or_other_executable() {
    assert!(!warned(&doctor_warnings(
        "c",
        "claude",
        r#"["--permission-mode", "acceptEdits", "--print", "{prompt}"]"#
    )));
    assert!(!warned(&doctor_warnings(
        "d",
        "claude",
        r#"["--print", "--permission-prompt-tool", "mcp__x__y", "{prompt}"]"#
    )));
    assert!(!warned(&doctor_warnings(
        "e",
        "other",
        r#"["--print", "{prompt}"]"#
    )));
    assert!(!warned(&doctor_warnings("f", "claude", r#"["{prompt}"]"#)));
}
