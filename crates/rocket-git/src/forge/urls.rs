use super::*;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForgeConfig {
    provider: ForgeProvider,
    server: String,
    api: String,
    azure_api_version: &'static str,
    fixture: Option<u16>,
    account_check_only: bool,
    publish_check: Option<String>,
}
impl ForgeConfig {
    pub fn new(provider: ForgeProvider, server: &str) -> Result<Self> {
        let server = server.trim_end_matches('/');
        let rest = server
            .strip_prefix("https://")
            .ok_or_else(|| invalid("Hosting servers require an HTTPS URL."))?;
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        if authority.is_empty()
            || server.len() > 512
            || !authority.is_ascii()
            || !authority
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':'))
            || authority.starts_with(['.', '-'])
            || authority.ends_with(['.', '-'])
            || authority.matches(':').count() > 1
            || authority.split_once(':').is_some_and(|(h, p)| {
                h.is_empty() || p.parse::<u16>().ok().filter(|p| *p > 0).is_none()
            })
            || (provider != ForgeProvider::AzureServer && path
                .split('/')
                .any(|s| !s.is_empty() && (!segment(s) || !s.is_ascii() || s.contains(' '))))
        {
            return Err(invalid(
                "Use a plain HTTPS server URL without credentials, query, fragment or encoded path components.",
            ));
        }
        let collection;
        let path = if provider == ForgeProvider::AzureServer {
            collection = azure_server::collection_path(path)?;
            collection.as_str()
        } else { path };
        let server = format!(
            "https://{}{}",
            authority.to_ascii_lowercase(),
            if path.is_empty() {
                String::new()
            } else {
                format!("/{path}")
            }
        );
        if server.len() > 512 { return Err(invalid("The encoded HTTPS server URL must fit within 512 bytes.")); }
        match provider {
            ForgeProvider::GitHub if server != "https://github.com" => {
                return Err(invalid(
                    "This adapter supports github.com. Choose another provider for its server.",
                ));
            }
            ForgeProvider::GitHubEnterprise if !path.is_empty()
                || matches!(authority.split(':').next().unwrap_or_default().to_ascii_lowercase().as_str(), "github.com" | "api.github.com") => {
                return Err(invalid("For GitHub Enterprise Server, enter its HTTPS server address without /api/v3 or a repository path. Use the GitHub provider for github.com."));
            }
            ForgeProvider::Bitbucket if server != "https://bitbucket.org" => {
                return Err(invalid(
                    "This adapter supports Bitbucket Cloud at https://bitbucket.org.",
                ));
            }
            ForgeProvider::AzureServer if {
                let host = authority.split(':').next().unwrap_or_default().to_ascii_lowercase();
                host == "dev.azure.com" || host.ends_with(".dev.azure.com") || host == "visualstudio.com" || host.ends_with(".visualstudio.com")
            } => return Err(invalid("Use the Azure DevOps Services provider for Microsoft-hosted organizations. Server requires your on-premises HTTPS collection URL.")),
            ForgeProvider::Azure
                if authority != "dev.azure.com" || path.is_empty() || path.contains('/') =>
            {
                return Err(invalid(
                    "Use https://dev.azure.com/your-organization for Azure DevOps Services.",
                ));
            }
            _ => {}
        }
        let api = match provider {
            ForgeProvider::GitHub => "https://api.github.com".into(),
            ForgeProvider::GitHubEnterprise => format!("{server}/api/v3"),
            ForgeProvider::Bitbucket => "https://api.bitbucket.org/2.0".into(),
            ForgeProvider::BitbucketServer => format!("{server}/rest/api/1.0"),
            ForgeProvider::GitLab => format!("{server}/api/v4"),
            ForgeProvider::Gitea => format!("{server}/api/v1"),
            ForgeProvider::Azure | ForgeProvider::AzureServer => server.clone(),
        };
        Ok(Self {
            provider,
            server,
            api,
            azure_api_version: if provider == ForgeProvider::AzureServer { "7.0" } else { "7.1" },
            fixture: None,
            account_check_only: false,
            publish_check: None,
        })
    }
    /// Explicit contract selection for on-premises Server; never negotiated or downgraded.
    pub fn with_azure_api_version(mut self, version: &str) -> Result<Self> {
        if self.provider != ForgeProvider::AzureServer {
            return Err(invalid("API version selection is only available for Azure DevOps Server."));
        }
        self.azure_api_version = match version {
            "6.0" => "6.0", "7.0" => "7.0", "7.1" => "7.1",
            _ => return Err(invalid("Choose Azure REST API 6.0, 7.0 or 7.1 for this server.")),
        };
        Ok(self)
    }
    pub fn azure_api_version(&self) -> &'static str { self.azure_api_version }
    /// Explicit test-only transport override. Provider identity remains unchanged.
    pub fn with_fixture(mut self, port: u16) -> Result<Self> {
        if port == 0 || self.account_check_only || self.publish_check.is_some() {
            return Err(invalid("Fixture port must be nonzero."));
        }
        self.fixture = Some(port);
        Ok(self)
    }
    /// Explicit live acceptance scope: the returned session can only verify
    /// the GitHub account. It cannot browse repositories or perform API writes.
    pub fn with_github_account_check(mut self) -> Result<Self> {
        if self.provider != ForgeProvider::GitHub || self.fixture.is_some() || self.publish_check.is_some() {
            return Err(invalid("Live account checks require github.com without a fixture override."));
        }
        self.account_check_only = true;
        Ok(self)
    }
    pub fn account_check_only(&self) -> bool {
        self.account_check_only
    }
    /// Operator-only live acceptance: one randomly named private repository.
    /// This scope cannot delete repositories or access other repository APIs.
    pub fn with_github_publish_check(mut self, repository: &str) -> Result<Self> {
        let (owner, name) = repository.split_once('/').ok_or_else(|| invalid("Invalid publish acceptance repository."))?;
        let suffix = name.strip_prefix("rocket-publish-test-").unwrap_or_default();
        if self.provider != ForgeProvider::GitHub || self.fixture.is_some() || self.account_check_only
            || owner.is_empty() || owner.len() > 39 || owner.starts_with('-') || owner.ends_with('-')
            || !owner.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || suffix.len() != 32 || !suffix.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
            return Err(invalid("Live publication requires one explicit github.com disposable repository."));
        }
        self.publish_check = Some(repository.to_owned());
        Ok(self)
    }
    pub fn publish_check_repository(&self) -> Option<&str> { self.publish_check.as_deref() }
    pub(super) fn allows_publish_request(&self, method: &str, path: &str, body: Option<&str>, mutation: bool) -> bool {
        let Some(repository) = self.publish_check_repository() else { return true };
        if method == "GET" && body.is_none() && !mutation {
            return path == "/user" || path == format!("/repos/{repository}");
        }
        if method != "POST" || path != "/user/repos" || !mutation { return false; }
        let Some(value @ Json::Object(_)) = body.and_then(|b| crate::hosting::json::parse(b.as_bytes())) else { return false };
        let Json::Object(fields) = &value else { unreachable!() };
        fields.len() == 4 && fields.iter().all(|(k,_)| matches!(k.as_str(),"name"|"description"|"private"|"auto_init"))
            && value.str("name") == repository.split_once('/').map(|(_,n)|n)
            && value.str("description").is_some() && value.bool("private") == Some(true) && value.bool("auto_init") == Some(false)
    }
    pub(super) fn allows_request(&self, method: &str, path: &str, has_body: bool, mutation: bool) -> bool {
        !self.account_check_only || (method == "GET" && path == "/user" && !has_body && !mutation)
    }
    pub fn fixture_port(&self) -> Option<u16> {
        self.fixture
    }
    pub fn provider(&self) -> ForgeProvider {
        self.provider
    }
    pub fn server(&self) -> &str {
        &self.server
    }
    pub(super) fn api(&self) -> String {
        match self.fixture {
            Some(p) if self.provider == ForgeProvider::AzureServer => {
                let collection = self.server.strip_prefix("https://").unwrap().split_once('/').unwrap().1;
                format!("http://127.0.0.1:{p}/azure-server/{collection}")
            },
            Some(p) if self.provider == ForgeProvider::GitHubEnterprise => format!("http://127.0.0.1:{p}/github-enterprise/api/v3"),
            Some(p) => format!("http://127.0.0.1:{p}/{}", self.provider.key()),
            None => self.api.clone(),
        }
    }
}
pub(super) fn segment(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.len() <= 256
        && !s.chars().any(|c| {
            c.is_control()
                || matches!(
                    c,
                    '/' | '\\' | '?' | '#' | '%' | '@' | ':' | '"' | '<' | '>' | '|' | '*'
                )
        })
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForgeRepository {
    pub(super) config: ForgeConfig,
    path: String,
}
impl ForgeRepository {
    pub fn new(config: ForgeConfig, path: &str) -> Result<Self> {
        let parts: Vec<_> = path.split('/').collect();
        if path.len() > 1024
            || parts.len() < 2
            || parts.iter().any(|p| !segment(p))
            || (config.provider() != ForgeProvider::GitLab && parts.len() != 2)
            || (config.provider() == ForgeProvider::BitbucketServer && parts.first() == Some(&"~"))
        {
            return Err(invalid(
                "Enter owner/repository (nested namespaces allowed on GitLab), project/repository for Azure or Bitbucket Server, or ~user/repository for a personal Bitbucket project.",
            ));
        }
        Ok(Self {
            config,
            path: path.into(),
        })
    }
    /// Infer the repository only when the remote matches this selected server.
    /// SSH aliases are intentionally left for explicit profile selection.
    pub fn from_remote(config: ForgeConfig, remote: &[u8]) -> Option<Self> {
        let remote = std::str::from_utf8(remote).ok()?;
        if config.provider() == ForgeProvider::AzureServer { return azure_server::repository_from_remote(config, remote); }
        let prefix = format!("{}/", config.server());
        let path = if let Some(path) = remote.strip_prefix(&prefix) {
            if config.provider() == ForgeProvider::Azure {
                let (project, repo) = path.split_once("/_git/")?;
                format!("{project}/{repo}")
            } else if config.provider() == ForgeProvider::BitbucketServer {
                path.strip_prefix("scm/")?.to_owned()
            } else {
                path.to_owned()
            }
        } else {
            let host = config
                .server()
                .strip_prefix("https://")?
                .split('/')
                .next()?
                .split(':')
                .next()?;
            let path = if let Some(rest) = remote.strip_prefix("ssh://git@") {
                let (authority, path) = rest.split_once('/')?;
                let ssh_host = authority.split(':').next()?;
                if ssh_host != host {
                    return None;
                }
                path
            } else if config.provider() == ForgeProvider::Azure {
                let org = config.server().strip_prefix("https://dev.azure.com/")?;
                let rest = remote.strip_prefix("git@ssh.dev.azure.com:v3/")?;
                rest.strip_prefix(&format!("{org}/"))?
            } else {
                remote.strip_prefix(&format!("git@{host}:"))?
            };
            path.to_owned()
        };
        let path = decode(path.strip_suffix(".git").unwrap_or(&path))?;
        Self::new(config, &path).ok()
    }
    pub fn path(&self) -> &str {
        &self.path
    }
    /// Known public hosts only. Custom hosts need an explicit provider/server,
    /// never a guess based on a hostname or repository-supplied command.
    pub fn from_known_remote(remote: &[u8]) -> Option<Self> {
        for provider in [ForgeProvider::GitHub, ForgeProvider::GitLab, ForgeProvider::Bitbucket, ForgeProvider::Gitea] {
            if let Some(repo) = Self::from_remote(ForgeConfig::new(provider, provider.default_server()).ok()?, remote) {
                return Some(repo);
            }
        }
        let value = std::str::from_utf8(remote).ok()?;
        let rest = value.strip_prefix("https://dev.azure.com/")
            .or_else(|| value.strip_prefix("git@ssh.dev.azure.com:v3/"))?;
        let org = rest.split('/').next()?;
        let config = ForgeConfig::new(ForgeProvider::Azure, &format!("https://dev.azure.com/{org}")).ok()?;
        Self::from_remote(config, remote)
    }
    /// Browser URL for a validated full commit ID. Contains no remote credentials.
    pub fn commit_url(&self, commit: &ObjectId) -> String {
        let prefix = self.config.server();
        let id = commit.as_str();
        let path = self.encoded();
        match self.config.provider() {
            ForgeProvider::GitLab => format!("{prefix}/{path}/-/commit/{id}"),
            ForgeProvider::Bitbucket => format!("{prefix}/{path}/commits/{id}"),
            ForgeProvider::BitbucketServer => {
                let (project, repo) = self.parts();
                let project = if let Some(user) = project.strip_prefix('~') {
                    format!("users/{}", encode(user))
                } else { format!("projects/{}", encode(project)) };
                format!("{prefix}/{project}/repos/{}/commits/{id}", encode(repo))
            }
            ForgeProvider::Azure | ForgeProvider::AzureServer => {
                let (project, repo) = self.parts();
                format!("{prefix}/{}/_git/{}/commit/{id}", encode(project), encode(repo))
            }
            _ => format!("{prefix}/{path}/commit/{id}"),
        }
    }
    pub fn config(&self) -> &ForgeConfig {
        &self.config
    }
    pub(super) fn encoded(&self) -> String {
        self.path
            .split('/')
            .map(encode)
            .collect::<Vec<_>>()
            .join("/")
    }
    pub(super) fn parts(&self) -> (&str, &str) {
        self.path.split_once('/').expect("validated repository")
    }
    pub fn item_url(&self, kind: ForgeKind, number: u64) -> String {
        let prefix = self.config.server();
        if self.config.provider() == ForgeProvider::BitbucketServer {
            let (project, repo) = self.parts();
            let project = if let Some(user) = project.strip_prefix('~') {
                format!("users/{}", encode(user))
            } else { format!("projects/{}", encode(project)) };
            return format!("{prefix}/{project}/repos/{}/pull-requests/{number}", encode(repo));
        }
        let p = self.encoded();
        let suffix = match (self.config.provider(), kind) {
            (ForgeProvider::GitLab, ForgeKind::Pulls) => "-/merge_requests",
            (ForgeProvider::GitLab, ForgeKind::Issues) => "-/issues",
            (ForgeProvider::Bitbucket, _) => "pull-requests",
            (ForgeProvider::GitHub | ForgeProvider::GitHubEnterprise, ForgeKind::Pulls) => "pull",
            (_, ForgeKind::Pulls) => "pulls",
            (_, ForgeKind::Issues) => "issues",
        };
        if self.config.provider().is_azure() {
            let (project, repo) = self.parts();
            let project = encode(project);
            let repo = encode(repo);
            return match kind {
                ForgeKind::Pulls => format!("{prefix}/{project}/_git/{repo}/pullrequest/{number}"),
                ForgeKind::Issues => format!("{prefix}/{project}/_workitems/edit/{number}"),
            };
        }
        format!("{prefix}/{p}/{suffix}/{number}")
    }
}
pub(super) fn decode(s: &str) -> Option<String> {
    let mut out = Vec::new();
    let mut bytes = s.as_bytes().iter().copied();
    while let Some(b) = bytes.next() {
        if b == b'%' {
            let a = (bytes.next()? as char).to_digit(16)?;
            let b = (bytes.next()? as char).to_digit(16)?;
            out.push((a * 16 + b) as u8);
        } else {
            out.push(b);
        }
    }
    String::from_utf8(out).ok()
}
pub(crate) fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn azure_server_collection_versions_and_remote_boundaries() {
        let p = ForgeProvider::AzureServer;
        let c = ForgeConfig::new(p, "https://DEVOPS.example.com:8443/tfs/資料 Space/").unwrap();
        assert_eq!(c.server(), "https://devops.example.com:8443/tfs/%E8%B3%87%E6%96%99%20Space");
        assert_eq!(c.api(), c.server());
        assert_eq!(c.azure_api_version(), "7.0");
        assert_eq!(ForgeProvider::from_key("azure-server"), Some(p));
        for version in ["6.0", "7.0", "7.1"] { assert_eq!(c.clone().with_azure_api_version(version).unwrap().azure_api_version(), version); }
        for bad in ["", "5.0", "7.2", "7.1-preview.1", "7.0&other=1"] { assert!(c.clone().with_azure_api_version(bad).is_err()); }
        assert!(ForgeConfig::new(ForgeProvider::Azure, "https://dev.azure.com/org").unwrap().with_azure_api_version("7.0").is_err());
        for url in ["http://host/tfs/Collection", "https://host", "https://host/tfs//Collection", "https://host/tfs/%2e%2e", "https://host/tfs/a%2fb", "https://host/tfs/a%5cb", "https://host/tfs/a%252eb", "https://host/tfs/a?token=x", "https://host/tfs/a#x", "https://host/tfs/Collection/_apis", "https://host/tfs/Collection/team/_git/repo", "https://user:secret@host/tfs/Collection", "https://dev.azure.com/org", "https://org.visualstudio.com/DefaultCollection"] {
            assert!(ForgeConfig::new(p, url).is_err(), "{url}");
        }
        for remote in ["https://devops.example.com:8443/tfs/%E8%B3%87%E6%96%99%20Space/team/_git/資料.git", "https://DEVOPS.example.com:8443/tfs/資料 Space/team/_git/資料.git", "ssh://git@devops.example.com:22/tfs/%E8%B3%87%E6%96%99%20Space/team/_git/%E8%B3%87%E6%96%99"] {
            let repo = ForgeRepository::from_remote(c.clone(), remote.as_bytes()).unwrap();
            assert_eq!(repo.path(), "team/資料");
            assert_eq!(repo.item_url(ForgeKind::Pulls,42), format!("{}/team/_git/%E8%B3%87%E6%96%99/pullrequest/42",c.server()));
            assert_eq!(repo.commit_url(&ObjectId::parse(&"a".repeat(40)).unwrap()), format!("{}/team/_git/%E8%B3%87%E6%96%99/commit/{}", c.server(), "a".repeat(40)));
        }
        for remote in ["https://devops.example.com:8443/tfs/other/team/_git/repo", "ssh://git@devops.example.com/other/team/_git/repo", "ssh://git@alias/tfs/%E8%B3%87%E6%96%99%20Space/team/_git/repo", "ssh://git@devops.example.com:0/tfs/%E8%B3%87%E6%96%99%20Space/team/_git/repo", "https://devops.example.com:8443/tfs/%E8%B3%87%E6%96%99%20Space/team/_git/%2e%2e"] {
            assert!(ForgeRepository::from_remote(c.clone(),remote.as_bytes()).is_none(), "{remote}");
        }
        assert_eq!(c.with_fixture(12345).unwrap().api(), "http://127.0.0.1:12345/azure-server/tfs/%E8%B3%87%E6%96%99%20Space");
    }
    #[test]
    fn enterprise_origin_api_and_remote_links() {
        let p = ForgeProvider::GitHubEnterprise;
        let c = ForgeConfig::new(p, "https://GIT.example.com:8443/").unwrap();
        assert_eq!(c.server(), "https://git.example.com:8443");
        assert_eq!(c.api(), "https://git.example.com:8443/api/v3");
        assert_eq!(ForgeProvider::from_key("github-enterprise"), Some(p));
        assert_ne!(ForgeProvider::from_key("github"), Some(p));
        for url in ["http://git.example.com", "https://git.example.com/api/v3", "https://git.example.com/team/repo", "https://user:token@git.example.com", "https://git.example.com?token=x", "https://git.example.com/#fragment", "https://git.example.com:0", "https://github.com", "https://API.GITHUB.COM:443"] {
            assert!(ForgeConfig::new(p,url).is_err(), "{url}");
        }
        for remote in ["https://git.example.com:8443/team/資料.git", "git@git.example.com:team/資料.git", "ssh://git@git.example.com:2222/team/%E8%B3%87%E6%96%99.git"] {
            let repo = ForgeRepository::from_remote(c.clone(),remote.as_bytes()).unwrap();
            assert_eq!(repo.path(), "team/資料");
            assert_eq!(repo.item_url(ForgeKind::Pulls,42), "https://git.example.com:8443/team/%E8%B3%87%E6%96%99/pull/42");
            assert_eq!(repo.item_url(ForgeKind::Issues,3), "https://git.example.com:8443/team/%E8%B3%87%E6%96%99/issues/3");
        }
        for remote in ["https://git.example.com/team/repo.git", "https://git.example.com:8443.evil/team/repo.git", "git@evil.example:team/repo.git"] {
            assert!(ForgeRepository::from_remote(c.clone(),remote.as_bytes()).is_none());
        }
        assert_eq!(c.with_fixture(12345).unwrap().api(), "http://127.0.0.1:12345/github-enterprise/api/v3");
    }
    #[test]
    fn bitbucket_server_context_paths_and_personal_projects() {
        let c = ForgeConfig::new(ForgeProvider::BitbucketServer,"https://git.example.com:8443/bitbucket").unwrap();
        assert_eq!(c.api(),"https://git.example.com:8443/bitbucket/rest/api/1.0");
        for remote in ["https://git.example.com:8443/bitbucket/scm/TEAM/repo.git", "ssh://git@git.example.com:7999/TEAM/repo.git", "git@git.example.com:TEAM/repo.git"] {
            assert_eq!(ForgeRepository::from_remote(c.clone(),remote.as_bytes()).unwrap().path(),"TEAM/repo");
        }
        let personal=ForgeRepository::new(c.clone(),"~user/資料").unwrap();
        assert_eq!(personal.item_url(ForgeKind::Pulls,7),"https://git.example.com:8443/bitbucket/users/user/repos/%E8%B3%87%E6%96%99/pull-requests/7");
        assert!(ForgeRepository::new(c.clone(),"~/repo").is_err());
        for remote in ["https://git.example.com/scm/TEAM/repo.git","https://evil.example.com:8443/bitbucket/scm/TEAM/repo.git","https://git.example.com:8443/bitbucket/scm/TEAM/%2e%2e.git"] {
            assert!(ForgeRepository::from_remote(c.clone(),remote.as_bytes()).is_none());
        }
        assert_eq!(ForgeProvider::from_key("bitbucket"),Some(ForgeProvider::Bitbucket));
        assert_eq!(ForgeProvider::from_key("bitbucket-server"),Some(ForgeProvider::BitbucketServer));
    }
    #[test]
    fn origins_and_unicode_are_explicit() {
        for bad in [
            "http://gitlab.com",
            "https://user:pass@gitlab.com",
            "https://gitlab.com?x",
            "https://gitlab.com/%2e",
            "https://gitlab.com/a/../b",
            "https://gitlab.com:0",
        ] {
            assert!(
                ForgeConfig::new(ForgeProvider::GitLab, bad).is_err(),
                "{bad}"
            );
        }
        let c = ForgeConfig::new(ForgeProvider::GitLab, "https://git.example:443/root").unwrap();
        let r = ForgeRepository::new(c, "team/nested/資料").unwrap();
        assert_eq!(r.encoded(), "team/nested/%E8%B3%87%E6%96%99");
        assert!(ForgeRepository::new(r.config.clone(), "team/../x").is_err());
        assert!(
            ForgeConfig::new(ForgeProvider::Azure, "https://dev.azure.com/org/project").is_err()
        );
    }
}

#[cfg(test)]
mod remote_tests {
    use super::*;
    #[test]
    fn live_publish_scope_allows_only_exact_private_creation() {
        let repository="Tester/rocket-publish-test-0123456789abcdef0123456789abcdef";
        let c=ForgeConfig::new(ForgeProvider::GitHub,"https://github.com").unwrap().with_github_publish_check(repository).unwrap();
        let body="{\"name\":\"rocket-publish-test-0123456789abcdef0123456789abcdef\",\"description\":\"Synthetic\",\"private\":true,\"auto_init\":false}";
        assert!(c.allows_publish_request("GET","/user",None,false));
        assert!(c.allows_publish_request("GET",&format!("/repos/{repository}"),None,false));
        assert!(c.allows_publish_request("POST","/user/repos",Some(body),true));
        for (method,path,body,mutation) in [
            ("GET","/repos/Tester/existing",None,false),
            ("GET","/user?redirect=x",None,false),
            ("DELETE","/user/repos",None,true),
            ("POST","/user/repos",Some(body),false),
            ("POST","/orgs/Tester/repos",Some(body),true),
            ("GET","/user",Some(body),false),
        ] { assert!(!c.allows_publish_request(method,path,body,mutation)); }
        for invalid in [body.replace("true","false"),body.replace("0123456789abcdef0123456789abcdef","ffffffffffffffffffffffffffffffff"),body.replace("\"auto_init\":false","\"auto_init\":true"),body.replace("\"description\":\"Synthetic\"","\"homepage\":\"https://example.invalid\"")] {
            assert!(!c.allows_publish_request("POST","/user/repos",Some(&invalid),true));
        }
        assert!(c.clone().with_fixture(12345).is_err());assert!(c.with_github_account_check().is_err());
        for invalid in ["Tester/ordinary","Tester/rocket-publish-test-123", "Tester/rocket-publish-test-0123456789abcdef0123456789abcdef/extra", "Tester/rocket-publish-test-0123456789ABCDEF0123456789ABCDEF"] {
            assert!(ForgeConfig::new(ForgeProvider::GitHub,"https://github.com").unwrap().with_github_publish_check(invalid).is_err());
        }
    }
    #[test]
    fn live_account_check_refuses_repository_requests_and_mutations() {
        let c = ForgeConfig::new(ForgeProvider::GitHub, "https://github.com").unwrap()
            .with_github_account_check().unwrap();
        assert!(c.allows_request("GET", "/user", false, false));
        for (method, path, body, mutation) in [
            ("GET", "/user?other", false, false),
            ("GET", "/repos/JasXTD/rocket-auth-test", false, false),
            ("POST", "/user", false, false),
            ("GET", "/user", true, false),
            ("GET", "/user", false, true),
        ] { assert!(!c.allows_request(method, path, body, mutation)); }
        assert!(c.with_fixture(12345).is_err());
        assert!(ForgeConfig::new(ForgeProvider::GitLab, "https://gitlab.com").unwrap().with_github_account_check().is_err());
        assert!(ForgeConfig::new(ForgeProvider::GitHub, "https://github.com").unwrap().with_fixture(12345).unwrap().with_github_account_check().is_err());
    }
    #[test]
    fn remotes_require_the_selected_server_and_preserve_nested_unicode() {
        let c = ForgeConfig::new(ForgeProvider::GitLab, "https://gitlab.example").unwrap();
        for url in [
            "https://gitlab.example/team/sub/%E8%B3%87%E6%96%99.git",
            "git@gitlab.example:team/sub/資料.git",
            "ssh://git@gitlab.example:2222/team/sub/資料.git",
        ] {
            assert_eq!(
                ForgeRepository::from_remote(c.clone(), url.as_bytes())
                    .unwrap()
                    .path(),
                "team/sub/資料"
            );
        }
        for url in [
            "https://gitlab.example.attacker.test/team/repo",
            "https://gitlab.example/team/%2e%2e",
            "git@alias:team/repo",
        ] {
            assert!(ForgeRepository::from_remote(c.clone(), url.as_bytes()).is_none());
        }
        let a = ForgeConfig::new(ForgeProvider::Azure, "https://dev.azure.com/org").unwrap();
        for url in [
            "https://dev.azure.com/org/project/_git/repo",
            "git@ssh.dev.azure.com:v3/org/project/repo",
        ] {
            assert_eq!(
                ForgeRepository::from_remote(a.clone(), url.as_bytes())
                    .unwrap()
                    .path(),
                "project/repo"
            );
        }
    }
}
