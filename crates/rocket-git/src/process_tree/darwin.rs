//! Distinguish Darwin's all-zombie killpg EPERM from an actual permission denial.
use std::{ffi::c_void, io};

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
#[link(name = "proc")]
unsafe extern "C" {
    fn proc_listpids(kind: u32, info: u32, buffer: *mut c_void, bytes: i32) -> i32;
    fn proc_pidinfo(pid: i32, flavor: i32, arg: u64, buffer: *mut c_void, bytes: i32) -> i32;
    fn __error() -> *mut i32;
}

pub(super) fn has_live_members(group: i32) -> io::Result<bool> {
    if group <= 1 {
        return Err(io::Error::other("Invalid owned process group"));
    }
    // PROC_PGRP_ONLY=2. The kernel includes zombies; a complete snapshot and
    // each member's state are required before accepting an EPERM as cleanup.
    let mut capacity = 64usize;
    loop {
        let mut pids = vec![0i32; capacity];
        // libproc returns zero for both an empty group and a syscall error.
        // SAFETY: __error points to this thread's errno; writable PID buffer.
        let bytes = unsafe {
            *__error() = 0;
            proc_listpids(
                2,
                group as u32,
                pids.as_mut_ptr().cast(),
                (capacity * 4) as i32,
            )
        };
        if bytes <= 0 && io::Error::last_os_error().raw_os_error() != Some(0) {
            return Err(io::Error::last_os_error());
        }
        if bytes < 0 {
            return Err(io::Error::other("Invalid process-group inspection length"));
        }
        if bytes as usize > capacity * 4 || bytes % 4 != 0 {
            return Err(io::Error::other("Invalid process-group inspection result"));
        }
        if bytes as usize == capacity * 4 {
            if capacity == 131_072 {
                return Err(io::Error::other(
                    "Process-group inspection exceeded its bound",
                ));
            }
            capacity *= 2;
            continue;
        }
        for pid in pids.into_iter().take(bytes as usize / 4) {
            if pid <= 1 {
                return Err(io::Error::other("Unexpected process-group member"));
            }
            let mut info = BsdInfo::default();
            // SAFETY: SDK-checked proc_bsdshortinfo, PROC_PIDT_SHORTBSDINFO=13;
            // arg=1 requests zombie information as well as live processes.
            let read = unsafe {
                *__error() = 0;
                proc_pidinfo(pid, 13, 1, (&mut info as *mut BsdInfo).cast(), 64)
            };
            if read != 64 {
                let error = io::Error::last_os_error();
                if read <= 0 && error.raw_os_error() == Some(3) {
                    continue;
                } // ESRCH: reaped meanwhile.
                return Err(if read <= 0 && error.raw_os_error() != Some(0) {
                    error
                } else {
                    io::Error::other("Incomplete process-group member inspection")
                });
            }
            if info.pid != pid as u32 {
                return Err(io::Error::other("Process inspection returned another PID"));
            }
            if info.group == group as u32 && info.status != 5 {
                return Ok(true);
            } // SZOMB=5
        }
        return Ok(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process_tree::{ProcessTree, poll_exit};
    use std::{
        io::{BufRead, BufReader},
        process::{Child, Command, Stdio},
        thread,
        time::{Duration, Instant},
    };
    struct Fixture {
        child: Option<Child>,
        tree: ProcessTree,
    }
    impl Fixture {
        fn new(script: &str) -> Self {
            let mut command = Command::new("/bin/sh");
            command
                .args(["-c", script])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null());
            let starting = ProcessTree::prepare(&mut command).unwrap();
            let child = command.spawn().unwrap();
            let tree = ProcessTree::attach(starting, &child).unwrap();
            Self {
                child: Some(child),
                tree,
            }
        }
        fn observed_exit(&mut self) {
            let deadline = Instant::now() + Duration::from_secs(5);
            while poll_exit(self.child.as_mut().unwrap()).unwrap().is_none() {
                assert!(Instant::now() < deadline, "fixture did not exit");
                thread::sleep(Duration::from_millis(10));
            }
        }
        fn reap(&mut self) -> std::process::ExitStatus {
            self.child.take().unwrap().wait().unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            if let Some(mut child) = self.child.take() {
                let _ = self.tree.terminate();
                let _ = child.wait();
            }
        }
    }
    #[test]
    fn exited_unreaped_group_is_cleanup_success_and_preserves_exit_status() {
        let mut fixture = Fixture::new("exit 7");
        fixture.observed_exit();
        let pid = fixture.child.as_ref().unwrap().id() as i32;
        assert!(!has_live_members(pid).unwrap());
        fixture.tree.terminate().unwrap();
        // wait still owns the child, unlike try_wait's eager reap.
        assert_eq!(fixture.reap().code(), Some(7));
    }
    #[test]
    fn cleanup_finds_live_descendants_of_an_exited_parent_and_preserves_a_canary() {
        let mut canary = Fixture::new("exec /bin/sleep 30");
        let mut fixture = Fixture::new("/bin/sleep 30 & echo $!; exit 7");
        let mut line = String::new();
        BufReader::new(fixture.child.as_mut().unwrap().stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert!(line.trim().parse::<u32>().unwrap() > 1);
        fixture.observed_exit();
        let group = fixture.child.as_ref().unwrap().id() as i32;
        assert!(has_live_members(group).unwrap());
        fixture.tree.terminate().unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while has_live_members(group).unwrap() {
            assert!(Instant::now() < deadline, "owned descendant survived");
            thread::sleep(Duration::from_millis(10));
        }
        assert!(poll_exit(canary.child.as_mut().unwrap()).unwrap().is_none());
        assert_eq!(fixture.reap().code(), Some(7));
    }
    #[test]
    fn inspection_refuses_broadcast_and_caller_group_selectors() {
        for group in [-1, 0, 1] {
            assert!(has_live_members(group).is_err());
        }
    }
}
