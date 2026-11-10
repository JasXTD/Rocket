//! Search inside files on the Files page: the search row, results grouped by file
//! as Git streams them, and a file opened at a match. Git runs on a worker; Stop
//! cancels it at once and keeps what was found.
use crate::{
    app::*,
    backend::{Backend, user_error},
    pages,
    style::*,
    views::{path_field, query_field},
};
use eframe::egui::*;
use rocket_git::*;
use std::{collections::HashMap, ops::Range, sync::mpsc, time::Duration};

/// Files with more matches than this start collapsed.
const OPEN_MATCHES: usize = 20;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    /// Files on disk now, untracked ones included and ignored ones skipped.
    #[default]
    Checkout,
    /// Files as committed at the revision the folders show.
    Revision,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    Complete,
    TooMany,
    TimedOut,
    Stopped,
}

pub struct Group {
    pub file: FileSearchFile,
    pub detail: FileSearchDetail,
    pub open: bool,
    pub not_utf8: bool,
}

/// A file opened at a match; `content` arrives from a worker.
pub struct Opened {
    pub group: usize,
    pub line: usize,
    /// The match's 1-based line number.
    pub number: u64,
    pub path: Vec<u8>,
    pub source: FileSearchSource,
    pub content: Option<crate::tree::Content>,
    pub error: Option<String>,
    /// Scroll the match into view on the next frame.
    pub scroll: bool,
}

#[derive(Default)]
pub struct State {
    pub query: String,
    pub regex: bool,
    pub match_case: bool,
    pub scope: Scope,
    pub path: String,
    /// The left pane shows results rather than folders.
    pub results: bool,
    pub groups: Vec<Group>,
    pub matches: usize,
    pub ended: Option<Ended>,
    pub error: Option<String>,
    pub bad_pattern: bool,
    pub elapsed: Option<Duration>,
    /// What the shown results were searched with, and where, in words.
    pub ran: Option<(FileSearchQuery, String)>,
    pub selected: Option<(usize, usize)>,
    pub opened: Option<Opened>,
    /// Narrow window: the file is shown instead of the results.
    pub show_file: bool,
    pub stopping: bool,
    /// Give the search field the keyboard on the next frame.
    pub focus: bool,
    task: Option<ReadTask<FileSearchSummary>>,
    incoming: Option<mpsc::Receiver<FileSearchFile>>,
    open_task: Option<ReadTask<crate::tree::Content>>,
}

impl State {
    pub fn running(&self) -> bool {
        self.task.is_some()
    }
    pub fn opening(&self) -> bool {
        self.open_task.is_some()
    }
    /// Stop the search; the files found so far stay.
    pub fn stop(&mut self) {
        if let Some(task) = &self.task {
            task.cancel();
            self.stopping = true;
        }
    }

    /// Start a search of the checkout, or of `revision` (the commit the folders show).
    pub(crate) fn start(&mut self, backend: Backend, repo: Repository, revision: Option<&ObjectId>) {
        // Dropping a task cancels it, and its late results can't arrive.
        self.task = None;
        self.incoming = None;
        self.open_task = None;
        self.groups.clear();
        self.matches = 0;
        self.ended = None;
        self.error = None;
        self.bad_pattern = false;
        self.elapsed = None;
        self.selected = None;
        self.opened = None;
        self.show_file = false;
        self.stopping = false;
        self.results = true;
        self.ran = None;
        if self.query.trim().is_empty() {
            return;
        }
        let path = match crate::history::search_path(&self.path) {
            Ok(path) => path,
            Err(reason) => {
                self.error = Some(reason);
                return;
            }
        };
        let (source, label) = match (self.scope, revision) {
            (Scope::Checkout, _) => (FileSearchSource::Checkout, crate::i18n::tr("the checkout").to_owned()),
            (Scope::Revision, Some(id)) => (FileSearchSource::Commit(id.clone()), short(id)),
            (Scope::Revision, None) => {
                self.error = Some(crate::i18n::tr("Browse a revision first, or search the checkout.").into());
                return;
            }
        };
        let query = FileSearchQuery {
            text: self.query.clone(),
            regex: self.regex,
            match_case: self.match_case,
            path,
            source,
        };
        self.ran = Some((query.clone(), label));
        let (sender, receiver) = mpsc::channel();
        match ReadTask::spawn(move |cancel| {
            backend.log.request("file-search", |log| {
                repo.with_log(log.clone()).search_files(&query, &cancel, &mut |file| {
                    let _ = sender.send(file);
                })
            })
        }) {
            Ok(task) => {
                self.task = Some(task);
                self.incoming = Some(receiver);
            }
            Err(error) => self.error = Some(user_error(&error)),
        }
    }

    fn push(&mut self, file: FileSearchFile) {
        self.matches += file.lines.len();
        let not_utf8 = file.lines.iter().any(|l| l.not_utf8);
        let open = file.lines.len() <= OPEN_MATCHES;
        self.groups.push(Group {
            file,
            detail: FileSearchDetail::default(),
            open,
            not_utf8,
        });
    }

    /// Take what the workers delivered. True while something still runs.
    pub(crate) fn poll(&mut self) -> bool {
        if let Some(receiver) = &self.incoming {
            let files: Vec<_> = receiver.try_iter().collect();
            for file in files {
                self.push(file);
            }
        }
        if let Some(result) = self.task.as_mut().and_then(ReadTask::try_finish) {
            self.task = None;
            self.stopping = false;
            if let Some(receiver) = self.incoming.take() {
                for file in receiver.try_iter() {
                    self.push(file);
                }
            }
            match result {
                Ok(summary) => {
                    self.ended = Some(match summary.end {
                        FileSearchEnd::Complete => Ended::Complete,
                        FileSearchEnd::TooMany => Ended::TooMany,
                        FileSearchEnd::TimedOut => Ended::TimedOut,
                    });
                    self.elapsed = Some(summary.elapsed);
                    for (group, detail) in self.groups.iter_mut().zip(summary.details) {
                        group.detail = detail;
                    }
                }
                Err(Error::Cancelled) => self.ended = Some(Ended::Stopped),
                Err(error) => {
                    self.bad_pattern = pattern_error(&error).is_some();
                    self.error = Some(match &error {
                        Error::InvalidInput(message) => message.clone(),
                        _ => user_error(&error),
                    });
                }
            }
        }
        if let Some(result) = self.open_task.as_mut().and_then(ReadTask::try_finish) {
            self.open_task = None;
            if let Some(opened) = &mut self.opened {
                match result {
                    Ok(content) => opened.content = Some(content),
                    Err(Error::Cancelled) => {}
                    Err(error) => {
                        opened.error = Some(match &error {
                            Error::InvalidInput(message) => message.clone(),
                            _ => crate::tree::tree_error(&error, true),
                        })
                    }
                }
            }
        }
        self.running() || self.opening()
    }

    /// Open the file of match `line` in file `group` at that line.
    pub(crate) fn open(&mut self, backend: Backend, repo: Repository, group: usize, line: usize) {
        let Some((query, _)) = &self.ran else { return };
        let Some(found) = self.groups.get(group) else { return };
        let Some(hit) = found.file.lines.get(line) else { return };
        self.selected = Some((group, line));
        self.show_file = true;
        if let Some(opened) = &mut self.opened
            && opened.group == group
            && opened.error.is_none()
            && let Some(content) = &opened.content
            && (content.first..content.first + content.lines.len()).contains(&(hit.number as usize))
        {
            // The same file, and its shown lines include this one: only move there.
            opened.line = line;
            opened.number = hit.number;
            opened.scroll = true;
            return;
        }
        let source = query.source.clone();
        let path = found.file.path.clone();
        let number = hit.number;
        let columns: HashMap<u64, Vec<Range<usize>>> = found
            .file
            .lines
            .iter()
            .map(|l| (l.number, l.columns.clone()))
            .collect();
        self.opened = Some(Opened {
            group,
            line,
            number,
            path: path.clone(),
            source: source.clone(),
            content: None,
            error: None,
            scroll: true,
        });
        match ReadTask::spawn(move |cancel| {
            backend.log.request("file-search-open", |log| {
                let content = repo
                    .with_log(log.clone())
                    .search_file(&source, &path, number, &cancel)?;
                crate::tree::prepare_search(content, number, &columns, &cancel)
            })
        }) {
            Ok(task) => self.open_task = Some(task),
            Err(error) => {
                if let Some(opened) = &mut self.opened {
                    opened.error = Some(user_error(&error));
                }
            }
        }
    }

    /// The match `delta` steps from the selected one, across files.
    pub(crate) fn step(&self, delta: i32) -> Option<(usize, usize)> {
        let all: Vec<(usize, usize)> = self
            .groups
            .iter()
            .enumerate()
            .flat_map(|(g, group)| (0..group.file.lines.len()).map(move |l| (g, l)))
            .collect();
        if all.is_empty() {
            return None;
        }
        let at = self
            .selected
            .and_then(|s| all.iter().position(|m| *m == s))
            .map_or(if delta > 0 { -1 } else { 0 }, |i| i as i64);
        let next = (at + i64::from(delta)).clamp(0, all.len() as i64 - 1) as usize;
        Some(all[next])
    }
}

fn short(id: &ObjectId) -> String {
    id.as_str().chars().take(7).collect()
}

/// A file's size as the Files page shows it.
fn size(bytes: u64) -> String {
    crate::tree::size(bytes as usize)
}

enum Row {
    File(usize),
    Line(usize, usize),
    More,
}

fn rows(state: &State) -> Vec<Row> {
    let mut rows = Vec::new();
    for (g, group) in state.groups.iter().enumerate() {
        rows.push(Row::File(g));
        if group.open {
            rows.extend((0..group.file.lines.len()).map(|l| Row::Line(g, l)));
            if group.file.capped {
                rows.push(Row::More);
            }
        }
    }
    rows
}

fn emit(intents: &mut Vec<Intent>, intent: crate::tree::TreeIntent) {
    intents.push(Intent::Tree(intent));
}

/// The search row under Browse revision.
pub(crate) fn search_row(
    ui: &mut Ui,
    s: &mut State,
    revision: Option<&ObjectId>,
    loading: bool,
    narrow: bool,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
) {
    let row = |ui: &mut Ui, s: &mut State, intents: &mut Vec<Intent>, widgets: &mut HashMap<String, UiControl>, part: u8| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if part != 2 {
            let width = if narrow { (ui.available_width() - 150.0).max(160.0) } else { 360.0 };
            let (field, _) = query_field(
                ui,
                &mut s.query,
                crate::i18n::tr("Search in files"),
                width,
                Some((&mut s.match_case, &mut s.regex)),
                s.bad_pattern,
                ["files-search", "files-search-case", "files-search-regex"],
                widgets,
            );
            if s.focus {
                field.request_focus();
                s.focus = false;
            }
            if field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                emit(intents, crate::tree::TreeIntent::Search);
            }
            scope_picker(ui, s, revision, widgets);
        }
        if part != 1 {
            let path_width = if narrow { (ui.available_width() - 90.0).clamp(120.0, 300.0) } else { 170.0 };
            path_field(ui, &mut s.path, path_width, "files-search-path", widgets);
            if s.running() {
                let r = ui.add_enabled(!s.stopping, Button::new(crate::i18n::tr("Stop")));
                widgets.insert("files-search-stop".into(), (&r).into());
                if r.clicked() {
                    emit(intents, crate::tree::TreeIntent::StopSearch);
                }
            } else {
                // A search of the revision waits until the revision chosen has loaded.
                let ready = !(s.scope == Scope::Revision && loading);
                let r = pages::primary(ui, !s.query.trim().is_empty() && ready, crate::i18n::tr("Search"));
                widgets.insert("files-search-go".into(), (&r).into());
                if r.clicked() {
                    emit(intents, crate::tree::TreeIntent::Search);
                }
            }
            if !narrow && let Some(text) = done_text(s) {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add(Label::new(RichText::new(text).size(12.0).color(FAINT())).truncate());
                });
            }
        }
    };
    if narrow {
        ui.horizontal(|ui| row(ui, s, intents, widgets, 1));
        ui.horizontal(|ui| row(ui, s, intents, widgets, 2));
    } else {
        ui.horizontal(|ui| row(ui, s, intents, widgets, 0));
    }
}

/// "105 matches in 5 files · 0.3 s" once a search finished.
fn done_text(s: &State) -> Option<String> {
    let ended = s.ended?;
    if s.groups.is_empty() || ended == Ended::Stopped {
        return None;
    }
    let mut text = crate::i18n::trf!("Matches: {matches} · files: {files}", matches = pages::number_text(s.matches), files = pages::number_text(s.groups.len()));
    if let Some(elapsed) = s.elapsed {
        text.push_str(&format!(" · {:.1} s", elapsed.as_secs_f32()));
    }
    Some(text)
}

/// "In: Checkout" or "In: <revision>".
fn scope_picker(ui: &mut Ui, s: &mut State, revision: Option<&ObjectId>, widgets: &mut HashMap<String, UiControl>) {
    let label = match (s.scope, revision) {
        (Scope::Checkout, _) | (Scope::Revision, None) => crate::i18n::tr("Checkout").to_owned(),
        (Scope::Revision, Some(id)) => short(id),
    };
    let picker = ui.add(Button::new("").frame(false).min_size(vec2(128.0, 30.0)));
    widgets.insert("files-search-scope".into(), (&picker).into());
    let rect = picker.rect;
    ui.painter().rect(
        rect,
        7.0,
        if picker.hovered() { HOVER() } else { RAISED() },
        Stroke::new(1.0, BORDER()),
        StrokeKind::Inside,
    );
    ui.painter().text(pos2(rect.left() + 10.0, rect.center().y), Align2::LEFT_CENTER, crate::i18n::tr("In"), FontId::proportional(12.0), FAINT());
    clipped_text(
        ui,
        Rect::from_min_max(pos2(rect.left() + 28.0, rect.top()), pos2(rect.right() - 22.0, rect.bottom())),
        &label,
        13.0,
        TEXT(),
    );
    icon(ui.painter(), Rect::from_center_size(pos2(rect.right() - 12.0, rect.center().y), vec2(10.0, 10.0)), Icon::Chevron, FAINT());
    let picker = picker.on_hover_text(crate::i18n::tr("Where to search"));
    picker.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, crate::i18n::trf!("Search in: {label}", label)));
    Popup::menu(&picker).show(|ui| {
        ui.set_min_width(300.0);
        let mut choice = |ui: &mut Ui, scope: Scope, key: &str, title: String, detail: &str, enabled: bool| {
            let mut job = text::LayoutJob::default();
            job.append(&title, 0.0, TextFormat::simple(FontId::proportional(13.0), TEXT()));
            job.append(&format!("\n{detail}"), 0.0, TextFormat::simple(FontId::proportional(11.5), FAINT()));
            let r = ui.add_enabled(enabled, Button::selectable(s.scope == scope, job));
            widgets.insert(key.into(), (&r).into());
            if r.clicked() {
                s.scope = scope;
                ui.close();
            }
        };
        choice(ui, Scope::Checkout, "files-search-scope-checkout", crate::i18n::tr("Checkout").into(), crate::i18n::tr("Files on disk now, new ones included; ignored files are skipped"), true);
        choice(
            ui,
            Scope::Revision,
            "files-search-scope-revision",
            match revision {
                Some(id) => crate::i18n::trf!("Revision shown · {commit}", commit = short(id)),
                None => crate::i18n::tr("Revision shown").into(),
            },
            crate::i18n::tr("Files as committed, the snapshot the folders show"),
            revision.is_some(),
        );
    });
}

/// Folders | Results above the left pane.
pub(crate) fn pane_tabs(ui: &mut Ui, s: &State, intents: &mut Vec<Intent>, widgets: &mut HashMap<String, UiControl>) {
    Frame::new().inner_margin(Margin { left: 10, right: 10, top: 8, bottom: 6 }).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let count = (!s.groups.is_empty() || s.ran.is_some()).then_some(s.groups.len());
            let r = pages::tab_segments(ui, &[(crate::i18n::tr("Folders"), None, !s.results), (crate::i18n::tr("Results"), count, s.results)]);
            widgets.insert("files-pane-folders".into(), (&r[0]).into());
            widgets.insert("files-pane-results".into(), (&r[1]).into());
            if r[0].clicked() {
                emit(intents, crate::tree::TreeIntent::Pane(false));
            }
            if r[1].clicked() {
                emit(intents, crate::tree::TreeIntent::Pane(true));
            }
            if s.results && s.ran.is_some() {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(crate::i18n::tr("Binary files skipped")).size(11.5).color(FAINT()))
                        .on_hover_text(crate::i18n::tr("Files Git sees as binary aren't searched"));
                });
            }
        });
    });
}

/// The results list: a status strip, then files with their matching lines.
pub(crate) fn results_pane(ui: &mut Ui, s: &mut State, narrow: bool, intents: &mut Vec<Intent>, widgets: &mut HashMap<String, UiControl>) {
    if narrow && !s.groups.is_empty() && s.opened.is_some() {
        pages::strip(ui, |ui| {
            if let Some(text) = done_text(s) {
                ui.label(RichText::new(text).size(12.0).color(MUTED()));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let r = ui.button(crate::i18n::tr("Show file"));
                widgets.insert("files-show-file".into(), (&r).into());
                if r.clicked() {
                    s.show_file = true;
                }
            });
        });
    }
    if let Some(error) = s.error.clone() {
        message(ui, widgets, RED(), Some(Icon::Warning), &error);
        if s.bad_pattern {
            pages::empty(ui, crate::i18n::tr("The previous results were cleared."));
        }
        return;
    }
    if s.running() {
        Frame::new().inner_margin(Margin::symmetric(12, 6)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spinner();
                let text = if s.stopping {
                    crate::i18n::tr("Stopping").to_owned()
                } else {
                    crate::i18n::trf!("Searching · matches: {matches} · files: {files}", matches = pages::number_text(s.matches), files = pages::number_text(s.groups.len()))
                };
                let r = ui.label(RichText::new(text).size(12.0).color(MUTED()));
                widgets.insert("files-search-progress".into(), (&r).into());
            });
        });
    }
    match s.ended {
        Some(Ended::TooMany) => message(
            ui,
            widgets,
            AMBER(),
            Some(Icon::Warning),
            &crate::i18n::trf!("Showing the first {matches} matches in {files} files. Add a path or more text to narrow it.", matches = pages::number_text(s.matches), files = pages::number_text(s.groups.len())),
        ),
        Some(Ended::TimedOut) => message(
            ui,
            widgets,
            AMBER(),
            Some(Icon::Warning),
            &crate::i18n::trf!("Stopped after {seconds} seconds. These matches come from the files searched so far.", seconds = FILE_SEARCH_TIME.as_secs()),
        ),
        Some(Ended::Stopped) => message(
            ui,
            widgets,
            MUTED(),
            None,
            &crate::i18n::trf!("Stopped · matches: {matches} · files: {files}", matches = pages::number_text(s.matches), files = pages::number_text(s.groups.len())),
        ),
        _ => {}
    }
    if s.groups.is_empty() {
        if s.ended == Some(Ended::Complete) {
            let (query, label) = s.ran.as_ref().unwrap();
            let under = query
                .path
                .as_ref()
                .map(|p| crate::i18n::trf!(" under {path}", path = crate::tree::path_label(p.as_bytes())))
                .unwrap_or_default();
            let place = if query.source == FileSearchSource::Checkout {
                crate::i18n::trf!("in {label}", label)
            } else {
                crate::i18n::trf!("at {label}", label)
            };
            let note = if query.source == FileSearchSource::Checkout {
                crate::i18n::tr("Binary files aren't searched.")
            } else {
                crate::i18n::tr("Binary files and Git LFS content aren't searched.")
            };
            centered(ui, &crate::i18n::trf!("No matches{under} {place}", under, place), note, widgets);
        } else if s.ran.is_none() && !s.running() {
            centered(ui, crate::i18n::tr("Search inside files"), crate::i18n::tr("The checkout or the revision shown, grouped by file."), widgets);
        }
        return;
    }
    let rows = rows(s);
    let selected = s.selected;
    ui.spacing_mut().item_spacing.y = 0.0;
    let out = ScrollArea::vertical()
        .id_salt("files-results")
        .auto_shrink([false, false])
        .min_scrolled_height(0.0)
        .show_rows(ui, 26.0, rows.len(), |ui, range| {
            for i in range {
                match rows[i] {
                    Row::File(g) => file_row(ui, s, g, intents, widgets),
                    Row::Line(g, l) => line_row(ui, s, g, l, selected == Some((g, l)), intents, widgets),
                    Row::More => {
                        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
                        ui.painter().text(
                            pos2(rect.left() + 52.0, rect.center().y),
                            Align2::LEFT_CENTER,
                            crate::i18n::trf!("Showing the first {limit} matches in this file", limit = FILE_SEARCH_FILE_MATCHES),
                            FontId::proportional(12.0),
                            FAINT(),
                        );
                    }
                }
            }
        });
    let rect = out.inner_rect.intersect(ui.clip_rect());
    if rect.is_positive() {
        widgets.insert("files-results-scroll".into(), UiControl { rect, enabled: true });
    }
}

/// A line across the results pane about how the search ended.
fn message(ui: &mut Ui, widgets: &mut HashMap<String, UiControl>, color: Color32, mark: Option<Icon>, text: &str) {
    Frame::new().inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
        Frame::new()
            .fill(color.gamma_multiply(0.08))
            .stroke(Stroke::new(1.0, color.gamma_multiply(0.35)))
            .corner_radius(8)
            .inner_margin(Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let r = ui.horizontal(|ui| {
                    if let Some(mark) = mark {
                        let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                        icon(ui.painter(), rect, mark, color);
                    }
                    ui.add(Label::new(RichText::new(text).size(12.5).color(color)).wrap())
                });
                widgets.insert("files-search-message".into(), (&r.inner).into());
            });
    });
}

fn centered(ui: &mut Ui, title: &str, detail: &str, widgets: &mut HashMap<String, UiControl>) {
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.3).min(140.0));
        let r = ui.label(RichText::new(title).font(semibold(14.0)).color(TEXT()));
        widgets.insert("files-search-empty".into(), (&r).into());
        ui.label(RichText::new(detail).size(12.5).color(MUTED()));
    });
}

fn file_row(ui: &mut Ui, s: &mut State, g: usize, intents: &mut Vec<Intent>, widgets: &mut HashMap<String, UiControl>) {
    let group = &s.groups[g];
    let r = ui.add(Button::new("").frame(false).min_size(vec2(ui.available_width(), 26.0)));
    let rect = r.rect;
    if r.hovered() {
        ui.painter().rect_filled(rect, 4.0, HOVER());
    }
    icon(
        ui.painter(),
        Rect::from_center_size(pos2(rect.left() + 14.0, rect.center().y), vec2(10.0, 10.0)),
        if group.open { Icon::Chevron } else { Icon::ChevronRight },
        FAINT(),
    );
    let label = crate::tree::path_label(&group.file.path);
    let (dir, name) = label.rsplit_once('/').unwrap_or(("", label.as_str()));
    let count = if group.file.capped {
        format!("{}+", group.file.lines.len())
    } else {
        group.file.lines.len().to_string()
    };
    let count_galley = ui.painter().layout_no_wrap(count, FontId::proportional(12.0), FAINT());
    let right = rect.right() - 10.0 - count_galley.size().x;
    ui.painter().galley(pos2(right, rect.center().y - count_galley.size().y / 2.0), count_galley, FAINT());
    let mut x = rect.left() + 26.0;
    let mut job = text::LayoutJob::default();
    job.append(name, 0.0, TextFormat::simple(semibold(13.0), TEXT()));
    if !dir.is_empty() {
        job.append(&format!("  {dir}"), 0.0, TextFormat::simple(FontId::proportional(12.0), FAINT()));
    }
    let mut tags: Vec<(String, Color32)> = Vec::new();
    if group.detail.lfs_pointer {
        tags.push((crate::i18n::tr("Git LFS pointer").into(), BLUE()));
    }
    if group.detail.size.is_some_and(|n| n as usize > TREE_FILE_LIMIT) {
        tags.push((size(group.detail.size.unwrap()), AMBER()));
    }
    if group.not_utf8 {
        tags.push((crate::i18n::tr("Not UTF-8").into(), MUTED()));
    }
    let tag_width: f32 = tags.iter().map(|(t, _)| t.len() as f32 * 6.4 + 22.0).sum();
    job.wrap = text::TextWrapping::truncate_at_width((right - x - tag_width - 8.0).max(40.0));
    let galley = ui.painter().layout_job(job);
    let width = galley.size().x;
    ui.painter().galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, TEXT());
    x += width + 8.0;
    for (text, color) in tags {
        let galley = ui.painter().layout_no_wrap(text, semibold(11.0), color);
        let pill = Rect::from_min_size(pos2(x, rect.center().y - 10.0), vec2(galley.size().x + 16.0, 20.0));
        if pill.right() > right - 6.0 {
            break;
        }
        ui.painter().rect_filled(pill, 10.0, color.gamma_multiply(0.14));
        ui.painter().galley(pos2(pill.left() + 8.0, pill.center().y - galley.size().y / 2.0), galley, color);
        x = pill.right() + 6.0;
    }
    widgets.insert(format!("files-result-{g}"), (&r).into());
    let r = r.on_hover_text(&label);
    if r.clicked() {
        emit(intents, crate::tree::TreeIntent::ToggleGroup(g));
    }
}

fn line_row(
    ui: &mut Ui,
    s: &State,
    g: usize,
    l: usize,
    selected: bool,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
) {
    let line = &s.groups[g].file.lines[l];
    let r = ui.add(Button::new("").frame(false).min_size(vec2(ui.available_width(), 26.0)));
    let rect = r.rect;
    if selected {
        ui.painter().rect_filled(rect, 0.0, SELECTED());
        ui.painter().rect_filled(Rect::from_min_size(rect.min, vec2(3.0, rect.height())), 0.0, BLUE());
    } else if r.hovered() {
        ui.painter().rect_filled(rect, 0.0, HOVER());
    }
    ui.painter().text(
        pos2(rect.left() + 44.0, rect.center().y),
        Align2::RIGHT_CENTER,
        line.number.to_string(),
        FontId::monospace(11.5),
        FAINT(),
    );
    // Leading indentation says nothing in a list of matches.
    let trimmed = line.text.trim_start();
    let skip = line.text.chars().count() - trimmed.chars().count();
    let mut job = marked_job(trimmed, &line.marks, skip, line.cut_start && skip == 0, line.cut_end, 12.0);
    job.wrap = text::TextWrapping::truncate_at_width((rect.width() - 60.0).max(20.0));
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(pos2(rect.left() + 54.0, rect.center().y - galley.size().y / 2.0), galley, TEXT());
    widgets.insert(format!("files-match-{g}-{l}"), (&r).into());
    let r = r.on_hover_text(crate::i18n::trf!("Line {number}", number = line.number));
    if r.clicked() {
        emit(intents, crate::tree::TreeIntent::OpenMatch(g, l));
    }
    r.context_menu(|ui| {
        let path = crate::tree::path_label(&s.groups[g].file.path);
        let item = |ui: &mut Ui, widgets: &mut HashMap<String, UiControl>, key: &str, label: &str| {
            let r = ui.button(label);
            widgets.insert(key.into(), (&r).into());
            r.clicked()
        };
        if item(ui, widgets, "files-match-menu-open", crate::i18n::tr("Open at this line")) {
            emit(intents, crate::tree::TreeIntent::OpenMatch(g, l));
            ui.close();
        }
        if item(ui, widgets, "files-match-menu-commits", crate::i18n::tr("Find in commits")) {
            intents.push(Intent::FindInCommits(s.groups[g].file.path.clone()));
            ui.close();
        }
        ui.separator();
        if item(ui, widgets, "files-match-menu-copy-line", crate::i18n::tr("Copy line")) {
            ui.ctx().copy_text(line.text.clone());
            ui.close();
        }
        if item(ui, widgets, "files-match-menu-copy-path", crate::i18n::tr("Copy path")) {
            ui.ctx().copy_text(path);
            ui.close();
        }
    });
}

/// `text` with `marks` (character ranges, `skip` characters already dropped from
/// the front) shown as highlighted matches, and an ellipsis where it was cut.
pub(crate) fn marked_job(text: &str, marks: &[Range<usize>], skip: usize, cut_start: bool, cut_end: bool, size: f32) -> text::LayoutJob {
    let font = FontId::monospace(size);
    let plain = TextFormat::simple(font.clone(), TEXT());
    let hit = TextFormat {
        background: AMBER().gamma_multiply(0.32),
        color: TEXT(),
        ..TextFormat::simple(font.clone(), TEXT())
    };
    let mut job = text::LayoutJob::default();
    if cut_start {
        job.append("…", 0.0, TextFormat::simple(font.clone(), FAINT()));
    }
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let byte_at = |c: usize| chars.get(c).map_or(text.len(), |(b, _)| *b);
    let mut at = 0;
    for mark in marks {
        let (from, to) = (mark.start.saturating_sub(skip), mark.end.saturating_sub(skip));
        if to <= at || from >= chars.len() {
            continue;
        }
        let from = from.max(at);
        let to = to.min(chars.len());
        job.append(&text[byte_at(at)..byte_at(from)], 0.0, plain.clone());
        job.append(&text[byte_at(from)..byte_at(to)], 0.0, hit.clone());
        at = to;
    }
    job.append(&text[byte_at(at)..], 0.0, plain);
    if cut_end {
        job.append("…", 0.0, TextFormat::simple(font, FAINT()));
    }
    job
}

/// The opened file, scrolled to its match, with ↑ ↓ through all matches.
pub(crate) fn viewer(ui: &mut Ui, s: &mut State, narrow: bool, intents: &mut Vec<Intent>, widgets: &mut HashMap<String, UiControl>) {
    let (next, previous, selected) = (s.step(1), s.step(-1), s.selected);
    let (in_file, capped, lfs) = s
        .opened
        .as_ref()
        .and_then(|o| s.groups.get(o.group))
        .map_or((0, false, false), |g| (g.file.lines.len(), g.file.capped, g.detail.lfs_pointer));
    let no_results = s.groups.is_empty();
    let Some(opened) = &mut s.opened else {
        Frame::new().inner_margin(Margin::symmetric(24, 8)).show(ui, |ui| {
            pages::empty(ui, if no_results { crate::i18n::tr("Matches open here, at their line.") } else { crate::i18n::tr("Select a match to open the file at that line.") });
        });
        return;
    };
    let label = crate::tree::path_label(&opened.path);
    let (dir, name) = label.rsplit_once('/').map_or(("", label.as_str()), |(d, n)| (d, n));
    pages::strip(ui, |ui| {
        if narrow {
            let r = ui.button(crate::i18n::tr("Show results"));
            widgets.insert("files-show-results".into(), (&r).into());
            if r.clicked() {
                emit(intents, crate::tree::TreeIntent::ShowResults);
            }
        }
        let mut job = text::LayoutJob::default();
        if !dir.is_empty() {
            job.append(&format!("{dir}/"), 0.0, TextFormat::simple(FontId::proportional(13.0), FAINT()));
        }
        job.append(name, 0.0, TextFormat::simple(semibold(13.0), TEXT()));
        let r = ui.add(Label::new(job).truncate()).on_hover_text(&label);
        widgets.insert("files-open-label".into(), (&r).into());
        match &opened.source {
            FileSearchSource::Checkout => {
                pages::pill(ui, crate::i18n::tr("Checkout"), VIOLET());
            }
            FileSearchSource::Commit(id) => {
                pages::pill(ui, &crate::i18n::trf!("at {label}", label = &id.as_str()[..12.min(id.as_str().len())]), pages::INFO());
            }
        }
        if let Some(content) = &opened.content {
            let meta = if content.window {
                crate::tree::size(content.bytes)
            } else {
                crate::i18n::trf!("Lines: {count} · {size}", count = content.lines.len(), size = crate::tree::size(content.bytes))
            };
            ui.label(RichText::new(meta).size(12.0).color(FAINT()));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let r = ui.button(crate::i18n::tr("Find in commits")).on_hover_text(crate::i18n::tr("Commits that changed this text in this file"));
            widgets.insert("files-find-in-commits".into(), (&r).into());
            if r.clicked() {
                intents.push(Intent::FindInCommits(opened.path.clone()));
            }
            let down = ui.add_enabled(selected != next, Button::new("↓").min_size(vec2(26.0, 24.0)));
            widgets.insert("files-match-next".into(), (&down).into());
            if down.on_hover_text(crate::i18n::tr("Next match")).clicked() {
                emit(intents, crate::tree::TreeIntent::StepMatch(1));
            }
            let up = ui.add_enabled(selected != previous, Button::new("↑").min_size(vec2(26.0, 24.0)));
            widgets.insert("files-match-previous".into(), (&up).into());
            if up.on_hover_text(crate::i18n::tr("Previous match")).clicked() {
                emit(intents, crate::tree::TreeIntent::StepMatch(-1));
            }
            let of = if capped { format!("{in_file}+") } else { in_file.to_string() };
            ui.label(RichText::new(crate::i18n::trf!("Match {number} of {total}", number = opened.line + 1, total = of)).size(12.0).color(FAINT()));
        });
    });
    if let Some(error) = &opened.error {
        Frame::new().inner_margin(Margin::symmetric(14, 8)).show(ui, |ui| pages::callout(ui, RED(), error));
        return;
    }
    let Some(content) = &opened.content else {
        Frame::new().inner_margin(Margin::symmetric(24, 12)).show(ui, |ui| pages::loading(ui, crate::i18n::tr("Opening the file")));
        return;
    };
    let notes = Frame::new().inner_margin(Margin::symmetric(12, 6)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        if lfs {
            pages::callout(ui, BLUE(), crate::i18n::tr("Git LFS pointer. The file's content is stored outside Git, so only this pointer was searched."));
        }
        if content.window {
            let last = content.first + content.lines.len().saturating_sub(1);
            pages::callout(
                ui,
                BLUE(),
                &crate::i18n::trf!("Large file: showing lines {first} to {last}, around line {line}", first = pages::number_text(content.first), last = pages::number_text(last), line = pages::number_text(opened.number as usize)),
            );
        }
        if content.lossy {
            pages::callout(ui, MUTED(), crate::i18n::tr("Not UTF-8: invalid bytes are shown as �."));
        }
        if content.limited {
            pages::callout(ui, AMBER(), crate::i18n::tr("Preview limited to 50,000 lines and 8 KiB per line."));
        }
        if content.kind == HistoricalFileKind::Binary {
            pages::note(ui, crate::i18n::tr("This file is binary now, so its text can't be shown."));
            return false;
        }
        true
    }).inner;
    if !notes {
        return;
    }
    let focus = (opened.number as usize).checked_sub(content.first);
    let scroll = std::mem::take(&mut opened.scroll).then_some(()).and(focus);
    crate::tree::source_rows(
        ui,
        content,
        focus,
        scroll,
        ("files-search-source", label.as_str()),
        "files-source-scroll",
        widgets,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(number: u64) -> FileSearchLine {
        FileSearchLine { number, text: format!("hit {number}"), marks: vec![0..3], ..Default::default() }
    }

    #[test]
    fn marked_jobs_and_steps_follow_the_matches() {
        let job = marked_job("let x = x;", &[4..5, 8..9], 0, true, false, 12.0);
        let parts: Vec<&str> = job.sections.iter().map(|s| &job.text[s.byte_range.start.0..s.byte_range.end.0]).collect();
        assert_eq!(parts, ["…", "let ", "x", " = ", "x", ";"]);
        // Dropped leading characters move the marks left; ranges past the text are ignored.
        let job = marked_job("é = x", &[6..7, 40..41], 2, false, true, 12.0);
        let parts: Vec<&str> = job.sections.iter().map(|s| &job.text[s.byte_range.start.0..s.byte_range.end.0]).collect();
        assert_eq!(parts, ["é = ", "x", "", "…"]);
        let mut state = State::default();
        state.push(FileSearchFile { path: b"a".to_vec(), lines: vec![line(1), line(2)], capped: false });
        state.push(FileSearchFile { path: b"b".to_vec(), lines: (1..=25).map(line).collect(), capped: true });
        assert_eq!(state.matches, 27);
        assert!(state.groups[0].open && !state.groups[1].open, "files with many matches start collapsed");
        assert_eq!(state.step(1), Some((0, 0)));
        state.selected = Some((0, 1));
        assert_eq!((state.step(1), state.step(-1)), (Some((1, 0)), Some((0, 0))));
        state.selected = Some((1, 24));
        assert_eq!(state.step(1), Some((1, 24)), "the last match stays last");
        let rows = rows(&state);
        assert_eq!(rows.len(), 1 + 2 + 1, "a collapsed file shows only its row");
    }
}

/// Search in files through the app: checkout and revision, opening a match, a bad
/// expression and the hand-over to Commits, against a real repository.
#[cfg(all(test, windows))]
mod flow_tests {
    use crate::{
        app::{Intent, Rocket, View},
        code_search::{Ended, Scope},
        test_common as common,
        tree::TreeIntent,
    };
    use eframe::egui::Context;
    use rocket_git::HistorySearchKind;
    use std::{
        thread,
        time::{Duration, Instant},
    };

    fn settle(ctx: &Context, app: &mut Rocket) {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            app.poll(ctx);
            // Tabs open once Git is found, so busy comes first.
            if !app.busy()
                && app.tabs.first().is_some_and(|tab| {
                    !tab.tree.running()
                        && !tab.tree.search.running()
                        && !tab.tree.search.opening()
                        && !tab.search_state.scanning()
                })
            {
                return;
            }
            assert!(Instant::now() < deadline, "search did not finish");
            thread::sleep(Duration::from_millis(10));
        }
    }
    fn search(ctx: &Context, app: &mut Rocket, text: &str) -> Vec<String> {
        app.tabs[0].tree.search.query = text.into();
        app.dispatch(Intent::Tree(TreeIntent::Search));
        settle(ctx, app);
        app.tabs[0]
            .tree
            .search
            .groups
            .iter()
            .map(|g| String::from_utf8_lossy(&g.file.path).into_owned())
            .collect()
    }

    #[test]
    fn files_search_finds_opens_and_hands_over_to_commits() {
        common::isolated(
            "ROCKET_FILE_SEARCH_UI_CASE",
            "code_search::flow_tests::files_search_finds_opens_and_hands_over_to_commits",
            || {
                let f = common::Fixture::new();
                f.write("src/net/retry.rs", "pub const RETRY_LIMIT: u32 = 3;\n\nfn retry() {\n    for _ in 0..RETRY_LIMIT {}\n}\n");
                f.write("docs/guide.md", "Retry up to RETRY_LIMIT times.\n");
                f.commit("add retry");
                f.write("src/net/retry.rs", "pub const RETRY_LIMIT: u32 = 5;\n\nfn retry() {\n    for _ in 0..RETRY_LIMIT {}\n}\n");
                f.commit("raise retry limit");
                f.write("notes.txt", "untracked RETRY_LIMIT\n");
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(&ctx, vec![f.repository.clone()], f.root.join("logs"));
                settle(&ctx, &mut app);
                app.auto_refresh = false;
                app.dispatch(Intent::Tree(TreeIntent::FocusSearch));
                settle(&ctx, &mut app);
                assert_eq!(app.tabs[0].view, View::Tree);
                assert!(app.tabs[0].tree.listing.is_some() && app.tabs[0].tree.search.results);
                // The checkout includes the untracked file; case is ignored by default.
                assert_eq!(search(&ctx, &mut app, "retry_limit"), ["docs/guide.md", "notes.txt", "src/net/retry.rs"]);
                let s = &app.tabs[0].tree.search;
                assert_eq!((s.ended, s.matches), (Some(Ended::Complete), 4));
                // The revision shown has no untracked file.
                app.tabs[0].tree.search.scope = Scope::Revision;
                assert_eq!(search(&ctx, &mut app, "RETRY_LIMIT"), ["docs/guide.md", "src/net/retry.rs"]);
                app.dispatch(Intent::Tree(TreeIntent::OpenMatch(1, 1)));
                settle(&ctx, &mut app);
                let opened = app.tabs[0].tree.search.opened.as_ref().unwrap();
                let content = opened.content.as_ref().expect("the file opened");
                assert_eq!((opened.number, content.first), (4, 1));
                assert_eq!(content.lines[3], "    for _ in 0..RETRY_LIMIT {}");
                assert!(content.marks.contains_key(&0) && content.marks.contains_key(&3));
                // ↓ moves within the file without reading it again.
                app.dispatch(Intent::Tree(TreeIntent::StepMatch(-1)));
                assert_eq!(app.tabs[0].tree.search.opened.as_ref().unwrap().number, 1);
                assert!(!app.tabs[0].tree.search.opening());
                // A bad expression says why and clears the results.
                app.tabs[0].tree.search.regex = true;
                assert!(search(&ctx, &mut app, "RETRY_(").is_empty());
                let s = &app.tabs[0].tree.search;
                assert!(s.bad_pattern && s.error.as_deref().is_some_and(|e| e.starts_with("Not a valid regular expression")));
                // Find in commits carries the text, the expression and the file's path.
                assert_eq!(search(&ctx, &mut app, "RETRY_LIMIT"), ["docs/guide.md", "src/net/retry.rs"]);
                app.dispatch(Intent::FindInCommits(b"src/net/retry.rs".to_vec()));
                settle(&ctx, &mut app);
                let tab = &app.tabs[0];
                assert_eq!(tab.view, View::Commits);
                assert_eq!((tab.search_kind, tab.search_regex, tab.search_path.as_str()), (HistorySearchKind::Content, true, "src/net/retry.rs"));
                let subjects: Vec<_> = tab.search_state.commits.iter().map(|c| c.subject_lossy().into_owned()).collect();
                assert_eq!(subjects, ["raise retry limit", "add retry"]);
            },
        );
    }
}
