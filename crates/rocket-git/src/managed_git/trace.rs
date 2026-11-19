//! A bounded observer of verification I/O. Workers only update in-memory state;
//! the single process-wide reporter owns no runtime files and is never joined by
//! verification. A blocked diagnostic disk must not hold up Git or its cleanup.
use super::ManagedGitProgress;
use crate::{CancellationToken, DiagnosticLog, Result, diagnostics::ManagedGitIoSnapshot};
use std::{
    io::{self, Read},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU8, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    time::{Duration, Instant},
};

const CAPACITY: usize = 32;
const POLL: Duration = Duration::from_secs(1);
const SLOW: Duration = Duration::from_secs(5);
const REPEAT: Duration = Duration::from_secs(30);
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static NEXT: AtomicU64 = AtomicU64::new(1);

#[derive(Default)]
pub(super) struct Trace(Option<Arc<State>>);

struct State {
    id: u64,
    log: DiagnosticLog,
    progress: ManagedGitProgress,
    cancel: CancellationToken,
    started: Instant,
    finished: AtomicU8,
    // Slot zero is the coordinator; the other four match the bounded hash workers.
    slots: [Mutex<Slot>; 5],
}
impl Drop for State {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::Relaxed);
    }
}
#[derive(Clone, Copy)]
struct Slot {
    phase: &'static str,
    thread: u32,
    file: usize,
    size: u64,
    bytes: u64,
    since: Instant,
    os_error: Option<i32>,
}
impl Default for Slot {
    fn default() -> Self {
        Self {
            phase: "idle",
            thread: 0,
            file: 0,
            size: 0,
            bytes: 0,
            since: Instant::now(),
            os_error: None,
        }
    }
}

fn sender() -> Option<&'static SyncSender<Arc<State>>> {
    static SENDER: OnceLock<Option<SyncSender<Arc<State>>>> = OnceLock::new();
    SENDER
        .get_or_init(|| {
            let (tx, rx) = mpsc::sync_channel(CAPACITY);
            std::thread::Builder::new()
                .name("git-io-monitor".into())
                .spawn(move || {
                    monitor(rx, POLL, SLOW, REPEAT, |state, snapshot| {
                        state.log.managed_git_io(&snapshot);
                    })
                })
                .ok()
                .map(|_| tx)
        })
        .as_ref()
}

impl Trace {
    pub(super) fn start(
        log: Option<&DiagnosticLog>,
        cancel: &CancellationToken,
        progress: &ManagedGitProgress,
    ) -> Self {
        let Some(log) = log else {
            return Self::default();
        };
        Self::register(log, cancel, progress, sender())
    }

    fn register(
        log: &DiagnosticLog,
        cancel: &CancellationToken,
        progress: &ManagedGitProgress,
        sender: Option<&SyncSender<Arc<State>>>,
    ) -> Self {
        let Some(sender) = sender else {
            log.managed_git_monitor_unavailable();
            return Self::default();
        };
        let mut active = ACTIVE.load(Ordering::Relaxed);
        loop {
            if active >= CAPACITY {
                log.managed_git_monitor_unavailable();
                return Self::default();
            }
            match ACTIVE.compare_exchange_weak(
                active,
                active + 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(current) => active = current,
            }
        }
        let state = Arc::new(State {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            log: log.background_writer(),
            progress: progress.clone(),
            cancel: cancel.clone(),
            started: Instant::now(),
            finished: AtomicU8::new(0),
            slots: std::array::from_fn(|_| Mutex::new(Slot::default())),
        });
        let trace = Self(Some(state.clone()));
        trace.worker(0).phase("prepare");
        if sender.try_send(state).is_err() {
            log.managed_git_monitor_unavailable();
            return Self::default();
        }
        trace
    }

    pub(super) fn worker(&self, index: usize) -> Worker<'_> {
        let worker = Worker(self.0.as_ref().map(|s| &s.slots[index]));
        worker.update(|s| s.thread = native_thread());
        worker
    }

    pub(super) fn finish<T>(&self, result: &Result<T>) {
        if let Some(state) = &self.0 {
            state.finished.store(
                match result {
                    Ok(_) => 1,
                    Err(crate::Error::Cancelled) => 2,
                    Err(_) => 3,
                },
                Ordering::Release,
            );
        }
    }
}
impl Drop for Trace {
    fn drop(&mut self) {
        if let Some(state) = &self.0 {
            // An unwind also retires the observation. Never wait for the reporter.
            let _ = state
                .finished
                .compare_exchange(0, 4, Ordering::Release, Ordering::Relaxed);
        }
    }
}

pub(super) struct Worker<'a>(Option<&'a Mutex<Slot>>);
impl Worker<'_> {
    fn update(&self, update: impl FnOnce(&mut Slot)) {
        if let Some(slot) = self.0 {
            update(&mut slot.lock().unwrap_or_else(|e| e.into_inner()));
        }
    }
    pub(super) fn phase(&self, phase: &'static str) {
        self.update(|s| {
            s.phase = phase;
            s.since = Instant::now();
        });
    }
    pub(super) fn file(&self, index: usize, size: u64) {
        self.update(|s| {
            s.file = index;
            s.size = size;
            s.bytes = 0;
            s.os_error = None;
        });
    }
    pub(super) fn io<T>(
        &self,
        phase: &'static str,
        run: impl FnOnce() -> io::Result<T>,
    ) -> io::Result<T> {
        self.phase(phase);
        let result = run();
        if let Err(error) = &result {
            self.update(|s| s.os_error = error.raw_os_error());
        }
        result
    }
    pub(super) fn reader<'a, R: Read>(&'a self, reader: &'a mut R) -> impl Read + 'a {
        ObservedReader {
            worker: self,
            reader,
        }
    }
}
struct ObservedReader<'a, 's, R> {
    worker: &'a Worker<'s>,
    reader: &'a mut R,
}
impl<R: Read> Read for ObservedReader<'_, '_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let n = self.worker.io("read_file", || self.reader.read(buffer))?;
        self.worker.update(|s| {
            s.bytes += n as u64;
            s.phase = "hash_file";
            s.since = Instant::now();
        });
        Ok(n)
    }
}

struct Watched {
    state: Arc<State>,
    reported: Option<Instant>,
    reported_cancel: bool,
}
fn monitor(
    rx: Receiver<Arc<State>>,
    poll: Duration,
    slow: Duration,
    repeat: Duration,
    mut emit: impl FnMut(&State, ManagedGitIoSnapshot),
) {
    let mut watched: Vec<Watched> = Vec::new();
    loop {
        let incoming = if watched.is_empty() {
            rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected)
        } else {
            rx.recv_timeout(poll)
        };
        match incoming {
            Ok(state) => watched.push(Watched {
                state,
                reported: None,
                reported_cancel: false,
            }),
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        watched.retain_mut(|watch| {
            let state = &watch.state;
            let done = state.finished.load(Ordering::Acquire);
            let now = Instant::now();
            let cancelled = state.cancel.is_cancelled();
            // Preserve a new Stop request without waiting for the repeat interval.
            let due = (cancelled && !watch.reported_cancel)
                || watch
                    .reported
                    .is_none_or(|last| now.duration_since(last) >= repeat);
            let (checked, total, _, _) = state.progress.snapshot();
            let mut reported = false;
            for (index, slot) in state.slots.iter().enumerate() {
                // No observer ever waits on a worker lock, or holds it during logging.
                let Ok(slot) = slot.try_lock().map(|s| *s) else {
                    continue;
                };
                let waited = now.saturating_duration_since(slot.since);
                let report = if done != 0 {
                    slot.os_error.is_some() || (index == 0 && watch.reported.is_some())
                } else {
                    due && slot.phase != "idle" && waited >= slow
                };
                if report {
                    emit(
                        state,
                        ManagedGitIoSnapshot {
                            verification: state.id,
                            worker: index,
                            thread: slot.thread,
                            phase: slot.phase,
                            file: slot.file,
                            file_bytes: slot.size,
                            read_bytes: slot.bytes,
                            wait_ms: waited.as_millis() as u64,
                            elapsed_ms: state.started.elapsed().as_millis() as u64,
                            checked,
                            total,
                            cancelled,
                            os_error: slot.os_error,
                            outcome: match done {
                                0 => "running",
                                1 => "ok",
                                2 => "cancelled",
                                3 => "error",
                                _ => "abandoned",
                            },
                        },
                    );
                    reported = true;
                }
            }
            if reported {
                watch.reported = Some(now);
                watch.reported_cancel = cancelled;
            }
            done == 0
        });
    }
}

fn native_thread() -> u32 {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetCurrentThreadId() -> u32;
        }
        // SAFETY: This parameterless Windows function returns the caller's ID.
        unsafe { GetCurrentThreadId() }
    }
    #[cfg(not(windows))]
    {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, thread};

    struct Logs(std::path::PathBuf);
    impl Logs {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "rocket-io-PRIVATE-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).expect("create an owned diagnostic fixture");
            Self(path)
        }
        fn open(&self) -> DiagnosticLog {
            DiagnosticLog::open(&self.0).unwrap()
        }
        fn text(&self) -> String {
            fs::read_to_string(self.0.join("rocket.jsonl")).unwrap()
        }
    }
    impl Drop for Logs {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct PausedRead {
        partial: bool,
        entered: mpsc::Sender<()>,
        release: Receiver<()>,
    }
    impl Read for PausedRead {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            if self.partial {
                self.entered.send(()).unwrap();
                self.release.recv().unwrap();
            }
            self.partial = true;
            bytes[..4].copy_from_slice(b"data");
            Ok(4)
        }
    }

    #[test]
    fn blocked_read_is_reported_before_release_with_offset_and_cancellation() {
        let logs = Logs::new();
        let log = logs.open();
        let (tx, rx) = mpsc::sync_channel(2);
        let (events, receive) = mpsc::channel();
        let reporter = thread::spawn(move || {
            monitor(
                rx,
                Duration::from_millis(5),
                Duration::from_millis(10),
                Duration::from_secs(60),
                |state, snapshot| {
                    state.log.managed_git_io(&snapshot);
                    events.send(snapshot).unwrap();
                },
            )
        });
        let cancel = CancellationToken::new();
        let trace = Trace::register(&log, &cancel, &ManagedGitProgress::default(), Some(&tx));
        trace.worker(0).phase("idle");
        let stop = cancel.clone();
        let (entered, started) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let reader = thread::spawn(move || {
            let worker = trace.worker(1);
            worker.file(17, 8);
            let mut stream = PausedRead {
                partial: false,
                entered,
                release: wait,
            };
            let mut bytes = [0; 8];
            worker.reader(&mut stream).read_exact(&mut bytes).unwrap();
            assert_eq!(&bytes, b"datadata");
            let result = stop.check();
            trace.finish(&result);
            result
        });
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        let first = receive.recv_timeout(Duration::from_secs(5));
        cancel.cancel();
        let second = receive.recv_timeout(Duration::from_secs(5));
        release.send(()).unwrap();
        assert!(matches!(
            reader.join().unwrap(),
            Err(crate::Error::Cancelled)
        ));
        let end = loop {
            let event = receive.recv_timeout(Duration::from_secs(5)).unwrap();
            if event.outcome != "running" {
                break event;
            }
        };
        drop(tx);
        reporter.join().unwrap();
        let first = first.expect("blocked read was not observed while still blocked");
        assert_eq!(
            (first.worker, first.file, first.file_bytes, first.read_bytes),
            (1, 17, 8, 4)
        );
        assert_eq!(first.phase, "read_file");
        #[cfg(windows)]
        assert_ne!(first.thread, 0);
        assert!(second.unwrap().cancelled);
        assert_eq!(end.outcome, "cancelled");
        let text = logs.text();
        assert!(text.contains("managed_git_io_wait"));
        assert!(text.contains("\"read_bytes\":4"));
        assert!(!text.contains("PRIVATE"));
        assert!(!text.contains("datadata"));
    }

    #[test]
    fn a_stuck_reporter_and_full_queue_do_not_hold_verification_cleanup() {
        let logs = Logs::new();
        let log = logs.open();
        let (tx, rx) = mpsc::sync_channel(1);
        let (entered, started) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let reporter = thread::spawn(move || {
            let mut first = true;
            monitor(
                rx,
                Duration::from_millis(5),
                Duration::ZERO,
                Duration::from_secs(60),
                |_, _| {
                    if first {
                        first = false;
                        entered.send(()).unwrap();
                        wait.recv().unwrap();
                    }
                },
            );
        });
        let cancel = CancellationToken::new();
        let progress = ManagedGitProgress::default();
        let trace = Trace::register(&log, &cancel, &progress, Some(&tx));
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        let queued = Trace::register(&log, &cancel, &progress, Some(&tx));
        let refused = Trace::register(&log, &cancel, &progress, Some(&tx));
        assert!(refused.0.is_none());
        assert_eq!(log.health().dropped_events, 1);
        let (finished, done) = mpsc::channel();
        let verifier = thread::spawn(move || {
            trace.finish(&Ok(()));
            drop(trace);
            drop(queued);
            finished.send(()).unwrap();
        });
        let completed = done.recv_timeout(Duration::from_secs(1));
        release.send(()).unwrap();
        drop(tx);
        verifier.join().unwrap();
        reporter.join().unwrap();
        assert!(
            completed.is_ok(),
            "verification waited for blocked diagnostic output"
        );
    }

    #[test]
    fn returned_os_error_is_recorded_without_error_text_even_for_a_fast_failure() {
        let logs = Logs::new();
        let log = logs.open();
        let (tx, rx) = mpsc::sync_channel(1);
        let (events, received) = mpsc::channel();
        let reporter = thread::spawn(move || {
            monitor(
                rx,
                Duration::from_millis(5),
                SLOW,
                REPEAT,
                |state, snapshot| {
                    state.log.managed_git_io(&snapshot);
                    events.send(snapshot).unwrap();
                },
            )
        });
        let trace = Trace::register(
            &log,
            &CancellationToken::new(),
            &ManagedGitProgress::default(),
            Some(&tx),
        );
        let worker = trace.worker(1);
        worker.file(9, 123);
        let error = worker
            .io::<()>("open_file", || Err(io::Error::from_raw_os_error(5)))
            .unwrap_err();
        assert_eq!(error.raw_os_error(), Some(5));
        trace.finish::<()>(&Err(crate::Error::InvalidInput(
            "PRIVATE failure text".into(),
        )));
        let event = received.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(trace);
        drop(tx);
        reporter.join().unwrap();
        assert_eq!(event.os_error, Some(5));
        assert_eq!(event.phase, "open_file");
        assert_eq!(event.file, 9);
        assert_eq!(event.outcome, "error");
        assert!(!logs.text().contains("PRIVATE"));
    }
}
