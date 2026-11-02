//! Payload-free evidence about where a native control request spent its time.
use rocket_git::McpRequestMetrics;
use serde_json::{Value, json};
use std::{
    cell::Cell,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy)]
pub(crate) enum Phase {
    Queued,
    Ui,
    Guard,
    Validation,
    Input,
    Delivered,
}
impl Phase {
    fn code(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Ui => "ui",
            Self::Guard => "guard",
            Self::Validation => "validation",
            Self::Input => "input",
            Self::Delivered => "delivered",
        }
    }
}
pub(crate) struct Timing {
    number: u64,
    tool: String,
    started: Instant,
    phase: Cell<Phase>,
    changed: Cell<Instant>,
    queue: Cell<Duration>,
    validation: Cell<Duration>,
    delivered: Cell<bool>,
    outcome: Cell<&'static str>,
}
impl Timing {
    pub fn new(tool: &str) -> Self {
        Self::at(tool, Instant::now())
    }
    fn at(tool: &str, now: Instant) -> Self {
        Self {
            number: NEXT_REQUEST.fetch_add(1, Ordering::Relaxed),
            tool: tool.into(),
            started: now,
            phase: Cell::new(Phase::Queued),
            changed: Cell::new(now),
            queue: Cell::new(Duration::ZERO),
            validation: Cell::new(Duration::ZERO),
            delivered: Cell::new(false),
            outcome: Cell::new("abandoned"),
        }
    }
    pub fn mark(&self, phase: Phase) {
        self.mark_at(phase, Instant::now());
    }
    fn mark_at(&self, phase: Phase, now: Instant) {
        let elapsed = now.saturating_duration_since(self.changed.replace(now));
        match self.phase.replace(phase) {
            Phase::Queued => self.queue.set(self.queue.get() + elapsed),
            Phase::Validation => self.validation.set(self.validation.get() + elapsed),
            _ => {}
        }
        if matches!(phase, Phase::Delivered) {
            self.delivered.set(true);
        }
    }
    pub fn finish(&self, outcome: &'static str) {
        self.outcome.set(outcome);
    }
    pub fn metrics(&self) -> McpRequestMetrics<'_> {
        self.metrics_at(Instant::now())
    }
    fn metrics_at(&self, now: Instant) -> McpRequestMetrics<'_> {
        let elapsed = now.saturating_duration_since(self.changed.get());
        let ms = |d: Duration| d.as_millis().min(u64::MAX as u128) as u64;
        McpRequestMetrics {
            native_request: self.number,
            tool: &self.tool,
            phase: self.phase.get().code(),
            outcome: self.outcome.get(),
            elapsed_ms: ms(now.saturating_duration_since(self.started)),
            queue_ms: ms(self.queue.get()
                + if matches!(self.phase.get(), Phase::Queued) {
                    elapsed
                } else {
                    Duration::ZERO
                }),
            validation_ms: ms(self.validation.get()
                + if matches!(self.phase.get(), Phase::Validation) {
                    elapsed
                } else {
                    Duration::ZERO
                }),
            phase_ms: ms(elapsed),
            input_delivered: self.delivered.get(),
        }
    }
    pub fn value(&self) -> Value {
        let m = self.metrics();
        json!({"native_request":m.native_request,"phase":m.phase,"elapsed_ms":m.elapsed_ms,"queue_ms":m.queue_ms,
            "validation_ms":m.validation_ms,"phase_ms":m.phase_ms,"input_delivered":m.input_delivered})
    }
    pub fn should_log(&self) -> bool {
        !matches!(
            self.tool.as_str(),
            "rocket_state" | "rocket_logs" | "rocket_wait"
        ) || self.outcome.get() != "ok"
            || self.started.elapsed() >= Duration::from_millis(250)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queued_expiry_is_distinct_from_validation_and_delivered_input() {
        let start = Instant::now();
        let t = Timing::at("rocket_click", start);
        let expired = t.metrics_at(start + Duration::from_secs(21));
        assert_eq!(expired.queue_ms, 21_000);
        assert!(!expired.input_delivered);
        t.mark_at(Phase::Ui, start + Duration::from_millis(30));
        t.mark_at(Phase::Validation, start + Duration::from_millis(40));
        let validating = t.metrics_at(start + Duration::from_millis(540));
        assert_eq!(validating.queue_ms, 30);
        assert_eq!(validating.validation_ms, 500);
        t.mark_at(Phase::Input, start + Duration::from_millis(640));
        t.mark_at(Phase::Delivered, start + Duration::from_millis(660));
        let done = t.metrics_at(start + Duration::from_millis(700));
        assert_eq!(done.validation_ms, 600);
        assert_eq!(done.phase_ms, 40);
        assert!(done.input_delivered);
    }
}
