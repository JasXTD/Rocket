//! Provider-specific PR contracts. URLs are constructed from validated scopes;
//! returned links are never followed. Each collection is an explicit first-page
//! preview. Remote permission/merge rules remain authoritative at submission.
use super::*;
mod inline_reviews;
mod other_inline;
mod azure_diff;

pub(super) fn supports_review(p: ForgeProvider, a: ForgeReviewAction) -> bool {
    !(p == ForgeProvider::GitLab && a == ForgeReviewAction::RequestChanges)
}
pub(super) fn message_supported(p: ForgeProvider, a: ForgeReviewAction) -> bool {
    a == ForgeReviewAction::Comment || p.is_github() || p == ForgeProvider::Gitea
}
pub(super) fn supports_merge(p: ForgeProvider, m: ForgeMergeMethod) -> bool {
    m != ForgeMergeMethod::Rebase || p.is_github() || p == ForgeProvider::Gitea || p.is_azure()
}
pub(super) fn merge_condition(p: ForgeProvider) -> &'static str {
    match p {
        ForgeProvider::GitLab => {
            "The request includes the reviewed head. The project's configured merge method still applies; Rocket does not override it. Target tips can change after Rocket's check, and GitLab enforces its merge rules."
        }
        ForgeProvider::Bitbucket => {
            "Bitbucket Cloud does not document an expected-head condition for this endpoint. Both tips are rechecked immediately before sending, but either can change afterward. Inspect the final PR before relying on the result."
        }
        ForgeProvider::BitbucketServer => {
            "The server checks the reviewed PR version. Branch tips are rechecked before sending, but can change afterward. Server merge checks remain authoritative."
        }
        ForgeProvider::Azure | ForgeProvider::AzureServer => {
            "The request includes the reviewed source commit. Azure may finish asynchronously; an unconfirmed result requires a fresh PR inspection. Target policy is enforced by Azure."
        }
        _ => {
            "The request includes the reviewed head condition. Rocket also rechecks the target, which can still change after that check. The provider enforces its merge rules."
        }
    }
}
pub(super) fn validate_review(
    p: ForgeProvider,
    a: ForgeReviewAction,
    body: &str,
    comments: &[ForgeInlineComment],
) -> Result<()> {
    if !supports_review(p, a) {
        return Err(invalid(
            "This provider adapter cannot request changes as a review vote. Use a comment or open the provider.",
        ));
    }
    if !message_supported(p, a) && !body.is_empty() {
        return Err(invalid(
            "This provider's approval/vote endpoint does not accept a message. Publish a separate comment first; Rocket never silently drops review text or chains two writes.",
        ));
    }
    if !p.is_github() && p != ForgeProvider::Gitea && !comments.is_empty()
        && (comments.len() != 1 || a != ForgeReviewAction::Comment || !body.is_empty()) {
        return Err(invalid("This provider sends one inline discussion per review. Select Comment, leave the separate review message empty, and send other comments or votes separately. Rocket never chains partial writes."));
    }
    Ok(())
}
fn need<T>(v: Option<T>) -> Result<T> {
    v.ok_or(Error::Forge(ForgeFailure::InvalidResponse))
}
fn at<'a>(j: &'a Json, keys: &[&str]) -> Option<&'a Json> {
    keys.iter().try_fold(j, |v, k| v.get(k))
}
fn txt<'a>(j: &'a Json, keys: &[&str]) -> Option<&'a str> {
    match at(j, keys) {
        Some(Json::String(s)) => Some(s),
        _ => None,
    }
}
fn author(j: &Json, p: ForgeProvider) -> String {
    let keys: &[&str] = match p {
        ForgeProvider::GitLab => &["username", "name"],
        ForgeProvider::Bitbucket => &["display_name", "nickname"],
        ForgeProvider::BitbucketServer => &["displayName", "name"],
        ForgeProvider::Azure | ForgeProvider::AzureServer => &["displayName"],
        _ => &["login", "username"],
    };
    bounded(
        keys.iter()
            .find_map(|k| j.str(k))
            .unwrap_or("Unknown author"),
        512,
    )
}
pub(super) fn identity(j: &Json, p: ForgeProvider) -> Option<String> {
    let key = match p {
        ForgeProvider::GitLab => "username",
        ForgeProvider::Bitbucket => "uuid",
        ForgeProvider::BitbucketServer => "name",
        ForgeProvider::Azure | ForgeProvider::AzureServer => "id",
        _ => "login",
    };
    j.str(key)
        .filter(|v| !v.is_empty() && v.len() <= 512 && !v.chars().any(char::is_control))
        .map(str::to_owned)
}
fn unavailable<T>() -> ForgePullCollection<T> {
    ForgePullCollection {
        error: Some(ForgeFailure::Unsupported),
        ..Default::default()
    }
}
fn file(j: &Json, p: ForgeProvider) -> Option<ForgePullFile> {
    let (path, previous, status, patch, add, del) = match p {
        ForgeProvider::GitLab => (
            j.str("new_path")?,
            j.str("old_path").filter(|x| Some(*x) != j.str("new_path")),
            if j.bool("renamed_file") == Some(true) {
                "renamed"
            } else if j.bool("new_file") == Some(true) {
                "added"
            } else if j.bool("deleted_file") == Some(true) {
                "removed"
            } else {
                "modified"
            },
            j.str("diff"),
            0,
            0,
        ),
        ForgeProvider::Bitbucket => (
            txt(j, &["new", "path"]).or_else(|| txt(j, &["old", "path"]))?,
            txt(j, &["old", "path"]).filter(|x| Some(*x) != txt(j, &["new", "path"])),
            j.str("status").unwrap_or("modified"),
            None,
            j.u64("lines_added").unwrap_or(0),
            j.u64("lines_removed").unwrap_or(0),
        ),
        ForgeProvider::BitbucketServer => (
            txt(j, &["path", "toString"])?,
            txt(j, &["srcPath", "toString"]),
            j.str("type")?,
            None,
            0,
            0,
        ),
        ForgeProvider::Azure | ForgeProvider::AzureServer => (
            txt(j, &["item", "path"])?,
            j.str("originalPath"),
            j.str("changeType").unwrap_or("changed"),
            None,
            0,
            0,
        ),
        _ => (
            j.str("filename")?,
            j.str("previous_filename"),
            j.str("status").unwrap_or("modified"),
            None, // Gitea's file-list contract contains metadata, not diff hunks.
            j.u64("additions").unwrap_or(0),
            j.u64("deletions").unwrap_or(0),
        ),
    };
    if path.len() > 8192 || previous.is_some_and(|v| v.len() > 8192) {
        return None;
    }
    let patch = patch
        .filter(|s| {
            s.len() <= 16384
                && j.bool("collapsed") != Some(true)
                && j.bool("too_large") != Some(true)
        })
        ;
    let lines = patch.and_then(inline::parse_lines).unwrap_or_default();
    Some(ForgePullFile {
        path: bounded(path, 8192),
        previous: previous.map(|s| bounded(s, 8192)),
        status: bounded(status, 64),
        additions: add,
        deletions: del,
        lines,
        patch: patch.map(|s| multiline(s,16384)),
        raw_path: other_inline::safe_provider_path(p, path).then(||path.to_owned()),
        raw_previous: previous.filter(|s| other_inline::safe_provider_path(p, s)).map(str::to_owned),
        anchor: None,
    })
}
fn commit(j: &Json, p: ForgeProvider) -> Option<ForgePullCommit> {
    let id = match p {
        ForgeProvider::GitLab | ForgeProvider::BitbucketServer => j.str("id"),
        ForgeProvider::Bitbucket => j.str("hash"),
        ForgeProvider::Azure | ForgeProvider::AzureServer => j.str("commitId"),
        _ => j.str("sha"),
    }
    .and_then(oid)?;
    let text = j
        .str("message")
        .or_else(|| j.str("comment"))
        .or_else(|| txt(j, &["commit", "message"]))
        .unwrap_or("");
    let name = j
        .str("author_name")
        .or_else(|| txt(j, &["author", "name"]))
        .or_else(|| txt(j, &["author", "raw"]))
        .or_else(|| txt(j, &["commit", "author", "name"]))
        .unwrap_or("Unknown author");
    Some(ForgePullCommit {
        id,
        subject: bounded(text.lines().next().unwrap_or(""), 4096),
        author: bounded(name, 512),
    })
}
pub(super) fn note(j: &Json, p: ForgeProvider) -> Option<ForgePullComment> {
    let user = match p {
        ForgeProvider::Bitbucket | ForgeProvider::Gitea => j.get("user"),
        _ => j.get("author"),
    }?;
    let text = match p {
        ForgeProvider::Bitbucket => txt(j, &["content", "raw"]),
        ForgeProvider::BitbucketServer => j.str("text"),
        ForgeProvider::Azure | ForgeProvider::AzureServer => j.str("content"),
        _ => j.str("body"),
    }
    .unwrap_or("");
    Some(ForgePullComment {
        id: j.u64("id")?,
        author: author(user, p),
        body: multiline(text, 8192),
        date: j
            .str("created_at")
            .or_else(|| j.str("created_on"))
            .or_else(|| j.str("publishedDate"))
            .map(|v| bounded(v, 64))
            .unwrap_or_default(),
        path: None,
        line: None,
        reply_to: None,
    })
}
impl ForgeSession {
    pub(super) fn provider_path(&self, repo: &ForgeRepository, n: u64) -> Result<String> {
        self.check_repository(repo)?;
        if n == 0 || n > i32::MAX as u64 {
            return Err(invalid("Select a valid pull request."));
        }
        let prefix = if self.config.provider() == ForgeProvider::BitbucketServer {
            let (p, r) = repo.parts();
            format!("/projects/{}/repos/{}", encode(p), encode(r))
        } else {
            self.prefix(repo)
        };
        let noun = match self.config.provider() {
            ForgeProvider::GitLab => "merge_requests",
            ForgeProvider::Bitbucket | ForgeProvider::Azure | ForgeProvider::AzureServer => {
                "pullrequests"
            }
            ForgeProvider::BitbucketServer => "pull-requests",
            _ => "pulls",
        };
        Ok(format!("{prefix}/{noun}/{n}"))
    }
    pub(super) fn versioned(&self, path: &str) -> String {
        if self.config.provider().is_azure() {
            format!(
                "{path}{}api-version={}",
                if path.contains('?') { "&" } else { "?" },
                self.config.azure_api_version()
            )
        } else {
            path.into()
        }
    }
    fn provider_raw(
        &self,
        op: &'static str,
        repo: &ForgeRepository,
        n: u64,
        c: &CancellationToken,
    ) -> Result<Json> {
        self.request(
            op,
            "read_pull_details",
            "GET",
            &self.versioned(&self.provider_path(repo, n)?),
            None,
            false,
            c,
        )
        .map(|r| r.0)
    }
    pub(super) fn current_pull(
        &self,
        op: &'static str,
        repo: &ForgeRepository,
        n: u64,
        c: &CancellationToken,
    ) -> Result<ForgePullSummary> {
        if self.config.provider().is_github() {
            return self.pull_summary(op, &self.pull_scope(repo, n)?, n, c);
        }
        let j = self.provider_raw(op, repo, n, c)?;
        self.provider_summary(repo, n, &j, c)
    }
    fn provider_summary(
        &self,
        repo: &ForgeRepository,
        n: u64,
        j: &Json,
        c: &CancellationToken,
    ) -> Result<ForgePullSummary> {
        let p = self.config.provider();
        let id_key = match p {
            ForgeProvider::GitLab => "iid",
            ForgeProvider::Azure | ForgeProvider::AzureServer => "pullRequestId",
            ForgeProvider::Gitea => "number",
            _ => "id",
        };
        if j.u64(id_key) != Some(n) {
            return fail(ForgeFailure::InvalidResponse);
        }
        let (head, source, target, state, merged, draft, mergeable, merge_state, who) = match p {
            ForgeProvider::GitLab => (
                j.str("sha"),
                j.str("source_branch"),
                j.str("target_branch"),
                j.str("state"),
                j.str("state") == Some("merged"),
                j.bool("draft")
                    .or_else(|| j.bool("work_in_progress"))
                    .unwrap_or(false),
                j.str("detailed_merge_status").map(|s| s == "mergeable"),
                j.str("detailed_merge_status"),
                j.get("author"),
            ),
            ForgeProvider::Bitbucket => (
                txt(j, &["source", "commit", "hash"]),
                txt(j, &["source", "branch", "name"]),
                txt(j, &["destination", "branch", "name"]),
                j.str("state"),
                j.str("state") == Some("MERGED"),
                j.bool("draft").unwrap_or(false),
                None,
                None,
                j.get("author"),
            ),
            ForgeProvider::BitbucketServer => (
                txt(j, &["fromRef", "latestCommit"]),
                txt(j, &["fromRef", "id"]),
                txt(j, &["toRef", "id"]),
                j.str("state"),
                j.str("state") == Some("MERGED"),
                j.bool("draft").unwrap_or(false),
                None,
                None,
                at(j, &["author", "user"]),
            ),
            ForgeProvider::Azure | ForgeProvider::AzureServer => (
                txt(j, &["lastMergeSourceCommit", "commitId"]),
                j.str("sourceRefName"),
                j.str("targetRefName"),
                j.str("status"),
                j.str("status") == Some("completed"),
                j.bool("isDraft").unwrap_or(false),
                j.str("mergeStatus").map(|s| s == "succeeded"),
                j.str("mergeStatus"),
                j.get("createdBy"),
            ),
            ForgeProvider::Gitea => (
                txt(j, &["head", "sha"]),
                txt(j, &["head", "ref"]),
                txt(j, &["base", "ref"]),
                j.str("state"),
                j.bool("merged").unwrap_or(false),
                j.bool("draft")
                    .or_else(|| j.bool("work_in_progress"))
                    .unwrap_or(false),
                j.bool("mergeable"),
                None,
                j.get("user"),
            ),
            _ => return fail(ForgeFailure::Unsupported),
        };
        let raw_state = need(state)?;
        if !matches!(
            raw_state,
            "opened"
                | "open"
                | "OPEN"
                | "active"
                | "closed"
                | "merged"
                | "locked"
                | "MERGED"
                | "DECLINED"
                | "SUPERSEDED"
                | "completed"
                | "abandoned"
        ) {
            return fail(ForgeFailure::InvalidResponse);
        }
        let branch = |v: Option<&str>| -> Result<String> {
            let v = need(v)?;
            let v = if p.is_azure() || p == ForgeProvider::BitbucketServer {
                v.strip_prefix("refs/heads/")
                    .ok_or_else(|| invalid("The PR does not reference a branch."))?
            } else {
                v
            };
            validate_branch(v)?;
            Ok(v.into())
        };
        let source = branch(source)?;
        let target = branch(target)?;
        let base = if merged {
            match p {
                ForgeProvider::GitLab => txt(j, &["diff_refs", "start_sha"]),
                ForgeProvider::Bitbucket => txt(j, &["destination", "commit", "hash"]),
                ForgeProvider::BitbucketServer => txt(j, &["toRef", "latestCommit"]),
                ForgeProvider::Azure | ForgeProvider::AzureServer => {
                    txt(j, &["lastMergeTargetCommit", "commitId"])
                }
                _ => txt(j, &["base", "sha"]),
            }
            .and_then(oid)
            .map(Ok)
            .unwrap_or_else(|| self.branch_id(repo, &target, c))?
        } else {
            self.branch_id(repo, &target, c)?
        };
        let who = need(who)?;
        Ok(ForgePullSummary {
            number: n,
            title: need(string(j, "title", 4096))?,
            body: multiline(
                j.str(if p == ForgeProvider::Gitea {
                    "body"
                } else {
                    "description"
                })
                .unwrap_or(""),
                65536,
            ),
            author: author(who, p),
            author_identity: identity(who, p),
            diff_refs: if p == ForgeProvider::GitLab {
                j.get("diff_refs").and_then(|r| Some(DiffRefs {
                    base: oid(r.str("base_sha")?)?, head: oid(r.str("head_sha")?)?, start: oid(r.str("start_sha")?)?,
                }))
            } else { None },
            state: if matches!(raw_state, "opened" | "open" | "OPEN" | "active") {
                "open"
            } else {
                "closed"
            }
            .into(),
            draft,
            merged,
            mergeable,
            mergeable_state: merge_state.map(|s| bounded(s, 64)),
            head: need(head.and_then(oid))?,
            base: need(oid(&base))?,
            source,
            target,
            commits: j.u64("commits").unwrap_or(0),
            files: j.u64("changed_files").unwrap_or(0),
            additions: j.u64("additions").unwrap_or(0),
            deletions: j.u64("deletions").unwrap_or(0),
            counts_available: ["commits", "changed_files", "additions", "deletions"]
                .iter()
                .all(|k| j.u64(k).is_some()),
            revision: if p == ForgeProvider::BitbucketServer {
                Some(need(j.u64("version"))?)
            } else {
                None
            },
        })
    }
    fn first_page<T>(
        &self,
        path: &str,
        key: Option<&str>,
        step: &'static str,
        c: &CancellationToken,
        mut parse: impl FnMut(&Json) -> Option<T>,
    ) -> Result<ForgePullCollection<T>> {
        let mut run = || {
            let (j, headers) =
                self.request(READ, step, "GET", &self.versioned(path), None, false, c)?;
            let rows = need(
                key.map_or(Some(&j), |k| j.get(k))
                    .and_then(Json::array)
                    .filter(|a| a.len() <= 200),
            )?;
            let mut items = Vec::new();
            for row in rows {
                c.check()?;
                items.push(need(parse(row))?);
            }
            // No returned URL is followed. A full page or continuation metadata
            // is explicitly partial, even when an adapter cannot prove a total.
            let limited = rows.len() >= 50
                || j.str("next").is_some()
                || j.bool("isLastPage") == Some(false)
                || j.u64("nextSkip").is_some_and(|n| n > 0)
                || j.u64("nextTop").is_some_and(|n| n > 0)
                || headers.iter().any(|(k, v)| {
                    (k.eq_ignore_ascii_case("link") && v.contains("rel=\"next\""))
                        || k.eq_ignore_ascii_case("x-ms-continuationtoken")
                        || (k.eq_ignore_ascii_case("x-next-page") && !v.is_empty())
                });
            Ok(ForgePullCollection {
                items,
                limited,
                error: None,
            })
        };
        match run() {
            Err(Error::Forge(e)) if e != ForgeFailure::Unauthorized => Ok(ForgePullCollection {
                error: Some(e),
                ..Default::default()
            }),
            other => other,
        }
    }
    pub(super) fn provider_pull_details(
        &self,
        repo: &ForgeRepository,
        n: u64,
        c: &CancellationToken,
    ) -> Result<ForgePullDetails> {
        let p = self.config.provider();
        let path = self.provider_path(repo, n)?;
        let raw = self.provider_raw(READ, repo, n, c)?;
        let first = self.provider_summary(repo, n, &raw, c)?;
        let suffix = match p {
            ForgeProvider::GitLab => "?per_page=50&page=1",
            ForgeProvider::Bitbucket => "?pagelen=50&page=1",
            ForgeProvider::BitbucketServer => "?limit=50&start=0",
            ForgeProvider::Azure | ForgeProvider::AzureServer => "?$top=50",
            _ => "?limit=50&page=1",
        };
        let key = if p == ForgeProvider::Bitbucket || p == ForgeProvider::BitbucketServer {
            Some("values")
        } else if p.is_azure() {
            Some("value")
        } else {
            None
        };
        let commits = self.first_page(
            &format!("{path}/commits{suffix}"),
            key,
            "read_pull_commits",
            c,
            |j| commit(j, p),
        )?;
        let mut files = if p.is_azure() {
            match self.azure_files(repo, &path, &first.head, c) {
                Err(Error::Forge(e)) if e != ForgeFailure::Unauthorized && e != ForgeFailure::PullChanged => ForgePullCollection { error:Some(e), ..Default::default() },
                result => result?,
            }
        } else {
            let endpoint = match p {
                ForgeProvider::GitLab => format!("{path}/diffs{suffix}"),
                ForgeProvider::Bitbucket => format!(
                    "{}/diffstat/{}..{}?pagelen=50&topic=true",
                    self.prefix(repo),
                    first.head,
                    first.base
                ),
                ForgeProvider::BitbucketServer => format!("{path}/changes{suffix}"),
                _ => format!("{path}/files{suffix}"),
            };
            self.first_page(&endpoint, key, "read_pull_files", c, |j| file(j, p))?
        };
        if p == ForgeProvider::Gitea && files.error.is_none() && !files.items.is_empty() {
            match self.request_diff(&format!("{path}.diff?binary=false"), c).and_then(|diff| inline_reviews::attach_gitea_diff(&mut files.items, &diff, c)) {
                Ok(()) => (),
                Err(Error::Forge(e)) if e != ForgeFailure::Unauthorized => { files.error = Some(e); }
                Err(e) => return Err(e),
            }
        }
        if files.error.is_none() && !files.items.is_empty() && matches!(p, ForgeProvider::Bitbucket | ForgeProvider::BitbucketServer) {
            let result = if p == ForgeProvider::Bitbucket {
                self.request_diff(&format!("{}/diff/{}..{}?topic=true&context=3",self.prefix(repo),first.head,first.base), c)
                    .and_then(|diff|inline_reviews::attach_gitea_diff(&mut files.items,&diff,c)).map(|_|false)
            } else {
                self.request(READ,"read_pull_diff","GET",&format!("{path}/diff?contextLines=3&withComments=false"),None,false,c)
                    .and_then(|(j,_)|other_inline::attach_server_diff(&mut files.items,&j,&first.head,c))
            };
            match result { Ok(limited)=>files.limited |= limited,
                Err(Error::Forge(e)) if e!=ForgeFailure::Unauthorized && e!=ForgeFailure::PullChanged => files.error=Some(e), Err(e)=>return Err(e) }
        }
        let mut other_inline_comments = None;
        let discussion = if p.is_azure() || matches!(p,ForgeProvider::Bitbucket|ForgeProvider::BitbucketServer) {
            let (general, inline) = self.other_discussions(&path,c)?;
            other_inline_comments=Some(inline);
            general
        } else {
            let endpoint = if p == ForgeProvider::GitLab {
                format!("{path}/notes{suffix}")
            } else if p == ForgeProvider::Gitea {
                format!("/repos/{}/issues/{n}/comments{suffix}", repo.encoded())
            } else {
                format!("{path}/comments{suffix}")
            };
            self.first_page(&endpoint, key, "read_pull_discussion", c, |j| note(j, p))?
        };
        let reviews = match p {
            ForgeProvider::Gitea => self.first_page(
                &format!("{path}/reviews{suffix}"),
                None,
                "read_pull_reviews",
                c,
                |j| provider_review(j, p),
            )?,
            ForgeProvider::GitLab => self.first_page(
                &format!("{path}/approvals"),
                Some("approved_by"),
                "read_pull_reviews",
                c,
                |j| provider_review(j, p),
            )?,
            _ => {
                let rows = need(
                    raw.get(if p.is_azure() {
                        "reviewers"
                    } else if p == ForgeProvider::Bitbucket {
                        "participants"
                    } else {
                        "reviewers"
                    })
                    .and_then(Json::array)
                    .filter(|a| a.len() <= 200),
                )?;
                let mut items = vec![];
                for row in rows {
                    items.push(need(provider_review(row, p))?);
                }
                ForgePullCollection {
                    items,
                    limited: false,
                    error: None,
                }
            }
        };
        let inline_comments = match other_inline_comments { Some(rows)=>rows, None=>self.provider_inline_comments(&path, &reviews, c)? };
        let endpoint = match p {
            ForgeProvider::GitLab => format!("{path}/pipelines{suffix}"),
            ForgeProvider::Gitea => format!(
                "/repos/{}/commits/{}/statuses{suffix}",
                repo.encoded(),
                first.head
            ),
            _ => format!("{path}/statuses{suffix}"),
        };
        let checks = if p == ForgeProvider::BitbucketServer {
            unavailable()
        } else {
            self.first_page(&endpoint, key, "read_pull_checks", c, |j| {
                Some(ForgePullCheck {
                    name: bounded(
                        j.str("name")
                            .or_else(|| j.str("context"))
                            .or_else(|| txt(j, &["context", "name"]))
                            .unwrap_or("Pipeline"),
                        1024,
                    ),
                    status: bounded(j.str("state").or_else(|| j.str("status"))?, 64),
                    conclusion: None,
                })
            })?
        };
        let last = self.current_pull(READ, repo, n, c)?;
        if !same_revision(&first, &last) {
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
            statuses: unavailable(),
        })
    }
    pub(super) fn provider_submit_review(
        &self,
        pv: &ForgePullReviewPreview,
        current: &ForgePullSummary,
        c: &CancellationToken,
    ) -> Result<ForgePullReview> {
        let p = self.config.provider();
        validate_review(p, pv.action, &pv.body, &pv.comments)?;
        let path = self.provider_path(&pv.repository, current.number)?;
        if p == ForgeProvider::GitLab && !pv.comments.is_empty() {
            return self.submit_gitlab_inline(&path, pv, c);
        }
        if !pv.comments.is_empty() && p != ForgeProvider::Gitea {
            return self.submit_other_inline(&path, pv, c);
        }
        let (method, endpoint, payload) = match (p, pv.action) {
            (ForgeProvider::GitLab, ForgeReviewAction::Comment) => (
                "POST",
                format!("{path}/notes"),
                format!("{{\"body\":{}}}", quote(&pv.body)),
            ),
            (ForgeProvider::GitLab, ForgeReviewAction::Approve) => (
                "POST",
                format!("{path}/approve"),
                format!("{{\"sha\":{}}}", quote(&current.head)),
            ),
            (ForgeProvider::Bitbucket, ForgeReviewAction::Comment) => (
                "POST",
                format!("{path}/comments"),
                format!("{{\"content\":{{\"raw\":{}}}}}", quote(&pv.body)),
            ),
            (ForgeProvider::Bitbucket, a) => (
                "POST",
                format!(
                    "{path}/{}",
                    if a == ForgeReviewAction::Approve {
                        "approve"
                    } else {
                        "request-changes"
                    }
                ),
                "{}".into(),
            ),
            (ForgeProvider::BitbucketServer, ForgeReviewAction::Comment) => (
                "POST",
                format!("{path}/comments"),
                format!("{{\"text\":{}}}", quote(&pv.body)),
            ),
            (ForgeProvider::BitbucketServer, a) => {
                let users = self
                    .request(
                        SUBMIT,
                        "read_reviewer_identity",
                        "GET",
                        &format!("/users?filter={}&limit=50&start=0", encode(&self.login)),
                        None,
                        false,
                        c,
                    )?
                    .0;
                let rows = need(
                    users
                        .get("values")
                        .and_then(Json::array)
                        .filter(|a| a.len() <= 50),
                )?;
                let matches: Vec<_> = rows
                    .iter()
                    .filter(|u| u.str("name") == Some(&self.login))
                    .collect();
                let slug = matches
                    .first()
                    .filter(|_| matches.len() == 1)
                    .and_then(|u| u.str("slug"))
                    .filter(|s| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control))
                    .ok_or(Error::Forge(ForgeFailure::IdentityUnverified))?;
                (
                    "PUT",
                    format!("{path}/participants/{}", encode(slug)),
                    format!(
                        "{{\"status\":{}}}",
                        quote(if a == ForgeReviewAction::Approve {
                            "APPROVED"
                        } else {
                            "NEEDS_WORK"
                        })
                    ),
                )
            }
            (ForgeProvider::Azure | ForgeProvider::AzureServer, ForgeReviewAction::Comment) => (
                "POST",
                format!("{path}/threads"),
                format!(
                    "{{\"comments\":[{{\"parentCommentId\":0,\"content\":{},\"commentType\":1}}],\"status\":1}}",
                    quote(&pv.body)
                ),
            ),
            (ForgeProvider::Azure | ForgeProvider::AzureServer, a) => {
                let id = self
                    .account_identity()
                    .filter(|s| {
                        s.len() == 36 && s.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
                    })
                    .ok_or(Error::Forge(ForgeFailure::IdentityUnverified))?;
                (
                    "PUT",
                    format!("{path}/reviewers/{}", encode(id)),
                    format!(
                        "{{\"id\":{},\"vote\":{}}}",
                        quote(id),
                        if a == ForgeReviewAction::Approve {
                            10
                        } else {
                            -10
                        }
                    ),
                )
            }
            (ForgeProvider::Gitea, a) => (
                "POST",
                format!("{path}/reviews"),
                format!(
                    "{{\"commit_id\":{},\"event\":{},\"body\":{}{}}}",
                    quote(&current.head),
                    quote(match a {
                        ForgeReviewAction::Approve => "APPROVED",
                        ForgeReviewAction::Comment => "COMMENT",
                        ForgeReviewAction::RequestChanges => "REQUEST_CHANGES",
                    }),
                    quote(&pv.body), inline::gitea_payload(&pv.comments)
                ),
            ),
            _ => return fail(ForgeFailure::Unsupported),
        };
        let j = self
            .request(
                SUBMIT,
                "publish_pull_review",
                method,
                &self.versioned(&endpoint),
                Some(&payload),
                true,
                c,
            )
            .map_err(|e| {
                // Gitea creates pending comments before submitting the review;
                // even an HTTP rejection can leave a partially saved draft.
                if p == ForgeProvider::Gitea && !pv.comments.is_empty() {
                    Error::Forge(ForgeFailure::PullReviewUnknown)
                } else { review_error(e) }
            })?
            .0;
        let review = if p == ForgeProvider::Gitea {
            provider_review(&j, p).filter(|r| {
                r.id > 0 && r.author.eq_ignore_ascii_case(&self.login)
                    && r.state == pv.action.state()
                    && r.commit.as_deref() == Some(&current.head)
                    && (pv.comments.is_empty() || j.u64("comments_count") == Some(pv.comments.len() as u64))
            })
        } else if pv.action == ForgeReviewAction::Comment {
            let raw = if p.is_azure() {
                j.get("comments")
                    .and_then(Json::array)
                    .filter(|a| a.len() == 1)
                    .and_then(|a| a.first())
            } else {
                Some(&j)
            };
            raw.and_then(|raw| {
                let actor = raw.get(if p == ForgeProvider::Bitbucket {
                    "user"
                } else {
                    "author"
                })?;
                if identity(actor, p).as_deref() != self.account_identity() {
                    return None;
                }
                let n = note(raw, p)?;
                Some(ForgePullReview {
                    id: n.id,
                    author: n.author,
                    body: n.body,
                    state: "COMMENTED".into(),
                    commit: None,
                    date: n.date,
                })
            })
        } else if p == ForgeProvider::GitLab {
            if j.u64("iid") != Some(current.number) {
                None
            } else {
                j.get("approved_by")
                    .and_then(Json::array)
                    .and_then(|rows| {
                        rows.iter().find(|r| {
                            r.get("user").and_then(|u| identity(u, p)).as_deref()
                                == self.account_identity()
                        })
                    })
                    .and_then(|r| provider_review(r, p))
            }
        } else {
            let actor = if p.is_azure() {
                Some(&j)
            } else {
                j.get("user")
            };
            actor
                .filter(|a| identity(a, p).as_deref() == self.account_identity())
                .and_then(|_| provider_review(&j, p))
                .filter(|r| r.state == pv.action.state())
        };
        review.ok_or(Error::Forge(ForgeFailure::PullReviewUnknown))
    }
    pub(super) fn provider_merge_allowed(
        &self,
        op: &'static str,
        repo: &ForgeRepository,
        s: &ForgePullSummary,
        m: ForgeMergeMethod,
        c: &CancellationToken,
    ) -> Result<()> {
        if !supports_merge(self.config.provider(), m) || s.state != "open" || s.draft || s.merged {
            return fail(ForgeFailure::PullMergeBlocked);
        }
        match self.config.provider() {
            ForgeProvider::GitLab => {
                if s.mergeable_state.as_deref() != Some("mergeable") {
                    return fail(ForgeFailure::PullMergeBlocked);
                }
            }
            ForgeProvider::BitbucketServer => {
                let j = self
                    .request(
                        op,
                        "read_merge_permissions",
                        "GET",
                        &format!("{}/merge", self.provider_path(repo, s.number)?),
                        None,
                        false,
                        c,
                    )?
                    .0;
                if j.bool("canMerge") != Some(true) || j.bool("conflicted") != Some(false) {
                    return fail(ForgeFailure::PullMergeBlocked);
                }
            }
            ForgeProvider::Azure | ForgeProvider::AzureServer => {
                if s.mergeable_state.as_deref() != Some("succeeded") {
                    return fail(ForgeFailure::PullMergeBlocked);
                }
            }
            ForgeProvider::Gitea => {
                if s.mergeable != Some(true) {
                    return fail(ForgeFailure::PullMergeBlocked);
                }
                let j = self
                    .request(
                        op,
                        "read_merge_permissions",
                        "GET",
                        &self.prefix(repo),
                        None,
                        false,
                        c,
                    )?
                    .0;
                let flag = match m {
                    ForgeMergeMethod::Merge => "allow_merge_commits",
                    ForgeMergeMethod::Squash => "allow_squash_merge",
                    ForgeMergeMethod::Rebase => "allow_rebase",
                };
                if j.str("full_name") != Some(repo.path())
                    || j.bool("archived") != Some(false)
                    || j.bool(flag) != Some(true)
                    || at(&j, &["permissions"]).and_then(|v| v.bool("push")) != Some(true)
                {
                    return fail(ForgeFailure::PullMergeBlocked);
                }
            }
            ForgeProvider::Bitbucket => { /* Cloud performs its merge checks at submission; no invented readiness flag. */
            }
            _ => return fail(ForgeFailure::Unsupported),
        }
        Ok(())
    }
    pub(super) fn provider_merge(
        &self,
        repo: &ForgeRepository,
        s: &ForgePullSummary,
        m: ForgeMergeMethod,
        c: &CancellationToken,
    ) -> Result<ForgePullMergeResult> {
        let p = self.config.provider();
        let path = self.provider_path(repo, s.number)?;
        let (method, endpoint, payload) = match p {
            ForgeProvider::GitLab => (
                "PUT",
                format!("{path}/merge"),
                format!(
                    "{{\"sha\":{},\"squash\":{},\"should_remove_source_branch\":false,\"auto_merge\":false}}",
                    quote(&s.head),
                    m == ForgeMergeMethod::Squash
                ),
            ),
            ForgeProvider::Bitbucket => (
                "POST",
                format!("{path}/merge"),
                format!(
                    "{{\"type\":\"pullrequest\",\"close_source_branch\":false,\"merge_strategy\":{}}}",
                    quote(if m == ForgeMergeMethod::Squash {
                        "squash"
                    } else {
                        "merge_commit"
                    })
                ),
            ),
            ForgeProvider::BitbucketServer => (
                "POST",
                format!("{path}/merge"),
                format!(
                    "{{\"version\":{},\"strategyId\":{},\"autoMerge\":false}}",
                    need(s.revision)?,
                    quote(if m == ForgeMergeMethod::Squash {
                        "squash"
                    } else {
                        "no-ff"
                    })
                ),
            ),
            ForgeProvider::Azure | ForgeProvider::AzureServer => (
                "PATCH",
                path.clone(),
                format!(
                    "{{\"status\":\"completed\",\"lastMergeSourceCommit\":{{\"commitId\":{}}},\"completionOptions\":{{\"mergeStrategy\":{},\"deleteSourceBranch\":false,\"bypassPolicy\":false,\"transitionWorkItems\":false}}}}",
                    quote(&s.head),
                    quote(match m {
                        ForgeMergeMethod::Merge => "noFastForward",
                        ForgeMergeMethod::Squash => "squash",
                        ForgeMergeMethod::Rebase => "rebase",
                    })
                ),
            ),
            ForgeProvider::Gitea => (
                "POST",
                format!("{path}/merge"),
                format!(
                    "{{\"do\":{},\"head_commit_id\":{},\"force_merge\":false,\"delete_branch_after_merge\":false,\"merge_when_checks_succeed\":false}}",
                    quote(m.key()),
                    quote(&s.head)
                ),
            ),
            _ => return fail(ForgeFailure::Unsupported),
        };
        let value = self
            .request(
                "hosting-merge-submit",
                "merge_reviewed_head",
                method,
                &self.versioned(&endpoint),
                Some(&payload),
                true,
                c,
            )
            .map_err(merge_error)?
            .0;
        // Empty acknowledgments and queued Azure/Cloud merges are not proof of
        // completion. A scoped read must confirm both the PR and reviewed head.
        let raw = if p == ForgeProvider::Gitea {
            self.provider_raw("hosting-merge-submit", repo, s.number, c)
                .map_err(|_| Error::Forge(ForgeFailure::PullMergeUnknown))?
        } else {
            value
        };
        let current = self
            .provider_summary(repo, s.number, &raw, c)
            .map_err(|_| Error::Forge(ForgeFailure::PullMergeUnknown))?;
        if !current.merged
            || current.head != s.head
            || current.source != s.source
            || current.target != s.target
        {
            return fail(ForgeFailure::PullMergeUnknown);
        }
        let commit = match p {
            ForgeProvider::GitLab => raw
                .str("merge_commit_sha")
                .or_else(|| raw.str("squash_commit_sha")),
            ForgeProvider::Bitbucket => txt(&raw, &["merge_commit", "hash"]),
            ForgeProvider::BitbucketServer => txt(&raw, &["properties", "mergeCommit", "id"]),
            ForgeProvider::Azure | ForgeProvider::AzureServer => {
                txt(&raw, &["lastMergeCommit", "commitId"])
            }
            _ => raw.str("merge_commit_sha"),
        }
        .and_then(oid)
        .ok_or(Error::Forge(ForgeFailure::PullMergeUnknown))?;
        Ok(ForgePullMergeResult { commit })
    }
}
fn provider_review(j: &Json, p: ForgeProvider) -> Option<ForgePullReview> {
    let actor = if p.is_azure() { j } else { j.get("user")? };
    let state = match p {
        ForgeProvider::GitLab => "APPROVED",
        ForgeProvider::Bitbucket => match j.str("state") {
            Some("approved") => "APPROVED",
            Some("changes_requested") => "CHANGES_REQUESTED",
            _ => {
                if j.bool("approved") == Some(true) {
                    "APPROVED"
                } else {
                    "PENDING"
                }
            }
        },
        ForgeProvider::BitbucketServer => match j.str("status") {
            Some("APPROVED") => "APPROVED",
            Some("NEEDS_WORK") => "CHANGES_REQUESTED",
            _ => "PENDING",
        },
        ForgeProvider::Azure | ForgeProvider::AzureServer => match j.get("vote") {
            Some(Json::Number(n)) if n == "10" => "APPROVED",
            Some(Json::Number(n)) if n == "5" => "APPROVED_WITH_SUGGESTIONS",
            Some(Json::Number(n)) if n == "-10" || n == "-5" => "CHANGES_REQUESTED",
            _ => "PENDING",
        },
        _ => match j.str("state")? {
            "APPROVED" => "APPROVED",
            "REQUEST_CHANGES" | "CHANGES_REQUESTED" => "CHANGES_REQUESTED",
            "COMMENT" | "COMMENTED" => "COMMENTED",
            "PENDING" => "PENDING",
            "REQUEST_REVIEW" => "REQUESTED",
            _ => return None,
        },
    };
    Some(ForgePullReview {
        id: j.u64("id").unwrap_or(0),
        author: author(actor, p),
        body: body(j, 8192),
        state: state.into(),
        commit: j.str("commit_id").and_then(oid),
        date: j
            .str("submitted_at")
            .or_else(|| j.str("updated_on"))
            .map(|s| bounded(s, 64))
            .unwrap_or_default(),
    })
}
fn review_error(e: Error) -> Error {
    match e {
        Error::Forge(ForgeFailure::OutcomeUnknown) => Error::Forge(ForgeFailure::PullReviewUnknown),
        Error::Forge(ForgeFailure::Rejected) => Error::Forge(ForgeFailure::PullReviewRejected),
        other => other,
    }
}
fn merge_error(e: Error) -> Error {
    match e {
        Error::Forge(ForgeFailure::OutcomeUnknown) => Error::Forge(ForgeFailure::PullMergeUnknown),
        Error::Forge(ForgeFailure::Rejected) => Error::Forge(ForgeFailure::PullMergeRejected),
        other => other,
    }
}
