//! Explore's repository search on GitLab, and on Gitea and Forgejo (Codeberg); GitHub's
//! own search lives in hosting.rs. Results keep the host's order. Web and clone
//! addresses are rebuilt from the configured server, never taken from the answer: only
//! a loopback test fixture may name the repository to clone (a folder the test made),
//! as GitHub's loopback search does.
use super::*;
use crate::hosting::header;

/// Results asked for in one page: GitLab allows 100 and Forgejo 50 by default.
pub const REPOSITORY_SEARCH_PAGE: usize = 30;

/// How GitLab and Gitea order a search. Neither ranks by relevance for everyone:
/// GitLab's `similarity` covers only projects the account is a member of.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RepositorySearchSort {
    #[default]
    Stars,
    Updated,
}

/// One repository found on GitLab or Gitea. What the host doesn't list is `None`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositorySearchItem {
    /// The whole path, subgroups included: `acme/tools/fuzzyfind`.
    pub path: String,
    pub description: Option<String>,
    pub stars: u64,
    pub forks: u64,
    pub open_issues: Option<u64>,
    /// Gitea's main language; GitLab's list names none.
    pub language: Option<String>,
    /// Gitea's first license; GitLab's list names none.
    pub license: Option<String>,
    pub topics: Vec<String>,
    pub default_branch: Option<String>,
    /// Kilobytes: Gitea's size, or GitLab's for projects the account is a member of.
    pub size_kb: Option<u64>,
    /// GitLab's last activity or Gitea's last update, in Unix seconds.
    pub updated: Option<u64>,
    pub private: bool,
    /// Visible to everyone signed in to the server.
    pub internal: bool,
    pub fork: bool,
    pub archived: bool,
    pub mirror: bool,
    pub web_url: String,
    pub clone_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositorySearchPage {
    pub items: Vec<RepositorySearchItem>,
    /// `None` when the host leaves the total out, as GitLab.com does past 10,000.
    pub total: Option<u64>,
    pub next_page: Option<u32>,
}

/// What a search returned. A rate limit and a server that wants a sign-in are answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RepositorySearchOutcome {
    Page(RepositorySearchPage),
    /// Wait this many seconds, when the host said.
    RateLimited { retry_after: Option<u64> },
    /// The server lets only signed-in users search.
    SignInRequired,
}

impl ForgeProvider {
    /// Hosts whose repositories Explore searches: GitLab, and Gitea and Forgejo (Forgejo
    /// kept Gitea's search).
    pub fn searches_repositories(self) -> bool {
        self == Self::GitLab || self.is_gitea()
    }
    /// Whether a repository search can filter by language: GitLab can, Gitea can't.
    pub fn filters_language(self) -> bool {
        self == Self::GitLab
    }
}

impl ForgeSession {
    /// Whether requests carry an account's token.
    pub fn signed_in(&self) -> bool {
        !self.authorization.is_empty()
    }
    /// Search repositories by name, path or description; `page` starts at 1. GitLab
    /// also filters by `language`; Gitea can't, so it must be `None` there.
    pub fn search_repositories(
        &self,
        query: &str,
        sort: RepositorySearchSort,
        language: Option<&str>,
        page: u32,
        cancel: &CancellationToken,
    ) -> Result<RepositorySearchOutcome> {
        let provider = self.config.provider();
        if !provider.searches_repositories() {
            return fail(ForgeFailure::Unsupported);
        }
        let path = search_path(provider, query, sort, language, page, self.signed_in())?;
        let (value, headers) = match self.request("explore-search", "search_hosting_repositories", "GET", &path, None, false, cancel) {
            Ok(answer) => answer,
            Err(Error::Forge(ForgeFailure::RateLimited)) => {
                return Ok(RepositorySearchOutcome::RateLimited { retry_after: transport::take_retry_after() });
            }
            // Without a token, a refusal means the server lets only signed-in users in.
            Err(Error::Forge(ForgeFailure::Unauthorized | ForgeFailure::Forbidden)) if !self.signed_in() => {
                return Ok(RepositorySearchOutcome::SignInRequired);
            }
            Err(error) => return Err(error),
        };
        search_page(&self.config, &value, &headers, page.clamp(1, MAX_PAGE)).map(RepositorySearchOutcome::Page)
    }
}

/// GitLab stops offset pages at 50,000 projects; nobody pages that far by hand.
const MAX_PAGE: u32 = 10_000;

/// The request path of one search page, after checking what was typed.
fn search_path(
    provider: ForgeProvider,
    query: &str,
    sort: RepositorySearchSort,
    language: Option<&str>,
    page: u32,
    signed_in: bool,
) -> Result<String> {
    let text = query.trim();
    if text.is_empty() || text.chars().count() > 500 || text.chars().any(char::is_control) {
        return Err(invalid("Enter up to 500 characters to search for."));
    }
    let language = language.map(str::trim).filter(|l| !l.is_empty());
    if let Some(l) = language
        && (!provider.filters_language() || l.chars().count() > 60 || l.chars().any(char::is_control))
    {
        return Err(invalid("This host can't filter repositories by that language."));
    }
    let page = page.clamp(1, MAX_PAGE);
    Ok(match provider {
        ForgeProvider::GitLab => {
            let order = match sort {
                RepositorySearchSort::Stars => "star_count",
                RepositorySearchSort::Updated => "last_activity_at",
            };
            let mut path = format!(
                "/projects?search={}&order_by={order}&sort=desc&per_page={REPOSITORY_SEARCH_PAGE}&page={page}",
                encode(text)
            );
            if let Some(l) = language {
                path.push_str(&format!("&with_programming_language={}", encode(l)));
            }
            // GitLab sizes only the projects the account is a member of.
            if signed_in {
                path.push_str("&statistics=true");
            }
            path
        }
        ForgeProvider::Gitea | ForgeProvider::Forgejo => {
            let order = match sort {
                RepositorySearchSort::Stars => "stars",
                RepositorySearchSort::Updated => "updated",
            };
            // Descriptions too, as GitHub and GitLab match them.
            format!(
                "/repos/search?q={}&includeDesc=true&sort={order}&order=desc&limit={REPOSITORY_SEARCH_PAGE}&page={page}",
                encode(text)
            )
        }
        _ => return fail(ForgeFailure::Unsupported),
    })
}

/// One page from GitLab's project list or Gitea's `{ok, data}` answer and its headers.
fn search_page(config: &ForgeConfig, value: &Json, headers: &[(String, String)], page: u32) -> Result<RepositorySearchPage> {
    let provider = config.provider();
    let list = match provider {
        ForgeProvider::GitLab => value.array(),
        _ if value.bool("ok") == Some(false) => None,
        _ => value.get("data").and_then(Json::array),
    }
    .ok_or(Error::Forge(ForgeFailure::InvalidResponse))?;
    let items = list
        .iter()
        .filter_map(|item| match provider {
            ForgeProvider::GitLab => gitlab_item(config, item),
            _ => gitea_item(config, item),
        })
        .collect();
    let number = |name: &str| header(headers, name).and_then(|v| v.trim().parse::<u64>().ok());
    let (total, next_page) = match provider {
        ForgeProvider::GitLab => {
            let next = number("x-next-page").and_then(|n| u32::try_from(n).ok()).filter(|n| *n > page && *n <= MAX_PAGE);
            (number("x-total"), next)
        }
        _ => {
            let total = number("x-total-count");
            let len = list.len() as u64;
            // Gitea names the next page in Link, counted in the page size it used, which
            // a server may cap below what was asked. Without Link (one page of results),
            // this page's size stands for the pages before it.
            let next = match (linked_next(headers), total) {
                (Some(next), _) => next,
                (None, Some(total)) => (len > 0 && u64::from(page) * len < total).then_some(page + 1),
                (None, None) => (list.len() >= REPOSITORY_SEARCH_PAGE).then_some(page + 1),
            };
            (total, next.filter(|n| *n > page && *n <= MAX_PAGE))
        }
    };
    Ok(RepositorySearchPage { items, total, next_page })
}

/// The page Gitea's `Link` header names as next: `None` without the header, `Some(None)`
/// on the last page. Only the page number is read; the address isn't followed.
fn linked_next(headers: &[(String, String)]) -> Option<Option<u32>> {
    let link = header(headers, "link")?;
    Some(link.split(',').find_map(|part| {
        let (url, params) = part.split_once(';')?;
        params.split(';').any(|p| p.trim() == "rel=\"next\"").then_some(())?;
        let query = url.trim().strip_prefix('<')?.strip_suffix('>')?.split_once('?')?.1;
        query.split('&').find_map(|p| p.strip_prefix("page="))?.parse().ok()
    }))
}

fn gitlab_item(config: &ForgeConfig, item: &Json) -> Option<RepositorySearchItem> {
    let path = repository_path(item.str("path_with_namespace")?, true)?;
    let visibility = item.str("visibility");
    Some(RepositorySearchItem {
        description: text(item, "description", 2_000),
        stars: item.u64("star_count").unwrap_or(0),
        forks: item.u64("forks_count").unwrap_or(0),
        open_issues: item.u64("open_issues_count"),
        language: None,
        license: None,
        topics: topics(item.get("topics").or_else(|| item.get("tag_list"))),
        default_branch: text(item, "default_branch", 255),
        size_kb: item.get("statistics").and_then(|s| s.u64("repository_size")).map(|bytes| bytes.div_ceil(1024)),
        updated: item.str("last_activity_at").and_then(checks::timestamp),
        private: visibility == Some("private"),
        internal: visibility == Some("internal"),
        fork: item.get("forked_from_project").is_some_and(|p| matches!(p, Json::Object(_))),
        archived: item.bool("archived").unwrap_or(false),
        mirror: item.bool("mirror").unwrap_or(false),
        web_url: web_url(config, &path),
        clone_url: clone_url(config, item.str("http_url_to_repo"), &path),
        path,
    })
}

fn gitea_item(config: &ForgeConfig, item: &Json) -> Option<RepositorySearchItem> {
    let path = repository_path(item.str("full_name")?, false)?;
    let license = item.get("licenses").and_then(Json::array).and_then(|list| {
        list.iter().find_map(|l| match l {
            Json::String(l) if !l.trim().is_empty() => Some(crate::display_text::bounded(l, 100).text),
            _ => None,
        })
    });
    Some(RepositorySearchItem {
        description: text(item, "description", 2_000),
        stars: item.u64("stars_count").unwrap_or(0),
        forks: item.u64("forks_count").unwrap_or(0),
        open_issues: item.u64("open_issues_count"),
        language: text(item, "language", 60),
        license,
        topics: topics(item.get("topics")),
        default_branch: text(item, "default_branch", 255),
        size_kb: item.u64("size"),
        updated: item.str("updated_at").and_then(checks::timestamp),
        private: item.bool("private").unwrap_or(false),
        internal: item.bool("internal").unwrap_or(false),
        fork: item.bool("fork").unwrap_or(false),
        archived: item.bool("archived").unwrap_or(false),
        mirror: item.bool("mirror").unwrap_or(false),
        web_url: web_url(config, &path),
        clone_url: clone_url(config, item.str("clone_url"), &path),
        path,
    })
}

/// `owner/name`, or on GitLab `group/subgroup/name`: segments of the characters both
/// hosts allow in paths, so the addresses built from them need no escaping.
fn repository_path(path: &str, nested: bool) -> Option<String> {
    let parts: Vec<&str> = path.split('/').collect();
    let fits = |s: &&str| {
        !s.is_empty()
            && s.len() <= 255
            && *s != "."
            && *s != ".."
            && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    (path.len() <= 1024 && parts.len() >= 2 && (nested || parts.len() == 2) && parts.iter().all(fits)).then(|| path.to_owned())
}

fn web_url(config: &ForgeConfig, path: &str) -> String {
    format!("{}/{path}", config.server())
}

/// The configured server's clone address for `path`. Only a loopback fixture may name
/// another one, a folder the test made.
fn clone_url(config: &ForgeConfig, given: Option<&str>, path: &str) -> String {
    match given {
        Some(given) if config.fixture_port().is_some() && !given.is_empty() && given.len() <= 4096 && !given.chars().any(char::is_control) => {
            given.to_owned()
        }
        _ => format!("{}/{path}.git", config.server()),
    }
}

fn text(item: &Json, key: &str, max: usize) -> Option<String> {
    item.str(key)
        .map(|s| crate::display_text::bounded(s, max).text)
        .filter(|s| !s.trim().is_empty())
}

fn topics(list: Option<&Json>) -> Vec<String> {
    list.and_then(Json::array)
        .map(|list| {
            list.iter()
                .filter_map(|t| match t {
                    Json::String(t) if !t.trim().is_empty() && t.len() <= 50 => Some(crate::display_text::bounded(t, 50).text),
                    _ => None,
                })
                .take(20)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hosting::json;

    fn config(provider: ForgeProvider, server: &str) -> ForgeConfig {
        ForgeConfig::new(provider, server).unwrap()
    }
    fn parse(text: &str) -> Json {
        json::parse(text.as_bytes()).unwrap()
    }
    fn headers(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn search_paths_carry_the_host_order_language_and_sizes() {
        let lab = search_path(ForgeProvider::GitLab, " fuzzy finder ", RepositorySearchSort::Stars, Some("C++"), 2, true).unwrap();
        assert_eq!(lab, "/projects?search=fuzzy%20finder&order_by=star_count&sort=desc&per_page=30&page=2&with_programming_language=C%2B%2B&statistics=true");
        let anonymous = search_path(ForgeProvider::GitLab, "x", RepositorySearchSort::Updated, None, 0, false).unwrap();
        assert_eq!(anonymous, "/projects?search=x&order_by=last_activity_at&sort=desc&per_page=30&page=1");
        let tea = search_path(ForgeProvider::Gitea, "static site&q=evil", RepositorySearchSort::Updated, None, 3, true).unwrap();
        assert_eq!(tea, "/repos/search?q=static%20site%26q%3Devil&includeDesc=true&sort=updated&order=desc&limit=30&page=3");
        // Forgejo kept Gitea's search: the same path, and no language filter either.
        assert!(ForgeProvider::Forgejo.searches_repositories() && !ForgeProvider::Forgejo.filters_language());
        assert_eq!(search_path(ForgeProvider::Forgejo, "static site&q=evil", RepositorySearchSort::Updated, None, 3, true).unwrap(), tea);
        for (provider, query, language) in [
            (ForgeProvider::GitLab, "", None),
            (ForgeProvider::GitLab, "a\nb", None),
            (ForgeProvider::Gitea, "site", Some("Go")),
            (ForgeProvider::Forgejo, "site", Some("Go")),
            (ForgeProvider::GitHub, "site", None),
        ] {
            assert!(search_path(provider, query, RepositorySearchSort::Stars, language, 1, false).is_err(), "{provider:?} {query:?}");
        }
        assert!(search_path(ForgeProvider::GitLab, &"x".repeat(501), RepositorySearchSort::Stars, None, 1, false).is_err());
    }

    #[test]
    fn gitlab_projects_keep_namespaces_and_rebuild_addresses() {
        let c = config(ForgeProvider::GitLab, "https://gitlab.example.com/gitlab");
        let value = parse(r#"[
            {"path_with_namespace":"acme/tools/fuzzyfind","description":"Finder\u202e","star_count":1204,"forks_count":98,
             "open_issues_count":37,"topics":["cli","fuzzy-search"],"default_branch":"main","last_activity_at":"2026-11-15T03:12:44.123Z",
             "visibility":"public","archived":false,"http_url_to_repo":"https://evil.example/x.git","web_url":"https://evil.example/x",
             "statistics":{"repository_size":2049}},
            {"path_with_namespace":"contoso/platform/search-ui","star_count":3,"visibility":"private","forked_from_project":{"id":1},"archived":true},
            {"path_with_namespace":"team/internal-tool","visibility":"internal","tag_list":["old"]},
            {"path_with_namespace":"../escape"},
            {"path_with_namespace":"one"},
            {"path_with_namespace":"a b/c"},
            {"name":"no path"}
        ]"#);
        let page = search_page(&c, &value, &headers(&[("x-total", "1248"), ("x-next-page", "2")]), 1).unwrap();
        assert_eq!(page.total, Some(1248));
        assert_eq!(page.next_page, Some(2));
        let paths: Vec<&str> = page.items.iter().map(|i| i.path.as_str()).collect();
        assert_eq!(paths, ["acme/tools/fuzzyfind", "contoso/platform/search-ui", "team/internal-tool"]);
        let first = &page.items[0];
        assert_eq!(first.web_url, "https://gitlab.example.com/gitlab/acme/tools/fuzzyfind");
        assert_eq!(first.clone_url, "https://gitlab.example.com/gitlab/acme/tools/fuzzyfind.git");
        assert_eq!((first.stars, first.forks, first.open_issues), (1204, 98, Some(37)));
        assert_eq!(first.size_kb, Some(3));
        assert_eq!(first.updated, checks::timestamp("2026-11-15T03:12:44Z"));
        assert_eq!(first.topics, ["cli", "fuzzy-search"]);
        assert_eq!((first.language.as_deref(), first.license.as_deref()), (None, None));
        assert!(!first.description.as_deref().unwrap().contains('\u{202e}'), "display text drops bidi controls");
        let second = &page.items[1];
        assert!(second.private && second.fork && second.archived && !second.internal);
        assert_eq!((second.size_kb, second.open_issues, second.updated), (None, None, None));
        let third = &page.items[2];
        assert!(third.internal && !third.private);
        assert_eq!(third.topics, ["old"]);
    }

    #[test]
    fn gitlab_leaves_the_total_out_past_ten_thousand() {
        let c = config(ForgeProvider::GitLab, "https://gitlab.com");
        let page = search_page(&c, &parse("[]"), &headers(&[("x-next-page", "")]), 4).unwrap();
        assert_eq!((page.total, page.next_page), (None, None));
        let page = search_page(&c, &parse("[]"), &headers(&[("x-next-page", "3")]), 4).unwrap();
        assert_eq!(page.next_page, None, "a next page behind this one is refused");
        assert!(search_page(&c, &parse(r#"{"message":"x"}"#), &[], 1).is_err());
    }

    #[test]
    fn gitea_answers_are_read_from_ok_and_data_with_the_total_header() {
        let c = config(ForgeProvider::Gitea, "https://codeberg.org");
        let item = |name: &str| format!(r#"{{"full_name":"{name}","stars_count":42,"forks_count":6,"open_issues_count":3,"language":"Go","size":12288,
            "updated_at":"2026-11-13T10:20:30+01:00","default_branch":"main","private":false,"fork":false,"archived":false,"mirror":true,
            "internal":false,"topics":["static-site"],"licenses":["","MIT"],"clone_url":"https://elsewhere.example/x.git","html_url":"https://elsewhere.example/x"}}"#);
        let full: Vec<String> = (0..30).map(|i| item(&format!("owner/site-{i}"))).collect();
        let value = parse(&format!(r#"{{"ok":true,"data":[{}]}}"#, full.join(",")));
        let page = search_page(&c, &value, &headers(&[("x-total-count", "57")]), 1).unwrap();
        assert_eq!((page.total, page.next_page, page.items.len()), (Some(57), Some(2), 30));
        let first = &page.items[0];
        assert_eq!(first.path, "owner/site-0");
        assert_eq!(first.web_url, "https://codeberg.org/owner/site-0");
        assert_eq!(first.clone_url, "https://codeberg.org/owner/site-0.git");
        assert_eq!((first.language.as_deref(), first.license.as_deref(), first.size_kb), (Some("Go"), Some("MIT"), Some(12288)));
        assert_eq!(first.updated, checks::timestamp("2026-11-13T09:20:30Z"));
        assert!(first.mirror);
        let last = search_page(&c, &value, &headers(&[("x-total-count", "57")]), 2).unwrap();
        assert_eq!(last.next_page, None, "60 shown of 57");
        let unknown = search_page(&c, &parse(&format!(r#"{{"ok":true,"data":[{}]}}"#, item("owner/one"))), &[], 1).unwrap();
        assert_eq!((unknown.total, unknown.next_page), (None, None));
        let nested = parse(&format!(r#"{{"ok":true,"data":[{}]}}"#, item("group/sub/name")));
        assert!(search_page(&c, &nested, &[], 1).unwrap().items.is_empty(), "Gitea paths are owner/name");
        assert!(search_page(&c, &parse(r#"{"ok":false,"error":"x"}"#), &[], 1).is_err());
        assert!(search_page(&c, &parse("[]"), &[], 1).is_err());
    }

    /// A server that caps pages below the 30 asked for: Link names the next page, and
    /// the last page's Link has none; a single page without Link uses its own size.
    #[test]
    fn gitea_pages_follow_link_when_a_server_caps_them() {
        let c = config(ForgeProvider::Gitea, "https://git.example.org");
        let data = |n: usize| parse(&format!(r#"{{"ok":true,"data":[{}]}}"#, (0..n).map(|i| format!(r#"{{"full_name":"o/r{i}"}}"#)).collect::<Vec<_>>().join(",")));
        let link = |rels: &str| headers(&[("x-total-count", "45"), ("link", rels)]);
        let second = search_page(&c, &data(20), &link(r#"<https://git.example.org/api/v1/repos/search?limit=20&page=3&q=x>; rel="next",<https://git.example.org/api/v1/repos/search?limit=20&page=3&q=x>; rel="last""#), 2).unwrap();
        assert_eq!(second.next_page, Some(3), "20 a page: items 41-45 are on page 3");
        let last = search_page(&c, &data(5), &link(r#"<https://git.example.org/api/v1/repos/search?limit=20&page=2&q=x>; rel="prev",<https://git.example.org/api/v1/repos/search?limit=20&page=1&q=x>; rel="first""#), 3).unwrap();
        assert_eq!(last.next_page, None, "the last page names no next page");
        let back = search_page(&c, &data(5), &link(r#"<https://x/a?page=1>; rel="next""#), 3).unwrap();
        assert_eq!(back.next_page, None, "a next page behind this one is refused");
        let single = search_page(&c, &data(20), &headers(&[("x-total-count", "45")]), 1).unwrap();
        assert_eq!(single.next_page, Some(2), "without Link, 20 of 45 means more");
    }

    #[test]
    fn only_a_fixture_may_name_the_clone_address() {
        let fixture = config(ForgeProvider::Gitea, "https://codeberg.org").with_fixture(4321).unwrap();
        assert_eq!(clone_url(&fixture, Some("C:\\fixtures\\site"), "owner/site"), "C:\\fixtures\\site");
        assert_eq!(clone_url(&fixture, Some(""), "owner/site"), "https://codeberg.org/owner/site.git");
        let live = config(ForgeProvider::Gitea, "https://codeberg.org");
        assert_eq!(clone_url(&live, Some("C:\\fixtures\\site"), "owner/site"), "https://codeberg.org/owner/site.git");
    }
}
