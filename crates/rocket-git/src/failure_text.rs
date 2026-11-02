//! Fixed wording for diagnostic failure codes. A log keeps only the code, so this is how
//! a viewer finds the sentences people were shown. Nothing here carries repository data.
use crate::{
    AtomicPushFailure, CloneFailure, FetchFailure, ForcePushFailure, PullFailure, PushFailure,
    TagFailure,
};

/// What a failure means and what to do about it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FailureText {
    /// One sentence on what happened, for the failures that have their own.
    pub summary: Option<&'static str>,
    pub guidance: &'static str,
}

const FETCH: [FetchFailure; 18] = [
    FetchFailure::SshHostKeyChanged,
    FetchFailure::SshHostVerificationFailed,
    FetchFailure::SshKeyUnavailable,
    FetchFailure::SshKeyUnlockFailed,
    FetchFailure::SshAgentFailure,
    FetchFailure::SshAuthenticationRejected,
    FetchFailure::CredentialsUnavailable,
    FetchFailure::AuthenticationRejected,
    FetchFailure::SsoRequired,
    FetchFailure::AccessDenied,
    FetchFailure::RepositoryUnavailable,
    FetchFailure::ProxyAuthenticationRequired,
    FetchFailure::TlsVerificationFailed,
    FetchFailure::NameResolutionFailed,
    FetchFailure::ConnectionFailed,
    FetchFailure::ConnectionTimedOut,
    FetchFailure::ConnectionInterrupted,
    FetchFailure::Other,
];
const CLONE: [CloneFailure; 11] = [
    CloneFailure::DestinationExists,
    CloneFailure::InvalidDestination,
    CloneFailure::SourceUnavailable,
    CloneFailure::UnsupportedSource,
    CloneFailure::WorkspaceUnavailable,
    CloneFailure::MissingDefaultBranch,
    CloneFailure::VerificationFailed,
    CloneFailure::PublicationFailed,
    CloneFailure::OpenFailed,
    CloneFailure::CommandFailed,
    CloneFailure::TransportNotAllowed,
];
const PUSH: [PushFailure; 8] = [
    PushFailure::UnpublishedSubmodule,
    PushFailure::StaleLease,
    PushFailure::NonFastForward,
    PushFailure::FetchFirst,
    PushFailure::RemoteRejected,
    PushFailure::RemoteFailure,
    PushFailure::WriteAccessDenied,
    PushFailure::Other,
];
const PULL: [PullFailure; 11] = [
    PullFailure::DetachedHead,
    PullFailure::UnbornHead,
    PullFailure::MissingUpstream,
    PullFailure::UnsupportedUpstream,
    PullFailure::DirtyWorktree,
    PullFailure::OperationInProgress,
    PullFailure::UpstreamNotFetched,
    PullFailure::Diverged,
    PullFailure::StateChanged,
    PullFailure::UpdateFailed,
    PullFailure::VerificationFailed,
];
const TAG: [TagFailure; 18] = [
    TagFailure::InvalidName,
    TagFailure::InvalidMessage,
    TagFailure::AlreadyExists,
    TagFailure::StalePreview,
    TagFailure::SymbolicTag,
    TagFailure::IdentityUnavailable,
    TagFailure::TargetUnavailable,
    TagFailure::RecoveryReferenceConflict,
    TagFailure::ReferenceLocked,
    TagFailure::CommandFailed,
    TagFailure::VerificationFailed,
    TagFailure::RemoteChanged,
    TagFailure::RemoteExists,
    TagFailure::RemoteMissing,
    TagFailure::RemoteDestinationChanged,
    TagFailure::RemoteUnsupported,
    TagFailure::RemoteLimit,
    TagFailure::RemoteProtocolRequired,
];
const FORCE_PUSH: [ForcePushFailure; 11] = [
    ForcePushFailure::StalePreview,
    ForcePushFailure::ObservationChanged,
    ForcePushFailure::RemoteChanged,
    ForcePushFailure::RemoteMissing,
    ForcePushFailure::SymbolicBranch,
    ForcePushFailure::DestinationUnsupported,
    ForcePushFailure::HistoryUnavailable,
    ForcePushFailure::HistoryUnsupported,
    ForcePushFailure::NoReplacement,
    ForcePushFailure::VerificationFailed,
    ForcePushFailure::ProtocolRequired,
];
const ATOMIC_PUSH: [AtomicPushFailure; 11] = [
    AtomicPushFailure::Selection,
    AtomicPushFailure::StalePreview,
    AtomicPushFailure::RemoteChanged,
    AtomicPushFailure::Blocked,
    AtomicPushFailure::Unsupported,
    AtomicPushFailure::Unconfirmed,
    AtomicPushFailure::ObservationChanged,
    AtomicPushFailure::Destination,
    AtomicPushFailure::History,
    AtomicPushFailure::Symbolic,
    AtomicPushFailure::Protocol,
];

/// The wording for `code`, for the fetch, clone, push, pull and tag families.
/// Any other code has none yet; a viewer shows the code in words instead.
pub fn failure_text(code: &str) -> Option<FailureText> {
    let both = |summary, guidance| {
        Some(FailureText {
            summary: Some(summary),
            guidance,
        })
    };
    let alone = |guidance| {
        Some(FailureText {
            summary: None,
            guidance,
        })
    };
    if let Some(f) = FETCH.iter().find(|f| f.code() == code) {
        return both(f.summary(), f.guidance());
    }
    if let Some(f) = CLONE.iter().find(|f| f.code() == code) {
        return both(f.summary(), f.guidance());
    }
    if let Some(f) = PUSH.iter().find(|f| f.code() == code) {
        return both(f.summary(), f.guidance());
    }
    if let Some(f) = PULL.iter().find(|f| f.code() == code) {
        return both(f.summary(), f.guidance());
    }
    if let Some(f) = TAG.iter().find(|f| f.code() == code) {
        return alone(f.guidance());
    }
    if let Some(f) = FORCE_PUSH.iter().find(|f| f.code() == code) {
        return alone(f.guidance());
    }
    ATOMIC_PUSH
        .iter()
        .find(|f| f.code() == code)
        .and_then(|f| alone(f.guidance()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_finds_its_own_wording_and_an_unknown_code_finds_none() {
        let fetch = failure_text("authentication_rejected").unwrap();
        assert_eq!(fetch.summary, Some(FetchFailure::AuthenticationRejected.summary()));
        assert_eq!(fetch.guidance, FetchFailure::AuthenticationRejected.guidance());
        let push = failure_text("push_non_fast_forward").unwrap();
        assert_eq!(push.summary, Some(PushFailure::NonFastForward.summary()));
        let tag = failure_text("tag_invalid_message").unwrap();
        assert_eq!(tag.summary, None);
        assert_eq!(tag.guidance, TagFailure::InvalidMessage.guidance());
        for code in [
            "clone_destination_exists",
            "pull_diverged",
            "force_push_stale_preview",
            "atomic_push_blocked",
            "remote_tag_limit",
        ] {
            assert!(failure_text(code).is_some(), "{code} has wording");
        }
        assert_eq!(failure_text("not_a_code"), None);
        assert_eq!(failure_text(""), None);
    }

    #[test]
    fn no_code_is_listed_twice_with_different_wording() {
        let codes: Vec<&str> = FETCH
            .iter()
            .map(|f| f.code())
            .chain(CLONE.iter().map(|f| f.code()))
            .chain(PUSH.iter().map(|f| f.code()))
            .chain(PULL.iter().map(|f| f.code()))
            .chain(TAG.iter().map(|f| f.code()))
            .chain(FORCE_PUSH.iter().map(|f| f.code()))
            .chain(ATOMIC_PUSH.iter().map(|f| f.code()))
            .collect();
        let mut unique = codes.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), codes.len());
    }
}
