use super::*;
use crate::{
    hosting::{find_curl, header, json, neutral_directory, split_response},
    runner::Output,
};
use std::ffi::OsString;
static SESSION: AtomicU64 = AtomicU64::new(1);
thread_local! {
    /// The permission GitHub named for the last refused request on this thread
    /// (X-Accepted-GitHub-Permissions). Requests run synchronously on their
    /// worker thread, so a caller reads it right after its own refusal.
    static REFUSAL: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}
/// Takes what GitHub said the last refused request on this thread needed.
pub(super) fn take_refusal() -> Option<String> {
    REFUSAL.with(|r| r.borrow_mut().take())
}
fn credential_text(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 4096
        && s.bytes()
            .all(|b| (33..=126).contains(&b) && !matches!(b, b'"' | b'\\'))
}
fn basic(value: &str) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in value.as_bytes().chunks(3) {
        let n = ((c[0] as u32) << 16)
            | ((c.get(1).copied().unwrap_or(0) as u32) << 8)
            | c.get(2).copied().unwrap_or(0) as u32;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if c.len() > 1 {
            TABLE[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            TABLE[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}
impl GitClient {
    /// Browser OAuth via installed GCM with its credential store disabled. The
    /// token is returned only so the UI can optionally persist it using DPAPI.
    pub fn forge_browser_session(&self, config: ForgeConfig, cancel: &CancellationToken) -> Result<(ForgeSession, String)> {
        if !matches!(config.provider(), ForgeProvider::GitHub | ForgeProvider::Bitbucket) {
            return Err(invalid("Browser sign-in supports GitHub.com and Bitbucket Cloud. For a private server, use a token issued by that server."));
        }
        // Explicit loopback fixtures replace only the OAuth exchange. They must
        // never launch the user's browser or read their credential store.
        if config.fixture_port().is_some() {
            let session=ForgeSession {curl:find_curl(&self.runner.executable).ok_or(Error::Forge(ForgeFailure::Unavailable))?,cwd:neutral_directory()?,runner:self.runner.clone(),config:config.clone(),authorization:Arc::from("Bearer rocket-disposable-fixture"),login:String::new(),account_identity:None,enterprise_version:None,browser_auth:true,id:0,publish_fixture_root:None};
            let response=session.request("hosting-browser-sign-in","browser_authentication","POST","/rocket/browser-sign-in",Some("{}"),false,cancel)?.0;
            let token=response.str("access_token").filter(|s|credential_text(s)).ok_or(Error::Forge(ForgeFailure::InvalidResponse))?.to_owned();
            cancel.check()?;
            return self.forge_session(config,"",&token,cancel).map(|mut session|{session.browser_auth=true;(session,token)});
        }
        if self.runner.transport_scope.is_some() && !config.account_check_only() {
            return Err(invalid("Browser sign-in in a scoped test window requires a loopback fixture or the explicit GitHub account-only check."));
        }
        let cwd=neutral_directory()?;
        let mut runner=self.runner.clone();
        runner.limits.timeout=std::time::Duration::from_secs(300);
        runner.limits.stdout_bytes=64*1024;
        let output=runner.step("hosting-browser-sign-in","browser_authentication",|| runner.browser_credential(&cwd,config.provider(),cancel))?;
        if !output.status.success() {return Err(invalid("browser_sign_in_failed: Git Credential Manager could not complete browser sign-in. Check that GCM is installed, then retry or use an access token. Existing Git credentials were not changed."));}
        let text=String::from_utf8(output.stdout).map_err(|_| invalid("browser_sign_in_response: invalid credential response"))?;
        let token=text.lines().find_map(|line| line.strip_prefix("password=")).filter(|s| credential_text(s)).ok_or_else(||invalid("browser_sign_in_response: no valid token returned"))?.to_owned();
        cancel.check()?;
        let mut session=self.forge_session(config,"",&token,cancel)?;
        session.browser_auth=true;
        Ok((session,token))
    }
    pub fn forge_session(
        &self,
        config: ForgeConfig,
        username: &str,
        token: &str,
        cancel: &CancellationToken,
    ) -> Result<ForgeSession> {
        if !credential_text(token) {
            return Err(invalid(
                "Enter an access token without whitespace or control characters (maximum 4096 bytes).",
            ));
        }
        let authorization = match config.provider() {
            ForgeProvider::Azure | ForgeProvider::AzureServer => format!("Basic {}", basic(&format!(":{token}"))),
            ForgeProvider::Bitbucket if username.is_empty() => format!("Bearer {token}"),
            ForgeProvider::Bitbucket => {
                if username.len() > 320
                    || username
                        .chars()
                        .any(|c| c.is_control() || matches!(c, ':' | '"' | '\\'))
                {
                    return Err(invalid(
                        "Enter the Atlassian account email for this API token.",
                    ));
                }
                format!("Basic {}", basic(&format!("{username}:{token}")))
            }
            ForgeProvider::Gitea => format!("token {token}"),
            _ => format!("Bearer {token}"),
        };
        let mut runner = self.runner.clone();
        runner.limits = CommandLimits {
            timeout: Duration::from_secs(60),
            stdout_bytes: 8 * 1024 * 1024,
            stderr_bytes: 64 * 1024,
        };
        let mut session = ForgeSession {
            curl: find_curl(&runner.executable).ok_or(Error::Forge(ForgeFailure::Unavailable))?,
            cwd: neutral_directory()?,
            runner,
            config,
            authorization: Arc::from(authorization),
            login: String::new(),
            account_identity: None,
            enterprise_version: None,
            browser_auth: false,
            id: SESSION.fetch_add(1, Ordering::Relaxed),
            publish_fixture_root: None,
        };
        if session.config.provider() == ForgeProvider::BitbucketServer {
            let (_, headers) = session.request("hosting-connect", "verify_hosting_account", "GET", "/application-properties", None, false, cancel)?;
            // This endpoint can be anonymous. A successful JSON response is not
            // proof of identity; never substitute the user-entered username.
            let identities: Vec<_> = headers.iter().filter(|(k, _)| k.eq_ignore_ascii_case("x-ausername")).collect();
            let identity = (identities.len() == 1).then(|| identities[0].1.as_str())
                .and_then(super::urls::decode)
                .filter(|s| !s.is_empty() && s.len() <= 512 && !s.trim().is_empty()
                    && !s.trim().eq_ignore_ascii_case("anonymous") && !s.chars().any(char::is_control))
                .ok_or(Error::Forge(ForgeFailure::IdentityUnverified))?;
            if !username.is_empty() && username != identity {
                return fail(ForgeFailure::IdentityUnverified);
            }
            session.account_identity = Some(identity.clone());
            session.login = identity;
            return Ok(session);
        }
        let endpoint = if session.config.provider() == ForgeProvider::AzureServer {
            // Microsoft's connection endpoint is versionless, including on Server.
            "/_apis/connectionData"
        } else if session.config.provider() == ForgeProvider::Azure {
            "/_apis/connectionData?api-version=7.1-preview.1"
        } else {
            "/user"
        };
        let (value, headers) = session
            .request(
                "hosting-connect",
                "verify_hosting_account",
                "GET",
                endpoint,
                None,
                false,
                cancel,
            )?;
        if session.config.provider() == ForgeProvider::AzureServer {
            session.login = crate::display_text::bounded(azure_server::authenticated_user(&value)?, 512).text;
            session.account_identity = value.get("authenticatedUser").and_then(|u| u.str("id")).map(str::to_owned);
            cancel.check()?;
            return Ok(session);
        }
        if session.config.provider() == ForgeProvider::GitHubEnterprise {
            session.enterprise_version = enterprise_version(&headers);
        }
        let login = match session.config.provider() {
            ForgeProvider::GitLab => value.str("username"),
            ForgeProvider::Bitbucket => value.str("display_name").or_else(|| value.str("nickname")),
            ForgeProvider::Azure => value.get("authenticatedUser").and_then(|u| {
                u.str("providerDisplayName")
                    .or_else(|| u.str("customDisplayName"))
            }),
            _ => value.str("login"),
        }
        .filter(|s| !s.is_empty() && s.len() <= 512 && !s.chars().any(char::is_control))
        .ok_or(Error::Forge(ForgeFailure::InvalidResponse))?;
        session.login = login.into();
        session.account_identity = match session.config.provider() {
            ForgeProvider::Bitbucket => value.str("uuid"),
            ForgeProvider::Azure => value.get("authenticatedUser").and_then(|u|u.str("id")),
            _ => Some(login),
        }.filter(|s| !s.is_empty() && s.len()<=512 && !s.chars().any(char::is_control)).map(str::to_owned);
        if session.config.publish_check_repository().is_some_and(|r|
            !r.split_once('/').unwrap().0.eq_ignore_ascii_case(&session.login)) {
            return Err(invalid("hosting_publish_account: the account differs from the explicitly approved disposable repository owner."));
        }
        Ok(session)
    }
}
impl ForgeSession {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn request(
        &self,
        operation: &'static str,
        step: &'static str,
        method: &str,
        path: &str,
        body: Option<&str>,
        mutation: bool,
        cancel: &CancellationToken,
    ) -> Result<(Json, Vec<(String, String)>)> {
        self.request_content(operation, step, method, path, body, mutation, false, cancel)
    }
    pub(super) fn request_diff(&self, path: &str, cancel: &CancellationToken) -> Result<String> {
        let (value, _) = self.request_content("hosting-pull-details", "read_pull_diff", "GET", path, None, false, true, cancel)?;
        match value { Json::String(s) => Ok(s), _ => fail(ForgeFailure::InvalidResponse) }
    }
    #[allow(clippy::too_many_arguments)]
    fn request_content(&self, operation: &'static str, step: &'static str, method: &str,
        path: &str, body: Option<&str>, mutation: bool, text: bool, cancel: &CancellationToken,
    ) -> Result<(Json, Vec<(String, String)>)> {
        let run = || self.runner.step(operation,step,||{
            // A refusal names only its own request's permission.
            REFUSAL.with(|r|*r.borrow_mut()=None);
            cancel.check()?;
            if !self.config.allows_request(method, path, body.is_some(), mutation) {
                return Err(invalid("hosting_account_check_only: this live test session only permits GET /user; repository requests and writes are disabled."));
            }
            if !self.config.allows_publish_request(method, path, body, mutation) {
                return Err(invalid("hosting_publish_scope: this live session only permits account verification and private creation/inspection of its single disposable repository."));
            }
            if !path.starts_with('/')||!path.is_ascii()||path.bytes().any(|b|b<=32||matches!(b,b'"'|b'\\'|b'#')){return Err(invalid("Invalid hosting request path."));}
            let url = if path == "/graphql" && self.config.provider().is_github() {
                // GHES GraphQL is outside /api/v3. This fixed route cannot send
                // credentials to a response-provided URL or another origin.
                if self.config.provider() == ForgeProvider::GitHubEnterprise {
                    format!("{}/graphql", self.config.api().strip_suffix("/v3").ok_or_else(||invalid("Invalid enterprise API base."))?)
                } else { format!("{}{path}",self.config.api()) }
            } else { format!("{}{path}",self.config.api()) };
            // Config escaping is separate from JSON escaping. Body and auth only
            // travel on stdin. No response body/stderr is ever included in an error.
            let mut config=format!("url = {}\nrequest = {}\nheader = {}\nheader = \"Accept: application/json\"\nheader = \"Content-Type: application/json; charset=utf-8\"\nheader = \"X-GitHub-Api-Version: 2022-11-28\"\nuser-agent = \"Rocket/{}\"\n",
                quote(&url),quote(method),quote(&format!("Authorization: {}",self.authorization)),env!("CARGO_PKG_VERSION"));
            if text { config = config.replace("Accept: application/json", "Accept: text/plain"); }
            // The MCP fixture must stay on loopback even when the host has a proxy configured.
            if self.config.fixture_port().is_some() { config.push_str("noproxy = \"*\"\n"); }
            if let Some(body)=body{config.push_str(&format!("data-binary = {}\n",quote(body)));}
            let protocol=if self.config.fixture_port().is_some(){"=http"}else{"=https"};
            let args:Vec<OsString>=["-q","--silent","--show-error","--include","--suppress-connect-headers","--globoff","--proto",protocol,"--max-redirs","0","--connect-timeout","15","--max-time","50","--config","-"].into_iter().map(Into::into).collect();
            let output=self.runner.tool(&self.curl,"curl",&self.cwd,&args,config.into_bytes(),cancel);
            let Output{status,stdout,..}=match output{
                Ok(out)=>out,
                Err(_) if mutation=>return fail(ForgeFailure::OutcomeUnknown),
                Err(Error::Cancelled)=>return Err(Error::Cancelled),
                Err(Error::TimedOut)=>return Err(Error::TimedOut),
                Err(_)=>return fail(ForgeFailure::Network)
            };
            if !status.success(){return fail(if mutation{ForgeFailure::OutcomeUnknown}else if matches!(self.config.provider(),ForgeProvider::GitHubEnterprise | ForgeProvider::AzureServer) && matches!(status.code(),Some(51|60)){ForgeFailure::TlsVerification}else{ForgeFailure::Network});}
            let Some(response)=split_response(&stdout)else{return fail(if mutation{ForgeFailure::OutcomeUnknown}else{ForgeFailure::InvalidResponse});};
            let failure=match response.status{
                200..=299=>None,401=>Some(ForgeFailure::Unauthorized),
                403|429 if response.status==429||header(&response.headers,"x-ratelimit-remaining")==Some("0")||header(&response.headers,"retry-after").is_some()=>Some(ForgeFailure::RateLimited),
                403=>Some(ForgeFailure::Forbidden),404=>Some(ForgeFailure::NotFound),
                300..=399 if matches!(self.config.provider(),ForgeProvider::GitHubEnterprise | ForgeProvider::AzureServer) && !mutation=>Some(ForgeFailure::RedirectRefused),
                410 if self.config.provider()==ForgeProvider::GitHubEnterprise && !mutation=>Some(ForgeFailure::ApiVersionUnsupported),
                500..=599 if matches!(self.config.provider(),ForgeProvider::GitHubEnterprise | ForgeProvider::AzureServer) && !mutation=>Some(ForgeFailure::ServerUnavailable),
                400|410 if self.config.provider()==ForgeProvider::AzureServer && !mutation=>Some(ForgeFailure::AzureApiRejected),
                400|409|422=>Some(ForgeFailure::Rejected),
                405 if operation == "hosting-merge-submit" => Some(ForgeFailure::Rejected),
                _=>Some(if mutation{ForgeFailure::OutcomeUnknown}else{ForgeFailure::InvalidResponse})
            };
            if let Some(f)=failure{
                if f==ForgeFailure::Forbidden && self.config.provider().is_github() {
                    let named=header(&response.headers,"x-accepted-github-permissions").filter(|v|v.len()<=256&&v.is_ascii()).map(str::to_owned);
                    REFUSAL.with(|r|*r.borrow_mut()=named);
                }
                return fail(f);
            }
            // Gitea confirms synchronous merges with 200 or 204 and no body.
            // The merge adapter must verify the resulting PR with a scoped GET.
            // A re-run is confirmed by 201 alone: GitHub and Gitea send no body.
            let value=if text {
                // The raw diff is inert data. Keep it bounded and validate UTF-8
                // before parsing paths/hunks. No response links are followed.
                if response.body.len() > 256 * 1024 { return fail(ForgeFailure::InvalidResponse); }
                Json::String(std::str::from_utf8(response.body).map_err(|_| Error::Forge(ForgeFailure::InvalidResponse))?.to_owned())
            } else if (self.config.provider()==ForgeProvider::Gitea && operation=="hosting-merge-submit" && mutation && matches!(response.status,200|204) && response.body.is_empty())
                || (operation==super::checks::RERUN && mutation && matches!(response.status,201|202|204) && response.body.iter().all(u8::is_ascii_whitespace)) { Json::Null } else {json::parse(response.body).ok_or(Error::Forge(if mutation{ForgeFailure::OutcomeUnknown}else{ForgeFailure::InvalidResponse}))?};
            Ok((value,response.headers))
        });
        self.runner.step(operation, self.config.provider().diagnostic_step(), || {
            if self.config.provider() == ForgeProvider::AzureServer {
                // Fixed metadata only; never log the collection, request or PAT.
                let contract = match self.config.azure_api_version() {
                    "6.0" => "hosting_azure_api_6_0",
                    "7.1" => "hosting_azure_api_7_1",
                    _ => "hosting_azure_api_7_0",
                };
                self.runner.step(operation, contract, run)
            } else { run() }
        })
    }
}
fn enterprise_version(headers: &[(String, String)]) -> Option<String> {
    let mut values = headers.iter().filter(|(k,_)| k.eq_ignore_ascii_case("x-github-enterprise-version"));
    let raw = values.next()?.1.as_str();
    if values.next().is_some() { return None; }
    let value = raw.strip_prefix("enterprise-server@").unwrap_or(raw);
    if value.len() > 17 { return None; }
    let parts: Vec<_> = value.split('.').collect();
    (parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit())))
        .then(|| value.to_owned())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn enterprise_version_is_only_bounded_metadata() {
        let h = |v: &str| vec![("X-GitHub-Enterprise-Version".into(),v.into())];
        assert_eq!(enterprise_version(&h("3.21.0")).as_deref(),Some("3.21.0"));
        assert_eq!(enterprise_version(&h("enterprise-server@3.22.1")).as_deref(),Some("3.22.1"));
        for value in ["", "unknown", "3.21", "3.21.0\nsecret", "3.21.0\u{202e}", "9999999.1.0"] { assert_eq!(enterprise_version(&h(value)),None); }
        let mut duplicate=h("3.21.0");duplicate.extend(h("3.20.0"));assert_eq!(enterprise_version(&duplicate),None);
    }
    #[test]
    fn basic_and_quoting() {
        assert_eq!(basic(":abc"), "OmFiYw==");
        assert_eq!(
            json::parse(quote("a\\b\n雪\"").as_bytes()),
            Some(Json::String("a\\b\n雪\"".into()))
        );
    }
}
