//! Trusted native watchdog shared by qualification tests and the bound Mac helper.
//! The VM cannot call these functions. A compromised native VM could defeat this
//! thread; App Sandbox and kernel limits are separate defense in depth.
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

pub const NATIVE_THRESHOLD: u64 = 192 * 1024 * 1024;
pub const PARENT_EXIT: i32 = 90;
pub const DEADLINE: i32 = 91;
pub const PRESSURE: i32 = 92;
pub const SETUP: i32 = 93;

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn rocket_close_inherited() -> i32;
    fn rocket_attest() -> i32;
    fn rocket_limits() -> i32;
    fn rocket_memory(bytes: *mut u64) -> i32;
}

fn terminate(code: i32) -> ! {
    // No locks, formatting, allocator or user-controlled diagnostics on the
    // emergency path. Exit status carries the reason even if stdout is blocked.
    unsafe { libc::_exit(code) }
}

fn parent_alive(parent: libc::pid_t) -> bool {
    if unsafe { libc::getppid() } != parent {
        return false;
    }
    let mut fd = libc::pollfd {
        fd: 3,
        events: libc::POLLIN,
        revents: 0,
    };
    // No data belongs on this pipe: any byte, EOF, HUP or invalid FD fails closed.
    let rc = unsafe { libc::poll(&mut fd, 1, 0) };
    rc == 0 || rc < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR)
}

pub fn sample_memory() -> Result<u64, &'static str> {
    #[cfg(target_os = "macos")]
    {
        let mut bytes = 0;
        if unsafe { rocket_memory(&mut bytes) } != 0 || bytes == 0 {
            return Err("memory_sample");
        }
        Ok(bytes)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(0)
    } // WSL exercises lifecycle only; no claim of Mac native containment.
}

pub struct Guard {
    peak: Arc<AtomicU64>,
}
impl Guard {
    pub fn peak_bytes(&self) -> u64 {
        self.peak.load(Ordering::Relaxed)
    }
}

pub fn start(parent: libc::pid_t) -> Result<Guard, &'static str> {
    if parent <= 1 || !parent_alive(parent) {
        terminate(PARENT_EXIT);
    }
    for fd in 0..=3 {
        let mut info = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe { libc::fstat(fd, info.as_mut_ptr()) } != 0 {
            return Err("pipe_missing");
        }
        let kind = unsafe { info.assume_init() }.st_mode & libc::S_IFMT;
        if kind != libc::S_IFIFO && kind != libc::S_IFSOCK {
            return Err("pipe_type");
        }
    }
    #[cfg(target_os = "macos")]
    unsafe {
        if rocket_close_inherited() != 0 {
            return Err("descriptor_cleanup");
        }
        if rocket_attest() != 1 {
            return Err("sandbox_identity");
        }
        if rocket_limits() != 0 {
            return Err("kernel_limits");
        }
    }
    if !parent_alive(parent) {
        terminate(PARENT_EXIT);
    }
    let initial = sample_memory()?;
    if initial > NATIVE_THRESHOLD {
        terminate(PRESSURE);
    }
    let peak = Arc::new(AtomicU64::new(initial));
    let observed = peak.clone();
    let deadline = Instant::now() + Duration::from_secs(5);
    std::thread::Builder::new()
        .name("lua-containment".into())
        .stack_size(128 * 1024)
        .spawn(move || {
            loop {
                if !parent_alive(parent) {
                    terminate(PARENT_EXIT);
                }
                if Instant::now() >= deadline {
                    terminate(DEADLINE);
                }
                let bytes = sample_memory().unwrap_or_else(|_| terminate(SETUP));
                observed.fetch_max(bytes, Ordering::Relaxed);
                if bytes > NATIVE_THRESHOLD {
                    terminate(PRESSURE);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        })
        .map_err(|_| "watchdog_start")?;
    Ok(Guard { peak })
}

pub fn parent_argument() -> Result<libc::pid_t, &'static str> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 || args[0] != "--parent-pid" {
        return Err("parent_argument");
    }
    args[1].parse().map_err(|_| "parent_argument")
}
