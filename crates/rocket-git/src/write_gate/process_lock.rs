//! Cross-process ownership for cooperating Rocket clients. Never remove Git locks.
//! Keep this mutex name/identity protocol stable across Rocket versions.
#[cfg(windows)]
pub(super) use windows::ProcessLock;

#[cfg(windows)]
mod windows {
    use crate::{Error, Result, local_files};
    use std::{
        ffi::c_void,
        fs::OpenOptions,
        io,
        marker::PhantomData,
        os::windows::{
            fs::OpenOptionsExt,
            io::{AsRawHandle, FromRawHandle, OwnedHandle},
        },
        path::Path,
        rc::Rc,
    };

    #[repr(C)]
    struct FileInfo {
        attributes: u32,
        creation: [u32; 2],
        access: [u32; 2],
        write: [u32; 2],
        volume: u32,
        size_high: u32,
        size_low: u32,
        links: u32,
        index_high: u32,
        index_low: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandle(handle: *mut c_void, info: *mut FileInfo) -> i32;
        fn CreateMutexW(attributes: *const c_void, owner: i32, name: *const u16) -> *mut c_void;
        fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
        fn ReleaseMutex(handle: *mut c_void) -> i32;
    }

    pub(crate) struct ProcessLock {
        handle: OwnedHandle,
        // Windows mutex ownership belongs to the acquiring thread. Do not permit
        // moving or sharing this guard across threads, even though HANDLE is Send.
        _thread: PhantomData<Rc<()>>,
    }
    impl ProcessLock {
        pub(crate) fn acquire(common_dir: &Path) -> Result<Self> {
            let directory = OpenOptions::new()
                .read(true)
                .share_mode(1 | 2 | 4)
                .custom_flags(0x0220_0000) // BACKUP_SEMANTICS | OPEN_REPARSE_POINT
                .open(common_dir)
                .map_err(|e| Error::io("open repository write ownership", e))?;
            let meta = directory
                .metadata()
                .map_err(|e| Error::io("inspect repository write ownership", e))?;
            if !meta.is_dir() || local_files::redirected(&meta) {
                return Err(Error::io(
                    "identify repository write ownership",
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Repository metadata directory changed; refresh and retry",
                    ),
                ));
            }
            let mut info = std::mem::MaybeUninit::<FileInfo>::uninit();
            // SAFETY: directory owns a live handle; Win32 initializes this exact
            // BY_HANDLE_FILE_INFORMATION layout on success.
            if unsafe { GetFileInformationByHandle(directory.as_raw_handle(), info.as_mut_ptr()) }
                == 0
            {
                return Err(Error::io(
                    "identify repository write ownership",
                    io::Error::last_os_error(),
                ));
            }
            // SAFETY: successful Win32 call above initialized all fields.
            let info = unsafe { info.assume_init() };
            if info.index_high == 0 && info.index_low == 0 {
                return Err(Error::io(
                    "identify repository write ownership",
                    io::Error::new(
                        io::ErrorKind::Unsupported,
                        "Filesystem did not provide a stable repository identity",
                    ),
                ));
            }
            // Do not retain the directory handle: on Windows even SHARE_DELETE
            // prevents moving an ancestor checkout containing an embedded .git.
            // The mutex retains ownership across that move, keyed by file identity.
            drop(directory);
            // File identity handles case variants, junctions, alternate drive names
            // and linked worktrees. Global covers Windows sessions on this machine.
            // No repository path, username, token or command payload enters the name.
            let name: Vec<u16> = format!(
                "Global\\Rocket.RepositoryWrite.v1.{:08x}.{:08x}{:08x}",
                info.volume, info.index_high, info.index_low
            )
            .encode_utf16()
            .chain([0])
            .collect();
            // SAFETY: NUL-terminated live name; default ACL; non-inheritable handle.
            let raw = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
            if raw.is_null() {
                // Access denial or an unsupported OS/filesystem must fail closed.
                return Err(Error::io(
                    "open repository write ownership",
                    io::Error::last_os_error(),
                ));
            }
            // SAFETY: CreateMutex returned a new owned, valid handle.
            let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
            // SAFETY: owned mutex handle; zero timeout never waits on another process.
            match unsafe { WaitForSingleObject(handle.as_raw_handle(), 0) } {
                0 | 0x80 => Ok(Self {
                    handle,
                    _thread: PhantomData,
                }),
                // WAIT_ABANDONED also grants ownership. Existing Git state/preflight
                // checks remain in force; no stale Git locks are deleted or retried.
                258 => Err(Error::RepositoryBusy),
                _ => Err(Error::io(
                    "acquire repository write ownership",
                    io::Error::last_os_error(),
                )),
            }
        }
    }
    impl Drop for ProcessLock {
        fn drop(&mut self) {
            // SAFETY: only a successful wait constructs this !Send/!Sync guard;
            // therefore this same thread owns the mutex until this release.
            let released = unsafe { ReleaseMutex(self.handle.as_raw_handle()) };
            debug_assert_ne!(
                released, 0,
                "repository ownership released on the wrong thread"
            );
        }
    }
}

#[cfg(not(windows))]
pub(crate) struct ProcessLock {
    _file: std::fs::File,
}
#[cfg(not(windows))]
impl ProcessLock {
    pub(crate) fn acquire(common_dir: &std::path::Path) -> crate::Result<Self> {
        use crate::Error;
        use std::{
            fs::{self, OpenOptions, TryLockError},
            io,
        };
        let path = common_dir.join("rocket-write.lock");
        match fs::symlink_metadata(&path) {
            Ok(m) if !m.is_file() || m.file_type().is_symlink() => {
                return Err(Error::io(
                    "open repository write ownership",
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Repository ownership file must be a regular file",
                    ),
                ));
            }
            Err(e) if e.kind() != io::ErrorKind::NotFound => {
                return Err(Error::io("inspect repository write ownership", e));
            }
            _ => {}
        }
        // The file is persistent and never truncated/unlinked. Closing the
        // non-inheritable handle releases the OS lock, including on process exit.
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|e| Error::io("open repository write ownership", e))?;
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(TryLockError::WouldBlock) => Err(Error::RepositoryBusy),
            Err(TryLockError::Error(e)) => Err(Error::io("acquire repository write ownership", e)),
        }
    }
}
