//! Explicit, independently implemented hosting APIs. Git transport and credentials
//! are separate. API credentials never enter argv, repository config or diagnostics.
use crate::{hosting::json::Json, runner::Runner, *};
use std::{
    fmt,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
mod adapter;
mod azure_server;
mod checks;
pub use checks::*;
mod bitbucket_server;
mod pagination;
mod protection;
mod publish;
pub use publish::*;
mod pull_review;
pub use pull_review::*;
pub use protection::protect_hosting_secret;
mod transport;
mod urls;
pub(crate) use urls::encode;
pub use urls::{ForgeConfig, ForgeRepository};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ForgeProvider {
    #[default]
    GitHub,
    GitHubEnterprise,
    GitLab,
    Bitbucket,
    BitbucketServer,
    Azure,
    AzureServer,
    Gitea,
}
impl ForgeProvider {
    pub const ALL: [Self; 8] = [
        Self::GitHub,
        Self::GitHubEnterprise,
        Self::GitLab,
        Self::Bitbucket,
        Self::BitbucketServer,
        Self::Azure,
        Self::AzureServer,
        Self::Gitea,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::GitHub => "GitHub",
            Self::GitHubEnterprise => "GitHub Enterprise Server",
            Self::GitLab => "GitLab",
            Self::Bitbucket => "Bitbucket Cloud",
            Self::BitbucketServer => "Bitbucket Server / Data Center",
            Self::Azure => "Azure DevOps Services",
            Self::AzureServer => "Azure DevOps Server",
            Self::Gitea => "Gitea",
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Self::GitHub => "github",
            Self::GitHubEnterprise => "github-enterprise",
            Self::GitLab => "gitlab",
            Self::Bitbucket => "bitbucket",
            Self::BitbucketServer => "bitbucket-server",
            Self::Azure => "azure",
            Self::AzureServer => "azure-server",
            Self::Gitea => "gitea",
        }
    }
    fn diagnostic_step(self) -> &'static str {
        match self {
            Self::GitHub => "hosting_provider_github",
            Self::GitHubEnterprise => "hosting_provider_github_enterprise",
            Self::GitLab => "hosting_provider_gitlab",
            Self::Bitbucket => "hosting_provider_bitbucket",
            Self::BitbucketServer => "hosting_provider_bitbucket_server",
            Self::Azure => "hosting_provider_azure",
            Self::AzureServer => "hosting_provider_azure_server",
            Self::Gitea => "hosting_provider_gitea",
        }
    }
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.key() == key)
    }
    /// Shared GitHub repository APIs, not public-only browser OAuth or publishing.
    pub fn is_github(self) -> bool {
        matches!(self, Self::GitHub | Self::GitHubEnterprise)
    }
    pub fn is_azure(self) -> bool {
        matches!(self, Self::Azure | Self::AzureServer)
    }
    pub fn supports_search(self) -> bool {
        self.is_github() || self == Self::GitLab
    }
    pub fn default_server(self) -> &'static str {
        match self {
            Self::GitHub => "https://github.com",
            Self::GitHubEnterprise => "https://github.example.com",
            Self::GitLab => "https://gitlab.com",
            Self::Bitbucket => "https://bitbucket.org",
            Self::BitbucketServer => "https://bitbucket.example.com",
            Self::Azure => "https://dev.azure.com/organization",
            Self::AzureServer => "https://devops.example.com/tfs/DefaultCollection",
            Self::Gitea => "https://gitea.com",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForgeFailure {
    Unauthorized,
    IdentityUnverified,
    AzureIdentityUnverified,
    AzureApiRejected,
    Forbidden,
    NotFound,
    RateLimited,
    InvalidResponse,
    InvalidPagination,
    Network,
    TlsVerification,
    RedirectRefused,
    ServerUnavailable,
    ApiVersionUnsupported,
    Unavailable,
    Unsupported,
    Rejected,
    StaleReview,
    OutcomeUnknown,
    PullChanged,
    PullReviewStale,
    PendingReview,
    ReviewStateUnavailable,
    PullReviewRejected,
    PullReviewUnknown,
    PullMergeBlocked,
    PullMergeRejected,
    PullMergeUnknown,
}
impl ForgeFailure {
    pub fn code(self) -> &'static str {
        match self {
            Self::Unauthorized => "hosting_unauthorized",
            Self::IdentityUnverified => "hosting_identity_unverified",
            Self::AzureIdentityUnverified => "hosting_azure_identity_unverified",
            Self::AzureApiRejected => "hosting_azure_api_rejected",
            Self::Forbidden => "hosting_forbidden",
            Self::NotFound => "hosting_not_found",
            Self::RateLimited => "hosting_rate_limited",
            Self::InvalidResponse => "hosting_invalid_response",
            Self::InvalidPagination => "hosting_invalid_pagination",
            Self::Network => "hosting_network",
            Self::TlsVerification => "hosting_tls_verification",
            Self::RedirectRefused => "hosting_redirect_refused",
            Self::ServerUnavailable => "hosting_server_unavailable",
            Self::ApiVersionUnsupported => "hosting_api_version_unsupported",
            Self::Unavailable => "hosting_unavailable",
            Self::Unsupported => "hosting_unsupported",
            Self::Rejected => "hosting_rejected",
            Self::StaleReview => "hosting_review_stale",
            Self::OutcomeUnknown => "hosting_outcome_unknown",
            Self::PullChanged => "hosting_pull_changed",
            Self::PullReviewStale => "hosting_pull_review_stale",
            Self::PendingReview => "hosting_pending_review",
            Self::ReviewStateUnavailable => "hosting_review_state_unavailable",
            Self::PullReviewRejected => "hosting_pull_review_rejected",
            Self::PullReviewUnknown => "hosting_pull_review_unknown",
            Self::PullMergeBlocked => "hosting_pull_merge_blocked",
            Self::PullMergeRejected => "hosting_pull_merge_rejected",
            Self::PullMergeUnknown => "hosting_pull_merge_unknown",
        }
    }
    pub fn guidance(self) -> &'static str {
        match self {
            Self::PullMergeBlocked => "Merging is not ready or could not be confirmed. Refresh the PR; it must be open, not a draft, with the provider permitting the selected method for this account. Resolve required checks, reviews or conflicts on the provider first.",
            Self::PullMergeRejected => "The provider rejected the merge. The head may have changed, or repository rules, checks, approvals or a required merge queue may prevent it. Refresh and review again; Rocket does not retry or delete branches.",
            Self::PullMergeUnknown => "The merge request was interrupted or its response could not be verified. It may have merged. Refresh this same PR and check the provider before deciding what to do next. Rocket will not retry automatically.",
            Self::PullChanged => "The pull request changed while it was being read or reviewed. Refresh its details and review the current commits before submitting. Your draft is retained.",
            Self::PullReviewStale => "This preview expired, was already used, or belongs to another account or PR revision. Refresh and review again. Inline drafts are retained; remove and re-select comments from an earlier revision before submitting.",
            Self::PullReviewRejected => "The provider rejected this review. Check the PR state, review permissions and any pending review on the provider. Your draft is retained; Rocket does not retry automatically.",
            Self::PendingReview => "This account already has a pending review on Gitea. Finish or discard it on the provider, then refresh and review again. Rocket will not submit or replace those saved comments.",
            Self::ReviewStateUnavailable => "Rocket could not verify that this account has no pending Gitea review within the bounded review scan. Check the provider before continuing; no review was submitted.",
            Self::PullReviewUnknown => "The review submission was interrupted or its result could not be verified. It may have succeeded or saved only some pending comments. Refresh this PR's reviews and check the provider before explicitly deciding to submit again. Rocket will not retry automatically.",
            Self::Unauthorized => {
                "The server rejected this account's token. Check its expiry, host and account, then reconnect. For a browser session, sign in again. Existing Git credentials were not changed."
            }
            Self::IdentityUnverified => "Bitbucket did not confirm an authenticated user. Use a personal HTTP access token, check its expiry, and ask the server administrator whether a proxy removes the X-AUSERNAME response header. Public API access alone does not verify this account.",
            Self::AzureIdentityUnverified => "Azure DevOps Server did not confirm an active authenticated user. Check the collection URL and use a personal access token issued by that server. Anonymous or group access does not verify an account. Check proxy and IIS authentication with your administrator.",
            Self::AzureApiRejected => "Azure DevOps Server rejected the API request. Check the collection URL, selected REST API version and request filters. Use API 6.0 for Server 2020, 7.0 for 2022 or 7.1 for 2022.1 and newer. Rocket does not silently switch versions or retry.",
            Self::Forbidden => {
                "The server refused access. Check repository permissions, token scopes, authentication requirements and organization SSO policy."
            }
            Self::NotFound => {
                "The repository, branch or API resource was not found or is hidden from this account. Check the server and repository path."
            }
            Self::RateLimited => {
                "The hosting API rate limit was reached. Wait before refreshing. Rocket does not retry automatically."
            }
            Self::InvalidResponse => {
                "The server returned an unsupported or invalid response. Redirects are not followed; check the configured server."
            }
            Self::InvalidPagination => {
                "The server returned an invalid pagination link, or this page belongs to an earlier account, repository or filter. Refresh the list. Rocket only follows pages on the same API endpoint; it did not send credentials to another destination."
            }
            Self::Network => {
                "The hosting server could not be reached. Check the connection, proxy and TLS configuration."
            }
            Self::TlsVerification => "The server certificate could not be verified. Check the HTTPS server address and ask your administrator to fix the certificate chain or configure a trusted corporate CA. Rocket does not disable certificate verification.",
            Self::RedirectRefused => "The hosting API redirected the request. Rocket did not follow it. Enter the final HTTPS server address; if this is a browser sign-in redirect, ask your administrator about token access to the hosting API.",
            Self::ServerUnavailable => "The hosting server returned a service error. Check its availability or maintenance status and try again later. Rocket does not retry automatically.",
            Self::ApiVersionUnsupported => "The server reports that this API is no longer available. Rocket requests GitHub REST API version 2022-11-28. Check the server's supported API versions and update Rocket if that version has been retired; it will not silently switch versions or retry.",
            Self::Unavailable => {
                "The installed curl helper is unavailable. Git operations are unaffected."
            }
            Self::Unsupported => "This provider does not support the selected feature.",
            Self::Rejected => {
                "The server rejected the pull request. Check that the branches differ, contain changes, and do not already have an open request."
            }
            Self::StaleReview => {
                "This review expired, was already submitted, belongs to another account, or a remote branch changed. Review again before creating."
            }
            Self::OutcomeUnknown => {
                "The create request was interrupted or its result could not be verified. It may have succeeded. Check the provider's pull request list before trying again; Rocket will not automatically retry."
            }
        }
    }
}
fn fail<T>(failure: ForgeFailure) -> Result<T> {
    Err(Error::Forge(failure))
}
fn invalid(message: &str) -> Error {
    Error::InvalidInput(message.into())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ForgeKind {
    #[default]
    Pulls,
    Issues,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ForgeState {
    #[default]
    Open,
    Closed,
    All,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ForgeFilter {
    pub kind: ForgeKind,
    pub state: ForgeState,
    /// Azure work-item states are project-defined. Empty means every state.
    pub work_item_state: String,
    /// GitHub/GitLab server-side text/author/assignee search. Others reject it.
    pub search: ForgeSearch,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ForgeSearch {
    pub text: String,
    pub author: String,
    pub assignee: String,
}
impl ForgeSearch {
    pub fn validate_for(&self, provider: ForgeProvider) -> Result<()> {
        if provider.is_github() { return self.validate(); }
        if provider != ForgeProvider::GitLab { return fail(ForgeFailure::Unsupported); }
        // GitLab has separate query parameters, not GitHub's query language.
        // Literal punctuation and Unicode are encoded, never interpreted as filters.
        if self.text.len() > 256 || self.text.chars().any(char::is_control) {
            return Err(invalid("Search text must be at most 256 bytes without control characters."));
        }
        for login in [&self.author, &self.assignee] {
            if login.len() > 255 || !login.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b,b'-'|b'_'|b'.')) {
                return Err(invalid("Author and assignee must be GitLab usernames using letters, numbers, periods, underscores or hyphens (at most 255 bytes)."));
            }
        }
        Ok(())
    }
    pub fn is_empty(&self) -> bool {
        self.text.is_empty() && self.author.is_empty() && self.assignee.is_empty()
    }
    pub fn validate(&self) -> Result<()> {
        if self.text.len() > 256 || self.text.chars().any(|c| c.is_control() || matches!(c, '"' | '\\' | ':')) {
            return Err(invalid("Search text must be at most 256 bytes, without quotes, backslashes, colons or control characters. Use the author and assignee fields for people."));
        }
        for login in [&self.author, &self.assignee] {
            let name = login.strip_suffix("[bot]").unwrap_or(login);
            if login.len() > 100 || (!login.is_empty() && (name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))) {
                return Err(invalid("Author and assignee must be GitHub usernames, without spaces or query qualifiers."));
            }
        }
        Ok(())
    }
}
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ForgeCursor {
    pub page: u32,
    pub after: u64,
    continuation: Option<pagination::Continuation>,
}
impl fmt::Debug for ForgeCursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ForgeCursor")
            .field("page", &self.page)
            .field("after", &self.after)
            .field("has_continuation", &self.continuation.is_some())
            .finish()
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForgeItem {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub author: String,
    pub source: String,
    pub target: String,
    pub url: String,
}
#[derive(Clone, Debug)]
pub struct ForgePage {
    pub items: Vec<ForgeItem>,
    pub next: Option<ForgeCursor>,
    pub total: Option<u64>,
    /// GitHub timed out part of the search, or its 1,000-result cap was reached.
    pub incomplete: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForgeDraft {
    pub repository: ForgeRepository,
    pub source: String,
    pub target: String,
    pub title: String,
    pub body: String,
}
#[derive(Debug)]
pub struct ForgePreview {
    draft: ForgeDraft,
    source_repository: Option<ForgeRepository>,
    source_id: String,
    target_id: String,
    session: u64,
    created: Instant,
    submitted: AtomicBool,
}
impl ForgePreview {
    /// Stable across processes, but bound to the verified account, server, both
    /// remote tips and all submitted text. It never contains credentials.
    pub fn review_digest(&self, session: &ForgeSession) -> Result<String> {
        if self.session != session.id { return fail(ForgeFailure::StaleReview); }
        session.review_digest("create", &[
            self.draft.repository.path(), self.source_repository().path(),
            &self.draft.source, &self.draft.target, &self.source_id, &self.target_id,
            &self.draft.title, &self.draft.body,
        ])
    }
    pub fn draft(&self) -> &ForgeDraft {
        &self.draft
    }
    pub fn source_id(&self) -> &str {
        &self.source_id
    }
    pub fn target_id(&self) -> &str {
        &self.target_id
    }
    pub fn source_repository(&self) -> &ForgeRepository {
        self.source_repository.as_ref().unwrap_or(&self.draft.repository)
    }
}
#[derive(Clone)]
pub struct ForgeSession {
    runner: Runner,
    curl: PathBuf,
    cwd: crate::hosting::NeutralDirectory,
    config: ForgeConfig,
    authorization: Arc<str>,
    login: String,
    account_identity: Option<String>,
    enterprise_version: Option<String>,
    browser_auth: bool,
    id: u64,
    publish_fixture_root: Option<PathBuf>,
}
impl fmt::Debug for ForgeSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ForgeSession")
            .field("provider", &self.config.provider())
            .finish_non_exhaustive()
    }
}
impl ForgeSession {
    fn review_digest(&self, operation: &str, parts: &[&str]) -> Result<String> {
        let port = self.config.fixture_port().map(|p|p.to_string()).unwrap_or_default();
        let mut values = vec!["hosting-review-v1",operation,self.config.provider().key(),self.config.server(),
            self.config.azure_api_version(), &port, &self.login, self.account_identity().unwrap_or("")];
        values.extend_from_slice(parts);
        crate::saved_work::review_digest(&values.iter().map(|v|v.as_bytes()).collect::<Vec<_>>())
    }
    pub fn config(&self) -> &ForgeConfig {
        &self.config
    }
    pub fn login(&self) -> &str {
        &self.login
    }
    /// Stable provider identity, never a display name or a token.
    pub fn account_identity(&self) -> Option<&str> { self.account_identity.as_deref() }
    /// Bounded numeric version reported by GHES; absent or malformed headers are
    /// not evidence of any particular server release or feature support.
    pub fn enterprise_version(&self) -> Option<&str> {
        self.enterprise_version.as_deref()
    }
    pub fn temporary_browser_session(&self) -> bool {
        self.browser_auth && self.config.provider() == ForgeProvider::Bitbucket
    }
    pub fn with_log(&self, log: DiagnosticLog) -> Self {
        let mut s = self.clone();
        s.runner.log = Some(log);
        s
    }
    pub fn preview(&self, draft: ForgeDraft, cancel: &CancellationToken) -> Result<ForgePreview> {
        self.preview_from_repository(draft, None, cancel)
    }
    /// Existing already-pushed GitHub fork only; never creates a fork or pushes.
    pub fn preview_from_fork(&self, draft: ForgeDraft, source: ForgeRepository, cancel: &CancellationToken) -> Result<ForgePreview> {
        if !self.config.provider().is_github() { return fail(ForgeFailure::Unsupported); }
        self.check_repository(&source)?;
        if source.path().eq_ignore_ascii_case(draft.repository.path()) { return self.preview(draft, cancel); }
        self.preview_from_repository(draft, Some(source), cancel)
    }
    fn preview_from_repository(&self, draft: ForgeDraft, source_repository: Option<ForgeRepository>, cancel: &CancellationToken) -> Result<ForgePreview> {
        self.runner.step("hosting-preview","validate_pull_request",||{
            self.check_repository(&draft.repository)?;
            for branch in [&draft.source,&draft.target] { validate_branch(branch)?; }
            if (source_repository.is_none() && draft.source==draft.target) || draft.title.trim().is_empty() || draft.title.len()>256
                || draft.title.chars().any(char::is_control) || draft.body.len()>64*1024
                || draft.body.contains('\0') {return Err(invalid("Use distinct source/target branches, a title up to 256 bytes and a description up to 64 KiB."));}
            if let Some(source) = &source_repository { self.check_fork_network("hosting-preview", source, &draft.repository, cancel)?; }
            let source_id=self.branch_id(source_repository.as_ref().unwrap_or(&draft.repository),&draft.source,cancel)?;
            let target_id=self.branch_id(&draft.repository,&draft.target,cancel)?;
            if source_id==target_id {return Err(invalid("Both remote branches point to the same commit. Push the intended changes before reviewing."));}
            Ok(ForgePreview{draft,source_repository,source_id,target_id,session:self.id,created:Instant::now(),submitted:AtomicBool::new(false)})
        })
    }
    pub fn create(&self, preview: &ForgePreview, cancel: &CancellationToken) -> Result<ForgeItem> {
        self.runner
            .step("hosting-create", "create_pull_request", || {
                if preview.session != self.id
                    || preview.created.elapsed() > Duration::from_secs(120)
                    || preview.submitted.load(Ordering::Acquire)
                {
                    return fail(ForgeFailure::StaleReview);
                }
                let d = &preview.draft;
                if let Some(source) = &preview.source_repository { self.check_fork_network("hosting-create", source, &d.repository, cancel)?; }
                if self.branch_id(preview.source_repository(), &d.source, cancel)? != preview.source_id
                    || self.branch_id(&d.repository, &d.target, cancel)? != preview.target_id
                {
                    return fail(ForgeFailure::StaleReview);
                }
                cancel.check()?;
                if preview.submitted.swap(true, Ordering::AcqRel) {
                    return fail(ForgeFailure::StaleReview);
                }
                // No automatic retry. A successful remote write must survive late cancellation.
                let (path, body) = if let Some(source) = &preview.source_repository {
                    (format!("/repos/{}/pulls", d.repository.encoded()), format!(
                        "{{\"title\":{},\"body\":{},\"head\":{},\"head_repo\":{},\"base\":{},\"maintainer_can_modify\":false}}",
                        quote(&d.title), quote(&d.body), quote(&format!("{}:{}",source.parts().0,d.source)), quote(source.parts().1), quote(&d.target)))
                } else { self.create_request(d) };
                let value = self
                    .request(
                        "hosting-create",
                        "submit_pull_request",
                        "POST",
                        &path,
                        Some(&body),
                        true,
                        cancel,
                    )?
                    .0;
                if let Some(source) = &preview.source_repository {
                    let matches = |field: &str, repository: &ForgeRepository, branch: &str, sha: &str| {
                        value.get(field).is_some_and(|r| r.str("ref") == Some(branch) && r.str("sha") == Some(sha)
                            && r.get("repo").and_then(|r| r.str("full_name")).is_some_and(|name|name.eq_ignore_ascii_case(repository.path())))
                    };
                    if !matches("head",source,&d.source,&preview.source_id) || !matches("base",&d.repository,&d.target,&preview.target_id) {
                        return fail(ForgeFailure::OutcomeUnknown);
                    }
                }
                self.item(&d.repository, ForgeKind::Pulls, &value)
                    .ok_or(Error::Forge(ForgeFailure::OutcomeUnknown))
            })
    }
    fn check_repository(&self, repo: &ForgeRepository) -> Result<()> {
        if repo.config != self.config {
            return Err(invalid(
                "The repository belongs to a different hosting server.",
            ));
        }
        Ok(())
    }
    fn check_fork_network(&self, operation: &'static str, source: &ForgeRepository, target: &ForgeRepository, cancel: &CancellationToken) -> Result<()> {
        let root = |repo: &ForgeRepository| -> Result<u64> {
            self.check_repository(repo)?;
            let j = self.request(operation, "read_fork_network", "GET", &format!("/repos/{}", repo.encoded()), None, false, cancel)?.0;
            if !j.str("full_name").is_some_and(|name| name.eq_ignore_ascii_case(repo.path())) { return fail(ForgeFailure::InvalidResponse); }
            let id = if j.bool("fork") == Some(true) { j.get("source").and_then(|r|r.u64("id")) } else if j.bool("fork") == Some(false) { j.u64("id") } else { None };
            id.filter(|id| *id>0).ok_or(Error::Forge(ForgeFailure::InvalidResponse))
        };
        if root(source)? != root(target)? { return Err(invalid("The source and target repositories are not in the same GitHub fork network.")); }
        Ok(())
    }
}
fn validate_branch(branch: &str) -> Result<()> {
    if branch.is_empty()
        || branch.len() > 1024
        || branch.starts_with(['-', '/'])
        || branch.ends_with(['.', '/'])
        || branch.contains("..")
        || branch.contains("@{")
        || branch.contains("//")
        || branch
            .split('/')
            .any(|s| s.starts_with('.') || s.ends_with(".lock"))
        || branch
            .chars()
            .any(|c| c.is_control() || matches!(c, ' ' | '~' | '^' | ':' | '?' | '*' | '[' | '\\'))
    {
        Err(invalid("Enter a valid remote branch name."))
    } else {
        Ok(())
    }
}
/// JSON string quoting, for bounded strings created by Rocket. No shell quoting.
fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < ' ' => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
