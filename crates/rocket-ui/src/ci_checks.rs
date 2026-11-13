//! CI marks beside pushed commits and branches, the checks popover and the
//! reviewed re-run of failed jobs. Reads run on their own worker after history
//! is drawn; history never waits for them. A remembered hosting account whose
//! server matches a remote signs in quietly for reads; nothing is written
//! without the review.
use crate::{
    app::*,
    hosting_accounts::{self, Account},
    pages::{self, control},
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use std::{
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
    sync::Arc,
    time::{Duration, Instant},
};

/// While a mark shows Running, and Rocket is in front.
const RUNNING_EVERY: Duration = Duration::from_secs(30);
const IDLE_EVERY: Duration = Duration::from_secs(300);
const AFTER_ERROR: Duration = Duration::from_secs(120);
const AFTER_RATE_LIMIT: Duration = Duration::from_secs(600);
/// After a re-run starts, look again soon so the marks turn running.
const AFTER_RERUN: Duration = Duration::from_secs(8);

/// What the status bar says about this repository's checks as a whole.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Whole {
    /// The account can't read checks; the permission, when the host named it.
    Hidden(Option<String>),
    /// The host's rate limit is used up until this local time ("14:20").
    Paused(String),
    /// The last refresh failed; marks are faded and date from this time.
    Stale(String),
    /// The account's token was rejected.
    Rejected,
}

/// One repository tab's marks.
#[derive(Default)]
pub struct State {
    pub repository: Option<ForgeRepository>,
    session: Option<Arc<ForgeSession>>,
    pub marks: HashMap<String, ForgeCiState>,
    /// Branch tips the sidebar shows, by full ref name, and the commit each mark is for.
    pub tips: HashMap<Vec<u8>, String>,
    pub faded: bool,
    /// No loaded commit has a check, so no ring says Not checked anywhere.
    pub quiet: bool,
    pub whole: Option<Whole>,
    pub limited: bool,
    /// Successful refreshes, for MCP state and runners.
    pub refreshes: u64,
    task: Option<ReadTask<ForgeCiMarks>>,
    due: Option<Instant>,
    key: u64,
    asked: Vec<String>,
    /// When the loaded history or branches last changed; asked is rebuilt only then.
    history: u64,
}
impl State {
    pub fn reading(&self) -> bool {
        self.task.is_some()
    }
    pub fn mark(&self, commit: &str) -> Option<ForgeCiState> {
        self.marks.get(commit).copied().filter(|s| !(self.quiet && *s == ForgeCiState::NotChecked))
    }
    /// After marks change: a repository where nothing has a check shows no rings.
    fn settle(&mut self) {
        self.quiet = self.marks.values().all(|s| *s == ForgeCiState::NotChecked);
    }
    /// The account the marks and Actions read with.
    pub fn session(&self) -> Option<&Arc<ForgeSession>> {
        self.session.as_ref()
    }
    /// The mark for a branch row: its own commit on the host, or its upstream's.
    pub fn branch_mark(&self, reference: &[u8]) -> Option<(ForgeCiState, &str)> {
        let commit = self.tips.get(reference)?;
        Some((self.mark(commit)?, commit.as_str()))
    }
    /// Whether rows reserve room for a mark: a hosting repository is matched.
    pub fn active(&self) -> bool {
        self.repository.is_some()
    }
    pub fn host(&self) -> ForgeProvider {
        self.session.as_ref().map_or(ForgeProvider::GitHub, |s| s.config().provider())
    }
    /// A branch row's tooltip: which commit on the host the mark is for.
    pub fn branch_tip(&self, state: ForgeCiState, commit: &str, local_target: Option<&str>) -> String {
        let elsewhere = local_target.is_some_and(|t| t != commit);
        crate::i18n::trf!(
            "{state} on {host} for {commit}.{note} Click for the checks.",
            state = ci_label(state), host = host_name(self.host()), commit = &commit[..7],
            note = if elsewhere { crate::i18n::tr(" The branch is at another commit.") } else { "" }
        )
    }
    fn reset(&mut self) {
        if let Some(t) = &self.task {
            t.cancel();
        }
        *self = Self::default();
    }
}

/// The checks of one commit, opened from a mark or the details pane.
pub struct Popover {
    pub tab: u64,
    pub commit: String,
    anchor: Rect,
    pub details: Option<Arc<ForgeCommitChecks>>,
    pub error: Option<String>,
    pub notice: Option<String>,
    task: Option<ReadTask<ForgeCommitChecks>>,
    session: Arc<ForgeSession>,
    opened: u64,
}
impl Popover {
    pub fn reading(&self) -> bool {
        self.task.is_some()
    }
}
enum RerunStep {
    Preview(ReadTask<ForgeRerunPreview>),
    Send(WriteTask<ForgeRerunResult>),
}
/// What a re-run review was opened from: a commit's checks, or one run in Actions.
pub enum RerunSource {
    Commit(Arc<ForgeCommitChecks>),
    Run(Arc<ForgeRunDetails>),
}
/// The reviewed re-run.
pub struct Rerun {
    /// The tab it was opened for; None from Hosting, which no tab owns.
    pub tab: Option<u64>,
    pub source: RerunSource,
    session: Arc<ForgeSession>,
    pub preview: Option<Arc<ForgeRerunPreview>>,
    step: Option<RerunStep>,
    pub error: Option<String>,
    /// The request may have reached the host; no retry until checks are refreshed.
    pub unknown: bool,
    pub stopping: bool,
    opened: u64,
}
impl Rerun {
    pub fn repository(&self) -> &ForgeRepository {
        match &self.source {
            RerunSource::Commit(c) => c.repository(),
            RerunSource::Run(d) => d.repository(),
        }
    }
    /// The runs whose failed jobs would start again, before the review checks them.
    pub fn failed_runs(&self) -> Vec<ForgeCiRun> {
        match &self.source {
            RerunSource::Commit(c) => c.failed_runs().cloned().collect(),
            RerunSource::Run(d) => (!d.run.failed.is_empty()).then(|| d.run.clone()).into_iter().collect(),
        }
    }
    /// The commit's short ID, as the review's cards show it.
    fn commit(&self) -> String {
        let full = match &self.source {
            RerunSource::Commit(c) => c.commit().to_owned(),
            RerunSource::Run(d) => d.run.commit.clone().unwrap_or_default(),
        };
        full.chars().take(7).collect()
    }
    pub fn sending(&self) -> bool {
        matches!(self.step, Some(RerunStep::Send(_)))
    }
    pub fn reviewing(&self) -> bool {
        matches!(self.step, Some(RerunStep::Preview(_)))
    }
}

/// App-wide: remembered accounts, their verified sessions, the popover and the review.
#[derive(Default)]
pub struct Shared {
    accounts: Option<Vec<Account>>,
    loading: Option<ReadTask<hosting_accounts::Loaded>>,
    sessions: Vec<(String, Arc<ForgeSession>)>,
    connecting: Option<(String, ReadTask<ForgeSession>)>,
    /// Accounts whose sign-in failed, and when to try again: soon after a network
    /// error, not until the saved accounts change after a rejected token.
    refused: HashMap<String, Option<Instant>>,
    pub popover: Option<Popover>,
    pub rerun: Option<Rerun>,
    /// The saved-account file Hosting last read or wrote, so an edit there is seen.
    seen_accounts: Option<u64>,
}
impl Shared {
    pub fn signing_in(&self) -> bool {
        self.connecting.is_some()
    }
}
fn account_key(a: &Account) -> String {
    format!("{}|{}|{}", a.provider.key(), a.server, a.label)
}
fn hash<T: Hash>(value: &T) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut h);
    h.finish()
}
/// "14:20" on this computer's clock, `seconds` from now.
fn clock(seconds: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    rocket_git::local_time(now + seconds * 1000).map_or_else(String::new, |t| format!("{:02}:{:02}", t.hour, t.minute))
}

/// The mark itself: a check, a cross, a dot, a ring. Shapes differ, not only colours.
pub(crate) fn paint_mark(painter: &Painter, center: Pos2, state: ForgeCiState, faded: bool) {
    let alpha = |c: Color32| if faded { c.gamma_multiply(0.45) } else { c };
    match state {
        ForgeCiState::Passed => {
            let c = alpha(GREEN());
            let p = |x: f32, y: f32| center + vec2(x, y);
            painter.line_segment([p(-4.4, 0.4), p(-1.4, 3.4)], Stroke::new(2.1, c));
            painter.line_segment([p(-1.4, 3.4), p(4.6, -3.2)], Stroke::new(2.1, c));
        }
        ForgeCiState::Failed => {
            let c = alpha(RED());
            painter.line_segment([center + vec2(-3.4, -3.4), center + vec2(3.4, 3.4)], Stroke::new(2.1, c));
            painter.line_segment([center + vec2(3.4, -3.4), center + vec2(-3.4, 3.4)], Stroke::new(2.1, c));
        }
        ForgeCiState::Running => {
            painter.circle_filled(center, 4.0, alpha(AMBER()));
        }
        ForgeCiState::NotChecked => {
            painter.circle_stroke(center, 3.4, Stroke::new(1.4, alpha(FAINT())));
        }
    }
}
/// A check's own state in a list: queued is a ring, skipped a dash.
pub(crate) fn paint_check(painter: &Painter, center: Pos2, state: ForgeCheckState) {
    match state {
        ForgeCheckState::Failed => paint_mark(painter, center, ForgeCiState::Failed, false),
        ForgeCheckState::Passed => paint_mark(painter, center, ForgeCiState::Passed, false),
        ForgeCheckState::Running => paint_mark(painter, center, ForgeCiState::Running, false),
        ForgeCheckState::Queued => {
            painter.circle_stroke(center, 3.6, Stroke::new(1.8, AMBER()));
        }
        ForgeCheckState::Skipped | ForgeCheckState::Neutral => {
            painter.line_segment([center + vec2(-3.4, 0.0), center + vec2(3.4, 0.0)], Stroke::new(1.8, HINT()));
        }
        ForgeCheckState::Cancelled => {
            painter.circle_stroke(center, 4.4, Stroke::new(1.5, HINT()));
            painter.line_segment([center + vec2(-3.0, 3.0), center + vec2(3.0, -3.0)], Stroke::new(1.5, HINT()));
        }
    }
}
/// The tooltip of a mark in history.
pub(crate) fn mark_tip(state: ForgeCiState, host: ForgeProvider) -> String {
    let name = host_name(host);
    match state {
        ForgeCiState::Passed => crate::i18n::trf!("Checks passed on {name}. Click for the checks.", name),
        ForgeCiState::Failed => crate::i18n::trf!("A check failed on {name}. Click for the checks.", name),
        ForgeCiState::Running => crate::i18n::trf!("Checks are running on {name}. Click for the checks.", name),
        ForgeCiState::NotChecked => crate::i18n::trf!("No checks ran for this commit on {name}.", name),
    }
}
pub(crate) fn host_name(p: ForgeProvider) -> &'static str {
    match p {
        ForgeProvider::GitHub => "GitHub",
        ForgeProvider::GitHubEnterprise => "GitHub Enterprise",
        ForgeProvider::GitLab => "GitLab",
        ForgeProvider::Bitbucket => "Bitbucket",
        ForgeProvider::BitbucketServer => "Bitbucket Server",
        ForgeProvider::Azure | ForgeProvider::AzureServer => "Azure DevOps",
        ForgeProvider::Gitea => "Gitea",
    }
}
/// A check's group in the popover: its run, or "" for Other checks.
fn group_key(c: &ForgeCheck) -> &str {
    if c.run.is_some() { &c.group } else { "" }
}
/// Why Re-run waits: the quiet sign-in of a remembered account only reads.
pub(crate) const NOT_LIVE: &str = crate::i18n::mark("Re-running uses the account connected in Hosting. Connect it there first.");
pub(crate) fn not_live() -> &'static str { crate::i18n::tr(NOT_LIVE) }
/// Display labels live here; Git/provider enum keys and diagnostic summaries stay stable.
pub(crate) fn ci_label(state: ForgeCiState) -> &'static str {
    match state {
        ForgeCiState::Passed => crate::i18n::tr("Passed"),
        ForgeCiState::Failed => crate::i18n::tr("Failed"),
        ForgeCiState::Running => crate::i18n::tr("Running"),
        ForgeCiState::NotChecked => crate::i18n::tr("Not checked"),
    }
}
pub(crate) fn check_label(state: ForgeCheckState) -> &'static str {
    match state {
        ForgeCheckState::Passed => crate::i18n::tr("Passed"),
        ForgeCheckState::Failed => crate::i18n::tr("Failed"),
        ForgeCheckState::Running => crate::i18n::tr("Running"),
        ForgeCheckState::Queued => crate::i18n::tr("Queued"),
        ForgeCheckState::Neutral => crate::i18n::tr("Neutral"),
        ForgeCheckState::Cancelled => crate::i18n::tr("Cancelled"),
        ForgeCheckState::Skipped => crate::i18n::tr("Skipped"),
    }
}
pub(crate) fn run_title(run: &ForgeCiRun) -> String {
    match run.number_text() {
        Some(number) => crate::i18n::trf!("{name} · run {number}", name = &run.name, number),
        None => run.name.clone(),
    }
}
pub(crate) fn duration_text(seconds: u64) -> String {
    match seconds {
        s if s < 60 => crate::i18n::trf!("{seconds}s", seconds = s),
        s if s < 3600 => crate::i18n::trf!("{minutes}m {seconds}s", minutes = s / 60, seconds = format!("{:02}", s % 60)),
        s => crate::i18n::trf!("{hours}h {minutes}m", hours = s / 3600, minutes = format!("{:02}", (s % 3600) / 60)),
    }
}
pub(crate) fn rerun_refusal(block: ForgeRerunBlock) -> &'static str {
    match block {
        ForgeRerunBlock::StillRunning => crate::i18n::tr("The run is still running. Re-run its failed jobs when it finishes."),
        ForgeRerunBlock::TooOld => crate::i18n::tr("The host re-runs a run only within 30 days. Open it in the browser."),
        ForgeRerunBlock::NoWriteAccess => crate::i18n::tr("This account can read the repository but not write to it. Re-running needs write access."),
        ForgeRerunBlock::TokenScope => crate::i18n::tr("This token's scopes don't allow re-running. Give it repo (GitHub) or api (GitLab)."),
        ForgeRerunBlock::Unsupported => crate::i18n::tr("This host's API can't re-run failed jobs. Open the run in the browser."),
        ForgeRerunBlock::Unverified => crate::i18n::tr("Rocket couldn't confirm this account can write here. Refresh the checks to try again."),
    }
}
/// The host's own word for a re-run.
pub(crate) fn rerun_label(p: ForgeProvider) -> &'static str {
    if p == ForgeProvider::GitLab { crate::i18n::tr("Retry failed jobs") } else { crate::i18n::tr("Re-run failed jobs") }
}
/// The popover's words for MCP state and runners.
pub fn summary_text(checks: &ForgeCommitChecks) -> String {
    // This is an automation payload; do not translate its stored words.
    let mut parts = Vec::new();
    for (state, word) in [
        (ForgeCheckState::Failed, crate::i18n::mark("failed")),
        (ForgeCheckState::Running, crate::i18n::mark("running")),
        (ForgeCheckState::Queued, crate::i18n::mark("queued")),
        (ForgeCheckState::Passed, crate::i18n::mark("passed")),
        (ForgeCheckState::Neutral, crate::i18n::mark("neutral")),
        (ForgeCheckState::Cancelled, crate::i18n::mark("cancelled")),
        (ForgeCheckState::Skipped, crate::i18n::mark("skipped")),
    ] {
        let n = checks.count(state);
        if n > 0 { parts.push(format!("{n} {word}")); }
    }
    if parts.is_empty() { crate::i18n::mark("No checks ran").into() } else { parts.join(" · ") }
}
/// "1 failed · 4 passed · 1 skipped"
pub(crate) fn summary(checks: &ForgeCommitChecks) -> String {
    let mut parts = Vec::new();
    for (state, word) in [
        (ForgeCheckState::Failed, crate::i18n::mark("{count} failed")),
        (ForgeCheckState::Running, crate::i18n::mark("{count} running")),
        (ForgeCheckState::Queued, crate::i18n::mark("{count} queued")),
        (ForgeCheckState::Passed, crate::i18n::mark("{count} passed")),
        (ForgeCheckState::Neutral, crate::i18n::mark("{count} neutral")),
        (ForgeCheckState::Cancelled, crate::i18n::mark("{count} cancelled")),
        (ForgeCheckState::Skipped, crate::i18n::mark("{count} skipped")),
    ] {
        let n = checks.count(state);
        if n > 0 {
            parts.push(crate::i18n::fill(crate::i18n::tr(word), &[("count", &n)]));
        }
    }
    if parts.is_empty() { crate::i18n::tr("No checks ran").into() } else { parts.join(" · ") }
}
/// Where Open in browser goes: the failed run, a run, or the commit's page.
pub(crate) fn browser_url(checks: &ForgeCommitChecks) -> String {
    checks
        .failed_runs()
        .chain(checks.runs.iter())
        .find_map(|r| r.link.as_ref().filter(|l| l.external.is_none()).map(|l| l.url.clone()))
        .unwrap_or_else(|| checks.url.clone())
}

/// How a list of checks is laid out.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ListStyle {
    /// The popover: grouped by run, runs first, then Other checks.
    Grouped,
    /// The PR Checks tab: one list by state, with where each check came from.
    Wide,
    /// One run's jobs in Actions: no groups.
    Jobs,
}
/// The list of checks, failures first. Returns a link the user chose to open.
pub(crate) fn list(ui: &mut Ui, checks: &[ForgeCheck], host: ForgeProvider, widgets: &mut HashMap<String, UiControl>, prefix: &str, style: ListStyle) -> Option<String> {
    let mut open = None;
    let mut group = None::<&str>;
    let wide = style == ListStyle::Wide;
    let grouped = style == ListStyle::Grouped;
    // The popover groups by run, runs first and everything else under Other checks;
    // failures stay first within each group. The wide list is one list by state.
    // Ids keep each check's index in the host's order.
    let mut order: Vec<usize> = (0..checks.len()).collect();
    if grouped {
        // Groups with a failure come first; within a group, the host's order (failures first).
        let key = group_key;
        let mut worst: HashMap<&str, ForgeCheckState> = HashMap::new();
        for c in checks {
            let w = worst.entry(key(c)).or_insert(c.state);
            *w = (*w).min(c.state);
        }
        order.sort_by_key(|&i| { let c = &checks[i]; (worst[key(c)], c.run.is_none(), key(c)) });
    }
    for i in order {
        let check = &checks[i];
        let name = if check.run.is_some() { check.group.as_str() } else { crate::i18n::tr("Other checks") };
        if grouped && group != Some(name) {
            group = Some(name);
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                ui.label(RichText::new(name.to_uppercase()).font(semibold(10.5)).color(HINT()));
            });
        }
        let height = match style { ListStyle::Wide => 34.0, ListStyle::Jobs => 32.0, ListStyle::Grouped => 30.0 };
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
        response.on_hover_text(check_label(check.state));
        if wide && i > 0 {
            ui.painter().hline(rect.x_range(), rect.top(), Stroke::new(1.0, CARD_LINE()));
        }
        let painter = ui.painter().with_clip_rect(rect);
        paint_check(&painter, pos2(rect.left() + 14.0, rect.center().y), check.state);
        let failed = check.state == ForgeCheckState::Failed;
        let link_label = check.link.as_ref().map(|l| l.external.clone().unwrap_or_else(|| crate::i18n::tr("Open").into()));
        // The link sits at the right; its width comes off the name and detail.
        let link_width = link_label.as_ref().filter(|_| wide || failed || check.link.as_ref().is_some_and(|l| l.external.is_some())).map_or(0.0, |l| {
            painter.layout_no_wrap(l.clone(), FontId::proportional(12.0), BLUE()).size().x + 26.0
        });
        let right = rect.right() - 10.0 - link_width;
        let detail_color = if failed { RED() } else { HINT() };
        let detail = painter.layout_no_wrap(check.detail.clone(), FontId::proportional(12.0), detail_color);
        let detail_x = if wide { (rect.left() + rect.width() * 0.62).min(right - detail.size().x) } else { right - detail.size().x };
        painter.galley(pos2(detail_x, rect.center().y - detail.size().y / 2.0), detail, detail_color);
        let name_end = if wide { rect.left() + 30.0 + (rect.width() * 0.30).max(200.0) } else { detail_x - 10.0 };
        let mut job = text::LayoutJob::simple_singleline(check.name.clone(), if failed { semibold(13.0) } else { FontId::proportional(13.0) }, TEXT());
        job.wrap = text::TextWrapping::truncate_at_width((name_end - rect.left() - 30.0).max(20.0));
        let galley = painter.layout_job(job);
        painter.galley(pos2(rect.left() + 30.0, rect.center().y - galley.size().y / 2.0), galley, TEXT());
        if wide {
            let from = if check.run.is_some() { check.group.clone() } else { format!("{} · {}", check.group, host_name(host)) };
            let mut job = text::LayoutJob::simple_singleline(from, FontId::proportional(12.0), HINT());
            job.wrap = text::TextWrapping::truncate_at_width((detail_x - name_end - 14.0).max(20.0));
            let galley = painter.layout_job(job);
            painter.galley(pos2(name_end, rect.center().y - galley.size().y / 2.0), galley, HINT());
        }
        if link_width > 0.0
            && let (Some(link), Some(label)) = (&check.link, &link_label)
        {
            let text = painter.layout_no_wrap(label.clone(), FontId::proportional(12.0), BLUE());
            let link_rect = Rect::from_min_size(pos2(rect.right() - 6.0 - link_width, rect.center().y - 11.0), vec2(link_width - 4.0, 22.0));
            let r = ui.interact(link_rect, ui.scope_id().with((prefix, "open", i)), Sense::click()).on_hover_text(match &link.external {
                Some(host) => crate::i18n::trf!("Opens {host} in your browser", host),
                None => crate::i18n::tr("Opens this check on the host in your browser").into(),
            });
            control(widgets, format!("{prefix}-open-{i}"), &r, ui.clip_rect());
            let color = if r.hovered() { ACCENT_HOVER() } else { BLUE() };
            painter.galley(pos2(link_rect.left() + 4.0, link_rect.center().y - text.size().y / 2.0), text, color);
            icon(&painter, Rect::from_center_size(pos2(link_rect.right() - 8.0, link_rect.center().y), vec2(11.0, 11.0)), Icon::External, color);
            if r.clicked() {
                open = Some(link.url.clone());
            }
        }
    }
    open
}

impl Rocket {
    /// Matches each tab to a hosting repository, signs remembered accounts in for
    /// reads, refreshes marks when history or the remote changes, and on a timer.
    pub(crate) fn poll_ci(&mut self, ctx: &Context) {
        if !self.forge.enabled || self.forge.account_check_only || self.forge.publish_check.is_some() {
            for tab in &mut self.tabs {
                if tab.ci.active() {
                    tab.ci.reset();
                }
            }
            return;
        }
        self.poll_ci_accounts();
        let focused = ctx.input(|i| i.focused);
        let now = Instant::now();
        let live = self.forge.session.clone();
        let mut wanted: Option<Account> = None;
        let fixture = self.forge.fixture;
        for index in 0..self.tabs.len() {
            let (repository, session, account) = {
                let tab = &self.tabs[index];
                match tab.snapshot.as_ref() {
                    Some(snapshot) if !tab.review_window => match_repository(snapshot, live.as_ref(), self.ci.accounts.as_deref().unwrap_or(&[]), &self.ci.sessions, fixture),
                    _ => (None, None, None),
                }
            };
            if wanted.is_none()
                && let Some(account) = account.filter(|a| self.ci.refused.get(&account_key(a)).is_none_or(|retry| retry.is_some_and(|t| now >= t)))
            {
                wanted = Some(account);
            }
            let tab = &mut self.tabs[index];
            if repository != tab.ci.repository || session.as_ref().map(Arc::as_ptr) != tab.ci.session.as_ref().map(Arc::as_ptr) {
                tab.ci.reset();
                tab.ci.repository = repository;
                tab.ci.session = session;
            }
            ci_tab(tab, ctx, now, focused, self.backend.clone());
        }
        if let Some(account) = wanted
            && self.ci.connecting.is_none()
            && let Some(backend) = self.backend.clone()
        {
            let key = account_key(&account);
            let config = account_config(&account, fixture);
            if let Ok(config) = config {
                let token = account.token.clone();
                let username = account.username.clone();
                if let Ok(task) = crate::read_task::spawn(ctx, move |cancel| backend.forge_connect(config, &username, &token, &cancel)) {
                    self.ci.connecting = Some((key, task));
                }
            } else {
                self.ci.refused.insert(key, None);
            }
        }
        if let Some((key, task)) = &mut self.ci.connecting
            && let Some(result) = task.try_finish()
        {
            let key = key.clone();
            self.ci.connecting = None;
            match result {
                Ok(session) => {
                    self.ci.refused.remove(&key);
                    self.ci.sessions.push((key, Arc::new(session)));
                }
                // Unreachable or busy: try again later. A rejected token waits for an edit.
                Err(rocket_git::Error::Forge(ForgeFailure::Network | ForgeFailure::ServerUnavailable | ForgeFailure::RateLimited | ForgeFailure::TlsVerification))
                | Err(rocket_git::Error::TimedOut) => {
                    self.ci.refused.insert(key, Some(Instant::now() + AFTER_ERROR));
                }
                Err(_) => {
                    self.ci.refused.insert(key, None);
                }
            }
        }
        self.poll_ci_popover(ctx);
        self.poll_ci_rerun(ctx);
        self.poll_pull_ci(ctx);
    }
    /// The PR Checks tab reads its head as a mark's popover does, on hosts that
    /// can re-run; others keep the statuses the PR details read.
    fn poll_pull_ci(&mut self, ctx: &Context) {
        let backend = self.backend.clone();
        let session = self.forge.session.clone();
        let pull = &mut self.forge.pull;
        if let Some(task) = &mut pull.ci_task
            && let Some(result) = task.try_finish()
        {
            pull.ci_task = None;
            match result {
                Ok(checks) => {
                    pull.ci = Some(Arc::new(checks));
                    pull.ci_error = None;
                }
                Err(rocket_git::Error::Cancelled) => {}
                Err(e) => pull.ci_error = Some(failure_text(&e)),
            }
        }
        let (Some(details), Some(session)) = (pull.details.clone(), session) else { return };
        let head = details.summary().head.clone();
        let host = details.repository().config().provider();
        if !matches!(host, ForgeProvider::GitHub | ForgeProvider::GitHubEnterprise | ForgeProvider::GitLab | ForgeProvider::Gitea)
            || pull.ci_task.is_some()
            || pull.ci_for.as_deref() == Some(head.as_str())
        {
            return;
        }
        if pull.ci.as_ref().is_some_and(|c| c.commit() != head) {
            pull.ci = None;
        }
        pull.ci_for = Some(head.clone());
        pull.ci_error = None;
        let repository = details.repository().clone();
        pull.ci_task = start_details(ctx, backend, session, repository, head);
    }
    fn poll_ci_accounts(&mut self) {
        // Hosting loaded or saved accounts: use its list, and see later saves.
        let saved = self.forge.accounts_loaded.then(|| hash(&self.forge.accounts.iter().map(|a| (a.provider.key(), a.server.as_str(), a.label.as_str(), a.username.as_str(), a.token.as_str())).collect::<Vec<_>>()));
        if saved.is_some() && (self.ci.accounts.is_none() || saved != self.ci.seen_accounts) {
            // A new, edited or forgotten account: sign in again from the new list.
            self.ci.seen_accounts = saved;
            self.ci.accounts = Some(self.forge.accounts.clone());
            self.ci.refused.clear();
            self.ci.sessions.clear();
            return;
        }
        if self.ci.accounts.is_some() || self.ci.loading.is_some() {
            if let Some(task) = &mut self.ci.loading
                && let Some(result) = task.try_finish()
            {
                self.ci.loading = None;
                self.ci.accounts = Some(result.map(|l| l.accounts).unwrap_or_default());
            }
            return;
        }
        // Only a repository with a remote on a known host or a remembered server
        // reads the account file, once per run.
        let wants = self.tabs.iter().any(|t| t.snapshot.as_ref().is_some_and(|s| !s.remotes.is_empty()));
        if !wants {
            return;
        }
        let Some(backend) = self.backend.clone() else { return };
        let directory = self.state_directory.clone();
        if let Ok(task) = ReadTask::spawn(move |_| backend.log.request("hosting-accounts", |_| hosting_accounts::load(&directory))) {
            self.ci.loading = Some(task);
        }
    }
    /// Opens the checks of `commit` beside `anchor` (a mark, or the details line).
    pub(crate) fn open_ci_checks(&mut self, tab_id: u64, commit: String, anchor: Rect) {
        let Some(tab) = self.tabs.iter().find(|t| t.id == tab_id) else { return };
        let (Some(repository), Some(session)) = (tab.ci.repository.clone(), tab.ci.session.clone()) else { return };
        if self.ci.popover.as_ref().is_some_and(|p| p.tab == tab_id && p.commit == commit) {
            self.ci.popover = None;
            return;
        }
        let frame = self.context.cumulative_frame_nr();
        let mut popover = Popover { tab: tab_id, commit: commit.clone(), anchor, details: None, error: None, notice: None, task: None, session: session.clone(), opened: frame };
        popover.task = start_details(&self.context, self.backend.clone(), session, repository, commit);
        self.ci.popover = Some(popover);
    }
    fn poll_ci_popover(&mut self, ctx: &Context) {
        let Some(p) = &mut self.ci.popover else { return };
        if !self.tabs.iter().any(|t| t.id == p.tab && t.ci.session.as_ref().is_some_and(|s| Arc::ptr_eq(s, &p.session))) {
            self.ci.popover = None;
            return;
        }
        if let Some(result) = p.task.as_mut().and_then(ReadTask::try_finish) {
            p.task = None;
            match result {
                Ok(details) => {
                    let state = details.state();
                    let commit = details.commit().to_owned();
                    p.details = Some(Arc::new(details));
                    p.error = None;
                    // What the popover read also corrects the mark.
                    if p.details.as_ref().is_some_and(|d| d.error.is_none() && !d.limited)
                        && let Some(tab) = self.tabs.iter_mut().find(|t| t.id == p.tab)
                    {
                        tab.ci.marks.insert(commit, state);
                        tab.ci.settle();
                    }
                }
                Err(rocket_git::Error::Cancelled) => {}
                Err(e) => p.error = Some(failure_text(&e)),
            }
        }
        if p.task.is_some() {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
    }
    pub(crate) fn refresh_ci_popover(&mut self) {
        let Some(p) = &mut self.ci.popover else { return };
        if p.task.is_some() {
            return;
        }
        let Some(repository) = self.tabs.iter().find(|t| t.id == p.tab).and_then(|t| t.ci.repository.clone()) else { return };
        p.task = start_details(&self.context, self.backend.clone(), p.session.clone(), repository, p.commit.clone());
    }
    /// Opens the review for the checks the popover or Checks tab shows, or for
    /// the run Actions shows.
    pub(crate) fn open_ci_rerun(&mut self, tab_id: Option<u64>, source: RerunSource, session: Arc<ForgeSession>) {
        if self.dialog.as_ref().is_some_and(|d| !pages::browsing(d)) || self.ci.rerun.is_some() {
            return;
        }
        let Some(backend) = self.backend.clone() else { return };
        let frame = self.context.cumulative_frame_nr();
        let read = {
            let session = session.clone();
            match &source {
                RerunSource::Commit(checks) => {
                    let checks = checks.clone();
                    crate::read_task::spawn(&self.context, move |cancel| backend.forge_request("hosting-ci-rerun-preview", &session, |s| s.preview_rerun(&checks, &cancel)))
                }
                RerunSource::Run(details) => {
                    let details = details.clone();
                    crate::read_task::spawn(&self.context, move |cancel| backend.forge_request("hosting-ci-rerun-preview", &session, |s| s.preview_run_rerun(&details, &cancel)))
                }
            }
        };
        let mut rerun = Rerun { tab: tab_id, source, session, preview: None, step: None, error: None, unknown: false, stopping: false, opened: frame };
        match read {
            Ok(task) => rerun.step = Some(RerunStep::Preview(task)),
            Err(e) => rerun.error = Some(failure_text(&e)),
        }
        self.ci.rerun = Some(rerun);
    }
    fn poll_ci_rerun(&mut self, ctx: &Context) {
        let Some(r) = &mut self.ci.rerun else { return };
        let orphan = r.tab.is_some_and(|id| !self.tabs.iter().any(|t| t.id == id));
        if orphan && !r.sending() {
            self.ci.rerun = None;
            return;
        }
        match &mut r.step {
            Some(RerunStep::Preview(task)) => {
                if let Some(result) = task.try_finish() {
                    r.step = None;
                    match result {
                        Ok(preview) => r.preview = Some(Arc::new(preview)),
                        Err(rocket_git::Error::Cancelled) => {}
                        Err(e) => r.error = Some(failure_text(&e)),
                    }
                }
            }
            Some(RerunStep::Send(task)) => {
                if let Some(result) = task.try_finish() {
                    r.step = None;
                    r.stopping = false;
                    match result {
                        Ok(done) => {
                            let tab = r.tab.unwrap_or(u64::MAX);
                            let text = done.started.iter().map(|(run, attempt)| match attempt {
                                Some(a) => crate::i18n::trf!("{run}, attempt {attempt}", run, attempt = a),
                                None => run.clone(),
                            }).collect::<Vec<_>>().join("; ");
                            self.ci.rerun = None;
                            if let Some(p) = &mut self.ci.popover
                                && p.tab == tab
                            {
                                p.notice = Some(crate::i18n::trf!("Re-run started: {text}.", text));
                            }
                            self.refresh_ci_popover();
                            if let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) {
                                t.ci.due = Some(Instant::now() + AFTER_RERUN);
                                t.actions.rerun_started();
                            }
                            self.forge.pull.ci_for = None;
                            self.notice = Some(crate::i18n::trf!("Re-run started: {text}. Rocket refreshes the checks shortly.", text));
                            return;
                        }
                        Err(rocket_git::Error::Forge(ForgeFailure::OutcomeUnknown)) | Err(rocket_git::Error::Cancelled) => {
                            r.unknown = true;
                            r.preview = None;
                            r.error = None;
                            if orphan {
                                self.notice = Some(crate::i18n::tr("The re-run may have started; its repository was closed. Check the run on the host before trying again.").into());
                            }
                        }
                        Err(e) => {
                            r.preview = None;
                            r.error = Some(failure_text(&e));
                        }
                    }
                }
            }
            None => {}
        }
        if r.step.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
    fn ci_send(&mut self) {
        let Some(r) = &mut self.ci.rerun else { return };
        let (Some(preview), Some(backend)) = (r.preview.clone(), self.backend.clone()) else { return };
        if r.unknown || r.step.is_some() {
            return;
        }
        let session = r.session.clone();
        r.error = None;
        match WriteTask::spawn(move |cancel| backend.forge_request("hosting-ci-rerun", &session, |s| s.submit_rerun(&preview, &cancel))) {
            Ok(task) => r.step = Some(RerunStep::Send(task)),
            Err(e) => r.error = Some(failure_text(&e)),
        }
    }
    pub(crate) fn open_ci_link(&mut self, ctx: &Context, url: String) {
        if let Some(list) = &mut self.opened_urls {
            list.push(url);
        } else {
            ctx.open_url(OpenUrl::new_tab(url));
        }
    }
    /// The popover and the review, above the window.
    pub(crate) fn ci_ui(&mut self, ctx: &Context) {
        self.ci_popover_ui(ctx);
        self.ci_rerun_ui(ctx);
    }
    fn ci_popover_ui(&mut self, ctx: &Context) {
        let Some(p) = &self.ci.popover else { return };
        let frame = ctx.cumulative_frame_nr();
        let screen = ctx.content_rect();
        let width = 440.0_f32.min(screen.width() - 24.0);
        let host = p.session.config().provider();
        // Below the mark when it fits, else beside it, kept inside the window.
        let estimate = p.details.as_ref().map_or(150.0, |d| 120.0 + d.checks.len().min(12) as f32 * 30.0 + d.checks.iter().map(|c| &c.group).collect::<HashSet<_>>().len() as f32 * 28.0);
        let mut pos = pos2(p.anchor.right() + 8.0 - width, p.anchor.bottom() + 6.0);
        if pos.y + estimate > screen.bottom() - 30.0 {
            pos = pos2(p.anchor.left() - 8.0 - width, (p.anchor.center().y - estimate / 2.0).clamp(screen.top() + 110.0, (screen.bottom() - 30.0 - estimate).max(screen.top() + 110.0)));
        }
        pos.x = pos.x.clamp(screen.left() + 8.0, (screen.right() - width - 8.0).max(screen.left() + 8.0));
        let mut close = false;
        let mut open: Option<String> = None;
        let mut rerun = false;
        let mut refresh = false;
        let mut all_runs = false;
        let details = p.details.clone();
        let error = p.error.clone();
        let notice = p.notice.clone();
        let reading = p.task.is_some();
        let commit = p.commit.clone();
        let tab_id = p.tab;
        let opened = p.opened;
        let live = self.forge.session.as_ref().is_some_and(|s| Arc::ptr_eq(s, &p.session));
        let widgets = &mut self.frame_widgets;
        let area = Area::new(Id::unique("ci-popover")).order(Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
            pages::modal_frame().corner_radius(10).show(ui, |ui| {
                ui.set_width(width);
                ui.spacing_mut().item_spacing.y = 0.0;
                // Header: the commit's whole result, then which commit on which host.
                Frame::new().inner_margin(Margin { left: 14, right: 8, top: 12, bottom: 10 }).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if let Some(d) = &details {
                            let (rect, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                            paint_mark(ui.painter(), rect.center(), d.state(), false);
                        }
                        ui.vertical(|ui| {
                            ui.set_width(width - 90.0);
                            let title = details.as_ref().map_or_else(|| if reading { crate::i18n::tr("Reading checks").into() } else { crate::i18n::tr("Checks").into() }, |d| summary(d));
                            ui.add(Label::new(RichText::new(title).font(semibold(13.0))).truncate());
                            ui.add(Label::new(RichText::new(crate::i18n::trf!("{commit} on {repository} · {host}", commit = &commit[..7], repository = details.as_ref().map_or("", |d| d.repository().path()), host = host_name(host))).size(12.0).color(HINT())).truncate());
                        });
                        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                            let r = crate::chrome::icon_button(ui, Icon::Refresh, !reading, false, 26.0).on_hover_text(crate::i18n::tr("Refresh checks"));
                            control(widgets, "ci-popover-refresh", &r, ui.clip_rect());
                            refresh = r.clicked();
                        });
                    });
                });
                ui.painter().hline(ui.min_rect().x_range(), ui.cursor().top(), Stroke::new(1.0, CARD_LINE()));
                Frame::new().inner_margin(Margin { left: 10, right: 10, top: 6, bottom: 6 }).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    if let Some(notice) = &notice {
                        pages::notice_strip(ui, GREEN(), notice, None);
                    }
                    if let Some(error) = &error {
                        pages::notice_strip(ui, AMBER(), error, None);
                    }
                    if let Some(d) = &details {
                        if let Some(text) = hidden_text(d) {
                            pages::notice_strip(ui, MUTED(), &text, None);
                        }
                        if d.checks.is_empty() {
                            ui.label(RichText::new(crate::i18n::trf!("{host} has no check runs or statuses for this commit.", host = host_name(host))).size(12.0).color(HINT()));
                        }
                        ScrollArea::vertical().id_salt("ci-popover-list").max_height(330.0).show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            if let Some(url) = list(ui, &d.checks, host, widgets, "ci-popover", ListStyle::Grouped) {
                                open = Some(url);
                            }
                        });
                        if d.limited {
                            ui.label(RichText::new(crate::i18n::trf!("Showing {count}: open it in the browser for the rest.", count = d.checks.len())).size(12.0).color(HINT()));
                        }
                    } else if reading {
                        pages::busy_line(ui, crate::i18n::tr("Marks stay as they were until the answer arrives."));
                    }
                });
                if let Some(d) = &details {
                    Frame::new().fill(PANEL()).inner_margin(Margin { left: 12, right: 12, top: 10, bottom: 10 }).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let failed = d.failed_runs().next().is_some();
                            let offered = d.rerun.is_none() && failed && live;
                            if failed || d.rerun.is_some() {
                                let why = d.rerun.map(rerun_refusal).or((!live).then_some(not_live()));
                                let r = pages::primary(ui, offered && !reading, rerun_label(host));
                                let r = match why { Some(why) => r.on_disabled_hover_text(why), None => r };
                                control(widgets, "ci-popover-rerun", &r, ui.clip_rect());
                                rerun = r.clicked();
                            }
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let r = pages::ghost(ui, true, crate::i18n::tr("Open in browser"));
                                control(widgets, "ci-popover-open", &r, ui.clip_rect());
                                if r.clicked() {
                                    open = Some(browser_url(d));
                                }
                                let r = ui.add(Label::new(RichText::new(crate::i18n::tr("All runs")).size(12.5).color(BLUE())).sense(Sense::click())).on_hover_cursor(CursorIcon::PointingHand).on_hover_text(crate::i18n::tr("Opens Actions on this run's branch"));
                                control(widgets, "ci-popover-all-runs", &r, ui.clip_rect());
                                all_runs = r.clicked();
                            });
                        });
                        let why = d.rerun.filter(|_| d.failed_runs().next().is_some() || d.rerun == Some(ForgeRerunBlock::StillRunning)).map(rerun_refusal)
                            .or((!live && d.failed_runs().next().is_some()).then_some(not_live()));
                        if let Some(why) = why {
                            ui.add_space(6.0);
                            ui.label(RichText::new(why).size(12.0).color(HINT()));
                        }
                    });
                }
            });
        });
        control(&mut self.frame_widgets, "ci-popover", &area.response, ctx.content_rect());
        // A click outside closes it, as does Esc; not on the frame that opened it.
        if frame > opened + 1 {
            let outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|pos| !area.response.rect.contains(pos)));
            close |= outside && self.ci.rerun.is_none();
            close |= ctx.input(|i| i.key_pressed(Key::Escape)) && self.ci.rerun.is_none();
        }
        if let Some(url) = open {
            self.open_ci_link(ctx, url);
        }
        if refresh {
            if let Some(p) = &mut self.ci.popover {
                p.notice = None;
            }
            self.refresh_ci_popover();
        }
        if rerun
            && let Some(p) = &self.ci.popover
            && let Some(d) = p.details.clone()
        {
            let session = p.session.clone();
            self.open_ci_rerun(Some(tab_id), RerunSource::Commit(d), session);
        }
        if all_runs
            && let Some(d) = self.ci.popover.as_ref().and_then(|p| p.details.clone())
        {
            // Actions opens on the run's branch with that run selected: the failed one first.
            let run = d.failed_runs().chain(d.runs.iter()).next().cloned();
            let branch = run.as_ref().and_then(|r| r.branch.clone());
            self.ci.popover = None;
            self.open_actions(branch, run);
            return;
        }
        if close {
            self.ci.popover = None;
        }
    }
    fn ci_rerun_ui(&mut self, ctx: &Context) {
        let Some(r) = &self.ci.rerun else { return };
        let host = r.session.config().provider();
        let label = rerun_label(host);
        let subtitle = crate::i18n::trf!("{repository} · {host} · as {account}", repository = r.repository().path(), host = host_name(host), account = r.session.login());
        if ctx.cumulative_frame_nr() <= r.opened + 2 {
            pages::raise(ctx, "rocket-ci-rerun");
        }
        let head = pages::ReviewHeader { image: Icon::Refresh, title: label, subtitle: &subtitle, danger: false };
        let mut send = false;
        let mut stop = false;
        let mut accounts = false;
        let mut refresh = false;
        let preview = r.preview.clone();
        let runs: Vec<ForgeCiRun> = preview.as_ref().map_or_else(|| r.failed_runs(), |p| p.runs().to_vec());
        let error = r.error.clone();
        let unknown = r.unknown;
        let sending = r.sending();
        let reviewing = r.reviewing();
        let stopping = r.stopping;
        let commit = r.commit();
        let response = pages::review_at(ctx, "rocket-ci-rerun", 520.0, head, &mut self.frame_widgets, "ci-rerun-scroll", !sending, |ui, part, widgets| match part {
            pages::Part::Body => {
                ui.spacing_mut().item_spacing.y = 12.0;
                for run in &runs {
                    pages::titled_card(ui, |ui| {
                        ui.label(RichText::new(run_title(run)).strong());
                        if let Some(a) = run.attempt {
                            ui.label(RichText::new(crate::i18n::trf!("attempt {attempt}", attempt = a)).size(12.0).color(HINT()));
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(&commit).monospace().size(12.0));
                        });
                    }, |ui| {
                        for job in &run.failed {
                            ui.horizontal(|ui| {
                                let (rect, _) = ui.allocate_exact_size(vec2(16.0, 18.0), Sense::hover());
                                paint_check(ui.painter(), rect.center(), ForgeCheckState::Failed);
                                ui.label(RichText::new(job).strong());
                            });
                        }
                        ui.label(RichText::new(match host {
                            ForgeProvider::GitLab => crate::i18n::tr("GitLab also retries the pipeline's canceled jobs."),
                            _ => crate::i18n::tr("The host also runs the jobs that need them again."),
                        }).size(12.0).color(HINT()));
                    });
                }
                if reviewing {
                    pages::busy_line(ui, crate::i18n::tr("Checking the run and your access again"));
                }
                if let Some(error) = &error
                    && pages::notice_strip(ui, RED(), error, Some(crate::i18n::tr("Accounts"))).is_some_and(|r| { control(widgets, "ci-rerun-accounts", &r, ui.clip_rect()); r.clicked() })
                {
                    accounts = true;
                }
                if unknown && pages::notice_strip(ui, AMBER(), &crate::i18n::trf!("{host} may have started the re-run. Refresh checks before trying again.", host = host_name(host)), Some(crate::i18n::tr("Refresh checks"))).is_some_and(|r| { control(widgets, "ci-rerun-refresh", &r, ui.clip_rect()); r.clicked() }) {
                    refresh = true;
                }
            }
            pages::Part::Footer => {
                if sending {
                    let r = pages::ghost(ui, !stopping, if stopping { crate::i18n::tr("Stopping") } else { crate::i18n::tr("Stop") });
                    control(widgets, "ci-rerun-stop", &r, ui.clip_rect());
                    stop = r.clicked();
                    ui.spinner();
                    ui.label(RichText::new(crate::i18n::tr("Sending the request")).color(MUTED()));
                } else {
                    let r = pages::primary(ui, preview.is_some() && !unknown && error.is_none(), label);
                    control(widgets, "ci-rerun-submit", &r, ui.clip_rect());
                    send = r.clicked();
                }
            }
        });
        let close = response.should_close() && !sending;
        if send {
            self.ci_send();
        }
        if stop && let Some(r) = &mut self.ci.rerun
            && let Some(RerunStep::Send(task)) = &r.step
        {
            task.cancel();
            r.stopping = true;
        }
        if accounts {
            self.ci.rerun = None;
            self.dispatch(Intent::Preferences(crate::preferences::Page::Accounts));
            return;
        }
        if refresh {
            if let Some(tab) = self.ci.rerun.as_ref().and_then(|r| r.tab)
                && let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab)
            {
                t.actions.refresh();
            }
            self.ci.rerun = None;
            self.refresh_ci_popover();
            self.forge.pull.ci_for = None;
            return;
        }
        if close {
            self.ci.rerun = None;
        }
    }
    /// Whether Actions can list this tab's runs, or what stands in the way.
    /// Mirrors the matching the marks use; reads nothing.
    pub(crate) fn actions_reach(&self, tab: &Tab) -> crate::actions::Reach {
        use crate::actions::Reach;
        if tab.ci.active() {
            return Reach::Ready;
        }
        let Some(snapshot) = &tab.snapshot else { return Reach::NoRemote };
        let urls: Vec<&[u8]> = snapshot.remotes.iter().flat_map(|r| r.urls.iter().map(Vec::as_slice)).collect();
        let matched = |config: &ForgeConfig| urls.iter().find_map(|u| ForgeRepository::from_remote(config.clone(), u));
        let hosting = self.forge.enabled && !self.forge.account_check_only && self.forge.publish_check.is_none();
        if hosting && !urls.is_empty() {
            let accounts = self.ci.accounts.as_deref().unwrap_or(&[]);
            let configs: Vec<(&Account, ForgeConfig)> = accounts.iter().filter_map(|a| Some((a, account_config(a, self.forge.fixture).ok()?))).collect();
            // Remembered accounts in the order the marks try them; Bitbucket Server last.
            for (account, config) in configs.iter().filter(|(a, _)| a.provider != ForgeProvider::BitbucketServer) {
                if matched(config).is_some() {
                    let server = server_name(&account.server);
                    return match self.ci.refused.get(&account_key(account)) {
                        Some(None) => Reach::Rejected { host: account.provider, server },
                        Some(Some(_)) => Reach::Unreachable { host: account.provider, server },
                        None => Reach::SigningIn,
                    };
                }
            }
            let live = self.forge.session.as_ref().filter(|s| s.config().provider() == ForgeProvider::BitbucketServer).and_then(|s| matched(s.config()));
            let remembered = configs.iter().filter(|(a, _)| a.provider == ForgeProvider::BitbucketServer).find_map(|(_, c)| matched(c));
            if let Some(repo) = live.or(remembered) {
                return Reach::BitbucketServer { url: repo.runs_url() };
            }
            if self.ci.accounts.is_none() {
                return Reach::SigningIn;
            }
        }
        match urls.iter().find_map(|u| ForgeRepository::from_known_remote(u)) {
            Some(repo) => Reach::NoAccount { host: repo.config().provider(), server: repo.config().server().to_owned(), path: repo.path().to_owned() },
            None => Reach::NoRemote,
        }
    }
    /// Signs remembered accounts in again now, not after the wait a network error set.
    pub(crate) fn retry_ci_sign_in(&mut self) {
        self.ci.refused.retain(|_, retry| retry.is_none());
    }
    /// The status bar's whole-repository state, when there is one to say.
    pub(crate) fn ci_status(&mut self, ui: &mut Ui) {
        let Some(tab) = self.tab() else { return };
        let Some(whole) = tab.ci.whole.clone() else { return };
        let host = tab.ci.session.as_ref().map_or(ForgeProvider::GitHub, |s| s.config().provider());
        let (image, text, tip, color) = match &whole {
            Whole::Hidden(missing) => (Icon::Lock, crate::i18n::tr("Checks hidden").to_string(), match missing {
                Some(p) => crate::i18n::trf!("This account's token can't read checks here. Give it {permission}.", permission = p),
                None => crate::i18n::tr("This account's token can't read checks here. See Token permissions in the guide.").into(),
            }, MUTED()),
            Whole::Paused(at) => (Icon::Clock, crate::i18n::trf!("Checks paused until {at}", at), crate::i18n::trf!("{host}'s rate limit for this account is used up. Checks refresh again at {at}.", host = host_name(host), at), AMBER()),
            Whole::Stale(at) => (Icon::Warning, crate::i18n::trf!("Checks from {at}", at), crate::i18n::trf!("Couldn't reach {host}. Marks are faded until a refresh works.", host = host_name(host)), MUTED()),
            Whole::Rejected => (Icon::Lock, crate::i18n::tr("Checks hidden").to_string(), crate::i18n::trf!("{host} rejected this account's token. Reconnect it in Accounts.", host = host_name(host)), MUTED()),
        };
        // Called inside the status bar's right-to-left row: the words, then the icon to their left.
        let r = ui.scope(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let r = ui.add(Button::new(RichText::new(&text).size(12.0).color(color)).frame(false)).on_hover_text(tip);
            let (at, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
            icon(ui.painter(), at, image, color);
            r
        }).inner;
        control(&mut self.frame_widgets, "ci-status", &r, ui.clip_rect());
        if r.clicked() {
            if matches!(whole, Whole::Stale(_)) {
                let active = self.active;
                if let Some(tab) = self.tabs.get_mut(active) {
                    tab.ci.due = Some(Instant::now());
                    tab.ci.key = 0;
                }
            } else if matches!(whole, Whole::Rejected) {
                self.dispatch(Intent::Preferences(crate::preferences::Page::Accounts));
            } else {
                // Actions reads again and says what to do: which permission, or when
                // the limit resets.
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.actions.refresh();
                }
                self.open_actions(None, None);
            }
        }
    }
}
/// "github.com" for "https://github.com", "dev.azure.com" for an organization's URL.
pub(crate) fn server_name(server: &str) -> String {
    server.strip_prefix("https://").unwrap_or(server).split('/').next().unwrap_or(server).to_owned()
}
/// The current branch's mark when it calls for attention: failed or running.
/// The sidebar's Actions entry shows it as a dot.
pub fn head_dot(tab: &Tab) -> Option<(ForgeCiState, String)> {
    let snapshot = tab.snapshot.as_ref()?;
    let head = snapshot.branches.iter().find(|b| b.is_head && b.kind == BranchKind::Local)?;
    let (state, _) = tab.ci.branch_mark(&head.reference)?;
    matches!(state, ForgeCiState::Failed | ForgeCiState::Running).then(|| (state, short_ref(&head.reference)))
}
/// Why the account can't see some checks, from the host's own refusal.
fn hidden_text(d: &ForgeCommitChecks) -> Option<String> {
    match d.error? {
        ForgeFailure::Forbidden => Some(match &d.missing {
            Some(p) => crate::i18n::trf!("Some checks are hidden: this token needs {permission}.", permission = p),
            None => crate::i18n::tr("Some checks are hidden: this token can't read them.").into(),
        }),
        ForgeFailure::RateLimited => Some(crate::i18n::tr("The rate limit is used up; some checks weren't read.").into()),
        other => Some(crate::i18n::trf!("Some checks couldn't be read ({code}).", code = other.code())),
    }
}
fn failure_text(e: &rocket_git::Error) -> String {
    match e {
        rocket_git::Error::Forge(f) => format!("{} ({})", f.guidance(), f.code()),
        other => crate::backend::user_error(other),
    }
}
fn start_details(ctx: &Context, backend: Option<crate::backend::Backend>, session: Arc<ForgeSession>, repository: ForgeRepository, commit: String) -> Option<ReadTask<ForgeCommitChecks>> {
    let backend = backend?;
    crate::read_task::spawn(ctx, move |cancel| backend.forge_request("hosting-ci-details", &session, |s| s.commit_checks(&repository, &commit, &cancel))).ok()
}
fn account_config(a: &Account, fixture: Option<u16>) -> rocket_git::Result<ForgeConfig> {
    let mut config = ForgeConfig::new(a.provider, &a.server)?;
    if let Some(version) = &a.azure_api_version {
        config = config.with_azure_api_version(version)?;
    }
    if let Some(port) = fixture {
        config = config.with_fixture(port)?;
    }
    Ok(config)
}
/// The hosting repository for a tab: Hosting's connected session when it matches
/// a remote, else a remembered account for that server (its session once
/// verified). Bitbucket Server has no CI read here.
fn match_repository(snapshot: &crate::backend::Snapshot, live: Option<&Arc<ForgeSession>>, accounts: &[Account], sessions: &[(String, Arc<ForgeSession>)], fixture: Option<u16>) -> (Option<ForgeRepository>, Option<Arc<ForgeSession>>, Option<Account>) {
    let urls = || snapshot.remotes.iter().flat_map(|r| r.urls.iter());
    if let Some(session) = live
        && session.config().provider() != ForgeProvider::BitbucketServer
        && let Some(repo) = urls().find_map(|u| ForgeRepository::from_remote(session.config().clone(), u))
    {
        return (Some(repo), Some(session.clone()), None);
    }
    for account in accounts {
        if account.provider == ForgeProvider::BitbucketServer {
            continue;
        }
        let Ok(config) = account_config(account, fixture) else { continue };
        let Some(repo) = urls().find_map(|u| ForgeRepository::from_remote(config.clone(), u)) else { continue };
        let key = account_key(account);
        return match sessions.iter().find(|(k, _)| *k == key) {
            Some((_, session)) => (Some(repo), Some(session.clone()), None),
            None => (None, None, Some(account.clone())),
        };
    }
    (None, None, None)
}
/// Which commits to ask about: the remote's branch tips first, then the newest
/// loaded commits those tips reach. Unpushed commits can't have checks.
fn asked(tab: &Tab, repository: &ForgeRepository) -> (Vec<String>, HashMap<Vec<u8>, String>) {
    let Some(snapshot) = &tab.snapshot else { return (Vec::new(), HashMap::new()) };
    let remote = snapshot.remotes.iter().find(|r| r.urls.iter().any(|u| ForgeRepository::from_remote(repository.config().clone(), u).is_some_and(|m| m.path().eq_ignore_ascii_case(repository.path()))));
    let Some(remote) = remote else { return (Vec::new(), HashMap::new()) };
    let mut prefix = b"refs/remotes/".to_vec();
    prefix.extend_from_slice(&remote.name);
    prefix.push(b'/');
    let mut tips: HashMap<Vec<u8>, String> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    let remote_branches: Vec<&Branch> = snapshot.branches.iter().filter(|b| b.kind == BranchKind::Remote && b.symbolic_target.is_none() && b.reference.starts_with(&prefix)).collect();
    for b in &remote_branches {
        tips.insert(b.reference.clone(), b.target.as_str().to_owned());
        if !order.iter().any(|o| o == b.target.as_str()) && order.len() < 20 {
            order.push(b.target.as_str().to_owned());
        }
    }
    for b in snapshot.branches.iter().filter(|b| b.kind == BranchKind::Local) {
        if let Some(up) = &b.upstream
            && let Some(commit) = tips.get(up).cloned()
        {
            tips.insert(b.reference.clone(), commit);
        }
    }
    // Loaded history the remote's tips reach, newest first.
    let commits = tab.loaded_commits();
    let index: HashMap<&str, usize> = commits.iter().enumerate().map(|(i, c)| (c.id.as_str(), i)).collect();
    let mut reach = vec![false; commits.len()];
    let mut stack: Vec<usize> = remote_branches.iter().filter_map(|b| index.get(b.target.as_str()).copied()).collect();
    while let Some(i) = stack.pop() {
        if reach[i] {
            continue;
        }
        reach[i] = true;
        for p in &commits[i].parents {
            if let Some(&j) = index.get(p.as_str())
                && !reach[j]
            {
                stack.push(j);
            }
        }
    }
    for (i, c) in commits.iter().enumerate() {
        if order.len() >= CI_MARK_LIMIT {
            break;
        }
        if reach[i] && !order.iter().any(|o| o == c.id.as_str()) {
            order.push(c.id.as_str().to_owned());
        }
    }
    (order, tips)
}
/// One tab's refresh: when what it asks changes, on the timer, never twice at once.
fn ci_tab(tab: &mut Tab, ctx: &Context, now: Instant, focused: bool, backend: Option<crate::backend::Backend>) {
    let (Some(repository), Some(session)) = (tab.ci.repository.clone(), tab.ci.session.clone()) else { return };
    if let Some(task) = &mut tab.ci.task
        && let Some(result) = task.try_finish()
    {
        tab.ci.task = None;
        let running = |m: &HashMap<String, ForgeCiState>| m.values().any(|s| *s == ForgeCiState::Running);
        match result {
            Ok(marks) => {
                tab.ci.whole = marks.hidden.then(|| Whole::Hidden(marks.missing.clone()));
                tab.ci.marks = marks.marks.into_iter().collect();
                tab.ci.settle();
                tab.ci.limited = marks.limited;
                tab.ci.faded = false;
                tab.ci.refreshes += 1;
                tab.ci.due = Some(now + if running(&tab.ci.marks) { RUNNING_EVERY } else { IDLE_EVERY });
            }
            Err(rocket_git::Error::Cancelled) => tab.ci.due = Some(now + AFTER_ERROR),
            Err(rocket_git::Error::Forge(ForgeFailure::RateLimited)) => {
                tab.ci.faded = true;
                tab.ci.whole = Some(Whole::Paused(clock(AFTER_RATE_LIMIT.as_secs())));
                tab.ci.due = Some(now + AFTER_RATE_LIMIT);
            }
            Err(rocket_git::Error::Forge(ForgeFailure::Forbidden | ForgeFailure::NotFound)) => {
                tab.ci.marks.clear();
                tab.ci.whole = Some(Whole::Hidden(None));
                tab.ci.due = Some(now + IDLE_EVERY);
            }
            Err(rocket_git::Error::Forge(ForgeFailure::Unauthorized)) => {
                tab.ci.marks.clear();
                tab.ci.whole = Some(Whole::Rejected);
                tab.ci.due = Some(now + IDLE_EVERY);
            }
            Err(rocket_git::Error::Forge(ForgeFailure::Unsupported)) => {
                tab.ci.marks.clear();
                tab.ci.due = Some(now + Duration::from_secs(24 * 3600));
            }
            Err(_) => {
                tab.ci.faded = !tab.ci.marks.is_empty();
                tab.ci.whole = Some(Whole::Stale(clock(0)));
                tab.ci.due = Some(now + AFTER_ERROR);
            }
        }
    }
    if tab.ci.task.is_some() {
        return;
    }
    // Rebuild what to ask only when the loaded history or the branches changed.
    let history = hash(&(tab.refresh_count, tab.loaded_commits().len(), tab.loaded_commits().first().map(|c| c.id.as_str().to_owned()), tab.snapshot.as_ref().map(|s| s.branches.len())));
    if history != tab.ci.history || tab.ci.asked.is_empty() {
        let (order, tips) = asked(tab, &repository);
        tab.ci.history = history;
        tab.ci.tips = tips;
        tab.ci.asked = order;
    }
    let order = tab.ci.asked.clone();
    let key = hash(&(&repository.path().to_owned(), &order));
    let changed = key != tab.ci.key;
    let due = tab.ci.due.is_none_or(|d| now >= d);
    if order.is_empty() || !(changed || (due && focused)) {
        if focused && !order.is_empty() && let Some(d) = tab.ci.due {
            ctx.request_repaint_after(d.saturating_duration_since(now).max(Duration::from_millis(250)));
        }
        return;
    }
    tab.ci.key = key;
    let Some(backend) = backend else { return };
    if let Ok(task) = crate::read_task::spawn(ctx, move |cancel| backend.forge_request("hosting-ci-marks", &session, |s| s.ci_marks(&repository, &order, &cancel))) {
        tab.ci.task = Some(task);
    }
}

/// The commit details pane's Checks line: the mark, what it means, Show checks.
pub(crate) fn details_line(ui: &mut Ui, tab: &Tab, commit: &str, widgets: &mut HashMap<String, UiControl>, intents: &mut Vec<Intent>) {
    let Some(state) = tab.ci.mark(commit) else { return };
    let words = match state {
        ForgeCiState::Passed => crate::i18n::tr("Checks passed"),
        ForgeCiState::Failed => crate::i18n::tr("A check failed"),
        ForgeCiState::Running => crate::i18n::tr("Checks are running"),
        ForgeCiState::NotChecked => crate::i18n::tr("No checks ran"),
    };
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 6.0, HOVER());
    }
    paint_mark(ui.painter(), pos2(rect.left() + 12.0, rect.center().y), state, tab.ci.faded);
    let color = if state == ForgeCiState::Failed { RED() } else { TEXT() };
    ui.painter().text(pos2(rect.left() + 26.0, rect.center().y), Align2::LEFT_CENTER, crate::i18n::trf!("{state} on {host}", state = words, host = host_name(tab.ci.host())), FontId::proportional(12.5), color);
    ui.painter().text(pos2(rect.right() - 8.0, rect.center().y), Align2::RIGHT_CENTER, crate::i18n::tr("Show checks"), FontId::proportional(12.5), BLUE());
    control(widgets, "ci-details-checks", &response, ui.clip_rect());
    if response.on_hover_cursor(CursorIcon::PointingHand).clicked() {
        intents.push(Intent::CiChecks(commit.to_owned(), rect));
    }
}
