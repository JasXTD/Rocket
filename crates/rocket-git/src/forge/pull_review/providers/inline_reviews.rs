//! GitLab discussions and Gitea review comments use different position contracts.
use super::*;

/// One row of Gitea's review list for the pending-review check: who wrote it and
/// whether it is a pending draft. None for a row that can't be read safely.
fn gitea_review_row(j: &Json) -> Option<(String, bool)> {
    let state = j.str("state")?;
    if !matches!(
        state,
        "PENDING" | "APPROVED" | "COMMENT" | "COMMENTED" | "REQUEST_CHANGES" | "CHANGES_REQUESTED" | "REQUEST_REVIEW"
    ) {
        return None;
    }
    // A review requested from a team has no user ("user": null, with "team" set).
    // Only a pending review is a draft, and a draft always belongs to a user, so a
    // team row can never be this account's.
    let actor = match j.get("user") {
        None | Some(Json::Null) if state != "PENDING" && j.get("team").is_some_and(|t| !matches!(t, Json::Null)) => {
            String::new()
        }
        user => identity(user?, ForgeProvider::Gitea)?,
    };
    Some((actor, state == "PENDING"))
}

pub(super) fn attach_gitea_diff(
    files: &mut [ForgePullFile],
    diff: &str,
    c: &CancellationToken,
) -> Result<()> {
    if diff.len() > 256 * 1024 || diff.contains('\0') {
        return fail(ForgeFailure::InvalidResponse);
    }
    // Build all mappings first; a malformed/ambiguous response cannot leave a
    // partially enabled set of comment targets. Metadata URLs are never opened.
    let mut patches = Vec::new();
    let mut seen = HashSet::new();
    for section in diff
        .split_inclusive('\n')
        .fold(Vec::<String>::new(), |mut sections, line| {
            if line.starts_with("diff --git ") {
                sections.push(String::new());
            }
            if let Some(s) = sections.last_mut() {
                s.push_str(line);
            }
            sections
        })
    {
        c.check()?;
        let lines: Vec<_> = section.split_inclusive('\n').collect();
        let Some(hunk) = lines.iter().position(|s| s.starts_with("@@ -")) else {
            continue;
        };
        let headers = &lines[1..hunk];
        let path = |prefix: &str, side: &[u8]| -> Result<Option<String>> {
            let mut matches = headers
                .iter()
                .filter_map(|s| s.trim_end_matches('\n').strip_prefix(prefix));
            let value = matches
                .next()
                .ok_or(Error::Forge(ForgeFailure::InvalidResponse))?;
            if matches.next().is_some() {
                return fail(ForgeFailure::InvalidResponse);
            }
            let parsed = crate::patches::parse::header_path(value.as_bytes(), side)
                .map_err(|_| Error::Forge(ForgeFailure::InvalidResponse))?;
            parsed
                .map(|p| {
                    String::from_utf8(p.as_bytes().to_vec())
                        .map_err(|_| Error::Forge(ForgeFailure::InvalidResponse))
                })
                .transpose()
        };
        let old = path("--- ", b"a/")?;
        let new = path("+++ ", b"b/")?;
        let name = new
            .as_ref()
            .or(old.as_ref())
            .ok_or(Error::Forge(ForgeFailure::InvalidResponse))?;
        if !seen.insert(name.clone()) {
            return fail(ForgeFailure::InvalidResponse);
        }
        let Some((i, f)) = files
            .iter()
            .enumerate()
            .find(|(_, f)| f.raw_path.as_deref() == Some(name))
        else {
            continue;
        };
        let expected_old = if f.status == "added" {
            None
        } else {
            Some(f.raw_previous.as_deref().unwrap_or(name))
        };
        let expected_new = if matches!(f.status.as_str(), "removed" | "deleted") {
            None
        } else {
            Some(name.as_str())
        };
        if old.as_deref() != expected_old
            || new.as_deref() != expected_new
            || (f.previous.is_some() && f.raw_previous.is_none())
        {
            return fail(ForgeFailure::InvalidResponse);
        }
        let patch = lines[hunk..].concat();
        if patch.len() > 16384 {
            continue;
        }
        // Binary, oversized and incomplete hunks stay readable as metadata only.
        if let Some(rows) = inline::parse_lines(&patch) {
            patches.push((i, patch, rows));
        }
    }
    for (i, patch, rows) in patches {
        files[i].patch = Some(multiline(&patch, 16384));
        files[i].lines = rows;
    }
    Ok(())
}

impl ForgeSession {
    pub(in crate::forge::pull_review) fn ensure_no_pending_gitea_review(
        &self,
        repo: &ForgeRepository,
        number: u64,
        c: &CancellationToken,
    ) -> Result<()> {
        if self.config.provider() != ForgeProvider::Gitea {
            return Ok(());
        }
        let path = self.provider_path(repo, number)?;
        // Gitea's create-review endpoint consumes the caller's pending review.
        // Never knowingly submit someone else's saved draft from this account.
        // Numeric pages are constructed locally; returned URLs are not followed.
        for page in 1..=4 {
            let reviews = self.first_page(
                &format!("{path}/reviews?limit=50&page={page}"),
                None,
                "read_pending_reviews",
                c,
                gitea_review_row,
            )?;
            if reviews.error.is_some() {
                return fail(ForgeFailure::ReviewStateUnavailable);
            }
            if reviews
                .items
                .iter()
                .any(|(actor, pending)| *pending && actor.eq_ignore_ascii_case(&self.login))
            {
                return fail(ForgeFailure::PendingReview);
            }
            if !reviews.limited {
                return Ok(());
            }
        }
        fail(ForgeFailure::ReviewStateUnavailable)
    }
    pub(super) fn provider_inline_comments(
        &self,
        path: &str,
        reviews: &ForgePullCollection<ForgePullReview>,
        c: &CancellationToken,
    ) -> Result<ForgePullCollection<ForgePullComment>> {
        match self.config.provider() {
            ForgeProvider::GitLab => {
                let discussions = self.first_page(
                    &format!("{path}/discussions?per_page=50&page=1"),
                    None,
                    "read_pull_inline",
                    c,
                    |j| Some(j.clone()),
                )?;
                let mut out = ForgePullCollection {
                    items: vec![],
                    limited: discussions.limited,
                    error: discussions.error,
                };
                for d in discussions.items {
                    let Some(notes) = d
                        .get("notes")
                        .and_then(Json::array)
                        .filter(|a| a.len() <= 200)
                    else {
                        out.error = Some(ForgeFailure::InvalidResponse);
                        break;
                    };
                    for j in notes {
                        c.check()?;
                        let Some(position) = j.get("position") else {
                            continue;
                        };
                        if out.items.len() >= 200 {
                            out.limited = true;
                            break;
                        }
                        let Some(mut n) = note(j, ForgeProvider::GitLab) else {
                            out.error = Some(ForgeFailure::InvalidResponse);
                            break;
                        };
                        n.path = position
                            .str("new_path")
                            .or_else(|| position.str("old_path"))
                            .map(|p| bounded(p, 8192));
                        n.line = position
                            .u64("new_line")
                            .or_else(|| position.u64("old_line"));
                        out.items.push(n);
                    }
                }
                Ok(out)
            }
            ForgeProvider::Gitea => {
                let mut out = ForgePullCollection {
                    items: vec![],
                    limited: reviews.limited,
                    error: reviews.error,
                };
                let relevant: Vec<_> = reviews.items.iter().filter(|r| r.id > 0).collect();
                out.limited |= relevant.len() > 8;
                // Bound fan-out as well as response size. This is an explicit
                // preview of at most eight reviews, never an all-comments claim.
                for r in relevant.iter().take(8) {
                    let page = self.first_page(
                        &format!("{path}/reviews/{}/comments?limit=50&page=1", r.id),
                        None,
                        "read_pull_inline",
                        c,
                        |j| {
                            let mut n = note(j, ForgeProvider::Gitea)?;
                            n.path = j.str("path").map(|p| bounded(p, 8192));
                            n.line = j
                                .u64("position")
                                .filter(|n| *n > 0)
                                .or_else(|| j.u64("original_position").filter(|n| *n > 0));
                            Some(n)
                        },
                    )?;
                    out.limited |= page.limited;
                    if page.error.is_some() {
                        out.error = page.error;
                        break;
                    }
                    for n in page.items {
                        if out.items.len() >= 200 {
                            out.limited = true;
                            break;
                        }
                        out.items.push(n);
                    }
                    if out.items.len() >= 200 {
                        break;
                    }
                }
                Ok(out)
            }
            _ => Ok(unavailable()),
        }
    }
    pub(super) fn submit_gitlab_inline(
        &self,
        path: &str,
        pv: &ForgePullReviewPreview,
        c: &CancellationToken,
    ) -> Result<ForgePullReview> {
        let comment = &pv.comments[0]; // validate_review enforces exactly one.
        let body = inline::gitlab_payload(comment)?;
        let (j, _) = self
            .request(
                SUBMIT,
                "publish_pull_inline",
                "POST",
                &format!("{path}/discussions"),
                Some(&body),
                true,
                c,
            )
            .map_err(review_error)?;
        let result = j
            .get("notes")
            .and_then(Json::array)
            .filter(|a| a.len() == 1)
            .and_then(|a| a.first())
            .and_then(|j| {
                if !inline::gitlab_matches(j, comment)
                    || j.get("author")
                        .and_then(|a| identity(a, ForgeProvider::GitLab))
                        .as_deref()
                        != self.account_identity()
                {
                    return None;
                }
                let n = note(j, ForgeProvider::GitLab)?;
                (n.id > 0).then(|| ForgePullReview {
                    id: n.id,
                    author: n.author,
                    body: n.body,
                    state: "COMMENTED".into(),
                    commit: Some(pv.summary.head.clone()),
                    date: n.date,
                })
            });
        result.ok_or(Error::Forge(ForgeFailure::PullReviewUnknown))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn file() -> ForgePullFile {
        ForgePullFile {
            path: "資料/新.rs".into(),
            previous: Some("資料/旧.rs".into()),
            status: "renamed".into(),
            additions: 1,
            deletions: 1,
            patch: None,
            lines: vec![],
            raw_path: Some("資料/新.rs".into()),
            raw_previous: Some("資料/旧.rs".into()),
            anchor: None,
        }
    }
    fn diff(old: &str, new: &str, patch: &str) -> String {
        format!("diff --git a/ignored b/ignored\n--- {old}\n+++ {new}\n{patch}")
    }
    #[test]
    fn gitea_team_review_requests_do_not_block_the_pending_review_check() {
        let row = |s: &str| gitea_review_row(&crate::hosting::json::parse(s.as_bytes()).unwrap());
        // Gitea stores a team request with no reviewer, so the list has "user": null.
        assert_eq!(row(r#"{"id":3,"state":"REQUEST_REVIEW","user":null,"team":{"id":5,"name":"owners"}}"#), Some((String::new(), false)));
        assert_eq!(row(r#"{"id":4,"state":"PENDING","user":{"login":"Me"}}"#), Some(("Me".into(), true)));
        assert_eq!(row(r#"{"id":5,"state":"APPROVED","user":{"login":"other"}}"#), Some(("other".into(), false)));
        // A draft is always somebody's, and a row with neither user nor team is unknown.
        for unreadable in [
            r#"{"id":6,"state":"PENDING","user":null,"team":{"id":5}}"#,
            r#"{"id":7,"state":"REQUEST_REVIEW","user":null,"team":null}"#,
            r#"{"id":8,"state":"REQUEST_REVIEW"}"#,
            r#"{"id":9,"state":"SOMETHING_NEW","user":{"login":"other"}}"#,
        ] {
            assert_eq!(row(unreadable), None, "{unreadable}");
        }
    }
    #[test]
    fn gitea_diff_verifies_unicode_rename_headers_before_enabling_lines() {
        let mut files = vec![file()];
        let patch = "@@ -10,2 +20,2 @@\n context\n-old\n+new\n";
        attach_gitea_diff(
            &mut files,
            &diff("a/資料/旧.rs", "b/資料/新.rs", patch),
            &CancellationToken::new(),
        )
        .unwrap();
        assert_eq!(files[0].lines[1].old, Some(10));
        assert_eq!(files[0].lines[1].new, Some(20));
        let old = crate::patches::parse::quote_path(
            b"a/",
            &RepoPath::new("資料/旧.rs".as_bytes().to_vec()).unwrap(),
        );
        let new = crate::patches::parse::quote_path(
            b"b/",
            &RepoPath::new("資料/新.rs".as_bytes().to_vec()).unwrap(),
        );
        let mut quoted = vec![file()];
        attach_gitea_diff(
            &mut quoted,
            &diff(&old, &new, patch),
            &CancellationToken::new(),
        )
        .unwrap();
        assert_eq!(quoted[0].lines.len(), 4);
    }
    #[test]
    fn gitea_diff_refuses_mismatch_duplicate_cancel_and_truncated_hunks() {
        let good = diff("a/資料/旧.rs", "b/資料/新.rs", "@@ -1 +1 @@\n-old\n+new\n");
        for text in [
            good.clone() + &good,
            diff("a/other.rs", "b/資料/新.rs", "@@ -1 +1 @@\n-old\n+new\n"),
            diff("a/../x", "b/資料/新.rs", "@@ -1 +1 @@\n-old\n+new\n"),
        ] {
            let mut files = vec![file()];
            assert!(attach_gitea_diff(&mut files, &text, &CancellationToken::new()).is_err());
            assert!(files[0].lines.is_empty());
        }
        let mut files = vec![file()];
        attach_gitea_diff(
            &mut files,
            &diff(
                "a/資料/旧.rs",
                "b/資料/新.rs",
                "@@ -1,2 +1 @@\n-old\n+new\n",
            ),
            &CancellationToken::new(),
        )
        .unwrap();
        assert!(files[0].lines.is_empty());
        let c = CancellationToken::new();
        c.cancel();
        assert!(matches!(
            attach_gitea_diff(&mut files, &good, &c),
            Err(Error::Cancelled)
        ));
    }
}
