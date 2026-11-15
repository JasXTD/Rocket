//! Managed local processes in separate Git worktrees. No agent installation,
//! permission bypass, automatic merge or background relaunch.
pub mod brief;
pub mod briefing;
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
    /// The first message, made from what the session started from: put in the agent's
    /// input once its terminal starts, never sent by Rocket and never saved. Its label
    /// says where it came from ("#482").
    pub first: Option<(String, String)>,
}
impl Session {
    fn restored(record: Record) -> Self {
        Self {
            first: None,
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
    /// Which presets' programs are on this PC, in `Provider::ALL`'s order. Looked up
    /// when the dialog first shows and again after Browse chose one.
    pub found: Option<[bool; 4]>,
    /// What the session starts from, its task and first message (section 48).
    pub briefing: Option<briefing::Briefing>,
    /// The Issue or pull request field of a new session that starts from nothing yet.
    pub picker: briefing::Picker,
    /// The name and branch the dialog proposed, so a task chosen later can propose its
    /// own unless they were edited.
    auto: (String, String),
    /// What the dialog proposed before it started from an issue or pull request (branch
    /// and folder), and the folder the item proposed: × puts the first two back unless
    /// they were edited since.
    plain: (String, String, String),
}
/// Claude Code's option that starts a conversation planning, without changing files.
const PLAN: [&str; 2] = ["--permission-mode", "plan"];
impl Form {
    pub fn proposed(&self) -> Record {
        let mut r = self.record.clone();
        r.destination = self.destination.clone().into();
        if self.mode != Mode::Resume {
            r.setup = self.setup_enabled.then(|| self.setup.clone());
        }
        if self.mode == Mode::Create {
            r.origin = self.briefing.as_ref().map(|b| b.origin.clone());
            // Plan first starts Claude Code in its plan mode; the review lists the option
            // with the other arguments.
            let planning = self.briefing.as_ref().is_some_and(|b| b.task() == brief::Task::Plan);
            if planning && r.provider == Provider::Claude && !r.command.args().windows(2).any(|w| w == PLAN) {
                let mut lines = r.command.args();
                lines.extend(PLAN.map(str::to_owned));
                r.command.arguments = lines.join("\n");
            }
        }
        r
    }
    /// The task chosen in the dialog: a new message and, unless they were edited, the
    /// name and branch that go with it.
    pub fn choose(&mut self, choice: briefing::Choice, saved: &[brief::Saved]) {
        let Some(b) = &mut self.briefing else { return };
        b.choose(choice, saved);
        self.propose();
    }
    /// The name, branch and folder for the item and task, each only where it is still
    /// what the dialog proposed: what the user typed stays.
    fn propose(&mut self) {
        let Some(b) = &self.briefing else { return };
        let (name, branch) = proposal(&b.origin, b.task());
        if self.record.name.trim().is_empty() || self.record.name == self.auto.0 {
            self.record.name.clone_from(&name);
        }
        if self.record.branch == self.plain.0 || self.record.branch == self.auto.1 {
            self.record.branch.clone_from(&branch);
        }
        if self.destination == self.plain.1 || self.destination == self.plain.2 {
            let folder = format!("rocket-agent-{}", self.record.branch.trim_start_matches("agent/").replace('/', "-"));
            let path = self.record.source.parent().unwrap_or(&self.record.source).join(folder);
            self.destination = crate::views::display_path(&path);
            self.plain.2.clone_from(&self.destination);
        }
        self.auto = (name, branch);
    }
}

/// The name and branch a session from `o` starts with for `task`. A name is at most 160
/// bytes, so a long title is cut.
fn proposal(o: &model::Origin, task: brief::Task) -> (String, String) {
    let (mut name, branch) = proposal_parts(o, task);
    if name.len() > 150 {
        let mut end = 149;
        while !name.is_char_boundary(end) {
            end -= 1;
        }
        name.truncate(end);
        name.push('…');
    }
    (name, branch)
}
fn proposal_parts(o: &model::Origin, task: brief::Task) -> (String, String) {
    use brief::Task;
    let n = o.number;
    match (o.kind, task) {
        (model::Kind::Pull, Task::Comments | Task::Checks) => (
            format!("#{n} {}", crate::i18n::tr(task.label())),
            format!("agent/{n}-{}", if task == Task::Comments { "review-comments" } else { "failing-checks" }),
        ),
        (model::Kind::Run, _) => (
            crate::i18n::trf!("Fix {run}", run = &o.title),
            format!("agent/run-{n}-failing-checks"),
        ),
        _ => {
            // A title without Latin letters or digits names the branch by its number only.
            let slug = briefing::slug(&o.title, 30);
            let branch = if slug.is_empty() { format!("agent/{n}") } else { format!("agent/{n}-{slug}") };
            (format!("#{n} {}", o.title), branch)
        }
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
    /// The list groups folded away: Needs you, Running, Not running.
    pub folded: [bool; 3],
    pub search: String,
    /// GitHub Copilot's program chosen with Browse; empty looks for it. Saved with the
    /// profile. Claude Code's and Codex's are the AI features' settings.
    pub copilot: String,
    pub changes: changes::State,
    pub integration: integration::State,
    /// Tasks the user saved, for sessions started from an issue, pull request or run.
    pub saved: Vec<brief::Saved>,
    /// The repository's hosting account while Hosting has one connected, for reading
    /// what a session starts from. Kept current by the app each frame.
    pub hosting: Option<briefing::Hosting>,
    /// The Manage tasks dialog: which saved task is open in it, if it is shown.
    pub managing: Option<Option<usize>>,
    /// Frames Manage tasks is raised above the session dialog it was opened from.
    pub tasks_raise: u8,
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
        self.integration.task.is_some()
            || self.form.as_ref().is_some_and(|f| {
                f.task.is_some() || f.picker.busy() || f.briefing.as_ref().is_some_and(briefing::Briefing::reading)
            })
            || self.sessions.iter().any(|s| s.job.is_some() || s.pending)
            || self.changes.busy()
    }
    pub fn stop_all(&mut self) {
        self.integration.stop();
        self.changes.release();
        if let Some(f) = &mut self.form {
            if let Some(t) = &f.task {
                t.cancel();
            }
            if let Some(t) = f.picker.task.take() {
                t.cancel();
            }
            if let Some(b) = &mut f.briefing {
                b.stop_read();
            }
        }
        for s in &mut self.sessions {
            s.stop();
        }
    }
    pub fn new_form(&mut self, source: PathBuf, drafting: &crate::commit_draft::Settings) {
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
            command: provider.command(Chosen::of(drafting, &self.copilot)),
            tracking: true,
            ..Default::default()
        };
        self.form = Some(Form {
            // The field reads as the folder would be typed: without the extended-length
            // prefix (`\\?\`) the repository's path carries on Windows. Review resolves
            // whatever the field holds.
            destination: crate::views::display_path(&r.destination),
            record: r,
            setup_enabled: false,
            setup: Command::default(),
            mode: Mode::Create,
            plan: None,
            task: None,
            error: None,
            found: None,
            briefing: None,
            picker: briefing::Picker::default(),
            auto: (String::new(), String::new()),
            plain: Default::default(),
        });
        // What a plain session proposes, so an item picked later can tell what was typed.
        if let Some(f) = &mut self.form {
            f.plain = (f.record.branch.clone(), f.destination.clone(), String::new());
        }
    }
    /// A new session from an issue, a pull request or a CI run: the name, branch, base
    /// and folder proposed for it, and the host's text read, unless the page it came from
    /// already has it (`material`). A pull request's session starts on its head (`base`).
    pub fn new_form_from(
        &mut self,
        source: PathBuf,
        drafting: &crate::commit_draft::Settings,
        origin: Origin,
        material: Option<briefing::Material>,
        base: Option<String>,
        ctx: &Context,
    ) {
        self.new_form(source, drafting);
        self.brief_form(origin, material, base, ctx);
    }
    /// The open dialog starts from `origin` from now on.
    fn brief_form(&mut self, origin: Origin, material: Option<briefing::Material>, base: Option<String>, ctx: &Context) {
        let Some(f) = &mut self.form else { return };
        let mut b = briefing::Briefing::new(origin, material);
        if let Some(base) = base {
            f.record.base = base;
        }
        if b.material.is_some() {
            b.rebuild(&self.saved);
        } else {
            match &self.hosting {
                Some(h) => {
                    if let Err(e) = b.start_read(h, ctx) {
                        b.error = Some(user_error(&e));
                    }
                }
                None => b.error = Some(crate::i18n::tr("Connect this repository's hosting account in Hosting to read it.").into()),
            }
        }
        f.briefing = Some(b);
        f.propose();
    }
    /// The Issue or pull request field's choice.
    pub fn pick(&mut self, found: briefing::Found, ctx: &Context) {
        let Some(h) = self.hosting.clone() else { return };
        // An issue found by its number was read whole already.
        let material = found.issue.as_ref().map(|i| briefing::Material::Issue((**i).clone()));
        let origin = Origin {
            kind: found.kind,
            number: found.number,
            title: if found.title.is_empty() { format!("#{}", found.number) } else { found.title },
            url: found.url,
            host: h.host.clone(),
            repository: h.repo.path().to_owned(),
            push: None,
        };
        if let Some(f) = &mut self.form {
            f.picker = briefing::Picker::default();
        }
        self.brief_form(origin, material, None, ctx);
    }
    /// × on the dialog's source card: a session of the user's own, as before.
    pub fn unbrief(&mut self) {
        if let Some(f) = &mut self.form
            && let Some(mut b) = f.briefing.take()
        {
            b.stop_read();
            // What the item proposed goes; what the user typed stays.
            if f.record.name == f.auto.0 {
                f.record.name.clear();
            }
            if f.record.branch == f.auto.1 {
                f.record.branch.clone_from(&f.plain.0);
            }
            if f.destination == f.plain.2 {
                f.destination.clone_from(&f.plain.1);
            }
            f.auto = Default::default();
            f.record.base = "HEAD".into();
            f.plan = None;
        }
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
            found: None,
            briefing: None,
            picker: briefing::Picker::default(),
            auto: (String::new(), String::new()),
            plain: Default::default(),
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
        // The first message goes into the agent's input as reviewed, and never with
        // what looks like a secret in it.
        let first = match (&f.briefing, plan.mode) {
            (Some(b), Mode::Create) => {
                if !b.ready() || !b.reviewed() {
                    return Err(model::invalid("Review the host-sourced message and acknowledge the agent's access before starting."));
                }
                if rocket_git::scan_text_for_secret(&b.message).is_some() {
                    return Err(model::invalid("Remove what looks like a secret from the first message first."));
                }
                let text = brief::sendable(&b.message);
                (!text.trim().is_empty()).then(|| (b.origin.short(), text))
            }
            _ => None,
        };
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
        s.first = first;
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
        // The session just started must show in the list.
        self.folded = [false; 3];
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
        if let Some(f) = &mut self.form {
            let mut read = false;
            match &mut f.briefing {
                Some(b) => {
                    let reading = b.reading();
                    b.poll(&self.saved);
                    read = reading && !b.reading() && b.material.is_some();
                    // A pull request's session starts on the head that was just read, and
                    // pushes back to its branch on the remote that was this repository
                    // when the read started.
                    if read && let Some(briefing::Material::Pull { details, .. }) = b.material.as_deref() {
                        let p = details.summary();
                        f.record.base.clone_from(&p.head);
                        if b.origin.push.is_none() {
                            b.origin.push = briefing::push_back(b.remote(), &p.source, &p.head);
                        }
                    }
                    b.scan();
                }
                None if f.mode == Mode::Create => f.picker.poll(self.hosting.as_ref(), ctx),
                None => {}
            }
            // The title as read: the name, branch and folder it proposes, where untouched.
            if read {
                f.propose();
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
                        // The agent's own terminal, not setup's, gets the first message.
                        if !started.setup
                            && let Some((from, text)) = s.first.take()
                        {
                            t.command = text;
                            t.first = Some(from);
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
        } else if visible && self.form.is_none() {
            // Another tab of this page: the watch stays, so coming back to Changes knows
            // whether anything needs reading.
            self.changes.suspend();
        } else {
            // A session dialog or another page may move or remove the worktree.
            self.changes.release();
        }
        if self.working() || self.active() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
}
