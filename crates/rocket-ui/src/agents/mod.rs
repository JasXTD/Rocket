//! Managed local processes in separate Git worktrees. No agent installation,
//! permission bypass, automatic merge or background relaunch.
pub mod changes;
pub mod events;
mod discovery;
pub mod integration;
mod localization;
pub mod model;
pub mod runtime;
#[cfg(test)]
mod tests;
mod view;
use crate::{
    backend::{Backend, user_error},
    terminal::{EventLog, TerminalPane},
};
use eframe::egui::Context;
use model::*;
use rocket_git::{ReadTask, Result, WriteTask};
use runtime::*;
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pane {
    #[default]
    Terminal,
    Changes,
    Integrate,
    Details,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    All,
    Active,
    Finished,
    Attention,
}
pub struct Activity {
    pub phase: Phase,
    pub at: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Restored,
    Saving,
    Starting,
    Setup,
    Running,
    Stopping,
    Stopped,
    Exited,
    Failed,
}
impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Restored => "Restored",
            Self::Saving => "Saving session",
            Self::Starting => "Starting",
            Self::Setup => "Setting up",
            Self::Running => "Running",
            Self::Stopping => "Stopping",
            Self::Stopped => "Stopped",
            Self::Exited => "Exited",
            Self::Failed => "Failed",
        }
    }
}
pub struct Session {
    pub activity_state: events::Activity,
    pub attention: bool,
    pub notifications: u64,
    bridge: Option<events::Bridge>,
    event_at: u64,
    event_turn: Option<String>,
    pub record: Record,
    pub phase: Phase,
    pub terminal: Option<TerminalPane>,
    pub error: Option<String>,
    plan: Option<Arc<Plan>>,
    job: Option<WriteTask<Started>>,
    pending: bool,
    after_setup: bool,
    observing: bool,
    setup_process: bool,
    pub exit_code: Option<u32>,
    pub activity: VecDeque<Activity>,
    started: Option<Instant>,
    elapsed: Option<Duration>,
}
impl Session {
    fn restored(record: Record) -> Self {
        Self {
            activity_state: record.last_activity,
            attention: record.attention,
            notifications: 0,
            bridge: None,
            event_at: 0,
            event_turn: None,
            record,
            phase: Phase::Restored,
            terminal: None,
            error: None,
            plan: None,
            job: None,
            pending: false,
            after_setup: false,
            observing: false,
            setup_process: false,
            exit_code: None,
            activity: VecDeque::from([Activity {
                phase: Phase::Restored,
                at: Instant::now(),
            }]),
            started: None,
            elapsed: None,
        }
    }
    pub fn active(&self) -> bool {
        self.pending
            || self.job.is_some()
            || self
                .terminal
                .as_ref()
                .is_some_and(|t| !t.session.status().closed)
    }
    pub fn stop(&mut self) {
        if !self.active() {
            return;
        }
        self.pending = false;
        if let Some(job) = &self.job {
            job.cancel();
        }
        if let Some(t) = &mut self.terminal {
            t.stop();
        }
        self.phase = Phase::Stopping;
        self.bridge = None;
        self.observe_phase();
    }
    pub fn terminal_id(&self) -> u64 {
        self.record.id | (1 << 63)
    }
    pub fn resumable(&self)->bool {
        !self.active() && self.record.tracking && events::can_resume(&self.record) && self.record.conversation.is_some() && self.record.approval.is_some()
    }
    fn observe_event(&mut self, e:events::Event)->bool {
        if self.phase==Phase::Stopping || e.at<=self.event_at {return false;}
        if let Some(id)=&self.record.conversation {if id!=&e.id{return false;}}
        if e.kind!=events::Activity::Ready && !e.new_turn
            && let (Some(current),Some(incoming))=(&self.event_turn,&e.turn) && current!=incoming {return false;}
        self.record.conversation=Some(e.id);
        self.event_at=e.at;
        if e.turn.is_some(){self.event_turn=e.turn;}
        let changed=self.activity_state!=e.kind;
        self.activity_state=e.kind;
        let alert=changed&&matches!(e.kind,events::Activity::Waiting|events::Activity::Complete|events::Activity::Interrupted);
        if alert {self.attention=true;self.notifications=self.notifications.saturating_add(1);}
        else if e.kind==events::Activity::Working {self.attention=false;}
        self.record.attention=self.attention;self.record.last_activity=self.activity_state;
        alert
    }
    pub fn elapsed(&self) -> Option<Duration> {
        self.elapsed.or_else(|| self.started.map(|at| at.elapsed()))
    }
    fn observe_phase(&mut self) {
        if self.activity.back().is_none_or(|a| a.phase != self.phase) {
            if self.activity.len() == 32 {
                self.activity.pop_front();
            }
            self.activity.push_back(Activity {
                phase: self.phase,
                at: Instant::now(),
            });
        }
        if matches!(self.phase, Phase::Stopped | Phase::Exited | Phase::Failed)
            && self.elapsed.is_none()
        {
            self.elapsed = self.started.map(|at| at.elapsed());
        }
    }
}
pub struct Form {
    pub record: Record,
    pub destination: String,
    pub setup_enabled: bool,
    pub setup: Command,
    pub mode: Mode,
    pub plan: Option<Arc<Plan>>,
    pub task: Option<ReadTask<Plan>>,
    pub error: Option<String>,
}
impl Form {
    pub fn proposed(&self) -> Record {
        let mut r = self.record.clone();
        r.destination = self.destination.clone().into();
        if self.mode != Mode::Resume {
            r.setup = self.setup_enabled.then(|| self.setup.clone());
        }
        r
    }
}
#[derive(Default)]
pub struct State {
    next_id: u64,
    pub sessions: Vec<Session>,
    pub selected: Option<u64>,
    pub form: Option<Form>,
    pub notice: Option<String>,
    pub pane: Pane,
    pub filter: Filter,
    pub search: String,
    pub changes: changes::State,
    pub integration: integration::State,
}
impl State {
    pub fn load(&mut self, records: Vec<Record>) {
        self.integration.clear();
        self.changes = changes::State::default();
        self.pane = Pane::Terminal;
        self.form = None;
        self.sessions = records.into_iter().map(Session::restored).collect();
        self.selected = self.sessions.first().map(|s| s.record.id);
    }
    pub fn records(&self) -> Vec<Record> {
        self.sessions.iter().map(|s| s.record.clone()).collect()
    }
    pub fn selected(&self) -> Option<&Session> {
        self.sessions
            .iter()
            .find(|s| Some(s.record.id) == self.selected)
    }
    pub fn active(&self) -> bool {
        self.sessions.iter().any(Session::active)
    }
    pub fn working(&self) -> bool {
        self.integration.task.is_some() || self.form.as_ref().is_some_and(|f| f.task.is_some())
            || self.sessions.iter().any(|s| s.job.is_some() || s.pending)
            || self.changes.busy()
    }
    pub fn stop_all(&mut self) {
        self.integration.stop();
        self.changes.suspend();
        if let Some(f) = &self.form
            && let Some(t) = &f.task
        {
            t.cancel();
        }
        for s in &mut self.sessions {
            s.stop();
        }
    }
    pub fn new_form(&mut self, source: PathBuf) {
        let id = self.next_id.max(
            self.sessions
                .iter()
                .map(|s| s.record.id)
                .max()
                .unwrap_or(0)
                .saturating_add(1),
        );
        self.next_id = id.saturating_add(1);
        let provider = Provider::Claude;
        let destination = source
            .parent()
            .unwrap_or(&source)
            .join(format!("rocket-agent-{id}"));
        let r = Record {
            id,
            source,
            destination,
            base: "HEAD".into(),
            branch: format!("agents/task-{id}"),
            provider,
            command: provider.command(),
            tracking: true,
            ..Default::default()
        };
        self.form = Some(Form {
            destination: r.destination.to_string_lossy().into_owned(),
            record: r,
            setup_enabled: false,
            setup: Command::default(),
            mode: Mode::Create,
            plan: None,
            task: None,
            error: None,
        });
    }
    pub fn existing_form(&mut self, mode: Mode) {
        let Some(s) = self.selected().filter(|s| !s.active()) else {
            return;
        };
        let r = s.record.clone();
        self.form = Some(Form {
            destination: r.destination.to_string_lossy().into_owned(),
            setup_enabled: false,
            setup: r.setup.clone().unwrap_or_default(),
            record: r,
            mode,
            plan: None,
            task: None,
            error: None,
        });
    }
    pub fn preview(&mut self, backend: &Backend, ctx: &Context) -> Result<()> {
        let Some(f) = &mut self.form else {
            return Ok(());
        };
        let record = f.proposed();
        let mode = f.mode;
        if self.sessions.iter().any(|s| {
            s.record.id != record.id
                && (s.record.destination == record.destination
                    || s.record.source == record.source && s.record.branch == record.branch)
        }) {
            return Err(model::invalid(
                "Another managed session already uses this worktree or branch.",
            ));
        }
        f.error = None;
        f.plan = None;
        let backend = backend.clone();
        let ctx = ctx.clone();
        f.task = Some(ReadTask::spawn_with_wake(
            move |c| runtime::preview(&backend, record, mode, &c),
            move || ctx.request_repaint(),
        )?);
        Ok(())
    }
    pub fn resume(&mut self, backend:&Backend, ctx:&Context)->Result<()> {
        if self.selected().is_none_or(|s|!s.resumable()){return Err(model::invalid("No saved conversation is available to resume."));}
        self.existing_form(Mode::Resume);
        self.preview(backend,ctx)
    }
    pub fn confirm(&mut self) -> Result<()> {
        let f = self
            .form
            .as_ref()
            .ok_or_else(|| model::invalid("Review a session first."))?;
        let plan = f
            .plan
            .clone()
            .ok_or_else(|| model::invalid("Review a session first."))?;
        // Edits invalidate review in the UI; compare here too for non-UI callers.
        if f.proposed() != plan.record {
            return Err(model::invalid("Session changed. Review again."));
        }
        if let Some(s) = self.sessions.iter().find(|s| s.record.id == plan.record.id)
            && s.active()
        {
            return Err(model::invalid(
                "Stop this session before starting another process.",
            ));
        }
        if plan.mode == Mode::Create {
            if self.sessions.len() >= MAX_SESSIONS {
                return Err(model::invalid(
                    "Remove a saved session before creating another.",
                ));
            }
            let mut session = Session::restored(plan.record.clone());
            // A new record was never restored from disk. Its first observed
            // event is Saving below; restored sessions keep their real event.
            session.activity.clear();
            self.sessions.push(session);
        }
        let s = self
            .sessions
            .iter_mut()
            .find(|s| s.record.id == plan.record.id)
            .ok_or_else(|| model::invalid("Session no longer exists."))?;
        s.record = plan.record.clone();
        if plan.mode!=Mode::Remove {
            s.record.approval=Some(runtime::approval(&s.record,&plan.program.as_ref().unwrap().digest,&rocket_git::CancellationToken::new())?);
            if plan.mode!=Mode::Resume {s.record.conversation=None;}
        }
        s.activity_state=events::Activity::Unknown;s.attention=false;s.event_at=0;s.event_turn=None;s.bridge=None;
        s.record.attention=false;s.record.last_activity=events::Activity::Unknown;
        s.plan = Some(plan);
        s.pending = true;
        s.after_setup = false;
        s.error = None;
        s.exit_code = None;
        s.phase = Phase::Saving;
        s.started = Some(Instant::now());
        s.elapsed = None;
        s.observe_phase();
        self.selected = Some(s.record.id);
        self.pane = Pane::Terminal;
        self.filter = Filter::All;
        self.search.clear();
        self.form = None;
        Ok(())
    }
    pub(crate) fn poll(
        &mut self,
        backend: &Backend,
        ctx: &Context,
        persisted: &[Record],
        visible: bool,
        log: &mut EventLog,
    ) {
        if let Some(f) = &mut self.form
            && let Some(result) = f.task.as_mut().and_then(ReadTask::try_finish)
        {
            f.task = None;
            match result {
                Ok(plan) => {
                    f.record = plan.record.clone();
                    f.destination = plan.record.destination.to_string_lossy().into_owned();
                    f.plan = Some(Arc::new(plan));
                }
                Err(e) => f.error = Some(user_error(&e)),
            }
        }
        if self.form.as_ref().is_some_and(|f|f.mode==Mode::Resume&&f.plan.is_some()) {
            if let Err(e)=self.confirm(){self.notice=Some(user_error(&e));}
        }
        let mut removed = Vec::new();
        if self.pane!=Pane::Integrate || !visible || self.integration.review.as_ref().is_some_and(|r|Some(r.owner.id)!=self.selected) {
            self.integration.clear();
        } else {self.integration.poll();}
        for s in &mut self.sessions {
            if let Some(result) = s.job.as_mut().and_then(WriteTask::try_finish) {
                s.job = None;
                match result {
                    Ok(started) if started.removed => {
                        removed.push(s.record.id);
                        log.push("agent_worktree_removed", false);
                        self.notice = Some(
                            "Worktree removed. The branch and recovery reference are kept.".into(),
                        );
                    }
                    Ok(started) => {
                        s.bridge=started.bridge;
                        s.exit_code = None;
                        s.observing = true;
                        s.setup_process = started.setup;
                        let mut t = TerminalPane::new(
                            started.session.unwrap(),
                            crate::i18n::tr(if started.setup {
                                "Setup"
                            } else {
                                s.record.provider.label()
                            })
                            .into(),
                            false,
                        );
                        if s.phase == Phase::Stopping {
                            t.stop();
                        } else {
                            s.phase = Phase::Starting;
                        }
                        s.terminal = Some(t);
                    }
                    Err(e) => {
                        s.error = Some(user_error(&e));
                        s.phase = if s.phase == Phase::Stopping {
                            Phase::Stopped
                        } else {
                            Phase::Failed
                        };
                        s.plan = None;
                        log.push("agent_start_failed", true);
                    }
                }
            }
            let incoming=s.bridge.as_ref().map_or_else(Vec::new,events::Bridge::drain);
            for event in incoming {
                let previous=s.activity_state;
                if s.observe_event(event) {
                    ctx.send_viewport_cmd(eframe::egui::ViewportCommand::RequestUserAttention(eframe::egui::UserAttentionType::Informational));
                    log.push("agent_attention",false);
                }
                if previous!=s.activity_state {
                    log.push(match s.activity_state {
                        events::Activity::Ready=>"agent_conversation_ready",
                        events::Activity::Working=>"agent_turn_working",
                        events::Activity::Waiting=>"agent_waiting_input",
                        events::Activity::Complete=>"agent_turn_finished",
                        _=>"agent_activity_changed",
                    },false);
                }
            }
            if s.bridge.as_ref().is_some_and(events::Bridge::lost) {
                s.activity_state=events::Activity::Unavailable;s.attention=true;
                s.record.attention=true;s.record.last_activity=s.activity_state;
                log.push("agent_activity_unavailable",false);
            }
            if let Some(t) = &mut s.terminal {
                t.poll(
                    visible
                        && self.pane == Pane::Terminal
                        && self.form.is_none()
                        && Some(s.record.id) == self.selected,
                );
                t.events.clear(); // Agent diagnostics intentionally exclude input and output.
                if s.observing && s.phase == Phase::Starting && t.status.running {
                    s.phase = if s.setup_process {
                        Phase::Setup
                    } else {
                        Phase::Running
                    };
                    log.push(
                        if s.setup_process {
                            "agent_setup_started"
                        } else {
                            "agent_started"
                        },
                        false,
                    );
                }
                if s.observing && t.status.closed {
                    s.observing = false;
                    s.exit_code = t.status.exit_code;
                    if s.setup_process
                        && s.phase != Phase::Stopping
                        && t.status.exit_code == Some(0)
                        && t.status.error.is_none()
                    {
                        s.after_setup = true;
                        s.pending = true;
                        s.phase = Phase::Starting;
                        log.push("agent_setup_succeeded", false);
                    } else {
                        let stopped = s.phase == Phase::Stopping;
                        let failed = t.status.error.is_some() || t.status.exit_code != Some(0);
                        s.phase = if stopped {
                            Phase::Stopped
                        } else if failed {
                            Phase::Failed
                        } else {
                            Phase::Exited
                        };
                        if !stopped && failed {
                            s.error=Some(t.status.error.clone().unwrap_or_else(||format!("Process exited with code {}. Changes are kept in the worktree.",t.status.exit_code.unwrap_or(1))));
                        }
                        s.plan = None;
                        s.bridge = None;
                        if !stopped && failed {s.attention=true;s.notifications=s.notifications.saturating_add(1);ctx.send_viewport_cmd(eframe::egui::ViewportCommand::RequestUserAttention(eframe::egui::UserAttentionType::Informational));}
                        log.push(
                            if stopped {
                                "agent_stopped"
                            } else {
                                "agent_exited"
                            },
                            failed && !stopped,
                        );
                    }
                }
            }
            if s.phase == Phase::Stopping && !s.active() {
                s.phase = Phase::Stopped;
                s.plan = None;
                log.push("agent_stopped", false);
            }
            // Persist the planned worktree before creating it. Failed storage never
            // falls through to a process launch, even after setup completes.
            if s.pending && persisted.contains(&s.record) {
                let plan = s.plan.as_ref().unwrap().clone();
                let backend = backend.clone();
                let context = ctx.clone();
                let after_setup = s.after_setup;
                let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || context.request_repaint());
                match WriteTask::spawn(move |c| {
                    runtime::start(&backend, &plan, after_setup, wake, &c)
                }) {
                    Ok(job) => {
                        s.job = Some(job);
                        s.pending = false;
                        s.phase = Phase::Starting;
                        log.push("agent_start_requested", false);
                    }
                    Err(e) => {
                        s.pending = false;
                        s.phase = Phase::Failed;
                        s.error = Some(user_error(&e));
                        s.plan = None;
                    }
                }
            }
        }
        for s in &mut self.sessions {
            s.record.attention=s.attention;s.record.last_activity=s.activity_state;
            s.observe_phase();
        }
        self.sessions.retain(|s| !removed.contains(&s.record.id));
        if self.selected.is_some_and(|id| removed.contains(&id)) {
            self.selected = self.sessions.first().map(|s| s.record.id);
        }
        if visible && self.form.is_none() && self.pane == Pane::Changes {
            if let Some(record) = self.selected().map(|s| s.record.clone()) {
                self.changes.select_owner(&record);
                self.changes.poll(backend, ctx);
            } else {
                self.changes = changes::State::default();
            }
        } else {
            self.changes.suspend();
        }
        if self.working() || self.active() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
}
