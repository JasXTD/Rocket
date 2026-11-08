//! Observe an owned Darwin child's exit without releasing its PID before cleanup.
//! Shared with the terminal; the SDK assertion checks this siginfo_t layout.
use std::{
    io,
    os::unix::process::ExitStatusExt,
    process::{Child, ExitStatus},
};

#[repr(C)]
#[derive(Default)]
struct SigInfo {
    signo: i32,
    errno: i32,
    code: i32,
    pid: i32,
    uid: u32,
    status: i32,
    address: usize,
    value: usize,
    band: isize,
    reserved: [usize; 7],
}
const _: () = assert!(std::mem::size_of::<SigInfo>() == 104);
unsafe extern "C" {
    fn waitid(kind: i32, id: u32, info: *mut SigInfo, options: i32) -> i32;
}

/// The caller must stop owned descendants before Child::wait releases the PID.
pub(crate) fn poll_exit(child: &Child) -> io::Result<Option<ExitStatus>> {
    let mut info = SigInfo::default();
    // SAFETY: correct Darwin siginfo_t; P_PID=1, WEXITED|WNOHANG|WNOWAIT.
    if unsafe { waitid(1, child.id(), &mut info, 4 | 1 | 0x20) } != 0 {
        let error = io::Error::last_os_error();
        return if error.kind() == io::ErrorKind::Interrupted {
            Ok(None)
        } else {
            Err(error)
        };
    }
    if info.pid == 0 {
        return Ok(None);
    }
    if info.pid as u32 != child.id() {
        return Err(io::Error::other("Child wait returned another process"));
    }
    let raw = match info.code {
        1 => (info.status & 0xff) << 8,   // CLD_EXITED
        2 => info.status & 0x7f,          // CLD_KILLED
        3 => (info.status & 0x7f) | 0x80, // CLD_DUMPED
        _ => return Err(io::Error::other("Unexpected child exit status")),
    };
    Ok(Some(ExitStatus::from_raw(raw)))
}
