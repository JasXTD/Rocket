//! Runs only against a caller-provided disposable fixture. Injects input into this
//! eframe window; never controls other windows or reads an existing browser profile.
mod support;
use eframe::{App, Frame, egui::*};
use rocket_ui::app::{DialogKind, Intent, Rocket, View};
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
    pending_events: Vec<Event>,
    typing: Option<String>,
    started: Instant,
}
impl Probe {
    fn click(&mut self, key: &str) {
        let rect = self
            .app
            .frame_widgets
            .get(key)
            .unwrap_or_else(|| panic!("missing widget {key}"))
            .rect;
        let pos = rect.center();
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
    fn capture(&mut self, ctx: &Context, name: &'static str) {
        self.capture = Some(name);
        support::request_screenshot(ctx);
    }
}
impl App for Probe {
    fn raw_input_hook(&mut self, _ctx: &Context, input: &mut RawInput) {
        if self.pending_events.is_empty() {
            if let Some(text) = self.typing.take() {
                input.events.push(Event::Text(text));
            }
        } else {
            input.events.append(&mut self.pending_events);
        }
    }
    fn logic(&mut self, ctx: &Context, frame: &mut Frame) {
        self.app.logic(ctx, frame);
        ctx.request_repaint_after(Duration::from_millis(30));
    }
    fn ui(&mut self, ui: &mut Ui, frame: &mut Frame) {
        assert!(
            self.started.elapsed() < Duration::from_secs(180),
            "native UI probe timed out at {}",
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
        if self.steady < 4 {
            return;
        }
        self.steady = 0;
        match self.stage {
            0 => {
                assert!(self.app.tab().unwrap().error.is_none());
                assert!(!self.app.tab().unwrap().commits().is_empty());
                self.capture(&ctx, "01-commits");
            }
            1 => self.click("view-changes"),
            2 => {
                assert_eq!(self.app.tab().unwrap().view, View::Changes);
                self.capture(&ctx, "02-changes");
            }
            3 => self.click("stage-all"),
            4 => {
                assert!(
                    self.app
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
                        .any(|e| e.index != rocket_git::Change::Unchanged)
                );
                self.capture(&ctx, "03-staged");
            }
            5 => self.click("unstage-all"),
            6 => {
                assert!(
                    self.app
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
                        .all(|e| e.index == rocket_git::Change::Unchanged)
                );
                self.click("stage-all");
            }
            7 => {
                self.click("commit-message");
                self.typing = Some("Native UI fixture commit".into());
            }
            8 => {
                assert_eq!(self.app.tab().unwrap().draft, "Native UI fixture commit");
                self.click("commit");
            }
            9 => {
                assert_eq!(
                    self.app.tab().unwrap().commits()[0].subject_lossy(),
                    "Native UI fixture commit"
                );
                assert!(self.app.tab().unwrap().draft.is_empty());
                self.capture(&ctx, "04-committed");
            }
            10 => {
                self.app.show_console = false;
                self.app.dispatch(Intent::View(View::Commits));
            }
            11 => self.click("menu-repository"),
            12 => self.capture(&ctx, "05-repository-menu"),
            13 => self.pending_events.push(Event::Key {
                key: Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }),
            14 => self.click("menu-file"),
            15 => self.capture(&ctx, "06-file-menu"),
            16 => {
                Popup::close_all(&ctx);
                self.app.show_dialog(DialogKind::Clone);
            }
            17 => self.capture(&ctx, "07-clone-dialog"),
            18 => {
                self.app.dialog = None;
                self.app.dispatch(Intent::View(View::Branches));
            }
            19 => self.capture(&ctx, "08-branches"),
            20 => {
                self.app.dispatch(Intent::View(View::Commits));
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(1000.0, 700.0)));
            }
            21 => self.capture(&ctx, "09-compact-window"),
            22 => {
                self.app.open(self.output.join("does-not-exist"));
            }
            23 => {
                assert!(self.app.tab().unwrap().error.is_some());
                self.capture(&ctx, "10-invalid-path");
            }
            _ => {
                std::fs::write(self.output.join("native-result.json"),"{\"passed\":true,\"captures\":10,\"checks\":[\"real repository loaded\",\"pointer navigation\",\"stage all\",\"unstage all\",\"text input\",\"commit\",\"menus\",\"clone dialog\",\"branches\",\"resize\",\"invalid path\"]}").unwrap();
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
            .with_title("Rocket — isolated UI validation")
            .with_inner_size([1480.0, 920.0])
            .with_min_inner_size([900.0, 620.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "Rocket UI validation",
        options,
        Box::new(move |cc| {
            Ok(Box::new(Probe {
                app: Rocket::with_log_directory(&cc.egui_ctx, vec![repo], output.join("logs")),
                output,
                stage: 0,
                steady: 0,
                capture: None,
                pending_events: vec![],
                typing: None,
                started: Instant::now(),
            }))
        }),
    )
}
