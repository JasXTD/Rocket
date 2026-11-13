//! Native acceptance for the optional pre-push review; use disposable local repos only.
mod support;
use eframe::{App, Frame, egui::*};
use rocket_ui::{
    app::{DialogKind, Intent, Rocket},
    backend::Action,
};
use std::{
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};

struct Probe {
    app: Rocket,
    output: PathBuf,
    remote: PathBuf,
    mode: String,
    stage: u8,
    steady: usize,
    capture: Option<String>,
    events: Vec<Event>,
    started: Instant,
    captured_scanning: bool,
    before: Vec<u8>,
}
impl Probe {
    fn status(&self) -> serde_json::Value {
        self.app.dependencies.summary(self.app.tab().map(|t| t.id))
    }
    fn phase(&self) -> String {
        self.status()["push_check"]["phase"]
            .as_str()
            .unwrap()
            .into()
    }
    fn refs(&self) -> Vec<u8> {
        let mut command = Command::new("git");
        command
            .arg("-C")
            .arg(&self.remote)
            .args(["for-each-ref", "--format=%(refname) %(objectname)"]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // Do not steal focus with a console.
        }
        let out = command.output().unwrap();
        assert!(out.status.success());
        out.stdout
    }
    fn click(&mut self, key: &str) -> bool {
        // Each new attempt/phase needs an egui sizing frame, whose controls are
        // invisible and disabled. The overall deadline still fails a control
        // that never becomes actionable; never inject a click into that frame.
        let Some(control) = self.app.frame_widgets.get(key).filter(|c| c.enabled) else {
            return false;
        };
        if key.starts_with("pref-") && control.rect.height() < 18.0 {
            let region = self.app.frame_widgets["prefs-scroll"].rect;
            let direction = if control.rect.center().y < region.center().y {
                150.0
            } else {
                -150.0
            };
            self.events.extend([
                Event::PointerMoved(region.center()),
                Event::MouseWheel {
                    unit: MouseWheelUnit::Point,
                    delta: vec2(0.0, direction),
                    phase: TouchPhase::Move,
                    source: MouseWheelSource::Unknown,
                    modifiers: Modifiers::NONE,
                },
            ]);
            return false;
        }
        assert!(control.rect.height() > 12.0, "{key} must be visible");
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
        true
    }
    fn capture(&mut self, ctx: &Context, name: &str) {
        self.capture = Some(format!("{}-{name}", self.mode));
        support::request_screenshot(ctx);
    }
    fn push(&mut self) {
        self.app.execute(
            self.app.tab().unwrap().id,
            Action::Push {
                remote: b"origin".to_vec(),
                local: b"main".to_vec(),
                destination: b"main".to_vec(),
                set_upstream: false,
            },
        );
    }
    fn finish(&mut self, ctx: &Context) {
        std::fs::write(
            self.output.join(format!("{}-result.json", self.mode)),
            serde_json::to_vec_pretty(
                &serde_json::json!({"passed":true,"mode":self.mode,"state":self.status()}),
            )
            .unwrap(),
        )
        .unwrap();
        self.app.begin_quit();
        self.stage = 100;
        ctx.request_repaint();
    }
}
impl App for Probe {
    fn raw_input_hook(&mut self, ctx: &Context, input: &mut RawInput) {
        // A newly launched/restarted native window can be unfocused. Ask the OS
        // to focus it before delivering queued pointer events, just as a user
        // must activate the window before interacting with a modal.
        if !self.events.is_empty() && !input.focused {
            ctx.send_viewport_cmd(ViewportCommand::Focus);
            return;
        }
        input.events.append(&mut self.events);
    }
    fn logic(&mut self, ctx: &Context, frame: &mut Frame) {
        self.app.logic(ctx, frame);
        ctx.request_repaint_after(Duration::from_millis(30));
    }
    fn ui(&mut self, ui: &mut Ui, frame: &mut Frame) {
        assert!(
            self.started.elapsed() < Duration::from_secs(180),
            "{} timed out at {}: {}",
            self.mode,
            self.stage,
            self.status()
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
        if self.stage == 100 {
            if !self.app.busy() {
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
            return;
        }
        if self.capture.is_some()
            || self.app.backend.is_none()
            || self.app.tab().and_then(|t| t.snapshot.as_ref()).is_none()
        {
            return;
        }
        if self.mode == "scan-cancel" && self.stage == 5 && self.phase() == "scanning" {
            if !self
                .app
                .frame_widgets
                .get("dependency-push-cancel")
                .is_some_and(|c| c.enabled)
            {
                return;
            }
            assert!(
                ctx.content_rect()
                    .contains_rect(self.app.frame_widgets["dependency-push-cancel"].rect)
            );
            self.capture(&ctx, "scanning-780");
            assert!(self.click("dependency-push-cancel"));
            self.stage = 30;
            return;
        }
        self.steady += 1;
        if self.steady < 5 {
            return;
        }
        self.steady = 0;
        match self.stage {
            0 => {
                if self.app.busy() || self.app.dependencies.running() {
                    return;
                }
                self.before = self.refs();
                if self.mode == "first" {
                    assert!(!self.app.dependencies.before_push);
                    self.app.dependencies.enabled = false;
                    self.app.dispatch(Intent::Dialog(DialogKind::Preferences));
                    self.app.preferences.page = rocket_ui::preferences::Page::Security;
                } else {
                    assert!(
                        self.app.dependencies.before_push,
                        "opt-in must survive restart"
                    );
                    assert!(
                        !self.app.dependencies.enabled,
                        "independent background preference"
                    );
                    self.stage = 4;
                    return;
                }
            }
            1 => {
                if !self.click("pref-dependency-push") {
                    return;
                }
            }
            2 => {
                assert!(self.app.dependencies.before_push);
                self.capture(&ctx, "preference-1280");
            }
            3 => {
                if !self.click("dialog-close") {
                    return;
                }
            }
            4 => {
                if self.app.busy() {
                    return;
                }
                self.push();
            }
            5 => {
                if self.mode == "first" && self.phase() == "scanning" && !self.captured_scanning {
                    self.captured_scanning = true;
                    self.capture(&ctx, "scanning-1280");
                    return;
                }
                if self.mode == "clean" {
                    assert_ne!(self.phase(), "review");
                    if self.app.busy() {
                        return;
                    }
                    assert_eq!(self.phase(), "idle");
                    assert_ne!(
                        self.refs(),
                        self.before,
                        "clean committed version must publish automatically"
                    );
                    self.finish(&ctx);
                    return;
                }
                if self.mode == "failure" {
                    if self.app.busy() {
                        return;
                    }
                    assert_eq!(self.phase(), "failed");
                    assert_eq!(self.refs(), self.before);
                    assert!(
                        !self
                            .app
                            .frame_widgets
                            .contains_key("dependency-push-continue")
                    );
                    ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(780.0, 620.0)));
                    self.stage = 20;
                    return;
                }
                if self.phase() != "review" {
                    return;
                }
                assert_eq!(self.status()["push_check"]["findings"], 2);
                assert_eq!(self.refs(), self.before);
                assert_ne!(
                    self.mode, "scan-cancel",
                    "The scan must be cancelled while checking"
                );
                self.capture(&ctx, "warnings-1280");
            }
            6 => {
                // A new modal first performs an egui sizing pass. Validate after
                // the captured review frame, not the frame that changed phase.
                for key in ["dependency-push-package-0-0", "dependency-push-package-0-1"] {
                    let control = self.app.frame_widgets.get(key).unwrap_or_else(|| {
                        panic!(
                            "{key} missing from settled review: {:?}",
                            self.app.frame_widgets.keys().collect::<Vec<_>>()
                        )
                    });
                    assert!(
                        control.rect.height() > 12.0,
                        "Both grouped packages must be visible"
                    );
                }
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(780.0, 620.0)));
            }
            7 => {
                for key in ["dependency-push-cancel", "dependency-push-continue"] {
                    let control = &self.app.frame_widgets[key];
                    assert!(control.enabled && ctx.content_rect().contains_rect(control.rect));
                }
                self.capture(&ctx, "warnings-780");
                self.stage = 24;
                return;
            }
            8 => {
                if !self.click("dependency-push-cancel") {
                    return;
                }
            }
            9 => {
                if self.app.busy() {
                    return;
                }
                assert_eq!(self.refs(), self.before);
                assert_eq!(self.phase(), "idle");
                if self.mode == "restart" {
                    self.finish(&ctx);
                    return;
                }
                self.push();
            }
            10 => {
                if self.phase() != "review" {
                    return;
                }
                assert_eq!(self.refs(), self.before);
                if !self.click("dependency-push-continue") {
                    return;
                }
            }
            11 => {
                if self.app.busy() {
                    return;
                }
                assert_eq!(self.phase(), "idle");
                assert_ne!(self.refs(), self.before);
                self.finish(&ctx);
                return;
            }
            20 => {
                self.capture(&ctx, "stopped-780");
            }
            21 => {
                if !self.click("dependency-push-setup") {
                    return;
                }
            }
            22 => {
                assert!(
                    self.app
                        .frame_widgets
                        .contains_key("dependency-scanner-choose")
                );
                assert_eq!(self.refs(), self.before);
                self.capture(&ctx, "scanner-setup-780");
            }
            23 => {
                self.finish(&ctx);
                return;
            }
            30 => {
                if self.app.busy() {
                    return;
                }
                assert_eq!(self.phase(), "idle");
                assert_eq!(
                    self.refs(),
                    self.before,
                    "Cancelling the scan must not publish"
                );
                self.finish(&ctx);
                return;
            }
            24 => {
                if !self.click("dependency-push-package-0-0") {
                    return;
                }
            }
            25 => {
                for key in ["dependency-push-cancel", "dependency-push-continue"] {
                    assert!(
                        ctx.content_rect()
                            .contains_rect(self.app.frame_widgets[key].rect)
                    );
                }
                self.capture(&ctx, "expanded-780");
            }
            26 => {
                self.stage = 8;
                return;
            }
            _ => unreachable!(),
        }
        self.stage += 1;
    }
}
fn main() -> eframe::Result {
    let mut args = std::env::args_os().skip(1);
    let output = PathBuf::from(args.next().expect("output profile"));
    let repo = PathBuf::from(args.next().expect("disposable repository"));
    let remote = PathBuf::from(args.next().expect("local bare remote"));
    let mode = args
        .next()
        .expect("first/scan-cancel/restart/clean/failure")
        .to_str()
        .unwrap()
        .to_owned();
    std::fs::create_dir_all(&output).unwrap();
    eframe::run_native(
        "Rocket pre-push security acceptance",
        eframe::NativeOptions {
            viewport: ViewportBuilder::default()
                .with_inner_size(if mode == "scan-cancel" {
                    [780.0, 620.0]
                } else {
                    [1280.0, 860.0]
                })
                .with_min_inner_size([780.0, 620.0]),
            renderer: eframe::Renderer::Glow,
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Probe {
                app: Rocket::with_storage(
                    &cc.egui_ctx,
                    vec![repo],
                    output.join("logs"),
                    output.join("state"),
                ),
                output,
                remote,
                mode,
                stage: 0,
                steady: 0,
                capture: None,
                events: vec![],
                started: Instant::now(),
                captured_scanning: false,
                before: vec![],
            }))
        }),
    )
}
