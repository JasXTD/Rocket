//! Captures the redesigned layout against a caller-provided disposable fixture:
//! wide Commits and Changes, the command palette, the Activity drawer, the sidebar
//! icon rail, a narrow window, the Tags, Worktrees, Submodules and Git LFS pages
//! with a review dialog, then the Branches, Remotes, Stashes, Files at a commit and
//! File history pages, the conflict editor (when a second, conflicted fixture is
//! given), the same views at 780×620 with the drawer open (asserting that no view
//! control runs under it), Activity rows and the no-repository welcome screen. The
//! fixture needs a
//! tag, a stash, an `origin` remote and an unstaged tracked change. Uses an
//! isolated log/state directory; never controls other windows.
mod support;
use eframe::{App, Frame, egui::*};
use rocket_ui::{
    app::{Intent, Rocket, View},
    backend::Action,
    conflicts::ConflictIntent,
    file_history::FileIntent,
    pages::Page,
    stashes::StashIntent,
    tree::TreeIntent,
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

struct Probe {
    app: Rocket,
    output: PathBuf,
    conflict: Option<PathBuf>,
    stage: usize,
    steady: usize,
    capture: Option<&'static str>,
    captures: usize,
    started: Instant,
    pending_events: Vec<Event>,
}
impl Probe {
    fn capture(&mut self, ctx: &Context, name: &'static str) {
        self.capture = Some(name);
        self.captures += 1;
        ctx.send_viewport_cmd(ViewportCommand::Screenshot(UserData::default()));
    }
    fn expect(&self, keys: &[&str]) {
        for key in keys {
            assert!(self.app.frame_widgets.contains_key(*key), "missing {key}");
        }
    }
    /// Every published control whose key starts with `prefix` ends above the drawer.
    fn above_drawer(&self, prefix: &str) {
        let top = self.app.frame_widgets["drawer-activity"].rect.top();
        for (key, control) in &self.app.frame_widgets {
            if key.starts_with(prefix) {
                assert!(
                    control.rect.bottom() <= top + 1.0,
                    "{key} overlaps the drawer: {:?} vs top {top}",
                    control.rect
                );
            }
        }
    }
    fn tab(&mut self) -> &mut rocket_ui::app::Tab {
        let active = self.app.active;
        &mut self.app.tabs[active]
    }
    /// Click a published control through the real pointer path.
    fn click(&mut self, key: &str) {
        let control = self
            .app
            .frame_widgets
            .get(key)
            .copied()
            .unwrap_or_else(|| panic!("missing widget {key}"));
        assert!(control.enabled, "{key} is disabled");
        let pos = control.rect.center();
        self.pending_events.extend([
            Event::PointerMoved(pos),
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            },
        ]);
    }
}
impl App for Probe {
    fn raw_input_hook(&mut self, _ctx: &Context, input: &mut RawInput) {
        input.events.append(&mut self.pending_events);
    }
    fn logic(&mut self, ctx: &Context, frame: &mut Frame) {
        self.app.logic(ctx, frame);
        ctx.request_repaint_after(Duration::from_millis(30));
    }
    fn ui(&mut self, ui: &mut Ui, frame: &mut Frame) {
        assert!(
            self.started.elapsed() < Duration::from_secs(240),
            "layout probe timed out at {}",
            self.stage
        );
        let ctx = ui.ctx().clone();
        self.app.ui(ui, frame);
        for event in ctx.input(|i| i.events.clone()) {
            if let Event::Screenshot { image, .. } = event
                && let Some(name) = self.capture.take()
            {
                support::save_bmp(&self.output.join(format!("{name}.bmp")), &image).unwrap();
            }
        }
        if self.app.busy() || self.capture.is_some() {
            self.steady = 0;
            return;
        }
        self.steady += 1;
        if self.steady < 6 {
            return;
        }
        self.steady = 0;
        match self.stage {
            0 => {
                let tab = self.app.tab().expect("fixture opened");
                assert!(tab.error.is_none(), "{:?}", tab.error);
                let id = tab
                    .commits()
                    .first()
                    .expect("fixture has commits")
                    .id
                    .clone();
                self.app.dispatch(Intent::View(View::Commits));
                self.app.dispatch(Intent::SelectCommit(id));
            }
            1 => {
                for key in [
                    "toolbar-fetch",
                    "toolbar-palette",
                    "refresh",
                    "sidebar-labels",
                    "view-changes",
                    "detail-files",
                ] {
                    assert!(self.app.frame_widgets.contains_key(key), "missing {key}");
                }
                self.capture(&ctx, "01-commits-wide");
            }
            2 => {
                self.app.dispatch(Intent::View(View::Changes));
                let entry = self
                    .app
                    .tab()
                    .unwrap()
                    .snapshot
                    .as_ref()
                    .unwrap()
                    .status
                    .as_ref()
                    .unwrap()
                    .entries
                    .iter()
                    .find(|e| {
                        e.worktree != rocket_git::Change::Unchanged
                            && e.kind != rocket_git::EntryKind::Untracked
                    })
                    .cloned()
                    .expect("fixture has an unstaged tracked change");
                self.app.dispatch(Intent::SelectChange(entry, false));
            }
            3 => {
                for key in [
                    "stage-all",
                    "unstage-all",
                    "commit-message",
                    "commit",
                    "change-discard",
                ] {
                    assert!(self.app.frame_widgets.contains_key(key), "missing {key}");
                }
                self.capture(&ctx, "02-changes-wide");
            }
            4 => self.app.palette.show(),
            5 => {
                assert!(self.app.frame_widgets.contains_key("palette-input"));
                self.capture(&ctx, "03-palette");
            }
            6 => {
                self.app.palette.open = false;
                self.app.show_console = true;
            }
            7 => {
                assert!(self.app.frame_widgets.contains_key("drawer-terminal"));
                self.capture(&ctx, "04-activity-drawer");
            }
            8 => {
                self.app.show_console = false;
                self.app.sidebar_labels = false;
            }
            9 => {
                assert!(self.app.frame_widgets.contains_key("view-lfs"));
                self.capture(&ctx, "05-sidebar-rail");
            }
            10 => {
                self.app.sidebar_labels = true;
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(860.0, 640.0)));
            }
            11 => self.capture(&ctx, "06-narrow-changes"),
            12 => self.app.dispatch(Intent::View(View::Commits)),
            13 => self.capture(&ctx, "07-narrow-commits"),
            14 => {
                // Minimum window with a bottom drawer: the rail must scroll, not run
                // under the pinned labels toggle.
                self.app.show_console = true;
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(780.0, 620.0)));
            }
            15 => {
                let footer = self.app.frame_widgets["sidebar-labels"].rect;
                for (key, control) in &self.app.frame_widgets {
                    if key.starts_with("view-") && control.rect.center().x < footer.right() {
                        assert!(
                            control.rect.bottom() <= footer.top() + 1.0,
                            "{key} overlaps the labels toggle: {:?} vs {footer:?}",
                            control.rect
                        );
                    }
                }
                self.capture(&ctx, "08-short-rail");
            }
            // Management pages: browse on a page, decide in a review dialog.
            16 => {
                self.app.show_console = false;
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(1480.0, 920.0)));
            }
            17 => self.click("view-tags"),
            18 => {
                assert_eq!(self.app.page(), Some(Page::Tags));
                assert!(!self.app.blocking_dialog());
                self.click("dialog-tag-row-0");
            }
            19 => {
                assert!(
                    self.app
                        .dialog
                        .as_ref()
                        .unwrap()
                        .tags
                        .as_ref()
                        .unwrap()
                        .details
                        .is_some()
                );
                self.capture(&ctx, "09-tags-page");
            }
            20 => self.click("dialog-tag-delete"),
            21 => {
                let tags = self.app.dialog.as_ref().unwrap().tags.as_ref().unwrap();
                assert!(tags.preview.is_some(), "delete review loaded");
                assert!(self.app.blocking_dialog());
                // The inert page under a review publishes none of its own controls.
                assert!(!self.app.frame_widgets.contains_key("dialog-tag-new"));
                assert!(self.app.frame_widgets.contains_key("dialog-submit"));
                self.capture(&ctx, "10-tag-delete-review");
            }
            22 => self.click("dialog-cancel"),
            23 => {
                // Cancel returns to the page instead of closing it; nothing was written.
                let tags = self.app.dialog.as_ref().unwrap().tags.as_ref().unwrap();
                assert!(tags.action.is_none());
                assert!(self.app.write.is_none());
                assert_eq!(self.app.page(), Some(Page::Tags));
                self.click("view-worktrees");
            }
            24 => {
                assert_eq!(self.app.page(), Some(Page::Worktrees));
                self.click("dialog-worktree-row-0");
            }
            25 => self.capture(&ctx, "11-worktrees-page"),
            26 => self.click("view-submodules"),
            27 => self.capture(&ctx, "12-submodules-page"),
            28 => self.click("view-lfs"),
            29 => {
                assert_eq!(self.app.page(), Some(Page::Lfs));
                self.capture(&ctx, "13-lfs-page");
            }
            30 => self.click("dialog-lfs-tracking"),
            31 => self.capture(&ctx, "14-lfs-tracking"),
            32 => self.click("view-commits"),
            33 => {
                // Navigation leaves a browsing page.
                assert_eq!(self.app.page(), None);
                assert!(self.app.dialog.is_none());
                assert_eq!(self.app.tab().unwrap().view, View::Commits);
            }
            // Views converted to full-bleed pages.
            34 => self.app.dispatch(Intent::View(View::Branches)),
            35 => {
                assert_eq!(self.app.tab().unwrap().view, View::Branches);
                self.capture(&ctx, "15-branches-page");
            }
            36 => self.app.dispatch(Intent::View(View::Remotes)),
            37 => {
                self.expect(&[
                    "remote-add",
                    "remote-row-0",
                    "remote-fetch-0",
                    "remote-edit-fetch-0",
                    "remote-edit-push-0",
                    "remote-remove-0",
                ]);
                self.capture(&ctx, "16-remotes-page");
            }
            // A real fetch from the fixture's local origin leaves an Activity row.
            38 => self.click("remote-fetch-0"),
            39 => {
                let fetch = self.app.activities.last().expect("fetch recorded");
                assert!(fetch.outcome.command_succeeded, "{:?}", fetch.outcome);
                let entry = self
                    .app
                    .tab()
                    .unwrap()
                    .snapshot
                    .as_ref()
                    .unwrap()
                    .stashes
                    .first()
                    .cloned()
                    .expect("fixture has a stash");
                self.app.dispatch(Intent::View(View::Stashes));
                self.app.dispatch(Intent::Stash(StashIntent::Select(entry)));
            }
            40 => {
                assert!(
                    self.tab().stash.patch.is_some(),
                    "{:?}",
                    self.tab().stash.error
                );
                self.expect(&[
                    "stash-save",
                    "stash-select-0",
                    "stash-apply",
                    "stash-drop",
                    "stash-tracked",
                    "stash-file-picker",
                    "stash-diff-scroll",
                ]);
                self.capture(&ctx, "17-stashes-page");
            }
            41 => self.app.dispatch(Intent::Tree(TreeIntent::OpenSelected)),
            42 => {
                let tree = &self.tab().tree;
                assert!(tree.error.is_none(), "{:?}", tree.error);
                let index = tree
                    .listing
                    .as_ref()
                    .expect("tree listed")
                    .entries
                    .iter()
                    .position(|e| e.name() == b"README.md")
                    .expect("fixture has README.md");
                self.app.dispatch(Intent::Tree(TreeIntent::Enter(index)));
            }
            43 => {
                assert!(
                    self.tab().tree.content.is_some(),
                    "{:?}",
                    self.tab().tree.error
                );
                self.expect(&[
                    "tree-revision",
                    "tree-load",
                    "tree-cancel",
                    "tree-root",
                    "tree-entry-0",
                    "tree-list-scroll",
                    "tree-file-label",
                    "tree-restore",
                    "tree-source-scroll",
                ]);
                self.capture(&ctx, "18-files-at-commit");
            }
            44 => {
                self.tab().file_history.path_input = "README.md".into();
                self.app.dispatch(Intent::View(View::FileHistory));
                self.app.dispatch(Intent::File(FileIntent::Load));
            }
            45 => {
                let history = &self.tab().file_history;
                assert!(history.blame.is_some(), "{:?}", history.error);
                self.expect(&[
                    "file-path",
                    "file-load",
                    "file-cancel",
                    "file-more",
                    "file-diff",
                    "file-contents",
                    "file-blame",
                    "file-list-scroll",
                    "file-source-scroll",
                ]);
                // Pick a blamed line through the pointer to show its origin.
                self.click("file-line-2");
            }
            46 => {
                assert!(self.tab().file_history.origin.is_some());
                self.expect(&["file-origin-history"]);
                self.capture(&ctx, "19-file-history-blame");
            }
            47 => {
                if let Some(conflict) = self.conflict.clone() {
                    self.app.open(conflict);
                }
            }
            48 => {
                if self.conflict.is_some() {
                    let path = self
                        .app
                        .tab()
                        .unwrap()
                        .snapshot
                        .as_ref()
                        .unwrap()
                        .status
                        .as_ref()
                        .unwrap()
                        .entries
                        .iter()
                        .find(|e| e.kind == rocket_git::EntryKind::Unmerged)
                        .expect("conflicted fixture has an unmerged path")
                        .path
                        .clone();
                    self.app
                        .dispatch(Intent::Conflict(ConflictIntent::Open(path)));
                }
            }
            49 => {
                if self.conflict.is_some() {
                    let conflict = &self.tab().conflict;
                    assert!(conflict.preview.is_some(), "{:?}", conflict.error);
                    assert!(!conflict.blocks.blocks.is_empty());
                    self.expect(&[
                        "conflict-back",
                        "conflict-external",
                        "conflict-reload",
                        "conflict-save",
                        "conflict-stage",
                        "conflict-part-0",
                        "conflict-part-2",
                        "conflict-next",
                        "conflict-ours",
                        "conflict-both",
                        "conflict-result",
                        "conflict-editor-scroll",
                    ]);
                    self.capture(&ctx, "20-conflict-editor");
                }
            }
            // Minimum window with the drawer open: compact layouts must keep every
            // control of the view above the drawer.
            50 => {
                self.app.show_console = true;
                self.app.active = 0;
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(780.0, 620.0)));
                self.app.dispatch(Intent::View(View::Stashes));
            }
            51 => {
                self.expect(&["stash-picker", "stash-file-picker", "stash-diff-scroll"]);
                self.above_drawer("stash-");
                self.capture(&ctx, "21-narrow-stashes");
            }
            52 => self.app.dispatch(Intent::View(View::Tree)),
            53 => {
                self.expect(&["tree-revision", "tree-toggle-list", "tree-source-scroll"]);
                self.above_drawer("tree-");
                self.capture(&ctx, "22-narrow-files");
            }
            54 => self.app.dispatch(Intent::View(View::FileHistory)),
            55 => {
                self.expect(&["file-path", "file-toggle-list", "file-source-scroll"]);
                self.above_drawer("file-");
                self.capture(&ctx, "23-narrow-file-history");
            }
            56 => self.app.dispatch(Intent::View(View::Branches)),
            57 => self.capture(&ctx, "24-narrow-branches"),
            58 => {
                if self.conflict.is_some() {
                    self.app.active = 1;
                }
            }
            59 => {
                if self.conflict.is_some() {
                    self.expect(&["conflict-save", "conflict-part-0", "conflict-result"]);
                    self.above_drawer("conflict-");
                    self.capture(&ctx, "25-narrow-conflict");
                }
            }
            60 => {
                self.app.active = 0;
                self.app.show_console = false;
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(1480.0, 920.0)));
            }
            // A refused switch leaves a failed row next to the fetch.
            61 => {
                let id = self.app.tabs[0].id;
                self.app.dispatch(Intent::Execute(
                    id,
                    Action::SwitchBranch(b"no-such-branch".to_vec()),
                ));
            }
            62 => {
                assert!(
                    !self
                        .app
                        .activities
                        .last()
                        .unwrap()
                        .outcome
                        .command_succeeded
                );
                self.app.show_console = true;
            }
            63 => {
                let last = self.app.activities.len() - 1;
                let key = format!("console-details-{last}");
                if self.app.frame_widgets.contains_key(&key) {
                    self.click(&key);
                }
            }
            64 => self.capture(&ctx, "26-activity-rows"),
            65 => {
                self.app.show_console = false;
                let ids: Vec<u64> = self.app.tabs.iter().map(|t| t.id).collect();
                for id in ids {
                    self.app.remove_tab(id);
                }
            }
            66 => {
                assert!(self.app.tabs.is_empty());
                self.expect(&[
                    "open",
                    "repository-clone-welcome",
                    "repository-new-welcome",
                    "welcome-recent-0",
                ]);
                self.capture(&ctx, "27-welcome");
            }
            _ => {
                std::fs::write(
                    self.output.join("layout-result.json"),
                    format!("{{\"passed\":true,\"captures\":{}}}", self.captures),
                )
                .unwrap();
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
        }
        self.stage += 1;
    }
}
fn main() -> eframe::Result {
    let mut args = std::env::args_os().skip(1);
    let repo = PathBuf::from(args.next().expect("disposable repository path"));
    let output = PathBuf::from(args.next().expect("output directory"));
    // Optional: a disposable repository stopped in a merge with a conflicted file.
    let conflict = args.next().map(PathBuf::from);
    std::fs::create_dir_all(&output).unwrap();
    let options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_title("Rocket — layout validation")
            .with_inner_size([1480.0, 920.0])
            .with_min_inner_size([780.0, 620.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "Rocket layout validation",
        options,
        Box::new(move |cc| {
            Ok(Box::new(Probe {
                app: Rocket::with_log_directory(&cc.egui_ctx, vec![repo], output.join("logs")),
                output,
                conflict,
                stage: 0,
                steady: 0,
                capture: None,
                captures: 0,
                pending_events: vec![],
                started: Instant::now(),
            }))
        }),
    )
}
