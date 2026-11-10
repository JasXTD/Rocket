//! Provider contracts, not server-supplied routes. Writes always target a
//! discussion from a freshly revalidated snapshot and confirm the scoped result.
use super::super::providers::{identity, note};
use super::*;
mod parse;

const MAX_ROWS: usize = 200;
const MAX_THREADS: usize = 50;
const MAX_COMMENTS: usize = 20;
const MAX_DETAIL_READS: usize = 8;

impl ForgeSession {
    pub(super) fn provider_threads(
        &self,
        repo: &ForgeRepository,
        number: u64,
        c: &CancellationToken,
    ) -> Result<ForgePullThreads> {
        let path = self.provider_path(repo, number)?;
        let first = self.current_pull(READ, repo, number, c)?;
        let p = self.config.provider();
        let (mut threads, mut limited) = match p {
            ForgeProvider::GitLab => {
                let (rows, limited) =
                    self.thread_page(&format!("{path}/discussions?per_page=50&page=1"), None, c)?;
                let mut out = Vec::new();
                for row in &rows {
                    c.check()?;
                    if let Some(t) = parse::gitlab(row)? {
                        out.push(t);
                    }
                }
                (out, limited)
            }
            ForgeProvider::Bitbucket => {
                let (rows, limited) = self.thread_page(
                    &format!("{path}/comments?pagelen=50&page=1"),
                    Some("values"),
                    c,
                )?;
                parse::bitbucket(&rows, limited)?
            }
            ForgeProvider::BitbucketServer => {
                let (rows, mut limited) = self.thread_page(
                    &format!("{path}/activities?limit=50&start=0"),
                    Some("values"),
                    c,
                )?;
                let mut out = Vec::new();
                let mut seen = HashSet::new();
                for row in &rows {
                    c.check()?;
                    let Some(comment) = row.get("comment") else {
                        continue;
                    };
                    // Activities can repeat a thread and contain partial comment
                    // records. Read the original root by its numeric ID.
                    let id = required(comment.u64("id").filter(|n| *n > 0))?;
                    if comment
                        .get("parent")
                        .is_some_and(|v| !matches!(v, Json::Null))
                        || !seen.insert(id)
                    {
                        continue;
                    }
                    if out.len() == MAX_DETAIL_READS {
                        limited = true;
                        break;
                    }
                    let full = self.thread_get(&format!("{path}/comments/{id}"), c)?;
                    if full.u64("id") != Some(id) {
                        return fail(ForgeFailure::InvalidResponse);
                    }
                    out.push(parse::server(&full, row.get("commentAnchor"))?);
                }
                (out, limited)
            }
            ForgeProvider::Azure | ForgeProvider::AzureServer => {
                let (rows, limited) =
                    self.thread_page(&format!("{path}/threads"), Some("value"), c)?;
                let mut out = Vec::new();
                for row in &rows {
                    c.check()?;
                    if let Some(t) = parse::azure(row, p)? {
                        out.push(t);
                    }
                }
                (out, limited)
            }
            ForgeProvider::Gitea => {
                let (reviews, mut limited) =
                    self.thread_page(&format!("{path}/reviews?limit=50&page=1"), None, c)?;
                let mut out = Vec::new();
                let mut seen = HashSet::new();
                let mut reads = 0;
                for review in &reviews {
                    c.check()?;
                    if review.u64("comments_count") == Some(0)
                        || review.str("state") == Some("PENDING")
                    {
                        continue;
                    }
                    if reads == MAX_DETAIL_READS {
                        limited = true;
                        break;
                    }
                    let id = required(review.u64("id").filter(|n| *n > 0))?;
                    if !seen.insert(id) {
                        return fail(ForgeFailure::InvalidResponse);
                    }
                    reads += 1;
                    let (comments, partial) =
                        self.thread_page(&format!("{path}/reviews/{id}/comments"), None, c)?;
                    limited |= partial;
                    for comment in &comments {
                        if out.len() == MAX_THREADS {
                            limited = true;
                            break;
                        }
                        out.push(parse::gitea(comment, id)?);
                    }
                }
                (out, limited)
            }
            _ => return fail(ForgeFailure::Unsupported),
        };
        if threads.len() > MAX_THREADS {
            threads.truncate(MAX_THREADS);
            limited = true;
        }
        let mut ids = HashSet::new();
        if threads.iter().any(|t| !ids.insert(t.id.clone())) {
            return fail(ForgeFailure::InvalidResponse);
        }
        let last = self.current_pull(READ, repo, number, c)?;
        if !same_revision(&first, &last) {
            return fail(ForgeFailure::PullChanged);
        }
        Ok(ForgePullThreads {
            repository: repo.clone(),
            session: self.id,
            summary: last,
            threads,
            limited,
        })
    }

    fn thread_get(&self, path: &str, c: &CancellationToken) -> Result<Json> {
        self.request(
            READ,
            "read_review_threads",
            "GET",
            &self.versioned(path),
            None,
            false,
            c,
        )
        .map(|v| v.0)
    }
    fn thread_page(
        &self,
        path: &str,
        key: Option<&str>,
        c: &CancellationToken,
    ) -> Result<(Vec<Json>, bool)> {
        let (j, headers) = self.request(
            READ,
            "read_review_threads",
            "GET",
            &self.versioned(path),
            None,
            false,
            c,
        )?;
        let rows = required(
            key.map_or(Some(&j), |k| j.get(k))
                .and_then(Json::array)
                .filter(|r| r.len() <= MAX_ROWS),
        )?;
        let limited = rows.len() >= MAX_THREADS
            || j.str("next").is_some()
            || j.bool("isLastPage") == Some(false)
            || headers.iter().any(|(k, v)| {
                (k.eq_ignore_ascii_case("link") && v.contains("rel=\"next\""))
                    || (k.eq_ignore_ascii_case("x-next-page") && !v.is_empty())
                    || (k.eq_ignore_ascii_case("x-ms-continuationtoken") && !v.is_empty())
            });
        Ok((rows.to_vec(), limited))
    }

    pub(super) fn submit_provider_thread(
        &self,
        preview: &ForgeThreadPreview,
        thread: &ForgeReviewThread,
        c: &CancellationToken,
    ) -> Result<()> {
        let p = self.config.provider();
        let path = self.provider_path(preview.repository(), preview.number())?;
        let root = required(thread.comments.first())?.id;
        let resolve = preview.action == ForgeThreadAction::Resolve;
        let reply = preview.action == ForgeThreadAction::Reply;
        let (method, route, payload) = if reply {
            match p {
                ForgeProvider::GitLab => (
                    "POST",
                    format!("{path}/discussions/{}/notes", encode(&thread.id)),
                    format!("{{\"body\":{}}}", quote(&preview.body)),
                ),
                ForgeProvider::Bitbucket => (
                    "POST",
                    format!("{path}/comments"),
                    format!(
                        "{{\"content\":{{\"raw\":{}}},\"parent\":{{\"id\":{root}}}}}",
                        quote(&preview.body)
                    ),
                ),
                ForgeProvider::BitbucketServer => (
                    "POST",
                    format!("{path}/comments"),
                    format!(
                        "{{\"text\":{},\"parent\":{{\"id\":{root}}}}}",
                        quote(&preview.body)
                    ),
                ),
                ForgeProvider::Azure | ForgeProvider::AzureServer => (
                    "POST",
                    format!("{path}/threads/{}/comments", thread.id),
                    format!(
                        "{{\"content\":{},\"parentCommentId\":{root},\"commentType\":1}}",
                        quote(&preview.body)
                    ),
                ),
                // Gitea 1.26 exposes resolution, but no stable reply endpoint.
                _ => return fail(ForgeFailure::Unsupported),
            }
        } else {
            match p {
                ForgeProvider::GitLab => (
                    "PUT",
                    format!("{path}/discussions/{}", encode(&thread.id)),
                    format!("{{\"resolved\":{resolve}}}"),
                ),
                ForgeProvider::Bitbucket => (
                    if resolve { "POST" } else { "DELETE" },
                    format!("{path}/comments/{root}/resolve"),
                    String::new(),
                ),
                ForgeProvider::BitbucketServer => {
                    let version = required(thread.version)?;
                    (
                        "PUT",
                        format!("{path}/comments/{root}"),
                        format!("{{\"version\":{version},\"threadResolved\":{resolve}}}"),
                    )
                }
                ForgeProvider::Azure | ForgeProvider::AzureServer => (
                    "PATCH",
                    format!("{path}/threads/{}", thread.id),
                    format!(
                        "{{\"status\":{}}}",
                        quote(if resolve { "fixed" } else { "active" })
                    ),
                ),
                ForgeProvider::Gitea => (
                    "POST",
                    format!(
                        "/repos/{}/pulls/comments/{root}/{}",
                        preview.repository().encoded(),
                        if resolve { "resolve" } else { "unresolve" }
                    ),
                    String::new(),
                ),
                _ => return fail(ForgeFailure::Unsupported),
            }
        };
        let step = if reply {
            "reply_to_review_thread"
        } else {
            "change_review_thread_state"
        };
        let response = self
            .request(
                SUBMIT,
                step,
                method,
                &self.versioned(&route),
                (!payload.is_empty()).then_some(payload.as_str()),
                true,
                c,
            )?
            .0;
        // From this point a write may have happened. Failed/missing readback is
        // unknown, never a retryable cancellation or a false success.
        if reply {
            let author = if p == ForgeProvider::Bitbucket {
                response.get("user")
            } else {
                response.get("author")
            };
            let own = author.and_then(|a| identity(a, p)).as_deref() == self.account_identity()
                && self.account_identity().is_some();
            let parent = if p.is_azure() {
                response.u64("parentCommentId")
            } else {
                response.get("parent").and_then(|v| v.u64("id"))
            };
            let text = match p {
                ForgeProvider::Bitbucket => response.get("content").and_then(|v| v.str("raw")),
                ForgeProvider::BitbucketServer => response.str("text"),
                ForgeProvider::Azure | ForgeProvider::AzureServer => response.str("content"),
                _ => response.str("body"),
            };
            // A successful-looking response must identify a newly created reply,
            // not echo an older comment already present in the reviewed thread.
            let id = response.u64("id").filter(|n| {
                *n > 0 && *n != root && !thread.comments.iter().any(|comment| comment.id == *n)
            });
            if !own
                || id.is_none()
                || text != Some(&preview.body)
                || (p != ForgeProvider::GitLab && parent != Some(root))
            {
                return fail(ForgeFailure::PullReviewUnknown);
            }
            // The returned note is scoped by the request, but verify membership
            // so a malformed response cannot acknowledge the wrong discussion.
            let current = self
                .pull_threads(preview.repository(), preview.number(), c)
                .map_err(|_| Error::Forge(ForgeFailure::PullReviewUnknown))?;
            let found = current.threads.iter().find(|t| t.id == thread.id);
            if found.is_none_or(|t| !t.comments.iter().any(|v| Some(v.id) == id)) {
                return fail(ForgeFailure::PullReviewUnknown);
            }
        } else {
            let current = self
                .pull_threads(preview.repository(), preview.number(), c)
                .map_err(|_| Error::Forge(ForgeFailure::PullReviewUnknown))?;
            if current
                .threads
                .iter()
                .find(|t| t.id == thread.id)
                .is_none_or(|t| t.resolved != resolve)
            {
                return fail(ForgeFailure::PullReviewUnknown);
            }
        }
        Ok(())
    }
}
