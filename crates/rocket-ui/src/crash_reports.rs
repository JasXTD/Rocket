//! Local report review; all I/O runs outside the frame and export never overwrites.
use crate::{app::Rocket, pages, style::*};
use eframe::egui::*;
use rocket_git::{
    DiagnosticLog, Error, ReadTask, WriteTask,
    crash_reports::{self as reports, Kind, Report, Snapshot},
};
use std::path::PathBuf;

#[cfg(test)]
#[path = "crash_reports_tests.rs"]
mod tests;

#[derive(Default)]
pub struct State {
    pub open: bool,
    pub refresh_requested: bool,
    pub directory: PathBuf,
    pub export_path: String,
    pub export_open: bool,
    export_reveal: bool,
    feedback_reveal: bool,
    snapshot: Snapshot,
    selected: Option<String>,
    read: Option<ReadTask<Snapshot>>,
    write: Option<WriteTask<()>>,
    error: Option<String>,
    notice: Option<String>,
    stopping: bool,
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
impl State {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
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
            "reports":self.snapshot.reports.iter().map(|r|serde_json::json!({"id":r.id,"kind":r.kind.code(),"component":r.component.code(),"version":r.version})).collect::<Vec<_>>(),
            "selected":self.selected().map(|r|serde_json::from_str::<serde_json::Value>(&r.json()).expect("typed report serializes")),"export_open":self.export_open,"export_path":self.export_path})
    }
    fn load(&mut self, ctx: &Context, log: Option<DiagnosticLog>) {
        if self.exporting() {
            return;
        }
        self.read = None;
        self.error = None;
        self.notice = None;
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
    pub(crate) fn poll(&mut self) {
        if let Some(result) = self.read.as_mut().and_then(ReadTask::try_finish) {
            self.read = None;
            match result {
                Ok(snapshot) => {
                    self.snapshot = snapshot;
                    if self.selected().is_none() {
                        self.selected = self.snapshot.reports.first().map(|r| r.id.clone());
                    }
                }
                Err(Error::Cancelled) => {
                    self.notice = Some("Read stopped. Previous reports retained.".into())
                }
                Err(e) => self.error = Some(message(&e)),
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
impl Rocket {
    pub(crate) fn crash_reports_ui(&mut self, ctx: &Context) {
        let mut s = std::mem::take(&mut self.crash_reports);
        s.poll();
        if !s.open {
            self.crash_reports = s;
            return;
        }
        let log = self.backend.as_ref().map(|b| b.log.clone());
        if s.refresh_requested {
            s.load(ctx, log.clone());
        }
        let mut close = false;
        let mut save = false;
        Modal::new(Id::new("crash-reports-review")).frame(pages::modal_frame().inner_margin(18)).show(ctx,|ui|{
            ui.set_width((ctx.content_rect().width()-84.0).clamp(280.0,704.0));
            ui.spacing_mut().item_spacing.y=12.0;
            ui.horizontal(|ui|{
                ui.heading("Crash reports");
                ui.with_layout(Layout::right_to_left(Align::Center),|ui|{
                    let r=ui.add_enabled(!s.exporting(),Button::new("Close"));self.frame_widgets.insert("crash-close".into(),(&r).into());close=r.clicked();
                });
            });
            ui.label(RichText::new("SAVED ON THIS PC · NOTHING UPLOADED").color(BLUE()).size(11.0));
            let area=ScrollArea::vertical().id_salt("crash-report-body").max_height((ctx.content_rect().height()-266.0).max(140.0)).auto_shrink([false,true]).show(ui,|ui|{
                ui.set_width(ui.available_width());
                ui.label("Review what Rocket recorded, then copy a report or save a file to share.");
                if reports::capture_error().is_some(){ui.colored_label(AMBER(),"Crash capture is unavailable for this session. Existing reports are still available. Check local storage permissions.");}
                ui.horizontal(|ui|{
                    let r=ui.add_enabled(!s.exporting()&&s.read.is_none(),Button::new("Refresh"));pages::control(&mut self.frame_widgets,"crash-refresh",&r,ui.clip_rect());if r.clicked(){s.refresh_requested=true;}
                    if s.read.is_some(){ui.spinner();ui.label("Reading local reports");let r=ui.button("Stop");pages::control(&mut self.frame_widgets,"crash-stop-read",&r,ui.clip_rect());if r.clicked(){s.read=None;s.notice=Some("Read stopped. Previous reports retained.".into());}}
                });
                if let Some(error)=&s.error{let r=ui.colored_label(RED(),error);pages::control(&mut self.frame_widgets,"crash-feedback",&r,ui.clip_rect());if s.feedback_reveal{r.scroll_to_me(Some(Align::TOP));s.feedback_reveal=false;}}
                if let Some(notice)=&s.notice{let r=ui.colored_label(BLUE(),notice);pages::control(&mut self.frame_widgets,"crash-feedback",&r,ui.clip_rect());if s.feedback_reveal{r.scroll_to_me(Some(Align::TOP));s.feedback_reveal=false;}}
                if s.snapshot.damaged>0{ui.colored_label(AMBER(),format!("{} damaged or incomplete records were skipped. Valid reports remain available.",s.snapshot.damaged));}
                if s.snapshot.reports.is_empty()&&s.read.is_none()&&s.error.is_none(){
                    pages::well(ui,|ui|{ui.label(RichText::new("No crash reports").strong());ui.label("Reports appear here after Rocket records a Rust panic or an unhandled Windows exception.");});
                }
                for (i,r) in s.snapshot.reports.iter().enumerate(){
                    let title=format!("{} · {} · {}",if r.kind==Kind::RustPanic{"Rust panic"}else{"Windows exception"},r.component.code(),crate::activity_history::timestamp(r.time_ms));
                    let response=ui.add_enabled(!s.exporting(),Button::new(title).selected(s.selected.as_ref()==Some(&r.id)).wrap());
                    pages::control(&mut self.frame_widgets,format!("crash-report-{i}"),&response,ui.clip_rect());
                    if response.clicked(){s.selected=Some(r.id.clone());s.export_open=false;s.notice=None;}
                }
                if let Some(report)=s.selected(){
                    ui.separator();
                    ui.label(RichText::new(if report.kind==Kind::RustPanic{"Rust panic"}else{"Unhandled Windows exception"}).strong().size(17.0));
                    ui.label(if report.kind==Kind::RustPanic{"A thread panicked. It may have been caught; this does not establish that the whole app stopped."}else{"Windows reported an unhandled exception. Rocket did not attempt to resume the faulting process."});
                    Grid::new("crash-fields").num_columns(2).spacing([18.0,7.0]).show(ui,|ui|{
                        for (name,value) in [("Version",report.version.clone()),("Platform",format!("{} {}",report.os,report.arch)),("Component",report.component.code().into()),("Recorded",crate::activity_history::timestamp(report.time_ms)),("Code location",if report.source_file=="-"{"Unavailable".into()}else{format!("{}:{}:{}",report.source_file,report.line,report.column)}),
                            ("Exception",if report.exception_code==0{"Not applicable".into()}else{format!("0x{:08X}",report.exception_code)})]{ui.colored_label(MUTED(),name);ui.label(value);ui.end_row();}
                    });
                    CollapsingHeader::new("Code offsets").show(ui,|ui|{ui.label("Offsets identify code in this build. They contain no captured memory.");ui.monospace(format!("Fault: {} + 0x{:x}",report.fault.module_name(),report.fault.offset));for f in &report.frames{ui.monospace(format!("{} + 0x{:x}",f.module_name(),f.offset));}});
                }
                if s.export_open{
                    ui.separator();ui.label(RichText::new("Save a copy").strong());ui.label("Choose a new .json file. Existing files are never replaced.");
                    let r=ui.add_enabled(!s.exporting(),TextEdit::singleline(&mut s.export_path).desired_width(f32::INFINITY).hint_text("Full path to a new report file"));pages::control(&mut self.frame_widgets,"crash-export-path",&r,ui.clip_rect());
                    ui.horizontal_wrapped(|ui|{
                        let r=ui.add_enabled(!s.exporting(),Button::new("Save report"));pages::control(&mut self.frame_widgets,"crash-export-save",&r,ui.clip_rect());save=r.clicked();
                        let r=ui.add_enabled(!s.stopping,Button::new(if s.exporting(){"Stop export"}else{"Cancel export"}));pages::control(&mut self.frame_widgets,"crash-export-cancel",&r,ui.clip_rect());
                        if r.clicked(){if let Some(task)=&s.write{task.cancel();s.stopping=true;}else{s.export_open=false;s.error=None;s.notice=Some("Export cancelled. No file was written.".into());s.feedback_reveal=true;}}
                        if s.exporting(){ui.spinner();ui.label(if s.stopping{"Stopping · checking whether the file was saved"}else{"Saving report"});}
                    });
                    ui.add_space(4.0);
                    if s.export_reveal{ui.scroll_to_cursor(Some(Align::BOTTOM));s.export_reveal=false;}
                }
            });
            self.frame_widgets.insert("crash-scroll".into(),crate::app::UiControl{rect:area.inner_rect,enabled:true});
            ui.separator();ui.label(RichText::new("No repository contents, panic messages, credentials or memory dumps.").color(MUTED()).size(11.0));
            ui.horizontal_wrapped(|ui|{
                let ready=s.selected().is_some()&&!s.exporting()&&s.read.is_none();
                let r=ui.add_enabled(ready,Button::new("Copy report"));self.frame_widgets.insert("crash-copy".into(),(&r).into());
                if r.clicked()&&let Some(report)=s.selected(){ctx.copy_text(report.json());s.notice=Some("Report copied. Review it before sharing.".into());}
                let r=ui.add_enabled(ready,Button::new("Export report"));self.frame_widgets.insert("crash-export".into(),(&r).into());if r.clicked(){s.export_open=true;s.export_reveal=true;s.error=None;}
            });
        });
        if save {
            s.save(self.submodule_fixture_root.clone(), log);
        }
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            if let Some(task) = &s.write {
                task.cancel();
                s.stopping = true;
            } else if s.export_open {
                s.export_open = false;
                s.error = None;
                s.feedback_reveal = true;
                s.notice = Some("Export cancelled. No file was written.".into());
            } else {
                close = true;
            }
        }
        if close {
            s.open = false;
            s.read = None;
            s.refresh_requested = false;
            s.export_open = false;
        }
        if s.read.is_some() || s.exporting() || s.refresh_requested {
            ctx.request_repaint_after(std::time::Duration::from_millis(25));
        }
        self.crash_reports = s;
    }
}
