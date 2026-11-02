//! Fresh-profile native acceptance. Uses production controls and the real pinned helper.
use eframe::{App, Frame, egui::*};
use rocket_ui::app::{DialogKind, Intent, Rocket};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

struct Probe {
    app: Rocket,
    output: PathBuf,
    scanner: PathBuf,
    mode: String,
    stage: u8,
    steady: usize,
    capture: Option<String>,
    events: Vec<Event>,
    started: Instant,
    failed: bool,
}
impl Probe {
    fn status(&self) -> serde_json::Value {
        self.app.dependencies.summary(self.app.tab().map(|t| t.id))
    }
    fn scroll_preferences(&mut self, direction: f32) {
        let region = self
            .app
            .frame_widgets
            .get("prefs-scroll")
            .expect("Preferences scroll region");
        self.events.extend([
            Event::PointerMoved(region.rect.center()),
            Event::MouseWheel {
                unit: MouseWheelUnit::Point,
                delta: vec2(0.0, direction * 150.0),
                phase: TouchPhase::Move,
                modifiers: Modifiers::NONE,
            },
        ]);
    }
    fn reveal_setup_actions(&mut self) -> bool {
        if ["dependencies-update", "dependencies-scan"]
            .iter()
            .all(|key| {
                self.app
                    .frame_widgets
                    .get(*key)
                    .is_some_and(|c| c.rect.height() >= 20.0)
            })
        {
            return true;
        }
        self.scroll_preferences(-1.0);
        false
    }
    fn click(&mut self, key: &str, scroll: f32) -> bool {
        let control = self
            .app
            .frame_widgets
            .get(key)
            .filter(|c| c.rect.height() >= 16.0);
        let Some(control) = control else {
            self.scroll_preferences(scroll);
            return false;
        };
        assert!(control.enabled, "{key} disabled: {}", self.status());
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
    fn finish(&mut self, ctx: &Context) {
        let profile = &self.app.state_directory;
        let cache = profile.join("dependency-security/data/osv-scalibr/crates.io/all.zip");
        let report = serde_json::json!({"passed":!self.failed,"mode":self.mode,"profile":profile,
            "state":self.status(),"advisory_bytes":std::fs::metadata(cache).map(|m|m.len()).unwrap_or(0),
            "checks":if self.mode=="fresh" {vec!["no inherited scanner or data","picker cancellation","unreviewed file refused","verified scanner selected through picker","fresh public advisory download","offline scan","desktop and compact layouts"]} else {vec!["independent process profile state"]}});
        std::fs::write(
            self.output.join(format!("{}-result.json", self.mode)),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        self.app.quitting = true;
        ctx.send_viewport_cmd(ViewportCommand::Close);
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
            self.started.elapsed() < Duration::from_secs(420),
            "Setup probe timeout at {}: {}",
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
            || self
                .app
                .picker
                .as_ref()
                .is_some_and(|p| p.browser.loading())
        {
            self.steady = 0;
            return;
        }
        self.steady += 1;
        if self.steady < 5 {
            return;
        }
        self.steady = 0;
        let state = self.status();
        match self.stage {
            0 => {
                if state["setup_loaded"] != true || state["running"] == true {
                    return;
                }
                if self.mode != "restart" {
                    assert_eq!(state["scanner_ready"], false);
                    assert!(
                        !self
                            .app
                            .state_directory
                            .join("dependency-security/data")
                            .exists()
                    );
                }
                self.app.dispatch(Intent::Dialog(DialogKind::Preferences));
                self.app.preferences.page = rocket_ui::preferences::Page::Security;
            }
            1 => {
                if self.mode == "restart" {
                    if state["running"] == true {
                        return;
                    }
                    assert_eq!(state["scanner_ready"], true);
                    assert_eq!(state["status"], "scanned");
                    assert_eq!(state["packages"], 1);
                    self.capture(&ctx, "08-restarted-ready");
                } else {
                    assert_eq!(state["status"], "setup_required");
                    assert!(self.app.frame_widgets.contains_key("dependency-setup-open"));
                    assert!(
                        !self
                            .app
                            .frame_widgets
                            .contains_key("dependency-scanner-choose")
                    );
                    self.capture(
                        &ctx,
                        if self.mode == "fresh" {
                            "01-fresh-profile"
                        } else {
                            "09-independent-profile"
                        },
                    );
                }
            }
            2 if self.mode != "fresh" => self.finish(&ctx),
            2 => {
                if !self.click("dependency-setup-start", -1.0) {
                    return;
                }
            }
            3 => {
                assert!(!self.app.frame_widgets["dependencies-update"].enabled);
                self.capture(&ctx, "02-fresh-scanner-setup");
            }
            4 | 6 | 10 => {
                if !self.click("dependency-scanner-choose", 1.0) {
                    return;
                }
            }
            5 => {
                assert!(self.app.picker.is_some());
                if !self.click("picker-cancel", 0.0) {
                    return;
                }
            }
            7 | 11 => {
                let name = if self.stage == 7 {
                    "unreviewed.exe".into()
                } else {
                    self.scanner
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned()
                };
                self.app
                    .picker
                    .as_mut()
                    .expect("scanner picker")
                    .browser
                    .go(self.scanner.parent().unwrap().to_path_buf(), Some(name));
            }
            8 | 12 => {
                if !self.click("picker-choose", 0.0) {
                    return;
                }
            }
            9 => {
                if state["running"] == true {
                    return;
                }
                assert_eq!(state["scanner_ready"], false);
                assert_eq!(state["setup_error"], true);
                assert!(
                    !self
                        .app
                        .state_directory
                        .join("dependency-security/scanner-selection")
                        .exists()
                );
                self.capture(&ctx, "02-unreviewed-file-refused");
            }
            13 => {
                if state["running"] == true {
                    return;
                }
                assert_eq!(state["scanner_ready"], true);
                assert_eq!(state["status"], "incomplete");
                assert!(
                    std::fs::read_to_string(
                        self.app
                            .state_directory
                            .join("dependency-security/scanner-selection")
                    )
                    .unwrap()
                    .contains(self.scanner.to_str().unwrap())
                );
                self.capture(&ctx, "03-scanner-ready-no-data");
            }
            14 => ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(780.0, 620.0))),
            15 => {
                if !self.reveal_setup_actions() {
                    return;
                }
                self.capture(&ctx, "04-compact-setup");
            }
            16 => {
                if !self.click("dependencies-update", -1.0) {
                    return;
                }
            }
            17 => {
                if state["running"] == true {
                    self.capture(&ctx, "05-downloading");
                }
            }
            18 => {
                if state["running"] == true {
                    return;
                }
                if !self.reveal_setup_actions() {
                    return;
                }
                if state["status"] != "scanned" {
                    self.failed = true;
                    self.capture(&ctx, "download-failed");
                } else {
                    assert_eq!(state["packages"], 1);
                    assert!(
                        self.app
                            .state_directory
                            .join("dependency-security/data/osv-scalibr/crates.io/all.zip")
                            .is_file()
                    );
                    self.capture(&ctx, "06-ready-compact");
                }
            }
            19 => {
                if !self.click("security-overview", 1.0) {
                    return;
                }
            }
            20 => self.capture(&ctx, "10-overview-compact"),
            21 => ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(1280.0, 900.0))),
            22 => self.capture(&ctx, "07-ready-wide"),
            23 => {
                if !self.click("dependencies-scan", -1.0) {
                    return;
                }
            }
            24 => {
                if state["running"] == true {
                    return;
                }
                assert_eq!(state["status"], "scanned");
                assert_eq!(state["packages"], 1);
                if !self.click("pref-secret-protection", 1.0) {
                    return;
                }
            }
            25 => {
                assert!(!self.app.secret_protection);
                self.capture(&ctx, "11-secret-protection-off");
            }
            26 => {
                if !self.click("pref-secret-protection", 1.0) {
                    return;
                }
            }
            27 => {
                assert!(self.app.secret_protection);
                self.finish(&ctx);
            }
            _ => {}
        }
        self.stage += 1;
    }
}
fn main() -> eframe::Result {
    let mut args = std::env::args_os().skip(1);
    let output = PathBuf::from(args.next().expect("output"));
    let repo = PathBuf::from(args.next().expect("synthetic repository"));
    let scanner = PathBuf::from(args.next().expect("reviewed local scanner copy"));
    let mode = args
        .next()
        .expect("fresh, restart or isolated")
        .to_string_lossy()
        .into_owned();
    assert!(["fresh", "restart", "isolated"].contains(&mode.as_str()));
    assert!(
        !rocket_ui::dependency_security::scan::executable().is_file(),
        "Run this probe from the isolated portable folder; do not inherit the workspace scanner."
    );
    let profile = output.join("profile");
    assert_eq!(
        profile.exists(),
        mode == "restart",
        "Unexpected profile precondition"
    );
    std::fs::create_dir_all(&output).unwrap();
    let scope = output.parent().unwrap().to_path_buf();
    eframe::run_native(
        "Rocket OSV setup acceptance",
        eframe::NativeOptions {
            viewport: ViewportBuilder::default()
                .with_inner_size([1280.0, 900.0])
                .with_min_inner_size([780.0, 620.0]),
            renderer: eframe::Renderer::Glow,
            ..Default::default()
        },
        Box::new(move |cc| {
            let mut app = Rocket::with_storage(
                &cc.egui_ctx,
                vec![repo],
                output.join(format!("logs-{mode}")),
                profile,
            );
            app.browse_root = Some(scope);
            Ok(Box::new(Probe {
                app,
                output,
                scanner,
                mode,
                stage: 0,
                steady: 0,
                capture: None,
                events: vec![],
                started: Instant::now(),
                failed: false,
            }))
        }),
    )
}
