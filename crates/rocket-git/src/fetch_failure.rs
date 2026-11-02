//! Conservative hints from failed fetch stderr, never authorization decisions.

/// A best-effort interpretation of Git/SSH/helper diagnostics. A server or helper
/// can supply this text; the category is a hint, not proof of an underlying cause.
/// Unknown/localized output stays `Other`. Never use it to retry automatically,
/// remove credentials, or weaken host/TLS verification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FetchFailure {
    SshHostKeyChanged,
    SshHostVerificationFailed,
    SshKeyUnavailable,
    SshKeyUnlockFailed,
    SshAgentFailure,
    SshAuthenticationRejected,
    CredentialsUnavailable,
    AuthenticationRejected,
    SsoRequired,
    AccessDenied,
    RepositoryUnavailable,
    ProxyAuthenticationRequired,
    TlsVerificationFailed,
    NameResolutionFailed,
    ConnectionFailed,
    ConnectionTimedOut,
    ConnectionInterrupted,
    Other,
}

impl FetchFailure {
    /// Stable, payload-free diagnostic identifier.
    pub fn code(self) -> &'static str {
        match self {
            Self::SshHostKeyChanged => "ssh_host_key_changed",
            Self::SshHostVerificationFailed => "ssh_host_verification_failed",
            Self::SshKeyUnavailable => "ssh_key_unavailable",
            Self::SshKeyUnlockFailed => "ssh_key_unlock_failed",
            Self::SshAgentFailure => "ssh_agent_failure",
            Self::SshAuthenticationRejected => "ssh_authentication_rejected",
            Self::CredentialsUnavailable => "credentials_unavailable",
            Self::AuthenticationRejected => "authentication_rejected",
            Self::SsoRequired => "sso_required",
            Self::AccessDenied => "access_denied",
            Self::RepositoryUnavailable => "repository_unavailable",
            Self::ProxyAuthenticationRequired => "proxy_authentication_required",
            Self::TlsVerificationFailed => "tls_verification_failed",
            Self::NameResolutionFailed => "name_resolution_failed",
            Self::ConnectionFailed => "connection_failed",
            Self::ConnectionTimedOut => "connection_timed_out",
            Self::ConnectionInterrupted => "connection_interrupted",
            Self::Other => "unclassified_fetch_failure",
        }
    }

    pub fn summary(self) -> &'static str {
        match self {
            Self::SshHostKeyChanged => "SSH reports that the server's host key changed.",
            Self::SshHostVerificationFailed => "SSH could not verify the server's host key.",
            Self::SshKeyUnavailable => "SSH could not load a configured identity file.",
            Self::SshKeyUnlockFailed => "SSH could not unlock a private key.",
            Self::SshAgentFailure => "The SSH agent could not provide a signature.",
            Self::SshAuthenticationRejected => "The server did not accept SSH key authentication.",
            Self::CredentialsUnavailable => {
                "Git or its credential helper could not obtain credentials without prompting."
            }
            Self::AuthenticationRejected => {
                "The server rejected authentication. The credentials may be incorrect, expired, or revoked."
            }
            Self::SsoRequired => "The server requires organization SSO authorization.",
            Self::AccessDenied => {
                "The server denied access. Account permissions or organization policy may prevent this request."
            }
            Self::RepositoryUnavailable => {
                "The repository was not found or is not accessible to this account."
            }
            Self::ProxyAuthenticationRequired => "The proxy requires authentication.",
            Self::TlsVerificationFailed => "The HTTPS server certificate could not be verified.",
            Self::NameResolutionFailed => "The remote or proxy hostname could not be resolved.",
            Self::ConnectionFailed => {
                "A connection to the remote or proxy could not be established."
            }
            Self::ConnectionTimedOut => "The transport reported a connection timeout.",
            Self::ConnectionInterrupted => {
                "The remote connection ended before the transfer completed."
            }
            Self::Other => {
                "Git failed without a recognized authentication or connection diagnostic."
            }
        }
    }

    pub fn guidance(self) -> &'static str {
        match self {
            Self::SshHostKeyChanged | Self::SshHostVerificationFailed => {
                "Verify the host fingerprint using the provider's official documentation or your administrator before updating known_hosts. Keep strict host checking enabled."
            }
            Self::SshKeyUnavailable => {
                "Check the configured identity filename, format, and file permissions. Use the intended SSH client and key."
            }
            Self::SshKeyUnlockFailed => {
                "Unlock the intended key in your terminal with that SSH client's ssh-add, then retry explicitly. Do not put the passphrase in commands or logs."
            }
            Self::SshAgentFailure => {
                "Check that the intended SSH agent is running and the key is loaded and unlocked; approve any required hardware-key interaction, then retry explicitly."
            }
            Self::SshAuthenticationRejected => {
                "Check the SSH username, selected key, agent, and public-key registration on your account. For GitHub use the git SSH user. This error alone cannot distinguish an unloaded key from a rejected key."
            }
            Self::CredentialsUnavailable => {
                "Complete sign-in using the configured credential helper outside this fetch, then retry explicitly. For GitHub with Git Credential Manager, use git credential-manager github login --browser."
            }
            Self::AuthenticationRejected => {
                "Sign in again through the configured credential helper and verify the account and repository access. This error does not establish whether a token expired or was revoked."
            }
            Self::SsoRequired => {
                "Authorize the selected token or SSH key for the organization through the provider, then retry explicitly."
            }
            Self::AccessDenied | Self::RepositoryUnavailable => {
                "Check the remote URL, selected account, repository permissions, token scope, and any organization SSO requirements."
            }
            Self::ProxyAuthenticationRequired => {
                "Check your proxy configuration and approved proxy sign-in method before retrying."
            }
            Self::TlsVerificationFailed => {
                "Check the system clock, certificate trust, and any corporate proxy certificate with your administrator. Keep TLS verification enabled."
            }
            Self::NameResolutionFailed | Self::ConnectionFailed | Self::ConnectionTimedOut => {
                "Check the network, VPN, proxy, firewall, and remote availability, then retry explicitly. This is not evidence of bad credentials."
            }
            Self::ConnectionInterrupted => {
                "Check the network, proxy, and remote availability. Inspect the operation's final state before retrying explicitly; this is not evidence of bad credentials. For a clone, choose a new destination and inspect any retained workspace."
            }
            Self::Other => {
                "Check the remote and Git/SSH configuration. Review the retained Git output locally if more detail is needed."
            }
        }
    }

    pub(crate) fn classify(stderr: &[u8]) -> Self {
        // Git is run with LC_ALL=C, but third-party helpers may still localize.
        // Inspect stderr only; stdout contains fetch porcelain and is not an
        // authentication diagnostic. Nothing from these bytes is returned/logged.
        let text = String::from_utf8_lossy(stderr).to_ascii_lowercase();
        let has = |needle: &str| text.contains(needle);
        if has("remote host identification has changed") {
            Self::SshHostKeyChanged
        } else if has("host key verification failed") {
            Self::SshHostVerificationFailed
        } else if has("ssl certificate problem")
            || has("server certificate verification failed")
            || has("certificate verify failed")
            || has("cert_e_untrustedroot")
            || has("sec_e_untrusted_root")
        {
            Self::TlsVerificationFailed
        } else if has("could not resolve host:")
            || has("could not resolve proxy:")
            || has("could not resolve hostname ")
        {
            Self::NameResolutionFailed
        } else if has("connection timed out") || has("connection timeout") {
            Self::ConnectionTimedOut
        } else if has("connection refused")
            || has("network is unreachable")
            || has("no route to host")
            || (has("failed to connect to ") && has("port "))
        {
            Self::ConnectionFailed
        } else if has("proxy authentication required")
            || has("requested url returned error: 407")
            || has("connect tunnel failed, response 407")
        {
            Self::ProxyAuthenticationRequired
        } else if has("saml sso") || has("saml single sign-on") {
            Self::SsoRequired
        } else if has("incorrect passphrase supplied to decrypt private key")
            || has("bad passphrase")
        {
            Self::SshKeyUnlockFailed
        } else if has("sign_and_send_pubkey: signing failed") || has("agent refused operation") {
            Self::SshAgentFailure
        } else if text.lines().any(|line| {
            (line.starts_with("no such identity:")
                || line.starts_with("load key ")
                || line.starts_with("warning: identity file "))
                && (line.contains("no such file")
                    || line.contains("not accessible")
                    || line.contains("invalid format")
                    || line.contains("bad permissions")
                    || line.contains("permission denied"))
        }) {
            Self::SshKeyUnavailable
        } else if has("permission denied (publickey") {
            Self::SshAuthenticationRejected
        } else if has("authentication failed")
            || has("invalid username or token")
            || has("invalid username or password")
            || has("requested url returned error: 401")
        {
            Self::AuthenticationRejected
        } else if has("terminal prompts disabled")
            || has("cannot prompt because user interactivity has been disabled")
            || has("cannot prompt because terminal prompts have been disabled")
            || has("could not read username for ")
            || has("could not read password for ")
            || has("unable to get password from user")
        {
            Self::CredentialsUnavailable
        } else if has("requested url returned error: 403") {
            Self::AccessDenied
        } else if has("repository not found")
            || has("does not appear to be a git repository")
            || has("requested url returned error: 404")
            || text.lines().any(|line| {
                line.starts_with("fatal: repository '") && line.ends_with("' not found")
            })
        {
            Self::RepositoryUnavailable
        } else if has("connection reset by peer")
            || has("recv failure: connection was reset")
            || has("connection closed by remote host")
            || has("fetch-pack: unexpected disconnect while reading sideband packet")
            || has("send-pack: unexpected disconnect while reading sideband packet")
            || [
                "rpc failed; curl 18 ",
                "rpc failed; curl 56 ",
                "rpc failed; curl 92 ",
            ]
            .iter()
            .any(|hint| has(hint))
        {
            // Incomplete transport, not a diagnosis of its cause. Keep stronger
            // host/TLS/auth/server errors above; early EOF alone can also mean a
            // corrupt pack or a local failure and must remain unclassified.
            Self::ConnectionInterrupted
        } else {
            Self::Other
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;

    #[test]
    fn recognizes_diagnostics_and_preserves_specific_causes() {
        use FetchFailure::*;
        let cases = [
            (
                "WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!\nHost key verification failed.",
                SshHostKeyChanged,
            ),
            (
                "Host key verification failed.\nfatal: Could not read from remote repository.",
                SshHostVerificationFailed,
            ),
            (
                "Warning: Identity file PRIVATE not accessible: No such file or directory.\ngit@host: Permission denied (publickey).",
                SshKeyUnavailable,
            ),
            (
                "Load key \"PRIVATE\": invalid format\nPermission denied (publickey).",
                SshKeyUnavailable,
            ),
            (
                "Load key \"PRIVATE\": incorrect passphrase supplied to decrypt private key\nPermission denied (publickey).",
                SshKeyUnlockFailed,
            ),
            (
                "sign_and_send_pubkey: signing failed for ED25519 from agent: agent refused operation\nPermission denied (publickey).",
                SshAgentFailure,
            ),
            (
                "git@host: Permission denied (publickey).",
                SshAuthenticationRejected,
            ),
            (
                "fatal: could not read Username for 'https://PRIVATE': terminal prompts disabled",
                CredentialsUnavailable,
            ),
            (
                "fatal: Cannot prompt because user interactivity has been disabled.",
                CredentialsUnavailable,
            ),
            (
                "remote: Invalid username or token.\nfatal: Authentication failed for 'https://PRIVATE'",
                AuthenticationRejected,
            ),
            (
                "fatal: unable to access 'PRIVATE': The requested URL returned error: 401",
                AuthenticationRejected,
            ),
            (
                "remote: The organization has enabled or enforced SAML SSO.\nThe requested URL returned error: 403",
                SsoRequired,
            ),
            (
                "fatal: unable to access 'PRIVATE': The requested URL returned error: 403",
                AccessDenied,
            ),
            (
                "remote: Repository not found.\nfatal: Could not read from remote repository.",
                RepositoryUnavailable,
            ),
            (
                "fatal: repository 'https://PRIVATE' not found",
                RepositoryUnavailable,
            ),
            (
                "fatal: unable to access 'PRIVATE': CONNECT tunnel failed, response 407",
                ProxyAuthenticationRequired,
            ),
            (
                "fatal: unable to access 'PRIVATE': SSL certificate problem: self-signed certificate",
                TlsVerificationFailed,
            ),
            (
                "fatal: unable to access 'PRIVATE': schannel: SEC_E_UNTRUSTED_ROOT",
                TlsVerificationFailed,
            ),
            (
                "ssh: Could not resolve hostname PRIVATE: Name or service not known",
                NameResolutionFailed,
            ),
            (
                "fatal: unable to access 'PRIVATE': Could not resolve proxy: PRIVATE",
                NameResolutionFailed,
            ),
            (
                "ssh: connect to host PRIVATE port 22: Connection refused",
                ConnectionFailed,
            ),
            (
                "fatal: unable to access 'PRIVATE': Failed to connect to PRIVATE port 443 after 200 ms",
                ConnectionFailed,
            ),
            (
                "ssh: connect to host PRIVATE port 22: Connection timed out",
                ConnectionTimedOut,
            ),
        ];
        for (message, expected) in cases {
            assert_eq!(
                FetchFailure::classify(message.as_bytes()),
                expected,
                "{message}"
            );
        }
    }

    #[test]
    fn unknown_and_generic_permission_failures_do_not_guess_authentication() {
        for message in [
            b"fatal: Could not read from remote repository.".as_slice(),
            b"fatal: cannot lock ref: Permission denied",
            b"PRIVATE: 401 403 404 publickey password token expired revoked",
            b"fatal: protocol 'ssh' is not allowed",
            b"unrecognized helper output\xff\0",
            b"fatal: early EOF\nfatal: fetch-pack: invalid index-pack output",
            b"",
        ] {
            assert_eq!(FetchFailure::classify(message), FetchFailure::Other);
        }
    }

    #[test]
    fn interrupted_transfers_are_hints_and_preserve_stronger_errors() {
        for message in [
            "error: RPC failed; curl 56 Recv failure: Connection was reset\nfatal: early EOF",
            "error: RPC failed; curl 18 transfer closed with outstanding read data remaining",
            "error: RPC failed; curl 92 HTTP/2 stream was not closed cleanly",
            "fetch-pack: unexpected disconnect while reading sideband packet\nfatal: early EOF",
            "send-pack: unexpected disconnect while reading sideband packet",
            "Read from remote host PRIVATE: Connection reset by peer",
            "Connection closed by remote host.",
        ] {
            assert_eq!(
                FetchFailure::classify(message.as_bytes()),
                FetchFailure::ConnectionInterrupted
            );
        }
        for (message, expected) in [
            (
                "Authentication failed\nConnection reset by peer",
                FetchFailure::AuthenticationRejected,
            ),
            (
                "SSL certificate problem\nerror: RPC failed; curl 56 receiving data",
                FetchFailure::TlsVerificationFailed,
            ),
            (
                "remote: Repository not found.\nfetch-pack: unexpected disconnect while reading sideband packet",
                FetchFailure::RepositoryUnavailable,
            ),
        ] {
            assert_eq!(FetchFailure::classify(message.as_bytes()), expected);
        }
    }

    #[test]
    fn display_and_codes_never_echo_git_payload_but_raw_error_is_preserved() {
        let stderr = b"fatal: Authentication failed for 'https://PRIVATE:TOKEN@example.invalid/repo'\nAuthorization: Bearer PRIVATE\x1b[31m\n";
        let error = Error::Git {
            operation: "fetch",
            code: Some(128),
            stdout: b"PRIVATE".to_vec(),
            stderr: stderr.to_vec(),
        };
        assert_eq!(
            error.fetch_failure(),
            Some(FetchFailure::AuthenticationRejected)
        );
        let message = error.to_string();
        assert!(message.contains("authentication_rejected") && message.contains("Sign in again"));
        for private in [
            "PRIVATE",
            "TOKEN",
            "example.invalid",
            "Authorization",
            "\x1b",
        ] {
            assert!(!message.contains(private));
        }
        assert!(matches!(error, Error::Git { stderr: ref raw, .. } if raw == stderr));
        let error = Error::Git {
            operation: "commit",
            code: Some(1),
            stdout: Vec::new(),
            stderr: stderr.to_vec(),
        };
        assert_eq!(error.fetch_failure(), None);
        assert!(error.to_string().contains("PRIVATE")); // Other operation contracts unchanged.
        for error in [
            Error::Cancelled,
            Error::TimedOut,
            Error::InvalidInput("PRIVATE".into()),
        ] {
            assert_eq!(error.fetch_failure(), None);
        }
    }
}
