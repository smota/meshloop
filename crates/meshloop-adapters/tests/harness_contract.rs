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

fn capture_harness(args: &[&str]) -> CliHarness {
    CliHarness::new(CliHarnessConfig {
        name: "fixture".into(),
        executable: fixture_path(),
        version_args: vec!["--version".into()],
        probe_timeout: Duration::from_secs(5),
        invoke_args_template: args.iter().map(|s| (*s).into()).collect(),
    })
}

fn capture_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("meshloop-capture-{tag}-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn bounded_capture_drains_large_simultaneous_pipes_without_deadlock() {
    let dir = capture_dir("large");
    let harness = capture_harness(&["--large-output", "262144"]);
    let handle = harness.invoke(&spec(dir.clone(), 700)).unwrap();
    let outcome = harness.collect(&handle).unwrap();
    assert_eq!(outcome.exit_code, 0);
    assert_eq!(
        outcome.output_redacted,
        format!("{}\u{2026}", "x".repeat(2048))
    );
    fs::remove_dir_all(dir).ok();
}

#[test]
fn bounded_capture_overflow_is_fault_not_partial_success() {
    let dir = capture_dir("overflow");
    let harness = capture_harness(&["--large-output", "4194305"]);
    let handle = harness.invoke(&spec(dir.clone(), 701)).unwrap();
    assert!(
        matches!(harness.collect(&handle), Err(HarnessError::ProcessFault { detail }) if detail == "harness-output-overflow")
    );
    fs::remove_dir_all(dir).ok();
}

#[test]
fn bounded_capture_live_poll_and_cancel_do_not_wait_for_pipe_eof() {
    let dir = capture_dir("poll");
    let harness = capture_harness(&["--sleep-ms", "30000"]);
    let handle = harness.invoke(&spec(dir.clone(), 702)).unwrap();
    let start = std::time::Instant::now();
    assert!(harness.try_collect(&handle).unwrap().is_none());
    assert!(start.elapsed() < Duration::from_secs(1));
    let start = std::time::Instant::now();
    harness.cancel(&handle).unwrap();
    assert!(start.elapsed() < Duration::from_secs(5));
    fs::remove_dir_all(dir).ok();
}

#[test]
fn bounded_capture_inherited_pipes_do_not_block_polling_or_collection() {
    let dir = capture_dir("inherited");
    let harness = capture_harness(&["--exit-with-inherited-pipes"]);
    let mut input = spec(dir.clone(), 703);
    input.timeout = Duration::from_millis(500);
    let handle = harness.invoke(&input).unwrap();
    let start = std::time::Instant::now();
    loop {
        let poll = std::time::Instant::now();
        let result = harness.try_collect(&handle);
        assert!(poll.elapsed() < Duration::from_secs(1));
        match result {
            Ok(Some(outcome)) => {
                assert_eq!(outcome.exit_code, 0);
                break;
            }
            Err(HarnessError::ProcessFault { detail })
                if detail == "harness-output-eof-timeout" =>
            {
                break;
            }
            other => {
                assert!(matches!(other, Ok(None)), "{other:?}");
            }
        }
        assert!(start.elapsed() < Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(10));
    }
    fs::remove_dir_all(dir).ok();
}

#[test]
fn bounded_capture_preserves_capacity_exhaustion_signal() {
    let dir = capture_dir("quota");
    let harness = capture_harness(&["--exhausted"]);
    let handle = harness.invoke(&spec(dir.clone(), 704)).unwrap();
    assert!(matches!(
        harness.collect(&handle),
        Err(HarnessError::CapacityExhausted { .. })
    ));
    fs::remove_dir_all(dir).ok();
}

#[test]
fn bounded_capture_probe_overflow_reports_capture_fault() {
    let harness = CliHarness::new(CliHarnessConfig {
        name: "fixture".into(),
        executable: fixture_path(),
        version_args: vec!["--large-output".into(), "4194305".into()],
        probe_timeout: Duration::from_secs(5),
        invoke_args_template: vec!["--noop".into()],
    });
    assert!(
        matches!(harness.probe(), Err(HarnessError::ProcessFault { detail }) if detail == "harness-output-overflow")
    );
}

#[test]
fn bounded_capture_duplicate_live_attempt_preserves_first_prompt_and_handle() {
    let dir = capture_dir("duplicate");
    let harness = capture_harness(&["--sleep-ms", "30000"]);
    let first = spec(dir.clone(), 705);
    let handle = harness.invoke(&first).unwrap();
    let mut duplicate = first.clone();
    duplicate.prompt = "must not replace first".into();
    let result = harness.invoke(&duplicate);
    let original = fs::read_to_string(dir.join(".meshloop-prompt-705")).unwrap();
    let poll = harness.try_collect(&handle);
    harness.cancel(&handle).unwrap();
    assert!(result.is_err());
    assert_eq!(original, first.prompt);
    assert!(poll.unwrap().is_none());
    fs::remove_dir_all(dir).ok();
}

#[test]
fn bounded_capture_failed_spawn_releases_admission_and_attempt_reservation() {
    let dir = capture_dir("failed-spawn");
    let harness = CliHarness::new(CliHarnessConfig {
        name: "fixture".into(),
        executable: dir.join("missing-executable"),
        version_args: vec![],
        probe_timeout: Duration::from_secs(5),
        invoke_args_template: vec!["--noop".into()],
    });
    for _ in 0..20 {
        assert!(
            matches!(harness.invoke(&spec(dir.clone(), 706)), Err(HarnessError::ProcessFault { detail }) if detail != "harness-output-capacity-blocked" && detail != "harness-output-capture-failed")
        );
    }
    fs::remove_dir_all(dir).ok();
}
