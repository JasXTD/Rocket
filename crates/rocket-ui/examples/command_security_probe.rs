//! Native security acceptance with synthetic command-definition files and a local repository.
mod support;
use eframe::{App, Frame, egui::*};
use rocket_git::CommandContext;
use rocket_ui::{
    app::{DialogKind, Intent, Rocket},
    custom_commands::{Editor, Step},
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

struct Probe {
    app: Rocket,
    output: PathBuf,
    fixtures: PathBuf,
    program: String,
    stage: usize,
    steady: usize,
    capture: Option<String>,
    events: Vec<Event>,
    started: Instant,
    not_before: Option<Instant>,
}
impl Probe {
    fn editor(&mut self) -> &mut Editor {
        self.app.dialog.as_mut().unwrap().commands.as_mut().unwrap()
    }
    fn source(&mut self, name: &str) {
        let path = self.fixtures.join(name).to_string_lossy().into_owned();
        let e = self.editor();
        e.step = Step::Import;
        e.exchange_path = path;
    }
    fn click(&mut self, key: &str) {
        let control = &self.app.frame_widgets[key];
        assert!(control.enabled, "disabled {key} at {}", self.stage);
        let pos = control.rect.center();
        self.events.extend([
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
    fn capture(&mut self, ctx: &Context, name: &str) {
        self.capture = Some(name.into());
        support::request_screenshot(ctx);
    }
}
impl App for Probe {
    fn raw_input_hook(&mut self, _: &Context, input: &mut RawInput) {
        input.events.append(&mut self.events);
    }
    fn logic(&mut self, ctx: &Context, frame: &mut Frame) {
        self.app.logic(ctx, frame);
        ctx.request_repaint_after(Duration::from_millis(30));
    }
    fn ui(&mut self, ui: &mut Ui, frame: &mut Frame) {
        assert!(
            self.started.elapsed() < Duration::from_secs(120),
            "probe timeout at {}",
            self.stage
        );
        let ctx = ui.ctx().clone();
        self.app.ui(ui, frame);
        if let Some(image) = support::take_screenshot()
            && let Some(name) = self.capture.take()
        {
            let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
            image::save_buffer(
                self.output.join(format!("{name}.png")),
                &bytes,
                image.width() as u32,
                image.height() as u32,
                image::ColorType::Rgba8,
            )
            .unwrap();
        }
        if self.app.busy()
            || self.capture.is_some()
            || self.app.backend.is_none()
            || self.app.tab().and_then(|t| t.snapshot.as_ref()).is_none()
            || self
                .app
                .dialog
                .as_ref()
                .and_then(|d| d.commands.as_ref())
                .is_some_and(|e| e.transfer.is_some())
            || self.not_before.is_some_and(|t| Instant::now() < t)
        {
            self.steady = 0;
            return;
        }
        self.steady += 1;
        if self.steady < 4 {
            return;
        }
        self.steady = 0;
        match self.stage {
            0 => self
                .app
                .dispatch(Intent::Preferences(rocket_ui::preferences::Page::Commands)),
            1 => self.source("hidden.json"),
            2 => self.click("dialog-commands-import"),
            3 => {
                assert!(self.editor().error.as_ref().unwrap().contains("invisible"));
                assert!(self.editor().imported.is_none());
                self.capture(&ctx, "01-hidden-definition-refused");
            }
            4 => self.source("network.json"),
            5 => self.click("dialog-commands-import"),
            6 => {
                assert!(
                    self.editor()
                        .error
                        .as_ref()
                        .unwrap()
                        .contains("local absolute")
                );
                assert!(self.editor().imported.is_none());
                self.capture(&ctx, "02-network-definition-refused");
            }
            7 => self.source("valid.json"),
            8 => self.click("dialog-commands-import"),
            9 => {
                assert_eq!(self.editor().imported.as_ref().unwrap().len(), 1);
                assert!(self.app.activities.is_empty());
                self.capture(&ctx, "03-import-review");
            }
            10 => self.click("dialog-commands-import-accept"),
            11 => {
                assert!(self.app.frame_widgets["dialog-commands-program-check"].enabled);
                self.capture(&ctx, "04-unchecked-editor-desktop");
            }
            12 => {
                self.click("dialog-commands-program-check");
                self.not_before = Some(Instant::now() + Duration::from_millis(650));
            }
            13 => ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(780.0, 620.0))),
            14 => self.capture(&ctx, "05-explicit-check-compact"),
            15 => self.editor().list[0].program = r"\\example.invalid\share\fixture.exe".into(),
            16 => {
                assert!(!self.app.frame_widgets["dialog-commands-program-check"].enabled);
                assert!(!self.app.frame_widgets["dialog-commands-save"].enabled);
                self.capture(&ctx, "06-legacy-path-disabled");
            }
            17 => {
                let program = self.program.clone();
                self.editor().list[0].program = program;
            }
            18 => self.click("dialog-commands-save"),
            19 => {
                // Saved in Preferences; the run review below takes the dialog's place.
                assert!(!self.editor().changed());
                assert_eq!(self.app.commands.len(), 1);
                self.app.commands[0].arguments = "{branch}".into();
                let context = CommandContext {
                    repository: self.app.tabs[0].path.clone(),
                    branch: Some("feature/文\u{202e}\u{200b}\\u{202e}".into()),
                    ..Default::default()
                };
                self.app
                    .dispatch(Intent::RunContextCommand { index: 0, context });
            }
            20 => {
                assert!(self.app.frame_widgets["dialog-submit"].enabled);
                assert!(self.app.activities.is_empty());
                self.capture(&ctx, "07-escaped-execution-review");
            }
            21 => self.click("dialog-cancel"),
            22 => {
                assert!(self.app.activities.is_empty());
                self.app.commands[0].arguments = "--version".into();
                self.app.dispatch(Intent::Dialog(DialogKind::RunCommand(0)));
            }
            23 => self.click("dialog-submit"),
            24 => {
                let result = &self.app.activities.last().unwrap().outcome;
                assert!(result.command_succeeded, "{:?}", result.details);
                assert!(result.details.iter().any(|d| d.contains("git version")));
                let checks = [
                    "hidden Unicode definition refused",
                    "network executable definition refused",
                    "valid import reviewed before acceptance",
                    "explicit Check and compact editor",
                    "legacy unsafe path cannot be checked or saved",
                    "escaped contextual execution review",
                    "Cancel runs nothing",
                    "confirmed ordinary command executes",
                ];
                std::fs::write(
                    self.output.join("result.json"),
                    serde_json::to_vec_pretty(
                        &serde_json::json!({"passed":true,"checks":checks,"captures":7}),
                    )
                    .unwrap(),
                )
                .unwrap();
                self.app.quitting = true;
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
            _ => {}
        }
        self.stage += 1;
    }
}
fn main() -> eframe::Result {
    let mut args = std::env::args_os().skip(1);
    let output = PathBuf::from(args.next().expect("output"));
    let repository = PathBuf::from(args.next().expect("repository"));
    let fixtures = PathBuf::from(args.next().expect("definitions"));
    let program = args
        .next()
        .expect("approved installed Git")
        .into_string()
        .unwrap();
    std::fs::create_dir_all(&output).unwrap();
    eframe::run_native(
        "Rocket command security acceptance",
        eframe::NativeOptions {
            viewport: ViewportBuilder::default()
                .with_inner_size([1280.0, 860.0])
                .with_min_inner_size([780.0, 620.0]),
            renderer: eframe::Renderer::Glow,
            ..Default::default()
        },
        Box::new(move |cc| {
            let mut app = Rocket::with_storage(
                &cc.egui_ctx,
                vec![repository],
                output.join("logs"),
                output.join("state"),
            );
            app.dependencies.enabled = false;
            Ok(Box::new(Probe {
                app,
                output,
                fixtures,
                program,
                stage: 0,
                steady: 0,
                capture: None,
                events: vec![],
                started: Instant::now(),
                not_before: None,
            }))
        }),
    )
}
