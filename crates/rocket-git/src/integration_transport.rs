//! Narrow native transport for reviewed Lua integrations. Scripts never receive
//! credentials or run a shell. The existing runner owns cancellation/processes.
use crate::hosting::{find_curl, neutral_directory, split_response};
use crate::{CancellationToken, CommandLimits, Error, GitClient, Result};
use std::{ffi::OsString, time::Duration};

fn invalid(message: &str) -> Error {
    Error::InvalidInput(message.into())
}
/// A non-authenticating credential binding for protected drafts. This does not
/// retain a session-only token, and uses the existing Windows SHA-256 provider.
pub fn integration_credential_digest(token: &str) -> Result<String> {
    if token.is_empty() { return Ok(String::new()); }
    if token.len() > 4096 { return Err(invalid("integration_auth: Token exceeds its limit.")); }
    crate::advisory_tool::sha256_bytes(token.as_bytes())
        .map_err(|_|invalid("integration_auth: Cannot bind the draft to this credential."))
}
fn quote(s: &str) -> String {
    format!(
        "\"{}\"",
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t")
    )
}

#[derive(Clone)]
pub struct IntegrationConnection {
    origin: String,
    authorization: String,
    fixture: bool,
}
impl IntegrationConnection {
    pub fn new(origin: &str, authorization: &str) -> Result<Self> {
        let host = origin.strip_prefix("https://").ok_or_else(|| {
            invalid(
                "integration_origin: Enter an HTTPS server origin, without a path or credentials.",
            )
        })?;
        let (name, port) = host
            .split_once(':')
            .map_or((host, None), |(a, b)| (a, Some(b)));
        if name.is_empty()
            || name.len() > 253
            || name.split('.').any(|s| {
                s.is_empty()
                    || s.starts_with('-')
                    || s.ends_with('-')
                    || !s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
            || port.is_some_and(|p| p.parse::<u16>().ok().is_none_or(|n| n == 0))
        {
            return Err(invalid(
                "integration_origin: Enter an HTTPS hostname and optional port; paths, redirects and embedded credentials are not accepted.",
            ));
        }
        Self::checked(origin, authorization, false)
    }
    /// Only native fixture hosts may select this. Production preferences never
    /// accept HTTP or a script-provided replacement origin.
    pub fn loopback_fixture(port: u16, authorization: &str) -> Result<Self> {
        if port == 0 {
            return Err(invalid("integration_fixture: Invalid loopback port."));
        }
        Self::checked(&format!("http://127.0.0.1:{port}"), authorization, true)
    }
    fn checked(origin: &str, auth: &str, fixture: bool) -> Result<Self> {
        if auth.len() > 8192 || !auth.bytes().all(|b| (32..=126).contains(&b)) {
            return Err(invalid(
                "integration_auth: Invalid credential characters or length.",
            ));
        }
        Ok(Self {
            origin: origin.into(),
            authorization: auth.into(),
            fixture,
        })
    }
    pub fn origin(&self) -> &str {
        &self.origin
    }
}
pub struct IntegrationHttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

impl GitClient {
    pub fn integration_worker(
        &self,
        worker: &crate::IntegrationWorker,
        input: Vec<u8>,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>> {
        if input.len() > 8 * 1024 * 1024 {
            return Err(invalid(
                "integration_worker: The bundled Lua worker is missing, or the request exceeds its limit. Rebuild/install the complete Rocket bundle.",
            ));
        }
        let mut runner = self.runner.clone();
        runner.limits = CommandLimits {
            timeout: Duration::from_secs(5),
            stdout_bytes: 1024 * 1024,
            stderr_bytes: 4096,
        };
        runner.step("integration-run","integration_worker",||{
            let out=worker.run(input,&runner.limits,cancel)?;
            if !out.status.success(){return Err(invalid("integration_worker_failed: Lua worker stopped unexpectedly. The previous panel is preserved."));}Ok(out.stdout)
        })
    }
    pub fn integration_http(
        &self,
        connection: &IntegrationConnection,
        method: &str,
        path: &str,
        body: Option<&str>,
        reviewed: bool,
        cancel: &CancellationToken,
    ) -> Result<IntegrationHttpResponse> {
        let writing = method != "GET";
        if !matches!(method, "GET" | "POST" | "PUT" | "PATCH" | "DELETE")
            || writing && !reviewed
            || !writing && body.is_some()
        {
            return Err(invalid(
                "integration_permission: Writes require an explicit native review; GET cannot carry a body.",
            ));
        }
        if !path.starts_with('/')
            || path.starts_with("//")
            || path.len() > 8192
            || !path.is_ascii()
            || path
                .bytes()
                .any(|b| b <= 32 || b == 127 || matches!(b, b'"' | b'\\' | b'#'))
            || body.is_some_and(|s| s.len() > 256 * 1024)
        {
            return Err(invalid(
                "integration_request: Invalid relative request path or oversized body.",
            ));
        }
        if self.runner.transport_scope.is_some() && !connection.fixture {
            return Err(invalid(
                "integration_fixture: This isolated window only permits its loopback service.",
            ));
        }
        cancel.check()?;
        let curl = find_curl(&self.runner.executable).ok_or_else(|| {
            invalid("integration_transport: The installed HTTPS helper is unavailable.")
        })?;
        let mut runner = self.runner.clone();
        runner.limits = CommandLimits {
            timeout: Duration::from_secs(20),
            stdout_bytes: 1024 * 1024 + 64 * 1024,
            stderr_bytes: 4096,
        };
        let mut config = format!(
            "url = {}\nrequest = {}\nheader = \"Accept: application/json\"\nheader = \"Content-Type: application/json; charset=utf-8\"\nuser-agent = \"Rocket/{}\"\n",
            quote(&format!("{}{path}", connection.origin)),
            quote(method),
            env!("CARGO_PKG_VERSION")
        );
        if !connection.authorization.is_empty() {
            config.push_str(&format!(
                "header = {}\n",
                quote(&format!("Authorization: {}", connection.authorization))
            ));
        }
        if connection.fixture {
            config.push_str("noproxy = \"*\"\n");
        }
        if let Some(body) = body {
            config.push_str(&format!("data-binary = {}\n", quote(body)));
        }
        let args: Vec<OsString> = [
            "-q",
            "--silent",
            "--show-error",
            "--include",
            "--suppress-connect-headers",
            "--globoff",
            "--proto",
            if connection.fixture {
                "=http"
            } else {
                "=https"
            },
            "--max-redirs",
            "0",
            "--connect-timeout",
            "10",
            "--max-time",
            "18",
            "--config",
            "-",
        ]
        .into_iter()
        .map(Into::into)
        .collect();
        runner.step(if writing{"integration-submit"}else{"integration-read"},"integration_http",||{
            let out=match runner.tool(&curl,"curl",&neutral_directory()?,&args,config.into_bytes(),cancel){
                Ok(out)=>out,Err(_) if writing=>return Err(invalid("integration_outcome_unknown: The request may have reached the service. Check the remote result before allowing another write.")),Err(e)=>return Err(e)
            };
            if !out.status.success(){return Err(invalid(if writing{"integration_outcome_unknown: The remote result is unknown. Inspect the service before retrying."}else if matches!(out.status.code(),Some(51|60)){"integration_tls: The server certificate could not be verified. Check the server address and trusted certificate chain."}else{"integration_network: Could not reach the service. Check the connection, server address and proxy settings."}));}
            let response=split_response(&out.stdout).ok_or_else(||invalid(if writing{"integration_outcome_unknown: The response was incomplete. Inspect the service before retrying."}else{"integration_response: The service returned an invalid HTTP response."}))?;
            let message=match response.status {
                200..=299=>None,401=>Some("integration_unauthorized: Authentication was rejected. Check this connection's account; existing Git credentials were not changed."),
                403=>Some("integration_forbidden: The account cannot perform this operation. Check project permissions and token scopes."),
                404=>Some("integration_not_found: The service endpoint or item was not found."),
                409|412=>Some("integration_conflict: The remote item changed or rejected this operation. Refresh and review again."),
                400|405|422=>Some("integration_rejected: The service rejected the request. Check the extension settings and required fields."),
                429=>Some("integration_rate_limited: The service is rate limiting requests. Wait before trying again."),
                // A service can apply a mutation before redirecting. Keep the
                // durable uncertainty guard even though we never follow it.
                300..=399=>Some(if writing{"integration_outcome_unknown: The service redirected after a write and may have applied it. The redirect was refused. Inspect the remote result before allowing another write."}else{"integration_redirect: Redirects are refused so credentials stay on the approved server. Check the configured origin."}),
                _=>Some(if writing{"integration_outcome_unknown: The service did not confirm the result. Inspect the remote item before retrying."}else{"integration_server: The service returned an unexpected response. Try again after checking its status."})
            };
            if let Some(message)=message{return Err(invalid(message));}
            if response.body.len()>1024*1024{return Err(invalid(if writing{"integration_outcome_unknown: The service response exceeded the limit. Inspect the remote result."}else{"integration_response_limit: The response exceeded 1 MiB. Narrow the query or use pagination."}));}
            Ok(IntegrationHttpResponse{status:response.status,body:response.body.to_vec()})
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn integration_origins_are_explicit() {
        for s in [
            "http://host",
            "https://user@host",
            "https://host/path",
            "https://host?x",
            "https://host#x",
            "https://host\n",
            "https://host:0",
            "https://host:99999",
            "https://.host",
        ] {
            assert!(IntegrationConnection::new(s, "").is_err(), "{s:?}");
        }
        assert!(
            IntegrationConnection::new("https://jira.example.invalid:8443", "Bearer synthetic")
                .is_ok()
        );
        assert!(
            IntegrationConnection::new(
                "https://jira.example.invalid",
                "Bearer x\r\nInjected: true"
            )
            .is_err()
        );
    }
}
