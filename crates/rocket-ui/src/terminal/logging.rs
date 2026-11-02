//! One background writer for the fixed, payload-free terminal event vocabulary.
use rocket_git::{DiagnosticLog, LogHealth, WriteTask};
use std::collections::BTreeMap;

const BATCH: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Event {
    name: &'static str,
    failed: bool,
}

#[derive(Default)]
pub(crate) struct EventLog {
    // Counts retain repeated events without an unbounded queue or one thread per
    // frame. Keys come only from TerminalPane's fixed labels and exit outcome.
    pending: BTreeMap<Event, u64>,
    in_flight: Vec<Event>,
    task: Option<WriteTask<LogHealth>>,
    reported_drops: u64,
}

impl EventLog {
    pub(super) fn push(&mut self, name: &'static str, failed: bool) {
        let count = self.pending.entry(Event { name, failed }).or_default();
        *count = count.saturating_add(1);
    }

    pub(crate) fn busy(&self) -> bool {
        self.task.is_some() || !self.pending.is_empty()
    }

    pub(super) fn poll(&mut self, log: &DiagnosticLog) -> rocket_git::Result<Option<String>> {
        let mut notice = None;
        if let Some(result) = self.task.as_mut().and_then(WriteTask::try_finish) {
            self.task = None;
            match result {
                Ok(health) => {
                    self.in_flight.clear();
                    if health.dropped_events > self.reported_drops {
                        self.reported_drops = health.dropped_events;
                        notice = Some(format!(
                            "Some terminal diagnostics could not be saved ({} dropped log records). Check diagnostic log health.",
                            health.dropped_events
                        ));
                    }
                }
                Err(error) => {
                    // A failed worker may have written part of its batch. Retry
                    // at least once rather than silently losing the remainder.
                    for event in std::mem::take(&mut self.in_flight) {
                        self.push(event.name, event.failed);
                    }
                    return Err(error);
                }
            }
        }
        if self.task.is_none() && !self.pending.is_empty() {
            let batch: Vec<_> = self
                .pending
                .iter()
                .flat_map(|(&event, &count)| {
                    std::iter::repeat_n(event, count.min(BATCH as u64) as usize)
                })
                .take(BATCH)
                .collect();
            let events = batch.clone();
            let log = log.clone();
            // Nothing is removed from pending until spawning succeeds. The
            // logger mutex, file locks, flushes and health read all stay here.
            let task = WriteTask::spawn(move |_| {
                for event in events {
                    let _: rocket_git::Result<()> = log.request(event.name, |_| {
                        if event.failed {
                            Err(rocket_git::Error::InvalidInput(
                                "Terminal session ended with an I/O or startup error; see the terminal panel.".into(),
                            ))
                        } else {
                            Ok(())
                        }
                    });
                }
                Ok(log.health())
            })?;
            for event in &batch {
                *self.pending.get_mut(event).expect("queued event") -= 1;
            }
            self.pending.retain(|_, count| *count != 0);
            self.in_flight = batch;
            self.task = Some(task);
        }
        Ok(notice)
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::test_common as common;
    use std::{
        fs, thread,
        time::{Duration, Instant},
    };

    fn drain(events: &mut EventLog, log: &DiagnosticLog) -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut notices = Vec::new();
        while events.busy() {
            if let Some(notice) = events.poll(log).unwrap() {
                notices.push(notice);
            }
            assert!(Instant::now() < deadline, "diagnostics did not drain");
            thread::sleep(Duration::from_millis(5));
        }
        notices
    }

    #[test]
    fn bounded_counts_drain_repeated_events_and_failed_exit() {
        let fixture = common::Fixture::new();
        let log = DiagnosticLog::open(fixture.root.join("logs")).unwrap();
        let mut events = EventLog::default();
        let count = BATCH * 2 + 9;
        for _ in 0..count {
            events.push("terminal_input_queued", false);
        }
        events.push("terminal_exit", true);
        assert_eq!(
            events.pending.len(),
            2,
            "backlog storage grows by event kind, not count"
        );
        assert!(drain(&mut events, &log).is_empty());
        let rows: Vec<serde_json::Value> = fs::read_to_string(log.directory().join("rocket.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(rows.iter().filter(|r| r["event"] == "request_end" && r["operation"] == "terminal_input_queued").count(), count);
        assert!(rows.iter().any(|r| r["event"] == "request_end"
            && r["operation"] == "terminal_exit"
            && r["outcome"] != "ok"));
        assert_eq!(log.health().dropped_events, 0);
    }

    #[test]
    fn storage_failures_are_reported_without_leaving_a_worker_pending() {
        let fixture = common::Fixture::new();
        let log = DiagnosticLog::open(fixture.root.join("logs")).unwrap();
        let path = log.directory().join("rocket.jsonl");
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        let mut events = EventLog::default();
        events.push("terminal_stop", false);
        let notices = drain(&mut events, &log);
        assert_eq!(notices.len(), 1);
        assert!(notices[0].contains("could not be saved"));
        assert!(log.health().dropped_events > 0);
        assert!(!events.busy());
        assert!(events.poll(&log).unwrap().is_none());
    }
}
