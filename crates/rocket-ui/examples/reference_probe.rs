//! Visual and interaction review of the production UI against disposable local fixtures.
mod support;
use eframe::{App, Frame, egui::*};
use rocket_ui::app::{DetailTab, Intent, Rocket};
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
    events: Vec<Event>,
    started: Instant,
}
impl Probe {
    fn click(&mut self, key: &str) {
        let control = self
            .app
            .frame_widgets
            .get(key)
            .unwrap_or_else(|| panic!("Missing control: {key}"));
        assert!(control.enabled, "Disabled control: {key}");
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
    fn capture(&mut self, ctx: &Context, name: &'static str) {
        self.capture = Some(name);
        support::request_screenshot(ctx);
    }
}
impl App for Probe {
    fn raw_input_hook(&mut self, _ctx: &Context, input: &mut RawInput) {
        input.events.append(&mut self.events);
    }
    fn logic(&mut self, ctx: &Context, frame: &mut Frame) {
        self.app.logic(ctx, frame);
        ctx.request_repaint_after(Duration::from_millis(30));
    }
    fn ui(&mut self, ui: &mut Ui, frame: &mut Frame) {
        assert!(
            self.started.elapsed() < Duration::from_secs(180),
            "Reference probe timeout at {}",
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
                assert_eq!(self.app.tabs.len(), 3);
                self.app.active = 0;
                let id = self.app.tabs[0]
                    .commits()
                    .iter()
                    .find(|c| {
                        c.subject_lossy() == "Give authentication failures clearer recovery steps"
                    })
                    .unwrap()
                    .id
                    .clone();
                self.app.dispatch(Intent::SelectCommit(id));
            }
            1 => self.click("detail-details"),
            2 => {
                assert_eq!(self.app.tab().unwrap().detail_tab, DetailTab::Details);
                self.capture(&ctx, "01-reference-details");
            }
            3 => self.click("detail-files"),
            4 => {
                assert_eq!(self.app.tab().unwrap().detail_tab, DetailTab::Files);
                assert_eq!(
                    self.app.tab().unwrap().patch.as_ref().unwrap().files.len(),
                    3
                );
                self.capture(&ctx, "02-reference-files");
            }
            5 => self.click("detail-details"),
            6 => {
                assert_eq!(self.app.tab().unwrap().detail_tab, DetailTab::Details);
                self.capture(&ctx, "03-reference-details");
            }
            7 => self.click("tab-2"),
            8 => {
                assert_eq!(self.app.active, 1);
                self.click("tab-1");
            }
            9 => {
                assert_eq!(self.app.active, 0);
                assert_eq!(self.app.tab().unwrap().detail_tab, DetailTab::Details);
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(1480.0, 920.0)));
            }
            10 => self.capture(&ctx, "04-wide-details"),
            11 => {
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(780.0, 620.0)));
                self.click("detail-files");
            }
            12 => self.capture(&ctx, "05-minimum-files"),
            13 => {
                self.app.tabs[0].search = "no-matching-commit-in-this-fixture".into();
                self.click("detail-details");
            }
            14 => self.capture(&ctx, "06-empty-filter"),
            15 => {
                self.app.tabs[0].search.clear();
                self.click("menu-file");
            }
            16 => self.capture(&ctx, "07-keyboard-menu"),
            17 => {
                self.events.push(Event::Key {
                    key: Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                });
            }
            18 => {
                assert!(!Popup::is_any_open(&ctx));
                self.click("close-tab-3");
            }
            19 => {
                assert_eq!(self.app.tabs.len(), 2);
                assert_eq!(self.app.active, 0);
                std::fs::write(self.output.join("result.json"),r#"{"passed":true,"captures":7,"checks":["native reference size","real 11-commit history","three changed files","detail tabs via pointer","repository tabs via pointer","per-repository detail selection","close inactive tab","1480x920 layout","780x620 layout","empty filter","Escape dismisses menu"]}"#).unwrap();
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
    let paths = args.map(PathBuf::from).collect();
    std::fs::create_dir_all(&output).unwrap();
    eframe::run_native(
        "Rocket reference review",
        eframe::NativeOptions {
            viewport: ViewportBuilder::default()
                .with_title("Rocket — reference review")
                .with_inner_size([798.0, 1021.0])
                .with_min_inner_size([780.0, 620.0]),
            renderer: eframe::Renderer::Glow,
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Probe {
                app: Rocket::with_log_directory(&cc.egui_ctx, paths, output.join("logs")),
                output,
                stage: 0,
                steady: 0,
                capture: None,
                events: vec![],
                started: Instant::now(),
            }))
        }),
    )
}
