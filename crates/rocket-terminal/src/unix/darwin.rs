//! Darwin ABI, from Apple's sys/{termios,ttycom,fcntl,poll,signal,wait,proc_info}.h.
//! No Linux ioctl values, /proc lookup, or shell process-list commands.
use super::*;
pub(super) use crate::darwin_wait::poll_exit;
use std::{
    ffi::c_void,
    os::unix::fs::{FileTypeExt, OpenOptionsExt},
};

pub(super) const O_RDWR: c_int = 2;
pub(super) const O_NOCTTY: c_int = 0x20000;
pub(super) const O_NONBLOCK: c_int = 4;
pub(super) const O_CLOEXEC: c_int = 0x1000000;
pub(super) const F_GETFL: c_int = 3;
pub(super) const F_SETFL: c_int = 4;
pub(super) const TIOCSCTTY: c_ulong = 0x20007461;
pub(super) const TIOCSWINSZ: c_ulong = 0x80087467;
pub(super) const TCSANOW: c_int = 0;
pub(super) const IUTF8: c_ulong = 0x4000;
pub(super) const SIGCONT: c_int = 19;
pub(super) const SIGSTOP: c_int = 17;
pub(super) type PollCount = u32;
// C.UTF-8 is not shipped by macOS. Its en_US.UTF-8 character locale is available.
pub(super) const UTF8_LOCALE: &str = "en_US.UTF-8";

#[repr(C)]
pub(super) struct Termios {
    pub(super) input: c_ulong,
    output: c_ulong,
    control: c_ulong,
    local: c_ulong,
    characters: [u8; 20],
    input_speed: c_ulong,
    output_speed: c_ulong,
}
const _: () = assert!(std::mem::size_of::<Termios>() == 72);

pub(super) fn open_peer(fd: c_int) -> io::Result<OwnedFd> {
    let mut name = [0u8; 128];
    // SAFETY: Darwin TIOCPTYGNAME writes at most 128 bytes, including the NUL.
    checked(unsafe { ioctl(fd, 0x40807453 as c_ulong, name.as_mut_ptr()) })?;
    let nul = name
        .iter()
        .position(|b| *b == 0)
        .ok_or_else(|| io::Error::other("Terminal device name was not terminated"))?;
    let name = std::str::from_utf8(&name[..nul])
        .map_err(|_| io::Error::other("Invalid terminal device name"))?;
    // Names come from the kernel, never a repository. Refuse unexpected paths.
    if !name.starts_with("/dev/ttys")
        || name[9..].is_empty()
        || !name[9..].bytes().all(|b| b.is_ascii_alphanumeric())
    {
        return Err(io::Error::other("Unexpected terminal device name"));
    }
    // std sets close-on-exec. O_NOFOLLOW prevents replacing the leaf with a link.
    let peer = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(O_NOCTTY | 0x100)
        .open(name)?;
    if !peer.metadata()?.file_type().is_char_device() {
        return Err(io::Error::other("Terminal peer is not a character device"));
    }
    Ok(peer.into())
}

#[repr(C)]
#[derive(Default)]
struct BsdInfo {
    pid: u32,
    parent: u32,
    group: u32,
    status: u32,
    name: [u8; 16],
    flags: u32,
    uid: u32,
    gid: u32,
    ruid: u32,
    rgid: u32,
    svuid: u32,
    svgid: u32,
    reserved: u32,
}
const _: () = assert!(std::mem::size_of::<BsdInfo>() == 64);
unsafe extern "C" {
    fn getsid(pid: c_int) -> c_int;
    fn geteuid() -> u32;
}
#[link(name = "proc")]
unsafe extern "C" {
    fn proc_listpids(kind: u32, info: u32, buffer: *mut c_void, bytes: c_int) -> c_int;
    fn proc_pidinfo(
        pid: c_int,
        flavor: c_int,
        arg: u64,
        buffer: *mut c_void,
        bytes: c_int,
    ) -> c_int;
}

fn session_groups(session: i32) -> io::Result<BTreeSet<i32>> {
    // Enumerate only this user's processes. A session is not a sandbox: setsid
    // deliberately detaches a job, and privileged programs may be unsignallable.
    let uid = unsafe { geteuid() };
    let needed = unsafe { proc_listpids(4, uid, std::ptr::null_mut(), 0) };
    if needed <= 0 {
        return Err(io::Error::other("Cannot enumerate terminal jobs"));
    }
    let mut capacity = ((needed as usize / 4) + 64).min(131_072);
    loop {
        let mut pids = vec![0i32; capacity];
        // SAFETY: allocated writable PID array; size is bytes, not entry count.
        let bytes =
            unsafe { proc_listpids(4, uid, pids.as_mut_ptr().cast(), (capacity * 4) as i32) };
        if bytes <= 0 || bytes as usize > capacity * 4 || bytes % 4 != 0 {
            return Err(io::Error::other("Cannot enumerate terminal jobs"));
        }
        if bytes as usize == capacity * 4 {
            if capacity == 131_072 {
                return Err(io::Error::other(
                    "Terminal job enumeration exceeded its bound",
                ));
            }
            capacity = (capacity * 2).min(131_072);
            continue;
        }
        let mut groups = BTreeSet::new();
        for pid in pids.into_iter().take(bytes as usize / 4).filter(|p| *p > 1) {
            // SAFETY: inspection only; vanished processes no longer need stopping.
            if unsafe { getsid(pid) } != session {
                continue;
            }
            let mut info = BsdInfo::default();
            let read = unsafe { proc_pidinfo(pid, 13, 0, (&mut info as *mut BsdInfo).cast(), 64) };
            if read == 64
                && info.pid == pid as u32
                && info.uid == uid
                && info.status != 5
                && info.group > 1
                && info.group <= i32::MAX as u32
                && unsafe { getsid(pid) } == session
            {
                groups.insert(info.group as i32);
            }
        }
        return Ok(groups);
    }
}

pub(super) fn end_session(
    child: &mut Child,
    exited: &mut Option<std::process::ExitStatus>,
) -> io::Result<()> {
    let leader = child.id() as i32;
    let mut failure = None;
    let mut signal_groups = |number| {
        let mut groups = match session_groups(leader) {
            Ok(groups) => groups,
            Err(error) => {
                failure = Some(error);
                BTreeSet::new()
            }
        };
        // Still owned and unreaped, even if exit was already observed.
        groups.insert(leader);
        for group in groups.into_iter().filter(|group| *group > 1) {
            // SAFETY: only groups in the owned session; never group 0 or 1.
            unsafe { kill(-group, number) };
        }
    };
    signal_groups(SIGHUP);
    signal_groups(SIGCONT);
    let started = Instant::now();
    loop {
        if exited.is_none() {
            *exited = poll_exit(child)?;
        }
        if exited.is_some() && session_groups(leader)?.is_empty() {
            break;
        }
        if started.elapsed() >= GRACE {
            signal_groups(SIGKILL);
        }
        if started.elapsed() >= GRACE + Duration::from_secs(5) {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Terminal jobs did not stop within the shutdown limit",
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    // Already observed exit, so this reaps without a blocking UI wait.
    *exited = Some(child.wait()?);
    match failure {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pty_descriptors_are_close_on_exec_and_have_darwin_utf8_settings() {
        let (master, peer) = open_terminal(91, 17).unwrap();
        for fd in [master.as_raw_fd(), peer.as_raw_fd()] {
            assert_eq!(unsafe { fcntl(fd, 1) } & 1, 1, "FD_CLOEXEC");
        }
        let mut settings = std::mem::MaybeUninit::<Termios>::uninit();
        assert_eq!(
            unsafe { tcgetattr(peer.as_raw_fd(), settings.as_mut_ptr()) },
            0
        );
        assert_ne!(unsafe { settings.assume_init() }.input & IUTF8, 0);
    }
    #[test]
    fn exit_observation_keeps_pid_owned_until_reap() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "exit 7"])
            .spawn()
            .unwrap();
        let start = Instant::now();
        while poll_exit(&mut child).unwrap().is_none() {
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(POLL);
        }
        assert_eq!(poll_exit(&mut child).unwrap().unwrap().code(), Some(7));
        assert_eq!(child.wait().unwrap().code(), Some(7));
    }
}
