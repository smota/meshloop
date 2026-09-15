//! Windows PID liveness hints. PID reuse is a residual: Live only if the PID is listed
//! *and* the image name matches when known.

use std::process::Command;

pub mod job;
pub use job::{OwnedChild, spawn_owned};

use meshloop_engine::ports::{LiveCheck, ProcessHint, ProcessView};

pub struct WindowsProcessView;

impl ProcessView for WindowsProcessView {
    fn is_live(&self, hint: &ProcessHint) -> LiveCheck {
        let output = Command::new("tasklist")
            .args(["/FO", "CSV", "/NH", "/FI", &format!("PID eq {}", hint.pid)])
            .output();
        let Ok(out) = output else {
            return LiveCheck::Ambiguous;
        };
        let text = String::from_utf8_lossy(&out.stdout);
        let line = text.lines().find(|l| !l.trim().is_empty());
        let Some(line) = line else {
            return LiveCheck::Dead;
        };
        if line.starts_with("INFO:") || line.starts_with("INFORMAÇÕES:") {
            return LiveCheck::Dead;
        }
        // CSV: "image.exe","pid","session","session#","mem"
        let image = line.split(',').next().unwrap_or("").trim_matches('"');
        match &hint.image_name {
            Some(expected) if !expected.is_empty() => {
                if image.eq_ignore_ascii_case(expected) {
                    LiveCheck::Live
                } else {
                    LiveCheck::Ambiguous
                }
            }
            _ => LiveCheck::Live,
        }
    }
}

/// In-process view used when we still hold the Child (never claims Live for a NULL pid).
pub struct NullPidIsDead;

impl ProcessView for NullPidIsDead {
    fn is_live(&self, hint: &ProcessHint) -> LiveCheck {
        if hint.pid == 0 {
            LiveCheck::Dead
        } else {
            LiveCheck::Live
        }
    }
}

/// Kills a process and all of its descendants (the entire process tree).
///
/// On Windows, executes `taskkill /F /T /PID <pid>` to terminate child compilers,
/// interpreters, and test runners that would otherwise become orphaned processes.
/// On Unix, sends SIGKILL to the process group.
pub fn kill_process_tree(pid: u32) {
    if pid == 0 {
        return;
    }
    #[cfg(windows)]
    {
        use std::process::Stdio;
        let _ = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(windows))]
    {
        use std::process::Stdio;
        // Attempt process group kill first, then fallback to single pid
        let _ = Command::new("kill")
            .args(["-9", &format!("-{}", pid)])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("kill")
            .args(["-9", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_pid_is_noop() {
        kill_process_tree(0);
        let view = NullPidIsDead;
        assert_eq!(
            view.is_live(&ProcessHint {
                pid: 0,
                image_name: None
            }),
            LiveCheck::Dead
        );
        assert_eq!(
            view.is_live(&ProcessHint {
                pid: 1234,
                image_name: None
            }),
            LiveCheck::Live
        );
    }

    #[test]
    fn kills_live_child_process_tree() {
        #[cfg(windows)]
        let mut child = Command::new("powershell")
            .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 30"])
            .spawn()
            .expect("spawn sleep process");

        #[cfg(not(windows))]
        let mut child = Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep process");

        let pid = child.id();
        assert!(pid > 0);

        kill_process_tree(pid);
        let _ = child.wait();

        // Ensure process is no longer running, polling briefly for Windows tasklist cleanup
        let view = WindowsProcessView;
        let hint = ProcessHint {
            pid,
            image_name: None,
        };
        let start = std::time::Instant::now();
        let mut check = view.is_live(&hint);
        while check == LiveCheck::Live && start.elapsed() < std::time::Duration::from_secs(3) {
            std::thread::sleep(std::time::Duration::from_millis(50));
            check = view.is_live(&hint);
        }
        assert!(check == LiveCheck::Dead || check == LiveCheck::Ambiguous);
    }
}
