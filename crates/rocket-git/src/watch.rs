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
    /// Linux: the inotify instance this watch shares with others of the repository.
    #[cfg(target_os = "linux")]
    instance: Arc<platform::Instance>,
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
        #[cfg(not(target_os = "linux"))]
        platform::start(info, shared.clone(), wake)?;
        #[cfg(target_os = "linux")]
        let instance = platform::subscribe(info, shared.clone(), wake)?;
        Ok(Self {
            shared,
            #[cfg(windows)]
            roots,
            #[cfg(target_os = "linux")]
            instance,
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
        #[cfg(target_os = "linux")]
        {
            self.instance.is_current()
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        {
            Ok(false)
        }
    }
}
impl Drop for RepositoryWatch {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        #[cfg(target_os = "linux")]
        self.instance.unsubscribe(&self.shared);
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

/// inotify watches single folders, not trees. Every folder below each root gets its
/// own watch: found by walking the roots once, then followed as folders are created,
/// moved and removed. A queue overflow walks everything again.
///
/// The kernel limits watches per user (fs.inotify.max_user_watches). Rocket keeps to
/// half of that across its repositories, so editors and other programs keep theirs,
/// and refuses past it with the reason. Every refresh arms a new watch before the old
/// one is dropped, so all watches of one repository share one inotify instance: a
/// large repository is walked once and its folders are never watched twice.
#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use std::{
        collections::{HashMap, HashSet, VecDeque},
        ffi::{CString, OsStr, c_char, c_int, c_ulong},
        fs::{self, File},
        io::{self, PipeReader, PipeWriter, Read},
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::{ffi::OsStrExt, fs::MetadataExt},
        },
        path::{Path, PathBuf},
        sync::{Weak, atomic::AtomicUsize, mpsc},
        time::{Duration, Instant},
    };

    unsafe extern "C" {
        fn inotify_init1(flags: c_int) -> c_int;
        fn inotify_add_watch(fd: c_int, path: *const c_char, mask: u32) -> c_int;
        fn inotify_rm_watch(fd: c_int, watch: c_int) -> c_int;
        fn poll(fds: *mut PollFd, count: c_ulong, timeout: c_int) -> c_int;
    }
    #[repr(C)]
    struct PollFd {
        fd: c_int,
        events: i16,
        revents: i16,
    }
    const POLLIN: i16 = 0x1;
    // O_CLOEXEC and O_NONBLOCK in the generic Linux ABI (x86, Arm, RISC-V, PowerPC,
    // s390x, LoongArch). Elsewhere inotify_init1 refuses them and the watch reports it.
    const IN_CLOEXEC: c_int = 0o2_000_000;
    const IN_NONBLOCK: c_int = 0o4_000;
    const IN_MODIFY: u32 = 0x2;
    const IN_ATTRIB: u32 = 0x4;
    const IN_MOVED_FROM: u32 = 0x40;
    const IN_MOVED_TO: u32 = 0x80;
    const IN_CREATE: u32 = 0x100;
    const IN_DELETE: u32 = 0x200;
    const IN_DELETE_SELF: u32 = 0x400;
    const IN_MOVE_SELF: u32 = 0x800;
    const IN_Q_OVERFLOW: u32 = 0x4000;
    const IN_IGNORED: u32 = 0x8000;
    const IN_ONLYDIR: u32 = 0x0100_0000;
    const IN_DONT_FOLLOW: u32 = 0x0200_0000;
    const IN_EXCL_UNLINK: u32 = 0x0400_0000;
    const IN_ISDIR: u32 = 0x4000_0000;
    /// Writes, metadata, names and folders, as Windows reports them; never reads (no
    /// IN_ACCESS or IN_OPEN), so Rocket's own reads cannot trigger refreshes. Not
    /// IN_CLOSE_WRITE either: it also fires when a file opened for writing closes
    /// unwritten, as Rocket's write lock does after every write, which Windows never
    /// reports. Unlinked files stay quiet.
    const CHANGES: u32 = IN_MODIFY
        | IN_ATTRIB
        | IN_MOVED_FROM
        | IN_MOVED_TO
        | IN_CREATE
        | IN_DELETE
        | IN_DELETE_SELF
        | IN_MOVE_SELF
        | IN_ONLYDIR
        | IN_EXCL_UNLINK;
    /// The longest a new instance may spend walking before reporting it.
    const FIRST_WALK: Duration = Duration::from_secs(5);
    /// Watches held by all of this process's instances.
    static WATCHED: AtomicUsize = AtomicUsize::new(0);
    static INSTANCES: Mutex<Vec<Weak<Instance>>> = Mutex::new(Vec::new());

    type Identity = (u64, u64);
    fn identity(path: &Path) -> Result<Identity> {
        let meta = fs::metadata(path)
            .map_err(|error| Error::io("read watched directory identity", error))?;
        Ok((meta.dev(), meta.ino()))
    }

    /// A kernel limit, read when an instance starts so a raised limit applies at once.
    fn limit(name: &str, default: usize) -> usize {
        fs::read_to_string(format!("/proc/sys/fs/inotify/{name}"))
            .ok()
            .and_then(|text| text.trim().parse().ok())
            .unwrap_or(default)
    }

    struct Subscriber {
        shared: Arc<Shared>,
        wake: Arc<dyn Fn() + Send + Sync>,
    }

    pub(super) struct Instance {
        /// Watched roots, metadata first; the identity of a shared instance.
        roots: Vec<(PathBuf, bool)>,
        identities: Vec<(PathBuf, Identity)>,
        /// Locked while events are read and delivered, and while a watch joins, so a
        /// joining watch never receives an event that happened before it was armed.
        /// Always taken before `subscribers`.
        watcher: Mutex<Watcher>,
        /// Its own lock, so dropping a watch never waits for a walk.
        subscribers: Mutex<Vec<Subscriber>>,
        failed: AtomicBool,
        /// Dropping the instance closes this and stops its worker.
        _stop: PipeWriter,
    }

    impl Instance {
        pub(super) fn is_current(&self) -> Result<bool> {
            for (path, id) in &self.identities {
                if identity(path)? != *id {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        pub(super) fn unsubscribe(&self, shared: &Arc<Shared>) {
            self.subscribers
                .lock()
                .unwrap()
                .retain(|subscriber| !Arc::ptr_eq(&subscriber.shared, shared));
        }
        /// Read and deliver the queued events, at most `reads` buffers of them, with
        /// `watcher` locked. Returns once the queue is empty or the reads are used up.
        fn drain(&self, watcher: &mut Watcher, reads: usize) -> io::Result<()> {
            for _ in 0..reads {
                let read = loop {
                    match (&*watcher.inotify).read(&mut watcher.buffer) {
                        Ok(read) => break read,
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                        Err(error) => return Err(error),
                    }
                };
                let events = std::mem::take(&mut watcher.buffer);
                let handled = watcher.handle(&events[..read]);
                watcher.buffer = events;
                if let Some(metadata) = handled? {
                    self.notify(metadata);
                }
            }
            Ok(())
        }
        fn notify(&self, metadata: bool) {
            let mut woken = Vec::new();
            for subscriber in self.subscribers.lock().unwrap().iter() {
                if metadata {
                    subscriber
                        .shared
                        .metadata_generation
                        .fetch_add(1, Ordering::AcqRel);
                }
                if !subscriber.shared.changed.swap(true, Ordering::AcqRel) {
                    woken.push(subscriber.wake.clone());
                }
            }
            woken.iter().for_each(|wake| wake());
        }
        /// Called with `watcher` locked, as a join checks `failed` under that lock.
        fn fail(&self, error: &io::Error) {
            let mut woken = Vec::new();
            let subscribers = self.subscribers.lock().unwrap();
            self.failed.store(true, Ordering::Release);
            for subscriber in subscribers.iter() {
                subscriber.shared.failed.store(true, Ordering::Release);
                *subscriber.shared.error.lock().unwrap() =
                    Some(io::Error::new(error.kind(), error.to_string()));
                woken.push(subscriber.wake.clone());
            }
            drop(subscribers);
            woken.iter().for_each(|wake| wake());
        }
        /// Add a watch. Events already queued happened before it was armed: they go
        /// to the existing watches first, as a new Windows handle never sees them. A
        /// queue still filling after many reads may hand the newest to it as well,
        /// which costs at most one extra refresh.
        fn join(&self, subscriber: Subscriber) -> std::result::Result<(), Subscriber> {
            let mut watcher = self.watcher.lock().unwrap();
            if self.failed.load(Ordering::Acquire) {
                return Err(subscriber);
            }
            if let Err(error) = self.drain(&mut watcher, 64) {
                self.fail(&error);
                return Err(subscriber);
            }
            self.subscribers.lock().unwrap().push(subscriber);
            Ok(())
        }
    }

    /// Join the repository's current instance, or start one and walk its folders.
    pub(super) fn subscribe(
        info: &RepositoryInfo,
        shared: Arc<Shared>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Arc<Instance>> {
        // Metadata roots come first, so a folder in both trees counts as metadata.
        let mut roots = vec![
            (info.git_dir.clone(), true),
            (info.common_dir.clone(), true),
        ];
        if let Some(worktree) = &info.worktree {
            roots.push((worktree.clone(), false));
        }
        let mut seen = HashSet::new();
        roots.retain(|(path, _)| seen.insert(path.clone()));
        let mut subscriber = Subscriber { shared, wake };
        let existing = {
            let mut instances = INSTANCES.lock().unwrap();
            instances.retain(|instance| instance.strong_count() > 0);
            instances.iter().filter_map(Weak::upgrade).find(|instance| {
                instance.roots == roots
                    && !instance.failed.load(Ordering::Acquire)
                    && instance.is_current().unwrap_or(false)
            })
        };
        if let Some(instance) = existing {
            match instance.join(subscriber) {
                Ok(()) => return Ok(instance),
                Err(refused) => subscriber = refused, // It failed meanwhile: start anew.
            }
        }
        let instance = start(roots, subscriber)?;
        INSTANCES.lock().unwrap().push(Arc::downgrade(&instance));
        Ok(instance)
    }

    /// A new instance, walked on its own worker, with its first subscriber in place
    /// before any event is read. The worker keeps reading until the instance drops.
    fn start(roots: Vec<(PathBuf, bool)>, subscriber: Subscriber) -> Result<Arc<Instance>> {
        let identities = roots
            .iter()
            .map(|(path, _)| Ok((path.clone(), identity(path)?)))
            .collect::<Result<Vec<_>>>()?;
        let (ready, initialized) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("rocket-repository-watch".into())
            .spawn(move || {
                let started = io::pipe().and_then(|(stop, stop_writer)| {
                    let max_watches = limit("max_user_watches", 8192);
                    let watcher = Watcher::open(roots.clone(), max_watches, max_watches / 2)?;
                    Ok((stop, stop_writer, watcher))
                });
                let (stop, stop_writer, watcher) = match started {
                    Ok(started) => started,
                    Err(error) => {
                        let _ = ready.send(Err(error));
                        return;
                    }
                };
                // Polled here, so the descriptor stays open until this worker ends.
                let inotify = watcher.inotify.clone();
                let instance = Arc::new(Instance {
                    roots,
                    identities,
                    watcher: Mutex::new(watcher),
                    subscribers: Mutex::new(vec![subscriber]),
                    failed: AtomicBool::new(false),
                    _stop: stop_writer,
                });
                let alive = Arc::downgrade(&instance);
                // A caller that stopped waiting drops the instance, which ends this worker.
                if ready.send(Ok(instance)).is_ok() {
                    run(&inotify, &stop, &alive);
                }
            })
            .map_err(|source| Error::Io { action: "start repository watch worker", source })?;
        initialized
            .recv_timeout(FIRST_WALK + Duration::from_secs(1))
            .map_err(|_| Error::Io {
                action: "initialize repository watch",
                source: io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Watcher initialization did not complete",
                ),
            })?
            .map_err(|source| Error::Io { action: "start repository watch", source })
    }

    /// The worker: wait for events or the instance's end, then deliver what is queued.
    fn run(inotify: &File, stop: &PipeReader, alive: &Weak<Instance>) {
        loop {
            let mut fds = [
                PollFd { fd: inotify.as_raw_fd(), events: POLLIN, revents: 0 },
                PollFd { fd: stop.as_raw_fd(), events: POLLIN, revents: 0 },
            ];
            // SAFETY: two initialized entries owned by this frame, both descriptors kept
            // open by this worker; no timeout, as closing the stop pipe ends the wait.
            let polled = unsafe { poll(fds.as_mut_ptr(), 2, -1) };
            // Read errno before anything else can change it.
            let failed = (polled < 0).then(io::Error::last_os_error);
            if fds[1].revents != 0 {
                return; // The instance was dropped.
            }
            if failed.as_ref().is_some_and(|e| e.kind() == io::ErrorKind::Interrupted) {
                continue;
            }
            let Some(instance) = alive.upgrade() else {
                return;
            };
            let mut watcher = instance.watcher.lock().unwrap();
            let result = match failed {
                Some(error) => Err(error),
                // One buffer per wake: a join waiting for the lock gets its turn.
                None => instance.drain(&mut watcher, 1),
            };
            if let Err(error) = result {
                return instance.fail(&error);
            }
        }
    }

    struct Folder {
        path: PathBuf,
        metadata: bool,
    }

    /// The inotify descriptor and what each of its watch descriptors is.
    struct Watcher {
        inotify: Arc<File>,
        folders: HashMap<c_int, Folder>,
        roots: Vec<(PathBuf, bool)>,
        max_watches: usize,
        share: usize,
        deadline: Option<Instant>,
        buffer: Vec<u8>,
    }

    impl Drop for Watcher {
        fn drop(&mut self) {
            // Closing the descriptor releases every watch at once.
            WATCHED.fetch_sub(self.folders.len(), Ordering::AcqRel);
        }
    }

    /// EMFILE from inotify_init1: the per-user instance limit (or, rarely, this
    /// process's descriptor limit).
    fn instance_limit(error: io::Error) -> io::Error {
        if error.raw_os_error() != Some(24) {
            return error;
        }
        io::Error::other(format!(
            "the Linux limit on folder watchers is reached (fs.inotify.max_user_instances is {})",
            limit("max_user_instances", 128)
        ))
    }

    /// ENOSPC from inotify_add_watch: the per-user watch limit, which other programs'
    /// watches count against too.
    fn watch_limit(error: io::Error, max_watches: usize) -> io::Error {
        if error.raw_os_error() != Some(28) {
            return error;
        }
        io::Error::other(format!(
            "the Linux limit on watched folders is reached \
             (fs.inotify.max_user_watches is {max_watches}, shared with other programs)"
        ))
    }

    impl Watcher {
        /// Open an instance and walk `roots`, holding at most `share` watches across
        /// this process.
        fn open(roots: Vec<(PathBuf, bool)>, max_watches: usize, share: usize) -> io::Result<Self> {
            // SAFETY: no pointers; the result is a new descriptor or -1.
            let fd = unsafe { inotify_init1(IN_CLOEXEC | IN_NONBLOCK) };
            if fd < 0 {
                return Err(instance_limit(io::Error::last_os_error()));
            }
            let mut watcher = Self {
                // SAFETY: a new descriptor that nothing else owns.
                inotify: Arc::new(unsafe { File::from_raw_fd(fd) }),
                folders: HashMap::new(),
                roots: Vec::new(),
                max_watches,
                share,
                deadline: Some(Instant::now() + FIRST_WALK),
                // Holds at least one event with the longest name (16 + 256 bytes).
                buffer: vec![0; 64 * 1024],
            };
            watcher.walk(&roots, true)?;
            watcher.roots = roots;
            watcher.deadline = None;
            Ok(watcher)
        }

        /// Watch `path`, or find the watch it already has. `root` paths are Git's own
        /// canonical spelling; below them, links are never followed.
        fn watch(&mut self, path: &Path, metadata: bool, root: bool) -> io::Result<c_int> {
            let text = CString::new(path.as_os_str().as_bytes())
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid watch path"))?;
            let mask = if root { CHANGES } else { CHANGES | IN_DONT_FOLLOW };
            // SAFETY: a live descriptor and a NUL-terminated path for the call.
            let wd = unsafe { inotify_add_watch(self.inotify.as_raw_fd(), text.as_ptr(), mask) };
            if wd < 0 {
                return Err(watch_limit(io::Error::last_os_error(), self.max_watches));
            }
            if !self.folders.contains_key(&wd) {
                if WATCHED.fetch_add(1, Ordering::AcqRel) >= self.share {
                    WATCHED.fetch_sub(1, Ordering::AcqRel);
                    // SAFETY: a watch this instance just added and does not track.
                    unsafe { inotify_rm_watch(self.inotify.as_raw_fd(), wd) };
                    return Err(io::Error::other(format!(
                        "this repository has too many folders to watch: Rocket watches at \
                         most {} across open repositories, half of \
                         fs.inotify.max_user_watches ({})",
                        self.share, self.max_watches
                    )));
                }
                let path = path.to_path_buf();
                self.folders.insert(wd, Folder { path, metadata });
            }
            Ok(wd)
        }

        fn unwatch(&mut self, wd: c_int) {
            if self.folders.remove(&wd).is_some() {
                WATCHED.fetch_sub(1, Ordering::AcqRel);
                // SAFETY: a live descriptor; a watch the kernel already dropped only
                // makes this call fail, which changes nothing.
                unsafe { inotify_rm_watch(self.inotify.as_raw_fd(), wd) };
            }
        }

        /// Watch every folder below `starts`, each a root or a folder that appeared.
        /// Folders already watched get their current path, so moves are followed.
        /// Returns the watches this walk reached.
        fn walk(&mut self, starts: &[(PathBuf, bool)], roots: bool) -> io::Result<HashSet<c_int>> {
            let mut reached = HashSet::new();
            for (start, metadata) in starts {
                let mut pending = VecDeque::from([start.clone()]);
                while let Some(path) = pending.pop_front() {
                    if reached.len() % 256 == 0
                        && self.deadline.is_some_and(|deadline| Instant::now() > deadline)
                    {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            format!(
                                "watching this repository's folders took longer than {} seconds",
                                FIRST_WALK.as_secs()
                            ),
                        ));
                    }
                    let top = roots && path == *start;
                    let wd = match self.watch(&path, *metadata, top) {
                        Ok(wd) => wd,
                        // Below a root, a folder can vanish or be unreadable (Git cannot
                        // read it either); skip it rather than fail the repository.
                        Err(error) if !top && skippable(&error) => continue,
                        Err(error) => return Err(error),
                    };
                    // Reached twice in one walk, as `.git` from the worktree: keep
                    // the first (metadata) classification.
                    if !reached.insert(wd) {
                        continue;
                    }
                    if let Some(folder) = self.folders.get_mut(&wd) {
                        folder.path.clone_from(&path);
                        folder.metadata = *metadata;
                    }
                    let entries = match fs::read_dir(&path) {
                        Ok(entries) => entries,
                        Err(error) if !top && skippable(&error) => continue,
                        Err(error) => return Err(error),
                    };
                    for entry in entries.flatten() {
                        // The entry's own type: a link to a folder is not walked.
                        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                            pending.push_back(entry.path());
                        }
                    }
                }
            }
            Ok(reached)
        }

        /// Apply one read of events. Returns whether anything changed and whether
        /// that includes Git metadata; folders are watched before it is reported, so
        /// a refresh it starts cannot miss files created in them.
        fn handle(&mut self, events: &[u8]) -> io::Result<Option<bool>> {
            let (mut changed, mut metadata, mut overflow) = (false, false, false);
            let (mut arrived, mut departed) = (Vec::new(), Vec::new());
            let mut offset = 0;
            // struct inotify_event: wd, mask, cookie, len, then len bytes of name.
            while let Some(header) = events.get(offset..offset + 16) {
                let field = |at: usize| u32::from_ne_bytes(header[at..at + 4].try_into().unwrap());
                let (wd, mask, length) = (field(0) as c_int, field(4), field(12) as usize);
                let name = events.get(offset + 16..offset + 16 + length).unwrap_or_default();
                let name = &name[..name.iter().position(|b| *b == 0).unwrap_or(name.len())];
                offset += 16 + length;
                if mask & IN_Q_OVERFLOW != 0 {
                    (changed, metadata, overflow) = (true, true, true);
                    continue;
                }
                if mask & IN_IGNORED != 0 {
                    // The kernel dropped this watch: its folder was removed or unmounted.
                    if self.folders.remove(&wd).is_some() {
                        WATCHED.fetch_sub(1, Ordering::AcqRel);
                    }
                    changed = true;
                    continue;
                }
                changed = true;
                let Some(folder) = self.folders.get(&wd) else {
                    metadata = true; // A watch no longer known: classify conservatively.
                    continue;
                };
                metadata |= folder.metadata;
                if mask & IN_ISDIR != 0 && !name.is_empty() {
                    let path = folder.path.join(OsStr::from_bytes(name));
                    if mask & (IN_CREATE | IN_MOVED_TO) != 0 {
                        arrived.push((path, folder.metadata));
                    } else if mask & IN_MOVED_FROM != 0 {
                        departed.push(path);
                    }
                }
            }
            if overflow {
                // Events were lost, folder creations among them: walk everything again
                // and let go of watches the walk no longer reaches.
                let roots = self.roots.clone();
                let reached = self.walk(&roots, true)?;
                let stale: Vec<_> =
                    self.folders.keys().filter(|wd| !reached.contains(wd)).copied().collect();
                stale.into_iter().for_each(|wd| self.unwatch(wd));
            } else {
                // Let go of folders moved away first: one moved within the trees is
                // also among the arrivals and is watched again at its new path, and a
                // new folder may already have taken the old name.
                for path in departed {
                    let stale: Vec<_> = self
                        .folders
                        .iter()
                        .filter(|(_, folder)| folder.path.starts_with(&path))
                        .map(|(wd, _)| *wd)
                        .collect();
                    stale.into_iter().for_each(|wd| self.unwatch(wd));
                }
                self.walk(&arrived, false)?;
            }
            Ok(changed.then_some(metadata))
        }
    }

    /// A folder below a root that vanished, is no folder now (a link put in its place
    /// fails IN_ONLYDIR as one), or cannot be read.
    fn skippable(error: &io::Error) -> bool {
        matches!(
            error.kind(),
            io::ErrorKind::NotFound
                | io::ErrorKind::NotADirectory
                | io::ErrorKind::PermissionDenied
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::ObjectFormat;

        /// Watch counts are process-wide, so these tests take turns.
        static TURN: Mutex<()> = Mutex::new(());
        fn turn() -> std::sync::MutexGuard<'static, ()> {
            TURN.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
        }

        struct Tree(PathBuf);
        impl Tree {
            fn new(name: &str, folders: &[&str]) -> Self {
                let root = std::env::temp_dir()
                    .join(format!("rocket-watch-{name}-{}", std::process::id()));
                let _ = fs::remove_dir_all(&root);
                fs::create_dir(&root).unwrap();
                for folder in folders {
                    fs::create_dir_all(root.join(folder)).unwrap();
                }
                Self(root.canonicalize().unwrap())
            }
        }
        impl Drop for Tree {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }

        fn paths(watcher: &Watcher) -> Vec<PathBuf> {
            let mut paths: Vec<_> = watcher.folders.values().map(|f| f.path.clone()).collect();
            paths.sort();
            paths
        }

        /// The events the kernel queued for this watcher so far.
        fn queued(watcher: &mut Watcher) -> Vec<u8> {
            let mut buffer = vec![0; 64 * 1024];
            match (&*watcher.inotify).read(&mut buffer) {
                Ok(read) => buffer.truncate(read),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => buffer.clear(),
                Err(error) => panic!("{error}"),
            }
            buffer
        }

        #[test]
        fn folders_past_rockets_share_are_refused_and_released() {
            let _turn = turn();
            let tree = Tree::new("share", &["a/b", "c", "d/e/f"]); // Seven with the root.
            let roots = vec![(tree.0.clone(), false)];
            let before = WATCHED.load(Ordering::Acquire);
            let refused = Watcher::open(roots.clone(), 1000, before + 6)
                .err()
                .expect("seven folders pass a share of six");
            let text = refused.to_string();
            assert!(text.contains("too many folders"), "{text}");
            assert!(text.contains("half of fs.inotify.max_user_watches (1000)"), "{text}");
            assert_eq!(WATCHED.load(Ordering::Acquire), before, "a refused walk keeps nothing");
            let watcher =
                Watcher::open(roots, 1000, before + 7).unwrap();
            assert_eq!(watcher.folders.len(), 7);
            assert_eq!(WATCHED.load(Ordering::Acquire), before + 7);
            drop(watcher);
            assert_eq!(WATCHED.load(Ordering::Acquire), before);
        }

        #[test]
        fn created_and_moved_folders_are_followed() {
            let _turn = turn();
            let tree = Tree::new("moves", &["a/b"]);
            let roots = vec![(tree.0.clone(), false)];
            let mut watcher =
                Watcher::open(roots, 1 << 20, usize::MAX).unwrap();
            fs::create_dir_all(tree.0.join("new/deeper")).unwrap();
            let batch = queued(&mut watcher);
            assert_eq!(watcher.handle(&batch).unwrap(), Some(false), "a worktree change");
            assert!(
                paths(&watcher).contains(&tree.0.join("new/deeper")),
                "a folder made together with its parent is watched too"
            );
            fs::rename(tree.0.join("a"), tree.0.join("moved")).unwrap();
            let batch = queued(&mut watcher);
            watcher.handle(&batch).unwrap();
            let root = &tree.0;
            assert_eq!(
                paths(&watcher),
                [root.clone(), root.join("moved"), root.join("moved/b"), root.join("new"), root.join("new/deeper")]
            );
            // A folder made inside the moved one is found at its new path.
            fs::create_dir(root.join("moved/b/inner")).unwrap();
            let batch = queued(&mut watcher);
            watcher.handle(&batch).unwrap();
            assert!(paths(&watcher).contains(&root.join("moved/b/inner")));
            // Moved out of the tree, it is let go.
            let outside = Tree::new("moves-outside", &[]);
            fs::rename(root.join("moved"), outside.0.join("moved")).unwrap();
            let batch = queued(&mut watcher);
            watcher.handle(&batch).unwrap();
            assert_eq!(paths(&watcher), [root.clone(), root.join("new"), root.join("new/deeper")]);
        }

        #[test]
        fn an_overflow_walks_again_and_lets_go_of_vanished_folders() {
            let _turn = turn();
            let tree = Tree::new("overflow", &["gone/below", "kept"]);
            let roots = vec![(tree.0.clone(), false)];
            let mut watcher =
                Watcher::open(roots, 1 << 20, usize::MAX).unwrap();
            let before = WATCHED.load(Ordering::Acquire);
            fs::remove_dir_all(tree.0.join("gone")).unwrap();
            fs::create_dir_all(tree.0.join("kept/unseen")).unwrap();
            // The queue lost these events: only the overflow notice is handled.
            let mut overflow = (-1_i32).to_ne_bytes().to_vec();
            overflow.extend_from_slice(&IN_Q_OVERFLOW.to_ne_bytes());
            overflow.extend_from_slice(&[0; 8]);
            assert_eq!(watcher.handle(&overflow).unwrap(), Some(true), "lost events may be metadata");
            let root = &tree.0;
            assert_eq!(paths(&watcher), [root.clone(), root.join("kept"), root.join("kept/unseen")]);
            assert_eq!(WATCHED.load(Ordering::Acquire), before - 1, "two let go, one added");
        }

        #[test]
        fn kernel_limits_are_named() {
            let text = watch_limit(io::Error::from_raw_os_error(28), 8192).to_string();
            assert!(text.contains("fs.inotify.max_user_watches is 8192"), "{text}");
            let text = instance_limit(io::Error::from_raw_os_error(24)).to_string();
            assert!(text.contains("fs.inotify.max_user_instances"), "{text}");
            assert_eq!(watch_limit(io::Error::from_raw_os_error(13), 1).raw_os_error(), Some(13));
        }

        /// As a new Windows handle, a watch that joins a shared instance hears only of
        /// changes after it joined, even ones the worker had not read yet.
        #[test]
        fn a_joining_watch_hears_only_what_happens_after_it_joins() {
            let _turn = turn();
            let tree = Tree::new("join", &["src"]);
            let roots = vec![(tree.0.clone(), false)];
            let watcher = Watcher::open(roots.clone(), 1 << 20, usize::MAX).unwrap();
            let shared = || {
                Arc::new(Shared {
                    changed: AtomicBool::new(false),
                    stop: AtomicBool::new(false),
                    error: Mutex::new(None),
                    metadata_generation: AtomicU64::new(0),
                    failed: AtomicBool::new(false),
                })
            };
            let (first, second) = (shared(), shared());
            let quiet: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
            let (_, stop) = io::pipe().unwrap();
            // No worker: only the join and the drain below read events.
            let instance = Instance {
                roots,
                identities: Vec::new(),
                watcher: Mutex::new(watcher),
                subscribers: Mutex::new(vec![Subscriber { shared: first.clone(), wake: quiet.clone() }]),
                failed: AtomicBool::new(false),
                _stop: stop,
            };
            fs::write(tree.0.join("src/before"), "a").unwrap();
            assert!(instance.join(Subscriber { shared: second.clone(), wake: quiet }).is_ok());
            assert!(first.changed.load(Ordering::Acquire), "the earlier watch hears of it");
            assert!(!second.changed.load(Ordering::Acquire), "the joining watch does not");
            fs::write(tree.0.join("src/after"), "b").unwrap();
            instance.drain(&mut instance.watcher.lock().unwrap(), 1).unwrap();
            assert!(second.changed.load(Ordering::Acquire), "later changes reach it");
        }

        #[test]
        fn watches_of_one_repository_share_an_instance_until_it_is_replaced() {
            let _turn = turn();
            let tree = Tree::new("shared", &[".git/refs", "src"]);
            let info = RepositoryInfo {
                worktree: Some(tree.0.clone()),
                git_dir: tree.0.join(".git"),
                common_dir: tree.0.join(".git"),
                object_format: ObjectFormat::Sha1,
                is_shallow: false,
            };
            let quiet: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
            let held = WATCHED.load(Ordering::Acquire);
            let first = RepositoryWatch::new(&info, quiet.clone()).unwrap();
            let second = RepositoryWatch::new(&info, quiet.clone()).unwrap();
            assert!(Arc::ptr_eq(&first.instance, &second.instance));
            let wait = |watch: &RepositoryWatch| {
                let deadline = Instant::now() + Duration::from_secs(5);
                while !watch.take_changed().unwrap() {
                    assert!(Instant::now() < deadline, "notification missing");
                    std::thread::sleep(Duration::from_millis(10));
                }
            };
            fs::write(tree.0.join("src/file"), "a").unwrap();
            wait(&first);
            wait(&second);
            assert_eq!(second.metadata_generation(), Some(0));
            drop(first);
            fs::write(tree.0.join(".git/refs/probe"), "b").unwrap();
            wait(&second);
            assert!(second.metadata_generation().unwrap() > 0);
            // A replaced worktree folder needs its own instance.
            let outside = Tree::new("shared-moved", &[]);
            fs::rename(&tree.0, outside.0.join("old")).unwrap();
            fs::create_dir_all(tree.0.join(".git/refs")).unwrap();
            assert!(!second.is_current().unwrap());
            let third = RepositoryWatch::new(&info, quiet).unwrap();
            assert!(!Arc::ptr_eq(&second.instance, &third.instance));
            assert!(third.is_current().unwrap());
            // Workers release their watches as they stop; leave the count as found.
            drop((second, third));
            let deadline = Instant::now() + Duration::from_secs(5);
            while WATCHED.load(Ordering::Acquire) != held {
                assert!(Instant::now() < deadline, "watches were not released");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
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
