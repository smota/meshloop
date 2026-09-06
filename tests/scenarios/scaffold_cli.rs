//! Acceptance checks for the CLI's basic surface. `plan` and `run` are now implemented
//! (ADR 0001/0009/0013 — see crates/meshloop-cli's own tests for their real behavior); this
//! only checks the top-level dispatch: help/version, missing-argument rejection for the
//! real subcommands, and rejection of a genuinely unknown command. It no longer asserts
//! "no orchestration commands are implemented," since that claim is no longer true.
use std::{path::Path, process::Command};

#[test]
fn cli_reports_help_version_and_rejects_bad_invocations() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root");
    for (args, success, marker) in [
        (vec!["--help"], true, "meshloop plan --objective"),
        (vec!["--version"], true, "meshloop 0.1.0"),
        (vec!["plan"], false, "meshloop:plan requires --objective"),
        (vec!["run"], false, "meshloop:run requires --plan"),
        (vec!["reviewer"], false, "unprefixed"),
        (vec!["frobnicate"], false, "Unsupported command"),
    ] {
        let output = Command::new("cargo")
            .args([
                "run",
                "--quiet",
                "--offline",
                "--locked",
                "-p",
                "meshloop-cli",
                "--",
            ])
            .args(args)
            .current_dir(root)
            .output()
            .expect("run CLI");
        assert_eq!(output.status.success(), success);
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(text.contains(marker), "unexpected CLI output: {text}");
    }
}
