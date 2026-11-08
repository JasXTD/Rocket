//! Own child process groups so cancelling a Git read also closes helper-held pipes.
use std::{
    io,
    process::{Child, Command},
};

#[cfg(windows)]
mod threads;
#[cfg(target_os = "macos")]
mod darwin;

/// macOS keeps the leader PID reserved until group cleanup finishes. Other
/// platforms retain their existing Child::try_wait behavior.
pub(crate) fn poll_exit(child: &mut Child) -> io::Result<Option<std::process::ExitStatus>> {
    #[cfg(target_os = "macos")]
    return crate::darwin_wait::poll_exit(child);
    #[cfg(not(target_os = "macos"))]
    child.try_wait()
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::{
        ffi::c_void,
        os::windows::{io::AsRawHandle, process::CommandExt},
        ptr,
        sync::{Condvar, Mutex, Once, PoisonError},
        time::{Duration, Instant},
    };

    type Handle = *mut c_void;
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
        scheduling_class: u32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct ExtendedLimits {
        basic: BasicLimits,
        io_counters: [u64; 6],
        process_memory: usize,
        job_memory: usize,
        peak_process_memory: usize,
        peak_job_memory: usize,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> Handle;
        fn SetInformationJobObject(
            job: Handle,
            class: i32,
            info: *const c_void,
            length: u32,
        ) -> i32;
        fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32;
        fn TerminateJobObject(job: Handle, exit_code: u32) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
        fn GetStdHandle(which: u32) -> Handle;
        fn SetHandleInformation(handle: Handle, mask: u32, flags: u32) -> i32;
    }
    unsafe extern "C" {
        fn atexit(callback: extern "C" fn()) -> i32;
    }

    /// Starts in progress, and whether this process has begun to exit. A child is
    /// created suspended and runs only once it is in its job; if this process exited
    /// in between, the child would stay suspended for ever.
    static STARTS: Mutex<(usize, bool)> = Mutex::new((0, false));
    static STARTED: Condvar = Condvar::new();
    static SETUP: Once = Once::new();

    /// A child being started: from before it is created until it runs in its job.
    pub(crate) struct Starting(());
    impl Drop for Starting {
        fn drop(&mut self) {
            let mut starts = STARTS.lock().unwrap_or_else(PoisonError::into_inner);
            starts.0 -= 1;
            if starts.0 == 0 {
                STARTED.notify_all();
            }
        }
    }

    /// Runs when main returns (not on `std::process::exit` or a crash): waits up to
    /// 10 s for children being started to run in their jobs, and starts no more.
    /// Other threads keep running until this returns.
    extern "C" fn finish_starts() {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut starts = STARTS.lock().unwrap_or_else(PoisonError::into_inner);
        starts.1 = true;
        while starts.0 > 0 {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                break;
            };
            starts = STARTED
                .wait_timeout(starts, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    fn setup() {
        // Children get their standard handles explicitly (std duplicates any it
        // passes on). Left inheritable, this process's own would reach every child,
        // so a stray child could keep a caller's pipe open after this process exits.
        for which in [-10i32, -11, -12] {
            // SAFETY: GetStdHandle has no preconditions; SetHandleInformation only
            // clears the inherit flag of a live standard handle, ignoring failure.
            unsafe {
                let handle = GetStdHandle(which as u32);
                if !handle.is_null() && handle != -1isize as Handle {
                    SetHandleInformation(handle, 1, 0); // HANDLE_FLAG_INHERIT
                }
            }
        }
        // SAFETY: registers a function without captured state; if registration
        // fails, exit only skips the wait.
        unsafe {
            atexit(finish_starts);
        }
    }

    pub(crate) struct ProcessTree(Handle);
    impl ProcessTree {
        /// Make `command` start suspended, without a console window. Pass the token
        /// to `attach` once it is spawned; until then, exit waits for this start.
        pub(crate) fn prepare(command: &mut Command) -> io::Result<Starting> {
            crate::process_privilege::check()?;
            SETUP.call_once(setup);
            let mut starts = STARTS.lock().unwrap_or_else(PoisonError::into_inner);
            if starts.1 {
                return Err(io::Error::other("the process is exiting"));
            }
            starts.0 += 1;
            drop(starts);
            // No console window should appear when called by a desktop UI.
            command.creation_flags(0x0800_0004); // CREATE_NO_WINDOW | CREATE_SUSPENDED
            Ok(Starting(()))
        }
        pub(crate) fn attach(starting: Starting, child: &Child) -> io::Result<Self> {
            Self::attach_job(starting, child, true)
        }
        /// Like [`ProcessTree::attach`], but closing this handle leaves the programs
        /// running; only [`ProcessTree::terminate`] stops them. For user commands
        /// that may start an editor or terminal that should stay open.
        pub(crate) fn attach_lasting(starting: Starting, child: &Child) -> io::Result<Self> {
            Self::attach_job(starting, child, false)
        }
        fn attach_job(starting: Starting, child: &Child, kill_on_close: bool) -> io::Result<Self> {
            // SAFETY: null requests default security and an unnamed job; ownership is unique.
            let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            let tree = Self(handle);
            let mut limits = ExtendedLimits::default();
            if kill_on_close {
                limits.basic.flags = 0x2000; // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            }
            // SAFETY: the repr(C) layout matches JOBOBJECT_EXTENDED_LIMIT_INFORMATION.
            if unsafe {
                SetInformationJobObject(
                    handle,
                    9,
                    (&limits as *const ExtendedLimits).cast(),
                    size_of::<ExtendedLimits>() as u32,
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: both handles are live for this call. Nested jobs require Windows 8+.
            if unsafe { AssignProcessToJobObject(handle, child.as_raw_handle()) } == 0 {
                return Err(io::Error::last_os_error());
            }
            super::threads::resume_primary_thread(child)?;
            // Running in its job: exit no longer needs to wait for this child.
            drop(starting);
            Ok(tree)
        }
        pub(crate) fn terminate(&self) -> io::Result<()> {
            // SAFETY: self owns a live job handle until Drop.
            if unsafe { TerminateJobObject(self.0, 1) } == 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        }
        /// Processes still in this job, including those of nested jobs.
        pub(crate) fn active_processes(&self) -> io::Result<u32> {
            #[repr(C)]
            #[derive(Default)]
            struct Accounting {
                times: [i64; 4],
                page_faults: u32,
                total_processes: u32,
                active_processes: u32,
                terminated_processes: u32,
            }
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn QueryInformationJobObject(
                    job: Handle,
                    class: i32,
                    info: *mut c_void,
                    length: u32,
                    returned: *mut u32,
                ) -> i32;
            }
            let mut info = Accounting::default();
            // SAFETY: the repr(C) layout matches JOBOBJECT_BASIC_ACCOUNTING_INFORMATION
            // and self owns a live job handle.
            if unsafe {
                QueryInformationJobObject(
                    self.0,
                    1,
                    (&mut info as *mut Accounting).cast(),
                    size_of::<Accounting>() as u32,
                    ptr::null_mut(),
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(info.active_processes)
        }
        pub(crate) fn wait_stopped(&self) -> io::Result<()> {
            let deadline = Instant::now() + Duration::from_secs(5);
            while self.active_processes()? != 0 {
                if Instant::now() >= deadline {
                    return Err(io::Error::new(io::ErrorKind::TimedOut, "test process cleanup did not finish"));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(())
        }
    }
    impl Drop for ProcessTree {
        fn drop(&mut self) {
            // SAFETY: this unique job handle is closed exactly once, killing remaining members.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

#[cfg(unix)]
mod platform {
    use super::*;
    use std::os::unix::process::CommandExt;
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    /// The process group is set as the child is created, so starts need no tracking.
    pub(crate) struct Starting(());
    pub(crate) struct ProcessTree(i32);
    impl ProcessTree {
        pub(crate) fn prepare(command: &mut Command) -> io::Result<Starting> {
            command.process_group(0);
            Ok(Starting(()))
        }
        pub(crate) fn attach(_starting: Starting, child: &Child) -> io::Result<Self> {
            Ok(Self(child.id() as i32))
        }
        /// A process group is never killed on drop, so this equals `attach`.
        pub(crate) fn attach_lasting(starting: Starting, child: &Child) -> io::Result<Self> {
            Self::attach(starting, child)
        }
        pub(crate) fn terminate(&self) -> io::Result<()> {
            // SAFETY: the child was started in a new process group; negative PID addresses it.
            if unsafe { kill(-self.0, 9) } == 0 {
                return Ok(());
            }
            let error = io::Error::last_os_error();
            // Darwin killpg skips zombies and reports EPERM for an all-zombie
            // group. Do not suppress real permission failures: inspect every
            // remaining member while the caller still owns the unreaped leader.
            #[cfg(target_os = "macos")]
            if error.raw_os_error() == Some(1) && !super::darwin::has_live_members(self.0)? {
                return Ok(());
            }
            if error.raw_os_error() == Some(3) {
                Ok(())
            } else {
                Err(error)
            } // ESRCH: already gone
        }
        pub(crate) fn wait_stopped(&self) -> io::Result<()> {
            // SIGKILL was delivered to the entire owned group. The parent is
            // reaped by the caller; orphan zombies are reaped by their adopter.
            Ok(())
        }
    }
}

pub(crate) use platform::ProcessTree;

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::{
        io::Read,
        process::Stdio,
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };

    fn fixture(mode: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "process_tree::tests::start_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("ROCKET_START_FIXTURE", mode)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    /// Start a command as Rocket does: suspended, in a new job, then resumed.
    fn start(mut command: Command, kill_on_close: bool) -> (Child, ProcessTree) {
        let starting = ProcessTree::prepare(&mut command).unwrap();
        let child = command.spawn().unwrap();
        let tree = if kill_on_close {
            ProcessTree::attach(starting, &child)
        } else {
            ProcessTree::attach_lasting(starting, &child)
        };
        (child, tree.unwrap())
    }

    // A subprocess entry point, excluded from normal runs; the tests below run it.
    #[test]
    #[ignore = "subprocess fixture invoked by process_tree tests"]
    fn start_fixture() {
        match std::env::var("ROCKET_START_FIXTURE").unwrap().as_str() {
            "identify" => println!("child-pid={}", std::process::id()),
            "wait" => thread::sleep(Duration::from_secs(30)),
            // Return from main while another thread is still starting a child.
            "exit-while-starting" => {
                let (created, inside) = mpsc::channel();
                thread::spawn(move || {
                    let mut command = fixture("wait");
                    let starting = ProcessTree::prepare(&mut command).unwrap();
                    #[expect(
                        clippy::zombie_processes,
                        reason = "the fixture exits while this child starts; its job ends it"
                    )]
                    let child = command.spawn().unwrap();
                    created.send(()).unwrap();
                    thread::sleep(Duration::from_millis(500));
                    let _tree = ProcessTree::attach(starting, &child).unwrap();
                    thread::sleep(Duration::from_secs(30));
                });
                inside.recv().unwrap();
            }
            // Leave a child running in a lasting job.
            "lasting-child" => drop(start(fixture("wait"), false)),
            _ => panic!("unknown start fixture"),
        }
    }

    #[test]
    fn concurrent_starts_resume_the_correct_children_in_their_jobs() {
        let ready = std::sync::Barrier::new(5);
        thread::scope(|scope| {
            let tasks: Vec<_> = (0..5)
                .map(|_| {
                    scope.spawn(|| {
                        let mut command = fixture("identify");
                        command.stdout(Stdio::piped());
                        ready.wait();
                        let (child, tree) = start(command, true);
                        let id = child.id();
                        let output = child.wait_with_output().unwrap();
                        assert!(output.status.success());
                        assert!(
                            String::from_utf8(output.stdout)
                                .unwrap()
                                .contains(&format!("child-pid={id}"))
                        );
                        // Job accounting can settle just after the process handle
                        // is signalled. Require zero members, as in the exit test.
                        let deadline = Instant::now() + Duration::from_secs(5);
                        while tree.active_processes().unwrap() != 0 {
                            assert!(Instant::now() < deadline, "child remained in its job");
                            thread::sleep(Duration::from_millis(5));
                        }
                    })
                })
                .collect();
            for task in tasks {
                task.join().unwrap();
            }
        });
    }

    #[test]
    fn exiting_while_a_child_starts_does_not_leave_it_suspended() {
        let (mut parent, tree) = start(fixture("exit-while-starting"), true);
        assert!(parent.wait().unwrap().success());
        // The parent's job also holds its children. One left suspended would stay
        // in it; one resumed in its own job ended when the parent closed that job.
        let deadline = Instant::now() + Duration::from_secs(10);
        while tree.active_processes().unwrap() > 0 {
            assert!(Instant::now() < deadline, "a child was left suspended");
            thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn children_do_not_inherit_their_parents_standard_handles() {
        let mut command = fixture("lasting-child");
        command.stdout(Stdio::piped());
        let (mut parent, tree) = start(command, true);
        let mut stdout = parent.stdout.take().unwrap();
        let (sender, closed) = mpsc::channel();
        thread::spawn(move || {
            let _ = stdout.read_to_end(&mut Vec::new());
            let _ = sender.send(());
        });
        assert!(parent.wait().unwrap().success());
        // The lasting child still runs; it must not hold its parent's stdout open.
        assert!(
            closed.recv_timeout(Duration::from_secs(10)).is_ok(),
            "a child kept its parent's stdout open"
        );
        drop(tree); // also ends the lasting child
    }
}
