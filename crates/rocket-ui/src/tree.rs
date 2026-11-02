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
}
pub struct Content {
    pub kind: HistoricalFileKind,
    pub bytes: usize,
    pub lines: Vec<String>,
    pub limited: bool,
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
                        s.notice = Some("Folder read cancelled. Previous results retained.".into())
                    }
                    Err(e) => {
                        s.error = Some(format!(
                            "{} Previous results retained.",
                            tree_error(&e, false)
                        ))
                    }
                }
            }
            if let Some(result) = s.content_task.as_mut().and_then(ReadTask::try_finish) {
                s.content_task = None;
                match result {
                    Ok(content) => s.content = Some(content),
                    Err(Error::Cancelled) => {
                        s.notice = Some("File read cancelled. Retry or select another file.".into())
                    }
                    Err(e) => s.error = Some(tree_error(&e, true)),
                }
            }
            if s.running() {
                ctx.request_repaint_after(std::time::Duration::from_millis(30));
            } else {
                s.cancelling = false;
            }
        }
    }
}

fn tree_error(error: &Error, content: bool) -> String {
    match error {
        Error::Git { code, .. } => format!(
            "{} [git_exit_{}]",
            if content {
                "Could not read this file object. It may be missing or unavailable locally. Fetch the needed objects separately, then retry."
            } else {
                "Could not load this revision or folder. Check the commit, branch or tag and that its objects exist locally. An empty repository has no commit to browse."
            },
            code.map_or_else(|| "unknown".into(), |n| n.to_string())
        ),
        _ => user_error(error),
    }
}

fn prepare_content(
    file: HistoricalFile,
    cancel: &CancellationToken,
) -> rocket_git::Result<Content> {
    let mut lines = Vec::new();
    let mut limited = false;
    if file.kind == HistoricalFileKind::Text {
        for line in std::str::from_utf8(&file.bytes)
            .expect("backend validated UTF-8")
            .lines()
        {
            cancel.check()?;
            if lines.len() == 50_000 {
                limited = true;
                break;
            }
            let bytes = crate::text::prefix(line.as_bytes(), 8192);
            let mut display = String::new();
            // Controls must not change row geometry; preserve ordinary Unicode.
            for c in String::from_utf8_lossy(bytes).chars() {
                if c == '\t' {
                    display.push_str("    ");
                } else if c.is_control() {
                    display.extend(c.escape_default());
                } else {
                    display.push(c);
                }
            }
            if bytes.len() < line.len() {
                limited = true;
                display.push_str("… [display limited]");
            }
            lines.push(display);
        }
    }
    Ok(Content {
        kind: file.kind,
        bytes: file.bytes.len(),
        lines,
        limited,
    })
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
        TreeEntryKind::Directory => "Folder",
        TreeEntryKind::File => "File",
        TreeEntryKind::Executable => "Executable file",
        TreeEntryKind::Symlink => "Symbolic link",
        TreeEntryKind::Submodule => "Submodule",
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
    ui.set_clip_rect(ui.clip_rect().intersect(ui.available_rect_before_wrap()));
    let narrow = ui.available_width() < 800.0;
    let short = ui.available_height() < 340.0;
    let s = &mut tab.tree;
    if !short {
        pages::header(
            ui,
            Icon::Folder,
            "Repository files",
            "Read only · files as committed at a commit, branch or tag; the working tree is untouched",
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
                        .hint_text("Commit, branch or tag")
                        .vertical_align(Align::Center),
                );
                register(ui, widgets, "tree-revision", &r);
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                let load = pages::primary(ui, !s.revision.is_empty(), "Browse revision");
                register(ui, widgets, "tree-load", &load);
                if load.clicked() || enter {
                    emit(intents, TreeIntent::Load);
                }
                if button(
                    ui,
                    widgets,
                    "tree-cancel",
                    "Stop",
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
                    "Stopping read…"
                } else if s.listing.is_some() {
                    if s.running() {
                        "read only · loading…"
                    } else {
                        "read only"
                    }
                } else if s.running() {
                    "Loading revision…"
                } else {
                    "Choose a revision to browse its committed files."
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
        });
    pages::divider(ui);
    if narrow {
        if s.show_list || s.listing.is_none() {
            folder_view(ui, s, intents, widgets, narrow, commands, &context);
        } else {
            file_view(ui, s, intents, widgets, short, narrow);
        }
    } else {
        pages::list_pane(
            ui,
            Id::new(("tree-folders", tab.id)),
            285.0,
            190.0..=380.0,
            |ui| folder_view(ui, s, intents, widgets, narrow, commands, &context),
        );
        CentralPanel::default()
            .frame(Frame::new().fill(BG()))
            .show(ui, |ui| file_view(ui, s, intents, widgets, short, narrow));
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
        if button(ui, widgets, "tree-root", "Root", true) {
            emit(intents, TreeIntent::Ancestor(0));
        }
        if button(ui, widgets, "tree-up", "Up", s.trail.len() > 1) {
            emit(intents, TreeIntent::Ancestor(s.trail.len() - 2));
        }
        if button(
            ui,
            widgets,
            "tree-history",
            "Folder history",
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
                    "Show file"
                } else {
                    "Show folders"
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
                pages::empty(ui, "Browse HEAD, a branch, tag or commit ID to begin.");
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
                RichText::new(format!(
                    "{} {} · folders first",
                    list.entries.len(),
                    if list.entries.len() == 1 {
                        "entry"
                    } else {
                        "entries"
                    }
                ))
                .size(11.5)
                .color(FAINT()),
            );
            if list.entries.is_empty() {
                pages::empty(ui, "This folder is empty at the pinned commit.");
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
                    "Select a file to view its contents at the pinned commit.",
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
            if matches!(
                entry.kind(),
                TreeEntryKind::File | TreeEntryKind::Executable
            ) && let Ok(path) = RepoPath::new(entry.path().to_vec())
                && button(
                    ui,
                    widgets,
                    "tree-restore",
                    "Restore this file",
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
                    && button(ui, widgets, "tree-retry", "Retry", !s.running())
                {
                    emit(intents, TreeIntent::Retry);
                }
                let meta = match &s.content {
                    Some(c) if c.kind == HistoricalFileKind::Text => format!(
                        "{} line{} · {}",
                        c.lines.len(),
                        if c.lines.len() == 1 { "" } else { "s" },
                        size(c.bytes)
                    ),
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
                    "This entry points to a submodule commit. Browsing does not open or fetch the submodule.",
                );
                return false;
            }
            if entry.kind() == TreeEntryKind::Symlink {
                pages::callout(ui, AMBER(), "Link target text only. The link is not followed.");
            }
            let Some(content) = &s.content else {
                return false;
            };
            if content.kind == HistoricalFileKind::Binary {
                pages::note(
                    ui,
                    &format!(
                        "Binary or non-UTF-8 file · {} bytes. Text preview is unavailable.",
                        content.bytes
                    ),
                );
                return false;
            }
            if content.limited {
                pages::callout(
                    ui,
                    AMBER(),
                    "Preview limited to 50,000 lines and 8 KiB per line.",
                );
            }
            if content.lines.is_empty() {
                pages::note(ui, "Empty file at this commit.");
            }
            true
        })
        .inner;
    let Some(content) = s.content.as_ref().filter(|_| notes) else {
        return;
    };
    pages::divider(ui);
    let digits = content.lines.len().max(1).to_string().len() as f32;
    let gutter = (digits * 8.0 + 22.0).max(46.0);
    // Each row is exactly 22 points so the virtualized row range stays accurate.
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.spacing_mut().interact_size.y = 22.0;
    let out = ScrollArea::both()
        .id_salt(("tree-source", entry.commit(), entry.object(), entry.path()))
        .auto_shrink([false, false])
        .min_scrolled_height(0.0)
        .show_rows(ui, 22.0, content.lines.len(), |ui, rows| {
            for i in rows {
                ui.horizontal(|ui| {
                    ui.set_min_height(22.0);
                    ui.spacing_mut().item_spacing.x = 8.0;
                    pages::gutter(ui, &(i + 1).to_string(), gutter, 22.0);
                    ui.add(Label::new(RichText::new(&content.lines[i]).monospace()).extend());
                });
            }
        });
    let rect = out.inner_rect.intersect(ui.clip_rect());
    if rect.is_positive() {
        widgets.insert(
            "tree-source-scroll".into(),
            UiControl {
                rect,
                enabled: true,
            },
        );
    }
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
}
