//! Exercises the real Rocket widgets in an owned native window. The companion
//! runner prepares a synthetic repository and verifies remote state independently.
//! No action is dispatched directly to the controller or backend by this probe.
mod support;
use eframe::{App, Frame, egui::*};
use rocket_ui::app::{DialogKind, Rocket};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

struct Probe {
    app: Rocket,
    output: PathBuf,
    operation: String,
    branch: String,
    upstream: bool,
    stage: usize,
    steady: usize,
    events: Vec<Event>,
    typing: Option<String>,
    capture: Option<&'static str>,
    progress_captured: bool,
    started: Instant,
}
impl Probe {
    fn click(&mut self, key: &str) {
        let pos = self
            .app
            .frame_widgets
            .get(key)
            .unwrap_or_else(|| panic!("widget unavailable: {key}"))
            .rect
            .center();
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
        ctx.send_viewport_cmd(ViewportCommand::Screenshot(UserData::default()));
    }
    fn record(&self) {
        let activity = self.app.activities.last().expect("operation result");
        let outcome = &activity.outcome;
        let tab = self.app.tab().unwrap();
        let snapshot = tab.snapshot.as_ref().unwrap();
        let refresh_ok = tab.error.is_none() && unexpected(&snapshot.warnings).is_empty();
        let details = outcome
            .details
            .iter()
            .map(|s| json_string(s))
            .collect::<Vec<_>>()
            .join(",");
        let json = format!(
            "{{\"ui_completed\":true,\"operation\":{},\"command_succeeded\":{},\"summary\":{},\"details\":[{}],\"refresh_ok\":{},\"console_visible\":{},\"progress_captured\":{},\"activity_count\":{},\"elapsed_ms\":{}}}",
            json_string(&self.operation),
            outcome.command_succeeded,
            json_string(&outcome.summary),
            details,
            refresh_ok,
            self.app.show_console,
            self.progress_captured,
            self.app.activities.len(),
            activity.elapsed.as_millis()
        );
        // Raw Git output is deliberately absent from machine-readable evidence.
        std::fs::write(self.output.join("result.pending"), json).unwrap();
        std::fs::rename(
            self.output.join("result.pending"),
            self.output.join("result.json"),
        )
        .unwrap();
    }
}
impl App for Probe {
    fn raw_input_hook(&mut self, _ctx: &Context, input: &mut RawInput) {
        if self.events.is_empty() {
            if let Some(text) = self.typing.take() {
                input.events.extend([
                    Event::Key {
                        key: Key::A,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Modifiers::CTRL | Modifiers::COMMAND,
                    },
                    Event::Text(text),
                    Event::Key {
                        key: Key::A,
                        physical_key: None,
                        pressed: false,
                        repeat: false,
                        modifiers: Modifiers::CTRL | Modifiers::COMMAND,
                    },
                ]);
            }
        } else {
            input.events.append(&mut self.events);
        }
    }
    fn logic(&mut self, ctx: &Context, frame: &mut Frame) {
        assert!(
            self.started.elapsed() < Duration::from_secs(80),
            "UI probe timed out at stage {}",
            self.stage
        );
        self.app.logic(ctx, frame);
        ctx.request_repaint_after(Duration::from_millis(30));
    }
    fn ui(&mut self, ui: &mut Ui, frame: &mut Frame) {
        let ctx = ui.ctx().clone();
        self.app.ui(ui, frame);
        for event in ctx.input(|i| i.events.clone()) {
            if let Event::Screenshot { image, .. } = event
                && let Some(name) = self.capture.take()
            {
                support::save_bmp(&self.output.join(format!("{name}.bmp")), &image).unwrap();
            }
        }
        if self.app.write.is_some() && !self.progress_captured && self.capture.is_none() {
            self.progress_captured = true;
            self.capture(&ctx, "progress");
        }
        if self.app.busy()
            || self.capture.is_some()
            || !self.events.is_empty()
            || self.typing.is_some()
        {
            self.steady = 0;
            return;
        }
        self.steady += 1;
        if self.steady < 3 {
            return;
        }
        self.steady = 0;
        if self.stage <= 7 {
            // Retain the last reached stage even when a renderer/process timeout
            // prevents the final result. No repository data is written here.
            std::fs::write(self.output.join("probe-stage"), self.stage.to_string()).unwrap();
        }
        match self.stage {
            0 => {
                assert!(self.app.backend.is_some(), "Git unavailable");
                let tab = self.app.tab().expect("fixture tab");
                assert!(tab.error.is_none());
                let snapshot = tab.snapshot.as_ref().expect("fixture snapshot");
                let unexpected = unexpected(&snapshot.warnings);
                assert!(unexpected.is_empty(), "{unexpected:?}");
                assert_eq!(snapshot.remotes.len(), 1);
                assert_eq!(snapshot.remotes[0].name, b"origin");
                let head = match snapshot.head.as_ref().unwrap() {
                    rocket_git::Head::Branch { name, .. } => name,
                    _ => panic!("fixture must have a branch"),
                };
                assert_eq!(head, self.branch.as_bytes());
                self.click(&format!("toolbar-{}", self.operation));
            }
            1 => {
                let dialog = self
                    .app
                    .dialog
                    .as_ref()
                    .expect("toolbar must open its dialog");
                assert!(matches!(
                    (&*self.operation, &dialog.kind),
                    ("fetch", DialogKind::Fetch)
                        | ("pull", DialogKind::Pull)
                        | ("push", DialogKind::Push)
                ));
                if self.operation == "push" {
                    assert_eq!(dialog.branch, self.branch.as_bytes());
                    self.click("push-destination");
                    self.typing = Some(self.branch.clone());
                }
            }
            2 => {
                if self.operation == "push" {
                    assert_eq!(self.app.dialog.as_ref().unwrap().second, self.branch);
                    if self.upstream {
                        self.click("push-upstream");
                    }
                }
            }
            3 => {
                if self.operation == "push" {
                    assert_eq!(self.app.dialog.as_ref().unwrap().flag, self.upstream);
                }
                self.capture(&ctx, "dialog");
            }
            4 => self.click("dialog-submit"),
            5 => {
                assert!(self.app.dialog.is_none());
                assert_eq!(
                    self.app.activities.len(),
                    1,
                    "exactly one operation must execute"
                );
                assert_eq!(self.app.activities[0].title, self.operation);
                if !self.app.show_console {
                    self.click("toolbar-console");
                }
            }
            6 => {
                assert!(self.app.show_console);
                self.capture(&ctx, "result");
            }
            7 => {
                self.record();
                self.app.begin_quit();
            }
            _ => {}
        }
        self.stage += 1;
    }
}
/// Snapshot warnings other than the notice every refresh carries where automatic
/// refresh still needs Windows.
fn unexpected(warnings: &[String]) -> Vec<&String> {
    warnings
        .iter()
        .filter(|warning| cfg!(windows) || !warning.starts_with("Automatic refresh unavailable"))
        .collect()
}
fn json_string(value: &str) -> String {
    let mut result = String::from("\"");
    for c in value.chars() {
        match c {
            '\"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            c if c <= '\u{1f}' => result.push_str(&format!("\\u{:04x}", c as u32)),
            c => result.push(c),
        }
    }
    result.push('\"');
    result
}
fn main() -> eframe::Result {
    let mut args = std::env::args_os().skip(1);
    let repo = PathBuf::from(args.next().expect("fixture repository"));
    let output = PathBuf::from(args.next().expect("output directory"));
    let operation = args.next().unwrap().into_string().unwrap();
    assert!(matches!(operation.as_str(), "fetch" | "pull" | "push"));
    let branch = std::fs::read_to_string(repo.join(".git/rocket-ui-fixture"))
        .expect("fixture ownership marker");
    assert!(branch.starts_with("rocket-ui-test/") && !branch.contains(['\r', '\n']));
    let upstream = args.next().is_some_and(|s| s == "set-upstream");
    assert!(args.next().is_none());
    assert!(!output.exists(), "use a fresh evidence directory");
    std::fs::create_dir_all(&output).unwrap();
    let options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_title("Rocket — transport UI validation")
            .with_inner_size([1480.0, 920.0])
            .with_min_inner_size([900.0, 620.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "Rocket transport UI validation",
        options,
        Box::new(move |cc| {
            Ok(Box::new(Probe {
                app: Rocket::with_log_directory(&cc.egui_ctx, vec![repo], output.join("logs")),
                output,
                operation,
                branch,
                upstream,
                stage: 0,
                steady: 0,
                events: vec![],
                typing: None,
                capture: None,
                progress_captured: false,
                started: Instant::now(),
            }))
        }),
    )
}
