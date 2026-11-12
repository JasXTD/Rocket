//! Copying verified bytes must not make an OS-blocked source executable.
use crate::{Error, Result};
use std::{
    fs::File,
    os::{fd::AsRawFd, unix::fs::MetadataExt},
};

/// Validate the held Unix source before copying it for an Agents launch and again
/// before executing that reviewed copy. Does not alter permissions or metadata.
pub fn check_agent_executable_source(file: &File) -> Result<()> {
    if file
        .metadata()
        .map_err(|e| Error::io("inspect agent execution policy", e))?
        .mode()
        & 0o111
        == 0
    {
        return Err(Error::InvalidInput(
            "The selected agent file is not executable.".into(),
        ));
    }
    platform(file)
}

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
fn platform(file: &File) -> Result<()> {
    use std::ffi::{c_int, c_long};
    unsafe extern "C" {
        fn syscall(number: c_long, ...) -> c_long;
    }
    // Linux 5.8+: check the held inode with effective credentials, including ACLs
    // and its mount's noexec flag. The older faccessat emulation loses that fidelity.
    const SYS_FACCESSAT2: c_long = 439; // x86_64 and aarch64 Linux UAPI
    const X_OK: c_int = 1;
    const AT_EACCESS: c_int = 0x200;
    const AT_EMPTY_PATH: c_int = 0x1000;
    // SAFETY: live descriptor, NUL-terminated empty name, native scalar arguments.
    if unsafe {
        syscall(
            SYS_FACCESSAT2,
            file.as_raw_fd(),
            c"".as_ptr(),
            X_OK,
            AT_EMPTY_PATH | AT_EACCESS,
        )
    } != 0
    {
        return Err(Error::io(
            "check agent execution permission (including noexec mounts; Linux 5.8 or newer required)",
            std::io::Error::last_os_error(),
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn platform(file: &File) -> Result<()> {
    use std::ffi::{c_char, c_int, c_void};
    unsafe extern "C" {
        fn fgetxattr(
            fd: c_int,
            name: *const c_char,
            value: *mut c_void,
            size: usize,
            position: u32,
            options: c_int,
        ) -> isize;
    }
    // Fail closed for every quarantine value (including empty/approved-looking
    // values). A byte copy cannot carry Gatekeeper's original-file assessment.
    // Query only the size on the held file: no untrusted attribute allocation.
    // SAFETY: live descriptor, constant NUL-terminated name, documented size query.
    let size = unsafe {
        fgetxattr(
            file.as_raw_fd(),
            c"com.apple.quarantine".as_ptr(),
            std::ptr::null_mut(),
            0,
            0,
            0,
        )
    };
    if size >= 0 {
        return Err(Error::InvalidInput("The selected agent executable has macOS quarantine metadata. Rocket will not bypass Gatekeeper by copying it. Choose a trusted installed executable without quarantine metadata; Rocket does not remove quarantine or approve downloads.".into()));
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(93) {
        // ENOATTR, Darwin
        Ok(())
    } else {
        Err(Error::io("check agent quarantine metadata", error))
    }
}

#[cfg(not(any(
    target_os = "macos",
    all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )
)))]
fn platform(_: &File) -> Result<()> {
    Err(Error::InvalidInput(
        "Agent executable policy checks are unavailable on this platform.".into(),
    ))
}
