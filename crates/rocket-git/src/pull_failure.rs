/// Fixed, payload-free pull decisions. These codes do not include repository
/// paths, references, helper diagnostics, or credential material.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PullFailure {
    DetachedHead,
    UnbornHead,
    MissingUpstream,
    UnsupportedUpstream,
    DirtyWorktree,
    OperationInProgress,
    UpstreamNotFetched,
    Diverged,
    StateChanged,
    UpdateFailed,
    VerificationFailed,
}

impl PullFailure {
    pub fn code(self) -> &'static str {
        match self {
            Self::DetachedHead => "pull_detached_head",
            Self::UnbornHead => "pull_unborn_head",
            Self::MissingUpstream => "pull_missing_upstream",
            Self::UnsupportedUpstream => "pull_unsupported_upstream",
            Self::DirtyWorktree => "pull_dirty_worktree",
            Self::OperationInProgress => "pull_operation_in_progress",
            Self::UpstreamNotFetched => "pull_upstream_not_fetched",
            Self::Diverged => "pull_diverged",
            Self::StateChanged => "pull_state_changed",
            Self::UpdateFailed => "pull_update_failed",
            Self::VerificationFailed => "pull_verification_failed",
        }
    }

    pub fn summary(self) -> &'static str {
        match self {
            Self::DetachedHead => "HEAD is detached; no current branch can be pulled.",
            Self::UnbornHead => "The current branch has no commit yet.",
            Self::MissingUpstream => "The current branch has no complete upstream configuration.",
            Self::UnsupportedUpstream => {
                "The upstream configuration is ambiguous or outside the supported branch mappings."
            }
            Self::DirtyWorktree => "Tracked files, the index, or submodules have local changes.",
            Self::OperationInProgress => "Another Git operation is in progress.",
            Self::UpstreamNotFetched => "This fetch did not report the selected upstream branch.",
            Self::Diverged => "The local and fetched histories cannot be fast-forwarded.",
            Self::StateChanged => {
                "The current branch, upstream configuration or fetched tracking tip changed during pull."
            }
            Self::UpdateFailed => "Git did not confirm the local branch update.",
            Self::VerificationFailed => {
                "Git reported success, but the expected branch tip could not be verified."
            }
        }
    }

    pub fn guidance(self) -> &'static str {
        match self {
            Self::DetachedHead => {
                "Switch to a local branch with a configured upstream, then pull again."
            }
            Self::UnbornHead => {
                "Create an initial commit or clone the repository before using this pull operation."
            }
            Self::MissingUpstream => {
                "Configure the current branch's remote and upstream branch explicitly, then retry."
            }
            Self::UnsupportedUpstream => {
                "Select one configured remote and one included refs/heads branch with a direct tracking ref. Local-dot, multiple, excluded, and renamed mappings are unsupported."
            }
            Self::DirtyWorktree => {
                "Commit or stash tracked changes and handle submodule edits first. Rocket does not autostash."
            }
            Self::OperationInProgress => {
                "Finish or explicitly abort the existing merge, rebase, cherry-pick, or revert first."
            }
            Self::UpstreamNotFetched => {
                "Check that the remote branch still exists and is included in the fetch mapping. Rocket will not merge a stale tracking ref."
            }
            Self::Diverged => {
                "Inspect both histories and choose an explicit merge or rebase workflow. Rocket will not reset or rewrite your commits."
            }
            Self::StateChanged => {
                "Inspect the refreshed branch and upstream selection. Fetch and review again before applying a pull."
            }
            Self::UpdateFailed => {
                "Inspect refreshed status, file collisions, Git locks, and hooks. Retained Git output is available locally. Do not retry or remove locks automatically."
            }
            Self::VerificationFailed => {
                "Inspect the refreshed state and other Git clients. The successful command has not been rolled back; do not retry automatically."
            }
        }
    }
}
