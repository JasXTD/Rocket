//! File → New repository and Clone repository. Each checks its folder on a worker as
//! you type, so the primary button turns on only for a folder Rocket will use; the
//! checks are the ones creating and cloning run, without writing anything.
use crate::{
    app::*,
    backend::user_error,
    folder_browser::{self, PickTarget},
    pages,
    preferences::keycap,
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// How long typing pauses before a folder is checked.
const SETTLE: Duration = Duration::from_millis(150);

/// What a folder check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// A new folder in an existing parent.
    New,
    /// New repository: an existing folder and its top-level entries, up to
    /// [`INIT_ENTRY_LIMIT`].
    Existing(usize),
    /// New repository: a repository, or a folder inside one; the repository to open.
    Repository(PathBuf),
    /// Clone: something is already there.
    Exists,
    /// A folder above it does not exist.
    MissingParent,
    /// Any other refusal, in words.
    Refused(String),
}

/// The folder fields of an open New repository or Clone dialog, and their check.
#[derive(Default)]
pub struct Form {
    /// Clone: the folder the new folder goes into.
    pub parent: String,
    /// Clone: the new folder's name; it follows the source until edited.
    pub name: String,
    pub name_edited: bool,
    checked: Option<(PathBuf, Status)>,
    /// A path waiting for typing to settle, and since when.
    wanted: Option<(PathBuf, Instant)>,
    task: Option<(PathBuf, ReadTask<Status>)>,
    /// The source code folder the form started in.
    source: Option<PathBuf>,
    /// Its check on a worker, with the field's text then and its usual start.
    start: Option<(ReadTask<bool>, String, String)>,
}

impl Form {
    /// A check is waiting or running.
    pub fn checking(&self) -> bool {
        self.wanted.is_some() || self.task.is_some()
    }

    /// A form starting in the source code folder, which is shown at once and checked
    /// on a worker: a drive may be slow to answer.
    fn starting_in(folder: PathBuf, shown: String, usual: String) -> Self {
        let target = folder.clone();
        Self {
            source: Some(folder),
            start: ReadTask::spawn(move |_| Ok(target.is_dir())).ok().map(|task| (task, shown, usual)),
            ..Default::default()
        }
    }

    /// The text for a field still showing a source code folder found to be missing.
    fn missing_source(&mut self, ctx: &Context, field: &str) -> Option<String> {
        let (task, shown, usual) = self.start.as_mut()?;
        let Some(found) = task.try_finish() else {
            ctx.request_repaint_after(Duration::from_millis(30));
            return None;
        };
        let replacement = (!found.unwrap_or(false) && shown == field).then(|| std::mem::take(usual));
        self.start = None;
        if replacement.is_some() {
            self.source = None;
        }
        replacement
    }

    #[cfg(all(test, windows))]
    pub(crate) fn start_pending(&self) -> bool {
        self.start.is_some()
    }

    /// The repository New repository offers to open instead, once checked.
    pub fn repository(&self) -> Option<&Path> {
        match &self.checked {
            Some((_, Status::Repository(root))) => Some(root),
            _ => None,
        }
    }

    /// The status of `path`, checked with `check` on a worker once typing settles.
    /// `None` until the current path has been checked.
    pub(crate) fn status(
        &mut self,
        ctx: &Context,
        path: Option<PathBuf>,
        scope: Option<&Path>,
        check: fn(&Path) -> Status,
    ) -> Option<Status> {
        if let Some((checked, task)) = self.task.as_mut()
            && let Some(result) = task.try_finish()
        {
            let status = result.unwrap_or_else(|e| Status::Refused(user_error(&e)));
            self.checked = Some((checked.clone(), status));
            self.task = None;
        }
        let Some(path) = path else {
            self.wanted = None;
            self.task = None;
            return None;
        };
        if let Some((checked, status)) = &self.checked
            && *checked == path
        {
            return Some(status.clone());
        }
        // The MCP test host keeps a window's folders inside its test folder.
        if scope.is_some_and(|s| !folder_browser::inside(&path, s)) {
            let status =
                Status::Refused("Choose a folder inside the test repositories folder.".into());
            self.checked = Some((path, status.clone()));
            self.wanted = None;
            self.task = None;
            return Some(status);
        }
        if self.task.as_ref().is_some_and(|(p, _)| *p == path) {
            ctx.request_repaint_after(Duration::from_millis(30));
            return None;
        }
        match &self.wanted {
            Some((wanted, since)) if *wanted == path && since.elapsed() >= SETTLE => {
                self.wanted = None;
                let target = path.clone();
                match ReadTask::spawn(move |_| Ok(check(&target))) {
                    Ok(task) => self.task = Some((path, task)),
                    Err(e) => self.checked = Some((path, Status::Refused(user_error(&e)))),
                }
                ctx.request_repaint_after(Duration::from_millis(30));
            }
            Some((wanted, since)) if *wanted == path => {
                ctx.request_repaint_after(SETTLE.saturating_sub(since.elapsed()));
            }
            _ => {
                // A new path: drop an older check and wait for typing to settle.
                self.task = None;
                self.wanted = Some((path, Instant::now()));
                ctx.request_repaint_after(SETTLE);
            }
        }
        None
    }
}

fn check_init(path: &Path) -> Status {
    match check_init_destination(path) {
        Ok(InitDestination::New) => Status::New,
        Ok(InitDestination::Existing { entries }) => Status::Existing(entries),
        Ok(InitDestination::Repository(root)) => Status::Repository(root),
        Ok(InitDestination::MissingParent) => Status::MissingParent,
        Err(e) => Status::Refused(user_error(&e)),
    }
}

fn check_clone(path: &Path) -> Status {
    match check_clone_destination(path) {
        Ok(CloneDestination::New(_)) => Status::New,
        Ok(CloneDestination::Exists) => Status::Exists,
        Ok(CloneDestination::MissingParent) => Status::MissingParent,
        Err(e) => Status::Refused(user_error(&e)),
    }
}

/// The folder name a clone source suggests: its last segment without `.git`.
pub(crate) fn suggested_name(source: &str) -> String {
    let last = source
        .trim()
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\', ':'])
        .next()
        .unwrap_or("");
    last.strip_suffix(".git").unwrap_or(last).to_owned()
}

enum Part {
    Body,
    Hint,
    Buttons,
    /// The header's close button, with whether it was clicked.
    Close(bool),
}

/// What the dialog asks for once it is drawn.
enum Next {
    Stay,
    Close,
    Submit,
    /// Keep: stop the running Keep.
    Stop,
    Browse(PickTarget),
    Open(PathBuf),
}

impl Rocket {
    /// File → New repository and Clone repository.
    pub(crate) fn repository_form_dialog(&mut self, ctx: &Context) {
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        let clone = matches!(dialog.kind, DialogKind::Clone);
        if let DialogKind::KeepPreview(id) = dialog.kind {
            self.keep_dialog(ctx, dialog, id);
            return;
        }
        if dialog.create.is_none() {
            let form = if clone { self.clone_form(&dialog) } else { self.init_form(&mut dialog) };
            dialog.create = Some(form);
        }
        let idle = self.write.is_none() && !self.quitting;
        let scope = self.browse_root.clone();
        let widgets = &mut self.frame_widgets;
        let backend = self.backend.as_ref();
        let Dialog {
            first,
            second,
            connection,
            create,
            ..
        } = &mut dialog;
        let form = create.as_mut().expect("form created above");
        let mut next = Next::Stay;
        let (response, ready) = if clone {
            let shown = form.parent.clone();
            if let Some(usual) = form.missing_source(ctx, &shown) {
                form.parent = usual;
            }
            if !form.name_edited {
                form.name = suggested_name(first);
            }
            let destination = (!form.parent.trim().is_empty() && !form.name.trim().is_empty())
                .then(|| Path::new(form.parent.trim()).join(form.name.trim()));
            // Shown in automation state, as before the parent and name were split.
            *second = destination.as_ref().map_or_else(String::new, |p| p.display().to_string());
            let status = form.status(ctx, destination.clone(), scope.as_deref(), check_clone);
            let ready = idle && status == Some(Status::New) && !first.trim().is_empty();
            let response = form_modal(
                ctx,
                "rocket-clone",
                Icon::Fetch,
                "Clone repository",
                "Download a repository into a new folder.",
                widgets,
                |ui, part, widgets| match part {
                    Part::Body => clone_body(ui, first, connection, form, backend, destination.as_deref(), status.as_ref(), widgets, &mut next),
                    Part::Hint => shortcut_hint(ui, "to clone"),
                    Part::Buttons => buttons(ui, ready, "Clone", widgets, &mut next),
                    Part::Close(clicked) => {
                        if clicked {
                            next = Next::Close;
                        }
                    }
                },
            );
            (response, ready)
        } else {
            if let Some(usual) = form.missing_source(ctx, first) {
                *first = usual;
            }
            let path = (!first.trim().is_empty()).then(|| PathBuf::from(first.trim()));
            // The source code folder holds projects; it isn't one until a name is added.
            let at_source = form.source.as_deref().zip(path.as_deref()).is_some_and(|(a, b)| crate::general::same_folder(a, b));
            let status = form.status(ctx, path, scope.as_deref(), check_init);
            let ready = idle
                && !at_source
                && matches!(status, Some(Status::New | Status::Existing(_)))
                && !second.trim().is_empty();
            let checking = form.checking();
            let response = form_modal(
                ctx,
                "rocket-new-repository",
                Icon::Plus,
                "New repository",
                "Start tracking a new or existing project with Git.",
                widgets,
                |ui, part, widgets| match part {
                    Part::Body => init_body(ui, first, second, status.as_ref(), checking, at_source, widgets, &mut next),
                    Part::Hint => shortcut_hint(ui, "to create"),
                    Part::Buttons => buttons(ui, ready, "Create repository", widgets, &mut next),
                    Part::Close(clicked) => {
                        if clicked {
                            next = Next::Close;
                        }
                    }
                },
            );
            (response, ready)
        };
        if response.should_close() {
            next = Next::Close;
        }
        // Ctrl+Enter takes the primary action, as in the other dialogs, but not while
        // a Browse picker sits on top.
        if matches!(next, Next::Stay)
            && ready
            && self.picker.is_none()
            && ctx.input_mut(|i| i.consume_key(crate::shortcuts::primary_modifiers(), Key::Enter))
        {
            next = Next::Submit;
        }
        match next {
            Next::Stay => self.dialog = Some(dialog),
            Next::Close => {}
            Next::Submit if clone => {
                let text = dialog.first.trim().to_owned();
                let source = if text.starts_with("https://") || text.starts_with("ssh://") {
                    CloneSource::Url(text)
                } else {
                    CloneSource::Local(PathBuf::from(text))
                };
                self.clone_repository(source, PathBuf::from(dialog.second.trim()));
                self.show_console = true;
            }
            Next::Submit => {
                self.init_repository(PathBuf::from(dialog.first.trim()), dialog.second.clone());
                self.show_console = true;
            }
            Next::Browse(target) => {
                let current = match target {
                    PickTarget::CloneParent => dialog.create.as_ref().map(|f| f.parent.clone()).unwrap_or_default(),
                    _ => dialog.first.clone(),
                };
                self.dialog = Some(dialog);
                self.browse(target, &current);
            }
            Next::Open(path) => self.open(path),
            Next::Stop => {}
        }
    }

    /// Keep a preview: choose the folder, then follow the download, copy and checkout.
    fn keep_dialog(&mut self, ctx: &Context, mut dialog: Dialog, id: u64) {
        let Some((name, estimate)) = self.tabs.iter().find(|t| t.id == id).and_then(|t| t.preview.as_ref()).map(|p| {
            let have = p.meter.status().size;
            let estimate = p
                .request
                .size_kb
                .map(|kb| kb.saturating_mul(1024).saturating_sub(have))
                .filter(|b| *b > 0);
            (p.request.name.clone(), estimate)
        }) else {
            // The preview's tab closed: nothing to keep.
            return;
        };
        if dialog.create.is_none() {
            let mut form = self.clone_form(&dialog);
            form.name = name.rsplit('/').next().unwrap_or(&name).to_owned();
            form.name_edited = true;
            dialog.create = Some(form);
        }
        let running = self
            .write
            .as_ref()
            .filter(|w| w.label == "keep-preview" && self.keeping_preview == Some(id))
            .map(|w| (w.progress.clone(), w.fraction, w.cancelling));
        let idle = self.write.is_none() && !self.quitting;
        let scope = self.browse_root.clone();
        let widgets = &mut self.frame_widgets;
        let Dialog { second, create, error, .. } = &mut dialog;
        let form = create.as_mut().expect("form created above");
        let shown = form.parent.clone();
        if let Some(usual) = form.missing_source(ctx, &shown) {
            form.parent = usual;
        }
        let destination = (!form.parent.trim().is_empty() && !form.name.trim().is_empty())
            .then(|| Path::new(form.parent.trim()).join(form.name.trim()));
        *second = destination.as_ref().map_or_else(String::new, |p| p.display().to_string());
        let status = form.status(ctx, destination.clone(), scope.as_deref(), check_clone);
        let ready = idle && running.is_none() && status == Some(Status::New);
        let mut next = Next::Stay;
        // Running, the dialog follows the Keep; after a failure, Try again resumes it.
        let failed = running.is_none() && error.is_some();
        let short = name.rsplit('/').next().unwrap_or(&name).to_owned();
        let (title, subtitle) = if running.is_some() {
            (format!("Keeping {short}"), destination.as_deref().map_or_else(String::new, crate::views::display_path))
        } else {
            ("Keep repository".to_owned(), format!("Make a normal clone of {name} in a folder you choose."))
        };
        let response = form_modal(ctx, "rocket-keep-preview", Icon::Bookmark, &title, &subtitle, widgets, |ui, part, widgets| match part {
            Part::Body => match &running {
                Some((progress, fraction, _)) => keep_progress(ui, progress, *fraction, destination.as_deref()),
                None if failed => {
                    let error = error.as_deref().unwrap_or_default().trim_end_matches('.');
                    pages::callout(ui, RED(), &format!("{error}. The preview is unchanged."));
                }
                None => {
                    destination_fields(ui, form, destination.as_deref(), status.as_ref(), widgets, &mut next);
                    ui.add_space(4.0);
                    let note = match estimate {
                        Some(bytes) => format!("Downloads the rest of the file history first · about {}", crate::explore::size(bytes)),
                        None => "Downloads the rest of the file history first".to_owned(),
                    };
                    ui.label(RichText::new(note).size(12.5).color(MUTED()));
                }
            },
            Part::Hint => {
                if running.is_none() && !failed {
                    shortcut_hint(ui, "to keep");
                }
            }
            Part::Buttons => match &running {
                Some((_, _, cancelling)) => {
                    let stop = ui.add_enabled(!cancelling, Button::new(if *cancelling { "Stopping" } else { "Stop" }));
                    pages::control(widgets, "dialog-stop", &stop, ui.clip_rect());
                    if stop.clicked() {
                        next = Next::Stop;
                    }
                }
                None => buttons(ui, ready, if failed { "Try again" } else { "Keep" }, widgets, &mut next),
            },
            Part::Close(clicked) => {
                if clicked && running.is_none() {
                    next = Next::Close;
                }
            }
        });
        if response.should_close() && running.is_none() {
            next = Next::Close;
        }
        if matches!(next, Next::Stay)
            && ready
            && self.picker.is_none()
            && ctx.input_mut(|i| i.consume_key(crate::shortcuts::primary_modifiers(), Key::Enter))
        {
            next = Next::Submit;
        }
        match next {
            Next::Stay => self.dialog = Some(dialog),
            Next::Close => {}
            Next::Submit => {
                dialog.error = None;
                let target = PathBuf::from(dialog.second.trim());
                self.dialog = Some(dialog);
                self.keep_preview(id, target);
            }
            Next::Stop => {
                self.dialog = Some(dialog);
                self.cancel_write();
            }
            Next::Browse(target) => {
                let current = dialog.create.as_ref().map(|f| f.parent.clone()).unwrap_or_default();
                self.dialog = Some(dialog);
                self.browse(target, &current);
            }
            Next::Open(path) => self.open(path),
        }
    }

    /// Clone's first fields: a destination already filled in, else the folder of the
    /// latest repository (the test folder in the MCP host) and a name from the source.
    fn clone_form(&self, dialog: &Dialog) -> Form {
        let typed = Path::new(dialog.second.trim());
        if typed.is_absolute()
            && let (Some(parent), Some(name)) = (typed.parent(), typed.file_name())
        {
            return Form {
                parent: parent.display().to_string(),
                name: name.to_string_lossy().into_owned(),
                name_edited: true,
                ..Default::default()
            };
        }
        let usual = self
            .browse_root
            .clone()
            .or_else(|| self.recent.first().and_then(|p| p.parent()).map(Path::to_path_buf))
            .or_else(folder_browser::home)
            .map(|p| crate::views::display_path(&p))
            .unwrap_or_default();
        let Some(source) = self.source_folder() else {
            return Form {
                parent: usual,
                ..Default::default()
            };
        };
        let parent = crate::views::display_path(&source);
        Form {
            parent: parent.clone(),
            ..Form::starting_in(source, parent, usual)
        }
    }

    /// New repository's first field: the source code folder with the name left to add.
    fn init_form(&self, dialog: &mut Dialog) -> Form {
        let Some(source) = self.source_folder().filter(|_| dialog.first.trim().is_empty()) else {
            return Form::default();
        };
        let mut shown = crate::views::display_path(&source);
        if !shown.ends_with(['\\', '/']) {
            shown.push('\\');
        }
        dialog.first = shown.clone();
        Form::starting_in(source, shown, String::new())
    }
}

#[allow(clippy::too_many_arguments)]
fn clone_body(
    ui: &mut Ui,
    source: &mut String,
    connection: &mut Option<crate::remote_check::ConnectionTest>,
    form: &mut Form,
    backend: Option<&crate::backend::Backend>,
    destination: Option<&Path>,
    status: Option<&Status>,
    widgets: &mut HashMap<String, UiControl>,
    next: &mut Next,
) {
    caption(ui, "Source");
    ui.horizontal(|ui| {
        let edit = ui.add(
            TextEdit::singleline(source)
                .hint_text("https://github.com/owner/repository.git")
                .font(TextStyle::Monospace)
                .desired_width((ui.available_width() - 132.0).max(120.0)),
        );
        pages::control(widgets, "dialog-clone-source", &edit, ui.clip_rect());
        let target = (!source.trim().is_empty()).then(|| source.trim().to_owned());
        crate::remote_check::inline_button(ui, connection, target, backend, None, widgets);
    });
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(
            RichText::new("HTTPS URL, SSH address or a local repository folder ·")
                .size(12.0)
                .color(HINT()),
        );
        let choose = ui.add(
            Button::new(RichText::new("Choose a folder").size(12.0).color(BLUE())).frame(false),
        );
        pages::control(widgets, "dialog-clone-choose-source", &choose, ui.clip_rect());
        if choose.clicked() {
            *next = Next::Browse(PickTarget::CloneSource);
        }
    });
    crate::remote_check::inline_status(ui, connection);
    ui.add_space(10.0);
    destination_fields(ui, form, destination, status, widgets, next);
}

/// Parent folder and name with the "Clones into" check, shared by Clone and Keep.
fn destination_fields(
    ui: &mut Ui,
    form: &mut Form,
    destination: Option<&Path>,
    status: Option<&Status>,
    widgets: &mut HashMap<String, UiControl>,
    next: &mut Next,
) {
    caption(ui, "Destination");
    let gap = 10.0;
    let width = ui.available_width();
    let left = ((width - gap) * 0.62).floor();
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        ui.allocate_ui_with_layout(vec2(left, 56.0), Layout::top_down(Align::Min), |ui| {
            ui.label(RichText::new("Parent folder").size(12.0).color(MUTED()));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let edit = ui.add(
                    TextEdit::singleline(&mut form.parent)
                        .font(TextStyle::Monospace)
                        .desired_width((ui.available_width() - 84.0).max(80.0)),
                );
                pages::control(widgets, "dialog-clone-parent", &edit, ui.clip_rect());
                let browse = pages::tool(ui, Icon::Folder, "Browse", true);
                pages::control(widgets, "dialog-clone-browse", &browse, ui.clip_rect());
                if browse.clicked() {
                    *next = Next::Browse(PickTarget::CloneParent);
                }
            });
        });
        ui.allocate_ui_with_layout(
            vec2(ui.available_width(), 56.0),
            Layout::top_down(Align::Min),
            |ui| {
                ui.label(RichText::new("Folder name").size(12.0).color(MUTED()));
                // A row like the parent folder's, so both fields line up.
                let edit = ui
                    .horizontal(|ui| {
                        ui.add(
                            TextEdit::singleline(&mut form.name)
                                .font(TextStyle::Monospace)
                                .desired_width(ui.available_width()),
                        )
                    })
                    .inner;
                pages::control(widgets, "dialog-clone-name", &edit, ui.clip_rect());
                if edit.changed() {
                    form.name_edited = true;
                }
            },
        );
    });
    let (label, color, note) = match status {
        _ if destination.is_none() => ("Name needed", AMBER(), None),
        None => ("Checking", MUTED(), None),
        Some(Status::New) => ("New folder", GREEN(), None),
        Some(Status::Exists) => (
            "Already exists",
            RED(),
            Some("Rocket clones only into a new folder. Choose another name."),
        ),
        Some(Status::MissingParent) => ("Parent not found", AMBER(), Some("The parent folder must exist.")),
        Some(Status::Refused(text)) => ("Not allowed", RED(), Some(text.as_str())),
        Some(_) => ("Not allowed", RED(), None),
    };
    card(ui, |ui| {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
            icon(ui.painter(), rect, Icon::Folder, color);
            ui.label(RichText::new("Clones into").size(12.0).color(HINT()));
            let pill = ui
                .painter()
                .layout_no_wrap(label.into(), semibold(11.5), color)
                .size()
                .x
                + 26.0;
            let room = (ui.available_width() - pill).max(40.0);
            let path = destination.map_or_else(
                || "Choose a parent folder and a name".to_owned(),
                |p| fit_path(ui, &crate::views::display_path(p), room - 8.0),
            );
            ui.allocate_ui_with_layout(
                vec2(room, 20.0),
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.add(
                        Label::new(RichText::new(path).monospace().size(12.5).color(TEXT()))
                            .truncate(),
                    )
                },
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                pages::pill(ui, label, color);
            });
        });
    });
    if let Some(note) = note {
        ui.label(RichText::new(note).size(12.0).color(color));
    }
}

#[allow(clippy::too_many_arguments)]
fn init_body(
    ui: &mut Ui,
    folder: &mut String,
    branch: &mut String,
    status: Option<&Status>,
    checking: bool,
    at_source: bool,
    widgets: &mut HashMap<String, UiControl>,
    next: &mut Next,
) {
    caption(ui, "Project folder");
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let edit = ui.add(
            TextEdit::singleline(folder)
                .hint_text("C:\\Projects\\my-project")
                .font(TextStyle::Monospace)
                .desired_width((ui.available_width() - 84.0).max(120.0)),
        );
        pages::control(widgets, "dialog-init-path", &edit, ui.clip_rect());
        let browse = pages::tool(ui, Icon::Folder, "Browse", true);
        pages::control(widgets, "dialog-init-browse", &browse, ui.clip_rect());
        if browse.clicked() {
            *next = Next::Browse(PickTarget::InitFolder);
        }
    });
    let existing;
    let (image, color, title, text) = match status {
        _ if folder.trim().is_empty() => (Icon::Folder, MUTED(), "Choose a project folder", "Type a path or use Browse."),
        _ if at_source => (Icon::Folder, MUTED(), "Add the project's folder name", ""),
        None if checking => (Icon::Folder, MUTED(), "Checking the folder", ""),
        None => (Icon::Folder, MUTED(), "Choose a project folder", "Type a path or use Browse."),
        Some(Status::New) => (Icon::Plus, GREEN(), "New folder", "Rocket creates it, then starts tracking it with Git."),
        Some(Status::Existing(entries)) => {
            existing = match *entries {
                0 => "Empty folder".to_owned(),
                1 => "Existing folder · 1 item".to_owned(),
                n if n >= INIT_ENTRY_LIMIT => format!("Existing folder · {INIT_ENTRY_LIMIT}+ items"),
                n => format!("Existing folder · {n} items"),
            };
            (Icon::Folder, BLUE(), existing.as_str(), "Your files stay as they are. Nothing is staged or committed.")
        }
        Some(Status::Repository(_)) => (
            Icon::Branch,
            RED(),
            "Already a Git repository",
            "Repositories, and folders inside one, cannot be created again.",
        ),
        Some(Status::MissingParent) => (Icon::Warning, AMBER(), "Parent folder not found", "Choose a folder whose parent exists."),
        Some(Status::Refused(reason)) => (Icon::Warning, RED(), "This folder cannot be used", reason.as_str()),
        Some(Status::Exists) => (Icon::Warning, RED(), "This folder cannot be used", ""),
    };
    card(ui, |ui| {
        ui.horizontal_top(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
            icon(ui.painter(), rect, image, color);
            let open = match status {
                Some(Status::Repository(root)) => Some(root.clone()),
                _ => None,
            };
            let text_width = ui.available_width() - if open.is_some() { 84.0 } else { 0.0 };
            ui.vertical(|ui| {
                ui.set_max_width(text_width);
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(RichText::new(title).font(semibold(13.0)).color(TEXT()));
                if !text.is_empty() {
                    ui.add(Label::new(RichText::new(text).size(12.5).color(MUTED())).wrap());
                }
            });
            if let Some(root) = open {
                ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                    let r = ui.button("Open it").on_hover_text(crate::views::display_path(&root));
                    pages::control(widgets, "dialog-init-open", &r, ui.clip_rect());
                    if r.clicked() {
                        *next = Next::Open(root);
                    }
                });
            }
        });
    });
    ui.add_space(10.0);
    caption(ui, "Initial branch");
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let edit = ui.add(
            TextEdit::singleline(branch)
                .font(TextStyle::Monospace)
                .desired_width(220.0),
        );
        pages::control(widgets, "dialog-init-branch", &edit, ui.clip_rect());
        for name in ["main", "master"] {
            let chip = ui.add(
                Button::new(RichText::new(name).monospace().size(12.0))
                    .selected(branch.trim() == name)
                    .corner_radius(12)
                    .min_size(vec2(0.0, 24.0)),
            );
            pages::control(widgets, format!("dialog-init-branch-{name}"), &chip, ui.clip_rect());
            if chip.clicked() {
                *branch = name.to_owned();
            }
        }
    });
}

/// A path shortened from the front to fit `width`, keeping the folders nearest its end:
/// the new folder's name matters more than the drive.
pub(crate) fn fit_path(ui: &Ui, path: &str, width: f32) -> String {
    let fits = |text: &str| {
        ui.painter()
            .layout_no_wrap(text.to_owned(), FontId::monospace(12.5), TEXT())
            .size()
            .x
            <= width
    };
    if fits(path) {
        return path.to_owned();
    }
    let parts: Vec<&str> = path.split('\\').collect();
    (1..parts.len())
        .map(|start| format!("…\\{}", parts[start..].join("\\")))
        .find(|text| fits(text))
        .unwrap_or_else(|| format!("…\\{}", parts.last().copied().unwrap_or_default()))
}

/// Keep's three steps, the running one from the write's progress.
fn keep_progress(ui: &mut Ui, progress: &str, fraction: Option<f32>, destination: Option<&Path>) {
    let stage = if progress.starts_with("Downloading files") || progress.starts_with("Preparing") {
        0
    } else if progress.starts_with("Cloning") || progress.starts_with("Receiving") || progress.starts_with("Resolving") || progress.starts_with("Counting") || progress.starts_with("Compressing") {
        1
    } else {
        2
    };
    let place = destination.map_or_else(String::new, |p| crate::views::display_path(p));
    for (i, label) in ["Download the rest of the file history".to_owned(), format!("Copy to {place}"), "Check out the default branch".to_owned()].iter().enumerate() {
        ui.horizontal(|ui| {
            ui.set_min_height(34.0);
            let (mark, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
            let painter = ui.painter();
            if i < stage {
                painter.circle_filled(mark.center(), 10.0, GREEN().gamma_multiply(0.18));
                icon(painter, mark.shrink(5.0), Icon::Check, GREEN());
            } else if i == stage {
                painter.circle_stroke(mark.center(), 8.5, Stroke::new(2.0, BLUE()));
            } else {
                painter.circle_stroke(mark.center(), 8.5, Stroke::new(1.5, HAIRLINE()));
            }
            ui.add_space(6.0);
            let text = RichText::new(label).color(if i > stage { HINT() } else { TEXT() });
            ui.add(Label::new(if i == stage { text.strong() } else { text }).truncate());
            if i == stage {
                // "1,240 of 3,912 files" while the history downloads.
                let detail = progress.strip_prefix("Downloading files: ").map_or_else(|| progress.trim_end_matches('…').to_owned(), |rest| format!("{rest} files"));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(detail).size(12.0).color(HINT()));
                });
            }
        });
        if i == stage && let Some(fraction) = fraction {
            ui.add(ProgressBar::new(fraction).desired_height(6.0));
        }
    }
}

fn caption(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).font(semibold(13.0)));
}

fn card(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    Frame::new()
        .fill(CARD())
        .stroke(Stroke::new(1.0, CARD_STROKE()))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            content(ui);
        });
}

fn shortcut_hint(ui: &mut Ui, text: &str) {
    keycap(ui, "Ctrl");
    keycap(ui, "Enter");
    ui.label(RichText::new(text).size(12.0).color(HINT()));
}

fn buttons(
    ui: &mut Ui,
    ready: bool,
    label: &str,
    widgets: &mut HashMap<String, UiControl>,
    next: &mut Next,
) {
    let submit = pages::primary(ui, ready, label);
    pages::control(widgets, "dialog-submit", &submit, ui.clip_rect());
    if submit.clicked() {
        *next = Next::Submit;
    }
}

/// The dialogs' frame: a header with the guide and close, the body, then a footer
/// with a hint on the left and the buttons on the right.
fn form_modal(
    ctx: &Context,
    id: &str,
    image: Icon,
    title: &str,
    subtitle: &str,
    widgets: &mut HashMap<String, UiControl>,
    mut content: impl FnMut(&mut Ui, Part, &mut HashMap<String, UiControl>),
) -> ModalResponse<()> {
    let width = (ctx.content_rect().width() - 48.0).clamp(360.0, 640.0);
    let height = (ctx.content_rect().height() - 220.0).max(120.0);
    Modal::new(Id::new(id))
        .backdrop_color(Color32::from_black_alpha(120))
        .frame(pages::modal_frame())
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 0.0;
            Frame::new()
                .inner_margin(Margin {
                    left: 20,
                    right: 16,
                    top: 18,
                    bottom: 10,
                })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_top(|ui| {
                        let (badge, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
                        ui.painter().rect_filled(badge, 8.0, ACCENT_SOFT());
                        icon(ui.painter(), badge.shrink(9.0), image, BLUE());
                        ui.add_space(6.0);
                        ui.vertical(|ui| {
                            ui.set_max_width(ui.available_width() - 70.0);
                            ui.spacing_mut().item_spacing.y = 3.0;
                            ui.label(RichText::new(title).font(semibold(16.0)));
                            ui.add(Label::new(RichText::new(subtitle).color(MUTED())).truncate());
                        });
                        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                            let close = crate::chrome::icon_button(ui, Icon::Close, true, false, 28.0)
                                .on_hover_text("Close (Esc)");
                            pages::control(widgets, "dialog-close", &close, ui.clip_rect());
                            // Automation's old id for the footer's Cancel.
                            pages::control(widgets, "dialog-cancel", &close, ui.clip_rect());
                            content(ui, Part::Close(close.clicked()), widgets);
                            if crate::guide::available(ui.ctx()) {
                                crate::guide::header_button(ui);
                            }
                        });
                    });
                });
            ScrollArea::vertical()
                .id_salt((id, "body"))
                .max_height(height)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    Frame::new()
                        .inner_margin(Margin {
                            left: 20,
                            right: 20,
                            top: 6,
                            bottom: 18,
                        })
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = 6.0;
                            content(ui, Part::Body, widgets);
                        });
                });
            let footer = Frame::new()
                .fill(SUNKEN())
                .inner_margin(Margin::symmetric(20, 12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.allocate_ui_with_layout(
                        vec2(ui.available_width(), ui.spacing().interact_size.y),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            ui.spacing_mut().item_spacing.x = 5.0;
                            content(ui, Part::Hint, widgets);
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                content(ui, Part::Buttons, widgets);
                            });
                        },
                    );
                })
                .response
                .rect;
            ui.painter()
                .hline(footer.x_range(), footer.top(), Stroke::new(1.0, HAIRLINE()));
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_common as common;

    fn frame(ctx: &Context, app: &mut Rocket, events: Vec<Event>) {
        app.frame_widgets.clear();
        let mut output = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 860.0))),
                events,
                ..Default::default()
            },
            |ui| {
                let ctx = ui.ctx().clone();
                app.repository_form_dialog(&ctx);
                app.picker_dialog(&ctx);
            },
        );
        output.textures_delta.clear();
    }

    /// Frames until the folder check has run for the current fields.
    fn settle(ctx: &Context, app: &mut Rocket) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            frame(ctx, app, vec![]);
            let checking = app
                .dialog
                .as_ref()
                .and_then(|d| d.create.as_ref())
                .is_some_and(Form::checking);
            if !checking {
                frame(ctx, app, vec![]);
                return;
            }
            assert!(Instant::now() < deadline, "the folder check finished");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn click(ctx: &Context, app: &mut Rocket, id: &str) {
        let at = app.frame_widgets[id].rect.center();
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        frame(ctx, app, vec![Event::PointerMoved(at), button(true)]);
        frame(ctx, app, vec![button(false)]);
    }

    fn enabled(app: &Rocket, id: &str) -> bool {
        app.frame_widgets.get(id).is_some_and(|c| c.enabled)
    }

    #[test]
    fn new_repository_says_what_creating_would_do_before_it_is_allowed() {
        common::isolated(
            "ROCKET_FORMS_UI_CASE",
            "repository_forms::tests::new_repository_says_what_creating_would_do_before_it_is_allowed",
            || {
                let fixture = common::Fixture::new();
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(&ctx, vec![], fixture.root.join("logs"));
                // The fixture path has `..` parts, which creating refuses.
                let root = fixture.root.canonicalize().unwrap();
                let set = |app: &mut Rocket, path: &Path| {
                    app.dialog.as_mut().unwrap().first = path.display().to_string();
                };
                app.show_dialog(DialogKind::NewRepository);
                set(&mut app, &root.join("fresh"));
                settle(&ctx, &mut app);
                assert!(enabled(&app, "dialog-submit"), "a new folder can be created");
                assert!(!app.frame_widgets.contains_key("dialog-init-open"));
                let repository = fixture.repository.canonicalize().unwrap();
                set(&mut app, &repository.join("src"));
                settle(&ctx, &mut app);
                assert!(!enabled(&app, "dialog-submit"), "not inside a repository");
                assert!(app.frame_widgets.contains_key("dialog-init-open"));
                set(&mut app, &root.join("missing").join("app"));
                settle(&ctx, &mut app);
                assert!(!enabled(&app, "dialog-submit"), "not without a parent folder");
                click(&ctx, &mut app, "dialog-close");
                assert!(app.dialog.is_none(), "the header's close button closes it");
                assert!(!root.join("fresh").exists(), "nothing was created");
            },
        );
    }

    #[test]
    fn clone_names_the_folder_from_the_source_and_refuses_an_existing_one() {
        common::isolated(
            "ROCKET_FORMS_UI_CASE",
            "repository_forms::tests::clone_names_the_folder_from_the_source_and_refuses_an_existing_one",
            || {
                let fixture = common::Fixture::new();
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(&ctx, vec![], fixture.root.join("logs"));
                let root = fixture.root.canonicalize().unwrap();
                app.show_dialog(DialogKind::Clone);
                frame(&ctx, &mut app, vec![]);
                let dialog = app.dialog.as_mut().unwrap();
                dialog.first = "https://example.com/team/app.git".into();
                dialog.create.as_mut().unwrap().parent = root.display().to_string();
                settle(&ctx, &mut app);
                let dialog = app.dialog.as_ref().unwrap();
                assert_eq!(dialog.create.as_ref().unwrap().name, "app");
                assert_eq!(Path::new(&dialog.second), root.join("app"));
                assert!(enabled(&app, "dialog-submit"), "a new folder in an existing parent");
                std::fs::create_dir(root.join("taken")).unwrap();
                let form = app.dialog.as_mut().unwrap().create.as_mut().unwrap();
                form.name = "taken".into();
                form.name_edited = true;
                settle(&ctx, &mut app);
                assert!(!enabled(&app, "dialog-submit"), "an existing folder is refused");
                // A name typed by hand stays when the source changes.
                app.dialog.as_mut().unwrap().first = "https://example.com/team/other.git".into();
                frame(&ctx, &mut app, vec![]);
                assert_eq!(app.dialog.as_ref().unwrap().create.as_ref().unwrap().name, "taken");
            },
        );
    }

    #[test]
    fn browse_picks_the_clone_parent_above_the_dialog() {
        common::isolated(
            "ROCKET_FORMS_UI_CASE",
            "repository_forms::tests::browse_picks_the_clone_parent_above_the_dialog",
            || {
                let fixture = common::Fixture::new();
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(&ctx, vec![], fixture.root.join("logs"));
                let root = fixture.root.canonicalize().unwrap();
                let projects = root.join("projects");
                std::fs::create_dir(&projects).unwrap();
                app.show_dialog(DialogKind::Clone);
                frame(&ctx, &mut app, vec![]);
                app.dialog.as_mut().unwrap().create.as_mut().unwrap().parent =
                    projects.display().to_string();
                settle(&ctx, &mut app);
                click(&ctx, &mut app, "dialog-clone-browse");
                assert!(
                    app.picker.as_ref().is_some_and(|p| p.target == PickTarget::CloneParent),
                    "Browse opens the folder picker for the parent folder"
                );
                assert!(app.dialog.is_some(), "the dialog stays under the picker");
                // A new modal is placed once its size is known.
                frame(&ctx, &mut app, vec![]);
                frame(&ctx, &mut app, vec![]);
                click(&ctx, &mut app, "picker-choose");
                assert!(app.picker.is_none());
                let parent = &app.dialog.as_ref().unwrap().create.as_ref().unwrap().parent;
                assert_eq!(
                    crate::views::display_path(Path::new(parent)),
                    crate::views::display_path(&projects),
                    "the chosen folder fills the parent field"
                );
            },
        );
    }

    #[test]
    fn clone_sources_suggest_their_folder_name() {
        for (source, name) in [
            ("https://github.com/rocket-org/upstream.git", "upstream"),
            ("https://example.com/team/app/", "app"),
            ("ssh://git@host/owner/repo.git", "repo"),
            ("git@github.com:owner/tool.git", "tool"),
            ("C:\\Work\\shared-lib", "shared-lib"),
            ("  D:\\repos\\site.git\\  ", "site"),
            ("", ""),
        ] {
            assert_eq!(suggested_name(source), name, "{source}");
        }
    }
}
