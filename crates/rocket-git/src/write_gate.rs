use crate::{CancellationToken, Error, Result};
use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, OnceLock, Weak},
    time::Duration,
};

mod process_lock;
use process_lock::ProcessLock;

static GATES: OnceLock<Mutex<HashMap<PathBuf, Weak<WriteGate>>>> = OnceLock::new();

/// In-process FIFO plus cross-process ownership of the common repository directory.
/// Git locks and operation-specific revalidation still arbitrate with other applications.
pub(crate) fn for_repository(common_dir: &Path) -> Result<Arc<WriteGate>> {
    let mut gates = GATES
        .get_or_init(Mutex::default)
        .lock()
        .map_err(|_| Error::StatePoisoned("write gate registry"))?;
    gates.retain(|_, gate| gate.strong_count() != 0);
    if let Some(gate) = gates.get(common_dir).and_then(Weak::upgrade) {
        return Ok(gate);
    }
    let gate = Arc::new(WriteGate {
        common_dir: common_dir.to_owned(),
        state: Mutex::default(),
        changed: Condvar::new(),
    });
    gates.insert(common_dir.to_owned(), Arc::downgrade(&gate));
    Ok(gate)
}

#[derive(Debug)]
pub(crate) struct WriteGate {
    common_dir: PathBuf,
    state: Mutex<State>,
    changed: Condvar,
}

#[derive(Debug, Default)]
struct State {
    active: bool,
    next: u64,
    waiters: VecDeque<u64>,
}

impl WriteGate {
    pub(crate) fn acquire(&self, cancel: &CancellationToken) -> Result<WritePermit<'_>> {
        cancel.check()?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::StatePoisoned("write queue"))?;
        let ticket = state.next;
        state.next = state
            .next
            .checked_add(1)
            .ok_or_else(|| Error::InvalidInput("write ticket overflow".into()))?;
        state.waiters.push_back(ticket);
        loop {
            if cancel.is_cancelled() {
                state.waiters.retain(|&waiting| waiting != ticket);
                self.changed.notify_all();
                return Err(Error::Cancelled);
            }
            if !state.active && state.waiters.front() == Some(&ticket) {
                state.waiters.pop_front();
                state.active = true;
                // Own the local queue before opening the OS lock, but do not hold
                // its mutex during filesystem work. Any error/cancellation drops
                // this permit, waking the next local waiter. Other processes fail
                // fast: they must refresh/review before explicitly retrying.
                drop(state);
                let mut permit = WritePermit {
                    gate: self,
                    process_lock: None,
                };
                permit.process_lock = Some(ProcessLock::acquire(&self.common_dir)?);
                cancel.check()?;
                return Ok(permit);
            }
            // A token can be cancelled without a queue notification. The bounded wait
            // lets pending jobs observe that cancellation without busy polling.
            state = self
                .changed
                .wait_timeout(state, Duration::from_millis(25))
                .map_err(|_| Error::StatePoisoned("write queue"))?
                .0;
        }
    }
}

pub(crate) struct WritePermit<'a> {
    gate: &'a WriteGate,
    process_lock: Option<ProcessLock>,
}
impl Drop for WritePermit<'_> {
    fn drop(&mut self) {
        // Release OS ownership before a local waiter can attempt to acquire it.
        drop(self.process_lock.take());
        if let Ok(mut state) = self.gate.state.lock() {
            state.active = false;
            self.gate.changed.notify_all();
        }
        // A poisoned mutex stays poisoned; later callers receive StatePoisoned.
    }
}

#[cfg(test)]
mod tests;
