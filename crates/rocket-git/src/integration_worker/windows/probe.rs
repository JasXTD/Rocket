//! Compiled only as an owned native test fixture, never shipped or downloaded.
#![windows_subsystem = "windows"]
use std::{
    ffi::c_void,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    process::{Command, Stdio},
    time::Duration,
};
#[link(name = "kernel32")]
unsafe extern "system" {
    fn VirtualAlloc(address: *mut c_void, size: usize, kind: u32, protection: u32) -> *mut c_void;
    fn VirtualFree(address: *mut c_void, size: usize, kind: u32) -> i32;
    fn GetCurrentProcess() -> *mut c_void;
    fn GetProcessHandleCount(process: *mut c_void, count: *mut u32) -> i32;
}
#[link(name = "ws2_32")]
unsafe extern "system" {
    fn WSAStartup(version: u16, data: *mut c_void) -> i32;
}
fn main() {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    let mut lines = input.lines();
    match lines.next().unwrap_or("") {
        "permissions" => {
            let read = std::fs::read(lines.next().unwrap()).is_ok();
            let write = std::fs::write(lines.next().unwrap(), b"fixture-only").is_ok();
            let address: SocketAddr = lines.next().unwrap().parse().unwrap();
            // The parent deliberately holds 512 inheritable file handles. Count
            // actual child handles without querying invalid/reused numeric slots.
            let mut handles = 0;
            assert_ne!(
                unsafe { GetProcessHandleCount(GetCurrentProcess(), &mut handles) },
                0
            );
            let inherited = handles >= 512;
            // Rust assumes Winsock initialization succeeds and panics if the OS
            // denies it. Probe that denial explicitly before using std sockets.
            let mut data = [0u64; 64];
            let startup = unsafe { WSAStartup(0x202, data.as_mut_ptr().cast()) };
            let network_error = if startup != 0 {
                startup
            } else {
                match TcpStream::connect_timeout(&address, Duration::from_millis(500)) {
                    Ok(_) => 0,
                    Err(e) => e.raw_os_error().unwrap_or(-1),
                }
            };
            let network = network_error == 0;
            eprintln!("network_error={network_error}");
            let child = Command::new(std::env::current_exe().unwrap())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            let child = match child {
                Ok(mut child) => {
                    let _ = child.wait();
                    true
                }
                Err(_) => false,
            };
            let memory = unsafe { VirtualAlloc(std::ptr::null_mut(), 4096, 0x3000, 0x40) };
            let executable_memory = !memory.is_null();
            if executable_memory {
                unsafe {
                    VirtualFree(memory, 0, 0x8000);
                }
            }
            let large =
                unsafe { VirtualAlloc(std::ptr::null_mut(), 256 * 1024 * 1024, 0x3000, 0x04) };
            let large_allocation = !large.is_null();
            if large_allocation {
                unsafe {
                    VirtualFree(large, 0, 0x8000);
                }
            }
            println!(
                "read={read};write={write};network={network};child={child};executable_memory={executable_memory};large_allocation={large_allocation};inherited={inherited}"
            );
        }
        "wait" => std::thread::sleep(Duration::from_secs(60)),
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
