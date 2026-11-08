//! Custom commands: user-defined programs with literal arguments, run in the
//! repository folder after a dialog shows the exact command line. No shell parses
//! anything; output goes to Activity and Cancel stops what the command started.
use crate::{
    app::*,
    backend::{Outcome, user_error},
    folder_browser::PickTarget,
    pages::{self, control},
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use std::{
    collections::{BTreeMap, HashMap},
    ffi::OsString,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// Most inputs one command asks for.
const MAX_INPUTS: usize = 8;
const KINDS: [CommandInputKind; 4] = [
    CommandInputKind::Text,
    CommandInputKind::Branch,
    CommandInputKind::File,
    CommandInputKind::Checkbox,
];
/// The label column of the run form: values start at the same x on every row.
const LABEL_WIDTH: f32 = 120.0;
/// How long typing must pause before a path is checked on disk.
const SETTLE: Duration = Duration::from_millis(300);
/// The file name Export suggests in a folder chosen with Browse.
const EXPORT_NAME: &str = "rocket-commands.json";

/// What the right side of the Custom commands dialog shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Step {
    /// The selected command, or the empty state when there are none.
    #[default]
    Edit,
    Import,
    Export,
}

/// The editor's working copy; Save replaces the saved list.
pub struct Editor {
    pub list: Vec<CustomCommand>,
    /// The command open for editing under its row.
    pub selected: Option<usize>,
    pub step: Step,
    /// Find: shows the commands whose name, submenu, program or arguments contain it.
    pub filter: String,
    /// The definitions file to import from or export to.
    pub exchange_path: String,
    pub transfer: Option<ReadTask<Option<Vec<CustomCommand>>>>,
    /// Definitions read for review; nothing is added until the review is accepted.
    pub imported: Option<Vec<CustomCommand>>,
    pub notice: Option<String>,
    pub error: Option<String>,
    /// Rocket's own wording of the last error, shown as small print.
    error_detail: Option<String>,
    /// What a running export reports when it finishes; `None` while importing.
    exporting: Option<String>,
    /// The list as the dialog opened it, to tell what is unsaved.
    saved: Vec<CustomCommand>,
    program: Probe<ProgramCheck>,
    /// Only an explicit Check action authorizes a filesystem probe of this exact text.
    program_requested: Option<String>,
    destination: Probe<ExportTarget>,
}
impl Editor {
    pub fn new(list: Vec<CustomCommand>) -> Self {
        let selected = (!list.is_empty()).then_some(0);
        Self {
            saved: list.clone(),
            list,
            selected,
            step: Step::Edit,
            filter: String::new(),
            exchange_path: String::new(),
            transfer: None,
            imported: None,
            notice: None,
            error: None,
            error_detail: None,
            exporting: None,
            program: Probe::default(),
            program_requested: None,
            destination: Probe::default(),
        }
    }
    /// Why the list cannot be saved, if it cannot.
    pub fn problem(&self) -> Option<String> {
        if self.transfer.is_some() || self.imported.is_some() {
            return Some("Finish or cancel the import or export first.".into());
        }
        if self.list.len() > MAX_CUSTOM_COMMANDS {
            return Some(format!("Keep at most {MAX_CUSTOM_COMMANDS} commands."));
        }
        self.list.iter().find_map(|c| {
            c.validate()
                .err()
                .map(|e| format!("{}: {}", display_name(c), plain(&e)))
        })
    }
    /// The list differs from the saved one.
    pub fn changed(&self) -> bool {
        self.list != self.saved
    }
    /// The list was saved: what is unsaved is measured from it now.
    pub(crate) fn mark_saved(&mut self) {
        self.saved = self.list.clone();
    }
    /// Back to the saved list, every command closed.
    pub(crate) fn discard(&mut self) {
        self.list = self.saved.clone();
        self.selected = None;
        self.program_requested = None;
        self.program = Probe::default();
        self.clear_messages();
    }
    /// Browse chose the file to import, or the folder to export into; an export
    /// keeps a typed .json file name.
    pub(crate) fn picked(&mut self, export: bool, path: &Path) {
        let path = if export {
            let typed = Path::new(self.exchange_path.trim())
                .file_name()
                .filter(|n| Path::new(n).extension().is_some_and(|x| x.eq_ignore_ascii_case("json")))
                .map(ToOwned::to_owned);
            path.join(typed.unwrap_or_else(|| EXPORT_NAME.into()))
        } else {
            path.to_path_buf()
        };
        self.exchange_path = path.display().to_string();
        self.clear_messages();
    }
    fn clear_messages(&mut self) {
        self.notice = None;
        self.error = None;
        self.error_detail = None;
    }
    fn fail(&mut self, error: &Error) {
        let (message, detail) = exchange_failure(error);
        self.notice = None;
        self.error = Some(message);
        self.error_detail = detail;
    }
    /// Take a finished import or export.
    fn poll(&mut self) {
        let Some(result) = self.transfer.as_mut().and_then(ReadTask::try_finish) else {
            return;
        };
        self.transfer = None;
        let exported = self.exporting.take();
        match result {
            Ok(Some(list)) => self.imported = Some(list),
            Ok(None) => {
                self.notice = exported;
                // The new file exists now; check the path again when it is shown.
                self.destination = Probe::default();
            }
            Err(error) => self.fail(&error),
        }
    }
    /// Read the definitions file for review, or write every command to a new file.
    fn start_transfer(&mut self, import: bool, scope: Option<&Path>, log: Option<&DiagnosticLog>) {
        let path = PathBuf::from(self.exchange_path.trim());
        let scope = scope.map(Path::to_owned);
        let commands = self.list.clone();
        let log = log.cloned();
        self.clear_messages();
        self.exporting = (!import).then(|| {
            format!(
                "Exported {} to {}.",
                counted(commands.len(), "command", "commands"),
                file_name(&path)
            )
        });
        let task = ReadTask::spawn(move |c| {
            let work = || {
                if import {
                    crate::command_definitions::read(&path, scope.as_deref(), &c).map(Some)
                } else {
                    crate::command_definitions::export(&path, &commands, scope.as_deref(), &c)
                        .map(|()| None)
                }
            };
            match log {
                Some(log) => log.request(
                    if import {
                        "custom-command-import"
                    } else {
                        "custom-command-export"
                    },
                    |_| work(),
                ),
                None => work(),
            }
        });
        match task {
            Ok(t) => self.transfer = Some(t),
            Err(error) => {
                self.exporting = None;
                self.fail(&error);
            }
        }
    }
    /// Add the reviewed definitions that are new; same-named ones are skipped.
    fn accept_import(&mut self) {
        let Some(incoming) = self.imported.take() else {
            return;
        };
        self.program_requested = None;
        self.program = Probe::default();
        let fresh = fresh(&self.list, &incoming);
        let first = self.list.len();
        self.list.extend(
            incoming
                .into_iter()
                .zip(fresh)
                .filter_map(|(command, new)| new.then_some(command)),
        );
        self.selected = (self.list.len() > first)
            .then_some(first)
            .or(self.selected);
        self.step = Step::Edit;
        self.clear_messages();
    }
}

/// A path checked on a worker once typing settles, so the dialog never waits on the
/// disk. `status` is `None` until the current text has been checked.
struct Probe<T> {
    checked: Option<(String, std::result::Result<T, String>)>,
    wanted: Option<(String, Instant)>,
    task: Option<(String, ReadTask<T>)>,
}
impl<T> Default for Probe<T> {
    fn default() -> Self {
        Self {
            checked: None,
            wanted: None,
            task: None,
        }
    }
}
impl<T: Clone + Send + 'static> Probe<T> {
    fn status(
        &mut self,
        ctx: &Context,
        text: &str,
        scope: Option<&Path>,
        check: fn(&Path, Option<&Path>) -> T,
    ) -> Option<std::result::Result<T, String>> {
        if let Some((checked, task)) = self.task.as_mut()
            && let Some(result) = task.try_finish()
        {
            self.checked = Some((checked.clone(), result.map_err(|e| user_error(&e))));
            self.task = None;
        }
        if let Some((checked, status)) = &self.checked
            && checked == text
        {
            return Some(status.clone());
        }
        if self.task.as_ref().is_some_and(|(t, _)| t == text) {
            ctx.request_repaint_after(Duration::from_millis(30));
            return None;
        }
        match &self.wanted {
            Some((wanted, since)) if wanted == text && since.elapsed() >= SETTLE => {
                self.wanted = None;
                let (path, scope) = (PathBuf::from(text), scope.map(Path::to_owned));
                match ReadTask::spawn(move |_| Ok(check(&path, scope.as_deref()))) {
                    Ok(task) => self.task = Some((text.to_owned(), task)),
                    Err(e) => self.checked = Some((text.to_owned(), Err(user_error(&e)))),
                }
                ctx.request_repaint_after(Duration::from_millis(30));
            }
            Some((wanted, since)) if wanted == text => {
                ctx.request_repaint_after(SETTLE.saturating_sub(since.elapsed()));
            }
            _ => {
                // New text: drop an older check and wait for typing to settle.
                self.task = None;
                self.wanted = Some((text.to_owned(), Instant::now()));
                ctx.request_repaint_after(SETTLE);
            }
        }
        None
    }
}

/// Whether a command's program exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProgramCheck {
    Invalid,
    Found,
    Missing,
    /// The MCP test host does not look outside its test folder.
    Outside,
}
fn check_program(path: &Path, scope: Option<&Path>) -> ProgramCheck {
    if !valid_custom_program_path(path) {
        return ProgramCheck::Invalid;
    }
    if scope.is_some_and(|s| !crate::folder_browser::inside(path, s)) {
        ProgramCheck::Outside
    } else if path.is_file() {
        ProgramCheck::Found
    } else {
        ProgramCheck::Missing
    }
}

/// What exporting to a path would do. The export itself never replaces a file; this
/// only says so before the click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExportTarget {
    New,
    Exists,
    MissingFolder,
    Outside,
}
fn check_export(path: &Path, scope: Option<&Path>) -> ExportTarget {
    if scope.is_some_and(|s| !crate::folder_browser::inside(path, s)) {
        ExportTarget::Outside
    } else if std::fs::symlink_metadata(path).is_ok() {
        ExportTarget::Exists
    } else if path.parent().is_some_and(Path::is_dir) {
        ExportTarget::New
    } else {
        ExportTarget::MissingFolder
    }
}

/// The values a command line can use, from the repository's current selection.
pub fn context(tab: &Tab) -> CommandContext {
    let branch = tab
        .snapshot
        .as_ref()
        .and_then(|s| s.head.as_ref())
        .and_then(|h| match h {
            Head::Branch { name, .. } => String::from_utf8(name.clone()).ok(),
            _ => None,
        });
    let file = tab.selected_change.as_ref().and_then(|(path, _)| {
        std::str::from_utf8(path.as_bytes())
            .ok()
            .map(|p| tab.path.join(p.replace('/', std::path::MAIN_SEPARATOR_STR)))
    });
    CommandContext {
        repository: tab.path.clone(),
        branch,
        commit: tab.selected_commit.clone(),
        file,
        submodule: None,
        historical_file: false,
    }
}

#[derive(Default)]
struct Out {
    browse: Option<PickTarget>,
}

/// The label column of the page's form: fields start at the same x on every row.
const FORM_LABEL: f32 = 92.0;

/// Preferences → Custom commands: the commands grouped by the menu they show in, the open
/// one edited under its row; Import and Export take the list's place. Nothing runs from
/// here, and Save commands in the Preferences footer replaces the saved list. Returns a
/// Browse request.
pub(crate) fn page(
    ui: &mut Ui,
    e: &mut Editor,
    widgets: &mut HashMap<String, UiControl>,
    scope: Option<&Path>,
    log: Option<&DiagnosticLog>,
) -> Option<PickTarget> {
    e.poll();
    if e.transfer.is_some() {
        ui.ctx().request_repaint_after(Duration::from_millis(25));
    }
    let mut out = Out::default();
    toolbar(ui, e, widgets);
    ui.add_space(10.0);
    match e.step {
        Step::Edit if e.list.is_empty() => empty_state(ui, e, widgets),
        Step::Edit => table(ui, e, widgets, &mut out, scope),
        Step::Import | Step::Export => {
            card(ui, Margin::same(16), |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                if e.step == Step::Import {
                    import_step(ui, e, widgets, &mut out, scope, log)
                } else {
                    export_step(ui, e, widgets, &mut out, scope, log)
                }
            });
        }
    }
    out.browse
}

/// Where Browse starts for `target`: the command's program when it is a valid path, else
/// the definitions file.
pub(crate) fn browse_start(e: Option<&Editor>, target: PickTarget) -> String {
    let Some(e) = e else {
        return String::new();
    };
    match target {
        PickTarget::CommandProgram(i) => e
            .list
            .get(i)
            .filter(|c| valid_custom_program_path(Path::new(&c.program)))
            .map(|c| c.program.clone())
            .unwrap_or_default(),
        _ => e.exchange_path.clone(),
    }
}

/// The table and step panels: a dark card with a hairline edge.
fn card<R>(ui: &mut Ui, margin: Margin, content: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::new()
        .fill(PANEL())
        .stroke(Stroke::new(1.0, CARD_STROKE()))
        .corner_radius(8)
        .inner_margin(margin)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            content(ui)
        })
        .inner
}

/// Find, then Import, Export and New command at the right.
fn toolbar(ui: &mut Ui, e: &mut Editor, widgets: &mut HashMap<String, UiControl>) {
    let busy = e.transfer.is_some();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let width = (ui.available_width() - 320.0).clamp(160.0, 280.0);
        let r = search_field(ui, &mut e.filter, "Find a command", width);
        control(widgets, "dialog-commands-filter", &r, ui.clip_rect());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let room = e.list.len() < MAX_CUSTOM_COMMANDS;
            let r = pages::primary(ui, room && !busy, "New command");
            control(widgets, "dialog-commands-new", &r, ui.clip_rect());
            if r.clicked() {
                new_command(e);
            }
            // Right to left: Export at the right, Import before it.
            for (step, label, key) in [
                (Step::Export, "Export", "dialog-commands-show-export"),
                (Step::Import, "Import", "dialog-commands-show-import"),
            ] {
                let on = e.step == step;
                let r = ui.add_enabled(
                    !busy,
                    Button::new(RichText::new(label).color(if on { TEXT() } else { MUTED() }))
                        .selected(on)
                        .frame_when_inactive(on),
                );
                control(widgets, key, &r, ui.clip_rect());
                if r.clicked() {
                    e.step = if on { Step::Edit } else { step };
                    e.clear_messages();
                }
            }
        });
    });
}

/// The Command column's width; Runs takes the rest of the row.
fn name_column(width: f32) -> f32 {
    (width * 0.36).clamp(150.0, 260.0)
}

/// Whether `c` matches the Find text: its name, submenu, program or arguments.
fn found_by(c: &CustomCommand, query: &str) -> bool {
    query.is_empty()
        || [&c.name, &c.group, &c.program, &c.arguments]
            .iter()
            .any(|text| text.to_lowercase().contains(query))
}

/// The commands under the menus they show in: a row each, the open one's fields under it.
fn table(
    ui: &mut Ui,
    e: &mut Editor,
    widgets: &mut HashMap<String, UiControl>,
    out: &mut Out,
    scope: Option<&Path>,
) {
    let busy = e.transfer.is_some();
    let query = e.filter.trim().to_lowercase();
    let mut picked = None;
    card(ui, Margin::ZERO, |ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
        ui.painter()
            .rect_filled(rect, CornerRadius { nw: 7, ne: 7, sw: 0, se: 0 }, CARD());
        ui.painter()
            .hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, CARD_LINE()));
        let name_width = name_column(rect.width());
        for (x, title) in [(12.0, "COMMAND"), (name_width + 12.0, "RUNS")] {
            ui.painter().text(
                pos2(rect.left() + x, rect.center().y),
                Align2::LEFT_CENTER,
                title,
                semibold(11.0),
                FAINT(),
            );
        }
        let mut shown = 0;
        'menus: for kind in CommandScope::ALL {
            let rows: Vec<usize> = (0..e.list.len())
                .filter(|&i| e.list[i].scope == kind && found_by(&e.list[i], &query))
                .collect();
            if rows.is_empty() {
                continue;
            }
            group_heading(ui, kind, rows.len());
            for i in rows {
                let open = e.selected == Some(i);
                let changed = !e.saved.contains(&e.list[i]);
                let r = ui
                    .add_enabled_ui(!busy, |ui| command_row(ui, &e.list[i], open, changed))
                    .inner;
                control(widgets, format!("dialog-commands-row-{i}"), &r, ui.clip_rect());
                if r.clicked() {
                    picked = Some((!open).then_some(i));
                }
                shown += 1;
                if open && edit_panel(ui, e, i, widgets, out, scope) {
                    // A command was added or deleted: the rows after it moved.
                    ui.ctx().request_repaint();
                    break 'menus;
                }
            }
        }
        if shown == 0 {
            Frame::new()
                .inner_margin(Margin::symmetric(12, 14))
                .show(ui, |ui| {
                    ui.label(RichText::new("No command matches.").color(HINT()));
                });
        }
    });
    if let Some(selected) = picked {
        e.selected = selected;
        e.step = Step::Edit;
        e.clear_messages();
    }
}

/// The menu a group of commands shows in, and how many there are.
fn group_heading(ui: &mut Ui, kind: CommandScope, count: usize) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, NAV());
    ui.painter()
        .hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, CARD_LINE()));
    let title = match kind {
        CommandScope::Repository => "REPOSITORY MENU",
        CommandScope::Commit => "COMMITS · RIGHT-CLICK",
        CommandScope::File => "FILES · RIGHT-CLICK",
        CommandScope::Branch => "BRANCHES · RIGHT-CLICK",
        CommandScope::Submodule => "SUBMODULES · RIGHT-CLICK",
    };
    let galley = ui
        .painter()
        .layout_no_wrap(title.to_owned(), semibold(11.0), FAINT());
    let x = rect.left() + 12.0;
    let after = x + galley.size().x + 8.0;
    ui.painter()
        .galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, FAINT());
    ui.painter().text(
        pos2(after, rect.center().y),
        Align2::LEFT_CENTER,
        count.to_string(),
        FontId::proportional(11.5),
        HINT(),
    );
}

/// One command: a dot when it is unsaved, its submenu and name, what it runs, and a
/// chevron. The whole row opens and closes its fields.
fn command_row(ui: &mut Ui, c: &CustomCommand, open: bool, changed: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::click());
    let enabled = ui.is_enabled();
    let hot = enabled && (response.hovered() || response.has_focus());
    let painter = ui.painter();
    if open {
        painter.rect_filled(rect, 0.0, BLUE().gamma_multiply(0.06));
    } else {
        if hot {
            painter.rect_filled(rect, 0.0, HOVER());
        }
        painter.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, CARD_LINE()));
    }
    let name_width = name_column(rect.width());
    let mut x = rect.left() + 12.0;
    if changed {
        painter.circle_filled(pos2(x + 3.0, rect.center().y), 3.0, BLUE());
        x += 12.0;
    }
    let mut job = text::LayoutJob::default();
    let group = c.group.trim();
    if !group.is_empty() {
        job.append(
            &format!("{} › ", custom_command_display(group)),
            0.0,
            TextFormat::simple(FontId::proportional(13.0), HINT()),
        );
    }
    let font = if open { semibold(13.0) } else { FontId::proportional(13.0) };
    job.append(&display_name(c), 0.0, TextFormat::simple(font, TEXT()));
    job.wrap = text::TextWrapping::truncate_at_width((rect.left() + name_width - x).max(40.0));
    let galley = painter.layout_job(job);
    painter.galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, TEXT());
    let mut x = rect.left() + name_width + 12.0;
    if c.action == CommandAction::Url {
        let mark = Rect::from_center_size(pos2(x + 6.5, rect.center().y), vec2(13.0, 13.0));
        icon(painter, mark, Icon::External, FAINT());
        x += 19.0;
    }
    let mut job = runs_job(c);
    job.wrap = text::TextWrapping::truncate_at_width((rect.right() - 36.0 - x).max(40.0));
    let galley = painter.layout_job(job);
    painter.galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, MUTED());
    let chevron = Rect::from_center_size(pos2(rect.right() - 18.0, rect.center().y), vec2(14.0, 14.0));
    icon(
        painter,
        chevron,
        if open { Icon::Chevron } else { Icon::ChevronRight },
        if open { BLUE() } else { FAINT() },
    );
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, enabled, open, display_name(c)));
    response
}

/// What a command runs, on one line: the program's file name and its arguments, or the
/// web link, with placeholders in the accent.
fn runs_job(c: &CustomCommand) -> text::LayoutJob {
    let font = FontId::monospace(12.0);
    let missing = |text: &str| {
        text::LayoutJob::simple_singleline(text.to_owned(), FontId::proportional(12.5), HINT())
    };
    if c.action == CommandAction::Url {
        return match c.program.trim() {
            "" => missing("No web link yet"),
            url => highlighted_in(&custom_command_display(url), font, MUTED()),
        };
    }
    let program = file_name(Path::new(c.program.trim()));
    if program.is_empty() {
        return missing("No program yet");
    }
    let line = format!("{program} {}", c.argument_lines().join(" "));
    highlighted_in(&custom_command_display(line.trim_end()), font, MUTED())
}

fn new_command(e: &mut Editor) {
    e.list.push(CustomCommand {
        name: "New command".into(),
        ..Default::default()
    });
    e.selected = Some(e.list.len() - 1);
    e.step = Step::Edit;
    // The new command must be visible to be edited.
    e.filter.clear();
    e.clear_messages();
}

/// A form row: the label in the fixed column, then `content`, top-aligned so a taller
/// field keeps its label on its first line.
fn form_row<R>(ui: &mut Ui, label: &str, content: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        label_cell(ui, label, FORM_LABEL);
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            content(ui)
        })
        .inner
    })
    .inner
}

fn label_cell(ui: &mut Ui, text: &str, width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, CONTROL), Sense::hover());
    ui.painter().text(
        rect.left_center(),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(13.0),
        MUTED(),
    );
}

fn text_field(ui: &mut Ui, value: &mut String, width: f32, hint: &str) -> Response {
    ui.add_sized(
        [width.max(60.0), CONTROL],
        TextEdit::singleline(value)
            .char_limit(64)
            .vertical_align(Align::Center)
            .hint_text(RichText::new(hint).color(FAINT()).italics()),
    )
}

/// The placeholders a command can use, and its inputs; a click inserts one at the cursor.
fn insert_chips(
    ui: &mut Ui,
    command: &mut CustomCommand,
    id: Id,
    widgets: &mut HashMap<String, UiControl>,
) {
    let url = command.action == CommandAction::Url;
    let tokens: Vec<(String, bool)> = COMMAND_PLACEHOLDERS
        .iter()
        .map(|p| ((*p).to_owned(), false))
        .chain(command.inputs.iter().filter(|i| valid_key(&i.key)).map(|i| {
            (format!("{{input:{}}}", i.key), i.kind == CommandInputKind::Checkbox)
        }))
        .collect();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        ui.label(RichText::new("Insert").size(12.0).color(HINT()));
        for (n, (token, whole_line)) in tokens.iter().enumerate() {
            let r = token_chip(ui, token).on_hover_text(if *whole_line {
                "A checkbox takes a whole line; unchecked leaves the line out."
            } else {
                "Inserted at the cursor"
            });
            control(widgets, format!("dialog-commands-insert-{n}"), &r, ui.clip_rect());
            if r.clicked() {
                let text = if url { &mut command.program } else { &mut command.arguments };
                insert(ui.ctx(), id, text, token, *whole_line && !url);
            }
        }
    });
}

/// The open command's fields under its row: name and submenu, where it shows, what it
/// runs, the questions it asks, and how it runs with the defaults. Returns whether a
/// command was added or deleted, which moves the rows after it.
fn edit_panel(
    ui: &mut Ui,
    e: &mut Editor,
    index: usize,
    widgets: &mut HashMap<String, UiControl>,
    out: &mut Out,
    scope: Option<&Path>,
) -> bool {
    let program = e.list[index].program.clone();
    if e.program_requested.as_deref() != Some(&program) {
        e.program_requested = None;
        e.program = Probe::default();
    }
    let requested = e.program_requested.as_deref() == Some(&program);
    let checked = if requested
        && e.list[index].action == CommandAction::Program
        && runnable_path(Path::new(&program))
    {
        e.program.status(ui.ctx(), &program, scope, check_program)
    } else {
        None
    };
    let busy = e.transfer.is_some();
    let room = e.list.len() < MAX_CUSTOM_COMMANDS;
    let (mut delete, mut duplicate, mut request_check) = (false, false, false);
    let command = &mut e.list[index];
    let panel = Frame::new()
        .fill(BLUE().gamma_multiply(0.04))
        .inner_margin(Margin { left: 18, right: 16, top: 6, bottom: 14 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 10.0;
            ui.add_enabled_ui(!busy, |ui| {
                ui.spacing_mut().item_spacing.y = 10.0;
                if ui.available_width() >= 600.0 {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        let half = (ui.available_width() - 12.0) / 2.0;
                        label_cell(ui, "Name", FORM_LABEL);
                        let r = text_field(ui, &mut command.name, half - FORM_LABEL - 12.0, "");
                        control(widgets, "dialog-commands-name", &r, ui.clip_rect());
                        label_cell(ui, "Submenu", 64.0);
                        let width = ui.available_width();
                        let r = text_field(ui, &mut command.group, width, "None · e.g. Tools / Review");
                        control(widgets, "dialog-commands-group", &r, ui.clip_rect());
                    });
                } else {
                    form_row(ui, "Name", |ui| {
                        let width = ui.available_width();
                        let r = text_field(ui, &mut command.name, width, "");
                        control(widgets, "dialog-commands-name", &r, ui.clip_rect());
                    });
                    form_row(ui, "Submenu", |ui| {
                        let width = ui.available_width();
                        let r = text_field(ui, &mut command.group, width, "None · e.g. Tools / Review");
                        control(widgets, "dialog-commands-group", &r, ui.clip_rect());
                    });
                }
                form_row(ui, "Shows in", |ui| {
                    let items: Vec<(&str, bool)> = CommandScope::ALL
                        .iter()
                        .map(|s| (s.label(), command.scope == *s))
                        .collect();
                    for (kind, r) in CommandScope::ALL.into_iter().zip(pages::segments(ui, &items)) {
                        control(widgets, format!("dialog-commands-scope-{}", kind.key()), &r, ui.clip_rect());
                        if r.clicked() {
                            command.scope = kind;
                        }
                    }
                });
                form_row(ui, "Runs", |ui| {
                    let options = [(CommandAction::Program, "A program"), (CommandAction::Url, "A web link")];
                    let items: Vec<(&str, bool)> =
                        options.iter().map(|(a, l)| (*l, command.action == *a)).collect();
                    for ((action, _), r) in options.into_iter().zip(pages::segments(ui, &items)) {
                        control(widgets, format!("dialog-commands-action-{}", action.key()), &r, ui.clip_rect());
                        if r.clicked() && command.action != action {
                            command.action = action;
                            if action == CommandAction::Url {
                                command.arguments.clear();
                            }
                        }
                    }
                });
                let url = command.action == CommandAction::Url;
                let id = Id::new(("custom-command-arguments", index));
                form_row(ui, if url { "Web link" } else { "Program" }, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let buttons = if url { 0.0 } else { 2.0 * 76.0 + 16.0 };
                        let r = ui.add_sized(
                            [(ui.available_width() - buttons).max(80.0), CONTROL],
                            TextEdit::singleline(&mut command.program)
                                .id(if url { id } else { Id::new(("command-program", index)) })
                                .vertical_align(Align::Center)
                                .font(TextStyle::Monospace)
                                .hint_text(
                                    RichText::new(if url {
                                        "https://example.com/commit/{commit}?file={path}"
                                    } else {
                                        r"C:\Program Files\Git\cmd\git.exe"
                                    })
                                    .monospace()
                                    .color(FAINT())
                                    .italics(),
                                ),
                        );
                        control(widgets, "dialog-commands-program", &r, ui.clip_rect());
                        if !url {
                            let b = ui.add_sized([76.0, CONTROL], Button::new("Browse"));
                            control(widgets, "dialog-commands-program-browse", &b, ui.clip_rect());
                            if b.clicked() {
                                out.browse = Some(PickTarget::CommandProgram(index));
                            }
                            let b = ui.add_enabled(
                                runnable_path(Path::new(&command.program)),
                                Button::new("Check").min_size(vec2(76.0, CONTROL)),
                            );
                            control(widgets, "dialog-commands-program-check", &b, ui.clip_rect());
                            request_check = b.clicked();
                        }
                    });
                    if url {
                        ui.label(
                            RichText::new("Values are URL-encoded and the host stays fixed; every open is reviewed first.")
                                .size(12.0)
                                .color(HINT()),
                        );
                        insert_chips(ui, command, id, widgets);
                    } else {
                        let same = command.program == program;
                        program_line(ui, &command.program, if same { checked } else { None }, requested && same);
                    }
                });
                if !url {
                    form_row(ui, "Arguments", |ui| {
                        let r = arguments_editor(ui, &mut command.arguments, id);
                        control(widgets, "dialog-commands-arguments", &r, ui.clip_rect());
                        insert_chips(ui, command, id, widgets);
                    });
                }
                form_row(ui, "Asks first", |ui| {
                    if command.inputs.is_empty() {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Nothing").color(HINT()));
                            let r = pages::ghost(ui, true, "Add a question");
                            control(widgets, "dialog-commands-add-input", &r, ui.clip_rect());
                            if r.clicked() {
                                add_input(command);
                            }
                        });
                    } else {
                        inputs_table(ui, command, widgets);
                        let r = pages::ghost(ui, command.inputs.len() < MAX_INPUTS, "Add a question");
                        control(widgets, "dialog-commands-add-input", &r, ui.clip_rect());
                        if r.clicked() {
                            add_input(command);
                        }
                    }
                });
                form_row(ui, if url { "Opens" } else { "Runs as" }, |ui| match preview(command) {
                    Ok(arguments) => {
                        if url {
                            ui.add(
                                Label::new(RichText::new(custom_command_display(&command.program)).monospace())
                                    .wrap(),
                            );
                        } else {
                            command_chips(ui, &command.program, &arguments);
                        }
                    }
                    Err(problem) => {
                        ui.add(Label::new(RichText::new(problem).color(AMBER())).wrap());
                    }
                });
                ui.allocate_ui_with_layout(
                    vec2(ui.available_width(), CONTROL),
                    Layout::right_to_left(Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let r = pages::danger_ghost(ui, true, "Delete");
                        control(widgets, "dialog-commands-delete", &r, ui.clip_rect());
                        delete = r.clicked();
                        let r = pages::ghost(ui, room, "Duplicate");
                        control(widgets, "dialog-commands-duplicate", &r, ui.clip_rect());
                        duplicate = r.clicked();
                    },
                );
            });
        });
    // The open row's accent runs down its fields.
    let rect = panel.response.rect;
    ui.painter()
        .rect_filled(Rect::from_min_size(rect.min, vec2(3.0, rect.height())), 0.0, BLUE());
    ui.painter()
        .hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, CARD_LINE()));
    if request_check {
        e.program_requested = Some(e.list[index].program.clone());
        e.program = Probe::default();
    }
    if delete {
        e.list.remove(index);
        e.selected = None;
        return true;
    }
    if duplicate {
        let mut copy = e.list[index].clone();
        copy.name = format!("{} copy", copy.name.trim());
        e.list.insert(index + 1, copy);
        e.selected = Some(index + 1);
        return true;
    }
    false
}

/// An absolute path to an .exe on Windows: the only programs a command may name.
fn runnable_path(path: &Path) -> bool {
    valid_custom_program_path(path)
}

/// What the program check found, under the Program field.
fn program_line(ui: &mut Ui, program: &str, checked: Option<std::result::Result<ProgramCheck, String>>, requested: bool) {
    let path = Path::new(program);
    let (image, color, text) = if program.trim().is_empty() {
        (None, HINT(), "The full path of the .exe this command runs.".to_owned())
    } else if !path.is_absolute() {
        (Some(Icon::Warning), AMBER(), "Use the full path, starting with the drive.".to_owned())
    } else if !runnable_path(path) {
        (Some(Icon::Warning), AMBER(), "Choose a local .exe path without hidden characters, network shares or device namespaces.".to_owned())
    } else {
        match checked {
            Some(Ok(ProgramCheck::Found)) => (Some(Icon::Check), GREEN(), format!("{} found", custom_command_display(&file_name(path)))),
            Some(Ok(ProgramCheck::Missing)) => (
                Some(Icon::Warning),
                AMBER(),
                "No file at this path. Check it, or choose the program with Browse.".to_owned(),
            ),
            Some(Err(reason)) => (None, HINT(), format!("Could not check this path: {reason}")),
            // Keeps the line's height while the check runs, so the form does not jump.
            None if !requested => (None, HINT(), "Not checked. Choose Check to inspect this path.".to_owned()),
            None => (None, HINT(), "Checking the path".to_owned()),
            Some(Ok(ProgramCheck::Invalid)) => (Some(Icon::Warning), AMBER(), "This program path is not allowed.".to_owned()),
            Some(Ok(ProgramCheck::Outside)) => return,
        }
    };
    status_line(ui, image, color, &text);
}

/// One status line with an optional icon; it keeps its height when empty.
fn status_line(ui: &mut Ui, image: Option<Icon>, color: Color32, text: &str) {
    ui.horizontal(|ui| {
        ui.set_min_height(18.0);
        ui.spacing_mut().item_spacing.x = 5.0;
        if let Some(image) = image {
            let (rect, _) = ui.allocate_exact_size(vec2(13.0, 13.0), Sense::hover());
            icon(ui.painter(), rect, image, color);
        }
        ui.add(Label::new(RichText::new(text).size(12.0).color(color)).wrap());
    });
}

/// The arguments, one per line, with line numbers and their placeholders in color.
fn arguments_editor(ui: &mut Ui, arguments: &mut String, id: Id) -> Response {
    let font = FontId::monospace(12.5);
    let gutter = 30.0;
    let frame = Frame::new()
        .fill(BG())
        .stroke(Stroke::new(1.0, BORDER()))
        .corner_radius(6)
        .inner_margin(Margin { left: gutter as i8, right: 0, top: 0, bottom: 0 })
        .show(ui, |ui| {
            let mut layouter = |ui: &Ui, text: &dyn TextBuffer, wrap_width: f32| {
                let mut job = highlighted(text.as_str(), font.clone());
                job.wrap.max_width = wrap_width;
                ui.fonts_mut(|f| f.layout_job(job))
            };
            TextEdit::multiline(arguments)
                .id(id)
                .frame(Frame::new().inner_margin(Margin::symmetric(8, 6)))
                .desired_rows(4)
                .desired_width(f32::INFINITY)
                .char_limit(16384)
                .hint_text(RichText::new("log\n-1\n{commit}").font(font.clone()).color(FAINT()))
                .layouter(&mut layouter)
                .show(ui)
        });
    let output = frame.inner;
    let rect = frame.response.rect;
    let painter = ui.painter();
    painter.vline(rect.left() + gutter, rect.y_range(), Stroke::new(1.0, HAIRLINE()));
    // A number at the first row of each line, so a wrapped argument keeps one number.
    let mut number = 1;
    let mut starts_line = true;
    for row in &output.galley.rows {
        if starts_line {
            painter.text(
                pos2(rect.left() + gutter - 7.0, output.galley_pos.y + row.rect().center().y),
                Align2::RIGHT_CENTER,
                number.to_string(),
                FontId::monospace(11.5),
                FAINT(),
            );
            number += 1;
        }
        starts_line = row.ends_with_newline;
    }
    let response = output.response.response;
    if response.has_focus() {
        painter.rect_stroke(rect, 6.0, Stroke::new(1.0, BLUE()), StrokeKind::Inside);
    }
    response
}

/// One argument line as text and placeholders.
#[derive(Debug, PartialEq, Eq)]
enum Piece<'a> {
    Text(&'a str),
    /// `{repo}`, `{branch}`, `{commit}` or `{file}`.
    Placeholder(&'a str),
    /// `{input:key}`: the key.
    Input(&'a str),
    /// Braces that form no placeholder; the definition will not save.
    Stray(&'a str),
}

fn pieces(text: &str) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find(['{', '}']) {
        if open > 0 {
            out.push(Piece::Text(&rest[..open]));
        }
        let tail = &rest[open..];
        match tail.starts_with('{').then(|| tail.find('}')).flatten() {
            Some(end) => {
                let token = &tail[..=end];
                out.push(if COMMAND_PLACEHOLDERS.contains(&token) {
                    Piece::Placeholder(token)
                } else if let Some(key) = token
                    .strip_prefix("{input:")
                    .and_then(|k| k.strip_suffix('}'))
                    .filter(|k| valid_key(k))
                {
                    Piece::Input(key)
                } else {
                    Piece::Stray(token)
                });
                rest = &tail[end + 1..];
            }
            None => {
                out.push(Piece::Stray(&tail[..1]));
                rest = &tail[1..];
            }
        }
    }
    if !rest.is_empty() {
        out.push(Piece::Text(rest));
    }
    out
}

fn valid_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= 32 && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Argument text with placeholders in the accent and stray braces in red.
fn highlighted(text: &str, font: FontId) -> text::LayoutJob {
    highlighted_in(text, font, TEXT())
}

/// `highlighted`, with plain text in `ink`.
fn highlighted_in(text: &str, font: FontId, ink: Color32) -> text::LayoutJob {
    let mut job = text::LayoutJob::default();
    for piece in pieces(text) {
        let (part, color) = match piece {
            Piece::Text(t) => (t.to_owned(), ink),
            Piece::Placeholder(t) => (t.to_owned(), BLUE()),
            Piece::Input(key) => (format!("{{input:{key}}}"), BLUE()),
            Piece::Stray(t) => (t.to_owned(), RED()),
        };
        job.append(&part, 0.0, TextFormat::simple(font.clone(), color));
    }
    job
}

/// Put `token` at the arguments' cursor, or on a new last line when the field has not
/// been used. A checkbox placeholder gets a line of its own.
fn insert(ctx: &Context, id: Id, text: &mut String, token: &str, whole_line: bool) {
    let mut state = TextEdit::load_state(ctx, id).unwrap_or_default();
    let cursor = state.cursor.char_range().map(|r| {
        text.char_indices()
            .nth(r.primary.index.0)
            .map_or(text.len(), |(byte, _)| byte)
    });
    let (at, piece) = match cursor {
        Some(at) if !whole_line => (at, token.to_owned()),
        _ => {
            let at = cursor.unwrap_or(text.len());
            let start = text[..at].rfind('\n').map_or(0, |n| n + 1);
            let end = text[at..].find('\n').map_or(text.len(), |n| at + n);
            if text[start..end].is_empty() {
                (start, token.to_owned())
            } else {
                (end, format!("\n{token}"))
            }
        }
    };
    text.insert_str(at, &piece);
    let after = text[..at + piece.len()].chars().count();
    state
        .cursor
        .set_char_range(Some(text::CCursorRange::one(text::CCursor::new(after))));
    state.store(ctx, id);
    ctx.memory_mut(|m| m.request_focus(id));
}

/// A placeholder to insert, as a small pill.
fn token_chip(ui: &mut Ui, token: &str) -> Response {
    let galley = ui
        .painter()
        .layout_no_wrap(token.to_owned(), FontId::monospace(11.5), MUTED());
    let (rect, response) =
        ui.allocate_exact_size(vec2(galley.size().x + 16.0, 22.0), Sense::click());
    let hot = ui.is_enabled() && (response.hovered() || response.has_focus());
    let edge = if hot { EDGE_ACCENT() } else { WINDOW_EDGE() };
    ui.painter().rect(
        rect,
        11.0,
        FIELD(),
        Stroke::new(1.0, edge),
        StrokeKind::Inside,
    );
    ui.painter().galley(
        rect.center() - galley.size() / 2.0,
        galley,
        if hot { TEXT() } else { MUTED() },
    );
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, token));
    response
}

fn add_input(command: &mut CustomCommand) {
    let key = (1..)
        .map(|i| format!("field{i}"))
        .find(|k| !command.inputs.iter().any(|i| &i.key == k))
        .unwrap();
    command.inputs.push(CommandInput {
        key,
        label: "Input".into(),
        required: true,
        ..Default::default()
    });
}

fn kind_label(kind: CommandInputKind) -> &'static str {
    match kind {
        CommandInputKind::Text => "Text",
        CommandInputKind::Branch => "Branch",
        CommandInputKind::File => "File",
        CommandInputKind::Checkbox => "Checkbox",
    }
}

/// The inputs as a table: key, label, kind, default, and Required (or, for a checkbox,
/// the argument it adds).
fn inputs_table(ui: &mut Ui, command: &mut CustomCommand, widgets: &mut HashMap<String, UiControl>) {
    let dialog_layer = ui.layer_id();
    let mut remove = None;
    Frame::new()
        .stroke(Stroke::new(1.0, CARD_STROKE()))
        .corner_radius(8)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            let gap = 8.0;
            let fixed = 96.0 + 96.0 + 112.0 + 24.0 + gap * 5.0;
            let flex = ((ui.available_width() - 20.0 - fixed) / 2.0).max(64.0);
            let widths = [96.0, flex, 96.0, flex, 112.0, 24.0];
            Frame::new()
                .fill(CARD())
                .corner_radius(CornerRadius { nw: 7, ne: 7, sw: 0, se: 0 })
                .inner_margin(Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for (title, width) in ["Key", "Label", "Kind", "Default", "Required / adds", ""].iter().zip(widths) {
                            cell(ui, width, 18.0, |ui| {
                                ui.label(RichText::new(*title).font(semibold(11.5)).color(HINT()));
                            });
                        }
                    });
                });
            for (i, input) in command.inputs.iter_mut().enumerate() {
                ui.push_id(i, |ui| {
                    let top = ui.cursor().top();
                    ui.painter()
                        .hline(ui.max_rect().x_range(), top, Stroke::new(1.0, CARD_LINE()));
                    Frame::new().inner_margin(Margin::symmetric(10, 5)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            let id = |field: &str| format!("dialog-commands-input-{i}-{field}");
                            cell(ui, widths[0], 26.0, |ui| {
                                let r = ui.add_sized(
                                    [widths[0], 24.0],
                                    TextEdit::singleline(&mut input.key)
                                        .vertical_align(Align::Center)
                                        .char_limit(32)
                                        .font(TextStyle::Monospace),
                                );
                                control(widgets, id("key"), &r, ui.clip_rect());
                            });
                            cell(ui, widths[1], 26.0, |ui| {
                                let r = ui.add_sized(
                                    [widths[1], 24.0],
                                    TextEdit::singleline(&mut input.label).char_limit(64).vertical_align(Align::Center),
                                );
                                control(widgets, id("label"), &r, ui.clip_rect());
                            });
                            cell(ui, widths[2], 26.0, |ui| {
                                let before = input.kind;
                                let r = ComboBox::from_id_salt(("input-kind", i))
                                    .selected_text(kind_label(input.kind))
                                    .width(widths[2])
                                    .show_ui(ui, |ui| {
                                        // Modal and popup share a layer order; keep the list above the dialog.
                                        ui.ctx().set_sublayer(dialog_layer, ui.layer_id());
                                        for kind in KINDS {
                                            if command.action==CommandAction::Url && kind==CommandInputKind::Checkbox {continue;}
                                            let r = ui.selectable_value(&mut input.kind, kind, kind_label(kind));
                                            control(widgets, id(kind.name()), &r, ui.clip_rect());
                                        }
                                    });
                                control(widgets, id("kind"), &r.response, ui.clip_rect());
                                if input.kind != before {
                                    kind_changed(input, before);
                                }
                            });
                            cell(ui, widths[3], 26.0, |ui| {
                                if input.kind == CommandInputKind::Checkbox {
                                    let mut on = input.default_value == "true";
                                    let r = ui.checkbox(&mut on, "On");
                                    control(widgets, id("default"), &r, ui.clip_rect());
                                    if r.changed() {
                                        input.default_value = on.to_string();
                                    }
                                } else {
                                    let mut edit = TextEdit::singleline(&mut input.default_value)
                                        .vertical_align(Align::Center)
                                        .char_limit(4096)
                                        .hint_text("Empty");
                                    if input.kind != CommandInputKind::Text {
                                        edit = edit.font(TextStyle::Monospace);
                                    }
                                    let r = ui.add_sized([widths[3], 24.0], edit);
                                    control(widgets, id("default"), &r, ui.clip_rect());
                                }
                            });
                            cell(ui, widths[4], 26.0, |ui| {
                                if input.kind == CommandInputKind::Checkbox {
                                    let r = ui.add_sized(
                                        [widths[4], 24.0],
                                        TextEdit::singleline(&mut input.checked_value)
                                            .vertical_align(Align::Center)
                                            .char_limit(4096)
                                            .font(TextStyle::Monospace)
                                            .hint_text("--flag"),
                                    );
                                    control(widgets, id("checked"), &r, ui.clip_rect());
                                } else {
                                    let r = ui.checkbox(&mut input.required, "Required");
                                    control(widgets, id("required"), &r, ui.clip_rect());
                                }
                            });
                            cell(ui, widths[5], 26.0, |ui| {
                                let r = crate::chrome::icon_button(ui, Icon::Close, true, false, 24.0)
                                    .on_hover_text("Remove input");
                                control(widgets, id("remove"), &r, ui.clip_rect());
                                if r.clicked() {
                                    remove = Some(i);
                                }
                            });
                        });
                    });
                });
            }
        });
    if let Some(i) = remove {
        command.inputs.remove(i);
    }
}

/// A checkbox's default is on or off and it adds one argument; other kinds take text.
fn kind_changed(input: &mut CommandInput, before: CommandInputKind) {
    if input.kind == CommandInputKind::Checkbox {
        input.default_value = "false".into();
        if input.checked_value.is_empty() {
            input.checked_value = "--flag".into();
        }
    } else if before == CommandInputKind::Checkbox {
        input.default_value.clear();
    }
}

/// A fixed-width table cell, its content centered vertically.
fn cell<R>(ui: &mut Ui, width: f32, height: f32, content: impl FnOnce(&mut Ui) -> R) -> R {
    ui.allocate_ui_with_layout(vec2(width, height), Layout::left_to_right(Align::Center), |ui| {
        ui.set_width(width);
        content(ui)
    })
    .inner
}

/// The command line with the defaults: inputs filled in, the selection placeholders
/// kept, and an unchecked checkbox's line left out.
fn preview(command: &CustomCommand) -> std::result::Result<Vec<Vec<(String, bool)>>, String> {
    command.validate().map_err(|e| plain(&e))?;
    let input = |key: &str| command.inputs.iter().find(|i| i.key == key);
    let mut arguments = Vec::new();
    let mut total = 0;
    let part = |text: &str, placeholder: bool, total: &mut usize| -> std::result::Result<_, String> {
        *total += text.len();
        if *total > CUSTOM_ARGUMENT_LIMIT {
            return Err("The expanded command line exceeds 32000 bytes.".into());
        }
        Ok((text.to_owned(), placeholder))
    };
    for line in command.argument_lines() {
        let parts = pieces(line);
        if let [Piece::Input(key)] = parts.as_slice()
            && let Some(i) = input(key).filter(|i| i.kind == CommandInputKind::Checkbox)
        {
            if i.default_value == "true" {
                total += 1;
                arguments.push(vec![part(&i.checked_value, false, &mut total)?]);
            }
            continue;
        }
        total += 1; // Match the runner: one separator per argument, not per fragment.
        arguments.push(
            parts
                .into_iter()
                .map(|p| match p {
                    Piece::Text(t) | Piece::Stray(t) => part(t, false, &mut total),
                    Piece::Placeholder(t) => part(t, true, &mut total),
                    Piece::Input(key) => part(input(key).map(|i| i.default_value.as_str()).unwrap_or_default(), false, &mut total),
                })
                .collect::<std::result::Result<_, _>>()?,
        );
    }
    Ok(arguments)
}

/// The program, then one chip per argument, each exactly as it is passed.
fn command_chips(ui: &mut Ui, program: &str, arguments: &[Vec<(String, bool)>]) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(5.0, 5.0);
        let path = Path::new(program.trim());
        argument_chip(ui, &[(file_name(path), false)], true)
            .on_hover_text(custom_command_display(&crate::views::display_path(path)));
        for (n, parts) in arguments.iter().enumerate() {
            let text: String = parts.iter().map(|(t, _)| t.as_str()).collect();
            let r = argument_chip(ui, parts, false);
            // Escapes can be wider than the raw value. Always expose the complete argument.
            r.on_hover_text(format!("Argument {}: {}", n + 1, custom_command_display(&text)));
        }
    });
}

/// One argument as a chip; placeholders in the accent, an empty argument named so.
fn argument_chip(ui: &mut Ui, parts: &[(String, bool)], program: bool) -> Response {
    let font = FontId::monospace(12.0);
    let mut job = text::LayoutJob::default();
    let ink = if program { ACCENT_TEXT() } else { TEXT() };
    if parts.iter().all(|(t, _)| t.is_empty()) {
        job.append("empty", 0.0, TextFormat { italics: true, ..TextFormat::simple(FontId::proportional(12.0), HINT()) });
    }
    for (text, placeholder) in parts {
        job.append(&custom_command_display(text), 0.0, TextFormat::simple(font.clone(), if *placeholder { BLUE() } else { ink }));
    }
    job.wrap = text::TextWrapping::truncate_at_width((ui.max_rect().width() - 14.0).max(40.0));
    let galley = ui.painter().layout_job(job);
    let (rect, response) = ui.allocate_exact_size(galley.size() + vec2(14.0, 4.0), Sense::hover());
    let fill = if program { ACCENT_SOFT() } else { CHIP() };
    ui.painter().rect_filled(rect, 4.0, fill);
    ui.painter().galley(rect.min + vec2(7.0, 2.0), galley, ink);
    response
}

/// The empty state: what custom commands are, and how to start.
fn empty_state(ui: &mut Ui, e: &mut Editor, widgets: &mut HashMap<String, UiControl>) {
    card(ui, Margin::symmetric(16, 32), |ui| {
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            let (rect, _) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
            icon(ui.painter(), rect, Icon::Console, FAINT());
            ui.label(RichText::new("No custom commands yet").font(semibold(14.0)));
            ui.scope(|ui| {
                ui.set_max_width(360.0);
                ui.add(
                    Label::new(
                        RichText::new("Run your own programs from the Repository menu and right-click menus.")
                            .color(HINT()),
                    )
                    .wrap(),
                );
            });
            let widths: Vec<f32> = ["New command", "Import"]
                .iter()
                .map(|t| {
                    ui.painter()
                        .layout_no_wrap((*t).to_owned(), FontId::proportional(13.0), TEXT())
                        .size()
                        .x
                        + 24.0
                })
                .collect();
            ui.allocate_ui_with_layout(
                vec2(widths[0] + widths[1] + 8.0, CONTROL),
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let r = pages::primary(ui, true, "New command");
                    control(widgets, "dialog-commands-empty-new", &r, ui.clip_rect());
                    if r.clicked() {
                        new_command(e);
                    }
                    let r = ui.button("Import");
                    control(widgets, "dialog-commands-empty-import", &r, ui.clip_rect());
                    if r.clicked() {
                        e.step = Step::Import;
                        e.clear_messages();
                    }
                },
            );
        });
    });
}

fn step_title(ui: &mut Ui, title: &str, about: &str) {
    ui.label(RichText::new(title).font(semibold(15.0)));
    ui.add(Label::new(RichText::new(about).color(MUTED())).wrap());
    ui.add_space(6.0);
}

/// The definitions file field and its Browse.
fn path_row(
    ui: &mut Ui,
    e: &mut Editor,
    widgets: &mut HashMap<String, UiControl>,
    out: &mut Out,
    target: PickTarget,
    hint: &str,
) {
    let idle = e.transfer.is_none();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let browse = 76.0;
        let width = ui.available_width() - browse - 8.0;
        let path = &mut e.exchange_path;
        let r = ui
            .add_enabled_ui(idle, |ui| {
                ui.add_sized(
                    [width, CONTROL],
                    TextEdit::singleline(path)
                        .vertical_align(Align::Center)
                        .font(TextStyle::Monospace)
                        .hint_text(hint),
                )
            })
            .inner;
        control(widgets, "dialog-commands-share-path", &r, ui.clip_rect());
        if r.changed() {
            e.clear_messages();
        }
        let b = ui
            .add_enabled_ui(idle, |ui| ui.add_sized([browse, CONTROL], Button::new("Browse")))
            .inner;
        control(widgets, "dialog-commands-share-browse", &b, ui.clip_rect());
        if b.clicked() {
            out.browse = Some(target);
        }
    });
}

/// The last import or export error, with Rocket's wording as small print.
fn failure(ui: &mut Ui, e: &Editor) {
    if let Some(error) = &e.error {
        status_line(ui, Some(Icon::Warning), RED(), error);
        if let Some(detail) = &e.error_detail {
            ui.add(Label::new(RichText::new(detail).size(11.0).monospace().color(FAINT())).wrap());
        }
    }
}

/// A running import or export, with Stop.
fn working(ui: &mut Ui, e: &mut Editor, widgets: &mut HashMap<String, UiControl>, text: &str) {
    ui.horizontal(|ui| {
        ui.add(Spinner::new().size(14.0));
        ui.label(RichText::new(text).color(MUTED()));
        let r = ui.button("Stop");
        control(widgets, "dialog-commands-transfer-cancel", &r, ui.clip_rect());
        if r.clicked() {
            let exporting = e.exporting.take().is_some();
            e.transfer = None;
            e.notice = Some(if exporting {
                "Cancelled. Check the folder: a finished export may already be there.".into()
            } else {
                "Import cancelled. Your commands are unchanged.".into()
            });
        }
    });
}

/// Back to the selected command, and the step's own action, right-aligned.
fn step_buttons(
    ui: &mut Ui,
    e: &mut Editor,
    widgets: &mut HashMap<String, UiControl>,
    (label, key, ready): (&str, &str, bool),
) -> bool {
    let mut go = false;
    ui.add_space(4.0);
    ui.allocate_ui_with_layout(vec2(ui.available_width(), CONTROL), Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let r = pages::primary(ui, ready, label);
        control(widgets, key, &r, ui.clip_rect());
        go = r.clicked();
        let r = ui.button("Back");
        control(widgets, "dialog-commands-back", &r, ui.clip_rect());
        if r.clicked() {
            e.step = Step::Edit;
            e.clear_messages();
        }
    });
    go
}

/// Import: choose a file, review what it holds, then add the new commands.
fn import_step(
    ui: &mut Ui,
    e: &mut Editor,
    widgets: &mut HashMap<String, UiControl>,
    out: &mut Out,
    scope: Option<&Path>,
    log: Option<&DiagnosticLog>,
) {
    step_title(ui, "Import commands", "Choose a definitions file. Nothing runs during import.");
    path_row(ui, e, widgets, out, PickTarget::CommandImport, r"C:\Users\you\Documents\rocket-commands.json");
    failure(ui, e);
    if let Some(n) = &e.notice {
        status_line(ui, None, HINT(), n);
    }
    if e.transfer.is_some() {
        working(ui, e, widgets, "Reading the file");
    } else if e.imported.is_some() {
        ui.add_space(4.0);
        import_review(ui, e, widgets);
    } else {
        let ready = !e.exchange_path.trim().is_empty();
        if step_buttons(ui, e, widgets, ("Review file", "dialog-commands-import", ready)) {
            e.start_transfer(true, scope, log);
        }
    }
}

/// Which incoming definitions are new: same type, group and name as one you have, or as an
/// earlier one in the file, is skipped.
fn fresh(list: &[CustomCommand], incoming: &[CustomCommand]) -> Vec<bool> {
    let same = |a: &CustomCommand, b: &CustomCommand| {
        a.scope == b.scope && a.name.trim() == b.name.trim() && a.group.trim() == b.group.trim()
    };
    incoming
        .iter()
        .enumerate()
        .map(|(i, c)| !list.iter().any(|old| same(old, c)) && !incoming[..i].iter().any(|old| same(old, c)))
        .collect()
}

/// What the file holds, each definition marked New or Same name.
fn import_review(ui: &mut Ui, e: &mut Editor, widgets: &mut HashMap<String, UiControl>) {
    let Some(incoming) = &e.imported else {
        return;
    };
    let fresh = fresh(&e.list, incoming);
    let adding = fresh.iter().filter(|f| **f).count();
    let skipped = incoming.len() - adding;
    let total = e.list.len() + adding;
    let room = total <= MAX_CUSTOM_COMMANDS;
    let file = custom_command_display(&file_name(Path::new(e.exchange_path.trim())));
    let (mut accept, mut cancel) = (false, false);
    pages::titled_card(
        ui,
        |ui| {
            check_mark(ui);
            ui.add(
                Label::new(
                    RichText::new(format!("{} in {file}", counted(incoming.len(), "command", "commands")))
                        .font(semibold(13.0)),
                )
                .truncate(),
            );
        },
        |ui| {
            for (n, (c, new)) in incoming.iter().zip(&fresh).enumerate() {
                if n > 0 {
                    pages::divider(ui);
                }
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.set_max_width(ui.available_width() - 90.0);
                        ui.spacing_mut().item_spacing.y = 1.0;
                        let mut job = text::LayoutJob::default();
                        job.append(&display_name(c), 0.0, TextFormat::simple(semibold(13.0), TEXT()));
                        if !c.group.trim().is_empty() {
                            job.append(&format!(" · {}", custom_command_display(c.group.trim())), 0.0, TextFormat::simple(FontId::proportional(13.0), HINT()));
                        }
                        job.wrap = text::TextWrapping::truncate_at_width(ui.available_width());
                        ui.label(job);
                        let line = format!("{} {}", file_name(Path::new(c.program.trim())), c.argument_lines().join(" "));
                        ui.add(Label::new(RichText::new(custom_command_display(&line)).monospace().size(12.0).color(HINT())).truncate())
                            .on_hover_text(definition(c));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let (text, color) = if *new { ("New", GREEN()) } else { ("Same name", AMBER()) };
                        ui.label(RichText::new(text).size(12.0).color(color));
                    });
                });
            }
            pages::divider(ui);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let note = if !room {
                    format!("Rocket keeps at most {MAX_CUSTOM_COMMANDS} commands; adding these would make {total}.")
                } else if skipped == 1 {
                    "1 has the same type, group and name as a command you have and is skipped.".to_owned()
                } else if skipped > 1 {
                    format!("{skipped} have the same type, group and name as commands you have and are skipped.")
                } else {
                    "Added commands are kept once you save commands.".to_owned()
                };
                let ready = room && adding > 0;
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let label = if adding == 0 { "Add commands".to_owned() } else { format!("Add {}", counted(adding, "command", "commands")) };
                    let r = pages::primary(ui, ready, &label);
                    control(widgets, "dialog-commands-import-accept", &r, ui.clip_rect());
                    accept = r.clicked();
                    let r = ui.button("Cancel import");
                    control(widgets, "dialog-commands-import-cancel", &r, ui.clip_rect());
                    cancel = r.clicked();
                    let color = if room && skipped == 0 { HINT() } else { AMBER() };
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.add(Label::new(RichText::new(note).size(12.5).color(color)).wrap());
                    });
                });
            });
        },
    );
    if accept {
        e.accept_import();
    } else if cancel {
        e.imported = None;
        e.notice = Some("Import cancelled. Your commands are unchanged.".into());
    }
}

/// A definition in full, for the review's tooltip.
fn definition(c: &CustomCommand) -> String {
    if c.action==CommandAction::Url {return format!("HTTPS URL template: {}\nValues are encoded; each open requires review.",custom_command_display(&c.program));}
    let mut text = format!("Program: {}\nArguments:", custom_command_display(&c.program));
    for line in c.argument_lines() {
        text.push_str("\n  ");
        text.push_str(&custom_command_display(line));
    }
    for i in &c.inputs {
        text.push_str(&format!("\nInput {{input:{}}}: {} ({})", custom_command_display(&i.key), custom_command_display(&i.label), kind_label(i.kind).to_lowercase()));
    }
    text
}

/// Export: every command to a new file; an existing file is never replaced.
fn export_step(
    ui: &mut Ui,
    e: &mut Editor,
    widgets: &mut HashMap<String, UiControl>,
    out: &mut Out,
    scope: Option<&Path>,
    log: Option<&DiagnosticLog>,
) {
    let count = e.list.len();
    let about = match count {
        0 => "There are no commands to export yet.".to_owned(),
        1 => "Writes the command to a new file. An existing file is never replaced.".to_owned(),
        n => format!("Writes all {n} commands to a new file. An existing file is never replaced."),
    };
    step_title(ui, "Export commands", &about);
    path_row(ui, e, widgets, out, PickTarget::CommandExport, r"C:\Users\you\Documents\rocket-commands.json");
    let path = e.exchange_path.trim().to_owned();
    let problem = e.problem();
    if e.error.is_some() {
        failure(ui, e);
    } else if let Some(n) = &e.notice {
        status_line(ui, Some(Icon::Check), GREEN(), n);
    } else if e.transfer.is_none() {
        if let Some(problem) = problem.as_ref().filter(|_| count > 0) {
            status_line(ui, Some(Icon::Warning), AMBER(), &format!("Fix this first: {problem}"));
        } else if path.is_empty() {
            status_line(ui, None, HINT(), "The full path of a new .json file.");
        } else if !Path::new(&path).is_absolute() {
            status_line(ui, Some(Icon::Warning), AMBER(), "Use the full path, starting with the drive.");
        } else {
            let (image, color, text) = match e.destination.status(ui.ctx(), &path, scope, check_export) {
                Some(Ok(ExportTarget::New)) => {
                    let folder = Path::new(&path).parent().map(file_name).unwrap_or_default();
                    (Some(Icon::Check), GREEN(), format!("New file in {folder}"))
                }
                Some(Ok(ExportTarget::Exists)) => (
                    Some(Icon::Warning),
                    AMBER(),
                    "A file with this name already exists. Rocket never replaces it; choose another name.".to_owned(),
                ),
                Some(Ok(ExportTarget::MissingFolder)) => {
                    (Some(Icon::Warning), AMBER(), "This folder does not exist.".to_owned())
                }
                Some(Ok(ExportTarget::Outside)) => (
                    Some(Icon::Warning),
                    AMBER(),
                    "Choose a file inside the test repositories folder.".to_owned(),
                ),
                Some(Err(reason)) => (None, HINT(), format!("Could not check this path: {reason}")),
                None => (None, HINT(), String::new()),
            };
            status_line(ui, image, color, &text);
        }
    }
    if e.transfer.is_some() {
        working(ui, e, widgets, "Writing the file");
    } else {
        let ready = count > 0 && !path.is_empty() && problem.is_none();
        let label = match count {
            0 | 1 => "Export command".to_owned(),
            n => format!("Export {n} commands"),
        };
        if step_buttons(ui, e, widgets, (&label, "dialog-commands-export", ready)) {
            e.start_transfer(false, scope, log);
        }
    }
}

/// The Preferences footer on Custom commands, laid out right to left: Save commands,
/// Discard once something changed, then where the list stands. `warn` says closing waits
/// for a decision. Returns (save, discard).
pub(crate) fn footer(
    ui: &mut Ui,
    e: &Editor,
    widgets: &mut HashMap<String, UiControl>,
    warn: bool,
) -> (bool, bool) {
    ui.spacing_mut().item_spacing.x = 8.0;
    let problem = e.problem();
    let ready = problem.is_none() && e.changed();
    let r = pages::primary(ui, ready, "Save commands");
    control(widgets, "dialog-commands-save", &r, ui.clip_rect());
    let save = r.clicked() || (ready && ui.input_mut(|i| i.consume_key(crate::shortcuts::primary_modifiers(), Key::Enter)));
    let mut discard = false;
    if e.changed() && e.transfer.is_none() {
        let r = ui.button("Discard");
        control(widgets, "dialog-commands-discard", &r, ui.clip_rect());
        discard = r.clicked();
    }
    let (working, dot, color, text) = if e.transfer.is_some() {
        let text = if e.exporting.is_some() { "Writing the definitions file" } else { "Reading the definitions file" };
        (true, false, HINT(), text.to_owned())
    } else if e.imported.is_some() {
        (false, false, HINT(), "Review the import, then add or cancel it".to_owned())
    } else if warn && e.changed() {
        (false, true, AMBER(), "Save or discard your command changes first".to_owned())
    } else if let Some(problem) = problem {
        (false, false, AMBER(), problem)
    } else if e.changed() {
        (false, true, AMBER(), format!("Unsaved changes · {}", unsaved(e)))
    } else {
        let text = match e.list.len() {
            0 => "No commands yet".to_owned(),
            n => format!("{} saved", counted(n, "command", "commands")),
        };
        (false, false, HINT(), text)
    };
    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if working {
            ui.add(Spinner::new().size(14.0));
        }
        if dot {
            let (rect, _) = ui.allocate_exact_size(vec2(7.0, 7.0), Sense::hover());
            ui.painter().circle_filled(rect.center(), 3.5, AMBER());
        }
        ui.add(Label::new(RichText::new(text).size(12.5).color(color)).truncate());
    });
    (save, discard)
}

/// What is unsaved, in a few words.
fn unsaved(e: &Editor) -> String {
    let (now, before) = (e.list.len(), e.saved.len());
    if now > before {
        format!("{} added", now - before)
    } else if now < before {
        format!("{} removed", before - now)
    } else {
        let edited: Vec<&CustomCommand> = e
            .list
            .iter()
            .zip(&e.saved)
            .filter(|(a, b)| a != b)
            .map(|(a, _)| a)
            .collect();
        match edited.as_slice() {
            [one] => format!("{} edited", display_name(one)),
            many => format!("{} edited", many.len()),
        }
    }
}

fn check_mark(ui: &mut Ui) {
    let (mark, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
    ui.painter()
        .circle_stroke(mark.center(), 6.0, Stroke::new(1.4, GREEN()));
    icon(ui.painter(), mark.shrink(3.5), Icon::Check, GREEN());
}

/// The name a command shows, never blank.
fn display_name(c: &CustomCommand) -> String {
    custom_command_display(match c.name.trim() {
        "" => "Unnamed command",
        name => name,
    })
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| crate::views::display_path(path), |n| n.to_string_lossy().into_owned())
}

fn counted(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

/// A definition problem in plain words, without the error type's prefixes.
fn plain(error: &Error) -> String {
    let text = user_error(error);
    let text = text.strip_prefix("invalid input: ").unwrap_or(&text);
    sentence(text.strip_prefix("custom command: ").unwrap_or(text))
}

/// `text` with a capital first letter and a closing period.
fn sentence(text: &str) -> String {
    let mut chars = text.trim().chars();
    let mut out: String = chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default();
    if !out.is_empty() && !out.ends_with(['.', '!', '?']) {
        out.push('.');
    }
    out
}

/// An import or export failure in plain words, and Rocket's wording as small print.
fn exchange_failure(error: &Error) -> (String, Option<String>) {
    let detail = Some(user_error(error));
    let message = match error {
        Error::Io { source, .. } => match source.kind() {
            std::io::ErrorKind::NotFound => "No file or folder at this path. Check it, or choose it with Browse.",
            std::io::ErrorKind::AlreadyExists => {
                "A file with this name already exists. Rocket never replaces it; choose another name."
            }
            std::io::ErrorKind::PermissionDenied => "Windows denied access to this file or its folder.",
            _ => "Rocket could not use this file.",
        }
        .to_owned(),
        Error::InvalidInput(text) if text.starts_with("command_definitions_invalid") => {
            "This is not a definitions file Rocket accepts: it needs version 1, at most 20 commands, at most 8 inputs per command and no other fields.".to_owned()
        }
        Error::InvalidInput(_) => plain(error),
        Error::Cancelled => return ("Cancelled.".into(), None),
        _ => return (user_error(error), None),
    };
    (message, detail)
}

/// Run dialog inputs: one label column, values to the right. Editable values are
/// ephemeral and expanded exactly once as literal arguments. Returns the file input
/// whose Browse was clicked, with its current value.
pub(crate) fn form(
    ui: &mut Ui,
    command: &CustomCommand,
    values: &mut BTreeMap<String, String>,
    branches: &[String],
    widgets: &mut HashMap<String, UiControl>,
) -> Option<(usize, String)> {
    let mut browse = None;
    for (index, input) in command.inputs.iter().enumerate() {
        let value = values
            .entry(input.key.clone())
            .or_insert_with(|| input.default_value.clone());
        let key = format!("dialog-command-input-{}", input.key);
        let required = input.required && input.kind != CommandInputKind::Checkbox;
        let blocking = missing(command, input, value);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            input_label(ui, &input.label, required);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                match input.kind {
                    CommandInputKind::Checkbox => {
                        let mut checked = value == "true";
                        let mut job = text::LayoutJob::default();
                        job.append("adds ", 0.0, TextFormat::simple(FontId::proportional(12.5), HINT()));
                        job.append(&custom_command_display(&input.checked_value), 0.0, TextFormat::simple(FontId::monospace(12.0), MUTED()));
                        ui.horizontal(|ui| {
                            ui.set_min_height(CONTROL);
                            let r = ui.checkbox(&mut checked, job);
                            control(widgets, &key, &r, ui.clip_rect());
                            if r.changed() {
                                *value = checked.to_string();
                            }
                        });
                    }
                    kind => {
                        ui.horizontal(|ui| {
                            let side = match kind {
                                CommandInputKind::Branch => 36.0,
                                CommandInputKind::File => 84.0,
                                _ => 0.0,
                            };
                            ui.spacing_mut().item_spacing.x = if kind == CommandInputKind::Branch { 4.0 } else { 8.0 };
                            let mut edit = TextEdit::singleline(value).char_limit(4096).vertical_align(Align::Center);
                            if kind != CommandInputKind::Text {
                                edit = edit.font(TextStyle::Monospace);
                            }
                            let r = ui.add_sized([ui.available_width() - side, CONTROL], edit);
                            if blocking && !r.has_focus() {
                                ui.painter().rect_stroke(r.rect, 6.0, Stroke::new(1.0, AMBER()), StrokeKind::Inside);
                            }
                            control(widgets, &key, &r, ui.clip_rect());
                            match kind {
                                CommandInputKind::Branch => {
                                    ui.spacing_mut().combo_width = 32.0;
                                    if let Some(branch) = crate::branch_picker::pick(
                                        ui,
                                        widgets,
                                        &format!("{key}-branch"),
                                        "",
                                        branches,
                                        !branches.is_empty(),
                                    ) {
                                        *value = branch;
                                    }
                                }
                                CommandInputKind::File => {
                                    let r = ui.add_sized([76.0, CONTROL], Button::new("Browse"));
                                    control(widgets, format!("{key}-browse"), &r, ui.clip_rect());
                                    if r.clicked() {
                                        browse = Some((index, value.clone()));
                                    }
                                }
                                _ => {}
                            }
                        });
                        if blocking {
                            ui.label(RichText::new("Required").size(12.0).color(AMBER()));
                        }
                    }
                }
            });
        });
        ui.add_space(4.0);
    }
    browse
}

/// An input's name in the label column, truncated to it; a required one gets a faint *.
fn input_label(ui: &mut Ui, label: &str, required: bool) {
    let mut job = text::LayoutJob::default();
    job.append(&custom_command_display(label), 0.0, TextFormat::simple(FontId::proportional(13.0), MUTED()));
    if required {
        job.append(" *", 0.0, TextFormat::simple(FontId::proportional(13.0), FAINT()));
    }
    job.wrap = text::TextWrapping::truncate_at_width(LABEL_WIDTH);
    let galley = ui.painter().layout_job(job);
    let (rect, response) = ui.allocate_exact_size(vec2(LABEL_WIDTH, CONTROL), Sense::hover());
    let elided = galley.elided;
    ui.painter().galley(
        pos2(rect.left(), rect.center().y - galley.size().y / 2.0),
        galley,
        MUTED(),
    );
    if elided {
        response.on_hover_text(label);
    }
}

/// A required input that the arguments use and that is still empty.
fn missing(command: &CustomCommand, input: &CommandInput, value: &str) -> bool {
    input.required
        && input.kind != CommandInputKind::Checkbox
        && value.is_empty()
        && (if command.action==CommandAction::Url {&command.program} else {&command.arguments}).contains(&format!("{{input:{}}}", input.key))
}

/// Where a run stands.
enum Plan {
    Ready(Vec<OsString>),
    Url(String),
    /// A required input is empty: its label.
    Missing(String),
    /// Why the command line cannot be built: its definition or the selection.
    Blocked(String),
}

fn plan(command: &CustomCommand, context: &CommandContext, values: &BTreeMap<String, String>) -> Plan {
    if let Err(e) = command.validate() {
        return Plan::Blocked(format!("{} Fix it in Custom commands.", plain(&e)));
    }
    let value = |i: &CommandInput| values.get(&i.key).cloned().unwrap_or_else(|| i.default_value.clone());
    if let Some(input) = command.inputs.iter().find(|i| missing(command, i, &value(i))) {
        return Plan::Missing(input.label.clone());
    }
    if command.action==CommandAction::Url { return match command.expand_url(context,values) {Ok(url)=>Plan::Url(url),Err(reason)=>Plan::Blocked(sentence(&reason))}; }
    match command.expand_with_values(context, values) {
        Ok(arguments) => Plan::Ready(arguments),
        Err(reason) => Plan::Blocked(sentence(&reason)),
    }
}

/// Run dialog body: exactly what will run. Returns the command line when it can.
pub(crate) fn confirmation(
    ui: &mut Ui,
    command: Option<&CustomCommand>,
    context: Option<&CommandContext>,
    values: &BTreeMap<String, String>,
) -> Option<(PathBuf, Vec<OsString>)> {
    let Some(command) = command else {
        pages::callout(ui, AMBER(), "This command was removed. Close and choose again.");
        return None;
    };
    let Some(context) = context else {
        pages::callout(ui, AMBER(), "Open a repository first.");
        return None;
    };
    let plan = plan(command, context, values);
    if context.historical_file {
        pages::callout(ui,HINT(),"Historical selection: {commit} is the selected revision; {path} is its relative path. {file} names the current checkout path, which may differ or no longer exist.");
    }
    if command.action==CommandAction::Url {
        pages::callout(ui,HINT(),"Open this exact address in your default browser. The selected values shown below will be sent to that website.");
        match &plan {
            Plan::Url(url)=>{ui.add(Label::new(RichText::new(custom_command_display(url)).monospace()).wrap());}
            Plan::Blocked(reason)=>{ui.add(Label::new(RichText::new(reason).color(AMBER())).wrap());}
            _=>{ui.label("Fill in the required inputs to review the destination.");}
        }
        return None;
    }
    if !command.inputs.is_empty() {
        ui.add_space(6.0);
    }
    let program = custom_command_display(&command.program);
    pages::titled_card(
        ui,
        |ui| {
            ui.label(RichText::new("What will run").font(semibold(13.0)));
            let shown = crate::repository_forms::fit_path(ui, &program, ui.available_width());
            ui.add(Label::new(RichText::new(shown).monospace().size(12.0).color(HINT())).truncate())
                .on_hover_text(&program);
        },
        |ui| match &plan {
            Plan::Ready(arguments) => {
                let parts: Vec<Vec<(String, bool)>> = arguments
                    .iter()
                    .map(|a| vec![(a.to_string_lossy().into_owned(), false)])
                    .collect();
                command_chips(ui, &command.program, &parts);
                ui.weak("Hidden characters and backslashes are shown as escapes. Original values are passed to the program.");
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let count = match arguments.len() {
                        0 => "No arguments".to_owned(),
                        1 => "1 literal argument".to_owned(),
                        n => format!("{n} literal arguments"),
                    };
                    ui.label(RichText::new(format!("{count} · in")).size(12.0).color(HINT()));
                    let folder = custom_command_display(&crate::views::display_path(&context.repository));
                    let shown = crate::repository_forms::fit_path(ui, &folder, ui.available_width());
                    ui.add(Label::new(RichText::new(shown).monospace().size(12.0).color(HINT())).truncate())
                        .on_hover_text(folder);
                });
            }
            Plan::Url(_) => unreachable!("URL confirmation returned above"),
            Plan::Missing(_) => {
                ui.label(RichText::new("Fill in the required inputs to see the exact command line.").color(HINT()));
            }
            Plan::Blocked(reason) => {
                ui.add(Label::new(RichText::new(reason).color(AMBER())).wrap());
            }
        },
    );
    match plan {
        Plan::Ready(arguments) => Some((PathBuf::from(&command.program), arguments)),
        _ => None,
    }
}

/// The Run dialog's footer status: where the run stands.
pub(crate) fn run_status(
    command: Option<&CustomCommand>,
    context: Option<&CommandContext>,
    values: &BTreeMap<String, String>,
) -> String {
    match (command, context) {
        (None, _) => "This command was removed".into(),
        (_, None) => "Open a repository first".into(),
        (Some(command), Some(context)) => match plan(command, context, values) {
            Plan::Ready(_) => "Stop in Activity ends what the command started".into(),
            Plan::Url(_) => "Opens after confirmation; browser navigation cannot be cancelled here".into(),
            Plan::Missing(label) => format!("Fill in {} to run", custom_command_display(&label)),
            Plan::Blocked(_) => "Nothing runs until this is resolved".into(),
        },
    }
}

/// Activity text for a finished command.
pub(crate) fn outcome(out: &mut Outcome, name: &str, report: CustomCommandReport) {
    out.summary = match &report.exit {
        Ok(Some(0)) => {
            out.command_succeeded = true;
            format!("{name} finished.")
        }
        Ok(Some(code)) => format!("{name} exited with code {code}."),
        Ok(None) => format!("{name} ended without an exit code."),
        Err(Error::Cancelled) => {
            format!("{name} was stopped, with the programs it started.")
        }
        Err(e) => {
            out.details.push(user_error(e));
            format!("{name} did not run to completion.")
        }
    };
    for (label, bytes) in [("Output", &report.stdout), ("Errors", &report.stderr)] {
        if !bytes.is_empty() {
            out.details.push(format!("{label}:"));
            out.details.extend(
                String::from_utf8_lossy(bytes)
                    .lines()
                    .take(200)
                    .map(str::to_owned),
            );
        }
    }
    if report.truncated {
        out.details.push(format!(
            "Only the first {} KiB of each stream is kept.",
            CUSTOM_OUTPUT_LIMIT / 1024
        ));
    }
    out.raw_output
        .push_str(&String::from_utf8_lossy(&report.stdout));
    out.raw_output
        .push_str(&String::from_utf8_lossy(&report.stderr));
    if let Err(e) = report.status {
        out.details
            .push(format!("Refresh changes: {}", user_error(&e)));
    }
}

/// The exact command line of custom command `index` in the owner's repository,
/// when its definition is valid and the selection has every value it uses.
pub fn command_line(
    commands: &[CustomCommand],
    tabs: &[Tab],
    owner: Option<u64>,
    index: usize,
    values: &BTreeMap<String, String>,
    captured: Option<&CommandContext>,
) -> Option<(String, PathBuf, Vec<OsString>)> {
    let command = commands.get(index)?;
    if command.action != CommandAction::Program { return None; }
    let tab = tabs.iter().find(|t| Some(t.id) == owner)?;
    command.validate().ok()?;
    let context = selected_context(tab, captured)?;
    let arguments = command.expand_with_values(&context, values).ok()?;
    Some((
        command.name.trim().to_owned(),
        PathBuf::from(&command.program),
        arguments,
    ))
}

pub fn command_url(commands:&[CustomCommand],tabs:&[Tab],owner:Option<u64>,index:usize,
    values:&BTreeMap<String,String>,captured:Option<&CommandContext>) -> Option<String> {
    let command=commands.get(index)?;
    let tab=tabs.iter().find(|t|Some(t.id)==owner)?;
    command.expand_url(&selected_context(tab,captured)?,values).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form_command() -> CustomCommand {
        CustomCommand {
            name: "Form".into(),
            program: if cfg!(windows) { r"C:\Tools\git.exe" } else { "/usr/bin/git" }.into(),
            arguments: "log\n{input:compact}\n--format={input:format}\n{commit}".into(),
            inputs: vec![
                CommandInput {
                    key: "compact".into(),
                    label: "Compact".into(),
                    kind: CommandInputKind::Checkbox,
                    default_value: "false".into(),
                    checked_value: "--oneline".into(),
                    ..Default::default()
                },
                CommandInput {
                    key: "format".into(),
                    label: "Format".into(),
                    required: true,
                    default_value: "%H".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn arguments_split_into_text_placeholders_and_stray_braces() {
        assert_eq!(
            pieces("--format={input:format} {commit}"),
            [
                Piece::Text("--format="),
                Piece::Input("format"),
                Piece::Text(" "),
                Piece::Placeholder("{commit}")
            ]
        );
        assert_eq!(
            pieces("a}b{c"),
            [Piece::Text("a"), Piece::Stray("}"), Piece::Text("b"), Piece::Stray("{"), Piece::Text("c")]
        );
        assert_eq!(pieces("{input:bad-key}"), [Piece::Stray("{input:bad-key}")]);
    }

    #[test]
    fn the_preview_fills_defaults_and_leaves_out_an_unchecked_box() {
        let shown = |c: &CustomCommand| -> Vec<String> {
            preview(c)
                .unwrap()
                .iter()
                .map(|parts| parts.iter().map(|(t, _)| t.as_str()).collect())
                .collect()
        };
        let mut command = form_command();
        assert_eq!(shown(&command), ["log", "--format=%H", "{commit}"]);
        // The selection placeholder is marked so it can be colored.
        assert!(preview(&command).unwrap()[2][0].1);
        command.inputs[0].default_value = "true".into();
        assert_eq!(shown(&command), ["log", "--oneline", "--format=%H", "{commit}"]);
        command.program = "git.exe".into();
        assert!(preview(&command).unwrap_err().starts_with("Choose the program"));
    }

    #[test]
    fn a_run_names_the_empty_required_input_before_the_missing_selection() {
        let command = form_command();
        let folder = if cfg!(windows) { r"C:\repo" } else { "/repo" };
        let context = CommandContext {
            repository: PathBuf::from(folder),
            ..Default::default()
        };
        let mut values = BTreeMap::from([("format".to_owned(), String::new())]);
        assert_eq!(run_status(Some(&command), Some(&context), &values), "Fill in Format to run");
        values.insert("format".into(), "%s".into());
        assert!(matches!(
            plan(&command, &context, &values),
            Plan::Blocked(reason) if reason == "{commit} needs a selected commit."
        ));
        assert_eq!(
            run_status(Some(&command), Some(&context), &values),
            "Nothing runs until this is resolved"
        );
        let head = "a".repeat(40);
        let context = CommandContext {
            commit: Some(ObjectId::parse(&head).unwrap()),
            ..context
        };
        assert!(matches!(
            plan(&command, &context, &values),
            Plan::Ready(a) if a == ["log", "--format=%s", head.as_str()].map(OsString::from)
        ));
        assert_eq!(run_status(None, Some(&context), &values), "This command was removed");
    }

    #[test]
    fn an_import_adds_new_definitions_and_skips_ones_you_have() {
        let have = CustomCommand {
            name: "Log".into(),
            group: "Reviews".into(),
            ..Default::default()
        };
        let named = |name: &str| CustomCommand {
            name: name.into(),
            ..Default::default()
        };
        let incoming = vec![
            CustomCommand {
                name: " Log ".into(),
                group: "Reviews".into(),
                ..Default::default()
            },
            named("Blame"),
            named("Blame"),
        ];
        assert_eq!(fresh(std::slice::from_ref(&have), &incoming), [false, true, false]);
        let mut e = Editor::new(vec![have]);
        e.step = Step::Import;
        e.imported = Some(incoming);
        e.accept_import();
        let names: Vec<&str> = e.list.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Log", "Blame"]);
        assert_eq!((e.selected, e.step, e.changed()), (Some(1), Step::Edit, true));
    }

    #[test]
    fn export_browse_keeps_a_typed_json_name() {
        let folder = Path::new(if cfg!(windows) { r"C:\Shared" } else { "/shared" });
        let mut e = Editor::new(Vec::new());
        e.exchange_path = if cfg!(windows) { r"C:\Old\team 文.json" } else { "/old/team 文.json" }.into();
        e.picked(true, folder);
        assert_eq!(Path::new(&e.exchange_path), folder.join("team 文.json"));
        e.exchange_path = "notes.txt".into();
        e.picked(true, folder);
        assert_eq!(Path::new(&e.exchange_path), folder.join(EXPORT_NAME));
        e.picked(false, &folder.join("in.json"));
        assert_eq!(Path::new(&e.exchange_path), folder.join("in.json"));
    }

    #[test]
    fn import_and_export_failures_read_as_words_with_the_code_as_small_print() {
        let invalid = Error::InvalidInput("command_definitions_invalid: expected version 1".into());
        let (message, detail) = exchange_failure(&invalid);
        assert!(message.starts_with("This is not a definitions file"), "{message}");
        assert!(detail.unwrap().contains("command_definitions_invalid"));
        let exists = Error::Io {
            action: "publish command export to a new filename",
            source: std::io::Error::from(std::io::ErrorKind::AlreadyExists),
        };
        assert!(exchange_failure(&exists).0.starts_with("A file with this name already exists"));
        assert_eq!(exchange_failure(&Error::Cancelled), ("Cancelled.".to_owned(), None));
    }
}

#[cfg(test)]
mod flow_tests {
    use super::Step;
    use crate::{
        app::{DialogKind, Intent},
        batch::flow_tests::{click, enabled, frame, open, wait},
        test_common as common,
    };
    use eframe::egui::{Context, Event, Modifiers, MouseWheelUnit, TouchPhase, vec2};
    use rocket_git::{CustomCommand, ObjectId};

    #[test]
    fn the_editor_saves_only_valid_changes_and_imports_after_review() {
        common::isolated(
            "ROCKET_COMMANDS_UI_CASE",
            "custom_commands::flow_tests::the_editor_saves_only_valid_changes_and_imports_after_review",
            || {
                let f = common::Fixture::new();
                f.write("a.txt", "1\n");
                f.commit("base");
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                let git = app.backend.as_ref().unwrap().executable.to_string_lossy().into_owned();
                app.dispatch(Intent::Preferences(crate::preferences::Page::Commands));
                // Nothing to save yet; the empty state offers a first command and Import.
                assert!(!enabled(&ctx, &mut app, "dialog-commands-save"));
                assert!(enabled(&ctx, &mut app, "dialog-commands-empty-import"));
                click(&ctx, &mut app, "dialog-commands-new");
                // A new command has no program yet, so it cannot be saved.
                assert!(!enabled(&ctx, &mut app, "dialog-commands-save"));
                fn editor(app: &mut crate::app::Rocket) -> &mut super::Editor {
                    app.dialog.as_mut().unwrap().commands.as_mut().unwrap()
                }
                editor(&mut app).list[0].program = git.clone();
                editor(&mut app).list[0].arguments = "rev-parse\nHEAD".into();
                assert!(enabled(&ctx, &mut app, "dialog-commands-save"));
                // Inserted where the field was never used: on a line of its own at the end.
                click(&ctx, &mut app, "dialog-commands-insert-2");
                assert_eq!(editor(&mut app).list[0].arguments, "rev-parse\nHEAD\n{commit}");
                // An import is reviewed first, and one you already have is skipped.
                let status = CustomCommand {
                    name: "Status".into(),
                    program: git,
                    arguments: "status".into(),
                    ..Default::default()
                };
                let duplicate = editor(&mut app).list[0].clone();
                editor(&mut app).step = Step::Import;
                editor(&mut app).imported = Some(vec![duplicate, status]);
                assert!(!enabled(&ctx, &mut app, "dialog-commands-save"));
                click(&ctx, &mut app, "dialog-commands-import-accept");
                let names: Vec<String> = editor(&mut app).list.iter().map(|c| c.name.clone()).collect();
                assert_eq!(names, ["New command", "Status"]);
                assert_eq!((editor(&mut app).selected, editor(&mut app).step), (Some(1), Step::Edit));
                // Delete ends the command's page; scroll down to it.
                let pane = app.frame_widgets["dialog-scroll"].rect.center();
                frame(
                    &ctx,
                    &mut app,
                    vec![
                        Event::PointerMoved(pane),
                        Event::MouseWheel {
                            unit: MouseWheelUnit::Point,
                            delta: vec2(0.0, -2000.0),
                            phase: TouchPhase::Move,
                            modifiers: Modifiers::NONE,
                        },
                    ],
                );
                click(&ctx, &mut app, "dialog-commands-delete");
                click(&ctx, &mut app, "dialog-commands-save");
                // Saved in place: Preferences stays open with nothing unsaved.
                assert!(!editor(&mut app).changed());
                assert_eq!(app.commands.len(), 1);
                assert_eq!(app.commands[0].arguments, "rev-parse\nHEAD\n{commit}");
            },
        );
    }

    #[test]
    fn a_saved_command_runs_only_after_confirmation_with_its_selection() {
        common::isolated(
            "ROCKET_COMMANDS_UI_CASE",
            "custom_commands::flow_tests::a_saved_command_runs_only_after_confirmation_with_its_selection",
            || {
                let f = common::Fixture::new();
                f.write("a.txt", "1\n");
                f.commit("base");
                let head = String::from_utf8(f.run(&["rev-parse", "HEAD"])).unwrap();
                let head = head.trim().to_owned();
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                let git = app.backend.as_ref().unwrap().executable.clone();
                let menu = |app: &mut crate::app::Rocket, key: &str| {
                    click(&ctx, app, "menu-repository");
                    click(&ctx, app, "repository-custom-commands");
                    click(&ctx, app, key);
                };
                menu(&mut app, "command-edit");
                assert_eq!(app.preferences_page(), Some(crate::preferences::Page::Commands));
                click(&ctx, &mut app, "dialog-commands-new");
                let editor = app.dialog.as_mut().unwrap().commands.as_mut().unwrap();
                editor.list[0] = CustomCommand {
                    name: "Show commit".into(),
                    program: git.to_string_lossy().into_owned(),
                    arguments: "log\n-1\n--format=%H %s\n{commit}".into(),
                    ..Default::default()
                };
                click(&ctx, &mut app, "dialog-commands-save");
                click(&ctx, &mut app, "dialog-close");
                assert!(app.dialog.is_none());
                assert_eq!(app.commands.len(), 1);
                // {commit} needs a selected commit: the run stays unavailable until then.
                app.tabs[0].selected_commit = None;
                app.dispatch(Intent::Dialog(DialogKind::RunCommand(0)));
                assert!(!enabled(&ctx, &mut app, "dialog-submit"));
                click(&ctx, &mut app, "dialog-cancel");
                app.dispatch(Intent::SelectCommit(ObjectId::parse(&head).unwrap()));
                wait(&ctx, &mut app);
                menu(&mut app, "command-run-0");
                click(&ctx, &mut app, "dialog-submit");
                wait(&ctx, &mut app);
                let activity = app.activities.last().unwrap();
                assert!(
                    activity.outcome.command_succeeded,
                    "{:?}",
                    activity.outcome.details
                );
                assert_eq!(activity.outcome.summary, "Show commit finished.");
                assert!(
                    activity
                        .outcome
                        .details
                        .iter()
                        .any(|d| d == &format!("{head} base")),
                    "{:?}",
                    activity.outcome.details
                );
            },
        );
    }

    #[test]
    fn a_url_command_opens_once_after_review_and_dismisses_the_dialog() {
        common::isolated(
            "ROCKET_COMMANDS_UI_CASE",
            "custom_commands::flow_tests::a_url_command_opens_once_after_review_and_dismisses_the_dialog",
            || {
                let f = common::Fixture::new();
                f.write("a.txt", "1\n");
                f.commit("base");
                let head = String::from_utf8(f.run(&["rev-parse", "HEAD"])).unwrap();
                let head = ObjectId::parse(head.trim()).unwrap();
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                app.opened_urls = Some(Vec::new());
                app.commands = vec![CustomCommand {
                    name: "Review historical file".into(),
                    scope: rocket_git::CommandScope::File,
                    action: rocket_git::CommandAction::Url,
                    program: "https://example.invalid/review?commit={commit}&file={path}".into(),
                    ..Default::default()
                }];
                let captured = super::historical_context(
                    &super::context(&app.tabs[0]),
                    &head,
                    "旧 文件.txt".as_bytes(),
                );
                let expected = app.commands[0]
                    .expand_url(&captured, &Default::default())
                    .unwrap();
                app.dispatch(Intent::RunContextCommand { index: 0, context: captured.clone() });
                click(&ctx, &mut app, "dialog-cancel");
                assert!(app.dialog.is_none());
                assert!(app.opened_urls.as_ref().unwrap().is_empty());

                app.dispatch(Intent::RunContextCommand { index: 0, context: captured });
                click(&ctx, &mut app, "dialog-submit");
                assert_eq!(app.opened_urls.as_ref().unwrap(), &[expected]);
                assert!(app.dialog.is_none(), "URL confirmation must close after opening");
                frame(&ctx, &mut app, Vec::new());
                assert_eq!(app.opened_urls.as_ref().unwrap().len(), 1);
                assert!(app.write.is_none());
            },
        );
    }

    /// Preferences stays open while command edits are unsaved, on any page; Discard
    /// lets it close without saving.
    #[test]
    fn preferences_keeps_unsaved_commands_until_saved_or_discarded() {
        common::isolated(
            "ROCKET_COMMANDS_UI_CASE",
            "custom_commands::flow_tests::preferences_keeps_unsaved_commands_until_saved_or_discarded",
            || {
                let f = common::Fixture::new();
                f.write("a.txt", "1\n");
                f.commit("base");
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                fn editor(app: &mut crate::app::Rocket) -> &mut super::Editor {
                    app.dialog.as_mut().unwrap().commands.as_mut().unwrap()
                }
                app.dispatch(Intent::Preferences(crate::preferences::Page::Commands));
                click(&ctx, &mut app, "dialog-commands-new");
                // Another page keeps the edit; closing returns to it and asks for a decision.
                click(&ctx, &mut app, "pref-page-general");
                click(&ctx, &mut app, "dialog-close");
                assert_eq!(app.preferences_page(), Some(crate::preferences::Page::Commands));
                assert!(app.preferences.unsaved_warning);
                assert_eq!(editor(&mut app).list.len(), 1);
                assert!(!enabled(&ctx, &mut app, "dialog-commands-save"), "a command without a program cannot be saved");
                click(&ctx, &mut app, "dialog-commands-discard");
                assert!(editor(&mut app).list.is_empty() && !app.preferences.unsaved_warning);
                click(&ctx, &mut app, "dialog-close");
                assert!(app.dialog.is_none());
                assert!(app.commands.is_empty());
            },
        );
    }
}

/// Captured contexts belong to the dialog's repository, never the newly active tab.
pub fn selected_context(tab: &Tab, captured: Option<&CommandContext>) -> Option<CommandContext> {
    match captured {
        Some(c) if c.repository == tab.path => Some(c.clone()),
        Some(_) => None,
        None => Some(context(tab)),
    }
}

pub(crate) fn context_menu(ui: &mut Ui, commands: &[CustomCommand], scope: CommandScope,
    context: &CommandContext, intents: &mut Vec<crate::app::Intent>, widgets: &mut HashMap<String, UiControl>) {
    if !commands.iter().any(|c| c.scope == scope) { return; }
    ui.separator();
    ui.label(RichText::new(format!("{} commands",scope.label())).size(11.0).color(HINT()));
    let available = match scope {
            CommandScope::Repository => true,
            CommandScope::Commit => context.commit.is_some(),
            CommandScope::File => context.file.is_some(),
            CommandScope::Branch => context.branch.is_some(),
            CommandScope::Submodule => context.submodule.is_some(),
    };
    menu_entries(ui,commands,scope,Some(context),available,intents,widgets);
}

/// At most twenty definitions; grouping is bounded by each 64-character group path.
pub(crate) fn menu_entries(ui:&mut Ui,commands:&[CustomCommand],scope:CommandScope,
    context:Option<&CommandContext>,available:bool,intents:&mut Vec<Intent>,widgets:&mut HashMap<String,UiControl>) {
    let rows:Vec<_>=commands.iter().enumerate().filter(|(_,c)|c.scope==scope).map(|(i,c)|
        (i,c.group.split('/').map(str::trim).filter(|s|!s.is_empty()).collect::<Vec<_>>())).collect();
    CommandMenu { commands, context, available, intents, widgets }.level(ui, &rows, 0);
}
struct CommandMenu<'a> {
    commands: &'a [CustomCommand],
    context: Option<&'a CommandContext>,
    available: bool,
    intents: &'a mut Vec<Intent>,
    widgets: &'a mut HashMap<String, UiControl>,
}
impl CommandMenu<'_> {
    fn level(&mut self, ui: &mut Ui, rows: &[(usize, Vec<&str>)], depth: usize) {
        let prefix = if self.context.is_some() { "context-command" } else { "command-run" };
        for (index, _) in rows.iter().filter(|(_, path)| path.len() == depth) {
            let command = &self.commands[*index];
            let response = ui.add_enabled(self.available && command.validate().is_ok(), Button::new(display_name(command)));
            control(self.widgets, format!("{prefix}-{index}"), &response, ui.clip_rect());
            if response.clicked() {
                self.intents.push(match self.context {
                    Some(context) => Intent::RunContextCommand { index: *index, context: context.clone() },
                    None => Intent::Dialog(DialogKind::RunCommand(*index)),
                });
                ui.close();
            }
        }
        let mut groups = BTreeMap::<&str, Vec<(usize, Vec<&str>)>>::new();
        for (index, path) in rows.iter().filter(|(_, path)| path.len() > depth) {
            groups.entry(path[depth]).or_default().push((*index, path.clone()));
        }
        for (label, children) in groups {
            let first = children[0].0;
            let row = ui.menu_button(custom_command_display(label), |ui| self.level(ui, &children, depth + 1));
            control(self.widgets, format!("{prefix}-group-{first}-{depth}"), &row.response, ui.clip_rect());
        }
    }
}

pub(crate) fn historical_context(base:&CommandContext,commit:&ObjectId,path:&[u8])->CommandContext {
    let mut context=base.clone();
    context.commit=Some(commit.clone());
    context.file=std::str::from_utf8(path).ok().map(|p|context.repository.join(p.replace('/',std::path::MAIN_SEPARATOR_STR)));
    context.historical_file=true;
    context
}

#[cfg(test)]
mod security_tests;
