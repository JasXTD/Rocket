//! Commit-graph I/O stays in held directories, even if a pathname is replaced.
//! Unix uses *at calls; Windows pins directories against replacement/reparse edits.
use std::{
    ffi::OsStr,
    fs::File,
    io,
    path::{Component, Path},
};

pub(super) use platform::Directory;

fn leaf(name: &OsStr) -> io::Result<()> {
    let mut parts = Path::new(name).components();
    if matches!(parts.next(), Some(Component::Normal(_))) && parts.next().is_none() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Commit index entry must be one filename",
        ))
    }
}

fn regular(file: File) -> io::Result<File> {
    let metadata = file.metadata()?;
    if metadata.is_file() && !crate::local_files::redirected(&metadata) {
        Ok(file)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Commit index requires regular files, not links or reparse points",
        ))
    }
}

pub(super) fn optional<T>(result: io::Result<T>) -> io::Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::{
        fs::{self, OpenOptions},
        os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
        path::PathBuf,
    };

    pub(crate) struct Directory {
        path: PathBuf,
        _pins: Vec<File>,
    }
    impl Directory {
        pub(crate) fn open(path: &Path) -> io::Result<Self> {
            if !path.is_absolute() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Commit index directory must be absolute",
                ));
            }
            // Repository discovery supplies the canonical common Git directory.
            // Pin that boundary, without requiring read access to unrelated
            // ancestors such as the drive root or the user's home directory.
            Self::pin(path)
        }
        fn pin(path: &Path) -> io::Result<Self> {
            // Deny writes as well as deletion: FSCTL_SET_REPARSE_POINT requires
            // a write-capable handle. A directory must not turn into a junction
            // while a subsequent pathname-based Windows open is using it.
            let file = OpenOptions::new()
                .read(true)
                .share_mode(1)
                .custom_flags(0x0220_0000)
                .open(path)?;
            let metadata = file.metadata()?;
            if !metadata.is_dir() || crate::local_files::redirected(&metadata) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Refusing linked or reparse-point commit index directory",
                ));
            }
            Ok(Self {
                path: path.to_owned(),
                _pins: vec![file],
            })
        }
        pub(crate) fn child(&self, name: &OsStr) -> io::Result<Option<Self>> {
            leaf(name)?;
            let path = self.path.join(name);
            // Keep NotFound separate from a broken symlink/junction or any other
            // failure. The parent remains pinned throughout this open.
            if optional(fs::symlink_metadata(&path))?.is_none() {
                return Ok(None);
            }
            let mut child = Self::pin(&path)?;
            // Retain every ancestor from the original common-directory anchor.
            // Duplicating handles preserves their sharing restrictions.
            for pin in &self._pins {
                child._pins.push(pin.try_clone()?);
            }
            Ok(Some(child))
        }
        pub(crate) fn file(&self, name: &OsStr) -> io::Result<Option<File>> {
            leaf(name)?;
            optional(
                OpenOptions::new()
                    .read(true)
                    .custom_flags(0x0020_0000)
                    .open(self.path.join(name)),
            )?
            .map(regular)
            .transpose()
        }
        pub(crate) fn entries(
            &self,
        ) -> io::Result<impl Iterator<Item = io::Result<std::ffi::OsString>>> {
            Ok(fs::read_dir(&self.path)?.map(|entry| entry.map(|entry| entry.file_name())))
        }
        pub(crate) fn remove_file(&self, name: &OsStr) -> io::Result<()> {
            leaf(name)?;
            // DELETE | FILE_WRITE_ATTRIBUTES | GENERIC_READ. Hold the exact leaf
            // against writes/deletion; never chmod a pathname (or a hard-link peer).
            let Some(file) = optional(
                OpenOptions::new()
                    .access_mode(0x8001_0100)
                    .share_mode(1)
                    .custom_flags(0x0020_0000)
                    .open(self.path.join(name)),
            )?
            else {
                return Ok(());
            };
            let file = regular(file)?;
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn SetFileInformationByHandle(
                    handle: *mut std::ffi::c_void,
                    class: u32,
                    data: *const std::ffi::c_void,
                    size: u32,
                ) -> i32;
            }
            // FileDispositionInfoEx, DELETE | IGNORE_READONLY_ATTRIBUTE. This
            // deletes the held directory entry without changing shared attributes.
            let flags: u32 = 0x11;
            // SAFETY: live DELETE-capable handle and a correctly sized flags field.
            if unsafe {
                SetFileInformationByHandle(
                    file.as_raw_handle(),
                    21,
                    (&flags as *const u32).cast(),
                    4,
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
        pub(crate) fn remove_empty_child(&self, name: &OsStr) -> io::Result<()> {
            leaf(name)?;
            fs::remove_dir(self.path.join(name))
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod platform {
    use super::*;
    use std::{
        ffi::{CString, OsString, c_char, c_int, c_void},
        os::{
            fd::{AsRawFd, FromRawFd, IntoRawFd},
            unix::ffi::{OsStrExt, OsStringExt},
        },
        ptr::NonNull,
    };

    #[cfg(all(target_os = "linux", not(target_arch = "aarch64")))]
    const DIRECTORY: c_int = 0x1_0000;
    #[cfg(all(target_os = "linux", not(target_arch = "aarch64")))]
    const NOFOLLOW: c_int = 0x2_0000;
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    const DIRECTORY: c_int = 0x4000;
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    const NOFOLLOW: c_int = 0x8000;
    #[cfg(target_os = "linux")]
    const CLOEXEC: c_int = 0x8_0000;
    #[cfg(target_os = "linux")]
    const NONBLOCK: c_int = 0x800;
    #[cfg(target_os = "linux")]
    const REMOVEDIR: c_int = 0x200;
    #[cfg(target_os = "macos")]
    const DIRECTORY: c_int = 0x10_0000;
    #[cfg(target_os = "macos")]
    const NOFOLLOW: c_int = 0x100;
    #[cfg(target_os = "macos")]
    const CLOEXEC: c_int = 0x100_0000;
    #[cfg(target_os = "macos")]
    const NONBLOCK: c_int = 4;
    #[cfg(target_os = "macos")]
    const REMOVEDIR: c_int = 0x80;

    unsafe extern "C" {
        fn openat(fd: c_int, path: *const c_char, flags: c_int, ...) -> c_int;
        fn unlinkat(fd: c_int, path: *const c_char, flags: c_int) -> c_int;
        // Intel Darwin has two directory ABIs; opening and reading the stream
        // must both use the 64-bit-inode variant, as the native headers do.
        #[cfg_attr(
            all(target_os = "macos", target_arch = "x86_64"),
            link_name = "fdopendir$INODE64"
        )]
        fn fdopendir(fd: c_int) -> *mut c_void;
        #[cfg_attr(
            all(target_os = "macos", target_arch = "x86_64"),
            link_name = "readdir$INODE64"
        )]
        fn readdir(directory: *mut c_void) -> *mut c_void;
        fn closedir(directory: *mut c_void) -> c_int;
        #[cfg(target_os = "linux")]
        fn __errno_location() -> *mut c_int;
        #[cfg(target_os = "macos")]
        fn __error() -> *mut c_int;
    }
    fn name(name: &OsStr) -> io::Result<CString> {
        leaf(name)?;
        CString::new(name.as_bytes()).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "NUL in commit index filename")
        })
    }
    fn open_in(parent: &File, entry: &OsStr, flags: c_int) -> io::Result<File> {
        let name = name(entry)?;
        // SAFETY: held directory, live single-component name, no create flags.
        let fd = unsafe {
            openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                flags | NOFOLLOW | CLOEXEC | NONBLOCK,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the new descriptor is owned only by this File.
        Ok(unsafe { File::from_raw_fd(fd) })
    }
    pub(crate) struct Directory(File);
    impl Directory {
        pub(crate) fn open(path: &Path) -> io::Result<Self> {
            if !path.is_absolute() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Commit index directory must be absolute",
                ));
            }
            let mut current = Self(File::open("/")?);
            for component in path.components() {
                match component {
                    Component::RootDir => {}
                    Component::Normal(part) => {
                        current = Self(open_in(&current.0, part, DIRECTORY)?)
                    }
                    _ => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "Invalid commit index directory",
                        ));
                    }
                }
            }
            Ok(current)
        }
        pub(crate) fn child(&self, name: &OsStr) -> io::Result<Option<Self>> {
            optional(open_in(&self.0, name, DIRECTORY)).map(|file| file.map(Self))
        }
        pub(crate) fn file(&self, name: &OsStr) -> io::Result<Option<File>> {
            optional(open_in(&self.0, name, 0))?
                .map(regular)
                .transpose()
        }
        pub(crate) fn entries(&self) -> io::Result<Entries> {
            // Opening '.' gives this iterator an independent directory offset.
            // SAFETY: held descriptor and a constant name, read-only directory open.
            let fd = unsafe {
                openat(
                    self.0.as_raw_fd(),
                    c".".as_ptr(),
                    DIRECTORY | NOFOLLOW | CLOEXEC,
                )
            };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: newly owned descriptor, transferred to DIR only on success.
            let file = unsafe { File::from_raw_fd(fd) };
            // SAFETY: fd is a live, readable directory descriptor.
            let Some(directory) = NonNull::new(unsafe { fdopendir(fd) }) else {
                return Err(io::Error::last_os_error());
            };
            let _ = file.into_raw_fd();
            Ok(Entries {
                directory,
                finished: false,
            })
        }
        pub(crate) fn remove_file(&self, name: &OsStr) -> io::Result<()> {
            if self.file(name)?.is_none() {
                return Ok(());
            }
            self.unlink(name, 0)
        }
        fn unlink(&self, entry: &OsStr, flags: c_int) -> io::Result<()> {
            let entry = name(entry)?;
            // SAFETY: held directory and checked single filename. unlinkat never
            // follows the leaf, including a link swapped in after file validation.
            if unsafe { unlinkat(self.0.as_raw_fd(), entry.as_ptr(), flags) } == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        }
        pub(crate) fn remove_empty_child(&self, name: &OsStr) -> io::Result<()> {
            self.unlink(name, REMOVEDIR)
        }
    }

    pub(crate) struct Entries {
        directory: NonNull<c_void>,
        finished: bool,
    }
    impl Iterator for Entries {
        type Item = io::Result<OsString>;
        fn next(&mut self) -> Option<Self::Item> {
            if self.finished {
                return None;
            }
            // SAFETY: these functions return this thread's errno storage.
            #[cfg(target_os = "linux")]
            let errno = unsafe { __errno_location() };
            #[cfg(target_os = "macos")]
            let errno = unsafe { __error() };
            // SAFETY: owned DIR, used only synchronously; returned bytes remain
            // live until the next readdir. errno distinguishes EOF from failure.
            let entry = unsafe {
                *errno = 0;
                readdir(self.directory.as_ptr())
            }
            .cast::<u8>();
            if entry.is_null() {
                self.finished = true;
                let error = io::Error::last_os_error();
                return (error.raw_os_error() != Some(0)).then_some(Err(error));
            }
            // 64-bit Linux and Darwin dirent layouts (sys/dirent.h): two u64
            // fields, u16 record length; Darwin then has a u16 name length.
            #[cfg(target_os = "linux")]
            const NAME_OFFSET: usize = 19;
            #[cfg(target_os = "macos")]
            const NAME_OFFSET: usize = 21;
            // SAFETY: libc supplies a complete record with the fixed prefix.
            let length = unsafe { entry.add(16).cast::<u16>().read_unaligned() } as usize;
            if length <= NAME_OFFSET || length > 4096 {
                self.finished = true;
                return Some(Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Invalid commit index directory entry",
                )));
            }
            // SAFETY: length is the record size returned by libc; copy before the
            // next call. Do not make a reference to a larger, fixed-size dirent.
            let bytes =
                unsafe { std::slice::from_raw_parts(entry.add(NAME_OFFSET), length - NAME_OFFSET) };
            let Some(end) = bytes.iter().position(|byte| *byte == 0) else {
                self.finished = true;
                return Some(Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Unterminated commit index entry",
                )));
            };
            Some(Ok(OsString::from_vec(bytes[..end].to_vec())))
        }
    }
    impl Drop for Entries {
        fn drop(&mut self) {
            // SAFETY: this object exclusively owns DIR and its descriptor.
            unsafe {
                closedir(self.directory.as_ptr());
            }
        }
    }
}
