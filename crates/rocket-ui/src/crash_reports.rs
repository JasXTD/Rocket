//! Local report review; all I/O runs outside the frame and export never overwrites.
use crate::{
    activity_history::{clock, day, local, started, timestamp},
    app::{Rocket, UiControl},
    folder_browser::PickTarget,
    pages::{self, Part, ReviewHeader, Row},
    style::*,
};
use eframe::egui::*;
use rocket_git::{
    DiagnosticLog, Error, ReadTask, WriteTask,
    crash_reports::{self as reports, Component, Kind, Report, Snapshot},
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(test)]
#[path = "crash_reports_tests.rs"]
mod tests;

/// A report recorded this soon before Rocket starts is offered once on that start.
const RECENT_MS: u64 = 15 * 60 * 1000;
/// Below this body width the list becomes a picker above the details.
const SPLIT_WIDTH: f32 = 760.0;

type Widgets = HashMap<String, UiControl>;

#[derive(Default)]
pub struct State {
    pub open: bool,
    pub refresh_requested: bool,
    pub directory: PathBuf,
    /// The new file a save publishes. Save as builds it from the folder and name.
    pub export_path: String,
    pub export_open: bool,
    pub export_dir: String,
    pub export_name: String,
    /// The report the next-start notice offers, until it is opened or dismissed.
    pub recent: Option<String>,
    feedback_reveal: bool,
    offsets_open: bool,
    started_ms: u64,
    startup_checked: bool,
    startup_read: bool,
    snapshot: Snapshot,
    selected: Option<String>,
    read: Option<ReadTask<Snapshot>>,
    write: Option<WriteTask<()>>,
    error: Option<String>,
    notice: Option<String>,
    stopping: bool,
    /// The last error came from reading the reports, so Retry reads again.
    read_failed: bool,
}
fn message(error: &Error) -> String {
    match error {
        Error::Io{source,..} if source.kind()==std::io::ErrorKind::AlreadyExists=>"That file already exists. Choose a new filename; nothing was replaced.".into(),
        Error::Io{source,..} if source.kind()==std::io::ErrorKind::PermissionDenied=>"The report folder or file cannot be accessed. Check its permissions and try again.".into(),
        Error::Io{source,..} if source.kind()==std::io::ErrorKind::InvalidInput=>"Choose an absolute path to a new JSON file in an existing folder without redirected directories.".into(),
        Error::Cancelled=>"Stopped before saving. No destination file was created.".into(),
        Error::InvalidInput(s)|Error::Unsupported(s)=>s.clone(),
        _=>"The report operation could not complete. Check local storage and try again.".into(),
    }
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}
impl State {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            started_ms: now_ms(),
            ..Default::default()
        }
    }
    fn selected(&self) -> Option<&Report> {
        self.selected
            .as_ref()
            .and_then(|id| self.snapshot.reports.iter().find(|r| &r.id == id))
    }
    pub fn exporting(&self) -> bool {
        self.write.is_some()
    }
    pub(crate) fn shutdown(&mut self) {
        self.open = false;
        self.export_open = false;
        self.refresh_requested = false;
        self.read = None;
        if let Some(task) = &self.write {
            task.cancel();
            self.stopping = true;
        }
    }
    pub fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({"open":self.open,"loading":self.read.is_some()||self.refresh_requested,"exporting":self.exporting(),"stopping":self.stopping,
            "directory":self.directory,"error":self.error,"notice":self.notice,"damaged":self.snapshot.damaged,
            "reports":self.snapshot.reports.iter().map(|r|serde_json::json!({"id":r.id,"kind":r.kind.code(),"component":r.component.code(),"version":r.version,"title":title(r)})).collect::<Vec<_>>(),
            "selected":self.selected().map(|r|serde_json::from_str::<serde_json::Value>(&r.json()).expect("typed report serializes")),"export_open":self.export_open,"export_path":self.export_path,
            "export_dir":self.export_dir,"export_name":self.export_name,"offsets_open":self.offsets_open,"recent":self.recent})
    }
    fn load(&mut self, ctx: &Context, log: Option<DiagnosticLog>) {
        if self.exporting() {
            return;
        }
        self.read = None;
        self.error = None;
        self.notice = None;
        self.read_failed = false;
        self.refresh_requested = false;
        let directory = self.directory.clone();
        match crate::read_task::spawn(ctx, move |cancel| match log {
            Some(log) => log.request("crash-report-read", |_| reports::read(&directory, &cancel)),
            None => reports::read(&directory, &cancel),
        }) {
            Ok(task) => self.read = Some(task),
            Err(e) => self.error = Some(message(&e)),
        }
    }
    /// Once per process: read the reports quietly to offer one recorded just before
    /// this start. Opening the dialog reads again; this read never reports errors.
    fn check_startup(&mut self, ctx: &Context, log: Option<DiagnosticLog>) {
        if std::mem::replace(&mut self.startup_checked, true) || self.started_ms == 0 {
            return;
        }
        self.startup_read = true;
        self.load(ctx, log);
    }
    fn save(&mut self, scope: Option<PathBuf>, log: Option<DiagnosticLog>) {
        if self.exporting() {
            return;
        }
        let Some(report) = self.selected().cloned() else {
            return;
        };
        let path = PathBuf::from(&self.export_path);
        self.error = None;
        self.notice = None;
        self.stopping = false;
        match WriteTask::spawn(move |cancel| {
            let save = || {
                if !path.is_absolute()
                    || path.components().any(|c| {
                        matches!(
                            c,
                            std::path::Component::ParentDir | std::path::Component::CurDir
                        )
                    })
                    || path
                        .file_name()
                        .is_none_or(|n| n.to_string_lossy().contains(':'))
                    || path
                        .extension()
                        .is_none_or(|e| !e.eq_ignore_ascii_case("json"))
                {
                    return Err(Error::InvalidInput(
                        "Choose an absolute path to a new .json file in an existing folder.".into(),
                    ));
                }
                let _held = scope
                    .as_ref()
                    .map(|root| {
                        crate::folder_browser::scope::pin_directory(
                            path.parent().expect("absolute file"),
                            root,
                        )
                    })
                    .transpose()?;
                reports::export(&report, &path, &cancel)
            };
            match log {
                Some(log) => log.request("crash-report-export", |_| save()),
                None => save(),
            }
        }) {
            Ok(task) => self.write = Some(task),
            Err(e) => self.error = Some(message(&e)),
        }
    }
    /// Save as: the folder and name fields become the new file's path.
    fn save_as(&mut self, scope: Option<PathBuf>, log: Option<DiagnosticLog>) {
        let name = self.export_name.trim();
        if name.is_empty()
            || name.contains(['/', '\\'])
            || !Path::new(name)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("json"))
        {
            self.error = Some("Use a file name ending in .json.".into());
            self.feedback_reveal = true;
            return;
        }
        self.export_path = Path::new(self.export_dir.trim())
            .join(name)
            .display()
            .to_string();
        self.save(scope, log);
    }
    /// Opens Save as with a suggested name; the folder stays as last chosen.
    fn start_export(&mut self, scope: Option<&Path>) {
        let Some(report) = self.selected() else { return };
        let name = default_name(report);
        if self.export_dir.trim().is_empty() {
            self.export_dir = crate::folder_browser::save_folder(scope);
        }
        self.export_name = name;
        self.export_open = true;
        self.error = None;
        self.notice = None;
    }
    /// Opens the dialog, on `id` when given.
    pub(crate) fn show(&mut self, id: Option<String>) {
        self.open = true;
        self.refresh_requested = true;
        self.export_open = false;
        self.recent = None;
        if id.is_some() {
            self.selected = id;
        }
    }
    pub(crate) fn picked_folder(&mut self, folder: &Path) {
        self.export_dir = crate::views::display_path(folder);
        self.error = None;
    }
    pub(crate) fn poll(&mut self) {
        if let Some(result) = self.read.as_mut().and_then(ReadTask::try_finish) {
            self.read = None;
            let startup = std::mem::take(&mut self.startup_read);
            match result {
                Ok(snapshot) => {
                    if startup && !self.open {
                        self.recent = recent(&snapshot, self.started_ms);
                    }
                    self.snapshot = snapshot;
                    if self.selected().is_none() {
                        self.selected = self.snapshot.reports.first().map(|r| r.id.clone());
                    }
                }
                Err(_) if startup => {}
                Err(Error::Cancelled) => {
                    self.notice = Some("Read stopped. Previous reports retained.".into())
                }
                Err(e) => {
                    self.error = Some(message(&e));
                    self.read_failed = true;
                }
            }
        }
        if let Some(result) = self.write.as_mut().and_then(WriteTask::try_finish) {
            self.write = None;
            self.stopping = false;
            self.feedback_reveal = true;
            match result {
                Ok(()) => {
                    self.notice = Some("Report saved. Nothing was uploaded.".into());
                    self.export_open = false;
                }
                Err(Error::Cancelled) => {
                    self.notice = Some(
                        "Export stopped before publication. No destination file was created."
                            .into(),
                    )
                }
                Err(e) => self.error = Some(message(&e)),
            }
        }
    }
}

fn app_name(component: Component) -> &'static str {
    match component {
        Component::Desktop => "Rocket",
        Component::Cli => "Rocket CLI",
        Component::Mcp => "MCP server",
    }
}
fn title(r: &Report) -> String {
    match r.kind {
        Kind::WindowsException => format!("{} stopped", app_name(r.component)),
        Kind::RustPanic => format!("{} internal error", app_name(r.component)),
    }
}
fn kind_label(r: &Report) -> &'static str {
    match r.kind {
        Kind::WindowsException => "Windows exception",
        Kind::RustPanic => "Panic",
    }
}
/// Names for common Windows exception codes; others show only the number.
fn exception_name(code: u32) -> Option<&'static str> {
    Some(match code {
        0xC000_0005 => "Access violation",
        0xC000_00FD => "Stack overflow",
        0xC000_0409 => "Stack buffer overrun",
        0xC000_0374 => "Heap corruption",
        0xC000_001D => "Illegal instruction",
        0xC000_0096 => "Privileged instruction",
        0xC000_0094 => "Integer division by zero",
        0xE06D_7363 => "C++ exception",
        0x8000_0003 => "Breakpoint",
        _ => return None,
    })
}
/// "22:49" today, "Monday, 15:12" this week, else the date and time.
fn when(ms: u64, now: u64) -> String {
    let d = day(ms, now);
    if d == "Today" {
        clock(ms)
    } else {
        format!("{d}, {}", clock(ms))
    }
}
fn detail_line(r: &Report, now: u64) -> String {
    let what = match r.kind {
        Kind::WindowsException => exception_name(r.exception_code).unwrap_or("Windows exception"),
        Kind::RustPanic => "Panic",
    };
    format!("{what} · {}", when(r.time_ms, now))
}
fn system(r: &Report) -> String {
    let os = match r.os {
        "windows" => "Windows",
        "linux" => "Linux",
        "macos" => "macOS",
        other => other,
    };
    let arch = match r.arch {
        "x86_64" => "x64",
        "aarch64" => "ARM64",
        other => other,
    };
    format!("{os} · {arch}")
}
fn default_name(r: &Report) -> String {
    let t = local(r.time_ms);
    format!(
        "rocket-crash-{:04}-{:02}-{:02}-{:02}{:02}.json",
        t.year, t.month, t.day, t.hour, t.minute
    )
}
fn offsets(r: &Report) -> Vec<String> {
    let mut lines = Vec::new();
    if r.kind == Kind::WindowsException {
        lines.push(format!("{} + 0x{:x}", r.fault.module_name(), r.fault.offset));
    }
    lines.extend(r.frames.iter().map(|f| format!("{} + 0x{:x}", f.module_name(), f.offset)));
    lines
}
/// The newest report from an app with a window, recorded shortly before this start.
fn recent(snapshot: &Snapshot, started: u64) -> Option<String> {
    snapshot
        .reports
        .iter()
        .filter(|r| {
            r.component != Component::Cli
                && r.time_ms <= started
                && started - r.time_ms <= RECENT_MS
        })
        .max_by_key(|r| r.time_ms)
        .map(|r| r.id.clone())
}

#[derive(Default)]
struct Actions {
    copy: bool,
    export: bool,
    save: bool,
    cancel_export: bool,
    browse: bool,
    open_folder: bool,
    refresh: bool,
    stop_read: bool,
}

impl Rocket {
    pub(crate) fn crash_reports_ui(&mut self, ctx: &Context) {
        let mut s = std::mem::take(&mut self.crash_reports);
        s.poll();
        let log = self.backend.as_ref().map(|b| b.log.clone());
        s.check_startup(ctx, log.clone());
        if !s.open {
            if let Some(id) = recent_notice(ctx, &mut s, &mut self.frame_widgets) {
                s.show(Some(id));
            }
            self.crash_reports = s;
            return;
        }
        if s.refresh_requested {
            s.load(ctx, log.clone());
        }
        let mut act = Actions::default();
        let now = now_ms();
        let response = pages::review_at(
            ctx,
            "crash-reports-review",
            940.0,
            ReviewHeader {
                image: Icon::Warning,
                title: "Crash reports",
                subtitle: "Saved on this PC · never uploaded",
                danger: false,
            },
            &mut self.frame_widgets,
            "crash-scroll",
            true,
            |ui, part, widgets| match part {
                Part::Body => body(ui, &mut s, widgets, &mut act, now),
                Part::Footer => footer(ui, &mut s, widgets, &mut act),
            },
        );
        // Automation closes this dialog by its own name, as before the review frame.
        if let Some(close) = self.frame_widgets.get("dialog-close").copied() {
            self.frame_widgets.insert("crash-close".into(), close);
        }
        let scope = self.submodule_fixture_root.clone();
        if act.copy && let Some(report) = s.selected() {
            ctx.copy_text(report.json());
            s.notice = Some("Report copied. Review it before sharing.".into());
            s.error = None;
        }
        if act.export {
            s.start_export(scope.as_deref());
        }
        if act.save {
            s.save_as(scope.clone(), log.clone());
        }
        if act.cancel_export {
            if let Some(task) = &s.write {
                task.cancel();
                s.stopping = true;
            } else {
                s.export_open = false;
                s.error = None;
            }
        }
        if act.refresh {
            s.refresh_requested = true;
        }
        if act.stop_read {
            s.read = None;
            s.notice = Some("Read stopped. Previous reports retained.".into());
        }
        if act.open_folder {
            let folder = s.directory.clone();
            self.show_folder(&folder);
        }
        // Esc and × stop a save first, then close Save as, then the dialog.
        if response.should_close() {
            if let Some(task) = &s.write {
                task.cancel();
                s.stopping = true;
            } else if s.export_open && !response.close_clicked() {
                s.export_open = false;
                s.error = None;
            } else {
                s.open = false;
                s.read = None;
                s.refresh_requested = false;
                s.export_open = false;
            }
        }
        if s.read.is_some() || s.exporting() || s.refresh_requested {
            ctx.request_repaint_after(std::time::Duration::from_millis(25));
        }
        let browse = act.browse.then(|| s.export_dir.clone());
        self.crash_reports = s;
        if let Some(current) = browse {
            self.browse(PickTarget::CrashReportFolder, &current);
        }
    }
}

/// The next-start notice; returns the report to open.
fn recent_notice(ctx: &Context, s: &mut State, widgets: &mut Widgets) -> Option<String> {
    let id = s.recent.clone()?;
    let Some(report) = s.snapshot.reports.iter().find(|r| r.id == id) else {
        s.recent = None;
        return None;
    };
    let minutes = s.started_ms.saturating_sub(report.time_ms) / 60_000;
    let ago = match minutes {
        0 => "moments ago".to_owned(),
        1 => "1 minute ago".to_owned(),
        n => format!("{n} minutes ago"),
    };
    let text = match report.kind {
        Kind::WindowsException => format!("{} closed unexpectedly {ago}.", app_name(report.component)),
        Kind::RustPanic => format!("{} recorded an internal error {ago}.", app_name(report.component)),
    };
    let image = if report.kind == Kind::WindowsException { Icon::Blocked } else { Icon::Warning };
    let color = if report.kind == Kind::WindowsException { RED() } else { AMBER() };
    let mut open = None;
    Area::new(Id::new("crash-recent-notice"))
        .order(Order::Foreground)
        .anchor(Align2::RIGHT_BOTTOM, vec2(-16.0, -40.0))
        .show(ctx, |ui| {
            pages::modal_frame()
                .inner_margin(Margin {
                    left: 14,
                    right: 10,
                    top: 10,
                    bottom: 10,
                })
                .show(ui, |ui| {
                    ui.set_max_width(440.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        let (rect, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                        icon(ui.painter(), rect, image, color);
                        ui.add(Label::new(RichText::new(text).color(TEXT())).wrap());
                        let r = ui.button("View report");
                        pages::control(widgets, "crash-toast-view", &r, ui.clip_rect());
                        if r.clicked() {
                            open = Some(id.clone());
                        }
                        let r = crate::chrome::icon_button(ui, Icon::Close, true, false, 24.0)
                            .on_hover_text("Dismiss");
                        pages::control(widgets, "crash-toast-dismiss", &r, ui.clip_rect());
                        if r.clicked() {
                            s.recent = None;
                        }
                    });
                });
        });
    open
}

fn feedback(ui: &mut Ui, s: &mut State, widgets: &mut Widgets, color: Color32, text: &str, retry: bool) -> bool {
    let mut clicked = false;
    let frame = Frame::new()
        .fill(color.gamma_multiply(0.08))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.35)))
        .corner_radius(8)
        .inner_margin(Margin {
            left: 12,
            right: 8,
            top: 6,
            bottom: 6,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if retry {
                        let r = ui.button("Retry");
                        pages::control(widgets, "crash-retry", &r, ui.clip_rect());
                        clicked = r.clicked();
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.add(Label::new(RichText::new(text).size(12.5).color(color)).wrap());
                    });
                });
            });
        });
    if s.feedback_reveal {
        frame.response.scroll_to_me(Some(Align::TOP));
        s.feedback_reveal = false;
    }
    pages::control(widgets, "crash-feedback", &frame.response, ui.clip_rect());
    clicked
}

fn body(ui: &mut Ui, s: &mut State, widgets: &mut Widgets, act: &mut Actions, now: u64) {
    if let Some(error) = s.error.clone() {
        let retry = s.read_failed && s.read.is_none() && !s.exporting();
        act.refresh |= feedback(ui, s, widgets, RED(), &error, retry);
    } else if let Some(notice) = s.notice.clone() {
        feedback(ui, s, widgets, BLUE(), &notice, false);
    }
    if reports::capture_error().is_some() {
        pages::callout(
            ui,
            AMBER(),
            "Crash recording is off for this session. Check that Rocket can write to its data folder.",
        );
    }
    if s.snapshot.damaged > 0 {
        let n = s.snapshot.damaged;
        pages::callout(
            ui,
            AMBER(),
            &format!(
                "{n} damaged {} skipped. The others are shown.",
                if n == 1 { "record was" } else { "records were" }
            ),
        );
    }
    let loading = s.read.is_some() || s.refresh_requested;
    if s.snapshot.reports.is_empty() {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 180.0), Sense::hover());
        let mut empty = ui.new_child(
            UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::top_down(Align::Center)),
        );
        empty.add_space(46.0);
        if loading {
            empty.add(Spinner::new().size(18.0));
            empty.label(RichText::new("Reading local reports").color(MUTED()));
        } else if s.error.is_none() {
            let (mark, _) = empty.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
            icon(empty.painter(), mark, Icon::Check, GREEN());
            empty.label(RichText::new("No crash reports").font(semibold(14.0)));
            empty.label(RichText::new("Rocket hasn't recorded a crash on this PC").color(MUTED()));
        }
        return;
    }
    let height = (ui.ctx().content_rect().height() - 330.0).clamp(220.0, 520.0);
    if ui.available_width() < SPLIT_WIDTH {
        picker(ui, s, widgets, act, now, loading);
        ui.add_space(4.0);
        ui.painter().hline(
            ui.available_rect_before_wrap().x_range(),
            ui.cursor().top(),
            Stroke::new(1.0, HAIRLINE()),
        );
        ui.add_space(6.0);
        details(ui, s, widgets, now);
        return;
    }
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.vertical(|ui| {
            ui.set_width(300.0);
            list(ui, s, widgets, act, now, loading, height);
        });
        ui.add_space(10.0);
        let (line, _) = ui.allocate_exact_size(vec2(1.0, height), Sense::hover());
        ui.painter()
            .vline(line.center().x, line.y_range(), Stroke::new(1.0, HAIRLINE()));
        ui.add_space(16.0);
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            let pane = ScrollArea::vertical()
                .id_salt("crash-details")
                .max_height(height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_width(ui.available_width() - 6.0);
                    details(ui, s, widgets, now);
                });
            pages::region(widgets, "crash-details", pane.inner_rect.intersect(ui.clip_rect()), true);
        });
    });
}

fn list_head(ui: &mut Ui, s: &State, widgets: &mut Widgets, act: &mut Actions, loading: bool) {
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 28.0),
        Layout::right_to_left(Align::Center),
        |ui| {
            if loading {
                let r = ui.button("Stop");
                pages::control(widgets, "crash-stop-read", &r, ui.clip_rect());
                act.stop_read |= r.clicked() && s.read.is_some();
                ui.add(Spinner::new().size(14.0));
            } else {
                let r = crate::chrome::icon_button(ui, Icon::Refresh, !s.exporting(), false, 26.0)
                    .on_hover_text("Read again");
                pages::control(widgets, "crash-refresh", &r, ui.clip_rect());
                act.refresh |= r.clicked();
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(RichText::new("REPORTS").size(11.0).color(HINT()).strong());
                ui.label(
                    RichText::new(s.snapshot.reports.len().to_string())
                        .size(12.0)
                        .color(FAINT()),
                );
            });
        },
    );
}

fn list(ui: &mut Ui, s: &mut State, widgets: &mut Widgets, act: &mut Actions, now: u64, loading: bool, height: f32) {
    list_head(ui, s, widgets, act, loading);
    let list = ScrollArea::vertical()
        .id_salt("crash-list")
        .max_height(height - 32.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_width(ui.available_width() - 4.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            let mut group = "";
            for (i, r) in s.snapshot.reports.iter().enumerate() {
                let today = day(r.time_ms, now) == "Today";
                let label = if today { "TODAY" } else { "EARLIER" };
                if label != group {
                    group = label;
                    ui.add_space(if i == 0 { 2.0 } else { 8.0 });
                    ui.label(RichText::new(label).size(11.0).color(FAINT()).strong());
                }
                let selected = s.selected.as_ref() == Some(&r.id);
                let title = title(r);
                let detail = detail_line(r, now);
                let response = pages::row(
                    ui,
                    Row {
                        image: if r.kind == Kind::WindowsException { Icon::Blocked } else { Icon::Warning },
                        title: &title,
                        detail: &detail,
                        pills: vec![(r.version.clone(), HINT())],
                        indent: 0.0,
                    },
                    selected,
                    !s.exporting(),
                );
                pages::control(widgets, format!("crash-report-{i}"), &response, ui.clip_rect());
                if response.clicked() {
                    s.selected = Some(r.id.clone());
                    s.export_open = false;
                    s.notice = None;
                    s.error = None;
                }
            }
        });
    pages::region(widgets, "crash-list", list.inner_rect.intersect(ui.clip_rect()), true);
}

/// The compact window's list: one picker above the details.
fn picker(ui: &mut Ui, s: &mut State, widgets: &mut Widgets, act: &mut Actions, now: u64, loading: bool) {
    list_head(ui, s, widgets, act, loading);
    let count = s.snapshot.reports.len();
    let index = s
        .selected
        .as_ref()
        .and_then(|id| s.snapshot.reports.iter().position(|r| &r.id == id))
        .unwrap_or(0);
    let current = s.snapshot.reports.get(index).map_or_else(String::new, |r| {
        format!("{} · {} · {} of {count}", title(r), when(r.time_ms, now), index + 1)
    });
    let layer = ui.layer_id();
    let response = ComboBox::from_id_salt("crash-picker")
        .selected_text(current)
        .truncate()
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            ui.ctx().set_sublayer(layer, ui.layer_id());
            for (i, r) in s.snapshot.reports.iter().enumerate() {
                let label = format!("{} · {}", title(r), detail_line(r, now));
                let selected = s.selected.as_ref() == Some(&r.id);
                let row = ui.selectable_label(selected, label);
                pages::control(widgets, format!("crash-report-{i}"), &row, ui.clip_rect());
                if row.clicked() {
                    s.selected = Some(r.id.clone());
                    s.export_open = false;
                    s.notice = None;
                    s.error = None;
                    ui.close();
                }
            }
        })
        .response;
    pages::control(widgets, "crash-picker", &response, ui.clip_rect());
}

fn fact(ui: &mut Ui, key: &str, value: impl FnOnce(&mut Ui)) {
    pages::key_value(ui, key, 92.0, |ui| {
        // A row that wraps its parts is as tall as one plain line, so the facts keep
        // even spacing.
        ui.spacing_mut().interact_size.y = 18.0;
        value(ui)
    });
}

fn details(ui: &mut Ui, s: &mut State, widgets: &mut Widgets, now: u64) {
    let Some(r) = s.selected().cloned() else {
        ui.label(RichText::new("Select a report").color(MUTED()));
        return;
    };
    ui.spacing_mut().item_spacing.y = 9.0;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (mark, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
        let exception = r.kind == Kind::WindowsException;
        icon(
            ui.painter(),
            mark,
            if exception { Icon::Blocked } else { Icon::Warning },
            if exception { RED() } else { AMBER() },
        );
        ui.label(RichText::new(title(&r)).font(semibold(16.0)));
        pages::pill(ui, kind_label(&r), if exception { RED() } else { AMBER() });
        pages::pill(ui, &format!("Rocket {}", r.version), HINT());
    });
    ui.add(
        Label::new(
            RichText::new(match r.kind {
                Kind::WindowsException => format!(
                    "Windows ended {} after an error it could not handle.",
                    app_name(r.component)
                ),
                Kind::RustPanic => {
                    "A Rocket thread stopped with an internal error. Rocket may have kept running."
                        .to_owned()
                }
            })
            .color(TEXT_SOFT()),
        )
        .wrap(),
    );
    if r.kind == Kind::WindowsException && r.fault.module_name() == "opengl32" {
        pages::callout(
            ui,
            AMBER(),
            "It failed in OpenGL (opengl32), which draws Rocket's window. Check for a graphics driver update.",
        );
    }
    ui.painter().hline(
        ui.available_rect_before_wrap().x_range(),
        ui.cursor().top(),
        Stroke::new(1.0, HAIRLINE()),
    );
    ui.add_space(2.0);
    fact(ui, "When", |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(started(r.time_ms, now));
            if r.time_ms <= 253_402_300_799_999 {
                ui.label(
                    RichText::new(timestamp(r.time_ms).get(11..).unwrap_or_default())
                        .color(FAINT()),
                );
            }
        });
    });
    fact(ui, "App", |ui| {
        ui.label(format!("{} ({})", app_name(r.component), r.component.code()));
    });
    fact(ui, "Version", |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(&r.version);
            if r.version != env!("CARGO_PKG_VERSION") {
                ui.label(
                    RichText::new(format!("· you now run {}", env!("CARGO_PKG_VERSION")))
                        .color(FAINT()),
                );
            }
        });
    });
    fact(ui, "System", |ui| {
        ui.label(system(&r));
    });
    if r.kind == Kind::WindowsException {
        fact(ui, "Error", |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("0x{:08X}", r.exception_code)).monospace());
                ui.label(exception_name(r.exception_code).unwrap_or("Not a standard Windows code"));
            });
        });
        fact(ui, "Failed in", |ui| {
            ui.label(
                RichText::new(format!("{} + 0x{:x}", r.fault.module_name(), r.fault.offset))
                    .monospace(),
            );
        });
    }
    fact(ui, "In code", |ui| {
        if r.source_file == "-" {
            ui.label(RichText::new("Not recorded").color(MUTED()));
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.label(RichText::new(&r.source_file).monospace());
                ui.label(format!(", line {}", r.line));
            });
        }
    });
    fact(ui, "Process", |ui| {
        ui.label(format!("{} · thread {}", r.pid, r.thread));
    });
    let lines = offsets(&r);
    if lines.is_empty() {
        return;
    }
    ui.add_space(4.0);
    Frame::new()
        .fill(VIEW_BG())
        .stroke(Stroke::new(1.0, CARD_STROKE()))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let r = pages::disclosure(ui, s.offsets_open, &format!("Code offsets · {}", lines.len()));
                pages::control(widgets, "crash-offsets", &r, ui.clip_rect());
                if r.clicked() {
                    s.offsets_open = !s.offsets_open;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let r = crate::chrome::icon_button(ui, Icon::Copy, true, false, 26.0)
                        .on_hover_text("Copy offsets");
                    pages::control(widgets, "crash-copy-offsets", &r, ui.clip_rect());
                    if r.clicked() {
                        ui.ctx().copy_text(lines.join("\n"));
                        s.notice = Some("Code offsets copied.".into());
                    }
                });
            });
            if s.offsets_open {
                ui.spacing_mut().item_spacing.y = 2.0;
                for (i, line) in lines.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            [22.0, 16.0],
                            Label::new(RichText::new(i.to_string()).monospace().color(DIM())),
                        );
                        ui.label(RichText::new(line).monospace().color(TEXT_SOFT()));
                    });
                }
            }
        });
}

fn footer(ui: &mut Ui, s: &mut State, widgets: &mut Widgets, act: &mut Actions) {
    if s.export_open {
        let busy = s.exporting();
        let ready = !busy && !s.export_dir.trim().is_empty() && !s.export_name.trim().is_empty();
        let r = pages::primary(ui, ready, if busy { "Saving" } else { "Save" });
        pages::control(widgets, "crash-export-save", &r, ui.clip_rect());
        act.save |= r.clicked();
        let r = if busy {
            ui.add_enabled(!s.stopping, Button::new(if s.stopping { "Stopping" } else { "Stop" }))
        } else {
            crate::chrome::icon_button(ui, Icon::Close, true, false, 28.0).on_hover_text("Don't save")
        };
        pages::control(widgets, "crash-export-cancel", &r, ui.clip_rect());
        act.cancel_export |= r.clicked();
        let width = ((ui.available_width() - 210.0) / 2.0).clamp(110.0, 260.0);
        let r = ui.add_enabled(
            !busy,
            TextEdit::singleline(&mut s.export_name)
                .id_salt("crash-export-name")
                .desired_width(width)
                .hint_text("File name.json"),
        );
        pages::control(widgets, "crash-export-name", &r, ui.clip_rect());
        let r = ui.add_enabled(!busy, Button::new("Browse"));
        pages::control(widgets, "crash-export-browse", &r, ui.clip_rect());
        act.browse |= r.clicked();
        let r = ui.add_enabled(
            !busy,
            TextEdit::singleline(&mut s.export_dir)
                .id_salt("crash-export-folder")
                .desired_width(width)
                .hint_text("Folder"),
        );
        pages::control(widgets, "crash-export-folder", &r, ui.clip_rect());
        if busy {
            ui.add(Spinner::new().size(14.0));
        }
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            ui.add(Label::new(RichText::new("Save a copy").font(semibold(13.0))).truncate());
        });
        return;
    }
    let ready = s.selected().is_some() && !s.exporting() && s.read.is_none();
    let r = pages::primary(ui, ready, "Copy report");
    pages::control(widgets, "crash-copy", &r, ui.clip_rect());
    act.copy |= r.clicked();
    let r = ui.add_enabled(ready, Button::new("Save as"));
    pages::control(widgets, "crash-export", &r, ui.clip_rect());
    act.export |= r.clicked();
    let r = ui.button("Open folder");
    pages::control(widgets, "crash-open-folder", &r, ui.clip_rect());
    act.open_folder |= r.clicked();
    pages::footer_status(ui, false, "No repository data, messages or memory");
}
