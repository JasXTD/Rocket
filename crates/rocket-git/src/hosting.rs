//! Read-only github.com integration. Sign-in reuses the github.com credential the
//! configured Git credential helper (normally Git Credential Manager) stores for
//! HTTPS Git. Requests go only to `https://api.github.com` through Windows'
//! `curl.exe`; the token is passed on standard input, kept in memory and never
//! logged. Nothing on the account is changed.
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
mod urls;
use json::Json;
pub use urls::{GitHubRemote, github_web_url};

/// Repositories per page (GitHub's maximum).
pub const GITHUB_REPOSITORY_PAGE: usize = 100;
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
    cwd: PathBuf,
    endpoint: GitHubEndpoint,
    token: Arc<str>,
    account: GitHubAccount,
}
impl fmt::Debug for GitHubSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitHubSession")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

/// A neutral, empty working directory: credential lookup must not read a
/// repository's local configuration, and curl must not find a `.curlrc` there.
pub(crate) fn neutral_directory() -> Result<PathBuf> {
    let directory = std::env::temp_dir().join(format!("rocket-github-{}", std::process::id()));
    std::fs::create_dir_all(&directory)
        .map_err(|e| Error::io("prepare GitHub request folder", e))?;
    Ok(directory)
}
/// On Windows use only the OS-selected System32 curl. Environment variables and
/// a helper beside Git must not redirect credential-bearing hosting requests.
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
    #[cfg(not(windows))]
    {
        let mut candidates = Vec::new();
        if let Some(root) = std::env::var_os("SystemRoot") {
            candidates.push(PathBuf::from(root).join("System32").join("curl.exe"));
        }
        if let Some(bin) = _git.parent() {
            candidates.push(bin.join("curl.exe"));
            if let Some(root) = bin.parent() {
                candidates.push(root.join("mingw64").join("bin").join("curl.exe"));
            }
        }
        candidates
            .into_iter()
            .find(|p| std::fs::symlink_metadata(p).is_ok_and(|m| m.is_file()))
    }
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
            token,
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
    fn get(
        &self,
        operation: &'static str,
        step: &'static str,
        path: &str,
        cancel: &CancellationToken,
    ) -> Result<(Json, Vec<(String, String)>)> {
        self.runner.step(operation, step, || {
            cancel.check()?;
            let base = self.endpoint.base();
            // curl config syntax; every value here is validated ASCII without quotes
            // or backslashes. The token never appears in arguments or logs.
            let config = format!(
                "url = \"{base}{path}\"\nheader = \"Authorization: Bearer {}\"\nheader = \"Accept: application/vnd.github+json\"\nheader = \"X-GitHub-Api-Version: 2022-11-28\"\nuser-agent = \"Rocket/{}\"\n",
                self.token,
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
            classify(code, &headers)?;
            let value = json::parse(body).ok_or(Error::Hosting(HostingFailure::InvalidResponse))?;
            Ok((value, headers))
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
}
