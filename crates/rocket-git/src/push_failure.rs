use crate::{FetchFailure, PushRejection};

/// Advisory diagnostics only. Never use these hints to force a push, retry a
/// remote write automatically, remove credentials, or weaken TLS/SSH checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PushFailure {
    UnpublishedSubmodule,
    StaleLease,
    NonFastForward,
    FetchFirst,
    RemoteRejected,
    RemoteFailure,
    WriteAccessDenied,
    Transport(FetchFailure),
    Other,
}

impl PushFailure {
    pub fn code(self) -> &'static str {
        match self {
            Self::UnpublishedSubmodule => "push_unpublished_submodule",
            Self::StaleLease => "push_stale_lease",
            Self::NonFastForward => "push_non_fast_forward",
            Self::FetchFirst => "push_fetch_first",
            Self::RemoteRejected => "push_remote_rejected",
            Self::RemoteFailure => "push_remote_failure",
            Self::WriteAccessDenied => "push_access_denied",
            Self::Transport(failure) => failure.code(),
            Self::Other => "unclassified_push_failure",
        }
    }

    pub fn summary(self) -> &'static str {
        match self {
            Self::UnpublishedSubmodule => "The parent push references submodule commits that are not published to a known remote.",
            Self::StaleLease => "The remote branch no longer matches the reviewed commit.",
            Self::NonFastForward => {
                "The destination contains history that this push would replace."
            }
            Self::FetchFirst => "The destination has commits that are not available locally.",
            Self::RemoteRejected => "The server refused this push.",
            Self::RemoteFailure => "The server did not confirm this push.",
            Self::WriteAccessDenied => "The server denied write access to this repository.",
            Self::Transport(failure) => failure.summary(),
            Self::Other => {
                "Git could not complete the push and did not provide a recognized rejection or transport diagnostic."
            }
        }
    }

    pub fn guidance(self) -> &'static str {
        match self {
            Self::UnpublishedSubmodule => "Publish the referenced submodule commits first, fetch its remote history, then retry the parent push. Inspect Submodules and retained Git output locally for the affected paths. Rocket did not push submodules automatically. This check uses locally available submodule and remote information; initialize missing submodules before relying on it.",
            Self::StaleLease => {
                "The lease refused this update. Inspect the latest remote commits and prepare a new review; do not retry the old preview or use plain force."
            }
            Self::NonFastForward | Self::FetchFirst => {
                "Fetch the selected remote, inspect the branch differences, and integrate the remote commits before trying another normal push."
            }
            Self::RemoteRejected => {
                "Check repository protection rules, write policy, and server hooks. Read retained Git output locally for details; do not bypass the rejection automatically."
            }
            Self::RemoteFailure => {
                "Check the destination on the remote before retrying. A lost response does not mean the remote stayed unchanged."
            }
            Self::WriteAccessDenied => {
                "Check that the selected account/token has repository write permission and any required organization authorization. Read access alone does not allow pushing."
            }
            Self::Transport(FetchFailure::CredentialsUnavailable) => {
                "Complete sign-in with the configured credential helper outside this push, verify repository write access, then retry explicitly."
            }
            Self::Transport(failure) => failure.guidance(),
            Self::Other => {
                "Check the remote configuration and local pre-push/server hooks. Inspect retained Git output locally and verify the remote destination before retrying an unconfirmed write."
            }
        }
    }

    pub(crate) fn classify(stdout: &[u8], stderr: &[u8]) -> Self {
        // Status records take precedence over arbitrary hook stderr. Classification
        // is a hint; the report separately validates the exact source/destination.
        for line in stdout.split(|b| *b == b'\n') {
            if let Some(rest) = line.strip_prefix(b"!\t")
                && let Some(summary) = rest.splitn(2, |b| *b == b'\t').nth(1)
            {
                return match rejection(summary) {
                    PushRejection::StaleLease => Self::StaleLease,
                    PushRejection::NonFastForward => Self::NonFastForward,
                    PushRejection::FetchFirst => Self::FetchFirst,
                    PushRejection::RemoteRejected => Self::RemoteRejected,
                    PushRejection::RemoteFailure => Self::RemoteFailure,
                    PushRejection::Other => Self::Other,
                };
            }
        }
        let text = String::from_utf8_lossy(stderr).to_ascii_lowercase();
        if text.contains("following submodule paths contain changes that can")
            || text.contains("aborting") && text.contains("not be found on any remote")
        {
            return Self::UnpublishedSubmodule;
        }
        if text.contains("write access to repository not granted")
            || text.lines().any(|line| {
                line.starts_with("error: permission to ") && line.contains(" denied to ")
            })
        {
            return Self::WriteAccessDenied;
        }
        match FetchFailure::classify(stderr) {
            FetchFailure::Other => Self::Other,
            failure => Self::Transport(failure),
        }
    }
}

pub(crate) fn rejection(summary: &[u8]) -> PushRejection {
    if summary == b"[rejected] (stale info)" {
        PushRejection::StaleLease
    } else if summary == b"[rejected] (non-fast-forward)" {
        PushRejection::NonFastForward
    } else if summary == b"[rejected] (fetch first)" {
        PushRejection::FetchFirst
    } else if summary.starts_with(b"[remote rejected]") {
        PushRejection::RemoteRejected
    } else if summary.starts_with(b"[remote failure]") {
        PushRejection::RemoteFailure
    } else {
        PushRejection::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;
    #[test]
    fn push_errors_are_specific_payload_free_and_separate_from_fetch() {
        let cases: &[(&[u8], &[u8], PushFailure)] = &[
            (
                b"!\tPRIVATE:refs/heads/PRIVATE\t[rejected] (stale info)\n",
                b"PRIVATE",
                PushFailure::StaleLease,
            ),
            (
                b"!\trefs/heads/a:refs/heads/b\t[rejected] (non-fast-forward)\n",
                b"PRIVATE",
                PushFailure::NonFastForward,
            ),
            (
                b"!\trefs/heads/a:refs/heads/b\t[rejected] (fetch first)\n",
                b"PRIVATE",
                PushFailure::FetchFirst,
            ),
            (
                b"!\trefs/heads/a:refs/heads/b\t[remote rejected] (PRIVATE)\n",
                b"Authentication failed PRIVATE",
                PushFailure::RemoteRejected,
            ),
            (
                b"!\trefs/heads/a:refs/heads/b\t[remote failure] (PRIVATE)\n",
                b"",
                PushFailure::RemoteFailure,
            ),
            (
                b"",
                b"ERROR: Permission to PRIVATE denied to PRIVATE.",
                PushFailure::WriteAccessDenied,
            ),
            (
                b"",
                b"remote: Write access to repository not granted. PRIVATE",
                PushFailure::WriteAccessDenied,
            ),
            (
                b"",
                b"Authentication failed for https://PRIVATE",
                PushFailure::Transport(FetchFailure::AuthenticationRejected),
            ),
            (
                b"",
                b"Permission denied (publickey). PRIVATE",
                PushFailure::Transport(FetchFailure::SshAuthenticationRejected),
            ),
            (
                b"",
                b"Host key verification failed. PRIVATE",
                PushFailure::Transport(FetchFailure::SshHostVerificationFailed),
            ),
            (
                b"",
                b"could not read Username: terminal prompts disabled PRIVATE",
                PushFailure::Transport(FetchFailure::CredentialsUnavailable),
            ),
            (
                b"PRIVATE",
                b"unrecognized PRIVATE hook failure",
                PushFailure::Other,
            ),
        ];
        for (stdout, stderr, expected) in cases {
            let error = Error::Git {
                operation: "push",
                code: Some(1),
                stdout: stdout.to_vec(),
                stderr: stderr.to_vec(),
            };
            assert_eq!(error.push_failure(), Some(*expected));
            assert!(error.fetch_failure().is_none());
            assert!(!error.to_string().contains("PRIVATE"));
            assert!(error.to_string().contains(expected.code()));
            assert!(!expected.guidance().contains("this fetch"));
            if let Error::Git { stdout: kept, .. } = error {
                assert_eq!(kept, *stdout);
            }
        }
        assert!(Error::Cancelled.push_failure().is_none());
        assert!(Error::TimedOut.push_failure().is_none());
    }
}
