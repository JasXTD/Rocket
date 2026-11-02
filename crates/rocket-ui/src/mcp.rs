//! Application-wide MCP settings. Files and local service requests stay on workers.
use crate::{
    app::UiControl,
    pages,
    preferences::{card_foot, card_frame, card_header, status_line, text_row},
    style::*,
};
use eframe::egui::*;
use rocket_git::{Error, ReadTask};
use rocket_mcp_control::{self as control, Config, Status};
use std::{collections::HashMap, path::PathBuf};

/// The label column of the page's cards.
const LABEL: f32 = 124.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Load,
    Save,
    Start,
    Stop,
    Rotate,
    CopyToken,
}
struct ResultState {
    config: Config,
    status: Status,
    token: Option<String>,
    message: Option<String>,
    error: Option<String>,
}
pub struct Panel {
    pub port: String,
    pub root: String,
    pub config: Option<Config>,
    pub status: Status,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub hosted: bool,
    profile: PathBuf,
    task: Option<ReadTask<ResultState>>,
    requested: bool,
    confirm_rotation: bool,
    /// The last action started; its messages show on the card it concerns.
    job: Action,
    /// The last status check failed, so whether the server runs is unknown.
    unknown: bool,
}
impl Panel {
    pub fn profile_path(&self) -> PathBuf { self.profile.clone() }
    pub fn new(profile: PathBuf) -> Self {
        Self {
            port: control::DEFAULT_PORT.to_string(),
            root: String::new(),
            config: None,
            status: Status::default(),
            error: None,
            notice: None,
            hosted: false,
            profile,
            task: None,
            requested: false,
            confirm_rotation: false,
            job: Action::Load,
            unknown: false,
        }
    }
    pub fn set_profile(&mut self, profile: PathBuf, hosted: bool) {
        *self = Self::new(profile);
        self.hosted = hosted;
    }
    pub fn busy(&self) -> bool {
        self.task.is_some()
    }
    pub fn open(&mut self) {
        if !self.requested {
            self.perform(Action::Load);
        }
    }
    /// The port or folder field differs from the saved settings.
    fn edited(&self) -> bool {
        self.config.as_ref().is_some_and(|c| {
            self.port.trim() != c.port.to_string() || self.root.trim() != c.root.to_string_lossy()
        })
    }
    /// The first check has not answered yet: the rows it fills hold placeholders.
    fn waiting(&self) -> bool {
        self.config.is_none() && self.busy()
    }
    /// What a running task does to the server: the word for its pill and the line for
    /// the server card's foot.
    fn server_work(&self) -> Option<(&'static str, &'static str)> {
        if !self.busy() {
            return None;
        }
        match self.job {
            Action::Load => Some(("Checking", "Checking the server on this computer")),
            Action::Start => Some(("Starting", "Starting the server")),
            Action::Stop => Some(("Stopping", "Stopping the server")),
            Action::Save | Action::Rotate | Action::CopyToken => None,
        }
    }
    /// The last action's error, when that action is one of `jobs`.
    fn error_for(&self, jobs: &[Action]) -> Option<String> {
        self.error.clone().filter(|_| jobs.contains(&self.job))
    }
    /// The last action's result, when that action is one of `jobs`.
    fn notice_for(&self, jobs: &[Action]) -> Option<String> {
        self.notice.clone().filter(|_| jobs.contains(&self.job))
    }
    pub fn poll(&mut self, ctx: &Context) {
        if let Some(result) = self.task.as_mut().and_then(ReadTask::try_finish) {
            self.task = None;
            match result {
                Ok(result) => {
                    // Unsaved port or folder edits stay, unless this result is their save.
                    if !self.edited() || self.job == Action::Save {
                        self.port = result.config.port.to_string();
                        self.root = result.config.root.to_string_lossy().into();
                    }
                    self.config = Some(result.config);
                    self.status = result.status;
                    self.unknown = result.error.is_some();
                    self.error = result.error;
                    self.notice = result.message;
                    if let Some(token) = result.token {
                        ctx.copy_text(token);
                    }
                }
                Err(error) => {
                    // A check that failed outright leaves the server's state unknown.
                    self.unknown |= self.job == Action::Load;
                    self.error = Some(crate::backend::user_error(&error));
                }
            }
        }
        if self.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
    }
    fn perform(&mut self, action: Action) {
        if self.busy() || (self.hosted && action != Action::Load) {
            return;
        }
        self.job = action;
        self.error = None;
        self.notice = None;
        let proposed = if action == Action::Save {
            let Ok(port) = self.port.trim().parse::<u16>() else {
                self.error = Some("Enter a port from 1024 to 65535.".into());
                return;
            };
            Some(Config {
                port,
                root: self.root.trim().into(),
            })
        } else {
            None
        };
        let profile = self.profile.clone();
        self.requested = true;
        self.task = match ReadTask::spawn(move |_| {
            let run = || -> std::io::Result<ResultState> {
                let fallback = profile.join("fixtures");
                if !profile.join("settings").exists() {
                    std::fs::create_dir_all(&fallback)?;
                }
                let mut config = control::initialize(&profile, &fallback)?;
                let mut copied = None;
                let mut message = None;
                match action {
                    Action::Load => {}
                    Action::Save => {
                        let value = proposed.as_ref().unwrap();
                        control::save(&profile, value)?;
                        config = value.clone();
                        message = Some("MCP settings saved.".into());
                    }
                    Action::Start => {
                        let executable = std::env::current_exe()?.with_file_name("rocket-mcp.exe");
                        control::start(&profile, &executable)?;
                        message = Some("Local MCP service is running.".into());
                    }
                    Action::Stop => {
                        control::stop(&profile)?;
                        message = Some("Local MCP service stopped.".into());
                    }
                    Action::Rotate => {
                        control::rotate_token(&profile)?;
                        message = Some(
                            "Access token rotated. Update clients that stored the previous token."
                                .into(),
                        );
                    }
                    Action::CopyToken => {
                        copied = Some(control::token(&profile)?);
                        message = Some(
                            "Access token copied. Treat the clipboard value as private.".into(),
                        );
                    }
                }
                let (status, error) = match control::status(&profile) {
                    Ok(status) => (status, None),
                    Err(error) => (Status::default(), Some(error.to_string())),
                };
                Ok(ResultState {
                    config,
                    status,
                    token: copied,
                    message,
                    error,
                })
            };
            run().map_err(|source| Error::Io {
                action: "manage local MCP service",
                source,
            })
        }) {
            Ok(task) => Some(task),
            Err(error) => {
                self.error = Some(crate::backend::user_error(&error));
                None
            }
        };
    }
    /// Preferences → MCP server: the server, its connection and its access token, each a
    /// card whose rows keep their places while the server is checked. Returns whether
    /// Browse was clicked for the test repositories folder.
    pub fn page(&mut self, ui: &mut Ui, widgets: &mut HashMap<String, UiControl>) -> bool {
        self.open();
        self.server_card(ui, widgets);
        let browse = self.connection_card(ui, widgets);
        self.token_card(ui, widgets);
        browse
    }

    /// The server: its state, endpoint and process, then Start server or Stop server.
    fn server_card(&mut self, ui: &mut Ui, widgets: &mut HashMap<String, UiControl>) {
        let running = self.status.running;
        let busy = self.busy();
        let waiting = self.waiting();
        let work = self.server_work();
        let ready = !busy && self.config.is_some() && !self.hosted;
        let edited = self.edited();
        let error = self.error_for(&[Action::Load, Action::Start, Action::Stop]);
        let (mut check, mut toggle) = (false, false);
        card_frame(ui, |ui| {
            card_header(ui, "Local MCP server", |ui| {
                let r = crate::chrome::icon_button(ui, Icon::Refresh, !busy, false, 28.0)
                    .on_hover_text("Check status");
                pages::control(widgets, "mcp-status", &r, ui.clip_rect());
                check = r.clicked();
                let (text, color) = match work {
                    Some((word, _)) => (word, HINT()),
                    None if running => ("Running", GREEN()),
                    None if self.unknown => ("Status unavailable", AMBER()),
                    None => ("Stopped", MUTED()),
                };
                pages::pill(ui, text, color);
                if work.is_some() {
                    ui.spinner();
                }
            });
            Frame::new().inner_margin(Margin::symmetric(0, 6)).show(ui, |ui| {
                text_row(ui, "Endpoint", LABEL, |ui| match &self.config {
                    _ if waiting => placeholder(ui, 220.0, 220.0),
                    Some(config) => {
                        let endpoint = config.endpoint();
                        ui.label(
                            RichText::new(&endpoint)
                                .monospace()
                                .size(12.5)
                                .color(if running { TEXT() } else { HINT() }),
                        );
                        let r = pages::tool(ui, Icon::Copy, "Copy", true).on_hover_text("Copy endpoint");
                        pages::control(widgets, "mcp-copy-endpoint", &r, ui.clip_rect());
                        if r.clicked() {
                            ui.ctx().copy_text(endpoint);
                        }
                    }
                    None => {
                        ui.label(RichText::new("Unknown").color(HINT()));
                    }
                });
                text_row(ui, "Process", LABEL, |ui| {
                    if waiting {
                        placeholder(ui, 180.0, 180.0);
                        return;
                    }
                    let (text, color) = if running {
                        let status = &self.status;
                        let text = format!(
                            "Rocket {} · process {} · this computer only",
                            status.version, status.pid
                        );
                        (text, MUTED())
                    } else if self.unknown {
                        ("Unknown".to_owned(), HINT())
                    } else {
                        ("Not running".to_owned(), HINT())
                    };
                    ui.add(Label::new(RichText::new(text).color(color)).truncate());
                });
            });
            messages(ui, error.as_deref(), None);
            card_foot(ui, None, |ui| {
                let r = if running {
                    ui.add_enabled(ready, Button::new("Stop server"))
                } else {
                    let r = pages::primary(ui, ready && !edited, "Start server");
                    if edited {
                        r.on_disabled_hover_text("Save or discard the connection changes first")
                    } else {
                        r
                    }
                };
                pages::control(widgets, if running { "mcp-stop" } else { "mcp-start" }, &r, ui.clip_rect());
                toggle = r.clicked();
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    let (text, color) = if self.hosted {
                        ("Controlled by the service. Change or stop it from the main Rocket window.", AMBER())
                    } else if let Some((_, line)) = work {
                        (line, HINT())
                    } else if running {
                        ("Clients on this computer connect with the access token", HINT())
                    } else {
                        ("Tools on this computer can connect once it runs", HINT())
                    };
                    ui.add(Label::new(RichText::new(text).size(12.5).color(color)).truncate())
                        .on_hover_text(text);
                });
            });
        });
        if check {
            self.perform(Action::Load);
        } else if toggle {
            self.perform(if running { Action::Stop } else { Action::Start });
        }
    }

    /// The port and test repositories folder, with Save and Discard once one is edited.
    /// Returns whether Browse was clicked.
    fn connection_card(&mut self, ui: &mut Ui, widgets: &mut HashMap<String, UiControl>) -> bool {
        let running = self.status.running;
        let waiting = self.waiting();
        let editable = !self.busy() && !running && !self.hosted && self.config.is_some();
        let changed = self.config.as_ref().map_or((false, false), |c| {
            (
                self.port.trim() != c.port.to_string(),
                self.root.trim() != c.root.to_string_lossy(),
            )
        });
        let error = self.error_for(&[Action::Save]);
        let notice = self.notice_for(&[Action::Save]);
        let (mut browse, mut save, mut discard) = (false, false, false);
        card_frame(ui, |ui| {
            card_header(ui, "Connection", |ui| {
                if running {
                    ui.label(RichText::new("Stop the server to change these").size(12.0).color(HINT()));
                    let (rect, _) = ui.allocate_exact_size(vec2(13.0, 13.0), Sense::hover());
                    icon(ui.painter(), rect, Icon::Lock, FAINT());
                }
            });
            Frame::new().inner_margin(Margin::symmetric(0, 6)).show(ui, |ui| {
                ui.add_enabled_ui(editable, |ui| {
                    text_row(ui, "Port", LABEL, |ui| {
                        if waiting {
                            placeholder(ui, 120.0, 90.0);
                            return;
                        }
                        let r = ui.add_sized(
                            [120.0, CONTROL],
                            TextEdit::singleline(&mut self.port)
                                .font(FontId::monospace(12.5))
                                .char_limit(5)
                                .vertical_align(Align::Center),
                        );
                        pages::control(widgets, "mcp-port", &r, ui.clip_rect());
                    });
                    text_row(ui, "Test repositories", LABEL, |ui| {
                        // The field's width is kept while it waits, so Browse never moves.
                        let width = (ui.available_width() - 76.0 - ui.spacing().item_spacing.x).max(80.0);
                        if waiting {
                            placeholder(ui, width, 300.0);
                        } else {
                            let r = ui
                                .add_sized(
                                    [width, CONTROL],
                                    TextEdit::singleline(&mut self.root)
                                        .font(FontId::monospace(12.0))
                                        .vertical_align(Align::Center),
                                )
                                .on_hover_text("Only marked test repositories in this folder can be controlled.");
                            pages::control(widgets, "mcp-root", &r, ui.clip_rect());
                        }
                        let r = ui.add_sized([76.0, CONTROL], Button::new("Browse"));
                        pages::control(widgets, "mcp-browse", &r, ui.clip_rect());
                        browse = r.clicked();
                    });
                });
            });
            messages(ui, error.as_deref(), notice.as_deref());
            // Save and Discard appear once a field is edited.
            if changed.0 || changed.1 {
                card_foot(ui, Some(AMBER().gamma_multiply(0.05)), |ui| {
                    let r = pages::primary(ui, editable, "Save");
                    pages::control(widgets, "mcp-save", &r, ui.clip_rect());
                    save = r.clicked();
                    let r = ui.add_enabled(editable, Button::new("Discard"));
                    pages::control(widgets, "mcp-discard", &r, ui.clip_rect());
                    discard = r.clicked();
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let (dot, _) = ui.allocate_exact_size(vec2(7.0, 7.0), Sense::hover());
                        ui.painter().circle_filled(dot.center(), 3.5, AMBER());
                        let text = match changed {
                            (true, true) => "Port and folder changed",
                            (true, false) => "Port changed",
                            _ => "Folder changed",
                        };
                        ui.label(RichText::new(text).size(12.5).color(AMBER()));
                    });
                });
            }
        });
        if save {
            self.perform(Action::Save);
        } else if discard {
            if let Some(config) = &self.config {
                self.port = config.port.to_string();
                self.root = config.root.to_string_lossy().into();
            }
            if self.job == Action::Save {
                self.error = None;
                self.notice = None;
            }
        }
        browse
    }

    /// The access token, never shown: Copy token, and Rotate token after a confirmation.
    fn token_card(&mut self, ui: &mut Ui, widgets: &mut HashMap<String, UiControl>) {
        let running = self.status.running;
        if running {
            // A running server keeps its token; a pending confirmation no longer applies.
            self.confirm_rotation = false;
        }
        let busy = self.busy();
        let ready = !busy && self.config.is_some() && !self.hosted;
        let waiting = self.waiting();
        let confirming = self.confirm_rotation;
        let jobs = [Action::Rotate, Action::CopyToken];
        let error = self.error_for(&jobs);
        let notice = self.notice_for(&jobs);
        let (mut copy, mut rotate, mut confirm, mut cancel) = (false, false, false, false);
        card_frame(ui, |ui| {
            card_header(ui, "Access token", |_| {});
            Frame::new().inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if waiting {
                        placeholder(ui, 196.0, 160.0);
                    } else {
                        masked_token(ui);
                    }
                    let r = ui.add_enabled(ready, Button::new("Copy token"));
                    pages::control(widgets, "mcp-copy-token", &r, ui.clip_rect());
                    copy = r.clicked();
                    if !confirming {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let r = ui
                                .add_enabled(ready && !running, quiet(Button::new("Rotate token")))
                                .on_disabled_hover_text("Stop the server first");
                            pages::control(widgets, "mcp-rotate", &r, ui.clip_rect());
                            rotate = r.clicked();
                        });
                    }
                });
            });
            if confirming {
                Frame::new()
                    .inner_margin(Margin { left: 14, right: 14, top: 0, bottom: 12 })
                    .show(ui, |ui| {
                        Frame::new()
                            .fill(AMBER().gamma_multiply(0.08))
                            .stroke(Stroke::new(1.0, AMBER().gamma_multiply(0.3)))
                            .corner_radius(7)
                            .inner_margin(Margin::symmetric(12, 7))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new("Clients using the current token will need the new one.")
                                            .size(12.5)
                                            .color(AMBER()),
                                    );
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        let r = ui.add_enabled(
                                            !busy,
                                            Button::new(
                                                RichText::new("Rotate token")
                                                    .color(WARN_INK())
                                                    .strong(),
                                            )
                                            .fill(AMBER()),
                                        );
                                        pages::control(widgets, "mcp-rotate-confirm", &r, ui.clip_rect());
                                        confirm = r.clicked();
                                        let r = ui.button("Cancel");
                                        pages::control(widgets, "mcp-rotate-cancel", &r, ui.clip_rect());
                                        cancel = r.clicked();
                                    });
                                });
                            });
                    });
            }
            messages(ui, error.as_deref(), notice.as_deref());
        });
        if copy {
            self.perform(Action::CopyToken);
        }
        if rotate {
            self.confirm_rotation = true;
        }
        if cancel {
            self.confirm_rotation = false;
        }
        if confirm {
            self.confirm_rotation = false;
            self.perform(Action::Rotate);
        }
    }
}

/// Where a value appears once the server has answered: a bar `bar` wide in a slot `slot`
/// wide, so what follows keeps its place.
fn placeholder(ui: &mut Ui, slot: f32, bar: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(slot, CONTROL), Sense::hover());
    let bar = Rect::from_min_size(
        pos2(rect.left(), rect.center().y - 6.0),
        vec2(bar.min(slot), 12.0),
    );
    ui.painter().rect_filled(bar, 6.0, HOVER());
}

/// The access token, never shown: a masked field with a lock.
fn masked_token(ui: &mut Ui) {
    let (rect, response) = ui.allocate_exact_size(vec2(196.0, CONTROL), Sense::hover());
    ui.painter()
        .rect(rect, 6.0, BG(), Stroke::new(1.0, BORDER()), StrokeKind::Inside);
    let lock = Rect::from_center_size(pos2(rect.left() + 17.0, rect.center().y), vec2(13.0, 13.0));
    icon(ui.painter(), lock, Icon::Lock, FAINT());
    ui.painter().text(
        pos2(rect.left() + 32.0, rect.center().y),
        Align2::LEFT_CENTER,
        "•••• •••• •••• ••••",
        FontId::monospace(12.5),
        MUTED(),
    );
    response.on_hover_text("Rocket never shows the token");
}

/// The last action's error or result, under a card's rows.
fn messages(ui: &mut Ui, error: Option<&str>, notice: Option<&str>) {
    if error.is_none() && notice.is_none() {
        return;
    }
    Frame::new()
        .inner_margin(Margin { left: 14, right: 14, top: 0, bottom: 10 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            if let Some(error) = error {
                status_line(ui, RED(), error);
            }
            if let Some(notice) = notice {
                status_line(ui, GREEN(), notice);
            }
        });
}

#[cfg(test)]
mod tests {
    use crate::{
        batch::flow_tests::{click, enabled, frame, open, wait},
        preferences::Page,
        test_common as common,
    };
    use eframe::egui::Context;

    /// The rows hold placeholders until the first check answers. Afterwards an edited port
    /// shows Save and Discard, and Start server waits until the edit is saved or discarded.
    #[test]
    fn the_page_waits_for_its_check_and_start_waits_for_edits() {
        common::isolated(
            "ROCKET_MCP_PAGE_CASE",
            "mcp::tests::the_page_waits_for_its_check_and_start_waits_for_edits",
            || {
                let f = common::Fixture::new();
                f.write("a.txt", "1\n");
                f.commit("base");
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                // A port nothing listens on, so the server reads as stopped.
                let port = std::net::TcpListener::bind("127.0.0.1:0")
                    .unwrap()
                    .local_addr()
                    .unwrap()
                    .port();
                let profile = app.mcp.profile_path();
                let root = f.root.join("mcp-fixtures");
                std::fs::create_dir_all(&root).unwrap();
                std::fs::create_dir_all(&profile).unwrap();
                std::fs::write(
                    profile.join("settings"),
                    format!("ROCKET-MCP-1\n{port}\n{}\n", root.display()),
                )
                .unwrap();
                app.show_preferences(Page::Mcp);
                frame(&ctx, &mut app, vec![]);
                frame(&ctx, &mut app, vec![]);
                // Test frames leave worker results for wait(), so the first check is pending.
                assert!(app.mcp.busy());
                assert!(!app.frame_widgets.contains_key("mcp-port"));
                assert!(!app.frame_widgets["mcp-start"].enabled);
                assert!(!app.frame_widgets["mcp-status"].enabled);
                wait(&ctx, &mut app);
                assert!(!app.mcp.status.running && app.mcp.error.is_none(), "{:?}", app.mcp.error);
                assert_eq!(app.mcp.port, port.to_string());
                assert!(enabled(&ctx, &mut app, "mcp-start"));
                assert!(enabled(&ctx, &mut app, "mcp-port"));
                assert!(!app.frame_widgets.contains_key("mcp-save"));
                // An edit: Save and Discard appear, and Start waits for one of them.
                app.mcp.port = if port == 50999 { "50998" } else { "50999" }.into();
                assert!(enabled(&ctx, &mut app, "mcp-save"));
                assert!(!app.frame_widgets["mcp-start"].enabled);
                click(&ctx, &mut app, "mcp-discard");
                assert_eq!(app.mcp.port, port.to_string());
                assert!(enabled(&ctx, &mut app, "mcp-start"));
                assert!(!app.frame_widgets.contains_key("mcp-save"));
            },
        );
    }
}
