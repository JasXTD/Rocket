//! On-demand local timings. No repository text is stored in a report or export.
use crate::{
    app::{DialogKind, Rocket, UiControl},
    backend::Backend,
    pages,
    preferences::{card_foot, card_frame, card_note},
    style::*,
};
use eframe::egui::*;
use rocket_git::{Error, ReadTask, Result};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub const PASSES: usize = 3;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Refresh,
    History,
    Diff,
}
impl Operation {
    pub const ALL: [Self; 3] = [Self::Refresh, Self::History, Self::Diff];
    pub fn code(self) -> &'static str {
        match self {
            Self::Refresh => "refresh",
            Self::History => "history",
            Self::Diff => "diff",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Refresh => "Full refresh",
            Self::History => "History · first 200",
            Self::Diff => "Commit diff",
        }
    }
    /// The operation's name in the timings table and the running status.
    fn title(self) -> &'static str {
        match self {
            Self::Refresh => "Full refresh",
            Self::History => "History",
            Self::Diff => "Commit diff",
        }
    }
    /// What the operation reads, after its name in the table.
    fn detail(self) -> &'static str {
        match self {
            Self::History => "first 200 commits",
            Self::Refresh | Self::Diff => "",
        }
    }
    pub(crate) fn log_label(self) -> &'static str {
        match self {
            Self::Refresh => "benchmark-refresh",
            Self::History => "benchmark-history",
            Self::Diff => "benchmark-diff",
        }
    }
}
#[derive(Clone, Debug)]
pub struct Sample {
    pub operation: Operation,
    pub pass: usize,
    pub elapsed_us: Option<u64>,
    pub items: usize,
    pub limited: bool,
}
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub samples: Vec<Sample>,
    pub active: Option<Operation>,
    pub pass: usize,
    pub changed: bool,
    pub observation_unavailable: bool,
    pub elapsed_ms: u64,
    pub outcome: &'static str,
    pub failure: Option<&'static str>,
}
impl Report {
    pub fn summary(&self) -> Value {
        json!({"schema":1,"rocket_version":env!("CARGO_PKG_VERSION"),"passes":PASSES,
            "outcome":self.outcome,"failure":self.failure,"elapsed_ms":self.elapsed_ms,
            "changes_observed":self.changed,"change_observation_unavailable":self.observation_unavailable,
            "scope":"Read-only local backend timings; 200-commit HEAD history and starting HEAD diff. First pass uses existing caches; caches are not cleared. Excludes rendering, network and external tools. Paths, refs, identities and content are omitted.",
            "samples":self.samples.iter().map(|s|json!({"operation":s.operation.code(),"pass":s.pass,
                "elapsed_us":s.elapsed_us,"items":s.items,"limited":s.limited,"skipped":s.elapsed_us.is_none()})).collect::<Vec<_>>()})
    }
    /// First-pass and repeat-only statistics. Skipped/failed reads are not zeroes.
    pub fn timings(&self, operation: Operation) -> (Option<f64>, Option<f64>, Option<(f64, f64)>) {
        let first = self
            .samples
            .iter()
            .find(|s| s.operation == operation && s.pass == 1)
            .and_then(|s| s.elapsed_us)
            .map(|n| n as f64 / 1000.0);
        let mut repeats: Vec<_> = self
            .samples
            .iter()
            .filter(|s| s.operation == operation && s.pass > 1)
            .filter_map(|s| s.elapsed_us)
            .collect();
        repeats.sort_unstable();
        let median = if repeats.is_empty() {
            None
        } else {
            Some(
                (repeats[(repeats.len() - 1) / 2] as f64 + repeats[repeats.len() / 2] as f64)
                    / 2000.0,
            )
        };
        let values: Vec<_> = self
            .samples
            .iter()
            .filter(|s| s.operation == operation)
            .filter_map(|s| s.elapsed_us)
            .collect();
        let range = values
            .iter()
            .min()
            .zip(values.iter().max())
            .map(|(lo, hi)| (*lo as f64 / 1000.0, *hi as f64 / 1000.0));
        (first, median, range)
    }
}
pub type Progress = Arc<Mutex<Report>>;
pub(crate) fn update(progress: &Progress, f: impl FnOnce(&mut Report)) -> Result<()> {
    let mut report = progress
        .lock()
        .map_err(|_| Error::StatePoisoned("benchmark progress"))?;
    f(&mut report);
    Ok(())
}

#[derive(Default)]
pub struct State {
    pub owner: Option<u64>,
    pub report: Report,
    pub stopping: bool,
    pub error: Option<String>,
    pub copied: bool,
    close_after_stop: bool,
    progress: Progress,
    task: Option<ReadTask<()>>,
    started: Option<Instant>,
}
impl State {
    pub fn running(&self) -> bool {
        self.task.is_some()
    }
    pub fn cancel(&mut self) {
        if let Some(task) = &self.task {
            self.stopping = true;
            task.cancel();
        }
    }
    pub fn snapshot(&self) -> Value {
        json!({"owner":self.owner,"running":self.running(),"stopping":self.stopping,
        "active":self.report.active.map(Operation::code),"pass":self.report.pass,"error":self.error,"copied":self.copied,"report":self.report.summary()})
    }
    fn start(&mut self, ctx: &Context, backend: Backend, path: std::path::PathBuf, owner: u64) {
        if self.running() {
            return;
        }
        self.owner = Some(owner);
        self.error = None;
        self.copied = false;
        self.stopping = false;
        self.close_after_stop = false;
        self.report = Report {
            outcome: "running",
            ..Default::default()
        };
        self.progress = Arc::new(Mutex::new(self.report.clone()));
        let progress = self.progress.clone();
        self.started = Some(Instant::now());
        match crate::read_task::spawn(ctx, move |cancel| {
            backend.benchmark(&path, &progress, &cancel)
        }) {
            Ok(task) => self.task = Some(task),
            Err(e) => {
                self.report.outcome = "failed";
                self.error = Some(crate::backend::user_error(&e));
            }
        }
    }
    fn poll(&mut self, ctx: &Context) {
        if !self.running() {
            return;
        }
        // The worker holds this lock only to publish a few numbers, never for I/O.
        if let Ok(report) = self.progress.try_lock() {
            self.report = report.clone();
        }
        if let Some(started) = self.started {
            self.report.elapsed_ms = started.elapsed().as_millis() as u64;
        }
        if let Some(result) = self.task.as_mut().and_then(ReadTask::try_finish) {
            self.task = None;
            if let Ok(report) = self.progress.lock() {
                self.report = report.clone();
            }
            self.report.active = None;
            self.report.elapsed_ms = self
                .started
                .take()
                .map_or(0, |t| t.elapsed().as_millis() as u64);
            self.report.outcome = match result {
                Ok(()) if !self.stopping => "complete",
                Ok(()) | Err(Error::Cancelled) => "cancelled",
                Err(e) => {
                    self.error = Some(crate::backend::user_error(&e));
                    "failed"
                }
            };
            self.stopping = false;
            ctx.request_repaint();
        }
        if self.running() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }
}

impl Rocket {
    pub(crate) fn poll_benchmark(&mut self, ctx: &Context) {
        let visible = self.page() == Some(pages::Page::Benchmark);
        if self.quitting || !visible || self.tab().map(|t| t.id) != self.benchmark.owner {
            self.benchmark.cancel();
        }
        self.benchmark.poll(ctx);
        if let Some(d) = self
            .dialog
            .as_mut()
            .filter(|d| matches!(d.kind, DialogKind::Benchmark))
        {
            d.flag = self.benchmark.running();
            if !self.benchmark.running() && self.benchmark.close_after_stop {
                self.dialog = None;
                self.benchmark.close_after_stop = false;
            }
        }
    }
    pub(crate) fn benchmark_page(&mut self, ui: &mut Ui) {
        let owner = self.tab().map(|t| t.id);
        if !self.benchmark.running() && self.benchmark.owner != owner {
            self.benchmark = State {
                owner,
                ..Default::default()
            };
        }
        let ready = !self.busy()
            && !self.auto_fetch.tabs.values().any(|t| t.running())
            && self.backend.is_some()
            && self.tab().is_some_and(|t| t.snapshot.is_some());
        let widgets = &mut self.frame_widgets;
        let s = &mut self.benchmark;
        let mut start = false;
        let mut close = false;
        pages::header(
            ui,
            Icon::Activity,
            "Repository benchmark",
            "Local read timings · 3 passes · read only",
            |ui| {
                let r = pages::ghost(ui, true, "Close");
                pages::control(widgets, "benchmark-close", &r, ui.clip_rect());
                close |= r.clicked();
                // Stop takes Run's place while the benchmark runs.
                if s.running() {
                    let r = pages::danger_ghost(
                        ui,
                        !s.stopping,
                        if s.stopping { "Stopping" } else { "Stop" },
                    );
                    pages::control(widgets, "benchmark-stop", &r, ui.clip_rect());
                    if r.clicked() {
                        s.cancel();
                    }
                } else {
                    let r = pages::primary(ui, ready, "Run benchmark")
                        .on_disabled_hover_text("Waiting for other repository operations to finish");
                    pages::control(widgets, "benchmark-run", &r, ui.clip_rect());
                    start = r.clicked();
                }
            },
        );
        if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            close = true;
        }
        if close {
            if s.running() {
                s.close_after_stop = true;
                s.cancel();
            } else {
                self.dialog = None;
                return;
            }
        }
        let output = ScrollArea::vertical()
            .id_salt("benchmark-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                Frame::new()
                    .inner_margin(Margin { left: 24, right: 24, top: 0, bottom: 18 })
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        s.timings_card(ui, widgets, ready);
                    });
            });
        pages::region(
            widgets,
            "benchmark-scroll",
            output.inner_rect.intersect(ui.clip_rect()),
            true,
        );
        if start && let (Some(backend), Some(tab)) = (self.backend.clone(), self.tab()) {
            let (path, id) = (tab.path.clone(), tab.id);
            self.benchmark.start(ui.ctx(), backend, path, id);
            if let Some(d) = &mut self.dialog {
                d.flag = self.benchmark.running();
            }
        }
    }
}

impl State {
    /// The run's status for the Timings bar: its text, colour and whether it is still going.
    fn status(&self, ready: bool) -> (String, Color32, bool) {
        if self.stopping {
            return ("Stopping".into(), AMBER(), true);
        }
        if self.running() {
            let read = self.report.active.map_or("Preparing", Operation::title);
            let pass = self.report.pass.max(1);
            return (format!("Pass {pass} of {PASSES} · {read}"), BLUE(), true);
        }
        let (text, color) = match self.report.outcome {
            "complete" => ("Complete", GREEN()),
            "cancelled" => ("Stopped", pages::NEUTRAL()),
            "failed" => ("Failed", RED()),
            _ if !ready => ("Waiting for other operations", pages::NEUTRAL()),
            _ => ("Ready", pages::NEUTRAL()),
        };
        (text.into(), color, false)
    }

    /// The Timings card: the bar with the run's status and progress, the table, a line for what
    /// failed or needs a rerun, and Copy summary in its foot.
    fn timings_card(&mut self, ui: &mut Ui, widgets: &mut HashMap<String, UiControl>, ready: bool) {
        let running = self.running();
        let (status, color, spinning) = self.status(ready);
        let skipped = self.report.samples.iter().filter(|s| s.elapsed_us.is_none()).count();
        let reads = self.report.samples.len() - skipped;
        let progress = if self.report.samples.is_empty() && !running {
            "Not run yet".to_owned()
        } else {
            let skipped = if skipped > 0 { format!(" · {skipped} skipped") } else { String::new() };
            format!(
                "{reads} of {} reads{skipped} · {:.1} s",
                PASSES * Operation::ALL.len(),
                self.report.elapsed_ms as f64 / 1000.0
            )
        };
        let mut lines: Vec<(Color32, String)> = Vec::new();
        if self.report.outcome == "failed" && !running {
            lines.push((RED(), self.error.clone().unwrap_or_else(|| "See Activity & diagnostics.".into())));
        }
        if self.report.changed {
            lines.push((AMBER(), "The repository changed during this run. Run again before comparing timings.".into()));
        }
        if self.report.observation_unavailable {
            lines.push((AMBER(), "Change observation was unavailable, so stability could not be checked.".into()));
        }
        if self.report.samples.iter().any(|s| s.limited) {
            lines.push((AMBER(), "A read reached its limit; its timing covers the bounded result only.".into()));
        }
        card_frame(ui, |ui| {
            bar(ui, &status, color, spinning, &progress);
            table(ui, &self.report, running);
            if !lines.is_empty() {
                card_note(ui, |ui| {
                    for (color, text) in &lines {
                        pages::callout(ui, *color, text);
                    }
                });
            }
            card_foot(ui, None, |ui| {
                let (image, label) = if self.copied {
                    (Icon::Check, "Copied")
                } else {
                    (Icon::Copy, "Copy summary")
                };
                let enabled = !running && !self.report.samples.is_empty();
                // The tool paints its own ink, so it is dimmed here while there is nothing to copy.
                let r = ui
                    .scope(|ui| {
                        if !enabled {
                            ui.multiply_opacity(0.45);
                        }
                        pages::tool(ui, image, label, enabled)
                    })
                    .inner;
                pages::control(widgets, "benchmark-copy", &r, ui.clip_rect());
                if r.clicked() {
                    ui.ctx().copy_text(
                        serde_json::to_string_pretty(&self.report.summary()).expect("numeric report"),
                    );
                    self.copied = true;
                }
            });
        });
    }
}

/// The Timings card's bar: the title and the run's status pill on the left, and the reads and time
/// so far at the right edge.
fn bar(ui: &mut Ui, status: &str, color: Color32, spinning: bool, progress: &str) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
    response.widget_info(|| {
        WidgetInfo::labeled(WidgetType::Label, true, format!("Timings: {status}, {progress}"))
    });
    ui.painter()
        .rect_filled(rect, CornerRadius { nw: 7, ne: 7, sw: 0, se: 0 }, CARD());
    ui.painter()
        .hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, CARD_LINE()));
    let inner = rect.shrink2(vec2(14.0, 0.0));
    let title = ui
        .painter()
        .layout_no_wrap("Timings".into(), semibold(13.0), TEXT());
    let title_width = title.size().x;
    ui.painter()
        .galley(pos2(inner.left(), rect.center().y - title.size().y / 2.0), title, TEXT());
    let text = ui
        .painter()
        .layout_no_wrap(status.into(), semibold(11.0), color);
    let lead = if spinning { 14.0 } else { 0.0 };
    let pill = Rect::from_min_size(
        pos2(inner.left() + title_width + 10.0, rect.center().y - 10.0),
        vec2(text.size().x + 16.0 + lead, 20.0),
    );
    ui.painter()
        .rect_filled(pill, 10.0, color.gamma_multiply(0.14));
    if spinning {
        Spinner::new().size(10.0).color(color).paint_at(
            ui,
            Rect::from_center_size(pos2(pill.left() + 13.0, pill.center().y), vec2(10.0, 10.0)),
        );
    }
    ui.painter().galley(
        pos2(pill.left() + 8.0 + lead, pill.center().y - text.size().y / 2.0),
        text,
        color,
    );
    let mut job = text::LayoutJob::simple_singleline(progress.into(), FontId::proportional(12.0), HINT());
    job.wrap = text::TextWrapping::truncate_at_width((inner.right() - pill.right() - 12.0).max(0.0));
    let progress = ui.painter().layout_job(job);
    ui.painter().galley(
        pos2(inner.right() - progress.size().x, rect.center().y - progress.size().y / 2.0),
        progress,
        HINT(),
    );
}

/// The timings: an operation per row with its first, repeat-median and range timings aligned
/// right. While the benchmark runs, the read in progress has a tint and a spinner, and reads still
/// to come show placeholders, so the table keeps its shape.
fn table(ui: &mut Ui, report: &Report, running: bool) {
    let width = ui.available_width();
    // Where the Operation, First, Repeat median and Range columns end.
    let ends = [0.4, 0.6, 0.8, 1.0].map(|f| f * width);
    let (head, _) = ui.allocate_exact_size(vec2(width, 34.0), Sense::hover());
    let font = FontId::proportional(12.0);
    ui.painter().text(
        pos2(head.left() + 14.0, head.center().y),
        Align2::LEFT_CENTER,
        "Operation",
        font.clone(),
        HINT(),
    );
    for (column, label) in ["First", "Repeat median", "Range"].into_iter().enumerate() {
        ui.painter().text(
            pos2(head.left() + ends[column + 1] - 14.0, head.center().y),
            Align2::RIGHT_CENTER,
            label,
            font.clone(),
            HINT(),
        );
    }
    ui.painter()
        .hline(head.x_range(), head.bottom() - 0.5, Stroke::new(1.0, ROW_LINE()));
    let ms = |v: f64| format!("{v:.1} ms");
    for (index, operation) in Operation::ALL.into_iter().enumerate() {
        let (row, response) = ui.allocate_exact_size(vec2(width, 46.0), Sense::hover());
        let active = running && report.active == Some(operation);
        if active {
            ui.painter().rect_filled(row, 0.0, BLUE().gamma_multiply(0.05));
            ui.painter()
                .rect_filled(Rect::from_min_size(row.min, vec2(3.0, row.height())), 0.0, BLUE());
        }
        let name = ui
            .painter()
            .layout_no_wrap(operation.title().into(), FontId::proportional(13.0), TEXT());
        let name_width = name.size().x;
        ui.painter()
            .galley(pos2(row.left() + 14.0, row.center().y - name.size().y / 2.0), name, TEXT());
        if !operation.detail().is_empty() {
            let mut job = text::LayoutJob::simple_singleline(
                operation.detail().into(),
                FontId::proportional(12.0),
                FAINT(),
            );
            job.wrap = text::TextWrapping::truncate_at_width((ends[0] - 22.0 - name_width - 8.0).max(0.0));
            let detail = ui.painter().layout_job(job);
            ui.painter().galley(
                pos2(row.left() + 22.0 + name_width, row.center().y - detail.size().y / 2.0),
                detail,
                FAINT(),
            );
        }
        let (first, median, range) = report.timings(operation);
        let mut samples = report.samples.iter().filter(|s| s.operation == operation).peekable();
        let skipped = samples.peek().is_some() && samples.all(|s| s.elapsed_us.is_none());
        let summary = if skipped {
            ui.painter().text(
                pos2(row.left() + ends[0] + 14.0, row.center().y),
                Align2::LEFT_CENTER,
                "Skipped · no commits yet",
                FontId::proportional(13.0),
                FAINT(),
            );
            format!("{}: skipped, no commits yet", operation.title())
        } else {
            let cells = [
                first.map(ms),
                median.map(ms),
                range.map(|(lo, hi)| format!("{lo:.1}–{hi:.1} ms")),
            ];
            for (column, cell) in cells.iter().enumerate() {
                let right = row.left() + ends[column + 1] - 14.0;
                // The read in progress: the first pass fills First, later passes Repeat median.
                let measuring = active && (if report.pass <= 1 { column == 0 } else { column == 1 });
                match cell {
                    Some(text) => {
                        let (font, color) = match column {
                            1 => (semibold(13.0), TEXT()),
                            2 => (FontId::proportional(13.0), MUTED()),
                            _ => (FontId::proportional(13.0), TEXT()),
                        };
                        ui.painter()
                            .text(pos2(right, row.center().y), Align2::RIGHT_CENTER, text, font, color);
                    }
                    None if measuring => {
                        Spinner::new().size(13.0).color(BLUE()).paint_at(
                            ui,
                            Rect::from_center_size(pos2(right - 7.0, row.center().y), vec2(13.0, 13.0)),
                        );
                    }
                    None if running => {
                        ui.painter().rect_filled(
                            Rect::from_min_max(
                                pos2(right - 58.0, row.center().y - 5.0),
                                pos2(right, row.center().y + 5.0),
                            ),
                            5.0,
                            RAISED(),
                        );
                    }
                    None => {
                        ui.painter().text(
                            pos2(right, row.center().y),
                            Align2::RIGHT_CENTER,
                            "—",
                            FontId::proportional(13.0),
                            FAINT(),
                        );
                    }
                }
            }
            let shown = |cell: &Option<String>| cell.clone().unwrap_or_else(|| "none yet".into());
            format!(
                "{}: first {}, repeat median {}, range {}",
                operation.title(),
                shown(&cells[0]),
                shown(&cells[1]),
                shown(&cells[2])
            )
        };
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &summary));
        if index + 1 < Operation::ALL.len() {
            ui.painter()
                .hline(row.x_range(), row.bottom() - 0.5, Stroke::new(1.0, ROW_LINE()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The text the Timings card paints for `state`.
    fn painted(state: &mut State, ready: bool) -> Vec<String> {
        fn texts(shape: &Shape, shown: &mut Vec<String>) {
            match shape {
                Shape::Vec(shapes) => shapes.iter().for_each(|s| texts(s, shown)),
                Shape::Text(text) => shown.push(text.galley.text().to_owned()),
                _ => {}
            }
        }
        let ctx = Context::default();
        crate::style::install(&ctx);
        let mut widgets = HashMap::new();
        let mut output = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 700.0))),
                ..Default::default()
            },
            |ui| state.timings_card(ui, &mut widgets, ready),
        );
        output.textures_delta.clear();
        let mut shown = Vec::new();
        for clipped in &output.shapes {
            texts(&clipped.shape, &mut shown);
        }
        shown
    }

    fn sample(operation: Operation, pass: usize, elapsed_us: Option<u64>) -> Sample {
        Sample {
            operation,
            pass,
            elapsed_us,
            items: 1,
            limited: false,
        }
    }

    #[test]
    fn timings_card_shows_the_run_and_its_numbers_without_explanations() {
        let has = |shown: &[String], text: &str| shown.iter().any(|t| t == text);
        // A finished run: its status, progress and values; the explanations live in the guide.
        let mut done = State::default();
        for (operation, values) in [
            (Operation::Refresh, [866_200, 731_200, 1_244_700]),
            (Operation::History, [367_300, 259_000, 583_500]),
            (Operation::Diff, [218_800, 168_200, 237_100]),
        ] {
            for (pass, value) in values.into_iter().enumerate() {
                done.report.samples.push(sample(operation, pass + 1, Some(value)));
            }
        }
        done.report.outcome = "complete";
        done.report.elapsed_ms = 5_700;
        let shown = painted(&mut done, true);
        for text in ["Timings", "Complete", "9 of 9 reads · 5.7 s", "Full refresh", "first 200 commits", "866.2 ms", "731.2–1244.7 ms", "Copy summary"] {
            assert!(has(&shown, text), "{text} in {shown:?}");
        }
        assert!(
            !shown.iter().any(|t| t.contains("What is measured") || t.contains("Caches are not cleared")),
            "{shown:?}"
        );
        // Before a run while other operations are busy: the reason, and dashes for every value.
        let shown = painted(&mut State::default(), false);
        for text in ["Waiting for other operations", "Not run yet", "—"] {
            assert!(has(&shown, text), "{text} in {shown:?}");
        }
        // A repository with no commits: history and diff are skipped, never shown as zero.
        let mut unborn = State::default();
        for pass in 1..=PASSES {
            unborn.report.samples.push(sample(Operation::Refresh, pass, Some(400_000)));
            unborn.report.samples.push(sample(Operation::History, pass, None));
            unborn.report.samples.push(sample(Operation::Diff, pass, None));
        }
        unborn.report.outcome = "complete";
        let shown = painted(&mut unborn, true);
        assert!(has(&shown, "3 of 9 reads · 6 skipped · 0.0 s"), "{shown:?}");
        assert_eq!(shown.iter().filter(|t| *t == "Skipped · no commits yet").count(), 2, "{shown:?}");
        // While it runs: the pass and read in the bar, and no dash for reads still to come.
        let mut running = State {
            task: Some(
                ReadTask::spawn(|cancel| {
                    while !cancel.is_cancelled() {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    Err(Error::Cancelled)
                })
                .unwrap(),
            ),
            ..Default::default()
        };
        running.report.samples.push(sample(Operation::Refresh, 1, Some(866_200)));
        running.report.active = Some(Operation::History);
        running.report.pass = 1;
        let shown = painted(&mut running, true);
        assert!(has(&shown, "Pass 1 of 3 · History"), "{shown:?}");
        assert!(!has(&shown, "—"), "reads still to come show placeholders: {shown:?}");
        running.cancel();
    }

    #[test]
    fn repeat_median_excludes_first_and_skipped_reads() {
        let mut r = Report::default();
        for (pass, value) in [(1, 9000), (2, 1000), (3, 3000)] {
            r.samples.push(Sample {
                operation: Operation::Refresh,
                pass,
                elapsed_us: Some(value),
                items: 1,
                limited: false,
            });
        }
        r.samples.push(Sample {
            operation: Operation::Diff,
            pass: 1,
            elapsed_us: None,
            items: 0,
            limited: false,
        });
        assert_eq!(
            r.timings(Operation::Refresh),
            (Some(9.0), Some(2.0), Some((1.0, 9.0)))
        );
        assert_eq!(r.timings(Operation::Diff), (None, None, None));
    }
    #[test]
    fn summary_uses_only_numeric_data_and_fixed_labels() {
        let r = Report {
            outcome: "cancelled",
            changed: true,
            observation_unavailable: true,
            ..Default::default()
        };
        let v = r.summary();
        assert_eq!(v["outcome"], "cancelled");
        assert_eq!(v["changes_observed"], true);
        assert!(v.get("error").is_none());
        assert!(v.get("path").is_none());
    }
    #[test]
    fn cancellation_keeps_completed_samples_and_final_state_after_later_polls() {
        let ctx = Context::default();
        let progress = Arc::new(Mutex::new(Report {
            outcome: "running",
            samples: vec![Sample {
                operation: Operation::Refresh,
                pass: 1,
                elapsed_us: Some(100),
                items: 1,
                limited: false,
            }],
            ..Default::default()
        }));
        let task = ReadTask::spawn(|cancel| {
            while !cancel.is_cancelled() {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(Error::Cancelled)
        })
        .unwrap();
        let mut state = State {
            task: Some(task),
            progress,
            started: Some(Instant::now()),
            ..Default::default()
        };
        state.cancel();
        assert!(state.running() && state.stopping);
        let deadline = Instant::now() + Duration::from_secs(5);
        while state.running() {
            state.poll(&ctx);
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        for _ in 0..3 {
            state.poll(&ctx);
            assert_eq!(state.report.outcome, "cancelled");
            assert_eq!(state.report.samples.len(), 1);
            assert!(!state.stopping);
        }
    }
}
