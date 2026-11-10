//! Read-only github.com integration. Sign-in reuses the github.com credential the
//! configured Git credential helper (normally Git Credential Manager) stores for
//! HTTPS Git. Requests go only to `https://api.github.com` through the system's
//! curl: Windows' System32 `curl.exe`, or `/usr/bin/curl` on Linux. The token is
//! passed on standard input, kept in memory and never logged. Nothing on the
//! account is changed.
use crate::{
    runner::{Output, Runner},
    *,
};
use std::{
    ffi::OsString,
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
pub(crate) mod json;
mod neutral;
mod urls;
use json::Json;
pub(crate) use neutral::{NeutralDirectory, neutral_directory};
pub use urls::{GitHubRemote, github_web_url};

/// Repositories per page (GitHub's maximum).
pub const GITHUB_REPOSITORY_PAGE: usize = 100;
/// Search results per page.
pub const GITHUB_SEARCH_PAGE: usize = 30;
/// GitHub returns at most this many results for any search.
pub const GITHUB_SEARCH_LIMIT: u64 = 1_000;
/// Notifications per page.
pub const GITHUB_NOTIFICATION_PAGE: usize = 50;
const RESPONSE_LIMIT: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostingFailure {
    SignInRequired,
    Unauthorized,
    Forbidden,
    RateLimited,
    NotFound,
    Network,
    InvalidResponse,
    Unavailable,
    Disabled,
}
impl HostingFailure {
    pub fn code(self) -> &'static str {
        match self {
            Self::SignInRequired => "github_sign_in_required",
            Self::Unauthorized => "github_unauthorized",
            Self::Forbidden => "github_forbidden",
            Self::RateLimited => "github_rate_limited",
            Self::NotFound => "github_not_found",
            Self::Network => "github_network",
            Self::InvalidResponse => "github_invalid_response",
            Self::Unavailable => "github_unavailable",
            Self::Disabled => "github_disabled",
        }
    }
    pub fn guidance(self) -> &'static str {
        match self {
            Self::SignInRequired => {
                "Git's credential helper has no github.com sign-in. Choose Sign in to let Git Credential Manager sign you in for this Rocket session, or sign in once through a Git HTTPS fetch or push to github.com so Git stores it."
            }
            Self::Unauthorized => {
                "GitHub rejected the stored github.com credential, or it is not an access token. Rocket does not remove stored credentials: remove the git:https://github.com entry in Windows Credential Manager, then choose Sign in."
            }
            Self::Forbidden => {
                "GitHub refused this request. The sign-in may lack the needed access (for example organization SSO approval or scopes). Nothing was changed."
            }
            Self::RateLimited => {
                "GitHub's API rate limit was reached. Wait for it to reset, then refresh."
            }
            Self::NotFound => {
                "GitHub did not find this repository with the current sign-in. It may be private to another account, renamed or deleted."
            }
            Self::Network => {
                "GitHub could not be reached. Check the network connection and proxy settings, then refresh."
            }
            Self::InvalidResponse => {
                "GitHub returned a response Rocket could not read. Nothing was changed; refresh to retry."
            }
            Self::Unavailable if cfg!(target_os = "linux") => {
                "GitHub requests need the system's /usr/bin/curl, owned by root and changeable by no one else. Install your distribution's curl package. GitHub features are unavailable; Git itself is unaffected."
            }
            Self::Unavailable if cfg!(target_os = "macos") => {
                "macOS's trusted /usr/bin/curl could not be verified. Check your macOS installation. GitHub features are unavailable; Git itself is unaffected."
            }
            Self::Unavailable => {
                "Windows' curl.exe, used for GitHub requests, was not found. GitHub features are unavailable; Git itself is unaffected."
            }
            Self::Disabled => "GitHub access is disabled in this window.",
        }
    }
}
fn fail<T>(failure: HostingFailure) -> Result<T> {
    Err(Error::Hosting(failure))
}

/// Where API requests go: github.com, or a loopback test server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitHubEndpoint {
    Public,
    /// Only plain HTTP on 127.0.0.1, for automated tests.
    Loopback(u16),
}
impl GitHubEndpoint {
    /// Accept `http://127.0.0.1:<port>` or `http://localhost:<port>` only.
    pub fn loopback(url: &str) -> Option<Self> {
        let rest = url.strip_prefix("http://")?.trim_end_matches('/');
        let port = rest
            .strip_prefix("127.0.0.1:")
            .or_else(|| rest.strip_prefix("localhost:"))?;
        port.parse().ok().filter(|p| *p != 0).map(Self::Loopback)
    }
    fn base(self) -> String {
        match self {
            Self::Public => "https://api.github.com".into(),
            Self::Loopback(port) => format!("http://127.0.0.1:{port}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubAccount {
    pub login: String,
    pub name: Option<String>,
    pub html_url: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubRepository {
    pub remote: GitHubRemote,
    pub description: Option<String>,
    pub private: bool,
    pub fork: bool,
    pub archived: bool,
    pub default_branch: String,
    pub html_url: String,
    pub clone_url: String,
    pub ssh_url: String,
    pub pushed_at: Option<String>,
    /// The upstream repository of a fork, with its default branch; only present
    /// when one repository is read directly.
    pub parent: Option<(GitHubRemote, String)>,
}
/// One repository in GitHub's search results.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubSearchItem {
    pub remote: GitHubRemote,
    pub description: Option<String>,
    pub stars: u64,
    pub forks: u64,
    pub open_issues: u64,
    pub language: Option<String>,
    /// The SPDX identifier, or the license's name when it has none.
    pub license: Option<String>,
    pub topics: Vec<String>,
    pub default_branch: String,
    /// GitHub's size of the whole repository, in kilobytes.
    pub size_kb: u64,
    pub pushed_at: Option<String>,
    pub updated_at: Option<String>,
    pub private: bool,
    pub fork: bool,
    pub archived: bool,
    pub html_url: String,
    pub clone_url: String,
}

/// How GitHub orders search results.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GitHubSearchSort {
    #[default]
    BestMatch,
    Stars,
    Updated,
}

/// One page of a repository search, with GitHub's own count of searches left.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubSearchPage {
    pub total: u64,
    /// GitHub stopped early (a timeout); the results it found are still here.
    pub incomplete: bool,
    pub items: Vec<GitHubSearchItem>,
    pub next_page: Option<u32>,
    pub remaining: Option<u64>,
    /// When the search limit resets, in Unix seconds.
    pub reset_at: Option<u64>,
}

/// What a search returned. A rate limit and a refused query are answers, not errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHubSearchOutcome {
    Page(GitHubSearchPage),
    /// Wait until `reset_at` (Unix seconds) or for `retry_after` seconds.
    RateLimited { reset_at: Option<u64>, retry_after: Option<u64> },
    /// GitHub's 422: too long, too many AND/OR/NOT, or an unknown user or organization.
    Refused,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubNotification {
    pub id: String,
    pub repository: GitHubRemote,
    pub title: String,
    pub kind: String,
    pub reason: String,
    pub unread: bool,
    pub updated_at: String,
    pub web_url: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubPullRequest {
    pub number: u64,
    pub title: String,
    pub html_url: String,
    pub draft: bool,
    pub base: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubList<T> {
    pub items: Vec<T>,
    /// The next page to request, from GitHub's `Link` header.
    pub next_page: Option<u32>,
}

/// A signed-in, read-only connection. Debug output and errors never include the
/// token.
#[derive(Clone)]
pub struct GitHubSession {
    runner: Runner,
    curl: PathBuf,
    cwd: NeutralDirectory,
    endpoint: GitHubEndpoint,
    /// None for a session without sign-in: public data only, at GitHub's lower limits.
    token: Option<Arc<str>>,
    account: GitHubAccount,
}
impl fmt::Debug for GitHubSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitHubSession")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

/// On Windows use only the OS-selected System32 curl, and on Linux/macOS the system's
/// /usr/bin/curl (see `system_program`). Environment variables, PATH and a helper
/// beside Git must not redirect credential-bearing hosting requests.
pub(crate) fn find_curl(_git: &Path) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        use std::os::windows::{ffi::OsStringExt, fs::MetadataExt};
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetSystemDirectoryW(buffer: *mut u16, size: u32) -> u32;
        }
        let mut buffer = vec![0u16; 32768];
        // The OS API writes at most buffer.len() UTF-16 units, including its NUL.
        let length =
            unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
        if length == 0 || length >= buffer.len() {
            return None;
        }
        let path =
            PathBuf::from(std::ffi::OsString::from_wide(&buffer[..length])).join("curl.exe");
        std::fs::symlink_metadata(&path)
            .is_ok_and(|meta| meta.is_file() && meta.file_attributes() & 0x400 == 0)
            .then_some(path)
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        system_program("curl")
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        // Unsupported platforms must not guess a credential-bearing helper.
        None
    }
}
/// Linux/macOS: a program from /usr/bin only, not a link, owned by root in
/// a folder root owns, and writable by no one else, as Windows runs only its System32
/// curl. A copy on PATH, beside Git or in the user's folders is never used. Hosting,
/// extension requests and app updates all take their programs from here.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn system_program(name: &str) -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let folder = Path::new("/usr/bin");
    let path = folder.join(name);
    let protected = |path: &Path| {
        std::fs::symlink_metadata(path)
            .is_ok_and(|meta| meta.uid() == 0 && meta.mode() & 0o022 == 0)
    };
    let file = std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_file());
    (file && protected(&path) && protected(folder)).then_some(path)
}
fn token_text(password: &str) -> Option<&str> {
    (!password.is_empty()
        && password.len() <= 255
        && password
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')))
    .then_some(password)
}

impl GitClient {
    /// Sign in with the github.com credential stored by Git's credential helper,
    /// then read the account. Unless `interactive`, the helper must not prompt.
    pub fn github_session(
        &self,
        endpoint: GitHubEndpoint,
        interactive: bool,
        cancel: &CancellationToken,
    ) -> Result<GitHubSession> {
        let cwd = neutral_directory()?;
        let curl = find_curl(&self.runner.executable)
            .ok_or(Error::Hosting(HostingFailure::Unavailable))?;
        let token = self.runner.step("github-sign-in", "read_credential", || {
            let mut runner = self.runner.clone();
            // Git Credential Manager is a separate .NET helper that may contact
            // GitHub; a cold start on a busy machine took 10-30 s without any
            // prompt. Interactive sign-in leaves time for its own sign-in window.
            runner.limits.timeout = Duration::from_secs(if interactive { 300 } else { 60 });
            runner.limits.stdout_bytes = 64 * 1024;
            let output = runner.credential(
                &cwd,
                interactive,
                b"protocol=https\nhost=github.com\n\n".to_vec(),
                cancel,
            )?;
            if !output.status.success() {
                return fail(HostingFailure::SignInRequired);
            }
            let text = String::from_utf8(output.stdout)
                .map_err(|_| Error::Hosting(HostingFailure::InvalidResponse))?;
            let password = text
                .lines()
                .find_map(|line| line.strip_prefix("password="))
                .ok_or(Error::Hosting(HostingFailure::SignInRequired))?;
            let token = token_text(password).ok_or(Error::Hosting(HostingFailure::Unauthorized))?;
            Ok(Arc::<str>::from(token))
        })?;
        let mut runner = self.runner.clone();
        runner.limits = CommandLimits {
            timeout: Duration::from_secs(60),
            stdout_bytes: RESPONSE_LIMIT,
            stderr_bytes: 64 * 1024,
        };
        let mut session = GitHubSession {
            runner,
            curl,
            cwd,
            endpoint,
            token: Some(token),
            account: GitHubAccount {
                login: String::new(),
                name: None,
                html_url: String::new(),
            },
        };
        let user = session
            .get("github-sign-in", "read_github_user", "/user", cancel)?
            .0;
        session.account = account(&user).ok_or(Error::Hosting(HostingFailure::InvalidResponse))?;
        Ok(session)
    }
}

impl GitClient {
    /// A session without sign-in, for searching public repositories. No credential
    /// helper runs and no account is read.
    pub fn github_anonymous(&self, endpoint: GitHubEndpoint) -> Result<GitHubSession> {
        let cwd = neutral_directory()?;
        let curl = find_curl(&self.runner.executable)
            .ok_or(Error::Hosting(HostingFailure::Unavailable))?;
        let mut runner = self.runner.clone();
        runner.limits = CommandLimits {
            timeout: Duration::from_secs(60),
            stdout_bytes: RESPONSE_LIMIT,
            stderr_bytes: 64 * 1024,
        };
        Ok(GitHubSession {
            runner,
            curl,
            cwd,
            endpoint,
            token: None,
            account: GitHubAccount {
                login: String::new(),
                name: None,
                html_url: String::new(),
            },
        })
    }
}

fn account(user: &Json) -> Option<GitHubAccount> {
    let login = user.str("login")?;
    GitHubRemote::new(login, "x")?;
    Some(GitHubAccount {
        login: login.into(),
        name: user.str("name").filter(|n| !n.is_empty()).map(Into::into),
        html_url: user
            .str("html_url")
            .and_then(github_web_url)
            .map_or_else(|| format!("https://github.com/{login}"), Into::into),
    })
}
fn repository(item: &Json) -> Option<GitHubRepository> {
    let owner = item.get("owner")?.str("login")?;
    let remote = GitHubRemote::new(owner, item.str("name")?)?;
    let parent = item.get("parent").and_then(|p| {
        let remote = GitHubRemote::new(p.get("owner")?.str("login")?, p.str("name")?)?;
        Some((remote, p.str("default_branch")?.to_owned()))
    });
    Some(GitHubRepository {
        description: item
            .str("description")
            .filter(|d| !d.is_empty())
            .map(Into::into),
        private: item.bool("private").unwrap_or(false),
        fork: item.bool("fork").unwrap_or(false),
        archived: item.bool("archived").unwrap_or(false),
        default_branch: item.str("default_branch").unwrap_or("main").into(),
        html_url: github_web_url(item.str("html_url")?)
            .map_or_else(|| remote.web_url(), Into::into),
        clone_url: format!("https://github.com/{}.git", remote.full_name()),
        ssh_url: format!("git@github.com:{}.git", remote.full_name()),
        pushed_at: item.str("pushed_at").map(Into::into),
        parent,
        remote,
    })
}
fn search_item(item: &Json, endpoint: GitHubEndpoint) -> Option<GitHubSearchItem> {
    let owner = item.get("owner")?.str("login")?;
    let remote = GitHubRemote::new(owner, item.str("name")?)?;
    let text = |key: &str, limit: usize| {
        item.str(key)
            .filter(|t| !t.is_empty() && t.len() <= limit && !t.chars().any(char::is_control))
            .map(str::to_owned)
    };
    let license = item.get("license").and_then(|l| {
        l.str("spdx_id")
            .filter(|id| !id.is_empty() && *id != "NOASSERTION")
            .or_else(|| l.str("name"))
            .filter(|t| !t.is_empty() && t.len() <= 100)
            .map(str::to_owned)
    });
    let topics = item
        .get("topics")
        .and_then(Json::array)
        .map(|list| {
            list.iter()
                .filter_map(|t| match t {
                    Json::String(t) if !t.is_empty() && t.len() <= 50 => Some(t.clone()),
                    _ => None,
                })
                .take(20)
                .collect()
        })
        .unwrap_or_default();
    // GitHub's own clone address is rebuilt from the owner and name, as everywhere
    // else here. Only the loopback test server may name a local fixture instead.
    let clone_url = match endpoint {
        GitHubEndpoint::Loopback(_) => item.str("clone_url").map(str::to_owned),
        GitHubEndpoint::Public => None,
    }
    .unwrap_or_else(|| format!("https://github.com/{}.git", remote.full_name()));
    Some(GitHubSearchItem {
        description: text("description", 2_000),
        stars: item.u64("stargazers_count").unwrap_or(0),
        forks: item.u64("forks_count").unwrap_or(0),
        open_issues: item.u64("open_issues_count").unwrap_or(0),
        language: text("language", 60),
        license,
        topics,
        default_branch: text("default_branch", 255).unwrap_or_else(|| "main".into()),
        size_kb: item.u64("size").unwrap_or(0),
        pushed_at: text("pushed_at", 40),
        updated_at: text("updated_at", 40),
        private: item.bool("private").unwrap_or(false),
        fork: item.bool("fork").unwrap_or(false),
        archived: item.bool("archived").unwrap_or(false),
        html_url: item
            .str("html_url")
            .and_then(github_web_url)
            .map_or_else(|| remote.web_url(), Into::into),
        clone_url,
        remote,
    })
}

/// A search query as GitHub's `q` parameter. GitHub counts at most 256 characters
/// without its qualifiers and judges the rest itself; Rocket only refuses what can
/// never be a query.
fn search_query(text: &str) -> Result<String> {
    let text = text.trim();
    if text.is_empty() || text.chars().count() > 1_000 || text.chars().any(char::is_control) {
        return Err(Error::InvalidInput("enter up to 1,000 characters to search for".into()));
    }
    Ok(urls::encode(text, false))
}

fn number_header(headers: &[(String, String)], name: &str) -> Option<u64> {
    header(headers, name)?.trim().parse().ok()
}

fn notification(item: &Json) -> Option<GitHubNotification> {
    let repo = item.get("repository")?;
    let repository = GitHubRemote::new(repo.get("owner")?.str("login")?, repo.str("name")?)?;
    let subject = item.get("subject")?;
    let kind = subject.str("type").unwrap_or("Unknown");
    Some(GitHubNotification {
        id: item.id("id")?,
        title: subject.str("title").unwrap_or("").into(),
        kind: kind.into(),
        reason: item.str("reason").unwrap_or("").into(),
        unread: item.bool("unread").unwrap_or(false),
        updated_at: item.str("updated_at").unwrap_or("").into(),
        web_url: urls::notification_url(&repository, kind, subject.str("url")),
        repository,
    })
}
fn pull_request(item: &Json) -> Option<GitHubPullRequest> {
    Some(GitHubPullRequest {
        number: item.u64("number")?,
        title: item.str("title").unwrap_or("").into(),
        html_url: github_web_url(item.str("html_url")?)?.into(),
        draft: item.bool("draft").unwrap_or(false),
        base: item.get("base")?.str("ref")?.into(),
    })
}

/// The status line and headers of the final response, then its body.
/// The final response in curl's `--include` output; header names are lowercase.
pub(crate) struct Response<'a> {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: &'a [u8],
}
pub(crate) fn split_response(stdout: &[u8]) -> Option<Response<'_>> {
    let mut rest = stdout;
    loop {
        let end = rest.windows(4).position(|w| w == b"\r\n\r\n")?;
        let head = std::str::from_utf8(&rest[..end]).ok()?;
        let body = &rest[end + 4..];
        let mut lines = head.split("\r\n");
        let status: u16 = lines.next()?.split(' ').nth(1)?.parse().ok()?;
        if (100..200).contains(&status) {
            rest = body;
            continue;
        }
        let headers = lines
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned()))
            .collect();
        return Some(Response {
            status,
            headers,
            body,
        });
    }
}
pub(crate) fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}
/// The `page` of a `rel="next"` link that stays on the same API endpoint.
fn next_page(headers: &[(String, String)], base: &str) -> Option<u32> {
    header(headers, "link")?.split(',').find_map(|part| {
        let (url, rel) = part.split_once(';')?;
        (rel.trim() == "rel=\"next\"").then_some(())?;
        let url = url.trim().strip_prefix('<')?.strip_suffix('>')?;
        let query = url.strip_prefix(base)?.split_once('?')?.1;
        query
            .split('&')
            .find_map(|p| p.strip_prefix("page="))?
            .parse()
            .ok()
    })
}
fn classify(status: u16, headers: &[(String, String)]) -> Result<()> {
    match status {
        200..=299 => Ok(()),
        401 => fail(HostingFailure::Unauthorized),
        403 | 429
            if status == 429
                || header(headers, "x-ratelimit-remaining") == Some("0")
                || header(headers, "retry-after").is_some() =>
        {
            fail(HostingFailure::RateLimited)
        }
        403 => fail(HostingFailure::Forbidden),
        404 => fail(HostingFailure::NotFound),
        _ => fail(HostingFailure::InvalidResponse),
    }
}

impl GitHubSession {
    pub fn account(&self) -> &GitHubAccount {
        &self.account
    }
    pub fn endpoint(&self) -> GitHubEndpoint {
        self.endpoint
    }
    /// Record later requests under a new diagnostic request context.
    pub fn with_log(&self, log: DiagnosticLog) -> Self {
        let mut session = self.clone();
        session.runner.log = Some(log);
        session
    }
    /// Whether requests carry the account's token.
    pub fn signed_in(&self) -> bool {
        self.token.is_some()
    }
    fn get(
        &self,
        operation: &'static str,
        step: &'static str,
        path: &str,
        cancel: &CancellationToken,
    ) -> Result<(Json, Vec<(String, String)>)> {
        self.runner.step(operation, step, || {
            let (code, headers, body) = self.send(path, cancel)?;
            classify(code, &headers)?;
            let value = json::parse(&body).ok_or(Error::Hosting(HostingFailure::InvalidResponse))?;
            Ok((value, headers))
        })
    }
    /// One GET: the final status, headers and body, without judging the status.
    #[allow(clippy::type_complexity)]
    fn send(&self, path: &str, cancel: &CancellationToken) -> Result<(u16, Vec<(String, String)>, Vec<u8>)> {
        {
            cancel.check()?;
            let base = self.endpoint.base();
            // curl config syntax; every value here is validated ASCII without quotes
            // or backslashes. The token never appears in arguments or logs.
            let authorization = self
                .token
                .as_ref()
                .map(|token| format!("header = \"Authorization: Bearer {token}\"\n"))
                .unwrap_or_default();
            let config = format!(
                "url = \"{base}{path}\"\n{authorization}header = \"Accept: application/vnd.github+json\"\nheader = \"X-GitHub-Api-Version: 2022-11-28\"\nuser-agent = \"Rocket/{}\"\n",
                env!("CARGO_PKG_VERSION")
            );
            let protocol = match self.endpoint {
                GitHubEndpoint::Public => "=https",
                GitHubEndpoint::Loopback(_) => "=http",
            };
            let arguments: Vec<OsString> = [
                "-q",
                "--silent",
                "--show-error",
                "--include",
                "--suppress-connect-headers",
                "--globoff",
                "--proto",
                protocol,
                "--max-redirs",
                "0",
                "--connect-timeout",
                "15",
                "--max-time",
                "50",
                "--config",
                "-",
            ]
            .iter()
            .map(OsString::from)
            .collect();
            let Output { status, stdout, .. } = self.runner.tool(
                &self.curl,
                "curl",
                &self.cwd,
                &arguments,
                config.into_bytes(),
                cancel,
            )?;
            if !status.success() {
                return fail(HostingFailure::Network);
            }
            let Response {
                status: code,
                headers,
                body,
            } = split_response(&stdout).ok_or(Error::Hosting(HostingFailure::InvalidResponse))?;
            Ok((code, headers, body.to_vec()))
        }
    }
    /// Search the repositories this session can see: public ones, and private ones
    /// when signed in. `page` starts at 1; GitHub stops after 1,000 results.
    pub fn search_repositories(
        &self,
        query: &str,
        sort: GitHubSearchSort,
        page: u32,
        cancel: &CancellationToken,
    ) -> Result<GitHubSearchOutcome> {
        let q = search_query(query)?;
        let page = page.max(1);
        let order = match sort {
            GitHubSearchSort::BestMatch => "",
            GitHubSearchSort::Stars => "&sort=stars&order=desc",
            GitHubSearchSort::Updated => "&sort=updated&order=desc",
        };
        let path = format!("/search/repositories?q={q}{order}&per_page={GITHUB_SEARCH_PAGE}&page={page}");
        self.runner.step("github-search", "search_github_repositories", || {
            let (code, headers, body) = self.send(&path, cancel)?;
            let remaining = number_header(&headers, "x-ratelimit-remaining");
            let reset_at = number_header(&headers, "x-ratelimit-reset");
            match code {
                200..=299 => {}
                403 | 429
                    if code == 429
                        || remaining == Some(0)
                        || header(&headers, "retry-after").is_some() =>
                {
                    return Ok(GitHubSearchOutcome::RateLimited {
                        reset_at,
                        retry_after: number_header(&headers, "retry-after"),
                    });
                }
                422 => return Ok(GitHubSearchOutcome::Refused),
                _ => classify(code, &headers)?,
            }
            let value = json::parse(&body).ok_or(Error::Hosting(HostingFailure::InvalidResponse))?;
            let items: Vec<_> = value
                .get("items")
                .and_then(Json::array)
                .ok_or(Error::Hosting(HostingFailure::InvalidResponse))?
                .iter()
                .filter_map(|item| search_item(item, self.endpoint))
                .collect();
            let total = value.u64("total_count").unwrap_or(items.len() as u64);
            let shown = (page as u64 - 1) * GITHUB_SEARCH_PAGE as u64 + items.len() as u64;
            let next_page = (items.len() == GITHUB_SEARCH_PAGE
                && shown < total.min(GITHUB_SEARCH_LIMIT))
            .then_some(page + 1);
            Ok(GitHubSearchOutcome::Page(GitHubSearchPage {
                total,
                incomplete: value.bool("incomplete_results").unwrap_or(false),
                items,
                next_page,
                remaining,
                reset_at,
            }))
        })
    }
    fn list<T>(
        &self,
        operation: &'static str,
        step: &'static str,
        path: &str,
        item: fn(&Json) -> Option<T>,
        cancel: &CancellationToken,
    ) -> Result<GitHubList<T>> {
        let (value, headers) = self.get(operation, step, path, cancel)?;
        let items = value
            .array()
            .ok_or(Error::Hosting(HostingFailure::InvalidResponse))?
            .iter()
            .filter_map(item)
            .collect();
        Ok(GitHubList {
            items,
            next_page: next_page(&headers, &self.endpoint.base()),
        })
    }
    /// Repositories the account owns, collaborates on or reaches through an
    /// organization, most recently pushed first.
    pub fn repositories(
        &self,
        page: u32,
        cancel: &CancellationToken,
    ) -> Result<GitHubList<GitHubRepository>> {
        self.list(
            "github-repositories",
            "read_github_repositories",
            &format!(
                "/user/repos?per_page={GITHUB_REPOSITORY_PAGE}&page={}&sort=pushed&affiliation=owner,collaborator,organization_member",
                page.max(1)
            ),
            repository,
            cancel,
        )
    }
    /// Unread notifications (or all recent ones), newest first. Reading them does
    /// not mark them read.
    pub fn notifications(
        &self,
        all: bool,
        page: u32,
        cancel: &CancellationToken,
    ) -> Result<GitHubList<GitHubNotification>> {
        self.list(
            "github-notifications",
            "read_github_notifications",
            &format!(
                "/notifications?all={all}&per_page={GITHUB_NOTIFICATION_PAGE}&page={}",
                page.max(1)
            ),
            notification,
            cancel,
        )
    }
    /// One repository, including the upstream of a fork.
    pub fn repository(
        &self,
        remote: &GitHubRemote,
        cancel: &CancellationToken,
    ) -> Result<GitHubRepository> {
        let (value, _) = self.get(
            "github-repository",
            "read_github_repository",
            &format!("/repos/{}/{}", remote.owner, remote.name),
            cancel,
        )?;
        repository(&value).ok_or(Error::Hosting(HostingFailure::InvalidResponse))
    }
    /// The open pull request whose head is `head_owner:branch` in `base`, if any.
    pub fn open_pull_request(
        &self,
        base: &GitHubRemote,
        head_owner: &str,
        branch: &str,
        cancel: &CancellationToken,
    ) -> Result<Option<GitHubPullRequest>> {
        let list = self.list(
            "github-pulls",
            "read_github_pulls",
            &format!(
                "/repos/{}/{}/pulls?state=open&per_page=5&head={}",
                base.owner,
                base.name,
                urls::encode(&format!("{head_owner}:{branch}"), false)
            ),
            pull_request,
            cancel,
        )?;
        Ok(list.items.into_iter().next())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn responses_links_and_status_classes() {
        let raw = b"HTTP/1.1 100 Continue\r\n\r\nHTTP/2 200\r\nLink: <https://api.github.com/user/repos?per_page=100&page=3>; rel=\"next\", <https://api.github.com/user/repos?page=9>; rel=\"last\"\r\nX-RateLimit-Remaining: 4999\r\n\r\n[]";
        let Response {
            status,
            headers,
            body,
        } = split_response(raw).unwrap();
        assert_eq!((status, body), (200, &b"[]"[..]));
        assert_eq!(next_page(&headers, "https://api.github.com"), Some(3));
        // A next link to another host is ignored.
        let foreign = vec![(
            "link".to_owned(),
            "<https://example.com/user/repos?page=2>; rel=\"next\"".to_owned(),
        )];
        assert_eq!(next_page(&foreign, "https://api.github.com"), None);
        let limit = vec![("x-ratelimit-remaining".to_owned(), "0".to_owned())];
        assert!(matches!(
            classify(403, &limit),
            Err(Error::Hosting(HostingFailure::RateLimited))
        ));
        assert!(matches!(
            classify(403, &[]),
            Err(Error::Hosting(HostingFailure::Forbidden))
        ));
        assert!(matches!(
            classify(429, &[]),
            Err(Error::Hosting(HostingFailure::RateLimited))
        ));
        assert!(matches!(
            classify(401, &[]),
            Err(Error::Hosting(HostingFailure::Unauthorized))
        ));
        assert!(matches!(
            classify(301, &[]),
            Err(Error::Hosting(HostingFailure::InvalidResponse))
        ));
        assert!(split_response(b"HTTP/1.1 200 OK\r\nno end").is_none());
    }
    #[test]
    fn endpoints_and_tokens() {
        assert_eq!(
            GitHubEndpoint::loopback("http://127.0.0.1:8123/"),
            Some(GitHubEndpoint::Loopback(8123))
        );
        assert_eq!(
            GitHubEndpoint::loopback("http://localhost:9"),
            Some(GitHubEndpoint::Loopback(9))
        );
        for bad in [
            "https://127.0.0.1:1",
            "http://192.168.1.2:80",
            "http://127.0.0.1",
            "http://127.0.0.1:0",
            "http://127.0.0.1.evil:80",
        ] {
            assert_eq!(GitHubEndpoint::loopback(bad), None, "{bad}");
        }
        assert_eq!(token_text("gho_AbC123-x.y"), Some("gho_AbC123-x.y"));
        for bad in ["", "a b", "a\"b", "a\\b", "pässword"] {
            assert_eq!(token_text(bad), None);
        }
    }
    #[test]
    #[cfg(target_os = "linux")]
    fn linux_runs_only_the_files_root_owns_in_usr_bin() {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(system_program("curl").as_deref(), Some(Path::new("/usr/bin/curl")));
        assert_eq!(system_program("rocket-no-such-program"), None);
        // A link is refused, even when root owns it and the file it names.
        let link = std::fs::read_dir("/usr/bin")
            .unwrap()
            .flatten()
            .find(|entry| {
                entry.file_type().is_ok_and(|kind| kind.is_symlink())
                    && std::fs::metadata(entry.path())
                        .is_ok_and(|meta| meta.is_file() && meta.uid() == 0 && meta.mode() & 0o022 == 0)
            })
            .expect("a link to a program in /usr/bin");
        let name = link.file_name().into_string().unwrap();
        assert_eq!(system_program(&name), None, "{name}");
    }
}
