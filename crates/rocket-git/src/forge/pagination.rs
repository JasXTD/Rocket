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
        let query = url.strip_prefix(&expected).filter(|q| !q.is_empty());
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
}
