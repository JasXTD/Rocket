//! The MSIX launch bridge is trusted native code, never a Lua runtime. Desktop
//! Bridge injects package capabilities into a directly-created LPAC. A verified
//! copy of this same helper first leaves that package context, then uses the
//! unchanged LPAC launcher. Lua receives only three pipes. The trusted relay also
//! receives a wait-only cancellation event, and owns the native profile. The
//! parent owns the outer kill-on-close job and temporary executable stage.
use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(super) const JOB_MEMORY: usize = 160 * 1024 * 1024;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetPackageFullName(process: Handle, length: *mut u32, name: *mut u16) -> i32;
    fn QueryInformationJobObject(
        job: Handle,
        class: i32,
        info: Handle,
        size: u32,
        returned: *mut u32,
    ) -> i32;
    fn GetStdHandle(which: u32) -> Handle;
    fn GetFileType(file: Handle) -> u32;
}
#[link(name = "userenv")]
unsafe extern "system" {
    fn DeriveAppContainerSidFromAppContainerName(name: *const u16, sid: *mut Handle) -> i32;
}

pub(super) fn validate_nonce(nonce: &str) -> io::Result<()> {
    if nonce.len() != 32 || !nonce.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(io::Error::other("Invalid Lua bridge identity"));
    }
    Ok(())
}

pub(super) fn assert_unpacked(process: Handle) -> io::Result<()> {
    let mut size = 0;
    // SAFETY: query a live process; no output buffer is needed for NO_PACKAGE.
    if unsafe { GetPackageFullName(process, &mut size, ptr::null_mut()) } != 15700 {
        return Err(io::Error::other(
            "Lua bridge still has Windows package identity",
        ));
    }
    let token = token(process)?;
    let app = token_info(&token, TOKEN_IS_APP_CONTAINER)?;
    let capabilities = token_info(&token, TOKEN_CAPABILITIES)?;
    // SAFETY: these bounded/aligned GetTokenInformation buffers start with DWORDs.
    if unsafe { *app.as_ptr().cast::<u32>() != 0 || *capabilities.as_ptr().cast::<u32>() != 0 } {
        return Err(io::Error::other(
            "Lua bridge inherited a container or capabilities",
        ));
    }
    Ok(())
}

pub(super) fn derive_sid(nonce: &str) -> io::Result<Sid> {
    validate_nonce(nonce)?;
    let name = wide(OsStr::new(&format!("Rocket.Lua.{nonce}")))?;
    let mut sid = ptr::null_mut();
    // SAFETY: only derives a deterministic SID; creates/adopts no profile.
    let status = unsafe { DeriveAppContainerSidFromAppContainerName(name.as_ptr(), &mut sid) };
    if status < 0 {
        return Err(io::Error::other("Cannot derive Lua container identity"));
    }
    Ok(Sid(sid))
}

struct CancelWatch {
    done: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl CancelWatch {
    fn new(raw: usize, cancel: CancellationToken) -> io::Result<Self> {
        let handle = raw as Handle;
        let mut flags = 0;
        // Validate the inherited handle before taking ownership. It must be
        // separate from standard I/O; nothing supplied by Lua reaches this path.
        checked(unsafe { GetHandleInformation(handle, &mut flags) })?;
        if flags & 1 == 0
            || [(-10i32) as u32, (-11i32) as u32, (-12i32) as u32]
                .into_iter()
                .any(|which| unsafe { GetStdHandle(which) } == handle)
        {
            return Err(io::Error::other(
                "Lua bridge requires its cancellation event",
            ));
        }
        let event = owned(handle)?;
        checked(unsafe { SetHandleInformation(event.as_raw_handle(), 1, 0) })?;
        let done = Arc::new(AtomicBool::new(false));
        let stopped = done.clone();
        let thread = std::thread::Builder::new()
            .name("lua-stop".into())
            .spawn(move || {
                while !stopped.load(Ordering::Acquire) {
                    // Invalid/broken transport also cancels, never leaves Lua running.
                    if unsafe { WaitForSingleObject(event.as_raw_handle(), 20) } != 0x102 {
                        cancel.cancel();
                        break;
                    }
                }
            })?;
        Ok(Self {
            done,
            thread: Some(thread),
        })
    }
}
impl Drop for CancelWatch {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(super) fn entry(nonce: &str, cancel_event: usize) -> io::Result<()> {
    validate_nonce(nonce)?;
    // SAFETY: pseudo handle is used only for read-only token/identity queries.
    assert_unpacked(unsafe { GetCurrentProcess() })?;
    let mut job = JobLimits::default();
    // SAFETY: a null job queries the caller's job; output is correctly sized.
    checked(unsafe {
        QueryInformationJobObject(
            ptr::null_mut(),
            9,
            (&mut job as *mut JobLimits).cast(),
            size_of::<JobLimits>() as u32,
            ptr::null_mut(),
        )
    })?;
    if job.basic.flags & (0x2000 | 8 | 0x200 | 0x100 | 2) != (0x2000 | 8 | 0x200 | 0x100 | 2)
        || job.basic.active_processes != 2
        || job.job_memory != JOB_MEMORY
        || job.process_memory != 128 * 1024 * 1024
        || job.basic.process_time != 50_000_000
    {
        return Err(io::Error::other(
            "Lua bridge requires its bounded owner job",
        ));
    }
    for which in [(-10i32) as u32, (-11i32) as u32, (-12i32) as u32] {
        // SAFETY: inspect the supplied standard handles without taking ownership.
        if unsafe { GetFileType(GetStdHandle(which)) } != 3 {
            return Err(io::Error::other(
                "Lua bridge requires anonymous-pipe transport",
            ));
        }
    }
    let executable = std::env::current_exe()?;
    local_path(&executable)?;
    let directory = executable
        .parent()
        .ok_or_else(|| io::Error::other("Missing bridge stage"))?;
    if executable.file_name() != Some(OsStr::new("rocket-lua-host.exe"))
        || directory.file_name() != Some(OsStr::new(&format!("rocket-lua-{nonce}")))
    {
        return Err(io::Error::other(
            "Lua bridge must run from its verified stage",
        ));
    }
    // The parent holds the same file against writes/replacement until both
    // processes exit. This second lock lasts through the child lifetime too.
    let _image = locked_file(&executable)?;
    let cancel = CancellationToken::new();
    let _stop = CancelWatch::new(cancel_event, cancel.clone())?;
    cancel
        .check()
        .map_err(|e| io::Error::other(e.to_string()))?;
    // Create in the unpackaged context. Never adopt an existing profile; only
    // this fresh profile is removed after the contained child is reaped.
    let mut profile = Profile::new(nonce)?;
    let (mut process, pipes) = spawn_paths(&executable, directory, Launch::Worker(&profile.sid))?;
    // No untrusted input is read until the child has passed the original LPAC,
    // zero-capability and access checks. Never deserialize or evaluate Lua here.
    let mut input = Vec::new();
    std::io::stdin()
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut input)?;
    if input.len() > 8 * 1024 * 1024 {
        return Err(io::Error::other("Lua input exceeds its limit"));
    }
    let limits = CommandLimits {
        timeout: Duration::from_secs(5),
        stdout_bytes: 1024 * 1024,
        stderr_bytes: 64 * 1024,
    };
    let result = runner::collect_worker_process(&mut process, pipes, input, &limits, &cancel);
    drop(process);
    let cleaned = profile.cleanup();
    let result = result.map_err(|e| io::Error::other(e.to_string()))?;
    cleaned?;
    std::io::stdout().write_all(&result.stdout)?;
    std::io::stderr().write_all(&result.stderr)?;
    if !result.status.success() {
        return Err(io::Error::other("Isolated Lua worker failed"));
    }
    Ok(())
}
