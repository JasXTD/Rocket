use crate::{CancellationToken, Error, Result};
use std::{
    sync::mpsc::{self, Receiver, TryRecvError},
    thread::{self, JoinHandle},
};

/// A background read. Dropping or cancelling it requests cleanup of its active Git command.
/// Completion is explicitly consumed once via `try_finish` or `wait`.
pub struct ReadTask<T> {
    task: Task<T>,
}

/// A background mutation. Completed reports survive later cancellation so the caller
/// can see whether Git succeeded, failed, or needs reconciliation. Use this for writes.
pub struct WriteTask<T> {
    task: Task<T>,
}

impl<T: Send + 'static> ReadTask<T> {
    pub fn spawn(
        read: impl FnOnce(CancellationToken) -> Result<T> + Send + 'static,
    ) -> Result<Self> {
        Self::spawn_with_wake(read, || {})
    }

    /// Notify the consumer after a returned result is available, never before it
    /// is published. A receiver already dropped at publication is not woken.
    /// The callback must not block or panic. Keep polling for worker panics,
    /// which disconnect the channel without returning a result.
    pub fn spawn_with_wake(
        read: impl FnOnce(CancellationToken) -> Result<T> + Send + 'static,
        wake: impl FnOnce() + Send + 'static,
    ) -> Result<Self> {
        Ok(Self {
            task: Task::spawn("rocket-git-read", true, read, wake)?,
        })
    }
    pub fn cancel(&self) {
        self.task.cancel.cancel();
    }
    pub fn try_finish(&mut self) -> Option<Result<T>> {
        self.task.try_finish()
    }
    pub fn wait(self) -> Result<T> {
        self.task.wait()
    }
}

impl<T: Send + 'static> WriteTask<T> {
    pub fn spawn(
        write: impl FnOnce(CancellationToken) -> Result<T> + Send + 'static,
    ) -> Result<Self> {
        Ok(Self {
            task: Task::spawn("rocket-git-write", false, write, || {})?,
        })
    }
    pub fn cancel(&self) {
        self.task.cancel.cancel();
    }
    pub fn try_finish(&mut self) -> Option<Result<T>> {
        self.task.try_finish()
    }
    pub fn wait(self) -> Result<T> {
        self.task.wait()
    }
}

struct Task<T> {
    cancel: CancellationToken,
    receiver: Receiver<Result<T>>,
    worker: Option<JoinHandle<()>>,
    finished: bool,
    discard_on_cancel: bool,
}

impl<T: Send + 'static> Task<T> {
    fn spawn(
        name: &str,
        discard_on_cancel: bool,
        operation: impl FnOnce(CancellationToken) -> Result<T> + Send + 'static,
        wake: impl FnOnce() + Send + 'static,
    ) -> Result<Self> {
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let (sender, receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let result = token.check().and_then(|()| operation(token));
                if sender.send(result).is_ok() {
                    wake();
                }
            })
            .map_err(|e| Error::io("start background worker", e))?;
        Ok(Self {
            cancel,
            receiver,
            worker: Some(worker),
            finished: false,
            discard_on_cancel,
        })
    }

    fn deliver(&self, result: Result<T>) -> Result<T> {
        if self.discard_on_cancel {
            self.cancel.check().and(result)
        } else {
            result
        }
    }

    /// Nonblocking UI polling. A consumed result is never delivered again.
    pub fn try_finish(&mut self) -> Option<Result<T>> {
        if self.finished {
            return None;
        }
        match self.receiver.try_recv() {
            Ok(result) => {
                self.finished = true;
                Some(self.deliver(result))
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.finished = true;
                Some(Err(Error::WorkerPanicked))
            }
        }
    }

    pub fn wait(mut self) -> Result<T> {
        if self.finished {
            return Err(Error::InvalidInput(
                "task result was already consumed".into(),
            ));
        }
        let result = self.receiver.recv().unwrap_or(Err(Error::WorkerPanicked));
        self.finished = true;
        if self
            .worker
            .take()
            .expect("task has a worker")
            .join()
            .is_err()
        {
            return Err(Error::WorkerPanicked);
        }
        self.deliver(result)
    }
}

impl<T> Drop for Task<T> {
    fn drop(&mut self) {
        // Do not block the UI on Drop. The worker owns its process and finishes cleanup;
        // callers requiring a completion barrier should cancel() followed by wait().
        self.cancel.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn completion_is_delivered_once_without_blocking_polling() {
        let (release, gate) = mpsc::channel();
        let mut task = ReadTask::spawn(move |_| {
            gate.recv().unwrap();
            Ok(42)
        })
        .unwrap();
        assert!(task.try_finish().is_none());
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(result) = task.try_finish() {
                assert_eq!(result.unwrap(), 42);
                break;
            }
            assert!(Instant::now() < deadline);
            thread::yield_now();
        }
        assert!(task.try_finish().is_none());
        assert!(matches!(task.wait(), Err(Error::InvalidInput(_))));
    }

    #[test]
    fn cancelling_a_buffered_result_does_not_deliver_stale_success() {
        let mut task = ReadTask::spawn(|_| Ok(42)).unwrap();
        task.task.worker.take().unwrap().join().unwrap(); // result is definitely queued
        task.cancel();
        assert!(matches!(task.try_finish(), Some(Err(Error::Cancelled))));
    }

    #[test]
    fn wake_publishes_completion_before_notifying_the_consumer() {
        let (notified, ready) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let mut task = ReadTask::spawn_with_wake(
            |_| Ok(42),
            move || {
                notified.send(()).unwrap();
                gate.recv().unwrap();
            },
        )
        .unwrap();
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        // The UI can consume the result even while the wake callback is running.
        assert_eq!(task.try_finish().unwrap().unwrap(), 42);
        assert!(task.try_finish().is_none());
        release.send(()).unwrap();
        task.task.worker.take().unwrap().join().unwrap();
        assert!(ready.try_recv().is_err());
    }

    #[test]
    fn waking_does_not_bypass_cancelled_read_delivery() {
        let (notified, ready) = mpsc::channel();
        let mut task = ReadTask::spawn_with_wake(
            |_| Ok(42),
            move || {
                notified.send(()).unwrap();
            },
        )
        .unwrap();
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        task.cancel();
        assert!(matches!(task.try_finish(), Some(Err(Error::Cancelled))));
    }

    #[test]
    fn a_dropped_consumer_is_not_woken() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        let woke = Arc::new(AtomicBool::new(false));
        let callback_woke = woke.clone();
        let (started, ready) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let mut task = ReadTask::spawn_with_wake(
            move |_| {
                started.send(()).unwrap();
                gate.recv().unwrap();
                Ok(42)
            },
            move || {
                callback_woke.store(true, Ordering::SeqCst);
            },
        )
        .unwrap();
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        let worker = task.task.worker.take().unwrap();
        drop(task);
        release.send(()).unwrap();
        worker.join().unwrap();
        assert!(!woke.load(Ordering::SeqCst));
    }

    #[test]
    fn notifying_read_still_reports_a_worker_panic() {
        let mut task = ReadTask::<()>::spawn_with_wake(
            |_| panic!("fixture panic"),
            || panic!("a panicked operation does not return a result"),
        )
        .unwrap();
        assert!(task.task.worker.take().unwrap().join().is_err());
        task.cancel();
        assert!(matches!(
            task.try_finish(),
            Some(Err(Error::WorkerPanicked))
        ));
        assert!(task.try_finish().is_none());
    }

    #[test]
    fn worker_panic_is_reported_as_an_error() {
        let task = ReadTask::<()>::spawn(|_| panic!("fixture panic")).unwrap();
        assert!(matches!(task.wait(), Err(Error::WorkerPanicked)));
    }

    #[test]
    fn write_completion_survives_cancellation_after_result_is_queued() {
        let mut task = WriteTask::spawn(|_| Ok(42)).unwrap();
        task.task.worker.take().unwrap().join().unwrap();
        task.cancel();
        assert_eq!(task.try_finish().unwrap().unwrap(), 42);
        assert!(task.try_finish().is_none());
    }

    #[test]
    fn write_task_preserves_a_report_returned_after_cancellation() {
        let (started, ready) = mpsc::channel();
        let task = WriteTask::spawn(move |cancel| {
            started.send(()).unwrap();
            while !cancel.is_cancelled() {
                thread::yield_now();
            }
            Ok("operation reconciled")
        })
        .unwrap();
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        task.cancel();
        assert_eq!(task.wait().unwrap(), "operation reconciled");
    }
}
