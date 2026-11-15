//! Where Explore searches: GitHub, each GitLab, Gitea or Forgejo account in Preferences →
//! Accounts (or Hosting's connected one), and gitlab.com and codeberg.org without an
//! account. Also one result type for every host, and GitHub's search words, which the
//! other hosts would look for in names and descriptions.
use crate::hosting_accounts::Account;
use rocket_git::{ForgeConfig, ForgeProvider, GitHubSearchItem, RepositorySearchItem};

/// Hosts that need no account: their public repositories, with the host's lower limits.
/// Codeberg runs Forgejo, so a Forgejo account there takes the place of this entry.
pub const PUBLIC: [(ForgeProvider, &str); 2] = [(ForgeProvider::GitLab, "https://gitlab.com"), (ForgeProvider::Forgejo, "https://codeberg.org")];

/// One place Explore can search.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Host {
    /// Stable while Rocket runs: `github`, `account:<provider>|<server>|<label>`,
    /// `live:<provider>|<server>` or `public:<provider>|<server>`.
    pub key: String,
    pub provider: ForgeProvider,
    /// The HTTPS server, as Preferences → Accounts names it.
    pub server: String,
    pub kind: HostKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostKind {
    /// GitHub, with the github.com sign-in Git stores.
    GitHub,
    /// A remembered account in Preferences → Accounts.
    Account { label: String },
    /// Hosting's connected account, not remembered.
    Live,
    /// No account: public repositories only.
    Public,
}

impl Host {
    pub fn github() -> Self {
        Self {
            key: "github".into(),
            provider: ForgeProvider::GitHub,
            server: ForgeProvider::GitHub.default_server().into(),
            kind: HostKind::GitHub,
        }
    }
    pub fn is_github(&self) -> bool {
        self.kind == HostKind::GitHub
    }
    /// The host's name in sentences: GitHub, GitLab, Gitea, Forgejo, or Codeberg for
    /// codeberg.org.
    pub fn name(&self) -> &'static str {
        match self.provider {
            ForgeProvider::Gitea | ForgeProvider::Forgejo if self.site() == "codeberg.org" => "Codeberg",
            ForgeProvider::GitHub => "GitHub",
            ForgeProvider::GitLab => "GitLab",
            ForgeProvider::Gitea => "Gitea",
            ForgeProvider::Forgejo => "Forgejo",
            _ => "GitHub",
        }
    }
    /// The server's host name: `gitlab.com`, `git.example.org`.
    pub fn site(&self) -> String {
        crate::ci_checks::server_name(&self.server)
    }
    /// The account's name in Preferences → Accounts.
    pub fn label(&self) -> Option<&str> {
        match &self.kind {
            HostKind::Account { label } => Some(label),
            _ => None,
        }
    }
    /// The server's API settings, through the loopback fixture when Rocket runs under test.
    pub fn config(&self, fixture: Option<u16>) -> rocket_git::Result<ForgeConfig> {
        let config = ForgeConfig::new(self.provider, &self.server)?;
        match fixture {
            Some(port) => config.with_fixture(port),
            None => Ok(config),
        }
    }
}

/// The hosts in menu order: GitHub, then each account's server, then hosts that need
/// no account and have none. `forge` is false while hosting is off (tests without a
/// fixture), which leaves GitHub alone.
pub fn hosts(forge: bool, accounts: &[Account], live: Option<(ForgeProvider, &str)>) -> Vec<Host> {
    let mut list = vec![Host::github()];
    if !forge {
        return list;
    }
    for a in accounts.iter().filter(|a| a.provider.searches_repositories()) {
        list.push(Host {
            key: format!("account:{}", crate::ci_checks::account_key(a)),
            provider: a.provider,
            server: a.server.clone(),
            kind: HostKind::Account { label: a.label.clone() },
        });
    }
    let has = |list: &[Host], provider: ForgeProvider, server: &str| list.iter().any(|h| h.provider == provider && h.server == server);
    if let Some((provider, server)) = live
        && provider.searches_repositories()
        && !has(&list, provider, server)
    {
        list.push(Host {
            key: format!("live:{}|{server}", provider.key()),
            provider,
            server: server.to_owned(),
            kind: HostKind::Live,
        });
    }
    for (provider, server) in PUBLIC {
        if !has(&list, provider, server) {
            list.push(Host {
                key: format!("public:{}|{server}", provider.key()),
                provider,
                server: server.into(),
                kind: HostKind::Public,
            });
        }
    }
    list
}

/// Host names that take GitLab's and Gitea's short SSH address, `git@host:group/name.git`:
/// the public hosts, gitea.com, and the servers of GitHub, GitLab and Gitea accounts.
pub fn forge_sites(accounts: &[Account]) -> Vec<String> {
    let mut sites: Vec<String> = ["github.com", "gitlab.com", "codeberg.org", "gitea.com"].into_iter().map(String::from).collect();
    for a in accounts {
        if matches!(a.provider, ForgeProvider::GitHub | ForgeProvider::GitHubEnterprise | ForgeProvider::GitLab | ForgeProvider::Gitea | ForgeProvider::Forgejo) {
            let site = crate::ci_checks::server_name(&a.server);
            let site = site.split(':').next().unwrap_or(&site).to_ascii_lowercase();
            if !sites.contains(&site) {
                sites.push(site);
            }
        }
    }
    sites
}

/// One search result, from any host. What the host doesn't list is `None`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// `owner/name`, or a GitLab path with its subgroups.
    pub path: String,
    pub description: Option<String>,
    pub stars: u64,
    pub forks: u64,
    pub open_issues: Option<u64>,
    pub language: Option<String>,
    pub license: Option<String>,
    pub topics: Vec<String>,
    pub default_branch: Option<String>,
    /// The host's size of the whole repository, in kilobytes.
    pub size_kb: Option<u64>,
    /// GitHub's last push, GitLab's last activity or Gitea's last update, in Unix seconds.
    pub updated: Option<i64>,
    pub private: bool,
    /// Visible to everyone signed in to the server.
    pub internal: bool,
    pub fork: bool,
    pub archived: bool,
    pub mirror: bool,
    pub web_url: String,
    pub clone_url: String,
}

impl Item {
    pub fn from_github(i: GitHubSearchItem) -> Self {
        Self {
            path: i.remote.full_name(),
            updated: i.pushed_at.as_deref().and_then(super::unix),
            description: i.description,
            stars: i.stars,
            forks: i.forks,
            open_issues: Some(i.open_issues),
            language: i.language,
            license: i.license,
            topics: i.topics,
            default_branch: Some(i.default_branch),
            size_kb: Some(i.size_kb),
            private: i.private,
            internal: false,
            fork: i.fork,
            archived: i.archived,
            mirror: false,
            web_url: i.html_url,
            clone_url: i.clone_url,
        }
    }
    pub fn from_forge(i: RepositorySearchItem) -> Self {
        Self {
            path: i.path,
            updated: i.updated.and_then(|u| i64::try_from(u).ok()),
            description: i.description,
            stars: i.stars,
            forks: i.forks,
            open_issues: i.open_issues,
            language: i.language,
            license: i.license,
            topics: i.topics,
            default_branch: i.default_branch,
            size_kb: i.size_kb,
            private: i.private,
            internal: i.internal,
            fork: i.fork,
            archived: i.archived,
            mirror: i.mirror,
            web_url: i.web_url,
            clone_url: i.clone_url,
        }
    }
    /// The namespace as it shows before the name, `acme / tools`, and the name.
    pub fn owner_and_name(&self) -> (String, &str) {
        match self.path.rsplit_once('/') {
            Some((owner, name)) => (owner.replace('/', " / "), name),
            None => (String::new(), &self.path),
        }
    }
}

/// The qualifiers of GitHub's repository search: other hosts would look for
/// `language:go` in names and descriptions.
const QUALIFIERS: [&str; 22] = [
    "archived", "created", "followers", "fork", "forks", "good-first-issues", "help-wanted-issues", "in", "is", "language",
    "license", "mirror", "org", "pushed", "repo", "size", "sort", "stars", "template", "topic", "topics", "user",
];

/// The words in `query` that only GitHub's search reads, such as `language:go` or `-stars:>100`.
pub fn github_words(query: &str) -> Vec<&str> {
    query
        .split_whitespace()
        .filter(|w| {
            w.trim_start_matches('-')
                .split_once(':')
                .is_some_and(|(k, v)| !v.is_empty() && QUALIFIERS.contains(&k.to_ascii_lowercase().as_str()))
        })
        .collect()
}

/// `query` without GitHub's search words.
pub fn without_github_words(query: &str) -> String {
    let words = github_words(query);
    query.split_whitespace().filter(|w| !words.contains(w)).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(provider: ForgeProvider, server: &str, label: &str) -> Account {
        Account { label: label.into(), provider, server: server.into(), username: String::new(), token: "t".into(), azure_api_version: None }
    }

    #[test]
    fn the_menu_lists_github_accounts_then_hosts_without_one() {
        assert_eq!(hosts(false, &[account(ForgeProvider::GitLab, "https://gitlab.com", "Me")], None), vec![Host::github()], "hosting off: GitHub alone");
        let accounts = [
            account(ForgeProvider::GitLab, "https://gitlab.com", "Me"),
            account(ForgeProvider::Bitbucket, "https://bitbucket.org", "Work"),
            account(ForgeProvider::Gitea, "https://git.example.org", "Home server"),
        ];
        let list = hosts(true, &accounts, Some((ForgeProvider::GitLab, "https://gitlab.example.com")));
        let keys: Vec<&str> = list.iter().map(|h| h.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "github",
                "account:gitlab|https://gitlab.com|Me",
                "account:gitea|https://git.example.org|Home server",
                "live:gitlab|https://gitlab.example.com",
                "public:forgejo|https://codeberg.org",
            ],
            "gitlab.com moves up to the account; Bitbucket can't search"
        );
        assert_eq!((list[2].name(), list[2].site(), list[2].label()), ("Gitea", "git.example.org".into(), Some("Home server")));
        assert_eq!(list[4].name(), "Codeberg");
        let bare = hosts(true, &[], None);
        assert_eq!(bare.iter().map(|h| h.key.as_str()).collect::<Vec<_>>(), ["github", "public:gitlab|https://gitlab.com", "public:forgejo|https://codeberg.org"]);
        // Codeberg runs Forgejo: a Forgejo account there takes the public entry's place.
        let codeberg = hosts(true, &[account(ForgeProvider::Forgejo, "https://codeberg.org", "Mine")], None);
        assert_eq!(codeberg.iter().map(|h| h.key.as_str()).collect::<Vec<_>>(), ["github", "account:forgejo|https://codeberg.org|Mine", "public:gitlab|https://gitlab.com"]);
        assert_eq!(codeberg[1].name(), "Codeberg");
        let own = hosts(true, &[account(ForgeProvider::Forgejo, "https://forge.example.org", "Forge")], None);
        assert_eq!((own[1].name(), own.len()), ("Forgejo", 4));
        let live_twice = hosts(true, &accounts, Some((ForgeProvider::Gitea, "https://git.example.org")));
        assert!(!live_twice.iter().any(|h| h.kind == HostKind::Live), "Hosting's account is the remembered one");
    }

    #[test]
    fn short_ssh_sites_come_from_forge_accounts() {
        let sites = forge_sites(&[
            account(ForgeProvider::GitLab, "https://GitLab.Example.com:8443/gitlab", "Work"),
            account(ForgeProvider::Azure, "https://dev.azure.com/org", "Azure"),
            account(ForgeProvider::Forgejo, "https://forge.example.org", "Forge"),
        ]);
        assert_eq!(sites, ["github.com", "gitlab.com", "codeberg.org", "gitea.com", "gitlab.example.com", "forge.example.org"]);
    }

    #[test]
    fn github_words_are_found_and_taken_out() {
        assert_eq!(github_words("fuzzy language:go -stars:>100 in:name topic: http://x"), ["language:go", "-stars:>100", "in:name"]);
        assert_eq!(without_github_words("fuzzy  language:go finder stars:>100"), "fuzzy finder");
        assert!(github_words("c++ node:test").is_empty());
    }

    #[test]
    fn items_keep_the_whole_path_and_split_the_namespace() {
        let item = |path: &str| Item {
            path: path.into(),
            description: None,
            stars: 0,
            forks: 0,
            open_issues: None,
            language: None,
            license: None,
            topics: Vec::new(),
            default_branch: None,
            size_kb: None,
            updated: None,
            private: false,
            internal: false,
            fork: false,
            archived: false,
            mirror: false,
            web_url: String::new(),
            clone_url: String::new(),
        };
        assert_eq!(item("acme/tools/fuzzyfind").owner_and_name(), ("acme / tools".into(), "fuzzyfind"));
        assert_eq!(item("owner/name").owner_and_name(), ("owner".into(), "name"));
    }
}
