//! Private shell startup helper, built and packaged by build.rs. It starts the program
//! the terminal names, by its full path, with the command line it gets.
//! Reset the inheritable Ctrl+C-ignore attribute only inside the owned ConPTY.
//! Never change the desktop/MCP process's console policy or load code from PATH.
#![deny(unsafe_op_in_unsafe_fn)]
#[path = "../../rocket-git/src/process_privilege.rs"]
mod process_privilege;
use std::{
    ffi::{OsStr, c_void},
    io,
    mem::size_of,
    os::windows::ffi::OsStrExt,
    path::Path,
    ptr,
};
type Handle = *mut c_void;
#[repr(C)]
#[derive(Default)]
struct Startup {
    size: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    position_and_size: [u32; 7],
    flags: u32,
    show: u16,
    reserved_size: u16,
    reserved_bytes: *mut u8,
    input: Handle,
    output: Handle,
    error: Handle,
}
#[repr(C)]
#[derive(Default)]
struct ProcessInfo {
    process: Handle,
    thread: Handle,
    pid: u32,
    tid: u32,
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn SetConsoleCtrlHandler(
        handler: Option<unsafe extern "system" fn(u32) -> i32>,
        add: i32,
    ) -> i32;
    fn CreateProcessW(
        application: *const u16,
        command: *mut u16,
        process_attributes: *const c_void,
        thread_attributes: *const c_void,
        inherit: i32,
        flags: u32,
        environment: *const c_void,
        directory: *const u16,
        startup: *const Startup,
        info: *mut ProcessInfo,
    ) -> i32;
    fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
    fn GetExitCodeProcess(handle: Handle, code: *mut u32) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
}
struct Owned(Handle);
impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: uniquely owned live process/thread handle.
        unsafe {
            CloseHandle(self.0);
        }
    }
}
fn check(value: i32) -> io::Result<()> {
    if value == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
unsafe extern "system" fn keep_host_alive(event: u32) -> i32 {
    // A handler is not inherited. Ignoring via SetConsoleCtrlHandler(None, true)
    // here would recreate the bug in the shell and its foreground commands.
    i32::from(event == 0)
}
fn wide(value: &OsStr) -> io::Result<Vec<u16>> {
    let mut text: Vec<u16> = value.encode_wide().collect();
    if text.contains(&0) {
        return Err(io::Error::other("Invalid shell command"));
    }
    text.push(0);
    Ok(text)
}
fn run(program: &OsStr, command: &OsStr) -> io::Result<u32> {
    process_privilege::check()?;
    if !Path::new(program).is_absolute() {
        return Err(io::Error::other("The shell needs a full path"));
    }
    let mut command = wide(command)?;
    let program = wide(program)?;
    // SAFETY: these calls affect only this helper. The parent created it inside
    // the terminal's kill-on-close job and attached it to the owned ConPTY.
    check(unsafe { SetConsoleCtrlHandler(None, 0) })?;
    check(unsafe { SetConsoleCtrlHandler(Some(keep_host_alive), 1) })?;
    let startup = Startup {
        size: size_of::<Startup>() as u32,
        ..Default::default()
    };
    let mut info = ProcessInfo::default();
    // SAFETY: live UTF-16 strings and initialized SDK structs. Inherit this
    // console, environment and job, but no inheritable ordinary handles. The
    // shell is created running, so no suspended child can be left at host exit.
    check(unsafe {
        CreateProcessW(
            program.as_ptr(),
            command.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            0,
            0,
            ptr::null(),
            ptr::null(),
            &startup,
            &mut info,
        )
    })?;
    let process = Owned(info.process);
    drop(Owned(info.thread));
    // SAFETY: owned process handle; the parent can stop the entire job at any time.
    if unsafe { WaitForSingleObject(process.0, u32::MAX) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let mut code = 0;
    check(unsafe { GetExitCodeProcess(process.0, &mut code) })?;
    Ok(code)
}
fn main() {
    let mut args = std::env::args_os().skip(1);
    let (Some(program), Some(command), None) = (args.next(), args.next(), args.next()) else {
        eprintln!("Rocket terminal could not start the shell: unexpected arguments");
        std::process::exit(126);
    };
    let code = match run(&program, &command) {
        Ok(code) => code,
        Err(error) => {
            // Do not print the startup command, paths, history or environment.
            let name = Path::new(&program).file_stem().unwrap_or_default().to_string_lossy();
            eprintln!("Rocket terminal could not start {name}: {error}");
            126
        }
    };
    std::process::exit(code as i32);
}

#[cfg(test)]
mod privilege_tests {
    use super::*;
    #[test]
    fn direct_terminal_helper_refuses_before_inspecting_or_starting_shell() {
        for state in [Err(()), Ok((true, 0x2000)), Ok((false, 0x3000)), Ok((true, 0x4000))] {
            process_privilege::with_test_token(state, || {
                let error = run(OsStr::new("not-an-absolute-program"), OsStr::new("must-not-run")).unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
            });
        }
    }
}
