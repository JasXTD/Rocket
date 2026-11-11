use super::*;
use crate::{
    app::{Rocket, UiControl, View},
    i18n::tr,
    pages,
    style::*,
};
use eframe::egui::*;
use std::collections::HashMap;

mod changes;
mod form;
type Widgets = HashMap<String, UiControl>;
enum Action {
    New,
    Preview,
    Confirm,
    Cancel,
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
}
fn button(ui: &mut Ui, w: &mut Widgets, id: &str, label: &'static str, enabled: bool) -> bool {
    let r = ui.add_enabled(enabled, Button::new(tr(label)));
    pages::control(w, id, &r, ui.clip_rect());
    r.clicked()
}
fn field(
    ui: &mut Ui,
    w: &mut Widgets,
    id: &str,
    label: &'static str,
    value: &mut String,
    multiline: bool,
) {
    ui.label(RichText::new(tr(label)).color(HINT()).size(12.0));
    let edit = if multiline {
        TextEdit::multiline(value).desired_rows(2)
    } else {
        TextEdit::singleline(value)
    };
    let r = ui.add(edit.desired_width(f32::INFINITY).id_salt(id));
    pages::control(w, id, &r, ui.clip_rect());
}
fn command(ui: &mut Ui, c: &Command) {
    ui.add(Label::new(RichText::new(&c.program).monospace()).wrap());
    for (i, a) in c.args().iter().enumerate() {
        ui.add(Label::new(RichText::new(format!("argv[{}] = {a:?}", i + 1)).monospace()).wrap());
    }
}
fn duration(d: Duration) -> String {
    let s = d.as_secs();
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}
fn line(ui: &Ui, rect: Rect, text: &str, font: FontId, color: Color32) {
    let mut job = text::LayoutJob::simple_singleline(text.into(), font, color);
    job.wrap = text::TextWrapping::truncate_at_width(rect.width().max(0.0));
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(
        pos2(rect.left(), rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
}
fn child(ui: &mut Ui, id: &'static str, rect: Rect) -> Ui {
    let mut out = ui.new_child(
        UiBuilder::new()
            .id_salt(id)
            .max_rect(rect)
            .layout(Layout::top_down(Align::Min)),
    );
    out.set_clip_rect(rect.intersect(ui.clip_rect()));
    out
}
impl Rocket {
    pub(crate) fn agents_view(&mut self, ui: &mut Ui) {
        let ready = self.profile_storage_ready() && !self.quitting && self.write.is_none();
        let source = self
            .tab()
            .filter(|t| !t.is_preview() && t.worktree())
            .map(|t| t.path.clone());
        let mut action = None;
        ui.add_enabled_ui(self.agents.form.is_none(),|ui| {
            Frame::new().inner_margin(12).show(ui,|ui| {
                ui.horizontal(|ui| {
                    ui.heading(tr("Agents"));
                    ui.label(RichText::new(format!("{} / {}",self.agents.sessions.iter().filter(|s|s.active()).count(),self.agents.sessions.len())).color(HINT()));
                    ui.with_layout(Layout::right_to_left(Align::Center),|ui| {
                        if button(ui,&mut self.frame_widgets,"agents-new","New session",ready&&source.is_some()&&self.agents.sessions.len()<MAX_SESSIONS){action=Some(Action::New);}
                    });
                });
                ui.add(Label::new(RichText::new(tr("Separate worktrees. Your local tools. Review changes before merging.")).color(HINT()).size(12.0)).truncate());
                if let Some(e)=&self.state_error {ui.colored_label(RED(),e);}
                if let Some(n)=&self.agents.notice {ui.add(Label::new(if n=="Worktree removed. The branch and recovery reference are kept."{tr("Worktree removed. The branch and recovery reference are kept.")}else{n}).wrap());}
                ui.add_space(6.0);
                let (rect,_)=ui.allocate_exact_size(ui.available_size().max(vec2(0.0,80.0)),Sense::hover());
                if self.agents.sessions.is_empty() {
                    let mut body=child(ui,"agents-empty",rect.shrink(18.0));
                    body.add_space(20.0);body.heading(tr("Give each task its own worktree"));
                    body.label(tr("Start Claude Code, Codex or your own executable. Your current checkout stays in place."));
                    body.label(tr("Install and sign in to your preferred tool first. Rocket uses its existing local login."));return;
                }
                let width=(rect.width()*0.25).clamp(160.0,248.0);
                let left=Rect::from_min_max(rect.min,pos2(rect.left()+width,rect.bottom()));
                let right=Rect::from_min_max(pos2(left.right()+14.0,rect.top()),rect.max);
                ui.painter().vline(left.right()+6.0,rect.y_range(),Stroke::new(1.0,HAIRLINE()));
                sessions(&mut child(ui,"agents-list",left),&mut self.agents,&mut self.frame_widgets);
                workspace(&mut child(ui,"agents-workspace",right),&mut self.agents,&mut self.frame_widgets,ready,&mut action);
            });
        });
        if let Some(f) = &mut self.agents.form {
            form::show(ui.ctx(), &mut self.frame_widgets, f, ready, &mut action);
        }
        let result = match action {
            Some(Action::New) => {
                if let Some(source) = source {
                    self.agents.new_form(source);
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
            Some(Action::Resume) => self.backend.as_ref().map_or(Ok(()),|b|self.agents.resume(b,&self.context)),
            Some(Action::Acknowledge) => {
                if let Some(s)=self.agents.sessions.iter_mut().find(|s|Some(s.record.id)==self.agents.selected){s.attention=false;s.record.attention=false;}
                Ok(())
            }
            Some(Action::CheckIntegration) => {
                if let (Some(b),Some(s))=(self.backend.clone(),self.agents.selected()) {
                    if s.active(){Err(model::invalid("Stop the agent before checking integration."))}
                    else {self.agents.integration.start(b,s.record.clone(),&self.context)}
                }else{Ok(())}
            }
            Some(Action::StopIntegration) => {self.agents.integration.stop();Ok(())}
            Some(Action::CompareIntegration) | Some(Action::MergeIntegration) => {
                let merge=matches!(action,Some(Action::MergeIntegration));
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
    fn agent_integration_handoff(&mut self, merge:bool)->Result<()> {
        let s=self.agents.selected().ok_or_else(||model::invalid("Select a session."))?;
        let r=self.agents.integration.review.as_ref().ok_or_else(||model::invalid("Check integration first."))?;
        if s.active() || s.record!=r.owner {return Err(model::invalid("The session changed. Stop it and check integration again."));}
        let record=s.record.clone();let preview=r.merge.clone();let comparison=r.comparison.clone();
        self.open(record.source);
        let tab=self.tabs.get_mut(self.active).ok_or_else(||model::invalid("Open the original checkout."))?;
        if tab.snapshot.is_none() {return Err(model::invalid("The original checkout is loading. Return to Agents and check integration when it is ready."));}
        if merge {
            self.show_dialog(crate::app::DialogKind::Merge);
            if let Some(d)=&mut self.dialog {
                d.first=preview.revision().into();d.mode=preview.mode();
                d.merge=Some(crate::merge::DialogState{preview:Some(preview),..Default::default()});
            }
            self.terminal_log.push("agent_integration_reviewed",false);
        }else{
            tab.compare.base=comparison.base.to_string();tab.compare.target=comparison.target.to_string();tab.compare.local_staged=None;
            self.compare_intent(crate::compare::CompareIntent::Open);
            self.compare_intent(crate::compare::CompareIntent::Load);
        }
        Ok(())
    }
}
fn sessions(ui: &mut Ui, state: &mut State, w: &mut Widgets) {
    let r = ui.add(
        TextEdit::singleline(&mut state.search)
            .hint_text(tr("Find sessions"))
            .desired_width(f32::INFINITY),
    );
    pages::control(w, "agents-search", &r, ui.clip_rect());
    ui.horizontal_wrapped(|ui| {
        for (filter, id, label) in [
            (Filter::All, "all", "All"),
            (Filter::Active, "active", "Active sessions"),
            (Filter::Finished, "finished", "Inactive sessions"),
            (Filter::Attention, "attention", "Needs attention"),
        ] {
            let r = ui.selectable_label(state.filter == filter, tr(label));
            pages::control(w, format!("agents-filter-{id}"), &r, ui.clip_rect());
            if r.clicked() {
                state.filter = filter;
            }
        }
    });
    let needle = state.search.trim().to_lowercase();
    let out = ScrollArea::vertical()
        .id_salt("agents-list")
        .auto_shrink([false, false])
        .min_scrolled_height(0.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            let mut found = false;
            for s in &state.sessions {
                if state.filter == Filter::Active && !s.active()
                    || state.filter == Filter::Finished && s.active()
                    || state.filter == Filter::Attention && !s.attention
                {
                    continue;
                }
                if !needle.is_empty()
                    && !format!(
                        "{} {} {}",
                        s.record.name,
                        s.record.branch,
                        s.record.provider.label()
                    )
                    .to_lowercase()
                    .contains(&needle)
                {
                    continue;
                }
                found = true;
                let (rect, r) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 80.0), Sense::click());
                let selected = state.selected == Some(s.record.id);
                if selected || r.hovered() {
                    ui.painter().rect_filled(
                        rect,
                        5.0,
                        if selected {
                            ui.visuals().selection.bg_fill
                        } else {
                            RAISED()
                        },
                    );
                }
                if selected {
                    ui.painter().rect_filled(
                        Rect::from_min_size(rect.min, vec2(3.0, rect.height())),
                        2.0,
                        BLUE(),
                    );
                }
                let text = rect.shrink2(vec2(10.0, 5.0));
                line(
                    ui,
                    Rect::from_min_size(text.min, vec2(text.width(), 24.0)),
                    &s.record.name,
                    semibold(13.0),
                    TEXT(),
                );
                line(
                    ui,
                    Rect::from_min_size(text.min + vec2(0.0, 25.0), vec2(text.width(), 20.0)),
                    &s.record.branch,
                    FontId::monospace(11.0),
                    MUTED(),
                );
                line(
                    ui,
                    Rect::from_min_size(text.min + vec2(0.0, 47.0), vec2(text.width(), 20.0)),
                    &format!(
                        "{} · {}",
                        tr(s.record.provider.label()),
                        tr(if s.active(){s.activity_state.label()}else{s.phase.label()})
                    ),
                    FontId::proportional(11.0),
                    if s.phase == Phase::Failed {
                        RED()
                    } else if s.attention { AMBER() } else {
                        HINT()
                    },
                );
                pages::control(
                    w,
                    format!("agents-session-{}", s.record.id),
                    &r,
                    ui.clip_rect(),
                );
                if r.clicked() {
                    state.integration.clear();
                    state.selected = Some(s.record.id);
                    state.changes.suspend();
                }
                r.on_hover_text(format!(
                    "{}\n{}",
                    s.record.name,
                    s.record.destination.display()
                ));
            }
            if !found {
                ui.label(tr("No matching sessions"));
            }
        });
    pages::region(w, "agents-list-scroll", out.inner_rect, ui.is_enabled());
}
fn workspace(
    ui: &mut Ui,
    state: &mut State,
    w: &mut Widgets,
    ready: bool,
    action: &mut Option<Action>,
) {
    let Some(s) = state
        .sessions
        .iter_mut()
        .find(|s| Some(s.record.id) == state.selected)
    else {
        return;
    };
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if s.active() {
                if button(
                    ui,
                    w,
                    "agents-stop",
                    if s.phase == Phase::Stopping {
                        "Stopping"
                    } else {
                        "Stop"
                    },
                    s.phase != Phase::Stopping,
                ) {
                    *action = Some(Action::Stop(s.record.id));
                }
            } else {
                if s.resumable() {
                    if button(ui,w,"agents-resume","Resume conversation",ready){*action=Some(Action::Resume);}
                } else if button(ui,w,"agents-restart","Review and restart",ready){*action=Some(Action::Restart);}
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add(Label::new(RichText::new(&s.record.name).font(semibold(16.0))).truncate());
            });
        });
    });
    ui.add(
        Label::new(
            RichText::new(format!(
                "{} · {} · {}",
                tr(s.record.provider.label()),
                s.record.branch,
                tr(s.phase.label())
            ))
            .color(if s.phase == Phase::Failed {
                RED()
            } else {
                HINT()
            })
            .size(12.0),
        )
        .truncate(),
    );
    ui.horizontal_wrapped(|ui| {
        if s.active() {ui.label(RichText::new(tr(s.activity_state.label())).color(if s.attention{AMBER()}else{HINT()}));}
        if s.attention && button(ui,w,"agents-acknowledge","Mark as seen",true){*action=Some(Action::Acknowledge);}
    });
    ui.horizontal_wrapped(|ui| {
        for (pane, id, label) in [
            (Pane::Terminal, "terminal", "Terminal"),
            (Pane::Changes, "changes", "Changes"),
            (Pane::Integrate, "integrate", "Review and integrate"),
            (Pane::Details, "details", "Details"),
        ] {
            let r = ui.selectable_label(state.pane == pane, tr(label));
            pages::control(w, format!("agents-tab-{id}"), &r, ui.clip_rect());
            if r.clicked() {
                state.pane = pane;
            }
        }
    });
    ui.separator();
    let (rect, _) = ui.allocate_exact_size(ui.available_size().max(vec2(0.0, 0.0)), Sense::hover());
    let mut body = child(ui, "agents-selected", rect);
    match state.pane {
        Pane::Terminal => {
            if let Some(e) = &s.error {
                body.add(Label::new(RichText::new(e).color(RED())).wrap());
            }
            let id = s.terminal_id();
            let stopping = s.phase == Phase::Stopping;
            if let Some(t) = &mut s.terminal {
                if let Some(p) = t.show_agent(&mut body, id, w, stopping) {
                    *action = Some(match p {
                        crate::terminal::PaneAction::Settings => Action::TerminalSettings,
                        crate::terminal::PaneAction::TryAgain => Action::Restart,
                    });
                }
            } else {
                if s.active() {
                    body.spinner();
                }
                body.label(tr(if s.phase==Phase::Restored{"Saved session. No process was restarted. Review the worktree and executable before running again."}else if s.phase==Phase::Saving{"Saving session before starting any command."}else{"No terminal output yet"}));
            }
            for id in [
                "terminal-screen",
                "terminal-command",
                "terminal-send",
                "terminal-settings",
                "terminal-try-again",
            ] {
                if let Some(c) = w.get_mut(id) {
                    c.rect = c.rect.intersect(rect);
                    c.enabled &= ui.is_enabled();
                }
            }
        }
        Pane::Changes => {
            state.changes.select_owner(&s.record);
            changes::show(
                &mut body,
                &state.changes,
                w,
                &s.record.destination,
                ready,
                action,
            );
        }
        Pane::Details => details(&mut body, s, w, ready, action),
        Pane::Integrate => integrate(&mut body,s,&state.integration,w,ready,action),
    }
}
fn details(ui: &mut Ui, s: &Session, w: &mut Widgets, ready: bool, action: &mut Option<Action>) {
    let out=ScrollArea::vertical().id_salt(("agent-details",s.record.id)).auto_shrink([false,false]).min_scrolled_height(0.0).show(ui,|ui|{
        pages::field(ui,&tr("Worktree folder"),&s.record.destination.to_string_lossy(),true);
        pages::field(ui,&tr("Base"),&s.record.base,true);
        pages::field(ui,&tr("Conversation activity"),tr(s.activity_state.label()),false);
        if let Some(id)=&s.record.conversation{pages::field(ui,&tr("Conversation ID"),id,true);}
        if let Some(d)=s.elapsed(){pages::field(ui,&tr("Elapsed"),&duration(d),false);}
        if let Some(c)=s.exit_code{pages::field(ui,&tr("Exit code"),&c.to_string(),false);}
        if let Some(e)=&s.error{ui.colored_label(RED(),e);}
        ui.separator();ui.strong(tr("Session activity"));
        for e in s.activity.iter().rev(){ui.horizontal(|ui|{ui.label(RichText::new(duration(e.at.elapsed())).monospace().color(HINT()));ui.label(tr(e.phase.label()));});}
        pages::note(ui,&tr("Time since each event; current app session only."));
        ui.separator();ui.strong(tr("Agent executable"));command(ui,&s.record.command);
        if let Some(setup)=&s.record.setup{ui.strong(tr("Setup command"));command(ui,setup);}
        ui.separator();
        ui.horizontal_wrapped(|ui|{
            if button(ui,w,"agents-restart","Review and restart",ready&&!s.active()){*action=Some(Action::Restart);}
            if button(ui,w,"agents-open","Review changes",ready){*action=Some(Action::Open(s.record.destination.clone()));}
            if button(ui,w,"agents-remove","Remove worktree",ready&&!s.active()){*action=Some(Action::Remove);}
            if button(ui,w,"agents-forget","Forget session, keep files",ready&&!s.active()){*action=Some(Action::Forget);}
        });
        pages::note(ui,&tr("Stopping keeps the worktree and all changes."));
        pages::note(ui,&tr("Worktrees separate files, not permissions. Agents run with your user account and keep their own permission prompts."));
        pages::note(ui,&tr("Credentials and terminal output are not saved in session metadata or diagnostic logs. Do not put secrets in arguments."));
    });
    pages::region(w, "agents-details-scroll", out.inner_rect, ui.is_enabled());
}

fn integrate(ui:&mut Ui, s:&Session, state:&integration::State,w:&mut Widgets,ready:bool,action:&mut Option<Action>) {
    let out=ScrollArea::vertical().id_salt("agents-integration").auto_shrink([false,false]).show(ui,|ui|{
        ui.heading(tr("Review and integrate"));
        ui.label(tr("1. Review the worktree changes and commit the work you want to keep."));
        if button(ui,w,"agents-open","Open worktree changes",ready){*action=Some(Action::Open(s.record.destination.clone()));}
        ui.add_space(10.0);
        ui.label(tr("2. Stop the agent, then compare its committed branch with the original checkout."));
        ui.label(tr("Local changes must be committed or stashed in both checkouts. Nothing is saved or discarded automatically."));
        if s.active(){ui.colored_label(AMBER(),tr("Stop this session before integration."));}
        ui.horizontal_wrapped(|ui|{
            if button(ui,w,"agents-integration-check","Check integration",ready&&!s.active()&&state.task.is_none()){*action=Some(Action::CheckIntegration);}
            if state.task.is_some(){ui.spinner();if button(ui,w,"agents-integration-stop",if state.stopping{"Stopping"}else{"Stop"},!state.stopping){*action=Some(Action::StopIntegration);}}
        });
        if let Some(e)=&state.error{ui.colored_label(RED(),e);}
        if let Some(r)=&state.review {
            let into=match r.merge.head(){rocket_git::Head::Branch{name,..}=>String::from_utf8_lossy(name),_=>std::borrow::Cow::Borrowed("Detached")};
            pages::field(ui,&tr("Original checkout"),&r.owner.source.to_string_lossy(),true);
            pages::field(ui,&tr("Merge into"),&into,true);
            pages::field(ui,&tr("Agent branch"),&r.owner.branch,true);
            ui.label(format!("{} {} · {} {}",r.merge.incoming(),tr("incoming commits"),r.comparison.files.len(),tr("changed files")));
            if button(ui,w,"agents-integration-compare","Compare committed changes",ready&&!s.active()){*action=Some(Action::CompareIntegration);}
            ui.add_space(10.0);ui.label(tr("3. Review the merge and explicitly apply it. Conflicts use the existing conflict editor and Continue / Abort recovery."));
            if button(ui,w,"agents-integration-merge","Review merge",ready&&!s.active()&&r.merge.incoming()>0){*action=Some(Action::MergeIntegration);}
            pages::note(ui,&tr("The merge checks both branch tips and the original checkout again. The agent branch and worktree stay available afterward. Nothing is pushed."));
        }
    });pages::region(w,"agents-integration-scroll",out.inner_rect,ui.is_enabled());
}
