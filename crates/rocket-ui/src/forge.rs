//! Hosting is a background page; global task ownership keeps a submitted request's
//! result available even if navigation closes the page. No Git checkout changes.
use crate::i18n::{tr, trf, trn};
use crate::{
    app::*,
    backend::user_error,
    hosting_accounts::{self, Account, Loaded},
    pages::{self, control},
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use std::{collections::HashMap, sync::Arc};
pub mod publish;
pub mod search;
pub mod pull_review;
pub mod drafts;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Section {
    #[default]
    Pulls,
    PullDetail,
    Issues,
    Create,
    Publish,
    Accounts,
    Drafts,
}
enum Done {
    Connected(Arc<ForgeSession>, Option<Loaded>),
    Page(ForgePage, bool, Option<search::Recent>),
    Review(ForgePreview),
    Created(ForgeItem),
    Accounts(Loaded),
    PublishReview(GitHubPublishPreview),
    Published(Box<GitHubPublishReport>),
    PullDetails(Box<ForgePullDetails>),
    PullReview(ForgePullReviewPreview),
    PullSubmitted(ForgePullReview),
    PullMergePreview(ForgePullMergePreview),
    PullMerged(ForgePullMergeResult),
    PullThreads(ForgePullThreads),
    PullThreadPreview(ForgeThreadPreview),
    PullThreadSubmitted,
}
pub struct State {
    pub drafts: drafts::State,
    pub pull: pull_review::State,
    pub publish: publish::State,
    pub enabled: bool,
    pub fixture: Option<u16>,
    pub account_check_only: bool,
    pub publish_check: Option<String>,
    pub section: Section,
    pub provider: ForgeProvider,
    pub server: String,
    pub azure_api_version: String,
    pub username: String,
    pub label: String,
    pub token: String,
    pub remember: bool,
    pub accounts: Vec<Account>,
    pub selected_account: Option<usize>,
    pub session: Option<Arc<ForgeSession>>,
    pub repository: String,
    pub source: String,
    pub from_fork: bool,
    pub source_repository: String,
    pub target: String,
    pub title: String,
    pub body: String,
    pub state: ForgeState,
    pub work_state: String,
    pub filter: String,
    pub search: ForgeSearch,
    pub searches: Vec<search::Recent>,
    pub total: Option<u64>,
    pub incomplete: bool,
    pub items: Vec<ForgeItem>,
    pub next: Option<ForgeCursor>,
    pub loaded: bool,
    pub preview: Option<Arc<ForgePreview>>,
    pub created: Option<ForgeItem>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub stopping: bool,
    pub uncertain: bool,
    uncertain_repo: Option<ForgeRepository>,
    pub operation: &'static str,
    /// The operation the shown error came from, for its title.
    failed: &'static str,
    pub accounts_loaded: bool,
    snapshot: Option<Vec<u8>>,
    task: Option<WriteTask<Done>>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            drafts: drafts::State::default(),
            pull: pull_review::State::default(),
            publish: publish::State::default(),
            enabled: true,
            fixture: None,
            account_check_only: false,
            publish_check: None,
            section: Section::Pulls,
            provider: ForgeProvider::GitHub,
            server: ForgeProvider::GitHub.default_server().into(),
            azure_api_version: "7.0".into(),
            username: String::new(),
            label: "Personal".into(),
            token: String::new(),
            remember: false,
            accounts: Vec::new(),
            selected_account: None,
            session: None,
            repository: String::new(),
            source: String::new(),
            from_fork: false,
            source_repository: String::new(),
            target: "main".into(),
            title: String::new(),
            body: String::new(),
            state: ForgeState::Open,
            work_state: String::new(),
            filter: String::new(),
            search: ForgeSearch::default(),
            searches: Vec::new(),
            total: None,
            incomplete: false,
            items: Vec::new(),
            next: None,
            loaded: false,
            preview: None,
            created: None,
            error: None,
            notice: None,
            stopping: false,
            uncertain: false,
            uncertain_repo: None,
            operation: "",
            failed: "",
            accounts_loaded: false,
            snapshot: None,
            task: None,
        }
    }
}
impl State {
    pub fn running(&self) -> bool {
        self.task.is_some() || self.drafts.pending.is_some()
    }
    pub fn cancel(&mut self) {
        self.drafts.cancel_pending();
        if let Some(t) = &self.task {
            t.cancel();
            self.stopping = true;
            self.notice = Some(
                if matches!(self.operation, "create" | "publish" | "pull-submit" | "pull-merge-submit" | "pull-thread-submit") {
                    tr("Stopping… The server may already have received the request.")
                } else {
                    tr("Stopping…")
                }
                .into(),
            );
        }
    }
    pub fn editing(&self) -> bool {
        self.running()
            || self.pull.has_draft()
            || self.pull.merge.preview.is_some()
            || self.pull.preview.is_some()
            || self.preview.is_some()
            || matches!(self.section, Section::Create | Section::Publish | Section::Accounts)
    }
    fn repo(&self) -> rocket_git::Result<ForgeRepository> {
        let session = self.session.as_ref().ok_or_else(|| {
            rocket_git::Error::InvalidInput(tr("Connect a hosting account first.").into())
        })?;
        ForgeRepository::new(session.config().clone(), &self.repository)
    }
    pub fn visible(&self) -> Vec<usize> {
        let f = self.filter.to_lowercase();
        self.items
            .iter()
            .enumerate()
            .filter(|(_, i)| {
                format!("{} {} {} {}", i.number, i.title, i.author, i.state)
                    .to_lowercase()
                    .contains(&f)
            })
            .map(|(i, _)| i)
            .collect()
    }
    fn can_acknowledge(&self) -> bool {
        self.uncertain
            && self.loaded
            && self.section == Section::Pulls
            && self.repo().ok().as_ref() == self.uncertain_repo.as_ref()
    }
    fn clear_results(&mut self) {
        self.total = None;
        self.incomplete = false;
        self.publish.preview = None;
        self.items.clear();
        self.next = None;
        self.loaded = false;
        self.preview = None;
        self.created = None;
    }
}
impl Rocket {
    pub(crate) fn start_forge_page(&mut self) {
        // Preferences → Accounts reads the saved accounts too.
        if self.preferences_page() == Some(crate::preferences::Page::Accounts) {
            if !self.forge.accounts_loaded && !self.forge.running() {
                self.forge_action("load");
            }
            return;
        }
        if !self
            .dialog
            .as_ref()
            .is_some_and(|d| matches!(d.kind, DialogKind::Hosting))
        {
            return;
        }
        if self.forge.source.is_empty()
            && let Some(name) = self
                .tab()
                .and_then(|t| t.snapshot.as_ref())
                .and_then(|s| s.head.as_ref())
                .and_then(|h| match h {
                    Head::Branch { name, .. } => String::from_utf8(name.clone()).ok(),
                    _ => None,
                })
        {
            self.forge.source = name;
        }
        if self.forge.repository.is_empty() {
            let remote = self
                .tab()
                .and_then(|t| t.snapshot.as_ref())
                .and_then(|s| s.remotes.first())
                .and_then(|r| r.urls.first())
                .and_then(|u| GitHubRemote::parse(u));
            if let Some(remote) = remote {
                self.forge.repository = remote.full_name();
            }
        }
        if !self.forge.accounts_loaded && !self.forge.running() {
            self.forge_action("load");
        }
    }
    pub(crate) fn poll_forge(&mut self) {
        let Some(result) = self.forge.task.as_mut().and_then(WriteTask::try_finish) else {
            return;
        };
        let published_root = match &result {
            Ok(Done::Published(report)) if report.remote_added => self.forge.publish.root.clone(),
            _ => None,
        };
        let detected = if let Ok(Done::Connected(session, _)) = &result {
            self.tab()
                .and_then(|t| t.snapshot.as_ref())
                .and_then(|snapshot| {
                    snapshot
                        .remotes
                        .iter()
                        .flat_map(|r| r.urls.iter())
                        .find_map(|url| ForgeRepository::from_remote(session.config().clone(), url))
                })
                .map(|repo| repo.path().to_owned())
        } else {
            None
        };
        let s = &mut self.forge;
        s.task = None;
        let stopped = s.stopping;
        s.stopping = false;
        let op = s.operation;
        s.operation = "";
        if matches!(op, "pull-submit" | "pull-merge-submit" | "pull-thread-submit") { s.drafts.write_ahead = None; }
        match result {
            Ok(Done::PullThreads(threads)) if !stopped => {
                s.pull.threads.checked=s.pull.threads.unknown&&s.pull.repository.as_ref()==Some(threads.repository())&&s.pull.number==threads.number();
                s.pull.threads.snapshot=Some(Arc::new(threads));s.notice=Some(tr("Discussion threads refreshed.").into());
            }
            Ok(Done::PullThreadPreview(preview)) if !stopped => {s.pull.threads.preview=Some(Arc::new(preview));}
            Ok(Done::PullThreadSubmitted) => {s.pull.threads.body.clear();s.pull.threads.action=None;s.pull.threads.preview=None;s.pull.threads.unknown=false;s.notice=Some(tr("Thread action confirmed by the server. Refresh threads to view it.").into());}
            Ok(Done::PullDetails(details)) if !stopped => {
                s.pull.checked_unknown = s.pull.unknown.as_ref().is_some_and(|(repo, n)| repo == details.repository() && *n == details.summary().number) && details.reviews.error.is_none() && !details.reviews.limited;
                s.pull.details = Some(Arc::new(*details));
                s.pull.merge.reconcile(s.pull.details.as_ref().unwrap());
                drafts::rebind(s);
                s.pull.file = 0;
                s.notice = Some(tr("PR details refreshed.").into());
                // The Checks tab reads the refreshed head again.
                s.pull.ci_for = None;
            }
            Ok(Done::PullReview(preview)) if !stopped => { s.pull.preview = Some(Arc::new(preview)); s.notice = None; }
            Ok(Done::PullMergePreview(preview)) if !stopped => { s.pull.merge.preview = Some(Arc::new(preview)); s.notice = None; }
            Ok(Done::PullMerged(result)) => {
                s.notice = Some(trf!("The provider confirmed the merge at {commit}. Local files and branches are unchanged; refresh the PR.", commit = result.commit));
                s.pull.merge.result = Some(result.commit); s.pull.merge.preview = None;
                s.pull.merge.submission = None; s.pull.merge.unknown = None;
            }
            Ok(Done::PullSubmitted(review)) => {
                let reference = if review.id == 0 { tr("Review action").into() } else { trf!("Review #{number}", number = review.id) };
                let revision = review.commit.as_ref().map(|id| trf!(" on commit {id}", id)).unwrap_or_default();
                s.notice = Some(trf!("{reference} confirmed by the provider{revision}. Your local repository is unchanged.", reference, revision));
                s.pull.submitted = Some(review);
                s.pull.preview = None;
                s.pull.body.clear();
                s.pull.comments.clear(); s.pull.inline_body.clear(); s.pull.inline_selection = None; s.pull.inline_editing = None;
                s.pull.composing = false;
                s.pull.submission = None;
            }
            Ok(Done::PublishReview(preview)) if !stopped => {
                s.publish.preview = Some(Arc::new(preview));
                s.notice = None;
            }
            Ok(Done::Published(report)) => {
                s.publish.preview = None;
                s.publish.report = Some(*report);
                s.notice = None;
            }
            Ok(Done::Created(item)) => {
                s.notice = Some(trf!("Created pull request #{number}. Your local repository is unchanged.", number = item.number));
                s.created = Some(item);
                s.preview = None;
                s.uncertain = false;
                s.uncertain_repo = None;
            }
            Ok(Done::Accounts(loaded)) => {
                s.accounts = loaded.accounts;
                s.snapshot = loaded.snapshot;
                s.accounts_loaded = true;
                s.notice =
                    Some(tr("Saved accounts loaded. Select one to connect, or add an account.").into());
            }
            Ok(Done::Connected(session, loaded)) => {
                if let Some(loaded) = loaded {
                    s.accounts = loaded.accounts;
                    s.snapshot = loaded.snapshot;
                    s.accounts_loaded = true;
                }
                s.token.clear();
                if stopped {
                    s.notice =
                        Some(tr("Connection stopped. Any completed account save is retained.").into());
                } else {
                    let session_note = if session.temporary_browser_session() { tr(" Temporary Bitbucket browser session; sign in again when it expires. No browser token was saved. PR permissions depend on the consent grant; use an API token if access is denied.") } else { "" };
                    let server_note = if session.config().provider() == ForgeProvider::GitHubEnterprise {
                        trf!(" GitHub Enterprise Server: {version}.", version = session.enterprise_version().unwrap_or(tr("version not reported")))
                    } else if session.config().provider() == ForgeProvider::AzureServer {
                        trf!(" Selected Azure REST API {version}.", version = session.config().azure_api_version())
                    } else { String::new() };
                    s.notice = Some(trf!("Connected as {login}. Git and SSH credentials are unchanged.{session_note}{server_note}", login = session.login(), session_note, server_note));
                    if let Some(repository) = detected {
                        s.repository = repository;
                    }
                    s.session = Some(session);
                    s.section = Section::Pulls;
                    s.clear_results();
                }
            }
            Ok(Done::Page(page, append, recent)) if !stopped => {
                s.total = page.total;
                s.incomplete = page.incomplete;
                if let Some(recent) = recent { search::remember(s, recent); }
                if !append {
                    s.items.clear();
                }
                for item in page.items {
                    if let Some(i) = s.items.iter().position(|i| i.number == item.number) {
                        s.items[i] = item;
                    } else {
                        s.items.push(item);
                    }
                }
                s.next = page.next;
                s.loaded = true;
                s.notice=Some(if s.uncertain{tr("List refreshed. Inspect the provider before deciding whether another create request is necessary.")}else{tr("Hosting results refreshed.")}.into());
            }
            Ok(Done::Review(preview)) if !stopped => {
                s.preview = Some(Arc::new(preview));
                s.notice = None;
            }
            Ok(_) | Err(rocket_git::Error::Cancelled) => {
                if op=="pull-thread-submit"{s.pull.threads.preview=None;}
                if op == "pull-submit" { s.pull.preview = None; s.pull.submission = None; }
                if op == "pull-merge-submit" { s.pull.merge.preview = None; s.pull.merge.submission = None; }
                s.notice = Some(tr("Cancelled. Existing results are retained.").into());
            }
            Err(error) => {
                if op=="pull-thread-submit"{s.pull.threads.preview=None;s.pull.threads.unknown=matches!(error,Error::Forge(ForgeFailure::PullReviewUnknown));s.pull.threads.checked=false;}
                if op == "pull-merge-submit" {
                    if matches!(error,rocket_git::Error::Forge(ForgeFailure::PullMergeUnknown)) {
                        s.pull.merge.unknown = s.pull.merge.submission.take(); s.pull.merge.checked_unknown = false;
                    } else { s.pull.merge.submission = None; }
                    s.pull.merge.preview = None;
                }
                if op == "pull-submit" {
                    if matches!(error, rocket_git::Error::Forge(ForgeFailure::PullReviewUnknown)) {
                        s.pull.unknown = s.pull.submission.take();
                        s.pull.checked_unknown = false;
                    } else {
                        s.pull.submission = None;
                    }
                    s.pull.preview = None;
                }
                s.uncertain |= matches!(
                    error,
                    rocket_git::Error::Forge(ForgeFailure::OutcomeUnknown)
                );
                if matches!(
                    error,
                    rocket_git::Error::Forge(ForgeFailure::OutcomeUnknown)
                ) {
                    s.uncertain_repo = s.preview.as_ref().map(|p| p.draft().repository.clone());
                }
                if op == "publish" { s.publish.preview = None; }
                if op == "create" {
                    s.preview = None;
                }
                if matches!(error, rocket_git::Error::Forge(ForgeFailure::Unauthorized | ForgeFailure::IdentityUnverified | ForgeFailure::AzureIdentityUnverified)) {
                    s.session = None;
                    s.section = Section::Accounts;
                    s.token.clear();
                }
                s.error = Some(user_error(&error));
                s.failed = op;
                s.notice = None;
            }
        }
        if let Some(root) = published_root {
            let ids: Vec<_> = self.tabs.iter_mut().filter(|tab| tab.path == root).map(|tab| {
                // Drop any pre-publication read so it cannot replace the fresh
                // remote/upstream snapshot. Hosting pauses background refresh.
                tab.refresh = None;
                tab.id
            }).collect();
            for id in ids { self.refresh(id); }
        }
    }
    fn forge_action(&mut self, action: &str) {
        if action.starts_with("pull-") { self.start_pull_action(action); return; }
        if action.starts_with("publish-") { self.start_github_publish(action); return; }
        if self.forge.running() {
            return;
        }
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let directory = self.state_directory.clone();
        let s = &mut self.forge;
        s.error = None;
        s.failed = "";
        s.notice = None;
        if !s.enabled {
            s.error = Some(tr("Hosting access is disabled in this window.").into());
            return;
        }
        if s.account_check_only && (!matches!(action, "load" | "browser") || s.remember) {
            s.error = Some(tr("This live acceptance window only verifies browser sign-in with Remember disabled. Hosting writes and saved credentials are disabled.").into());
            return;
        }
        let task = (|| -> rocket_git::Result<WriteTask<Done>> {
            match action {
                "load" => {
                    s.operation = "accounts";
                    Ok(WriteTask::spawn(move |_| {
                        backend.log.request("hosting-accounts", |_| {
                            hosting_accounts::load(&directory).map(Done::Accounts)
                        })
                    })?)
                }
                "forget" => {
                    let Some(index) = s.selected_account else {
                        return Err(rocket_git::Error::InvalidInput(
                            tr("Select a saved account first.").into(),
                        ));
                    };
                    let mut accounts = s.accounts.clone();
                    if index >= accounts.len() {
                        return Err(rocket_git::Error::InvalidInput(
                            tr("Saved account selection changed.").into(),
                        ));
                    }
                    accounts.remove(index);
                    let expected = s.snapshot.clone();
                    s.selected_account = None;
                    s.token.clear();
                    s.operation = "accounts";
                    Ok(WriteTask::spawn(move |_| {
                        backend.log.request("hosting-accounts", |_| {
                            hosting_accounts::save(&directory, accounts, expected)
                                .map(Done::Accounts)
                        })
                    })?)
                }
                "connect" | "browser" => {
                    let browser = action == "browser";
                    let mut config = ForgeConfig::new(s.provider, &s.server)?;
                    if s.provider == ForgeProvider::AzureServer { config = config.with_azure_api_version(&s.azure_api_version)?; }
                    if let Some(port) = s.fixture {
                        config = config.with_fixture(port)?;
                    }
                    if s.account_check_only {
                        config = config.with_github_account_check()?;
                    }
                    if let Some(repository) = &s.publish_check {
                        config = config.with_github_publish_check(repository)?;
                    }
                    if s.label.trim().is_empty()
                        || s.label.len() > 128
                        || s.label.chars().any(char::is_control)
                    {
                        return Err(rocket_git::Error::InvalidInput(
                            tr("Give this account a name up to 128 bytes.").into(),
                        ));
                    }
                    let mut account = Account {
                        label: s.label.clone(),
                        provider: s.provider,
                        server: config.server().into(),
                        username: s.username.clone(),
                        token: s.token.clone(),
                        azure_api_version: (s.provider == ForgeProvider::AzureServer).then(|| s.azure_api_version.clone()),
                    };
                    // GCM does not return refresh tokens through Git's credential
                    // protocol. Do not save a one-hour Cloud token as a durable account.
                    let remember = s.remember && !(browser && s.provider == ForgeProvider::Bitbucket);
                    if remember && !s.accounts_loaded {
                        return Err(rocket_git::Error::InvalidInput(
                            tr("Load saved accounts successfully before changing them.").into(),
                        ));
                    }
                    let mut accounts = s.accounts.clone();
                    let expected = s.snapshot.clone();
                    if remember {
                        if accounts
                            .iter()
                            .enumerate()
                            .any(|(i, a)| Some(i) != s.selected_account && a.label == account.label)
                        {
                            return Err(rocket_git::Error::InvalidInput(tr("That saved account name is already in use. Select it to edit, or use a new name.").into()));
                        }
                        if let Some(index) = s.selected_account.filter(|i| *i < accounts.len()) {
                            accounts[index] = account.clone();
                        } else {
                            accounts.push(account.clone());
                        }
                    }
                    s.operation = "connect";
                    if browser { s.notice=Some(tr("Waiting for browser sign-in through Git Credential Manager. Existing Git credentials are not read or replaced.").into()); }
                    Ok(WriteTask::spawn(move |cancel| {
                        let session = if browser {
                            let (session, token) = backend.forge_browser_connect(config, &cancel)?;
                            account.token=token;
                            account.username.clear();
                            for saved in &mut accounts { if saved.label==account.label {saved.token=account.token.clone();saved.username.clear();} }
                            session
                        } else { backend.forge_connect(
                            config,
                            &account.username,
                            &account.token,
                            &cancel,
                        )? };
                        cancel.check()?;
                        let loaded = if remember {
                            Some(backend.log.request("hosting-accounts", |_| {
                                hosting_accounts::save(&directory, accounts, expected)
                            })?)
                        } else {
                            None
                        };
                        Ok(Done::Connected(Arc::new(session), loaded))
                    })?)
                }
                "refresh" | "more" => {
                    let repo = s.repo()?;
                    let session = s.session.clone().expect("repo verifies session");
                    let filter = ForgeFilter {
                        kind: if s.section == Section::Issues {
                            ForgeKind::Issues
                        } else {
                            ForgeKind::Pulls
                        },
                        state: s.state,
                        work_item_state: s.work_state.clone(),
                        search: if s.provider.supports_search() { s.search.clone() } else { ForgeSearch::default() },
                    };
                    let recent = s.provider.is_github().then(|| search::Recent::from_state(s));
                    let append = action == "more";
                    let cursor = if append {
                        s.next.clone().ok_or_else(|| {
                            rocket_git::Error::InvalidInput(tr("There are no more pages.").into())
                        })?
                    } else {
                        ForgeCursor::default()
                    };
                    s.operation = "list";
                    s.preview = None;
                    Ok(WriteTask::spawn(move |cancel| {
                        backend
                            .forge_request("hosting-list", &session, |session| {
                                session.list(&repo, &filter, cursor, &cancel)
                            })
                            .map(|page| Done::Page(page, append, recent))
                    })?)
                }
                "review" => {
                    if s.uncertain {
                        return Err(rocket_git::Error::InvalidInput(tr("Check the previous request's remote outcome before starting another review.").into()));
                    }
                    let draft = ForgeDraft {
                        repository: s.repo()?,
                        source: s.source.clone(),
                        target: s.target.clone(),
                        title: s.title.clone(),
                        body: s.body.clone(),
                    };
                    let session = s.session.clone().expect("repo verifies session");
                    let source_repo = if s.from_fork && session.config().provider().is_github() { Some(ForgeRepository::new(session.config().clone(), &s.source_repository)?) } else { None };
                    s.operation = "review";
                    // A previous successful PR is not the result of this new attempt.
                    s.created = None;
                    Ok(WriteTask::spawn(move |cancel| {
                        backend
                            .forge_request("hosting-preview", &session, |session| {
                                if let Some(source) = source_repo { session.preview_from_fork(draft, source, &cancel) } else { session.preview(draft, &cancel) }
                            })
                            .map(Done::Review)
                    })?)
                }
                "create" => {
                    let preview = s.preview.clone().ok_or_else(|| {
                        rocket_git::Error::InvalidInput(tr("Review this pull request first.").into())
                    })?;
                    let session = s.session.clone().ok_or_else(|| {
                        rocket_git::Error::InvalidInput(tr("Connect an account first.").into())
                    })?;
                    s.operation = "create";
                    Ok(WriteTask::spawn(move |cancel| {
                        backend
                            .forge_request("hosting-create", &session, |session| {
                                session.create(&preview, &cancel)
                            })
                            .map(Done::Created)
                    })?)
                }
                _ => Err(rocket_git::Error::InvalidInput(
                    tr("Unknown hosting action.").into(),
                )),
            }
        })();
        match task {
            Ok(task) => s.task = Some(task),
            Err(error) => {
                s.error = Some(user_error(&error));
                s.failed = "";
                s.operation = "";
            }
        }
    }
    /// Branch names New pull request offers: local branches to open from, and every
    /// branch name the repository knows, without its remote, to open into.
    fn forge_branch_choices(&self) -> (Vec<String>, Vec<String>) {
        let Some(snapshot) = self.tab().and_then(|t| t.snapshot.as_ref()) else {
            return Default::default();
        };
        let (mut sources, mut targets) = (Vec::new(), Vec::<String>::new());
        for branch in &snapshot.branches {
            let name = short_ref(&branch.reference);
            let target = match branch.kind {
                BranchKind::Local => {
                    sources.push(name.clone());
                    name
                }
                // origin/HEAD names no branch of its own.
                BranchKind::Remote if branch.symbolic_target.is_none() => {
                    match name.split_once('/') {
                        Some((_, rest)) => rest.to_owned(),
                        None => continue,
                    }
                }
                BranchKind::Remote => continue,
            };
            if !targets.contains(&target) {
                targets.push(target);
            }
        }
        (sources, targets)
    }
    /// Hosting: provider, server and sign-in state in the header; tabs with counts; the
    /// list, the new pull request form or the accounts below. Problems say what happened
    /// and what to do, with Rocket's code as small print.
    pub(crate) fn forge_page(&mut self, ui: &mut Ui) {
        // The tab whose Actions lists this pull request's repository, for All runs.
        self.forge.pull.actions_tab = self.forge.pull.details.as_ref().and_then(|d| {
            let pull = d.repository();
            self.tabs
                .iter()
                .find(|t| t.ci.repository.as_ref().is_some_and(|r| r.path().eq_ignore_ascii_case(pull.path()) && r.config().server() == pull.config().server()))
                .map(|t| t.id)
        });
        let (sources, targets) = self.forge_branch_choices();
        let writable = self.can_write();
        let compact = ui.available_width() < 700.0;
        self.prepare_publish_form();
        let s = &mut self.forge;
        let widgets = &mut self.frame_widgets;
        let busy = s.running();
        let mut out = PageOut::default();
        let mut close = false;
        // Accounts are in Preferences. A signed-out session that asked for them (a
        // rejected token) shows the lists' sign-in prompt and keeps its message.
        if s.section == Section::Accounts {
            s.section = Section::Pulls;
        }
        let mut start = None;
        hosting_header(ui, s, widgets, busy, compact, &mut close, &mut out.accounts, &mut start);
        if let Some(section) = start {
            show_section(s, section);
        }
        let azure = s.provider.is_azure();
        let count = |section: Section| (s.section == section && s.loaded).then_some(s.items.len());
        let selected = pages::tabs(
            ui,
            &[
                pages::TabItem {
                    label: if compact { tr("Pulls") } else { tr("Pull requests") },
                    count: count(Section::Pulls),
                    selected: matches!(s.section, Section::Pulls | Section::PullDetail),
                    enabled: !busy,
                    key: "hosting-tab-pulls",
                },
                pages::TabItem {
                    label: if azure { tr("Work items") } else { tr("Issues") },
                    count: count(Section::Issues),
                    selected: s.section == Section::Issues,
                    enabled: !busy,
                    key: "hosting-tab-issues",
                },
                pages::TabItem { label: tr("Saved drafts"), count: Some(s.drafts.records.len()), selected: s.section==Section::Drafts, enabled: !busy, key: "hosting-tab-drafts" },
            ],
            widgets,
        );
        if let Some(index) = selected {
            show_section(s, [Section::Pulls, Section::Issues, Section::Drafts][index]);
        }
        let output = ScrollArea::vertical()
            .id_salt("hosting-content")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                Frame::new()
                    .inner_margin(Margin { left: 24, right: 24, top: 16, bottom: 24 })
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 12.0;
                        if !s.enabled {
                            banner(
                                ui,
                                widgets,
                                RED(),
                                tr("Hosting is disabled"),
                                tr("This MCP window requires an explicit loopback API fixture."),
                                None,
                            );
                        }
                        match s.section {
                            Section::PullDetail => pull_review::section(ui, s, widgets, busy, &mut out),
                            // Accounts never shows here; see the top of this function.
                            Section::Pulls | Section::Issues | Section::Accounts => list_section(ui, s, widgets, busy, &mut out),
                            Section::Create => create_section(ui, s, widgets, busy, (sources.as_slice(), targets.as_slice()), &mut out),
                            Section::Publish => publish::section(ui, s, widgets, busy, writable, &sources, &mut out),
                            Section::Drafts => drafts::panel(ui,s,widgets,busy,&mut out),
                        }
                    });
            });
        pages::region(
            widgets,
            "hosting-scroll",
            output.inner_rect.intersect(ui.clip_rect()),
            true,
        );
        if out.changed {
            s.clear_results();
        }
        if let Some(d) = &mut self.dialog {
            d.flag = s.editing();
        }
        if close {
            self.dialog = None;
        }
        if let Some(url) = out.open {
            if let Some(list) = &mut self.opened_urls {
                list.push(url);
            } else {
                ui.ctx().open_url(eframe::egui::OpenUrl::new_tab(url));
            }
        }
        if out.ci_rerun
            && let (Some(ci), Some(session)) = (self.forge.pull.ci.clone(), self.forge.session.clone())
        {
            self.open_ci_rerun(None, crate::ci_checks::RerunSource::Commit(ci), session);
        }
        if let Some(branch) = out.ci_runs
            && let Some(index) = self.forge.pull.actions_tab.and_then(|id| self.tabs.iter().position(|t| t.id == id))
        {
            self.active = index;
            self.open_actions(Some(branch), None);
            return;
        }
        if let Some(a) = out.action {
            self.forge_action(a);
        }
        if let Some(number) = out.pull {
            self.open_pull_details(number);
        }
        if out.accounts && !close {
            self.show_accounts_from_hosting();
        }
    }

    /// Preferences → Accounts: the saved accounts, then the selected or new account's
    /// form. Connecting, saving and forgetting work as they did on the Hosting page.
    pub(crate) fn accounts_preferences(&mut self, ui: &mut Ui) {
        let s = &mut self.forge;
        let widgets = &mut self.frame_widgets;
        let busy = s.running();
        let mut out = PageOut::default();
        ui.spacing_mut().item_spacing.y = 12.0;
        if !s.enabled {
            banner(
                ui,
                widgets,
                RED(),
                tr("Hosting is disabled"),
                tr("This MCP window requires an explicit loopback API fixture."),
                None,
            );
        }
        if let Some(error) = s.error.clone() {
            banner(ui, widgets, RED(), &failure_title(s.failed, &error, ""), &error, None);
        }
        if let Some(n) = &s.notice {
            notice_line(ui, n);
        }
        accounts_body(ui, s, widgets, busy, &mut out);
        if out.changed {
            s.clear_results();
        }
        if let Some(url) = out.open {
            if let Some(list) = &mut self.opened_urls {
                list.push(url);
            } else {
                ui.ctx().open_url(eframe::egui::OpenUrl::new_tab(url));
            }
        }
        if let Some(a) = out.action {
            self.forge_action(a);
        }
    }
}

/// What the page does once it is drawn.
#[derive(Default)]
struct PageOut {
    pull: Option<u64>,
    action: Option<&'static str>,
    open: Option<String>,
    /// The repository or a filter the server applies changed: loaded results are stale.
    changed: bool,
    /// Accounts was chosen: Preferences opens there and returns to Hosting.
    accounts: bool,
    /// Re-run failed jobs was chosen on the Checks tab: its review opens.
    ci_rerun: bool,
    /// All runs for this branch was chosen on the Checks tab: Actions opens.
    ci_runs: Option<String>,
}

pub(crate) fn show_section(s: &mut State, next: Section) {
    if next != s.section {
        s.section = next;
        s.preview = None;
        s.error = None;
        s.notice = None;
        if matches!(next, Section::Pulls | Section::Issues) {
            s.clear_results();
        }
    }
}

/// A server address as people say it: no scheme, no trailing slash.
fn host(server: &str) -> &str {
    let server = server.trim();
    server
        .strip_prefix("https://")
        .or_else(|| server.strip_prefix("http://"))
        .unwrap_or(server)
        .trim_end_matches('/')
}

/// The page title, the provider and server, whether you are signed in, then what you
/// can start here (New pull request, Publish repository) and Close.
#[allow(clippy::too_many_arguments)]
fn hosting_header(
    ui: &mut Ui,
    s: &mut State,
    widgets: &mut HashMap<String, UiControl>,
    busy: bool,
    compact: bool,
    close: &mut bool,
    accounts: &mut bool,
    start: &mut Option<Section>,
) {
    let (provider, server, login) = match &s.session {
        Some(session) => (
            session.config().provider(),
            session.config().server().to_owned(),
            Some(session.login().to_owned()),
        ),
        None => (s.provider, s.server.clone(), None),
    };
    Frame::new()
        .inner_margin(Margin { left: 24, right: 20, top: 16, bottom: 12 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    *close = btn(ui, widgets, "dialog-cancel", tr("Close"), !busy);
                    // These open their forms below; the old tab names still reach them.
                    let r = pages::primary(ui, !busy && s.session.is_some(), if compact { tr("New PR") } else { tr("New pull request") });
                    control(widgets, "hosting-tab-create", &r, ui.clip_rect());
                    if r.clicked() {
                        *start = Some(Section::Create);
                    }
                    let publish = !busy
                        && s.session.as_ref().is_some_and(|x| x.config().provider() == ForgeProvider::GitHub)
                        && !s.account_check_only;
                    if btn(ui, widgets, "hosting-tab-publish", if compact { tr("Publish") } else { tr("Publish repository") }, publish) {
                        *start = Some(Section::Publish);
                    }
                    if busy {
                        let label = if s.stopping { tr("Stopping") } else { tr("Stop") };
                        if btn(ui, widgets, "hosting-cancel", label, !s.stopping) {
                            s.cancel();
                        }
                        ui.spinner();
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let (tile, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
                        ui.painter().rect_filled(tile, 9.0, ACCENT_SOFT());
                        icon(ui.painter(), tile.shrink(10.0), Icon::Remote, BLUE());
                        ui.add_space(4.0);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 3.0;
                            ui.label(RichText::new(tr("Hosting")).font(semibold(18.0)));
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                tag(ui, provider.label());
                                ui.add(Label::new(RichText::new(host(&server)).color(MUTED())).truncate());
                                let (dot, text) = match &login {
                                    Some(login) => (GREEN(), trf!("Signed in as {login}", login)),
                                    None => (FAINT(), tr("Not signed in").to_owned()),
                                };
                                let (rect, _) = ui.allocate_exact_size(vec2(7.0, 7.0), Sense::hover());
                                ui.painter().circle_filled(rect.center(), 3.5, dot);
                                ui.add(Label::new(RichText::new(text).color(MUTED())).truncate());
                                // Accounts live in Preferences; the old tab's name still reaches them.
                                let r = ui.add_enabled(!busy, Button::new(RichText::new(tr("Accounts")).color(BLUE())).frame(false));
                                control(widgets, "hosting-accounts", &r, ui.clip_rect());
                                control(widgets, "hosting-tab-accounts", &r, ui.clip_rect());
                                *accounts |= r.on_hover_text(tr("Preferences → Accounts")).clicked();
                            });
                        });
                    });
                });
            });
        });
}

/// A small label on a raised fill, such as the provider's name.
fn tag(ui: &mut Ui, text: &str) {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), FontId::proportional(12.0), MUTED());
    let (rect, _) = ui.allocate_exact_size(galley.size() + vec2(14.0, 4.0), Sense::hover());
    ui.painter().rect_filled(rect, 4.0, CHIP());
    ui.painter().galley(rect.min + vec2(7.0, 2.0), galley, MUTED());
}

/// Pull requests or issues: the repository, state and filter in one row, then the list.
fn list_section(
    ui: &mut Ui,
    s: &mut State,
    widgets: &mut HashMap<String, UiControl>,
    busy: bool,
    out: &mut PageOut,
) {
    search::controls(ui, s, widgets, busy, out);
    let azure = s.provider.is_azure();
    let issues = s.section == Section::Issues;
    let things = match (issues, azure) {
        (false, _) => tr("pull requests"),
        (true, true) => tr("work items"),
        (true, false) => tr("issues"),
    };
    if matches!(s.provider, ForgeProvider::Bitbucket | ForgeProvider::BitbucketServer) && issues {
        notice_line(
            ui,
            if s.provider == ForgeProvider::BitbucketServer { tr("Bitbucket Server / Data Center has no native issues API. Jira integration is separate. Pull requests remain available.") } else { tr("Bitbucket Cloud retired its native issue tracker in August 2026. Pull requests remain available.") },
        );
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(10.0, 8.0);
        let label = if azure { tr("Project / repository") } else { tr("Owner / repository") };
        out.changed |= icon_field(ui, widgets, "hosting-repository", Icon::Folder, label, &mut s.repository, 230.0, !busy).changed();
        if azure && issues {
            let hint = tr("Exact work-item state (empty for all)");
            out.changed |= icon_field(ui, widgets, "hosting-work-state", Icon::Sliders, hint, &mut s.work_state, 230.0, !busy).changed();
        } else {
            let states = [
                (ForgeState::Open, tr("Open"), "open"),
                (ForgeState::Closed, if issues { tr("Closed") } else { tr("Closed and merged") }, "closed"),
                (ForgeState::All, tr("All"), "all"),
            ];
            let items: Vec<(&str, bool)> = states.iter().map(|(state, label, _)| (*label, s.state == *state)).collect();
            let responses = ui.add_enabled_ui(!busy, |ui| pages::segments(ui, &items)).inner;
            for (r, (state, _, key)) in responses.iter().zip(states) {
                control(widgets, format!("hosting-state-{key}"), r, ui.clip_rect());
                if r.clicked() {
                    s.state = state;
                    out.changed = true;
                }
            }
        }
        // In a wrapping row available_width() is the whole row: measure what is left of it.
        let left = ui.max_rect().right() - ui.cursor().left();
        let width = (left - 40.0).max(180.0);
        icon_field(ui, widgets, "hosting-filter", Icon::Search, &trf!("Filter loaded {things}", things), &mut s.filter, width, true);
        let r = crate::chrome::icon_button(ui, Icon::Refresh, !busy && s.session.is_some(), false, 30.0)
            .on_hover_text(tr("Refresh"));
        control(widgets, "hosting-refresh", &r, ui.clip_rect());
        if r.clicked() {
            out.action = Some("refresh");
        }
    });
    if let Some(error) = s.error.clone() {
        let retry = (s.failed == "list" && s.session.is_some()).then_some(("hosting-error-refresh", tr("Refresh"), !busy));
        if banner(ui, widgets, RED(), &failure_title(s.failed, &error, things), &error, retry) {
            out.action = Some("refresh");
        }
    }
    if let Some(n) = &s.notice {
        notice_line(ui, n);
    }
    if s.uncertain && s.loaded {
        let context = s
            .uncertain_repo
            .as_ref()
            .map(|r| format!("{} / {}", host(r.config().server()), r.path()))
            .unwrap_or_default();
        let ready = !busy && s.can_acknowledge();
        if banner(
            ui,
            widgets,
            AMBER(),
            tr("The previous pull request may have been created"),
            &trf!("Check Pull requests for {context} before creating another request.", context),
            // Disabled until the list shows the repository the request went to.
            Some(("hosting-acknowledge", tr("I checked the remote outcome"), ready)),
        ) {
            s.uncertain = false;
            s.uncertain_repo = None;
            s.notice = Some(tr("You can start a new review if another pull request is needed.").into());
        }
    }
    let repository = match s.repository.trim() {
        "" => tr("this repository").to_owned(),
        name => name.to_owned(),
    };
    if s.session.is_none() {
        Frame::new()
            .stroke(Stroke::new(1.0, WINDOW_EDGE()))
            .corner_radius(10)
            .inner_margin(Margin::symmetric(20, 30))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.vertical_centered(|ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    ui.label(RichText::new(trf!("Sign in to see {things} for {repository}", things, repository)).font(semibold(13.5)));
                    ui.label(
                        RichText::new(trf!("Add a {provider} account in Preferences, or choose a saved one.", provider = s.provider.label()))
                        .color(HINT()),
                    );
                    let r = pages::primary(ui, !busy, tr("Open Accounts"));
                    control(widgets, "hosting-go-accounts", &r, ui.clip_rect());
                    out.accounts |= r.clicked();
                });
            });
        return;
    }
    if !s.loaded {
        let text = if busy {
            trf!("Loading {things} for {repository}", things, repository)
        } else {
            trf!("Refresh to load {things} for {repository}.", things, repository)
        };
        ui.label(RichText::new(text).color(HINT()));
        return;
    }
    list_card(ui, s, widgets, issues, things, busy, out);
}

/// The loaded pull requests or issues, then how many are shown and Load more.
fn list_card(
    ui: &mut Ui,
    s: &State,
    widgets: &mut HashMap<String, UiControl>,
    issues: bool,
    things: &str,
    busy: bool,
    out: &mut PageOut,
) {
    let visible = s.visible();
    Frame::new()
        .stroke(Stroke::new(1.0, CARD_STROKE()))
        .corner_radius(10)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            if visible.is_empty() {
                let text = if s.items.is_empty() {
                    trf!("No {things} here on the loaded pages.", things)
                } else {
                    trf!("No loaded {things} match the filter.", things)
                };
                Frame::new().inner_margin(Margin::symmetric(14, 14)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(text).color(HINT()));
                });
            }
            for i in &visible {
                let details = (!issues && s.session.is_some()).then_some(!busy);
                let (open, show) = item_row(ui, &s.items[*i], issues, widgets, *i, details);
                if open {
                    out.open = Some(s.items[*i].url.clone());
                }
                if show {
                    out.pull = Some(s.items[*i].number);
                }
                let y = ui.cursor().top();
                ui.painter()
                    .hline(ui.max_rect().x_range(), y, Stroke::new(1.0, CARD_LINE()));
            }
            Frame::new()
                .fill(SUNKEN())
                .corner_radius(CornerRadius { nw: 0, ne: 0, sw: 9, se: 9 })
                .inner_margin(Margin::symmetric(14, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if btn(ui, widgets, "hosting-more", tr("Load more"), !busy && s.next.is_some()) {
                                out.action = Some("more");
                            }
                            let shown = if visible.len() < s.items.len() {
                                trf!("Showing {shown} of {loaded} loaded", shown = visible.len(), loaded = s.items.len())
                            } else {
                                trf!("Showing {shown}", shown = s.items.len())
                            };
                            let rest = if s.error.as_deref().is_some_and(|e| e.starts_with("hosting_invalid_pagination")) {
                                tr("the next page could not be loaded")
                            } else if s.next.is_some() {
                                tr("more on the server")
                            } else {
                                tr("all loaded")
                            };
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                ui.add(Label::new(RichText::new(format!("{shown} · {rest}")).size(12.5).color(HINT())).truncate());
                            });
                        });
                    });
                });
        });
}

/// One pull request or issue: its state, title and number, who opened it and its
/// branches, then Details (pull requests, when `details` says whether it is enabled)
/// and Open in browser. Returns whether Open and Details were clicked.
fn item_row(
    ui: &mut Ui,
    item: &ForgeItem,
    issues: bool,
    widgets: &mut HashMap<String, UiControl>,
    index: usize,
    details: Option<bool>,
) -> (bool, bool) {
    let (state, color) = state_look(&item.state);
    let background = ui.painter().add(Shape::Noop);
    let mut open = false;
    let mut show = false;
    let frame = Frame::new().inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            let (mark, _) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
            ui.painter().circle_filled(mark.center(), 13.0, color.gamma_multiply(0.14));
            icon(ui.painter(), mark.shrink(6.5), if issues { Icon::Info } else { Icon::Merge }, color);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let r = crate::chrome::icon_button(ui, Icon::External, true, false, 28.0)
                    .on_hover_text(tr("Open in browser"));
                control(widgets, format!("hosting-item-{index}"), &r, ui.clip_rect());
                open = r.clicked();
                if let Some(enabled) = details {
                    show = btn(ui, widgets, &format!("hosting-details-{index}"), tr("Details"), enabled);
                }
                pages::pill(ui, &state, color);
                ui.with_layout(Layout::top_down(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    let width = ui.available_width();
                    let mut title = text::LayoutJob::default();
                    title.append(&item.title, 0.0, TextFormat::simple(semibold(13.0), TEXT()));
                    title.append(&format!(" #{}", item.number), 0.0, TextFormat::simple(FontId::proportional(13.0), HINT()));
                    title.wrap = text::TextWrapping::truncate_at_width(width);
                    ui.label(title);
                    let mut meta = text::LayoutJob::default();
                    let plain = TextFormat::simple(FontId::proportional(12.0), HINT());
                    let branch = TextFormat {
                        background: CHIP(),
                        ..TextFormat::simple(FontId::monospace(11.5), TEXT_SOFT())
                    };
                    meta.append(if item.author.is_empty() { tr("Unknown author") } else { &item.author }, 0.0, plain.clone());
                    if !item.source.is_empty() {
                        meta.append("  ·  ", 0.0, plain.clone());
                        meta.append(&format!(" {} ", item.source), 0.0, branch.clone());
                        if !item.target.is_empty() {
                            meta.append("  →  ", 0.0, plain.clone());
                            meta.append(&format!(" {} ", item.target), 0.0, branch);
                        }
                    }
                    meta.wrap = text::TextWrapping::truncate_at_width(width);
                    ui.label(meta);
                });
            });
        });
    });
    if ui.rect_contains_pointer(frame.response.rect) {
        ui.painter().set(
            background,
            epaint::RectShape::filled(frame.response.rect, 0, LIST_HOT()),
        );
    }
    (open, show)
}

/// A provider's state word as shown, and its color.
fn state_look(state: &str) -> (String, Color32) {
    let lower = state.trim().to_lowercase();
    let color = match lower.as_str() {
        "open" | "opened" | "active" | "new" => GREEN(),
        "merged" | "completed" | "done" | "resolved" => VIOLET(),
        "closed" | "declined" | "abandoned" | "removed" => RED(),
        _ => BLUE(),
    };
    let translated = match lower.as_str() {
        "open" | "opened" => Some(tr("Open")), "closed" => Some(tr("Closed")),
        "active" => Some(tr("Active")), "new" => Some(tr("New")),
        "merged" => Some(tr("Merged")), "completed" => Some(tr("Completed")),
        "done" => Some(tr("Done")), "resolved" => Some(tr("Resolved")),
        "declined" => Some(tr("Declined")), "abandoned" => Some(tr("Abandoned")),
        "removed" => Some(tr("Removed")), _ => None,
    };
    if let Some(word) = translated { return (word.to_owned(), color); }
    let word = if lower == "opened" { "open".to_owned() } else { lower };
    let mut chars = word.chars();
    let shown = chars
        .next()
        .map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect());
    (shown, color)
}

/// New pull request: where from and into, its text, then a review before it is created.
fn create_section(
    ui: &mut Ui,
    s: &mut State,
    widgets: &mut HashMap<String, UiControl>,
    busy: bool,
    (sources, targets): (&[String], &[String]),
    out: &mut PageOut,
) {
    let edit = !busy && s.preview.is_none();
    if let Some(error) = s.error.clone() {
        banner(ui, widgets, RED(), &failure_title(s.failed, &error, tr("pull requests")), &error, None);
    }
    if let Some(n) = &s.notice {
        notice_line(ui, n);
    }
    if s.uncertain && !s.error.as_deref().is_some_and(|e| e.starts_with("hosting_outcome_unknown")) {
        banner(
            ui,
            widgets,
            RED(),
            tr("Outcome unknown"),
            tr("Refresh Pull requests and check the remote before creating another request."),
            None,
        );
    }
    let server = s.session.as_ref().map(|x| host(x.config().server()).to_owned()).unwrap_or_default();
    ui.scope(|ui| {
        ui.set_max_width(ui.available_width().min(720.0));
        ui.spacing_mut().item_spacing.y = 12.0;
        let width = ui.available_width();
        form_field(ui, widgets, ("hosting-repository", tr("Repository"), &trf!("on {server}", server)), &mut s.repository, width, edit, false);
        if s.session.as_ref().is_some_and(|x|x.config().provider().is_github()) {
            let r = ui.add_enabled(edit,Checkbox::new(&mut s.from_fork,tr("Use a different source repository (fork)")));
            control(widgets,"hosting-from-fork",&r,ui.clip_rect());
            if s.from_fork {
                form_field(ui,widgets,("hosting-source-repository",tr("Source repository"),tr("owner/repository, in the same GitHub fork network")),&mut s.source_repository,width,edit,false);
                ui.colored_label(MUTED(),tr("The Repository field above is the target. Branches must already be pushed; Rocket does not create a fork or push files."));
            }
        }
        // Names on one row and fields on the next, so the arrow sits between the fields.
        let half = ((width - 32.0) / 2.0).max(120.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 5.0;
            ui.allocate_ui_with_layout(vec2(ui.available_width(), 16.0), Layout::left_to_right(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.allocate_ui_with_layout(vec2(half + 32.0, 16.0), Layout::left_to_right(Align::Center), |ui| {
                    ui.set_width(half + 32.0);
                    label(ui, tr("From"), "");
                });
                label(ui, tr("Into"), "");
            });
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                branch_field(ui, widgets, "hosting-source", &mut s.source, sources, half, edit);
                ui.allocate_ui_with_layout(vec2(16.0, CONTROL), Layout::centered_and_justified(Direction::LeftToRight), |ui| {
                    ui.label(RichText::new("→").color(HINT()));
                });
                branch_field(ui, widgets, "hosting-target", &mut s.target, targets, half, edit);
            });
        });
        form_field(ui, widgets, ("hosting-title", tr("Title"), ""), &mut s.title, width, edit, false);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 5.0;
            label(ui, tr("Description"), tr("(optional)"));
            let r = ui.add_enabled(
                edit,
                TextEdit::multiline(&mut s.body).desired_rows(4).desired_width(width),
            );
            control(widgets, "hosting-body", &r, ui.clip_rect());
        });
        if let Some(preview) = &s.preview {
            let short = |id: &str| id.chars().take(7).collect::<String>();
            let draft = preview.draft();
            Frame::new()
                .stroke(Stroke::new(1.0, CARD_STROKE()))
                .corner_radius(8)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 0.0;
                    Frame::new()
                        .fill(CARD())
                        .corner_radius(CornerRadius { nw: 7, ne: 7, sw: 0, se: 0 })
                        .inner_margin(Margin::symmetric(12, 8))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                let (mark, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                                ui.painter().circle_stroke(mark.center(), 6.0, Stroke::new(1.4, GREEN()));
                                icon(ui.painter(), mark.shrink(3.5), Icon::Check, GREEN());
                                ui.vertical(|ui| {
                                    ui.add(Label::new(RichText::new(trf!("Opens a pull request in {repository}", repository = draft.repository.path())).font(semibold(13.0))).wrap());
                                    ui.add(Label::new(RichText::new(trf!("on {server}", server = host(draft.repository.config().server()))).size(12.0).color(HINT())).wrap());
                                });
                            });
                        });
                    let top = ui.cursor().top();
                    ui.painter().hline(ui.max_rect().x_range(), top, Stroke::new(1.0, CARD_STROKE()));
                    Frame::new().inner_margin(Margin::symmetric(12, 10)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 6.0;
                        if preview.source_repository() != &draft.repository {
                            ui.add(Label::new(trf!("Source: {source} → Target: {target}", source = preview.source_repository().path(), target = draft.repository.path())).wrap());
                            ui.colored_label(MUTED(),tr("Both tips are rechecked before creation. GitHub has no atomic compare-and-create condition; inspect any uncertain outcome before retrying."));
                        }
                        for (name, branch, id) in [
                            (tr("From"), &draft.source, preview.source_id()),
                            (tr("Into"), &draft.target, preview.target_id()),
                        ] {
                            ui.horizontal(|ui| {
                                key_label(ui, name);
                                let mut job = text::LayoutJob::default();
                                job.append(branch, 0.0, TextFormat::simple(FontId::monospace(12.5), TEXT()));
                                job.append(tr(" at "), 0.0, TextFormat::simple(FontId::proportional(12.5), FAINT()));
                                job.append(&short(id), 0.0, TextFormat::simple(FontId::monospace(12.5), TEXT()));
                                ui.label(job).on_hover_text(id);
                            });
                        }
                        ui.horizontal(|ui| {
                            key_label(ui, tr("Title"));
                            ui.add(Label::new(&draft.title).wrap());
                        });
                    });
                });
        }
        ui.allocate_ui_with_layout(vec2(ui.available_width(), CONTROL), Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if s.preview.is_some() {
                let r = pages::primary(ui, !busy && !s.uncertain, tr("Create pull request"));
                control(widgets, "hosting-create", &r, ui.clip_rect());
                if r.clicked() {
                    out.action = Some("create");
                }
                if btn(ui, widgets, "hosting-back", tr("Back to edit"), !busy) {
                    s.preview = None;
                }
            } else {
                let r = pages::primary(ui, !busy && s.session.is_some() && !s.uncertain, tr("Review pull request"));
                control(widgets, "hosting-review", &r, ui.clip_rect());
                if r.clicked() {
                    out.action = Some("review");
                }
            }
            if let Some(item) = &s.created {
                let r = pages::tool(ui, Icon::External, &trf!("Open pull request #{number}", number = item.number), true);
                control(widgets, "hosting-created", &r, ui.clip_rect());
                if r.clicked() {
                    out.open = Some(item.url.clone());
                }
            }
        });
    });
}

/// The name column of the review card.
fn key_label(ui: &mut Ui, text: &str) {
    ui.allocate_ui_with_layout(vec2(90.0, 18.0), Layout::left_to_right(Align::Center), |ui| {
        ui.set_width(90.0);
        ui.label(RichText::new(text).color(HINT()));
    });
}

/// A branch name with a picker of the repository's branches beside it.
fn branch_field(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    key: &str,
    value: &mut String,
    choices: &[String],
    width: f32,
    enabled: bool,
) {
    ui.scope(|ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            // ComboBox's icon and padding can exceed its requested minimum.
            let picker_width = (ui.spacing().icon_width + ui.spacing().icon_spacing
                + 2.0 * ui.spacing().button_padding.x).max(32.0);
            let r = ui
                .add_enabled_ui(enabled, |ui| {
                    ui.add_sized(
                        [width - picker_width - ui.spacing().item_spacing.x, CONTROL],
                        TextEdit::singleline(value).font(TextStyle::Monospace).align(Align2::LEFT_CENTER),
                    )
                })
                .inner;
            control(widgets, key, &r, ui.clip_rect());
            ui.spacing_mut().combo_width = picker_width;
            if let Some(branch) = crate::branch_picker::pick(
                ui,
                widgets,
                &format!("{key}-pick"),
                "",
                choices,
                enabled && !choices.is_empty(),
            ) {
                *value = branch;
            }
        });
    });
}

/// Accounts: the saved accounts, then one form to connect.
/// Providers you run yourself, listed after the cloud services.
fn self_hosted(provider: ForgeProvider) -> bool {
    matches!(
        provider,
        ForgeProvider::GitHubEnterprise | ForgeProvider::BitbucketServer | ForgeProvider::AzureServer | ForgeProvider::Gitea
    )
}

/// The saved accounts above the account form, in one column: the form needs the
/// width for its provider choices.
fn accounts_body(
    ui: &mut Ui,
    s: &mut State,
    widgets: &mut HashMap<String, UiControl>,
    busy: bool,
    out: &mut PageOut,
) {
    if saved_accounts(ui, s, widgets, busy) {
        out.action = Some("load");
    }
    account_form(ui, s, widgets, busy, out);
}

/// The saved accounts as a list; choosing one fills the form. Returns whether Reload
/// was clicked.
fn saved_accounts(ui: &mut Ui, s: &mut State, widgets: &mut HashMap<String, UiControl>, busy: bool) -> bool {
    let mut picked = None;
    let mut fresh = false;
    let mut reload = false;
    Frame::new()
        .stroke(Stroke::new(1.0, CARD_STROKE()))
        .corner_radius(10)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            Frame::new()
                .fill(CARD())
                .corner_radius(CornerRadius { nw: 9, ne: 9, sw: 0, se: 0 })
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(tr("SAVED ACCOUNTS")).font(semibold(11.0)).color(HINT()));
                });
            if s.accounts.is_empty() {
                Frame::new().inner_margin(Margin::symmetric(12, 10)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let text = if s.accounts_loaded { tr("No saved accounts.") } else { tr("Saved accounts are not loaded.") };
                    ui.label(RichText::new(text).size(12.5).color(HINT()));
                });
            }
            for (i, a) in s.accounts.iter().enumerate() {
                let y = ui.cursor().top();
                ui.painter().hline(ui.max_rect().x_range(), y, Stroke::new(1.0, CARD_LINE()));
                let selected = s.selected_account == Some(i);
                let connected = selected && s.session.is_some();
                let mut line = format!("{} · {}", a.provider.label(), host(&a.server));
                if !a.username.is_empty() {
                    line.push_str(" · ");
                    line.push_str(&a.username);
                }
                let r = ui
                    .add_enabled_ui(!busy, |ui| account_row(ui, &a.label, &line, selected, connected))
                    .inner;
                control(widgets, format!("hosting-account-{i}"), &r, ui.clip_rect());
                if r.clicked() {
                    picked = Some(i);
                }
            }
            let y = ui.cursor().top();
            ui.painter().hline(ui.max_rect().x_range(), y, Stroke::new(1.0, CARD_LINE()));
            Frame::new().inner_margin(Margin::symmetric(8, 6)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let r = pages::ghost(ui, !busy, tr("+ New account"));
                    control(widgets, "hosting-new-account", &r, ui.clip_rect());
                    fresh = r.clicked();
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let r = pages::ghost(ui, !busy, tr("Reload"));
                        control(widgets, "hosting-load-accounts", &r, ui.clip_rect());
                        reload = r.clicked();
                    });
                });
            });
        });
    if let Some(i) = picked
        && let Some(a) = s.accounts.get(i).cloned()
    {
        s.selected_account = Some(i);
        s.label = a.label;
        s.provider = a.provider;
        s.server = a.server;
        s.azure_api_version = a.azure_api_version.unwrap_or_else(|| "7.0".into());
        s.username = a.username;
        s.token = a.token;
        s.remember = true;
        s.session = None;
        s.clear_results();
    }
    if fresh {
        s.selected_account = None;
        s.azure_api_version = "7.0".into();
        s.label.clear();
        s.username.clear();
        s.token.clear();
        s.remember = false;
        s.session = None;
        s.clear_results();
    }
    reload
}

/// One saved account: its name, provider, server and user, and whether it is connected.
fn account_row(ui: &mut Ui, name: &str, line: &str, selected: bool, connected: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 48.0), Sense::click());
    let hot = ui.is_enabled() && (response.hovered() || response.has_focus());
    if selected {
        ui.painter().rect_filled(rect, 0.0, CHIP());
    } else if hot {
        ui.painter().rect_filled(rect, 0.0, LIST_HOT());
    }
    let (status, color) = if connected { (tr("Connected"), GREEN()) } else { (tr("Saved"), HINT()) };
    let status = ui
        .painter()
        .layout_no_wrap(status.to_owned(), FontId::proportional(12.0), color);
    let right = rect.right() - 12.0 - status.size().x;
    ui.painter().galley(
        pos2(right, rect.center().y - status.size().y / 2.0),
        status,
        color,
    );
    let width = (right - rect.left() - 24.0).max(0.0);
    for (text, font, ink, y) in [
        (name, semibold(13.0), TEXT(), rect.top() + 15.0),
        (line, FontId::proportional(12.0), HINT(), rect.top() + 33.0),
    ] {
        let mut job = text::LayoutJob::simple_singleline(text.to_owned(), font, ink);
        job.wrap = text::TextWrapping::truncate_at_width(width);
        let galley = ui.painter().layout_job(job);
        ui.painter()
            .galley(pos2(rect.left() + 12.0, y - galley.size().y / 2.0), galley, ink);
    }
    response.widget_info(|| WidgetInfo::selected(Role::Button, true, selected, name));
    response
}

/// The account form: provider, name, server, user and token, then Connect.
fn account_form(
    ui: &mut Ui,
    s: &mut State,
    widgets: &mut HashMap<String, UiControl>,
    busy: bool,
    out: &mut PageOut,
) {
    Frame::new()
        .stroke(Stroke::new(1.0, CARD_STROKE()))
        .corner_radius(10)
        .inner_margin(Margin::symmetric(16, 14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 12.0;
            // Which account this is: a saved one, or a new one until it is saved.
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let title = match s.selected_account.and_then(|i| s.accounts.get(i)) {
                    Some(a) if !a.label.trim().is_empty() => a.label.clone(),
                    Some(a) => a.provider.label().to_owned(),
                    None => tr("New account").to_owned(),
                };
                ui.label(RichText::new(title).font(semibold(14.0)));
                let (text, color) = match (&s.session, s.selected_account) {
                    (Some(_), _) => (tr("Connected"), GREEN()),
                    (None, Some(_)) => (tr("Not signed in"), AMBER()),
                    (None, None) => (tr("Not saved"), HINT()),
                };
                pages::pill(ui, text, color);
            });
            // Cloud services and self-hosted servers, each group on its own row. Choosing
            // another provider starts a new account.
            let mut picked = None;
            ui.add_enabled_ui(!busy, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                for (group, own_server) in [(tr("Cloud"), false), (tr("Self-hosted"), true)] {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                        let (rect, _) = ui.allocate_exact_size(vec2(80.0, 24.0), Sense::hover());
                        ui.painter().text(rect.left_center(), Align2::LEFT_CENTER, group, FontId::proportional(12.5), HINT());
                        for p in ForgeProvider::ALL.into_iter().filter(|p| self_hosted(*p) == own_server) {
                            let r = ui.add(Button::new(RichText::new(p.label()).size(12.5))
                                .selected(p == s.provider).corner_radius(5)
                                .wrap_mode(TextWrapMode::Extend));
                            control(widgets, format!("hosting-provider-{}", p.key()), &r, ui.clip_rect());
                            if r.clicked() {
                                picked = Some(p);
                            }
                        }
                    });
                }
            });
            if let Some(p) = picked
                && p != s.provider
            {
                s.provider = p;
                s.server = p.default_server().into();
                s.azure_api_version = "7.0".into();
                s.token.clear();
                s.username.clear();
                s.session = None;
                s.selected_account = None;
                s.clear_results();
            }
            let width = ui.available_width();
            let half = ((width - 12.0) / 2.0).max(140.0);
            let two = width >= 2.0 * 140.0 + 12.0;
            let column = if two { half } else { width };
            let token_only = matches!(s.provider, ForgeProvider::GitHubEnterprise | ForgeProvider::AzureServer);
            let server_label = match s.provider { ForgeProvider::Azure => tr("Organization URL"), ForgeProvider::AzureServer => tr("Collection URL"), _ => tr("Server URL") };
            let user_label = if s.provider == ForgeProvider::Bitbucket { tr("Atlassian email") } else { tr("Username") };
            let mut account_changed = false;
            let mut field = |ui: &mut Ui, (key, name, hint): (&str, &str, &str), value: &mut String, password: bool| {
                form_field(ui, widgets, (key, name, hint), value, column, !busy, password)
            };
            // The profile name is only a label; the others decide what to connect to.
            if token_only {
                // These providers identify the user from the PAT, not a typed username.
                if two {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        field(ui, ("hosting-label", tr("Profile name"), ""), &mut s.label, false);
                        account_changed |= field(ui, ("hosting-server", server_label, s.provider.default_server()), &mut s.server, false);
                    });
                } else {
                    field(ui, ("hosting-label", tr("Profile name"), ""), &mut s.label, false);
                    account_changed |= field(ui, ("hosting-server", server_label, s.provider.default_server()), &mut s.server, false);
                }
                account_changed |= form_field(ui, widgets, ("hosting-token", tr("Personal access token"), ""), &mut s.token, width, !busy, true);
            } else if two {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    field(ui, ("hosting-label", tr("Profile name"), ""), &mut s.label, false);
                    account_changed |= field(ui, ("hosting-server", server_label, ""), &mut s.server, false);
                });
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    account_changed |= field(ui, ("hosting-username", user_label, tr("(optional)")), &mut s.username, false);
                    account_changed |= field(ui, ("hosting-token", tr("Access token"), ""), &mut s.token, true);
                });
            } else {
                field(ui, ("hosting-label", tr("Profile name"), ""), &mut s.label, false);
                account_changed |= field(ui, ("hosting-server", server_label, ""), &mut s.server, false);
                account_changed |= field(ui, ("hosting-username", user_label, tr("(optional)")), &mut s.username, false);
                account_changed |= field(ui, ("hosting-token", tr("Access token"), ""), &mut s.token, true);
            }
            if s.provider == ForgeProvider::AzureServer {
                ui.label(RichText::new(tr("REST API compatibility")).size(12.5).color(HINT()));
                ui.horizontal_wrapped(|ui| {
                    for (version, id, label) in [("6.0", "6-0", "2020 (6.0)"), ("7.0", "7-0", "2022 (7.0)"), ("7.1", "7-1", "2022.1+ (7.1)")] {
                        let r = ui.add_enabled(!busy, Button::new(label).selected(s.azure_api_version == version));
                        control(widgets, format!("hosting-azure-api-{id}"), &r, ui.clip_rect());
                        if r.clicked() && s.azure_api_version != version {
                            s.azure_api_version = version.into();
                            account_changed = true;
                        }
                    }
                });
            }
            if account_changed {
                s.session = None;
                s.clear_results();
            }
            // The token each service needs, in a line; the guide has the details.
            let scopes = match s.provider {
                ForgeProvider::GitHub => tr("Token: read access to repository metadata, issues and pull requests; write access to create them."),
                ForgeProvider::GitHubEnterprise => tr("A token from this server, and its HTTPS address without /api/v3."),
                ForgeProvider::GitLab => tr("Token scope read_api to browse; api to create merge requests."),
                ForgeProvider::Bitbucket => tr("Atlassian API token with repository and pull request access. Browser sessions last an hour and are not saved."),
                ForgeProvider::BitbucketServer => tr("Personal HTTP access token. Include any context path in the server URL."),
                ForgeProvider::AzureServer => tr("The collection URL with any virtual directory, and a PAT from that server."),
                ForgeProvider::Azure => tr("PAT with Code (Read) and Work Items (Read); Code (Read & write) to create pull requests."),
                ForgeProvider::Gitea => tr("Token with repository and issue read access; repository write access to create pull requests."),
            };
            pages::with_guide_link(ui, &[scopes], 12.5, HINT(), widgets, "hosting-guide");
            let r = ui.add_enabled(!busy, checkbox(&mut s.remember, tr("Remember on this computer")));
            control(widgets, "hosting-remember", &r, ui.clip_rect());
            // Keep every action in one wrapping layout so narrow forms cannot
            // paint the right-aligned buttons on top of Forget saved account.
            ui.with_layout(Layout::right_to_left(Align::Min).with_main_wrap(true), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let r = pages::primary(ui, !busy && s.enabled, tr("Connect and verify"));
                control(widgets, "hosting-connect", &r, ui.clip_rect());
                if r.clicked() {
                    out.action = Some("connect");
                }
                if matches!(s.provider, ForgeProvider::GitHub | ForgeProvider::Bitbucket)
                    && btn(ui, widgets, "hosting-browser", tr("Sign in with browser"), !busy && s.enabled)
                {
                    out.action = Some("browser");
                }
                if btn(ui, widgets, "hosting-disconnect", tr("Disconnect"), !busy && s.session.is_some()) {
                    s.session = None;
                    s.token.clear();
                    s.clear_results();
                    s.notice = Some(tr("Session disconnected. Saved profiles are unchanged.").into());
                }
                let r = pages::danger_ghost(ui, !busy && s.selected_account.is_some(), tr("Forget saved account"));
                control(widgets, "hosting-forget", &r, ui.clip_rect());
                if r.clicked() {
                    out.action = Some("forget");
                }
            });
        });
}

/// What a failed request was doing, as the problem's title.
fn failure_title(operation: &str, error: &str, things: &str) -> String {
    let code = split_code(error).0.unwrap_or_default();
    match operation {
        "list" if code == "hosting_invalid_pagination" => trf!("Couldn't load more {things}", things),
        "list" => trf!("Couldn't load {things}", things),
        "review" => tr("Couldn't review the pull request").into(),
        "create" if code == "hosting_outcome_unknown" => tr("The pull request may have been created").into(),
        "create" => tr("Couldn't create the pull request").into(),
        "connect" => tr("Couldn't connect").into(),
        "accounts" => tr("Couldn't use the saved accounts").into(),
        _ => String::new(),
    }
}

/// A hosting error's code and its guidance: `hosting_x: what to do`.
fn split_code(error: &str) -> (Option<&str>, &str) {
    match error.split_once(": ") {
        Some((code, rest))
            if code.starts_with("hosting_") && code.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') =>
        {
            (Some(code), rest)
        }
        _ => (None, error),
    }
}

/// A problem in plain words: what happened, what to do, and Rocket's code as small
/// print, with an optional action on the right. Returns whether it was clicked.
fn banner(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    color: Color32,
    title: &str,
    error: &str,
    action: Option<(&str, &str, bool)>,
) -> bool {
    let (code, message) = split_code(error);
    let mut clicked = false;
    Frame::new()
        .fill(color.gamma_multiply(0.08))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.3)))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(14, 11))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                    if let Some((key, label, enabled)) = action {
                        let r = ui.add_enabled(enabled, Button::new(label));
                        control(widgets, key, &r, ui.clip_rect());
                        clicked = r.clicked();
                    }
                    ui.with_layout(Layout::top_down(Align::Min), |ui| {
                        ui.spacing_mut().item_spacing.y = 3.0;
                        if !title.is_empty() {
                            ui.label(RichText::new(title).font(semibold(13.0)).color(color));
                        }
                        ui.add(Label::new(RichText::new(message).color(TEXT_SOFT())).wrap());
                        if let Some(code) = code {
                            ui.label(RichText::new(code).monospace().size(11.5).color(FAINT()));
                        }
                    });
                });
            });
        });
    clicked
}

/// Where things stand after a request, in one quiet line.
fn notice_line(ui: &mut Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
        icon(ui.painter(), rect, Icon::Info, BLUE());
        ui.add(Label::new(RichText::new(text).size(12.5).color(MUTED())).wrap());
    });
}

/// A toolbar field with its icon inside; its name is the hint and the tooltip.
#[allow(clippy::too_many_arguments)]
fn icon_field(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    key: &str,
    image: Icon,
    hint: &str,
    value: &mut String,
    width: f32,
    enabled: bool,
) -> Response {
    let r = ui
        .add_enabled_ui(enabled, |ui| {
            ui.add_sized(
                [width, CONTROL],
                TextEdit::singleline(value)
                    .hint_text(hint)
                    .align(Align2::LEFT_CENTER)
                    .margin(Margin { left: 28, right: 8, top: 4, bottom: 4 }),
            )
        })
        .inner;
    icon(
        ui.painter(),
        Rect::from_center_size(pos2(r.rect.left() + 15.0, r.rect.center().y), vec2(13.0, 13.0)),
        image,
        HINT(),
    );
    control(widgets, key, &r, ui.clip_rect());
    r.on_hover_text(hint)
}

/// A short field with its name drawn inside it, such as "author:".
fn prefix_field(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    key: &str,
    prefix: &str,
    value: &mut String,
    width: f32,
    enabled: bool,
) -> Response {
    let font = FontId::proportional(12.0);
    let inset = ui.painter().layout_no_wrap(prefix.to_owned(), font.clone(), HINT()).size().x + 14.0;
    let r = ui
        .add_enabled_ui(enabled, |ui| {
            ui.add_sized(
                [width, CONTROL],
                TextEdit::singleline(value)
                    .char_limit(256)
                    .align(Align2::LEFT_CENTER)
                    .margin(Margin { left: inset.min(120.0) as i8, right: 8, top: 4, bottom: 4 }),
            )
        })
        .inner;
    ui.painter().text(pos2(r.rect.left() + 9.0, r.rect.center().y), Align2::LEFT_CENTER, prefix, font, HINT());
    control(widgets, key, &r, ui.clip_rect());
    r
}

/// A field's name, with a fainter hint after it.
fn label(ui: &mut Ui, name: &str, hint: &str) {
    let mut job = text::LayoutJob::default();
    job.append(name, 0.0, TextFormat::simple(FontId::proportional(12.0), MUTED()));
    if !hint.is_empty() {
        job.append(&format!("  {hint}"), 0.0, TextFormat::simple(FontId::proportional(12.0), FAINT()));
    }
    ui.label(job);
}

/// A labeled single-line field. Returns whether it changed.
fn form_field(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    (key, name, hint): (&str, &str, &str),
    value: &mut String,
    width: f32,
    enabled: bool,
    password: bool,
) -> bool {
    ui.vertical(|ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing.y = 5.0;
        label(ui, name, hint);
        let r = ui
            .add_enabled_ui(enabled, |ui| {
                ui.add_sized(
                    [width, CONTROL],
                    TextEdit::singleline(value).password(password).align(Align2::LEFT_CENTER),
                )
            })
            .inner;
        control(widgets, key, &r, ui.clip_rect());
        r.changed()
    })
    .inner
}

fn btn(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    key: &str,
    label: &str,
    enabled: bool,
) -> bool {
    let r = ui.add_enabled(enabled, Button::new(label));
    control(widgets, key, &r, ui.clip_rect());
    r.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_form_fits_compact_widths() {
        for width in [440.0, 680.0, 1200.0] {
            let ctx = Context::default();
            crate::style::install(&ctx);
            let mut state = State { section: Section::Create, ..Default::default() };
            let mut widgets = HashMap::new();
            let mut bounds = Rect::NOTHING;
            for _ in 0..3 {
                let mut output = ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 1800.0))), ..Default::default() }, |ui| {
                    Frame::new().inner_margin(24).show(ui, |ui| {
                        create_section(ui, &mut state, &mut widgets, false, (&[], &[]), &mut PageOut::default());
                    });
                    bounds = ui.min_rect();
                });
                output.textures_delta.clear();
            }
            assert!(bounds.right() <= width + 1.0, "Create form overflows {width}: {bounds:?}");
            assert!(widgets["hosting-review"].rect.right() <= width - 24.0);
        }
    }

    #[test]
    fn provider_accounts_fit_compact_widths() {
        for width in [440.0, 700.0, 1200.0] {
            for provider in [ForgeProvider::Bitbucket, ForgeProvider::BitbucketServer, ForgeProvider::GitHubEnterprise, ForgeProvider::AzureServer] {
                let ctx=Context::default();
                crate::style::install(&ctx);
                let mut s=State { provider, server:provider.default_server().into(), ..Default::default() };
                let mut widgets=HashMap::new();let mut bounds=Rect::NOTHING;
                for _ in 0..3 {
                    widgets.clear();
                    let mut output=ctx.run_ui(RawInput{screen_rect:Some(Rect::from_min_size(Pos2::ZERO,vec2(width,1800.0))),..Default::default()},|ui|{
                        accounts_body(ui,&mut s,&mut widgets,false,&mut PageOut::default());bounds=ui.min_rect();
                    });
                    output.textures_delta.clear();
                }
                assert!(bounds.right()<=width+1.0,"{provider:?} overflow at {width}: {bounds:?}");
                if matches!(provider, ForgeProvider::GitHubEnterprise | ForgeProvider::AzureServer) {
                    assert!(!widgets.contains_key("hosting-browser"));
                    assert!(!widgets.contains_key("hosting-username"));
                    assert!(widgets["hosting-token"].rect.right() <= width + 1.0);
                }
                if provider == ForgeProvider::AzureServer {
                    for id in ["hosting-azure-api-6-0", "hosting-azure-api-7-0", "hosting-azure-api-7-1"] {
                        assert!(widgets[id].rect.right() <= width + 1.0, "{id} exceeds {width}");
                    }
                }
                for id in ["hosting-provider-bitbucket-server","hosting-provider-gitea","hosting-connect"] {
                    let c=&widgets[id];assert!(c.rect.width()>20.0&&c.rect.height()>18.0,"{id} clipped at {width}: {:?}",c.rect);
                }
            }
        }
    }

    #[test]
    fn hosting_problems_split_into_words_and_their_code() {
        let pagination = "hosting_invalid_pagination: The server returned an invalid pagination link.";
        assert_eq!(
            split_code(pagination),
            (Some("hosting_invalid_pagination"), "The server returned an invalid pagination link.")
        );
        assert_eq!(split_code("Connect a hosting account first."), (None, "Connect a hosting account first."));
        assert_eq!(split_code("Hosting_x: y"), (None, "Hosting_x: y"));
        assert_eq!(failure_title("list", pagination, "pull requests"), "Couldn't load more pull requests");
        assert_eq!(failure_title("list", "hosting_network: x", "issues"), "Couldn't load issues");
        assert_eq!(
            failure_title("create", "hosting_outcome_unknown: x", ""),
            "The pull request may have been created"
        );
        assert_eq!(failure_title("", "Connect a hosting account first.", ""), "");
    }

    #[test]
    fn servers_and_states_read_as_people_say_them() {
        assert_eq!(host("https://gitea.com/"), "gitea.com");
        assert_eq!(host(" http://127.0.0.1:8080 "), "127.0.0.1:8080");
        assert_eq!(host("https://dev.azure.com/org"), "dev.azure.com/org");
        assert_eq!(state_look("opened"), ("Open".to_owned(), GREEN()));
        assert_eq!(state_look("MERGED"), ("Merged".to_owned(), VIOLET()));
        assert_eq!(state_look("abandoned").1, RED());
        assert_eq!(state_look("Committed"), ("Committed".to_owned(), BLUE()));
    }

    #[test]
    fn the_list_fits_the_page_with_refresh_beside_the_filter() {
        // A wrapping row reports the whole row as available: a filter sized from that
        // widened the page and pushed each row's state and Open button out of view.
        let ctx = Context::default();
        crate::style::install(&ctx);
        let mut widgets = HashMap::new();
        let mut s = State {
            section: Section::Pulls,
            loaded: true,
            next: Some(ForgeCursor::default()),
            ..Default::default()
        };
        s.items = (1..=3)
            .map(|number| ForgeItem {
                number,
                title: "PRIVATE 改善 Unicode".into(),
                state: "open".into(),
                author: "fixture".into(),
                source: "feature/界".into(),
                target: "main".into(),
                url: "https://example.invalid/pull".into(),
            })
            .collect();
        let (mut content, mut viewport) = (Rect::NOTHING, Rect::NOTHING);
        for _ in 0..2 {
            widgets.clear();
            let screen = Rect::from_min_size(Pos2::ZERO, vec2(1244.0, 650.0));
            let mut output = ctx.run_ui(RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| {
                viewport = ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        content = Frame::new()
                            .inner_margin(Margin { left: 24, right: 24, top: 16, bottom: 24 })
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                let mut page = PageOut::default();
                                list_section(ui, &mut s, &mut widgets, false, &mut page);
                                list_card(ui, &s, &mut widgets, false, "pull requests", false, &mut page);
                            })
                            .response
                            .rect;
                    })
                    .inner_rect;
            });
            output.textures_delta.clear();
        }
        assert!(content.width() <= viewport.width() + 0.5, "{content:?} in {viewport:?}");
        let (filter, refresh) = (widgets["hosting-filter"].rect, widgets["hosting-refresh"].rect);
        assert!((refresh.center().y - filter.center().y).abs() < 4.0, "Refresh wrapped: {refresh:?}");
        let open = widgets["hosting-item-0"].rect;
        assert!(open.right() <= viewport.right() - 24.0, "{open:?}");
        assert!(widgets.contains_key("hosting-more"));
    }
}

#[cfg(test)]
mod flow_tests {
    use super::Section;
    use crate::{
        app::{DialogKind, Intent},
        batch::flow_tests::{click, enabled, open, wait},
        test_common as common,
    };
    use eframe::egui::Context;
    use rocket_git::ForgeProvider;

    #[test]
    fn signed_out_lists_point_to_accounts_and_the_form_keeps_its_controls() {
        common::isolated(
            "ROCKET_HOSTING_UI_CASE",
            "forge::flow_tests::signed_out_lists_point_to_accounts_and_the_form_keeps_its_controls",
            || {
                let f = common::Fixture::new();
                f.write("a.txt", "1\n");
                f.commit("base");
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                app.dispatch(Intent::Dialog(DialogKind::Hosting));
                // Opening the page reads saved accounts (none in a fixture's state folder).
                wait(&ctx, &mut app);
                assert!(app.forge.accounts_loaded && app.forge.accounts.is_empty());
                click(&ctx, &mut app, "hosting-tab-pulls");
                assert!(!enabled(&ctx, &mut app, "hosting-refresh"), "nothing to refresh signed out");
                click(&ctx, &mut app, "hosting-go-accounts");
                assert_eq!(app.forge.section, Section::Accounts);
                assert_eq!(app.preferences_page(), Some(crate::preferences::Page::Accounts));
                click(&ctx, &mut app, "hosting-provider-gitlab");
                assert_eq!(app.forge.provider, ForgeProvider::GitLab);
                assert_eq!(app.forge.server, ForgeProvider::GitLab.default_server());
                for (key, on) in [
                    ("hosting-connect", true),
                    ("hosting-load-accounts", true),
                    ("hosting-new-account", true),
                    ("hosting-remember", true),
                    ("hosting-forget", false),
                    ("hosting-disconnect", false),
                ] {
                    assert_eq!(enabled(&ctx, &mut app, key), on, "{key}");
                }
                // Closing Preferences goes back to Hosting at its lists; the header's
                // Accounts (and the old tab's name) open Preferences again.
                click(&ctx, &mut app, "dialog-cancel");
                assert_eq!(app.page(), Some(crate::pages::Page::Hosting));
                assert_eq!(app.forge.section, Section::Pulls);
                click(&ctx, &mut app, "hosting-tab-accounts");
                assert_eq!(app.preferences_page(), Some(crate::preferences::Page::Accounts));
                click(&ctx, &mut app, "dialog-cancel");
                assert_eq!(app.page(), Some(crate::pages::Page::Hosting));
                click(&ctx, &mut app, "dialog-cancel");
                assert!(app.dialog.is_none());
            },
        );
    }
}
