//! Contract tests against the fixture harness binary (runtime-design.md §2's "fixture/stub
//! harness binary in test fixtures") — never against a real subscription-backed CLI.

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use meshloop_adapters::harness::{CliHarness, CliHarnessConfig};
use meshloop_domain::capability::{Compatibility, HarnessError};
use meshloop_domain::evidence::AttemptId;
use meshloop_domain::task_graph::TaskId;
use meshloop_engine::agent::AgentSpec;
use meshloop_engine::ports::{HarnessCapabilities, HarnessHandle};

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_fixture_harness"))
}

fn harness_with_invoke() -> CliHarness {
    CliHarness::new(CliHarnessConfig {
        name: "fixture".into(),
        executable: fixture_path(),
        version_args: vec!["--version".into()],
        probe_timeout: Duration::from_secs(10),
        invoke_args_template: vec!["--prompt-file".into(), "{prompt_file}".into()],
    })
}

fn spec(worktree: PathBuf, attempt: u32) -> AgentSpec {
    AgentSpec {
        task_id: TaskId(1),
        attempt_id: AttemptId(attempt),
        harness: "fixture".into(),
        model_ref: "m".into(),
        worktree_path: worktree,
        prompt: "do the thing".into(),
        timeout: Duration::from_secs(5),
    }
}

#[test]
fn probe_reports_the_fixtures_real_reported_version() {
    let harness = CliHarness::new(CliHarnessConfig {
        name: "fixture".into(),
        executable: fixture_path(),
        version_args: vec!["--version".into()],
        probe_timeout: Duration::from_secs(10),
        invoke_args_template: vec![],
    });
    let profile = harness.probe().expect("probe should succeed");
    assert_eq!(profile.version, "fixture-harness 1.0.0");
    assert_eq!(profile.compatibility, Compatibility::Compatible);
}

#[test]
fn probe_marks_a_failing_executable_unsupported_not_a_crash() {
    let harness = CliHarness::new(CliHarnessConfig {
        name: "fixture".into(),
        executable: fixture_path(),
        version_args: vec!["--fail".into()],
        probe_timeout: Duration::from_secs(10),
        invoke_args_template: vec![],
    });
    let profile = harness
        .probe()
        .expect("probe should not error, just report unsupported");
    assert_eq!(profile.compatibility, Compatibility::Unsupported);
}

#[test]
fn probe_of_a_non_exiting_version_call_times_out_unsupported() {
    // Regression for issue #9: a bare interactive CLI used to block `plan` forever.
    let harness = CliHarness::new(CliHarnessConfig {
        name: "fixture".into(),
        executable: fixture_path(),
        version_args: vec!["--hang".into()],
        probe_timeout: Duration::from_millis(500),
        invoke_args_template: vec!["--prompt-file".into(), "{prompt_file}".into()],
    });
    let start = std::time::Instant::now();
    let profile = harness
        .probe()
        .expect("a hung probe reports unsupported, not an error");
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "probe must be bounded, took {:?}",
        start.elapsed()
    );
    assert_eq!(profile.compatibility, Compatibility::Unsupported);
    assert!(!profile.is_dispatchable());
    let reason = profile.rejection_reason().unwrap_or_default();
    assert!(reason.contains("probe timed out"), "{reason}");
}

#[test]
fn probe_without_an_invoke_template_is_not_dispatchable_and_says_why() {
    let harness = CliHarness::new(CliHarnessConfig {
        name: "fixture".into(),
        executable: fixture_path(),
        version_args: vec!["--version".into()],
        probe_timeout: Duration::from_secs(10),
        invoke_args_template: vec![],
    });
    let profile = harness.probe().expect("probe should succeed");
    assert_eq!(profile.compatibility, Compatibility::Compatible);
    assert!(!profile.is_dispatchable());
    let reason = profile.rejection_reason().unwrap_or_default();
    assert!(reason.contains("invoke_args_template"), "{reason}");
}

#[test]
fn probe_failure_reason_names_the_exit_code() {
    let harness = CliHarness::new(CliHarnessConfig {
        name: "fixture".into(),
        executable: fixture_path(),
        version_args: vec!["--fail".into()],
        probe_timeout: Duration::from_secs(10),
        invoke_args_template: vec![],
    });
    let profile = harness.probe().expect("probe should succeed");
    let reason = profile.rejection_reason().unwrap_or_default();
    assert!(reason.contains("exited with 1"), "{reason}");
}

#[test]
fn invoke_without_a_configured_template_is_unsupported() {
    let harness = CliHarness::new(CliHarnessConfig {
        name: "fixture".into(),
        executable: fixture_path(),
        version_args: vec!["--version".into()],
        probe_timeout: Duration::from_secs(10),
        invoke_args_template: vec![],
    });
    let dir = std::env::temp_dir();
    let result = harness.invoke(&spec(dir, 1));
    assert!(matches!(result, Err(HarnessError::Unsupported)));
}

#[test]
fn invoke_and_collect_round_trip_the_rendered_prompt() {
    let harness = harness_with_invoke();
    let dir = std::env::temp_dir().join(format!("meshloop-test-{}-a", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let handle = harness
        .invoke(&spec(dir.clone(), 2))
        .expect("invoke should succeed");
    let outcome = harness.collect(&handle).expect("collect should succeed");
    assert_eq!(outcome.exit_code, 0);
    assert!(
        outcome
            .output_redacted
            .contains("FIXTURE_HANDLED:do the thing")
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn cancel_terminates_a_hung_process_and_is_idempotent() {
    let harness = CliHarness::new(CliHarnessConfig {
        name: "fixture".into(),
        executable: fixture_path(),
        version_args: vec!["--version".into()],
        probe_timeout: Duration::from_secs(10),
        invoke_args_template: vec!["--hang".into()],
    });
    let dir = std::env::temp_dir().join(format!("meshloop-test-{}-b", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let handle = harness
        .invoke(&spec(dir.clone(), 3))
        .expect("invoke should succeed");
    harness
        .cancel(&handle)
        .expect("first cancel should succeed");
    harness
        .cancel(&handle)
        .expect("second cancel on an already-gone handle is a no-op");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn collect_on_an_unknown_handle_is_unsupported_not_a_panic() {
    let harness = harness_with_invoke();
    let handle = HarnessHandle {
        attempt_id: AttemptId(999),
        pid: None,
        pane_id: None,
    };
    assert!(matches!(
        harness.collect(&handle),
        Err(HarnessError::ProcessFault { .. })
    ));
}

fn wait_until_dead(pid: u32) -> bool {
    use meshloop_engine::ports::{LiveCheck, ProcessHint, ProcessView};
    let view = meshloop_adapters::process::HostProcessView {};
    let hint = ProcessHint {
        pid,
        image_name: None,
    };
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if view.is_live(&hint) != LiveCheck::Live {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
fn cancel_kills_an_orphaned_tree_left_by_a_previous_process() {
    // Simulates recovery after a crash (#30): the attempt's process tree is still running but
    // this harness instance never started it, so only the recorded pid is known.
    let dir = std::env::temp_dir().join(format!("meshloop-test-{}-orphan", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let pid_file = dir.join("grandchild.pid");
    let mut cmd = std::process::Command::new(fixture_path());
    cmd.arg("--spawn-grandchild")
        .arg(&pid_file)
        .arg(dir.join("heartbeat.txt"));
    let orphan = meshloop_adapters::process::spawn_owned(cmd).expect("spawn orphan tree");
    let leader = orphan.id();
    let start = std::time::Instant::now();
    let grandchild: u32 = loop {
        if let Some(pid) = fs::read_to_string(&pid_file)
            .ok()
            .and_then(|s| s.trim().parse().ok())
        {
            break pid;
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "grandchild never started"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    // Drop ownership without killing, as a crashed process would.
    std::mem::forget(orphan);

    let harness = CliHarness::new(CliHarnessConfig {
        name: "fixture".into(),
        executable: fixture_path(),
        version_args: vec!["--version".into()],
        probe_timeout: Duration::from_secs(10),
        invoke_args_template: vec![],
    });
    let handle = HarnessHandle {
        attempt_id: AttemptId(4242),
        pid: Some(leader),
        pane_id: None,
    };
    harness
        .cancel(&handle)
        .expect("cancel of an untracked attempt");

    assert!(
        wait_until_dead(leader),
        "orphaned leader {leader} survived cancel"
    );
    assert!(
        wait_until_dead(grandchild),
        "orphaned grandchild {grandchild} survived cancel"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn cancel_leaves_an_unrelated_process_with_a_recorded_pid_alone() {
    // A recycled pid must not be killed: the recorded pid now belongs to another executable.
    #[cfg(windows)]
    let mut other = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 30"])
        .spawn()
        .expect("spawn unrelated process");
    #[cfg(not(windows))]
    let mut other = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .expect("spawn unrelated process");
    let harness = CliHarness::new(CliHarnessConfig {
        name: "fixture".into(),
        executable: fixture_path(),
        version_args: vec!["--version".into()],
        probe_timeout: Duration::from_secs(10),
        invoke_args_template: vec![],
    });
    let handle = HarnessHandle {
        attempt_id: AttemptId(4243),
        pid: Some(other.id()),
        pane_id: None,
    };
    harness
        .cancel(&handle)
        .expect("cancel of an untracked attempt");
    std::thread::sleep(Duration::from_millis(300));
    let still_running = other.try_wait().expect("poll unrelated process").is_none();
    let _ = other.kill();
    let _ = other.wait();
    assert!(
        still_running,
        "cancel killed a process that is not this harness"
    );
}
