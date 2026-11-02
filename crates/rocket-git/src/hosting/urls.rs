//! github.com repository identities and the web links Rocket may open. Only
//! `https://github.com/...` links are produced or accepted for a browser.

const WEB: &str = "https://github.com/";

/// A github.com repository named by a remote URL.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GitHubRemote {
    pub owner: String,
    pub name: String,
}

fn valid_name(part: &str, owner: bool) -> bool {
    !part.is_empty()
        && part.len() <= 100
        && part != "."
        && part != ".."
        && part
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || (!owner && (b == b'_' || b == b'.')))
}

impl GitHubRemote {
    pub fn new(owner: &str, name: &str) -> Option<Self> {
        (valid_name(owner, true) && valid_name(name, false)).then(|| Self {
            owner: owner.into(),
            name: name.into(),
        })
    }
    /// Recognize `https://github.com/o/r(.git)`, `ssh://git@github.com/o/r` and
    /// `git@github.com:o/r`. Other hosts, ports and `insteadOf` rewrites are not
    /// interpreted.
    pub fn parse(url: &[u8]) -> Option<Self> {
        let url = std::str::from_utf8(url).ok()?.trim();
        let rest = if let Some(rest) = url.strip_prefix("https://") {
            let (host, path) = rest.split_once('/')?;
            let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
            host.eq_ignore_ascii_case("github.com").then_some(path)?
        } else if let Some(rest) = url.strip_prefix("ssh://") {
            let (host, path) = rest.split_once('/')?;
            let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
            let host = host.strip_suffix(":22").unwrap_or(host);
            host.eq_ignore_ascii_case("github.com").then_some(path)?
        } else {
            let (user_host, path) = url.split_once(':')?;
            let host = user_host.rsplit_once('@').map_or(user_host, |(_, h)| h);
            (host.eq_ignore_ascii_case("github.com") && !path.starts_with('/')).then_some(path)?
        };
        let rest = rest.strip_suffix('/').unwrap_or(rest);
        let (owner, name) = rest.split_once('/')?;
        let name = name.strip_suffix(".git").unwrap_or(name);
        Self::new(owner, name)
    }
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
    pub fn web_url(&self) -> String {
        format!("{WEB}{}/{}", self.owner, self.name)
    }
    pub fn pulls_url(&self) -> String {
        format!("{}/pulls", self.web_url())
    }
    pub fn branch_url(&self, branch: &str) -> String {
        format!("{}/tree/{}", self.web_url(), encode(branch, true))
    }
    pub fn commit_url(&self, id: &crate::ObjectId) -> String {
        format!("{}/commit/{}", self.web_url(), id.as_str())
    }
    /// GitHub's "Open a pull request" page comparing `head` with `base`. A head in
    /// another repository is written `owner:branch`.
    pub fn compare_url(&self, base: &str, head_owner: Option<&str>, head: &str) -> String {
        let head = match head_owner {
            Some(owner) if owner != self.owner => format!("{owner}:{}", encode(head, true)),
            _ => encode(head, true),
        };
        format!(
            "{}/compare/{}...{head}?expand=1",
            self.web_url(),
            encode(base, true)
        )
    }
}

/// Percent-encode UTF-8 for a URL path or query, keeping unreserved characters
/// (and `/` in branch paths, which GitHub accepts there).
pub(crate) fn encode(text: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric()
            || matches!(b, b'-' | b'_' | b'.' | b'~')
            || (keep_slash && b == b'/')
        {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The URL itself when it is a well-formed `https://github.com/` page that may be
/// handed to the browser: no credentials, spaces, quotes or control characters.
/// Commas are refused too: Windows Explorer splits its command line at them.
pub fn github_web_url(url: &str) -> Option<&str> {
    let path = url.strip_prefix(WEB)?;
    (!path.is_empty()
        && url.len() <= 2048
        && url.bytes().all(|b| {
            b.is_ascii_graphic()
                && !matches!(
                    b,
                    b'"' | b'\'' | b'<' | b'>' | b'\\' | b'^' | b'`' | b'{' | b'|' | b'}' | b','
                )
        }))
    .then_some(url)
}

/// The web page for a notification subject: GitHub returns API URLs, so known
/// subject kinds are mapped and anything else falls back to the repository.
pub(crate) fn notification_url(
    repository: &GitHubRemote,
    kind: &str,
    subject: Option<&str>,
) -> String {
    let base = repository.web_url();
    let api = format!(
        "https://api.github.com/repos/{}/{}/",
        repository.owner, repository.name
    );
    let tail = subject.and_then(|s| s.strip_prefix(api.as_str()));
    let number = |prefix: &str| {
        tail.and_then(|t| t.strip_prefix(prefix)).filter(|n| {
            !n.is_empty() && n.len() <= 40 && n.bytes().all(|b| b.is_ascii_alphanumeric())
        })
    };
    match kind {
        "PullRequest" => number("pulls/").map(|n| format!("{base}/pull/{n}")),
        "Issue" => number("issues/").map(|n| format!("{base}/issues/{n}")),
        "Commit" => number("commits/").map(|n| format!("{base}/commit/{n}")),
        "Release" => Some(format!("{base}/releases")),
        "Discussion" => Some(format!("{base}/discussions")),
        "CheckSuite" | "WorkflowRun" => Some(format!("{base}/actions")),
        _ => None,
    }
    .unwrap_or(base)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remotes_and_links() {
        let r = GitHubRemote::new("rocket-dev", "rocket.git-ui_2").unwrap();
        for url in [
            "https://github.com/rocket-dev/rocket.git-ui_2.git",
            "https://user@github.com/rocket-dev/rocket.git-ui_2/",
            "https://GitHub.com/rocket-dev/rocket.git-ui_2",
            "git@github.com:rocket-dev/rocket.git-ui_2.git",
            "ssh://git@github.com/rocket-dev/rocket.git-ui_2.git",
            "ssh://git@github.com:22/rocket-dev/rocket.git-ui_2",
        ] {
            assert_eq!(
                GitHubRemote::parse(url.as_bytes()),
                Some(r.clone()),
                "{url}"
            );
        }
        for url in [
            "https://gitlab.com/a/b.git",
            "https://github.com.evil.example/a/b",
            "https://github.com/a",
            "https://github.com/a/b/c",
            "git@github.com:/a/b",
            "ssh://git@github.com:2222/a/b",
            "https://github.com/a_b/c",
            "https://github.com/../c",
            "C:\\repos\\b",
        ] {
            assert_eq!(GitHubRemote::parse(url.as_bytes()), None, "{url}");
        }
        assert_eq!(r.full_name(), "rocket-dev/rocket.git-ui_2");
        assert_eq!(
            r.branch_url("feature/中文 #1"),
            "https://github.com/rocket-dev/rocket.git-ui_2/tree/feature/%E4%B8%AD%E6%96%87%20%231"
        );
        assert_eq!(
            r.compare_url("main", None, "feature/a&b"),
            "https://github.com/rocket-dev/rocket.git-ui_2/compare/main...feature/a%26b?expand=1"
        );
        assert_eq!(
            r.compare_url("main", Some("fork-owner"), "fix"),
            "https://github.com/rocket-dev/rocket.git-ui_2/compare/main...fork-owner:fix?expand=1"
        );
        for url in [
            r.web_url(),
            r.pulls_url(),
            r.branch_url("a b,c"),
            r.compare_url("m", None, "x\"y"),
        ] {
            assert!(github_web_url(&url).is_some(), "{url}");
        }
        for bad in [
            "http://github.com/a/b",
            "https://github.com/",
            "https://github.com.evil/a",
            "https://github.com/a b",
            "https://github.com/a\"onload",
            "https://github.com/a\n",
            "https://github.com/a,/e,https://example.com",
            "javascript:alert(1)",
        ] {
            assert!(github_web_url(bad).is_none(), "{bad}");
        }
    }
    #[test]
    fn notification_subjects_map_to_pages() {
        let r = GitHubRemote::new("o", "r").unwrap();
        let api = "https://api.github.com/repos/o/r/";
        assert_eq!(
            notification_url(&r, "PullRequest", Some(&format!("{api}pulls/42"))),
            "https://github.com/o/r/pull/42"
        );
        assert_eq!(
            notification_url(&r, "Issue", Some(&format!("{api}issues/7"))),
            "https://github.com/o/r/issues/7"
        );
        assert_eq!(
            notification_url(&r, "Commit", Some(&format!("{api}commits/abc123"))),
            "https://github.com/o/r/commit/abc123"
        );
        assert_eq!(
            notification_url(&r, "Discussion", None),
            "https://github.com/o/r/discussions"
        );
        // A subject from another repository, or an unknown kind, opens the repository.
        assert_eq!(
            notification_url(
                &r,
                "PullRequest",
                Some("https://api.github.com/repos/x/y/pulls/1")
            ),
            "https://github.com/o/r"
        );
        assert_eq!(
            notification_url(&r, "Unknown", None),
            "https://github.com/o/r"
        );
        assert_eq!(
            notification_url(&r, "Issue", Some(&format!("{api}issues/7?x"))),
            "https://github.com/o/r"
        );
    }
}
