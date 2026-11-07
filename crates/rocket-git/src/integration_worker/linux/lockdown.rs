//! The Lua worker's lock-down on Linux x86_64, which the worker applies to itself before
//! it reads any script. On Windows the launcher starts the worker inside its container.
//! Here the dynamic loader still has to open the worker's libraries after exec, so the
//! strict rules can only begin in the worker's own code; nothing untrusted has been read
//! by then.
//!
//! Std only, with its own system call declarations, so the sandbox tests can compile this
//! same file into their probe worker.
use std::{
    ffi::{c_int, c_long, c_uint, c_ulong, c_void},
    io,
};

unsafe extern "C" {
    fn syscall(number: c_long, ...) -> c_long;
    fn prctl(option: c_int, ...) -> c_int;
    fn getrlimit(resource: c_int, limit: *mut Limit) -> c_int;
    fn setrlimit(resource: c_int, limit: *const Limit) -> c_int;
    fn getpid() -> c_int;
    fn close(fd: c_int) -> c_int;
}

/// The worker's CPU time, as the Windows job allows five seconds.
pub const CPU_SECONDS: u64 = 5;
/// Its private writable memory, as the Windows job caps process memory at 128 MiB.
pub const MEMORY_BYTES: u64 = 128 * 1024 * 1024;

pub const RLIMIT_CPU: c_int = 0;
const RLIMIT_FSIZE: c_int = 1;
const RLIMIT_DATA: c_int = 2;
pub const RLIMIT_CORE: c_int = 4;
const RLIMIT_NOFILE: c_int = 7;

#[repr(C)]
struct Limit {
    current: u64,
    maximum: u64,
}

// x86_64 system call numbers (arch/x86/entry/syscalls/syscall_64.tbl).
const SYS_READ: u32 = 0;
const SYS_WRITE: u32 = 1;
const SYS_CLOSE: u32 = 3;
const SYS_MMAP: u32 = 9;
const SYS_MPROTECT: u32 = 10;
const SYS_MUNMAP: u32 = 11;
const SYS_BRK: u32 = 12;
const SYS_RT_SIGACTION: u32 = 13;
const SYS_RT_SIGPROCMASK: u32 = 14;
const SYS_RT_SIGRETURN: u32 = 15;
const SYS_READV: u32 = 19;
const SYS_WRITEV: u32 = 20;
const SYS_SCHED_YIELD: u32 = 24;
const SYS_MREMAP: u32 = 25;
const SYS_MADVISE: u32 = 28;
const SYS_GETPID: u32 = 39;
const SYS_SOCKET: u32 = 41;
const SYS_EXIT: u32 = 60;
const SYS_GETTIMEOFDAY: u32 = 96;
const SYS_SIGALTSTACK: u32 = 131;
const SYS_GETTID: u32 = 186;
const SYS_TIME: u32 = 201;
const SYS_FUTEX: u32 = 202;
const SYS_RESTART_SYSCALL: u32 = 219;
const SYS_CLOCK_GETTIME: u32 = 228;
const SYS_EXIT_GROUP: u32 = 231;
const SYS_TGKILL: u32 = 234;
const SYS_OPENAT: u32 = 257;
const SYS_SECCOMP: u32 = 317;
const SYS_GETRANDOM: u32 = 318;
const SYS_CLOSE_RANGE: u32 = 436;
const SYS_LANDLOCK_CREATE_RULESET: u32 = 444;
const SYS_LANDLOCK_RESTRICT_SELF: u32 = 446;

/// Calls the worker may make once locked down, whatever their arguments. Each one acts
/// only on the worker itself: its memory, signal handling, clocks and exit. `read`,
/// `write`, `mmap`, `mprotect` and `tgkill` are allowed with checked arguments below.
pub const ALLOWED: &[u32] = &[
    SYS_CLOSE,
    SYS_MUNMAP,
    SYS_BRK,
    SYS_RT_SIGACTION,
    SYS_RT_SIGPROCMASK,
    SYS_RT_SIGRETURN,
    SYS_SCHED_YIELD,
    SYS_MREMAP,
    SYS_MADVISE,
    SYS_GETPID,
    SYS_EXIT,
    SYS_GETTIMEOFDAY,
    SYS_SIGALTSTACK,
    SYS_GETTID,
    SYS_TIME,
    SYS_FUTEX,
    SYS_RESTART_SYSCALL,
    SYS_CLOCK_GETTIME,
    SYS_EXIT_GROUP,
    SYS_GETRANDOM,
];

const AT_FDCWD: c_long = -100;
const O_DIRECTORY: c_long = 0o200000;
const O_CLOEXEC: c_long = 0o2000000;
const AF_UNIX: c_long = 1;
const SOCK_STREAM: c_long = 1;
const SOCK_CLOEXEC: c_long = 0o2000000;
const EPERM: i32 = 1;
const EACCES: i32 = 13;
const PR_SET_DUMPABLE: c_int = 4;
const PR_SET_NAME: c_int = 15;
pub const PR_SET_NO_NEW_PRIVS: c_int = 38;
pub const PROT_EXEC: u32 = 4;
const CLOSE_RANGE_CLOEXEC: c_long = 1 << 2;
const LANDLOCK_CREATE_RULESET_VERSION: c_long = 1;
const SECCOMP_SET_MODE_FILTER: c_long = 1;
const SECCOMP_FILTER_FLAG_TSYNC: c_long = 1;

/// Locks this process down for good. Landlock denies all file access, and on kernels
/// that support it network access and signals out of the sandbox. Seccomp leaves only
/// the calls in `filter`. Resource limits cap CPU time and memory and stop core dumps,
/// file writes and new descriptors.
///
/// Only a single-threaded process that hasn't read untrusted input may call this: the
/// Lua worker, first thing.
pub fn apply() -> io::Result<()> {
    // Refuse before changing anything: nothing below can be undone, and a process with
    // other threads is not the worker.
    if !single_threaded()? {
        return Err(io::Error::other(
            "only the single-threaded Lua worker may lock itself down",
        ));
    }
    // SAFETY: closes only this process's own descriptors above standard error; nothing
    // in the worker holds one.
    check(unsafe {
        syscall(
            SYS_CLOSE_RANGE as c_long,
            3 as c_long,
            c_uint::MAX as c_long,
            0 as c_long,
        )
    })?;
    limit(RLIMIT_CORE, 0, 0)?;
    limit(RLIMIT_FSIZE, 0, 0)?;
    limit(RLIMIT_CPU, CPU_SECONDS, CPU_SECONDS + 1)?;
    limit(RLIMIT_DATA, MEMORY_BYTES, MEMORY_BYTES)?;
    // Executing through /proc/self/fd named the process after a number.
    // SAFETY: a terminated name of 16 bytes, the kernel's limit, read during the call.
    unsafe { prctl(PR_SET_NAME, c"rocket-lua-host".as_ptr()) };
    control(PR_SET_NO_NEW_PRIVS, 1)?;
    // Also keeps other programs of the same user from attaching to the worker.
    control(PR_SET_DUMPABLE, 0)?;
    landlock()?;
    expect_refused(open_root(), EACCES, "Landlock")?;
    limit(RLIMIT_NOFILE, 3, 3)?;
    seccomp()?;
    expect_refused(open_root(), EPERM, "seccomp")?;
    // SAFETY: plain integer arguments; the result is a descriptor or -1.
    let socket = unsafe {
        syscall(
            SYS_SOCKET as c_long,
            AF_UNIX,
            SOCK_STREAM | SOCK_CLOEXEC,
            0 as c_long,
        )
    };
    expect_refused(socket, EPERM, "seccomp")
}

/// Landlock's ABI version on this kernel, or None where it is missing or switched off.
pub fn landlock_abi() -> Option<u32> {
    // SAFETY: the version query reads no memory: null attributes of size 0.
    let version = unsafe {
        syscall(
            SYS_LANDLOCK_CREATE_RULESET as c_long,
            std::ptr::null::<c_void>(),
            0usize,
            LANDLOCK_CREATE_RULESET_VERSION,
        )
    };
    u32::try_from(version).ok().filter(|&abi| abi >= 1)
}

/// `struct landlock_ruleset_attr`.
#[repr(C)]
#[derive(Debug, PartialEq, Eq)]
pub struct Ruleset {
    pub handled_access_fs: u64,
    pub handled_access_net: u64,
    pub scoped: u64,
}

/// Everything this ABI's Landlock can deny, and the size of the attributes that name it:
/// a kernel refuses rights and fields it doesn't know. Rights that later ABIs add stay
/// allowed until they are listed here; seccomp refuses their calls anyway.
pub fn ruleset(abi: u32) -> (Ruleset, usize) {
    let files = match abi {
        // EXECUTE to MAKE_SYM
        1 => (1 << 13) - 1,
        // REFER
        2 => (1 << 14) - 1,
        // TRUNCATE
        3 | 4 => (1 << 15) - 1,
        // IOCTL_DEV
        _ => (1 << 16) - 1,
    };
    // BIND_TCP and CONNECT_TCP
    let network = if abi >= 4 { 0b11 } else { 0 };
    // ABSTRACT_UNIX_SOCKET and SIGNAL
    let scoped = if abi >= 6 { 0b11 } else { 0 };
    let size = match abi {
        1..=3 => 8,
        4 | 5 => 16,
        _ => 24,
    };
    (
        Ruleset {
            handled_access_fs: files,
            handled_access_net: network,
            scoped,
        },
        size,
    )
}

fn landlock() -> io::Result<()> {
    let abi = landlock_abi()
        .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "Landlock is unavailable"))?;
    let (attributes, size) = ruleset(abi);
    // SAFETY: `attributes` outlives the call, and `size` covers only fields this ABI knows.
    let ruleset = check(unsafe {
        syscall(
            SYS_LANDLOCK_CREATE_RULESET as c_long,
            &attributes as *const Ruleset,
            size,
            0 as c_long,
        )
    })? as c_int;
    // No rules are added, so everything the ruleset handles is denied everywhere.
    // SAFETY: a ruleset descriptor this function owns; flags 0.
    let restricted = check(unsafe {
        syscall(
            SYS_LANDLOCK_RESTRICT_SELF as c_long,
            ruleset as c_long,
            0 as c_long,
        )
    });
    // SAFETY: the ruleset descriptor is closed once, here.
    unsafe { close(ruleset) };
    restricted.map(drop)
}

/// One classic BPF instruction, `struct sock_filter`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Instruction {
    pub code: u16,
    pub jt: u8,
    pub jf: u8,
    pub k: u32,
}

/// `struct sock_fprog`.
#[repr(C)]
struct Program {
    length: u16,
    filter: *const Instruction,
}

const LOAD: u16 = 0x20; // BPF_LD | BPF_W | BPF_ABS
const EQUAL: u16 = 0x15; // BPF_JMP | BPF_JEQ | BPF_K
const ANY_BITS: u16 = 0x45; // BPF_JMP | BPF_JSET | BPF_K
const RETURN: u16 = 0x06; // BPF_RET | BPF_K
pub const AUDIT_ARCH_X86_64: u32 = 0xC000_003E;
pub const X32_SYSCALL_BIT: u32 = 0x4000_0000;
pub const ALLOW: u32 = 0x7FFF_0000; // SECCOMP_RET_ALLOW
pub const KILL: u32 = 0x8000_0000; // SECCOMP_RET_KILL_PROCESS
pub const DENY: u32 = 0x0005_0000 | EPERM as u32; // SECCOMP_RET_ERRNO with EPERM
// Offsets in `struct seccomp_data`; an argument's low 32 bits come first on x86_64,
// and the kernel truncates the int arguments checked here to them.
const NUMBER: u32 = 0;
const ARCH: u32 = 4;
const fn argument(index: u32) -> u32 {
    16 + 8 * index
}
const fn op(code: u16, jt: u8, jf: u8, k: u32) -> Instruction {
    Instruction { code, jt, jf, k }
}

/// The seccomp filter for a worker whose process ID is `pid`. Another architecture or
/// x32 call kills the worker; any call not allowed here fails with EPERM. Jumps are
/// relative to the next instruction, and every check is a short block of its own.
pub fn filter(pid: u32) -> Vec<Instruction> {
    let mut program = vec![
        op(LOAD, 0, 0, ARCH),
        op(EQUAL, 1, 0, AUDIT_ARCH_X86_64),
        op(RETURN, 0, 0, KILL),
        op(LOAD, 0, 0, NUMBER),
        op(ANY_BITS, 0, 1, X32_SYSCALL_BIT),
        op(RETURN, 0, 0, KILL),
    ];
    for &call in ALLOWED {
        program.extend([op(EQUAL, 0, 1, call), op(RETURN, 0, 0, ALLOW)]);
    }
    // Reads from standard input only.
    for call in [SYS_READ, SYS_READV] {
        program.extend([
            op(EQUAL, 0, 4, call),
            op(LOAD, 0, 0, argument(0)),
            op(EQUAL, 0, 1, 0),
            op(RETURN, 0, 0, ALLOW),
            op(RETURN, 0, 0, DENY),
        ]);
    }
    // Writes to standard output and error only.
    for call in [SYS_WRITE, SYS_WRITEV] {
        program.extend([
            op(EQUAL, 0, 5, call),
            op(LOAD, 0, 0, argument(0)),
            op(EQUAL, 2, 0, 1),
            op(EQUAL, 1, 0, 2),
            op(RETURN, 0, 0, DENY),
            op(RETURN, 0, 0, ALLOW),
        ]);
    }
    // Never executable memory.
    for call in [SYS_MMAP, SYS_MPROTECT] {
        program.extend([
            op(EQUAL, 0, 4, call),
            op(LOAD, 0, 0, argument(2)),
            op(ANY_BITS, 0, 1, PROT_EXEC),
            op(RETURN, 0, 0, DENY),
            op(RETURN, 0, 0, ALLOW),
        ]);
    }
    // Signals to itself only, as abort() sends them.
    program.extend([
        op(EQUAL, 0, 4, SYS_TGKILL),
        op(LOAD, 0, 0, argument(0)),
        op(EQUAL, 0, 1, pid),
        op(RETURN, 0, 0, ALLOW),
        op(RETURN, 0, 0, DENY),
    ]);
    program.push(op(RETURN, 0, 0, DENY));
    program
}

fn seccomp() -> io::Result<()> {
    // SAFETY: getpid has no preconditions. The worker can't start processes once locked
    // down, so its ID stays the same.
    let pid = unsafe { getpid() } as u32;
    let instructions = filter(pid);
    let program = Program {
        length: u16::try_from(instructions.len()).map_err(io::Error::other)?,
        filter: instructions.as_ptr(),
    };
    // SAFETY: `program` points at `instructions`, alive for the call; the kernel copies
    // them. TSYNC applies the filter to every thread, or fails.
    match unsafe {
        syscall(
            SYS_SECCOMP as c_long,
            SECCOMP_SET_MODE_FILTER,
            SECCOMP_FILTER_FLAG_TSYNC,
            &program as *const Program,
        )
    } {
        0 => Ok(()),
        -1 => Err(io::Error::last_os_error()),
        _ => Err(io::Error::other("another thread could not be locked down")),
    }
}

/// Lowers a resource limit; an unprivileged process can't raise a hard limit. Allocation
/// free, as the launcher also calls it between fork and exec.
pub fn limit(resource: c_int, soft: u64, hard: u64) -> io::Result<()> {
    let mut current = Limit {
        current: 0,
        maximum: 0,
    };
    // SAFETY: a writable struct in the kernel's layout.
    if unsafe { getrlimit(resource, &mut current) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let maximum = hard.min(current.maximum);
    let wanted = Limit {
        current: soft.min(maximum),
        maximum,
    };
    // SAFETY: a valid struct in the kernel's layout, read during the call.
    if unsafe { setrlimit(resource, &wanted) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Marks every descriptor but standard input, output and error close-on-exec, so none
/// reaches the worker. Allocation free, for the launcher between fork and exec.
pub fn close_on_exec_above_stderr() -> io::Result<()> {
    // SAFETY: changes only the descriptor flags of this process's own descriptors.
    check(unsafe {
        syscall(
            SYS_CLOSE_RANGE as c_long,
            3 as c_long,
            c_uint::MAX as c_long,
            CLOSE_RANGE_CLOEXEC,
        )
    })
    .map(drop)
}

/// Sets a prctl option that takes one integer.
pub fn control(option: c_int, value: c_ulong) -> io::Result<()> {
    let zero: c_ulong = 0;
    // SAFETY: integer arguments only.
    if unsafe { prctl(option, value, zero, zero, zero) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn single_threaded() -> io::Result<bool> {
    let status = std::fs::read_to_string("/proc/self/status")?;
    Ok(status.lines().any(|line| {
        line.strip_prefix("Threads:")
            .is_some_and(|count| count.trim() == "1")
    }))
}

fn open_root() -> c_long {
    // SAFETY: a terminated path and flags; the result is a descriptor or -1.
    unsafe {
        syscall(
            SYS_OPENAT as c_long,
            AT_FDCWD,
            c"/".as_ptr(),
            O_DIRECTORY | O_CLOEXEC,
        )
    }
}

/// Checks that a probe the lock-down must refuse failed with the error that shows which
/// layer refused it.
fn expect_refused(result: c_long, errno: i32, layer: &str) -> io::Result<()> {
    let error = io::Error::last_os_error();
    if result >= 0 {
        // SAFETY: the probe opened this descriptor; it is closed once, here.
        unsafe { close(result as c_int) };
        return Err(io::Error::other(format!("{layer} left access open")));
    }
    if error.raw_os_error() != Some(errno) {
        return Err(error);
    }
    Ok(())
}

fn check(result: c_long) -> io::Result<c_long> {
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(result)
    }
}
