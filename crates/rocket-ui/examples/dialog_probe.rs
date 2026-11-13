//! Captures the restyled modal dialogs against a caller-provided disposable
//! fixture: Clone, Push, New branch, the Delete-branch confirmation, the
//! interactive rebase editor and its review, the reflog browser and a hard-reset
//! review, Amend, File backups, Export patch, Preferences, About and Shortcuts,
//! plus the rebase editor in a 780×620 window. Every dialog is left through its
//! `dialog-cancel` (or `dialog-submit` for informational dialogs) control with
//! real pointer clicks, and nothing is written. The fixture needs commits on the
//! current branch, a `base` branch three commits back, a `topic` branch and an
//! `origin` remote. Uses an isolated log/state directory; never controls other
//! windows.
mod support;
use eframe::{App, Frame, egui::*};
use rocket_git::{CommitActionRequest, CommitOperation, RebaseAction};
use rocket_ui::{
    app::{DialogKind, Intent, Rocket},
    backend::Action,
    patches::Mode,
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

struct Probe {
    app: Rocket,
    output: PathBuf,
    stage: usize,
    steady: usize,
    capture: Option<&'static str>,
    captures: usize,
    started: Instant,
    pending_events: Vec<Event>,
}
impl Probe {
    fn capture(&mut self, ctx: &Context, name: &'static str) {
        // Every dialog has a way out: its header ×, published as dialog-cancel (and as
        // dialog-submit on informational ones).
        assert!(
            ["dialog-cancel", "dialog-submit"]
                .iter()
                .any(|key| self.app.frame_widgets.contains_key(*key)),
            "{name}: no dialog controls"
        );
        self.capture = Some(name);
        self.captures += 1;
        support::request_screenshot(ctx);
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
    fn open(&mut self, kind: DialogKind) {
        assert!(self.app.dialog.is_none(), "previous dialog still open");
        self.app.dispatch(Intent::Dialog(kind));
        assert!(self.app.dialog.is_some(), "dialog did not open");
    }
    /// The previous `dialog-cancel` click closed the dialog without writing.
    fn closed(&self) {
        assert!(self.app.dialog.is_none(), "dialog-cancel did not close");
        assert!(self.app.write.is_none(), "a dialog started a write");
    }
    fn dialog(&mut self) -> &mut rocket_ui::app::Dialog {
        self.app.dialog.as_mut().expect("dialog open")
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
            self.started.elapsed() < Duration::from_secs(150),
            "dialog probe timed out at {}",
            self.stage
        );
        let ctx = ui.ctx().clone();
        self.app.ui(ui, frame);
        if let Some(image) = support::take_screenshot()
            && let Some(name) = self.capture.take()
        {
            support::save_bmp(&self.output.join(format!("{name}.bmp")), &image).unwrap();
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
                self.open(DialogKind::Clone);
                self.dialog().first = "C:\\Fixtures\\source-repository".into();
            }
            1 => {
                // The destination is a parent folder and a name: a new folder in the
                // existing output folder; nothing is cloned into it.
                let parent = self.output.display().to_string();
                let form = self.dialog().create.as_mut().expect("clone form");
                if form.parent != parent {
                    form.parent = parent;
                    form.name = "new-clone".into();
                    form.name_edited = true;
                    return;
                }
                assert!(self.app.frame_widgets["dialog-submit"].enabled);
                self.capture(&ctx, "01-clone");
            }
            2 => self.click("dialog-cancel"),
            3 => {
                self.closed();
                self.open(DialogKind::Push);
            }
            4 => {
                assert!(self.app.frame_widgets.contains_key("push-destination"));
                assert!(self.app.frame_widgets.contains_key("push-upstream"));
                assert!(self.app.frame_widgets["dialog-submit"].enabled);
                self.capture(&ctx, "02-push");
            }
            5 => self.click("dialog-cancel"),
            6 => {
                self.closed();
                self.open(DialogKind::NewBranch);
                self.dialog().first = "feature/dialog-restyle".into();
            }
            7 => self.capture(&ctx, "03-new-branch"),
            8 => self.click("dialog-cancel"),
            9 => {
                self.closed();
                self.open(DialogKind::Confirm(
                    Action::DeleteBranch(b"topic".to_vec()),
                    "Delete local branch topic? Git will refuse unmerged branches.".into(),
                ));
            }
            10 => self.capture(&ctx, "04-delete-branch"),
            11 => self.click("dialog-cancel"),
            12 => {
                self.closed();
                self.open(DialogKind::Rebase(RebaseAction::Interactive));
                self.dialog().first = "base".into();
            }
            13 => self.click("dialog-preview-refresh"),
            14 => {
                let s = self.dialog().rebase.as_ref().unwrap();
                assert!(s.preview.is_some(), "rebase preview loaded");
                assert!(s.entries.len() >= 3, "fixture has three commits to replay");
                self.click("dialog-rebase-action-2");
            }
            15 => self.click("dialog-rebase-action-2-drop"),
            16 => self.click("dialog-rebase-action-1"),
            17 => self.click("dialog-rebase-action-1-reword"),
            18 => {
                let s = self.dialog().rebase.as_ref().unwrap();
                assert_eq!(s.selected, 1, "changing an action selects its row");
                assert!(self.app.frame_widgets.contains_key("dialog-rebase-message"));
                assert!(self.app.frame_widgets.contains_key("dialog-rebase-up-1"));
                assert!(self.app.frame_widgets.contains_key("dialog-rebase-down-1"));
                self.capture(&ctx, "05-interactive-rebase");
            }
            19 => self.click("dialog-submit"),
            20 => {
                assert!(self.dialog().rebase.as_ref().unwrap().reviewed);
                assert!(self.app.frame_widgets.contains_key("dialog-rebase-edit"));
                self.capture(&ctx, "06-interactive-review");
            }
            21 => self.click("dialog-cancel"),
            22 => {
                self.closed();
                self.open(DialogKind::Recovery);
            }
            23 => self.click("dialog-reflog-row-1"),
            24 => {
                assert!(self.dialog().recovery.as_ref().unwrap().target.is_some());
                self.capture(&ctx, "07-recovery");
            }
            25 => self.click("dialog-recovery-reset"),
            26 => self.click("dialog-reset-hard"),
            27 => {
                let s = self.dialog().recovery.as_ref().unwrap();
                assert!(s.preview.is_some(), "reset preview loaded");
                assert!(self.app.frame_widgets.contains_key("dialog-recovery-back"));
                self.capture(&ctx, "08-hard-reset");
            }
            28 => self.click("dialog-cancel"),
            29 => {
                self.closed();
                self.open(DialogKind::CommitAction(CommitActionRequest {
                    operation: CommitOperation::Amend,
                    targets: Vec::new(),
                    mainline: None,
                    recovery: None,
                }));
            }
            30 => {
                assert!(self.dialog().commit.as_ref().unwrap().preview.is_some());
                assert!(self.app.frame_widgets.contains_key("dialog-amend-message"));
                self.capture(&ctx, "09-amend");
            }
            31 => self.click("dialog-cancel"),
            32 => {
                self.closed();
                self.open(DialogKind::FileEdit(None));
            }
            33 => {
                assert!(self.app.frame_widgets.contains_key("dialog-file-path"));
                self.capture(&ctx, "10-file-backups");
            }
            34 => self.click("dialog-cancel"),
            35 => {
                self.closed();
                self.open(DialogKind::Patch(Mode::Export));
            }
            36 => {
                assert!(
                    self.app
                        .frame_widgets
                        .contains_key("dialog-patch-source-staged")
                );
                self.capture(&ctx, "11-patch-export");
            }
            37 => self.click("dialog-cancel"),
            38 => {
                self.closed();
                self.open(DialogKind::Preferences);
            }
            39 => {
                // Preferences has pages: row density is on Appearance.
                if !self.app.frame_widgets.contains_key("rows-compact") {
                    self.click("pref-page-appearance");
                    return;
                }
                assert!(self.app.frame_widgets.contains_key("pref-page-tools"));
                assert!(!self.app.frame_widgets.contains_key("dialog-cancel"));
                self.capture(&ctx, "12-preferences");
            }
            // Informational dialogs close with Done.
            40 => self.click("dialog-submit"),
            41 => {
                self.closed();
                self.open(DialogKind::About);
            }
            42 => self.capture(&ctx, "13-about"),
            43 => self.click("dialog-submit"),
            44 => {
                // Keyboard shortcuts is a Preferences page; its × is also dialog-submit.
                self.closed();
                self.app.dispatch(Intent::Preferences(rocket_ui::preferences::Page::Shortcuts));
                assert!(self.app.dialog.is_some(), "Preferences did not open");
            }
            45 => self.capture(&ctx, "14-shortcuts"),
            46 => self.click("dialog-submit"),
            47 => {
                self.closed();
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(780.0, 620.0)));
            }
            48 => {
                self.open(DialogKind::Rebase(RebaseAction::Interactive));
                self.dialog().first = "base".into();
            }
            49 => self.click("dialog-preview-refresh"),
            50 => {
                // The footer stays inside the window at the minimum size.
                let viewport = ctx.content_rect();
                for key in ["dialog-submit", "dialog-preview-refresh", "dialog-cancel"] {
                    let rect = self.app.frame_widgets[key].rect;
                    assert!(viewport.contains_rect(rect), "{key} outside {viewport:?}");
                }
                self.capture(&ctx, "15-narrow-rebase");
            }
            51 => self.click("dialog-cancel"),
            52 => self.closed(),
            _ => {
                std::fs::write(
                    self.output.join("dialog-result.json"),
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
    std::fs::create_dir_all(&output).unwrap();
    let options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_title("Rocket — dialog validation")
            .with_inner_size([1480.0, 920.0])
            .with_min_inner_size([780.0, 620.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "Rocket dialog validation",
        options,
        Box::new(move |cc| {
            Ok(Box::new(Probe {
                app: Rocket::with_log_directory(&cc.egui_ctx, vec![repo], output.join("logs")),
                output,
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
