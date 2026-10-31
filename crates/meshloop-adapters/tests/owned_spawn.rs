//! Executed process compatibility and retained ownership contracts (ADR 0035).
use meshloop_adapters::process::{OwnedChild, OwnedSpawnSpec, OwnedStdio, spawn_owned};
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn fixture() -> PathBuf {
    env!("CARGO_BIN_EXE_fixture_harness").into()
}
fn directory() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "meshloop-owned-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}
fn spec() -> OwnedSpawnSpec {
    let mut spec = OwnedSpawnSpec::new(fixture());
    spec.stdin(OwnedStdio::Null)
        .stdout(OwnedStdio::Piped)
        .stderr(OwnedStdio::Piped);
    spec
}
fn collect(mut child: OwnedChild) -> (Vec<u8>, Vec<u8>) {
    let read = |pipe: Option<meshloop_adapters::process::job::OwnedReadPipe>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            pipe.unwrap().read_to_end(&mut bytes).unwrap();
            bytes
        })
    };
    let stdout = read(child.stdout.take());
    let stderr = read(child.stderr.take());
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if start.elapsed() > Duration::from_secs(5) {
            child.kill_tree();
            panic!("owned fixture timed out");
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(status.success());
    (stdout.join().unwrap(), stderr.join().unwrap())
}

#[test]
fn preserves_structured_arguments_unicode_cwd_and_environment() {
    let cwd = directory();
    let arguments = [
        "",
        "plain",
        "space value",
        "tab\there",
        "line\nbreak",
        "embedded\"quote",
        "\\",
        "tail \\",
        "\\\"",
        "Olá 東京 🦀",
    ];
    let mut input = spec();
    input
        .arg("--inspect-spawn")
        .args(arguments)
        .current_dir(&cwd)
        .env_clear()
        .env("ML_OWNED_VALUE", "Olá 東京 🦀")
        .env("ML_OWNED_REMOVE", "remove")
        .env_remove("ML_OWNED_REMOVE");
    let (bytes, error) = collect(spawn_owned(input).unwrap());
    assert!(error.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(report["arguments"], serde_json::json!(arguments));
    assert_eq!(
        PathBuf::from(report["directory"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        cwd.canonicalize().unwrap()
    );
    assert_eq!(report["environment"]["ML_OWNED_VALUE"], "Olá 東京 🦀");
    assert!(report["environment"].get("ML_OWNED_REMOVE").is_none());
    fs::remove_dir_all(cwd).unwrap();
}

#[test]
fn simultaneous_large_pipes_and_piped_stdin_make_progress() {
    let mut input = spec();
    input.args(["--large-output", "262144"]);
    let (stdout, stderr) = collect(spawn_owned(input).unwrap());
    assert_eq!(stdout, vec![b'x'; 262144]);
    assert_eq!(stderr, vec![b'e'; 262144]);
    let mut input = spec();
    input.arg("--echo-stdin").stdin(OwnedStdio::Piped);
    let mut child = spawn_owned(input).unwrap();
    let mut pipe = child.stdin.take().unwrap();
    pipe.write_all(b"structured input\n").unwrap();
    drop(pipe);
    assert_eq!(collect(child).0, b"structured input\n");
}

#[test]
fn invalid_input_is_redacted_and_never_creates_a_child() {
    let mut input = OwnedSpawnSpec::new("private-missing-executable.exe");
    input.arg("private-secret-prompt");
    let error = spawn_owned(input).err().unwrap().to_string();
    assert!(!error.contains("private-"));
    let mut input = spec();
    input.arg("invalid\0argument");
    assert!(spawn_owned(input).is_err());
}

#[cfg(windows)]
#[test]
fn windows_environment_keys_use_os_case_folding_and_batch_is_rejected() {
    let mut input = spec();
    input
        .arg("--inspect-spawn")
        .env_clear()
        .env("ML_ÖWNED", "first")
        .env("ml_öwned", "second")
        .env("ML_REMOVE", "gone")
        .env_remove("ml_remove");
    let (bytes, _) = collect(spawn_owned(input).unwrap());
    let report: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let environment = report["environment"].as_object().unwrap();
    assert_eq!(environment.get("ml_öwned").unwrap(), "second");
    assert!(!environment.contains_key("ML_ÖWNED"));
    assert!(
        !environment
            .keys()
            .any(|key| key.eq_ignore_ascii_case("ML_REMOVE"))
    );
    for name in ["private.cmd", "private.BAT"] {
        assert_eq!(
            spawn_owned(OwnedSpawnSpec::new(name)).err().unwrap().kind(),
            std::io::ErrorKind::InvalidInput
        );
    }
}

#[cfg(windows)]
#[test]
fn windows_retained_job_observes_child_and_immediate_grandchild_with_unrelated_survivor() {
    let cwd = directory();
    let pid = cwd.join("grandchild.pid");
    let heartbeat = cwd.join("heartbeat");
    let mut input = spec();
    input.arg("--spawn-grandchild").arg(&pid).arg(&heartbeat);
    let mut child = spawn_owned(input).unwrap();
    let mut unrelated = meshloop_adapters::process::spawn_command(
        std::process::Command::new(fixture()).args(["--sleep-ms", "30000"]),
    )
    .unwrap();
    let start = Instant::now();
    while !heartbeat.exists() && start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(heartbeat.exists(), "first-action descendant did not start");
    let observed = child.terminate_and_observe(Duration::from_secs(3)).unwrap();
    assert!(observed.job_empty);
    assert!(!observed.direct_child.success());
    assert!(
        child.try_wait().unwrap().is_some(),
        "cached exit remains available after native handle release"
    );
    assert!(unrelated.try_wait().unwrap().is_none());
    unrelated.kill().unwrap();
    unrelated.wait().unwrap();
    fs::remove_dir_all(cwd).unwrap();
}

#[cfg(windows)]
#[test]
fn windows_resolution_matches_std_for_child_path_and_relative_program_with_cwd() {
    use std::process::{Command, Stdio};
    let cwd = directory();
    fs::copy(fixture(), cwd.join("fixture_harness.exe")).unwrap();
    let mut input = OwnedSpawnSpec::new("fixture_harness");
    input
        .arg("--inspect-spawn")
        .env("PATH", &cwd)
        .current_dir(&cwd)
        .stdin(OwnedStdio::Null)
        .stdout(OwnedStdio::Piped)
        .stderr(OwnedStdio::Piped);
    let native: serde_json::Value =
        serde_json::from_slice(&collect(spawn_owned(input).unwrap()).0).unwrap();
    let standard = meshloop_adapters::process::spawn_command(
        Command::new("fixture_harness")
            .arg("--inspect-spawn")
            .env("PATH", &cwd)
            .current_dir(&cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .unwrap()
    .wait_with_output()
    .unwrap();
    let standard: serde_json::Value = serde_json::from_slice(&standard.stdout).unwrap();
    assert_eq!(
        PathBuf::from(native["executable"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        PathBuf::from(standard["executable"].as_str().unwrap())
            .canonicalize()
            .unwrap()
    );
    // This relative subpath is valid only in the parent cwd, not the requested cwd.
    let parent = std::env::current_dir().unwrap();
    let parent = parent.canonicalize().unwrap();
    let executable = fixture().canonicalize().unwrap();
    let from: Vec<_> = parent.components().collect();
    let to: Vec<_> = executable.components().collect();
    let shared = from
        .iter()
        .zip(&to)
        .take_while(|(left, right)| left == right)
        .count();
    let mut relative = PathBuf::new();
    for _ in shared..from.len() {
        relative.push("..");
    }
    for component in &to[shared..] {
        relative.push(component.as_os_str());
    }
    let mut input = OwnedSpawnSpec::new(&relative);
    input
        .arg("--inspect-spawn")
        .current_dir(&cwd)
        .stdin(OwnedStdio::Null)
        .stdout(OwnedStdio::Piped)
        .stderr(OwnedStdio::Piped);
    let native: serde_json::Value =
        serde_json::from_slice(&collect(spawn_owned(input).unwrap()).0).unwrap();
    let standard = meshloop_adapters::process::spawn_command(
        Command::new(&relative)
            .arg("--inspect-spawn")
            .current_dir(&cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .unwrap()
    .wait_with_output()
    .unwrap();
    let standard: serde_json::Value = serde_json::from_slice(&standard.stdout).unwrap();
    assert_eq!(
        PathBuf::from(native["executable"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        PathBuf::from(standard["executable"].as_str().unwrap())
            .canonicalize()
            .unwrap()
    );
    fs::remove_dir_all(cwd).unwrap();
}
