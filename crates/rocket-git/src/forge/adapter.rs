use super::*;
const PAGE: usize = 50;
fn val<'a>(v: &'a Json, keys: &[&str]) -> Option<&'a Json> {
    let mut r = v;
    for k in keys {
        r = r.get(k)?;
    }
    Some(r)
}
fn text(v: &Json, keys: &[&str]) -> String {
    text_ref(v, keys).to_owned()
}
fn text_ref<'a>(v: &'a Json, keys: &[&str]) -> &'a str {
    match val(v, keys) {
        Some(Json::String(s)) => s,
        _ => "",
    }
}
fn number(v: &Json, key: &str) -> Option<u64> {
    v.u64(key).filter(|n| *n > 0)
}
fn sha(v: &str) -> bool {
    matches!(v.len(), 40 | 64) && v.bytes().all(|b| b.is_ascii_hexdigit())
}
impl ForgeSession {
    pub(super) fn prefix(&self, r: &ForgeRepository) -> String {
        match self.config.provider() {
            ForgeProvider::GitLab => format!("/projects/{}", encode(r.path())),
            ForgeProvider::Bitbucket => format!("/repositories/{}", r.encoded()),
            ForgeProvider::Azure | ForgeProvider::AzureServer => {
                let (p, n) = r.parts();
                format!("/{}/_apis/git/repositories/{}", encode(p), encode(n))
            }
            _ => format!("/repos/{}", r.encoded()),
        }
    }
    pub(super) fn branch_id(
        &self,
        r: &ForgeRepository,
        branch: &str,
        cancel: &CancellationToken,
    ) -> Result<String> {
        self.check_repository(r)?;
        validate_branch(branch)?;
        if self.config.provider() == ForgeProvider::BitbucketServer {
            return self.server_branch_id(r, branch, cancel);
        }
        let pre = self.prefix(r);
        let path = match self.config.provider() {
            ForgeProvider::GitHub | ForgeProvider::GitHubEnterprise => format!("{pre}/branches/{}", encode(branch)),
            ForgeProvider::GitLab => format!("{pre}/repository/branches/{}", encode(branch)),
            ForgeProvider::Bitbucket => format!("{pre}/refs/branches/{}", encode(branch)),
            ForgeProvider::Azure | ForgeProvider::AzureServer => format!(
                "{pre}/refs?filter={}&$top=100&api-version={}",
                encode(&format!("heads/{branch}")), self.config.azure_api_version()
            ),
            ForgeProvider::Gitea => format!("{pre}/branches/{}", encode(branch)),
            ForgeProvider::BitbucketServer => unreachable!("separate server adapter"),
        };
        let v = self
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
        let id = match self.config.provider() {
            ForgeProvider::GitHub | ForgeProvider::GitHubEnterprise => text(&v, &["commit", "sha"]),
            ForgeProvider::GitLab | ForgeProvider::Gitea => text(&v, &["commit", "id"]),
            ForgeProvider::Bitbucket => text(&v, &["target", "hash"]),
            ForgeProvider::BitbucketServer => unreachable!("separate server adapter"),
            ForgeProvider::Azure | ForgeProvider::AzureServer => {
                let name = format!("refs/heads/{branch}");
                let items = v
                    .get("value")
                    .and_then(Json::array)
                    .ok_or(Error::Forge(ForgeFailure::InvalidResponse))?;
                items
                    .iter()
                    .find(|v| v.str("name") == Some(name.as_str()))
                    .and_then(|v| v.str("objectId"))
                    .unwrap_or("")
                    .into()
            }
        };
        if !sha(&id) {
            return fail(ForgeFailure::NotFound);
        }
        Ok(id)
    }
    fn azure_pull_path(&self, repo: &ForgeRepository, filter: &ForgeFilter, page: u32) -> String {
        format!("{}/pullrequests?searchCriteria.status={}&$top={PAGE}&$skip={}&api-version={}",
            self.prefix(repo), if filter.state == ForgeState::Open { "active" } else { "all" },
            (page - 1) * PAGE as u32, self.config.azure_api_version())
    }
    pub fn list(
        &self,
        repo: &ForgeRepository,
        filter: &ForgeFilter,
        cursor: ForgeCursor,
        cancel: &CancellationToken,
    ) -> Result<ForgePage> {
        self.check_repository(repo)?;
        cancel.check()?;
        let gitlab_search = self.config.provider() == ForgeProvider::GitLab && !filter.search.is_empty();
        if gitlab_search {
            filter.search.validate_for(ForgeProvider::GitLab)?;
        } else if !filter.search.is_empty() {
            return self.search(repo, filter, cursor, cancel);
        }
        let page = cursor.page.max(1);
        if page > 100 {
            return Err(invalid(
                "The hosting view is limited to 100 pages. Narrow the filter.",
            ));
        }
        if self.config.provider() == ForgeProvider::BitbucketServer {
            return self.server_list(repo, filter, cursor, cancel);
        }
        if self.config.provider() == ForgeProvider::Bitbucket && filter.kind == ForgeKind::Issues {
            return fail(ForgeFailure::Unsupported);
        }
        if self.config.provider().is_azure() && filter.kind == ForgeKind::Issues {
            return self.work_items(repo, filter, cursor, cancel);
        }
        let pulls = filter.kind == ForgeKind::Pulls;
        let pre = self.prefix(repo);
        let state = match filter.state {
            ForgeState::Open => "open",
            ForgeState::Closed => "closed",
            ForgeState::All => "all",
        };
        let continuation = self.continuation_path(repo, filter, &cursor)?;
        let mut path = if let Some(next) = continuation.filter(|_| !gitlab_search) {
            next.to_owned()
        } else {
            match self.config.provider() {
                ForgeProvider::GitHub | ForgeProvider::GitHubEnterprise => format!(
                    "{pre}/{}?state={state}&sort=updated&direction=desc&per_page={PAGE}&page={page}",
                    if pulls { "pulls" } else { "issues" }
                ),
                ForgeProvider::GitLab => format!(
                    "{pre}/{}?scope=all&state={}&order_by=updated_at&sort=desc&per_page={PAGE}&page={page}",
                    if pulls { "merge_requests" } else { "issues" },
                    match filter.state {
                        ForgeState::Open => "opened",
                        ForgeState::Closed if !pulls => "closed",
                        _ => "all",
                    }
                ),
                ForgeProvider::Gitea => format!(
                    "{pre}/{}?state={state}&sort=recentupdate&limit={PAGE}&page={page}{}",
                    if pulls { "pulls" } else { "issues" },
                    if pulls { "" } else { "&type=issues" }
                ),
                ForgeProvider::Bitbucket => {
                    let states = match filter.state {
                        ForgeState::Open => "state=OPEN",
                        ForgeState::Closed => "state=MERGED&state=DECLINED",
                        ForgeState::All => "state=OPEN&state=MERGED&state=DECLINED",
                    };
                    // Lists leave reviewers and participants out unless asked.
                    let fields = if filter.reviewers {
                        "&fields=%2Bvalues.reviewers,%2Bvalues.participants"
                    } else {
                        ""
                    };
                    format!(
                        "{pre}/pullrequests?{states}&sort=-updated_on&pagelen={PAGE}&page={page}{fields}"
                    )
                }
                ForgeProvider::Azure | ForgeProvider::AzureServer => self.azure_pull_path(repo, filter, page),
                ForgeProvider::BitbucketServer => unreachable!("separate server adapter"),
            }
        };
        if gitlab_search {
            // Rebuild all filters for every page rather than trusting a server's
            // continuation query to preserve the user's search or repository scope.
            for (key, value) in [("search", &filter.search.text), ("author_username", &filter.search.author), ("assignee_username[]", &filter.search.assignee)] {
                if !value.is_empty() { path.push_str(&format!("&{key}={}", urls::encode(value))); }
            }
            if !filter.search.text.is_empty() { path.push_str("&in=title,description"); }
        }
        let (v, headers) = self.request(
            if gitlab_search { "hosting-search" } else { "hosting-list" },
            if gitlab_search { "search_repository" } else { "read_hosting_page" },
            "GET",
            &path,
            None,
            false,
            cancel,
        )?;
        let array = match self.config.provider() {
            ForgeProvider::Bitbucket => v.get("values"),
            ForgeProvider::Azure | ForgeProvider::AzureServer => v.get("value"),
            _ => Some(&v),
        }
        .and_then(Json::array)
        .ok_or(Error::Forge(ForgeFailure::InvalidResponse))?;
        if array.len() > PAGE {
            return fail(ForgeFailure::InvalidResponse);
        }
        let numbered_next = || ForgeCursor {
            page: page + 1,
            ..Default::default()
        };
        let link = if self.config.provider() == ForgeProvider::Bitbucket {
            Some(match v.get("next") {
                None | Some(Json::Null) => None,
                Some(Json::String(s)) if s.is_empty() => None,
                Some(Json::String(s)) => Some(s.as_str()),
                _ => return fail(ForgeFailure::InvalidPagination),
            })
        } else {
            super::pagination::next_link(&headers)?
        };
        let next = if self.config.provider() == ForgeProvider::AzureServer {
            let next_path = self.azure_pull_path(repo, filter, page + 1);
            let expected = format!("{}{next_path}", self.config.api());
            match link {
                Some(Some(url)) if url != expected => return fail(ForgeFailure::InvalidPagination),
                Some(None) => None,
                Some(Some(_)) => Some(self.linked_cursor(repo, filter, page, &path, &expected)?),
                None if array.len() == PAGE => Some(self.linked_cursor(repo, filter, page, &path, &expected)?),
                None => None,
            }
        } else { match link {
            Some(Some(url)) => Some(self.linked_cursor(repo, filter, page, &path, url)?),
            Some(None) => None,
            None if self.config.provider() == ForgeProvider::GitLab => {
                match crate::hosting::header(&headers, "x-next-page") {
                    Some("") => None,
                    Some(n) if n.parse::<u32>().ok() == Some(page + 1) => Some(numbered_next()),
                    Some(_) => return fail(ForgeFailure::InvalidPagination),
                    None => (array.len() == PAGE).then(numbered_next),
                }
            }
            None => (array.len() == PAGE).then(numbered_next),
        }};
        let next = if gitlab_search {
            next.map(|_| self.query_cursor(repo, filter, page + 1, 0))
        } else { next };
        let mut items = Vec::new();
        for value in array {
            if !pulls
                && self.config.provider().is_github()
                && value.get("pull_request").is_some()
            {
                continue;
            }
            let item = self
                .item(repo, filter.kind, value)
                .ok_or(Error::Forge(ForgeFailure::InvalidResponse))?;
            if filter.state == ForgeState::Closed
                && ((self.config.provider().is_azure() && item.state == "active")
                    || (self.config.provider() == ForgeProvider::GitLab
                        && matches!(item.state.as_str(), "opened" | "locked")))
            {
                continue;
            }
            items.push(item);
        }
        Ok(ForgePage { items, next, total: None, incomplete: false })
    }
    fn search(&self, repo: &ForgeRepository, filter: &ForgeFilter, cursor: ForgeCursor, cancel: &CancellationToken) -> Result<ForgePage> {
        if !self.config.provider().is_github() {
            return fail(ForgeFailure::Unsupported);
        }
        filter.search.validate()?;
        if repo.path().chars().any(char::is_whitespace) {
            return Err(invalid("GitHub search requires an owner/repository without whitespace."));
        }
        // Validate the previous page's context, then construct our own next URL.
        // Never send a server-supplied query that could broaden the repository scope.
        self.continuation_path(repo, filter, &cursor)?;
        let page = cursor.page.max(1);
        if page > 20 { return Err(invalid("GitHub search is limited to 1,000 results. Narrow the search.")); }
        let mut query = format!("repo:{} is:{}", repo.path(), if filter.kind == ForgeKind::Pulls { "pr" } else { "issue" });
        match filter.state {
            ForgeState::Open => query.push_str(" is:open"),
            ForgeState::Closed => query.push_str(" is:closed"),
            ForgeState::All => {},
        }
        let search = &filter.search;
        if !search.text.trim().is_empty() { query.push_str(&format!(" in:title,body \"{}\"", search.text.trim())); }
        if !search.author.is_empty() { query.push_str(&format!(" author:{}", search.author)); }
        if !search.assignee.is_empty() { query.push_str(&format!(" assignee:{}", search.assignee)); }
        let path = |page| format!("/search/issues?q={}&sort=updated&order=desc&per_page={PAGE}&page={page}", encode(&query));
        let current = path(page);
        let (v, _) = self.request("hosting-list", "search_hosting_page", "GET", &current, None, false, cancel)?;
        let total = v.u64("total_count").ok_or(Error::Forge(ForgeFailure::InvalidResponse))?;
        let incomplete = match v.get("incomplete_results") {
            Some(Json::Bool(value)) => *value,
            _ => return fail(ForgeFailure::InvalidResponse),
        };
        let array = v.get("items").and_then(Json::array).ok_or(Error::Forge(ForgeFailure::InvalidResponse))?;
        if array.len() > PAGE { return fail(ForgeFailure::InvalidResponse); }
        let mut items = Vec::new();
        for value in array {
            if value.get("pull_request").is_some() != (filter.kind == ForgeKind::Pulls) {
                return fail(ForgeFailure::InvalidResponse);
            }
            // item() pins every result link to this exact repository and item type.
            items.push(self.item(repo, filter.kind, value).ok_or(Error::Forge(ForgeFailure::InvalidResponse))?);
        }
        let next = if !array.is_empty() && u64::from(page) * (PAGE as u64) < total.min(1000) {
            Some(self.linked_cursor(repo, filter, page, &current, &format!("{}{}", self.config.api(), path(page + 1)))?)
        } else { None };
        Ok(ForgePage { items, next, total: Some(total), incomplete: incomplete || total > 1000 })
    }
    fn work_items(
        &self,
        repo: &ForgeRepository,
        filter: &ForgeFilter,
        cursor: ForgeCursor,
        cancel: &CancellationToken,
    ) -> Result<ForgePage> {
        let server = self.config.provider() == ForgeProvider::AzureServer;
        if server { self.continuation_path(repo, filter, &cursor)?; }
        let state = &filter.work_item_state;
        if state.len() > 128 || state.chars().any(char::is_control) {
            return Err(invalid(
                "Enter an exact Azure work-item state up to 128 bytes.",
            ));
        }
        let project = repo.parts().0;
        let literal = |s: &str| s.replace('\'', "''");
        let query = format!(
            "SELECT [System.Id] FROM WorkItems WHERE [System.TeamProject] = '{}' AND [System.Id] > {}{} ORDER BY [System.Id] ASC",
            literal(project),
            cursor.after,
            if state.is_empty() {
                String::new()
            } else {
                format!(" AND [System.State] = '{}'", literal(state))
            }
        );
        let body = format!("{{\"query\":{}}}", quote(&query));
        // WIQL POST is a read query, not a mutation.
        let v = self
            .request(
                "hosting-list",
                "query_work_items",
                "POST",
                &format!(
                    "/{}/_apis/wit/wiql?$top={PAGE}&api-version={}",
                    encode(project), self.config.azure_api_version()
                ),
                Some(&body),
                false,
                cancel,
            )?
            .0;
        let ids = v
            .get("workItems")
            .and_then(Json::array)
            .ok_or(Error::Forge(ForgeFailure::InvalidResponse))?;
        if ids.len() > PAGE {
            return fail(ForgeFailure::InvalidResponse);
        }
        let ids: Vec<u64> = ids
            .iter()
            .map(|v| {
                number(v, "id")
                    .filter(|n| *n > cursor.after)
                    .ok_or(Error::Forge(ForgeFailure::InvalidResponse))
            })
            .collect::<Result<_>>()?;
        if ids.is_empty() {
            return Ok(ForgePage {
                items: Vec::new(),
                next: None,
                total: None,
                incomplete: false,
            });
        }
        if ids.windows(2).any(|p| p[0] >= p[1]) {
            return fail(ForgeFailure::InvalidResponse);
        }
        let path = format!(
            "/{}/_apis/wit/workitems?ids={}&fields=System.Title,System.State,System.CreatedBy{}&api-version={}",
            encode(project),
            ids.iter().map(u64::to_string).collect::<Vec<_>>().join(","),
            if server { ",System.TeamProject" } else { "" }, self.config.azure_api_version()
        );
        let v = self
            .request(
                "hosting-list",
                "read_work_items",
                "GET",
                &path,
                None,
                false,
                cancel,
            )?
            .0;
        let values = v
            .get("value")
            .and_then(Json::array)
            .ok_or(Error::Forge(ForgeFailure::InvalidResponse))?;
        // A collection response must not relabel another project's work items.
        if server && (values.len() != ids.len() || values.iter().any(|v|
            !text_ref(v, &["fields", "System.TeamProject"]).eq_ignore_ascii_case(project))) {
            return fail(ForgeFailure::InvalidResponse);
        }
        let items = values
            .iter()
            .map(|v| {
                self.item(repo, ForgeKind::Issues, v)
                    .filter(|v| ids.contains(&v.number))
                    .ok_or(Error::Forge(ForgeFailure::InvalidResponse))
            })
            .collect::<Result<Vec<_>>>()?;
        if server && items.iter().enumerate().any(|(i,item)| items[..i].iter().any(|previous| previous.number == item.number)) {
            return fail(ForgeFailure::InvalidResponse);
        }
        let next = (ids.len() == PAGE).then(|| {
            let page = cursor.page.max(1) + 1;
            let after = *ids.last().expect("nonempty IDs");
            if server { self.query_cursor(repo, filter, page, after) }
            else { ForgeCursor { page, after, ..Default::default() } }
        });
        Ok(ForgePage { items, total: None, incomplete: false, next })
    }
    pub(super) fn item(&self, r: &ForgeRepository, kind: ForgeKind, v: &Json) -> Option<ForgeItem> {
        if self.config.provider() == ForgeProvider::BitbucketServer {
            return bitbucket_server::server_item(r, v);
        }
        let azure_issue =
            self.config.provider().is_azure() && kind == ForgeKind::Issues;
        let number = number(
            v,
            match self.config.provider() {
                ForgeProvider::GitLab => "iid",
                ForgeProvider::Bitbucket => "id",
                ForgeProvider::Azure | ForgeProvider::AzureServer => {
                    if azure_issue {
                        "id"
                    } else {
                        "pullRequestId"
                    }
                }
                _ => "number",
            },
        )?;
        let title = if azure_issue {
            text_ref(v, &["fields", "System.Title"])
        } else {
            text_ref(v, &["title"])
        };
        if title.is_empty() || title.len() > 16 * 1024 {
            return None;
        }
        let author = match self.config.provider() {
            ForgeProvider::GitHub | ForgeProvider::GitHubEnterprise | ForgeProvider::Gitea => text_ref(v, &["user", "login"]),
            ForgeProvider::GitLab => text_ref(v, &["author", "username"]),
            ForgeProvider::Bitbucket => text_ref(v, &["author", "display_name"]),
            ForgeProvider::Azure | ForgeProvider::AzureServer if azure_issue => {
                text_ref(v, &["fields", "System.CreatedBy", "displayName"])
            }
            ForgeProvider::Azure | ForgeProvider::AzureServer => text_ref(v, &["createdBy", "displayName"]),
            ForgeProvider::BitbucketServer => unreachable!("separate server adapter"),
        };
        let state = if azure_issue {
            text_ref(v, &["fields", "System.State"])
        } else if self.config.provider().is_azure() {
            text_ref(v, &["status"])
        } else {
            text_ref(v, &["state"])
        };
        let (source, target) = match self.config.provider() {
            ForgeProvider::GitHub | ForgeProvider::GitHubEnterprise | ForgeProvider::Gitea => {
                (text_ref(v, &["head", "ref"]), text_ref(v, &["base", "ref"]))
            }
            ForgeProvider::GitLab => (text_ref(v, &["source_branch"]), text_ref(v, &["target_branch"])),
            ForgeProvider::Bitbucket => (
                text_ref(v, &["source", "branch", "name"]),
                text_ref(v, &["destination", "branch", "name"]),
            ),
            ForgeProvider::Azure | ForgeProvider::AzureServer => (
                text_ref(v, &["sourceRefName"]).trim_start_matches("refs/heads/"),
                text_ref(v, &["targetRefName"]).trim_start_matches("refs/heads/"),
            ),
            ForgeProvider::BitbucketServer => unreachable!("separate server adapter"),
        };
        Some(ForgeItem {
            timing: (kind == ForgeKind::Pulls).then(|| super::insights::timing(self.config.provider(), v)),
            reviewers: (kind == ForgeKind::Pulls)
                .then(|| super::insights::review_requests(self.config.provider(), r.parts().0, v))
                .flatten(),
            number,
            title: crate::display_text::bounded(title, 1024).text,
            state: crate::display_text::bounded(state, 64).text,
            author: crate::display_text::bounded(author, 256).text,
            source: crate::display_text::bounded(source, 1024).text,
            target: crate::display_text::bounded(target, 1024).text,
            url: r.item_url(kind, number),
        })
    }
    pub(super) fn create_request(&self, d: &ForgeDraft) -> (String, String) {
        if self.config.provider() == ForgeProvider::BitbucketServer {
            return bitbucket_server::server_create(d);
        }
        let pre = self.prefix(&d.repository);
        let title = quote(&d.title);
        let body = quote(&d.body);
        let source = quote(&d.source);
        let target = quote(&d.target);
        match self.config.provider() {
            ForgeProvider::GitHub | ForgeProvider::GitHubEnterprise | ForgeProvider::Gitea => (
                format!("{pre}/pulls"),
                format!(
                    "{{\"title\":{title},\"body\":{body},\"head\":{source},\"base\":{target}}}"
                ),
            ),
            ForgeProvider::GitLab => (
                format!("{pre}/merge_requests"),
                format!(
                    "{{\"title\":{title},\"description\":{body},\"source_branch\":{source},\"target_branch\":{target}}}"
                ),
            ),
            ForgeProvider::Bitbucket => (
                format!("{pre}/pullrequests"),
                format!(
                    "{{\"title\":{title},\"description\":{body},\"source\":{{\"branch\":{{\"name\":{source}}}}},\"destination\":{{\"branch\":{{\"name\":{target}}}}},\"close_source_branch\":false}}"
                ),
            ),
            ForgeProvider::Azure | ForgeProvider::AzureServer => (
                format!("{pre}/pullrequests?api-version={}", self.config.azure_api_version()),
                format!(
                    "{{\"title\":{title},\"description\":{body},\"sourceRefName\":{},\"targetRefName\":{}}}",
                    quote(&format!("refs/heads/{}", d.source)),
                    quote(&format!("refs/heads/{}", d.target))
                ),
            ),
            ForgeProvider::BitbucketServer => unreachable!("separate server adapter"),
        }
    }
}
