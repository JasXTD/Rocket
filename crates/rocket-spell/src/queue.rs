//! Bounded requests with cancellation when a read's Pending is dropped.
use super::{Error, Request, stopped};
use std::{
    collections::VecDeque,
    sync::{Arc, Condvar, Mutex},
};

// The UI keeps three texts, one suggestion and one dictionary change. Leave
// room for those callers while imposing backpressure on other producers.
const CAPACITY: usize = 16;

#[derive(Default)]
struct State {
    pending: VecDeque<Request>,
    closed: bool,
}
#[derive(Default)]
struct Queue {
    state: Mutex<State>,
    ready: Condvar,
}
pub(super) struct Sender(Arc<Queue>);
pub(super) struct Receiver(Arc<Queue>);

pub(super) fn channel() -> (Sender, Receiver) {
    let queue = Arc::new(Queue::default());
    (Sender(Arc::clone(&queue)), Receiver(queue))
}
impl Sender {
    pub(super) fn send(&self, request: Request) {
        let mut state = self.0.state.lock().expect("spell queue lock poisoned");
        // Evicted UI texts release their payload immediately on the next send,
        // even while the worker is busy inside Windows' non-cancellable check.
        state.pending.retain(Request::needed);
        if state.closed {
            request.fail(stopped());
        } else if state.pending.len() == CAPACITY {
            request.fail(Error::Failed("its request queue is full".into()));
        } else {
            state.pending.push_back(request);
            self.0.ready.notify_one();
        }
    }
}
impl Drop for Sender {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().expect("spell queue lock poisoned");
        state.closed = true;
        state.pending.retain(Request::needed);
        self.0.ready.notify_one();
    }
}
impl Receiver {
    pub(super) fn next(&self) -> Option<Request> {
        let mut state = self.0.state.lock().expect("spell queue lock poisoned");
        loop {
            while let Some(request) = state.pending.pop_front() {
                if request.needed() {
                    return Some(request);
                }
            }
            if state.closed {
                return None;
            }
            state = self.0.ready.wait(state).expect("spell queue lock poisoned");
        }
    }
}
impl Drop for Receiver {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().expect("spell queue lock poisoned");
        state.closed = true;
        // No consumer remains, including on a worker failure. Dropping replies
        // wakes every live Pending with the existing stopped-worker error.
        state.pending.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::response;

    #[test]
    fn discarded_reads_do_not_run_and_current_checks_keep_their_order() {
        let (sender, receiver) = channel();
        for i in 0..1000 {
            let (reply, pending) = response();
            sender.send(Request::Check(format!("obsolete {i}"), reply));
            drop(pending);
        }
        let (first_reply, first) = response();
        let (second_reply, second) = response();
        sender.send(Request::Check("summary".into(), first_reply));
        sender.send(Request::Check("description".into(), second_reply));
        drop(sender);
        for text in ["summary", "description"] {
            let Some(Request::Check(actual, reply)) = receiver.next() else {
                panic!("missing current text")
            };
            assert_eq!(actual, text);
            reply.send(Ok(Vec::new()));
        }
        assert!(receiver.next().is_none());
        assert!(first.wait().unwrap().is_empty());
        assert!(second.wait().unwrap().is_empty());
    }

    #[test]
    fn full_queue_refuses_explicitly_and_cancellation_frees_capacity() {
        let (sender, receiver) = channel();
        let mut live = Vec::new();
        for _ in 0..CAPACITY {
            let (reply, pending) = response();
            sender.send(Request::Suggest("word".into(), reply));
            live.push(pending);
        }
        let (reply, refused) = response();
        sender.send(Request::Suggest("overflow".into(), reply));
        assert_eq!(
            refused.wait(),
            Err(Error::Failed("its request queue is full".into()))
        );
        live.clear();
        let (reply, latest) = response();
        sender.send(Request::Suggest("latest".into(), reply));
        let Some(Request::Suggest(text, reply)) = receiver.next() else {
            panic!("cancelled work should free a slot")
        };
        assert_eq!(text, "latest");
        reply.send(Ok(vec!["replacement".into()]));
        assert_eq!(latest.wait().unwrap(), ["replacement"]);
    }

    #[test]
    fn accepted_dictionary_changes_survive_discarded_answers_and_shutdown() {
        let (sender, receiver) = channel();
        let (reply, pending) = response();
        sender.send(Request::Ignore("session word".into(), reply));
        drop(pending);
        let (reply, pending) = response();
        sender.send(Request::Add("dictionary word".into(), reply));
        drop(pending);
        drop(sender);
        assert!(
            matches!(receiver.next(), Some(Request::Ignore(word, _)) if word == "session word")
        );
        assert!(
            matches!(receiver.next(), Some(Request::Add(word, _)) if word == "dictionary word")
        );
        assert!(receiver.next().is_none());
        // This tests delivery, without modifying the real Windows dictionary.
    }

    #[test]
    fn losing_the_worker_fails_pending_and_new_requests() {
        let (sender, receiver) = channel();
        let (reply, pending) = response();
        sender.send(Request::Check("pending".into(), reply));
        drop(receiver);
        assert_eq!(pending.wait(), Err(stopped()));
        let (reply, pending) = response();
        sender.send(Request::Check("new".into(), reply));
        assert_eq!(pending.wait(), Err(stopped()));
    }
}
