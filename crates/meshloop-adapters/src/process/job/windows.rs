//! Native atomic Windows owned startup. All unsafe code stays in this adapter.
#![allow(unsafe_code)]

use std::cmp::Ordering;
use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_FAILED,
    WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Globalization::{
    CSTR_EQUAL, CSTR_GREATER_THAN, CSTR_LESS_THAN, CompareStringOrdinal,
};
use windows_sys::Win32::System::Console::{
    GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::JobObjects::*;
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::SystemInformation::{GetSystemDirectoryW, GetWindowsDirectoryW};
use windows_sys::Win32::System::Threading::*;

use super::{OwnedChild, OwnedReadPipe, OwnedTermination, OwnedWritePipe};
use crate::process::owned_spawn::{OwnedSpawnSpec, OwnedStdio};

fn invalid(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, reason)
}

pub(super) fn current_creation_token() -> io::Result<String> {
    use windows_sys::Win32::Foundation::FILETIME;
    let mut creation = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut exit = creation;
    let mut kernel = creation;
    let mut user = creation;
    // SAFETY: the pseudo-handle identifies only this process and is never closed.
    // All four writable FILETIME outputs are initialized, distinct, correctly
    // aligned stack objects that remain live for the complete synchronous call.
    if unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    } == 0
    {
        return Err(native_error("driver-creation-identity-failed"));
    }
    Ok(format!(
        "windows-{:08x}{:08x}",
        creation.dwHighDateTime, creation.dwLowDateTime
    ))
}
fn native_error(operation: &'static str) -> io::Error {
    let error = io::Error::last_os_error();
    io::Error::new(
        error.kind(),
        format!(
            "{operation} (os-code {})",
            error.raw_os_error().unwrap_or(0)
        ),
    )
}
fn redacted_error(operation: &'static str, error: io::Error) -> io::Error {
    io::Error::new(error.kind(), operation)
}
fn wide(value: &OsStr) -> io::Result<Vec<u16>> {
    let value: Vec<_> = value.encode_wide().collect();
    if value.contains(&0) {
        return Err(invalid("owned-input-contains-nul"));
    }
    if value.len() > i32::MAX as usize {
        return Err(invalid("owned-input-too-long"));
    }
    Ok(value)
}
fn terminated(mut value: Vec<u16>) -> Vec<u16> {
    value.push(0);
    value
}
fn handle(value: &OwnedHandle) -> HANDLE {
    value.as_raw_handle() as HANDLE
}

struct Job(OwnedHandle);
impl Job {
    fn create() -> io::Result<Self> {
        // SAFETY: null attributes/name request an anonymous non-inheritable handle.
        let raw = unsafe { CreateJobObjectW(null(), null()) };
        if raw.is_null() {
            return Err(native_error("owned-job-create-failed"));
        }
        // SAFETY: a successful CreateJobObjectW transfers one valid owned handle.
        let job = Self(unsafe { OwnedHandle::from_raw_handle(raw) });
        // SAFETY: this Win32 POD structure has a valid all-zero initial state.
        let mut information: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        information.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: the live job and correctly sized initialized POD buffer outlive the call.
        if unsafe {
            SetInformationJobObject(
                handle(&job.0),
                JobObjectExtendedLimitInformation,
                &information as *const _ as _,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        } == 0
        {
            return Err(native_error("owned-job-configure-failed"));
        }
        Ok(job)
    }
    fn terminate(&self) -> io::Result<()> {
        // SAFETY: this privately owned job handle is live; no PID lookup or fallback occurs.
        if unsafe { TerminateJobObject(handle(&self.0), 1) } == 0 {
            return Err(native_error("owned-job-termination-failed"));
        }
        Ok(())
    }
    fn empty(&self) -> io::Result<bool> {
        // SAFETY: this Win32 POD structure has a valid all-zero initial state.
        let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { zeroed() };
        // SAFETY: the writable buffer has the exact API-specified type/size and the job is live.
        if unsafe {
            QueryInformationJobObject(
                handle(&self.0),
                JobObjectBasicAccountingInformation,
                &mut accounting as *mut _ as _,
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                null_mut(),
            )
        } == 0
        {
            return Err(native_error("owned-job-query-failed"));
        }
        Ok(accounting.ActiveProcesses == 0)
    }
}

pub(super) struct WindowsChild {
    process: Option<OwnedHandle>,
    job: Job,
    pid: u32,
    status: Option<ExitStatus>,
}
impl WindowsChild {
    pub(super) fn id(&self) -> u32 {
        self.pid
    }
    pub(super) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        if let Some(status) = self.status {
            return Ok(Some(status));
        }
        let process = self
            .process
            .as_ref()
            .ok_or_else(|| io::Error::other("owned-child-handle-unavailable"))?;
        // SAFETY: process is a live SYNCHRONIZE-capable handle returned by CreateProcessW.
        match unsafe { WaitForSingleObject(handle(process), 0) } {
            WAIT_TIMEOUT => Ok(None),
            WAIT_OBJECT_0 => {
                let mut code = 0;
                // SAFETY: the handle is live and `code` is valid writable storage.
                if unsafe { GetExitCodeProcess(handle(process), &mut code) } == 0 {
                    return Err(native_error("owned-child-exit-query-failed"));
                }
                let status = ExitStatus::from_raw(code);
                self.status = Some(status);
                // Job accounting may retain an exited process while references remain.
                // Cache the observed status first, then release our native reference.
                self.process.take();
                Ok(Some(status))
            }
            WAIT_FAILED => Err(native_error("owned-child-wait-failed")),
            _ => Err(io::Error::other("owned-child-wait-unexpected")),
        }
    }
    pub(super) fn wait(&mut self) -> io::Result<ExitStatus> {
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            // SAFETY: the retained process handle remains valid across this bounded wait.
            let process = self
                .process
                .as_ref()
                .ok_or_else(|| io::Error::other("owned-child-handle-unavailable"))?;
            if unsafe { WaitForSingleObject(handle(process), 100) } == WAIT_FAILED {
                return Err(native_error("owned-child-wait-failed"));
            }
        }
    }
    pub(super) fn terminate_and_observe(
        &mut self,
        timeout: Duration,
    ) -> io::Result<OwnedTermination> {
        self.job.terminate()?;
        self.observe_termination(timeout)
    }
    fn observe_termination(&mut self, timeout: Duration) -> io::Result<OwnedTermination> {
        let start = Instant::now();
        loop {
            let direct = self.try_wait()?;
            // Do not treat direct-child exit or EOF as job-empty evidence.
            let empty = self.job.empty()?;
            if let Some(direct_child) = direct
                && empty
            {
                return Ok(OwnedTermination {
                    direct_child,
                    job_empty: true,
                });
            }
            if start.elapsed() >= timeout {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "owned-job-observation-timeout",
                ));
            }
            std::thread::sleep(
                Duration::from_millis(5).min(timeout.saturating_sub(start.elapsed())),
            );
        }
    }
}

// Win32's attribute-list storage follows native heap alignment (16 bytes on 64-bit,
// 8 on 32-bit Windows). Explicit 16-byte alignment satisfies both, unlike Vec<u8>.
#[repr(C, align(16))]
#[derive(Clone)]
struct AttributeBlock {
    _bytes: [u8; 16],
}
struct Attributes {
    storage: Vec<AttributeBlock>,
    initialized: bool,
}
impl Attributes {
    fn new() -> io::Result<Self> {
        let mut bytes = 0;
        // SAFETY: the documented sizing call accepts a null list and writes its size.
        unsafe { InitializeProcThreadAttributeList(null_mut(), 2, 0, &mut bytes) };
        if bytes == 0 || bytes > 1024 * 1024 {
            return Err(native_error("owned-attributes-size-failed"));
        }
        let mut result = Self {
            storage: vec![AttributeBlock { _bytes: [0; 16] }; bytes.div_ceil(16)],
            initialized: false,
        };
        // SAFETY: storage is 16-byte aligned, initialized, and at least the requested size.
        if unsafe { InitializeProcThreadAttributeList(result.pointer(), 2, 0, &mut bytes) } == 0 {
            // No initialized list exists, so avoid calling Delete on failed initialization.
            return Err(native_error("owned-attributes-init-failed"));
        }
        result.initialized = true;
        Ok(result)
    }
    fn pointer(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.storage.as_mut_ptr().cast()
    }
    // The pointed-to job/handle arrays are borrowed by Win32 until CreateProcessW returns.
    // This function is private; spawn keeps those arrays and their owning handles in scope.
    fn set(&mut self, attribute: u32, values: &[HANDLE]) -> io::Result<()> {
        // SAFETY: list is initialized; values is a correctly aligned contiguous HANDLE
        // array kept alive and unmoved through CreateProcessW by the sole caller.
        if unsafe {
            UpdateProcThreadAttribute(
                self.pointer(),
                0,
                attribute as usize,
                values.as_ptr().cast(),
                std::mem::size_of_val(values),
                null_mut(),
                null(),
            )
        } == 0
        {
            return Err(native_error("owned-attributes-update-failed"));
        }
        Ok(())
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        // SAFETY: only successfully initialized lists are stored in Attributes.
        if self.initialized {
            unsafe { DeleteProcThreadAttributeList(self.pointer()) };
        }
    }
}

fn duplicate(source: HANDLE) -> io::Result<OwnedHandle> {
    let mut result = null_mut();
    // SAFETY: GetCurrentProcess returns the documented pseudo-handle; source is a
    // retained handle or a current std slot. DuplicateHandle validates it. Only the
    // new child-only handle is inheritable; parent handle flags are never changed.
    if unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            source,
            GetCurrentProcess(),
            &mut result,
            0,
            1,
            DUPLICATE_SAME_ACCESS,
        )
    } == 0
    {
        return Err(native_error("owned-stdio-duplicate-failed"));
    }
    // SAFETY: successful DuplicateHandle transfers one new owned handle.
    Ok(unsafe { OwnedHandle::from_raw_handle(result) })
}
struct Stream {
    child: OwnedHandle,
    parent: Option<File>,
}
fn stream(mode: OwnedStdio, index: usize) -> io::Result<Stream> {
    match mode {
        OwnedStdio::Piped => {
            let (mut read, mut write) = (null_mut(), null_mut());
            // SAFETY: output pointers are valid; null security creates two non-inheritable
            // handles. There is no interval in which a parent pipe endpoint is inheritable.
            if unsafe { CreatePipe(&mut read, &mut write, null(), 0) } == 0 {
                return Err(native_error("owned-stdio-pipe-failed"));
            }
            // SAFETY: successful CreatePipe transfers two distinct valid owned handles.
            let read = unsafe { OwnedHandle::from_raw_handle(read) };
            // SAFETY: the write endpoint is independently owned and has not been wrapped.
            let write = unsafe { OwnedHandle::from_raw_handle(write) };
            let (child, parent) = if index == 0 {
                (read, write)
            } else {
                (write, read)
            };
            let inheritable = duplicate(handle(&child))?;
            Ok(Stream {
                child: inheritable,
                parent: Some(File::from(parent)),
            })
        }
        OwnedStdio::Inherit => {
            let slot = [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE][index];
            // SAFETY: GetStdHandle has no preconditions. Missing slots map to NUL below.
            let source = unsafe { GetStdHandle(slot) };
            if !source.is_null() && source != INVALID_HANDLE_VALUE {
                return Ok(Stream {
                    child: duplicate(source)?,
                    parent: None,
                });
            }
            stream(OwnedStdio::Null, index)
        }
        OwnedStdio::Null => {
            let file = OpenOptions::new()
                .read(index == 0)
                .write(index != 0)
                .open("NUL")
                .map_err(|e| redacted_error("owned-stdio-null-failed", e))?;
            Ok(Stream {
                child: duplicate(file.as_raw_handle() as HANDLE)?,
                parent: None,
            })
        }
    }
}

fn compare(left: &[u16], right: &[u16]) -> io::Result<Ordering> {
    // SAFETY: both slices have valid pointers/lengths and contain no NUL. Windows
    // ordinal case folding preserves exact original UTF-16 spellings in the block.
    match unsafe {
        CompareStringOrdinal(
            left.as_ptr(),
            left.len() as i32,
            right.as_ptr(),
            right.len() as i32,
            1,
        )
    } {
        CSTR_LESS_THAN => Ok(Ordering::Less),
        CSTR_EQUAL => Ok(Ordering::Equal),
        CSTR_GREATER_THAN => Ok(Ordering::Greater),
        _ => Err(native_error("owned-environment-comparison-failed")),
    }
}
fn environment(spec: &OwnedSpawnSpec) -> io::Result<Vec<(Vec<u16>, Vec<u16>)>> {
    let mut values = Vec::new();
    if !spec.clear_environment {
        for (key, value) in std::env::vars_os() {
            values.push((wide(&key)?, wide(&value)?));
        }
    }
    for (key, value) in &spec.environment {
        let key = wide(key)?;
        if key.is_empty() || key.contains(&(b'=' as u16)) {
            return Err(invalid("owned-environment-key-invalid"));
        }
        let mut index = 0;
        while index < values.len() {
            if compare(&values[index].0, &key)? == Ordering::Equal {
                values.remove(index);
            } else {
                index += 1;
            }
        }
        if let Some(value) = value {
            values.push((key, wide(value)?));
        }
    }
    let mut failure = None;
    values.sort_by(|(left, _), (right, _)| match compare(left, right) {
        Ok(order) => order,
        Err(error) => {
            failure = Some(error);
            Ordering::Equal
        }
    });
    if let Some(error) = failure {
        return Err(error);
    }
    Ok(values)
}
fn environment_block(values: &[(Vec<u16>, Vec<u16>)]) -> io::Result<Vec<u16>> {
    let mut block = Vec::new();
    for (key, value) in values {
        block.extend(key);
        block.push(b'=' as u16);
        block.extend(value);
        block.push(0);
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    if block.len() > 8 * 1024 * 1024 {
        return Err(invalid("owned-environment-too-large"));
    }
    Ok(block)
}
fn executable(spec: &OwnedSpawnSpec, values: &[(Vec<u16>, Vec<u16>)]) -> io::Result<PathBuf> {
    let units = wide(&spec.executable)?;
    if units.is_empty() || units.contains(&(b'"' as u16)) || matches!(units.last(), Some(47 | 92)) {
        return Err(invalid("owned-executable-invalid"));
    }
    let name = PathBuf::from(&spec.executable);
    let extension = name
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase());
    if matches!(extension.as_deref(), Some("bat" | "cmd")) {
        return Err(invalid("owned-batch-executable-unsupported"));
    }
    // Match stable Rust Command's Windows resolution: explicit subpaths resolve in
    // the parent's cwd; changed child PATH is searched before application/system
    // directories, followed by parent PATH. Child cwd never supplies a search root.
    let candidate = if units.iter().any(|u| matches!(u, 47 | 92 | 58)) {
        if extension.as_deref() == Some("exe") {
            name
        } else {
            let mut suffixed = spec.executable.clone();
            suffixed.push(".exe");
            let suffixed = PathBuf::from(suffixed);
            if suffixed.is_file() { suffixed } else { name }
        }
    } else {
        let mut search = Vec::new();
        let path_key: Vec<u16> = "PATH".encode_utf16().collect();
        let mut changed_path = spec.clear_environment;
        for (key, _) in &spec.environment {
            changed_path |= compare(&wide(key)?, &path_key)? == Ordering::Equal;
        }
        if changed_path {
            for (key, value) in values {
                if compare(key, &path_key)? == Ordering::Equal {
                    search.extend(
                        std::env::split_paths(&OsString::from_wide(value))
                            .filter(|p| !p.as_os_str().is_empty()),
                    );
                }
            }
        }
        if let Ok(current) = std::env::current_exe()
            && let Some(parent) = current.parent()
        {
            search.push(parent.to_path_buf());
        }
        for system in [true, false] {
            let mut buffer = vec![0u16; 32768];
            // SAFETY: buffer is valid writable UTF-16 storage of the provided length.
            let length = unsafe {
                if system {
                    GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32)
                } else {
                    GetWindowsDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32)
                }
            } as usize;
            if length == 0 || length >= buffer.len() {
                return Err(native_error("owned-system-directory-query-failed"));
            }
            search.push(PathBuf::from(OsString::from_wide(&buffer[..length])));
        }
        if let Some(parent_path) = std::env::var_os("PATH") {
            search
                .extend(std::env::split_paths(&parent_path).filter(|p| !p.as_os_str().is_empty()));
        }
        let mut filename = spec.executable.clone();
        if !units.contains(&(b'.' as u16)) {
            filename.push(".exe");
        }
        search
            .into_iter()
            .map(|directory| directory.join(&filename))
            .find(|path| path.is_file())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "owned-executable-not-found"))?
    };
    std::fs::canonicalize(candidate)
        .map_err(|e| redacted_error("owned-executable-resolution-failed", e))
}
// Microsoft CRT argv contract: quote every argument; double backslashes before
// a quote and before the closing quote. This preserves empty/Unicode arguments.
fn quote(argument: &[u16], result: &mut Vec<u16>) {
    result.push(b'"' as u16);
    let mut slashes = 0;
    for &unit in argument {
        if unit == b'\\' as u16 {
            slashes += 1;
            continue;
        }
        result.extend(std::iter::repeat_n(
            b'\\' as u16,
            if unit == b'"' as u16 {
                slashes * 2 + 1
            } else {
                slashes
            },
        ));
        slashes = 0;
        result.push(unit);
    }
    result.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    result.push(b'"' as u16);
}

pub(super) fn spawn(spec: OwnedSpawnSpec) -> io::Result<OwnedChild> {
    spawn_at_checkpoint(spec, |_| {})
}
fn spawn_at_checkpoint(
    spec: OwnedSpawnSpec,
    checkpoint: impl FnOnce(&mut OwnedChild),
) -> io::Result<OwnedChild> {
    spawn_with_checkpoints(spec, || {}, checkpoint)
}
fn spawn_with_checkpoints(
    spec: OwnedSpawnSpec,
    before_create: impl FnOnce(),
    checkpoint: impl FnOnce(&mut OwnedChild),
) -> io::Result<OwnedChild> {
    spawn_in_job(spec, Job::create()?, before_create, checkpoint)
}
fn spawn_in_job(
    spec: OwnedSpawnSpec,
    job: Job,
    before_create: impl FnOnce(),
    checkpoint: impl FnOnce(&mut OwnedChild),
) -> io::Result<OwnedChild> {
    let values = environment(&spec)?;
    let application = executable(&spec, &values)?;
    let application = terminated(wide(application.as_os_str())?);
    let directory = spec
        .directory
        .as_ref()
        .map(|path| {
            std::fs::canonicalize(path)
                .map_err(|e| redacted_error("owned-directory-resolution-failed", e))
                .and_then(|path| wide(path.as_os_str()))
                .map(terminated)
        })
        .transpose()?;
    let mut command_line = Vec::new();
    quote(&wide(&spec.executable)?, &mut command_line);
    for argument in &spec.arguments {
        command_line.push(b' ' as u16);
        quote(&wide(argument)?, &mut command_line);
    }
    if command_line.len() >= 32767 {
        return Err(invalid("owned-command-line-too-long"));
    }
    command_line.push(0);
    let environment = environment_block(&values)?;
    let guard = crate::process::spawn_guard()?;
    let mut input = stream(spec.streams[0], 0)?;
    let mut output = stream(spec.streams[1], 1)?;
    let mut error = stream(spec.streams[2], 2)?;
    let jobs = [handle(&job.0)];
    let handles = [
        handle(&input.child),
        handle(&output.child),
        handle(&error.child),
    ];
    let mut attributes = Attributes::new()?;
    attributes.set(PROC_THREAD_ATTRIBUTE_JOB_LIST, &jobs)?;
    attributes.set(PROC_THREAD_ATTRIBUTE_HANDLE_LIST, &handles)?;
    // SAFETY: STARTUPINFOEXW and PROCESS_INFORMATION are POD with valid zero state.
    let mut startup: STARTUPINFOEXW = unsafe { zeroed() };
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = handles[0];
    startup.StartupInfo.hStdOutput = handles[1];
    startup.StartupInfo.hStdError = handles[2];
    startup.lpAttributeList = attributes.pointer();
    // SAFETY: PROCESS_INFORMATION is a POD output buffer; only read on success.
    let mut process: PROCESS_INFORMATION = unsafe { zeroed() };
    before_create();
    // SAFETY: all UTF-16 strings are NUL-terminated; mutable command storage, environment,
    // cwd, aligned initialized attributes, job/handle arrays, and the owned handles remain
    // alive and unmoved through this synchronous call. HANDLE_LIST restricts inheritance
    // to the three child-only standard handles. The non-inheritable job is attached by
    // JOB_LIST atomically before any child instruction, with no suspended/assignment gap.
    if unsafe {
        CreateProcessW(
            application.as_ptr(),
            command_line.as_mut_ptr(),
            null(),
            null(),
            1,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT | CREATE_NO_WINDOW,
            environment.as_ptr().cast(),
            directory.as_ref().map_or(null(), |path| path.as_ptr()),
            &startup.StartupInfo,
            &mut process,
        )
    } == 0
    {
        return Err(native_error("owned-process-create-failed"));
    }
    // SAFETY: successful creation transfers two distinct valid handles. These are the
    // only owners; closing the thread handle does not affect the running primary thread.
    let native_process = unsafe { OwnedHandle::from_raw_handle(process.hProcess) };
    // SAFETY: hThread is independently owned and has not been wrapped or closed.
    drop(unsafe { OwnedHandle::from_raw_handle(process.hThread) });
    let mut child = OwnedChild {
        inner: WindowsChild {
            process: Some(native_process),
            job,
            pid: process.dwProcessId,
            status: None,
        },
        stdin: input
            .parent
            .take()
            .map(|pipe| Box::new(pipe) as OwnedWritePipe),
        stdout: output
            .parent
            .take()
            .map(|pipe| Box::new(pipe) as OwnedReadPipe),
        stderr: error
            .parent
            .take()
            .map(|pipe| Box::new(pipe) as OwnedReadPipe),
    };
    // Close every inheritable duplicate before another controlled std/detached
    // launch can run. The parent pipe endpoints have moved into OwnedChild.
    drop(attributes);
    drop(input);
    drop(output);
    drop(error);
    drop(guard);
    checkpoint(&mut child);
    Ok(child)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::{Child, Command, Stdio};
    use std::sync::{Arc, Barrier};
    use std::time::{SystemTime, UNIX_EPOCH};

    const FIXTURE: &str = "process::job::windows::tests::native_fixture";
    const MODE: &str = "ML_ATOMIC_TEST_MODE";
    const ROOT: &str = "ML_ATOMIC_TEST_ROOT";
    fn directory() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "meshloop-atomic-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }
    fn fixture_spec(mode: &str, root: &std::path::Path) -> OwnedSpawnSpec {
        let mut spec = OwnedSpawnSpec::new(std::env::current_exe().unwrap());
        spec.args(["--exact", FIXTURE, "--nocapture"])
            .env(MODE, mode)
            .env(ROOT, root)
            .stdin(OwnedStdio::Null)
            .stdout(OwnedStdio::Null)
            .stderr(OwnedStdio::Null);
        spec
    }
    fn fixture_command(mode: &str, root: &std::path::Path) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", FIXTURE, "--nocapture"])
            .env(MODE, mode)
            .env(ROOT, root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }
    fn wait_file(path: &std::path::Path) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            if fs::read(path).is_ok_and(|bytes| !bytes.is_empty()) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("native fixture checkpoint did not appear");
    }
    fn open_process(pid: u32) -> OwnedHandle {
        // SAFETY: this opens a process belonging to this test's unique fixture only.
        // The recorded pid is opened while the fixture's owner is still held alive.
        let process = unsafe {
            OpenProcess(
                PROCESS_TERMINATE | PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                0,
                pid,
            )
        };
        assert!(!process.is_null());
        // SAFETY: successful OpenProcess transfers a valid owned process handle.
        unsafe { OwnedHandle::from_raw_handle(process) }
    }
    fn dead(process: &OwnedHandle, timeout: Duration) -> bool {
        // SAFETY: the retained fixture process handle has SYNCHRONIZE access.
        unsafe {
            WaitForSingleObject(
                handle(process),
                timeout.as_millis().min(u32::MAX as u128 - 1) as u32,
            ) == WAIT_OBJECT_0
        }
    }
    struct Cleanup {
        owner: Child,
        descendants: Vec<OwnedHandle>,
        root: PathBuf,
    }
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = self.owner.kill();
            let _ = self.owner.wait();
            for process in &self.descendants {
                // SAFETY: cleanup acts only on exact retained handles for our own fixtures.
                unsafe { TerminateProcess(handle(process), 1) };
                let _ = dead(process, Duration::from_secs(2));
            }
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn native_fixture() {
        let Ok(mode) = std::env::var(MODE) else {
            return;
        };
        let root = PathBuf::from(std::env::var_os(ROOT).unwrap());
        match mode.as_str() {
            "heartbeat" => loop {
                fs::write(root.join("heartbeat"), b"running").unwrap();
                std::thread::sleep(Duration::from_millis(10));
            },
            "leader" => {
                // Descendant launch is the fixture's first action after mode decoding.
                let mut grandchild =
                    crate::process::spawn_command(&mut fixture_command("heartbeat", &root))
                        .unwrap();
                fs::write(root.join("grandchild"), grandchild.id().to_string()).unwrap();
                grandchild.wait().unwrap();
            }
            "checkpoint-owner" => {
                let _owned = spawn_at_checkpoint(fixture_spec("leader", &root), |child| {
                    fs::write(root.join("leader"), child.id().to_string()).unwrap();
                    wait_file(&root.join("grandchild"));
                    wait_file(&root.join("heartbeat"));
                    fs::write(root.join("checkpoint"), b"after-create-before-return").unwrap();
                    loop {
                        std::thread::sleep(Duration::from_secs(1));
                    }
                })
                .unwrap();
            }
            "legacy-owner" => {
                let _job = Job::create().unwrap();
                let mut child =
                    crate::process::spawn_command(&mut fixture_command("leader", &root)).unwrap();
                fs::write(root.join("leader"), child.id().to_string()).unwrap();
                wait_file(&root.join("grandchild"));
                wait_file(&root.join("heartbeat"));
                // Deliberately restore the rejected post-launch interval: neither
                // child nor its first-action descendant has entered the private job.
                fs::write(root.join("checkpoint"), b"before-post-launch-assignment").unwrap();
                let _ = child.wait();
            }
            "suspended-owner" => {
                use std::os::windows::process::CommandExt;
                let _job = Job::create().unwrap();
                let mut command = fixture_command("heartbeat", &root);
                command.creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW);
                let mut child = crate::process::spawn_command(&mut command).unwrap();
                fs::write(root.join("leader"), child.id().to_string()).unwrap();
                // Deliberately restore the rejected suspended-before-assignment gap.
                fs::write(root.join("checkpoint"), b"suspended-before-assignment").unwrap();
                let _ = child.wait();
            }
            "sleep" => std::thread::sleep(Duration::from_secs(30)),
            "noop" => {}
            "failure-handles" => check_creation_failure_handle_cleanup(&root),
            "handle-probe" => {
                let raw: usize = std::env::var("ML_ATOMIC_SENTINEL")
                    .unwrap()
                    .parse()
                    .unwrap();
                // SAFETY: SetEvent validates the numeric candidate. If the child
                // reused that value for another handle it cannot signal the parent's
                // kernel event. The parent observes the actual object, not just a
                // coincidentally occupied child handle-table slot.
                unsafe { SetEvent(raw as HANDLE) };
            }
            "missing-stdio" => {
                use windows_sys::Win32::System::Console::SetStdHandle;
                // SAFETY: isolated subprocess only; these slots are not shared with
                // the test runner and the original handles themselves remain owned.
                for slot in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
                    assert_ne!(unsafe { SetStdHandle(slot, null_mut()) }, 0);
                }
                let mut spec = fixture_spec("noop", &root);
                spec.streams = [OwnedStdio::Inherit; 3];
                assert!(spawn(spec).unwrap().wait().unwrap().success());
            }
            _ => panic!("unexpected native fixture mode"),
        }
    }

    fn crash_fixture(mode: &str, expect_survival: bool, grandchild: bool) {
        let root = directory();
        let owner = crate::process::spawn_command(&mut fixture_command(mode, &root)).unwrap();
        let mut cleanup = Cleanup {
            owner,
            descendants: Vec::new(),
            root,
        };
        wait_file(&cleanup.root.join("checkpoint"));
        let leader: u32 = fs::read_to_string(cleanup.root.join("leader"))
            .unwrap()
            .parse()
            .unwrap();
        cleanup.descendants.push(open_process(leader));
        if grandchild {
            let pid: u32 = fs::read_to_string(cleanup.root.join("grandchild"))
                .unwrap()
                .parse()
                .unwrap();
            cleanup.descendants.push(open_process(pid));
        }
        cleanup.owner.kill().unwrap();
        cleanup.owner.wait().unwrap();
        for descendant in &cleanup.descendants {
            assert_eq!(
                dead(
                    descendant,
                    Duration::from_millis(if expect_survival { 200 } else { 3000 })
                ),
                !expect_survival
            );
        }
        if mode == "suspended-owner" {
            assert!(!cleanup.root.join("heartbeat").exists());
        }
    }
    #[test]
    fn owner_crash_after_create_before_wrapper_return_kills_first_action_descendants() {
        crash_fixture("checkpoint-owner", false, true);
    }
    #[test]
    fn negative_control_detects_legacy_post_launch_escape() {
        crash_fixture("legacy-owner", true, true);
    }
    #[test]
    fn negative_control_detects_suspended_unowned_crash_gap() {
        crash_fixture("suspended-owner", true, false);
    }

    #[test]
    fn child_is_in_exact_job_and_unrelated_inheritable_sentinel_is_excluded() {
        use windows_sys::Win32::Foundation::{HANDLE_FLAG_INHERIT, SetHandleInformation};
        // SAFETY: unnamed event with null attributes returns one non-inheritable handle.
        let event = unsafe { CreateEventW(null(), 1, 0, null()) };
        assert!(!event.is_null());
        // SAFETY: CreateEventW succeeded and transfers sole ownership.
        let event = unsafe { OwnedHandle::from_raw_handle(event) };
        // Sentinel creation/mutation and owned creation are coordinated with launchers.
        let guard = crate::process::spawn_guard().unwrap();
        // SAFETY: this test alone owns the event handle and changes its inherit flag.
        assert_ne!(
            unsafe {
                SetHandleInformation(handle(&event), HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT)
            },
            0
        );
        drop(guard);
        let root = directory();
        let mut spec = fixture_spec("handle-probe", &root);
        spec.env("ML_ATOMIC_SENTINEL", (handle(&event) as usize).to_string());
        let mut child = spawn_at_checkpoint(spec, |child| {
            let mut member = 0;
            // SAFETY: both retained handles are valid and member is writable storage.
            assert_ne!(
                unsafe {
                    IsProcessInJob(
                        handle(child.inner.process.as_ref().unwrap()),
                        handle(&child.inner.job.0),
                        &mut member,
                    )
                },
                0
            );
            assert_ne!(member, 0);
        })
        .unwrap();
        assert!(child.wait().unwrap().success());
        // SAFETY: this is the parent's retained sentinel event, initially nonsignaled.
        assert_eq!(
            unsafe { WaitForSingleObject(handle(&event), 0) },
            WAIT_TIMEOUT
        );
        // Negative control: ordinary std inheritance intentionally includes the
        // sentinel, demonstrating that this object-level probe detects a real leak.
        let mut control = fixture_command("handle-probe", &root);
        control.env("ML_ATOMIC_SENTINEL", (handle(&event) as usize).to_string());
        assert!(
            crate::process::spawn_command(&mut control)
                .unwrap()
                .wait()
                .unwrap()
                .success()
        );
        // SAFETY: the same live event now must have been signaled by the control.
        assert_eq!(
            unsafe { WaitForSingleObject(handle(&event), 0) },
            WAIT_OBJECT_0
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_parent_standard_handles_map_to_null_without_parent_mutation() {
        let root = directory();
        let mut child =
            crate::process::spawn_command(&mut fixture_command("missing-stdio", &root)).unwrap();
        assert!(child.wait().unwrap().success());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unsupported_attributes_and_invalid_job_operations_fail_closed() {
        let mut attributes = Attributes::new().unwrap();
        let job = Job::create().unwrap();
        assert!(attributes.set(u32::MAX, &[handle(&job.0)]).is_err());
        // SAFETY: unnamed event creation is used to produce a valid wrong-type handle,
        // so Win32 itself rejects both job operations without invalid-handle UB.
        let event = unsafe { CreateEventW(null(), 1, 0, null()) };
        assert!(!event.is_null());
        // SAFETY: CreateEventW transfers sole ownership. Job's safe methods are tested
        // against this deliberately wrong native object type and must return errors.
        let wrong_job = Job(unsafe { OwnedHandle::from_raw_handle(event) });
        assert!(wrong_job.terminate().is_err());
        assert!(wrong_job.empty().is_err());
        let root = directory();
        let mut child = spawn(fixture_spec("sleep", &root)).unwrap();
        let real_job = std::mem::replace(&mut child.inner.job, wrong_job);
        assert!(
            child
                .terminate_and_observe(Duration::from_millis(10))
                .is_err()
        );
        assert!(
            child
                .inner
                .observe_termination(Duration::from_millis(10))
                .is_err()
        );
        // Restore the actual retained owner and clean up using its exact handles.
        child.inner.job = real_job;
        assert!(
            child
                .terminate_and_observe(Duration::from_secs(2))
                .unwrap()
                .job_empty
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn repeated_post_allocation_create_failures_release_native_handles() {
        let root = directory();
        // Isolate handle counts from concurrent unit tests in the parent runner.
        let mut child =
            crate::process::spawn_command(&mut fixture_command("failure-handles", &root)).unwrap();
        assert!(child.wait().unwrap().success());
        fs::remove_dir_all(root).unwrap();
    }
    fn check_creation_failure_handle_cleanup(root: &std::path::Path) {
        let bad = root.join("invalid.exe");
        fs::write(&bad, b"not a portable executable").unwrap();
        let run = || {
            let mut spec = OwnedSpawnSpec::new(&bad);
            spec.streams = [OwnedStdio::Piped; 3];
            assert!(spawn(spec).is_err());
        };
        run(); // warm native loader/cache paths before measuring
        let count = || {
            let mut count = 0;
            // SAFETY: the current-process pseudo-handle and output pointer are valid.
            assert_ne!(
                unsafe { GetProcessHandleCount(GetCurrentProcess(), &mut count) },
                0
            );
            count
        };
        let before = count();
        for _ in 0..64 {
            run();
        }
        let after = count();
        assert!(
            after <= before,
            "native handle count grew: {before} -> {after}"
        );
        fs::remove_file(bad).unwrap();
    }

    #[test]
    fn assignment_without_native_job_rights_is_rejected_before_child_execution() {
        let root = directory();
        let job = Job::create().unwrap();
        let mut raw = null_mut();
        // JOB_OBJECT_QUERY from WinNT.h; avoid a production SystemServices feature
        // solely for this test's deliberately restricted handle-access mask.
        const JOB_QUERY_ACCESS: u32 = 0x0004;
        // SAFETY: this duplicates our live job into a handle with QUERY access only,
        // deliberately excluding ASSIGN_PROCESS. The duplicate is non-inheritable.
        assert_ne!(
            unsafe {
                DuplicateHandle(
                    GetCurrentProcess(),
                    handle(&job.0),
                    GetCurrentProcess(),
                    &mut raw,
                    JOB_QUERY_ACCESS,
                    0,
                    0,
                )
            },
            0
        );
        // SAFETY: successful duplication transfers one new owned handle.
        let denied = Job(unsafe { OwnedHandle::from_raw_handle(raw) });
        let error = spawn_in_job(fixture_spec("heartbeat", &root), denied, || {}, |_| {})
            .err()
            .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(!root.join("heartbeat").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_owned_standard_and_detached_launches_do_not_retain_output_endpoints() {
        for detached in [false, true] {
            let root = directory();
            let prepared = Arc::new(Barrier::new(2));
            let release = Arc::new(Barrier::new(2));
            let other_prepared = prepared.clone();
            let other_release = release.clone();
            let owned_root = root.clone();
            let owned_launch = std::thread::spawn(move || {
                let mut spec = fixture_spec("noop", &owned_root);
                spec.stdout(OwnedStdio::Piped).stderr(OwnedStdio::Piped);
                spawn_with_checkpoints(
                    spec,
                    || {
                        // Pause with inheritable child-only duplicates live and guard held.
                        other_prepared.wait();
                        other_release.wait();
                    },
                    |_| {},
                )
                .unwrap()
            });
            prepared.wait();
            let (started_send, started_receive) = std::sync::mpsc::channel();
            let (finished_send, finished_receive) = std::sync::mpsc::channel();
            let other_root = root.clone();
            let launch = std::thread::spawn(move || {
                started_send.send(()).unwrap();
                let child = if detached {
                    crate::process::spawn_detached(fixture_command("sleep", &other_root)).unwrap()
                } else {
                    crate::process::spawn_command(&mut fixture_command("sleep", &other_root))
                        .unwrap()
                };
                finished_send.send(()).unwrap();
                child
            });
            started_receive
                .recv_timeout(Duration::from_secs(2))
                .unwrap();
            let blocked = finished_receive
                .recv_timeout(Duration::from_millis(100))
                .is_err();
            release.wait();
            let mut owned = owned_launch.join().unwrap();
            let mut other = launch.join().unwrap();
            let (send, receive) = std::sync::mpsc::channel();
            let mut stdout = owned.stdout.take().unwrap();
            let reader = std::thread::spawn(move || {
                let mut bytes = Vec::new();
                std::io::Read::read_to_end(&mut stdout, &mut bytes).unwrap();
                send.send(()).unwrap();
            });
            assert!(owned.wait().unwrap().success());
            let eof = receive.recv_timeout(Duration::from_secs(2));
            let survived = other.try_wait().unwrap().is_none();
            other.kill().unwrap();
            other.wait().unwrap();
            reader.join().unwrap();
            assert!(
                blocked,
                "controlled launch crossed the inheritable-duplicate window"
            );
            assert!(survived);
            assert!(
                eof.is_ok(),
                "unrelated spawn inherited an owned pipe endpoint"
            );
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn live_owned_job_observation_times_out_and_then_termination_succeeds() {
        let root = directory();
        let mut child = spawn(fixture_spec("sleep", &root)).unwrap();
        let start = Instant::now();
        // Exercise the exact observation loop on a deliberately still-live job.
        let error = child
            .inner
            .observe_termination(Duration::from_millis(100))
            .err()
            .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(child.try_wait().unwrap().is_none());
        assert!(
            child
                .terminate_and_observe(Duration::from_secs(2))
                .unwrap()
                .job_empty
        );
        fs::remove_dir_all(root).unwrap();
    }
}
