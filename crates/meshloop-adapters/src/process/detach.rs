//! Detached background spawn (ADR 0025 amendment, issue #5).
//!
//! A detached worker must not hold any of the caller's standard handles. On Windows,
//! `CreateProcess` with handle inheritance passes every inheritable handle in the caller to
//! the child, including the pipes a parent gave the caller as stdout/stderr, even though the
//! child's own stdio is null. A caller that captures output (`Command::output()`) would then
//! block until the detached worker exits.

#![allow(unsafe_code)]

use std::process::{Child, Command, Stdio};

/// Spawns `cmd` as a background process in its own process group, with null stdio and
/// without inheriting the caller's standard handles.
pub fn spawn_detached(mut cmd: Command) -> std::io::Result<Child> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        spawn_windows(cmd)
    }
    #[cfg(not(windows))]
    {
        // exec only keeps the fds that were explicitly set up, so the caller's pipes cannot
        // leak here; the own process group mirrors CREATE_NEW_PROCESS_GROUP.
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
        cmd.spawn()
    }
}

#[cfg(windows)]
fn spawn_windows(mut cmd: Command) -> std::io::Result<Child> {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::Foundation::{
        GetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE,
        SetHandleInformation,
    };
    use windows_sys::Win32::System::Console::{
        GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };

    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);

    // Clear the inherit flag on the caller's std handles for the duration of the spawn.
    // Concurrent spawns elsewhere are unaffected: std duplicates inherited stdio into new
    // inheritable handles rather than relying on these flags.
    let mut cleared: Vec<HANDLE> = Vec::new();
    for id in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: GetStdHandle has no preconditions; it returns null or INVALID_HANDLE_VALUE
        // when the slot is unset, both of which are skipped below.
        let handle = unsafe { GetStdHandle(id) };
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            continue;
        }
        let mut flags: u32 = 0;
        // SAFETY: `handle` is a live std handle of this process and `flags` is a valid,
        // writable u32 for the duration of the call.
        if unsafe { GetHandleInformation(handle, &mut flags) } == 0 {
            continue;
        }
        if flags & HANDLE_FLAG_INHERIT == 0 {
            continue; // already non-inheritable, or the same handle as an earlier slot
        }
        // SAFETY: changes only the inherit flag of this process's own live handle.
        if unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) } != 0 {
            cleared.push(handle);
        }
    }

    let result = cmd.spawn();

    for handle in cleared {
        // SAFETY: restores the flag cleared above on the same handle, which this process
        // still owns (std handles are not closed by spawning).
        unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) };
    }
    result
}
