//! Linux x86_64: runs the hash-checked Lua worker from a sealed copy in memory, with no
//! environment and no descriptors but its three pipes. The worker then locks itself down
//! (`lockdown.rs`) before it reads any script.
use super::IntegrationWorker;
use crate::{
    CancellationToken, CommandLimits, Error, Result,
    runner::{self, Output, ProcessPipes, RunningProcess},
};
use std::{
    ffi::{c_char, c_int, c_uint, c_ulong},
    fs::File,
    io::{self, Read, Seek},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::{fs::MetadataExt, process::CommandExt},
    },
    path::Path,
    process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus, Stdio},
};

mod lockdown;

const MAX_WORKER: u64 = 64 * 1024 * 1024;
const MFD_CLOEXEC: c_uint = 1;
const MFD_ALLOW_SEALING: c_uint = 2;
const MFD_EXEC: c_uint = 0x10;
const F_ADD_SEALS: c_int = 1033;
// F_SEAL_SEAL, F_SEAL_SHRINK, F_SEAL_GROW and F_SEAL_WRITE.
const SEALS: c_int = 1 | 2 | 4 | 8;
const PR_SET_PDEATHSIG: c_int = 1;
const SIGKILL: c_int = 9;
const EINVAL: i32 = 22;
const ESRCH: i32 = 3;

unsafe extern "C" {
    fn memfd_create(name: *const c_char, flags: c_uint) -> c_int;
    fn fcntl(fd: c_int, command: c_int, ...) -> c_int;
    fn setsid() -> c_int;
    fn getppid() -> c_int;
    fn geteuid() -> u32;
    fn kill(pid: c_int, signal: c_int) -> c_int;
}

fn failure(message: &str) -> Error {
    Error::InvalidInput(format!("integration_worker_security: {message}"))
}

pub(super) fn enter_current() -> Result<()> {
    lockdown::apply().map_err(|e| Error::io("lock down Lua worker", e))
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
    // The worker refuses to run without it too; this says why before anything starts.
    if lockdown::landlock_abi().is_none() {
        return Err(Error::Unsupported(
            "integration_worker_sandbox: Lua extensions need Landlock, which Linux 5.13 and later provide when it is switched on. No unrestricted fallback is provided.".into(),
        ));
    }
    let mut original =
        protected_worker(&worker.path).map_err(|e| Error::io("open Lua worker", e))?;
    let mut sealed = sealed_copy(&mut original).map_err(|e| Error::io("copy Lua worker", e))?;
    drop(original);
    // The hash covers exactly what runs: the sealed copy can't change any more.
    let actual = crate::advisory_tool::digest(&mut sealed, MAX_WORKER, cancel)?;
    if actual != worker.sha256 {
        return Err(failure(
            "Lua worker hash mismatch. Restore the complete matching Rocket bundle; no worker was started.",
        ));
    }
    cancel.check()?;
    let (mut process, pipes) =
        spawn(&sealed).map_err(|e| Error::io("start isolated Lua worker", e))?;
    // The child has opened its own copy for exec by now.
    drop(sealed);
    on_spawn(process.child.id());
    let result = runner::collect_worker_process(&mut process, pipes, input, limits, cancel);
    drop(process);
    result
}

/// The worker file itself, not a link, that only its owner (you, or root for a system
/// install) can change, as Rocket checks the OSV-Scanner it runs.
fn protected_worker(path: &Path) -> io::Result<File> {
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "worker path must be absolute",
        ));
    }
    let file = File::open(path)?;
    let meta = file.metadata()?;
    let direct = crate::local_files::opened_without_link(&file, path)?;
    // SAFETY: geteuid has no preconditions and cannot fail.
    let user = unsafe { geteuid() };
    if !direct
        || !meta.is_file()
        || ![0, user].contains(&meta.uid())
        || meta.mode() & 0o022 != 0
        || meta.len() == 0
        || meta.len() > MAX_WORKER
    {
        return Err(io::Error::other(
            "the worker must be a bounded regular file, not a link, that only its owner can change",
        ));
    }
    Ok(file)
}

/// Copies the worker into a memory file and seals it, so nothing, Rocket included, can
/// change it between the hash check and exec. Windows stages a locked private copy instead.
fn sealed_copy(original: &mut File) -> io::Result<File> {
    let mut copy = File::from(memory_file()?);
    let copied = io::copy(&mut original.take(MAX_WORKER + 1), &mut copy)?;
    if copied == 0 || copied > MAX_WORKER {
        return Err(io::Error::other("Lua worker exceeds its limit"));
    }
    // SAFETY: a live memory file this function owns; the seals take integer flags.
    if unsafe { fcntl(copy.as_raw_fd(), F_ADD_SEALS, SEALS) } == -1 {
        return Err(io::Error::last_os_error());
    }
    copy.rewind()?;
    Ok(copy)
}

fn memory_file() -> io::Result<OwnedFd> {
    let name = c"rocket-lua-host";
    let flags = MFD_CLOEXEC | MFD_ALLOW_SEALING;
    // SAFETY: a terminated name and flags; the result is a new descriptor or -1.
    let mut fd = unsafe { memfd_create(name.as_ptr(), flags | MFD_EXEC) };
    // Kernels before 6.3 don't know MFD_EXEC; their memory files are executable anyway.
    if fd == -1 && io::Error::last_os_error().raw_os_error() == Some(EINVAL) {
        // SAFETY: as above.
        fd = unsafe { memfd_create(name.as_ptr(), flags) };
    }
    if fd == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a new descriptor that nothing else owns.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn spawn(
    program: &File,
) -> io::Result<(Process, ProcessPipes<ChildStdin, ChildStdout, ChildStderr>)> {
    let parent = std::process::id() as c_int;
    let mut command = Command::new(format!("/proc/self/fd/{}", program.as_raw_fd()));
    command
        .arg0("rocket-lua-host")
        .env_clear()
        .current_dir("/")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // SAFETY: the hook runs in the new child between fork and exec, where only
    // async-signal-safe calls are allowed. It makes plain system calls and allocates nothing.
    unsafe {
        command.pre_exec(move || {
            // Its own session and process group, without a terminal; terminate() ends
            // the group.
            if setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            lockdown::control(lockdown::PR_SET_NO_NEW_PRIVS, 1)?;
            // Clear ambient, permitted, effective and inheritable capabilities in
            // this child only. A capability change can clear PDEATHSIG, so do it first.
            lockdown::clear_capabilities()?;
            // Killed when the thread that started it ends, as closing the job kills the
            // Windows worker. That thread waits for the worker in run_observed.
            lockdown::control(PR_SET_PDEATHSIG, SIGKILL as c_ulong)?;
            if getppid() != parent {
                return Err(io::Error::from_raw_os_error(ESRCH));
            }
            lockdown::limit(lockdown::RLIMIT_CORE, 0, 0)?;
            lockdown::limit(
                lockdown::RLIMIT_CPU,
                lockdown::CPU_SECONDS,
                lockdown::CPU_SECONDS + 1,
            )?;
            lockdown::close_on_exec_above_stderr()
        });
    }
    let mut child = command.spawn()?;
    let pipes = (child.stdin.take(), child.stdout.take(), child.stderr.take());
    let process = Process {
        child,
        reaped: false,
    };
    let (Some(stdin), Some(stdout), Some(stderr)) = pipes else {
        return Err(io::Error::other("worker pipes are missing"));
    };
    Ok((
        process,
        ProcessPipes {
            stdin,
            stdout,
            stderr,
        },
    ))
}

struct Process {
    child: Child,
    reaped: bool,
}
impl RunningProcess for Process {
    fn terminate(&self) -> io::Result<()> {
        // A reaped worker's ID may belong to another process by now.
        if self.reaped {
            return Ok(());
        }
        let pid = self.child.id() as c_int;
        // The worker leads its own process group; the direct kill covers the moment
        // before setsid. An unreaped worker's ID can't have been reused.
        // SAFETY: signals only our own unreaped child and its group.
        let results = unsafe { [kill(-pid, SIGKILL), kill(pid, SIGKILL)] };
        if results.contains(&0) {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ESRCH) {
            return Ok(());
        }
        Err(error)
    }
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        let status = self.child.try_wait()?;
        self.reaped |= status.is_some();
        Ok(status)
    }
    fn wait(&mut self) -> io::Result<ExitStatus> {
        let status = self.child.wait()?;
        self.reaped = true;
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
