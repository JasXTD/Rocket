//! A fresh capability-free LPAC per invocation. Only three anonymous pipes enter
//! the container; its temporary per-user profile is removed after the process exits.
use super::IntegrationWorker;
use crate::{
    CancellationToken, CommandLimits, Error, Result,
    runner::{self, Output, ProcessPipes, RunningProcess},
};
use std::{
    ffi::{OsStr, c_void},
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, Write},
    os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::ExitStatusExt,
    },
    path::{Component, Path, PathBuf, Prefix},
    process::ExitStatus,
    ptr,
    time::{Duration, Instant},
};

type Handle = *mut c_void;
const MAX_WORKER: u64 = 64 * 1024 * 1024;
/// How long removing the stage waits for whatever still holds its copy of the worker.
const CLEANUP_WAIT: Duration = Duration::from_secs(5);
// Values from winnt.h and WinBase.h; these are process attributes (INPUT bit set).
const TOKEN_IS_APP_CONTAINER: u32 = 29;
const TOKEN_CAPABILITIES: u32 = 30;
const TOKEN_APP_CONTAINER_SID: u32 = 31;
const ATTRIBUTE_HANDLE_LIST: usize = 0x20002;
const ATTRIBUTE_SECURITY_CAPABILITIES: usize = 0x20009;
const ATTRIBUTE_JOB_LIST: usize = 0x2000d;
const ATTRIBUTE_CHILD_PROCESS_POLICY: usize = 0x2000e;
const ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY: usize = 0x2000f;
const ATTRIBUTE_MITIGATION_POLICY: usize = 0x20007;
#[repr(C)]
struct SecurityAttributes {
    size: u32,
    descriptor: Handle,
    inherit: i32,
}
#[repr(C)]
#[derive(Default)]
struct StartupInfo {
    size: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    columns: u32,
    rows: u32,
    fill: u32,
    flags: u32,
    show: u16,
    reserved_size: u16,
    reserved_bytes: *mut u8,
    stdin: Handle,
    stdout: Handle,
    stderr: Handle,
}
#[repr(C)]
struct StartupInfoEx {
    info: StartupInfo,
    attributes: Handle,
}
#[repr(C)]
#[derive(Default)]
struct ProcessInfo {
    process: Handle,
    thread: Handle,
    process_id: u32,
    thread_id: u32,
}
#[repr(C)]
struct Capabilities {
    sid: Handle,
    capabilities: Handle,
    count: u32,
    reserved: u32,
}
#[repr(C)]
struct GenericMapping {
    read: u32,
    write: u32,
    execute: u32,
    all: u32,
}
#[repr(C)]
#[derive(Default)]
struct BasicLimits {
    process_time: i64,
    job_time: i64,
    flags: u32,
    min_working_set: usize,
    max_working_set: usize,
    active_processes: u32,
    affinity: usize,
    priority: u32,
    scheduling: u32,
}
#[repr(C)]
#[derive(Default)]
struct JobLimits {
    basic: BasicLimits,
    io: [u64; 6],
    process_memory: usize,
    job_memory: usize,
    peak_process_memory: usize,
    peak_job_memory: usize,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateDirectoryW(path: *const u16, security: *const SecurityAttributes) -> i32;
    fn CreatePipe(
        read: *mut Handle,
        write: *mut Handle,
        security: *const SecurityAttributes,
        size: u32,
    ) -> i32;
    fn SetHandleInformation(handle: Handle, mask: u32, flags: u32) -> i32;
    fn InitializeProcThreadAttributeList(
        list: Handle,
        count: u32,
        flags: u32,
        size: *mut usize,
    ) -> i32;
    fn UpdateProcThreadAttribute(
        list: Handle,
        flags: u32,
        attribute: usize,
        value: Handle,
        size: usize,
        previous: Handle,
        returned: *mut usize,
    ) -> i32;
    fn DeleteProcThreadAttributeList(list: Handle);
    fn CreateProcessW(
        app: *const u16,
        command: *mut u16,
        process_security: Handle,
        thread_security: Handle,
        inherit: i32,
        flags: u32,
        environment: Handle,
        directory: *const u16,
        startup: *const StartupInfo,
        info: *mut ProcessInfo,
    ) -> i32;
    fn CreateJobObjectW(security: *const c_void, name: *const u16) -> Handle;
    fn SetInformationJobObject(job: Handle, class: i32, info: *const c_void, size: u32) -> i32;
    fn TerminateJobObject(job: Handle, code: u32) -> i32;
    fn ResumeThread(thread: Handle) -> u32;
    fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
    fn GetExitCodeProcess(process: Handle, code: *mut u32) -> i32;
    fn GetCurrentProcess() -> Handle;
    fn GetWindowsDirectoryW(buffer: *mut u16, size: u32) -> u32;
    fn GetSystemDirectoryW(buffer: *mut u16, size: u32) -> u32;
    fn GetDriveTypeW(root: *const u16) -> u32;
    fn LocalFree(memory: Handle) -> Handle;
}
#[link(name = "advapi32")]
unsafe extern "system" {
    fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> i32;
    fn DuplicateToken(token: Handle, level: i32, duplicate: *mut Handle) -> i32;
    fn AccessCheck(
        descriptor: Handle,
        token: Handle,
        desired: u32,
        mapping: *const GenericMapping,
        privileges: Handle,
        length: *mut u32,
        granted: *mut u32,
        status: *mut i32,
    ) -> i32;
    fn GetTokenInformation(
        token: Handle,
        class: u32,
        info: Handle,
        size: u32,
        returned: *mut u32,
    ) -> i32;
    fn ConvertSidToStringSidW(sid: Handle, text: *mut *mut u16) -> i32;
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        text: *const u16,
        revision: u32,
        descriptor: *mut Handle,
        size: *mut u32,
    ) -> i32;
    fn FreeSid(sid: Handle) -> Handle;
}
#[link(name = "userenv")]
unsafe extern "system" {
    fn CreateAppContainerProfile(
        name: *const u16,
        display: *const u16,
        description: *const u16,
        capabilities: Handle,
        count: u32,
        sid: *mut Handle,
    ) -> i32;
    fn DeleteAppContainerProfile(name: *const u16) -> i32;
}
#[link(name = "bcrypt")]
unsafe extern "system" {
    fn BCryptGenRandom(algorithm: Handle, bytes: *mut u8, length: u32, flags: u32) -> i32;
}

fn checked(value: i32) -> io::Result<()> {
    if value == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
fn at(action: &str, error: io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("{action}: {error}"))
}
fn failure(message: &str) -> Error {
    Error::InvalidInput(format!("integration_worker_security: {message}"))
}
fn wide(value: &OsStr) -> io::Result<Vec<u16>> {
    let mut result: Vec<_> = value.encode_wide().collect();
    if result.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "NUL in worker path",
        ));
    }
    result.push(0);
    Ok(result)
}
fn owned(value: Handle) -> io::Result<OwnedHandle> {
    if value.is_null() || value == -1isize as Handle {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: each successful creation transfers one unique live handle here.
    Ok(unsafe { OwnedHandle::from_raw_handle(value) })
}
struct LocalMemory(Handle);
impl Drop for LocalMemory {
    fn drop(&mut self) {
        /* SAFETY: allocated by a LocalAlloc-family API. */
        unsafe {
            LocalFree(self.0);
        }
    }
}
struct Sid(Handle);
impl Drop for Sid {
    fn drop(&mut self) {
        /* SAFETY: returned by CreateAppContainerProfile. */
        unsafe {
            FreeSid(self.0);
        }
    }
}
struct Profile {
    name: Vec<u16>,
    sid: Sid,
    active: bool,
}
impl Profile {
    fn new(nonce: &str) -> io::Result<Self> {
        let name = wide(OsStr::new(&format!("Rocket.Lua.{nonce}")))?;
        let mut sid = ptr::null_mut();
        // SAFETY: terminated unique names, no capabilities, valid SID output. Never
        // adopt an existing profile: its permissions may have been changed.
        let status = unsafe {
            CreateAppContainerProfile(
                name.as_ptr(),
                name.as_ptr(),
                name.as_ptr(),
                ptr::null_mut(),
                0,
                &mut sid,
            )
        };
        if status < 0 {
            return Err(io::Error::other(format!(
                "CreateAppContainerProfile failed: {status:#x}"
            )));
        }
        Ok(Self {
            name,
            sid: Sid(sid),
            active: true,
        })
    }
    fn cleanup(&mut self) -> io::Result<()> {
        if self.active {
            // SAFETY: this unique profile was created by us, and the worker and
            // its handles are gone before explicit cleanup. No shared profile is deleted.
            let status = unsafe { DeleteAppContainerProfile(self.name.as_ptr()) };
            if status < 0 {
                return Err(io::Error::other(format!(
                    "DeleteAppContainerProfile failed: {status:#x}"
                )));
            }
            self.active = false;
        }
        Ok(())
    }
}
impl Drop for Profile {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}
fn sid_text(sid: Handle) -> io::Result<String> {
    let mut text = ptr::null_mut();
    // SAFETY: caller provides a live SID; the API owns the resulting terminated string.
    checked(unsafe { ConvertSidToStringSidW(sid, &mut text) })?;
    let allocation = LocalMemory(text.cast());
    let mut length = 0;
    // SAFETY: a valid converted SID string is terminated and shorter than 256 WCHARs.
    while length < 256 && unsafe { *text.add(length) } != 0 {
        length += 1;
    }
    if length == 256 {
        return Err(io::Error::other("invalid SID string"));
    }
    // SAFETY: counted only the readable string returned by the API.
    let result = String::from_utf16(unsafe { std::slice::from_raw_parts(text, length) })
        .map_err(io::Error::other);
    drop(allocation);
    result
}
fn token(process: Handle) -> io::Result<OwnedHandle> {
    let mut token = ptr::null_mut();
    // SAFETY: process is live; query and duplicate access support a read-only AccessCheck.
    checked(unsafe { OpenProcessToken(process, 8 | 2, &mut token) })?;
    owned(token)
}
fn token_info(token: &OwnedHandle, class: u32) -> io::Result<Vec<usize>> {
    let mut needed = 0;
    // SAFETY: initial call only obtains the required buffer size.
    unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            class,
            ptr::null_mut(),
            0,
            &mut needed,
        );
    }
    if needed == 0 || needed > 65536 {
        return Err(io::Error::last_os_error());
    }
    let mut buffer = vec![0usize; (needed as usize).div_ceil(size_of::<usize>())];
    // SAFETY: aligned buffer is at least the requested size and the token is live.
    checked(unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            class,
            buffer.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    })?;
    Ok(buffer)
}
fn assert_container(process: Handle) -> io::Result<()> {
    let token = token(process)?;
    let value = token_info(&token, TOKEN_IS_APP_CONTAINER)?;
    // SAFETY: this class returns a DWORD; the aligned allocation contains it.
    if unsafe { *value.as_ptr().cast::<u32>() } != 1 {
        return Err(io::Error::other("Lua process is not an AppContainer"));
    }
    let capabilities = token_info(&token, TOKEN_CAPABILITIES)?; // TOKEN_GROUPS begins with a DWORD count.
    // SAFETY: GetTokenInformation returned a valid TOKEN_GROUPS buffer.
    if unsafe { *capabilities.as_ptr().cast::<u32>() } != 0 {
        return Err(io::Error::other(
            "Lua container has unexpected capabilities",
        ));
    }
    // Class 46 is not a supported Win32 DWORD query on Windows 10. Verify the
    // actual LPAC access rule with documented AccessCheck instead: its own SID
    // must grant access, while ALL APPLICATION PACKAGES alone must not.
    let sid = token_info(&token, TOKEN_APP_CONTAINER_SID)?;
    let own_sid = sid_text(sid[0] as Handle)?;
    if !container_access(&token, &own_sid)? || container_access(&token, "S-1-15-2-1")? {
        return Err(io::Error::other(
            "Lua container lacks LPAC access restrictions",
        ));
    }
    Ok(())
}

fn container_access(token: &OwnedHandle, allowed_sid: &str) -> io::Result<bool> {
    let text = wide(OsStr::new(&format!(
        "O:SYG:SYD:(A;;0x1;;;WD)(A;;0x1;;;{allowed_sid})"
    )))?;
    let mut descriptor = ptr::null_mut();
    // SAFETY: fixed descriptor grammar and an OS-produced SID, writable output.
    checked(unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text.as_ptr(),
            1,
            &mut descriptor,
            ptr::null_mut(),
        )
    })?;
    let descriptor = LocalMemory(descriptor);
    let mut duplicate = ptr::null_mut();
    // SAFETY: live token with duplicate access; AccessCheck requires an impersonation token.
    checked(unsafe { DuplicateToken(token.as_raw_handle(), 2, &mut duplicate) })?;
    let duplicate = owned(duplicate)?;
    let mapping = GenericMapping {
        read: 1,
        write: 1,
        execute: 1,
        all: 1,
    };
    let mut privileges = [0u64; 128];
    let mut length = size_of_val(&privileges) as u32;
    let mut granted = 0;
    let mut access = 0;
    // SAFETY: valid self-relative descriptor, live impersonation token and aligned
    // output buffers. Failure is an error, never interpreted as a successful denial.
    checked(unsafe {
        AccessCheck(
            descriptor.0,
            duplicate.as_raw_handle(),
            1,
            &mapping,
            privileges.as_mut_ptr().cast(),
            &mut length,
            &mut granted,
            &mut access,
        )
    })?;
    Ok(access != 0 && granted & 1 != 0)
}

pub(super) fn check_current() -> Result<()> {
    // SAFETY: this pseudo handle refers to the calling process and is never closed.
    assert_container(unsafe { GetCurrentProcess() })
        .map_err(|e| Error::io("require Lua sandbox", e))
}

struct Attributes(Vec<usize>);
impl Attributes {
    fn new() -> io::Result<Self> {
        let mut bytes = 0;
        // SAFETY: the first call obtains the allocation size for six attributes.
        unsafe {
            InitializeProcThreadAttributeList(ptr::null_mut(), 6, 0, &mut bytes);
        }
        if bytes == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut storage = vec![0usize; bytes.div_ceil(size_of::<usize>())];
        // SAFETY: storage is correctly aligned and sized, and remains owned until Drop.
        checked(unsafe {
            InitializeProcThreadAttributeList(storage.as_mut_ptr().cast(), 6, 0, &mut bytes)
        })?;
        Ok(Self(storage))
    }
    fn pointer(&mut self) -> Handle {
        self.0.as_mut_ptr().cast()
    }
    /// The referenced value must remain alive and unmoved until this list is dropped.
    unsafe fn add<T>(&mut self, attribute: usize, value: &T) -> io::Result<()> {
        // SAFETY: required of the caller; list owns its initialized storage.
        checked(unsafe {
            UpdateProcThreadAttribute(
                self.pointer(),
                0,
                attribute,
                (value as *const T).cast_mut().cast(),
                size_of::<T>(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        })
        .map_err(|e| at(&format!("set sandbox attribute {attribute:x}"), e))
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        /* SAFETY: initialized, uniquely owned attribute list. */
        unsafe {
            DeleteProcThreadAttributeList(self.pointer());
        }
    }
}

/// Files are not executable until copied from a locked, hash-verified source.
/// The child gets read/execute access only to this ephemeral directory.
struct Stage {
    directory: PathBuf,
    executable: PathBuf,
    file: Option<File>,
    directory_lock: Option<File>,
    /// Spent by the first cleanup, so Drop after a failed one tries once and moves on.
    wait: Duration,
}
impl Stage {
    fn new(bytes: &[u8], sid: &Sid, nonce: &str) -> io::Result<Self> {
        // SAFETY: current-process pseudo handle does not transfer ownership.
        let user_token = token(unsafe { GetCurrentProcess() })?;
        let user = token_info(&user_token, 1)?; // TOKEN_USER begins with a SID pointer.
        let user_sid = sid_text(user[0] as Handle)?;
        let descriptor_text = wide(OsStr::new(&format!(
            "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;{user_sid})(A;OICI;GRGX;;;{})",
            sid_text(sid.0)?
        )))?;
        let mut descriptor = ptr::null_mut();
        // SAFETY: terminated SDDL, revision 1, writable output; allocation is released by LocalMemory.
        checked(unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                descriptor_text.as_ptr(),
                1,
                &mut descriptor,
                ptr::null_mut(),
            )
        })?;
        let descriptor = LocalMemory(descriptor);
        let security = SecurityAttributes {
            size: size_of::<SecurityAttributes>() as u32,
            descriptor: descriptor.0,
            inherit: 0,
        };
        let directory = std::env::temp_dir().join(format!("rocket-lua-{nonce}"));
        local_path(&directory)?;
        let encoded = wide(directory.as_os_str())?;
        // SAFETY: fresh random path and explicit DACL; existing paths cause failure.
        checked(unsafe { CreateDirectoryW(encoded.as_ptr(), &security) })?;
        let mut stage = Self {
            executable: directory.join("rocket-lua-host.exe"),
            directory,
            file: None,
            directory_lock: None,
            wait: CLEANUP_WAIT,
        };
        stage.directory_lock = Some(
            OpenOptions::new()
                .read(true)
                .share_mode(3)
                .custom_flags(0x0220_0000)
                .open(&stage.directory)?,
        );
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&stage.executable)?;
        output.write_all(bytes)?;
        output.sync_all()?;
        drop(output);
        stage.file = Some(locked_file(&stage.executable)?);
        Ok(stage)
    }
    fn cleanup(&mut self) -> io::Result<()> {
        self.file.take();
        self.directory_lock.take();
        let deadline = Instant::now() + std::mem::take(&mut self.wait);
        remove_when_free(&self.executable, |path| fs::remove_file(path), deadline)?;
        remove_when_free(&self.directory, |path| fs::remove_dir(path), deadline)
    }
}
impl Drop for Stage {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}
/// Runs `remove` until it works, the path is gone or `deadline` passes. A worker killed just
/// after it started leaves its file busy for some milliseconds: kill-on-close and image-section
/// release finish on different kernel paths, and a scanner may still be reading a new program.
fn remove_when_free(
    path: &Path,
    remove: fn(&Path) -> io::Result<()>,
    deadline: Instant,
) -> io::Result<()> {
    let mut pause = Duration::from_millis(2);
    loop {
        match remove(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            // ERROR_ACCESS_DENIED and ERROR_SHARING_VIOLATION, both seen in that moment.
            Err(e) if matches!(e.raw_os_error(), Some(5 | 32)) && Instant::now() < deadline => {
                std::thread::sleep(pause);
                pause = (pause * 2).min(Duration::from_millis(100));
            }
            result => return result,
        }
    }
}
fn local_path(path: &Path) -> io::Result<()> {
    if !path.is_absolute()
        || !matches!(path.components().next(), Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "worker path must be on a local drive",
        ));
    }
    let root = wide(path.ancestors().last().unwrap().as_os_str())?;
    // SAFETY: terminated absolute drive root. A mapped drive must not turn
    // worker verification/staging into network access with the user's credentials.
    if !matches!(unsafe { GetDriveTypeW(root.as_ptr()) }, 2 | 3 | 5 | 6) {
        return Err(io::Error::other("worker path is not on a local drive"));
    }
    for part in path.components() {
        if matches!(part, Component::ParentDir | Component::CurDir)
            || matches!(part, Component::Normal(s) if s.encode_wide().any(|c| c == 0 || c == b':' as u16))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid worker path component",
            ));
        }
    }
    let mut parents: Vec<_> = path.ancestors().collect();
    parents.reverse();
    for ancestor in parents {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_attributes() & 0x400 != 0 => {
                return Err(io::Error::other("worker path contains a reparse point"));
            }
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => (),
        }
    }
    Ok(())
}
fn locked_file(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .custom_flags(0x0020_0000)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.file_attributes() & 0x400 != 0
        || metadata.len() == 0
        || metadata.len() > MAX_WORKER
    {
        return Err(io::Error::other("worker must be a bounded regular file"));
    }
    Ok(file)
}
fn pipe(child_reads: bool) -> io::Result<(File, OwnedHandle)> {
    let security = SecurityAttributes {
        size: size_of::<SecurityAttributes>() as u32,
        descriptor: ptr::null_mut(),
        inherit: 1,
    };
    let (mut read, mut write) = (ptr::null_mut(), ptr::null_mut());
    // SAFETY: writable outputs, correctly sized SECURITY_ATTRIBUTES.
    checked(unsafe { CreatePipe(&mut read, &mut write, &security, 0) })?;
    let (read, write) = (owned(read)?, owned(write)?);
    let (parent, child) = if child_reads {
        (write, read)
    } else {
        (read, write)
    };
    // SAFETY: only our end loses inheritance; the child end is explicitly allowlisted below.
    checked(unsafe { SetHandleInformation(parent.as_raw_handle(), 1, 0) })?;
    Ok((File::from(parent), child))
}
struct Process {
    process: OwnedHandle,
    job: OwnedHandle,
}
impl RunningProcess for Process {
    fn terminate(&self) -> io::Result<()> {
        /* SAFETY: unique live job, with one contained process. */
        checked(unsafe { TerminateJobObject(self.job.as_raw_handle(), 1) })
    }
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        // SAFETY: live process handle; zero timeout never blocks.
        match unsafe { WaitForSingleObject(self.process.as_raw_handle(), 0) } {
            0x102 => Ok(None),
            0 => self.wait().map(Some),
            _ => Err(io::Error::last_os_error()),
        }
    }
    fn wait(&mut self) -> io::Result<ExitStatus> {
        // SAFETY: live process handle; callers terminate the job before failure-path waits.
        if unsafe { WaitForSingleObject(self.process.as_raw_handle(), u32::MAX) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut code = 0;
        // SAFETY: process is signaled and output is writable.
        checked(unsafe { GetExitCodeProcess(self.process.as_raw_handle(), &mut code) })?;
        Ok(ExitStatus::from_raw(code))
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.terminate();
        // SAFETY: best-effort bounded reap on early errors; closing the job also kills the child.
        unsafe {
            WaitForSingleObject(self.process.as_raw_handle(), 5000);
        }
    }
}
fn system_directory(windows: bool) -> io::Result<Vec<u16>> {
    let mut value = vec![0u16; 32768];
    // SAFETY: writable buffer; these APIs do not consult environment variables.
    let size = unsafe {
        if windows {
            GetWindowsDirectoryW(value.as_mut_ptr(), value.len() as u32)
        } else {
            GetSystemDirectoryW(value.as_mut_ptr(), value.len() as u32)
        }
    };
    if size == 0 || size as usize >= value.len() {
        return Err(io::Error::last_os_error());
    }
    value.truncate(size as usize);
    Ok(value)
}
fn spawn(stage: &Stage, sid: &Sid) -> io::Result<(Process, ProcessPipes<File, File, File>)> {
    // SAFETY: unnamed job with default security; the handle is never inherited.
    let job = owned(unsafe { CreateJobObjectW(ptr::null_mut(), ptr::null()) })?;
    let mut limits = JobLimits::default();
    limits.basic.flags = 0x2000 | 8 | 0x100 | 2; // kill on close, one process, memory and CPU caps.
    limits.basic.active_processes = 1;
    limits.basic.process_time = 50_000_000;
    limits.process_memory = 128 * 1024 * 1024;
    // SAFETY: SDK-compatible JOBOBJECT_EXTENDED_LIMIT_INFORMATION, live job.
    checked(unsafe {
        SetInformationJobObject(
            job.as_raw_handle(),
            9,
            (&limits as *const JobLimits).cast(),
            size_of::<JobLimits>() as u32,
        )
    })?;
    let (stdin, child_stdin) = pipe(true)?;
    let (stdout, child_stdout) = pipe(false)?;
    let (stderr, child_stderr) = pipe(false)?;
    let handles = [
        child_stdin.as_raw_handle(),
        child_stdout.as_raw_handle(),
        child_stderr.as_raw_handle(),
    ];
    let jobs = [job.as_raw_handle()];
    let capabilities = Capabilities {
        sid: sid.0,
        capabilities: ptr::null_mut(),
        count: 0,
        reserved: 0,
    };
    let lpac: u32 = 1;
    let no_children: u32 = 1;
    let mitigations: u64 = (1 << 28) | (1 << 32) | (1 << 36) | (1 << 52) | (1 << 56) | (1 << 60);
    let mut attributes = Attributes::new()?;
    // SAFETY: every referenced value and its owned handles/SID outlive attributes
    // and CreateProcess. No value is modified after its address is registered.
    unsafe {
        attributes.add(ATTRIBUTE_HANDLE_LIST, &handles)?;
        attributes.add(ATTRIBUTE_SECURITY_CAPABILITIES, &capabilities)?;
        attributes.add(ATTRIBUTE_JOB_LIST, &jobs)?;
        attributes.add(ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY, &lpac)?;
        attributes.add(ATTRIBUTE_CHILD_PROCESS_POLICY, &no_children)?;
        attributes.add(ATTRIBUTE_MITIGATION_POLICY, &mitigations)?;
    }
    let startup = StartupInfoEx {
        info: StartupInfo {
            size: size_of::<StartupInfoEx>() as u32,
            flags: 0x100,
            stdin: handles[0],
            stdout: handles[1],
            stderr: handles[2],
            ..Default::default()
        },
        attributes: attributes.pointer(),
    };
    let executable = wide(stage.executable.as_os_str())?;
    let mut command = vec![b'"' as u16];
    command.extend_from_slice(&executable[..executable.len() - 1]);
    command.extend([b'"' as u16, 0]);
    let directory = wide(stage.directory.as_os_str())?;
    // Windows needs these base directory variables while constructing the
    // AppContainer environment. Never inherit the user's environment or secrets.
    let stage_path = &directory[..directory.len() - 1];
    let system = system_directory(false)?;
    let windows = system_directory(true)?;
    let mut environment = Vec::new();
    for (name, value) in [
        ("APPDATA", stage_path),
        ("LOCALAPPDATA", stage_path),
        ("PATH", system.as_slice()),
        ("SystemRoot", windows.as_slice()),
        ("TEMP", stage_path),
        ("TMP", stage_path),
        ("USERPROFILE", stage_path),
        ("windir", windows.as_slice()),
    ] {
        environment.extend(name.encode_utf16());
        environment.push(b'=' as u16);
        environment.extend_from_slice(value);
        environment.push(0);
    }
    environment.push(0);
    let mut info = ProcessInfo::default();
    // SAFETY: all structures/buffers match Win32 layouts and remain live. The job
    // is attached atomically at creation, before suspension can strand a child.
    checked(unsafe {
        CreateProcessW(
            executable.as_ptr(),
            command.as_mut_ptr(),
            ptr::null_mut(),
            ptr::null_mut(),
            1,
            0x0808_0404,
            environment.as_mut_ptr().cast(),
            directory.as_ptr(),
            &startup.info,
            &mut info,
        )
    })
    .map_err(|e| at("CreateProcessW", e))?;
    let process = Process {
        process: owned(info.process)?,
        job,
    };
    let thread = owned(info.thread)?;
    assert_container(process.process.as_raw_handle()).map_err(|e| at("verify sandbox token", e))?;
    // SAFETY: containment and token checks passed; this is the suspended primary thread.
    if unsafe { ResumeThread(thread.as_raw_handle()) } == u32::MAX {
        return Err(io::Error::last_os_error());
    }
    Ok((
        process,
        ProcessPipes {
            stdin,
            stdout,
            stderr,
        },
    ))
}

pub(super) fn run(
    worker: &IntegrationWorker,
    input: Vec<u8>,
    limits: &CommandLimits,
    cancel: &CancellationToken,
) -> Result<Output> {
    run_observed(worker, input, limits, cancel, |_| {})
}

fn run_observed(
    worker: &IntegrationWorker,
    input: Vec<u8>,
    limits: &CommandLimits,
    cancel: &CancellationToken,
    on_spawn: impl FnOnce(Handle),
) -> Result<Output> {
    if limits.timeout.is_zero() || limits.stdout_bytes == 0 || limits.stderr_bytes == 0 {
        return Err(failure("Invalid worker limits."));
    }
    local_path(&worker.path).map_err(|e| Error::io("validate Lua worker path", e))?;
    let mut original = locked_file(&worker.path).map_err(|e| Error::io("lock Lua worker", e))?;
    let actual = crate::advisory_tool::digest(&mut original, MAX_WORKER, cancel)?;
    if actual != worker.sha256 {
        return Err(failure(
            "Lua worker hash mismatch. Restore the complete matching Rocket bundle; no worker was started.",
        ));
    }
    original
        .rewind()
        .map_err(|e| Error::io("read verified Lua worker", e))?;
    let mut bytes = Vec::new();
    (&mut original)
        .take(MAX_WORKER + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io("copy verified Lua worker", e))?;
    if bytes.len() as u64 > MAX_WORKER {
        return Err(failure("Lua worker exceeds its limit."));
    }
    cancel.check()?;
    let mut random = [0u8; 16];
    // SAFETY: system RNG writes exactly the valid output slice; no provider handle is needed.
    if unsafe { BCryptGenRandom(ptr::null_mut(), random.as_mut_ptr(), random.len() as u32, 2) } < 0
    {
        return Err(failure("Cannot create a fresh sandbox identity."));
    }
    let nonce: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let mut profile = Profile::new(&nonce).map_err(|e| Error::io("create Lua container", e))?;
    let mut stage = Stage::new(&bytes, &profile.sid, &nonce)
        .map_err(|e| Error::io("prepare Lua sandbox", e))?;
    let staged_hash =
        crate::advisory_tool::digest(stage.file.as_mut().unwrap(), MAX_WORKER, cancel)?;
    if staged_hash != worker.sha256 {
        return Err(failure("Staged Lua worker failed verification."));
    }
    cancel.check()?;
    let (mut process, pipes) =
        spawn(&stage, &profile.sid).map_err(|e| Error::io("start isolated Lua worker", e))?;
    on_spawn(process.process.as_raw_handle());
    let result = runner::collect_worker_process(&mut process, pipes, input, limits, cancel);
    drop(process);
    // How the run ended comes first: a folder left behind must not hide a stop or a timeout.
    let stage_cleaned = stage
        .cleanup()
        .map_err(|e| Error::io("clean Lua sandbox", e));
    let profile_cleaned = profile
        .cleanup()
        .map_err(|e| Error::io("clean Lua container", e));
    let output = result?;
    stage_cleaned?;
    profile_cleaned?;
    Ok(output)
}

#[cfg(test)]
mod tests;
