//! Native acceptance of bounded commit-mail review using disposable local inputs.
mod support;
use eframe::{App, Frame, egui::*};
use rocket_git::MailImportAction;
use rocket_ui::app::{DialogKind, Intent, Rocket};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

struct Probe {
    app: Rocket,
    output: PathBuf,
    folder: PathBuf,
    source: PathBuf,
    original: Vec<u8>,
    repository: PathBuf,
    stage: usize,
    steady: usize,
    capture: Option<String>,
    events: Vec<Event>,
    started: Instant,
}
impl Probe {
    fn click(&mut self, name: &str) {
        let control = &self.app.frame_widgets[name];
        assert!(control.enabled, "{name} disabled at stage {}", self.stage);
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
    fn review(&self) -> Option<&rocket_git::MailImportPreview> {
        self.app
            .dialog
            .as_ref()?
            .mail_import
            .as_ref()?
            .preview
            .as_deref()
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
            "security review probe timed out at {}",
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
                .and_then(|d| d.mail_import.as_ref())
                .is_some_and(|s| s.task.is_some())
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
            0 => {
                self.app.dispatch(Intent::Dialog(DialogKind::MailImport(
                    MailImportAction::Start,
                )));
                self.app.dialog.as_mut().unwrap().first =
                    self.folder.to_string_lossy().into_owned();
            }
            1 => self.click("dialog-preview-refresh"),
            2 => {
                assert!(self.review().unwrap().requires_truncation_acknowledgement());
                assert!(!self.app.frame_widgets["dialog-submit"].enabled);
                self.capture(&ctx, "01-limited-desktop");
            }
            3 => ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(780.0, 620.0))),
            4 => self.capture(&ctx, "02-limited-compact"),
            5 => self.click("dialog-series-acknowledge"),
            6 => {
                assert!(self.review().unwrap().truncation_acknowledged());
                assert!(self.app.frame_widgets["dialog-submit"].enabled);
                self.capture(&ctx, "03-acknowledged");
            }
            7 => {
                std::fs::write(&self.source, [self.original.as_slice(), b"\n"].concat()).unwrap();
                self.click("dialog-submit");
            }
            8 => {
                let error = self
                    .app
                    .dialog
                    .as_ref()
                    .and_then(|d| d.error.as_ref())
                    .unwrap();
                assert!(error.contains("series_source_changed"), "{error}");
                assert!(self.review().is_none());
                self.capture(&ctx, "04-stale-input-refused");
            }
            9 => {
                std::fs::write(&self.source, &self.original).unwrap();
                self.click("dialog-preview-refresh");
            }
            10 => {
                assert!(!self.review().unwrap().truncation_acknowledged());
                assert!(!self.app.frame_widgets["dialog-submit"].enabled);
                self.capture(&ctx, "05-refreshed-unaccepted");
            }
            11 => self.click("dialog-series-acknowledge"),
            12 => self.click("dialog-submit"),
            13 => {
                assert!(self.app.dialog.is_none());
                let content = std::fs::read_to_string(self.repository.join("review.txt")).unwrap();
                assert!(content.ends_with("original-tail-not-in-preview\n"));
                let checks = [
                    "desktop bounded preview",
                    "compact bounded preview",
                    "pointer acknowledgement gates import",
                    "changed source refused after acknowledgement",
                    "fresh review resets acknowledgement",
                    "approved complete source imported",
                ];
                std::fs::write(
                    self.output.join("result.json"),
                    serde_json::to_vec_pretty(
                        &serde_json::json!({"passed":true,"checks":checks,"captures":5}),
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
    let repository = PathBuf::from(args.next().expect("disposable repository"));
    let folder = PathBuf::from(args.next().expect("patch folder"));
    let source = folder.join("0001-review.patch");
    let original = std::fs::read(&source).unwrap();
    std::fs::create_dir_all(&output).unwrap();
    eframe::run_native(
        "Rocket security review acceptance",
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
                vec![repository.clone()],
                output.join("logs"),
                output.join("state"),
            );
            app.dependencies.enabled = false;
            Ok(Box::new(Probe {
                app,
                output,
                folder,
                source,
                original,
                repository,
                stage: 0,
                steady: 0,
                capture: None,
                events: vec![],
                started: Instant::now(),
            }))
        }),
    )
}
