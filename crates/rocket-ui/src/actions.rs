//! Actions: the repository's CI runs from its host, newest first, one run's
//! jobs, and the reviewed re-run of that run's failed jobs. Reads only while the
//! page shows: one request per page of runs or branch filter, the selected run's
//! jobs when it is chosen, and the newest page again every 30 s while a run runs.
use crate::{
    app::*,
    ci_checks::{self, ListStyle, RerunSource, host_name, paint_check, rerun_label},
    pages::{self, control},
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

const RUNNING_EVERY: Duration = Duration::from_secs(30);
const AFTER_RERUN: Duration = Duration::from_secs(8);
const AFTER_RATE_LIMIT: Duration = Duration::from_secs(600);
/// Coming back to the page reads again when the list is older than this.
const STALE_AFTER: Duration = Duration::from_secs(60);
/// Below this width the list and a run's details take turns.
const COMPACT: f32 = 850.0;

/// Which runs of the loaded list show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Show {
    #[default]
    All,
    Failed,
    Running,
}
impl Show {
    pub fn key(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Failed => "failed",
            Self::Running => "running",
        }
    }
    fn includes(self, state: ForgeCheckState) -> bool {
        match self {
            Self::All => true,
            Self::Failed => state == ForgeCheckState::Failed,
            Self::Running => matches!(state, ForgeCheckState::Running | ForgeCheckState::Queued),
        }
    }
}

/// Whether the page can list runs, or what stands in the way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reach {
    Ready,
    /// A remembered account for this server is signing in.
    SigningIn,
    /// The remembered account's sign-in was refused.
    Rejected { host: ForgeProvider, server: String },
    /// The remembered account's server couldn't be reached; Rocket tries again.
    Unreachable { host: ForgeProvider, server: String },
    /// The repository is on a public host Rocket reads, without an account for it.
    NoAccount { host: ForgeProvider, server: String, path: String },
    /// Bitbucket Server doesn't report CI to Rocket.
    BitbucketServer { url: String },
    /// No remote on a host Rocket reads.
    NoRemote,
}
impl Reach {
    pub fn key(&self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::SigningIn => "signing_in",
            Self::Rejected { .. } => "rejected",
            Self::Unreachable { .. } => "unreachable",
            Self::NoAccount { .. } => "no_account",
            Self::BitbucketServer { .. } => "bitbucket_server",
            Self::NoRemote => "no_remote",
        }
    }
}

/// What a list was read for; a change starts the list again. The session is held,
/// so an account signed in again later can't pass for this one.
#[derive(Clone)]
struct Key {
    repository: String,
    branch: Option<String>,
    session: Arc<ForgeSession>,
}
impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        self.repository == other.repository && self.branch == other.branch && Arc::ptr_eq(&self.session, &other.session)
    }
}

/// One repository tab's Actions page.
#[derive(Default)]
pub struct State {
    pub reach: Option<Reach>,
    /// The branch a link chose for the second filter segment; None is the current branch.
    pub branch: Option<String>,
    /// The list is for one branch, not all of them.
    pub on_branch: bool,
    pub show: Show,
    pub runs: Vec<ForgeCiRun>,
    pub more: bool,
    pub pages: usize,
    key: Option<Key>,
    task: Option<(Key, usize, ReadTask<ForgeRunList>)>,
    /// The last read's failure; a list read before it stays, faded.
    pub failure: Option<ForgeFailure>,
    pub error: Option<String>,
    /// The host refused to list runs, and the permission it named.
    pub hidden: Option<Option<String>>,
    /// "21:02": when the list was read.
    pub read_at: Option<String>,
    read: Option<Instant>,
    /// The rate limit is used up until this local time.
    pub paused_until: Option<String>,
    due: Option<Instant>,
    pub selected: Option<u64>,
    /// A run a link selected, shown until the list has it.
    linked: Option<ForgeCiRun>,
    pub details: Option<Arc<ForgeRunDetails>>,
    details_task: Option<(u64, ReadTask<ForgeRunDetails>)>,
    /// The list read the details were read after.
    details_reads: u64,
    details_want: bool,
    /// The run whose details read failed, so it isn't read again every frame.
    details_failed: Option<u64>,
    pub details_error: Option<String>,
    /// Narrow window: the selected run's details show instead of the list.
    pub focused: bool,
    /// Successful list reads, for MCP state and runners.
    pub reads: u64,
    /// The page asked for this page of runs.
    want: Option<usize>,
    /// The runs are read with Hosting's connected account, which can re-run.
    pub live: bool,
    /// The last frame the page showed: a gap means it was left and opened again.
    seen: u64,
}
impl State {
    pub fn reading(&self) -> bool {
        self.task.is_some()
    }
    pub fn reading_details(&self) -> bool {
        self.details_task.is_some()
    }
    /// The run the details pane shows: from the list, or the one a link chose.
    pub fn selected_run(&self) -> Option<&ForgeCiRun> {
        let id = self.selected?;
        self.runs.iter().find(|r| r.id == id).or(self.linked.as_ref().filter(|r| r.id == id))
    }
    /// Read the newest page and the selected run again.
    pub fn refresh(&mut self) {
        self.want = Some(1);
        self.details_want = true;
        self.details_failed = None;
    }
    /// After a re-run started: the list and the run read again shortly.
    pub(crate) fn rerun_started(&mut self) {
        self.due = Some(Instant::now() + AFTER_RERUN);
        self.details_failed = None;
    }
    fn select(&mut self, id: u64) {
        if self.selected != Some(id) {
            self.selected = Some(id);
            self.details_error = None;
            self.details_failed = None;
        }
    }
    fn take(&mut self, list: ForgeRunList, page: usize, now: Instant) {
        self.failure = None;
        self.error = None;
        self.paused_until = None;
        self.hidden = list.hidden.then(|| list.missing.clone());
        if page == 1 {
            // A refresh: the newest page replaces the top; older pages read before stay below.
            let mut runs = list.runs;
            if self.pages > 1 {
                for old in self.runs.drain(..) {
                    if !runs.iter().any(|r| r.id == old.id) {
                        runs.push(old);
                    }
                }
            } else {
                self.more = list.more;
            }
            self.runs = runs;
            self.pages = self.pages.max(1);
        } else {
            for run in list.runs {
                match self.runs.iter_mut().find(|r| r.id == run.id) {
                    Some(r) => *r = run,
                    None => self.runs.push(run),
                }
            }
            self.more = list.more;
            self.pages = page;
        }
        self.reads += 1;
        self.read = Some(now);
        self.read_at = Some(clock(0));
        let running = self.runs.iter().any(|r| matches!(r.state, ForgeCheckState::Running | ForgeCheckState::Queued));
        self.due = running.then(|| now + RUNNING_EVERY);
        if self.linked.as_ref().is_some_and(|l| self.runs.iter().any(|r| r.id == l.id)) {
            self.linked = None;
        }
        // The run read alone says more than its row in a list (GitLab's duration):
        // keep that row while the run is unchanged.
        if let Some(d) = &self.details
            && let Some(row) = self.runs.iter_mut().find(|r| r.id == d.run.id && r.attempt == d.run.attempt && r.state == d.run.state)
        {
            *row = d.run.clone();
        }
    }
    /// Another repository, account or branch filter: the list starts again.
    fn restart(&mut self, key: Key) {
        if let Some((_, _, task)) = self.task.take() {
            task.cancel();
        }
        // Before the first list, any selection came from a link: keep it.
        let other = self.key.as_ref().is_some_and(|k| k.repository != key.repository || !Arc::ptr_eq(&k.session, &key.session));
        if other {
            if let Some((_, task)) = self.details_task.take() {
                task.cancel();
            }
            self.selected = None;
            self.linked = None;
            self.details = None;
            self.details_error = None;
            self.details_failed = None;
            self.focused = false;
        }
        self.runs.clear();
        self.more = false;
        self.pages = 0;
        self.failure = None;
        self.error = None;
        self.hidden = None;
        self.paused_until = None;
        self.read = None;
        self.read_at = None;
        self.due = None;
        self.key = Some(key);
        self.want = Some(1);
    }
}

/// What the page asks of the app.
#[derive(Clone, Debug)]
pub enum ActionsIntent {
    /// Review re-running the selected run's failed jobs.
    Rerun,
    /// Open a run, a job or the host's runs page in the browser.
    Open(String),
    /// Preferences → Accounts, with this host chosen when no account is being edited.
    Connect { host: ForgeProvider, server: String },
    /// Show this commit in Commits.
    ShowCommit(String),
    /// Sign the remembered account in again now.
    RetrySignIn,
}

/// "14:20" on this computer's clock, `seconds` from now.
fn clock(seconds: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    rocket_git::local_time(now + seconds * 1000).map_or_else(String::new, |t| format!("{:02}:{:02}", t.hour, t.minute))
}
fn now_seconds() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}
fn failure_text(e: &rocket_git::Error) -> String {
    match e {
        rocket_git::Error::Forge(f) => format!("{} ({})", f.guidance(), f.code()),
        other => crate::backend::user_error(other),
    }
}
/// The host's name for its CI, as the page's subtitle starts.
pub(crate) fn ci_name(host: ForgeProvider) -> &'static str {
    match host {
        ForgeProvider::GitHub | ForgeProvider::GitHubEnterprise => "GitHub Actions",
        ForgeProvider::GitLab => "GitLab pipelines",
        ForgeProvider::Gitea => "Gitea Actions",
        ForgeProvider::Bitbucket => "Bitbucket Pipelines",
        ForgeProvider::Azure | ForgeProvider::AzureServer => "Azure Pipelines",
        ForgeProvider::BitbucketServer => "Bitbucket Server",
    }
}
/// The current branch's short name, when HEAD is on one.
fn current_branch(tab: &Tab) -> Option<String> {
    let snapshot = tab.snapshot.as_ref()?;
    snapshot.branches.iter().find(|b| b.is_head && b.kind == BranchKind::Local).map(|b| short_ref(&b.reference))
}

impl Rocket {
    /// Opens Actions on the current tab; with a branch, a run or both, on that
    /// branch with that run selected.
    pub(crate) fn open_actions(&mut self, branch: Option<String>, run: Option<ForgeCiRun>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let a = &mut tab.actions;
        if branch.is_some() || run.is_some() {
            a.on_branch = branch.is_some();
            a.branch = branch;
            a.show = Show::All;
            match run {
                Some(run) => {
                    a.select(run.id);
                    a.focused = true;
                    a.linked = Some(run);
                }
                // A branch alone: its runs, and the wide page chooses one.
                None => {
                    a.selected = None;
                    a.linked = None;
                    a.focused = false;
                }
            }
        }
        self.dispatch(Intent::View(View::Actions));
    }
    pub(crate) fn actions_intent(&mut self, intent: ActionsIntent) {
        match intent {
            ActionsIntent::Rerun => {
                let Some(tab) = self.tab() else { return };
                let (Some(details), Some(session)) = (tab.actions.details.clone(), tab.ci.session().cloned()) else { return };
                // Only Hosting's connected account re-runs; a quiet sign-in only reads.
                if !self.forge.session.as_ref().is_some_and(|s| Arc::ptr_eq(s, &session)) {
                    return;
                }
                let id = tab.id;
                self.open_ci_rerun(Some(id), RerunSource::Run(details), session);
            }
            ActionsIntent::Open(url) => {
                let ctx = self.context.clone();
                self.open_ci_link(&ctx, url);
            }
            ActionsIntent::Connect { host, server } => {
                // Choose the host only when no account is being entered or connected.
                if self.forge.session.is_none() && self.forge.token.is_empty() {
                    self.forge.provider = host;
                    self.forge.server = server;
                }
                self.dispatch(Intent::Preferences(crate::preferences::Page::Accounts));
            }
            ActionsIntent::ShowCommit(id) => {
                if let Ok(id) = ObjectId::parse(&id) {
                    self.dispatch(Intent::SelectCommit(id));
                }
            }
            ActionsIntent::RetrySignIn => self.retry_ci_sign_in(),
        }
    }
    /// Finishes reads for every tab, and reads for the tab whose Actions page shows.
    pub(crate) fn poll_actions(&mut self, ctx: &Context) {
        let now = Instant::now();
        for tab in &mut self.tabs {
            finish(&mut tab.actions, now);
        }
        let shown = self.page().is_none() && self.tabs.get(self.active).is_some_and(|t| t.view == View::Actions && t.snapshot.is_some());
        if !shown {
            return;
        }
        let reach = self.actions_reach(&self.tabs[self.active]);
        let live = self.forge.session.clone();
        let backend = self.backend.clone();
        let focused = ctx.input(|i| i.focused);
        let tab = &mut self.tabs[self.active];
        let current = current_branch(tab);
        let repository = tab.ci.repository.clone();
        let session = tab.ci.session().cloned();
        let a = &mut tab.actions;
        let frame = ctx.cumulative_frame_nr();
        let entered = a.seen + 1 != frame;
        a.seen = frame;
        a.live = live.as_ref().zip(session.as_ref()).is_some_and(|(l, s)| Arc::ptr_eq(l, s));
        a.reach = Some(reach.clone());
        let (Reach::Ready, Some(repository), Some(session), Some(backend)) = (reach, repository, session, backend) else { return };
        if a.branch.is_none() && current.is_none() {
            a.on_branch = false;
        }
        let filter = a.on_branch.then(|| a.branch.clone().or(current)).flatten();
        let key = Key { repository: repository.path().to_owned(), branch: filter.clone(), session: session.clone() };
        if a.key.as_ref() != Some(&key) {
            a.restart(key.clone());
        }
        let due = a.due.is_some_and(|d| now >= d) && focused;
        let stale = a.read.is_none_or(|r| now.duration_since(r) > STALE_AFTER) && a.paused_until.is_none();
        // A read of the newest page in flight answers a refresh asked meanwhile.
        if a.want == Some(1) && a.task.as_ref().is_some_and(|(k, page, _)| *page == 1 && a.key.as_ref() == Some(k)) {
            a.want = None;
        }
        // Timers and coming back ask only when nothing is being read; a page asked
        // for (Load more, Refresh) waits for the read in flight.
        let page = a.want.or_else(|| (due || (entered && stale)).then_some(1));
        if a.task.is_none()
            && let Some(page) = page
        {
            a.want = None;
            a.due = None;
            let (r, s, b, k) = (repository.clone(), session.clone(), filter, key);
            let backend = backend.clone();
            match crate::read_task::spawn(ctx, move |cancel| backend.forge_request("hosting-ci-runs", &s, |s| s.ci_runs(&r, b.as_deref(), page, &cancel))) {
                Ok(task) => a.task = Some((k, page, task)),
                Err(e) => a.error = Some(failure_text(&e)),
            }
        }
        if let Some(d) = a.due.filter(|_| focused) {
            ctx.request_repaint_after(d.saturating_duration_since(now).max(Duration::from_millis(250)));
        }
        // The selected run's jobs: when chosen, after a refresh while it runs or
        // when its attempt changed, and when asked.
        let Some(run) = a.selected_run().cloned() else { return };
        let have = a.details.as_ref().filter(|d| d.run.id == run.id);
        let changed = have.is_some_and(|d| {
            a.reads != a.details_reads
                && (d.run.attempt != run.attempt || d.run.state != run.state || matches!(d.run.state, ForgeCheckState::Running | ForgeCheckState::Queued))
        });
        let wanted = (have.is_none() && a.details_failed != Some(run.id)) || changed || a.details_want;
        let reading = a.details_task.as_ref().is_some_and(|(id, _)| *id == run.id);
        if wanted && !reading {
            if let Some((_, task)) = a.details_task.take() {
                task.cancel();
            }
            a.details_want = false;
            a.details_reads = a.reads;
            let id = run.id;
            match crate::read_task::spawn(ctx, move |cancel| backend.forge_request("hosting-ci-run", &session, |s| s.ci_run(&repository, &run, &cancel))) {
                Ok(task) => a.details_task = Some((id, task)),
                Err(e) => a.details_error = Some(failure_text(&e)),
            }
        }
    }
}

/// Keeps a finished read's answer when it is still for what the page shows.
fn finish(a: &mut State, now: Instant) {
    if let Some((key, page, task)) = &mut a.task
        && let Some(result) = task.try_finish()
    {
        let (key, page) = (key.clone(), *page);
        a.task = None;
        if a.key.as_ref() == Some(&key) {
            match result {
                Ok(list) => a.take(list, page, now),
                Err(rocket_git::Error::Cancelled) => {}
                Err(e @ rocket_git::Error::Forge(ForgeFailure::RateLimited)) => {
                    a.failure = Some(ForgeFailure::RateLimited);
                    a.error = Some(failure_text(&e));
                    a.paused_until = Some(clock(AFTER_RATE_LIMIT.as_secs()));
                    a.due = Some(now + AFTER_RATE_LIMIT);
                }
                Err(e) => {
                    a.failure = match &e {
                        rocket_git::Error::Forge(f) => Some(*f),
                        _ => None,
                    };
                    a.error = Some(failure_text(&e));
                    a.due = None;
                }
            }
        }
    }
    if let Some((id, task)) = &mut a.details_task
        && let Some(result) = task.try_finish()
    {
        let id = *id;
        a.details_task = None;
        if a.selected == Some(id) {
            match result {
                Ok(details) => {
                    // The run read alone corrects its row.
                    if let Some(row) = a.runs.iter_mut().find(|r| r.id == id) {
                        *row = details.run.clone();
                    }
                    a.details = Some(Arc::new(details));
                    a.details_error = None;
                }
                Err(rocket_git::Error::Cancelled) => {}
                Err(e) => {
                    a.details_failed = Some(id);
                    a.details_error = Some(failure_text(&e));
                }
            }
        }
    }
}

/// The page.
pub(crate) fn view(ui: &mut Ui, tab: &mut Tab, intents: &mut Vec<Intent>, widgets: &mut HashMap<String, UiControl>) {
    ui.set_clip_rect(ui.clip_rect().intersect(ui.available_rect_before_wrap()));
    let compact = ui.available_width() < COMPACT;
    let reach = tab.actions.reach.clone().unwrap_or(Reach::SigningIn);
    let host = tab.ci.host();
    let session = tab.ci.session().cloned();
    let repository = tab.ci.repository.clone();
    let ready = reach == Reach::Ready && repository.is_some() && session.is_some();
    // Header: what CI this is, for which repository and account.
    let subtitle = match (&reach, &repository, &session) {
        (Reach::Ready, Some(r), Some(s)) if !compact => crate::i18n::trf!("{service} · {repository} · signed in as {account}", service = ci_name(host), repository = r.path(), account = s.login()),
        (Reach::Ready, Some(r), _) => format!("{} · {}", ci_name(host), r.path()),
        (Reach::NoAccount { host, path, .. }, _, _) => format!("{} · {path}", host_name(*host)),
        _ => crate::i18n::tr("CI runs from the repository's host").to_string(),
    };
    let reading = tab.actions.reading();
    let mut refresh = false;
    pages::header_with(
        ui,
        |painter, tile| icon(painter, tile.shrink(10.0), Icon::Actions, BLUE()),
        crate::i18n::tr("Actions"),
        &subtitle,
        |ui| {
            if !ready {
                return;
            }
            // Right to left: Open on the host at the edge, Refresh before it.
            if !compact
                && let Some(r) = &repository
                && external_button(ui, &crate::i18n::trf!("Open on {host}", host = host_name(host)), widgets, "actions-open-host")
            {
                intents.push(Intent::Actions(ActionsIntent::Open(r.runs_url())));
            }
            let r = crate::chrome::icon_button(ui, Icon::Refresh, !reading, false, 28.0).on_hover_text(crate::i18n::tr("Refresh runs"));
            control(widgets, "actions-refresh", &r, ui.clip_rect());
            refresh = r.clicked();
        },
    );
    if refresh {
        tab.actions.refresh();
    }
    if !ready {
        reach_state(ui, &reach, intents, widgets);
        return;
    }
    let (Some(repository), Some(session)) = (repository, session) else { return };
    if let Some(missing) = tab.actions.hidden.clone() {
        let (what, give) = permission(host, missing.as_deref());
        empty_state(
            ui,
            Icon::Lock,
            &crate::i18n::trf!("{account}'s token can't read {what} for {repository}", account = session.login(), what, repository = repository.path()),
            &crate::i18n::trf!("{permission}, then refresh.", permission = give),
            widgets,
            |ui, widgets| {
                let r = pages::primary(ui, true, crate::i18n::tr("Accounts"));
                control(widgets, "actions-accounts", &r, ui.clip_rect());
                if r.clicked() {
                    intents.push(Intent::Preferences(crate::preferences::Page::Accounts));
                }
            },
        );
        return;
    }
    let current = current_branch(tab);
    // The filters stay while a branch's list is empty or failed: All branches is a click away.
    if !tab.actions.runs.is_empty() || tab.actions.on_branch {
        filters(ui, tab, current.as_deref(), compact, widgets);
    }
    strips(ui, tab, host, &session, intents, widgets);
    let a = &tab.actions;
    if a.runs.is_empty() {
        if a.failure == Some(ForgeFailure::Unauthorized) {
            empty_state(ui, Icon::Lock, &crate::i18n::trf!("{host} rejected {account}'s token", host = host_name(host), account = session.login()), crate::i18n::tr("Reconnect the account in Accounts."), widgets, |ui, widgets| {
                let r = pages::primary(ui, true, crate::i18n::tr("Accounts"));
                control(widgets, "actions-accounts", &r, ui.clip_rect());
                if r.clicked() {
                    intents.push(Intent::Preferences(crate::preferences::Page::Accounts));
                }
            });
            return;
        }
        if let Some(error) = a.error.clone().filter(|_| !a.reading()) {
            let title = if a.failure == Some(ForgeFailure::RateLimited) { crate::i18n::trf!("{host}'s rate limit is used up", host = host_name(host)) } else { crate::i18n::tr("Couldn't read runs").to_string() };
            let text = match &a.paused_until {
                Some(at) => crate::i18n::trf!("Runs refresh at {at}.", at),
                None => error,
            };
            empty_state(ui, Icon::Warning, &title, &text, widgets, |ui, widgets| {
                let r = pages::ghost(ui, true, crate::i18n::tr("Refresh"));
                control(widgets, "actions-retry", &r, ui.clip_rect());
                if r.clicked() {
                    refresh = true;
                }
            });
            if refresh {
                tab.actions.refresh();
            }
            return;
        }
        if a.reading() || a.reads == 0 {
            skeleton(ui);
            return;
        }
        if !a.on_branch {
            let url = repository.runs_url();
            let (title, text, button) = no_runs(host, repository.path());
            empty_state(ui, Icon::Actions, &title, text, widgets, |ui, widgets| {
                if external_button(ui, &button, widgets, "actions-open-host") {
                    intents.push(Intent::Actions(ActionsIntent::Open(url.clone())));
                }
            });
            return;
        }
    }
    // Subjects the host didn't send come from loaded history, as does Show in Commits.
    let wanted: Vec<&str> = tab.actions.runs.iter().filter(|r| r.subject.is_none()).filter_map(|r| r.commit.as_deref()).collect();
    let subjects: HashMap<String, String> = tab.loaded_commits().iter().filter(|c| wanted.contains(&c.id.as_str())).map(|c| (c.id.as_str().to_owned(), c.subject_lossy().into_owned())).collect();
    let loaded: Vec<String> = tab.actions.selected_run().and_then(|r| r.commit.clone()).filter(|id| tab.loaded_commits().iter().any(|c| c.id.as_str() == id)).into_iter().collect();
    let a = &mut tab.actions;
    // A wide page always shows a run: the newest failed one, else the newest,
    // also when a filter or a new list left the chosen run out.
    if !compact && a.selected_run().is_none() {
        let shown: Vec<&ForgeCiRun> = a.runs.iter().filter(|r| a.show.includes(r.state)).collect();
        if let Some(id) = shown.iter().find(|r| r.state == ForgeCheckState::Failed).or(shown.first()).map(|r| r.id) {
            a.select(id);
        }
    }
    let body = ui.available_rect_before_wrap();
    let margin = if compact { 16.0 } else { 24.0 };
    let body = Rect::from_min_max(pos2(body.left() + margin, body.top()), pos2(body.right() - margin, body.bottom() - 14.0));
    if body.height() < 60.0 {
        return;
    }
    if compact {
        if a.focused && a.selected_run().is_some() {
            let mut child = ui.new_child(UiBuilder::new().max_rect(body).layout(Layout::top_down(Align::Min)));
            let r = child.add(Button::new(RichText::new(crate::i18n::tr("‹ All runs")).size(12.5).color(BLUE())).frame(false));
            control(widgets, "actions-back", &r, child.clip_rect());
            let back = r.clicked();
            let rest = Rect::from_min_max(pos2(body.left(), child.cursor().top() + 4.0), body.max);
            details_card(ui, rest, a, host, &subjects, &loaded, intents, widgets);
            if back {
                a.focused = false;
            }
        } else {
            list_card(ui, body, a, compact, &subjects, widgets);
        }
    } else {
        let width = (body.width() * 0.48).clamp(380.0, 640.0);
        let left = Rect::from_min_max(body.min, pos2(body.left() + width, body.bottom()));
        let right = Rect::from_min_max(pos2(left.right() + 14.0, body.top()), body.max);
        list_card(ui, left, a, compact, &subjects, widgets);
        if right.width() > 200.0 {
            details_card(ui, right, a, host, &subjects, &loaded, intents, widgets);
        }
    }
    ui.allocate_rect(body, Sense::hover());
}

/// A button with the external-link mark after its words. Returns whether it was clicked.
fn external_button(ui: &mut Ui, label: &str, widgets: &mut HashMap<String, UiControl>, id: &str) -> bool {
    let galley = ui.painter().layout_no_wrap(label.to_owned(), FontId::proportional(13.0), TEXT());
    let size = vec2(galley.size().x + 38.0, CONTROL);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let hovered = response.hovered() || response.has_focus();
    ui.painter().rect(rect, 6.0, if hovered { HOVER() } else { RAISED() }, Stroke::new(1.0, BORDER()), StrokeKind::Inside);
    ui.painter().galley(pos2(rect.left() + 12.0, rect.center().y - galley.size().y / 2.0), galley, TEXT());
    icon(ui.painter(), Rect::from_center_size(pos2(rect.right() - 15.0, rect.center().y), vec2(12.0, 12.0)), Icon::External, MUTED());
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, label));
    control(widgets, id, &response, ui.clip_rect());
    response.on_hover_cursor(CursorIcon::PointingHand).clicked()
}

/// (what the token can't read, what to give it) on this host.
fn permission(host: ForgeProvider, missing: Option<&str>) -> (&'static str, String) {
    match host {
        ForgeProvider::GitHub | ForgeProvider::GitHubEnterprise => (crate::i18n::tr("Actions"), crate::i18n::trf!("Give it {permission} (fine-grained) or repo (classic)", permission = missing.unwrap_or("Actions: Read"))),
        ForgeProvider::GitLab => (crate::i18n::tr("pipelines"), crate::i18n::tr("Give it the read_api scope").into()),
        ForgeProvider::Gitea => (crate::i18n::tr("Actions"), crate::i18n::tr("Give it read:repository").into()),
        ForgeProvider::Bitbucket => (crate::i18n::tr("pipelines"), crate::i18n::tr("Give it read:pipeline:bitbucket").into()),
        ForgeProvider::Azure | ForgeProvider::AzureServer => (crate::i18n::tr("builds"), crate::i18n::tr("Give it Build (Read)").into()),
        ForgeProvider::BitbucketServer => (crate::i18n::tr("builds"), crate::i18n::tr("Check the account in Accounts").into()),
    }
}

/// (title, text, button) when the host has no run for the repository at all.
fn no_runs(host: ForgeProvider, path: &str) -> (String, &'static str, String) {
    match host {
        ForgeProvider::GitHub | ForgeProvider::GitHubEnterprise => (crate::i18n::trf!("No workflow has run for {path}", path), crate::i18n::tr("Add a workflow under .github/workflows and push. Its runs show here and beside your commits."), crate::i18n::tr("Open Actions on GitHub").into()),
        ForgeProvider::Gitea => (crate::i18n::trf!("No workflow has run for {path}", path), crate::i18n::tr("Add a workflow under .gitea/workflows and push. Its runs show here and beside your commits."), crate::i18n::tr("Open Actions on Gitea").into()),
        ForgeProvider::GitLab => (crate::i18n::trf!("No pipeline has run for {path}", path), crate::i18n::tr("Add a .gitlab-ci.yml file and push. Its pipelines show here and beside your commits."), crate::i18n::tr("Open pipelines on GitLab").into()),
        ForgeProvider::Bitbucket => (crate::i18n::trf!("No pipeline has run for {path}", path), crate::i18n::tr("Add a bitbucket-pipelines.yml file and push. Its pipelines show here and beside your commits."), crate::i18n::tr("Open Pipelines on Bitbucket").into()),
        _ => (crate::i18n::trf!("No pipeline has built {path}", path), crate::i18n::tr("Create a pipeline for this repository in Azure Pipelines. Its builds show here and beside your commits."), crate::i18n::tr("Open Pipelines on Azure DevOps").into()),
    }
}

/// The page when it can't list runs: what stands in the way and the one step that helps.
fn reach_state(ui: &mut Ui, reach: &Reach, intents: &mut Vec<Intent>, widgets: &mut HashMap<String, UiControl>) {
    match reach {
        Reach::Ready => {}
        Reach::SigningIn => skeleton(ui),
        Reach::NoAccount { host, server, path } => {
            let name = host_name(*host);
            let site = ci_checks::server_name(server);
            empty_state(
                ui,
                Icon::Remote,
                &crate::i18n::trf!("Connect {name} to see Actions", name),
                &crate::i18n::trf!("This repository is on {site} as {path}. Rocket reads its runs and checks with a {name} account.", site, path, name),
                widgets,
                |ui, widgets| {
                    let r = pages::primary(ui, true, &crate::i18n::trf!("Connect {name}", name));
                    control(widgets, "actions-connect", &r, ui.clip_rect());
                    if r.clicked() {
                        intents.push(Intent::Actions(ActionsIntent::Connect { host: *host, server: server.clone() }));
                    }
                },
            );
        }
        Reach::Rejected { host, server } => empty_state(
            ui,
            Icon::Lock,
            &crate::i18n::trf!("{host} didn't accept the remembered account", host = host_name(*host)),
            &crate::i18n::trf!("Rocket couldn't sign in to {server} with it. Check the account in Accounts.", server),
            widgets,
            |ui, widgets| {
                let r = pages::primary(ui, true, crate::i18n::tr("Accounts"));
                control(widgets, "actions-accounts", &r, ui.clip_rect());
                if r.clicked() {
                    intents.push(Intent::Preferences(crate::preferences::Page::Accounts));
                }
            },
        ),
        Reach::Unreachable { host, server } => empty_state(
            ui,
            Icon::Warning,
            &crate::i18n::trf!("Couldn't reach {server}", server),
            &crate::i18n::trf!("Rocket signs in to {host} again in a few minutes.", host = host_name(*host)),
            widgets,
            |ui, widgets| {
                let r = pages::ghost(ui, true, crate::i18n::tr("Try again"));
                control(widgets, "actions-retry", &r, ui.clip_rect());
                if r.clicked() {
                    intents.push(Intent::Actions(ActionsIntent::RetrySignIn));
                }
            },
        ),
        Reach::BitbucketServer { url } => empty_state(
            ui,
            Icon::Info,
            crate::i18n::tr("Bitbucket Server doesn't report CI to Rocket"),
            crate::i18n::tr("Builds from Jenkins or Bamboo show on the server's pull requests and commits."),
            widgets,
            |ui, widgets| {
                if external_button(ui, crate::i18n::tr("Open on Bitbucket Server"), widgets, "actions-open-host") {
                    intents.push(Intent::Actions(ActionsIntent::Open(url.clone())));
                }
            },
        ),
        Reach::NoRemote => empty_state(
            ui,
            Icon::Branch,
            crate::i18n::tr("No remote on a host Rocket reads"),
            crate::i18n::tr("Actions shows runs from GitHub, GitHub Enterprise, GitLab, Gitea, Bitbucket Cloud and Azure DevOps."),
            widgets,
            |ui, widgets| {
                let r = pages::ghost(ui, true, crate::i18n::tr("Remotes"));
                control(widgets, "actions-remotes", &r, ui.clip_rect());
                if r.clicked() {
                    intents.push(Intent::View(View::Remotes));
                }
            },
        ),
    }
}

/// A centred card: an icon tile, a title, a sentence, its buttons and the guide link.
fn empty_state(ui: &mut Ui, image: Icon, title: &str, text: &str, widgets: &mut HashMap<String, UiControl>, buttons: impl FnOnce(&mut Ui, &mut HashMap<String, UiControl>)) {
    let area = ui.available_rect_before_wrap();
    let width = (area.width() - 32.0).clamp(240.0, 560.0);
    let height = 300.0_f32.min(area.height() - 24.0).max(200.0);
    let rect = Rect::from_center_size(pos2(area.center().x, area.top() + 16.0 + height / 2.0), vec2(width, height));
    ui.painter().rect(rect, 8.0, VIEW_BG(), Stroke::new(1.0, CARD_STROKE()), StrokeKind::Inside);
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect.shrink2(vec2(32.0, 24.0))).layout(Layout::top_down(Align::Center)));
    child.spacing_mut().item_spacing.y = 10.0;
    let content = child.available_rect_before_wrap();
    // Centre the content: a tile, two lines and a row of buttons, about 200 points.
    child.add_space(((content.height() - 180.0) / 2.0).max(0.0));
    let (tile, _) = child.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
    child.painter().rect_filled(tile, 12.0, RAISED());
    icon(child.painter(), tile.shrink(12.0), image, BLUE());
    child.label(RichText::new(title).font(semibold(15.0)));
    child.add(Label::new(RichText::new(text).size(12.5).color(MUTED())).wrap());
    // One button, centred like the lines above it.
    buttons(&mut child, widgets);
    let r = child.add(Label::new(RichText::new(crate::i18n::tr("How CI checks work")).size(12.0).color(BLUE())).sense(Sense::click())).on_hover_cursor(CursorIcon::PointingHand);
    control(widgets, "actions-guide", &r, child.clip_rect());
    if r.clicked() {
        crate::guide::request(child.ctx());
    }
    ui.allocate_rect(rect, Sense::hover());
}

/// The first read: rows that stand in for runs, and what is happening.
fn skeleton(ui: &mut Ui) {
    let area = ui.available_rect_before_wrap();
    let rect = Rect::from_min_size(pos2(area.left() + 24.0, area.top() + 4.0), vec2((area.width() - 48.0).clamp(200.0, 640.0), 300.0_f32.min(area.height() - 8.0).max(120.0)));
    ui.painter().rect(rect, 8.0, VIEW_BG(), Stroke::new(1.0, CARD_STROKE()), StrokeKind::Inside);
    let rows = ((rect.height() - 40.0) / 52.0).floor().max(1.0) as usize;
    for i in 0..rows.min(5) {
        let row = Rect::from_min_size(pos2(rect.left(), rect.top() + i as f32 * 52.0), vec2(rect.width(), 52.0));
        if i > 0 {
            ui.painter().hline(row.x_range(), row.top(), Stroke::new(1.0, CARD_LINE()));
        }
        ui.painter().circle_filled(pos2(row.left() + 22.0, row.center().y), 7.0, RAISED());
        ui.painter().rect_filled(Rect::from_min_size(pos2(row.left() + 40.0, row.center().y - 11.0), vec2(row.width() * 0.32, 10.0)), 5.0, RAISED());
        ui.painter().rect_filled(Rect::from_min_size(pos2(row.left() + 40.0, row.center().y + 4.0), vec2(row.width() * 0.56, 8.0)), 4.0, HAIRLINE());
    }
    let line = pos2(rect.left() + 14.0, rect.bottom() - 20.0);
    let mut child = ui.new_child(UiBuilder::new().max_rect(Rect::from_min_size(line - vec2(0.0, 10.0), vec2(rect.width() - 28.0, 22.0))).layout(Layout::left_to_right(Align::Center)));
    child.spinner();
    child.label(RichText::new(crate::i18n::tr("Reading runs")).size(12.0).color(HINT()));
    ui.ctx().request_repaint_after(Duration::from_millis(120));
    ui.allocate_rect(rect, Sense::hover());
}

/// The branch and state filters, and how many runs the list holds.
fn filters(ui: &mut Ui, tab: &mut Tab, current: Option<&str>, compact: bool, widgets: &mut HashMap<String, UiControl>) {
    let a = &mut tab.actions;
    let label = a.branch.clone().or(current.map(str::to_owned));
    let counts = [
        a.runs.len(),
        a.runs.iter().filter(|r| Show::Failed.includes(r.state)).count(),
        a.runs.iter().filter(|r| Show::Running.includes(r.state)).count(),
    ];
    Frame::new().inner_margin(Margin { left: if compact { 16 } else { 24 }, right: if compact { 16 } else { 24 }, top: 0, bottom: 10 }).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            if !compact && let Some(label) = &label {
                let r = pages::segments(ui, &[(crate::i18n::tr("All branches"), !a.on_branch), (label.as_str(), a.on_branch)]);
                control(widgets, "actions-branch-all", &r[0], ui.clip_rect());
                control(widgets, "actions-branch-current", &r[1], ui.clip_rect());
                if r[0].clicked() && a.on_branch {
                    a.on_branch = false;
                    // A link's branch goes back to the current one once all branches show.
                    a.branch = None;
                }
                if r[1].clicked() {
                    a.on_branch = true;
                }
            }
            let r = pages::count_segments(ui, &[(crate::i18n::tr("All"), counts[0], a.show == Show::All), (crate::i18n::tr("Failed"), counts[1], a.show == Show::Failed), (crate::i18n::tr("Running"), counts[2], a.show == Show::Running)]);
            for (response, (show, key)) in r.iter().zip([(Show::All, "actions-show-all"), (Show::Failed, "actions-show-failed"), (Show::Running, "actions-show-running")]) {
                control(widgets, key, response, ui.clip_rect());
                if response.clicked() {
                    a.show = show;
                }
            }
            if !compact && !a.runs.is_empty() {
                let words = crate::i18n::trf!("Newest runs: {count}", count = a.runs.len());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(words).size(12.0).color(HINT()));
                });
            }
        });
    });
}

/// What the last read says above the list: a used-up rate limit, an unreachable host.
fn strips(ui: &mut Ui, tab: &mut Tab, host: ForgeProvider, session: &ForgeSession, intents: &mut Vec<Intent>, widgets: &mut HashMap<String, UiControl>) {
    // Runs and commit checks can need different permissions (Gitea, Bitbucket, Azure).
    if let Some(ci_checks::Whole::Hidden(missing)) = &tab.ci.whole
        && tab.actions.hidden.is_none()
    {
        let text = match missing {
            Some(p) => crate::i18n::trf!("Checks on commits are hidden: this token needs {permission}.", permission = p),
            None => crate::i18n::tr("Checks on commits are hidden: this token can't read them.").to_string(),
        };
        Frame::new().inner_margin(Margin { left: 24, right: 24, top: 0, bottom: 10 }).show(ui, |ui| {
            if let Some(r) = pages::notice_strip(ui, MUTED(), &text, Some(crate::i18n::tr("Accounts"))) {
                control(widgets, "actions-marks-accounts", &r, ui.clip_rect());
                if r.clicked() {
                    intents.push(Intent::Preferences(crate::preferences::Page::Accounts));
                }
            }
        });
    }
    let a = &mut tab.actions;
    if a.runs.is_empty() {
        return;
    }
    let since = a.read_at.clone().map(|t| crate::i18n::trf!(" This list is from {time}.", time = t)).unwrap_or_default();
    let unreachable = matches!(a.failure, None | Some(ForgeFailure::Network | ForgeFailure::ServerUnavailable | ForgeFailure::TlsVerification));
    let text = match (&a.paused_until, &a.error) {
        (Some(at), _) => Some((AMBER(), crate::i18n::trf!("{host}'s rate limit for {account} is used up. Runs refresh at {at}.{since}", host = host_name(host), account = session.login(), at, since))),
        (None, Some(_)) if unreachable => Some((MUTED(), crate::i18n::trf!("Couldn't reach {server}.{since}", server = ci_checks::server_name(session.config().server()), since))),
        (None, Some(error)) => Some((MUTED(), format!("{error}{since}"))),
        _ => None,
    };
    let Some((color, text)) = text else { return };
    let mut refresh = false;
    Frame::new().inner_margin(Margin { left: 24, right: 24, top: 0, bottom: 10 }).show(ui, |ui| {
        if let Some(r) = pages::notice_strip(ui, color, &text, a.paused_until.is_none().then_some(crate::i18n::tr("Refresh"))) {
            control(widgets, "actions-strip-refresh", &r, ui.clip_rect());
            refresh = r.clicked();
        }
    });
    if refresh {
        a.refresh();
    }
}

/// The runs, newest first, the selected one marked.
fn list_card(ui: &mut Ui, rect: Rect, a: &mut State, compact: bool, subjects: &HashMap<String, String>, widgets: &mut HashMap<String, UiControl>) {
    ui.painter().rect(rect, 8.0, VIEW_BG(), Stroke::new(1.0, CARD_STROKE()), StrokeKind::Inside);
    let faded = a.error.is_some() || a.paused_until.is_some();
    let shown: Vec<usize> = (0..a.runs.len()).filter(|&i| a.show.includes(a.runs[i].state)).collect();
    let footer = 40.0;
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect.shrink(1.0)).layout(Layout::top_down(Align::Min)));
    child.set_clip_rect(rect.shrink(1.0).intersect(ui.clip_rect()));
    let mut clicked = None;
    let now = now_seconds();
    let list_height = (rect.height() - footer - 2.0).max(40.0);
    ScrollArea::vertical().id_salt("actions-runs").max_height(list_height).auto_shrink([false, false]).show(&mut child, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        if shown.is_empty() {
            ui.add_space(18.0);
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                let words = match a.show {
                    Show::Failed => crate::i18n::trf!("No failures in the newest runs (checked: {count}).", count = a.runs.len()),
                    Show::Running => crate::i18n::tr("Nothing is running.").into(),
                    Show::All if a.on_branch => crate::i18n::tr("No run on this branch among the newest the host listed.").into(),
                    Show::All => crate::i18n::tr("No runs.").into(),
                };
                ui.label(RichText::new(words).size(12.5).color(HINT()));
            });
        }
        for (n, &i) in shown.iter().enumerate() {
            let run = &a.runs[i];
            let subject = run.subject.clone().or_else(|| run.commit.as_ref().and_then(|c| subjects.get(c).cloned()));
            let response = run_row(ui, run, a.selected == Some(run.id), compact, subject.as_deref(), now, n > 0, faded);
            control(widgets, format!("actions-run-{}", run.id), &response, ui.clip_rect());
            if response.clicked() {
                clicked = Some(run.id);
            }
        }
    });
    if let Some(id) = clicked {
        a.select(id);
        a.focused = compact;
    }
    // Footer: how many show, and Load more while the host has older runs.
    let foot = Rect::from_min_max(pos2(rect.left(), rect.bottom() - footer), rect.max);
    ui.painter().hline(foot.x_range(), foot.top(), Stroke::new(1.0, CARD_LINE()));
    let words = match (shown.len(), a.runs.len()) {
        (s, n) if s == n => crate::i18n::trf!("Runs: {count}", count = n),
        (s, n) => crate::i18n::trf!("{shown} of {total} shown", shown = s, total = n),
    };
    ui.painter().text(pos2(foot.left() + 12.0, foot.center().y), Align2::LEFT_CENTER, words, FontId::proportional(12.0), HINT());
    if a.more || a.reading() && a.pages >= 1 {
        let mut child = ui.new_child(UiBuilder::new().max_rect(foot.shrink2(vec2(8.0, 6.0))).layout(Layout::right_to_left(Align::Center)));
        let loading = a.task.as_ref().is_some_and(|(_, page, _)| *page > 1);
        if loading {
            child.spinner();
        } else if a.more {
            let r = child.add_enabled(!a.reading(), Button::new(RichText::new(crate::i18n::tr("Load more")).size(12.5).color(BLUE())).frame(false));
            control(widgets, "actions-load-more", &r, child.clip_rect());
            if r.clicked() {
                a.want = Some(a.pages + 1);
            }
        }
    }
    ui.allocate_rect(rect, Sense::hover());
}

/// "Running for 1m 12s" in amber, "4m 41s" in red for a failure, "Cancelled".
fn took(run: &ForgeCiRun, now: u64) -> (String, Color32) {
    let time = run.took.map(ci_checks::duration_text);
    match run.state {
        ForgeCheckState::Running => (run.started().map_or_else(|| crate::i18n::tr("Running").into(), |s| crate::i18n::trf!("Running for {duration}", duration = ci_checks::duration_text(now.saturating_sub(s)))), AMBER()),
        ForgeCheckState::Queued => (crate::i18n::tr("Queued").into(), AMBER()),
        ForgeCheckState::Failed => (time.unwrap_or_else(|| crate::i18n::tr("Failed").into()), RED()),
        ForgeCheckState::Cancelled => (crate::i18n::tr("Cancelled").into(), MUTED()),
        ForgeCheckState::Skipped => (crate::i18n::tr("Skipped").into(), MUTED()),
        ForgeCheckState::Passed => (time.unwrap_or_else(|| crate::i18n::tr("Passed").into()), MUTED()),
        ForgeCheckState::Neutral => (time.unwrap_or_else(|| crate::i18n::tr("Completed").into()), MUTED()),
    }
}
/// "CI #812"
fn run_name(run: &ForgeCiRun) -> String {
    match run.number_text() {
        Some(n) => format!("{} #{n}", run.name),
        None => run.name.clone(),
    }
}
/// A branch name in a small rounded chip. Returns its width.
fn branch_chip(painter: &Painter, left: Pos2, name: &str, max: f32) -> f32 {
    let mut job = text::LayoutJob::simple_singleline(name.to_owned(), FontId::proportional(11.5), BLUE());
    job.wrap = text::TextWrapping::truncate_at_width((max - 14.0).max(20.0));
    let galley = painter.layout_job(job);
    let rect = Rect::from_min_size(pos2(left.x, left.y - 10.0), vec2(galley.size().x + 14.0, 20.0));
    painter.rect(rect, 10.0, ACCENT_SOFT(), Stroke::NONE, StrokeKind::Inside);
    painter.galley(pos2(rect.left() + 7.0, rect.center().y - galley.size().y / 2.0), galley, BLUE());
    rect.width()
}
#[allow(clippy::too_many_arguments)]
fn run_row(ui: &mut Ui, run: &ForgeCiRun, selected: bool, compact: bool, subject: Option<&str>, now: u64, line: bool, faded: bool) -> Response {
    let height = if compact { 46.0 } else { 52.0 };
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    if line {
        painter.hline(rect.x_range(), rect.top(), Stroke::new(1.0, CARD_LINE()));
    }
    if selected {
        painter.rect_filled(rect, 0.0, SELECTED());
        painter.rect_filled(Rect::from_min_size(rect.min, vec2(3.0, rect.height())), 0.0, BLUE());
    } else if response.hovered() {
        painter.rect_filled(rect, 0.0, HOVER());
    }
    let fade = |c: Color32| if faded { c.gamma_multiply(0.55) } else { c };
    paint_check(&painter, pos2(rect.left() + 22.0, rect.center().y), run.state);
    // The right column: how long it took, then when it ran.
    let (time, color) = took(run, now);
    let time = painter.layout_no_wrap(time, FontId::proportional(12.0), fade(color));
    let when = run.created().map(|t| ago(now.saturating_sub(t), t as i64)).unwrap_or_default();
    let when = painter.layout_no_wrap(when, FontId::proportional(12.0), fade(HINT()));
    let right = rect.right() - 12.0;
    let column = time.size().x.max(when.size().x);
    painter.galley(pos2(right - time.size().x, rect.center().y - 2.0 - time.size().y), time, fade(color));
    painter.galley(pos2(right - when.size().x, rect.center().y + 2.0), when, fade(HINT()));
    let text_right = right - column - 12.0;
    // The first line: name, what started it, and its branch.
    let left = rect.left() + 40.0;
    let mut x = left;
    let top = rect.center().y - 9.0;
    let name = painter.layout_no_wrap(run_name(run), semibold(13.0), fade(TEXT()));
    let name_width = name.size().x.min(text_right - x);
    painter.with_clip_rect(Rect::from_min_max(pos2(x, rect.top()), pos2(text_right, rect.bottom()))).galley(pos2(x, top - name.size().y / 2.0), name, fade(TEXT()));
    x += name_width + 8.0;
    if let Some(event) = &run.event
        && x < text_right - 30.0
    {
        let galley = painter.layout_no_wrap(event.clone(), FontId::proportional(12.0), fade(HINT()));
        let w = galley.size().x;
        painter.with_clip_rect(Rect::from_min_max(pos2(x, rect.top()), pos2(text_right, rect.bottom()))).galley(pos2(x, top - galley.size().y / 2.0), galley, fade(HINT()));
        x += w + 8.0;
    }
    if !compact
        && let Some(branch) = &run.branch
        && x < text_right - 40.0
    {
        branch_chip(&painter.with_clip_rect(Rect::from_min_max(pos2(x, rect.top()), pos2(text_right, rect.bottom()))), pos2(x, top), branch, text_right - x);
    }
    // The second line: the commit and its subject.
    let mut job = text::LayoutJob::default();
    if let Some(commit) = &run.commit {
        job.append(&commit[..7.min(commit.len())], 0.0, TextFormat::simple(FontId::monospace(11.5), fade(HINT())));
        job.append("  ", 0.0, TextFormat::simple(FontId::proportional(12.0), fade(MUTED())));
    }
    if let Some(subject) = subject {
        job.append(subject, 0.0, TextFormat::simple(FontId::proportional(12.0), fade(MUTED())));
    }
    job.wrap = text::TextWrapping::truncate_at_width((text_right - left).max(20.0));
    let galley = painter.layout_job(job);
    painter.galley(pos2(left, rect.center().y + 9.0 - galley.size().y / 2.0), galley, fade(MUTED()));
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, run_name(run)));
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// A full sentence lets each language place the run name and state naturally.
fn state_words(state: ForgeCheckState, run: &str) -> String {
    match state {
        ForgeCheckState::Failed => crate::i18n::trf!("{run} failed", run),
        ForgeCheckState::Running => crate::i18n::trf!("{run} is running", run),
        ForgeCheckState::Queued => crate::i18n::trf!("{run} is queued", run),
        ForgeCheckState::Passed => crate::i18n::trf!("{run} passed", run),
        ForgeCheckState::Cancelled => crate::i18n::trf!("{run} was cancelled", run),
        ForgeCheckState::Skipped => crate::i18n::trf!("{run} was skipped", run),
        ForgeCheckState::Neutral => crate::i18n::trf!("{run} finished", run),
    }
}

/// The selected run: what it is, its commit, Re-run and Open, and its jobs.
#[allow(clippy::too_many_arguments)]
fn details_card(ui: &mut Ui, rect: Rect, a: &mut State, host: ForgeProvider, subjects: &HashMap<String, String>, loaded: &[String], intents: &mut Vec<Intent>, widgets: &mut HashMap<String, UiControl>) {
    ui.painter().rect(rect, 8.0, VIEW_BG(), Stroke::new(1.0, CARD_STROKE()), StrokeKind::Inside);
    let Some(row) = a.selected_run().cloned() else {
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, crate::i18n::tr("Choose a run to see its jobs."), FontId::proportional(12.5), HINT());
        return;
    };
    let details = a.details.clone().filter(|d| d.run.id == row.id);
    let run = details.as_ref().map_or(row.clone(), |d| d.run.clone());
    let reading = a.reading_details();
    let live = a.live;
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect.shrink2(vec2(14.0, 12.0))).layout(Layout::top_down(Align::Min)));
    child.set_clip_rect(rect.shrink(1.0).intersect(ui.clip_rect()));
    let ui = &mut child;
    ui.spacing_mut().item_spacing.y = 8.0;
    // Title: the run and its result, then attempt, event and branch.
    ui.horizontal(|ui| {
        let (mark, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
        paint_check(ui.painter(), mark.center(), run.state);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.add(Label::new(RichText::new(state_words(run.state, &run_name(&run))).font(semibold(14.0))).truncate());
            let mut parts: Vec<String> = Vec::new();
            if let Some(n) = run.attempt {
                parts.push(crate::i18n::trf!("attempt {attempt}", attempt = n));
            }
            if let Some(e) = &run.event {
                parts.push(e.clone());
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(RichText::new(parts.join(" · ")).size(12.0).color(HINT()));
                if let Some(branch) = &run.branch {
                    let (r, _) = ui.allocate_exact_size(vec2(ui.painter().layout_no_wrap(branch.clone(), FontId::proportional(11.5), BLUE()).size().x.min(220.0) + 14.0, 20.0), Sense::hover());
                    branch_chip(ui.painter(), pos2(r.left(), r.center().y), branch, r.width());
                }
            });
        });
    });
    // The commit, with Show in Commits when history has it.
    if let Some(commit) = run.commit.clone() {
        let subject = run.subject.clone().or_else(|| subjects.get(&commit).cloned());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(RichText::new(&commit[..7.min(commit.len())]).monospace().size(12.0).background_color(RAISED()));
            let has = loaded.contains(&commit);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let r = ui.add_enabled(has, Button::new(RichText::new(crate::i18n::tr("Show in Commits")).size(12.5).color(if has { BLUE() } else { FAINT() })).frame(false));
                let r = r.on_disabled_hover_text(crate::i18n::tr("This commit isn't in the loaded history. Fetch, then try again."));
                control(widgets, "actions-show-commit", &r, ui.clip_rect());
                if r.clicked() {
                    intents.push(Intent::Actions(ActionsIntent::ShowCommit(commit.clone())));
                }
                if let Some(subject) = &subject {
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.add(Label::new(RichText::new(subject).size(12.5).color(TEXT_SOFT())).truncate());
                    });
                }
            });
        });
    }
    // Re-run failed jobs and Open in browser.
    let failed = details.as_ref().is_some_and(|d| !d.run.failed.is_empty());
    let block = details.as_ref().and_then(|d| d.rerun);
    let offered = failed && block.is_none() && live && !reading;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if failed {
            let why = block.map(ci_checks::rerun_refusal).or((!live).then_some(ci_checks::not_live()));
            let r = pages::primary(ui, offered, rerun_label(host));
            let r = match why {
                Some(why) => r.on_disabled_hover_text(why),
                None => r,
            };
            control(widgets, "actions-rerun", &r, ui.clip_rect());
            if r.clicked() {
                intents.push(Intent::Actions(ActionsIntent::Rerun));
            }
        }
        let url = run.link.as_ref().filter(|l| l.external.is_none()).map(|l| l.url.clone());
        if let Some(url) = url
            && external_button(ui, crate::i18n::tr("Open in browser"), widgets, "actions-open-run")
        {
            intents.push(Intent::Actions(ActionsIntent::Open(url)));
        }
    });
    // Why Re-run waits, or what couldn't be read.
    if failed && let Some(why) = block.filter(|b| *b != ForgeRerunBlock::Unsupported || run.state == ForgeCheckState::Failed) {
        ui.add(Label::new(RichText::new(ci_checks::rerun_refusal(why)).size(12.0).color(HINT())).wrap());
    } else if failed
        && !live
        && pages::notice_strip(ui, MUTED(), ci_checks::not_live(), Some(crate::i18n::tr("Connect in Hosting"))).is_some_and(|r| {
            control(widgets, "actions-connect-hosting", &r, ui.clip_rect());
            r.clicked()
        })
    {
        intents.push(Intent::Dialog(DialogKind::Hosting));
    }
    if let Some(error) = &a.details_error {
        pages::notice_strip(ui, AMBER(), error, None);
    } else if let Some(d) = &details
        && let Some(e) = d.error
    {
        let text = match (e, &d.missing) {
            (ForgeFailure::Forbidden, Some(p)) => crate::i18n::trf!("Some jobs are hidden: this token needs {permission}.", permission = p),
            (ForgeFailure::Forbidden, None) => crate::i18n::tr("Some jobs are hidden: this token can't read them.").to_string(),
            (other, _) => crate::i18n::trf!("Part of the run couldn't be read ({code}).", code = other.code()),
        };
        pages::notice_strip(ui, MUTED(), &text, None);
    }
    // Its jobs.
    let line = ui.cursor().top();
    ui.painter().hline(rect.shrink(1.0).x_range(), line, Stroke::new(1.0, CARD_LINE()));
    match &details {
        Some(d) => {
            let note = if offered {
                Some(if host == ForgeProvider::GitLab { crate::i18n::tr("GitLab also retries the pipeline's canceled jobs.") } else { crate::i18n::tr("The host also runs the jobs that need the failed ones again.") })
            } else {
                None
            };
            let spare = if note.is_some() || d.limited { 30.0 } else { 6.0 };
            ScrollArea::vertical().id_salt(("actions-jobs", d.run.id)).max_height((ui.available_height() - spare).max(40.0)).auto_shrink([false, true]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                if d.jobs.is_empty() {
                    ui.add_space(6.0);
                    ui.label(RichText::new(crate::i18n::tr("The host reported no jobs for this run.")).size(12.5).color(HINT()));
                }
                if let Some(url) = ci_checks::list(ui, &d.jobs, host, widgets, "actions-jobs", ListStyle::Jobs) {
                    intents.push(Intent::Actions(ActionsIntent::Open(url)));
                }
            });
            if d.limited {
                ui.label(RichText::new(crate::i18n::trf!("Showing {count}: open the run in the browser for the rest.", count = d.jobs.len())).size(12.0).color(HINT()));
            } else if let Some(note) = note {
                ui.label(RichText::new(note).size(12.0).color(HINT()));
            }
        }
        None if reading => pages::busy_line(ui, crate::i18n::tr("Reading jobs")),
        None => {}
    }
}
