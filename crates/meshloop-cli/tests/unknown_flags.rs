//! An unsupported flag is a non-zero exit, with an `ok:false` envelope under `--json`.

use std::process::Command;

use serde_json::Value;

fn meshloop(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_meshloop"))
        .args(args)
        .output()
        .expect("run meshloop")
}

#[test]
fn unknown_flag_exits_non_zero_with_message() {
    let out = meshloop(&["status", "--bogus"]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("meshloop:status does not accept --bogus"),
        "{stderr}"
    );
}

#[test]
fn unknown_flag_with_json_reports_ok_false() {
    let out = meshloop(&["status", "--bogus", "--json"]);
    assert_eq!(out.status.code(), Some(2));
    let v: Value = serde_json::from_slice(&out.stdout).expect("json envelope");
    assert_eq!(v["ok"], false);
    let error = v["error"].as_str().unwrap_or_default();
    assert!(
        error.contains("--bogus") && error.contains("status"),
        "{error}"
    );
}
