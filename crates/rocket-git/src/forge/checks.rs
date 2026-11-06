//! CI results for pushed commits, read from the hosting API, and a reviewed,
//! single-use re-run of failed jobs. Reads are bounded and never follow a URL
//! a response supplies; links are only checked and handed to the browser.
use super::*;
use std::time::{SystemTime, UNIX_EPOCH};
mod hosts;

pub(crate) const MARKS: &str = "hosting-ci-marks";
pub(crate) const DETAILS: &str = "hosting-ci-details";
pub(crate) const RERUN_PREVIEW: &str = "hosting-ci-rerun-preview";
pub(crate) const RERUN: &str = "hosting-ci-rerun";
pub(crate) const RUNS: &str = "hosting-ci-runs";
pub(crate) const RUN: &str = "hosting-ci-run";
/// Runs one page of the Actions list reads.
pub const CI_RUN_PAGE: usize = 30;
/// Pages the Actions list loads at most: the newest 150 runs.
pub const CI_RUN_PAGES: usize = 5;
/// Commits one marks request may ask about. GitHub answers them in one query.
pub const CI_MARK_LIMIT: usize = 50;
/// Hosts without a batch endpoint answer one commit per request, one at a time.
pub const CI_MARK_REQUEST_LIMIT: usize = 20;
const CHECK_LIMIT: usize = 100;
const REVIEW_LIFETIME: Duration = Duration::from_secs(120);
/// GitHub re-runs a run only within 30 days of its first run.
const RERUN_WINDOW: u64 = 30 * 24 * 3600;

/// One commit's whole result, the mark beside it in history and the sidebar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ForgeCiState {
    Passed,
    Failed,
    Running,
    /// The host knows the commit, but no check ran on it.
    NotChecked,
}
impl ForgeCiState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Passed => "Passed",
            Self::Failed => "Failed",
            Self::Running => "Running",
            Self::NotChecked => "Not checked",
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct ForgeCiMarks {
    /// Only commits the host reported; a commit it doesn't know has no mark.
    pub marks: Vec<(String, ForgeCiState)>,
    /// More commits were asked about than one refresh reads.
    pub limited: bool,
    /// The host refused to show this account the checks.
    pub hidden: bool,
    /// The permission the refusal named, in the page's words ("Actions: Read").
    pub missing: Option<String>,
}
/// One check's state in a list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ForgeCheckState {
    Failed,
    Cancelled,
    Running,
    Queued,
    Passed,
    Neutral,
    Skipped,
}
impl ForgeCheckState {
    pub fn key(self) -> &'static str {
        match self {
            Self::Failed => "failed",
            Self::Running => "running",
            Self::Queued => "queued",
            Self::Passed => "passed",
            Self::Neutral => "neutral",
            Self::Cancelled => "cancelled",
            Self::Skipped => "skipped",
        }
    }
}
/// Where a check's Open goes. A link on another site keeps its host name so the
/// page can say where it leads before it opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForgeCheckLink {
    pub url: String,
    /// The host name when the link leaves the account's server.
    pub external: Option<String>,
}
#[derive(Clone, Debug)]
pub struct ForgeCheck {
    pub name: String,
    /// The group the check belongs to, such as "CI · run 812", or the status's app.
    pub group: String,
    pub state: ForgeCheckState,
    /// "Failed after 4m 41s", "2m 28s", a status description.
    pub detail: String,
    pub link: Option<ForgeCheckLink>,
    /// The CI run it belongs to, when the host has runs (GitHub Actions, GitLab, Gitea).
    pub run: Option<u64>,
}
/// Why a run's failed jobs can't be re-run now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForgeRerunBlock {
    StillRunning,
    TooOld,
    NoWriteAccess,
    /// A classic token without the repo scope (GitHub), or a GitLab token without api.
    TokenScope,
    /// This host's API can't re-run, such as Bitbucket.
    Unsupported,
    /// Write access couldn't be read, so Re-run isn't offered.
    Unverified,
}
impl ForgeRerunBlock {
    pub fn code(self) -> &'static str {
        match self {
            Self::StillRunning => "ci_rerun_running",
            Self::TooOld => "ci_rerun_too_old",
            Self::NoWriteAccess => "ci_rerun_no_write",
            Self::TokenScope => "ci_rerun_token_scope",
            Self::Unsupported => "ci_rerun_unsupported",
            Self::Unverified => "ci_rerun_unverified",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForgeCiRun {
    pub id: u64,
    /// The workflow's name ("CI"), or "Pipeline" or "Build" when the host gives none.
    pub name: String,
    pub number: Option<u64>,
    pub attempt: Option<u64>,
    pub state: ForgeCheckState,
    pub link: Option<ForgeCheckLink>,
    /// Failed job names, as the review lists them.
    pub failed: Vec<String>,
    /// The branch it ran for, when the host names one.
    pub branch: Option<String>,
    /// What started it, in words: "push", "pull request #42", "schedule".
    pub event: Option<String>,
    /// The commit it ran on.
    pub commit: Option<String>,
    /// The commit's subject or the run's title, when the host sends one.
    pub subject: Option<String>,
    /// How long a finished run took, in seconds.
    pub took: Option<u64>,
    /// Azure's build number ("20261104.3"), which isn't a number.
    build: Option<String>,
    /// Bitbucket names a pipeline by a UUID; its build number is the id.
    uuid: Option<String>,
    created: Option<u64>,
    started: Option<u64>,
}
impl ForgeCiRun {
    fn new(id: u64, name: String, state: ForgeCheckState) -> Self {
        Self { id, name, number: None, attempt: None, state, link: None, failed: Vec::new(), branch: None, event: None, commit: None, subject: None, took: None, build: None, uuid: None, created: None, started: None }
    }
    /// "CI · run 812"
    pub fn title(&self) -> String {
        match self.number_text() {
            Some(n) => format!("{} · run {n}", self.name),
            None => self.name.clone(),
        }
    }
    /// "812", or Azure's "20261104.3".
    pub fn number_text(&self) -> Option<String> {
        self.number.map(|n| n.to_string()).or_else(|| self.build.clone())
    }
    /// Seconds since the epoch when the host created the run.
    pub fn created(&self) -> Option<u64> {
        self.created
    }
    /// When it started running, or was created when the host doesn't say.
    pub fn started(&self) -> Option<u64> {
        self.started.or(self.created)
    }
}
/// One page of a repository's runs, newest first.
#[derive(Clone, Debug)]
pub struct ForgeRunList {
    repository: ForgeRepository,
    session: u64,
    pub runs: Vec<ForgeCiRun>,
    /// The page this is, from 1.
    pub page: usize,
    /// The host has older runs than this page.
    pub more: bool,
    /// The host refused to show this account the runs.
    pub hidden: bool,
    /// The permission the refusal named, in the page's words ("Actions: Read").
    pub missing: Option<String>,
}
impl ForgeRunList {
    pub fn repository(&self) -> &ForgeRepository {
        &self.repository
    }
    /// Whether this list came from that session.
    pub fn from_session(&self, session: &ForgeSession) -> bool {
        self.session == session.id
    }
}
/// One run as the host reports it now, with its jobs.
#[derive(Clone, Debug)]
pub struct ForgeRunDetails {
    repository: ForgeRepository,
    session: u64,
    /// The run, read again, with its failed jobs.
    pub run: ForgeCiRun,
    /// Jobs (GitLab jobs, Bitbucket steps, Azure timeline jobs), failures first.
    pub jobs: Vec<ForgeCheck>,
    /// More jobs exist than were read.
    pub limited: bool,
    /// A read that failed; what was read before it is still shown.
    pub error: Option<ForgeFailure>,
    /// The permission a refused read needed, as the host named it.
    pub missing: Option<String>,
    /// Why Re-run isn't offered, or None when it is.
    pub rerun: Option<ForgeRerunBlock>,
}
impl ForgeRunDetails {
    pub fn repository(&self) -> &ForgeRepository {
        &self.repository
    }
    pub fn count(&self, state: ForgeCheckState) -> usize {
        self.jobs.iter().filter(|c| c.state == state).count()
    }
}
#[derive(Clone, Debug)]
pub struct ForgeCommitChecks {
    repository: ForgeRepository,
    session: u64,
    commit: String,
    /// Failures first, then running, queued, passed, neutral, cancelled, skipped.
    pub checks: Vec<ForgeCheck>,
    pub runs: Vec<ForgeCiRun>,
    /// More checks exist than were read.
    pub limited: bool,
    /// A read that failed; what was read before it is still shown.
    pub error: Option<ForgeFailure>,
    /// The permission a refused read needed, as the host named it ("Checks: Read").
    pub missing: Option<String>,
    /// Why Re-run isn't offered, or None when it is.
    pub rerun: Option<ForgeRerunBlock>,
    /// The browser page for this commit's checks on the host.
    pub url: String,
}
impl ForgeCommitChecks {
    pub fn repository(&self) -> &ForgeRepository {
        &self.repository
    }
    pub fn commit(&self) -> &str {
        &self.commit
    }
    pub fn count(&self, state: ForgeCheckState) -> usize {
        self.checks.iter().filter(|c| c.state == state).count()
    }
    /// The whole commit's result, as its mark shows it.
    pub fn state(&self) -> ForgeCiState {
        ci_rollup(self.checks.iter().map(|c| c.state))
    }
    /// Runs whose failed jobs a re-run would start again.
    pub fn failed_runs(&self) -> impl Iterator<Item = &ForgeCiRun> {
        self.runs.iter().filter(|r| r.state == ForgeCheckState::Failed && !r.failed.is_empty())
    }
}
pub fn ci_rollup(states: impl Iterator<Item = ForgeCheckState>) -> ForgeCiState {
    let mut any = false;
    let mut running = false;
    for s in states {
        any = true;
        match s {
            ForgeCheckState::Failed | ForgeCheckState::Cancelled => return ForgeCiState::Failed,
            ForgeCheckState::Running | ForgeCheckState::Queued => running = true,
            _ => {}
        }
    }
    if running {
        ForgeCiState::Running
    } else if any {
        ForgeCiState::Passed
    } else {
        ForgeCiState::NotChecked
    }
}
/// A reviewed re-run: what the review shows, single use, expiring after two minutes.
#[derive(Debug)]
pub struct ForgeRerunPreview {
    repository: ForgeRepository,
    session: u64,
    commit: String,
    runs: Vec<ForgeCiRun>,
    created: Instant,
    used: AtomicBool,
    /// Made from one run in Actions: checked again by that run, not the commit.
    from_run: bool,
}
impl ForgeRerunPreview {
    pub fn repository(&self) -> &ForgeRepository {
        &self.repository
    }
    pub fn commit(&self) -> &str {
        &self.commit
    }
    pub fn runs(&self) -> &[ForgeCiRun] {
        &self.runs
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForgeRerunResult {
    /// (run title, the attempt that started when the host reports it)
    pub started: Vec<(String, Option<u64>)>,
}

impl ForgeSession {
    /// The mark for each of up to CI_MARK_LIMIT commits. Commits the host
    /// doesn't know are left out. Bitbucket Server has no read here.
    pub fn ci_marks(&self, repo: &ForgeRepository, commits: &[String], c: &CancellationToken) -> Result<ForgeCiMarks> {
        self.runner.step(MARKS, "read_ci_marks", || {
            self.check_repository(repo)?;
            let mut asked: Vec<String> = Vec::new();
            for id in commits {
                let id = oid(id).ok_or_else(|| invalid("A commit ID for checks is invalid."))?;
                if !asked.contains(&id) {
                    asked.push(id);
                }
            }
            let limited = asked.len() > CI_MARK_LIMIT;
            asked.truncate(CI_MARK_LIMIT);
            if asked.is_empty() {
                return Ok(ForgeCiMarks::default());
            }
            let read = || -> Result<ForgeCiMarks> {
                let marks = match self.config.provider() {
                    p if p.is_github() => match self.github_marks(repo, &asked, c) {
                        // Some tokens can't read the rollup; Actions runs still give a mark.
                        Err(Error::Forge(ForgeFailure::Forbidden | ForgeFailure::Unsupported)) => self.github_run_marks(repo, &asked, c)?,
                        other => other?,
                    },
                    ForgeProvider::BitbucketServer => return fail(ForgeFailure::Unsupported),
                    _ => {
                        let mut asked = asked.clone();
                        let per_commit = matches!(self.config.provider(), ForgeProvider::Gitea | ForgeProvider::Bitbucket);
                        let limited_hosts = per_commit && asked.len() > CI_MARK_REQUEST_LIMIT;
                        if per_commit {
                            asked.truncate(CI_MARK_REQUEST_LIMIT);
                        }
                        let marks = self.host_marks(repo, &asked, c)?;
                        return Ok(ForgeCiMarks { marks, limited: limited || limited_hosts, ..Default::default() });
                    }
                };
                Ok(ForgeCiMarks { marks, limited, ..Default::default() })
            };
            match read() {
                // A refusal is an answer: the page says the checks are hidden, and why.
                Err(Error::Forge(ForgeFailure::Forbidden)) => Ok(ForgeCiMarks {
                    hidden: true,
                    missing: transport::take_refusal().as_deref().and_then(permission_name),
                    ..Default::default()
                }),
                other => other,
            }
        })
    }
    /// The checks of one commit, with what a re-run would start and whether it can.
    pub fn commit_checks(&self, repo: &ForgeRepository, commit: &str, c: &CancellationToken) -> Result<ForgeCommitChecks> {
        self.runner.step(DETAILS, "read_ci_details", || {
            self.check_repository(repo)?;
            let commit = oid(commit).ok_or_else(|| invalid("Select a valid commit."))?;
            let mut out = ForgeCommitChecks {
                repository: repo.clone(),
                session: self.id,
                url: repo.commit_url(&ObjectId::parse(&commit)?),
                commit,
                checks: Vec::new(),
                runs: Vec::new(),
                limited: false,
                error: None,
                missing: None,
                rerun: None,
            };
            match self.config.provider() {
                p if p.is_github() => self.github_checks(&mut out, c)?,
                ForgeProvider::BitbucketServer => return fail(ForgeFailure::Unsupported),
                _ => self.host_checks(&mut out, c)?,
            }
            out.checks.sort_by(|a, b| a.state.cmp(&b.state).then_with(|| a.group.cmp(&b.group)).then_with(|| a.name.cmp(&b.name)));
            Ok(out)
        })
    }
    /// Checks again what a re-run would start: the runs still failed, not
    /// running, recent enough, and this account can still write.
    pub fn preview_rerun(&self, checks: &ForgeCommitChecks, c: &CancellationToken) -> Result<ForgeRerunPreview> {
        self.runner.step(RERUN_PREVIEW, "validate_ci_rerun", || {
            if checks.session != self.id {
                return fail(ForgeFailure::StaleReview);
            }
            self.check_repository(&checks.repository)?;
            let fresh = self.commit_checks(&checks.repository, &checks.commit, c)?;
            if let Some(block) = fresh.rerun {
                return Err(invalid(ci_rerun_refusal(block)));
            }
            if let Some(e) = fresh.error {
                return Err(invalid(&format!("Some checks couldn't be read ({}), so Rocket can't confirm what a re-run would start. Refresh the checks.", e.code())));
            }
            let github = self.config.provider().is_github();
            let now = ci_now();
            let runs: Vec<ForgeCiRun> = fresh.failed_runs().filter(|r| !(github && too_old(r, now))).cloned().collect();
            if runs.is_empty() {
                return Err(invalid("No failed run is left to re-run. Refresh the checks."));
            }
            Ok(ForgeRerunPreview { repository: fresh.repository.clone(), session: self.id, commit: fresh.commit.clone(), runs, created: Instant::now(), used: AtomicBool::new(false), from_run: false })
        })
    }
    /// The newest runs of the repository, CI_RUN_PAGE at a time, for one branch
    /// or all. Bitbucket Server has no CI read here.
    pub fn ci_runs(&self, repo: &ForgeRepository, branch: Option<&str>, page: usize, c: &CancellationToken) -> Result<ForgeRunList> {
        self.runner.step(RUNS, "read_ci_runs", || {
            self.check_repository(repo)?;
            if !(1..=CI_RUN_PAGES).contains(&page) {
                return Err(invalid("Actions lists only the newest runs."));
            }
            if let Some(branch) = branch {
                validate_branch(branch)?;
            }
            let mut list = ForgeRunList { repository: repo.clone(), session: self.id, runs: Vec::new(), page, more: false, hidden: false, missing: None };
            if self.config.provider() == ForgeProvider::BitbucketServer {
                return fail(ForgeFailure::Unsupported);
            }
            match self.host_runs(&mut list, branch, c) {
                // A refusal is an answer: the page says the runs are hidden, and why.
                Err(Error::Forge(ForgeFailure::Forbidden)) => {
                    list.runs.clear();
                    list.hidden = true;
                    list.missing = transport::take_refusal().as_deref().and_then(permission_name);
                }
                other => other?,
            }
            list.more &= page < CI_RUN_PAGES && !list.hidden;
            Ok(list)
        })
    }
    /// One run read again, with its jobs and whether its failed jobs can be re-run.
    pub fn ci_run(&self, repo: &ForgeRepository, run: &ForgeCiRun, c: &CancellationToken) -> Result<ForgeRunDetails> {
        self.runner.step(RUN, "read_ci_run", || {
            self.check_repository(repo)?;
            let mut out = ForgeRunDetails { repository: repo.clone(), session: self.id, run: run.clone(), jobs: Vec::new(), limited: false, error: None, missing: None, rerun: None };
            match self.config.provider() {
                ForgeProvider::BitbucketServer => return fail(ForgeFailure::Unsupported),
                _ => self.run_details(&mut out, c)?,
            }
            // Failures first; otherwise the host's order.
            out.jobs.sort_by_key(|j| j.state);
            let gitlab = self.config.provider() == ForgeProvider::GitLab;
            out.run.failed = out.jobs.iter().filter(|j| j.state == ForgeCheckState::Failed || (gitlab && j.state == ForgeCheckState::Cancelled)).map(|j| j.name.clone()).collect();
            if out.run.failed.is_empty() {
                return Ok(out);
            }
            // A cancelled or passing run with a failed job (continue-on-error) offers no re-run.
            if matches!(out.run.state, ForgeCheckState::Running | ForgeCheckState::Queued) {
                out.rerun = Some(ForgeRerunBlock::StillRunning);
            } else if self.retryable(&out.run) {
                out.rerun = if self.config.provider().is_github() && too_old(&out.run, ci_now()) {
                    Some(ForgeRerunBlock::TooOld)
                } else if out.error.is_some() {
                    Some(ForgeRerunBlock::Unverified)
                } else {
                    match self.rerun_access(RUN, repo, c)? {
                        Ok(block) => block,
                        Err(e) => {
                            self.keep(&mut out.error, &mut out.missing, e)?;
                            Some(ForgeRerunBlock::Unverified)
                        }
                    }
                };
            } else if matches!(self.config.provider(), ForgeProvider::Bitbucket | ForgeProvider::Azure | ForgeProvider::AzureServer) {
                out.rerun = Some(ForgeRerunBlock::Unsupported);
            } else {
                // The host re-runs failed jobs only of a failed run: nothing to offer here.
                out.run.failed.clear();
            }
            Ok(out)
        })
    }
    /// Checks one run again for Actions: still failed, the same attempt, and
    /// this account can still write.
    pub fn preview_run_rerun(&self, details: &ForgeRunDetails, c: &CancellationToken) -> Result<ForgeRerunPreview> {
        self.runner.step(RERUN_PREVIEW, "validate_ci_rerun", || {
            if details.session != self.id {
                return fail(ForgeFailure::StaleReview);
            }
            self.check_repository(&details.repository)?;
            let fresh = self.ci_run(&details.repository, &details.run, c)?;
            if let Some(block) = fresh.rerun {
                return Err(invalid(ci_rerun_refusal(block)));
            }
            if let Some(e) = fresh.error {
                return Err(invalid(&format!("Part of the run couldn't be read ({}), so Rocket can't confirm what a re-run would start. Refresh the run.", e.code())));
            }
            if !self.retryable(&fresh.run) {
                return Err(invalid("This run has no failed jobs left to re-run. Refresh the run."));
            }
            let commit = fresh.run.commit.clone().unwrap_or_default();
            Ok(ForgeRerunPreview { repository: fresh.repository.clone(), session: self.id, commit, runs: vec![fresh.run], created: Instant::now(), used: AtomicBool::new(false), from_run: true })
        })
    }
    /// A run whose failed jobs the host re-runs: failed (GitLab also retries a
    /// canceled pipeline) with a failed job.
    fn retryable(&self, run: &ForgeCiRun) -> bool {
        let gitlab = self.config.provider() == ForgeProvider::GitLab;
        (run.state == ForgeCheckState::Failed || (gitlab && run.state == ForgeCheckState::Cancelled)) && !run.failed.is_empty()
    }
    /// Whether this account may re-run here: write access on the repository
    /// and, for a classic GitHub token or a GitLab token, the scope for it.
    /// Ok(Err): the access couldn't be read, so Re-run isn't offered. Err: the
    /// host answered for another repository, which fails the whole read.
    fn rerun_access(&self, op: &'static str, repo: &ForgeRepository, c: &CancellationToken) -> Result<Result<Option<ForgeRerunBlock>>> {
        let p = self.config.provider();
        if p.is_github() || p == ForgeProvider::Gitea {
            let (j, headers) = match self.request(op, "read_ci_permissions", "GET", &format!("/repos/{}", repo.encoded()), None, false, c) {
                Ok(answer) => answer,
                Err(e) => return Ok(Err(e)),
            };
            if !j.str("full_name").is_some_and(|n| n.eq_ignore_ascii_case(repo.path())) {
                return fail(ForgeFailure::InvalidResponse);
            }
            // GitHub must say the repository isn't archived; Gitea may leave it out.
            let archived = if p.is_github() { j.bool("archived") != Some(false) } else { j.bool("archived") == Some(true) };
            if archived || j.get("permissions").and_then(|p| p.bool("push")) != Some(true) {
                return Ok(Ok(Some(ForgeRerunBlock::NoWriteAccess)));
            }
            if p.is_github()
                && let Some(scopes) = header(&headers, "x-oauth-scopes")
                && !scopes.split(',').any(|s| s.trim() == "repo")
            {
                return Ok(Ok(Some(ForgeRerunBlock::TokenScope)));
            }
            return Ok(Ok(None));
        }
        if p != ForgeProvider::GitLab {
            return Ok(Ok(Some(ForgeRerunBlock::Unsupported)));
        }
        // Developer (30) or above can retry; a token needs the api scope.
        let j = match self.request(op, "read_ci_permissions", "GET", &self.prefix(repo), None, false, c) {
            Ok((j, _)) => j,
            Err(e) => return Ok(Err(e)),
        };
        let level = |k: &str| j.get("permissions").and_then(|p| p.get(k)).and_then(|a| a.u64("access_level")).unwrap_or(0);
        if j.bool("archived") == Some(true) || level("project_access").max(level("group_access")) < 30 {
            return Ok(Ok(Some(ForgeRerunBlock::NoWriteAccess)));
        }
        if let Ok((j, _)) = self.request(op, "read_ci_token_scopes", "GET", "/personal_access_tokens/self", None, false, c)
            && let Some(scopes) = j.get("scopes").and_then(Json::array)
            && !scopes.iter().any(|s| *s == Json::String("api".into()))
        {
            return Ok(Ok(Some(ForgeRerunBlock::TokenScope)));
        }
        Ok(Ok(None))
    }
    /// Sends the reviewed re-run once. No automatic retry: a request that may
    /// have reached the host and has no confirmed answer is OutcomeUnknown.
    pub fn submit_rerun(&self, preview: &ForgeRerunPreview, c: &CancellationToken) -> Result<ForgeRerunResult> {
        self.runner.step(RERUN, "submit_ci_rerun", || {
            if preview.session != self.id || preview.created.elapsed() > REVIEW_LIFETIME || preview.used.load(Ordering::Acquire) {
                return fail(ForgeFailure::StaleReview);
            }
            if preview.from_run {
                // Actions reviewed one run: that run, read again, must still match.
                let run = preview.runs.first().ok_or(Error::Forge(ForgeFailure::StaleReview))?;
                let fresh = self.ci_run(&preview.repository, run, c)?;
                if fresh.rerun.is_some() || fresh.error.is_some() || !self.retryable(&fresh.run) || fresh.run.attempt != run.attempt {
                    return fail(ForgeFailure::StaleReview);
                }
            } else {
                let fresh = self.commit_checks(&preview.repository, &preview.commit, c)?;
                if fresh.rerun.is_some() || fresh.error.is_some() {
                    return fail(ForgeFailure::StaleReview);
                }
                for run in &preview.runs {
                    if !fresh.failed_runs().any(|r| r.id == run.id && r.attempt == run.attempt) {
                        return fail(ForgeFailure::StaleReview);
                    }
                }
            }
            c.check()?;
            if preview.used.swap(true, Ordering::AcqRel) {
                return fail(ForgeFailure::StaleReview);
            }
            let mut result = ForgeRerunResult { started: Vec::new() };
            for run in &preview.runs {
                // Runs go one at a time; an uncertain answer stops the rest.
                match self.send_rerun(&preview.repository, run, c) {
                    Ok(attempt) => result.started.push((run.title(), attempt)),
                    Err(Error::Forge(ForgeFailure::Forbidden | ForgeFailure::Rejected)) => {
                        let needed = transport::take_refusal().as_deref().and_then(permission_name);
                        let host = self.config.provider().label();
                        let why = match (&needed, self.config.provider()) {
                            (Some(p), _) => format!("this token needs {p}"),
                            (None, ForgeProvider::GitLab) => "re-running needs the api scope and the Developer role, or Maintainer on a protected branch".into(),
                            _ => "it refused the request; check the token's permissions and the run".into(),
                        };
                        let done = if result.started.is_empty() { "Nothing was re-run.".to_string() } else { format!("{} started again.", result.started.iter().map(|s| s.0.as_str()).collect::<Vec<_>>().join(", ")) };
                        return Err(invalid(&format!("{host} refused {}: {why}. {done}", run.title())));
                    }
                    Err(Error::Forge(f)) if !result.started.is_empty() && f != ForgeFailure::OutcomeUnknown => {
                        let done = result.started.iter().map(|s| s.0.as_str()).collect::<Vec<_>>().join(", ");
                        return Err(invalid(&format!("{done} started again, but {} wasn't sent: {} ({}).", run.title(), f.guidance(), f.code())));
                    }
                    Err(e) => return Err(e),
                }
            }
            Ok(result)
        })
    }
    fn send_rerun(&self, repo: &ForgeRepository, run: &ForgeCiRun, c: &CancellationToken) -> Result<Option<u64>> {
        let path = match self.config.provider() {
            p if p.is_github() => format!("/repos/{}/actions/runs/{}/rerun-failed-jobs", repo.encoded(), run.id),
            ForgeProvider::Gitea => format!("/repos/{}/actions/runs/{}/rerun-failed-jobs", repo.encoded(), run.id),
            ForgeProvider::GitLab => format!("{}/pipelines/{}/retry", self.prefix(repo), run.id),
            _ => return fail(ForgeFailure::Unsupported),
        };
        let body = if self.config.provider().is_github() { "{\"enable_debug_logging\":false}" } else { "{}" };
        let (value, _) = self.request(RERUN, "send_ci_rerun", "POST", &path, Some(body), true, c)?;
        Ok(match self.config.provider() {
            // GitLab answers with the retried pipeline; its id must match.
            ForgeProvider::GitLab => {
                if value.u64("id") != Some(run.id) {
                    return fail(ForgeFailure::OutcomeUnknown);
                }
                None
            }
            _ => run.attempt.map(|a| a + 1),
        })
    }
    fn github_marks(&self, repo: &ForgeRepository, asked: &[String], c: &CancellationToken) -> Result<Vec<(String, ForgeCiState)>> {
        let (owner, name) = repo.parts();
        let mut query = String::from("query RocketCiMarks($owner:String!,$name:String!");
        for i in 0..asked.len() {
            query.push_str(&format!(",$c{i}:GitObjectID!"));
        }
        query.push_str("){repository(owner:$owner,name:$name){nameWithOwner");
        for i in 0..asked.len() {
            query.push_str(&format!(" c{i}:object(oid:$c{i}){{...on Commit{{oid statusCheckRollup{{state}}}}}}"));
        }
        query.push_str("}}");
        let mut variables = format!("\"owner\":{},\"name\":{}", quote(owner), quote(name));
        for (i, id) in asked.iter().enumerate() {
            variables.push_str(&format!(",\"c{i}\":{}", quote(id)));
        }
        let payload = format!("{{\"query\":{},\"variables\":{{{variables}}}}}", quote(&query));
        let response = self.request(MARKS, "read_ci_rollup", "POST", "/graphql", Some(&payload), false, c)?.0;
        if let Some(failure) = graphql_failure(&response) {
            return fail(failure);
        }
        let repository = response.get("data").and_then(|d| d.get("repository")).ok_or(Error::Forge(ForgeFailure::InvalidResponse))?;
        if !repository.str("nameWithOwner").is_some_and(|p| p.eq_ignore_ascii_case(repo.path())) {
            return fail(ForgeFailure::InvalidResponse);
        }
        let mut marks = Vec::new();
        for (i, id) in asked.iter().enumerate() {
            let Some(commit) = repository.get(&format!("c{i}")) else { return fail(ForgeFailure::InvalidResponse) };
            if *commit == Json::Null {
                continue;
            }
            if commit.str("oid").map(str::to_ascii_lowercase).as_deref() != Some(id) {
                return fail(ForgeFailure::InvalidResponse);
            }
            let state = match commit.get("statusCheckRollup") {
                Some(Json::Null) | None => ForgeCiState::NotChecked,
                Some(r) => match r.str("state") {
                    Some("SUCCESS") => ForgeCiState::Passed,
                    Some("FAILURE" | "ERROR") => ForgeCiState::Failed,
                    Some("PENDING" | "EXPECTED") => ForgeCiState::Running,
                    _ => return fail(ForgeFailure::InvalidResponse),
                },
            };
            marks.push((id.clone(), state));
        }
        Ok(marks)
    }
    fn github_run_marks(&self, repo: &ForgeRepository, asked: &[String], c: &CancellationToken) -> Result<Vec<(String, ForgeCiState)>> {
        let j = self.request(MARKS, "read_ci_run_marks", "GET", &format!("/repos/{}/actions/runs?per_page=100&exclude_pull_requests=true", repo.encoded()), None, false, c)?.0;
        let rows = j.get("workflow_runs").and_then(Json::array).filter(|a| a.len() <= 100).ok_or(Error::Forge(ForgeFailure::InvalidResponse))?;
        let mut marks = Vec::new();
        for id in asked {
            let mut states = Vec::new();
            for row in rows.iter().filter(|r| r.str("head_sha").map(str::to_ascii_lowercase).as_deref() == Some(id.as_str())) {
                states.push(github_run(row, &self.config, repo).ok_or(Error::Forge(ForgeFailure::InvalidResponse))?.state);
            }
            if !states.is_empty() {
                marks.push((id.clone(), ci_rollup(states.into_iter())));
            }
        }
        Ok(marks)
    }
    fn github_checks(&self, out: &mut ForgeCommitChecks, c: &CancellationToken) -> Result<()> {
        let repo = out.repository.clone();
        let base = format!("/repos/{}", repo.encoded());
        let sha = out.commit.clone();
        // Check runs, then commit statuses, then Actions runs: each failure is
        // kept, and what was read before it still shows.
        let mut suites: Vec<(u64, usize)> = Vec::new();
        match self.request(DETAILS, "read_ci_check_runs", "GET", &format!("{base}/commits/{sha}/check-runs?per_page={CHECK_LIMIT}&filter=latest"), None, false, c) {
            Ok((j, _)) => {
                let rows = j.get("check_runs").and_then(Json::array).filter(|a| a.len() <= CHECK_LIMIT).ok_or(Error::Forge(ForgeFailure::InvalidResponse))?;
                out.limited |= j.u64("total_count").is_some_and(|t| t > rows.len() as u64);
                for row in rows {
                    if row.str("head_sha").map(str::to_ascii_lowercase).as_deref() != Some(&sha) {
                        return fail(ForgeFailure::InvalidResponse);
                    }
                    let check = github_check_run(row, &self.config, &repo).ok_or(Error::Forge(ForgeFailure::InvalidResponse))?;
                    if let Some(suite) = row.get("check_suite").and_then(|s| s.u64("id")) {
                        suites.push((suite, out.checks.len()));
                    }
                    out.checks.push(check);
                }
            }
            Err(e) => self.keep_failure(out, e)?,
        }
        match self.request(DETAILS, "read_ci_statuses", "GET", &format!("{base}/commits/{sha}/status?per_page={CHECK_LIMIT}"), None, false, c) {
            Ok((j, _)) => {
                if j.str("sha").map(str::to_ascii_lowercase).as_deref() != Some(&sha) {
                    return fail(ForgeFailure::InvalidResponse);
                }
                let rows = j.get("statuses").and_then(Json::array).filter(|a| a.len() <= CHECK_LIMIT).ok_or(Error::Forge(ForgeFailure::InvalidResponse))?;
                out.limited |= j.u64("total_count").is_some_and(|t| t > rows.len() as u64);
                for row in rows {
                    out.checks.push(status_check(row, &self.config, "context", "state", "description", "target_url").ok_or(Error::Forge(ForgeFailure::InvalidResponse))?);
                }
            }
            Err(e) => self.keep_failure(out, e)?,
        }
        match self.request(DETAILS, "read_ci_runs", "GET", &format!("{base}/actions/runs?head_sha={sha}&per_page=20&exclude_pull_requests=true"), None, false, c) {
            Ok((j, _)) => {
                let rows = j.get("workflow_runs").and_then(Json::array).filter(|a| a.len() <= 20).ok_or(Error::Forge(ForgeFailure::InvalidResponse))?;
                for row in rows {
                    if row.str("head_sha").map(str::to_ascii_lowercase).as_deref() != Some(&sha) {
                        return fail(ForgeFailure::InvalidResponse);
                    }
                    let run = github_run(row, &self.config, &repo).ok_or(Error::Forge(ForgeFailure::InvalidResponse))?;
                    // A run's jobs are its suite's check runs: label them and
                    // collect the failed ones for the review.
                    let suite = row.u64("check_suite_id");
                    let mut run = run;
                    for (id, index) in &suites {
                        if Some(*id) == suite {
                            let check = &mut out.checks[*index];
                            check.group = run.title();
                            check.run = Some(run.id);
                            if check.state == ForgeCheckState::Failed {
                                run.failed.push(check.name.clone());
                            }
                        }
                    }
                    out.runs.push(run);
                }
            }
            Err(e) => self.keep_failure(out, e)?,
        }
        if out.failed_runs().next().is_none() {
            out.rerun = rerun_block(&out.runs, ci_now(), true);
            return Ok(());
        }
        // Re-run is offered only when GitHub says this account can write here
        // (as Merge checks) and a classic token's scopes allow it.
        let access = match self.rerun_access(DETAILS, &repo, c)? {
            Ok(access) => access,
            Err(e) => {
                self.keep_failure(out, e)?;
                out.rerun = Some(ForgeRerunBlock::Unverified);
                return Ok(());
            }
        };
        out.rerun = rerun_block(&out.runs, ci_now(), true).or(access);
        Ok(())
    }
    /// Keeps a read's failure for the page. A rejected token stops everything.
    fn keep_failure(&self, out: &mut ForgeCommitChecks, e: Error) -> Result<()> {
        self.keep(&mut out.error, &mut out.missing, e)
    }
    fn keep(&self, error: &mut Option<ForgeFailure>, missing: &mut Option<String>, e: Error) -> Result<()> {
        match e {
            Error::Forge(ForgeFailure::Unauthorized) => Err(e),
            Error::Forge(f) => {
                if error.is_none() {
                    *error = Some(f);
                }
                if f == ForgeFailure::Forbidden && missing.is_none() {
                    *missing = transport::take_refusal().as_deref().and_then(permission_name);
                }
                Ok(())
            }
            other => Err(other),
        }
    }
}
impl ForgeRepository {
    /// The host's page listing this repository's CI runs.
    pub fn runs_url(&self) -> String {
        let server = self.config().server();
        let path = self.encoded();
        let (project, name) = self.parts();
        match self.config().provider() {
            ForgeProvider::GitLab => format!("{server}/{path}/-/pipelines"),
            ForgeProvider::Bitbucket => format!("{server}/{path}/pipelines"),
            // Bitbucket Server has no runs page; its repository page shows builds per commit.
            ForgeProvider::BitbucketServer => match project.strip_prefix('~') {
                Some(user) => format!("{server}/users/{}/repos/{}/browse", encode(user), encode(name)),
                None => format!("{server}/projects/{}/repos/{}/browse", encode(project), encode(name)),
            },
            ForgeProvider::Azure | ForgeProvider::AzureServer => format!("{server}/{}/_build", encode(project)),
            _ => format!("{server}/{path}/actions"),
        }
    }
}
/// Why a host refused, in words for the strip. Bounded, from Rocket's own table.
pub fn ci_rerun_refusal(block: ForgeRerunBlock) -> &'static str {
    match block {
        ForgeRerunBlock::StillRunning => "The run is still running. Re-run its failed jobs when it finishes.",
        ForgeRerunBlock::TooOld => "The host re-runs a run only within 30 days. Open it in the browser.",
        ForgeRerunBlock::NoWriteAccess => "This account can read the repository but not write to it. Re-running needs write access.",
        ForgeRerunBlock::TokenScope => "This token's scopes don't allow re-running. Give it repo (GitHub) or api (GitLab).",
        ForgeRerunBlock::Unsupported => "This host's API can't re-run failed jobs. Open the run in the browser.",
        ForgeRerunBlock::Unverified => "Rocket couldn't confirm this account can write here. Refresh the checks to try again.",
    }
}
fn too_old(run: &ForgeCiRun, now: u64) -> bool {
    run.created.is_some_and(|t| now.saturating_sub(t) > RERUN_WINDOW)
}
fn rerun_block(runs: &[ForgeCiRun], now: u64, window: bool) -> Option<ForgeRerunBlock> {
    let failed: Vec<_> = runs.iter().filter(|r| r.state == ForgeCheckState::Failed && !r.failed.is_empty()).collect();
    if failed.is_empty() {
        // A job failed in a run that hasn't finished: the host refuses until it does.
        return runs
            .iter()
            .any(|r| matches!(r.state, ForgeCheckState::Running | ForgeCheckState::Queued) && !r.failed.is_empty())
            .then_some(ForgeRerunBlock::StillRunning);
    }
    if window && failed.iter().all(|r| too_old(r, now)) {
        return Some(ForgeRerunBlock::TooOld);
    }
    None
}
/// GitHub names a refused request's permission (X-Accepted-GitHub-Permissions:
/// "actions=read"); a fixed table turns it into the page's words.
fn permission_name(value: &str) -> Option<String> {
    let first = value.split(';').next()?.split(',').next()?.trim();
    let (key, level) = first.split_once('=')?;
    let name = match key.trim() {
        "actions" => "Actions",
        "checks" => "Checks",
        "statuses" => "Commit statuses",
        "contents" => "Contents",
        "metadata" => "Metadata",
        "pull_requests" => "Pull requests",
        _ => return None,
    };
    let level = match level.trim() {
        "read" => "Read",
        "write" => "Read and write",
        _ => return None,
    };
    Some(format!("{name}: {level}"))
}
fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    let mut values = headers.iter().filter(|(k, _)| k.eq_ignore_ascii_case(name));
    let first = values.next()?;
    values.next().is_none().then_some(first.1.as_str()).filter(|v| v.len() <= 512)
}
fn graphql_failure(response: &Json) -> Option<ForgeFailure> {
    let rows = response.get("errors")?;
    let first = rows.array().and_then(|r| r.first());
    let code = first.and_then(|f| f.str("type").or_else(|| f.get("extensions").and_then(|v| v.str("code"))));
    Some(match code {
        Some("FORBIDDEN") => ForgeFailure::Forbidden,
        Some("NOT_FOUND") => ForgeFailure::NotFound,
        Some("RATE_LIMITED") => ForgeFailure::RateLimited,
        Some("GRAPHQL_VALIDATION_FAILED") => ForgeFailure::Unsupported,
        _ => ForgeFailure::InvalidResponse,
    })
}
fn oid(s: &str) -> Option<String> {
    (s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())).then(|| s.to_ascii_lowercase())
}
fn text(j: &Json, key: &str, max: usize) -> Option<String> {
    j.str(key).map(|s| crate::display_text::bounded(s, max).text)
}
pub(crate) fn ci_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}
/// Seconds since the epoch for "2026-11-04T09:12:03Z" or with an offset/fraction.
fn timestamp(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let n = |r: std::ops::Range<usize>| s.get(r).filter(|t| t.bytes().all(|c| c.is_ascii_digit())).and_then(|t| t.parse::<i64>().ok());
    let (y, mo, d, h, mi, se) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    let mut rest = &s[19..];
    if let Some(fraction) = rest.strip_prefix('.') {
        let digits = fraction.bytes().take_while(u8::is_ascii_digit).count();
        rest = &fraction[digits..];
    }
    let offset = match rest {
        "Z" | "z" => 0,
        r if r.len() == 6 && r.is_ascii() && matches!(r.as_bytes()[0], b'+' | b'-') && r.as_bytes()[3] == b':' => {
            let hours: i64 = r[1..3].parse().ok()?;
            let minutes: i64 = r[4..6].parse().ok()?;
            let sign = if &r[..1] == "-" { -1 } else { 1 };
            sign * (hours * 3600 + minutes * 60)
        }
        _ => return None,
    };
    // Days from civil, Howard Hinnant's algorithm.
    let (y, m) = if mo <= 2 { (y - 1, mo + 9) } else { (y, mo - 3) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    let secs = days * 86400 + h * 3600 + mi * 60 + se - offset;
    u64::try_from(secs).ok()
}
/// "4m 41s", "55s", "1h 02m".
pub fn ci_duration_text(seconds: u64) -> String {
    match seconds {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m {:02}s", s / 60, s % 60),
        s => format!("{}h {:02}m", s / 3600, (s % 3600) / 60),
    }
}
fn span(start: Option<u64>, end: Option<u64>) -> Option<u64> {
    end?.checked_sub(start?)
}
/// A URL a response supplied, kept only when it is plain HTTPS without
/// credentials. The account's own server keeps no host name; any other site does.
fn link(url: Option<&str>, config: &ForgeConfig) -> Option<ForgeCheckLink> {
    let url = url?;
    if url.len() > 2048 || !url.is_ascii() || url.bytes().any(|b| b <= b' ' || matches!(b, b'"' | b'\\' | b'<' | b'>')) {
        return None;
    }
    let rest = url.strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    let host = authority.split(':').next()?.to_ascii_lowercase();
    if host.is_empty() || !host.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-')) {
        return None;
    }
    let own = config.server().strip_prefix("https://").and_then(|s| s.split('/').next()).map(str::to_ascii_lowercase);
    let external = (own.as_deref() != Some(authority.to_ascii_lowercase().as_str())).then_some(host);
    Some(ForgeCheckLink { url: url.into(), external })
}
fn github_check_run(row: &Json, config: &ForgeConfig, _repo: &ForgeRepository) -> Option<ForgeCheck> {
    let name = text(row, "name", 256)?;
    let status = row.str("status")?;
    let conclusion = row.str("conclusion");
    let started = row.str("started_at").and_then(timestamp);
    let completed = row.str("completed_at").and_then(timestamp);
    let took = span(started, completed).map(ci_duration_text);
    let (state, detail) = match (status, conclusion) {
        ("completed", Some("success")) => (ForgeCheckState::Passed, took.unwrap_or_else(|| "Passed".into())),
        ("completed", Some("failure")) => (ForgeCheckState::Failed, took.map_or("Failed".into(), |t| format!("Failed after {t}"))),
        ("completed", Some("timed_out")) => (ForgeCheckState::Failed, took.map_or("Timed out".into(), |t| format!("Timed out after {t}"))),
        ("completed", Some("startup_failure")) => (ForgeCheckState::Failed, "Failed to start".into()),
        ("completed", Some("cancelled")) => (ForgeCheckState::Cancelled, "Cancelled".into()),
        ("completed", Some("skipped")) => (ForgeCheckState::Skipped, "Skipped".into()),
        ("completed", Some("neutral")) => (ForgeCheckState::Neutral, "Neutral".into()),
        ("completed", Some("stale")) => (ForgeCheckState::Neutral, "Stale".into()),
        ("completed", Some("action_required")) => (ForgeCheckState::Queued, "Needs approval".into()),
        ("in_progress", _) => (ForgeCheckState::Running, started.map_or("Running".into(), |s| format!("Running for {}", ci_duration_text(ci_now().saturating_sub(s))))),
        ("queued", _) => (ForgeCheckState::Queued, "Queued".into()),
        ("waiting" | "requested" | "pending", _) => (ForgeCheckState::Queued, "Waiting".into()),
        _ => return None,
    };
    let app = row.get("app").and_then(|a| text(a, "name", 128)).unwrap_or_else(|| "Checks".into());
    Some(ForgeCheck { name, group: app, state, detail, link: link(row.str("html_url").or_else(|| row.str("details_url")), config), run: None })
}
/// A GitHub Actions or Gitea Actions run.
fn github_run(row: &Json, config: &ForgeConfig, _repo: &ForgeRepository) -> Option<ForgeCiRun> {
    let id = row.u64("id").filter(|n| *n > 0)?;
    let status = row.str("status")?;
    let state = match (status, row.str("conclusion")) {
        ("completed", Some("failure" | "timed_out" | "startup_failure")) => ForgeCheckState::Failed,
        ("completed", Some("success")) => ForgeCheckState::Passed,
        ("completed", Some("cancelled")) => ForgeCheckState::Cancelled,
        ("completed", Some("skipped")) => ForgeCheckState::Skipped,
        ("completed", _) => ForgeCheckState::Neutral,
        ("in_progress", _) => ForgeCheckState::Running,
        _ => ForgeCheckState::Queued,
    };
    // Gitea may name a run only by its workflow file: "ci.yml@refs/heads/main".
    let file = row.str("path").and_then(|p| p.split('@').next()).and_then(|p| p.rsplit('/').next()).filter(|f| !f.is_empty()).map(|f| crate::display_text::bounded(f, 128).text);
    let mut run = ForgeCiRun::new(id, text(row, "name", 128).filter(|n| !n.trim().is_empty()).or(file).unwrap_or_else(|| "Workflow".into()), state);
    run.number = row.u64("run_number");
    run.attempt = row.u64("run_attempt");
    run.link = link(row.str("html_url"), config);
    run.branch = branch_name(row.str("head_branch"));
    let pull = row.get("pull_requests").and_then(Json::array).and_then(|a| a.first()).and_then(|p| p.u64("number"));
    run.event = row.str("event").map(|e| match e {
        "push" => "push".into(),
        "pull_request" | "pull_request_target" => pull.map_or_else(|| "pull request".into(), |n| format!("pull request #{n}")),
        "schedule" => "schedule".into(),
        "workflow_dispatch" => "manual".into(),
        "merge_group" => "merge queue".into(),
        "repository_dispatch" => "API".into(),
        other => words(other),
    });
    run.commit = row.str("head_sha").and_then(oid);
    run.subject = row.get("head_commit").and_then(|h| h.str("message")).or_else(|| row.str("display_title")).and_then(first_line);
    run.created = row.str("created_at").and_then(timestamp);
    run.started = row.str("run_started_at").or_else(|| row.str("started_at")).and_then(timestamp);
    if status == "completed" {
        let finished = row.str("completed_at").or_else(|| row.str("updated_at")).and_then(timestamp);
        run.took = span(run.started(), finished);
    }
    Some(run)
}
/// A branch name a host reported, without "refs/heads/".
fn branch_name(value: Option<&str>) -> Option<String> {
    let name = value?;
    let name = name.strip_prefix("refs/heads/").unwrap_or(name);
    (!name.trim().is_empty()).then(|| crate::display_text::bounded(name, 255).text)
}
/// The first line of a commit message or title, bounded.
fn first_line(message: &str) -> Option<String> {
    let line = message.lines().map(str::trim).find(|l| !l.is_empty())?;
    Some(crate::display_text::bounded(line, 200).text)
}
/// A host's own event name in words: "workflow_run" is "workflow run".
fn words(value: &str) -> String {
    crate::display_text::bounded(&value.replace('_', " "), 40).text
}
/// One commit status (GitHub, Gitea, Bitbucket and Azure name the fields differently).
fn status_check(row: &Json, config: &ForgeConfig, name: &str, state: &str, description: &str, url: &str) -> Option<ForgeCheck> {
    let label = text(row, name, 256).or_else(|| row.get("context").and_then(|c| text(c, "name", 256)))?;
    let raw = row.str(state)?;
    let (state, word) = match raw.to_ascii_lowercase().as_str() {
        "success" | "successful" | "succeeded" => (ForgeCheckState::Passed, "Passed"),
        "failure" | "failed" | "error" => (ForgeCheckState::Failed, "Failed"),
        "pending" | "inprogress" => (ForgeCheckState::Running, "Pending"),
        "warning" => (ForgeCheckState::Passed, "Passed with warnings"),
        "stopped" => (ForgeCheckState::Cancelled, "Stopped"),
        "skipped" | "notapplicable" | "notset" => (ForgeCheckState::Skipped, "Skipped"),
        _ => return None,
    };
    let detail = text(row, description, 140).filter(|d| !d.trim().is_empty()).unwrap_or_else(|| word.into());
    Some(ForgeCheck { name: label, group: "Commit status".into(), state, detail, link: link(row.str(url), config), run: None })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timestamps_and_durations() {
        assert_eq!(timestamp("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(timestamp("2026-11-04T09:12:03Z"), Some(1_793_783_523));
        assert_eq!(timestamp("2026-11-04T17:12:03+08:00"), Some(1_793_783_523));
        assert_eq!(timestamp("2026-11-04T09:12:03.123Z"), Some(1_793_783_523));
        for bad in ["", "2026-11-04", "2026-13-04T00:00:00Z", "2026-11-04T09:12:03", "2026-11-04T09:12:03+0800"] {
            assert_eq!(timestamp(bad), None, "{bad}");
        }
        assert_eq!(ci_duration_text(55), "55s");
        assert_eq!(ci_duration_text(281), "4m 41s");
        assert_eq!(ci_duration_text(3720), "1h 02m");
    }
    #[test]
    fn links_keep_only_plain_https_and_name_other_sites() {
        let c = ForgeConfig::new(ForgeProvider::GitHub, "https://github.com").unwrap();
        assert_eq!(link(Some("https://github.com/team/sample/actions/runs/1/job/2"), &c).unwrap().external, None);
        assert_eq!(link(Some("https://app.netlify.com/sites/x"), &c).unwrap().external.as_deref(), Some("app.netlify.com"));
        for bad in ["http://github.com/x", "https://user:pw@github.com/x", "https://github.com/a b", "javascript:alert(1)", "https://exa\"mple.com/"] {
            assert!(link(Some(bad), &c).is_none(), "{bad}");
        }
    }
    #[test]
    fn rollup_and_permission_names() {
        use ForgeCheckState::*;
        assert_eq!(ci_rollup([Passed, Skipped].into_iter()), ForgeCiState::Passed);
        assert_eq!(ci_rollup([Passed, Running].into_iter()), ForgeCiState::Running);
        assert_eq!(ci_rollup([Running, Failed].into_iter()), ForgeCiState::Failed);
        assert_eq!(ci_rollup([Cancelled].into_iter()), ForgeCiState::Failed);
        assert_eq!(ci_rollup([].into_iter()), ForgeCiState::NotChecked);
        assert_eq!(permission_name("actions=write").as_deref(), Some("Actions: Read and write"));
        assert_eq!(permission_name("checks=read; contents=read").as_deref(), Some("Checks: Read"));
        assert_eq!(permission_name("statuses=read, metadata=read").as_deref(), Some("Commit statuses: Read"));
        assert_eq!(permission_name("unknown=admin"), None);
    }
}
