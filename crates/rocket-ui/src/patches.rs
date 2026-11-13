//! Reviewed patch exchange and selective discard. Workers own raw bytes; widgets
//! show bounded previews and never synthesize patches from display strings.
use crate::{
    app::*,
    backend::{Action, Outcome, user_error},
    pages,
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use std::{path::PathBuf, sync::Arc};

#[derive(Clone)]
pub enum Mode {
    Import,
    Export,
    Discard(Arc<ChangeReview>, Vec<usize>),
    Stash(Arc<ChangeReview>, Vec<usize>),
}
impl Mode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Import => "patch-import",
            Self::Export => "patch-export",
            Self::Discard(..) => "discard-selection",
            Self::Stash(..) => "stash-selection",
        }
    }
    fn title(&self) -> &'static str {
        match self {
            Self::Import => crate::i18n::tr("Import patch"),
            Self::Export => crate::i18n::tr("Export patch"),
            Self::Discard(..) => crate::i18n::tr("Discard selected changes"),
            Self::Stash(..) => crate::i18n::tr("Stash selected changes"),
        }
    }
}
/// Where a patch to import comes from without the dialog's path field.
pub enum PatchSource {
    /// A file dropped on the window.
    File(PathBuf),
    /// Text pasted with Ctrl+V or Repository → Patches → Apply Patch from Clipboard.
    Text(String),
}

/// Whether pasted text is worth reviewing as a patch: a Git diff header, or a
/// `---` line followed by a `+++` line. The review decides what can be applied.
pub fn looks_like_patch(text: &str) -> bool {
    let mut old_header = false;
    for line in text.lines() {
        if line.starts_with("diff --git ") || (old_header && line.starts_with("+++ ")) {
            return true;
        }
        old_header = line.starts_with("--- ");
    }
    false
}

#[derive(Clone)]
pub enum Preview {
    Apply(Arc<PatchPreview>),
    Export(Arc<PatchExportPreview>),
    Series(Arc<PatchSeriesPreview>),
    Stash(Arc<StashSelectionPreview>),
}
impl Preview {
    fn action(&self) -> Action {
        match self {
            Self::Apply(p) => Action::ApplyPatch(p.clone()),
            Self::Export(p) => Action::ExportPatch(p.clone()),
            Self::Series(p) => Action::ExportPatchSeries(p.clone()),
            Self::Stash(p) => Action::Workflow{review:Arc::new(crate::workflows::Review::StashSelection(p.as_ref().clone())),selected:Vec::new()},
        }
    }
    pub fn paths(&self) -> Vec<String> {
        match self {
            Self::Series(p) => p.entries().iter().flat_map(|e|e.paths.iter().map(|p|p.display_lossy().into_owned())).take(250).collect(),
            Self::Stash(p) => p.files().iter().map(|f|f.path().display_lossy().into_owned()).collect(),
            Self::Apply(p) => p
                .files()
                .iter()
                .map(|f| f.path().display_lossy().into_owned())
                .collect(),
            Self::Export(p) => p
                .paths()
                .iter()
                .map(|p| p.display_lossy().into_owned())
                .collect(),
        }
    }
}
pub struct DialogState {
    pub mode: Mode,
    pub path: String,
    /// Import: a patch pasted from the clipboard, used instead of `path`.
    pub text: Option<Arc<Vec<u8>>>,
    /// The commit to export; for a commit series, the last one.
    pub revision: String,
    /// A commit series' first commit.
    pub base: String,
    pub source: usize,
    pub preview: Option<Preview>,
    pub task: Option<ReadTask<(Preview, Vec<Vec<crate::patch_diff::Shown>>)>>,
    pub applying: bool,
    pub notice: Option<String>,
    pub file: usize,
    /// The reviewed patch's files as shown; one list per commit for a series.
    pub(crate) shown: Vec<Vec<crate::patch_diff::Shown>>,
    /// Stash selected changes: the stash's message.
    pub message: String,
    /// The folder a commit series was just saved to.
    pub saved: Option<PathBuf>,
}
impl DialogState {
    pub fn running(&self) -> bool {
        self.task.is_some()
    }
    /// A field changed: what was reviewed or saved no longer applies.
    fn touched(&mut self) {
        self.preview = None;
        self.notice = None;
        self.saved = None;
    }
}
impl Rocket {
    pub(crate) fn start_patch_dialog(&mut self) {
        let Some(d) = &mut self.dialog else { return };
        let DialogKind::Patch(mode) = &d.kind else {
            return;
        };
        // A selection's stash is named after its file until the message is edited.
        let message = match mode {
            Mode::Stash(review, _) => crate::diff::Patch::parse(review.patch())
                .files
                .first()
                .map(|f| {
                    let name = f.label.rsplit('/').next().unwrap_or(&f.label);
                    crate::i18n::trf!("Selected changes in {name}", name)
                })
                .unwrap_or_else(|| crate::i18n::tr("Selected changes").into()),
            _ => String::new(),
        };
        d.patch = Some(DialogState {
            mode: mode.clone(),
            path: String::new(),
            text: None,
            revision: "HEAD".into(),
            base: "HEAD".into(),
            source: 0,
            preview: None,
            task: None,
            applying: false,
            notice: None,
            file: 0,
            shown: Vec::new(),
            message,
            saved: None,
        });
        if matches!(mode, Mode::Discard(..) | Mode::Stash(..)) {
            self.read_patch_preview();
        }
    }
    fn read_patch_preview(&mut self) {
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.patch else { return };
        if s.applying {
            return;
        }
        s.preview = None;
        s.shown = Vec::new();
        s.task = None;
        s.notice = None;
        s.file = 0;
        d.error = None;
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let Some(root) = self
            .tabs
            .iter()
            .find(|t| Some(t.id) == d.owner)
            .map(|t| t.path.clone())
        else {
            return;
        };
        let mode = s.mode.clone();
        let path = PathBuf::from(s.path.trim());
        let text = s.text.clone();
        let series = s.source == 3;
        let base = s.base.trim().to_owned();
        let tip = s.revision.trim().to_owned();
        let source = match s.source {
            1 => PatchExportSource::Staged,
            2 => PatchExportSource::Commit(tip.clone()),
            _ => PatchExportSource::Unstaged,
        };
        let message = s.message.trim().to_owned();
        let review = match &mode {
            Mode::Stash(review, _) => Some(review.clone()),
            _ => None,
        };
        match ReadTask::spawn(move |cancel| {
            let preview = match mode {
            Mode::Import => match &text {
                Some(text) => backend.patch_text_preview(&root, text, &cancel),
                None => backend.patch_import_preview(&root, &path, &cancel),
            }
            .map(|p| Preview::Apply(Arc::new(p))),
            Mode::Export if series => backend.patch_series_preview(&root,&base,&tip,&path,&cancel).map(|p|Preview::Series(Arc::new(p))),
            Mode::Export => backend
                .patch_export_preview(&root, source, &path, &cancel)
                .map(|p| Preview::Export(Arc::new(p))),
            Mode::Discard(review, lines) => backend
                .discard_selection_preview(&root, &review, &lines, &cancel)
                .map(|p| Preview::Apply(Arc::new(p))),
            Mode::Stash(review,lines)=>backend.stash_selection_preview(&root,&review,&lines,&message,&cancel).map(|p|Preview::Stash(Arc::new(p))),
            }?;
            // What the dialog draws, worked out here rather than on every frame.
            let shown = match &preview {
                Preview::Apply(p) => vec![crate::patch_diff::files(p.patch())],
                Preview::Export(p) => vec![crate::patch_diff::files(p.patch())],
                Preview::Series(p) => p.entries().iter().map(|e| crate::patch_diff::files(e.patch())).collect(),
                Preview::Stash(_) => vec![review.map(|r| crate::patch_diff::files(r.patch())).unwrap_or_default()],
            };
            Ok((preview, shown))
        }) {
            Ok(task) => s.task = Some(task),
            Err(e) => d.error = Some(user_error(&e)),
        }
    }
    /// Open Export on Commit series, with its range when the commits are chosen.
    pub(crate) fn export_series(&mut self, range: Option<(String, String)>) {
        if self.tab().is_none() {
            return;
        }
        self.show_dialog(DialogKind::Patch(Mode::Export));
        let Some(s) = self.dialog.as_mut().and_then(|d| d.patch.as_mut()) else {
            return;
        };
        s.source = 3;
        if let Some((first, last)) = range {
            s.base = first;
            s.revision = last;
        }
    }
    /// Open the reviewed patch import for the active repository with a dropped
    /// patch file or pasted patch text, and start its review at once.
    pub(crate) fn import_patch(&mut self, source: PatchSource) {
        if self.tab().is_none() {
            self.notice = Some(crate::i18n::tr("Open a repository first to apply a patch.").into());
            return;
        }
        self.show_dialog(DialogKind::Patch(Mode::Import));
        let Some(s) = self.dialog.as_mut().and_then(|d| d.patch.as_mut()) else {
            return;
        };
        match source {
            PatchSource::File(path) => s.path = crate::views::display_path(&path),
            PatchSource::Text(text) => s.text = Some(Arc::new(text.into_bytes())),
        }
        self.read_patch_preview();
    }
    pub(crate) fn poll_patch_dialog(&mut self) {
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.patch else { return };
        if let Some(result) = s.task.as_mut().and_then(ReadTask::try_finish) {
            s.task = None;
            match result {
                Ok((p, shown)) => {
                    s.preview = Some(p);
                    s.shown = shown;
                }
                Err(e) => d.error = Some(user_error(&e)),
            }
        }
    }
    pub(crate) fn finish_patch_dialog(&mut self, owner: Option<u64>, outcome: &Outcome) {
        let Some(d) = &mut self.dialog else { return };
        if d.owner != owner {
            return;
        }
        let Some(s) = &mut d.patch else { return };
        if !s.applying {
            return;
        }
        s.applying = false;
        s.preview = None;
        let text = format!("{}\n{}", outcome.summary, outcome.details.join("\n"));
        if outcome.command_succeeded && !outcome.recovery_pending {
            let series = matches!(s.mode, Mode::Export) && s.source == 3;
            s.saved = series.then(|| PathBuf::from(s.path.trim()));
            s.notice = Some(if series { outcome.summary.clone() } else { text });
            d.error = None;
        } else {
            d.error = Some(text);
        }
    }
    pub(crate) fn patch_dialog(&mut self, ctx: &Context) {
        let can_write = self.can_write();
        let running = pages::Running::of(self.write.as_ref());
        let Some(mut d) = self.dialog.take() else {
            return;
        };
        let Some(s) = &mut d.patch else {
            self.dialog = Some(d);
            return;
        };
        let context = Sources::of(self.tabs.iter().find(|t| Some(t.id) == d.owner), s.source);
        let (mut refresh, mut apply, mut close, mut cancel) = (false, false, false, false);
        let (mut browse, mut show) = (false, None);
        let export = matches!(s.mode, Mode::Export);
        let discard = matches!(s.mode, Mode::Discard(..) | Mode::Stash(..));
        let stashing = matches!(s.mode, Mode::Stash(..));
        let series = export && s.source == 3;
        let head = pages::ReviewHeader {
            image: Icon::File,
            title: if series { crate::i18n::tr("Export commit series") } else { s.mode.title() },
            subtitle: match s.mode {
                Mode::Import => {
                    crate::i18n::tr("Applies a patch to your working files. Every file it changes is backed up first.")
                }
                Mode::Export if series => {
                    crate::i18n::tr("One patch file for each commit, oldest first. Nothing in this repository changes.")
                }
                Mode::Export => {
                    crate::i18n::tr("Save changes as a patch file to share or apply somewhere else. Nothing in this repository changes.")
                }
                Mode::Stash(..) => {
                    crate::i18n::tr("Saves the lines you picked as a stash, then takes them out of the file. Your other changes stay.")
                }
                Mode::Discard(..) => {
                    crate::i18n::tr("Review file changes and any executable-mode updates; recovery copies are kept.")
                }
            },
            // Discarding the selected lines removes them from the working files.
            danger: discard && !stashing,
        };
        let response = pages::review_at(
            ctx,
            "patch-dialog",
            if series { 980.0 } else { 860.0 },
            head,
            &mut self.frame_widgets,
            "dialog-patch-dialog-scroll",
            true,
            |ui, part, widgets| match part {
                pages::Part::Body => {
                    ui.spacing_mut().item_spacing.y = 12.0;
                    // Results lead the body so a long preview never hides them.
                    if let (Some(text), Some(folder)) = (&s.notice, &s.saved) {
                        if saved_strip(ui, text, widgets) {
                            show = Some(folder.clone());
                        }
                    } else if let Some(text) = &s.notice {
                        pages::callout(ui, GREEN(), text);
                    }
                    match &d.error {
                        // Shown under the folder it's about.
                        Some(error) if series && error.contains(FOLDER_EXISTS) => {}
                        Some(error) if series => pages::refusal(ui, AMBER(), error),
                        Some(error) => pages::callout(ui, AMBER(), error),
                        None => {}
                    }
                    let enabled = !s.running() && !s.applying;
                    match &s.mode {
                        Mode::Import => import_body(ui, s, widgets, enabled),
                        Mode::Export => export_body(ui, s, &context, d.error.as_deref(), &mut browse, widgets, enabled),
                        Mode::Stash(..) => stash_body(ui, s, widgets, enabled),
                        Mode::Discard(..) => discard_body(ui, s, widgets),
                    }
                    if s.running() {
                        pages::waiting(
                            ui,
                            crate::i18n::tr("Reading source, checking files and building the preview…"),
                        );
                    }
                }
                pages::Part::Footer if s.applying => {
                    cancel = pages::running_footer(ui, widgets, &running, &["dialog-cancel"]);
                }
                pages::Part::Footer if s.saved.is_some() => {
                    pages::footer_status(ui, false, crate::i18n::tr("Saved"));
                }
                pages::Part::Footer => {
                    let label = match (&s.mode, &s.preview) {
                        (Mode::Export, Some(Preview::Series(p))) => {
                            crate::i18n::trf!("Save patches: {count}", count = p.entries().len())
                        }
                        (Mode::Export, _) if series => crate::i18n::tr("Save patches").into(),
                        (Mode::Export, _) => crate::i18n::tr("Save patch").into(),
                        (Mode::Stash(_, lines), _) => crate::i18n::trf!("Stash lines: {count}", count = lines.len()),
                        (Mode::Discard(..), _) => crate::i18n::tr("Back up & discard selected").into(),
                        (Mode::Import, _) => crate::i18n::tr("Apply patch").into(),
                    };
                    let ready = can_write && !s.running() && !s.applying && s.preview.is_some();
                    let r = if discard && !stashing {
                        pages::danger(ui, ready, &label)
                    } else {
                        pages::primary(ui, ready, &label)
                    };
                    register(ui, widgets, "dialog-submit", &r);
                    apply = r.clicked();
                    let can_review = !s.running()
                        && !s.applying
                        && (discard || s.text.is_some() || !s.path.trim().is_empty())
                        && !(stashing && s.message.trim().is_empty());
                    let r = if s.preview.is_some() || d.error.is_some() || discard {
                        ui.add_enabled(can_review, Button::new(crate::i18n::tr("Review again")))
                    } else {
                        pages::primary(ui, can_review, crate::i18n::tr("Review"))
                    };
                    register(ui, widgets, "dialog-patch-review", &r);
                    refresh = r.clicked();
                    // Stop the running read; the dialog stays open.
                    let busy = s.running();
                    if busy {
                        let r = ui.button(crate::i18n::tr("Stop"));
                        register(ui, widgets, "dialog-cancel", &r);
                        cancel = r.clicked();
                    }
                    let status = if busy {
                        crate::i18n::tr("Reviewing").to_owned()
                    } else if d.error.is_some() {
                        crate::i18n::tr("Fix the problem above, then review again").to_owned()
                    } else if let Some(preview) = &s.preview {
                        reviewed_status(preview)
                    } else {
                        crate::i18n::tr("Review to see exactly what will happen").to_owned()
                    };
                    pages::footer_status(ui, false, &status);
                }
            },
        );
        // The ×, Esc or a click outside closes only an idle dialog.
        close |= response.should_close() && !s.applying && !s.running();
        let action = apply
            .then(|| s.preview.as_ref().map(Preview::action))
            .flatten();
        if cancel && !s.applying {
            s.task = None;
            s.preview = None;
            s.notice =
                Some(crate::i18n::tr("Preview cancelled. No working files or patch output were changed.").into());
        }
        self.dialog = if close { None } else { Some(d) };
        if cancel && self.write.is_some() {
            self.cancel_write();
        }
        if refresh {
            self.read_patch_preview();
        }
        if let Some(folder) = show {
            self.show_folder(&folder);
        }
        if browse {
            let current = self.dialog.as_ref().and_then(|d| d.patch.as_ref()).map(|s| s.path.clone()).unwrap_or_default();
            self.browse(crate::folder_browser::PickTarget::SeriesSave, &current);
        }
        if let Some(action) = action
            && let Some(owner) = self.dialog.as_ref().and_then(|d| d.owner)
        {
            self.execute(owner, action);
            if self.write.is_some()
                && let Some(s) = self.dialog.as_mut().and_then(|d| d.patch.as_mut())
            {
                s.applying = true;
            }
        }
    }
}

/// A refused series folder, shown under the field rather than above the form.
const FOLDER_EXISTS: &str = "patch_series_output_exists";

/// The green line once a series is saved, with the way to its folder. Returns a click.
fn saved_strip(ui: &mut Ui, text: &str, widgets: &mut std::collections::HashMap<String, UiControl>) -> bool {
    let mut clicked = false;
    Frame::new()
        .fill(GREEN().gamma_multiply(0.08))
        .stroke(Stroke::new(1.0, GREEN().gamma_multiply(0.3)))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(12, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (mark, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                icon(ui.painter(), mark, Icon::Check, GREEN());
                ui.label(RichText::new(text).size(12.5).color(GREEN()));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let r = pages::tool(ui, Icon::Folder, crate::i18n::tr("Show in Explorer"), true);
                    register(ui, widgets, "dialog-patch-show", &r);
                    clicked = r.clicked();
                });
            });
        });
    clicked
}

/// What the export dialog shows about the repository: change counts and recent commits.
struct Sources {
    unstaged: Option<usize>,
    staged: Option<usize>,
    /// Commits in history to pick from.
    choices: Vec<crate::commit_picker::Choice>,
    head: Option<ObjectId>,
}
impl Sources {
    fn of(tab: Option<&Tab>, source: usize) -> Self {
        let status = tab.and_then(|t| t.snapshot.as_ref()).and_then(|s| s.status.as_ref());
        let count = |f: fn(&StatusEntry) -> bool| status.map(|s| s.entries.iter().filter(|e| f(e)).count());
        Self {
            unstaged: count(|e| {
                !matches!(e.worktree, Change::Unchanged)
                    && !matches!(e.kind, EntryKind::Untracked | EntryKind::Ignored)
            }),
            staged: count(StatusEntry::staged),
            // A series picks from the whole list; the other sources show none or a few.
            choices: tab.map_or_else(Vec::new, |t| {
                let (commits, limit) = match source {
                    3 => (t.loaded_commits(), 300),
                    2 => (t.commits(), 8),
                    _ => (t.commits(), 0),
                };
                commits
                    .iter()
                    .take(limit)
                    .map(|c| crate::commit_picker::Choice {
                        id: c.id.clone(),
                        subject: subject(c),
                        meta: format!("{} · {}", display(&c.author.name), crate::tags::age(c.author.timestamp)),
                    })
                    .collect()
            }),
            head: tab.and_then(|t| t.snapshot.as_ref()).and_then(|s| match s.head.as_ref() {
                Some(Head::Branch { target, .. } | Head::Detached { target }) => Some(target.clone()),
                _ => None,
            }),
        }
    }
}

/// The footer line once a review is ready.
fn reviewed_status(preview: &Preview) -> String {
    match preview {
        Preview::Series(p) => crate::i18n::trf!("Reviewed · {commits} · nothing has changed yet", commits = crate::i18n::trn!(p.entries().len(), "{n} commit", "{n} commits")),
        Preview::Export(p) => crate::i18n::trf!("Reviewed · {files} · {size}", files = files_count(p.paths().len()), size = size(p.patch().len())),
        Preview::Stash(_) => crate::i18n::tr("Reviewed · nothing has changed yet · the file is backed up first").into(),
        Preview::Apply(_) => crate::i18n::tr("Reviewed · nothing has changed yet").into(),
    }
}

fn files_count(n: usize) -> String {
    crate::i18n::trn!(n, "{n} file", "{n} files")
}

/// "1.2 KB", "412 bytes".
pub(crate) fn size(bytes: usize) -> String {
    match bytes {
        0..1024 => crate::i18n::trf!("{bytes} bytes", bytes),
        1024..1_048_576 => format!("{:.1} KB", bytes as f64 / 1024.0),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

/// A label in the form's left column, level with the first line of its field.
fn form_label(ui: &mut Ui, text: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(84.0, 30.0), Sense::hover());
    ui.painter().text(
        rect.left_center(),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(13.0),
        MUTED(),
    );
}

/// A form row: label column, then the content.
pub(crate) fn form_row(ui: &mut Ui, label: &str, content: impl FnOnce(&mut Ui)) {
    ui.horizontal_top(|ui| {
        form_label(ui, label);
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            content(ui)
        });
    });
}

/// "parser fix 文.patch · …\folder" for a path, or the path as typed.
fn split_path(path: &str) -> (String, String) {
    let shown = crate::views::display_path(std::path::Path::new(path.trim()));
    let path = std::path::Path::new(&shown);
    match (path.file_name(), path.parent()) {
        (Some(name), Some(parent)) => (
            name.to_string_lossy().into_owned(),
            parent.to_string_lossy().into_owned(),
        ),
        _ => (shown.clone(), String::new()),
    }
}

/// Patch files on the left, the chosen file's diff on the right.
fn files_and_diff(
    ui: &mut Ui,
    widgets: &mut std::collections::HashMap<String, UiControl>,
    files: &[crate::patch_diff::Shown],
    letters: &[char],
    selected: &mut usize,
    id: &str,
) {
    if files.is_empty() {
        return;
    }
    *selected = (*selected).min(files.len() - 1);
    let height = 300.0;
    ui.horizontal_top(|ui| {
        let list = 230.0_f32.min(ui.available_width() * 0.35);
        ui.allocate_ui_with_layout(vec2(list, height), Layout::top_down(Align::Min), |ui| {
            ui.set_width(list);
            ScrollArea::vertical()
                .id_salt((id, "files"))
                .max_height(height)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for (i, file) in files.iter().enumerate() {
                        let letter = letters.get(i).copied().unwrap_or('M');
                        let r = file_row(ui, letter, file, *selected == i);
                        register(ui, widgets, format!("dialog-patch-file-{i}"), &r);
                        if r.clicked() {
                            *selected = i;
                        }
                    }
                });
        });
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            crate::patch_diff::rows(ui, (id, "diff", *selected), std::slice::from_ref(&files[*selected]), false, None, height - 8.0);
        });
    });
}

/// One file in a patch's list: what happens to it, its name and folder, its counts.
fn file_row(ui: &mut Ui, letter: char, file: &crate::patch_diff::Shown, selected: bool) -> Response {
    let response = ui.add(
        Button::new("")
            .frame(false)
            .min_size(vec2(ui.available_width(), 32.0)),
    );
    let rect = response.rect;
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 6.0, SELECTED());
    } else if response.hovered() || response.has_focus() {
        painter.rect_filled(rect, 6.0, HOVER());
    }
    let color = match letter {
        'A' => GREEN(),
        'D' => RED(),
        _ => BLUE(),
    };
    painter.text(
        pos2(rect.left() + 12.0, rect.center().y),
        Align2::CENTER_CENTER,
        letter,
        FontId::monospace(12.0),
        color,
    );
    let counts = painter.layout_no_wrap(
        format!("+{} −{}", file.added, file.removed),
        FontId::monospace(11.5),
        HINT(),
    );
    let right = rect.right() - 8.0 - counts.size().x;
    painter.galley(pos2(right, rect.center().y - counts.size().y / 2.0), counts, HINT());
    let (folder, name) = file
        .label
        .rsplit_once('/')
        .map_or(("", file.label.as_str()), |(folder, name)| (folder, name));
    let mut job = text::LayoutJob::default();
    job.append(name, 0.0, TextFormat::simple(FontId::proportional(13.0), TEXT()));
    if !folder.is_empty() {
        job.append(&format!("  {folder}"), 0.0, TextFormat::simple(FontId::proportional(11.5), FAINT()));
    }
    job.wrap = text::TextWrapping::truncate_at_width((right - rect.left() - 34.0).max(20.0));
    let galley = painter.layout_job(job);
    painter.galley(pos2(rect.left() + 26.0, rect.center().y - galley.size().y / 2.0), galley, TEXT());
    response.widget_info(|| WidgetInfo::selected(Role::Button, true, selected, &file.label));
    response
}

/// Import: where the patch comes from, a summary, then its files and diff.
fn import_body(
    ui: &mut Ui,
    s: &mut DialogState,
    widgets: &mut std::collections::HashMap<String, UiControl>,
    enabled: bool,
) {
    form_row(ui, crate::i18n::tr("Patch"), |ui| {
        if let Some(text) = s.text.clone() {
            ui.horizontal(|ui| {
                let (mark, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                icon(ui.painter(), mark, Icon::File, MUTED());
                ui.label(RichText::new(crate::i18n::tr("From the clipboard")).strong());
                let lines = text.split(|b| *b == b'\n').filter(|l| !l.is_empty()).count();
                ui.label(RichText::new(crate::i18n::trf!("· lines: {count} · {size}", count = lines, size = size(text.len()))).color(HINT()));
                let r = ui.add_enabled(enabled, Button::new(crate::i18n::tr("Use a file instead")));
                register(ui, widgets, "dialog-patch-use-file", &r);
                if r.clicked() {
                    s.text = None;
                    s.preview = None;
                    s.notice = None;
                }
            });
        } else {
            let r = ui.add_enabled(
                enabled,
                TextEdit::singleline(&mut s.path)
                    .desired_width(f32::INFINITY)
                    .min_size(vec2(0.0, 30.0))
                    .align(Align2::LEFT_CENTER)
                    .hint_text(crate::i18n::tr("Patch file path, or drop a patch file on the window")),
            );
            register(ui, widgets, "dialog-patch-path", &r);
            if r.changed() {
                s.preview = None;
                s.notice = None;
            }
            if !s.path.trim().is_empty() {
                let (name, folder) = split_path(&s.path);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.label(RichText::new(name).size(12.0).strong());
                    if !folder.is_empty() {
                        ui.add(Label::new(RichText::new(format!(" · {folder}")).size(12.0).color(FAINT())).truncate());
                    }
                });
            }
        }
    });
    let Some(Preview::Apply(p)) = &s.preview else {
        return;
    };
    let files = s.shown.first().map(Vec::as_slice).unwrap_or_default();
    let letters: Vec<char> = files
        .iter()
        .map(|f| {
            let path = f.label.as_str();
            match p.files().iter().find(|e| e.path().display_lossy() == path) {
                Some(e) if e.before().is_none() => 'A',
                Some(e) if e.after().is_none() => 'D',
                _ => 'M',
            }
        })
        .collect();
    // The summary: files, line counts, new and removed files, and that it applies.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let (added, removed) = files.iter().fold((0, 0), |(a, r), f| (a + f.added, r + f.removed));
        ui.label(RichText::new(files_count(p.files().len())).strong());
        ui.label(RichText::new(format!("+{added}")).monospace().color(GREEN()));
        ui.label(RichText::new(format!("−{removed}")).monospace().color(RED()));
        for letter in ['M', 'A', 'D'] {
            let n = letters.iter().filter(|l| **l == letter).count();
            if n > 0 {
                let text = match letter {
                    'M' => crate::i18n::trf!("{count} changed", count = n),
                    'A' => crate::i18n::trf!("{count} new", count = n),
                    _ => crate::i18n::trf!("{count} deleted", count = n),
                };
                ui.label(RichText::new(text).color(HINT()));
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(crate::i18n::tr("Applies cleanly")).color(GREEN()));
            let (mark, _) = ui.allocate_exact_size(vec2(13.0, 13.0), Sense::hover());
            icon(ui.painter(), mark, Icon::Check, GREEN());
        });
    });
    if !p.mode_changes().is_empty() {
        pages::callout(ui, AMBER(), crate::i18n::tr("Executable modes will update Git's index. Existing staged content stays; new executable files are staged. The original index is backed up."));
    }
    files_and_diff(ui, widgets, files, &letters, &mut s.file, "patch-import");
}

/// Export: what to export, the commit or range, where to save it, then the result.
fn export_body(
    ui: &mut Ui,
    s: &mut DialogState,
    context: &Sources,
    error: Option<&str>,
    browse: &mut bool,
    widgets: &mut std::collections::HashMap<String, UiControl>,
    enabled: bool,
) {
    form_row(ui, crate::i18n::tr("Export"), |ui| {
        ui.add_enabled_ui(enabled, |ui| {
            let count = |n: Option<usize>| n.map_or_else(String::new, |n| format!("  {n}"));
            let sources = [
                (0, crate::i18n::trf!("Unstaged{count}", count = count(context.unstaged)), "dialog-patch-source-unstaged"),
                (1, crate::i18n::trf!("Staged{count}", count = count(context.staged)), "dialog-patch-source-staged"),
                (2, crate::i18n::tr("A commit").to_owned(), "dialog-patch-source-commit"),
                (3, crate::i18n::tr("Commit series").to_owned(), "dialog-patch-source-series"),
            ];
            let items: Vec<_> = sources.iter().map(|(i, label, _)| (label.as_str(), s.source == *i)).collect();
            for (r, (i, _, key)) in pages::segments(ui, &items).iter().zip(&sources) {
                register(ui, widgets, *key, r);
                if r.clicked() && s.source != *i {
                    s.source = *i;
                    s.touched();
                    s.file = 0;
                }
            }
        });
    });
    let series = s.source == 3;
    if s.source == 2 {
        form_row(ui, crate::i18n::tr("Commit"), |ui| {
            let r = ui.add_enabled(
                enabled,
                TextEdit::singleline(&mut s.revision)
                    .font(TextStyle::Monospace)
                    .hint_text(crate::i18n::tr("Commit, branch or tag"))
                    .desired_width(f32::INFINITY)
                    .min_size(vec2(0.0, 30.0))
                    .align(Align2::LEFT_CENTER),
            );
            register(ui, widgets, "dialog-patch-revision", &r);
            if r.changed() {
                s.touched();
            }
            // Recent commits from history, to pick instead of typing.
            Frame::new()
                .stroke(Stroke::new(1.0, CARD_STROKE()))
                .corner_radius(8)
                .inner_margin(Margin::symmetric(4, 4))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 1.0;
                    for (i, choice) in context.choices.iter().enumerate() {
                        let on = s.revision.trim() == choice.id.as_str();
                        let r = crate::commit_picker::row(ui, choice, on, enabled);
                        register(ui, widgets, format!("dialog-patch-pick-{i}"), &r);
                        if r.clicked() && !on {
                            s.revision = choice.id.to_string();
                            s.touched();
                        }
                    }
                    if context.choices.is_empty() {
                        pages::note(ui, crate::i18n::tr("Type a commit ID, branch or tag."));
                    }
                });
        });
    }
    if series {
        form_row(ui, crate::i18n::tr("Commits"), |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                // The two fields share what the labels, the arrow and the four gaps
                // between them leave: 110 at least, as before, and more where First and
                // Last are longer words.
                let (first, last) = (crate::i18n::tr("First"), crate::i18n::tr("Last"));
                let wide = |text: &str| {
                    let font = TextStyle::Body.resolve(ui.style());
                    ui.painter().layout_no_wrap(text.to_owned(), font, HINT()).size().x
                };
                let fixed = (wide(first) + wide("→") + wide(last) + 4.0 * 8.0).ceil().max(110.0);
                let half = ((ui.available_width() - fixed) / 2.0).max(120.0);
                ui.label(RichText::new(first).color(HINT()));
                let (head, choices) = (context.head.as_ref(), &context.choices);
                if crate::commit_picker::field(ui, &mut s.base, "dialog-patch-base", choices, head, half, enabled, widgets) {
                    s.touched();
                }
                ui.label(RichText::new("→").color(FAINT()));
                ui.label(RichText::new(last).color(HINT()));
                if crate::commit_picker::field(ui, &mut s.revision, "dialog-patch-revision", choices, head, half, enabled, widgets) {
                    s.touched();
                }
            });
        });
        if let Some(Preview::Series(p)) = &s.preview {
            let shown: Vec<crate::series_view::Row> = p
                .entries()
                .iter()
                .zip(&s.shown)
                .map(|(e, files)| crate::series_view::Row {
                    subject: String::from_utf8_lossy(&e.subject).chars().take(240).collect(),
                    meta: format!(
                        "{} · {} · {}",
                        &e.id.as_str()[..7],
                        String::from_utf8_lossy(&e.author),
                        crate::series_view::short_date(&String::from_utf8_lossy(&e.date))
                    ),
                    added: files.iter().map(|f| f.added).sum(),
                    removed: files.iter().map(|f| f.removed).sum(),
                })
                .collect();
            let count = crate::i18n::trf!("{commits}, oldest first", commits = crate::i18n::trn!(shown.len(), "{n} commit", "{n} commits"));
            let files = s.shown.get(s.file).map(Vec::as_slice).unwrap_or_default();
            crate::series_view::split(
                ui,
                &format!("{count} · {}", size(p.total_bytes())),
                &shown,
                &mut s.file,
                files,
                crate::series_view::Keys { row: "dialog-patch-commit", scroll: "dialog-patch-commits-scroll" },
                widgets,
                enabled,
                (ui.ctx().content_rect().height() - 640.0).clamp(200.0, 300.0),
            );
        }
    }
    form_row(ui, if series { crate::i18n::tr("Save to") } else { crate::i18n::tr("Save as") }, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            // The button is measured first and the field takes what it leaves: Browse is
            // a longer word in some languages.
            let label = crate::i18n::tr("Browse");
            let button = if series { pages::button_width(ui, label, 0.0) } else { 0.0 };
            let width = if series { ui.available_width() - button - 8.0 } else { f32::INFINITY };
            let r = ui.add_enabled(
                enabled,
                TextEdit::singleline(&mut s.path)
                    .desired_width(width)
                    .min_size(vec2(0.0, 30.0))
                    .align(Align2::LEFT_CENTER)
                    .hint_text(if series { "C:\\Exports\\new-series" } else { "C:\\folder\\changes.patch" }),
            );
            register(ui, widgets, "dialog-patch-path", &r);
            if r.changed() {
                s.touched();
            }
            if series {
                let r = ui.add_enabled(enabled, Button::new(label).min_size(vec2(button, 30.0)));
                register(ui, widgets, "dialog-patch-browse", &r);
                *browse = r.clicked();
            }
        });
        if s.saved.is_some() {
            pages::note(ui, crate::i18n::tr("Working files and staging didn't change."));
            return;
        }
        if series && let Some(error) = error.filter(|e| e.contains(FOLDER_EXISTS)) {
            // A refusal at saving time comes after the lines about what was saved.
            let text = pages::plain_error(error).replace('\n', ". ");
            let text = text.find(crate::i18n::tr("This folder")).map_or(text.as_str(), |at| &text[at..]);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (mark, _) = ui.allocate_exact_size(vec2(13.0, 13.0), Sense::hover());
                icon(ui.painter(), mark, Icon::Warning, RED());
                ui.add(Label::new(RichText::new(text).size(12.0).color(RED())).wrap());
            });
            return;
        }
        match &s.preview {
            Some(Preview::Series(p)) => crate::tags::check(
                ui,
                true,
                &crate::i18n::trf!("New folder · patch files to save inside: {count}", count = p.entries().len()),
            ),
            Some(Preview::Export(_)) => crate::tags::check(ui, true, crate::i18n::tr("New file")),
            _ => pages::note(
                ui,
                if series {
                    crate::i18n::tr("A folder that doesn't exist yet; Rocket creates it.")
                } else {
                    crate::i18n::tr("A file that doesn't exist yet; existing files are never overwritten.")
                },
            ),
        }
    });
    if let Some(Preview::Export(p)) = &s.preview {
        if let Some(id) = p.commit() {
            pages::note(ui, &crate::i18n::trf!("Pinned to commit {commit}", commit = id.as_str()));
        }
        let files = s.shown.first().map(Vec::as_slice).unwrap_or_default();
        files_and_diff(ui, widgets, files, &[], &mut s.file, "patch-export");
    }
}

/// Stash selected changes: the message, then the file's changes with the chosen ones
/// marked.
fn stash_body(
    ui: &mut Ui,
    s: &mut DialogState,
    widgets: &mut std::collections::HashMap<String, UiControl>,
    enabled: bool,
) {
    let Mode::Stash(review, lines) = &s.mode else {
        return;
    };
    ui.label(RichText::new(crate::i18n::tr("Stash message")).size(12.0).color(MUTED()));
    let r = ui.add_enabled(
        enabled,
        TextEdit::singleline(&mut s.message)
            .desired_width(f32::INFINITY)
            .min_size(vec2(0.0, 30.0))
            .align(Align2::LEFT_CENTER),
    );
    register(ui, widgets, "dialog-stash-message", &r);
    if r.changed() {
        s.preview = None;
        s.notice = None;
    }
    let total = review.changed_lines().len();
    let chosen = lines.len();
    let file = s
        .shown
        .first()
        .and_then(|files| files.first())
        .map_or_else(String::new, |f| f.label.clone());
    let (folder, name) = file.rsplit_once('/').map_or(("", file.as_str()), |(f, n)| (f, n));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(RichText::new(crate::i18n::trf!("Changed lines: {chosen} of {total}", chosen, total)).strong());
        ui.label(RichText::new(crate::i18n::tr("in")).color(HINT()));
        let mut job = text::LayoutJob::default();
        if !folder.is_empty() {
            job.append(&format!("{folder}/"), 0.0, TextFormat::simple(FontId::proportional(13.0), FAINT()));
        }
        job.append(name, 0.0, TextFormat::simple(FontId::proportional(13.0), TEXT()));
        ui.label(job);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(crate::i18n::tr("Stays in the file")).size(12.0).color(HINT()));
            let (mark, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
            ui.painter().rect_filled(mark, 2.0, MUTED().gamma_multiply(0.5));
            ui.add_space(8.0);
            ui.label(RichText::new(crate::i18n::tr("Goes into the stash")).size(12.0).color(HINT()));
            let (mark, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
            ui.painter().rect_filled(mark, 2.0, BLUE());
        });
    });
    if let Some(files) = s.shown.first() {
        let marks: std::collections::BTreeSet<usize> = lines.iter().copied().collect();
        crate::patch_diff::rows(ui, "stash-selection-diff", files, false, Some(&marks), 300.0);
    }
    let rest = total.saturating_sub(chosen);
    pages::note(
        ui,
        &if rest == 0 {
            crate::i18n::trf!("Afterwards {name} has none of these changes. Get the stash back from Stashes whenever you like.", name)
        } else {
            crate::i18n::trf!("Afterwards {name} keeps the remaining changed lines: {count}. Get the stash back from Stashes whenever you like.", name, count = rest)
        },
    );
}

/// Discard selected changes: the selection, then each file before and after.
fn discard_body(
    ui: &mut Ui,
    s: &mut DialogState,
    widgets: &mut std::collections::HashMap<String, UiControl>,
) {
    if let Mode::Discard(_, lines) = &s.mode {
        pages::caption(ui, crate::i18n::tr("What will be discarded"));
        pages::well(ui, |ui| {
            pages::field(ui, crate::i18n::tr("Selection"), &crate::i18n::trf!("Changed lines: {count}", count = lines.len()), false)
        });
        pages::note(ui, crate::i18n::tr("Unselected changes are retained."));
    }
    let Some(Preview::Apply(p)) = &s.preview else {
        return;
    };
    let paths = s.preview.as_ref().map(Preview::paths).unwrap_or_default();
    pages::caption(ui, crate::i18n::tr("Files"));
    pages::well(ui, |ui| {
        ScrollArea::vertical()
            .id_salt("dialog-patch-paths")
            .max_height(95.0)
            .show(ui, |ui| {
                for (i, path) in paths.iter().enumerate() {
                    let r = ui.add(
                        Button::selectable(s.file == i, RichText::new(path).monospace()).truncate(),
                    );
                    register(ui, widgets, format!("dialog-patch-file-{i}"), &r);
                    if r.clicked() {
                        s.file = i;
                    }
                    r.on_hover_text(path);
                }
            });
    });
    if let Some(file) = p.files().get(s.file) {
        if ui.available_width() > 610.0 {
            ui.columns(2, |cols| {
                file_preview(&mut cols[0], crate::i18n::tr("Current working file"), file.before());
                file_preview(&mut cols[1], crate::i18n::tr("Reviewed result"), file.after());
            });
        } else {
            file_preview(ui, crate::i18n::tr("Current working file"), file.before());
            file_preview(ui, crate::i18n::tr("Reviewed result"), file.after());
        }
    }
}

fn register(
    ui: &Ui,
    widgets: &mut std::collections::HashMap<String, UiControl>,
    key: impl Into<String>,
    r: &Response,
) {
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

// Text previews are bounded; binary payloads use a small byte sample below.
// even when the reviewed source contains megabytes of text.
fn excerpt(bytes: &[u8], characters: usize) -> (String, bool) {
    let prefix = &bytes[..bytes.len().min(characters * 4)];
    let valid = match std::str::from_utf8(prefix) {
        Ok(text) => text,
        Err(error) => std::str::from_utf8(&prefix[..error.valid_up_to()]).unwrap(),
    };
    let text: String = valid.chars().take(characters).collect();
    let limited = text.len() < bytes.len();
    (text, limited)
}
fn file_preview(ui: &mut Ui, title: &str, bytes: Option<&[u8]>) {
    pages::caption(ui, title);
    let Some(bytes) = bytes else {
        ui.label(RichText::new(crate::i18n::tr("File absent")).color(MUTED()));
        return;
    };
    pages::note(ui, &crate::i18n::trf!("{bytes} bytes", bytes = bytes.len()));
    if bytes.contains(&0) || std::str::from_utf8(bytes).is_err() {
        pages::note(ui,crate::i18n::tr("Binary data · exact bytes are preserved"));
        ui.label(RichText::new(bytes.iter().take(24).map(|b|format!("{b:02x}")).collect::<Vec<_>>().join(" ")).monospace());
        return;
    }
    let (text, limited) = excerpt(bytes, 1800);
    pages::well(ui, |ui| {
        // Columns justify labels by default, which strips leading indentation.
        // Code must retain whitespace even in the side-by-side layout.
        ui.with_layout(Layout::top_down(Align::Min), |ui| {
            ui.add(
                Label::new(
                    RichText::new(if text.is_empty() {
                        crate::i18n::tr("Empty file").into()
                    } else {
                        text
                    })
                    .monospace(),
                )
                .wrap(),
            );
        });
    });
    if limited {
        pages::callout(
            ui,
            AMBER(),
            crate::i18n::tr("Showing the first 1,800 characters; the whole file is used."),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_preview_preserves_indentation_inside_columns() {
        let ctx = Context::default();
        // The preview's caption uses the app's semibold family.
        crate::style::install(&ctx);
        let mut output = ctx.run_ui(RawInput::default(), |ui| {
            ui.columns(2, |columns| {
                file_preview(
                    &mut columns[0],
                    "Result",
                    Some(b"fn main() {\n    nested();\n}"),
                );
            });
        });
        output.textures_delta.clear();
        let code = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                Shape::Text(text) if text.galley.job.text.starts_with("fn main()") => {
                    Some(&text.galley)
                }
                _ => None,
            })
            .expect("Code preview must render text");
        let first = code.rows[0]
            .glyphs
            .iter()
            .find(|g| !g.chr.is_whitespace())
            .unwrap();
        let nested = code.rows[1]
            .glyphs
            .iter()
            .find(|g| !g.chr.is_whitespace())
            .unwrap();
        assert!(
            nested.pos.x > first.pos.x + 10.0,
            "Column layout stripped indentation from the code preview"
        );
    }
}

#[cfg(test)]
mod import_tests {
    use super::looks_like_patch;
    use crate::{
        app::{DialogKind, Intent, Rocket},
        batch::flow_tests::{click, open, wait},
        test_common as common,
    };
    use eframe::egui::{Context, DroppedFile, Event, Pos2, RawInput, Rect, vec2};
    use std::{
        path::{Path, PathBuf},
        sync::Arc,
        thread,
        time::Duration,
    };

    #[derive(Debug)]
    struct Dropped(PathBuf);
    impl DroppedFile for Dropped {
        fn path(&self) -> &Path {
            &self.0
        }
        fn bytes(&self) -> Result<Vec<u8>, String> {
            std::fs::read(&self.0).map_err(|e| e.to_string())
        }
    }

    /// One frame with window-level input, handled as the app frame does.
    fn input_frame(ctx: &Context, app: &mut Rocket, raw: RawInput) {
        app.frame_widgets.clear();
        let mut output = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1480.0, 1000.0))),
                ..raw
            },
            |ui| {
                let mut intents = Vec::new();
                app.dropped_or_pasted(&ui.ctx().clone(), &mut intents);
                app.shell(ui, &mut intents);
                for intent in intents {
                    app.dispatch(intent);
                }
                app.dialog_ui(&ui.ctx().clone());
            },
        );
        output.textures_delta.clear();
    }

    /// A one-file patch changing line 2 of a.txt, and the fixture with a.txt unchanged.
    fn fixture() -> (common::Fixture, String) {
        let f = common::Fixture::new();
        f.write("a.txt", "one\ntwo\n");
        f.commit("base");
        f.write("a.txt", "one\n2\n");
        let patch = String::from_utf8(f.run(&["diff", "--no-color"])).unwrap();
        f.run(&["checkout", "--", "a.txt"]);
        (f, patch)
    }

    fn import(app: &Rocket) -> &super::DialogState {
        let d = app.dialog.as_ref().expect("a dialog");
        assert!(matches!(d.kind, DialogKind::Patch(super::Mode::Import)));
        d.patch.as_ref().unwrap()
    }

    /// The stash is saved under the reviewed message, which starts as the file's name.
    #[test]
    fn stash_selection_saves_under_the_reviewed_message() {
        common::isolated(
            "ROCKET_PATCH_IMPORT_UI_CASE",
            "patches::import_tests::stash_selection_saves_under_the_reviewed_message",
            || {
                let f = common::Fixture::new();
                let lines = |second: &str, last: &str| {
                    format!("one\n{second}\nthree\nfour\nfive\nsix\nseven\neight\nnine\n{last}\n")
                };
                f.write("a.txt", lines("two", "ten"));
                f.commit("base");
                f.write("a.txt", lines("2", "10"));
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                let path = rocket_git::RepoPath::new(b"a.txt".to_vec()).unwrap();
                let review = f
                    .repo()
                    .review_change(&path, false, &rocket_git::CancellationToken::new())
                    .unwrap();
                // The first hunk's change: "two" becomes "2".
                let first = review.changed_lines()[..2].to_vec();
                app.dispatch(Intent::Dialog(DialogKind::Patch(super::Mode::Stash(
                    Arc::new(review),
                    first,
                ))));
                wait(&ctx, &mut app);
                let s = app.dialog.as_ref().unwrap().patch.as_ref().unwrap();
                assert_eq!(s.message, "Selected changes in a.txt");
                assert!(s.preview.is_some(), "{:?}", app.dialog.as_ref().unwrap().error);
                app.dialog.as_mut().unwrap().patch.as_mut().unwrap().message = "Cache key 文".into();
                click(&ctx, &mut app, "dialog-patch-review");
                wait(&ctx, &mut app);
                click(&ctx, &mut app, "dialog-submit");
                wait(&ctx, &mut app);
                let list = String::from_utf8(f.run(&["stash", "list", "--format=%s"])).unwrap();
                assert!(list.contains("Cache key 文"), "{list}");
                assert_eq!(
                    std::fs::read_to_string(f.repository.join("a.txt")).unwrap(),
                    lines("two", "10")
                );
            },
        );
    }

    /// Export Commit Series opens on its tab, with the commits a selection chose.
    #[test]
    fn export_series_opens_on_the_series_tab_with_its_range() {
        common::isolated(
            "ROCKET_PATCH_IMPORT_UI_CASE",
            "patches::import_tests::export_series_opens_on_the_series_tab_with_its_range",
            || {
                let (f, _) = fixture();
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                app.dispatch(Intent::ExportSeries(None));
                let s = app.dialog.as_ref().unwrap().patch.as_ref().unwrap();
                assert_eq!((s.source, s.base.as_str(), s.revision.as_str()), (3, "HEAD", "HEAD"));
                app.dialog = None;
                app.dispatch(Intent::ExportSeries(Some(("1111111".into(), "2222222".into()))));
                let s = app.dialog.as_ref().unwrap().patch.as_ref().unwrap();
                assert_eq!((s.base.as_str(), s.revision.as_str()), ("1111111", "2222222"));
            },
        );
    }

    #[test]
    fn patch_text_detection() {
        assert!(looks_like_patch("diff --git a/a b/a\n"));
        assert!(looks_like_patch("note\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n"));
        assert!(!looks_like_patch("--- a/x\nnot a header\n+++ b/x\n"));
        assert!(!looks_like_patch("Just text\n"));
    }

    #[test]
    fn a_dropped_patch_file_opens_its_review_and_applies() {
        common::isolated(
            "ROCKET_PATCH_IMPORT_UI_CASE",
            "patches::import_tests::a_dropped_patch_file_opens_its_review_and_applies",
            || {
                let (f, patch) = fixture();
                let file = f.root.join("change 文.patch");
                std::fs::write(&file, &patch).unwrap();
                // File Explorer drops a plain absolute path; the fixture root has `..` parts.
                let file = PathBuf::from(crate::views::display_path(
                    &std::fs::canonicalize(&file).unwrap(),
                ));
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                let dropped = RawInput {
                    dropped_files: vec![Arc::new(Dropped(file.clone()))],
                    ..Default::default()
                };
                input_frame(&ctx, &mut app, dropped);
                wait(&ctx, &mut app);
                let s = import(&app);
                assert_eq!(s.path, crate::views::display_path(&file));
                assert!(
                    s.text.is_none() && s.preview.is_some(),
                    "{:?}",
                    app.dialog.as_ref().unwrap().error
                );
                click(&ctx, &mut app, "dialog-submit");
                wait(&ctx, &mut app);
                assert_eq!(
                    std::fs::read_to_string(f.repository.join("a.txt")).unwrap(),
                    "one\n2\n"
                );
            },
        );
    }

    #[test]
    fn pasted_patch_text_opens_its_review_and_other_text_does_not() {
        common::isolated(
            "ROCKET_PATCH_IMPORT_UI_CASE",
            "patches::import_tests::pasted_patch_text_opens_its_review_and_other_text_does_not",
            || {
                let (f, patch) = fixture();
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                let paste = |text: &str| RawInput {
                    events: vec![Event::Paste(text.into())],
                    ..Default::default()
                };
                // Ordinary text: a notice, no dialog.
                input_frame(&ctx, &mut app, paste("Hello there"));
                assert!(app.dialog.is_none());
                assert!(app.notice.as_deref().unwrap().contains("not a patch"));
                // Ctrl+V with a patch: the review of the pasted text.
                input_frame(&ctx, &mut app, paste(&patch));
                wait(&ctx, &mut app);
                let s = import(&app);
                assert_eq!(s.text.as_deref().map(Vec::as_slice), Some(patch.as_bytes()));
                assert!(s.path.is_empty() && s.preview.is_some());
                // While the dialog is open, another paste is left to its fields.
                input_frame(&ctx, &mut app, paste(&patch));
                assert_eq!(
                    import(&app).text.as_deref().map(Vec::len),
                    Some(patch.len())
                );
                app.dialog = None;
                // The menu's request: the next paste is used; with none it expires.
                app.dispatch(Intent::PastePatch);
                assert!(app.busy());
                input_frame(&ctx, &mut app, paste(&patch));
                wait(&ctx, &mut app);
                assert!(import(&app).preview.is_some());
                app.dialog = None;
                app.dispatch(Intent::PastePatch);
                thread::sleep(Duration::from_millis(2100));
                input_frame(&ctx, &mut app, RawInput::default());
                assert!(app.paste_patch.is_none() && !app.busy());
                assert!(app.notice.as_deref().unwrap().contains("no text"));
            },
        );
    }
}
