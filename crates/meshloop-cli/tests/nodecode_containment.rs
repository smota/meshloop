//! Issue #138: the operator containment recipe for nodecode workers
//! (`adapters/nodecode/meshloop-nodecode-sandbox`) confines writes to the attempt
//! worktree, hides the operator's homes, clears the environment, exposes one
//! credential read-only, and leaves no process behind when its process group or its
//! parent is killed.
//!
//! The check is `adapters/nodecode/containment-test.sh`. It needs bubblewrap and git
//! but no nodecode, no model and no network, and it prints `SKIP:` and passes where
//! bubblewrap or unprivileged user namespaces are unavailable. Linux only.

#![cfg(target_os = "linux")]

use std::path::Path;
use std::process::{Command, Stdio};

#[test]
fn nodecode_sandbox_confines_writes_environment_and_process_tree() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = root.join("adapters/nodecode/containment-test.sh");
    // Cargo's per-test temporary directory lives under target/, on the build disk.
    let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("nodecode-containment");
    std::fs::create_dir_all(&base).expect("create base dir");
    let mut cmd = Command::new("bash");
    cmd.arg(&script)
        .arg(&base)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let out = meshloop_adapters::process::spawn_command(&mut cmd)
        .expect("spawn containment-test.sh")
        .wait_with_output()
        .expect("wait for containment-test.sh");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    println!("{text}");
    assert!(out.status.success(), "containment-test.sh failed:\n{text}");
    assert!(
        text.contains("containment-test: all checks passed") || text.contains("SKIP:"),
        "containment-test.sh neither passed nor skipped:\n{text}"
    );
}
