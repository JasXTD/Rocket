//! Bounded GitHub PR reads and single-use, exact-commit review submissions.
//! Returned URLs/patches are data: never execute patches or follow content URLs.
use super::*;
use std::collections::HashSet;
mod inline;
pub use inline::*;
mod merge;
pub use merge::*;
mod threads;
pub use threads::*;
mod providers;

const READ: &str = "hosting-pull-details";
const PREVIEW: &str = "hosting-review-preview";
const SUBMIT: &str = "hosting-review-submit";
const LIMIT: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForgePullSummary {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub author: String,
    pub state: String,
    pub draft: bool,
    pub merged: bool,
    pub mergeable: Option<bool>,
    pub mergeable_state: Option<String>,
    pub head: String,
    pub base: String,
    pub source: String,
    pub target: String,
    pub commits: u64,
    pub files: u64,
    pub additions: u64,
    pub deletions: u64,
    pub counts_available: bool,
    revision: Option<u64>,
    author_identity: Option<String>,
}
#[derive(Clone, Debug)]
pub struct ForgePullCommit {
    pub id: String,
    pub subject: String,
    pub author: String,
}
#[derive(Clone, Debug)]
pub struct ForgePullFile {
    pub path: String,
    pub previous: Option<String>,
    pub status: String,
    pub additions: u64,
    pub deletions: u64,
    /// Missing for binary/large files or when Rocket's bounded patch budget is used.
    pub patch: Option<String>,
    pub lines: Vec<ForgePullLine>,
    raw_path: Option<String>,
}
#[derive(Clone, Debug)]
pub struct ForgePullComment {
    pub id: u64,
    pub author: String,
    pub body: String,
    pub date: String,
    pub path: Option<String>,
    pub line: Option<u64>,
    pub reply_to: Option<u64>,
}
#[derive(Clone, Debug)]
pub struct ForgePullReview {
    pub id: u64,
    pub author: String,
    pub body: String,
    pub state: String,
    pub commit: Option<String>,
    pub date: String,
}
#[derive(Clone, Debug)]
pub struct ForgePullCheck {
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
}
#[derive(Clone, Debug)]
pub struct ForgePullCollection<T> {
    pub items: Vec<T>,
    /// A bounded preview, not a claim that all remote items have been loaded.
    pub limited: bool,
    pub error: Option<ForgeFailure>,
}
impl<T> Default for ForgePullCollection<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            limited: false,
            error: None,
        }
    }
}
#[derive(Clone, Debug)]
pub struct ForgePullDetails {
    repository: ForgeRepository,
    session: u64,
    summary: ForgePullSummary,
    pub commits: ForgePullCollection<ForgePullCommit>,
    pub files: ForgePullCollection<ForgePullFile>,
    pub discussion: ForgePullCollection<ForgePullComment>,
    pub inline_comments: ForgePullCollection<ForgePullComment>,
    pub reviews: ForgePullCollection<ForgePullReview>,
    pub checks: ForgePullCollection<ForgePullCheck>,
    pub statuses: ForgePullCollection<ForgePullCheck>,
}
impl ForgePullDetails {
    pub fn supports_review(&self, action: ForgeReviewAction) -> bool { providers::supports_review(self.repository.config().provider(),action) }
    pub fn review_message_supported(&self, action: ForgeReviewAction) -> bool { providers::message_supported(self.repository.config().provider(),action) }
    pub fn supports_merge(&self, method: ForgeMergeMethod) -> bool { providers::supports_merge(self.repository.config().provider(),method) }
    pub fn merge_condition(&self) -> &'static str { providers::merge_condition(self.repository.config().provider()) }
    pub fn repository(&self) -> &ForgeRepository {
        &self.repository
    }
    pub fn summary(&self) -> &ForgePullSummary {
        &self.summary
    }
    pub fn url(&self) -> String {
        self.repository
            .item_url(ForgeKind::Pulls, self.summary.number)
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ForgeReviewAction {
    #[default]
    Comment,
    Approve,
    RequestChanges,
}
impl ForgeReviewAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Comment => "Comment",
            Self::Approve => "Approve",
            Self::RequestChanges => "Request changes",
        }
    }
    pub fn event(self) -> &'static str {
        match self {
            Self::Comment => "COMMENT",
            Self::Approve => "APPROVE",
            Self::RequestChanges => "REQUEST_CHANGES",
        }
    }
    fn state(self) -> &'static str {
        match self {
            Self::Comment => "COMMENTED",
            Self::Approve => "APPROVED",
            Self::RequestChanges => "CHANGES_REQUESTED",
        }
    }
}
pub struct ForgePullReviewPreview {
    repository: ForgeRepository,
    summary: ForgePullSummary,
    session: u64,
    action: ForgeReviewAction,
    body: String,
    comments: Vec<ForgeInlineComment>,
    created: Instant,
    used: AtomicBool,
}
impl ForgePullReviewPreview {
    pub fn repository(&self) -> &ForgeRepository {
        &self.repository
    }
    pub fn summary(&self) -> &ForgePullSummary {
        &self.summary
    }
    pub fn action(&self) -> ForgeReviewAction {
        self.action
    }
    pub fn body(&self) -> &str {
        &self.body
    }
    pub fn comments(&self) -> &[ForgeInlineComment] { &self.comments }
}
impl ForgeSession {
    fn pull_scope(&self, repo: &ForgeRepository, number: u64) -> Result<String> {
        self.check_repository(repo)?;
        if !self.config.provider().is_github() {
            return fail(ForgeFailure::Unsupported);
        }
        if number == 0 {
            return Err(invalid("Select a valid pull request."));
        }
        Ok(format!("/repos/{}/pulls/{number}", repo.encoded()))
    }
    fn pull_summary(
        &self,
        op: &'static str,
        path: &str,
        number: u64,
        c: &CancellationToken,
    ) -> Result<ForgePullSummary> {
        let j = self
            .request(op, "read_pull_details", "GET", path, None, false, c)?
            .0;
        let parse = || -> Option<ForgePullSummary> {
            if j.u64("number")? != number {
                return None;
            }
            let head = j.get("head")?;
            let base = j.get("base")?;
            let state = j.str("state")?;
            if !matches!(state, "open" | "closed") {
                return None;
            }
            Some(ForgePullSummary {
                number,
                title: string(&j, "title", 4096)?,
                body: body(&j, 64 * 1024),
                author: user(&j)?,
                state: state.into(),
                draft: j.bool("draft")?,
                merged: j.bool("merged")?,
                mergeable: j.bool("mergeable"),
                mergeable_state: optional_string(&j, "mergeable_state", 64),
                head: oid(head.str("sha")?)?,
                base: oid(base.str("sha")?)?,
                source: string(head, "ref", 1024)?,
                target: string(base, "ref", 1024)?,
                commits: j.u64("commits")?,
                files: j.u64("changed_files")?,
                additions: j.u64("additions")?,
                deletions: j.u64("deletions")?,
                counts_available: true, revision: None, author_identity: None,
            })
        };
        parse().ok_or(Error::Forge(ForgeFailure::InvalidResponse))
    }
    /// Reads are bounded to 200 entries per collection. Partial/unavailable data
    /// is explicit. Head/base are checked again before publishing the snapshot.
    pub fn pull_details(
        &self,
        repo: &ForgeRepository,
        number: u64,
        c: &CancellationToken,
    ) -> Result<ForgePullDetails> {
        if !self.config.provider().is_github(){return self.provider_pull_details(repo,number,c);}
        let path = self.pull_scope(repo, number)?;
        let first = self.pull_summary(READ, &path, number, c)?;
        let mut commits = self.pull_collection(
            repo,
            &format!("{path}/commits"),
            None,
            "read_pull_commits",
            c,
            |j| {
                let commit = j.get("commit")?;
                Some(ForgePullCommit {
                    id: oid(j.str("sha")?)?,
                    subject: bounded(commit.str("message")?.lines().next().unwrap_or(""), 4096),
                    author: bounded(
                        j.get("author")
                            .and_then(|x| x.str("login"))
                            .or_else(|| commit.get("author").and_then(|x| x.str("name")))
                            .unwrap_or("Unknown author"),
                        256,
                    ),
                })
            },
        )?;
        let mut budget = 1024 * 1024usize;
        let mut files = self.pull_collection(
            repo,
            &format!("{path}/files"),
            None,
            "read_pull_files",
            c,
            |j| {
                let raw_patch = j
                    .str("patch")
                    .filter(|p| p.len() <= 64 * 1024 && p.len() <= budget);
                let lines = raw_patch.and_then(inline::parse_lines).unwrap_or_default();
                let patch = raw_patch.map(|p| { budget -= p.len(); multiline(p, 64 * 1024) });
                let raw_path = j.str("filename").filter(|p| inline::safe_path(p)).map(str::to_owned);
                Some(ForgePullFile {
                    path: string(j, "filename", 8192)?,
                    previous: optional_string(j, "previous_filename", 8192),
                    status: string(j, "status", 64)?,
                    additions: j.u64("additions")?,
                    deletions: j.u64("deletions")?,
                    patch,
                    lines,
                    raw_path,
                })
            },
        )?;
        commits.limited |= (commits.items.len() as u64) < first.commits;
        files.limited |= (files.items.len() as u64) < first.files;
        let discussion = self.pull_collection(
            repo,
            &format!("/repos/{}/issues/{number}/comments", repo.encoded()),
            None,
            "read_pull_discussion",
            c,
            |j| comment(j, false),
        )?;
        let inline_comments = self.pull_collection(
            repo,
            &format!("{path}/comments"),
            None,
            "read_pull_inline_comments",
            c,
            |j| comment(j, true),
        )?;
        let reviews = self.pull_collection(
            repo,
            &format!("{path}/reviews"),
            None,
            "read_pull_reviews",
            c,
            review,
        )?;
        // Never use API-supplied URLs for these credentialed requests. CI is
        // queried for the exact head in the base repository; fork checks may be
        // unavailable here. This does not evaluate branch protection rules.
        let commit_path = format!("/repos/{}/commits/{}", repo.encoded(), first.head);
        let checks = self.pull_collection(
            repo,
            &format!("{commit_path}/check-runs"),
            Some("check_runs"),
            "read_pull_checks",
            c,
            |j| {
                if oid(j.str("head_sha")?)? != first.head {
                    return None;
                }
                Some(ForgePullCheck {
                    name: string(j, "name", 1024)?,
                    status: string(j, "status", 64)?,
                    conclusion: optional_string(j, "conclusion", 64),
                })
            },
        )?;
        let statuses = self.pull_collection(
            repo,
            &format!("{commit_path}/status"),
            Some("statuses"),
            "read_pull_statuses",
            c,
            |j| {
                Some(ForgePullCheck {
                    name: string(j, "context", 1024)?,
                    status: string(j, "state", 64)?,
                    conclusion: None,
                })
            },
        )?;
        let last = self.pull_summary(READ, &path, number, c)?;
        if !same_revision(&first, &last)
            || first.commits != last.commits
            || first.files != last.files
        {
            return fail(ForgeFailure::PullChanged);
        }
        Ok(ForgePullDetails {
            repository: repo.clone(),
            session: self.id,
            summary: last,
            commits,
            files,
            discussion,
            inline_comments,
            reviews,
            checks,
            statuses,
        })
    }
    fn pull_collection<T>(
        &self,
        repo: &ForgeRepository,
        endpoint: &str,
        key: Option<&str>,
        step: &'static str,
        c: &CancellationToken,
        mut parse: impl FnMut(&Json) -> Option<T>,
    ) -> Result<ForgePullCollection<T>> {
        let mut collect = || -> Result<ForgePullCollection<T>> {
            let mut out = ForgePullCollection::default();
            let mut path = format!("{endpoint}?per_page=50&page=1");
            let mut seen = HashSet::new();
            for page in 1..=4 {
                c.check()?;
                if !seen.insert(path.clone()) {
                    return fail(ForgeFailure::InvalidPagination);
                }
                let (j, headers) = self.request(READ, step, "GET", &path, None, false, c)?;
                if key == Some("statuses") && j.str("sha") != endpoint.split('/').rev().nth(1) {
                    return fail(ForgeFailure::InvalidResponse);
                }
                let values = key
                    .map_or(Some(&j), |key| j.get(key))
                    .and_then(Json::array)
                    .ok_or(Error::Forge(ForgeFailure::InvalidResponse))?;
                // A hostile endpoint must not bypass the advertised bound.
                if values.len() > 100 {
                    return fail(ForgeFailure::InvalidResponse);
                }
                for item in values {
                    if out.items.len() == LIMIT {
                        out.limited = true;
                        break;
                    }
                    out.items
                        .push(parse(item).ok_or(Error::Forge(ForgeFailure::InvalidResponse))?);
                }
                if let Some(total) = j.u64("total_count") {
                    out.limited |= total > out.items.len() as u64;
                }
                let next = pagination::next_link(&headers)?.flatten();
                let Some(next) = next else { break };
                let cursor =
                    self.linked_cursor(repo, &ForgeFilter::default(), page, &path, next)?;
                if page == 4 || out.items.len() == LIMIT {
                    out.limited = true;
                    break;
                }
                path = self
                    .continuation_path(repo, &ForgeFilter::default(), &cursor)?
                    .ok_or(Error::Forge(ForgeFailure::InvalidPagination))?
                    .to_owned();
                // A next page resolves an earlier total_count observation.
                out.limited = false;
            }
            Ok(out)
        };
        match collect() {
            Err(Error::Forge(e)) if e != ForgeFailure::Unauthorized => Ok(ForgePullCollection {
                error: Some(e),
                ..Default::default()
            }),
            other => other,
        }
    }
    pub fn preview_pull_review(
        &self,
        details: &ForgePullDetails,
        action: ForgeReviewAction,
        body: &str,
        c: &CancellationToken,
    ) -> Result<ForgePullReviewPreview> {
        self.preview_pull_review_with_comments(details, action, body, &[], c)
    }
    pub fn preview_pull_review_with_comments(
        &self, details: &ForgePullDetails, action: ForgeReviewAction, body: &str,
        comments: &[ForgeInlineComment], c: &CancellationToken,
    ) -> Result<ForgePullReviewPreview> {
        self.runner.step(PREVIEW, "validate_pull_review", || {
            if details.session != self.id { return fail(ForgeFailure::PullReviewStale); }
            providers::validate_review(self.config.provider(),action,body,comments)?;
            inline::validate_comments(details, comments, body.len())?;
            if body.len() > 64 * 1024 || body.contains('\0') || (action != ForgeReviewAction::Approve && details.review_message_supported(action) && body.trim().is_empty()) { return Err(invalid("This review action requires a message of at most 64 KiB without NUL.")); }
            let current = self.current_pull(PREVIEW, &details.repository, details.summary.number, c)?;
            if !same_revision(&details.summary, &current) { return fail(ForgeFailure::PullChanged); }
            self.review_allowed(&current, action)?;
            Ok(ForgePullReviewPreview { repository: details.repository.clone(), summary: current, session: self.id, action, body: body.into(), comments: comments.to_vec(), created: Instant::now(), used: AtomicBool::new(false) })
        })
    }
    fn review_allowed(&self, current: &ForgePullSummary, action: ForgeReviewAction) -> Result<()> {
        if current.state != "open" || current.merged {
            return Err(invalid(
                "This pull request is closed or merged. Open it on the provider to inspect its history.",
            ));
        }
        if action != ForgeReviewAction::Comment && current.author_identity.as_deref().map_or_else(||current.author.eq_ignore_ascii_case(&self.login),|id|self.account_identity().is_some_and(|mine|mine.eq_ignore_ascii_case(id)))
        {
            return Err(invalid(
                "You cannot approve or request changes on your own pull request. Use Comment instead.",
            ));
        }
        Ok(())
    }
    pub fn submit_pull_review(
        &self,
        preview: &ForgePullReviewPreview,
        c: &CancellationToken,
    ) -> Result<ForgePullReview> {
        self.runner.step(SUBMIT, "submit_pull_review", || {
            if preview.session != self.id
                || preview.created.elapsed() > Duration::from_secs(120)
                || preview.used.load(Ordering::Acquire)
            {
                return fail(ForgeFailure::PullReviewStale);
            }
            let current = self.current_pull(SUBMIT, &preview.repository, preview.summary.number, c)?;
            if !same_revision(&preview.summary, &current) {
                return fail(ForgeFailure::PullChanged);
            }
            self.review_allowed(&current, preview.action)?;
            c.check()?;
            if preview.used.swap(true, Ordering::AcqRel) {
                return fail(ForgeFailure::PullReviewStale);
            }
            if !self.config.provider().is_github(){return self.provider_submit_review(preview,&current,c);}
            let path = self.pull_scope(&preview.repository, preview.summary.number)?;
            let body = format!(
                "{{\"commit_id\":{},\"event\":{},\"body\":{}{}}}",
                quote(&current.head),
                quote(preview.action.event()),
                quote(&preview.body),
                inline::payload(&preview.comments)
            );
            // GitHub has no conditional review POST. commit_id pins a racing
            // submission to the reviewed commit even if a push lands now.
            let response = self
                .request(
                    SUBMIT,
                    "publish_pull_review",
                    "POST",
                    &format!("{path}/reviews"),
                    Some(&body),
                    true,
                    c,
                )
                .map_err(|e| match e {
                    Error::Forge(ForgeFailure::OutcomeUnknown) => {
                        Error::Forge(ForgeFailure::PullReviewUnknown)
                    }
                    Error::Forge(ForgeFailure::Rejected) => {
                        Error::Forge(ForgeFailure::PullReviewRejected)
                    }
                    other => other,
                })?;
            let result = review(&response.0)
                .filter(|r| {
                    r.author.eq_ignore_ascii_case(&self.login)
                        && r.commit.as_deref() == Some(&current.head)
                        && r.state == preview.action.state()
                        && !r.date.is_empty()
                })
                .ok_or(Error::Forge(ForgeFailure::PullReviewUnknown))?;
            // A verified response wins over late cancellation; never conceal a
            // confirmed published review just because the user pressed Stop.
            Ok(result)
        })
    }
}
fn same_revision(a: &ForgePullSummary, b: &ForgePullSummary) -> bool {
    a.number == b.number
        && a.head == b.head
        && a.base == b.base
        && a.source == b.source
        && a.target == b.target
        && a.state == b.state
        && a.merged == b.merged
        && a.draft == b.draft
        && a.author == b.author
        && a.author_identity == b.author_identity
        && a.revision == b.revision
}
fn oid(s: &str) -> Option<String> {
    (s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())).then(|| s.to_ascii_lowercase())
}
fn string(j: &Json, k: &str, max: usize) -> Option<String> {
    j.str(k)
        .filter(|s| s.len() <= max)
        .map(|s| crate::display_text::bounded(s, max).text)
}
fn optional_string(j: &Json, k: &str, max: usize) -> Option<String> {
    string(j, k, max)
}
fn bounded(s: &str, max: usize) -> String {
    crate::display_text::bounded(s, max).text
}
fn multiline(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s
            .split('\n')
            .map(|line| bounded(line, max))
            .collect::<Vec<_>>()
            .join("\n");
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n[Preview truncated; open GitHub for the full text.]",
        multiline(&s[..end], max)
    )
}
fn body(j: &Json, max: usize) -> String {
    multiline(j.str("body").unwrap_or(""), max)
}
fn user(j: &Json) -> Option<String> {
    Some(
        j.get("user")
            .and_then(|u| string(u, "login", 256))
            .unwrap_or_else(|| "Deleted user".into()),
    )
}
fn comment(j: &Json, inline: bool) -> Option<ForgePullComment> {
    Some(ForgePullComment {
        id: j.u64("id").filter(|n| *n > 0)?,
        author: user(j)?,
        body: body(j, 8192),
        date: string(j, "created_at", 64)?,
        path: if inline {
            Some(string(j, "path", 8192)?)
        } else {
            None
        },
        line: j.u64("line"),
        reply_to: j.u64("in_reply_to_id"),
    })
}
fn review(j: &Json) -> Option<ForgePullReview> {
    let state = j.str("state")?;
    if !matches!(
        state,
        "APPROVED" | "COMMENTED" | "CHANGES_REQUESTED" | "DISMISSED" | "PENDING"
    ) {
        return None;
    }
    Some(ForgePullReview {
        id: j.u64("id").filter(|n| *n > 0)?,
        author: user(j)?,
        body: body(j, 8192),
        state: state.into(),
        commit: j.str("commit_id").and_then(oid),
        date: string(j, "submitted_at", 64).unwrap_or_default(),
    })
}
