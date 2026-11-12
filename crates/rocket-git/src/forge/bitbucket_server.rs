//! Bitbucket Server/Data Center REST 1.0. Cloud's 2.0 schema is separate.
use super::*;

const LIMIT: usize = 50;

fn prefix(repo: &ForgeRepository) -> String {
    let (project, name) = repo.parts();
    // Personal projects also accept the project key (~username) in REST URLs.
    format!("/projects/{}/repos/{}", encode(project), encode(name))
}

fn values(value: &Json) -> Result<&[Json]> {
    value
        .get("values")
        .and_then(Json::array)
        .filter(|v| v.len() <= LIMIT)
        .ok_or(Error::Forge(ForgeFailure::InvalidResponse))
}

/// Follow the server's offset, never start + size: offsets need not be contiguous.
fn next_start(value: &Json, start: u64) -> Result<Option<u64>> {
    if value.u64("start") != Some(start) {
        return fail(ForgeFailure::InvalidPagination);
    }
    match value.bool("isLastPage") {
        Some(true) => Ok(None),
        Some(false) => value
            .u64("nextPageStart")
            .filter(|n| *n > start && *n <= i32::MAX as u64)
            .map(Some)
            .ok_or(Error::Forge(ForgeFailure::InvalidPagination)),
        None => fail(ForgeFailure::InvalidPagination),
    }
}

fn list_path(repo: &ForgeRepository, state: ForgeState, start: u64) -> String {
    let state = match state {
        ForgeState::Open => "OPEN",
        ForgeState::Closed | ForgeState::All => "ALL",
    };
    format!(
        "{}/pull-requests?state={state}&order=NEWEST&limit={LIMIT}&start={start}",
        prefix(repo)
    )
}

impl ForgeSession {
    pub(super) fn server_branch_id(
        &self,
        repo: &ForgeRepository,
        branch: &str,
        cancel: &CancellationToken,
    ) -> Result<String> {
        let expected = format!("refs/heads/{branch}");
        let mut start = 0;
        for _ in 0..100 {
            let path = format!(
                "{}/branches?filterText={}&boostMatches=true&limit={LIMIT}&start={start}",
                prefix(repo),
                encode(branch)
            );
            let value = self
                .request(
                    "hosting-preview",
                    "read_remote_branch",
                    "GET",
                    &path,
                    None,
                    false,
                    cancel,
                )?
                .0;
            // filterText is a fuzzy filter, not an exact branch lookup.
            let mut matched = values(&value)?
                .iter()
                .filter(|v| v.str("id") == Some(&expected));
            if let Some(branch) = matched.next() {
                if matched.next().is_some() {
                    return fail(ForgeFailure::InvalidResponse);
                }
                return branch
                    .str("latestCommit")
                    .filter(|s| {
                        matches!(s.len(), 40 | 64) && s.bytes().all(|b| b.is_ascii_hexdigit())
                    })
                    .map(str::to_owned)
                    .ok_or(Error::Forge(ForgeFailure::InvalidResponse));
            }
            match next_start(&value, start)? {
                Some(n) => start = n,
                None => return fail(ForgeFailure::NotFound),
            }
        }
        Err(invalid(
            "Too many matching Bitbucket branches. Use a more specific branch name.",
        ))
    }

    pub(super) fn server_list(
        &self,
        repo: &ForgeRepository,
        filter: &ForgeFilter,
        cursor: ForgeCursor,
        cancel: &CancellationToken,
    ) -> Result<ForgePage> {
        if filter.kind == ForgeKind::Issues {
            return fail(ForgeFailure::Unsupported);
        }
        let first = list_path(repo, filter.state, 0);
        let path = self
            .continuation_path(repo, filter, &cursor)?
            .unwrap_or(&first);
        // Only our bound cursors may select subsequent server offsets.
        if cursor.continuation.is_none() && (cursor.page > 1 || cursor.after != 0) {
            return fail(ForgeFailure::InvalidPagination);
        }
        let start = path
            .rsplit_once("&start=")
            .and_then(|(_, s)| s.parse::<u64>().ok())
            .ok_or(Error::Forge(ForgeFailure::InvalidPagination))?;
        let value = self
            .request(
                "hosting-list",
                "read_hosting_page",
                "GET",
                path,
                None,
                false,
                cancel,
            )?
            .0;
        let array = values(&value)?;
        let next = match next_start(&value, start)? {
            Some(start) => Some(self.linked_cursor(
                repo,
                filter,
                cursor.page.max(1),
                path,
                &format!(
                    "{}{}",
                    self.config.api(),
                    list_path(repo, filter.state, start)
                ),
            )?),
            None => None,
        };
        let mut items = Vec::new();
        for v in array {
            let item = server_item(repo, v).ok_or(Error::Forge(ForgeFailure::InvalidResponse))?;
            if filter.state == ForgeState::Closed && item.state == "OPEN" {
                continue;
            }
            items.push(item);
        }
        Ok(ForgePage {
            items,
            next,
            total: None,
            incomplete: false,
        })
    }
}

pub(super) fn server_item(repo: &ForgeRepository, v: &Json) -> Option<ForgeItem> {
    fn field<'a>(v: &'a Json, k: &str) -> &'a str {
        v.str(k).unwrap_or("")
    }
    fn clean(s: &str, max: usize) -> String {
        crate::display_text::bounded(s, max).text
    }
    let number = v.u64("id").filter(|n| *n > 0)?;
    let state = v
        .str("state")
        .filter(|s| matches!(*s, "OPEN" | "MERGED" | "DECLINED"))?;
    let author = v.get("author")?.get("user")?;
    let source = v.get("fromRef")?.str("id")?.strip_prefix("refs/heads/")?;
    let target = v.get("toRef")?.str("id")?.strip_prefix("refs/heads/")?;
    Some(ForgeItem {
        timing: Some(super::insights::timing(ForgeProvider::BitbucketServer, v)),
        reviewers: super::insights::review_requests(ForgeProvider::BitbucketServer, repo.parts().0, v),
        number,
        title: clean(v.str("title")?, 1024),
        state: state.into(),
        author: clean(
            author
                .str("displayName")
                .unwrap_or_else(|| field(author, "name")),
            256,
        ),
        source: clean(source, 1024),
        target: clean(target, 1024),
        url: repo.item_url(ForgeKind::Pulls, number),
    })
}

pub(super) fn server_create(draft: &ForgeDraft) -> (String, String) {
    let (project, repo) = draft.repository.parts();
    let repository = format!(
        "{{\"slug\":{},\"project\":{{\"key\":{}}}}}",
        quote(repo),
        quote(project)
    );
    let source = quote(&format!("refs/heads/{}", draft.source));
    let target = quote(&format!("refs/heads/{}", draft.target));
    (
        format!("{}/pull-requests", prefix(&draft.repository)),
        format!(
            "{{\"title\":{},\"description\":{},\"fromRef\":{{\"id\":{source},\"repository\":{repository}}},\"toRef\":{{\"id\":{target},\"repository\":{repository}}}}}",
            quote(&draft.title),
            quote(&draft.body)
        ),
    )
}
