//! CommandCheckRunner must capture rustc/cargo stderr and drain pipes concurrently.

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use meshloop_adapters::check::CommandCheckRunner;
use meshloop_engine::ports::CheckRunner;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_fixture_harness"))
}

fn disposable_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "meshloop-check-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn captures_rustc_type_errors_from_stderr() {
    let dir = disposable_dir();
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("src/lib.rs"),
        "pub fn f() {\n    let x: &str = 1;\n    let _ = (x, y);\n}\n",
    )
    .unwrap();
    let runner = CommandCheckRunner;
    let argv = vec![
        "rustc".into(),
        "--edition".into(),
        "2021".into(),
        "--crate-type".into(),
        "lib".into(),
        "--emit".into(),
        "metadata".into(),
        "-o".into(),
        "check.rmeta".into(),
        "src/lib.rs".into(),
    ];
    let ev = runner
        .run(&dir, &argv, Duration::from_secs(30))
        .expect("rustc check should return evidence");
    let _ = fs::remove_dir_all(&dir);
    assert_ne!(ev.exit_code, 0, "two type errors must fail rustc");
    assert!(
        ev.output_redacted.contains("E0308"),
        "stderr omitted E0308: {}",
        ev.output_redacted
    );
    assert!(
        ev.output_redacted.contains("E0425"),
        "stderr omitted E0425: {}",
        ev.output_redacted
    );
}

#[test]
fn drains_large_stderr_without_pipe_deadlock() {
    let dir = disposable_dir();
    let runner = CommandCheckRunner;
    let exe = fixture_path().to_string_lossy().into_owned();
    let argv = vec![exe, "--fill-stderr".into(), "65536".into()];
    let ev = runner
        .run(&dir, &argv, Duration::from_secs(10))
        .expect("fill-stderr should complete");
    let _ = fs::remove_dir_all(&dir);
    assert!(
        ev.output_redacted.contains("E0308"),
        "expected rustc-shaped stderr, got {}",
        ev.output_redacted
    );
    // redact() caps at 2 KiB; reaching the cap means the 64 KiB pipe was drained
    // instead of deadlocking on the Windows 4 KiB buffer.
    assert!(
        ev.output_redacted.contains('…') || ev.output_redacted.len() >= 2048,
        "expected redaction cap after draining large stderr, got len {}",
        ev.output_redacted.len()
    );
}
