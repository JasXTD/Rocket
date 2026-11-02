use rocket_git::{CancellationToken, Error, Result};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

static REQUESTED: AtomicBool = AtomicBool::new(false);

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn SetConsoleCtrlHandler(
        handler: Option<unsafe extern "system" fn(u32) -> i32>,
        add: i32,
    ) -> i32;
}
#[cfg(windows)]
unsafe extern "system" fn handler(event: u32) -> i32 {
    if event == 0 || event == 1 {
        REQUESTED.store(true, Ordering::Release);
        1
    } else {
        0
    }
}

pub struct Guard {
    done: Arc<AtomicBool>,
    watcher: Option<thread::JoinHandle<()>>,
}
impl Guard {
    pub fn install(cancel: CancellationToken, after: Option<u64>) -> Result<Self> {
        REQUESTED.store(false, Ordering::Release);
        #[cfg(windows)]
        {
            // SAFETY: static WINAPI callback; only an atomic store in the handler.
            if unsafe { SetConsoleCtrlHandler(Some(handler), 1) } == 0 {
                return Err(Error::Io {
                    action: "install CLI cancellation handler",
                    source: std::io::Error::last_os_error(),
                });
            }
        }
        let done = Arc::new(AtomicBool::new(false));
        let stop = done.clone();
        // Zero is deterministic pre-cancellation, useful for scripts and tests.
        if after == Some(0) {
            cancel.cancel();
        }
        let watcher = thread::Builder::new()
            .name("rocket-cli-cancel".into())
            .spawn(move || {
                let started = Instant::now();
                while !stop.load(Ordering::Acquire) {
                    if REQUESTED.load(Ordering::Acquire)
                        || after.is_some_and(|n| started.elapsed() >= Duration::from_millis(n))
                    {
                        cancel.cancel();
                        // Output stays on stderr, and contains no arguments or paths.
                        use std::io::Write;
                        let _ = writeln!(
                            std::io::stderr(),
                            "Stopping; waiting for Git cleanup and final observations..."
                        );
                        break;
                    }
                    thread::park_timeout(Duration::from_millis(10));
                }
            });
        match watcher {
            Ok(watcher) => Ok(Self {
                done,
                watcher: Some(watcher),
            }),
            Err(source) => {
                #[cfg(windows)]
                unsafe {
                    SetConsoleCtrlHandler(Some(handler), 0);
                }
                Err(Error::Io {
                    action: "start CLI cancellation watcher",
                    source,
                })
            }
        }
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Release);
        if let Some(watcher) = self.watcher.take() {
            watcher.thread().unpark();
            let _ = watcher.join();
        }
        #[cfg(windows)]
        {
            // SAFETY: unregister our own static callback after all backend work.
            unsafe {
                SetConsoleCtrlHandler(Some(handler), 0);
            }
        }
    }
}
