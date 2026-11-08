//! Apple Silicon: the fixed import-free Lua guest in a signed Pulley helper.
//! A private snapshot avoids executing a mutable installed pathname. This is not
//! authentication against a malicious native process running as the same user
//! (which can also replace an unsigned Rocket host). Packages and guest code
//! cannot access the snapshot or choose its bytes, environment or arguments.
use super::IntegrationWorker;
use crate::{
    CancellationToken, CommandLimits, Error, Result,
    runner::{self, Output, ProcessPipes, RunningProcess},
};
use std::{
    ffi::c_void,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::{
        fd::AsRawFd,
        unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
            net::UnixStream,
            process::CommandExt,
        },
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    ptr,
};

const FILES: [&str; 3] = [
    "Contents/MacOS/rocket-lua-host",
    "Contents/Info.plist",
    "Contents/_CodeSignature/CodeResources",
];
const MAX_WORKER: u64 = 64 * 1024 * 1024;
unsafe extern "C" {
    fn geteuid() -> u32;
    fn setsid() -> i32;
    fn dup2(from: i32, to: i32) -> i32;
    fn fcntl(fd: i32, operation: i32, ...) -> i32;
    fn kill(pid: i32, signal: i32) -> i32;
}
#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecRandomCopyBytes(random: *const c_void, count: usize, bytes: *mut c_void) -> i32;
    fn SecStaticCodeCreateWithPath(url: *const c_void, flags: u32, code: *mut *const c_void)
    -> i32;
    fn SecStaticCodeCheckValidity(
        code: *const c_void,
        flags: u32,
        requirement: *const c_void,
    ) -> i32;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFURLCreateFromFileSystemRepresentation(
        allocator: *const c_void,
        bytes: *const u8,
        len: isize,
        directory: u8,
    ) -> *const c_void;
    fn CFRelease(object: *const c_void);
}

fn failure(message: &str) -> Error {
    Error::InvalidInput(format!("integration_worker_security: {message}"))
}

struct Snapshot {
    app: PathBuf,
    device: u64,
    inode: u64,
}
impl Snapshot {
    fn create(worker: &IntegrationWorker, cancel: &CancellationToken) -> Result<Self> {
        let source = worker
            .path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .filter(|app| app.join(FILES[0]) == worker.path && app.is_absolute())
            .ok_or_else(|| failure("The Mac Lua helper must be inside its signed app bundle."))?;
        let source = crate::local_storage::resolve_system_alias(source)
            .map_err(|e| Error::io("inspect Lua helper bundle", e))?;
        let storage = crate::local_storage::directory()
            .map_err(|e| Error::io("open Lua private storage", e))?
            .join("lua-runs");
        crate::local_storage::create_private(&storage)
            .map_err(|e| Error::io("open Lua run storage", e))?;
        let mut random = [0u8; 16];
        // SAFETY: null selects the system RNG and the output slice is live.
        if unsafe { SecRandomCopyBytes(ptr::null(), random.len(), random.as_mut_ptr().cast()) } != 0
        {
            return Err(failure("Unable to allocate a private Lua run identity."));
        }
        let name: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let app = storage.join(format!("{name}.app"));
        crate::local_storage::create_private_new(&app)
            .map_err(|e| Error::io("create Lua private snapshot", e))?;
        let meta =
            fs::symlink_metadata(&app).map_err(|e| Error::io("inspect Lua private snapshot", e))?;
        let snapshot = Self {
            app,
            device: meta.dev(),
            inode: meta.ino(),
        };
        for (index, relative) in FILES.iter().enumerate() {
            cancel.check()?;
            let path = source.join(relative);
            let limit = if index == 0 { MAX_WORKER } else { 1024 * 1024 };
            for parent in path.parent().unwrap().ancestors() {
                let meta = fs::symlink_metadata(parent)
                    .map_err(|e| Error::io("inspect Lua helper directory", e))?;
                // Root-owned sticky temporary roots cannot have their existing
                // user-owned children replaced by another user.
                let sticky_root = meta.uid() == 0 && meta.mode() & 0o1000 != 0;
                if !meta.is_dir()
                    || ![0, unsafe { geteuid() }].contains(&meta.uid())
                    || meta.mode() & 0o022 != 0 && !sticky_root
                {
                    return Err(failure("Unsafe Lua helper directory or symbolic link."));
                }
            }
            let mut options = OpenOptions::new();
            options.read(true);
            crate::local_files::no_follow(&mut options);
            let mut input = options
                .open(&path)
                .map_err(|e| Error::io("open Lua helper", e))?;
            let info = input
                .metadata()
                .map_err(|e| Error::io("inspect Lua helper", e))?;
            // SAFETY: geteuid has no preconditions.
            if !crate::local_files::opened_without_link(&input, &path)
                .map_err(|e| Error::io("inspect Lua helper link", e))?
                || !info.is_file()
                || info.nlink() != 1
                || info.mode() & 0o022 != 0
                || ![0, unsafe { geteuid() }].contains(&info.uid())
                || info.len() == 0
                || info.len() > limit
            {
                return Err(failure(
                    "Lua helper files must be bounded, private-owner regular files without links.",
                ));
            }
            let destination = snapshot.app.join(relative);
            crate::local_storage::create_private(destination.parent().unwrap())
                .map_err(|e| Error::io("create Lua snapshot folder", e))?;
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&destination)
                .map_err(|e| Error::io("copy Lua helper", e))?;
            let mut total = 0u64;
            let mut buffer = [0; 64 * 1024];
            loop {
                cancel.check()?;
                let count = input
                    .read(&mut buffer)
                    .map_err(|e| Error::io("read Lua helper", e))?;
                if count == 0 {
                    break;
                }
                total += count as u64;
                if total > limit {
                    return Err(failure("Lua helper changed or exceeds its limit."));
                }
                output
                    .write_all(&buffer[..count])
                    .map_err(|e| Error::io("copy Lua helper", e))?;
            }
            drop(output);
            fs::set_permissions(
                &destination,
                fs::Permissions::from_mode(if index == 0 { 0o500 } else { 0o400 }),
            )
            .map_err(|e| Error::io("protect Lua helper snapshot", e))?;
        }
        let mut executable = File::open(snapshot.app.join(FILES[0]))
            .map_err(|e| Error::io("verify Lua snapshot", e))?;
        if crate::advisory_tool::digest(&mut executable, MAX_WORKER, cancel)? != worker.sha256 {
            return Err(failure(
                "Lua worker hash mismatch. Restore the complete matching Rocket bundle; no worker was started.",
            ));
        }
        snapshot.verify_signature()?;
        cancel.check()?;
        Ok(snapshot)
    }

    fn verify_signature(&self) -> Result<()> {
        let bytes = self.app.as_os_str().as_bytes();
        // SAFETY: buffers are live; owned CF objects are released exactly once.
        unsafe {
            let url = CFURLCreateFromFileSystemRepresentation(
                ptr::null(),
                bytes.as_ptr(),
                bytes.len() as isize,
                1,
            );
            if url.is_null() {
                return Err(failure("Cannot inspect the signed Lua helper."));
            }
            let mut code = ptr::null();
            let created = SecStaticCodeCreateWithPath(url, 0, &mut code);
            CFRelease(url);
            if created != 0 || code.is_null() {
                return Err(failure("The Lua helper's app signature is missing."));
            }
            // kSecCSStrictValidate: verify all sealed resources as well as code.
            let checked = SecStaticCodeCheckValidity(code, 1 << 12, ptr::null());
            CFRelease(code);
            if checked != 0 {
                return Err(failure(
                    "The Lua helper signature or sealed resources changed. Restore the matching bundle.",
                ));
            }
        }
        Ok(())
    }
}
impl Drop for Snapshot {
    fn drop(&mut self) {
        // Never recursively remove a path that could have been redirected. Only
        // the three files and fixed directories this invocation created.
        let Ok(meta) = fs::symlink_metadata(&self.app) else {
            return;
        };
        if !meta.is_dir() || meta.dev() != self.device || meta.ino() != self.inode {
            return;
        }
        for relative in FILES {
            let _ = fs::remove_file(self.app.join(relative));
        }
        for relative in ["Contents/MacOS", "Contents/_CodeSignature", "Contents", ""] {
            let _ = fs::remove_dir(self.app.join(relative));
        }
    }
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
    on_spawn: impl FnOnce(u32),
) -> Result<Output> {
    if limits.timeout.is_zero() || limits.stdout_bytes == 0 || limits.stderr_bytes == 0 {
        return Err(failure("Invalid worker limits."));
    }
    let snapshot = Snapshot::create(worker, cancel)?;
    let (read, lifeline) = UnixStream::pair().map_err(|e| Error::io("create Lua lifeline", e))?;
    let reader = read.as_raw_fd();
    let mut command = Command::new(snapshot.app.join(FILES[0]));
    command
        .arg("--parent-pid")
        .arg(std::process::id().to_string())
        .env_clear()
        .current_dir("/")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // SAFETY: only async-signal-safe system calls between fork and exec. The
    // worker closes every other inherited descriptor before reading any input.
    unsafe {
        command.pre_exec(move || {
            if setsid() == -1 || dup2(reader, 3) == -1 || fcntl(3, 2, 0) == -1 {
                // F_SETFD, clear CLOEXEC only for lifeline
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    cancel.check()?;
    let child = command
        .spawn()
        .map_err(|e| Error::io("start signed Lua helper", e))?;
    let mut process = Process {
        child,
        reaped: false,
        _lifeline: lifeline,
    };
    drop(read);
    on_spawn(process.child.id());
    let pipes = ProcessPipes {
        stdin: process
            .child
            .stdin
            .take()
            .ok_or_else(|| failure("Missing Lua input pipe."))?,
        stdout: process
            .child
            .stdout
            .take()
            .ok_or_else(|| failure("Missing Lua output pipe."))?,
        stderr: process
            .child
            .stderr
            .take()
            .ok_or_else(|| failure("Missing Lua error pipe."))?,
    };
    let result = runner::collect_worker_process(&mut process, pipes, input, limits, cancel);
    drop(process); // terminate/reap before the signed snapshot is removed
    drop(snapshot);
    let out = result?;
    let message = match out.status.code() {
        Some(90) => {
            Some("integration_worker_parent: Lua stopped because its parent connection closed.")
        }
        Some(91) => Some(
            "integration_worker_timeout: Lua exceeded its execution deadline. The previous panel is preserved.",
        ),
        Some(92) => Some(
            "integration_worker_memory: Lua exceeded its native memory threshold. The previous panel is preserved.",
        ),
        Some(93) => Some(
            "integration_worker_sandbox: The signed Mac helper could not establish its isolation. No script was run.",
        ),
        Some(2) => Some(
            "integration_worker_vm: Lua exceeded a VM limit or the fixed guest could not run. The previous panel is preserved.",
        ),
        _ => None,
    };
    if let Some(message) = message {
        return Err(Error::InvalidInput(message.into()));
    }
    Ok(out)
}
struct Process {
    child: Child,
    reaped: bool,
    _lifeline: UnixStream,
}
impl RunningProcess for Process {
    fn terminate(&self) -> io::Result<()> {
        if self.reaped {
            return Ok(());
        }
        let pid = self.child.id() as i32;
        // SAFETY: only our unreaped child and its dedicated process group.
        if unsafe { kill(-pid, 9) == 0 || kill(pid, 9) == 0 } {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(3) {
            Ok(())
        } else {
            Err(error)
        }
    }
    fn wait(&mut self) -> io::Result<ExitStatus> {
        let status = self.child.wait()?;
        self.reaped = true;
        Ok(status)
    }
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        let status = self.child.try_wait()?;
        self.reaped |= status.is_some();
        Ok(status)
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.terminate();
        let _ = self.wait();
    }
}

#[cfg(test)]
mod tests;
