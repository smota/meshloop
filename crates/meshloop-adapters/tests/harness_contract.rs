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
