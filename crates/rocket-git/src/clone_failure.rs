use crate::FetchFailure;

/// Fixed clone decisions and advisory transport hints; never contains input data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloneFailure {
    DestinationExists,
    InvalidDestination,
    SourceUnavailable,
    UnsupportedSource,
    WorkspaceUnavailable,
    MissingDefaultBranch,
    VerificationFailed,
    PublicationFailed,
    OpenFailed,
    CommandFailed,
    TransportNotAllowed,
    /// A preview stopped at its size limit; nothing was published.
    SizeLimit,
    Transport(FetchFailure),
}

impl CloneFailure {
    pub(crate) fn classify(stderr: &[u8]) -> Self {
        if stderr.split(|&b| b == b'\n').any(|line| {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            line.starts_with(b"fatal: transport '") && line.ends_with(b"' not allowed")
        }) {
            return Self::TransportNotAllowed;
        }
        match FetchFailure::classify(stderr) {
            FetchFailure::Other => Self::CommandFailed,
            failure => Self::Transport(failure),
        }
    }
    pub fn code(self) -> &'static str {
        match self {
            Self::DestinationExists => "clone_destination_exists",
            Self::InvalidDestination => "clone_invalid_destination",
            Self::SourceUnavailable => "clone_source_unavailable",
            Self::UnsupportedSource => "clone_unsupported_source",
            Self::WorkspaceUnavailable => "clone_workspace_unavailable",
            Self::MissingDefaultBranch => "clone_missing_default_branch",
            Self::VerificationFailed => "clone_verification_failed",
            Self::PublicationFailed => "clone_publication_failed",
            Self::OpenFailed => "clone_open_failed",
            Self::CommandFailed => "clone_command_failed",
            Self::TransportNotAllowed => "clone_transport_not_allowed",
            Self::SizeLimit => "clone_size_limit",
            Self::Transport(failure) => failure.code(),
        }
    }
    pub fn summary(self) -> &'static str {
        match self {
            Self::DestinationExists => "The destination already exists; it was not replaced.",
            Self::InvalidDestination => "The destination is invalid or its parent is unavailable.",
            Self::SourceUnavailable => "The selected local source directory is unavailable.",
            Self::UnsupportedSource => {
                "The clone source is outside the supported local/HTTPS/SSH forms."
            }
            Self::WorkspaceUnavailable => "The temporary clone workspace could not be prepared.",
            Self::MissingDefaultBranch => {
                "The source has branches but its default HEAD could not be checked out."
            }
            Self::VerificationFailed => {
                "The staged clone could not be verified as a complete, self-contained checkout."
            }
            Self::PublicationFailed => "The staged clone could not be moved to the destination.",
            Self::OpenFailed => "The published clone could not be reopened.",
            Self::CommandFailed => "Git did not confirm a completed clone.",
            Self::TransportNotAllowed => {
                "Git refused a transport outside this clone's allowed protocols."
            }
            Self::SizeLimit => "The download passed its size limit and was stopped; nothing was kept.",
            Self::Transport(failure) => failure.summary(),
        }
    }
    pub fn guidance(self) -> &'static str {
        match self {
            Self::DestinationExists => {
                "Choose a new destination. Existing files and even empty directories are never reused or removed."
            }
            Self::InvalidDestination => {
                "Choose a new directory name beneath an existing writable parent. Avoid roots, dot names, and reserved names."
            }
            Self::SourceUnavailable => {
                "Select an existing local Git repository directory. Use the URL source option for HTTPS or SSH."
            }
            Self::UnsupportedSource => {
                "Use a local directory, an https:// URL without embedded credentials, or an ssh:// URL. Use a credential helper or SSH agent for authentication."
            }
            Self::WorkspaceUnavailable => {
                "Check the destination parent's permissions and available disk space. Inspect any reported retained workspace before another attempt."
            }
            Self::MissingDefaultBranch => {
                "Correct the source's default branch before starting a new clone. Inspect the retained clone if you need its fetched objects."
            }
            Self::VerificationFailed => {
                "Inspect the retained workspace and Git configuration. The destination was not published; no recursive cleanup or retry was attempted."
            }
            Self::PublicationFailed => {
                "Inspect the retained workspace, destination, permissions, and other applications. Do not retry or remove paths automatically."
            }
            Self::OpenFailed => {
                "The destination was already published. Inspect it directly; do not clone again or assume rollback."
            }
            Self::CommandFailed => {
                "Inspect the retained workspace and Git configuration. Retained command output is available locally; partial files are not automatically deleted."
            }
            Self::TransportNotAllowed => {
                "Check the source type and Git URL rewrites. Local clones allow file; URL clones allow HTTPS/SSH, further limited by inherited GIT_ALLOW_PROTOCOL. Rocket does not widen that policy."
            }
            Self::SizeLimit => {
                "Clone the repository instead, or preview it again without the limit."
            }
            Self::Transport(FetchFailure::CredentialsUnavailable) => {
                "Sign in with the configured credential helper outside this clone, then start a new request explicitly. Existing saved credentials are not erased by Rocket."
            }
            Self::Transport(FetchFailure::RepositoryUnavailable) => {
                "For a local source, select a Git repository directory. For a URL, check the repository address, selected account, permissions, and organization access requirements."
            }
            Self::Transport(failure) => failure.guidance(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;
    #[test]
    fn clone_errors_preserve_raw_data_without_echoing_it_or_guessing_authentication() {
        for (operation, stderr, expected) in [
            (
                "clone",
                "fatal: Authentication failed PRIVATE_TOKEN",
                CloneFailure::Transport(FetchFailure::AuthenticationRejected),
            ),
            (
                "clone",
                "Permission denied (publickey). PRIVATE_KEY",
                CloneFailure::Transport(FetchFailure::SshAuthenticationRejected),
            ),
            (
                "clone",
                "PRIVATE_UNKNOWN_MESSAGE",
                CloneFailure::CommandFailed,
            ),
            (
                "clone",
                "fetch-pack: unexpected disconnect while reading sideband packet\nPRIVATE_ENDPOINT",
                CloneFailure::Transport(FetchFailure::ConnectionInterrupted),
            ),
            (
                "clone-verify",
                "PRIVATE_PATH",
                CloneFailure::VerificationFailed,
            ),
            ("clone-open", "PRIVATE_PATH", CloneFailure::OpenFailed),
            (
                "clone",
                "fatal: transport 'ext' not allowed\r\nPRIVATE",
                CloneFailure::TransportNotAllowed,
            ),
        ] {
            let error = Error::Git {
                operation,
                code: Some(1),
                stdout: b"PRIVATE_STDOUT".to_vec(),
                stderr: stderr.as_bytes().to_vec(),
            };
            assert_eq!(error.clone_failure(), Some(expected));
            assert!(error.to_string().contains(expected.code()));
            assert!(!error.to_string().contains("PRIVATE"));
            if let Error::Git { stderr: raw, .. } = error {
                assert_eq!(raw, stderr.as_bytes());
            }
        }
        for error in [
            Error::Cancelled,
            Error::TimedOut,
            Error::OutputLimit {
                stream: "stderr",
                limit: 10,
            },
        ] {
            assert_eq!(error.clone_failure(), None);
        }
    }
}
