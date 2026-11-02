//! Disposable native acceptance helper; never shipped with Rocket.
//! Holds only the window thread of the runner's rocket-mcp child for 21.1 seconds.
use std::{ffi::c_void, fs::OpenOptions, io::Write, path::PathBuf, time::Duration};
type Handle = *mut c_void;
#[link(name = "kernel32")]
unsafe extern "system" {
    fn OpenProcess(access: u32, inherit: i32, id: u32) -> Handle;
    fn QueryFullProcessImageNameW(
        process: Handle,
        flags: u32,
        name: *mut u16,
        size: *mut u32,
    ) -> i32;
    fn OpenThread(access: u32, inherit: i32, id: u32) -> Handle;
    fn GetProcessIdOfThread(thread: Handle) -> u32;
    fn SuspendThread(thread: Handle) -> u32;
    fn ResumeThread(thread: Handle) -> u32;
    fn CloseHandle(handle: Handle) -> i32;
}
#[link(name = "user32")]
unsafe extern "system" {
    fn EnumWindows(callback: unsafe extern "system" fn(Handle, isize) -> i32, data: isize) -> i32;
    fn IsWindowVisible(window: Handle) -> i32;
    fn GetWindowThreadProcessId(window: Handle, process: *mut u32) -> u32;
}
struct OwnedHandle(Handle);
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
struct HeldThread(OwnedHandle);
impl Drop for HeldThread {
    fn drop(&mut self) {
        unsafe {
            ResumeThread(self.0.0);
        }
    }
}
struct Search {
    process: u32,
    thread: u32,
}
unsafe extern "system" fn window(window: Handle, data: isize) -> i32 {
    // SAFETY: EnumWindows is synchronous and receives this live stack value.
    let search = unsafe { &mut *(data as *mut Search) };
    let mut process = 0;
    let thread = unsafe { GetWindowThreadProcessId(window, &mut process) };
    if process == search.process && unsafe { IsWindowVisible(window) } != 0 {
        search.thread = thread;
        return 0;
    }
    1
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let process: u32 = args
        .next()
        .ok_or("expected owned child PID")?
        .to_str()
        .ok_or("PID encoding")?
        .parse()?;
    let marker = PathBuf::from(args.next().ok_or("expected absolute fixture marker")?);
    if !marker.is_absolute() || marker.exists() || args.next().is_some() {
        return Err("expected new absolute fixture marker".into());
    }
    // QUERY_LIMITED_INFORMATION, no process mutation access.
    let handle = unsafe { OpenProcess(0x1000, 0, process) };
    if handle.is_null() {
        return Err(std::io::Error::last_os_error().into());
    }
    let handle = OwnedHandle(handle);
    let mut name = vec![0u16; 32768];
    let mut size = name.len() as u32;
    if unsafe { QueryFullProcessImageNameW(handle.0, 0, name.as_mut_ptr(), &mut size) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let executable = PathBuf::from(String::from_utf16(&name[..size as usize])?);
    if !executable
        .file_name()
        .is_some_and(|s| s.eq_ignore_ascii_case("rocket-mcp.exe"))
    {
        return Err("expected the owned rocket-mcp process".into());
    }
    let mut search = Search { process, thread: 0 };
    unsafe {
        EnumWindows(window, &mut search as *mut Search as isize);
    }
    if search.thread == 0 {
        return Err("owned native window is missing".into());
    }
    // SUSPEND_RESUME | QUERY_LIMITED_INFORMATION; verify thread ownership again.
    let thread = unsafe { OpenThread(0x802, 0, search.thread) };
    if thread.is_null() {
        return Err(std::io::Error::last_os_error().into());
    }
    let thread = OwnedHandle(thread);
    if unsafe { GetProcessIdOfThread(thread.0) } != process {
        return Err("window ownership changed".into());
    }
    let prior = unsafe { SuspendThread(thread.0) };
    if prior == u32::MAX {
        return Err(std::io::Error::last_os_error().into());
    }
    let _held = HeldThread(thread); // Always undo exactly our own suspension on return/unwind.
    if prior != 0 {
        return Err("owned UI thread was already held".into());
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(marker)?;
    file.write_all(b"held")?;
    drop(file);
    std::thread::sleep(Duration::from_millis(21_100));
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
