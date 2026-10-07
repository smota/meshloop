//! Host PID liveness hints. PID reuse is a residual: Live only if the PID is listed
//! *and* the image name matches when known.

use std::process::Command;

pub mod job;
pub use job::{OwnedChild, spawn_owned};

use meshloop_engine::ports::{LiveCheck, ProcessHint, ProcessView};

/// Keeps a child from getting a console window of its own. A console-less parent (such as
/// the detached `resume` worker) otherwise makes Windows allocate one for every console
/// child and hand it to the default terminal, which then reports an error when the job closes.
/// Standard handles are still passed explicitly, so inherited stdio keeps working.
/// No-op off Windows. Overwrites any creation flags already set on `cmd`.
pub fn hide_console(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

pub struct WindowsProcessView;

impl ProcessView for WindowsProcessView {
    fn is_live(&self, hint: &ProcessHint) -> LiveCheck {
        let mut cmd = Command::new("tasklist");
        cmd.args(["/FO", "CSV", "/NH", "/FI", &format!("PID eq {}", hint.pid)]);
        hide_console(&mut cmd);
        let output = cmd.output();
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

/// POSIX liveness via `ps -o stat= -p <pid>`: no row or a zombie (`Z`) is Dead, since a
/// zombie can no longer run. The image name is not checked here.
pub struct PosixProcessView;

impl ProcessView for PosixProcessView {
    fn is_live(&self, hint: &ProcessHint) -> LiveCheck {
        if hint.pid == 0 {
            return LiveCheck::Dead;
        }
        let output = Command::new("ps")
            .args(["-o", "stat=", "-p", &hint.pid.to_string()])
            .output();
        let Ok(out) = output else {
            return LiveCheck::Ambiguous;
        };
        let text = String::from_utf8_lossy(&out.stdout);
        match text.trim().chars().next() {
            None | Some('Z') => LiveCheck::Dead,
            Some(_) => LiveCheck::Live,
        }
    }
}

/// The liveness view for the current host.
#[cfg(windows)]
pub type HostProcessView = WindowsProcessView;
/// The liveness view for the current host.
#[cfg(not(windows))]
pub type HostProcessView = PosixProcessView;

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
        let mut cmd = Command::new("taskkill");
        cmd.args(["/F", "/T", "/PID", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        hide_console(&mut cmd);
        let _ = cmd.status();
    }
    #[cfg(not(windows))]
    {
        use std::process::Stdio;
        // Process group kill first, then the single pid. `-s KILL --` keeps the negative
        // pgid from being parsed as an option by any kill implementation.
        let _ = Command::new("kill")
            .args(["-s", "KILL", "--", &format!("-{pid}")])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("kill")
            .args(["-s", "KILL", "--", &pid.to_string()])
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
        let view = HostProcessView {};
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

    #[cfg(not(windows))]
    #[test]
    fn posix_view_reports_live_then_dead_including_zombie() {
        let mut child = Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep");
        let hint = ProcessHint {
            pid: child.id(),
            image_name: None,
        };
        let view = PosixProcessView;
        assert_eq!(view.is_live(&hint), LiveCheck::Live);
        // Killed but not yet reaped: a zombie must already count as Dead.
        child.kill().expect("kill sleep");
        let start = std::time::Instant::now();
        let mut check = view.is_live(&hint);
        while check == LiveCheck::Live && start.elapsed() < std::time::Duration::from_secs(3) {
            std::thread::sleep(std::time::Duration::from_millis(20));
            check = view.is_live(&hint);
        }
        assert_eq!(check, LiveCheck::Dead);
        let _ = child.wait();
        assert_eq!(view.is_live(&hint), LiveCheck::Dead);
    }
}
