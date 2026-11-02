//! Disposable native acceptance probe for the production dependency-warning page.
use eframe::{App, Frame, egui::*};
use rocket_ui::app::{DialogKind, Intent, Rocket};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

struct Probe {
    app: Rocket,
    output: PathBuf,
    repo: PathBuf,
    fixtures: PathBuf,
    stage: u8,
    steady: usize,
    capture: Option<String>,
    events: Vec<Event>,
    started: Instant,
}
impl Probe {
    fn click(&mut self, key: &str) -> bool {
        let control = &self.app.frame_widgets[key];
        assert!(control.enabled, "{key} is disabled");
        if control.rect.height() < 18.0 {
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
                    modifiers: Modifiers::NONE,
                },
            ]);
            return false;
        }
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
        self.capture = Some(name.into());
        ctx.send_viewport_cmd(ViewportCommand::Screenshot(UserData::default()));
    }
    fn status(&self) -> serde_json::Value {
        self.app.dependencies.summary(self.app.tab().map(|t| t.id))
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
            self.started.elapsed() < Duration::from_secs(180),
            "Dependency probe timeout at stage {}: {}",
            self.stage,
            self.status()
        );
        let ctx = ui.ctx().clone();
        self.app.ui(ui, frame);
        for event in ctx.input(|i| i.events.clone()) {
            if let Event::Screenshot { image, .. } = event
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
        }
        if self.app.busy()
            || self.capture.is_some()
            || self.app.backend.is_none()
            || self.app.tab().and_then(|t| t.snapshot.as_ref()).is_none()
        {
            self.steady = 0;
            return;
        }
        self.steady += 1;
        if self.steady < 5 {
            return;
        }
        self.steady = 0;
        match self.stage {
            0 => {
                self.app.dispatch(Intent::Dialog(DialogKind::Preferences));
                self.app.preferences.page = rocket_ui::preferences::Page::Security;
            }
            1 => {
                if self.status()["status"] != "scanned" {
                    return;
                }
                assert_eq!(self.status()["findings"], 2);
                self.capture(&ctx, "01-vulnerable-1280");
            }
            2 => {
                if !self.click("dependency-finding-0") {
                    return;
                }
            }
            3 => {
                assert!(
                    self.app
                        .frame_widgets
                        .contains_key("dependency-advisory-0-0")
                );
                self.capture(&ctx, "06-finding-expanded");
            }
            4 => {
                if !self.click("dependency-finding-0") {
                    return;
                }
            }
            5 => ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(780.0, 620.0))),
            6 => self.capture(&ctx, "02-vulnerable-780"),
            7 => {
                if !self.click("pref-dependency-warnings") {
                    return;
                }
            }
            8 => {
                assert!(!self.app.dependencies.enabled);
                self.capture(&ctx, "03-warnings-off");
            }
            9 => {
                if !self.click("pref-dependency-warnings") {
                    return;
                }
            }
            10 => {
                if self.status()["running"] == true {
                    return;
                }
                let cache = self
                    .app
                    .state_directory
                    .join("dependency-security/data/osv-scalibr/npm/all.zip");
                if cache.is_file() {
                    std::fs::rename(&cache, cache.with_extension("zip.saved")).unwrap();
                }
                if !self.click("dependencies-scan") {
                    return;
                }
            }
            11 => {
                if self.status()["running"] == true {
                    return;
                }
                assert_eq!(self.status()["status"], "incomplete");
                self.capture(&ctx, "04-missing-data");
            }
            12 => {
                let cache = self
                    .app
                    .state_directory
                    .join("dependency-security/data/osv-scalibr/npm/all.zip");
                if !cache.is_file() {
                    std::fs::rename(cache.with_extension("zip.saved"), &cache).unwrap();
                }
                for name in ["Cargo.lock", "package-lock.json"] {
                    std::fs::copy(self.fixtures.join("fixed").join(name), self.repo.join(name))
                        .unwrap();
                }
                if !self.click("dependencies-scan") {
                    return;
                }
            }
            13 => {
                if self.status()["running"] == true {
                    return;
                }
                assert_eq!(self.status()["status"], "scanned");
                assert_eq!(self.status()["findings"], 0);
                self.capture(&ctx, "05-fixed");
            }
            14 => {
                std::fs::write(self.output.join("result.json"),serde_json::to_vec_pretty(&serde_json::json!({"passed":true,"captures":6,"checks":["actual offline warnings","1280 and 780 layouts","expandable finding and advisory link","pointer toggle off/on","missing cache shown incomplete","fixed versions clear warnings"],"state":self.status()})).unwrap()).unwrap();
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
    let output = PathBuf::from(args.next().expect("output directory"));
    let repo = PathBuf::from(args.next().expect("disposable repository"));
    let fixtures = PathBuf::from(args.next().expect("synthetic fixtures"));
    std::fs::create_dir_all(&output).unwrap();
    eframe::run_native(
        "Rocket dependency security acceptance",
        eframe::NativeOptions {
            viewport: ViewportBuilder::default()
                .with_inner_size([1280.0, 860.0])
                .with_min_inner_size([780.0, 620.0]),
            renderer: eframe::Renderer::Glow,
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Probe {
                app: Rocket::with_storage(
                    &cc.egui_ctx,
                    vec![repo.clone()],
                    output.join("logs"),
                    output.join("state"),
                ),
                output,
                repo,
                fixtures,
                stage: 0,
                steady: 0,
                capture: None,
                events: vec![],
                started: Instant::now(),
            }))
        }),
    )
}
