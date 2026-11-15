//! Explore: search GitHub's repositories, a GitLab or Gitea server's, or paste any
//! repository address, then preview one read-only in a tab or clone it. A search runs
//! only when asked (Enter or Search): GitHub allows 30 searches a minute signed in and
//! 10 without, so typing alone sends nothing. Results stay when a search is refused or
//! limited.
use crate::{
    app::*,
    backend::user_error,
    i18n::{tr, trf, trn},
    pages::{self, control},
    preview_tab::PreviewRequest,
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

mod hosts;
pub use hosts::{Host, HostKind, Item, forge_sites, github_words, hosts, without_github_words};

/// The languages offered in the Language list; others can be typed as `language:`.
pub const LANGUAGES: [&str; 16] = [
    "C", "C#", "C++", "Go", "Java", "JavaScript", "Kotlin", "Lua", "PHP", "Python", "Ruby",
    "Rust", "Shell", "Swift", "TypeScript", "Zig",
];

/// Shown under the search box once this few searches are left.
const FEW_LEFT: u64 = 5;

/// Why the list shows a message instead of, or above, results.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    Offline,
    /// GitHub's 422.
    Refused,
    NoResults,
    Failed(String),
    /// The server lets only signed-in users search.
    SignInRequired,
    /// The host refused the account's token.
    TokenRefused,
    /// GitHub's search words typed for another host, which would look for them in
    /// names and descriptions: shown instead of sending them.
    GitHubWords(Vec<String>),
}

enum Done {
    GitHub {
        session: Arc<GitHubSession>,
        outcome: Result<GitHubSearchOutcome>,
    },
    Forge(Box<ForgeDone>),
}

/// A GitLab or Gitea search that finished, and what it was.
struct ForgeDone {
    /// The host searched; the results are its server's only.
    host: String,
    /// The remembered account the search signed in with, by Accounts' key.
    account: Option<String>,
    /// The account list's generation when the search started: a session or a refusal
    /// from before an account change isn't kept.
    generation: u64,
    /// The session used, when the search got that far: kept for the next search.
    session: Option<Arc<ForgeSession>>,
    /// What was searched, with which language and order, for Load more.
    query: String,
    language: Option<&'static str>,
    sort: RepositorySearchSort,
    outcome: Result<RepositorySearchOutcome>,
}

#[derive(Default)]
pub struct State {
    /// What the box holds.
    pub query: String,
    /// The search the list shows: the box's text with the language added.
    pub searched: String,
    pub language: Option<&'static str>,
    pub sort: GitHubSearchSort,
    /// The order on GitLab and Gitea, which can't rank by relevance.
    pub forge_sort: RepositorySearchSort,
    /// The language and order of the GitLab or Gitea search the list shows, which Load
    /// more keeps however the controls changed since.
    searched_language: Option<&'static str>,
    searched_sort: RepositorySearchSort,
    /// Other hosts' countdowns, by host key, kept while another host is chosen.
    waits: HashMap<String, Instant>,
    /// The chosen host's key, kept while Rocket runs; empty for GitHub.
    pub host: String,
    pub results: Vec<Item>,
    /// `None` when the host leaves the total out (GitLab.com past 10,000).
    pub total: Option<u64>,
    pub incomplete: bool,
    pub next_page: Option<u32>,
    pub selected: Option<usize>,
    /// Searches GitHub says are left this minute.
    pub remaining: Option<u64>,
    /// Rate limited until then; earlier results stay.
    pub wait_until: Option<Instant>,
    pub problem: Option<Problem>,
    /// The session searches use when there is no sign-in.
    pub anonymous: Option<Arc<GitHubSession>>,
    /// Sessions without an account for gitlab.com and codeberg.org, by host key.
    public: HashMap<String, Arc<ForgeSession>>,
    /// A silent sign-in was tried this run; don't keep asking the credential helper.
    pub tried_sign_in: bool,
    /// Preview anyway was offered for this result (bigger than the preview limit).
    pub confirm_large: Option<usize>,
    /// Put the keyboard in the search box on the next frame.
    pub focus: bool,
    task: Option<(ReadTask<Done>, bool)>,
}

impl State {
    pub fn running(&self) -> bool {
        self.task.is_some()
    }
    /// Seconds until searching is allowed again.
    pub fn wait_seconds(&self) -> Option<u64> {
        let until = self.wait_until?;
        let left = until.saturating_duration_since(Instant::now());
        (left > Duration::ZERO).then(|| left.as_secs() + u64::from(left.subsec_nanos() > 0))
    }
    pub fn cancel(&mut self) {
        self.task = None;
    }
    /// Choose the host `key`: the current host's countdown is kept aside and the new
    /// host's, if it has one running, comes back.
    fn choose_host(&mut self, key: String) {
        let now = Instant::now();
        if let Some(until) = self.wait_until.take().filter(|until| *until > now) {
            self.waits.insert(self.host.clone(), until);
        }
        self.waits.retain(|_, until| *until > now);
        self.wait_until = self.waits.remove(&key);
        self.host = key;
    }
}

/// A repository address typed or pasted into the box, previewable without a search.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Address {
    pub source: CloneSource,
    /// What origin should name once kept: the URL, or the folder.
    pub url: String,
    /// "owner/name", a GitLab path with its subgroups, or the folder's name.
    pub name: String,
    pub host: String,
    /// A github.com page, for Open on GitHub.
    pub web: Option<String>,
    /// Typed in the short SSH form, `git@host:group/name.git`; `url` is its ssh:// form.
    pub short: bool,
    /// The short SSH form on a server Rocket doesn't know as GitHub, GitLab or Gitea,
    /// where its path may start in the home folder: no preview, the card asks for ssh://.
    pub unknown_server: bool,
}

/// [`address_on`] with only the well-known hosts' short SSH form.
pub fn address(text: &str) -> Option<Address> {
    address_on(text, &forge_sites(&[]))
}

/// The box's text as a repository address: an https:// or ssh:// URL, the short SSH
/// form, or an existing local repository folder. Anything else is a search. The short
/// form opens as ssh:// on `sites`, the GitHub, GitLab and Gitea hosts Rocket knows.
pub fn address_on(text: &str, sites: &[String]) -> Option<Address> {
    let text = text.trim();
    if text.is_empty() || text.chars().any(char::is_control) {
        return None;
    }
    if let Some(rest) = text.strip_prefix("https://").or_else(|| text.strip_prefix("ssh://")) {
        let (authority, path) = rest.split_once('/')?;
        let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        let host = host.split(':').next().unwrap_or(host).to_ascii_lowercase();
        let mut parts = path.split('/').filter(|p| !p.is_empty());
        if host == "github.com" {
            // A pasted page such as …/owner/name/tree/main still means the repository.
            let owner = parts.next()?;
            let name = parts.next()?;
            let remote = GitHubRemote::new(owner, name.strip_suffix(".git").unwrap_or(name))?;
            return Some(Address {
                source: CloneSource::Url(format!("https://github.com/{}.git", remote.full_name())),
                url: format!("https://github.com/{}.git", remote.full_name()),
                name: remote.full_name(),
                host,
                web: Some(remote.web_url()),
                short: false,
                unknown_server: false,
            });
        }
        // GitLab's pages put "/-/" between the project and the page: the project ends
        // there, and so does the page's ?query or #line.
        let kept = match text.find("/-/") {
            Some(at) => &text[..at],
            None => text.strip_suffix("/-").unwrap_or(text),
        };
        if kept.contains(['?', '#', '\\']) || kept.contains(char::is_whitespace) {
            return None;
        }
        let segments: Vec<&str> = kept.splitn(4, '/').nth(3).unwrap_or("").split('/').filter(|p| !p.is_empty()).collect();
        if segments.is_empty() {
            return None;
        }
        let path = segments.join("/");
        let name = path.strip_suffix(".git").unwrap_or(&path).to_owned();
        let url = kept.to_owned();
        return Some(Address {
            source: CloneSource::Url(url.clone()),
            url,
            name,
            host,
            web: None,
            short: false,
            unknown_server: false,
        });
    }
    if let Some(short) = short_ssh(text, sites) {
        return Some(short);
    }
    let path = Path::new(text);
    if path.is_absolute() && path.is_dir() {
        let name = path.file_name()?.to_string_lossy().into_owned();
        return Some(Address {
            source: CloneSource::Local(path.to_path_buf()),
            url: text.to_owned(),
            name,
            host: "this computer".into(),
            web: None,
            short: false,
            unknown_server: false,
        });
    }
    None
}

/// Git's short SSH form, `user@host:path` with no slash before the colon. On a known
/// host the path is the repository's, so it opens as `ssh://user@host/path`. On another
/// server a path without a leading slash may start in the home folder, which ssh:// can't
/// say for sure, so it stays unopened and the card asks for the ssh:// form.
fn short_ssh(text: &str, sites: &[String]) -> Option<Address> {
    if text.contains("://") || text.contains(char::is_whitespace) || text.contains(['?', '#', '\\']) {
        return None;
    }
    let (login, path) = text.split_once(':')?;
    let (user, host) = login.rsplit_once('@')?;
    let host = host.to_ascii_lowercase();
    let user_ok = !user.is_empty() && user.len() <= 64 && user.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    let host_ok = !host.is_empty()
        && host.len() <= 253
        && host.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.'))
        && !host.starts_with(['-', '.']);
    if login.contains('/') || !user_ok || !host_ok || path.is_empty() || path.len() > 1024 {
        return None;
    }
    let relative = path.trim_start_matches('/');
    let segments: Vec<&str> = relative.split('/').filter(|p| !p.is_empty()).collect();
    if segments.is_empty() {
        return None;
    }
    let joined = segments.join("/");
    let name = joined.strip_suffix(".git").unwrap_or(&joined).to_owned();
    let web = match segments.as_slice() {
        [owner, repo] if host == "github.com" => GitHubRemote::new(owner, repo.strip_suffix(".git").unwrap_or(repo)).map(|r| r.web_url()),
        _ => None,
    };
    let url = format!("ssh://{user}@{host}/{joined}");
    // A path from the root means the same in both forms.
    let known = sites.contains(&host) || path.starts_with('/');
    Some(Address {
        source: CloneSource::Url(url.clone()),
        url,
        name,
        host,
        web,
        short: true,
        unknown_server: !known,
    })
}

/// The query GitHub gets: the box's text and the chosen language.
pub fn compose(query: &str, language: Option<&str>) -> String {
    let query = query.trim();
    match language {
        Some(l) if !query.to_ascii_lowercase().contains("language:") => {
            let l = if l.contains(' ') { format!("\"{l}\"") } else { l.to_owned() };
            format!("{query} language:{l}")
        }
        _ => query.to_owned(),
    }
}

/// 18214 → "18.2k", 1_200_000 → "1.2M".
pub fn count(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => trim(format!("{:.1}", n as f64 / 1_000.0)) + "k",
        _ => trim(format!("{:.1}", n as f64 / 1_000_000.0)) + "M",
    }
}
fn trim(text: String) -> String {
    text.strip_suffix(".0").map(str::to_owned).unwrap_or(text)
}

/// Bytes as "41 MB" or "4.9 GB".
pub fn size(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    let mb = bytes as f64 / MB;
    if mb < 1.0 {
        format!("{} KB", (bytes / 1024).max(1))
    } else if mb < 1024.0 {
        format!("{} MB", mb.round() as u64)
    } else {
        trim(format!("{:.1}", mb / 1024.0)) + " GB"
    }
}

/// GitHub's "2026-11-02T10:00:00Z" as Unix seconds.
pub fn unix(time: &str) -> Option<i64> {
    let (date, clock) = time.strip_suffix('Z')?.split_once('T')?;
    let mut d = date.split('-').map(|v| v.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let mut c = clock.split(':').map(|v| v.parse::<i64>().ok());
    let (h, min, s) = (c.next()??, c.next()??, c.next()??);
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146_097 + doe - 719_468) * 86_400 + h * 3_600 + min * 60 + s)
}

/// "Updated 3 days ago" for Unix seconds.
fn updated(at: Option<i64>) -> Option<String> {
    let at = at?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs() as i64;
    Some(trf!("Updated {when}", when = ago(now.saturating_sub(at).max(0) as u64, at)))
}

/// "2026-11-02" in UTC for Unix seconds (Howard Hinnant's civil-from-days).
fn date(at: i64) -> String {
    let z = at.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[derive(Default)]
struct Flags {
    search: bool,
    more: bool,
    stop: bool,
    sign_in: bool,
    close: bool,
    select: Option<usize>,
    preview: Option<(PreviewRequest, bool)>,
    clone: Option<String>,
    open: Option<String>,
    sort: Option<GitHubSearchSort>,
    forge_sort: Option<RepositorySearchSort>,
    language: Option<Option<&'static str>>,
    confirm: Option<usize>,
    /// Another host was chosen in the menu, by key.
    host: Option<String>,
    /// Add account for the chosen host, or open Preferences → Accounts.
    add_account: bool,
    accounts: bool,
    /// Search again without GitHub's search words.
    without_words: bool,
}

fn request_for(item: &Item, host: &Host) -> PreviewRequest {
    PreviewRequest {
        source: if item.clone_url.starts_with("https://") || item.clone_url.starts_with("ssh://") {
            CloneSource::Url(item.clone_url.clone())
        } else {
            CloneSource::Local(PathBuf::from(&item.clone_url))
        },
        url: item.clone_url.clone(),
        name: item.path.clone(),
        host: host.site(),
        web: Some(item.web_url.clone()),
        site: host.name(),
        size_kb: item.size_kb,
        branch: item.default_branch.clone(),
    }
}

fn request_from(address: &Address) -> PreviewRequest {
    PreviewRequest {
        source: address.source.clone(),
        url: address.url.clone(),
        name: address.name.clone(),
        host: address.host.clone(),
        web: address.web.clone(),
        site: "GitHub",
        size_kb: None,
        branch: None,
    }
}

impl Rocket {
    /// Open Explore, optionally with text to search for or an address to show.
    pub(crate) fn open_explore(&mut self, query: Option<String>) {
        if !matches!(self.dialog.as_ref().map(|d| &d.kind), Some(DialogKind::Explore)) {
            self.show_dialog(DialogKind::Explore);
        }
        // The GitLab and Gitea accounts in Preferences → Accounts are hosts to search.
        self.ci.want_accounts();
        // Back from Accounts, a sign-in message may no longer hold; the next search says.
        if matches!(self.explore.problem, Some(Problem::SignInRequired | Problem::TokenRefused)) {
            self.explore.problem = None;
        }
        self.explore.focus = true;
        if let Some(query) = query {
            self.explore.query = query;
            if self.explore_address().is_none() && !self.explore.query.trim().is_empty() {
                self.start_explore_search(false, false);
            }
        }
    }

    /// Where Explore can search now, in menu order.
    pub fn explore_hosts(&self) -> Vec<Host> {
        let forge = self.forge.enabled && !self.forge.account_check_only && self.forge.publish_check.is_none();
        let live = self.forge.session.as_ref().map(|s| (s.config().provider(), s.config().server()));
        hosts(forge, self.ci.remembered(), live)
    }

    /// The chosen host. When it has gone, the one host left for the same server takes
    /// its place (an account added for gitlab.com replaces gitlab.com without one, and
    /// the other way round); with none, or two accounts there, GitHub.
    pub fn explore_host(&self) -> Host {
        let key = &self.explore.host;
        if key.is_empty() {
            return Host::github();
        }
        let list = self.explore_hosts();
        let (provider, server) = place(key);
        list.iter()
            .find(|h| h.key == *key)
            .cloned()
            .or_else(|| {
                let mut same = list.iter().filter(|h| !h.is_github() && h.provider.key() == provider && h.server == server);
                match (same.next(), same.next()) {
                    (Some(only), None) => Some(only.clone()),
                    // Two accounts on one server: don't guess which one was meant.
                    _ => None,
                }
            })
            .unwrap_or_else(Host::github)
    }

    /// [`Self::explore_host`], remembered as the choice, so results that arrive belong
    /// to it. The same server's results stay; another host's go.
    fn settle_explore_host(&mut self) -> Host {
        let host = self.explore_host();
        let key = if host.is_github() { String::new() } else { host.key.clone() };
        if key != self.explore.host {
            let s = &mut self.explore;
            if place(&s.host) != place(&key) {
                s.cancel();
                s.results.clear();
                s.selected = None;
                s.total = None;
                s.next_page = None;
                s.problem = None;
                s.searched.clear();
                s.remaining = None;
            }
            // A sign-in message was about the host as it was.
            if matches!(s.problem, Some(Problem::SignInRequired | Problem::TokenRefused)) {
                s.problem = None;
            }
            s.choose_host(key);
        }
        host
    }

    /// The box's text as an address, with the short SSH form opened on the GitHub,
    /// GitLab and Gitea servers of the remembered accounts too.
    pub fn explore_address(&self) -> Option<Address> {
        address_on(&self.explore.query, &forge_sites(self.ci.remembered()))
    }

    /// Run the box's search, or the next page of the shown one, on the chosen host.
    pub(crate) fn start_explore_search(&mut self, more: bool, interactive: bool) {
        let host = self.settle_explore_host();
        if !host.is_github() {
            self.start_forge_search(host, more);
            return;
        }
        self.start_github_search(more, interactive);
    }

    /// GitHub's search. Without a session, sign in silently once (or with the helper's
    /// window when `interactive`), and otherwise search public repositories without one.
    fn start_github_search(&mut self, more: bool, interactive: bool) {
        let s = &mut self.explore;
        if s.wait_seconds().is_some() && !interactive {
            return;
        }
        let Some(endpoint) = self.github_endpoint else {
            s.problem = Some(Problem::Failed(user_error(&Error::Hosting(HostingFailure::Disabled))));
            return;
        };
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let query = if more { s.searched.clone() } else { compose(&s.query, s.language) };
        if query.trim().is_empty() {
            return;
        }
        let page = if more { s.next_page.unwrap_or(1) } else { 1 };
        if !more {
            s.searched = query.clone();
            s.confirm_large = None;
        }
        s.problem = None;
        let sort = s.sort;
        let signed = self.github.clone();
        let anonymous = s.anonymous.clone();
        let try_sign_in = interactive || (!s.tried_sign_in && signed.is_none());
        s.tried_sign_in |= try_sign_in;
        match ReadTask::spawn(move |cancel| {
            let session = match (signed, try_sign_in) {
                (Some(session), false) => session,
                (_, true) => match backend.github_session(endpoint, interactive, &cancel) {
                    Ok(session) => Arc::new(session),
                    Err(Error::Hosting(
                        HostingFailure::SignInRequired | HostingFailure::Unauthorized,
                    )) if !interactive => match anonymous {
                        Some(session) => session,
                        None => Arc::new(backend.github_anonymous(endpoint)?),
                    },
                    Err(error) => return Err(error),
                },
                (None, false) => match anonymous {
                    Some(session) => session,
                    None => Arc::new(backend.github_anonymous(endpoint)?),
                },
            };
            let outcome = backend.github("github-search", &session, |g| {
                g.search_repositories(&query, sort, page, &cancel)
            });
            Ok(Done::GitHub { session, outcome })
        }) {
            Ok(task) => s.task = Some((task, more)),
            Err(error) => s.problem = Some(Problem::Failed(user_error(&error))),
        }
    }

    /// A GitLab or Gitea search: through the account's verified session, after signing
    /// the account in, or without an account on gitlab.com or codeberg.org. GitHub's
    /// search words are pointed out instead of sent.
    fn start_forge_search(&mut self, host: Host, more: bool) {
        let fixture = self.forge.fixture;
        let s = &mut self.explore;
        if s.wait_seconds().is_some() {
            return;
        }
        let query = if more { s.searched.clone() } else { s.query.trim().to_owned() };
        if query.is_empty() {
            return;
        }
        if !more {
            let words = github_words(&query);
            if !words.is_empty() {
                // A search still running would replace this with results for other words.
                s.cancel();
                s.problem = Some(Problem::GitHubWords(words.into_iter().map(str::to_owned).collect()));
                return;
            }
        }
        let Some(backend) = self.backend.clone() else {
            return;
        };
        /// A session that worked, or what to sign in with.
        enum Reach {
            Session(Arc<ForgeSession>),
            Account(ForgeConfig, String, String),
            Public(ForgeConfig),
        }
        let mut account = None;
        let reach = match &host.kind {
            HostKind::Account { .. } => {
                let key = host.key.trim_start_matches("account:").to_owned();
                if self.ci.token_refused(&key) {
                    // Not again until the account changes, as the check marks do.
                    self.explore.problem = Some(Problem::TokenRefused);
                    return;
                }
                let Some(a) = self.ci.remembered().iter().find(|a| crate::ci_checks::account_key(a) == key).cloned() else {
                    return;
                };
                account = Some(key.clone());
                match self.ci.session(&key) {
                    Some(session) => Reach::Session(session),
                    None => match crate::ci_checks::account_config(&a, fixture) {
                        Ok(config) => Reach::Account(config, a.username, a.token),
                        Err(error) => {
                            self.explore.problem = Some(Problem::Failed(user_error(&error)));
                            return;
                        }
                    },
                }
            }
            HostKind::Live => match &self.forge.session {
                Some(session) => Reach::Session(session.clone()),
                None => return,
            },
            HostKind::Public => match self.explore.public.get(&host.key) {
                Some(session) => Reach::Session(session.clone()),
                None => match host.config(fixture) {
                    Ok(config) => Reach::Public(config),
                    Err(error) => {
                        self.explore.problem = Some(Problem::Failed(user_error(&error)));
                        return;
                    }
                },
            },
            HostKind::GitHub => return,
        };
        let generation = self.ci.generation();
        let s = &mut self.explore;
        // Load more keeps the language and order of the list it adds to.
        let (language, sort) = if more {
            (s.searched_language, s.searched_sort)
        } else {
            (if host.provider.filters_language() { s.language } else { None }, s.forge_sort)
        };
        let page = if more { s.next_page.unwrap_or(1) } else { 1 };
        if !more {
            s.confirm_large = None;
        }
        s.problem = None;
        let key = host.key;
        match ReadTask::spawn(move |cancel| {
            let reached = match reach {
                Reach::Session(session) => Ok(session),
                Reach::Account(config, username, token) => backend.forge_connect(config, &username, &token, &cancel).map(Arc::new),
                Reach::Public(config) => backend.forge_public(config).map(Arc::new),
            };
            let (session, outcome) = match reached {
                Ok(session) => {
                    let outcome = backend.forge_request("explore-search", &session, |s| s.search_repositories(&query, sort, language, page, &cancel));
                    (Some(session), outcome)
                }
                Err(error) => (None, Err(error)),
            };
            Ok(Done::Forge(Box::new(ForgeDone { host: key, account, generation, session, query, language, sort, outcome })))
        }) {
            Ok(task) => s.task = Some((task, more)),
            Err(error) => s.problem = Some(Problem::Failed(user_error(&error))),
        }
    }

    pub(crate) fn poll_explore(&mut self, ctx: &Context) {
        let s = &mut self.explore;
        if s.wait_until.is_some() {
            if s.wait_seconds().is_some() {
                ctx.request_repaint_after(Duration::from_millis(250));
            } else {
                s.wait_until = None;
            }
        }
        let Some(result) = s.task.as_mut().and_then(|(task, _)| task.try_finish()) else {
            return;
        };
        let more = s.task.take().is_some_and(|(_, more)| more);
        let done = match result {
            Ok(done) => done,
            Err(Error::Cancelled) => return,
            Err(error) => {
                s.problem = Some(problem(&error));
                return;
            }
        };
        match done {
            Done::GitHub { session, outcome } => self.finish_github_search(session, outcome, more),
            Done::Forge(done) => self.finish_forge_search(*done, more),
        }
    }

    fn finish_forge_search(&mut self, done: ForgeDone, more: bool) {
        let ForgeDone { host, account, generation, session, query, language, sort, outcome } = done;
        // The account list changed while an account's search ran: its session, its
        // refusal and its answer are about an account as it was.
        if account.is_some() && generation != self.ci.generation() {
            return;
        }
        let refused = matches!(outcome, Err(Error::Forge(ForgeFailure::Unauthorized)));
        // Keep what worked, for the next search and for the commits' check marks.
        if let Some(session) = &session
            && !refused
        {
            match &account {
                Some(key) => self.ci.keep_session(key.clone(), session.clone()),
                None if !session.signed_in() => {
                    self.explore.public.insert(host.clone(), session.clone());
                }
                None => {}
            }
        }
        if let Some(key) = &account {
            match &outcome {
                Err(Error::Forge(ForgeFailure::Unauthorized)) => self.ci.refuse_token(key.clone()),
                Err(Error::Forge(ForgeFailure::Network | ForgeFailure::ServerUnavailable | ForgeFailure::TlsVerification) | Error::TimedOut) if session.is_none() => {
                    self.ci.refuse(key.clone(), Some(Instant::now() + crate::ci_checks::AFTER_ERROR));
                }
                _ => {}
            }
        }
        let s = &mut self.explore;
        // The same server's answer still counts when the choice moved to its account.
        if place(&host) != place(&s.host) {
            return;
        }
        // The list now shows this search, which Load more continues; a failed search
        // leaves the list it had, unless there was none.
        if !more && (matches!(outcome, Ok(RepositorySearchOutcome::Page(_))) || s.results.is_empty()) {
            s.searched = query;
            s.searched_language = language;
            s.searched_sort = sort;
        }
        match outcome {
            Ok(RepositorySearchOutcome::Page(page)) => {
                if !more {
                    s.results.clear();
                    s.selected = None;
                }
                for item in page.items {
                    if !s.results.iter().any(|r| r.path == item.path) {
                        s.results.push(Item::from_forge(item));
                    }
                }
                if !more && !s.results.is_empty() {
                    s.selected = Some(0);
                }
                s.total = page.total;
                s.incomplete = false;
                s.next_page = page.next_page;
                s.remaining = None;
                s.problem = s.results.is_empty().then_some(Problem::NoResults);
            }
            Ok(RepositorySearchOutcome::RateLimited { retry_after }) => {
                s.wait_until = Some(Instant::now() + Duration::from_secs(retry_after.unwrap_or(60).clamp(1, 3_600)));
            }
            Ok(RepositorySearchOutcome::SignInRequired) => s.problem = Some(Problem::SignInRequired),
            // Limited while signing in: wait a minute, as for a search the host limits.
            Err(Error::Forge(ForgeFailure::RateLimited)) => s.wait_until = Some(Instant::now() + Duration::from_secs(60)),
            Err(Error::Forge(ForgeFailure::Unauthorized)) => s.problem = Some(Problem::TokenRefused),
            Err(error) => s.problem = Some(problem(&error)),
        }
    }

    fn finish_github_search(&mut self, session: Arc<GitHubSession>, outcome: Result<GitHubSearchOutcome>, more: bool) {
        let s = &mut self.explore;
        if session.signed_in() {
            self.github = Some(session);
        } else {
            s.anonymous = Some(session);
        }
        let s = &mut self.explore;
        if !s.host.is_empty() {
            // Another host was chosen meanwhile: these results are not its.
            return;
        }
        match outcome {
            Ok(GitHubSearchOutcome::Page(page)) => {
                if !more {
                    s.results.clear();
                    s.selected = None;
                }
                for item in page.items {
                    let item = Item::from_github(item);
                    if !s.results.iter().any(|r| r.path == item.path) {
                        s.results.push(item);
                    }
                }
                if !more && !s.results.is_empty() {
                    s.selected = Some(0);
                }
                s.total = Some(page.total);
                s.incomplete = page.incomplete;
                s.next_page = page.next_page;
                s.remaining = page.remaining;
                s.problem = s.results.is_empty().then_some(Problem::NoResults);
            }
            Ok(GitHubSearchOutcome::RateLimited { reset_at, retry_after }) => {
                let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
                let wait = retry_after
                    .or_else(|| reset_at.map(|r| r.saturating_sub(now)))
                    .unwrap_or(60)
                    .clamp(1, 3_600);
                s.wait_until = Some(Instant::now() + Duration::from_secs(wait));
                s.remaining = Some(0);
            }
            Ok(GitHubSearchOutcome::Refused) => s.problem = Some(Problem::Refused),
            Err(error) => {
                if matches!(error, Error::Hosting(HostingFailure::Unauthorized)) {
                    // The stored sign-in stopped working: search without it next time.
                    self.github = None;
                }
                self.explore.problem = Some(problem(&error));
            }
        }
    }

    pub(crate) fn explore_page(&mut self, ui: &mut Ui) {
        let Some(backend_ready) = self.backend.as_ref().map(|_| true) else {
            return;
        };
        let can_clone = backend_ready && self.write.is_none();
        let signed = self.github.as_ref().map(|g| g.account().login.clone());
        let limit = self.preview_limit;
        let list = self.explore_hosts();
        let host = self.settle_explore_host();
        let typed = self.explore_address();
        // The chosen account and, once it signed in, who it is.
        let account = match &host.kind {
            HostKind::Account { label } => Some((label.clone(), self.ci.session(host.key.trim_start_matches("account:")).map(|s| s.login().to_owned()))),
            HostKind::Live => self.forge.session.as_ref().map(|s| (String::new(), Some(s.login().to_owned()))),
            _ => None,
        };
        let mut f = Flags::default();
        let widgets = &mut self.frame_widgets;
        let s = &mut self.explore;
        let running = s.task.is_some();
        let subtitle = if list.len() > 1 {
            tr("Search GitHub, GitLab or Gitea, or paste any repository address to preview it")
        } else {
            tr("Search GitHub, or paste any repository address to preview it")
        };
        pages::header_with(
            ui,
            |painter, tile| icon(painter, tile.shrink(10.0), Icon::Compass, BLUE()),
            tr("Explore"),
            subtitle,
            |ui| {
                let close = pages::ghost(ui, true, tr("Close"));
                control(widgets, "explore-close", &close, ui.clip_rect());
                f.close = close.clicked();
                match (&host.kind, &signed, &account) {
                    (HostKind::GitHub, Some(login), _) | (_, _, Some((_, Some(login)))) => {
                        ui.label(RichText::new(trf!("Signed in as {login}", login)).color(MUTED()));
                        dot(ui, GREEN());
                    }
                    (HostKind::GitHub, None, _) => {
                        let r = ui.add_enabled(!running, Button::new(RichText::new(tr("Sign in")).color(BLUE())).frame(false));
                        control(widgets, "explore-sign-in", &r, ui.clip_rect());
                        f.sign_in = r.on_hover_text(tr("Uses the github.com sign-in Git stores; it adds private repositories and allows 30 searches a minute")).clicked();
                        ui.label(RichText::new(tr("Not signed in · public repositories")).color(MUTED()));
                        dot(ui, HINT());
                    }
                    (HostKind::Public, ..) => {
                        let r = ui.add_enabled(!running, Button::new(RichText::new(tr("Add account")).color(BLUE())).frame(false));
                        control(widgets, "explore-add-account", &r, ui.clip_rect());
                        f.add_account = r.on_hover_text(trf!("Opens Preferences → Accounts for {site}", site = host.site())).clicked();
                        ui.label(RichText::new(tr("Not signed in · public repositories")).color(MUTED()));
                        dot(ui, HINT());
                    }
                    (_, _, Some((label, None))) if !label.is_empty() => {
                        ui.label(RichText::new(trf!("Signs in with {account}", account = label)).color(MUTED()));
                        dot(ui, HINT());
                    }
                    _ => {}
                }
            },
        );
        // The host, the search box, language and order.
        let languages = host.is_github() || host.provider.filters_language();
        Frame::new()
            .inner_margin(Margin { left: 24, right: 24, top: 0, bottom: 10 })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if list.len() > 1 {
                        host_menu(ui, &list, &host, !running, widgets, &mut f);
                    }
                    // The order and language controls take their own width from the right;
                    // the box gets the rest, so nothing pushes the page past the window.
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if host.is_github() {
                            for (sort, label, key) in [
                                (GitHubSearchSort::Updated, tr("Updated"), "explore-sort-updated"),
                                (GitHubSearchSort::Stars, tr("Stars"), "explore-sort-stars"),
                                (GitHubSearchSort::BestMatch, tr("Best match"), "explore-sort-best"),
                            ] {
                                let r = pages::choice(ui, true, s.sort == sort, label);
                                control(widgets, key, &r, ui.clip_rect());
                                if r.clicked() && s.sort != sort {
                                    f.sort = Some(sort);
                                }
                            }
                        } else {
                            for (sort, label, key) in [
                                (RepositorySearchSort::Updated, tr("Updated"), "explore-sort-updated"),
                                (RepositorySearchSort::Stars, tr("Stars"), "explore-sort-stars"),
                            ] {
                                let r = pages::choice(ui, true, s.forge_sort == sort, label);
                                control(widgets, key, &r, ui.clip_rect());
                                if r.clicked() && s.forge_sort != sort {
                                    f.forge_sort = Some(sort);
                                }
                            }
                        }
                        let mut language = s.language;
                        let shown = if languages { language.unwrap_or(tr("Any")) } else { tr("Any") };
                        let combo = ui.add_enabled_ui(languages, |ui| {
                            ComboBox::from_id_salt("explore-language")
                                .selected_text(trf!("Language: {language}", language = shown))
                                .width(150.0)
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut language, None, tr("Any"));
                                    for l in LANGUAGES {
                                        ui.selectable_value(&mut language, Some(l), l);
                                    }
                                })
                        });
                        let combo = combo.response.on_disabled_hover_text(trf!("{host} can't filter by language", host = host.name()));
                        control(widgets, "explore-language", &combo, ui.clip_rect());
                        if languages && language != s.language {
                            f.language = Some(language);
                        }
                        let waiting = s.wait_seconds();
                        let search = ui.add_enabled(
                            !running && waiting.is_none() && typed.is_none() && !s.query.trim().is_empty(),
                            Button::new(tr("Search")).min_size(vec2(0.0, 32.0)),
                        );
                        control(widgets, "explore-search", &search, ui.clip_rect());
                        f.search |= search.clicked();
                        let edit = ui.add_sized(
                            vec2(ui.available_width().max(160.0), 32.0),
                            TextEdit::singleline(&mut s.query)
                                .hint_text(trf!("Search {host} or paste a repository address", host = host.name()))
                                .align(Align2::LEFT_CENTER)
                                .margin(Margin::symmetric(10, 6)),
                        );
                        control(widgets, "explore-query", &edit, ui.clip_rect());
                        if std::mem::take(&mut s.focus) {
                            edit.request_focus();
                        }
                        if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && typed.is_none() {
                            f.search = true;
                        }
                    });
                });
                // One line under the box: counts, limits and problems.
                ui.add_space(8.0);
                if let Some(seconds) = s.wait_seconds() {
                    pages::callout(
                        ui,
                        AMBER(),
                        &trf!(
                            "{host}'s search limit is reached. You can search again in {wait}.",
                            host = host.name(),
                            wait = format!("{}:{:02}", seconds / 60, seconds % 60)
                        ),
                    );
                } else if let Some(problem) = &s.problem {
                    match problem {
                        Problem::Refused => pages::callout(ui, RED(), tr("GitHub can't run this search. Check user: and org: names, and use at most five AND, OR or NOT.")),
                        Problem::Failed(text) => pages::callout(ui, RED(), text),
                        Problem::SignInRequired => {
                            let text = trf!("{site} lets only signed-in users search.", site = host.site());
                            f.add_account |= callout_button(ui, RED(), &text, tr("Add account"), "explore-problem-add-account", widgets);
                        }
                        Problem::TokenRefused => {
                            let text = match &host.kind {
                                HostKind::Account { label } => trf!(
                                    "{site} refused the saved token for {account}. Update it in Preferences → Accounts.",
                                    site = host.site(),
                                    account = label
                                ),
                                _ => trf!("{site} refused the token. Update it in Preferences → Accounts.", site = host.site()),
                            };
                            f.accounts |= callout_button(ui, RED(), &text, tr("Open Accounts"), "explore-problem-accounts", widgets);
                        }
                        Problem::GitHubWords(words) => {
                            let mut text = trf!("{host} doesn't read GitHub's search words: {words}.", host = host.name(), words = words.join(", "));
                            if host.provider.filters_language() && words.iter().any(|w| w.trim_start_matches('-').to_ascii_lowercase().starts_with("language:")) {
                                text = format!("{text} {}", tr("Use the Language list for the language."));
                            }
                            f.without_words |= callout_button(ui, AMBER(), &text, tr("Search without them"), "explore-without-words", widgets);
                        }
                        Problem::Offline | Problem::NoResults => {}
                    }
                }
                if typed.is_none() && !s.results.is_empty() {
                    ui.horizontal(|ui| {
                        match s.total {
                            Some(total) => {
                                let total = total.min(9_999_999);
                                ui.label(RichText::new(trn!(total, "{n} repository", "{n} repositories")).color(TEXT()));
                            }
                            None if host.provider == ForgeProvider::GitLab => {
                                ui.label(RichText::new(tr("More than 10,000 repositories")).color(TEXT()));
                            }
                            None => {}
                        }
                        ui.label(RichText::new(trf!("· showing {n}", n = s.results.len())).color(HINT()));
                        if s.incomplete {
                            ui.label(RichText::new(tr("· GitHub stopped early; there may be more")).color(HINT()));
                        }
                        if let Some(left) = s.remaining.filter(|l| *l <= FEW_LEFT && s.wait_until.is_none()) {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(RichText::new(trn!(left, "{n} search left this minute", "{n} searches left this minute")).size(12.0).color(HINT()));
                            });
                        }
                    });
                }
            });
        if let Some(address) = &typed {
            address_card(ui, address, can_clone, widgets, &mut f);
        } else {
            pages::split(ui, |ui, side| match side {
                pages::Side::List => results_list(ui, s, &host, Some(limit), running, widgets, &mut f),
                pages::Side::Details => details(ui, s, &host, Some(limit), can_clone, widgets, &mut f),
            });
        }
        pages::details_scroll(ui, widgets, true);
        self.finish_explore_frame(f);
    }

    fn finish_explore_frame(&mut self, f: Flags) {
        if f.close {
            self.explore.cancel();
            self.dialog = None;
            return;
        }
        if let Some(key) = f.host {
            let key = if key == Host::github().key { String::new() } else { key };
            if key != self.explore.host {
                let s = &mut self.explore;
                s.cancel();
                // A host's countdown waits for it, while another host is searched.
                s.choose_host(key);
                s.results.clear();
                s.selected = None;
                s.total = None;
                s.next_page = None;
                s.incomplete = false;
                s.remaining = None;
                s.problem = None;
                s.searched.clear();
                s.confirm_large = None;
                // As Sort and Language do: the words in the box, searched on the new host.
                if !self.explore.query.trim().is_empty() && self.explore_address().is_none() {
                    self.start_explore_search(false, false);
                }
                return;
            }
        }
        if f.add_account {
            self.explore_add_account();
            return;
        }
        if f.accounts {
            self.show_explore_accounts();
            return;
        }
        if let Some(i) = f.select {
            self.explore.selected = Some(i);
            self.explore.confirm_large = None;
        }
        if let Some(i) = f.confirm {
            self.explore.confirm_large = Some(i);
        }
        if let Some(language) = f.language {
            self.explore.language = language;
            if !self.explore.query.trim().is_empty() {
                self.start_explore_search(false, false);
            }
        }
        if let Some(sort) = f.sort {
            self.explore.sort = sort;
            if !self.explore.query.trim().is_empty() {
                self.start_explore_search(false, false);
            }
        }
        if let Some(sort) = f.forge_sort {
            self.explore.forge_sort = sort;
            if !self.explore.query.trim().is_empty() {
                self.start_explore_search(false, false);
            }
        }
        if f.without_words {
            self.explore.query = without_github_words(&self.explore.query);
            self.explore.problem = None;
            if !self.explore.query.trim().is_empty() {
                self.start_explore_search(false, false);
            }
        }
        if f.stop {
            self.explore.cancel();
        } else if f.sign_in {
            self.start_explore_search(false, true);
        } else if f.search {
            self.start_explore_search(false, false);
        } else if f.more {
            self.start_explore_search(true, false);
        }
        if let Some(url) = f.open {
            self.open_host_page(&url);
        }
        if let Some(url) = f.clone {
            self.show_dialog(DialogKind::Clone);
            if let Some(d) = &mut self.dialog {
                d.first = url;
            }
            return;
        }
        if let Some((request, unlimited)) = f.preview {
            self.start_preview(request, unlimited);
        }
    }

    /// Preferences → Accounts with the chosen host's provider and server filled in, as
    /// Actions' Connect does, unless an account is being entered or connected there.
    fn explore_add_account(&mut self) {
        let host = self.explore_host();
        if !host.is_github() && self.forge.session.is_none() && self.forge.token.is_empty() {
            self.forge.provider = host.provider;
            self.forge.server = host.server;
        }
        self.show_explore_accounts();
    }

    /// Preferences → Accounts; closing Preferences comes back to Explore.
    fn show_explore_accounts(&mut self) {
        self.explore.cancel();
        self.dispatch(Intent::Preferences(crate::preferences::Page::Accounts));
        self.preferences.explore_return = true;
    }

    /// A repository's page on its host, in the browser: github.com pages through
    /// GitHub's own check; GitLab and Gitea pages are built from the configured server.
    pub(crate) fn open_host_page(&mut self, url: &str) {
        if github_web_url(url).is_some() {
            self.open_github(url);
        } else {
            self.open_reviewed_url(url, "explore-open");
        }
    }
}

/// The provider and server a host key names: `gitlab` and `https://gitlab.com` for
/// `account:gitlab|https://gitlab.com|Work`; empty for GitHub's.
fn place(key: &str) -> (&str, &str) {
    let mut parts = key.split_once(':').map_or("", |(_, rest)| rest).splitn(3, '|');
    (parts.next().unwrap_or(""), parts.next().unwrap_or(""))
}

fn problem(error: &Error) -> Problem {
    match error {
        Error::Hosting(HostingFailure::Network) | Error::Forge(ForgeFailure::Network | ForgeFailure::ServerUnavailable) => Problem::Offline,
        other => Problem::Failed(user_error(other)),
    }
}

fn dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(7.0, 7.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.5, color);
}

/// [`pages::callout`] with one button at its right end. Returns whether it was clicked.
fn callout_button(ui: &mut Ui, color: Color32, text: &str, button: &str, key: &str, widgets: &mut HashMap<String, UiControl>) -> bool {
    let mut clicked = false;
    Frame::new()
        .fill(color.gamma_multiply(0.08))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.3)))
        .corner_radius(8)
        .inner_margin(Margin { left: 12, right: 8, top: 6, bottom: 6 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let r = ui.add(Button::new(button).min_size(vec2(0.0, 28.0)));
                    control(widgets, key, &r, ui.clip_rect());
                    clicked = r.clicked();
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.add(Label::new(RichText::new(text).size(12.5).color(color)).wrap());
                    });
                });
            });
        });
    clicked
}

/// The host button at the start of the search row, and its menu: GitHub, the
/// accounts' servers, the hosts that need no account, and Manage accounts.
fn host_menu(
    ui: &mut Ui,
    list: &[Host],
    host: &Host,
    enabled: bool,
    widgets: &mut HashMap<String, UiControl>,
    f: &mut Flags,
) {
    let mut job = text::LayoutJob::default();
    job.append(host.name(), 0.0, TextFormat::simple(semibold(13.0), TEXT()));
    job.append(&format!("  {}", host.site()), 0.0, TextFormat::simple(FontId::proportional(12.5), HINT()));
    // Room for the chevron.
    job.append("     ", 0.0, TextFormat::simple(FontId::proportional(12.5), HINT()));
    let button = ui.add_enabled(enabled, Button::new(job).min_size(vec2(0.0, 32.0)));
    icon(
        ui.painter(),
        Rect::from_center_size(pos2(button.rect.right() - 13.0, button.rect.center().y), vec2(10.0, 10.0)),
        Icon::Chevron,
        FAINT(),
    );
    control(widgets, "explore-host", &button, ui.clip_rect());
    let button = button.on_hover_text(tr("Where to search"));
    Popup::menu(&button).width(380.0).show(|ui| {
        let mut row = |ui: &mut Ui, i: usize, h: &Host| {
            let detail = match &h.kind {
                HostKind::GitHub => tr("Git's github.com sign-in").to_owned(),
                HostKind::Account { label } => label.clone(),
                HostKind::Live => tr("Hosting's account").to_owned(),
                HostKind::Public => tr("Public repositories only").to_owned(),
            };
            let mut job = text::LayoutJob::default();
            job.append(h.name(), 0.0, TextFormat::simple(semibold(13.0), TEXT()));
            job.append(&format!("  {}", h.site()), 0.0, TextFormat::simple(FontId::proportional(12.5), MUTED()));
            job.append(&format!("  ·  {detail}"), 0.0, TextFormat::simple(FontId::proportional(12.0), HINT()));
            let on = h.key == host.key;
            let r = ui.add(Button::selectable(on, job).min_size(vec2(ui.available_width(), 30.0)));
            control(widgets, format!("explore-host-{i}"), &r, ui.clip_rect());
            if r.clicked() {
                f.host = Some(h.key.clone());
                ui.close();
            }
        };
        let caption = |ui: &mut Ui, text: &str| {
            ui.add_space(4.0);
            ui.label(RichText::new(text).size(11.0).color(HINT()).strong());
        };
        for (i, h) in list.iter().enumerate().filter(|(_, h)| h.is_github()) {
            row(ui, i, h);
        }
        if list.iter().any(|h| matches!(h.kind, HostKind::Account { .. } | HostKind::Live)) {
            caption(ui, tr("YOUR ACCOUNTS"));
            for (i, h) in list.iter().enumerate().filter(|(_, h)| matches!(h.kind, HostKind::Account { .. } | HostKind::Live)) {
                row(ui, i, h);
            }
        }
        if list.iter().any(|h| h.kind == HostKind::Public) {
            caption(ui, tr("NO ACCOUNT NEEDED"));
            for (i, h) in list.iter().enumerate().filter(|(_, h)| h.kind == HostKind::Public) {
                row(ui, i, h);
            }
        }
        ui.separator();
        let r = ui.add(Button::new(tr("Manage accounts")).frame(false).min_size(vec2(ui.available_width(), 28.0)));
        control(widgets, "explore-host-accounts", &r, ui.clip_rect());
        if r.clicked() {
            f.accounts = true;
            ui.close();
        }
    });
}

/// One result: namespace and name, pills, description, then stars, language, last
/// update and license, in a 70-point row. What the host doesn't list is left out.
fn result_row(ui: &mut Ui, item: &Item, selected: bool, limit: Option<u64>) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 70.0), Sense::click());
    let painter = ui.painter_at(rect);
    if selected {
        painter.rect_filled(rect, 8.0, SELECTED());
        painter.rect_filled(Rect::from_min_size(rect.min + vec2(0.0, 12.0), vec2(3.0, rect.height() - 24.0)), 1.5, BLUE());
    } else if response.hovered() {
        painter.rect_filled(rect, 8.0, HOVER());
    }
    let (owner, name) = item.owner_and_name();
    let tile = Rect::from_min_size(rect.min + vec2(12.0, 12.0), vec2(30.0, 30.0));
    painter.rect_filled(tile, 7.0, RAISED());
    let initial = item.path.chars().next().unwrap_or('?').to_uppercase().to_string();
    painter.text(tile.center(), Align2::CENTER_CENTER, initial, semibold(12.0), MUTED());
    let left = tile.right() + 12.0;
    let width = rect.right() - left - 12.0;
    let mut job = text::LayoutJob::default();
    if !owner.is_empty() {
        job.append(&format!("{owner} / "), 0.0, TextFormat::simple(FontId::proportional(13.0), MUTED()));
    }
    job.append(name, 0.0, TextFormat::simple(semibold(13.0), TEXT()));
    job.wrap = text::TextWrapping::truncate_at_width(width * 0.7);
    let title = painter.layout_job(job);
    let title_width = title.size().x;
    painter.galley(pos2(left, rect.top() + 9.0), title, TEXT());
    let mut x = left + title_width + 8.0;
    let mut pills: Vec<(String, Color32)> = Vec::new();
    if item.private {
        pills.push((tr("Private").into(), AMBER()));
    }
    if item.internal {
        pills.push((tr("Internal").into(), AMBER()));
    }
    if let Some(kb) = item.size_kb.filter(|kb| limit.is_some_and(|l| kb.saturating_mul(1024) > l)) {
        pills.push((size(kb.saturating_mul(1024)), AMBER()));
    }
    if item.archived {
        pills.push((tr("Archived").into(), FAINT()));
    }
    if item.fork {
        pills.push((tr("Fork").into(), VIOLET()));
    }
    if item.mirror {
        pills.push((tr("Mirror").into(), FAINT()));
    }
    for (text, color) in pills {
        let galley = painter.layout_no_wrap(text, semibold(11.0), color);
        let pill = Rect::from_min_size(pos2(x, rect.top() + 9.0), vec2(galley.size().x + 14.0, 18.0));
        if pill.right() > rect.right() - 8.0 {
            break;
        }
        painter.rect_filled(pill, 9.0, color.gamma_multiply(0.16));
        painter.galley(pos2(pill.left() + 7.0, pill.center().y - galley.size().y / 2.0), galley, color);
        x = pill.right() + 6.0;
    }
    let description = item.description.as_deref().unwrap_or(tr("No description"));
    let mut job = text::LayoutJob::simple_singleline(
        description.into(),
        FontId::proportional(12.5),
        if item.description.is_some() { TEXT() } else { HINT() },
    );
    job.wrap = text::TextWrapping::truncate_at_width(width);
    painter.galley(pos2(left, rect.top() + 29.0), painter.layout_job(job), TEXT());
    // Stars, language, last update, license.
    let mut x = left;
    let y = rect.top() + 49.0;
    let star = Rect::from_min_size(pos2(x, y), vec2(13.0, 13.0));
    icon(&painter, star, Icon::Star, FAINT());
    x += 17.0;
    let meta = |painter: &Painter, x: &mut f32, text: String| {
        let galley = painter.layout_no_wrap(text, FontId::proportional(12.0), FAINT());
        let w = galley.size().x;
        painter.galley(pos2(*x, y), galley, FAINT());
        *x += w + 16.0;
    };
    meta(&painter, &mut x, count(item.stars));
    if let Some(language) = &item.language {
        painter.circle_filled(pos2(x + 4.0, y + 7.0), 4.5, language_color(language));
        x += 13.0;
        meta(&painter, &mut x, language.clone());
    }
    if let Some(when) = updated(item.updated) {
        meta(&painter, &mut x, when);
    }
    if let Some(license) = &item.license {
        meta(&painter, &mut x, license.clone());
    }
    response
}

/// GitHub's usual colors for the common languages; others get a neutral dot.
fn language_color(language: &str) -> Color32 {
    match language {
        "Go" => Color32::from_rgb(0x00, 0xAD, 0xD8),
        "Rust" => Color32::from_rgb(0xDE, 0xA5, 0x84),
        "Python" => Color32::from_rgb(0x35, 0x72, 0xA5),
        "TypeScript" => Color32::from_rgb(0x31, 0x78, 0xC6),
        "JavaScript" => Color32::from_rgb(0xF1, 0xE0, 0x5A),
        "C++" => Color32::from_rgb(0xF3, 0x4B, 0x7D),
        "C" => Color32::from_rgb(0x55, 0x55, 0x55),
        "C#" => Color32::from_rgb(0x17, 0x86, 0x00),
        "Java" => Color32::from_rgb(0xB0, 0x72, 0x19),
        "Lua" => Color32::from_rgb(0x7B, 0x83, 0xF5),
        "Shell" => Color32::from_rgb(0x89, 0xE0, 0x51),
        "Ruby" => Color32::from_rgb(0x70, 0x15, 0x16),
        "Kotlin" => Color32::from_rgb(0xA9, 0x7B, 0xFF),
        "Swift" => Color32::from_rgb(0xF0, 0x51, 0x38),
        "PHP" => Color32::from_rgb(0x4F, 0x5D, 0x95),
        "HTML" => Color32::from_rgb(0xE3, 0x4C, 0x26),
        _ => FAINT(),
    }
}

fn results_list(
    ui: &mut Ui,
    s: &State,
    host: &Host,
    limit: Option<u64>,
    running: bool,
    widgets: &mut HashMap<String, UiControl>,
    f: &mut Flags,
) {
    if s.results.is_empty() {
        if running {
            pages::loading_with(ui, &trf!("Searching {host}", host = host.name()), 40.0, |ui| {
                let stop = ui.button(tr("Stop"));
                control(widgets, "explore-stop", &stop, ui.clip_rect());
                f.stop = stop.clicked();
            });
            return;
        }
        match &s.problem {
            Some(Problem::Offline) => {
                let place = if host.is_github() { host.name().to_owned() } else { host.site() };
                pages::loading_with(ui, &trf!("Can't reach {place}", place), 40.0, |ui| {
                    ui.label(RichText::new(tr("Check your connection, then search again.")).color(MUTED()));
                    let again = ui.button(tr("Try again"));
                    control(widgets, "explore-retry", &again, ui.clip_rect());
                    f.search = again.clicked();
                });
                return;
            }
            Some(Problem::NoResults) => {
                pages::empty(
                    ui,
                    if host.is_github() {
                        tr("No repositories match. Try fewer words, or remove a qualifier.")
                    } else {
                        tr("No repositories match. Try fewer words.")
                    },
                );
                return;
            }
            _ => {}
        }
        if s.searched.is_empty() {
            pages::empty(ui, &trf!("Search {host}'s repositories, or paste an address from any host.", host = host.name()));
        }
        return;
    }
    let footer = 44.0;
    let height = (ui.available_height() - footer).max(80.0);
    let output = ScrollArea::vertical()
        .id_salt("explore-results")
        .max_height(height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for (i, item) in s.results.iter().enumerate() {
                let r = result_row(ui, item, s.selected == Some(i), limit).on_hover_cursor(CursorIcon::PointingHand);
                control(widgets, format!("explore-result-{i}"), &r, ui.clip_rect());
                if r.clicked() {
                    f.select = Some(i);
                }
            }
        });
    pages::region(widgets, "explore-results-scroll", output.inner_rect.intersect(ui.clip_rect()), true);
    ui.horizontal(|ui| {
        ui.set_min_height(36.0);
        let shown = s.results.len() as u64;
        let text = match s.total {
            Some(total) if host.is_github() && s.next_page.is_none() && shown >= GITHUB_SEARCH_LIMIT.min(total) && total > GITHUB_SEARCH_LIMIT => {
                trf!("{shown} of {total} loaded · GitHub stops here. Narrow the search to see others.", shown, total)
            }
            Some(total) => trf!("{shown} of {total} loaded", shown, total = total.max(shown)),
            None => trf!("{shown} loaded", shown),
        };
        ui.label(RichText::new(text).size(12.0).color(HINT()));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if running {
                let stop = ui.button(tr("Stop"));
                control(widgets, "explore-stop", &stop, ui.clip_rect());
                f.stop = stop.clicked();
                ui.spinner();
            } else {
                let more = ui.add_enabled(s.next_page.is_some() && s.wait_until.is_none(), Button::new(tr("Load more")));
                control(widgets, "explore-more", &more, ui.clip_rect());
                f.more = more.clicked();
            }
        });
    });
}

fn details(
    ui: &mut Ui,
    s: &State,
    host: &Host,
    limit: Option<u64>,
    can_clone: bool,
    widgets: &mut HashMap<String, UiControl>,
    f: &mut Flags,
) {
    let Some((index, item)) = s.selected.and_then(|i| s.results.get(i).map(|r| (i, r))) else {
        pages::empty(ui, tr("Select a repository to see it here."));
        return;
    };
    let (owner, name) = item.owner_and_name();
    ui.horizontal_wrapped(|ui| {
        if !owner.is_empty() {
            ui.label(RichText::new(format!("{owner} / ")).size(15.0).color(MUTED()));
        }
        ui.label(RichText::new(name).font(semibold(15.0)));
    });
    if let Some(description) = &item.description {
        ui.label(RichText::new(description).color(TEXT()));
    }
    ui.add_space(4.0);
    // Wraps in a narrow window instead of widening the card.
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
        icon(ui.painter(), rect, Icon::Star, AMBER());
        ui.label(RichText::new(trn!(item.stars, "{count} star", "{count} stars", count = group(item.stars))).color(TEXT()));
        let rest = match item.open_issues {
            Some(issues) => trf!("· {forks} forks · {issues} open issues", forks = group(item.forks), issues = group(issues)),
            None => trf!("· {forks} forks", forks = group(item.forks)),
        };
        ui.label(RichText::new(rest).color(MUTED()));
    });
    ui.add_space(6.0);
    // GitLab's list names no language or license, so it gets no row for them.
    if host.provider != ForgeProvider::GitLab {
        pages::field(ui, tr("Language"), item.language.as_deref().unwrap_or(tr("Not stated")), false);
        pages::field(ui, tr("License"), item.license.as_deref().unwrap_or(tr("None stated")), false);
    }
    if let Some(branch) = &item.default_branch {
        pages::field(ui, tr("Default branch"), branch, false);
    }
    match item.size_kb {
        Some(kb) => pages::field(ui, &trf!("Size on {host}", host = host.name()), &size(kb.saturating_mul(1024)), false),
        None => pages::field(ui, tr("Size"), tr("Unknown until it downloads"), false),
    }
    if let Some(at) = item.updated {
        let label = match host.provider {
            ForgeProvider::GitLab => tr("Last activity"),
            ForgeProvider::Gitea | ForgeProvider::Forgejo => tr("Last update"),
            _ => tr("Last push"),
        };
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        let shown = format!("{} · {}", ago(now.saturating_sub(at).max(0) as u64, at), date(at));
        pages::field(ui, label, &shown, false);
    }
    if !item.topics.is_empty() {
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            for topic in &item.topics {
                pages::pill(ui, topic, BLUE());
            }
        });
    }
    ui.add_space(10.0);
    let large = item.size_kb.is_some_and(|kb| limit.is_some_and(|l| kb.saturating_mul(1024) > l));
    if large && s.confirm_large == Some(index) {
        pages::callout(
            ui,
            AMBER(),
            &trf!(
                "About {size} on {host}. Previews stop at {limit}, and its history alone may pass that.",
                size = size(item.size_kb.unwrap_or(0).saturating_mul(1024)),
                host = host.name(),
                limit = size(limit.unwrap_or(PREVIEW_LIMIT))
            ),
        );
        ui.horizontal_wrapped(|ui| {
            let anyway = pages::ghost(ui, true, tr("Preview anyway"));
            control(widgets, "explore-preview-anyway", &anyway, ui.clip_rect());
            if anyway.clicked() {
                f.preview = Some((request_for(item, host), true));
            }
            let clone = pages::primary(ui, can_clone, tr("Clone instead"));
            control(widgets, "explore-clone-instead", &clone, ui.clip_rect());
            if clone.clicked() {
                f.clone = Some(item.clone_url.clone());
            }
        });
        return;
    }
    ui.horizontal_wrapped(|ui| {
        let preview = pages::primary(ui, true, tr("Preview")).on_hover_text(tr("Opens a read-only copy in a new tab"));
        control(widgets, "explore-preview", &preview, ui.clip_rect());
        if preview.clicked() {
            if large {
                f.confirm = Some(index);
            } else {
                f.preview = Some((request_for(item, host), false));
            }
        }
        let clone = pages::ghost(ui, can_clone, tr("Clone"));
        control(widgets, "explore-clone", &clone, ui.clip_rect());
        if clone.clicked() {
            f.clone = Some(item.clone_url.clone());
        }
        let open = ui.add(Button::new(RichText::new(trf!("Open on {host}", host = host.name())).color(BLUE())).frame(false));
        control(widgets, "explore-open", &open, ui.clip_rect());
        if open.clicked() {
            f.open = Some(item.web_url.clone());
        }
    });
}

/// 18214 → "18,214".
pub(crate) fn group(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A typed or pasted address: one card with Preview and Clone, any host. The short SSH
/// form says what it opens as; on a server Rocket doesn't know it asks for ssh:// instead.
fn address_card(
    ui: &mut Ui,
    address: &Address,
    can_clone: bool,
    widgets: &mut HashMap<String, UiControl>,
    f: &mut Flags,
) {
    Frame::new()
        .inner_margin(Margin { left: 24, right: 24, top: 4, bottom: 12 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            Frame::new()
                .fill(PANEL())
                .stroke(Stroke::new(1.0, HAIRLINE()))
                .corner_radius(10)
                .inner_margin(Margin::symmetric(16, 14))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let (tile, _) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::hover());
                        ui.painter().rect_filled(tile, 7.0, RAISED());
                        icon(ui.painter(), tile.shrink(8.0), Icon::Link, MUTED());
                        ui.add_space(6.0);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.label(RichText::new(&address.name).font(semibold(14.0)));
                            ui.label(RichText::new(trf!("{host} · size unknown until it downloads", host = address.host)).size(12.0).color(HINT()));
                            if address.unknown_server {
                                ui.label(RichText::new(trf!("Type it as an ssh:// address on {host}", host = address.host)).size(12.0).color(AMBER()));
                            } else if address.short {
                                ui.label(RichText::new(trf!("Opens as {url}", url = address.url)).size(12.0).color(TEXT_SOFT()));
                            }
                        });
                        if !address.unknown_server {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let clone = pages::ghost(ui, can_clone, tr("Clone"));
                                control(widgets, "explore-address-clone", &clone, ui.clip_rect());
                                if clone.clicked() {
                                    f.clone = Some(address.url.clone());
                                }
                                let preview = pages::primary(ui, true, tr("Preview"));
                                control(widgets, "explore-address-preview", &preview, ui.clip_rect());
                                if preview.clicked() {
                                    f.preview = Some((request_from(address), false));
                                }
                            });
                        }
                    });
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Explore replaces the content area, with tabs open or not: its controls are
    /// drawn and the repository view's are not.
    #[test]
    fn explore_replaces_the_content_area() {
        crate::test_common::isolated(
            "ROCKET_EXPLORE_UI_CASE",
            "explore::tests::explore_replaces_the_content_area",
            || {
                let fixture = crate::test_common::Fixture::new();
                let ctx = Context::default();
                let mut app = Rocket::with_storage(&ctx, vec![fixture.repository.clone()], fixture.root.join("logs"), fixture.root.join("state"));
                let deadline = Instant::now() + Duration::from_secs(40);
                while app.busy() || app.tab().is_some_and(|t| t.snapshot.is_none()) {
                    app.poll(&ctx);
                    assert!(Instant::now() < deadline, "repository loads");
                    std::thread::sleep(Duration::from_millis(10));
                }
                let frame = |app: &mut Rocket| {
                    app.frame_widgets.clear();
                    let bounds = Rect::from_min_size(Pos2::ZERO, vec2(1480.0, 920.0));
                    ctx.run_ui(RawInput { screen_rect: Some(bounds), ..Default::default() }, |ui| app.shell(ui, &mut Vec::new()))
                        .drop_without_applying_deltas();
                };
                frame(&mut app);
                assert!(!app.frame_widgets.contains_key("explore-query"));
                // No query: a query searches at once, and tests never reach GitHub.
                app.github_endpoint = None;
                app.dispatch(Intent::Explore(None));
                frame(&mut app);
                assert!(app.frame_widgets.get("explore-query").is_some_and(|c| c.enabled));
                assert!(app.frame_widgets.contains_key("explore-search"));
                assert!(!app.frame_widgets.contains_key("search-kind"), "the commit list is hidden");
                assert!(app.explore.searched.is_empty() && !app.explore.running(), "opening doesn't search");
            },
        );
    }

    /// Results, the details pane and every control stay inside the window, from the
    /// narrowest supported width up: an overflowing search row once pushed the details
    /// off-screen, and the two columns were 8 points wider than the window.
    #[test]
    fn explore_fits_the_window() {
        crate::test_common::isolated(
            "ROCKET_EXPLORE_UI_CASE",
            "explore::tests::explore_fits_the_window",
            || {
                let fixture = crate::test_common::Fixture::new();
                let ctx = Context::default();
                let mut app = Rocket::with_storage(&ctx, vec![fixture.repository.clone()], fixture.root.join("logs"), fixture.root.join("state"));
                let deadline = Instant::now() + Duration::from_secs(40);
                while app.busy() || app.tab().is_some_and(|t| t.snapshot.is_none()) {
                    app.poll(&ctx);
                    assert!(Instant::now() < deadline, "repository loads");
                    std::thread::sleep(Duration::from_millis(10));
                }
                app.github_endpoint = None;
                // What the window's own chrome lays out at each width, without Explore.
                let mut baseline = Vec::new();
                for width in [780.0, 1100.0, 1480.0] {
                    let bounds = Rect::from_min_size(Pos2::ZERO, vec2(width, 920.0));
                    for _ in 0..2 {
                        app.frame_widgets.clear();
                        ctx.run_ui(RawInput { screen_rect: Some(bounds), ..Default::default() }, |ui| app.shell(ui, &mut Vec::new()))
                            .drop_without_applying_deltas();
                    }
                    baseline.push(ctx.globally_used_rect().right());
                }
                app.dispatch(Intent::Explore(None));
                let item = |name: &str, stars: u64| GitHubSearchItem {
                    remote: GitHubRemote::new("acme-tools", name).unwrap(),
                    description: Some("A fast fuzzy finder for the terminal, with a description long enough to wrap".into()),
                    stars,
                    forks: 812,
                    open_issues: 37,
                    language: Some("Rust".into()),
                    license: Some("MIT".into()),
                    topics: vec!["cli".into(), "search".into(), "fuzzy".into()],
                    default_branch: "main".into(),
                    size_kb: 5_000_000,
                    pushed_at: Some("2026-11-02T10:00:00Z".into()),
                    updated_at: Some("2026-11-02T10:00:00Z".into()),
                    private: true,
                    fork: true,
                    archived: true,
                    html_url: format!("https://github.com/acme-tools/{name}"),
                    clone_url: format!("https://github.com/acme-tools/{name}.git"),
                };
                app.explore.query = "fuzzy finder".into();
                app.explore.searched = "fuzzy finder".into();
                app.explore.results = (0..30).map(|i| Item::from_github(item(&format!("finder-with-a-long-name-{i}"), 18_214 - i))).collect();
                app.explore.total = Some(2_184);
                app.explore.next_page = Some(2);
                app.explore.selected = Some(0);
                app.explore.remaining = Some(4);
                let fits = |app: &mut Rocket, keys: &[&str]| {
                    for (width, chrome) in [780.0, 1100.0, 1480.0].into_iter().zip(baseline.iter().copied()) {
                        let bounds = Rect::from_min_size(Pos2::ZERO, vec2(width, 920.0));
                        for _ in 0..2 {
                            app.frame_widgets.clear();
                            ctx.run_ui(RawInput { screen_rect: Some(bounds), ..Default::default() }, |ui| app.shell(ui, &mut Vec::new()))
                                .drop_without_applying_deltas();
                        }
                        for key in keys {
                            assert!(app.frame_widgets.contains_key(*key), "{key} at {width}");
                        }
                        // Control rectangles are clipped to the window, so check what was laid out.
                        let used = ctx.globally_used_rect();
                        assert!(chrome <= width + 0.5, "the window without Explore fits at {width}");
                        assert!(used.right() <= width + 0.5, "Explore lays out to {} in a {width} window", used.right());
                    }
                };
                fits(&mut app, &["explore-host", "explore-query", "explore-search", "explore-sort-updated", "explore-preview", "explore-clone", "explore-close", "explore-sign-in", "explore-more"]);
                // GitLab without an account: subgroups, no language, sizes unknown.
                let gitlab = app.explore_hosts().into_iter().find(|h| h.kind == HostKind::Public && h.provider == ForgeProvider::GitLab).unwrap();
                app.explore.host = gitlab.key.clone();
                app.explore.total = None;
                for (i, r) in app.explore.results.iter_mut().enumerate() {
                    r.path = format!("acme/tools/with-a-long-subgroup-name/finder-{i}");
                    (r.language, r.license, r.size_kb) = (None, None, None);
                }
                fits(&mut app, &["explore-host", "explore-query", "explore-add-account", "explore-sort-stars", "explore-language", "explore-preview", "explore-open"]);
                assert!(app.frame_widgets.get("explore-language").is_some_and(|c| c.enabled), "GitLab filters by language");
                let codeberg = app.explore_hosts().into_iter().find(|h| h.kind == HostKind::Public && h.provider == ForgeProvider::Forgejo).unwrap();
                app.explore.host = codeberg.key;
                fits(&mut app, &["explore-host", "explore-language"]);
                assert!(app.frame_widgets.get("explore-language").is_some_and(|c| !c.enabled), "Forgejo can't filter by language");
                assert!(!app.frame_widgets.contains_key("explore-sort-best"), "relevance is GitHub's");
                // German's longer words still fit, on GitHub with its three orders too.
                crate::i18n::scoped(crate::i18n::Language::German, || {
                    fits(&mut app, &["explore-host", "explore-language", "explore-search"]);
                    app.explore.host.clear();
                    fits(&mut app, &["explore-host", "explore-sort-best", "explore-sign-in"]);
                });
            },
        );
    }

    /// GitHub's search words typed for GitLab are pointed out, not sent: no search starts.
    #[test]
    fn github_words_stay_on_github() {
        crate::test_common::isolated(
            "ROCKET_EXPLORE_UI_CASE",
            "explore::tests::github_words_stay_on_github",
            || {
                let fixture = crate::test_common::Fixture::new();
                let ctx = Context::default();
                let mut app = Rocket::with_storage(&ctx, vec![], fixture.root.join("logs"), fixture.root.join("state"));
                let gitlab = app.explore_hosts().into_iter().find(|h| h.provider == ForgeProvider::GitLab).unwrap();
                app.explore.host = gitlab.key;
                app.explore.query = "fuzzy language:go stars:>100".into();
                app.start_explore_search(false, false);
                assert!(!app.explore.running(), "nothing is sent");
                assert_eq!(app.explore.problem, Some(Problem::GitHubWords(vec!["language:go".into(), "stars:>100".into()])));
                assert_eq!(without_github_words(&app.explore.query), "fuzzy");
            },
        );
    }

    #[test]
    fn addresses_are_told_from_searches() {
        let gh = address("https://github.com/Acme/Tool/tree/main/src").unwrap();
        assert_eq!(gh.name, "Acme/Tool");
        assert_eq!(gh.url, "https://github.com/Acme/Tool.git");
        assert_eq!(gh.web.as_deref(), Some("https://github.com/Acme/Tool"));
        let lab = address("https://gitlab.example.com/group/sub/tool.git").unwrap();
        assert_eq!((lab.name.as_str(), lab.host.as_str()), ("group/sub/tool", "gitlab.example.com"), "the whole path names it");
        assert!(matches!(lab.source, CloneSource::Url(_)));
        let page = address("https://gitlab.com/acme/tools/fuzzyfind/-/tree/main/src").unwrap();
        assert_eq!((page.name.as_str(), page.url.as_str()), ("acme/tools/fuzzyfind", "https://gitlab.com/acme/tools/fuzzyfind"), "GitLab's page ends the project at /-/");
        let line = address("https://gitlab.com/acme/tools/fuzzyfind/-/blob/main/README.md?ref_type=heads#L10").unwrap();
        assert_eq!((line.name.as_str(), line.url.as_str()), ("acme/tools/fuzzyfind", "https://gitlab.com/acme/tools/fuzzyfind"), "a copied line link");
        assert_eq!(address("https://gitlab.com/acme/x/-").unwrap().url, "https://gitlab.com/acme/x");
        assert!(address("https://host.example/a/b?x=1").is_none(), "a query outside a GitLab page isn't an address");
        let ssh = address("ssh://git@host.example:2222/team/app.git").unwrap();
        assert_eq!((ssh.name.as_str(), ssh.host.as_str()), ("team/app", "host.example"));
        for search in ["fuzzy finder", "language:rust stars:>100", "https://", "https://host", "C:\\missing\\folder", "C:/missing/folder"] {
            assert!(address(search).is_none(), "{search}");
        }
        assert!(address("https://host.example/a b").is_none());
    }

    /// GitLab's and Gitea's short SSH address opens as ssh:// on the hosts Rocket knows;
    /// elsewhere its path may start in the home folder, so the card asks for ssh://.
    #[test]
    fn short_ssh_addresses_open_on_known_hosts() {
        let lab = address("git@gitlab.com:acme/tools/fuzzyfind.git").unwrap();
        assert_eq!((lab.name.as_str(), lab.host.as_str(), lab.url.as_str()), ("acme/tools/fuzzyfind", "gitlab.com", "ssh://git@gitlab.com/acme/tools/fuzzyfind.git"));
        assert!(lab.short && !lab.unknown_server);
        assert_eq!(lab.source, CloneSource::Url("ssh://git@gitlab.com/acme/tools/fuzzyfind.git".into()));
        let hub = address("git@GitHub.com:Acme/Tool.git").unwrap();
        assert_eq!((hub.url.as_str(), hub.web.as_deref()), ("ssh://git@github.com/Acme/Tool.git", Some("https://github.com/Acme/Tool")));
        let unknown = address("git@git.example.org:team/app.git").unwrap();
        assert!(unknown.short && unknown.unknown_server, "a server Rocket doesn't know");
        let known = address_on("git@git.example.org:team/app.git", &["git.example.org".into()]).unwrap();
        assert!(!known.unknown_server, "a server of a remembered account");
        let rooted = address("deploy@build.example:/srv/git/app.git").unwrap();
        assert_eq!((rooted.url.as_str(), rooted.unknown_server), ("ssh://deploy@build.example/srv/git/app.git", false), "a path from the root means the same");
        for search in ["user@host:", "@host:repo", "git@host:a b", "a/b@host:repo", "git@-host:repo", "name:value"] {
            assert!(address(search).is_none_or(|a| !a.short), "{search}");
        }
    }

    /// Choosing another host keeps the limited host's countdown for when it comes back.
    #[test]
    fn countdowns_stay_with_their_host() {
        let mut s = State { host: "public:gitlab|https://gitlab.com".into(), wait_until: Some(Instant::now() + Duration::from_secs(50)), ..Default::default() };
        s.choose_host("public:forgejo|https://codeberg.org".into());
        assert!(s.wait_until.is_none(), "Codeberg isn't limited");
        s.choose_host("public:gitlab|https://gitlab.com".into());
        assert!(s.wait_seconds().is_some_and(|w| w > 40), "gitlab.com's countdown comes back");
        s.choose_host("account:gitlab|https://gitlab.com|Work".into());
        assert!(s.wait_until.is_none(), "the account has its own limit");
    }

    fn gitlab_account(label: &str) -> crate::hosting_accounts::Account {
        crate::hosting_accounts::Account {
            label: label.into(),
            provider: ForgeProvider::GitLab,
            server: "https://gitlab.com".into(),
            username: String::new(),
            token: "token".into(),
            azure_api_version: None,
        }
    }

    /// The choice follows gitlab.com to its one account but doesn't guess between two;
    /// an account's answer from before the account list changed is dropped; a refused
    /// token is kept apart from other failures; a limit while signing in counts down.
    #[test]
    fn answers_follow_the_account_list() {
        crate::test_common::isolated(
            "ROCKET_EXPLORE_UI_CASE",
            "explore::tests::answers_follow_the_account_list",
            || {
                let fixture = crate::test_common::Fixture::new();
                let ctx = Context::default();
                let mut app = Rocket::with_storage(&ctx, vec![], fixture.root.join("logs"), fixture.root.join("state"));
                app.explore.host = "public:gitlab|https://gitlab.com".into();
                app.ci.remember_for_test(vec![gitlab_account("Work")]);
                assert_eq!(app.explore_host().key, "account:gitlab|https://gitlab.com|Work", "gitlab.com moves to its account");
                app.explore.host = "account:gitlab|https://gitlab.com|Personal".into();
                app.ci.remember_for_test(vec![gitlab_account("Work"), gitlab_account("Home")]);
                assert!(app.explore_host().is_github(), "two accounts on one server: no guess");

                let key = "gitlab|https://gitlab.com|Work".to_owned();
                app.ci.remember_for_test(vec![gitlab_account("Work")]);
                app.explore.host = format!("account:{key}");
                let done = |generation: u64, outcome: Result<RepositorySearchOutcome>| ForgeDone {
                    host: format!("account:{key}"),
                    account: Some(key.clone()),
                    generation,
                    session: None,
                    query: "fuzzy".into(),
                    language: None,
                    sort: RepositorySearchSort::Stars,
                    outcome,
                };
                let stale = app.ci.generation() - 1;
                app.finish_forge_search(done(stale, Err(Error::Forge(ForgeFailure::Unauthorized))), false);
                assert!(!app.ci.token_refused(&key) && app.explore.problem.is_none(), "from before the list changed");
                app.finish_forge_search(done(app.ci.generation(), Err(Error::Forge(ForgeFailure::NotFound))), false);
                assert!(!app.ci.token_refused(&key), "a wrong path is not a refused token");
                app.finish_forge_search(done(app.ci.generation(), Err(Error::Forge(ForgeFailure::Unauthorized))), false);
                assert!(app.ci.token_refused(&key));
                assert_eq!(app.explore.problem, Some(Problem::TokenRefused));
                assert_eq!(app.explore.searched, "fuzzy", "an empty list takes the search it answers");
                app.explore.problem = None;
                app.finish_forge_search(done(app.ci.generation(), Err(Error::Forge(ForgeFailure::RateLimited))), false);
                assert!(app.explore.wait_seconds().is_some(), "a limit while signing in counts down");
            },
        );
    }

    /// A host key names its server, so an account added for gitlab.com can take the
    /// place of gitlab.com without one.
    #[test]
    fn host_keys_name_their_server() {
        assert_eq!(place("account:gitlab|https://gitlab.com|Work|team"), ("gitlab", "https://gitlab.com"));
        assert_eq!(place("public:forgejo|https://codeberg.org"), ("forgejo", "https://codeberg.org"));
        assert_eq!(place("live:gitlab|https://gitlab.example.com/gitlab"), ("gitlab", "https://gitlab.example.com/gitlab"));
        assert_eq!((place(""), place("github")), (("", ""), ("", "")));
    }

    #[test]
    fn dates_are_utc_days() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(1_793_613_600), "2026-11-02");
        assert_eq!(date(951_782_400), "2000-02-29");
    }

    #[test]
    fn queries_counts_sizes_and_times_read_as_words() {
        assert_eq!(compose(" fuzzy ", Some("Rust")), "fuzzy language:Rust");
        assert_eq!(compose("fuzzy language:go", Some("Rust")), "fuzzy language:go");
        assert_eq!(compose("x", None), "x");
        assert_eq!((count(640), count(18_214), count(1_000), count(1_260_000)), ("640".into(), "18.2k".into(), "1k".into(), "1.3M".into()));
        assert_eq!(group(18_214), "18,214");
        assert_eq!((size(500), size(41 * 1024 * 1024), size(5_261_334_938)), ("1 KB".into(), "41 MB".into(), "4.9 GB".into()));
        assert_eq!(unix("1970-01-02T00:00:01Z"), Some(86_401));
        assert_eq!(unix("2026-11-02T10:00:00Z"), Some(1_793_613_600));
        assert_eq!(unix("bad"), None);
    }
}
