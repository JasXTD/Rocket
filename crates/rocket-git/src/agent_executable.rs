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
    use std::{
        ffi::{CString, c_char, c_int},
        os::{fd::FromRawFd, unix::ffi::OsStrExt},
        path::Component,
    };
    unsafe extern "C" {
        fn openat(fd: c_int, path: *const c_char, flags: c_int, ...) -> c_int;
    }
    check_quarantine(file)?;
    // Gatekeeper can assess enclosing code bundles, including bundles without
    // an .app suffix. Inspect every enclosing directory of the HELD vnode.
    // Walk from / by descriptor so a substituted parent cannot redirect checks.
    let path = crate::local_files::opened_path(file)
        .map_err(|e| Error::io("locate agent executable policy context", e))?;
    let mut directory = File::open("/").map_err(|e| Error::io("open agent policy root", e))?;
    check_quarantine(&directory)?;
    let mut components = path.components().peekable();
    while let Some(component) = components.next() {
        let part = match component {
            Component::RootDir => continue,
            Component::Normal(part) => part,
            _ => {
                return Err(Error::InvalidInput(
                    "Agent executable has an invalid policy context.".into(),
                ));
            }
        };
        let leaf = components.peek().is_none();
        let name = CString::new(part.as_bytes())
            .map_err(|_| Error::InvalidInput("Agent executable path contains NUL.".into()))?;
        // Darwin O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK, plus O_DIRECTORY for
        // ancestors. No create flags or permission/attribute modifications.
        let flags = 0x100 | 0x100_0000 | 4 | if leaf { 0 } else { 0x10_0000 };
        // SAFETY: held directory, live single-component name, read-only flags.
        let fd = unsafe { openat(directory.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(Error::io(
                "open agent executable policy context",
                std::io::Error::last_os_error(),
            ));
        }
        // SAFETY: newly opened descriptor, exclusively owned here.
        let opened = unsafe { File::from_raw_fd(fd) };
        check_quarantine(&opened)?;
        if leaf {
            let expected = file
                .metadata()
                .map_err(|e| Error::io("inspect held agent", e))?;
            let actual = opened
                .metadata()
                .map_err(|e| Error::io("inspect agent context", e))?;
            let final_path = crate::local_files::opened_path(file)
                .map_err(|e| Error::io("recheck agent policy context", e))?;
            if actual.dev() != expected.dev()
                || actual.ino() != expected.ino()
                || path != final_path
            {
                return Err(Error::InvalidInput("Agent executable or enclosing directory changed during its security check. Review the session again.".into()));
            }
        }
        directory = opened;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn check_quarantine(file: &File) -> Result<()> {
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
        return Err(Error::InvalidInput("The selected agent executable or an enclosing folder/bundle has macOS quarantine metadata. Rocket will not bypass Gatekeeper by copying it. Choose a trusted installed executable without quarantine metadata; Rocket does not remove quarantine or approve downloads.".into()));
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
