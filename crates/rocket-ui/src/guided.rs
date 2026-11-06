//! Guided entry points and a summary derived only from the existing snapshot.
use crate::{
    app::*,
    backend::{Action, user_error},
    pages::{self, Part},
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use std::{collections::HashMap, sync::Arc};

#[derive(Default)]
pub struct Setup {
    pub open: bool,
    owner: Option<(u64, u64)>,
    task: Option<ReadTask<RepositoryIdentity>>,
    pub identity: Option<RepositoryIdentity>,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn summary_says_refreshing_only_while_a_refresh_is_queued_or_running() {
        let mut tab = Tab::new("synthetic".into(), None);
        assert!(!refreshing(&tab));
        // A file change queues an automatic refresh, which then runs in the background.
        tab.refresh_pending = Some(std::time::Instant::now());
        assert!(refreshing(&tab));
        tab.refresh_pending = None;
        tab.background_refresh = true;
        tab.refresh = Some(crate::read_task::spawn(&Context::default(), |_| Err(Error::Cancelled)).unwrap());
        assert!(refreshing(&tab));
        // Finished: the tab still remembers that the refresh ran in the background.
        tab.refresh = None;
        assert!(!refreshing(&tab), "Summary kept saying Refreshing after an automatic refresh finished");
    }
    #[test]
    fn undo_subject_preview_is_bounded_without_changing_utf8() {
        assert_eq!(preview_subject("commit 中文 🚀".as_bytes()), "commit 中文 🚀");
        let shown = preview_subject("資料🚀".repeat(500).as_bytes());
        assert!(shown.ends_with("… [preview shortened]"));
        assert!(shown.chars().count() < 350);
        assert!(!shown.contains('\u{fffd}'));
    }
    fn status() -> Status {
        Status {
            head: Head::Unborn {
                branch: b"main".to_vec(),
            },
            upstream: None,
            ahead_behind: None,
            entries: vec![],
        }
    }
    fn entry(kind: EntryKind, index: Change, worktree: Change) -> StatusEntry {
        StatusEntry {
            path: RepoPath::new(b"notes".to_vec()).unwrap(),
            original_path: None,
            kind,
            index,
            worktree,
            submodule: None,
            similarity: None,
        }
    }
    #[test]
    fn summary_distinguishes_staging_working_files_and_untracked() {
        let mut s = status();
        s.entries = vec![
            entry(EntryKind::Tracked, Change::Modified, Change::Modified),
            entry(EntryKind::Tracked, Change::Deleted, Change::Unchanged),
            entry(EntryKind::Untracked, Change::Unchanged, Change::Unchanged),
        ];
        let summary = summarize(&s, Some(&OperationState::default()));
        assert_eq!(
            (
                summary.staged,
                summary.unstaged,
                summary.untracked,
                summary.conflicts
            ),
            (2, 1, 1, 0)
        );
        assert!(summary.next.contains("Review staged"));
    }
    #[test]
    fn summary_prioritizes_unknown_state_conflicts_and_recovery() {
        let mut s = status();
        assert!(summarize(&s, None).next.contains("unavailable"));
        s.entries.push(entry(
            EntryKind::Unmerged,
            Change::Unmerged,
            Change::Unmerged,
        ));
        let summary = summarize(&s, Some(&OperationState::default()));
        assert_eq!(summary.staged, 0);
        assert_eq!(summary.conflicts, 1);
        assert!(summary.next.contains("Resolve conflicts"));
        s.entries.clear();
        let op = OperationState {
            rebase: true,
            ..Default::default()
        };
        assert!(summarize(&s, Some(&op)).next.contains("in progress"));
        assert!(
            summarize(&s, Some(&OperationState::default()))
                .next
                .contains("first local commit")
        );
    }
    #[test]
    fn summary_never_calls_missing_remote_comparison_up_to_date() {
        let mut s = status();
        assert_eq!(summarize(&s, None).remote, "No upstream configured");
        s.upstream = Some(b"origin/main".to_vec());
        assert_eq!(
            summarize(&s, None).remote,
            "Upstream comparison unavailable"
        );
        s.ahead_behind = Some((2, 3));
        assert_eq!(summarize(&s, None).remote, "2 outgoing · 3 incoming");
    }
}
impl Setup {
    pub fn running(&self) -> bool {
        self.task.is_some()
    }
    pub(crate) fn close(&mut self) {
        self.open = false;
        self.task = None;
    }
}
#[derive(Default)]
pub struct UndoState {
    pub task: Option<ReadTask<UndoCommitPreview>>,
    pub preview: Option<Arc<UndoCommitPreview>>,
    pub applying: bool,
    /// When the shown review arrived: Git refuses one older than two minutes.
    pub checked: Option<std::time::Instant>,
}
/// How old a review may get before the dialog checks again by itself, under Git's limit.
const UNDO_RECHECK: std::time::Duration = std::time::Duration::from_secs(105);
#[derive(Debug, PartialEq, Eq)]
pub struct Summary {
    pub staged: usize,
    pub unstaged: usize,
    pub untracked: usize,
    pub conflicts: usize,
    pub remote: String,
    pub next: &'static str,
}
fn preview_subject(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut chars = text.chars();
    let mut shown: String = chars.by_ref().take(320).collect();
    if chars.next().is_some() { shown.push_str("… [preview shortened]"); }
    shown
}
/// A commit as a card: a dot (a ring for the one being undone), its title and details,
/// and an optional tag.
fn commit_card(ui: &mut Ui, undone: bool, title: &str, meta: &str, tag: Option<&str>) {
    let (fill, line) = if undone { (AMBER().gamma_multiply(0.06), AMBER().gamma_multiply(0.45)) } else { (DEEP(), CARD_LINE()) };
    Frame::new().fill(fill).stroke(Stroke::new(1.0, line)).corner_radius(8).inner_margin(Margin::symmetric(14, 9)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            let (rect, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
            if undone {
                ui.painter().circle_stroke(rect.center(), 4.5, Stroke::new(2.0, AMBER()));
            } else {
                ui.painter().circle_filled(rect.center(), 5.0, BLUE());
            }
            let tag_width = tag.map_or(0.0, |t| ui.painter().layout_no_wrap(t.to_owned(), semibold(11.5), AMBER()).size().x + 28.0);
            ui.allocate_ui_with_layout(vec2((ui.available_width() - tag_width).max(60.0), 36.0), Layout::top_down(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.add(Label::new(RichText::new(title).font(semibold(13.0))).truncate());
                ui.add(Label::new(RichText::new(meta).size(12.0).color(HINT())).truncate());
            });
            if let Some(tag) = tag {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    pages::pill(ui, tag, AMBER());
                });
            }
        });
    });
}
/// A check mark and what has been checked.
fn check_line(ui: &mut Ui, text: &str) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
        icon(ui.painter(), rect, Icon::Check, GREEN());
        ui.label(text);
    });
}
pub fn summarize(status: &Status, state: Option<&OperationState>) -> Summary {
    let staged = status
        .entries
        .iter()
        .filter(|e| e.kind != EntryKind::Unmerged && e.staged())
        .count();
    let unstaged = status
        .entries
        .iter()
        .filter(|e| {
            e.kind != EntryKind::Untracked
                && e.kind != EntryKind::Ignored
                && e.worktree != Change::Unchanged
        })
        .count();
    let untracked = status
        .entries
        .iter()
        .filter(|e| e.kind == EntryKind::Untracked)
        .count();
    let conflicts = status
        .entries
        .iter()
        .filter(|e| e.kind == EntryKind::Unmerged)
        .count();
    let remote = match (&status.upstream, status.ahead_behind) {
        (None, _) => "No upstream configured".into(),
        (Some(_), None) => "Upstream comparison unavailable".into(),
        (Some(_), Some((a, b))) => format!("{a} outgoing · {b} incoming"),
    };
    let next = if state.is_none() {
        "Operation state is unavailable. Refresh before choosing a write."
    } else if conflicts > 0 {
        "Resolve conflicts in Changes, then review Continue or Abort."
    } else if state.is_some_and(|s| !s.is_idle()) {
        "A Git operation is in progress. Open Changes to review its recovery controls."
    } else if matches!(status.head, Head::Detached { .. }) {
        "You are viewing a detached commit. Create or switch to a branch before your next commit."
    } else if staged > 0 {
        "Review staged files, write a message and commit. This saves changes locally."
    } else if unstaged + untracked > 0 {
        "Review the changed files and stage only what belongs in your next commit."
    } else if matches!(status.head, Head::Unborn { .. }) {
        "Add a file in your editor, then review and stage it for your first local commit."
    } else if status.ahead_behind.is_some_and(|(_, b)| b > 0) {
        "Fetch to check the remote, then review Pull and choose how to integrate incoming commits."
    } else if status.ahead_behind.is_some_and(|(a, _)| a > 0) {
        "Your local commits are ready for review. Fetch before deciding to push."
    } else {
        "Your working copy is clean. Fetch when you want to check remote changes."
    };
    Summary {
        staged,
        unstaged,
        untracked,
        conflicts,
        remote,
        next,
    }
}
fn button(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    id: &str,
    label: &str,
    enabled: bool,
) -> bool {
    let r = ui.add_enabled(enabled, Button::new(label));
    pages::control(widgets, id, &r, ui.clip_rect());
    r.clicked()
}
/// One count on the Summary: the number large, what it counts below. Opens Changes.
fn tile(ui: &mut Ui, width: f32, count: usize, label: &str, color: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width, 72.0), Sense::click());
    let hot = response.hovered() || response.has_focus();
    ui.painter().rect_filled(rect, 8.0, if hot { CARD() } else { PANEL() });
    let stroke = if count > 0 && color != TEXT() { color.gamma_multiply(0.5) } else { CARD_STROKE() };
    ui.painter().rect_stroke(rect, 8.0, Stroke::new(1.0, stroke), StrokeKind::Inside);
    let ink = if count == 0 { FAINT() } else { color };
    ui.painter().text(rect.left_top() + vec2(16.0, 12.0), Align2::LEFT_TOP, count.to_string(), semibold(26.0), ink);
    ui.painter().text(rect.left_bottom() + vec2(16.0, -12.0), Align2::LEFT_BOTTOM, label, FontId::proportional(12.0), HINT());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, format!("{count} {label}")));
    response
}
/// A refresh is queued or running, so Summary's numbers may be out of date.
/// `background_refresh` can't tell: it says how the latest refresh started and stays
/// set after that refresh finishes.
pub fn refreshing(tab: &Tab) -> bool {
    tab.refresh.is_some() || tab.refresh_pending.is_some()
}
pub fn summary(
    ui: &mut Ui,
    tab: &Tab,
    idle: bool,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
) {
    let body = ScrollArea::vertical().id_salt("summary-scroll").show(ui, |ui| {
        let branch = tab.snapshot.as_ref().and_then(|s| s.status.as_ref()).map(|status| match &status.head {
            Head::Branch { name, .. } => display(name),
            Head::Unborn { branch } => format!("{} · no commits yet", display(branch)),
            Head::Detached { target } => format!("detached at {}", &target.as_str()[..9]),
        });
        let detail = match &branch {
            Some(branch) => format!("{} · {branch}", tab.title),
            None => tab.title.clone(),
        };
        pages::header(ui, Icon::Folder, "Summary", &detail, |ui| {
            if button(ui, widgets, "summary-setup", "Setup Guide", idle) { intents.push(Intent::GuidedSetup); }
            if button(ui, widgets, "summary-refresh", "Refresh", idle) { intents.push(Intent::Refresh); }
        });
        Frame::new().inner_margin(Margin { left: 24, right: 24, top: 4, bottom: 24 }).show(ui, |ui| {
            ui.set_width(ui.available_width().min(960.0));
            ui.spacing_mut().item_spacing.y = 12.0;
            if refreshing(tab) {
                pages::busy_line(ui, "Refreshing; the numbers below may be out of date.");
            }
            let Some(snapshot) = &tab.snapshot else { pages::callout(ui, AMBER(), "Repository information is unavailable."); return; };
            let Some(status) = &snapshot.status else {
                pages::callout(ui, AMBER(), if snapshot.info.worktree.is_none() { "This is a bare repository; it has no working files or staging view." } else { "Status is unavailable. Refresh to retry; missing data is not a clean checkout." });
                return;
            };
            let s = summarize(status, snapshot.state.as_ref());
            // The counts, each opening Changes. A file can count as staged and unstaged.
            let width = ((ui.available_width() - 36.0) / 4.0).max(90.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(12.0, 12.0);
                for (key, count, label, color) in [
                    ("summary-tile-staged", s.staged, "Staged", BLUE()),
                    ("summary-tile-unstaged", s.unstaged, "Unstaged", TEXT()),
                    ("summary-tile-untracked", s.untracked, "Untracked", TEXT()),
                    ("summary-tile-conflicts", s.conflicts, "Conflicts", RED()),
                ] {
                    let r = tile(ui, width, count, label, color).on_hover_text("Open Changes");
                    pages::control(widgets, key, &r, ui.clip_rect());
                    if r.clicked() && idle {
                        intents.push(Intent::View(View::Changes));
                    }
                }
            });
            // The next step, with the action that takes it.
            Frame::new()
                .fill(CARD())
                .stroke(Stroke::new(1.0, EDGE_HOT()))
                .corner_radius(8)
                .inner_margin(Margin::symmetric(16, 14))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 6.0;
                    pages::caption(ui, "Next step");
                    ui.add(Label::new(RichText::new(s.next).size(14.0)).wrap());
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let r = pages::primary(ui, idle, "Review changes");
                        pages::control(widgets, "summary-changes", &r, ui.clip_rect());
                        if r.clicked() { intents.push(Intent::View(View::Changes)); }
                        if button(ui, widgets, "summary-fetch", "Fetch", idle && !snapshot.remotes.is_empty() && !tab.loading()) { intents.push(Intent::Dialog(DialogKind::Fetch)); }
                        let r = pages::ghost(ui, idle && tab.undo_commit.is_some() && !tab.loading(), "Undo last commit");
                        pages::control(widgets, "summary-undo", &r, ui.clip_rect());
                        if r.clicked() { intents.push(Intent::Dialog(DialogKind::UndoCommit)); }
                    });
                });
            // The branch and how it compares with its upstream, as of the last fetch.
            Frame::new()
                .fill(PANEL())
                .stroke(Stroke::new(1.0, CARD_STROKE()))
                .corner_radius(8)
                .inner_margin(Margin::symmetric(16, 12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        let (rect, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                        icon(ui.painter(), rect, Icon::Branch, MUTED());
                        ui.label(RichText::new(branch.as_deref().unwrap_or("")).strong());
                        let color = if status.upstream.is_none() { AMBER() } else { MUTED() };
                        pages::pill(ui, &s.remote, color);
                        if let Some(upstream) = &status.upstream {
                            ui.label(RichText::new(display(upstream)).size(12.5).color(HINT()));
                        }
                    });
                    pages::note(ui, "Compared with your last fetch, not the live server.");
                });
        });
    });
    pages::region(
        widgets,
        "summary-scroll",
        body.inner_rect.intersect(ui.clip_rect()),
        true,
    );
}
impl Rocket {
    pub(crate) fn poll_guided(&mut self, ctx: &Context) {
        if self.setup.open && self.dialog.is_none() && self.write.is_none() && !self.quitting {
            // Read only after writes settle; otherwise saving identity can cache
            // its old values. A completed repository refresh invalidates this
            // small read too, including edits made in another application.
            let owner = self.tab().map(|t| (t.id, t.refresh_count));
            if owner != self.setup.owner {
                self.setup.owner = owner;
                self.setup.identity = None;
                self.setup.task = None;
                self.setup.error = None;
            }
            if self.setup.identity.is_none()
                && self.setup.task.is_none()
                && self.setup.error.is_none()
                && let (Some(tab), Some(backend)) = (self.tab(), self.backend.clone())
                && tab.snapshot.is_some()
                && !tab.loading()
            {
                let path = tab.path.clone();
                match ReadTask::spawn(move |c| backend.identity(&path, &c)) {
                    Ok(task) => self.setup.task = Some(task),
                    Err(e) => self.setup.error = Some(user_error(&e)),
                }
            }
            if let Some(result) = self.setup.task.as_mut().and_then(ReadTask::try_finish) {
                self.setup.task = None;
                match result {
                    Ok(identity) => self.setup.identity = Some(identity),
                    Err(e) => self.setup.error = Some(user_error(&e)),
                }
            }
        } else {
            self.setup.task = None;
        }
        if let Some(d) = &mut self.dialog
            && let Some(s) = &mut d.undo
        {
            if let Some(result) = s.task.as_mut().and_then(ReadTask::try_finish) {
                s.task = None;
                match result {
                    Ok(p) => {
                        s.preview = Some(Arc::new(p));
                        s.checked = Some(std::time::Instant::now());
                        d.error = None;
                    }
                    Err(e) => {
                        // A failed re-check leaves nothing to undo.
                        s.preview = None;
                        d.error = Some(user_error(&e));
                    }
                }
            }
            if s.task.is_some() {
                ctx.request_repaint_after(std::time::Duration::from_millis(40));
            } else if let Some(checked) = s.checked.filter(|_| s.preview.is_some()) {
                // Wake to check again before Git's limit, even with no input.
                ctx.request_repaint_after(UNDO_RECHECK.saturating_sub(checked.elapsed()));
            }
        }
        if self.setup.task.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(40));
        }
    }
    pub(crate) fn setup_view(&mut self, ui: &mut Ui, intents: &mut Vec<Intent>) {
        let repo = self.tab().is_some_and(|t| t.worktree() && !t.loading());
        let idle = self.write.is_none();
        let identity = self
            .setup
            .identity
            .as_ref()
            .is_some_and(|i| i.effective.name.is_some() && i.effective.email.is_some());
        // Which steps are done; the first that is not is the current one.
        let done = [self.backend.is_some(), !self.tabs.is_empty(), identity, false];
        let current = done.iter().position(|d| !d).unwrap_or(3);
        let body = ScrollArea::vertical().id_salt("setup-scroll").show(ui, |ui| {
            let mut close = false;
            pages::header(ui, Icon::Check, "Setup Guide", "Your first commit, one step at a time.", |ui| {
                close = button(ui, &mut self.frame_widgets, "setup-close", "Close guide", true);
            });
            if close {
                self.setup.open = false;
                self.setup.task = None;
            }
            Frame::new().inner_margin(Margin { left: 24, right: 24, top: 4, bottom: 24 }).show(ui, |ui| {
                ui.set_width(ui.available_width().min(760.0));
                ui.spacing_mut().item_spacing.y = 0.0;
                // Progress across the three steps that can be checked.
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    let count = done.iter().filter(|d| **d).count();
                    let width = (ui.available_width() - 90.0).max(80.0);
                    let (bar, _) = ui.allocate_exact_size(vec2(width, 6.0), Sense::hover());
                    ui.painter().rect_filled(bar, 3.0, RAISED());
                    ui.painter().rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * count as f32 / 4.0, 6.0)), 3.0, BLUE());
                    ui.label(RichText::new(format!("{count} of 4 done")).size(12.0).color(HINT()));
                });
                ui.add_space(18.0);
                let step = |ui: &mut Ui, n: usize, title: &str, content: &mut dyn FnMut(&mut Ui)| {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 16.0;
                        // The step's mark: a check when done, its number otherwise.
                        let (mark, _) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::hover());
                        let center = mark.center();
                        if done[n] {
                            ui.painter().circle_filled(center, 16.0, GREEN().gamma_multiply(0.16));
                            icon(ui.painter(), Rect::from_center_size(center, vec2(14.0, 14.0)), Icon::Check, GREEN());
                        } else if n == current {
                            ui.painter().circle_filled(center, 16.0, ACCENT_SOFT());
                            ui.painter().circle_stroke(center, 15.0, Stroke::new(2.0, BLUE()));
                            ui.painter().text(center, Align2::CENTER_CENTER, (n + 1).to_string(), semibold(13.0), BLUE());
                        } else {
                            ui.painter().circle_stroke(center, 15.0, Stroke::new(2.0, BORDER()));
                            ui.painter().text(center, Align2::CENTER_CENTER, (n + 1).to_string(), semibold(13.0), FAINT());
                        }
                        let fill = if n == current { CARD() } else { Color32::TRANSPARENT };
                        let stroke = if n == current { EDGE_HOT() } else { Color32::TRANSPARENT };
                        Frame::new()
                            .fill(fill)
                            .stroke(Stroke::new(1.0, stroke))
                            .corner_radius(8)
                            .inner_margin(if n == current { Margin::symmetric(14, 12) } else { Margin { left: 0, right: 0, top: 6, bottom: 6 } })
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                // The row lays out across; the step itself reads top to bottom.
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 6.0;
                                    let ink = if n > current { MUTED() } else { TEXT() };
                                    ui.label(RichText::new(title).font(semibold(14.0)).color(ink));
                                    content(ui);
                                });
                            });
                    });
                    ui.add_space(14.0);
                };
                let widgets = &mut self.frame_widgets;
                let setup = &mut self.setup;
                let version = self.backend.as_ref().map(|b| b.version.clone());
                let startup_error = self.startup_error.clone();
                let title = self.tabs.get(self.active).map(|t| t.title.clone());
                let backend = self.backend.is_some();
                step(ui, 0, "Git is ready", &mut |ui| {
                    if let Some(version) = &version { ui.label(RichText::new(format!("{version} found")).size(12.5).color(MUTED())); }
                    else if let Some(error) = &startup_error { pages::callout(ui, RED(), error); }
                    else { pages::busy_line(ui, "Finding Git"); }
                    let r = if current == 0 { pages::primary(ui, idle, "Git preferences") } else { pages::ghost(ui, idle, "Git preferences") };
                    pages::control(widgets, "setup-git", &r, ui.clip_rect());
                    if r.clicked() { intents.push(Intent::Preferences(crate::preferences::Page::Git)); }
                });
                step(ui, 1, "Choose a repository", &mut |ui| {
                    match &title {
                        Some(title) => ui.label(RichText::new(title).size(12.5).color(MUTED())),
                        None => ui.label(RichText::new("Open a checkout, start a new repository, or clone one.").size(12.5).color(MUTED())),
                    };
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        for (i, (id, name, kind)) in [("setup-open", "Open repository", DialogKind::Open), ("setup-new", "New repository", DialogKind::NewRepository), ("setup-clone", "Clone", DialogKind::Clone)].into_iter().enumerate() {
                            let enabled = idle && backend;
                            let r = if current == 1 && i == 0 { pages::primary(ui, enabled, name) } else if current == 1 { ui.add_enabled(enabled, Button::new(name)) } else { pages::ghost(ui, enabled, name) };
                            pages::control(widgets, id, &r, ui.clip_rect());
                            if r.clicked() { intents.push(Intent::Dialog(kind)); }
                        }
                    });
                });
                step(ui, 2, "Check your commit identity", &mut |ui| {
                    ui.label(RichText::new(if identity { "A name and email are set. Review them before committing." } else { "The name and email saved in each commit. Separate from your hosting sign-in." }).size(12.5).color(MUTED()));
                    if let Some(error) = &setup.error { pages::callout(ui, AMBER(), error); }
                    let r = if current == 2 { pages::primary(ui, repo && idle, "Review identity") } else { pages::ghost(ui, repo && idle, "Review identity") };
                    pages::control(widgets, "setup-identity", &r, ui.clip_rect());
                    if r.clicked() {
                        setup.identity = None; setup.error = None;
                        intents.push(Intent::Dialog(DialogKind::RepositorySettings));
                    }
                });
                step(ui, 3, "Stage and commit", &mut |ui| {
                    ui.label(RichText::new("Pick the changes to include in Changes, write a message, then Commit. Nothing is published.").size(12.5).color(MUTED()));
                    let r = if current == 3 { pages::primary(ui, repo && idle, "Open Changes") } else { ui.add_enabled(repo && idle, Button::new("Open Changes")) };
                    pages::control(widgets, "setup-changes", &r, ui.clip_rect());
                    if r.clicked() { setup.open = false; intents.push(Intent::View(View::Changes)); }
                });
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let r = pages::ghost(ui, repo && idle, "Repository summary");
                    pages::control(widgets, "setup-summary", &r, ui.clip_rect());
                    if r.clicked() { setup.open = false; intents.push(Intent::View(View::Summary)); }
                    ui.label(RichText::new("Reopen any time from Help → Setup Guide.").size(12.0).color(HINT()));
                });
            });
        });
        pages::region(
            &mut self.frame_widgets,
            "setup-scroll",
            body.inner_rect.intersect(ui.clip_rect()),
            true,
        );
    }
    pub(crate) fn start_undo_preview(&mut self) {
        if !self
            .dialog
            .as_ref()
            .is_some_and(|d| matches!(d.kind, DialogKind::UndoCommit))
        {
            return;
        }
        let record = self.tab().and_then(|t| t.undo_commit.clone());
        let path = self.tab().map(|t| t.path.clone());
        let backend = self.backend.clone();
        let d = self.dialog.as_mut().unwrap();
        // A re-check keeps showing the commits it checks; Undo waits for its result.
        let shown = d.undo.take().and_then(|s| s.preview);
        d.undo = Some(UndoState { preview: shown, ..Default::default() });
        d.error = None;
        if let (Some(record), Some(path), Some(backend)) = (record, path, backend) {
            match ReadTask::spawn(move |c| backend.preview_undo(&path, &record, &c)) {
                Ok(task) => d.undo.as_mut().unwrap().task = Some(task),
                Err(e) => d.error = Some(user_error(&e)),
            }
        } else {
            d.error = Some("No ordinary Rocket commit is available to undo in this session. Use Reflog & Recovery for earlier work.".into());
        }
    }
    pub(crate) fn undo_dialog(&mut self, ctx: &Context) {
        // Git refuses a review older than two minutes, so an aging one is checked again first.
        if self.dialog.as_ref().and_then(|d| d.undo.as_ref()).is_some_and(|s| {
            s.task.is_none() && !s.applying && s.preview.is_some() && s.checked.is_some_and(|t| t.elapsed() >= UNDO_RECHECK)
        }) {
            self.start_undo_preview();
        }
        // The two commits as cards: titles and times come from the loaded history.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let history = |id: &ObjectId| {
            self.tab()
                .and_then(|t| t.commits().iter().find(|c| c.id == *id))
                .map(|c| (c.subject_lossy().into_owned(), ago(now.saturating_sub(c.author.timestamp.max(0) as u64), c.author.timestamp)))
        };
        let cards = self.dialog.as_ref().and_then(|d| d.undo.as_ref()).and_then(|s| s.preview.as_ref()).map(|p| {
            let r = p.record();
            let short = |id: &ObjectId| id.as_str()[..7].to_owned();
            let meta = |id: &ObjectId, when: Option<&String>| when.map_or_else(|| short(id), |when| format!("{} · {when}", short(id)));
            let undone = (preview_subject(r.subject()), meta(r.commit(), history(r.commit()).as_ref().map(|h| &h.1)));
            let previous = r.previous().map(|id| {
                let found = history(id);
                (found.as_ref().map_or_else(|| format!("Commit {}", short(id)), |h| h.0.clone()), meta(id, found.as_ref().map(|h| &h.1)))
            });
            (display(r.branch()), undone, previous)
        });
        let Some(mut d) = self.dialog.take() else {
            return;
        };
        let s = d.undo.get_or_insert_with(Default::default);
        let mut submit = false;
        let mut retry = false;
        let mut stop = false;
        let running = self.write.is_some();
        let response = pages::review(
            ctx,
            "rocket-undo-commit",
            pages::ReviewHeader {
                image: Icon::Commit,
                title: "Undo last commit",
                subtitle: "Its changes go back to staged. Your working files don't change.",
                danger: false,
            },
            &mut self.frame_widgets,
            "dialog-undo-scroll",
            !running,
            |ui, part, widgets| match part {
                Part::Body => {
                    if let Some(error) = &d.error {
                        pages::callout(ui, RED(), error);
                    }
                    if running {
                        pages::busy_line(
                            ui,
                            self.write.as_ref().map_or("Undo in progress", |w| {
                                if w.cancelling {
                                    "Stopping; checking what completed"
                                } else {
                                    "Undoing the commit and checking the result"
                                }
                            }),
                        );
                    }
                    let (Some(p), Some((branch, undone, previous))) = (&s.preview, &cards) else {
                        if s.task.is_some() {
                            pages::waiting(ui, "Checking the commit, staging area and local changes");
                        }
                        return;
                    };
                    ui.label(RichText::new(format!("On {branch}")).size(12.0).color(HINT()));
                    commit_card(ui, true, &undone.0, &undone.1, Some("Undone"));
                    ui.horizontal(|ui| {
                        ui.add_space(16.0);
                        let (rect, _) = ui.allocate_exact_size(vec2(12.0, 14.0), Sense::hover());
                        let stroke = Stroke::new(1.4, HINT());
                        let (top, bottom) = (pos2(rect.center().x, rect.top() + 1.0), pos2(rect.center().x, rect.bottom() - 1.0));
                        ui.painter().line_segment([top, bottom], stroke);
                        ui.painter().line_segment([bottom, bottom + vec2(-4.0, -4.0)], stroke);
                        ui.painter().line_segment([bottom, bottom + vec2(4.0, -4.0)], stroke);
                        ui.label(RichText::new("The branch moves back to").size(12.0).color(HINT()));
                    });
                    match previous {
                        Some((title, meta)) => commit_card(ui, false, title, meta, None),
                        None => commit_card(ui, false, "Nothing before it", "This was the branch's first commit; afterwards it has none", None),
                    }
                    ui.add_space(4.0);
                    check_line(ui, "Staged, working and untracked files stay as they are");
                    check_line(ui, "Not on any remote branch Rocket tracks");
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                        icon(ui.painter(), rect, Icon::Check, GREEN());
                        ui.label("You can get the commit back from Reflog & Recovery");
                        let r = ui.add(Button::new("Copy recovery name").small().frame(false)).on_hover_text(p.recovery_reference());
                        pages::control(widgets, "dialog-undo-copy-recovery", &r, ui.clip_rect());
                        if r.clicked() {
                            ui.ctx().copy_text(p.recovery_reference().to_owned());
                        }
                    });
                }
                Part::Footer => {
                    if running {
                        if button(
                            ui,
                            widgets,
                            "cancel-operation",
                            self.write
                                .as_ref()
                                .map_or("Stop", ActiveWrite::cancel_label),
                            self.write.as_ref().is_some_and(ActiveWrite::can_cancel),
                        ) {
                            stop = true;
                        }
                    } else if s.preview.is_some() || s.task.is_some() {
                        let r = pages::primary(
                            ui,
                            s.preview.is_some() && s.task.is_none() && d.error.is_none() && !s.applying,
                            "Undo commit",
                        );
                        pages::control(widgets, "dialog-submit", &r, ui.clip_rect());
                        submit = r.clicked();
                        pages::footer_status(
                            ui,
                            s.task.is_some(),
                            if s.task.is_some() { "Checking" } else { "Checked · checks again by itself within 2 minutes" },
                        );
                    } else {
                        // After a refused Undo or a failed check there is nothing to undo
                        // until it is checked again.
                        let r = pages::primary(ui, true, "Check again");
                        pages::control(widgets, "dialog-undo-refresh", &r, ui.clip_rect());
                        retry = r.clicked();
                    }
                }
            },
        );
        let action = if submit {
            s.applying = true;
            s.preview.clone().map(Action::UndoCommit)
        } else {
            None
        };
        let owner = d.owner;
        if !(response.should_close() && !running) {
            self.dialog = Some(d);
        }
        if retry {
            self.start_undo_preview();
        }
        if stop {
            self.cancel_write();
        }
        if let (Some(owner), Some(action)) = (owner, action) {
            self.execute(owner, action);
        }
    }
    pub(crate) fn finish_undo(&mut self, owner: Option<u64>, outcome: &crate::backend::Outcome) {
        if let Some(d) = &mut self.dialog
            && d.owner == owner
            && let Some(s) = &mut d.undo
            && s.applying
        {
            s.applying = false;
            s.preview = None;
            if outcome.command_succeeded {
                self.dialog = None;
            } else {
                d.error = Some(
                    outcome
                        .details
                        .first()
                        .cloned()
                        .unwrap_or_else(|| outcome.summary.clone()),
                );
            }
        }
    }
}
