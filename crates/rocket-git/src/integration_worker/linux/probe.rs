//! Compiled only as an owned native test fixture, never shipped or downloaded. It locks
//! itself down with the worker's own code (`lockdown.rs`, copied beside it), then tries
//! to get out.
mod lockdown;

use std::{
    ffi::{c_int, c_long, c_void},
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    process::Command,
    time::Duration,
};

unsafe extern "C" {
    fn fcntl(fd: c_int, command: c_int, ...) -> c_int;
    fn mmap(
        address: *mut c_void,
        length: usize,
        protection: c_int,
        flags: c_int,
        fd: c_int,
        offset: c_long,
    ) -> *mut c_void;
    fn mprotect(address: *mut c_void, length: usize, protection: c_int) -> c_int;
    fn fork() -> c_int;
    fn _exit(status: c_int) -> !;
    fn kill(pid: c_int, signal: c_int) -> c_int;
    fn getppid() -> c_int;
}
const F_GETFD: c_int = 1;
const PROT_READ: c_int = 1;
const PROT_WRITE: c_int = 2;
const PROT_EXEC: c_int = 4;
const MAP_PRIVATE: c_int = 2;
const MAP_ANONYMOUS: c_int = 0x20;

fn mapped(length: usize, protection: c_int) -> Option<*mut c_void> {
    // SAFETY: a new private anonymous mapping that nothing else refers to.
    let address = unsafe {
        mmap(
            std::ptr::null_mut(),
            length,
            protection,
            MAP_PRIVATE | MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    (address as usize != usize::MAX).then_some(address)
}

fn main() {
    // Before the lock-down: a descriptor the launcher let through would show here.
    // SAFETY: F_GETFD only reads descriptor flags; unused numbers fail with EBADF.
    let inherited = (3..1024).any(|fd| unsafe { fcntl(fd, F_GETFD) } != -1);
    // SAFETY: getppid has no preconditions.
    let parent = unsafe { getppid() };
    if let Err(error) = lockdown::apply() {
        println!("lockdown={error}");
        std::process::exit(3);
    }
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    let mut lines = input.lines();
    match lines.next().unwrap_or("") {
        "permissions" => {
            let read = std::fs::read(lines.next().unwrap()).is_ok();
            let write = std::fs::write(lines.next().unwrap(), b"fixture-only").is_ok();
            let address: SocketAddr = lines.next().unwrap().parse().unwrap();
            let network = match TcpStream::connect_timeout(&address, Duration::from_millis(500)) {
                Ok(_) => true,
                Err(e) => {
                    eprintln!("network_error={}", e.raw_os_error().unwrap_or(-1));
                    false
                }
            };
            // SAFETY: a forked child only exits at once.
            let forked = match unsafe { fork() } {
                0 => unsafe { _exit(0) },
                pid => pid > 0,
            };
            let child = forked || Command::new("/bin/true").spawn().is_ok();
            let executable_memory = mapped(4096, PROT_READ | PROT_WRITE | PROT_EXEC).is_some()
                || mapped(4096, PROT_READ | PROT_WRITE).is_some_and(|page| {
                    // SAFETY: a page this probe mapped and owns.
                    unsafe { mprotect(page, 4096, PROT_READ | PROT_EXEC) == 0 }
                });
            let large_allocation = mapped(256 * 1024 * 1024, PROT_READ | PROT_WRITE).is_some();
            // SAFETY: signal 0 only checks permission; it delivers nothing.
            let signal = unsafe { kill(parent, 0) } == 0;
            println!(
                "read={read};write={write};network={network};child={child};executable_memory={executable_memory};large_allocation={large_allocation};inherited={inherited};signal={signal}"
            );
        }
        // The sandbox has no sleep call; spin until stopped.
        "wait" => loop {
            std::hint::spin_loop();
        },
        "flood" => loop {
            if std::io::stdout().write_all(&[b'x'; 8192]).is_err() {
                break;
            }
        },
        "echo" => {
            std::io::stdout()
                .write_all(lines.collect::<Vec<_>>().join("\n").as_bytes())
                .unwrap();
        }
        _ => println!("native-fixture"),
    }
}
