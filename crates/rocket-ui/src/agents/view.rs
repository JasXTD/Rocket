use super::*;
use crate::i18n::tr;
use crate::{
    app::{Rocket, UiControl, View},
    style::*,
};
use eframe::egui::*;
use std::collections::HashMap;

type Widgets = HashMap<String, UiControl>;
enum Action {
    New,
    Preview,
    Confirm,
    Cancel,
    Stop(u64),
    Restart,
    Remove,
    TerminalSettings,
    Open(PathBuf),
    Forget,
}
fn button(ui: &mut Ui, w: &mut Widgets, id: &str, label: &'static str, enabled: bool) -> bool {
    let r = ui.add_enabled(enabled, Button::new(tr(label)));
    w.insert(id.into(), (&r).into());
    r.clicked()
}
fn field(
    ui: &mut Ui,
    w: &mut Widgets,
    id: &str,
    label: &'static str,
    value: &mut String,
    multiline: bool,
) -> bool {
    ui.label(RichText::new(tr(label)).color(HINT()).size(12.0));
    let edit = if multiline {
        TextEdit::multiline(value).desired_rows(2)
    } else {
        TextEdit::singleline(value)
    }
    .desired_width(f32::INFINITY)
    .id_salt(id);
    let r = ui.add(edit);
    w.insert(id.into(), (&r).into());
    r.changed()
}
impl Rocket {
    pub(crate) fn agents_view(&mut self, ui: &mut Ui) {
        let ready = self.profile_storage_ready() && !self.quitting && self.write.is_none();
        let source = self
            .tab()
            .filter(|t| !t.is_preview() && t.worktree())
            .map(|t| t.path.clone());
        let mut action = None;
        Frame::new().inner_margin(18).show(ui,|ui| {
            ui.horizontal_wrapped(|ui|{
                ui.heading(tr("Agents"));ui.add_space(12.0);
                if button(ui,&mut self.frame_widgets,"agents-new","New session",ready&&source.is_some()&&self.agents.sessions.len()<MAX_SESSIONS){action=Some(Action::New);}
            });
            ui.label(RichText::new(tr("Separate worktrees. Your local tools. Review changes before merging.")).color(HINT()));
            if let Some(error)=&self.state_error {ui.colored_label(RED(),error);}
            if let Some(notice)=&self.agents.notice {ui.label(if notice=="Worktree removed. The branch and recovery reference are kept."{tr("Worktree removed. The branch and recovery reference are kept.")}else{notice});}
            ui.add_space(12.0);
            let scroll=ScrollArea::vertical().id_salt("agents-page-scroll").show(ui,|ui| {
                let content_width=ui.available_width();
                if let Some(f)=&mut self.agents.form {
                    form(ui,&mut self.frame_widgets,f,ready,&mut action);
                } else if self.agents.sessions.is_empty() {
                    ui.add_space(40.0);ui.heading(tr("Give each task its own worktree"));
                    ui.label(tr("Start Claude Code, Codex or your own executable. Your current checkout stays in place."));
                    ui.label(tr("Install and sign in to your preferred tool first. Rocket uses its existing local login."));
                } else {
                    let card_width=240.0_f32.min((content_width-26.0).max(120.0));
                    let columns=((content_width+ui.spacing().item_spacing.x)/(card_width+26.0+ui.spacing().item_spacing.x)).floor().max(1.0) as usize;
                    // Frames allocate as a whole. Explicit rows prevent their minimum
                    // widths from growing the page after a wide-to-compact resize.
                    for row in self.agents.sessions.chunks(columns) {
                      ui.horizontal_top(|ui| {
                        for s in row {
                            let selected=Some(s.record.id)==self.agents.selected;
                            let r=Frame::new().fill(if selected{ui.visuals().selection.bg_fill}else{VIEW_PANEL()})
                                .stroke(Stroke::new(1.0,if selected{BLUE()}else{ui.visuals().widgets.noninteractive.bg_stroke.color}))
                                .corner_radius(6).inner_margin(12).show(ui,|ui|ui.vertical(|ui|{
                                    ui.set_width(card_width);
                                    ui.set_min_height(62.0);
                                    ui.add(Label::new(RichText::new(&s.record.name).strong()).truncate()).on_hover_text(&s.record.name);
                                    ui.horizontal_wrapped(|ui|{ui.label(RichText::new(tr(s.phase.label())).color(if s.phase==Phase::Failed{RED()}else{HINT()}));ui.label(RichText::new(tr(s.record.provider.label())).small().color(HINT()));});
                                    ui.add(Label::new(RichText::new(&s.record.branch).monospace().small().color(HINT())).truncate()).on_hover_text(&s.record.branch);
                                })).response.interact(Sense::click());
                            self.frame_widgets.insert(format!("agents-session-{}",s.record.id),(&r).into());
                            if r.clicked(){self.agents.selected=Some(s.record.id);}
                        }
                      });
                    }
                    ui.separator();
                    if let Some(s)=self.agents.sessions.iter_mut().find(|s|Some(s.record.id)==self.agents.selected) {
                        ui.horizontal_wrapped(|ui|{
                            ui.heading(&s.record.name);
                            ui.label(RichText::new(tr(s.phase.label())).color(if s.phase==Phase::Failed{RED()}else{BLUE()}));
                            if let Some(code)=s.exit_code {ui.label(format!("{code}"));}
                        });
                        ui.label(format!("{} · {}",tr(s.record.provider.label()),s.record.branch));
                        ui.add(Label::new(s.record.destination.to_string_lossy()).wrap());
                        ui.horizontal_wrapped(|ui|{
                            if button(ui,&mut self.frame_widgets,"agents-open","Review changes",ready){action=Some(Action::Open(s.record.destination.clone()));}
                            if button(ui,&mut self.frame_widgets,"agents-stop","Stop",s.active()&&s.phase!=Phase::Stopping){action=Some(Action::Stop(s.record.id));}
                            if button(ui,&mut self.frame_widgets,"agents-restart","Review and restart",ready&&!s.active()){action=Some(Action::Restart);}
                            if button(ui,&mut self.frame_widgets,"agents-remove","Remove worktree",ready&&!s.active()){action=Some(Action::Remove);}
                        });
                        if s.phase==Phase::Restored {ui.label(tr("Saved session. No process was restarted. Review the worktree and executable before running again."));}
                        if matches!(s.phase,Phase::Stopping|Phase::Stopped) {ui.label(tr("Stopping keeps the worktree and all changes."));}
                        if s.phase==Phase::Saving {ui.spinner();ui.label(tr("Saving session before starting any command."));}
                        if let Some(error)=&s.error {ui.colored_label(RED(),error);}
                        let id=s.terminal_id();let stopping=s.phase==Phase::Stopping;
                        if let Some(t)=&mut s.terminal {
                            ui.add_space(10.0);
                            Frame::new().fill(VIEW_PANEL()).stroke(Stroke::new(1.0,ui.visuals().widgets.noninteractive.bg_stroke.color)).corner_radius(6).inner_margin(10).show(ui,|ui|{
                                ui.set_width((content_width-22.0).max(160.0));
                                let pane=ui.allocate_ui_with_layout(vec2(ui.available_width(),330.0),Layout::top_down(Align::Min),|ui|t.show(ui,id,&mut self.frame_widgets,stopping)).inner;
                                if let Some(pane)=pane {action=Some(match pane {crate::terminal::PaneAction::Settings=>Action::TerminalSettings,crate::terminal::PaneAction::TryAgain=>Action::Restart});}
                            });
                        }
                        CollapsingHeader::new(tr("Session details")).show(ui,|ui| {
                            ui.label(tr("Worktrees separate files, not permissions. Agents run with your user account and keep their own permission prompts."));
                            ui.label(tr("Credentials and terminal output are not saved in session metadata or diagnostic logs. Do not put secrets in arguments."));
                            command(ui,&s.record.command);
                            if button(ui,&mut self.frame_widgets,"agents-forget","Forget session, keep files",!s.active()&&ready){action=Some(Action::Forget);}
                        });
                    }
                }
            });
            // Registration must reflect the visible viewport, just like hit testing.
            // Otherwise an MCP click can target a terminal field below the window.
            for (id,control) in &mut self.frame_widgets {
                if id.starts_with("agents-") && id!="agents-new"
                    || ["terminal-screen","terminal-command","terminal-send","terminal-settings","terminal-try-again"].contains(&id.as_str()) {
                    control.rect=control.rect.intersect(scroll.inner_rect);
                }
            }
            self.frame_widgets.insert("agents-scroll".into(),UiControl{rect:scroll.inner_rect,enabled:true});
        });
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
                if let Some(f) = &mut self.agents.form
                    && let Some(t) = &f.task
                {
                    t.cancel();
                } else {
                    self.agents.form = None;
                }
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
            Some(Action::TerminalSettings) => {
                self.show_preferences(crate::preferences::Page::Terminal);
                Ok(())
            }
            Some(Action::Remove) => {
                self.agents.existing_form(Mode::Remove);
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
}
fn command(ui: &mut Ui, c: &Command) {
    ui.add(Label::new(RichText::new(&c.program).monospace()).wrap());
    for (i, a) in c.args().iter().enumerate() {
        ui.add(Label::new(RichText::new(format!("argv[{}] = {a:?}", i + 1)).monospace()).wrap());
    }
}
fn form(ui: &mut Ui, w: &mut Widgets, f: &mut Form, ready: bool, action: &mut Option<Action>) {
    let removing = f.mode == Mode::Remove;
    ui.heading(tr(if removing {
        "Remove worktree"
    } else if f.mode == Mode::Restart {
        "Review and restart"
    } else {
        "New agent session"
    }));
    if let Some(e) = &f.error {
        ui.colored_label(RED(), e);
    }
    if let Some(plan) = &f.plan {
        ui.label(&plan.record.name);
        ui.label(format!("{} · {}", plan.record.branch, plan.target));
        ui.add(Label::new(plan.record.destination.to_string_lossy()).wrap());
        if removing {
            ui.label(tr("Only a clean, unlocked worktree can be removed. The branch and recovery reference are kept."));
        } else {
            ui.label(tr("Agent executable"));
            command(ui, &plan.record.command);
            if let Some(p) = &plan.program {
                ui.label(
                    RichText::new(format!("SHA-256: {}", p.digest))
                        .small()
                        .color(HINT()),
                );
            }
            if let Some(setup) = &plan.record.setup {
                ui.label(tr("Setup command"));
                command(ui, setup);
            }
            ui.label(tr("Starts external programs with your account's access. Existing files and credentials are not copied. Setup runs only when explicitly selected."));
        }
        ui.horizontal_wrapped(|ui| {
            if button(ui, w, "agents-back", "Back", true) {
                f.plan = None;
            }
            if button(
                ui,
                w,
                "agents-confirm",
                if removing {
                    "Remove clean worktree"
                } else if f.mode == Mode::Restart {
                    "Start session"
                } else {
                    "Create and start"
                },
                ready,
            ) {
                *action = Some(Action::Confirm);
            }
            if button(ui, w, "agents-cancel", "Cancel", true) {
                *action = Some(Action::Cancel);
            }
        });
        return;
    }
    let editing = f.task.is_none();
    ui.add_enabled_ui(editing,|ui| {
        if !removing {
            field(ui,w,"agents-name","Task name",&mut f.record.name,false);
            ui.label(tr("Provider"));ui.horizontal_wrapped(|ui|{for p in Provider::ALL {let r=ui.selectable_label(f.record.provider==p,tr(p.label()));w.insert(format!("agents-provider-{}",p.key()),(&r).into());if r.clicked(){f.record.provider=p;f.record.command.program=p.installed();f.record.command.arguments.clear();}}});
        }
        if f.mode==Mode::Create {
            field(ui,w,"agents-base","Base",&mut f.record.base,false);
            field(ui,w,"agents-branch","New branch",&mut f.record.branch,false);
            field(ui,w,"agents-destination","Worktree folder",&mut f.destination,false);
        } else {ui.add(Label::new(&f.destination).wrap());ui.label(&f.record.branch);}
        if !removing {
            field(ui,w,"agents-program","Agent executable",&mut f.record.command.program,false);
            field(ui,w,"agents-arguments","Arguments · one per line, no shell expansion",&mut f.record.command.arguments,true);
            let r=ui.checkbox(&mut f.setup_enabled,tr("Run a setup command first"));w.insert("agents-setup-enabled".into(),(&r).into());
            if f.setup_enabled {
                field(ui,w,"agents-setup-program","Setup executable",&mut f.setup.program,false);
                field(ui,w,"agents-setup-arguments","Setup arguments",&mut f.setup.arguments,true);
                ui.label(tr("The agent starts only after setup exits successfully. Setup is optional on every restart."));
            }
            ui.label(tr("Use your installed tool and existing login. Rocket does not install agents or bypass their permission prompts."));
        }
    });
    ui.horizontal_wrapped(|ui| {
        if button(ui, w, "agents-review", "Review session", editing && ready) {
            *action = Some(Action::Preview);
        }
        if button(
            ui,
            w,
            "agents-cancel",
            if editing { "Cancel" } else { "Stop" },
            true,
        ) {
            *action = Some(Action::Cancel);
        }
        if f.task.is_some() {
            ui.spinner();
            ui.label(tr("Checking worktree and executables"));
        }
    });
}
