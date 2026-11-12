//! The Agents page: one header row, the session list, and the selected session with its
//! terminal, changes, integration steps and details. Canvas section 39.
use super::*;
use crate::{
    app::{Rocket, UiControl, View},
    i18n::{tr, trf},
    pages,
    style::*,
};
use eframe::egui::*;
use std::collections::HashMap;

mod changes;
mod details;
mod form;
mod integrate;
mod list;
mod parts;
#[cfg(test)]
mod tests;
use parts::{Look, action as button, child};

type Widgets = HashMap<String, UiControl>;

/// The page header's height.
const HEAD: f32 = 48.0;
/// Below this width the session list becomes a switcher above the selected session.
const NARROW: f32 = 760.0;
/// The selected session's content starts here, in line with `pages::tabs`.
const INSET: f32 = 24.0;

enum Action {
    New,
    Preview,
    Confirm,
    Cancel,
    /// Browse for the session dialog's program.
    Browse,
    Stop(u64),
    Restart,
    Resume,
    Acknowledge,
    CheckIntegration,
    StopIntegration,
    CompareIntegration,
    MergeIntegration,
    Remove,
    TerminalSettings,
    Open(PathBuf),
    Forget,
    Refresh,
    StopRead,
    File(usize),
    DismissNotice,
}

fn duration(d: Duration) -> String {
    let s = d.as_secs();
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

impl Rocket {
    pub(crate) fn agents_view(&mut self, ui: &mut Ui) {
        let ready = self.profile_storage_ready() && !self.quitting && self.write.is_none();
        let source = self
            .tab()
            .filter(|t| !t.is_preview() && t.worktree())
            .map(|t| t.path.clone());
        let can_start = ready && source.is_some() && self.agents.sessions.len() < MAX_SESSIONS;
        let mut action = None;
        ui.add_enabled_ui(self.agents.form.is_none(), |ui| {
            // A least height, as before: in a very short pane the page is cut at the
            // bottom, and no part of it is given a negative size.
            let mut page = ui.available_rect_before_wrap();
            page.max.y = page.max.y.max(page.min.y + HEAD + 80.0);
            ui.allocate_rect(page, Sense::hover());
            let head = Rect::from_min_size(page.min, vec2(page.width(), HEAD));
            page_head(&mut child(ui, "agents-head", head), &self.agents, &mut self.frame_widgets, can_start, &mut action);
            ui.painter()
                .hline(page.x_range(), head.bottom() - 0.5, Stroke::new(1.0, HAIRLINE()));
            let mut body = Rect::from_min_max(pos2(page.left(), head.bottom()), page.max);
            // What went wrong or what just happened, above everything it concerns.
            if self.state_error.is_some() || self.agents.notice.is_some() {
                let mut notes = child(ui, "agents-notes", body.shrink2(vec2(18.0, 0.0)));
                notes.add_space(10.0);
                if let Some(error) = &self.state_error {
                    parts::strip(&mut notes, RED(), Icon::Warning, error, None);
                }
                if let Some(notice) = &self.agents.notice {
                    let removed = notice == "Worktree removed. The branch and recovery reference are kept.";
                    let (color, image, text) = if removed {
                        (GREEN(), Icon::Check, tr("Worktree removed. The branch and recovery reference are kept."))
                    } else {
                        (AMBER(), Icon::Warning, notice.as_str())
                    };
                    if parts::strip(&mut notes, color, image, text, Some((&mut self.frame_widgets, "agents-notice-close"))) {
                        action = Some(Action::DismissNotice);
                    }
                }
                body.min.y = notes.min_rect().bottom().min(body.bottom());
            }
            if self.agents.sessions.is_empty() {
                empty(ui, body, &mut self.frame_widgets, can_start, &mut action);
                return;
            }
            let narrow = body.width() < NARROW;
            let selected = if narrow {
                body
            } else {
                let width = (body.width() * 0.26).clamp(240.0, 300.0);
                let rail = Rect::from_min_max(body.min, pos2(body.left() + width, body.bottom()));
                ui.painter()
                    .vline(rail.right() - 0.5, body.y_range(), Stroke::new(1.0, HAIRLINE()));
                list::rail(
                    &mut child(ui, "agents-list", rail.shrink2(vec2(8.0, 0.0))),
                    &mut self.agents,
                    &mut self.frame_widgets,
                );
                Rect::from_min_max(pos2(rail.right(), body.top()), body.max)
            };
            workspace(
                &mut child(ui, "agents-workspace", selected),
                &mut self.agents,
                &mut self.frame_widgets,
                ready,
                narrow,
                &mut action,
            );
        });
        let covered = self.picker.is_some();
        match &mut self.agents.form {
            Some(f) => {
                let chosen = Chosen::of(&self.drafting, &self.agents.copilot);
                form::show(ui.ctx(), &mut self.frame_widgets, f, chosen, ready, covered, &mut action);
            }
            None => form::forget(ui.ctx()),
        }
        let result = match action {
            Some(Action::New) => {
                if let Some(source) = source {
                    self.agents.new_form(source, &self.drafting);
                }
                Ok(())
            }
            Some(Action::Browse) => {
                if let Some(f) = &self.agents.form {
                    let current = f.record.command.program.clone();
                    self.browse(crate::folder_browser::PickTarget::AgentProgram, &current);
                }
                Ok(())
            }
            Some(Action::Preview) => self
                .backend
                .as_ref()
                .map_or(Ok(()), |b| self.agents.preview(b, &self.context)),
            Some(Action::Confirm) => self.agents.confirm(),
            Some(Action::Cancel) => {
                self.agents.form = None;
                Ok(())
            }
            Some(Action::Stop(id)) => {
                if let Some(s) = self.agents.sessions.iter_mut().find(|s| s.record.id == id) {
                    s.stop();
                    self.terminal_log.push("agent_stop_requested", false);
                }
                Ok(())
            }
            Some(Action::Restart) => {
                self.agents.existing_form(Mode::Restart);
                Ok(())
            }
            Some(Action::Resume) => self
                .backend
                .as_ref()
                .map_or(Ok(()), |b| self.agents.resume(b, &self.context)),
            Some(Action::Acknowledge) => {
                if let Some(s) = self
                    .agents
                    .sessions
                    .iter_mut()
                    .find(|s| Some(s.record.id) == self.agents.selected)
                {
                    s.attention = false;
                    s.record.attention = false;
                }
                Ok(())
            }
            Some(Action::CheckIntegration) => {
                if let (Some(b), Some(s)) = (self.backend.clone(), self.agents.selected()) {
                    if s.active() {
                        Err(model::invalid("Stop the agent before checking integration."))
                    } else {
                        self.agents.integration.start(b, s.record.clone(), &self.context)
                    }
                } else {
                    Ok(())
                }
            }
            Some(Action::StopIntegration) => {
                self.agents.integration.stop();
                Ok(())
            }
            Some(Action::CompareIntegration) | Some(Action::MergeIntegration) => {
                let merge = matches!(action, Some(Action::MergeIntegration));
                self.agent_integration_handoff(merge)
            }
            Some(Action::Remove) => {
                self.agents.existing_form(Mode::Remove);
                Ok(())
            }
            Some(Action::TerminalSettings) => {
                self.show_preferences(crate::preferences::Page::Terminal);
                Ok(())
            }
            Some(Action::Open(path)) => {
                self.open(path);
                if let Some(t) = self.tabs.get_mut(self.active) {
                    t.view = View::Changes;
                }
                Ok(())
            }
            Some(Action::Forget) => {
                self.agents
                    .sessions
                    .retain(|s| Some(s.record.id) != self.agents.selected || s.active());
                self.agents.selected = self.agents.sessions.first().map(|s| s.record.id);
                Ok(())
            }
            Some(Action::Refresh) => {
                if let Some(b) = &self.backend {
                    self.agents.changes.refresh(b, &self.context);
                }
                Ok(())
            }
            Some(Action::StopRead) => {
                self.agents.changes.stop();
                Ok(())
            }
            Some(Action::File(i)) => {
                if let Some(b) = &self.backend {
                    self.agents.changes.select(i, b, &self.context);
                }
                Ok(())
            }
            Some(Action::DismissNotice) => {
                self.agents.notice = None;
                Ok(())
            }
            None => Ok(()),
        };
        if let Err(e) = result {
            if let Some(f) = &mut self.agents.form {
                f.error = Some(user_error(&e));
            } else {
                self.agents.notice = Some(user_error(&e));
            }
        }
    }

    /// Browse chose the session dialog's program. For a preset it is also where that
    /// program is found from now on: Claude Code's and Codex's are the AI features' own
    /// settings, GitHub Copilot's is kept with Agents. Your own program has no preset
    /// to keep one for.
    pub(crate) fn agent_program_picked(&mut self, program: String) {
        let Some(f) = &mut self.agents.form else {
            return;
        };
        f.record.command.program.clone_from(&program);
        // Look again: the card of a program that is there now says Found.
        f.found = None;
        // Only a path a session could start is kept; review says what is wrong with
        // any other.
        if !rocket_git::valid_custom_program_path(std::path::Path::new(&program)) {
            return;
        }
        use rocket_git::DraftProvider;
        match f.record.provider {
            Provider::Claude => *self.drafting.program_setting_of_mut(DraftProvider::Claude) = program,
            Provider::Codex => *self.drafting.program_setting_of_mut(DraftProvider::Codex) = program,
            Provider::Copilot => self.agents.copilot = program,
            Provider::Custom => {}
        }
    }

    fn agent_integration_handoff(&mut self, merge: bool) -> Result<()> {
        let s = self
            .agents
            .selected()
            .ok_or_else(|| model::invalid("Select a session."))?;
        let r = self
            .agents
            .integration
            .review
            .as_ref()
            .ok_or_else(|| model::invalid("Check integration first."))?;
        if s.active() || s.record != r.owner {
            return Err(model::invalid("The session changed. Stop it and check integration again."));
        }
        let record = s.record.clone();
        let preview = r.merge.clone();
        let comparison = r.comparison.clone();
        self.open(record.source);
        let tab = self
            .tabs
            .get_mut(self.active)
            .ok_or_else(|| model::invalid("Open the original checkout."))?;
        if tab.snapshot.is_none() {
            return Err(model::invalid(
                "The original checkout is loading. Return to Agents and check integration when it is ready.",
            ));
        }
        if merge {
            self.show_dialog(crate::app::DialogKind::Merge);
            if let Some(d) = &mut self.dialog {
                d.first = preview.revision().into();
                d.mode = preview.mode();
                d.merge = Some(crate::merge::DialogState { preview: Some(preview), ..Default::default() });
            }
            self.terminal_log.push("agent_integration_reviewed", false);
        } else {
            tab.compare.base = comparison.base.to_string();
            tab.compare.target = comparison.target.to_string();
            tab.compare.local_staged = None;
            self.compare_intent(crate::compare::CompareIntent::Open);
            self.compare_intent(crate::compare::CompareIntent::Load);
        }
        Ok(())
    }
}

/// The page's one header row: its name, how many sessions run and how many wait for the
/// user, the Guide and New session.
fn page_head(ui: &mut Ui, state: &State, w: &mut Widgets, can_start: bool, action: &mut Option<Action>) {
    let rect = ui.max_rect();
    let inner = Rect::from_min_max(pos2(rect.left() + 18.0, rect.top()), pos2(rect.right() - 16.0, rect.bottom()));
    let mut row = ui.new_child(
        UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::right_to_left(Align::Center)),
    );
    row.spacing_mut().item_spacing.x = 8.0;
    let r = button(&mut row, Look::Primary, Some(Icon::Plus), false, tr("New session"), can_start);
    pages::control(w, "agents-new", &r, ui.clip_rect());
    if r.clicked() {
        *action = Some(Action::New);
    }
    crate::guide::header_button(&mut row);
    row.with_layout(Layout::left_to_right(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.label(RichText::new(tr("Agents")).font(semibold(16.0)));
        let running = state.sessions.iter().filter(|s| s.active()).count();
        let waiting = state.sessions.iter().filter(|s| s.attention).count();
        let font = FontId::proportional(12.0);
        let mut job = text::LayoutJob::default();
        if running > 0 {
            job.append(&trf!("{n} running", n = running), 0.0, TextFormat::simple(font.clone(), HINT()));
        }
        if waiting > 0 {
            if running > 0 {
                job.append(" · ", 0.0, TextFormat::simple(font.clone(), HINT()));
            }
            job.append(&trf!("{n} waiting for you", n = waiting), 0.0, TextFormat::simple(font, AMBER()));
        }
        if !job.is_empty() {
            ui.add(Label::new(job).truncate());
        }
    });
}

/// No sessions yet: what the page is for and the way to start.
fn empty(ui: &mut Ui, rect: Rect, w: &mut Widgets, can_start: bool, action: &mut Option<Action>) {
    let clip = ui.clip_rect();
    parts::centered(
        ui,
        rect,
        Icon::Prompt,
        BLUE(),
        tr("Give each task its own worktree"),
        tr("Run Claude Code, Codex, GitHub Copilot or your own program in a separate folder. This checkout stays as it is."),
        14.0 + CONTROL + 8.0 + 22.0,
        |ui| {
            ui.add_space(14.0);
            let r = button(ui, Look::Primary, Some(Icon::Plus), false, tr("New session"), can_start);
            pages::control(w, "agents-start", &r, clip);
            if r.clicked() {
                *action = Some(Action::New);
            }
            ui.add_space(8.0);
            // 22 points tall, so it is easy to hit and automation counts it as shown.
            let link = ui.add(
                Button::new(RichText::new(tr("How agents work")).size(12.5).color(ACCENT_TEXT()))
                    .frame(false)
                    .min_size(vec2(0.0, 22.0)),
            );
            pages::control(w, "agents-guide", &link, clip);
            if link.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                crate::guide::request(ui.ctx());
            }
        },
    );
}

/// The selected session: its header, the tabs and the pane they choose.
fn workspace(ui: &mut Ui, state: &mut State, w: &mut Widgets, ready: bool, narrow: bool, action: &mut Option<Action>) {
    ui.spacing_mut().item_spacing.y = 0.0;
    if narrow {
        list::switcher(ui, state, w);
    }
    let Some(s) = state
        .sessions
        .iter_mut()
        .find(|s| Some(s.record.id) == state.selected)
    else {
        return;
    };
    session_head(ui, s, w, ready, narrow, action);
    let files = state
        .changes
        .loaded
        .as_ref()
        .filter(|_| state.changes.owner.as_ref().is_some_and(|o| o.id == s.record.id))
        .map(|l| l.files.len());
    let panes = [
        (Pane::Terminal, "Terminal", "agents-tab-terminal", None),
        (Pane::Changes, "Changes", "agents-tab-changes", files),
        (Pane::Integrate, "Integrate", "agents-tab-integrate", None),
        (Pane::Details, "Details", "agents-tab-details", None),
    ];
    let items = panes.map(|(pane, label, key, count)| pages::TabItem {
        label: tr(label),
        count,
        selected: state.pane == pane,
        enabled: true,
        key,
    });
    if let Some(i) = pages::tabs(ui, &items, w) {
        state.pane = panes[i].0;
    }
    // `tabs` reports what it drew; under the session dialog the whole page is disabled.
    for (_, _, key, _) in panes {
        if let Some(c) = w.get_mut(key) {
            c.enabled &= ui.is_enabled();
        }
    }
    let (rect, _) = ui.allocate_exact_size(ui.available_size().max(Vec2::ZERO), Sense::hover());
    let mut body = child(ui, "agents-selected", rect);
    match state.pane {
        Pane::Terminal => terminal(&mut body, s, w, ready, narrow, action),
        Pane::Changes => {
            state.changes.select_owner(&s.record);
            changes::show(&mut body, &state.changes, w, &s.record.destination, ready, action);
        }
        Pane::Integrate => integrate::show(&mut body, s, &state.integration, w, ready, action),
        Pane::Details => details::show(&mut body, s, w, ready, action),
    }
}

/// The session's name with its state, the one action that fits that state, and under
/// them the program, branch, base and how long it has run. In a narrow window the
/// switcher above already names the session, so this is one row: state, those facts
/// and the action.
fn session_head(ui: &mut Ui, s: &Session, w: &mut Widgets, ready: bool, narrow: bool, action: &mut Option<Action>) {
    let inset = if narrow { 14.0 } else { INSET };
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), if narrow { 44.0 } else { 64.0 }), Sense::hover());
    let top = if narrow { rect.top() + 8.0 } else { rect.top() + 12.0 };
    let line = Rect::from_min_max(pos2(rect.left() + inset, top), pos2(rect.right() - 16.0, top + CONTROL));
    let mut row = ui.new_child(
        UiBuilder::new()
            .id_salt("agents-session-head")
            .max_rect(line)
            .layout(Layout::right_to_left(Align::Center)),
    );
    row.spacing_mut().item_spacing.x = 8.0;
    head_actions(&mut row, s, w, ready, ui.clip_rect(), action);
    row.with_layout(Layout::left_to_right(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        // What is left of the buttons is all the name and state get: with long labels
        // and little room they are cut there, never drawn over a button.
        ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
        if narrow {
            pages::pill(ui, tr(parts::state_label(s)), parts::state_color(s));
            let width = ui.available_width();
            ui.add(Label::new(facts(s, width)).truncate());
            return;
        }
        // The pill keeps its place: the name gives way first.
        let pill = ui
            .painter()
            .layout_no_wrap(tr(parts::state_label(s)).to_owned(), semibold(11.0), TEXT())
            .size()
            .x
            + 16.0;
        let room = (ui.available_width() - pill - 10.0).max(40.0);
        let name = ui
            .painter()
            .layout_no_wrap(s.record.name.clone(), semibold(16.0), TEXT())
            .size()
            .x;
        ui.allocate_ui_with_layout(vec2(name.min(room), CONTROL), Layout::left_to_right(Align::Center), |ui| {
            ui.add(Label::new(RichText::new(&s.record.name).font(semibold(16.0))).truncate())
                .on_hover_text(&s.record.name);
        });
        pages::pill(ui, tr(parts::state_label(s)), parts::state_color(s));
    });
    if narrow {
        return;
    }
    let meta = Rect::from_min_max(pos2(rect.left() + inset, line.bottom() + 2.0), pos2(rect.right() - 16.0, rect.bottom() - 2.0));
    let galley = ui.painter().layout_job(facts(s, meta.width()));
    ui.painter()
        .galley(pos2(meta.left(), meta.center().y - galley.size().y / 2.0), galley, HINT());
}

/// The one action that fits the session's state, and Mark as seen while it waits.
fn head_actions(row: &mut Ui, s: &Session, w: &mut Widgets, ready: bool, clip: Rect, action: &mut Option<Action>) {
    if s.active() {
        let stopping = s.phase == Phase::Stopping;
        let r = button(
            row,
            Look::Danger,
            (!stopping).then_some(Icon::Stop),
            stopping,
            tr(if stopping { "Stopping" } else { "Stop" }),
            !stopping,
        );
        pages::control(w, "agents-stop", &r, clip);
        if r.clicked() {
            *action = Some(Action::Stop(s.record.id));
        }
    } else if s.resumable() {
        let r = button(row, Look::Primary, Some(Icon::Play), false, tr("Resume conversation"), ready);
        pages::control(w, "agents-resume", &r, clip);
        if r.clicked() {
            *action = Some(Action::Resume);
        }
    } else {
        let r = button(row, Look::Primary, None, false, tr("Review and restart"), ready);
        pages::control(w, "agents-restart", &r, clip);
        if r.clicked() {
            *action = Some(Action::Restart);
        }
    }
    if s.attention {
        let r = button(row, Look::Quiet, None, false, tr("Mark as seen"), true);
        pages::control(w, "agents-acknowledge", &r, clip);
        if r.clicked() {
            *action = Some(Action::Acknowledge);
        }
    }
}

/// Program · branch · base · time on one line, cut at `width`.
fn facts(s: &Session, width: f32) -> text::LayoutJob {
    let font = FontId::proportional(12.0);
    let dot = |job: &mut text::LayoutJob| job.append("  ·  ", 0.0, TextFormat::simple(font.clone(), FAINT()));
    let mut job = text::LayoutJob::default();
    job.append(tr(s.record.provider.label()), 0.0, TextFormat::simple(font.clone(), HINT()));
    dot(&mut job);
    job.append(&s.record.branch, 0.0, TextFormat::simple(FontId::monospace(12.0), MUTED()));
    dot(&mut job);
    job.append(&trf!("from {base}", base = &s.record.base), 0.0, TextFormat::simple(font.clone(), HINT()));
    if let Some(time) = s.elapsed() {
        dot(&mut job);
        job.append(&parts::short_time(time), 0.0, TextFormat::simple(font.clone(), HINT()));
    }
    job.wrap = text::TextWrapping::truncate_at_width(width.max(0.0));
    job
}

/// The session's terminal with its input, in one card; or, with no terminal, why.
fn terminal(ui: &mut Ui, s: &mut Session, w: &mut Widgets, ready: bool, narrow: bool, action: &mut Option<Action>) {
    let clip = ui.clip_rect();
    let side = if narrow { 12.0 } else { 18.0 };
    let mut area = ui.max_rect().shrink2(vec2(side, 0.0));
    area.min.y += 12.0;
    area.max.y -= if narrow { 12.0 } else { 16.0 };
    if let Some(error) = &s.error {
        let mut top = child(ui, "agents-terminal-error", area);
        parts::strip(&mut top, RED(), Icon::Warning, error, None);
        area.min.y = (top.min_rect().bottom() + 10.0).min(area.bottom());
    }
    ui.painter()
        .rect(area, 8.0, DEEP(), Stroke::new(1.0, CARD_STROKE()), StrokeKind::Inside);
    let id = s.terminal_id();
    let stopping = s.phase == Phase::Stopping;
    let Some(t) = &mut s.terminal else {
        match s.phase {
            Phase::Saving | Phase::Starting | Phase::Setup | Phase::Stopping => {
                pages::loading(&mut child(ui, "agents-terminal-wait", area), tr(s.phase.label()));
            }
            Phase::Restored => {
                let resumable = s.resumable();
                parts::centered(
                    ui,
                    area,
                    Icon::Console,
                    MUTED(),
                    tr("Nothing is running"),
                    tr("Rocket reopened this session without starting it."),
                    if resumable { 14.0 + CONTROL } else { 0.0 },
                    |ui| {
                        // The header already offers the action that fits; here is the other one.
                        if resumable {
                            ui.add_space(14.0);
                            let r = button(ui, Look::Plain, None, false, tr("Review and restart"), ready);
                            pages::control(w, "agents-restart", &r, clip);
                            if r.clicked() {
                                *action = Some(Action::Restart);
                            }
                        }
                    },
                );
            }
            _ => parts::centered(ui, area, Icon::Console, MUTED(), tr("No terminal output yet"), "", 0.0, |_| ()),
        }
        return;
    };
    // The input's row is the card's foot: a band under the screen, closed by a hairline.
    let foot = Rect::from_min_max(pos2(area.left() + 1.0, area.bottom() - CONTROL - 17.0), area.max - vec2(1.0, 1.0));
    ui.painter()
        .rect_filled(foot, CornerRadius { nw: 0, ne: 0, sw: 7, se: 7 }, PANEL());
    ui.painter()
        .hline(foot.x_range(), foot.top(), Stroke::new(1.0, CARD_LINE()));
    let inner = Rect::from_min_max(area.min + vec2(12.0, 10.0), area.max - vec2(8.0, 8.0));
    let mut body = child(ui, "agents-terminal", inner);
    // The screen ends above the foot; the input sits inside it.
    body.spacing_mut().item_spacing.y = 10.0;
    if let Some(p) = t.show_agent(&mut body, id, w, stopping) {
        *action = Some(match p {
            crate::terminal::PaneAction::Settings => Action::TerminalSettings,
            crate::terminal::PaneAction::TryAgain => Action::Restart,
        });
    }
    for id in ["terminal-screen", "terminal-command", "terminal-send", "terminal-settings", "terminal-try-again"] {
        if let Some(c) = w.get_mut(id) {
            c.rect = c.rect.intersect(inner);
            c.enabled &= ui.is_enabled();
        }
    }
}
