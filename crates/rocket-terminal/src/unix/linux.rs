//! Linux-only PTY ABI and session enumeration.
use super::*;
pub(super) const O_RDWR: c_int = 0o2;
pub(super) const O_NOCTTY: c_int = 0o400;
pub(super) const O_NONBLOCK: c_int = 0o4000;
pub(super) const O_CLOEXEC: c_int = 0o2000000;
pub(super) const F_GETFL: c_int = 3;
pub(super) const F_SETFL: c_int = 4;
pub(super) const TIOCSCTTY: c_ulong = 0x540E;
pub(super) const TIOCSWINSZ: c_ulong = 0x5414;
/// Opens the terminal's other end from its master, without a path under /dev/pts.
pub(super) const TIOCGPTPEER: c_ulong = 0x5441;
pub(super) const TCSANOW: c_int = 0;
/// Erase whole UTF-8 characters in line editing (`cat`, `read`), not single bytes.
pub(super) const IUTF8: u32 = 0o40000;
pub(super) const SIGCONT: c_int = 18;
pub(super) const SIGSTOP: c_int = 19;
/// glibc's Linux `struct termios`.
#[repr(C)]
pub(super) struct Termios {
    pub(super) input: u32,
    pub(super) output: u32,
    pub(super) control: u32,
    pub(super) local: u32,
    pub(super) line: u8,
    pub(super) characters: [u8; 32],
    pub(super) input_speed: u32,
    pub(super) output_speed: u32,
}
const _: () = assert!(std::mem::size_of::<Termios>() == 60);

/// The process groups of the live processes in `session`, read from /proc.
pub(super) fn session_groups(session: i32) -> BTreeSet<i32> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return BTreeSet::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
            let (state, group, member) = parse_stat(&stat)?;
            (member == session && !matches!(state, 'Z' | 'X')).then_some(group)
        })
        .collect()
}

/// A process's state, process group and session from its /proc stat line. The name in
/// parentheses may itself contain spaces and parentheses, so fields count from the last.
pub(super) fn parse_stat(stat: &str) -> Option<(char, i32, i32)> {
    let mut fields = stat.get(stat.rfind(')')? + 1..)?.split_whitespace();
    let state = fields.next()?.chars().next()?;
    let _parent = fields.next()?;
    let group = fields.next()?.parse().ok()?;
    let session = fields.next()?.parse().ok()?;
    Some((state, group, session))
}

/// Ends the session `child` leads. Its own group is signalled only while it is not
/// reaped: its ID can't be reused until then. Other groups are those /proc lists.
fn cleanup_session(child: &mut Child, exited: &mut Option<std::process::ExitStatus>) {
    let leader = child.id() as i32;
    let signal_all = |exited: bool, number: c_int| {
        let mut groups = session_groups(leader);
        if !exited {
            groups.insert(leader);
        }
        // Never 0 or 1: kill(-0) would signal Rocket's own group.
        for group in groups.iter().filter(|group| **group > 1) {
            // SAFETY: a negative ID signals that process group; failures (already gone,
            // or a root process such as sudo's) are expected and ignored.
            unsafe { kill(-group, number) };
        }
        groups.is_empty()
    };
    let mut reap = |exited: &mut Option<std::process::ExitStatus>| {
        if exited.is_none() {
            *exited = child.try_wait().ok().flatten();
        }
        exited.is_some()
    };
    // A hang-up, as when a terminal window closes, and a continue for stopped jobs.
    signal_all(reap(exited), SIGHUP);
    signal_all(reap(exited), SIGCONT);
    let deadline = Instant::now() + GRACE;
    while Instant::now() < deadline {
        if reap(exited) && session_groups(leader).is_empty() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    // Whatever is left is killed; new processes may still appear while this runs.
    let deadline = Instant::now() + Duration::from_secs(5);
    while !signal_all(reap(exited), SIGKILL) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    while !reap(exited) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub(super) type PollCount = c_ulong;
pub(super) const UTF8_LOCALE: &str = "C.UTF-8";
pub(super) fn open_peer(fd: c_int) -> io::Result<OwnedFd> {
    // SAFETY: Linux TIOCGPTPEER returns a new descriptor, owned here.
    let peer = checked(unsafe { ioctl(fd, TIOCGPTPEER, O_RDWR | O_NOCTTY | O_CLOEXEC) })?;
    Ok(unsafe { OwnedFd::from_raw_fd(peer) })
}
pub(super) fn poll_exit(child: &mut Child) -> io::Result<Option<std::process::ExitStatus>> {
    child.try_wait()
}

pub(super) fn end_session(
    child: &mut Child,
    exited: &mut Option<std::process::ExitStatus>,
) -> io::Result<()> {
    cleanup_session(child, exited);
    Ok(())
}
