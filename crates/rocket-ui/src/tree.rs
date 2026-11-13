//! A per-tab immutable repository browser. Superseded tasks are dropped (and
//! cancelled); their receivers cannot publish late data into a newer selection.
use crate::{
    app::*,
    backend::{Backend, user_error},
    pages,
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use std::collections::HashMap;

pub enum TreeIntent {
    OpenSelected,
    Load,
    Enter(usize),
    Ancestor(usize),
    Cancel,
    Retry,
    /// Search inside files with what the search row holds.
    Search,
    StopSearch,
    /// Show results (true) or folders in the left pane.
    Pane(bool),
    ToggleGroup(usize),
    OpenMatch(usize, usize),
    StepMatch(i32),
    /// Narrow window: back from the file to the results.
    ShowResults,
    /// Open Files with the keyboard in the search field (Ctrl+Shift+F).
    FocusSearch,
    /// Browse this commit and search its files next.
    SearchAt(ObjectId),
}
pub struct Content {
    pub kind: HistoricalFileKind,
    pub bytes: usize,
    pub lines: Vec<String>,
    pub limited: bool,
    /// The line number of `lines[0]`.
    pub first: usize,
    /// Only lines around a match: the file is too large to show whole.
    pub window: bool,
    /// Not UTF-8: bytes that aren't valid UTF-8 show as U+FFFD.
    pub lossy: bool,
    /// Matches to mark, as character ranges, by index into `lines`.
    pub marks: HashMap<usize, Vec<std::ops::Range<usize>>>,
}
impl Content {
    fn empty(kind: HistoricalFileKind, bytes: usize) -> Self {
        Self {
            kind,
            bytes,
            lines: Vec::new(),
            limited: false,
            first: 1,
            window: false,
            lossy: false,
            marks: HashMap::new(),
        }
    }
}
#[derive(Default)]
pub struct State {
    pub revision: String,
    pub listing: Option<TreeListing>,
    pub trail: Vec<TreeDirectory>,
    pub selected: Option<usize>,
    pub content: Option<Content>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub show_list: bool,
    pub cancelling: bool,
    /// Search inside files, shown as the Results pane.
    pub search: crate::code_search::State,
    directory_task: Option<ReadTask<(TreeListing, Vec<TreeDirectory>)>>,
    content_task: Option<ReadTask<Content>>,
}
impl State {
    pub fn running(&self) -> bool {
        self.directory_task.is_some() || self.content_task.is_some()
    }
    pub fn directory_running(&self) -> bool {
        self.directory_task.is_some()
    }
    pub fn selected_entry(&self) -> Option<&TreeEntry> {
        self.listing.as_ref()?.entries.get(self.selected?)
    }
    pub fn stop(&mut self) {
        if let Some(t) = &self.directory_task {
            t.cancel();
        }
        if let Some(t) = &self.content_task {
            t.cancel();
        }
        self.cancelling = self.running();
    }
    fn supersede(&mut self) {
        self.directory_task = None;
        self.content_task = None;
        self.error = None;
        self.notice = None;
        self.cancelling = false;
    }
    fn load(&mut self, backend: Backend, repo: Repository, trail: Vec<TreeDirectory>) {
        self.supersede();
        let revision = self.revision.clone();
        let result = ReadTask::spawn(move |cancel| {
            backend.log.request("tree-list", |log| {
                let repo = repo.with_log(log.clone());
                let listing = if let Some(directory) = trail.last() {
                    repo.tree_directory(directory, &cancel)?
                } else {
                    repo.repository_tree(&revision, &cancel)?
                };
                let trail = if trail.is_empty() {
                    vec![listing.directory.clone()]
                } else {
                    trail
                };
                cancel.check()?;
                Ok((listing, trail))
            })
        });
        match result {
            Ok(task) => self.directory_task = Some(task),
            Err(e) => self.error = Some(tree_error(&e, false)),
        }
    }
    fn file(&mut self, backend: Backend, repo: Repository, index: usize) {
        let Some(entry) = self
            .listing
            .as_ref()
            .and_then(|l| l.entries.get(index))
            .cloned()
        else {
            return;
        };
        self.supersede();
        self.selected = Some(index);
        self.content = None;
        self.show_list = false;
        if entry.kind() == TreeEntryKind::Submodule {
            return;
        }
        match ReadTask::spawn(move |cancel| {
            backend.log.request("tree-content", |log| {
                let file = repo.with_log(log.clone()).tree_file(&entry, &cancel)?;
                let result = prepare_content(file, &cancel)?;
                cancel.check()?;
                Ok(result)
            })
        }) {
            Ok(task) => self.content_task = Some(task),
            Err(e) => self.error = Some(tree_error(&e, true)),
        }
    }
}

impl Rocket {
    pub(crate) fn tree_intent(&mut self, intent: TreeIntent) {
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let Some(repo) = tab.snapshot.as_ref().map(|s| s.repository.clone()) else {
            return;
        };
        match intent {
            TreeIntent::OpenSelected => {
                tab.view = View::Tree;
                tab.diff = None;
                tab.tree.revision = tab
                    .selected_commit
                    .as_ref()
                    .map_or_else(|| "HEAD".into(), ToString::to_string);
                tab.tree.load(backend, repo, Vec::new());
            }
            TreeIntent::Load => tab.tree.load(backend, repo, Vec::new()),
            TreeIntent::Cancel => tab.tree.stop(),
            TreeIntent::Ancestor(index) => {
                if index < tab.tree.trail.len() {
                    tab.tree
                        .load(backend, repo, tab.tree.trail[..=index].to_vec());
                }
            }
            TreeIntent::Enter(index) => {
                let Some(entry) = tab.tree.listing.as_ref().and_then(|l| l.entries.get(index))
                else {
                    return;
                };
                if let Some(directory) = entry.directory() {
                    let mut trail = tab.tree.trail.clone();
                    trail.push(directory);
                    tab.tree.load(backend, repo, trail);
                } else {
                    tab.tree.file(backend, repo, index);
                }
            }
            TreeIntent::Retry => {
                if let Some(index) = tab.tree.selected {
                    tab.tree.file(backend, repo, index);
                }
            }
            TreeIntent::Search => {
                // The revision shown is still loading: searching now would read the old one.
                if tab.tree.search.scope == crate::code_search::Scope::Revision && tab.tree.directory_running() {
                    return;
                }
                let revision = tab.tree.listing.as_ref().map(|l| l.directory.commit().clone());
                tab.tree.search.start(backend, repo, revision.as_ref());
            }
            TreeIntent::StopSearch => tab.tree.search.stop(),
            TreeIntent::Pane(results) => tab.tree.search.results = results,
            TreeIntent::ToggleGroup(index) => {
                if let Some(group) = tab.tree.search.groups.get_mut(index) {
                    group.open = !group.open;
                }
            }
            TreeIntent::OpenMatch(group, line) => tab.tree.search.open(backend, repo, group, line),
            TreeIntent::StepMatch(delta) => {
                if let Some((group, line)) = tab.tree.search.step(delta) {
                    if let Some(found) = tab.tree.search.groups.get_mut(group) {
                        found.open = true;
                    }
                    tab.tree.search.open(backend, repo, group, line);
                }
            }
            TreeIntent::ShowResults => tab.tree.search.show_file = false,
            TreeIntent::FocusSearch => {
                tab.view = View::Tree;
                tab.diff = None;
                if tab.tree.listing.is_none() && !tab.tree.running() {
                    tab.tree.revision = "HEAD".into();
                    tab.tree.load(backend, repo, Vec::new());
                }
                tab.tree.search.results = true;
                tab.tree.search.focus = true;
            }
            TreeIntent::SearchAt(commit) => {
                tab.view = View::Tree;
                tab.diff = None;
                tab.tree.revision = commit.to_string();
                tab.tree.load(backend, repo, Vec::new());
                tab.tree.search.scope = crate::code_search::Scope::Revision;
                tab.tree.search.results = true;
                tab.tree.search.focus = true;
            }
        }
    }
    pub(crate) fn poll_tree(&mut self, ctx: &Context) {
        for tab in &mut self.tabs {
            let s = &mut tab.tree;
            if let Some(result) = s.directory_task.as_mut().and_then(ReadTask::try_finish) {
                s.directory_task = None;
                match result {
                    Ok((listing, trail)) => {
                        s.listing = Some(listing);
                        s.trail = trail;
                        s.selected = None;
                        s.content = None;
                        s.show_list = true;
                    }
                    Err(Error::Cancelled) => {
                        s.notice = Some(crate::i18n::tr("Folder read cancelled. Previous results retained.").into())
                    }
                    Err(e) => {
                        s.error = Some(crate::i18n::trf!("{error} Previous results retained.", error = tree_error(&e, false)))
                    }
                }
            }
            if let Some(result) = s.content_task.as_mut().and_then(ReadTask::try_finish) {
                s.content_task = None;
                match result {
                    Ok(content) => s.content = Some(content),
                    Err(Error::Cancelled) => {
                        s.notice = Some(crate::i18n::tr("File read cancelled. Retry or select another file.").into())
                    }
                    Err(e) => s.error = Some(tree_error(&e, true)),
                }
            }
            let searching = s.search.poll();
            if s.running() || searching {
                ctx.request_repaint_after(std::time::Duration::from_millis(30));
            }
            if !s.running() {
                s.cancelling = false;
            }
        }
    }
}

pub(crate) fn tree_error(error: &Error, content: bool) -> String {
    match error {
        Error::Git { code, .. } => format!(
            "{} [git_exit_{}]",
            if content {
                crate::i18n::tr("Could not read this file object. It may be missing or unavailable locally. Fetch the needed objects separately, then retry.")
            } else {
                crate::i18n::tr("Could not load this revision or folder. Check the commit, branch or tag and that its objects exist locally. An empty repository has no commit to browse.")
            },
            code.map_or_else(|| "unknown".into(), |n| n.to_string())
        ),
        _ => user_error(error),
    }
}

/// Lines a preview shows; bytes kept of one line.
const PREVIEW_LINES: usize = 50_000;
const LINE_BYTES: usize = 8192;

/// A line for display, cut at [`LINE_BYTES`]: tabs as four spaces, controls escaped
/// so they can't change row geometry, bytes that aren't UTF-8 as U+FFFD. Also the
/// byte in `raw` where each shown character starts, and whether the line was cut.
fn display_line(raw: &[u8]) -> (String, Vec<usize>, bool) {
    let shown = crate::text::prefix(raw, LINE_BYTES);
    let mut text = String::new();
    let mut starts = Vec::new();
    let mut offset = 0;
    for chunk in shown.utf8_chunks() {
        for (i, c) in chunk.valid().char_indices() {
            let at = offset + i;
            if c == '\t' {
                text.push_str("    ");
                starts.extend([at; 4]);
            } else if c.is_control() {
                for e in c.escape_default() {
                    text.push(e);
                    starts.push(at);
                }
            } else {
                text.push(c);
                starts.push(at);
            }
        }
        offset += chunk.valid().len();
        if !chunk.invalid().is_empty() {
            text.push('\u{fffd}');
            starts.push(offset);
            offset += chunk.invalid().len();
        }
    }
    (text, starts, shown.len() < raw.len())
}

/// Lines as `str::lines` splits them: a `\r` before `\n` isn't part of the line,
/// and a final newline doesn't start an empty line.
fn split_lines(bytes: &[u8]) -> Vec<&[u8]> {
    let mut lines: Vec<&[u8]> = bytes.split(|b| *b == b'\n').collect();
    if lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines
        .into_iter()
        .map(|l| l.strip_suffix(b"\r").unwrap_or(l))
        .collect()
}

/// Byte ranges of a line as ranges of its shown characters.
fn char_marks(starts: &[usize], columns: &[std::ops::Range<usize>]) -> Vec<std::ops::Range<usize>> {
    columns
        .iter()
        .filter_map(|c| {
            let from = starts.partition_point(|&s| s < c.start);
            let to = starts.partition_point(|&s| s < c.end);
            (from < to).then_some(from..to)
        })
        .collect()
}

/// Text when valid UTF-8; otherwise still text, with replacement marks, unless Git
/// would call it binary (a NUL in the first 8,000 bytes).
fn text_kind(file: &HistoricalFile) -> Option<bool> {
    match file.kind {
        HistoricalFileKind::Text => Some(false),
        HistoricalFileKind::Binary if !file.bytes[..file.bytes.len().min(8000)].contains(&0) => Some(true),
        _ => None,
    }
}

/// Add `raw` lines numbered from `content.first`, marking `columns` where given.
fn push_lines(
    content: &mut Content,
    raw: &[&[u8]],
    columns: Option<&HashMap<u64, Vec<std::ops::Range<usize>>>>,
    cancel: &CancellationToken,
) -> rocket_git::Result<()> {
    for line in raw {
        cancel.check()?;
        if content.lines.len() == PREVIEW_LINES {
            content.limited = true;
            break;
        }
        let (mut shown, starts, cut) = display_line(line);
        let number = (content.first + content.lines.len()) as u64;
        if let Some(found) = columns.and_then(|c| c.get(&number)) {
            content.marks.insert(content.lines.len(), char_marks(&starts, found));
        }
        if cut {
            content.limited = true;
            shown.push_str("… [display limited]");
        }
        content.lines.push(shown);
    }
    Ok(())
}

fn prepare_content(
    file: HistoricalFile,
    cancel: &CancellationToken,
) -> rocket_git::Result<Content> {
    let mut content = Content::empty(file.kind, file.bytes.len());
    if let Some(lossy) = text_kind(&file) {
        content.kind = HistoricalFileKind::Text;
        content.lossy = lossy;
        push_lines(&mut content, &split_lines(&file.bytes), None, cancel)?;
    }
    Ok(content)
}

/// A file opened at a search match: its lines with that file's matches marked. A
/// file with more lines than a preview shows opens around the match instead.
pub(crate) fn prepare_search(
    file: SearchFileContent,
    focus: u64,
    columns: &HashMap<u64, Vec<std::ops::Range<usize>>>,
    cancel: &CancellationToken,
) -> rocket_git::Result<Content> {
    match file {
        SearchFileContent::Whole(file) => {
            let mut content = Content::empty(file.kind, file.bytes.len());
            let Some(lossy) = text_kind(&file) else {
                return Ok(content);
            };
            content.kind = HistoricalFileKind::Text;
            content.lossy = lossy;
            let lines = split_lines(&file.bytes);
            let mut range = 0..lines.len();
            if lines.len() > PREVIEW_LINES {
                let first = focus.saturating_sub(FILE_WINDOW_LINES).max(1) as usize;
                let last = (focus.saturating_add(FILE_WINDOW_LINES) as usize).min(lines.len());
                range = (first - 1).min(lines.len())..last.max(first - 1).min(lines.len());
                content.first = first;
                content.window = true;
            }
            push_lines(&mut content, &lines[range], Some(columns), cancel)?;
            Ok(content)
        }
        SearchFileContent::Window(window) => {
            let mut content = Content::empty(HistoricalFileKind::Text, window.size as usize);
            content.first = window.first as usize;
            content.window = true;
            content.lossy = window.lines.iter().any(|l| std::str::from_utf8(l).is_err());
            let lines: Vec<&[u8]> = window.lines.iter().map(Vec::as_slice).collect();
            push_lines(&mut content, &lines, Some(columns), cancel)?;
            for &i in &window.cut {
                if let Some(line) = content.lines.get_mut(i) {
                    line.push_str("… [display limited]");
                    content.limited = true;
                }
            }
            Ok(content)
        }
    }
}

/// Display-only, bounded and escaped. Exact original bytes remain in TreeEntry.
pub fn path_label(bytes: &[u8]) -> String {
    let prefix = crate::text::prefix(bytes, 4096);
    let mut label = String::new();
    for c in String::from_utf8_lossy(prefix).chars() {
        if c.is_control() {
            label.extend(c.escape_default());
        } else {
            label.push(c);
        }
    }
    if prefix.len() != bytes.len() {
        label.push_str("… [display limited]");
    }
    label
}
pub fn kind_label(kind: TreeEntryKind) -> &'static str {
    match kind {
        TreeEntryKind::Directory => crate::i18n::tr("Folder"),
        TreeEntryKind::File => crate::i18n::tr("File"),
        TreeEntryKind::Executable => crate::i18n::tr("Executable file"),
        TreeEntryKind::Symlink => crate::i18n::tr("Symbolic link"),
        TreeEntryKind::Submodule => crate::i18n::tr("Submodule"),
    }
}
fn register(ui: &Ui, widgets: &mut HashMap<String, UiControl>, key: &str, r: &Response) {
    let rect = r.interact_rect.intersect(ui.clip_rect());
    if rect.is_positive() {
        widgets.insert(
            key.into(),
            UiControl {
                rect,
                enabled: r.enabled(),
            },
        );
    }
}
fn button(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    key: &str,
    label: &str,
    enabled: bool,
) -> bool {
    let r = ui.add_enabled(enabled, Button::new(label));
    register(ui, widgets, key, &r);
    r.clicked()
}
fn emit(intents: &mut Vec<Intent>, intent: TreeIntent) {
    intents.push(Intent::Tree(intent));
}

/// Files at a commit: revision bar, a folder pane and the read-only file viewer.
/// Narrow panes show one of the two, switched by `tree-toggle-list`.
pub(crate) fn view(
    ui: &mut Ui,
    tab: &mut Tab,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
    commands:&[rocket_git::CustomCommand],
) {
    let context=crate::custom_commands::context(tab);
    let downloads = crate::preview_tab::downloads(tab);
    ui.set_clip_rect(ui.clip_rect().intersect(ui.available_rect_before_wrap()));
    let narrow = ui.available_width() < 800.0;
    let short = ui.available_height() < 340.0;
    let s = &mut tab.tree;
    if !short {
        pages::header(
            ui,
            Icon::Folder,
            crate::i18n::tr("Repository files"),
            crate::i18n::tr("Read only · browse a commit, or search the checkout or a commit"),
            |_| {},
        );
        pages::divider(ui);
    }
    Frame::new()
        .inner_margin(Margin::symmetric(24, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let r = ui.add_sized(
                    [(ui.available_width() - 330.0).clamp(70.0, 360.0), 28.0],
                    TextEdit::singleline(&mut s.revision)
                        .char_limit(4096)
                        .hint_text(crate::i18n::tr("Commit, branch or tag"))
                        .align(Align2::LEFT_CENTER),
                );
                register(ui, widgets, "tree-revision", &r);
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                let load = pages::primary(ui, !s.revision.is_empty(), crate::i18n::tr("Browse revision"));
                register(ui, widgets, "tree-load", &load);
                if load.clicked() || enter {
                    emit(intents, TreeIntent::Load);
                }
                if button(
                    ui,
                    widgets,
                    "tree-cancel",
                    crate::i18n::tr("Stop"),
                    s.running() && !s.cancelling,
                ) {
                    emit(intents, TreeIntent::Cancel);
                }
                ui.add_space(4.0);
                if s.running() {
                    ui.spinner();
                }
                let commit = s
                    .listing
                    .as_ref()
                    .map_or("", |l| l.directory.commit().as_str());
                if let Some(list) = &s.listing {
                    pages::pill(
                        ui,
                        &format!("at {}", &list.directory.commit().as_str()[..12]),
                        pages::INFO(),
                    )
                    .on_hover_text(commit);
                }
                let text = if s.cancelling {
                    crate::i18n::tr("Stopping read…")
                } else if s.listing.is_some() {
                    if s.running() {
                        crate::i18n::tr("read only · loading…")
                    } else {
                        crate::i18n::tr("read only")
                    }
                } else if s.running() {
                    crate::i18n::tr("Loading revision…")
                } else {
                    crate::i18n::tr("Choose a revision to browse its committed files.")
                };
                ui.add(Label::new(RichText::new(text).size(12.0).color(FAINT())).truncate())
                    .on_hover_text(commit);
            });
            if let Some(message) = s.error.as_ref().or(s.notice.as_ref()) {
                let color = if s.error.is_some() { RED() } else { AMBER() };
                let r = if short {
                    ui.add(Label::new(RichText::new(message).size(12.0).color(color)).truncate())
                } else {
                    ui.add_space(2.0);
                    ui.scope(|ui| pages::callout(ui, color, message)).response
                }
                .on_hover_text(message);
                register(ui, widgets, "tree-message", &r);
            }
            ui.add_space(4.0);
            let revision = s.listing.as_ref().map(|l| l.directory.commit().clone());
            let loading = s.directory_running();
            crate::code_search::search_row(ui, &mut s.search, revision.as_ref(), loading, narrow, intents, widgets);
        });
    pages::divider(ui);
    let results = s.search.results;
    if narrow {
        if results {
            if s.search.show_file && s.search.opened.is_some() {
                crate::code_search::viewer(ui, &mut s.search, narrow, intents, widgets);
            } else {
                crate::code_search::pane_tabs(ui, &s.search, intents, widgets);
                crate::code_search::results_pane(ui, &mut s.search, narrow, intents, widgets);
            }
        } else if s.show_list || s.listing.is_none() {
            crate::code_search::pane_tabs(ui, &s.search, intents, widgets);
            folder_view(ui, s, intents, widgets, narrow, commands, &context);
        } else {
            file_view(ui, s, intents, widgets, short, narrow, downloads.as_ref());
        }
    } else if results {
        pages::list_pane(
            ui,
            Id::unique(("tree-results", tab.id)),
            430.0,
            280.0..=640.0,
            |ui| {
                crate::code_search::pane_tabs(ui, &s.search, intents, widgets);
                crate::code_search::results_pane(ui, &mut s.search, narrow, intents, widgets);
            },
        );
        CentralPanel::default()
            .frame(Frame::new().fill(BG()))
            .show(ui, |ui| crate::code_search::viewer(ui, &mut s.search, narrow, intents, widgets));
    } else {
        pages::list_pane(
            ui,
            Id::unique(("tree-folders", tab.id)),
            285.0,
            190.0..=380.0,
            |ui| {
                crate::code_search::pane_tabs(ui, &s.search, intents, widgets);
                folder_view(ui, s, intents, widgets, narrow, commands, &context);
            },
        );
        CentralPanel::default()
            .frame(Frame::new().fill(BG()))
            .show(ui, |ui| file_view(ui, s, intents, widgets, short, narrow, downloads.as_ref()));
    }
}

/// Source lines with numbers from `content.first`, matches marked, the `focus` line
/// banded, scrolled to `scroll` (an index into the lines) once when given. Each row
/// is exactly 22 points so the virtualized row range stays accurate.
pub(crate) fn source_rows(
    ui: &mut Ui,
    content: &Content,
    focus: Option<usize>,
    scroll: Option<usize>,
    salt: impl AsIdSalt,
    scroll_id: &str,
    widgets: &mut HashMap<String, UiControl>,
) {
    pages::divider(ui);
    let last = content.first + content.lines.len().max(1) - 1;
    let digits = last.to_string().len() as f32;
    let gutter = (digits * 8.0 + 22.0).max(46.0);
    let size = ui
        .style()
        .text_styles
        .get(&TextStyle::Monospace)
        .map_or(13.0, |f| f.size);
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.spacing_mut().interact_size.y = 22.0;
    let mut area = ScrollArea::both()
        .id_salt(salt)
        .auto_shrink([false, false])
        .min_scrolled_height(0.0);
    if let Some(index) = scroll {
        // A third of the way down, so the lines before the match show too.
        let offset = (index as f32 * 22.0 - ui.available_height() / 3.0).max(0.0);
        area = area.vertical_scroll_offset(offset);
    }
    let out = area.show_rows(ui, 22.0, content.lines.len(), |ui, rows| {
        for i in rows {
            if Some(i) == focus {
                let at = ui.available_rect_before_wrap();
                let band = Rect::from_min_size(at.min, vec2(at.width().max(ui.clip_rect().width()), 22.0));
                ui.painter().rect_filled(band, 0.0, BLUE().gamma_multiply(0.12));
                ui.painter().rect_filled(Rect::from_min_size(band.min, vec2(3.0, 22.0)), 0.0, BLUE());
            }
            ui.horizontal(|ui| {
                ui.set_min_height(22.0);
                ui.spacing_mut().item_spacing.x = 8.0;
                pages::gutter(ui, &(content.first + i).to_string(), gutter, 22.0);
                match content.marks.get(&i) {
                    Some(marks) => ui.add(
                        Label::new(crate::code_search::marked_job(&content.lines[i], marks, 0, false, false, size)).extend(),
                    ),
                    None => ui.add(Label::new(RichText::new(&content.lines[i]).monospace()).extend()),
                };
            });
        }
    });
    let rect = out.inner_rect.intersect(ui.clip_rect());
    if rect.is_positive() {
        widgets.insert(scroll_id.into(), UiControl { rect, enabled: true });
    }
}

/// Root, Up and the current folder; in narrow panes also the folders/file switch.
fn trail_bar(
    ui: &mut Ui,
    s: &mut State,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
    narrow: bool,
) {
    if s.trail.is_empty() {
        return;
    }
    pages::strip(ui, |ui| {
        if button(ui, widgets, "tree-root", crate::i18n::tr("Root"), true) {
            emit(intents, TreeIntent::Ancestor(0));
        }
        if button(ui, widgets, "tree-up", crate::i18n::tr("Up"), s.trail.len() > 1) {
            emit(intents, TreeIntent::Ancestor(s.trail.len() - 2));
        }
        if button(
            ui,
            widgets,
            "tree-history",
            crate::i18n::tr("Folder history"),
            !s.running() && s.trail.len() > 1,
        ) {
            intents.push(Intent::File(crate::file_history::FileIntent::FromTree));
        }
        if narrow
            && button(
                ui,
                widgets,
                "tree-toggle-list",
                if s.show_list {
                    crate::i18n::tr("Show file")
                } else {
                    crate::i18n::tr("Show folders")
                },
                true,
            )
        {
            s.show_list = !s.show_list;
        }
        let path = s
            .trail
            .last()
            .map(|d| path_label(d.path()))
            .unwrap_or_default();
        ui.add(
            Label::new(
                RichText::new(if path.is_empty() { "/".into() } else { path }).font(semibold(12.5)),
            )
            .truncate(),
        )
        .on_hover_text(path_label(s.trail.last().unwrap().path()));
    });
}

fn folder_view(
    ui: &mut Ui,
    s: &mut State,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
    narrow: bool,
    commands:&[rocket_git::CustomCommand],
    context:&rocket_git::CommandContext,
) {
    trail_bar(ui, s, intents, widgets, narrow);
    let Some(list) = &s.listing else {
        Frame::new()
            .inner_margin(Margin::symmetric(16, 4))
            .show(ui, |ui| {
                pages::empty(ui, crate::i18n::tr("Browse HEAD, a branch, tag or commit ID to begin."));
            });
        return;
    };
    Frame::new()
        .inner_margin(Margin {
            left: 14,
            right: 14,
            top: 8,
            bottom: 4,
        })
        .show(ui, |ui| {
            ui.label(
                RichText::new(crate::i18n::trf!("Entries: {count} · folders first", count = list.entries.len()))
                .size(11.5)
                .color(FAINT()),
            );
            if list.entries.is_empty() {
                pages::empty(ui, crate::i18n::tr("This folder is empty at the pinned commit."));
            }
        });
    // Rows are 30 points; with zero spacing the row pitch is exact.
    ui.spacing_mut().item_spacing.y = 0.0;
    let out = Frame::new()
        .inner_margin(Margin::symmetric(6, 0))
        .show(ui, |ui| {
            ScrollArea::vertical()
                .id_salt((
                    "tree-folders",
                    list.directory.commit(),
                    list.directory.path(),
                ))
                .auto_shrink([false, false])
                .min_scrolled_height(0.0)
                .show_rows(ui, 30.0, list.entries.len(), |ui, rows| {
                    for i in rows {
                        let entry = &list.entries[i];
                        let name = path_label(entry.name());
                        let (image, detail) = match entry.kind() {
                            TreeEntryKind::Directory => (Icon::Folder, ""),
                            TreeEntryKind::File => (Icon::File, ""),
                            TreeEntryKind::Executable => (Icon::File, "executable"),
                            TreeEntryKind::Symlink => (Icon::File, "link"),
                            TreeEntryKind::Submodule => (Icon::Modules, "submodule"),
                        };
                        let r = pages::list_row(ui, image, &name, detail, s.selected == Some(i));
                        register(ui, widgets, &format!("tree-entry-{i}"), &r);
                        if matches!(entry.kind(),TreeEntryKind::File|TreeEntryKind::Executable|TreeEntryKind::Symlink) {
                            let context=crate::custom_commands::historical_context(context,list.directory.commit(),entry.path());
                            r.context_menu(|ui|crate::custom_commands::context_menu(ui,commands,rocket_git::CommandScope::File,&context,intents,widgets));
                        }
                        if r.clicked() {
                            emit(intents, TreeIntent::Enter(i));
                        }
                        r.on_hover_text(format!(
                            "{}\n{}\n{}",
                            path_label(entry.path()),
                            kind_label(entry.kind()),
                            entry.object()
                        ));
                    }
                })
        })
        .inner;
    let rect = out.inner_rect.intersect(ui.clip_rect());
    if rect.is_positive() {
        widgets.insert(
            "tree-list-scroll".into(),
            UiControl {
                rect,
                enabled: true,
            },
        );
    }
}

fn file_view(
    ui: &mut Ui,
    s: &mut State,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
    short: bool,
    narrow: bool,
    // A preview's downloads: no Restore, and its download and failure cards.
    preview: Option<&crate::preview_tab::Downloads>,
) {
    if narrow {
        trail_bar(ui, s, intents, widgets, narrow);
    }
    let Some(entry) = s.selected_entry().cloned() else {
        Frame::new()
            .inner_margin(Margin::symmetric(24, 8))
            .show(ui, |ui| {
                pages::empty(
                    ui,
                    crate::i18n::tr("Select a file to view its contents at the pinned commit."),
                );
            });
        return;
    };
    let label = path_label(entry.path());
    let (dir, name) = label
        .rsplit_once('/')
        .map_or(("", label.as_str()), |(d, n)| (d, n));
    pages::strip(ui, |ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if preview.is_none()
                && matches!(
                entry.kind(),
                TreeEntryKind::File | TreeEntryKind::Executable
            ) && let Ok(path) = RepoPath::new(entry.path().to_vec())
                && button(
                    ui,
                    widgets,
                    "tree-restore",
                    crate::i18n::tr("Restore this file"),
                    !s.running(),
                )
            {
                intents.push(Intent::Dialog(DialogKind::FileEdit(Some((
                    path,
                    FileEditSource::Commit(entry.commit().to_string()),
                )))));
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                if s.content.is_none()
                    && entry.kind() != TreeEntryKind::Submodule
                    && button(ui, widgets, "tree-retry", crate::i18n::tr("Retry"), !s.running())
                {
                    emit(intents, TreeIntent::Retry);
                }
                let meta = match &s.content {
                    Some(c) if c.kind == HistoricalFileKind::Text => crate::i18n::trf!("Lines: {count} · {size}", count = c.lines.len(), size = size(c.bytes)),
                    Some(c) => size(c.bytes),
                    None => kind_label(entry.kind()).into(),
                };
                ui.allocate_ui(vec2((ui.available_width() - 150.0).max(60.0), 28.0), |ui| {
                    let mut job = text::LayoutJob::default();
                    if !dir.is_empty() {
                        job.append(
                            &format!("{dir}/"),
                            0.0,
                            TextFormat::simple(FontId::proportional(13.0), FAINT()),
                        );
                    }
                    job.append(name, 0.0, TextFormat::simple(semibold(13.0), TEXT()));
                    let r = ui.add(Label::new(job).truncate()).on_hover_text(&label);
                    register(ui, widgets, "tree-file-label", &r);
                });
                ui.label(RichText::new(meta).size(12.0).color(FAINT()))
                    .on_hover_text(format!("{} · {}", kind_label(entry.kind()), entry.object()));
            });
        });
    });
    let notes = Frame::new()
        .inner_margin(Margin::symmetric(14, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            if !short {
                ui.add(
                    Label::new(
                        RichText::new(format!("{} · {}", kind_label(entry.kind()), entry.object()))
                            .size(11.5)
                            .monospace()
                            .color(FAINT()),
                    )
                    .truncate(),
                )
                .on_hover_text(entry.object().as_str());
            }
            if entry.kind() == TreeEntryKind::Submodule {
                pages::note(
                    ui,
                    crate::i18n::tr("This entry points to a submodule commit. Browsing does not open or fetch the submodule."),
                );
                return false;
            }
            if entry.kind() == TreeEntryKind::Symlink {
                pages::callout(ui, AMBER(), crate::i18n::tr("Link target text only. The link is not followed."));
            }
            let Some(content) = &s.content else {
                if s.running() {
                    if let Some(true) = crate::preview_tab::downloading(ui, preview, &crate::i18n::trf!("Downloading {name}", name), "tree-download-stop", widgets) {
                        emit(intents, TreeIntent::Cancel);
                    }
                } else if let Some(true) = crate::preview_tab::not_downloaded(ui, preview, "tree-download-retry", widgets) {
                    emit(intents, TreeIntent::Retry);
                }
                return false;
            };
            if content.kind == HistoricalFileKind::Binary {
                pages::note(
                    ui,
                    &crate::i18n::trf!("Binary file · {bytes} bytes. Text preview is unavailable.", bytes = content.bytes),
                );
                return false;
            }
            if content.lossy {
                pages::callout(ui, MUTED(), crate::i18n::tr("Not UTF-8: invalid bytes are shown as �."));
            }
            if content.limited {
                pages::callout(
                    ui,
                    AMBER(),
                    crate::i18n::tr("Preview limited to 50,000 lines and 8 KiB per line."),
                );
            }
            if content.lines.is_empty() {
                pages::note(ui, crate::i18n::tr("Empty file at this commit."));
            }
            true
        })
        .inner;
    let Some(content) = s.content.as_ref().filter(|_| notes) else {
        return;
    };
    source_rows(
        ui,
        content,
        None,
        None,
        ("tree-source", entry.commit(), entry.object(), entry.path()),
        "tree-source-scroll",
        widgets,
    );
}

/// Byte size for the file strip: B, KB or MB with one decimal.
pub(crate) fn size(bytes: usize) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{:.1} KB", bytes as f64 / 1024.0),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn content_preparation_preserves_unicode_and_bounds_layout() {
        let file = HistoricalFile {
            kind: HistoricalFileKind::Text,
            bytes: format!("你好 🚀 é\n{}\n\tend\u{0007}", "界".repeat(3000)).into_bytes(),
        };
        let c = prepare_content(file, &CancellationToken::new()).unwrap();
        assert_eq!(c.lines[0], "你好 🚀 é");
        assert!(c.limited);
        assert!(!c.lines[1].contains('\u{fffd}'));
        assert_eq!(c.lines[2], "    end\\u{7}");
        assert_eq!(path_label(b"line\nfile\t"), "line\\nfile\\t");
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(matches!(
            prepare_content(
                HistoricalFile {
                    kind: HistoricalFileKind::Text,
                    bytes: b"x".to_vec()
                },
                &cancel
            ),
            Err(Error::Cancelled)
        ));
    }

    #[test]
    fn non_utf8_text_previews_and_search_hits_open_marked_at_their_line() {
        let token = CancellationToken::new();
        // Latin-1 text without NUL previews with a replacement mark; NUL stays binary.
        let c = prepare_content(
            HistoricalFile { kind: HistoricalFileKind::Binary, bytes: b"caf\xe9 RETRY\r\nnext\n".to_vec() },
            &token,
        )
        .unwrap();
        assert!(c.lossy && c.kind == HistoricalFileKind::Text);
        assert_eq!(c.lines, ["caf\u{fffd} RETRY", "next"]);
        let c = prepare_content(HistoricalFile { kind: HistoricalFileKind::Binary, bytes: b"a\0b".to_vec() }, &token).unwrap();
        assert!(c.lines.is_empty() && c.kind == HistoricalFileKind::Binary && !c.lossy);
        // Byte columns become character marks after a tab and an invalid byte.
        let columns = HashMap::from([(2u64, vec![4..9])]);
        let whole = SearchFileContent::Whole(HistoricalFile {
            kind: HistoricalFileKind::Binary,
            bytes: b"first\n\tx\xe9 RETRY\n".to_vec(),
        });
        let c = prepare_search(whole, 2, &columns, &token).unwrap();
        assert_eq!((c.first, c.window, c.lossy), (1, false, true));
        assert_eq!(c.lines[1], "    x\u{fffd} RETRY");
        assert_eq!(c.marks.get(&1), Some(&vec![7..12]));
        // A window keeps its first line number; a huge whole file opens around the match.
        let window = SearchFileContent::Window(FileWindow {
            first: 498,
            lines: vec![b"a".to_vec(), b"RETRY b".to_vec()],
            size: 3_000_000,
            cut: vec![0],
            reaches_end: false,
        });
        let c = prepare_search(window, 499, &HashMap::from([(499u64, vec![0..5])]), &token).unwrap();
        assert_eq!((c.first, c.window, c.bytes), (498, true, 3_000_000));
        assert_eq!(c.marks.get(&1), Some(&vec![0..5]));
        // A line the window cut says so.
        assert!(c.limited && c.lines[0] == "a… [display limited]" && c.lines[1] == "RETRY b");
        let many = SearchFileContent::Whole(HistoricalFile {
            kind: HistoricalFileKind::Text,
            bytes: "x\n".repeat(60_000).into_bytes(),
        });
        let c = prepare_search(many, 55_000, &HashMap::new(), &token).unwrap();
        assert!(c.window && !c.limited);
        assert_eq!((c.first, c.lines.len()), (54_800, 401));
    }
}
