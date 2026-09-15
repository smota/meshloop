//! Integration tests for process-tree ownership and grandchild process termination.
//!
//! Implements the verification gates ratified in ADR 0025:
//! 1. `grandchild_dies_on_timeout`: Grandchild processes are reaped on timeout / kill_tree.
//! 2. `grandchild_dies_on_parent_abort`: Grandchild processes are forcibly terminated by OS
//!    kernel when the parent process aborts/crashes (JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE).

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use meshloop_adapters::process::{WindowsProcessView, spawn_owned};
use meshloop_engine::ports::{LiveCheck, ProcessHint, ProcessView};

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_fixture_harness"))
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("meshloop_test_{prefix}_{nanos}"));
    let _ = fs::create_dir_all(&dir);
    dir
}

#[test]
fn grandchild_dies_on_timeout() {
    let tmp = unique_temp_dir("grandchild_dies_on_timeout");
    let pid_file = tmp.join("grandchild.pid");
    let hb_file = tmp.join("heartbeat.txt");

    let mut cmd = std::process::Command::new(fixture_path());
    cmd.arg("--spawn-grandchild").arg(&pid_file).arg(&hb_file);

    let mut owned = spawn_owned(cmd).expect("spawn owned child");

    // Wait until grandchild is running and emitting heartbeats
    let start = Instant::now();
    let mut grandchild_pid: u32 = 0;
    while start.elapsed() < Duration::from_secs(5) {
        if pid_file.exists() && hb_file.exists() {
            let content = fs::read_to_string(&pid_file).unwrap_or_default();
            if let Ok(pid) = content.trim().parse::<u32>() {
                grandchild_pid = pid;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        grandchild_pid > 0,
        "Grandchild failed to report PID within timeout"
    );

    // Verify heartbeat is being written
    let initial_hb = fs::read_to_string(&hb_file).unwrap_or_default();
    std::thread::sleep(Duration::from_millis(150));
    let second_hb = fs::read_to_string(&hb_file).unwrap_or_default();
    assert_ne!(
        initial_hb, second_hb,
        "Grandchild should be actively updating heartbeats"
    );

    // Simulate timeout / orchestrator cancellation by killing the tree
    owned.kill_tree();

    // Verify child is dead
    assert!(owned.try_wait().unwrap().is_some());

    // Verify grandchild is dead (poll up to 3 seconds for Windows OS teardown)
    let view = WindowsProcessView;
    let hint = ProcessHint {
        pid: grandchild_pid,
        image_name: None,
    };
    let dead_start = Instant::now();
    let mut is_live = view.is_live(&hint);
    while is_live == LiveCheck::Live && dead_start.elapsed() < Duration::from_secs(4) {
        std::thread::sleep(Duration::from_millis(50));
        is_live = view.is_live(&hint);
    }
    assert_ne!(
        is_live,
        LiveCheck::Live,
        "Grandchild process must be dead after tree kill"
    );

    // Verify heartbeat file stopped changing
    let frozen_hb = fs::read_to_string(&hb_file).unwrap_or_default();
    std::thread::sleep(Duration::from_millis(200));
    let check_hb = fs::read_to_string(&hb_file).unwrap_or_default();
    assert_eq!(
        frozen_hb, check_hb,
        "Heartbeat must not advance after grandchild termination"
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn grandchild_dies_on_parent_abort() {
    let tmp = unique_temp_dir("grandchild_dies_on_parent_abort");
    let pid_file = tmp.join("grandchild.pid");
    let hb_file = tmp.join("heartbeat.txt");

    // Spawn sub-orchestrator process that spawns grandchild into a Job Object and calls abort()
    let mut cmd = std::process::Command::new(fixture_path());
    cmd.arg("--spawn-in-job-and-abort")
        .arg(&pid_file)
        .arg(&hb_file);

    let mut parent = cmd.spawn().expect("spawn sub-orchestrator");

    // Wait for sub-orchestrator to abort (must exit with non-zero / abnormal exit code)
    let parent_exit = parent.wait().expect("wait for sub-orchestrator abort");
    assert!(
        !parent_exit.success(),
        "Sub-orchestrator must exit with failure due to std::process::abort()"
    );

    // Read grandchild PID
    let pid_content = fs::read_to_string(&pid_file).expect("pid file must have been written");
    let grandchild_pid: u32 = pid_content
        .trim()
        .parse()
        .expect("grandchild PID must be a valid integer");
    assert!(grandchild_pid > 0);

    // Wait for OS kernel to clean up the Job Object via KILL_ON_JOB_CLOSE
    let view = WindowsProcessView;
    let hint = ProcessHint {
        pid: grandchild_pid,
        image_name: None,
    };
    let dead_start = Instant::now();
    let mut is_live = view.is_live(&hint);
    while is_live == LiveCheck::Live && dead_start.elapsed() < Duration::from_secs(4) {
        std::thread::sleep(Duration::from_millis(50));
        is_live = view.is_live(&hint);
    }
    assert_ne!(
        is_live,
        LiveCheck::Live,
        "Grandchild process must be dead after parent abort via Win32 Job Object KILL_ON_JOB_CLOSE"
    );

    // Verify heartbeat file stopped changing
    let frozen_hb = fs::read_to_string(&hb_file).unwrap_or_default();
    std::thread::sleep(Duration::from_millis(200));
    let check_hb = fs::read_to_string(&hb_file).unwrap_or_default();
    assert_eq!(
        frozen_hb, check_hb,
        "Heartbeat must not advance after grandchild termination"
    );

    let _ = fs::remove_dir_all(&tmp);
}
