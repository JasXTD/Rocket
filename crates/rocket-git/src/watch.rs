//! Coalesced native directory notifications. No scans or Git commands while idle.
use crate::{Error, RepositoryInfo, Result};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

struct Shared {
    changed: AtomicBool,
    stop: AtomicBool,
    error: Mutex<Option<std::io::Error>>,
    metadata_generation: AtomicU64,
    failed: AtomicBool,
}

/// Watches this worktree and its Git/common directories, including linked worktrees.
/// Dropping it requests worker shutdown; handles are closed by their owning worker.
pub struct RepositoryWatch {
    shared: Arc<Shared>,
    #[cfg(windows)]
    roots: Option<platform::Roots>,
}

impl RepositoryWatch {
    /// Call on a worker: opening notification handles can perform filesystem I/O.
    pub fn new(info: &RepositoryInfo, wake: Arc<dyn Fn() + Send + Sync>) -> Result<Self> {
        let shared = Arc::new(Shared {
            changed: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            error: Mutex::new(None),
            metadata_generation: AtomicU64::new(0),
            failed: AtomicBool::new(false),
        });
        #[cfg(windows)]
        let roots = platform::Roots::capture(info).ok();
        platform::start(info, shared.clone(), wake)?;
        Ok(Self {
            shared,
            #[cfg(windows)]
            roots,
        })
    }
    /// Nonblocking, bounded polling. Multiple filesystem events become one refresh.
    pub fn take_changed(&self) -> Result<bool> {
        if let Some(error) = self.shared.error.lock().unwrap().take() {
            return Err(Error::Io {
                action: "watch repository changes",
                source: error,
            });
        }
        Ok(self.shared.changed.swap(false, Ordering::AcqRel))
    }
    /// A worktree event does not advance this generation. Git/common directory
    /// events do; failures permanently disable selective reuse for this watcher.
    pub fn metadata_generation(&self) -> Option<u64> {
        let generation = self.shared.metadata_generation.load(Ordering::Acquire);
        (!self.shared.failed.load(Ordering::Acquire)).then_some(generation)
    }
    /// Worker-only identity check. A renamed/replaced directory can leave a
    /// native handle watching the old object even when the path is unchanged.
    /// Filesystems without stable IDs still get notifications and full refreshes.
    pub fn is_current(&self) -> Result<bool> {
        #[cfg(windows)]
        {
            self.roots
                .as_ref()
                .map_or(Ok(false), platform::Roots::is_current)
        }
        #[cfg(not(windows))]
        {
            Ok(false)
        }
    }
}
impl Drop for RepositoryWatch {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::{
        io,
        os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle},
        path::{Path, PathBuf},
    };

    // HANDLE is pointer-sized. These opaque values never become Rust references.
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn FindFirstChangeNotificationW(path: *const u16, subtree: i32, filter: u32) -> isize;
        fn FindNextChangeNotification(handle: isize) -> i32;
        fn FindCloseChangeNotification(handle: isize) -> i32;
        fn WaitForMultipleObjects(count: u32, handles: *const isize, all: i32, ms: u32) -> u32;
        fn GetFileInformationByHandleEx(
            handle: isize,
            class: i32,
            info: *mut std::ffi::c_void,
            size: u32,
        ) -> i32;
    }
    #[derive(Default, PartialEq, Eq)]
    #[repr(C)]
    struct DirectoryId {
        volume: u64,
        file: [u8; 16],
    }
    pub(super) struct Roots(Vec<(PathBuf, DirectoryId)>);
    impl Roots {
        pub(super) fn capture(info: &RepositoryInfo) -> Result<Self> {
            let mut paths = vec![info.git_dir.clone(), info.common_dir.clone()];
            paths.extend(info.worktree.clone());
            paths.sort();
            paths.dedup();
            paths
                .into_iter()
                .map(|path| Ok((path.clone(), directory_id(&path)?)))
                .collect::<Result<Vec<_>>>()
                .map(Self)
        }
        pub(super) fn is_current(&self) -> Result<bool> {
            for (path, id) in &self.0 {
                if directory_id(path)? != *id {
                    return Ok(false);
                }
            }
            Ok(true)
        }
    }
    fn directory_id(path: &Path) -> Result<DirectoryId> {
        // Metadata access only; share delete as well as reads/writes so watching
        // a repository never prevents a worktree move or directory replacement.
        let directory = std::fs::OpenOptions::new()
            .access_mode(0)
            .share_mode(7)
            .custom_flags(0x02000000)
            .open(path)
            .map_err(|error| Error::io("open watched directory identity", error))?;
        let mut id = DirectoryId::default();
        // SAFETY: FileIdInfo (18) writes exactly the repr(C) FILE_ID_INFO layout;
        // the live, owned handle and output buffer remain valid for the call.
        let ok = unsafe {
            GetFileInformationByHandleEx(
                directory.as_raw_handle() as isize,
                18,
                (&mut id as *mut DirectoryId).cast(),
                std::mem::size_of::<DirectoryId>() as u32,
            )
        };
        if ok == 0 {
            return Err(Error::io(
                "read watched directory identity",
                io::Error::last_os_error(),
            ));
        }
        Ok(id)
    }
    struct Notification(isize);
    impl Drop for Notification {
        fn drop(&mut self) {
            // SAFETY: this is the sole owner; the worker is no longer waiting on it.
            unsafe {
                FindCloseChangeNotification(self.0);
            }
        }
    }
    pub(super) fn start(
        info: &RepositoryInfo,
        shared: Arc<Shared>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<()> {
        // Metadata handles come first so a simultaneous .git/worktree signal
        // is classified conservatively. Keep nested metadata watches: a parent
        // worktree watch alone cannot distinguish editing a file from a ref.
        let mut roots = vec![
            (info.git_dir.clone(), true),
            (info.common_dir.clone(), true),
        ];
        if let Some(worktree) = &info.worktree {
            roots.push((worktree.clone(), false));
        }
        let (ready, initialized) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("rocket-repository-watch".into())
            .spawn(move || {
                // Create and rearm on this long-lived thread. Windows cancels
                // pending directory I/O when its initiating thread exits.
                let open = || -> Result<Vec<(Notification, bool)>> {
                    let mut watched = Vec::new();
                    let mut handles = Vec::new();
                    for (root, metadata) in roots {
                        if watched.contains(&root) {
                            continue;
                        }
                        let wide: Vec<u16> =
                            root.as_os_str().encode_wide().chain(Some(0)).collect();
                        if wide[..wide.len() - 1].contains(&0) {
                            return Err(Error::InvalidInput("Invalid watch path".into()));
                        }
                        // SAFETY: wide is NUL terminated and lives through the call. Notify on
                        // names, attributes, size and last-write, never last-access (read loops).
                        let handle =
                            unsafe { FindFirstChangeNotificationW(wide.as_ptr(), 1, 0x1f) };
                        if handle == -1 || handle == 0 {
                            return Err(Error::Io {
                                action: "start repository watch",
                                source: io::Error::last_os_error(),
                            });
                        }
                        handles.push((Notification(handle), metadata));
                        watched.push(root);
                    }
                    Ok(handles)
                };
                let handles = match open() {
                    Ok(handles) => {
                        if ready.send(Ok(())).is_err() {
                            return;
                        }
                        handles
                    }
                    Err(error) => {
                        let _ = ready.send(Err(error));
                        return;
                    }
                };
                let raw: Vec<_> = handles.iter().map(|(h, _)| h.0).collect();
                while !shared.stop.load(Ordering::Acquire) {
                    // SAFETY: 1..=3 distinct live handles, owned by this worker. No
                    // other thread closes them. The timeout bounds Drop cleanup.
                    let result =
                        unsafe { WaitForMultipleObjects(raw.len() as u32, raw.as_ptr(), 0, 100) };
                    if result == 258 {
                        continue;
                    } // WAIT_TIMEOUT: wait again, do not rearm.
                    if (result as usize) < raw.len() {
                        // SAFETY: rearm only the handle signalled by this wait.
                        if unsafe { FindNextChangeNotification(raw[result as usize]) } != 0 {
                            if handles[result as usize].1 {
                                shared.metadata_generation.fetch_add(1, Ordering::AcqRel);
                            }
                            if !shared.changed.swap(true, Ordering::AcqRel) {
                                wake();
                            }
                            continue;
                        }
                    }
                    shared.failed.store(true, Ordering::Release);
                    *shared.error.lock().unwrap() = Some(io::Error::last_os_error());
                    wake();
                    break;
                }
            })
            .map_err(|source| Error::Io {
                action: "start repository watch worker",
                source,
            })?;
        initialized
            .recv_timeout(std::time::Duration::from_secs(5))
            .map_err(|_| Error::Io {
                action: "initialize repository watch",
                source: io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Watcher initialization did not complete",
                ),
            })?
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;
    pub(super) fn start(
        _: &RepositoryInfo,
        _: Arc<Shared>,
        _: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<()> {
        Err(Error::Unsupported(
            "Automatic refresh currently requires Windows; use F5 to refresh".into(),
        ))
    }
}
