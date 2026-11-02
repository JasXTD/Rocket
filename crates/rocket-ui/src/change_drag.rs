//! Whole-file staging gestures. Only typed payloads from this repository tab
//! can be dropped; Git mutations still go through the ordinary write task.
use crate::{
    app::{Intent, Tab, UiControl, View},
    backend::Action,
    style::*,
};
use eframe::egui::*;
use rocket_git::{Change, EntryKind, Head, RepoPath, Status, StatusEntry};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub struct Selection {
    pub staged: Option<bool>,
    pub paths: HashSet<RepoPath>,
    anchor: Option<RepoPath>,
    pub notice: Option<String>,
}

pub(crate) fn belongs(entry: &StatusEntry, staged: bool) -> bool {
    !matches!(entry.kind, EntryKind::Unmerged | EntryKind::Ignored)
        && if staged {
            entry.kind != EntryKind::Untracked && entry.index != Change::Unchanged
        } else {
            entry.kind == EntryKind::Untracked || entry.worktree != Change::Unchanged
        }
}

impl Selection {
    pub fn contains(&self, path: &RepoPath, staged: bool) -> bool {
        self.staged == Some(staged) && self.paths.contains(path)
    }
    pub(crate) fn reconcile(&mut self, status: &Status) {
        if self.paths.is_empty() {
            return;
        }
        let eligible: HashSet<_> = status
            .entries
            .iter()
            .filter(|e| self.staged.is_some_and(|staged| belongs(e, staged)))
            .map(|e| &e.path)
            .collect();
        self.paths.retain(|path| eligible.contains(path));
        if self
            .anchor
            .as_ref()
            .is_some_and(|path| !self.paths.contains(path))
        {
            self.anchor = None;
        }
    }
    pub(crate) fn click(
        &mut self,
        path: &RepoPath,
        staged: bool,
        status: &Status,
        modifiers: Modifiers,
    ) {
        self.notice = None;
        if self.staged != Some(staged) {
            self.paths.clear();
            self.anchor = None;
        }
        self.staged = Some(staged);
        let ordered: Vec<_> = status
            .entries
            .iter()
            .filter(|e| belongs(e, staged))
            .map(|e| &e.path)
            .collect();
        let range = self.anchor.as_ref().and_then(|anchor| {
            Some((
                ordered.iter().position(|p| *p == anchor)?,
                ordered.iter().position(|p| *p == path)?,
            ))
        });
        if modifiers.shift
            && let Some((a, b)) = range
        {
            self.paths = ordered[a.min(b)..=a.max(b)]
                .iter()
                .map(|p| (*p).clone())
                .collect();
        } else if modifiers.ctrl || modifiers.command {
            if self.paths.contains(path) {
                self.paths.remove(path);
            } else {
                self.paths.insert(path.clone());
            }
            self.anchor = Some(path.clone());
        } else {
            self.paths = HashSet::from([path.clone()]);
            self.anchor = Some(path.clone());
        }
    }
}

#[derive(Clone)]
struct Payload {
    tab: u64,
    staged: bool,
    head: Head,
    refresh: u64,
    entries: Vec<StatusEntry>,
}

impl Payload {
    fn paths(
        &self,
        tab: &Tab,
        status: &Status,
        target_staged: bool,
        writable: bool,
    ) -> Result<Vec<RepoPath>, &'static str> {
        if !writable || self.tab != tab.id || tab.view != View::Changes {
            return Err("Drop cancelled: this repository is busy or the active view changed.");
        }
        if self.staged == target_staged || self.entries.is_empty() {
            return Err("Drop onto the opposite group to stage or unstage files.");
        }
        let current: HashMap<_, _> = status.entries.iter().map(|e| (&e.path, e)).collect();
        if self.refresh != tab.refresh_count
            || self.head != status.head
            || self.entries.iter().any(|entry| {
                !belongs(entry, self.staged) || current.get(&entry.path).copied() != Some(entry)
            })
        {
            return Err(
                "Repository changes were refreshed during the drag. Nothing was queued; select the files and drag again.",
            );
        }
        Ok(literal_paths(&self.entries))
    }
}

fn literal_paths(entries: &[StatusEntry]) -> Vec<RepoPath> {
    let mut paths = Vec::new();
    let mut seen = HashSet::new();
    for entry in entries {
        // A rename includes the old deletion. A copy leaves its original intact;
        // including that path could also unstage unrelated edits in the original.
        let original = if entry.index == Change::Renamed || entry.worktree == Change::Renamed {
            entry.original_path.as_ref()
        } else {
            None
        };
        for path in std::iter::once(&entry.path).chain(original) {
            if seen.insert(path) {
                paths.push(path.clone());
            }
        }
    }
    paths
}

/// Navigation, a modal or a new write invalidates a gesture, including when the
/// user leaves a tab and returns before releasing the pointer.
pub(crate) fn cancel_on_navigation(ctx: &Context, tab: Option<&Tab>, available: bool) {
    if let Some(payload) = DragAndDrop::payload::<Payload>(ctx)
        && (!available || !tab.is_some_and(|t| t.id == payload.tab && t.view == View::Changes))
    {
        DragAndDrop::clear_payload(ctx);
    }
}

pub(crate) fn source(
    ui: &Ui,
    response: &Response,
    tab: &mut Tab,
    entry: &StatusEntry,
    staged: bool,
    writable: bool,
) {
    if !writable || !belongs(entry, staged) {
        return;
    }
    if response.drag_started() {
        let Some(status) = tab.snapshot.as_ref().and_then(|s| s.status.as_ref()) else {
            return;
        };
        if !tab.change_selection.contains(&entry.path, staged) {
            tab.change_selection
                .click(&entry.path, staged, status, Modifiers::NONE);
        }
        tab.change_selection.notice = None;
        let entries = status
            .entries
            .iter()
            .filter(|e| belongs(e, staged) && tab.change_selection.contains(&e.path, staged))
            .cloned()
            .collect();
        response.dnd_set_drag_payload(Payload {
            tab: tab.id,
            staged,
            head: status.head.clone(),
            refresh: tab.refresh_count,
            entries,
        });
    }
    if response.dragged()
        && let Some(payload) = DragAndDrop::payload::<Payload>(ui.ctx())
        && let Some(pos) = ui.ctx().pointer_latest_pos()
    {
        let label = format!(
            "{} file{} · drop to {}",
            payload.entries.len(),
            if payload.entries.len() == 1 { "" } else { "s" },
            if staged { "unstage" } else { "stage" }
        );
        let painter = ui
            .ctx()
            .layer_painter(LayerId::new(Order::Tooltip, Id::new("stage-drag-label")));
        let galley = painter.layout_no_wrap(label, FontId::proportional(12.0), TEXT());
        let rect = Rect::from_min_size(pos + vec2(16.0, 16.0), galley.size() + vec2(16.0, 12.0));
        painter.rect_filled(rect, 5.0, RAISED());
        painter.rect_stroke(rect, 5.0, Stroke::new(1.0, BLUE()), StrokeKind::Inside);
        painter.galley(rect.min + vec2(8.0, 6.0), galley, TEXT());
    }
}

pub(crate) fn target(
    ui: &Ui,
    rect: Rect,
    tab: &mut Tab,
    status: &Status,
    staged: bool,
    writable: bool,
    widgets: &mut HashMap<String, UiControl>,
) -> Option<Intent> {
    let rect = rect.intersect(ui.clip_rect());
    if !rect.is_positive() {
        return None;
    }
    let name = if staged {
        "change-drop-staged"
    } else {
        "change-drop-unstaged"
    };
    widgets.insert(
        name.into(),
        UiControl {
            rect,
            enabled: writable && ui.is_enabled(),
        },
    );
    let response = ui.interact(rect, Id::new((name, tab.id)), Sense::hover());
    if let Some(payload) = response.dnd_hover_payload::<Payload>()
        && payload.tab == tab.id
        && payload.staged != staged
    {
        let valid = payload.paths(tab, status, staged, writable).is_ok();
        ui.painter().rect_stroke(
            rect,
            5.0,
            Stroke::new(1.5, if valid { BLUE() } else { RED() }),
            StrokeKind::Inside,
        );
        ui.ctx().set_cursor_icon(if valid {
            CursorIcon::Copy
        } else {
            CursorIcon::NotAllowed
        });
    }
    if let Some(payload) = response.dnd_release_payload::<Payload>() {
        // Same-side and foreign payloads are harmless no-ops, never a path list.
        if payload.tab != tab.id || payload.staged == staged {
            return None;
        }
        match payload.paths(tab, status, staged, writable) {
            Ok(paths) => {
                tab.change_selection.notice = None;
                return Some(Intent::Execute(
                    tab.id,
                    if staged {
                        Action::Stage(paths)
                    } else {
                        Action::Unstage(paths)
                    },
                ));
            }
            Err(message) => tab.change_selection.notice = Some(message.into()),
        }
    }
    None
}

pub(crate) fn scroll_edges(ui: &Ui) {
    if !DragAndDrop::has_payload_of_type::<Payload>(ui.ctx()) {
        return;
    }
    let Some(pos) = ui.ctx().pointer_latest_pos() else {
        return;
    };
    let clip = ui.clip_rect();
    if !clip.contains(pos) {
        return;
    }
    let delta = if pos.y < clip.top() + 22.0 {
        10.0
    } else if pos.y > clip.bottom() - 22.0 {
        -10.0
    } else {
        0.0
    };
    if delta != 0.0 {
        ui.scroll_with_delta(vec2(0.0, delta));
        ui.ctx().request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::Rocket,
        batch::flow_tests::{drag, enabled, frame, open, wait},
        test_common as common,
    };

    #[test]
    fn copied_path_keeps_its_original_out_of_the_staging_request() {
        let mut entry = StatusEntry {
            path: RepoPath::new(b"copy.txt".to_vec()).unwrap(),
            original_path: Some(RepoPath::new(b"original.txt".to_vec()).unwrap()),
            kind: EntryKind::Tracked,
            index: Change::Copied,
            worktree: Change::Unchanged,
            submodule: None,
            similarity: Some(100),
        };
        assert_eq!(literal_paths(&[entry.clone()]), vec![entry.path.clone()]);
        entry.index = Change::Renamed;
        assert_eq!(
            literal_paths(&[entry.clone(), entry.clone()]),
            vec![entry.path.clone(), entry.original_path.unwrap()]
        );
    }

    fn key(path: &str, staged: bool) -> String {
        format!(
            "change-{}-{}",
            if staged { "staged" } else { "unstaged" },
            path.as_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        )
    }
    fn changes(ctx: &Context, app: &mut Rocket) {
        app.tabs[0].view = View::Changes;
        frame(ctx, app, vec![]);
        frame(ctx, app, vec![]);
    }
    fn select_with(ctx: &Context, app: &mut Rocket, key: &str, modifiers: Modifiers) {
        assert!(enabled(ctx, app, key));
        let pos = app.frame_widgets[key].rect.center();
        let events = vec![
            Event::ModifiersChanged(modifiers),
            Event::PointerMoved(pos),
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: true,
                modifiers,
            },
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: false,
                modifiers,
            },
        ];
        let mut output = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1480.0, 1000.0))),
                events,
                ..Default::default()
            },
            |ui| {
                app.frame_widgets.clear();
                let mut intents = Vec::new();
                app.shell(ui, &mut intents);
                for intent in intents {
                    app.dispatch(intent);
                }
            },
        );
        output.textures_delta.clear();
        frame(ctx, app, vec![Event::ModifiersChanged(Modifiers::NONE)]);
        wait(ctx, app);
    }
    fn gesture_start(ctx: &Context, app: &mut Rocket, source: &str, target: &str) -> Pos2 {
        assert!(enabled(ctx, app, source));
        let from = app.frame_widgets[source].rect.center();
        let to = app.frame_widgets[target].rect.center();
        frame(
            ctx,
            app,
            vec![
                Event::PointerMoved(from),
                Event::PointerButton {
                    pos: from,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::NONE,
                },
            ],
        );
        for step in 1..=6 {
            frame(
                ctx,
                app,
                vec![Event::PointerMoved(
                    from + (to - from) * (step as f32 / 6.0),
                )],
            );
        }
        assert!(
            DragAndDrop::has_payload_of_type::<Payload>(ctx),
            "file drag payload must actually start"
        );
        to
    }
    fn release(ctx: &Context, app: &mut Rocket, pos: Pos2) {
        frame(
            ctx,
            app,
            vec![Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }],
        );
        wait(ctx, app);
    }

    #[test]
    fn unicode_multi_selection_stages_current_files_and_unstages_without_editing_them() {
        common::isolated(
            "ROCKET_CHANGE_DRAG_CASE",
            "change_drag::tests::unicode_multi_selection_stages_current_files_and_unstages_without_editing_them",
            || {
                let f = common::Fixture::new();
                f.write("資料.txt", "base\n");
                f.commit("base");
                f.write("資料.txt", "staged\n");
                f.run(&["add", "--", "資料.txt"]);
                f.write("資料.txt", "working\n");
                f.write("新增 é.txt", "untracked\n");
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                changes(&ctx, &mut app);
                select_with(&ctx, &mut app, &key("資料.txt", false), Modifiers::NONE);
                select_with(&ctx, &mut app, &key("新增 é.txt", false), Modifiers::CTRL);
                assert_eq!(app.tabs[0].change_selection.paths.len(), 2);
                drag(
                    &ctx,
                    &mut app,
                    &key("資料.txt", false),
                    "change-drop-staged",
                );
                wait(&ctx, &mut app);
                assert!(app.activities.last().unwrap().outcome.command_succeeded);
                assert_eq!(f.run(&["show", ":資料.txt"]), b"working\n");
                assert_eq!(f.run(&["show", ":新增 é.txt"]), b"untracked\n");
                select_with(&ctx, &mut app, &key("資料.txt", true), Modifiers::NONE);
                select_with(&ctx, &mut app, &key("新增 é.txt", true), Modifiers::CTRL);
                drag(
                    &ctx,
                    &mut app,
                    &key("資料.txt", true),
                    "change-drop-unstaged",
                );
                wait(&ctx, &mut app);
                assert!(f.run(&["diff", "--cached", "--name-only"]).is_empty());
                assert_eq!(
                    std::fs::read(f.repository.join("資料.txt")).unwrap(),
                    b"working\n"
                );
                assert_eq!(
                    std::fs::read(f.repository.join("新增 é.txt")).unwrap(),
                    b"untracked\n"
                );
            },
        );
    }

    #[test]
    fn renamed_file_unstages_both_paths_and_keeps_working_rename() {
        common::isolated(
            "ROCKET_CHANGE_DRAG_CASE",
            "change_drag::tests::renamed_file_unstages_both_paths_and_keeps_working_rename",
            || {
                let f = common::Fixture::new();
                f.write("before.txt", "content\n");
                f.commit("base");
                f.run(&["mv", "--", "before.txt", "之後.txt"]);
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                changes(&ctx, &mut app);
                drag(
                    &ctx,
                    &mut app,
                    &key("之後.txt", true),
                    "change-drop-unstaged",
                );
                wait(&ctx, &mut app);
                assert!(app.activities.last().unwrap().outcome.command_succeeded);
                assert!(f.run(&["diff", "--cached", "--name-only"]).is_empty());
                assert!(!f.repository.join("before.txt").exists());
                assert_eq!(
                    std::fs::read(f.repository.join("之後.txt")).unwrap(),
                    b"content\n"
                );
            },
        );
    }

    #[test]
    fn escape_same_group_navigation_and_refreshed_drag_do_not_write() {
        common::isolated(
            "ROCKET_CHANGE_DRAG_CASE",
            "change_drag::tests::escape_same_group_navigation_and_refreshed_drag_do_not_write",
            || {
                let f = common::Fixture::new();
                f.write("a.txt", "base\n");
                f.commit("base");
                f.write("a.txt", "working\n");
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                changes(&ctx, &mut app);
                let count = app.activities.len();
                let end = gesture_start(&ctx, &mut app, &key("a.txt", false), "change-drop-staged");
                frame(
                    &ctx,
                    &mut app,
                    vec![Event::Key {
                        key: Key::Escape,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Modifiers::NONE,
                    }],
                );
                release(&ctx, &mut app, end);
                assert_eq!(app.activities.len(), count);
                drag(&ctx, &mut app, &key("a.txt", false), "change-drop-unstaged");
                wait(&ctx, &mut app);
                assert_eq!(app.activities.len(), count);
                let end = gesture_start(&ctx, &mut app, &key("a.txt", false), "change-drop-staged");
                cancel_on_navigation(&ctx, None, true);
                release(&ctx, &mut app, end);
                assert_eq!(app.activities.len(), count);
                let end = gesture_start(&ctx, &mut app, &key("a.txt", false), "change-drop-staged");
                f.write("a.txt", "external edit\n");
                app.dispatch(Intent::Refresh);
                wait(&ctx, &mut app);
                release(&ctx, &mut app, end);
                assert!(
                    app.tabs[0]
                        .change_selection
                        .notice
                        .as_ref()
                        .is_some_and(|s| s.contains("refreshed during the drag"))
                );
                assert_eq!(app.activities.len(), count);
                assert!(f.run(&["diff", "--cached", "--name-only"]).is_empty());
                assert_eq!(
                    std::fs::read(f.repository.join("a.txt")).unwrap(),
                    b"external edit\n"
                );
            },
        );
    }

    #[test]
    fn locked_index_reports_failure_and_preserves_file_and_existing_lock() {
        common::isolated(
            "ROCKET_CHANGE_DRAG_CASE",
            "change_drag::tests::locked_index_reports_failure_and_preserves_file_and_existing_lock",
            || {
                let f = common::Fixture::new();
                f.write("a.txt", "base\n");
                f.commit("base");
                f.write("a.txt", "working\n");
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                changes(&ctx, &mut app);
                let lock = f.repository.join(".git/index.lock");
                std::fs::write(&lock, b"existing lock").unwrap();
                drag(&ctx, &mut app, &key("a.txt", false), "change-drop-staged");
                wait(&ctx, &mut app);
                assert!(!app.activities.last().unwrap().outcome.command_succeeded);
                assert_eq!(std::fs::read(&lock).unwrap(), b"existing lock");
                assert!(f.run(&["diff", "--cached", "--name-only"]).is_empty());
                assert_eq!(
                    std::fs::read(f.repository.join("a.txt")).unwrap(),
                    b"working\n"
                );
            },
        );
    }

    #[test]
    fn shift_selection_is_bounded_to_its_group_and_busy_or_foreign_drop_is_rejected() {
        common::isolated(
            "ROCKET_CHANGE_DRAG_CASE",
            "change_drag::tests::shift_selection_is_bounded_to_its_group_and_busy_or_foreign_drop_is_rejected",
            || {
                let f = common::Fixture::new();
                for p in ["a.txt", "b.txt", "c.txt"] {
                    f.write(p, "base\n");
                }
                f.commit("base");
                for p in ["a.txt", "b.txt", "c.txt"] {
                    f.write(p, "change\n");
                }
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                changes(&ctx, &mut app);
                select_with(&ctx, &mut app, &key("a.txt", false), Modifiers::NONE);
                select_with(&ctx, &mut app, &key("c.txt", false), Modifiers::SHIFT);
                assert_eq!(app.tabs[0].change_selection.paths.len(), 3);
                let end = gesture_start(&ctx, &mut app, &key("b.txt", false), "change-drop-staged");
                let payload = DragAndDrop::payload::<Payload>(&ctx).unwrap();
                let tab = &app.tabs[0];
                let status = tab.snapshot.as_ref().unwrap().status.as_ref().unwrap();
                assert!(payload.paths(tab, status, true, false).is_err());
                let mut foreign = (*payload).clone();
                foreign.tab += 1;
                assert!(foreign.paths(tab, status, true, true).is_err());
                cancel_on_navigation(&ctx, app.tab(), false);
                release(&ctx, &mut app, end);
                assert!(f.run(&["diff", "--cached", "--name-only"]).is_empty());
            },
        );
    }
}
