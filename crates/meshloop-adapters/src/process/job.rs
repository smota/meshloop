//! Process tree ownership and lifecycle isolation.
//!
//! Implements ADR 0025: Host process-tree ownership via Windows Job Objects and POSIX Process Groups.
//! Guarantees that child and grandchild processes are deterministically killed on timeout,
//! cancellation, or parent abort/crash, maintaining `conc.orphan_process_count = 0`.

#![allow(unsafe_code)]

use std::ops::{Deref, DerefMut};
use std::process::{Child, Command};

#[cfg(windows)]
#[allow(unsafe_code)]
pub struct JobObject {
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
// SAFETY: Win32 Job Object handles are thread-safe kernel object handles that can be closed
// or signaled from any thread in the owning process.
unsafe impl Send for JobObject {}
#[cfg(windows)]
unsafe impl Sync for JobObject {}

#[cfg(windows)]
#[allow(unsafe_code)]
impl JobObject {
    /// Creates a new anonymous Win32 Job Object configured with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
    pub fn create() -> std::io::Result<Self> {
        // SAFETY: Calling CreateJobObjectW with null security attributes and null name creates
        // an anonymous, non-inheritable job object.
        let handle = unsafe {
            windows_sys::Win32::System::JobObjects::CreateJobObjectW(
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error());
        }

        // Configure JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE so the Windows kernel forcefully terminates
        // all processes associated with this job when the handle is closed or when the parent process aborts/crashes.
        let mut info: windows_sys::Win32::System::JobObjects::JOBOBJECT_EXTENDED_LIMIT_INFORMATION =
            unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags =
            windows_sys::Win32::System::JobObjects::JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

        let ret = unsafe {
            windows_sys::Win32::System::JobObjects::SetInformationJobObject(
                handle,
                windows_sys::Win32::System::JobObjects::JobObjectExtendedLimitInformation,
                &info as *const _ as *const std::ffi::c_void,
                std::mem::size_of::<
                    windows_sys::Win32::System::JobObjects::JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                >() as u32,
            )
        };

        if ret == 0 {
            let err = std::io::Error::last_os_error();
            unsafe { windows_sys::Win32::Foundation::CloseHandle(handle) };
            return Err(err);
        }

        Ok(Self { handle })
    }

    /// Assigns an existing process handle to this Job Object.
    pub fn assign_process(
        &self,
        process_handle: std::os::windows::io::RawHandle,
    ) -> std::io::Result<()> {
        // SAFETY: The process_handle is obtained from an active `std::process::Child` handle.
        let ret = unsafe {
            windows_sys::Win32::System::JobObjects::AssignProcessToJobObject(
                self.handle,
                process_handle as windows_sys::Win32::Foundation::HANDLE,
            )
        };
        if ret == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    /// Terminates all processes associated with the Job Object immediately at the kernel level.
    pub fn terminate(&self, exit_code: u32) -> std::io::Result<()> {
        // SAFETY: self.handle is a valid, open Job Object handle.
        let ret = unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.handle, exit_code)
        };
        if ret == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
impl Drop for JobObject {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: self.handle is a valid open Win32 HANDLE created in JobObject::create.
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(self.handle);
            }
        }
    }
}

/// An owned child process bound to a process group or kernel Job Object.
///
/// Dropping or killing this struct ensures that child processes and all of their descendants
/// (grandchildren) are terminated deterministically.
pub struct OwnedChild {
    pub child: Child,
    #[cfg(windows)]
    pub job: JobObject,
}

impl OwnedChild {
    /// Returns the OS-assigned process identifier.
    pub fn id(&self) -> u32 {
        self.child.id()
    }

    /// Attempts to collect the exit status if already exited.
    pub fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }

    /// Waits synchronously for the child process to exit.
    pub fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        self.child.wait()
    }

    /// Kills the child and all descendant processes (entire process tree).
    pub fn kill_tree(&mut self) {
        #[cfg(windows)]
        {
            let _ = self.job.terminate(1);
        }
        super::kill_process_tree(self.child.id());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Deref for OwnedChild {
    type Target = Child;

    fn deref(&self) -> &Self::Target {
        &self.child
    }
}

impl DerefMut for OwnedChild {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.child
    }
}

/// Spawns a `Command` enclosed within an owned process group or Win32 Job Object.
pub fn spawn_owned(mut cmd: Command) -> std::io::Result<OwnedChild> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        let job = JobObject::create()?;
        let mut child = cmd.spawn()?;
        if let Err(err) = job.assign_process(child.as_raw_handle()) {
            // Handle race where child completed and exited immediately before assignment
            if child.try_wait()?.is_none() {
                let _ = child.kill();
                let _ = child.wait();
                return Err(err);
            }
        }
        Ok(OwnedChild { child, job })
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
        let child = cmd.spawn()?;
        Ok(OwnedChild { child })
    }
}
