//! File exploration owns its read receivers. Replacing a selection cancels the
//! old worker and makes late results unreachable; repository refreshes leave it pinned.
use crate::{
    app::{Intent, Rocket, Tab, UiControl},
    backend::{Backend, user_error},
    diff::{FilePatch, Kind, Line, Patch},
    pages,
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use std::{collections::HashMap, sync::Arc};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    Diff,
    Contents,
    #[default]
    Blame,
    /// The history of the lines selected in Blame (`git log -L`).
    Lines,
    /// The file between the selected commit and a second, Ctrl-clicked one.
    Compare,
}
pub enum FileIntent {
    Load,
    More,
    Select(usize),
    Mode(Mode),
    Cancel,
    BlamePage(usize),
    Origin(usize),
    /// Shift-click: extend the selected lines to this blame row.
    ExtendLines(usize),
    /// Ctrl-click: compare the file at this commit with the selected one.
    Compare(usize),
    OriginHistory,
    Back,
    FromChange,
    FromTree,
    ToggleList,
}
pub struct Content {
    pub kind: HistoricalFileKind,
    pub lines: Vec<String>,
    pub limited: bool,
}
enum Read {
    Content(Content),
    Diff(Patch),
    Blame(BlamePage),
}
#[derive(Default)]
pub struct State {
    pub path_input: String,
    pub directory_input: bool,
    pub directory: bool,
    pub path: Option<RepoPath>,
    pub tip: Option<ObjectId>,
    pub entries: Vec<FileHistoryEntry>,
    pub next: Option<FileHistoryCursor>,
    pub limited: bool,
    pub selected: Option<usize>,
    /// The second commit, Ctrl-clicked, whose file version the selected one is compared with.
    pub compare: Option<usize>,
    /// Lines selected in Blame, first and last, numbered as in the selected commit.
    pub lines: Option<(usize, usize)>,
    pub mode: Mode,
    pub content: Option<Content>,
    pub patch: Option<Patch>,
    pub blame: Option<BlamePage>,
    pub origin: Option<BlameLine>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub show_list: bool,
    pub cancelling: bool,
    history: Option<ReadTask<FileHistoryPage>>,
    detail: Option<ReadTask<Read>>,
    append: bool,
    back: Vec<(ObjectId, RepoPath, bool)>,
}
impl State {
    pub fn running(&self) -> bool {
        self.history.is_some() || self.detail.is_some()
    }
    pub fn history_running(&self) -> bool {
        self.history.is_some()
    }
    pub fn detail_running(&self) -> bool {
        self.detail.is_some()
    }
    pub fn stop(&mut self) {
        if let Some(t) = &self.history {
            t.cancel();
        }
        if let Some(t) = &self.detail {
            t.cancel();
        }
        self.cancelling = self.running();
    }
    fn clear_detail(&mut self) {
        // Inspecting a retained entry supersedes a pending replacement history.
        // A pagination read may continue because it preserves the same entries.
        if !self.append {
            self.history = None;
        }
        self.detail = None;
        self.content = None;
        self.patch = None;
        self.blame = None;
        self.origin = None;
        self.error = None;
        self.notice = None;
        self.cancelling = false;
    }
    fn start_detail(&mut self, backend: Backend, repo: Repository, first: usize) {
        let Some(entry) = self.selected.and_then(|i| self.entries.get(i)).cloned() else {
            return;
        };
        self.detail = None;
        self.error = None;
        self.notice = None;
        self.cancelling = false;
        let mode = if self.directory {
            Mode::Diff
        } else {
            self.mode
        };
        let lines = self.lines;
        // Newest first: the entry further down the list is the older version.
        let pair = self
            .compare
            .zip(self.selected)
            .and_then(|(other, selected)| {
                let (older, newer) = if other > selected {
                    (other, selected)
                } else {
                    (selected, other)
                };
                Some((
                    self.entries.get(older)?.clone(),
                    self.entries.get(newer)?.clone(),
                ))
            });
        let task = ReadTask::spawn(move |cancel| {
            let label = match mode {
                Mode::Diff => "file-diff",
                Mode::Contents => "file-content",
                Mode::Blame => "file-blame",
                Mode::Lines => "file-lines",
                Mode::Compare => "file-compare",
            };
            backend.log.request(label, |log| {
                let repo = repo.with_log(log.clone());
                let result = match mode {
                    Mode::Contents => {
                        let file = repo.historical_file(&entry.commit.id, &entry.path, &cancel)?;
                        let mut limited = false;
                        let lines = if file.kind == HistoricalFileKind::Text {
                            let text =
                                std::str::from_utf8(&file.bytes).expect("backend validated UTF-8");
                            let lines: Vec<_> = text
                                .lines()
                                .take(50_001)
                                .map(|s| {
                                    if s.len() > 8192 {
                                        limited = true;
                                    }
                                    preview_text(s.as_bytes(), 8192)
                                })
                                .collect();
                            let mut lines = lines;
                            if lines.len() > 50_000 {
                                lines.truncate(50_000);
                                limited = true;
                            }
                            lines
                        } else {
                            Vec::new()
                        };
                        Read::Content(Content {
                            kind: file.kind,
                            lines,
                            limited,
                        })
                    }
                    Mode::Diff => {
                        let mut paths = vec![entry.path.clone()];
                        if let Some(old) = entry.previous_path {
                            paths.push(old);
                        }
                        Read::Diff(historical_patch(
                            &repo
                                .diff(
                                    &DiffOptions {
                                        target: DiffTarget::Commit {
                                            id: entry.commit.id,
                                        },
                                        paths,
                                        ..Default::default()
                                    },
                                    &cancel,
                                )?
                                .patch,
                        ))
                    }
                    Mode::Blame => Read::Blame(repo.file_blame(
                        &entry.commit.id,
                        &entry.path,
                        first,
                        500,
                        &cancel,
                    )?),
                    Mode::Lines => {
                        let (first, last) = lines.ok_or_else(|| {
                            Error::InvalidInput(crate::i18n::tr("Select lines in Blame first.").into())
                        })?;
                        Read::Diff(line_patch(&repo.line_history(
                            &entry.commit.id,
                            &entry.path,
                            first..=last,
                            &cancel,
                        )?))
                    }
                    Mode::Compare => {
                        let (older, newer) = pair.ok_or_else(|| {
                            Error::InvalidInput(crate::i18n::tr("Ctrl-click a second commit to compare.").into())
                        })?;
                        let mut paths = vec![newer.path.clone()];
                        if older.path != newer.path {
                            paths.push(older.path.clone());
                        }
                        Read::Diff(historical_patch(
                            &repo
                                .diff(
                                    &DiffOptions {
                                        target: DiffTarget::Between {
                                            base: older.commit.id.to_string(),
                                            target: newer.commit.id.to_string(),
                                        },
                                        paths,
                                        ..Default::default()
                                    },
                                    &cancel,
                                )?
                                .patch,
                        ))
                    }
                };
                cancel.check()?;
                Ok(result)
            })
        });
        match task {
            Ok(t) => self.detail = Some(t),
            Err(e) => self.error = Some(user_error(&e)),
        }
    }
    pub(crate) fn load(
        &mut self,
        backend: Backend,
        repo: Repository,
        anchor: Option<(ObjectId, RepoPath, bool)>,
    ) {
        self.history = None;
        self.detail = None;
        self.cancelling = false;
        self.error = None;
        self.notice = None;
        self.append = false;
        let path = match anchor
            .as_ref()
            .map(|(_, p, _)| Ok(p.clone()))
            .unwrap_or_else(|| RepoPath::new(self.path_input.as_bytes().to_vec()))
        {
            Ok(p) => p,
            Err(e) => {
                self.error = Some(user_error(&e));
                let _ = backend.log.request("file-history", |_| Err::<(), _>(e));
                return;
            }
        };
        self.path_input = path.display_lossy().into_owned();
        let directory = anchor.as_ref().map_or(self.directory_input, |a| a.2);
        self.directory_input = directory;
        let task = ReadTask::spawn(move |cancel| {
            backend.log.request(if directory { "directory-history" } else { "file-history" }, |log| {
                let repo = repo.with_log(log.clone());
                let tip = if let Some((tip, _, _)) = anchor {
                    tip
                } else {
                    match repo.resolve_commit("HEAD", &cancel) {
                        Err(error @ Error::Git { .. }) => {
                            if matches!(repo.head(&cancel)?, Head::Unborn { .. }) {
                                return Err(Error::Unsupported(crate::i18n::tr("No committed history yet. Create the first commit before inspecting a file.").into()));
                            }
                            return Err(error);
                        }
                        result => result?,
                    }
                };
                let start = if directory {
                    FileHistoryStart::Directory { tip, path }
                } else {
                    FileHistoryStart::File { tip, path }
                };
                repo.file_history(start, 100, &cancel)
            })
        });
        match task {
            Ok(t) => self.history = Some(t),
            Err(e) => self.error = Some(user_error(&e)),
        }
    }
}
impl Rocket {
    pub(crate) fn file_intent(&mut self, action: FileIntent) {
        if let (Some(backend), Some(tab)) = (self.backend.clone(), self.tabs.get_mut(self.active)) { intent(tab, backend, action); }
    }
    pub(crate) fn poll_file_history(&mut self, ctx: &Context) {
        if let Some(backend) = self.backend.clone() { for tab in &mut self.tabs { poll_tab(tab, &backend, ctx); } }
    }
}
pub(crate) fn intent(tab: &mut Tab, backend: Backend, intent: FileIntent) {
        let Some(repo) = tab.snapshot.as_ref().map(|s| s.repository.clone()) else {
            return;
        };
        let s = &mut tab.file_history;
        match intent {
            FileIntent::ToggleList => s.show_list = !s.show_list,
            FileIntent::Cancel => s.stop(),
            FileIntent::Load => {
                s.back.clear();
                s.load(backend, repo, None);
            }
            FileIntent::FromChange => {
                if let Some((path, _)) = &tab.selected_change {
                    s.path_input = path.display_lossy().into_owned();
                    s.back.clear();
                    tab.view = crate::app::View::FileHistory;
                    let tip = tab
                        .snapshot
                        .as_ref()
                        .and_then(|s| s.head.as_ref())
                        .and_then(|head| match head {
                            Head::Branch { target, .. } | Head::Detached { target } => {
                                Some(target.clone())
                            }
                            _ => None,
                        });
                    if let Some(tip) = tip {
                        s.load(backend, repo, Some((tip, path.clone(), false)));
                    } else {
                        s.error=Some(crate::i18n::tr("No committed history yet. Create the first commit before inspecting this file.").into());
                    }
                }
            }
            FileIntent::FromTree => {
                let Some(listing) = &tab.tree.listing else {
                    return;
                };
                let Ok(path) = RepoPath::new(listing.directory.path().to_vec()) else {
                    return;
                };
                let tip = listing.directory.commit().clone();
                s.back.clear();
                tab.view = crate::app::View::FileHistory;
                s.load(backend, repo, Some((tip, path, true)));
            }
            FileIntent::More => {
                if s.running() {
                    return;
                }
                let Some(cursor) = s.next.clone() else {
                    return;
                };
                s.error = None;
                s.notice = None;
                s.append = true;
                let directory = s.directory;
                match ReadTask::spawn(move |cancel| {
                    backend.log.request(
                        if directory {
                            "directory-history"
                        } else {
                            "file-history"
                        },
                        |log| {
                            repo.with_log(log.clone()).file_history(
                                FileHistoryStart::Continue(cursor),
                                100,
                                &cancel,
                            )
                        },
                    )
                }) {
                    Ok(t) => s.history = Some(t),
                    Err(e) => s.error = Some(user_error(&e)),
                }
            }
            FileIntent::Select(i) => {
                if i >= s.entries.len() {
                    return;
                }
                // Line numbers and a comparison belong to the previous selection.
                s.compare = None;
                s.lines = None;
                if matches!(s.mode, Mode::Lines | Mode::Compare) {
                    s.mode = Mode::Blame;
                }
                s.selected = Some(i);
                s.show_list = false;
                s.clear_detail();
                s.start_detail(backend, repo, 1);
            }
            FileIntent::Mode(mode) => {
                if s.directory && mode != Mode::Diff
                    || mode == Mode::Lines && s.lines.is_none()
                    || mode == Mode::Compare && s.compare.is_none()
                {
                    return;
                }
                s.mode = mode;
                s.clear_detail();
                s.start_detail(backend, repo, 1);
            }
            FileIntent::BlamePage(first) => {
                s.origin = None;
                s.start_detail(backend, repo, first);
            }
            FileIntent::Origin(i) => {
                s.origin = s.blame.as_ref().and_then(|p| p.lines.get(i)).cloned();
                s.lines = s.origin.as_ref().map(|o| (o.line, o.line));
            }
            FileIntent::ExtendLines(i) => {
                let Some(line) = s
                    .blame
                    .as_ref()
                    .and_then(|p| p.lines.get(i))
                    .map(|l| l.line)
                else {
                    return;
                };
                // From the clicked origin line; without one, this line starts the range.
                let anchor = s.origin.as_ref().map_or(line, |o| o.line);
                s.lines = Some((anchor.min(line), anchor.max(line)));
            }
            FileIntent::Compare(i) => {
                if s.directory || i >= s.entries.len() || s.selected.is_none_or(|j| j == i) {
                    return;
                }
                s.compare = Some(i);
                s.mode = Mode::Compare;
                s.clear_detail();
                s.start_detail(backend, repo, 1);
            }
            FileIntent::OriginHistory => {
                let Some(origin) = s.origin.clone() else {
                    return;
                };
                if let (Some(tip), Some(path)) = (&s.tip, &s.path) {
                    if s.back.len() == 20 {
                        s.back.remove(0);
                    }
                    s.back.push((tip.clone(), path.clone(), s.directory));
                }
                s.load(backend, repo, Some((origin.commit, origin.path, false)));
            }
            FileIntent::Back => {
                if let Some(anchor) = s.back.pop() {
                    s.load(backend, repo, Some(anchor));
                }
            }
        }
}
pub(crate) fn poll_tab(tab: &mut Tab, backend: &Backend, ctx: &Context) {
            let s = &mut tab.file_history;
            if let Some(result) = s.history.as_mut().and_then(ReadTask::try_finish) {
                s.history = None;
                match result {
                    Ok(page) => {
                        if !s.append {
                            s.clear_detail();
                            s.entries.clear();
                            s.selected = None;
                        }
                        s.tip = Some(page.tip);
                        s.path = Some(page.path);
                        s.directory = page.directory;
                        if s.directory {
                            s.mode = Mode::Diff;
                        }
                        s.next = page.next;
                        s.limited = page.limited;
                        s.entries.extend(page.entries);
                        if s.selected.is_none() && !s.entries.is_empty() {
                            s.selected = Some(0);
                            s.show_list = false;
                            if let Some(snapshot) = &tab.snapshot {
                                s.start_detail(backend.clone(), snapshot.repository.clone(), 1);
                            }
                        }
                        if s.entries.is_empty() {
                            s.notice=Some(crate::i18n::tr("No history found. Check the exact repository-relative path at this revision.").into());
                        }
                    }
                    Err(Error::Cancelled) => {
                        s.notice = Some(crate::i18n::tr("Read cancelled. Previous results retained.").into())
                    }
                    Err(e) => s.error = Some(user_error(&e)),
                }
            }
            if let Some(result) = s.detail.as_mut().and_then(ReadTask::try_finish) {
                s.detail = None;
                match result {
                    Ok(Read::Content(c)) => s.content = Some(c),
                    Ok(Read::Diff(p)) => s.patch = Some(p),
                    Ok(Read::Blame(b)) => s.blame = Some(b),
                    Err(Error::Cancelled) => {
                        s.notice = Some(crate::i18n::tr("Read cancelled. Select a view or commit to retry.").into())
                    }
                    Err(e) => s.error = Some(user_error(&e)),
                }
            }
            if !s.running() {
                s.cancelling = false;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(30));
            }
}

fn control(
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
/// A line history as one patch: per commit, a header row, then its changes to the lines.
fn line_patch(page: &LineHistoryPage) -> Patch {
    let mut patch = Patch {
        limited: page.limited,
        ..Default::default()
    };
    for entry in &page.entries {
        let commit = &entry.commit;
        let header = Line {
            raw_index: 0,
            syntax: Vec::new(),
            text: format!(
                "● {} · {} · {} · {}",
                &commit.id.as_str()[..8],
                preview_text(commit.subject_lossy().as_bytes(), 120),
                preview_text(&commit.author.name, 70),
                date(commit.author.timestamp, commit.author.offset_minutes)
            ),
            kind: Kind::Hunk,
            old: None,
            new: None,
        };
        let mut lines = vec![header];
        for file in historical_patch(&entry.patch).files {
            lines.extend(file.lines.iter().cloned());
        }
        patch.files.push(FilePatch {
            label: commit.id.to_string(),
            lines: Arc::new(lines),
            ..Default::default()
        });
    }
    patch
}
fn emit(intents: &mut Vec<Intent>, intent: FileIntent) {
    intents.push(Intent::File(intent));
}
fn preview_text(bytes: &[u8], limit: usize) -> String {
    let prefix = crate::text::prefix(bytes, limit);
    let mut text = String::from_utf8_lossy(prefix).into_owned();
    if prefix.len() < bytes.len() {
        text.push_str("… [truncated]");
    }
    text
}

pub(crate) fn historical_patch(bytes: &[u8]) -> Patch {
    let mut patch = Patch::parse(bytes);
    readable_headers(&mut patch);
    patch
}
pub(crate) fn readable_headers(patch: &mut Patch) {
    // These strings are presentation only. The operation retains exact RepoPath
    // bytes; source lines must never be unquoted as though they were filenames.
    for file in &mut patch.files {
        for line in std::sync::Arc::make_mut(&mut file.lines)
            .iter_mut()
            .filter(|l| l.kind == Kind::Header)
        {
            if line.text.starts_with("diff --git ") {
                line.text = format!("File: {}", file.label);
            } else if line.text.starts_with("Binary files ") {
                line.text = "Binary content differs.".into();
            } else {
                for prefix in [
                    "rename from ",
                    "rename to ",
                    "copy from ",
                    "copy to ",
                    "--- ",
                    "+++ ",
                ] {
                    if let Some(path) = line.text.strip_prefix(prefix) {
                        let label = if matches!(prefix, "--- " | "+++ ") {
                            crate::text::diff_file_label(path)
                        } else {
                            crate::text::literal_path_label(path)
                        };
                        line.text = format!("{prefix}{label}");
                        break;
                    }
                }
            }
        }
    }
}

/// File history: path bar, a commit list pane and the Diff / Contents / Blame viewer.
/// Narrow panes show one of the two, switched by `file-toggle-list`.
pub(crate) fn view(
    ui: &mut Ui,
    tab: &mut Tab,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
    commands:&[rocket_git::CustomCommand],
    allow_windows: bool,
) {
    let context=crate::custom_commands::context(tab);
    if allow_windows && let Some(entry)=tab.file_history.selected.and_then(|i|tab.file_history.entries.get(i)) {
        let commit=entry.commit.id.clone();let path=entry.path.clone();
        ui.horizontal_wrapped(|ui| {
            for (key,label,blame) in [("file-open-review",crate::i18n::tr("Open history window"),false),("file-open-blame",crate::i18n::tr("Open blame window"),true)] {
                if control(ui,widgets,key,label,true){intents.push(Intent::OpenReview(crate::reviews::Request{commit:commit.clone(),path:Some(path.clone()),blame}));}
            }
        });
    }
    ui.set_clip_rect(ui.clip_rect().intersect(ui.available_rect_before_wrap()));
    let s = &mut tab.file_history;
    let narrow = ui.available_width() < 850.0;
    let short = ui.available_height() < if tab.review_window { 500.0 } else { 350.0 };
    // A review window draws its own header above this view.
    let compact = short || tab.review_window;
    if !compact {
        let description = match (&s.tip, &s.path) {
            (Some(tip), Some(path)) => crate::i18n::trf!("{path} · pinned {commit} · committed versions", path = path.display_lossy(), commit = &tip.as_str()[..8]),
            _ => crate::i18n::tr("Files follow renames · folders use a fixed path · committed versions only").into(),
        };
        pages::header(
            ui,
            Icon::File,
            crate::i18n::tr("File & folder history"),
            &description,
            |ui| {
                if !s.back.is_empty() && control(ui, widgets, "file-back", crate::i18n::tr("Back"), !s.running()) {
                    emit(intents, FileIntent::Back);
                }
            },
        );
        pages::divider(ui);
    }
    Frame::new()
        .inner_margin(Margin::symmetric(24, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if compact
                    && !s.back.is_empty()
                    && control(ui, widgets, "file-back", crate::i18n::tr("Back"), !s.running())
                {
                    emit(intents, FileIntent::Back);
                }
                let r = ui.add_sized(
                    [(ui.available_width() - 400.0).clamp(80.0, 420.0), 28.0],
                    TextEdit::singleline(&mut s.path_input)
                        .char_limit(4096)
                        .hint_text(crate::i18n::tr("Repository-relative path, e.g. src/main.rs"))
                        .vertical_align(Align::Center),
                );
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    emit(intents, FileIntent::Load);
                }
                register(ui, widgets, "file-path", &r);
                let r = ui.add(checkbox(&mut s.directory_input, crate::i18n::tr("Folder")));
                register(ui, widgets, "file-directory", &r);
                let load = pages::primary(ui, !s.path_input.is_empty(), if allow_windows {crate::i18n::tr("Load HEAD")} else {crate::i18n::tr("Load pinned")});
                register(ui, widgets, "file-load", &load);
                if load.clicked() {
                    emit(intents, FileIntent::Load);
                }
                if control(
                    ui,
                    widgets,
                    "file-cancel",
                    if s.cancelling { crate::i18n::tr("Stopping") } else { crate::i18n::tr("Stop") },
                    s.running() && !s.cancelling,
                ) {
                    emit(intents, FileIntent::Cancel);
                }
                if s.running() {
                    ui.add_space(4.0);
                    // A continuously animated child viewport can monopolize
                    // native redraws. Its reader already polls at 30 ms.
                    if !tab.review_window { ui.spinner(); }
                    ui.add(
                        Label::new(
                            RichText::new(if s.cancelling {
                                crate::i18n::tr("Stopping Git read…")
                            } else if s.history_running() {
                                crate::i18n::tr("Reading file history…")
                            } else {
                                crate::i18n::tr("Reading committed file…")
                            })
                            .size(12.0)
                            .color(MUTED()),
                        )
                        .truncate(),
                    );
                }
            });
            for (message, color) in [(&s.error, RED()), (&s.notice, MUTED())] {
                let Some(message) = message else { continue };
                if short || color == MUTED() {
                    ui.add(Label::new(RichText::new(message).size(12.0).color(color)).truncate())
                        .on_hover_text(message);
                } else {
                    ui.add_space(2.0);
                    pages::callout(ui, color, message);
                }
            }
        });
    if s.directory && !short {
        pages::strip(ui, |ui| {
            ui.add(Label::new(RichText::new(crate::i18n::tr("Fixed folder path · includes descendants · folder renames are not followed · select a file for blame")).size(12.0).color(MUTED())).truncate());
        });
    }
    if !short && !s.entries.is_empty() {
        pages::divider(ui);
        timeline(ui, s, intents, widgets);
    }
    pages::divider(ui);
    if narrow {
        if !short || s.show_list {
            pages::strip(ui, |ui| {
                if control(
                    ui,
                    widgets,
                    "file-toggle-list",
                    if s.show_list {
                        crate::i18n::tr("Inspect file")
                    } else {
                        crate::i18n::tr("Commit list")
                    },
                    !s.entries.is_empty(),
                ) {
                    s.show_list = !s.show_list;
                }
                if let Some(i) = s.selected {
                    ui.add(
                        Label::new(
                            RichText::new(crate::i18n::trf!("Commits: {loaded} / {total}", loaded = i + 1, total = s.entries.len()))
                                .size(12.0)
                                .color(FAINT()),
                        )
                        .truncate(),
                    );
                }
                if short
                    && s.next.is_some()
                    && control(ui, widgets, "file-more", crate::i18n::tr("Load older"), !s.running())
                {
                    emit(intents, FileIntent::More);
                }
            });
        }
        if s.show_list {
            history_list(ui, s, intents, widgets, short, commands, &context);
        } else {
            inspector(ui, s, intents, widgets, short);
        }
    } else {
        pages::list_pane(
            ui,
            Id::new(("file-history-list", tab.id)),
            300.0,
            220.0..=400.0,
            |ui| history_list(ui, s, intents, widgets, false, commands, &context),
        );
        CentralPanel::default()
            .frame(Frame::new().fill(BG()))
            .show(ui, |ui| inspector(ui, s, intents, widgets, false));
    }
}
/// Where each loaded commit sits on the timeline, oldest at the left: by its author
/// date, but at least `gap` apart so every commit stays visible and clickable.
fn timeline_positions(times: &[i64], left: f32, right: f32) -> Vec<f32> {
    let (Some(&first), Some(&last)) = (times.iter().min(), times.iter().max()) else {
        return Vec::new();
    };
    let width = (right - left).max(1.0);
    let gap = (width / times.len() as f32).min(6.0);
    let mut order: Vec<usize> = (0..times.len()).collect();
    // Oldest first; the list is newest first, so equal times keep list order reversed.
    order.sort_by_key(|&i| (times[i], std::cmp::Reverse(i)));
    let mut x = vec![0.0; times.len()];
    let mut previous = f32::NEG_INFINITY;
    for &i in &order {
        let at = if last == first {
            left + width / 2.0
        } else {
            left + (times[i] - first) as f32 / (last - first) as f32 * width
        };
        x[i] = at.max(previous + gap);
        previous = x[i];
    }
    // Pushed past the right end: shift back, keeping the spacing.
    let mut next = f32::INFINITY;
    for &i in order.iter().rev() {
        x[i] = x[i].min(right).min(next - gap);
        next = x[i];
    }
    x
}

/// The loaded commits on a time axis (Fork's commit timeline). A click selects a
/// commit and Ctrl-click compares, as in the list; the selected ones are marked.
fn timeline(
    ui: &mut Ui,
    s: &State,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 58.0), Sense::click());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, PANEL());
    let axis_y = rect.top() + 24.0;
    let (left, right) = (rect.left() + 28.0, rect.right() - 28.0);
    let times: Vec<i64> = s
        .entries
        .iter()
        .map(|e| e.commit.author.timestamp)
        .collect();
    let x = timeline_positions(&times, left, right);
    painter.hline(left - 8.0..=right + 8.0, axis_y, Stroke::new(1.0, BORDER()));
    let day = |e: &FileHistoryEntry| {
        let when = date(e.commit.author.timestamp, e.commit.author.offset_minutes);
        when.split(' ').next().unwrap_or(&when).to_owned()
    };
    let oldest = (0..x.len()).min_by(|&a, &b| x[a].total_cmp(&x[b]));
    let newest = (0..x.len()).max_by(|&a, &b| x[a].total_cmp(&x[b]));
    let label_y = rect.bottom() - 11.0;
    if let (Some(oldest), Some(newest)) = (oldest, newest) {
        painter.text(
            pos2(left - 8.0, label_y),
            Align2::LEFT_CENTER,
            day(&s.entries[oldest]),
            FontId::proportional(11.0),
            FAINT(),
        );
        if newest != oldest {
            painter.text(
                pos2(right + 8.0, label_y),
                Align2::RIGHT_CENTER,
                day(&s.entries[newest]),
                FontId::proportional(11.0),
                FAINT(),
            );
        }
    }
    painter.text(
        pos2(rect.center().x, label_y),
        Align2::CENTER_CENTER,
        crate::i18n::trf!("Timeline · loaded commits: {count}", count = s.entries.len()),
        FontId::proportional(11.0),
        FAINT(),
    );
    let pointer = response.hover_pos();
    let nearest = pointer.and_then(|p| {
        (0..x.len())
            .filter(|&i| (x[i] - p.x).abs() <= 8.0)
            .min_by(|&a, &b| (x[a] - p.x).abs().total_cmp(&(x[b] - p.x).abs()))
    });
    for (i, &at) in x.iter().enumerate() {
        let center = pos2(at, axis_y);
        let (radius, fill) = if s.selected == Some(i) {
            (6.0, BLUE())
        } else if s.compare == Some(i) {
            (6.0, VIOLET())
        } else if nearest == Some(i) {
            (5.0, TEXT())
        } else {
            (3.5, MUTED())
        };
        painter.circle(center, radius, fill, Stroke::new(1.5, PANEL()));
        let tick = Rect::from_center_size(center, vec2(8.0, 28.0)).intersect(ui.clip_rect());
        if tick.is_positive() {
            widgets.insert(
                format!("file-timeline-{}", s.entries[i].commit.id),
                UiControl {
                    rect: tick,
                    enabled: true,
                },
            );
        }
    }
    register(ui, widgets, "file-timeline", &response);
    if let Some(i) = nearest {
        let e = &s.entries[i];
        response.clone().on_hover_text_at_pointer(format!(
            "{} · {}\n{}",
            date(e.commit.author.timestamp, e.commit.author.offset_minutes),
            preview_text(&e.commit.author.name, 70),
            preview_text(e.commit.subject_lossy().as_bytes(), 120)
        ));
        if response.clicked() {
            emit(
                intents,
                if ui.input(|i| i.modifiers.command) {
                    FileIntent::Compare(i)
                } else {
                    FileIntent::Select(i)
                },
            );
        }
    }
}

fn history_list(
    ui: &mut Ui,
    s: &State,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
    short: bool,
    commands:&[rocket_git::CustomCommand],
    context:&rocket_git::CommandContext,
) {
    if !short {
        pages::bar(
            ui,
            crate::i18n::tr("Commits"),
            &crate::i18n::trf!("Loaded: {count} · newest first", count = s.entries.len()),
            |ui| {
                if control(
                    ui,
                    widgets,
                    "file-more",
                    crate::i18n::tr("Load older"),
                    s.next.is_some() && !s.running(),
                ) {
                    emit(intents, FileIntent::More);
                }
            },
        );
    }
    if s.limited {
        Frame::new()
            .inner_margin(Margin::symmetric(8, 6))
            .show(ui, |ui| {
                pages::callout(
                    ui,
                    AMBER(),
                    crate::i18n::tr("10,000 commit limit. Inspect an older commit to continue."),
                )
            });
    }
    // Rows are 46 points; the row pitch includes this spacing.
    ui.spacing_mut().item_spacing.y = 2.0;
    let out = Frame::new()
        .inner_margin(Margin::symmetric(6, 6))
        .show(ui, |ui| {
            ScrollArea::vertical()
                .id_salt("file-history-list-scroll")
                .auto_shrink([false, false])
                .min_scrolled_height(0.0)
                .show_rows(ui, 46.0, s.entries.len(), |ui, rows| {
                    for i in rows {
                        let entry = &s.entries[i];
                        let title = preview_text(entry.commit.subject_lossy().as_bytes(), 120);
                        // The date leads the author so narrow panes keep it.
                        let when = date(
                            entry.commit.author.timestamp,
                            entry.commit.author.offset_minutes,
                        );
                        let detail = format!(
                            "{} · {} · {}",
                            &entry.commit.id.as_str()[..8],
                            when.split(' ').next().unwrap_or(&when),
                            preview_text(&entry.commit.author.name, 70)
                        );
                        let mut pills = Vec::new();
                        if entry.previous_path.is_some() {
                            pills.push((crate::i18n::tr("Renamed").to_owned(), VIOLET()));
                        } else if entry.deleted {
                            pills.push((crate::i18n::tr("Deleted").to_owned(), RED()));
                        }
                        let r = pages::row(
                            ui,
                            pages::Row {
                                image: Icon::Commit,
                                title: &title,
                                detail: &detail,
                                pills,
                                indent: 0.0,
                            },
                            s.selected == Some(i) || s.compare == Some(i),
                            true,
                        );
                        if r.rect.intersects(ui.clip_rect()) {
                            widgets.insert(
                                format!("file-commit-{}", entry.commit.id),
                                UiControl {
                                    rect: r.rect.intersect(ui.clip_rect()),
                                    enabled: r.enabled(),
                                },
                            );
                        }
                        if r.clicked() {
                            emit(
                                intents,
                                if ui.input(|i| i.modifiers.command) {
                                    FileIntent::Compare(i)
                                } else {
                                    FileIntent::Select(i)
                                },
                            );
                        }
                        r.context_menu(|ui| {
                            let mut context=crate::custom_commands::historical_context(context,&entry.commit.id,entry.path.as_bytes());
                            if s.directory {context.file=None;}
                            crate::custom_commands::context_menu(ui,commands,rocket_git::CommandScope::Commit,&context,intents,widgets);
                            if !s.directory {crate::custom_commands::context_menu(ui,commands,rocket_git::CommandScope::File,&context,intents,widgets);}
                            let target=crate::integrations::CommandTarget::commit(&context.repository,&entry.commit.id);
                            crate::integrations::context_rows(ui,&target,crate::integrations::Divider::Before,intents,widgets);
                        });
                    }
                })
        })
        .inner;
    let rect = out.inner_rect.intersect(ui.clip_rect());
    if rect.is_positive() {
        widgets.insert(
            "file-list-scroll".into(),
            UiControl {
                rect,
                enabled: true,
            },
        );
    }
}
fn inspector(
    ui: &mut Ui,
    s: &State,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
    short: bool,
) {
    let Some(entry) = s.selected.and_then(|i| s.entries.get(i)) else {
        Frame::new()
            .inner_margin(Margin::symmetric(24, 8))
            .show(ui, |ui| {
                pages::empty(
                    ui,
                    crate::i18n::tr("Enter an exact path to read its committed history. Deleted paths are supported."),
                )
            });
        return;
    };
    pages::strip(ui, |ui| {
        // Laid out right to left: blame paging outermost, then the mode switch;
        // the commit subject takes the remaining width on the left.
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if s.mode == Mode::Blame
                && let Some(b) = &s.blame
            {
                let first = b.lines.first().map_or(1, |l| l.line);
                let last = b.lines.last().map_or(0, |l| l.line);
                if control(
                    ui,
                    widgets,
                    "file-blame-next",
                    if short { "›" } else { crate::i18n::tr("Next") },
                    b.next_line.is_some() && !s.running(),
                ) {
                    emit(intents, FileIntent::BlamePage(b.next_line.unwrap()));
                }
                ui.label(
                    RichText::new(format!("{first}–{last} / {}", b.total_lines))
                        .size(12.0)
                        .color(FAINT()),
                );
                if control(
                    ui,
                    widgets,
                    "file-blame-prev",
                    if short { "‹" } else { crate::i18n::tr("Previous") },
                    first > 1 && !s.running(),
                ) {
                    emit(
                        intents,
                        FileIntent::BlamePage(first.saturating_sub(500).max(1)),
                    );
                }
                ui.add_space(6.0);
            }
            let mut modes = vec![
                (Mode::Diff, "file-diff", crate::i18n::tr("Diff")),
                (Mode::Contents, "file-contents", crate::i18n::tr("Contents")),
                (Mode::Blame, "file-blame", crate::i18n::tr("Blame")),
            ];
            if s.lines.is_some() {
                modes.push((Mode::Lines, "file-lines", crate::i18n::tr("Lines")));
            }
            if s.compare.is_some() {
                modes.push((Mode::Compare, "file-compare", crate::i18n::tr("Compare")));
            }
            let modes = if s.directory { &modes[..1] } else { &modes[..] };
            let items: Vec<_> = modes.iter().map(|m| (m.2, s.mode == m.0)).collect();
            for ((mode, key, _), r) in modes.iter().copied().zip(pages::segments(ui, &items)) {
                register(ui, widgets, key, &r);
                if r.clicked() {
                    emit(intents, FileIntent::Mode(mode));
                }
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                if short && control(ui, widgets, "file-toggle-list", crate::i18n::tr("Commits"), true) {
                    emit(intents, FileIntent::ToggleList);
                }
                let subject = entry.commit.subject_lossy();
                ui.add(Label::new(RichText::new(subject.as_ref()).font(semibold(13.0))).truncate())
                    .on_hover_text(format!("{}\n{}", entry.commit.id, subject));
            });
        });
    });
    let shown = Frame::new()
        .inner_margin(Margin::symmetric(14, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 5.0;
            if short {
                ui.add(
                    Label::new(
                        RichText::new(format!(
                            "{} · {}{}",
                            &entry.commit.id.as_str()[..8],
                            entry.path.display_lossy(),
                            if entry.previous_path.is_some() {
                                crate::i18n::tr(" · renamed")
                            } else {
                                ""
                            }
                        ))
                        .size(12.0)
                        .color(BLUE()),
                    )
                    .truncate(),
                )
                .on_hover_text(crate::i18n::trf!("{id}\n{subject}\nRenamed from: {path}\nPinned history: {tip}", id = entry.commit.id, subject = entry.commit.subject_lossy(), path = entry.previous_path.as_ref().map_or_else(|| "—".into(), |p| p.display_lossy().into_owned()), tip = s.tip.as_ref().map_or("—", |t| t.as_str())));
            } else {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(&entry.commit.id.as_str()[..8])
                            .monospace()
                            .size(12.0)
                            .color(FAINT()),
                    );
                    ui.add(
                        Label::new(RichText::new(entry.path.display_lossy()).size(12.5).color(BLUE()))
                            .truncate(),
                    )
                    .on_hover_text(entry.path.display_lossy());
                });
                if let Some(old) = &entry.previous_path {
                    ui.add(
                        Label::new(
                            RichText::new(crate::i18n::trf!("Renamed from {path}", path = old.display_lossy()))
                                .size(12.0)
                                .color(GREEN()),
                        )
                        .truncate(),
                    )
                    .on_hover_text(old.display_lossy());
                }
            }
            if let Some(origin) = &s.origin {
                let label = format!(
                    "{} · {}:{}{}",
                    &origin.commit.as_str()[..8],
                    origin.path.display_lossy(),
                    origin.original_line,
                    if origin.boundary {
                        crate::i18n::tr(" · history boundary")
                    } else {
                        ""
                    }
                );
                Frame::new()
                    .fill(BLUE().gamma_multiply(0.08))
                    .stroke(Stroke::new(1.0, BLUE().gamma_multiply(0.3)))
                    .corner_radius(8)
                    .inner_margin(Margin::symmetric(10, 4))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.allocate_ui_with_layout(
                                vec2((ui.available_width() - 145.0).max(50.0), 23.0),
                                Layout::left_to_right(Align::Center),
                                |ui| {
                                    ui.add(
                                        Label::new(RichText::new(&label).size(12.0)).truncate(),
                                    )
                                    .on_hover_text(format!(
                                        "{label}\n{}",
                                        String::from_utf8_lossy(&origin.summary)
                                    ));
                                },
                            );
                            if control(
                                ui,
                                widgets,
                                "file-origin-history",
                                crate::i18n::tr("History from here"),
                                !s.running(),
                            ) {
                                emit(intents, FileIntent::OriginHistory);
                            }
                            if let Some((first, last)) = s.lines {
                                let label = if first == last {
                                    crate::i18n::trf!("Line {first} history", first)
                                } else {
                                    crate::i18n::trf!("Lines {first}–{last} history", first, last)
                                };
                                if control(ui, widgets, "file-line-history", &label, !s.running())
                                {
                                    emit(intents, FileIntent::Mode(Mode::Lines));
                                }
                            }
                        });
                        if !short {
                            ui.add(
                                Label::new(
                                    RichText::new(String::from_utf8_lossy(&origin.summary))
                                        .size(12.0)
                                        .color(MUTED()),
                                )
                                .truncate(),
                            );
                        }
                    });
            }
            match s.mode {
                Mode::Contents => {
                    if let Some(c) = &s.content {
                        let message = match c.kind {
                            HistoricalFileKind::Missing => Some(
                                crate::i18n::tr("File absent at this commit. Select an earlier version or view its deletion diff."),
                            ),
                            HistoricalFileKind::Binary => Some(
                                crate::i18n::tr("Binary or non-UTF-8 file. Text preview and blame are unavailable."),
                            ),
                            HistoricalFileKind::NonFile => {
                                Some(crate::i18n::tr("This path is a directory or submodule, not a file."))
                            }
                            _ => None,
                        };
                        if let Some(message) = message {
                            pages::note(ui, message);
                            return false;
                        }
                        if c.limited {
                            pages::callout(
                                ui,
                                AMBER(),
                                crate::i18n::tr("Preview limited to 50,000 lines and 8 KiB per line."),
                            );
                        }
                        if c.lines.is_empty() {
                            pages::note(ui, crate::i18n::tr("Empty file at this commit."));
                        }
                    }
                }
                Mode::Diff => {
                    if let Some(p) = &s.patch {
                        if p.limited {
                            pages::callout(ui, AMBER(), crate::i18n::tr("Diff preview limit reached."));
                        }
                        if p.files.is_empty() {
                            pages::note(ui, crate::i18n::tr("No first-parent patch for this commit."));
                        }
                    }
                }
                Mode::Blame => {
                    if s.blame.as_ref().is_some_and(|b| b.total_lines == 0) {
                        pages::note(ui, crate::i18n::tr("Empty file: no lines to attribute."));
                    } else if s.lines.is_none() {
                        pages::note(
                            ui,
                            crate::i18n::tr("Click a line's commit to select it; Shift-click another to select a range."),
                        );
                    }
                }
                Mode::Lines => {
                    if let (Some((first, last)), Some(p)) = (s.lines, &s.patch) {
                        pages::note(
                            ui,
                            &crate::i18n::trf!("Commits that changed lines {first}–{last} as of this commit, newest first.", first, last),
                        );
                        if p.limited {
                            pages::callout(ui, AMBER(), crate::i18n::tr("Showing the newest 200 commits."));
                        }
                    }
                }
                Mode::Compare => {
                    if let Some(other) = s.compare.and_then(|i| s.entries.get(i)) {
                        let (older, newer) = if s.compare > s.selected {
                            (other, entry)
                        } else {
                            (entry, other)
                        };
                        pages::note(
                            ui,
                            &crate::i18n::trf!("Changes from {from} to {to}, the older version first.", from = &older.commit.id.as_str()[..8], to = &newer.commit.id.as_str()[..8]),
                        );
                    }
                    if s.patch.as_ref().is_some_and(|p| p.files.is_empty()) {
                        pages::note(ui, crate::i18n::tr("The file is the same in both commits."));
                    }
                }
            }
            true
        })
        .inner;
    if !shown {
        return;
    }
    // The layout uses only visible rows below; strings are prepared on the worker
    // except small attribution labels. Large blobs never become one layout job.
    let count = match s.mode {
        Mode::Diff | Mode::Lines | Mode::Compare => s
            .patch
            .as_ref()
            .map_or(0, |p| p.files.iter().map(|f| f.lines.len()).sum()),
        Mode::Contents => s.content.as_ref().map_or(0, |c| c.lines.len()),
        Mode::Blame => s.blame.as_ref().map_or(0, |b| b.lines.len()),
    };
    pages::divider(ui);
    let numbers = match s.mode {
        Mode::Blame => s
            .blame
            .as_ref()
            .and_then(|b| b.lines.last())
            .map_or(1, |l| l.line),
        _ => count,
    };
    let gutter = ((numbers.max(1).to_string().len() as f32) * 8.0 + 22.0).max(46.0);
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.spacing_mut().interact_size.y = 23.0;
    ui.spacing_mut().button_padding.y = 2.0;
    let out = ScrollArea::both()
        .id_salt(("file-source", &entry.commit.id, s.mode as u8))
        .auto_shrink([false, false])
        .min_scrolled_height(0.0)
        .show_rows(ui, 23.0, count, |ui, range| {
            for i in range {
                let top = ui.cursor().top();
                ui.horizontal(|ui| {
                    ui.set_min_height(23.0);
                    ui.spacing_mut().item_spacing.x = 8.0;
                    match s.mode {
                        Mode::Blame => {
                            let b = &s.blame.as_ref().unwrap().lines[i];
                            let origin = s
                                .origin
                                .as_ref()
                                .is_some_and(|o| o.line == b.line && o.commit == b.commit);
                            let chosen = s
                                .lines
                                .is_some_and(|(first, last)| (first..=last).contains(&b.line));
                            if origin || chosen {
                                ui.painter().rect_filled(
                                    Rect::from_x_y_ranges(
                                        ui.clip_rect().x_range(),
                                        top..=top + 23.0,
                                    ),
                                    0.0,
                                    BLUE().gamma_multiply(0.08),
                                );
                            }
                            ui.add_space(6.0);
                            let r =
                                ui.add_sized(
                                    [175.0, 23.0],
                                    Button::new(
                                        RichText::new(format!(
                                            "{} · {}",
                                            &b.commit.as_str()[..8],
                                            preview_text(&b.author, 48)
                                        ))
                                        .size(12.0)
                                        .color(if origin { BLUE() } else { MUTED() }),
                                    )
                                    .frame(false)
                                    .truncate(),
                                );
                            if r.rect.intersects(ui.clip_rect()) {
                                widgets.insert(
                                    format!("file-line-{}", b.line),
                                    UiControl {
                                        rect: r.rect.intersect(ui.clip_rect()),
                                        enabled: r.enabled(),
                                    },
                                );
                            }
                            if r.clicked() {
                                emit(
                                    intents,
                                    if ui.input(|i| i.modifiers.shift) {
                                        FileIntent::ExtendLines(i)
                                    } else {
                                        FileIntent::Origin(i)
                                    },
                                );
                            }
                            r.on_hover_text(format!(
                                "{}\n{}\n{}:{}",
                                b.commit,
                                String::from_utf8_lossy(&b.summary),
                                b.path.display_lossy(),
                                b.original_line
                            ));
                            pages::gutter(ui, &b.line.to_string(), gutter, 23.0);
                            ui.add(
                                Label::new(
                                    RichText::new(preview_text(&b.content, 8192)).monospace(),
                                )
                                .extend(),
                            );
                        }
                        Mode::Contents => {
                            pages::gutter(ui, &(i + 1).to_string(), gutter, 23.0);
                            ui.add(
                                Label::new(
                                    RichText::new(&s.content.as_ref().unwrap().lines[i])
                                        .monospace(),
                                )
                                .extend(),
                            );
                        }
                        Mode::Diff | Mode::Lines | Mode::Compare => {
                            let mut offset = i;
                            for file in &s.patch.as_ref().unwrap().files {
                                if let Some(line) = file.lines.get(offset) {
                                    let (color, tint) = match line.kind {
                                        Kind::Added => (GREEN(), Some(GREEN())),
                                        Kind::Removed => (RED(), Some(RED())),
                                        Kind::Hunk => (BLUE(), Some(BLUE())),
                                        Kind::Header => (MUTED(), None),
                                        Kind::Context => (TEXT(), None),
                                    };
                                    if let Some(tint) = tint {
                                        ui.painter().rect_filled(
                                            Rect::from_x_y_ranges(
                                                ui.clip_rect().x_range(),
                                                top..=top + 23.0,
                                            ),
                                            0.0,
                                            tint.gamma_multiply(0.08),
                                        );
                                    }
                                    ui.add_space(6.0);
                                    ui.add(
                                        Label::new(
                                            RichText::new(&line.text).monospace().color(color),
                                        )
                                        .extend(),
                                    );
                                    break;
                                }
                                offset -= file.lines.len();
                            }
                        }
                    }
                });
            }
        });
    let rect = out.inner_rect.intersect(ui.clip_rect());
    if rect.is_positive() {
        widgets.insert(
            "file-source-scroll".into(),
            UiControl {
                rect,
                enabled: true,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timeline_positions_follow_time_and_keep_commits_apart() {
        // Newest first, as listed: the oldest goes to the left end.
        assert_eq!(
            timeline_positions(&[30, 20, 10], 0.0, 100.0),
            [100.0, 50.0, 0.0]
        );
        // Equal times spread out from the middle, oldest (last listed) leftmost.
        assert_eq!(
            timeline_positions(&[5, 5, 5], 0.0, 100.0),
            [62.0, 56.0, 50.0]
        );
        // Crowded at the right end: pushed back left, still 6 points apart.
        assert_eq!(
            timeline_positions(&[100, 99, 98, 0], 0.0, 100.0),
            [100.0, 94.0, 88.0, 0.0]
        );
        assert!(timeline_positions(&[], 0.0, 100.0).is_empty());
    }

    /// The timeline places the commits oldest to newest, and a click selects one.
    #[test]
    fn the_timeline_orders_and_selects_commits() {
        use crate::{
            batch::flow_tests::{click, open, wait},
            test_common as common,
        };
        use eframe::egui::Context;
        common::isolated(
            "ROCKET_FILE_HISTORY_UI_CASE",
            "file_history::tests::the_timeline_orders_and_selects_commits",
            || {
                let f = common::Fixture::new();
                for (text, message) in [
                    ("one\n", "first"),
                    ("two\n", "second"),
                    ("three\n", "third"),
                ] {
                    f.write("notes.txt", text);
                    f.commit(message);
                }
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                app.tabs[0].view = crate::app::View::FileHistory;
                app.tabs[0].file_history.path_input = "notes.txt".into();
                app.dispatch(Intent::File(FileIntent::Load));
                wait(&ctx, &mut app);
                let ids: Vec<_> = app.tabs[0]
                    .file_history
                    .entries
                    .iter()
                    .map(|e| e.commit.id.clone())
                    .collect();
                assert_eq!(ids.len(), 3);
                let x = |app: &Rocket, id: &ObjectId| {
                    app.frame_widgets[&format!("file-timeline-{id}")]
                        .rect
                        .center()
                        .x
                };
                // Oldest at the left, newest at the right.
                assert!(x(&app, &ids[2]) < x(&app, &ids[1]) && x(&app, &ids[1]) < x(&app, &ids[0]));
                click(&ctx, &mut app, &format!("file-timeline-{}", ids[2]));
                wait(&ctx, &mut app);
                assert_eq!(app.tabs[0].file_history.selected, Some(2));
                click(&ctx, &mut app, &format!("file-timeline-{}", ids[1]));
                wait(&ctx, &mut app);
                assert_eq!(app.tabs[0].file_history.selected, Some(1));
            },
        );
    }
    #[test]
    fn file_diff_headers_decode_unicode_without_rewriting_source() {
        let patch = historical_patch(b"diff --git a/old \"b/\\346\\226\\207.txt\"\nsimilarity index 90%\nrename from old\nrename to \"\\346\\226\\207.txt\"\n--- a/old\n+++ \"b/\\346\\226\\207.txt\"\n@@ -1 +1 @@\n-old\n+literal \\346\\226\\207\n");
        let lines = &patch.files[0].lines;
        assert!(lines.iter().any(|l| l.text == "rename to 文.txt"));
        assert!(lines.iter().any(|l| l.text == "+++ 文.txt"));
        assert!(lines.iter().any(|l| l.text == "+literal \\346\\226\\207"));
        assert_eq!(patch.files[0].added, 1);
        assert_eq!(patch.files[0].removed, 1);
        let renamed = historical_patch(b"diff --git a/a/old b/b/new\nsimilarity index 100%\nrename from a/old\nrename to b/new\n");
        assert!(
            renamed.files[0]
                .lines
                .iter()
                .any(|l| l.text == "rename from a/old")
        );
        assert!(
            renamed.files[0]
                .lines
                .iter()
                .any(|l| l.text == "rename to b/new")
        );
    }

    /// Line history from Blame and a two-commit comparison through real clicks:
    /// Shift-click extends the selected lines, Ctrl-click a second commit compares.
    #[test]
    fn blame_lines_show_their_history_and_two_commits_compare() {
        use crate::{
            batch::flow_tests::{click, enabled, open, wait},
            test_common as common,
        };
        use eframe::egui::{Context, Event, Modifiers, PointerButton, Pos2, RawInput, Rect, vec2};
        // A click with modifier keys held, as the app sees it.
        fn click_with(ctx: &Context, app: &mut Rocket, key: &str, modifiers: Modifiers) {
            assert!(enabled(ctx, app, key), "{key} is disabled");
            let pos = app.frame_widgets[key].rect.center();
            let press = |pressed| Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers,
            };
            // egui keeps the held keys from ModifiersChanged until the next such event.
            for events in [
                vec![
                    Event::ModifiersChanged(modifiers),
                    Event::PointerMoved(pos),
                    press(true),
                    press(false),
                ],
                vec![Event::ModifiersChanged(Modifiers::NONE)],
            ] {
                app.frame_widgets.clear();
                let mut output = ctx.run_ui(
                    RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1480.0, 1000.0))),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        let mut intents = Vec::new();
                        app.shell(ui, &mut intents);
                        for intent in intents {
                            app.dispatch(intent);
                        }
                    },
                );
                output.textures_delta.clear();
            }
        }
        common::isolated(
            "ROCKET_FILE_HISTORY_UI_CASE",
            "file_history::tests::blame_lines_show_their_history_and_two_commits_compare",
            || {
                let f = common::Fixture::new();
                for (text, message) in [
                    ("one\ntwo\nthree\n", "add notes"),
                    ("one\nTWO\nthree\n", "change two"),
                    ("one\nTWO\nTHREE\n", "change three"),
                ] {
                    f.write("notes.txt", text);
                    f.commit(message);
                }
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                app.tabs[0].view = crate::app::View::FileHistory;
                app.tabs[0].file_history.path_input = "notes.txt".into();
                app.dispatch(Intent::File(FileIntent::Load));
                wait(&ctx, &mut app);
                let s = &app.tabs[0].file_history;
                assert_eq!((s.entries.len(), s.mode), (3, Mode::Blame));
                // Click line 2's commit, then Shift-click line 3's: lines 2 to 3.
                click(&ctx, &mut app, "file-line-2");
                click_with(&ctx, &mut app, "file-line-3", Modifiers::SHIFT);
                assert_eq!(app.tabs[0].file_history.lines, Some((2, 3)));
                click(&ctx, &mut app, "file-line-history");
                wait(&ctx, &mut app);
                let s = &app.tabs[0].file_history;
                assert_eq!(s.mode, Mode::Lines);
                let headers: Vec<&str> = s
                    .patch
                    .as_ref()
                    .unwrap()
                    .files
                    .iter()
                    .map(|f| f.lines[0].text.as_str())
                    .collect();
                assert_eq!(headers.len(), 3, "{headers:?}");
                for (header, subject) in
                    headers
                        .iter()
                        .zip(["change three", "change two", "add notes"])
                {
                    assert!(header.contains(subject), "{headers:?}");
                }
                // Ctrl-click the oldest commit: the file from it to the selected one.
                let oldest = app.tabs[0].file_history.entries[2].commit.id.clone();
                click_with(
                    &ctx,
                    &mut app,
                    &format!("file-commit-{oldest}"),
                    // Windows input reports Ctrl as both ctrl and command.
                    Modifiers::CTRL | Modifiers::COMMAND,
                );
                wait(&ctx, &mut app);
                let s = &app.tabs[0].file_history;
                assert_eq!(
                    (s.mode, s.compare, s.selected),
                    (Mode::Compare, Some(2), Some(0))
                );
                let text: Vec<&str> = s
                    .patch
                    .as_ref()
                    .unwrap()
                    .files
                    .iter()
                    .flat_map(|f| f.lines.iter().map(|l| l.text.as_str()))
                    .collect();
                for line in ["-two", "-three", "+TWO", "+THREE"] {
                    assert!(text.contains(&line), "{text:?}");
                }
                // A plain click selects one commit again.
                click(&ctx, &mut app, &format!("file-commit-{oldest}"));
                let s = &app.tabs[0].file_history;
                assert_eq!(
                    (s.compare, s.lines, s.mode, s.selected),
                    (None, None, Mode::Blame, Some(2))
                );
                wait(&ctx, &mut app);
            },
        );
    }
}
