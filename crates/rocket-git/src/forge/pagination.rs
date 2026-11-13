//! Provider continuation URLs are data, not authority to contact another server.
use super::*;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Continuation {
    session: u64,
    repository: ForgeRepository,
    filter: ForgeFilter,
    path: String,
}

impl ForgeSession {
    pub(super) fn continuation_path<'a>(
        &self,
        repo: &ForgeRepository,
        filter: &ForgeFilter,
        cursor: &'a ForgeCursor,
    ) -> Result<Option<&'a str>> {
        let Some(next) = &cursor.continuation else {
            return Ok(None);
        };
        if next.session != self.id || &next.repository != repo || &next.filter != filter {
            return fail(ForgeFailure::InvalidPagination);
        }
        Ok(Some(&next.path))
    }

    /// WIQL builds its own body from the last ID. Empty path binds context only;
    /// it is checked by work_items and never followed as a continuation URL.
    pub(super) fn query_cursor(&self, repo: &ForgeRepository, filter: &ForgeFilter, page: u32, after: u64) -> ForgeCursor {
        ForgeCursor { page, after, continuation: Some(Continuation {
            session: self.id, repository: repo.clone(), filter: filter.clone(), path: String::new(),
        }) }
    }
    pub(super) fn linked_cursor(
        &self,
        repo: &ForgeRepository,
        filter: &ForgeFilter,
        page: u32,
        current: &str,
        url: &str,
    ) -> Result<ForgeCursor> {
        // Require the configured API origin, base path and exact collection.
        // Preserve the query verbatim: Bitbucket cursors need not be page numbers.
        let endpoint = current.split('?').next().expect("request path");
        let expected = format!("{}{endpoint}?", self.config.api());
        let query = url
            .strip_prefix(&expected)
            .or_else(|| {
                self.config.provider().is_github().then(|| {
                    github_id_query(&self.config.api(), &format!("/repos/{}", repo.encoded()), endpoint, url)
                })?
            })
            .filter(|q| !q.is_empty());
        if url.len() > 16 * 1024
            || !url.is_ascii()
            || url
                .bytes()
                .any(|b| b <= 32 || b == 127 || matches!(b, b'"' | b'\\' | b'#'))
            || query.is_none()
        {
            return fail(ForgeFailure::InvalidPagination);
        }
        let path = format!("{endpoint}?{}", query.expect("checked query"));
        if path == current {
            return fail(ForgeFailure::InvalidPagination);
        }
        Ok(ForgeCursor {
            page: page + 1,
            after: 0,
            continuation: Some(Continuation {
                session: self.id,
                repository: repo.clone(),
                filter: filter.clone(),
                path,
            }),
        })
    }
}

/// GitHub's Link header names the repository by its numeric ID: a request to
/// `/repos/octocat/Spoon-Knife/issues` is answered with
/// `/repositories/1300192/issues?page=4` (docs: "Using pagination in the REST API").
/// Accept that form for the same collection under the same API base, and take only
/// its query: the next request keeps Rocket's own `/repos/{owner}/{repo}` path, so a
/// link can never move the request to another repository.
fn github_id_query<'a>(api: &str, scope: &str, endpoint: &str, url: &'a str) -> Option<&'a str> {
    let collection = endpoint.strip_prefix(scope)?;
    if !collection.starts_with('/') {
        return None;
    }
    let tail = url.strip_prefix(api)?.strip_prefix("/repositories/")?;
    let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 || digits > 20 {
        return None;
    }
    tail[digits..].strip_prefix(collection)?.strip_prefix('?')
}

/// Distinguish an absent Link header from a terminal page with only prev/last.
/// Split outside angle brackets/quotes so commas in URL queries are preserved.
pub(super) fn next_link(headers: &[(String, String)]) -> Result<Option<Option<&str>>> {
    let mut present = false;
    let mut next = None;
    for (name, value) in headers {
        if !name.eq_ignore_ascii_case("link") {
            continue;
        }
        present = true;
        let (mut angle, mut quoted) = (false, false);
        for entry in value.split(|c| match c {
            '<' if !quoted => {
                angle = true;
                false
            }
            '>' if !quoted => {
                angle = false;
                false
            }
            '"' if !angle => {
                quoted = !quoted;
                false
            }
            ',' if !angle && !quoted => true,
            _ => false,
        }) {
            let Some((url, parameters)) = entry
                .trim()
                .strip_prefix('<')
                .and_then(|s| s.split_once('>'))
            else {
                return fail(ForgeFailure::InvalidPagination);
            };
            let is_next = parameters
                .split(';')
                .filter_map(|s| s.trim().split_once('='))
                .any(|(k, v)| {
                    k.trim().eq_ignore_ascii_case("rel")
                        && v.trim()
                            .trim_matches('"')
                            .split_ascii_whitespace()
                            .any(|r| r == "next")
                });
            if is_next && next.replace(url).is_some() {
                return fail(ForgeFailure::InvalidPagination);
            }
        }
        if angle || quoted {
            return fail(ForgeFailure::InvalidPagination);
        }
    }
    Ok(present.then_some(next))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn link_relations_preserve_opaque_queries_and_terminal_pages() {
        let headers = vec![("link".into(), "<https://host/api/repos/r/pulls?page=1>; rel=\"prev\", <https://host/api/repos/r/pulls?cursor=a,b;c>; type=\"application/json\"; rel=\"next\"".into())];
        assert_eq!(
            next_link(&headers).unwrap(),
            Some(Some("https://host/api/repos/r/pulls?cursor=a,b;c"))
        );
        assert_eq!(next_link(&[]).unwrap(), None);
        let terminal = vec![(
            "Link".into(),
            "<https://host/api/repos/r/pulls?page=1>; rel=\"prev\"".into(),
        )];
        assert_eq!(next_link(&terminal).unwrap(), Some(None));
        for malformed in [
            "<https://host/path?page=2>; rel=\"next\", <https://host/path?page=3>; rel=\"next\"",
            "<https://host/path?page=2; rel=\"next\"",
            "<https://host/path?page=2>; rel=\"next",
        ] {
            assert!(matches!(
                next_link(&[("link".into(), malformed.into())]),
                Err(Error::Forge(ForgeFailure::InvalidPagination))
            ));
        }
    }
    #[test]
    fn github_links_may_name_the_repository_by_id() {
        let api = "https://api.github.com";
        let scope = "/repos/octocat/Spoon-Knife";
        let issues = "/repos/octocat/Spoon-Knife/issues";
        // The documented example: the request path names the owner, the link the ID.
        assert_eq!(
            github_id_query(api, scope, issues, "https://api.github.com/repositories/1300192/issues?page=4"),
            Some("page=4")
        );
        assert_eq!(
            github_id_query(api, scope, "/repos/octocat/Spoon-Knife/pulls/42/commits",
                "https://api.github.com/repositories/9/pulls/42/commits?per_page=50&page=2"),
            Some("per_page=50&page=2")
        );
        for url in [
            "https://api.github.com/repositories/1300192/pulls?page=2",
            "https://api.github.com/repositories/1300192/issues/extra?page=2",
            "https://api.github.com/repositories/1300192/issues",
            "https://api.github.com/repositories/abc/issues?page=2",
            "https://api.github.com/repositories//issues?page=2",
            "https://api.github.com/repositories/123456789012345678901/issues?page=2",
            "https://api.github.com.evil.test/repositories/1/issues?page=2",
            "https://evil.test/repositories/1/issues?page=2",
            "https://api.github.com/repos/other/repo/issues?page=2",
        ] {
            assert_eq!(github_id_query(api, scope, issues, url), None, "{url}");
        }
        // An endpoint outside the repository scope never takes this form.
        assert_eq!(
            github_id_query(api, scope, "/search/issues", "https://api.github.com/repositories/1/issues?page=2"),
            None
        );
        assert_eq!(
            github_id_query(api, scope, "/repos/octocat/Spoon-Knifeissues", "https://api.github.com/repositories/1issues?page=2"),
            None
        );
    }
}
