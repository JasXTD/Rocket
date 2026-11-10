use crate::{
    app::{Rocket, UiControl},
    preferences::Page,
    style::*,
};
use eframe::egui::*;
use rocket_terminal::{Cell, Screen, Session, Status};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

mod logging;
pub(crate) use logging::EventLog;

/// What the drawer strip and the Terminal menu can do to a running shell.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum TerminalAction {
    Interrupt,
    ClearScrollback,
    End,
}

/// What a button in the drawer asks for.
pub(crate) enum PaneAction {
    Settings,
    TryAgain,
}

/// Past this, a shell with nothing on screen no longer reads as starting.
const START_LIMIT: Duration = Duration::from_secs(10);
/// A shell that ends with an error this soon didn't really start.
const EARLY_EXIT: Duration = Duration::from_secs(3);

pub struct TerminalPane {
    pub session: Session,
    pub screen: Screen,
    pub command: String,
    pub error: Option<String>,
    pub status: Status,
    /// What the drawer calls this session's shell. The setting can change while it runs.
    pub name: String,
    /// The shell can't run in this repository's folder, so it never started.
    network: bool,
    started: Instant,
    /// How long it ran, once it has ended.
    lasted: Option<Duration>,
    /// Bash can take seconds to print its prompt, and an empty drawer looks broken.
    printed: bool,
    pub(crate) events: Vec<&'static str>,
}
impl TerminalPane {
    pub(crate) fn new(session: Session, name: String, network: bool) -> Self {
        Self {
            session,
            screen: Screen::new(100, 12),
            command: String::new(),
            error: None,
            status: Status::default(),
            name,
            network,
            started: Instant::now(),
            lasted: None,
            printed: false,
            events: vec!["terminal_start"],
        }
    }
    pub(crate) fn poll(&mut self, visible: bool) {
        let status = self.session.status();
        if status != self.status {
            self.events.push(if status.closed { "terminal_exit" } else { "terminal_ready" });
            if status.closed && !self.status.closed {
                self.lasted = Some(self.started.elapsed());
            }
            self.status = status;
        }
        if visible && let Some(screen) = self.session.snapshot(self.screen.revision) {
            self.screen = screen;
            self.printed = self.printed || self.screen.cells.iter().any(|r| r.iter().any(|c| c.ch != ' '));
        }
    }
    fn starting(&self) -> bool {
        !self.printed && !self.status.closed && self.started.elapsed() < START_LIMIT
    }
    /// A session that never ran: its title, the detail under it and whether trying again
    /// can help.
    fn failure(&self) -> Option<(String, String, bool)> {
        if !self.status.closed || self.status.pid.is_some() {
            return None;
        }
        let error = self.status.error.as_deref()?;
        let detail = error.strip_prefix("Terminal: ").unwrap_or(error).to_owned();
        Some(if self.network {
            (format!("{} can't run in a network folder", self.name), detail, false)
        } else {
            (format!("{} didn't start", self.name), detail, true)
        })
    }
    /// The exit code of a shell that ended with an error right after it started.
    fn stopped_early(&self) -> Option<u32> {
        self.status.exit_code.filter(|code| {
            *code != 0 && self.status.pid.is_some() && self.lasted.is_some_and(|lasted| lasted < EARLY_EXIT)
        })
    }
    pub(crate) fn stop(&mut self) {
        if !self.session.status().closed {
            self.session.stop();
            self.events.push("terminal_stop");
        }
    }
    fn send(&mut self, bytes: &[u8], event: &'static str) {
        match self.session.input(bytes) {
            Ok(()) => {
                self.error = None;
                self.events.push(event);
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }
    /// The screen with the command line under it. The session's state and actions are in
    /// the drawer header (`Rocket::terminal_actions`).
    pub(crate) fn show(&mut self, ui: &mut Ui, id: u64, widgets: &mut HashMap<String, UiControl>, closing: bool) -> Option<PaneAction> {
        self.show_with_input(ui, id, widgets, closing, false)
    }
    pub(crate) fn show_agent(&mut self, ui: &mut Ui, id: u64, widgets: &mut HashMap<String, UiControl>, closing: bool) -> Option<PaneAction> {
        self.show_with_input(ui, id, widgets, closing, true)
    }
    fn show_with_input(&mut self, ui: &mut Ui, id: u64, widgets: &mut HashMap<String, UiControl>, closing: bool, agent: bool) -> Option<PaneAction> {
        let running = self.status.running && !closing;
        let failure = self.failure();
        // A session that never ran says why below, in place of its screen.
        if let Some(error) = self.error.as_ref().or(self.status.error.as_ref().filter(|_| failure.is_none())) {
            ui.colored_label(RED(), error);
        }
        // The command line sits under the screen, as a console's input does. Its room is
        // kept first, so the screen still ends on a whole row right above it.
        let body = vec2(
            ui.available_width(),
            (ui.available_height() - CONTROL - ui.spacing().item_spacing.y).max(0.0),
        );
        let action = ui
            .allocate_ui_with_layout(body, Layout::top_down(Align::Min), |ui| {
                ui.set_min_size(body);
                match failure {
                    Some(failure) => failure_state(ui, widgets, failure),
                    None => self.show_screen(ui, id, widgets, running),
                }
            })
            .inner;
        self.command_line(ui, id, widgets, running, agent);
        action
    }

    /// A line for the shell, sent with Enter or Send.
    fn command_line(&mut self, ui: &mut Ui, id: u64, widgets: &mut HashMap<String, UiControl>, running: bool, agent: bool) {
        const SEND: f32 = 64.0;
        // A single-line TextEdit consumes Enter as it surrenders focus.
        // Capture ownership before drawing it: lost_focus() can still be
        // true on the first keyboard frame after clicking the terminal.
        // Using that stale response would steal Enter from the shell.
        let command_focused =
            ui.input(|i| i.focused) && ui.memory(|m| m.has_focus(command_id(id)));
        let enter_pressed =
            ui.input(|i| i.key_pressed(Key::Enter) && i.modifiers == Modifiers::NONE);
        ui.horizontal(|ui| {
            let width = (ui.available_width() - SEND - ui.spacing().item_spacing.x).max(120.0);
            // Rocket's field: a raised box with a prompt mark, outlined brighter while it
            // holds the keyboard. Its text fills the box, so a click anywhere in it types.
            let r = Frame::new()
                .fill(RAISED())
                .stroke(Stroke::new(1.0, if command_focused { EDGE_HOT() } else { BORDER() }))
                .corner_radius(6)
                .inner_margin(Margin::symmetric(8, 0))
                .show(ui, |ui| {
                    // Less the margins and the outline: the box ends right before Send.
                    ui.set_width(width - 18.0);
                    // A row is at least interact_size tall; this one is the box less its
                    // outline, so the box is exactly as tall as Send.
                    ui.spacing_mut().interact_size.y = CONTROL - 2.0;
                    ui.horizontal(|ui| {
                        if !agent {
                            let (mark, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                            icon(ui.painter(), mark, Icon::Prompt, if running { MUTED() } else { FAINT() });
                        }
                        ui.add_enabled(
                            running,
                            TextEdit::singleline(&mut self.command)
                                .id(command_id(id))
                                .frame(Frame::NONE)
                                .font(TextStyle::Monospace)
                                .vertical_align(Align::Center)
                                .min_size(vec2(0.0, CONTROL - 2.0))
                                .desired_width(ui.available_width())
                                .hint_text(if agent { crate::i18n::tr("Type in this session…") } else { crate::i18n::tr("Command or input · Enter to send") }),
                        )
                    })
                    .inner
                })
                .inner;
            widgets.insert("terminal-command".into(), (&r).into());
            let enter = running && enter_pressed && command_focused;
            if enter {
                ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
            }
            let send = ui.add_enabled(running, Button::new("Send").min_size(vec2(SEND, CONTROL)));
            widgets.insert("terminal-send".into(), (&send).into());
            if enter || send.clicked() {
                let command = format!("{}\r", self.command);
                self.send(command.as_bytes(), "terminal_input_queued");
                if self.error.is_none() {
                    self.command.clear();
                }
                r.request_focus();
            }
        });
    }

    /// The shell's screen in whole rows, under the line of a shell that stopped at once.
    fn show_screen(&mut self, ui: &mut Ui, id: u64, widgets: &mut HashMap<String, UiControl>, running: bool) -> Option<PaneAction> {
        let mut action = None;
        if let Some(code) = self.stopped_early() {
            action = early_exit_line(ui, widgets, &self.name, code);
        }
        let font = FontId::monospace(13.0);
        let cell_size = ui.fonts_mut(|fonts| {
            vec2(
                fonts.glyph_width(&font, 'M'),
                fonts.row_height(&font).max(18.0),
            )
        });
        let columns = (ui.available_width() / cell_size.x)
            .floor()
            .clamp(20.0, 240.0) as u16;
        // Whole rows that fit. The shell keeps at least four rows; in a shorter drawer
        // the screen shows only whole rows (stuck to the bottom) instead of cutting the
        // top one through its glyphs.
        let fit = (ui.available_height() / cell_size.y).floor().max(1.0);
        let rows = fit.clamp(4.0, 100.0) as u16;
        if running {
            self.session.resize(columns, rows);
        }
        // What is left over, less than a row, goes above the screen: its last row meets
        // the command line.
        ui.add_space((ui.available_height() - fit * cell_size.y).max(0.0));
        let scrollback = self.screen.scrollback.len();
        // A process can exit before its first viewport resize. Its trailing
        // empty rows must not scroll a short startup error out of sight.
        let screen_rows = if self.status.closed {
            self.screen.cells.iter().rposition(|row|row.iter().any(|cell|cell.ch!=' ' || !cell.suffix.is_empty())).map_or(1,|last|last+1)
        } else {self.screen.cells.len()};
        let output = ui
            .scope(|ui| {
                // Virtual row offsets and painted rows must use the same spacing.
                // Setting this only inside the callback clips the first output rows.
                ui.spacing_mut().item_spacing.y = 0.0;
                ScrollArea::both()
                    .id_salt(("terminal-scroll", id))
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .max_height(fit * cell_size.y)
                    .min_scrolled_height(fit * cell_size.y)
                    .show_rows(
                        ui,
                        cell_size.y,
                        scrollback + screen_rows,
                        |ui, range| {
                            ui.set_min_width(self.screen.columns as f32 * cell_size.x);
                            for y in range {
                                let row = if y < scrollback {
                                    &self.screen.scrollback[y]
                                } else {
                                    &self.screen.cells[y - scrollback]
                                };
                                let (rect, _) = ui.allocate_exact_size(
                                    vec2(
                                        ui.available_width().max(row.len() as f32 * cell_size.x),
                                        cell_size.y,
                                    ),
                                    Sense::hover(),
                                );
                                let painter =
                                    ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
                                paint_row(&painter, row, rect.min, cell_size, &font);
                                if running
                                    && self.screen.cursor_visible
                                    && y == scrollback + self.screen.cursor.1
                                {
                                    let x = rect.left() + self.screen.cursor.0 as f32 * cell_size.x;
                                    painter.line_segment(
                                        [
                                            pos2(x, rect.bottom() - 1.0),
                                            pos2(x + cell_size.x, rect.bottom() - 1.0),
                                        ],
                                        Stroke::new(1.5, BLUE()),
                                    );
                                }
                            }
                        },
                    )
            })
            .inner;
        let response = ui.interact(output.inner_rect, screen_id(id), Sense::click());
        widgets.insert("terminal-screen".into(), (&response).into());
        if self.starting() {
            // Over the screen, which already holds the keyboard and the session's size.
            let mut over = ui.new_child(UiBuilder::new().max_rect(output.inner_rect).layout(Layout::top_down(Align::Center)));
            crate::pages::loading(&mut over, &format!("Starting {}", self.name));
        }
        if response.clicked() {
            response.request_focus();
        }
        if running && response.has_focus() {
            ui.memory_mut(|memory| {
                memory.set_focus_lock_filter(
                    screen_id(id),
                    EventFilter {
                        tab: true,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: true,
                    },
                )
            });
            // Consume only while the terminal screen owns focus. No global Git
            // shortcut should reinterpret Ctrl+C, Ctrl+W, arrows or Tab here.
            let events = ui.input_mut(|i| {
                let mut events = Vec::new();
                i.events.retain(|event| {
                    if matches!(
                        event,
                        Event::Copy | Event::Text(_) | Event::Paste(_) | Event::Key { .. }
                    ) {
                        events.push(event.clone());
                        false
                    } else {
                        true
                    }
                });
                events
            });
            for event in events {
                match event {
                    // egui-winit translates native Ctrl+C to Copy, without a
                    // Key::C press. Only the focused terminal consumes it;
                    // TextEdit fields retain their normal clipboard behavior.
                    Event::Copy if !cfg!(target_os = "macos") => self.send(&[3], "terminal_interrupt"),
                    // On macOS this is Command+C, not the terminal's Ctrl+C.
                    // Ctrl+C arrives as a key event and is encoded below.
                    Event::Copy => {},
                    Event::Text(text) | Event::Paste(text) => {
                        self.send(text.as_bytes(), "terminal_input_queued")
                    }
                    Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => {
                        let control = key.name().as_bytes();
                        let bytes = if modifiers.ctrl
                            && control.len() == 1
                            && control[0].is_ascii_alphabetic()
                        {
                            vec![control[0].to_ascii_uppercase() - b'A' + 1]
                        } else {
                            match key {
                                Key::Enter => b"\r".to_vec(),
                                Key::Backspace => vec![127],
                                Key::Tab if modifiers.shift => b"\x1b[Z".to_vec(),
                                Key::Tab => b"\t".to_vec(),
                                Key::Escape => vec![27],
                                Key::ArrowUp => b"\x1b[A".to_vec(),
                                Key::ArrowDown => b"\x1b[B".to_vec(),
                                Key::ArrowRight => b"\x1b[C".to_vec(),
                                Key::ArrowLeft => b"\x1b[D".to_vec(),
                                Key::Home => b"\x1b[H".to_vec(),
                                Key::End => b"\x1b[F".to_vec(),
                                Key::Delete => b"\x1b[3~".to_vec(),
                                Key::PageUp => b"\x1b[5~".to_vec(),
                                Key::PageDown => b"\x1b[6~".to_vec(),
                                _ => Vec::new(),
                            }
                        };
                        if !bytes.is_empty() {
                            self.send(
                                &bytes,
                                if bytes == [3] {
                                    "terminal_interrupt"
                                } else {
                                    "terminal_input_queued"
                                },
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
        action
    }
}

/// The drawer of a session that never ran, where its screen would be.
fn failure_state(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    (title, detail, retry): (String, String, bool),
) -> Option<PaneAction> {
    const BUTTONS: f32 = 240.0;
    let mut action = None;
    let mut buttons = |ui: &mut Ui| {
        let settings = ui.button("Terminal settings");
        widgets.insert("terminal-settings".into(), (&settings).into());
        if settings.clicked() {
            action = Some(PaneAction::Settings);
        }
        if retry {
            let again = ui.button("Try again");
            widgets.insert("terminal-try-again".into(), (&again).into());
            if again.clicked() {
                action = Some(PaneAction::TryAgain);
            }
        }
    };
    let size = ui.available_size();
    let color = if retry { RED() } else { AMBER() };
    if size.y < 130.0 {
        // A short drawer has room for one line.
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
            icon(ui.painter(), rect, Icon::Info, color);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.allocate_ui_with_layout(vec2(if retry { BUTTONS } else { BUTTONS / 2.0 }, CONTROL), Layout::right_to_left(Align::Center), &mut buttons);
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add(Label::new(RichText::new(format!("{title} · {detail}")).size(12.5)).truncate());
                });
            });
        });
        return action;
    }
    ui.allocate_ui_with_layout(size, Layout::top_down(Align::Center), |ui| {
        ui.set_min_size(size);
        ui.add_space(((size.y - 112.0) / 2.0).max(0.0));
        let (rect, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
        icon(ui.painter(), rect, Icon::Info, color);
        ui.add_space(2.0);
        ui.label(RichText::new(&title).font(semibold(13.0)));
        ui.add(Label::new(RichText::new(&detail).size(12.0).color(HINT())).wrap());
        ui.add_space(6.0);
        let width = if retry { BUTTONS - 34.0 } else { BUTTONS / 2.0 };
        ui.allocate_ui_with_layout(vec2(width, CONTROL), Layout::left_to_right(Align::Center), &mut buttons);
    });
    action
}

/// The line over the screen of a shell that ended with an error right after it started.
/// Its own output stays below, because that usually says why.
fn early_exit_line(ui: &mut Ui, widgets: &mut HashMap<String, UiControl>, name: &str, code: u32) -> Option<PaneAction> {
    let mut action = None;
    Frame::new()
        .fill(RED().gamma_multiply(0.08))
        .stroke(Stroke::new(1.0, RED().gamma_multiply(0.22)))
        .corner_radius(6)
        .inner_margin(Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(13.0, 13.0), Sense::hover());
                icon(ui.painter(), rect, Icon::Info, RED());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let settings = ui.button("Terminal settings");
                    widgets.insert("terminal-settings".into(), (&settings).into());
                    if settings.clicked() {
                        action = Some(PaneAction::Settings);
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.add(Label::new(RichText::new(format!("{name} stopped right after it started. Exit code {code}.")).size(12.5).color(RED())).truncate());
                    });
                });
            });
        });
    ui.add_space(4.0);
    action
}

fn paint_row(painter: &Painter, row: &[Cell], origin: Pos2, size: Vec2, font: &FontId) {
    for (column, cell) in row.iter().enumerate().filter(|(_, cell)| cell.width != 0) {
        let area = Rect::from_min_size(
            origin + vec2(column as f32 * size.x, 0.0),
            vec2(f32::from(cell.width) * size.x, size.y),
        );
        if cell.background != 0 {
            painter.rect_filled(area, 0.0, palette(cell.background));
        }
        if cell.ch == ' ' && cell.suffix.is_empty() {
            continue;
        }
        let text = cell.text();
        let color = palette(cell.foreground);
        let font = if cell.width > 1 || cell.suffix.contains(['\u{fe0e}', '\u{fe0f}']) {
            FontId::new(font.size, FontFamily::Name("terminal-wide".into()))
        } else {
            font.clone()
        };
        let mut galley = painter.layout_no_wrap(text.clone(), font.clone(), color);
        if galley.size().x > area.width() {
            // Some fallback faces have wider advances. Fit this cluster only;
            // its neighbour and the cursor remain on the terminal's cell grid.
            let fitted = FontId::new(
                font.size * area.width() / galley.size().x,
                font.family.clone(),
            );
            galley = painter.layout_no_wrap(text, fitted, color);
        }
        let position = area.min + (area.size() - galley.size()).max(Vec2::ZERO) * 0.5;
        painter.galley(position, galley, color);
    }
}
fn command_id(id: u64) -> Id {
    Id::new(("terminal-command", id))
}
fn screen_id(id: u64) -> Id {
    Id::new(("terminal-screen", id))
}
fn palette(color: u8) -> Color32 {
    ANSI()[usize::from(color.min(15))]
}
impl Rocket {
    pub fn open_terminal(&mut self) {
        if self.quitting {
            return;
        }
        self.activity_history.close();
        self.show_console = false;
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        if tab
            .terminal
            .as_ref()
            .is_some_and(|t| !t.session.status().closed)
        {
            self.show_terminal = true;
            self.drawer_collapsed = false;
            self.context
                .memory_mut(|m| m.request_focus(command_id(tab.id)));
            return;
        }
        let name = self.terminal_name();
        let launch = self.terminal_launch();
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let context = self.context.clone();
        let wake = Arc::new(move || context.request_repaint());
        let network = launch.as_ref().is_ok_and(|shell| !shell.runs_in(&tab.path));
        let result = match (&self.terminal_history_file, launch) {
            (Some(history), Ok(shell)) => {
                // Mac test sessions snapshot Bash 3.2 history. Each tab has at
                // most one live terminal, so give it its own file to prevent
                // concurrent snapshots in different tabs overwriting each other.
                #[cfg(target_os = "macos")]
                let history = history.with_extension(format!("tab-{}.log", tab.id));
                Session::start_shell_with_history(&tab.path, 100, 12, wake, &shell, history.as_path())
            }
            (None, Ok(_)) if network => Ok(Session::refused(crate::views::display_path(&tab.path))),
            (None, Ok(shell)) => Session::start_shell(&tab.path, 100, 12, wake, &shell),
            (_, Err(reason)) => Ok(Session::refused(reason)),
        };
        match result {
            Ok(session) => {
                tab.terminal = Some(TerminalPane::new(session, name, network));
                self.show_terminal = true;
                self.drawer_collapsed = false;
                self.context
                    .memory_mut(|m| m.request_focus(command_id(tab.id)));
            }
            Err(error) => self.notice = Some(format!("Cannot start {name}: {error}")),
        }
    }
    /// A collapsed drawer does not draw the screen, so it cannot hold the keyboard.
    pub(crate) fn terminal_focused(&self, ctx: &Context) -> bool {
        if self.tab().is_some_and(|t|t.view==crate::app::View::Agents) && self.agents.pane==crate::agents::Pane::Terminal && self.agents.form.is_none() && let Some(s)=self.agents.selected() {
            return ctx.memory(|m|m.has_focus(screen_id(s.terminal_id()))||m.has_focus(command_id(s.terminal_id())));
        }
        self.show_terminal
            && !self.drawer_collapsed
            && self.tab().is_some_and(|tab| {
                ctx.memory(|m| m.has_focus(screen_id(tab.id)) || m.has_focus(command_id(tab.id)))
            })
    }
    pub fn terminal_screen_focused(&self) -> bool {
        if self.tab().is_some_and(|t|t.view==crate::app::View::Agents) && self.agents.pane==crate::agents::Pane::Terminal && self.agents.form.is_none() && let Some(s)=self.agents.selected() {
            return self.context.memory(|m|m.has_focus(screen_id(s.terminal_id())));
        }
        self.show_terminal
            && !self.drawer_collapsed
            && self
                .tab()
                .is_some_and(|tab| self.context.memory(|m| m.has_focus(screen_id(tab.id))))
    }
    /// What the active repository's shell is called: its session's, or the next one's.
    pub(crate) fn terminal_session_name(&self) -> String {
        self.tab()
            .and_then(|tab| tab.terminal.as_ref())
            .map_or_else(|| self.terminal_name(), |terminal| terminal.name.clone())
    }
    /// The active repository's shell in a few words, and whether it is running.
    pub(crate) fn terminal_state(&self) -> Option<(bool, String)> {
        let terminal = self.tab()?.terminal.as_ref()?;
        let status = &terminal.status;
        Some(if status.closed && status.pid.is_none() {
            (false, "didn't start".to_owned())
        } else if status.closed {
            let code = status.exit_code.map_or(String::new(), |c| format!(" ({c})"));
            (false, format!("exited{code}"))
        } else if terminal.starting() {
            // Interrupt and End session stay on: a shell can hang while it starts.
            (status.running, "starting".to_owned())
        } else if status.running {
            (true, "running".to_owned())
        } else {
            (false, "starting".to_owned())
        })
    }
    /// Interrupt, clear or end the active repository's shell, from the drawer strip or
    /// the Terminal menu.
    pub(crate) fn terminal_action(&mut self, action: TerminalAction) {
        let quitting = self.quitting;
        let Some(terminal) = self
            .tabs
            .get_mut(self.active)
            .and_then(|tab| tab.terminal.as_mut())
        else {
            return;
        };
        match action {
            TerminalAction::Interrupt if terminal.status.running && !quitting => {
                terminal.send(&[3], "terminal_interrupt");
            }
            TerminalAction::Interrupt => {}
            TerminalAction::ClearScrollback => {
                terminal.session.clear_scrollback();
                terminal.events.push("terminal_clear_scrollback");
            }
            TerminalAction::End => terminal.stop(),
        }
    }
    /// The shell's state and actions in the drawer header, added right to left.
    pub(crate) fn terminal_actions(&mut self, ui: &mut Ui) {
        let state = self.terminal_state();
        let name = self.terminal_session_name();
        let running = state.as_ref().is_some_and(|(running, _)| *running) && !self.quitting;
        let ran = self.tab().and_then(|tab| tab.terminal.as_ref()).is_some_and(|t| t.status.pid.is_some());
        let mut chosen = None;
        for (key, image, label, enabled, action) in [
            ("terminal-end", Icon::Stop, "End session", running, TerminalAction::End),
            (
                "terminal-clear-scrollback",
                Icon::Trash,
                "Clear scrollback",
                ran,
                TerminalAction::ClearScrollback,
            ),
            ("terminal-interrupt", Icon::Interrupt, "Interrupt", running, TerminalAction::Interrupt),
        ] {
            let mut response = crate::pages::tool(ui, image, label, enabled);
            if action == TerminalAction::Interrupt {
                response = response.on_hover_text("Ctrl+C");
            }
            self.frame_widgets.insert(key.into(), (&response).into());
            if response.clicked() {
                chosen = Some(action);
            }
        }
        // Dropped rather than squeezed when the window is narrow.
        if let Some((_, state)) = state {
            let text = format!("{name} · {state}");
            let width = ui
                .painter()
                .layout_no_wrap(text.clone(), FontId::proportional(12.0), HINT())
                .size()
                .x;
            if width + 12.0 < ui.available_width() {
                ui.add_space(6.0);
                ui.label(RichText::new(text).size(12.0).color(HINT()));
            }
        }
        if let Some(action) = chosen {
            self.terminal_action(action);
        }
    }
    pub(crate) fn terminal_panel(&mut self, ui: &mut Ui) {
        let mut action = None;
        if let Some(tab) = self.tabs.get_mut(self.active)
            && let Some(terminal) = &mut tab.terminal
        {
            action = terminal.show(ui, tab.id, &mut self.frame_widgets, self.quitting);
        } else {
            ui.label(format!("Open Terminal → New Terminal to start {} in this repository.", self.terminal_name()));
        }
        match action {
            Some(PaneAction::Settings) => self.show_preferences(Page::Terminal),
            Some(PaneAction::TryAgain) => self.open_terminal(),
            None => {}
        }
    }
    pub(crate) fn poll_terminals(&mut self) {
        let visible = self.show_terminal && !self.drawer_collapsed && !self.quitting;
        // Retired tabs stay here until their owned shell has exited, so tab
        // closure records the same stop/exit events as the visible End button.
        for (terminal, visible) in self
            .tabs
            .iter_mut()
            .enumerate()
            .filter_map(|(index, t)| t.terminal.as_mut().map(|t| (t, visible && index == self.active)))
            .chain(self.closing_terminals.iter_mut().map(|t| (t, false)))
        {
            terminal.poll(visible);
            // Coalesce keystrokes per frame. Commands and output never enter logs.
            terminal.events.sort_unstable();
            terminal.events.dedup();
            if self.backend.is_some() {
                for event in terminal.events.drain(..) {
                    self.terminal_log.push(event, event == "terminal_exit" && terminal.status.error.is_some());
                }
            }
        }
        self.closing_terminals.retain(|t| !t.status.closed);
        if let Some(backend) = &self.backend {
            match self.terminal_log.poll(&backend.log) {
                Ok(Some(notice)) => self.notice = Some(notice),
                Ok(None) => {}
                Err(error) => {
                    self.quitting = false;
                    self.notice = Some(format!("Could not write terminal diagnostics: {}", crate::backend::user_error(&error)));
                }
            }
        }
        if self.terminal_log.busy() {
            // Ordinary repository actions remain usable while diagnostics drain.
            self.context.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
}

#[cfg(all(test, windows))]
mod lifecycle_tests;

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::{
        fs, thread,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn native_copy_event_interrupts_only_the_focused_terminal_and_keeps_the_shell() {
        fn wait(session: &Session, predicate: impl Fn(&str) -> bool) {
            let started = Instant::now();
            let mut text = String::new();
            loop {
                assert!(session.status().error.is_none());
                // Snapshot is nonblocking: startup or a worker holding the
                // screen lock can legitimately leave this poll without one.
                if let Some(screen) = session.snapshot(0) {
                    text = screen.text();
                    if predicate(&text) {
                        return;
                    }
                }
                assert!(
                    started.elapsed() < Duration::from_secs(8),
                    "terminal timed out: {text}"
                );
                thread::sleep(Duration::from_millis(20));
            }
        }
        let root = std::env::temp_dir().join(format!(
            "rocket-copy-interrupt-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let session =
            Session::start_with_history(&root, 150, 12, Arc::new(|| {}), &root.join("history.log"))
                .unwrap();
        wait(&session, |s| s.trim_end().ends_with("\n>"));
        let mut terminal = TerminalPane {
            status: session.status(),
            screen: Screen::new(150, 12),
            events: Vec::new(),
            ..TerminalPane::new(session, "Windows PowerShell".into(), false)
        };
        let ctx = Context::default();
        let mut widgets = HashMap::new();
        terminal.poll(false);
        assert!(terminal.status.running);
        assert_eq!(terminal.screen.revision, 1, "hidden terminal must not copy output");
        let shown = Instant::now();
        while terminal.screen.revision == 1 {
            terminal.poll(true);
            assert!(shown.elapsed() < Duration::from_secs(5));
            thread::yield_now();
        }
        let mut render = |terminal: &mut TerminalPane, events| {
            let mut output = ctx.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 500.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    terminal.show(ui, 1, &mut widgets, false);
                },
            );
            output.textures_delta.clear();
        };
        render(&mut terminal, vec![]);
        terminal.session.input(b"$global:RocketCopyState=73; [Console]::WriteLine('COPY_'+'READY'); Start-Sleep -Seconds 60\r").unwrap();
        wait(&terminal.session, |s| {
            s.lines().any(|l| l.trim() == "COPY_READY")
        });
        let pid = terminal.session.status().pid;
        terminal.command = "Copy keeps this draft".into();
        ctx.memory_mut(|m| m.request_focus(command_id(1)));
        render(&mut terminal, vec![Event::Copy]);
        assert_eq!(terminal.command, "Copy keeps this draft");
        assert!(!terminal.events.contains(&"terminal_interrupt"));
        // egui-winit emits Copy instead of Key::C for the Windows Ctrl+C press.
        ctx.memory_mut(|m| m.request_focus(screen_id(1)));
        render(&mut terminal, vec![Event::Copy]);
        wait(&terminal.session, |s| s.trim_end().ends_with("\n>"));
        terminal
            .session
            .input(b"[Console]::WriteLine('COPY_STATE_'+$global:RocketCopyState)\r")
            .unwrap();
        wait(&terminal.session, |s| {
            s.lines().any(|l| l.trim() == "COPY_STATE_73")
        });
        assert_eq!(terminal.session.status().pid, pid);
        assert_eq!(
            terminal
                .events
                .iter()
                .filter(|&&e| e == "terminal_interrupt")
                .count(),
            1
        );
        terminal.stop();
        let stopped = Instant::now();
        while !terminal.session.status().closed {
            assert!(stopped.elapsed() < Duration::from_secs(10));
            thread::sleep(Duration::from_millis(20));
        }
        terminal.poll(false);
        assert!(terminal.status.closed && terminal.events.contains(&"terminal_exit"));
        fs::remove_file(root.join("history.log")).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn drawer_shows_whole_terminal_rows() {
        use crate::test_common as common;
        common::isolated(
            "ROCKET_TERMINAL_UI_CASE",
            "terminal::tests::drawer_shows_whole_terminal_rows",
            || {
                let fixture = common::Fixture::new();
                fixture.write("file", "first");
                fixture.commit("first");
                let ctx = Context::default();
                let mut app = crate::app::Rocket::with_log_directory(
                    &ctx,
                    vec![fixture.repository.clone()],
                    fixture.root.join("logs"),
                );
                let deadline = Instant::now() + Duration::from_secs(40);
                while app.busy() {
                    app.poll(&ctx);
                    assert!(Instant::now() < deadline);
                    thread::sleep(Duration::from_millis(10));
                }
                app.auto_refresh = false;
                app.open_terminal();
                // 780x620 is the minimum window: the drawer leaves less than four rows.
                for size in [vec2(780.0, 620.0), vec2(900.0, 700.0), vec2(1480.0, 920.0)] {
                    let mut cell = 0.0;
                    for _ in 0..4 {
                        app.poll(&ctx);
                        app.frame_widgets.clear();
                        let mut output = ctx.run_ui(
                            RawInput {
                                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                                ..Default::default()
                            },
                            |ui| {
                                cell = ui.fonts_mut(|f| {
                                    f.row_height(&FontId::monospace(13.0)).max(18.0)
                                });
                                app.shell(ui, &mut Vec::new())
                            },
                        );
                        output.textures_delta.clear();
                    }
                    let screen = app.frame_widgets["terminal-screen"].rect;
                    let rows = screen.height() / cell;
                    assert!(
                        rows >= 1.0 && (rows - rows.round()).abs() < 0.01,
                        "partial terminal row at {size:?}: {screen:?}, row height {cell}"
                    );
                }
            },
        );
    }

    /// A repository open in a window of the real drawer; `frame` draws it once.
    fn with_drawer(test: impl FnOnce(&Context, &mut crate::app::Rocket, &crate::test_common::Fixture)) {
        let fixture = crate::test_common::Fixture::new();
        fixture.write("file", "first");
        fixture.commit("first");
        let ctx = Context::default();
        let mut app = crate::app::Rocket::with_log_directory(&ctx, vec![fixture.repository.clone()], fixture.root.join("logs"));
        let deadline = Instant::now() + Duration::from_secs(40);
        while app.busy() {
            app.poll(&ctx);
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
        app.auto_refresh = false;
        test(&ctx, &mut app, &fixture);
    }
    fn frame(ctx: &Context, app: &mut crate::app::Rocket, events: Vec<Event>) {
        app.poll(ctx);
        app.frame_widgets.clear();
        let mut output = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1480.0, 920.0))),
                events,
                ..Default::default()
            },
            |ui| {
                app.shell(ui, &mut Vec::new());
                let ctx = ui.ctx().clone();
                app.dialog_ui(&ctx);
            },
        );
        output.textures_delta.clear();
    }
    fn click(ctx: &Context, app: &mut crate::app::Rocket, id: &str) {
        let at = app.frame_widgets[id].rect.center();
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        frame(ctx, app, vec![Event::PointerMoved(at), button(true)]);
        frame(ctx, app, vec![button(false)]);
    }
    fn until_closed(ctx: &Context, app: &mut crate::app::Rocket) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !app.tabs[0].terminal.as_ref().unwrap().status.closed {
            frame(ctx, app, vec![]);
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(20));
        }
        frame(ctx, app, vec![]);
        frame(ctx, app, vec![]);
    }

    #[test]
    fn a_missing_shell_says_which_file_and_leads_to_the_setting() {
        crate::test_common::isolated("ROCKET_TERMINAL_UI_CASE", "terminal::tests::a_missing_shell_says_which_file_and_leads_to_the_setting", || {
            with_drawer(|ctx, app, fixture| {
                use crate::terminal_settings::{Choice, Settings};
                let program = fixture.root.join("no-such-shell.exe");
                app.terminal_shell.load(Settings { choice: Choice::Custom, program: program.display().to_string(), arguments: String::new() });
                app.open_terminal();
                until_closed(ctx, app);
                let terminal = app.tabs[0].terminal.as_ref().unwrap();
                assert_eq!(terminal.name, "no-such-shell");
                let (title, detail, retry) = terminal.failure().expect("it never ran");
                assert_eq!(title, "no-such-shell didn't start");
                assert!(detail.contains("no-such-shell.exe") && detail.ends_with("was not found."), "{detail}");
                assert!(retry);
                assert_eq!(app.terminal_state(), Some((false, "didn't start".to_owned())));
                assert!(!app.frame_widgets.contains_key("terminal-screen"), "there is no screen to show");
                assert!(!app.frame_widgets["terminal-end"].enabled);
                assert!(!app.frame_widgets["terminal-clear-scrollback"].enabled);
                click(ctx, app, "terminal-try-again");
                until_closed(ctx, app);
                assert!(app.tabs[0].terminal.as_ref().unwrap().failure().is_some(), "still missing");
                click(ctx, app, "terminal-settings");
                assert_eq!(app.preferences_page(), Some(crate::preferences::Page::Terminal));
            });
        });
    }

    #[test]
    fn a_shell_that_stops_at_once_says_so_above_its_own_output() {
        crate::test_common::isolated("ROCKET_TERMINAL_UI_CASE", "terminal::tests::a_shell_that_stops_at_once_says_so_above_its_own_output", || {
            with_drawer(|ctx, app, _| {
                use crate::terminal_settings::{Choice, Settings};
                let program = rocket_terminal::Shell::CommandPrompt.program().unwrap();
                app.terminal_shell.load(Settings {
                    choice: Choice::Custom,
                    program: program.display().to_string(),
                    arguments: "/C\necho SAID_WHY & exit 3".into(),
                });
                app.open_terminal();
                until_closed(ctx, app);
                let terminal = app.tabs[0].terminal.as_ref().unwrap();
                assert_eq!(terminal.stopped_early(), Some(3));
                assert!(terminal.failure().is_none(), "it did run");
                assert_eq!(app.terminal_state(), Some((false, "exited (3)".to_owned())));
                assert!(app.frame_widgets.contains_key("terminal-settings"));
                assert!(app.frame_widgets.contains_key("terminal-screen"), "its output stays");
                assert!(!app.frame_widgets.contains_key("terminal-try-again"));
            });
        });
    }

    #[test]
    fn command_prompt_refuses_a_network_folder_and_says_why() {
        crate::test_common::isolated("ROCKET_TERMINAL_UI_CASE", "terminal::tests::command_prompt_refuses_a_network_folder_and_says_why", || {
            with_drawer(|ctx, app, _| {
                use crate::terminal_settings::{Choice, Settings};
                app.terminal_shell.load(Settings { choice: Choice::CommandPrompt, ..Default::default() });
                let repository = std::mem::replace(&mut app.tabs[0].path, r"\\server\share\rocket".into());
                app.open_terminal();
                until_closed(ctx, app);
                let terminal = app.tabs[0].terminal.as_ref().unwrap();
                let (title, detail, retry) = terminal.failure().expect("nothing started");
                assert_eq!(title, "Command Prompt can't run in a network folder");
                assert_eq!(detail, r"\\server\share\rocket");
                assert!(!retry, "trying again can't help");
                assert!(app.frame_widgets.contains_key("terminal-settings"));
                assert!(!app.frame_widgets.contains_key("terminal-try-again"));
                app.tabs[0].path = repository;
            });
        });
    }

    #[test]
    fn the_drawer_names_the_session_and_an_mcp_window_keeps_windows_powershell() {
        crate::test_common::isolated("ROCKET_TERMINAL_UI_CASE", "terminal::tests::the_drawer_names_the_session_and_an_mcp_window_keeps_windows_powershell", || {
            with_drawer(|ctx, app, fixture| {
                use crate::terminal_settings::{Choice, Settings};
                app.terminal_shell.load(Settings { choice: Choice::CommandPrompt, ..Default::default() });
                assert_eq!(app.terminal_session_name(), "Command Prompt", "the next session, before one starts");
                app.open_terminal();
                let deadline = Instant::now() + Duration::from_secs(15);
                while !app.tabs[0].terminal.as_ref().unwrap().printed {
                    frame(ctx, app, vec![]);
                    assert!(Instant::now() < deadline, "no prompt");
                    thread::sleep(Duration::from_millis(20));
                }
                // A running session keeps its name when the setting changes.
                app.terminal_shell.load(Settings::default());
                assert_eq!(app.terminal_session_name(), "Command Prompt");
                app.terminal_action(TerminalAction::End);
                until_closed(ctx, app);
                app.tabs[0].terminal = None;

                // An MCP window's history file marks it: its setting is ignored.
                app.terminal_shell.load(Settings { choice: Choice::CommandPrompt, ..Default::default() });
                app.terminal_history_file = Some(fixture.root.join("history.log"));
                assert_eq!(app.terminal_name(), "Windows PowerShell");
                app.open_terminal();
                assert_eq!(app.tabs[0].terminal.as_ref().unwrap().name, "Windows PowerShell");
                let deadline = Instant::now() + Duration::from_secs(20);
                while !app.tabs[0].terminal.as_ref().unwrap().screen.text().contains("PS ") {
                    frame(ctx, app, vec![]);
                    assert!(Instant::now() < deadline, "no PowerShell prompt");
                    thread::sleep(Duration::from_millis(20));
                }
                app.terminal_action(TerminalAction::End);
                until_closed(ctx, app);
            });
        });
    }

    #[test]
    fn unicode_clusters_and_markers_paint_on_the_same_cell_grid() {
        let ctx = Context::default();
        ctx.set_fonts(crate::fonts::definitions());
        let mut screen = Screen::new(40, 6);
        screen.feed("abcd|\r\n中文|\r\n🚀🎉|\r\néöab|\r\n👩🏽‍💻ab|\r\n1️⃣ab|".as_bytes());
        let mut cell_size = Vec2::ZERO;
        let origin = pos2(10.0, 10.0);
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let font = FontId::monospace(13.0);
            cell_size = ui.fonts_mut(|f| vec2(f.glyph_width(&font, 'M'), 18.0));
            for (row, cells) in screen.cells.iter().enumerate() {
                paint_row(
                    ui.painter(),
                    cells,
                    origin + vec2(0.0, row as f32 * cell_size.y),
                    cell_size,
                    &font,
                );
            }
        });
        output.textures_delta.clear();
        let mut markers = Vec::new();
        let mut clusters = Vec::new();
        for shape in &output.shapes {
            if let epaint::Shape::Text(text) = &shape.shape {
                let value = text.galley.text();
                if value == "|" {
                    markers.push(text.pos.x);
                }
                if ["中", "🚀", "é", "👩🏽‍💻", "1️⃣"].contains(&value) {
                    clusters.push(value.to_owned());
                    assert!(text.galley.size().x <= 2.0 * cell_size.x + 0.1, "{value}");
                    assert!(text.pos.x >= origin.x && text.pos.x < origin.x + cell_size.x);
                    if ["👩🏽‍💻", "1️⃣"].contains(&value) {
                        let advancing = text
                            .galley
                            .rows
                            .iter()
                            .flat_map(|r| &r.glyphs)
                            .filter(|g| g.advance_width > 0.0)
                            .count();
                        assert_eq!(advancing, 1, "{value} must shape as one visible glyph");
                    }
                }
            }
        }
        assert_eq!(markers.len(), 6);
        assert_eq!(clusters.len(), 5);
        assert!(markers.iter().all(|x| (*x - markers[0]).abs() < 0.01));
        assert!(markers[0] >= origin.x + 4.0 * cell_size.x);
    }

    #[test]
    fn terminal_owns_navigation_keys_and_enter_submits_the_real_text_widget() {
        let root = std::env::temp_dir().join(format!(
            "rocket-terminal-ui-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let session =
            Session::start_with_history(&root, 100, 12, Arc::new(|| {}), &root.join("history.log"))
                .unwrap();
        let started = Instant::now();
        while !session
            .snapshot(0)
            .is_some_and(|s| s.text().contains("PS "))
        {
            assert!(session.status().error.is_none());
            assert!(started.elapsed() < Duration::from_secs(10));
            thread::sleep(Duration::from_millis(20));
        }
        let mut terminal = TerminalPane {
            status: session.status(),
            events: Vec::new(),
            ..TerminalPane::new(session, "Windows PowerShell".into(), false)
        };
        let ctx = Context::default();
        ctx.set_fonts(crate::fonts::definitions());
        let mut widgets = HashMap::new();
        let mut render = |terminal: &mut TerminalPane, events| {
            let mut output = ctx.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 500.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    terminal.show(ui, 1, &mut widgets, false);
                },
            );
            output.textures_delta.clear();
        };
        render(&mut terminal, vec![]);
        let key = |key, pressed| Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        terminal.command = "Write-Output ('ROCKET_' + 'ENTER_WIDGET')".into();
        ctx.memory_mut(|m| m.request_focus(command_id(1)));
        render(&mut terminal, vec![]);
        render(
            &mut terminal,
            vec![key(Key::Enter, true), key(Key::Enter, false)],
        );
        assert!(terminal.command.is_empty());
        let start = Instant::now();
        while !terminal
            .session
            .snapshot(0)
            .is_some_and(|s| s.text().contains("ROCKET_ENTER_WIDGET"))
        {
            assert!(start.elapsed() < Duration::from_secs(15));
            thread::sleep(Duration::from_millis(20));
        }
        let click = ctx.read_response(screen_id(1)).unwrap().rect.center();
        render(
            &mut terminal,
            vec![
                Event::PointerMoved(click),
                Event::PointerButton {
                    pos: click,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::NONE,
                },
                Event::PointerButton {
                    pos: click,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Modifiers::NONE,
                },
            ],
        );
        assert!(
            ctx.memory(|m| m.has_focus(screen_id(1))),
            "Clicking the terminal screen must focus it"
        );
        render(
            &mut terminal,
            vec![
                Event::Text("Write-Output ('ROCKET_' + 'SCREEN_INPUT')".into()),
                key(Key::Enter, true),
                key(Key::Enter, false),
            ],
        );
        assert!(
            ctx.memory(|m| m.has_focus(screen_id(1))),
            "Enter on the screen must not return focus to the command field"
        );
        let screen_input = Instant::now();
        while !terminal
            .session
            .snapshot(0)
            .is_some_and(|s| s.text().lines().any(|l| l == "ROCKET_SCREEN_INPUT"))
        {
            assert!(
                screen_input.elapsed() < Duration::from_secs(10),
                "{}",
                terminal.session.snapshot(0).unwrap().text()
            );
            thread::sleep(Duration::from_millis(20));
        }
        render(&mut terminal, vec![]);
        render(&mut terminal, vec![]);
        for navigation in [Key::Tab, Key::ArrowLeft, Key::ArrowUp, Key::Escape] {
            render(
                &mut terminal,
                vec![key(navigation, true), key(navigation, false)],
            );
            assert!(
                ctx.memory(|m| m.has_focus(screen_id(1))),
                "{navigation:?} escaped the terminal"
            );
        }
        terminal.stop();
        while !terminal.session.status().closed {
            assert!(start.elapsed() < Duration::from_secs(20));
            thread::sleep(Duration::from_millis(20));
        }
        fs::remove_file(root.join("history.log")).unwrap();
        fs::remove_dir(root).unwrap();
    }
}

#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use super::*;
    use crate::terminal_settings::{Choice, Settings};
    use std::{
        thread,
        time::{Duration, Instant},
    };

    /// A repository open in a window of the real drawer; `frame` draws it once.
    fn with_drawer(test: impl FnOnce(&Context, &mut crate::app::Rocket, &crate::test_common::Fixture)) {
        let fixture = crate::test_common::Fixture::new();
        fixture.write("file", "first");
        fixture.commit("first");
        let ctx = Context::default();
        let mut app = crate::app::Rocket::with_log_directory(&ctx, vec![fixture.repository.clone()], fixture.root.join("logs"));
        let deadline = Instant::now() + Duration::from_secs(40);
        while app.busy() {
            app.poll(&ctx);
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
        app.auto_refresh = false;
        test(&ctx, &mut app, &fixture);
    }
    fn frame(ctx: &Context, app: &mut crate::app::Rocket) {
        app.poll(ctx);
        app.frame_widgets.clear();
        let mut output = ctx.run_ui(
            RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1480.0, 920.0))), ..Default::default() },
            |ui| app.shell(ui, &mut Vec::new()),
        );
        output.textures_delta.clear();
    }
    fn until(ctx: &Context, app: &mut crate::app::Rocket, what: &str, done: impl Fn(&TerminalPane) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !done(app.tabs[0].terminal.as_ref().unwrap()) {
            frame(ctx, app);
            assert!(Instant::now() < deadline, "{what}: {}", app.tabs[0].terminal.as_ref().unwrap().screen.text());
            thread::sleep(Duration::from_millis(20));
        }
        frame(ctx, app);
    }

    #[test]
    fn the_drawer_runs_the_chosen_linux_shell_in_the_repository_and_an_mcp_window_runs_bash() {
        crate::test_common::isolated("ROCKET_TERMINAL_UI_CASE", "terminal::linux_tests::the_drawer_runs_the_chosen_linux_shell_in_the_repository_and_an_mcp_window_runs_bash", || {
            with_drawer(|ctx, app, fixture| {
                app.terminal_shell.load(Settings { choice: Choice::Bash, ..Default::default() });
                assert_eq!(app.terminal_session_name(), "Bash");
                app.open_terminal();
                until(ctx, app, "bash output", |t| t.printed);
                let terminal = app.tabs[0].terminal.as_ref().unwrap();
                assert_eq!(terminal.name, "Bash");
                terminal.session.input(b"echo ROCKET_IN_$(basename \"$PWD\")\r").unwrap();
                let folder = fixture.repository.file_name().unwrap().to_string_lossy().into_owned();
                until(ctx, app, "command output", |t| t.screen.text().lines().any(|l| l == format!("ROCKET_IN_{folder}")));
                assert_eq!(app.terminal_state(), Some((true, "running".to_owned())));
                app.terminal_action(TerminalAction::End);
                until(ctx, app, "end", |t| t.status.closed);
                assert_eq!(app.tabs[0].terminal.as_ref().unwrap().status.error, None);
                app.tabs[0].terminal = None;

                // An MCP window's history file marks it: its setting is ignored.
                app.terminal_history_file = Some(fixture.root.join("history.log"));
                assert_eq!(app.terminal_name(), "bash");
                app.open_terminal();
                assert_eq!(app.tabs[0].terminal.as_ref().unwrap().name, "bash");
                until(ctx, app, "Rocket's bash prompt", |t| t.screen.text().trim_end().ends_with("\n>"));
                app.terminal_action(TerminalAction::End);
                until(ctx, app, "end", |t| t.status.closed);
            });
        });
    }

    #[test]
    fn a_missing_linux_shell_says_which_file() {
        crate::test_common::isolated("ROCKET_TERMINAL_UI_CASE", "terminal::linux_tests::a_missing_linux_shell_says_which_file", || {
            with_drawer(|ctx, app, fixture| {
                let program = fixture.root.join("no-such-shell");
                app.terminal_shell.load(Settings { choice: Choice::Custom, program: program.display().to_string(), arguments: String::new() });
                app.open_terminal();
                until(ctx, app, "closed", |t| t.status.closed);
                let (title, detail, retry) = app.tabs[0].terminal.as_ref().unwrap().failure().expect("it never ran");
                assert_eq!(title, "no-such-shell didn't start");
                assert!(detail.contains("no-such-shell") && detail.ends_with("was not found."), "{detail}");
                assert!(retry);
            });
        });
    }
}

#[cfg(all(test, any(windows, target_os = "linux", target_os = "macos")))]
mod layout_tests {
    use super::*;
    use std::{
        thread,
        time::{Duration, Instant},
    };

    #[test]
    fn the_command_line_sits_under_whole_rows_even_in_the_shortest_drawer() {
        crate::test_common::isolated(
            "ROCKET_TERMINAL_UI_CASE",
            "terminal::layout_tests::the_command_line_sits_under_whole_rows_even_in_the_shortest_drawer",
            || {
                let fixture = crate::test_common::Fixture::new();
                fixture.write("file", "first");
                fixture.commit("first");
                let ctx = Context::default();
                let mut app = crate::app::Rocket::with_log_directory(&ctx, vec![fixture.repository.clone()], fixture.root.join("logs"));
                let deadline = Instant::now() + Duration::from_secs(40);
                while app.busy() {
                    app.poll(&ctx);
                    assert!(Instant::now() < deadline);
                    thread::sleep(Duration::from_millis(10));
                }
                app.auto_refresh = false;
                app.open_terminal();
                // 500 high leaves the drawer only its least height; 920 gives it its usual one.
                for size in [vec2(1000.0, 500.0), vec2(1480.0, 920.0)] {
                    let mut cell = 0.0;
                    let mut tops = Vec::new();
                    for _ in 0..24 {
                        app.poll(&ctx);
                        app.frame_widgets.clear();
                        let mut output = ctx.run_ui(
                            RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), ..Default::default() },
                            |ui| {
                                cell = ui.fonts_mut(|f| f.row_height(&FontId::monospace(13.0)).max(18.0));
                                app.shell(ui, &mut Vec::new())
                            },
                        );
                        output.textures_delta.clear();
                        tops.push(app.frame_widgets["drawer-terminal"].rect.top());
                    }
                    // The drawer keeps its height: it neither creeps up over the window nor
                    // shrinks to its least height, frame after frame.
                    assert!(tops[3..].iter().all(|top| (top - tops[3]).abs() < 0.5), "the drawer moved at {size:?}: {tops:?}");
                    let [screen, command, send] =
                        ["terminal-screen", "terminal-command", "terminal-send"].map(|key| app.frame_widgets[key].rect);
                    let rows = screen.height() / cell;
                    assert!(rows > 3.99 && (rows - rows.round()).abs() < 0.01, "{rows} rows at {size:?}");
                    assert!(command.top() >= screen.bottom() && command.top() - screen.bottom() < 12.0, "the command line right under the screen at {size:?}: {command:?}, {screen:?}");
                    // Inside the drawer's 8-point margin, above the 26-point status bar.
                    assert!(send.bottom() <= size.y - 26.0 - 8.0 + 0.5, "{send:?} at {size:?}");
                    // Send is as tall as the field, beside it, and ends where the screen does.
                    assert!((send.height() - CONTROL).abs() < 0.5 && (send.center().y - command.center().y).abs() < 1.0, "{send:?}, {command:?}");
                    assert!(send.left() > command.right() && send.left() - command.right() < 24.0, "{send:?}, {command:?}");
                    assert!((send.right() - screen.right()).abs() < 12.0, "{send:?}, {screen:?}");
                }
                app.terminal_action(TerminalAction::End);
                let deadline = Instant::now() + Duration::from_secs(20);
                while !app.tabs[0].terminal.as_ref().unwrap().session.status().closed {
                    assert!(Instant::now() < deadline, "the shell ends");
                    thread::sleep(Duration::from_millis(20));
                }
            },
        );
    }
}
