//! Recursive FSEvents notifications on a private serial dispatch queue.
//! No directory traversal, content reads or Git subprocesses while idle.
use super::*;
use std::{
    ffi::{CStr, c_char, c_void},
    io,
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::{Path, PathBuf},
    ptr,
    time::Duration,
};

#[derive(Clone)]
pub(super) struct Roots {
    identities: Vec<(PathBuf, u64, u64)>,
    paths: Vec<PathBuf>,
    metadata: Vec<PathBuf>,
}
impl Roots {
    fn capture(info: &RepositoryInfo) -> Result<Self> {
        let mut metadata = vec![info.git_dir.clone(), info.common_dir.clone()];
        let mut paths = metadata.clone();
        paths.extend(info.worktree.clone());
        paths.sort();
        paths.dedup();
        let identities = paths
            .iter()
            .map(|path| {
                let m = std::fs::metadata(path)
                    .map_err(|e| Error::io("inspect watched directory", e))?;
                if !m.is_dir() {
                    return Err(Error::InvalidInput(
                        "Watched path is not a directory".into(),
                    ));
                }
                Ok((path.clone(), m.dev(), m.ino()))
            })
            .collect::<Result<Vec<_>>>()?;
        // FSEvents reports resolved native paths (e.g. /private/tmp, not /tmp).
        for path in paths.iter_mut().chain(metadata.iter_mut()) {
            *path = std::fs::canonicalize(&*path)
                .map_err(|e| Error::io("resolve watched directory", e))?;
        }
        paths.sort();
        paths.dedup();
        metadata.sort();
        metadata.dedup();
        let roots = Self {
            identities,
            paths,
            metadata,
        };
        if !roots.is_current()? {
            return Err(Error::InvalidInput(
                "Repository changed while starting its watch".into(),
            ));
        }
        Ok(roots)
    }
    pub(super) fn is_current(&self) -> Result<bool> {
        for (path, dev, ino) in &self.identities {
            let m = match std::fs::metadata(path) {
                Ok(m) => m,
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
                Err(e) => return Err(Error::io("check watched directory identity", e)),
            };
            if !m.is_dir() || m.dev() != *dev || m.ino() != *ino {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn is_metadata(&self, path: &Path) -> bool {
        self.metadata.iter().any(|p| path.starts_with(p))
            // Unknown paths/normalization never permit selective metadata reuse.
            || !self.paths.iter().any(|p| path.starts_with(p))
    }
}

type Ref = *const c_void;
type Stream = *mut c_void;
type Queue = *mut c_void;
type Callback = unsafe extern "C" fn(Ref, *mut c_void, usize, *mut c_void, *const u32, *const u64);
#[repr(C)]
struct StreamContext {
    version: isize,
    info: *mut c_void,
    retain: Option<unsafe extern "C" fn(Ref) -> Ref>,
    release: Option<unsafe extern "C" fn(Ref)>,
    description: Option<unsafe extern "C" fn(Ref) -> Ref>,
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringCreateWithBytes(
        allocator: Ref,
        bytes: *const u8,
        length: isize,
        encoding: u32,
        external: u8,
    ) -> Ref;
    fn CFArrayCreate(allocator: Ref, values: *const Ref, length: isize, callbacks: Ref) -> Ref;
    fn CFRelease(value: Ref);
}
#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    fn FSEventStreamCreate(
        allocator: Ref,
        callback: Callback,
        context: *mut StreamContext,
        paths: Ref,
        since: u64,
        latency: f64,
        flags: u32,
    ) -> Stream;
    fn FSEventStreamSetDispatchQueue(stream: Stream, queue: Queue);
    fn FSEventStreamStart(stream: Stream) -> u8;
    fn FSEventStreamStop(stream: Stream);
    fn FSEventStreamInvalidate(stream: Stream);
    fn FSEventStreamRelease(stream: Stream);
    fn dispatch_queue_create(label: *const c_char, attributes: Ref) -> Queue;
    fn dispatch_sync_f(
        queue: Queue,
        context: *mut c_void,
        function: unsafe extern "C" fn(*mut c_void),
    );
    fn dispatch_release(object: Queue);
}
struct OwnedCf(Ref);
impl Drop for OwnedCf {
    fn drop(&mut self) {
        // SAFETY: this owns a non-null Create-rule reference.
        unsafe { CFRelease(self.0) }
    }
}
struct Context {
    roots: Roots,
    shared: Arc<Shared>,
    wake: Arc<dyn Fn() + Send + Sync>,
}
// Missing events or an invalidated root require a full read, never selective reuse.
const LOST: u32 = 0x1 | 0x2 | 0x4 | 0x8 | 0x20 | 0x40 | 0x80;

unsafe extern "C" fn events(
    _: Ref,
    data: *mut c_void,
    count: usize,
    paths: *mut c_void,
    flags: *const u32,
    _: *const u64,
) {
    // SAFETY: Session owns the context through invalidation and draining its queue;
    // FSEvents supplies count pointers, flag values and NUL-terminated paths.
    let context = unsafe { &*(data as *const Context) };
    if context.shared.stop.load(Ordering::Acquire) {
        return;
    }
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut changed = false;
        let mut metadata = false;
        // Bound callback work; a larger batch falls back to a complete refresh.
        if count > 65_536 || (count != 0 && (paths.is_null() || flags.is_null())) {
            context.shared.failed.store(true, Ordering::Release);
            changed = true;
            metadata = true;
        } else {
            for i in 0..count {
                let flag = unsafe { *flags.add(i) };
                if flag == 0x10 {
                    continue;
                } // HistoryDone contains no file change.
                changed = true;
                if flag & LOST != 0 {
                    context.shared.failed.store(true, Ordering::Release);
                }
                let path = unsafe { *(paths as *const *const c_char).add(i) };
                if path.is_null() || flag & LOST != 0 {
                    metadata = true;
                } else {
                    let bytes = unsafe { CStr::from_ptr(path) }.to_bytes();
                    metadata |= context
                        .roots
                        .is_metadata(Path::new(std::ffi::OsStr::from_bytes(bytes)));
                }
            }
        }
        if metadata {
            context
                .shared
                .metadata_generation
                .fetch_add(1, Ordering::AcqRel);
        }
        if changed {
            context.shared.changed.store(true, Ordering::Release);
            (context.wake)();
        }
    }));
    if outcome.is_err() {
        // Never unwind through Apple's callback ABI or silently keep a valid cache.
        context.shared.failed.store(true, Ordering::Release);
        context.shared.changed.store(true, Ordering::Release);
        if let Ok(mut error) = context.shared.error.lock() {
            *error = Some(io::Error::other("Repository notification callback failed"));
        }
    }
}
struct Session {
    stream: Stream,
    queue: Queue,
    _context: Box<Context>,
    _paths: OwnedCf,
    _strings: Vec<OwnedCf>,
}
unsafe extern "C" fn drained(_: *mut c_void) {}
impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: called on the owning worker, never its serial callback queue.
        // Invalidation detaches that queue. Drain already-submitted callbacks
        // before releasing the stream/queue/context so none can use freed memory.
        unsafe {
            FSEventStreamStop(self.stream);
            FSEventStreamInvalidate(self.stream);
            dispatch_sync_f(self.queue, ptr::null_mut(), drained);
            FSEventStreamRelease(self.stream);
            dispatch_release(self.queue);
        }
    }
}
impl Session {
    fn create(context: Context) -> io::Result<Self> {
        let mut strings = Vec::new();
        for path in &context.roots.paths {
            let bytes = path.as_os_str().as_bytes();
            if std::str::from_utf8(bytes).is_err() || bytes.contains(&0) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Repository watch requires a UTF-8 path without NUL",
                ));
            }
            // SAFETY: bounded live UTF-8 bytes, kCFStringEncodingUTF8.
            let string = unsafe {
                CFStringCreateWithBytes(
                    ptr::null(),
                    bytes.as_ptr(),
                    bytes.len() as isize,
                    0x08000100,
                    0,
                )
            };
            if string.is_null() {
                return Err(io::Error::other("Cannot create repository watch path"));
            }
            strings.push(OwnedCf(string));
        }
        let values: Vec<Ref> = strings.iter().map(|s| s.0).collect();
        // No array callbacks: Session explicitly owns both array and strings
        // until after stream teardown, without assuming how FSEvents retains them.
        let array = unsafe {
            CFArrayCreate(
                ptr::null(),
                values.as_ptr(),
                values.len() as isize,
                ptr::null(),
            )
        };
        if array.is_null() {
            return Err(io::Error::other("Cannot create repository watch paths"));
        }
        let array = OwnedCf(array);
        let mut context = Box::new(context);
        let mut native = StreamContext {
            version: 0,
            info: (&mut *context as *mut Context).cast(),
            retain: None,
            release: None,
            description: None,
        };
        // SAFETY: callback/context persist through invalidation; event paths are
        // char**, not CF types. SinceNow; NoDefer | WatchRoot | FileEvents.
        let stream = unsafe {
            FSEventStreamCreate(
                ptr::null(),
                events,
                &mut native,
                array.0,
                u64::MAX,
                0.05,
                0x2 | 0x4 | 0x10,
            )
        };
        if stream.is_null() {
            return Err(io::Error::other("Cannot create repository event stream"));
        }
        let queue = unsafe {
            dispatch_queue_create(
                c"io.github.jasxtd.rocket.repository-watch".as_ptr(),
                ptr::null(),
            )
        };
        if queue.is_null() {
            unsafe {
                FSEventStreamRelease(stream);
            }
            return Err(io::Error::other("Cannot create repository event queue"));
        }
        unsafe {
            FSEventStreamSetDispatchQueue(stream, queue);
        }
        let session = Self {
            stream,
            queue,
            _context: context,
            _paths: array,
            _strings: strings,
        };
        if unsafe { FSEventStreamStart(stream) } == 0 {
            return Err(io::Error::other("Cannot start repository event stream"));
        }
        Ok(session)
    }
}

pub(super) fn start(
    info: &RepositoryInfo,
    shared: Arc<Shared>,
    wake: Arc<dyn Fn() + Send + Sync>,
) -> Result<Roots> {
    let roots = Roots::capture(info)?;
    let context = Context {
        roots: roots.clone(),
        shared: shared.clone(),
        wake,
    };
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("rocket-fsevents".into())
        .spawn(move || {
            let session = match Session::create(context) {
                Ok(session) => session,
                Err(error) => {
                    let _ = tx.send(Err(error));
                    return;
                }
            };
            if tx.send(Ok(())).is_err() {
                return;
            }
            // Shutdown observation only: no idle filesystem scans or Git commands.
            while !shared.stop.load(Ordering::Acquire) {
                std::thread::park_timeout(Duration::from_millis(100));
            }
            drop(session);
        })
        .map_err(|e| Error::io("start repository notification worker", e))?;
    rx.recv()
        .map_err(|_| {
            Error::InvalidInput("Repository notification worker stopped during startup".into())
        })?
        .map_err(|e| Error::io("start repository notifications", e))?;
    Ok(roots)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn context() -> (Context, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let wake = calls.clone();
        (
            Context {
                roots: Roots {
                    identities: Vec::new(),
                    paths: vec!["/private/repo".into(), "/private/common".into()],
                    metadata: vec!["/private/repo/.git".into(), "/private/common".into()],
                },
                shared: Arc::new(Shared {
                    changed: AtomicBool::new(false),
                    stop: AtomicBool::new(false),
                    error: Mutex::new(None),
                    metadata_generation: AtomicU64::new(0),
                    failed: AtomicBool::new(false),
                }),
                wake: Arc::new(move || {
                    wake.fetch_add(1, Ordering::SeqCst);
                }),
            },
            calls,
        )
    }
    fn deliver(context: &mut Context, path: &CStr, flag: u32) {
        let paths = [path.as_ptr()];
        // Exercise the real callback with the SDK's char** / flags contract.
        unsafe {
            events(
                ptr::null(),
                (context as *mut Context).cast(),
                1,
                paths.as_ptr().cast_mut().cast(),
                &flag,
                ptr::null(),
            );
        }
    }
    #[test]
    fn callback_classifies_worktree_common_and_unknown_paths() {
        for (path, metadata) in [
            (c"/private/repo/src/a.rs", false),
            (c"/private/repo/.git/index", true),
            (c"/private/common/refs/heads/main", true),
            (c"/private/repository-other/a", true),
        ] {
            let (mut context, wake) = context();
            deliver(&mut context, path, 0x1000); // ItemModified
            assert!(context.shared.changed.load(Ordering::Acquire));
            assert_eq!(
                context.shared.metadata_generation.load(Ordering::Acquire),
                u64::from(metadata)
            );
            assert!(!context.shared.failed.load(Ordering::Acquire));
            assert_eq!(wake.load(Ordering::SeqCst), 1);
        }
    }
    #[test]
    fn lost_events_and_root_invalidation_disable_selective_reuse() {
        // Independent SDK event values: every loss/root flag must invalidate.
        for flag in [1, 2, 4, 8, 32, 64, 128] {
            let (mut context, wake) = context();
            deliver(&mut context, c"/private/repo", flag);
            assert!(context.shared.failed.load(Ordering::Acquire));
            assert!(context.shared.changed.load(Ordering::Acquire));
            assert_eq!(
                context.shared.metadata_generation.load(Ordering::Acquire),
                1
            );
            assert_eq!(wake.load(Ordering::SeqCst), 1);
        }
    }
    #[test]
    fn oversized_batches_invalidate_and_shutdown_suppresses_notifications() {
        let (mut context, wake) = context();
        unsafe {
            events(
                ptr::null(),
                (&mut context as *mut Context).cast(),
                65_537,
                ptr::null_mut(),
                ptr::null(),
                ptr::null(),
            );
        }
        assert!(context.shared.failed.load(Ordering::Acquire));
        assert_eq!(wake.load(Ordering::SeqCst), 1);
        context.shared.stop.store(true, Ordering::Release);
        deliver(&mut context, c"/private/repo/a", 0x1000);
        assert_eq!(wake.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn history_done_is_not_a_file_change() {
        let (mut context, wake) = context();
        deliver(&mut context, c"/private/repo", 16);
        assert!(!context.shared.changed.load(Ordering::Acquire));
        assert_eq!(wake.load(Ordering::SeqCst), 0);
    }
}
