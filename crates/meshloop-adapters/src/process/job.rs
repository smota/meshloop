//! Owned Windows jobs (ADR 0035) and POSIX process groups (ADR 0025).
use std::io::{self, Read, Write};
use std::process::ExitStatus;
use std::time::Duration;

use super::owned_spawn::OwnedSpawnSpec;

#[cfg(windows)]
mod windows;

pub(super) fn current_creation_token() -> io::Result<Option<String>> {
    #[cfg(windows)]
    {
        windows::current_creation_token().map(Some)
    }
    #[cfg(target_os = "linux")]
    {
        let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
        let stat = std::fs::read_to_string("/proc/self/stat")?;
        // comm (field 2) may contain spaces or parentheses. Fields after its final
        // closing parenthesis start at state (3); starttime is field 22.
        let start = stat
            .rsplit_once(')')
            .and_then(|(_, fields)| fields.split_whitespace().nth(19))
            .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
            .ok_or_else(|| io::Error::other("driver-creation-identity-failed"))?;
        Ok(Some(format!("linux-{}-{start}", boot.trim())))
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Ok(None)
    }
}

/// Parent-owned pipe endpoint. Its concrete native handle stays private.
pub type OwnedReadPipe = Box<dyn Read + Send>;
pub type OwnedWritePipe = Box<dyn Write + Send>;

/// A retained process-tree owner. Dropping the Windows job kills its descendants.
/// POSIX crash recovery remains best effort; a process group has no owner-close hook.
pub struct OwnedChild {
    #[cfg(windows)]
    inner: windows::WindowsChild,
    #[cfg(not(windows))]
    inner: std::process::Child,
    pub stdin: Option<OwnedWritePipe>,
    pub stdout: Option<OwnedReadPipe>,
    pub stderr: Option<OwnedReadPipe>,
}

/// Handle-based observation only; this is not a driver acknowledgement.
pub struct OwnedTermination {
    pub direct_child: ExitStatus,
    pub job_empty: bool,
}

impl OwnedChild {
    pub fn id(&self) -> u32 {
        self.inner.id()
    }
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.inner.try_wait()
    }
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        self.stdin.take();
        self.inner.wait()
    }
    /// Terminates and observes using retained native handles, within `timeout`.
    /// POSIX deliberately refuses Windows job-empty proof.
    pub fn terminate_and_observe(&mut self, timeout: Duration) -> io::Result<OwnedTermination> {
        #[cfg(windows)]
        {
            self.inner.terminate_and_observe(timeout)
        }
        #[cfg(not(windows))]
        {
            let _ = timeout;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "owned-job-proof-unsupported",
            ))
        }
    }
    /// Legacy best-effort cleanup. Its return must never be used as a quiescence proof.
    pub fn kill_tree(&mut self) {
        #[cfg(windows)]
        {
            let _ = self.terminate_and_observe(Duration::from_secs(2));
        }
        #[cfg(not(windows))]
        {
            super::kill_process_tree(self.inner.id());
            let _ = self.inner.kill();
            let _ = self.inner.wait();
        }
    }
}

pub fn spawn_owned(spec: OwnedSpawnSpec) -> io::Result<OwnedChild> {
    #[cfg(windows)]
    {
        windows::spawn(spec)
    }
    #[cfg(not(windows))]
    {
        use super::owned_spawn::OwnedStdio;
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};
        let stdio = |mode| match mode {
            OwnedStdio::Null => Stdio::null(),
            OwnedStdio::Piped => Stdio::piped(),
            OwnedStdio::Inherit => Stdio::inherit(),
        };
        let mut command = Command::new(spec.executable);
        command.args(spec.arguments).process_group(0);
        if let Some(directory) = spec.directory {
            command.current_dir(directory);
        }
        if spec.clear_environment {
            command.env_clear();
        }
        for (key, value) in spec.environment {
            match value {
                Some(value) => {
                    command.env(key, value);
                }
                None => {
                    command.env_remove(key);
                }
            }
        }
        command
            .stdin(stdio(spec.streams[0]))
            .stdout(stdio(spec.streams[1]))
            .stderr(stdio(spec.streams[2]));
        let mut inner = command.spawn()?;
        let stdin = inner
            .stdin
            .take()
            .map(|pipe| Box::new(pipe) as OwnedWritePipe);
        let stdout = inner
            .stdout
            .take()
            .map(|pipe| Box::new(pipe) as OwnedReadPipe);
        let stderr = inner
            .stderr
            .take()
            .map(|pipe| Box::new(pipe) as OwnedReadPipe);
        Ok(OwnedChild {
            inner,
            stdin,
            stdout,
            stderr,
        })
    }
}
