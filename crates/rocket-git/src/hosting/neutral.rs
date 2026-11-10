//! Empty, owned working directories for credential helpers and HTTP requests.
//!
//! A ceiling stops Git searching parents, not reading a repository in its cwd.
//! Never reuse a predictable or pre-existing directory: its local Git config
//! could execute a credential helper before the first HTTP request.
#[cfg(unix)]
use crate::local_storage;
use crate::{Error, Result};
use std::{
    fs, io,
    ops::Deref,
    path::{Path, PathBuf},
    sync::Arc,
};

/// Cloned hosting sessions share the directory until the last request completes.
#[derive(Clone)]
pub(crate) struct NeutralDirectory(Arc<OwnedDirectory>);

struct OwnedDirectory(PathBuf);

impl Deref for NeutralDirectory {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0.0
    }
}

impl Drop for OwnedDirectory {
    fn drop(&mut self) {
        // Never recursively delete a directory a helper has populated. Normal
        // requests keep this empty; unexpected files are preserved for the user.
        let _ = fs::remove_dir(&self.0);
    }
}

impl NeutralDirectory {
    fn create_new(path: PathBuf) -> io::Result<Self> {
        // Atomic mkdir, rejecting existing files/directories/links. On Unix this
        // also enforces 0700, ownership and safe ancestors (including Darwin ACLs).
        // Root and processes with our own identity remain outside this boundary.
        #[cfg(unix)]
        local_storage::create_private_new(&path)?;
        #[cfg(not(unix))]
        fs::create_dir(&path)?;
        Ok(Self(Arc::new(OwnedDirectory(path))))
    }
}

pub(crate) fn neutral_directory() -> Result<NeutralDirectory> {
    let root = std::env::temp_dir();
    for _ in 0..8 {
        let mut bytes = [0u8; 16];
        random(&mut bytes).map_err(|e| Error::io("generate private request folder name", e))?;
        let suffix: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        let path = root.join(format!("rocket-request-{suffix}"));
        match NeutralDirectory::create_new(path) {
            Ok(directory) => return Ok(directory),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(Error::io("prepare private request folder", e)),
        }
    }
    Err(Error::io(
        "prepare private request folder",
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Could not create a new private request folder; existing files were preserved",
        ),
    ))
}

#[cfg(windows)]
fn random(bytes: &mut [u8; 16]) -> io::Result<()> {
    #[link(name = "bcrypt")]
    unsafe extern "system" {
        fn BCryptGenRandom(
            algorithm: *mut std::ffi::c_void,
            bytes: *mut u8,
            length: u32,
            flags: u32,
        ) -> i32;
    }
    // SAFETY: a live writable 16-byte buffer; system-preferred RNG, no provider handle.
    let status = unsafe { BCryptGenRandom(std::ptr::null_mut(), bytes.as_mut_ptr(), 16, 2) };
    if status < 0 {
        Err(io::Error::other(
            "The system random generator is unavailable",
        ))
    } else {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn random(bytes: &mut [u8; 16]) -> io::Result<()> {
    unsafe extern "C" {
        fn getrandom(buffer: *mut u8, length: usize, flags: u32) -> isize;
    }
    let mut filled = 0;
    while filled < bytes.len() {
        // SAFETY: the remaining live, writable portion of the buffer.
        let count = unsafe { getrandom(bytes[filled..].as_mut_ptr(), bytes.len() - filled, 0) };
        if count < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "The system random generator returned no bytes",
            ));
        }
        filled += count as usize;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn random(bytes: &mut [u8; 16]) -> io::Result<()> {
    #[link(name = "Security", kind = "framework")]
    unsafe extern "C" {
        fn SecRandomCopyBytes(
            random: *const std::ffi::c_void,
            count: usize,
            bytes: *mut std::ffi::c_void,
        ) -> i32;
    }
    // SAFETY: the default secure RNG and a live writable buffer of this length.
    let status =
        unsafe { SecRandomCopyBytes(std::ptr::null(), bytes.len(), bytes.as_mut_ptr().cast()) };
    if status == 0 {
        Ok(())
    } else {
        Err(io::Error::other(
            "The system random generator is unavailable",
        ))
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn random(_: &mut [u8; 16]) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Private request folders require a supported system random generator",
    ))
}

#[cfg(test)]
mod tests;
