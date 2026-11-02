use std::{fmt, io};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Io {
        action: &'static str,
        source: io::Error,
    },
    Git {
        operation: &'static str,
        code: Option<i32>,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
    InvalidInput(String),
    InvalidOutput {
        operation: &'static str,
        detail: String,
    },
    Unsupported(String),
    SecretProtection(crate::SecretProtectionFailure),
    ExternalTool(crate::ExternalToolFailure),
    Clone(crate::CloneFailure),
    Pull(crate::PullFailure),
    ForcePush(crate::ForcePushFailure),
    AtomicPush(crate::AtomicPushFailure),
    Ignore(crate::IgnoreFailure),
    CommitAction(crate::CommitFailure),
    Rebase(crate::RebaseFailure),
    Merge(crate::MergeFailure),
    Bisect(crate::BisectFailure),
    Recovery(crate::RecoveryFailure),
    Tag(crate::TagFailure),
    Signing(crate::SigningFailure),
    Worktree(crate::WorktreeFailure),
    Submodule(crate::SubmoduleFailure),
    Lfs(crate::LfsFailure),
    Conflict(crate::ConflictFailure),
    Hosting(crate::HostingFailure),
    Forge(crate::ForgeFailure),
    Ssh(crate::SshFailure),
    FileEdit(crate::FileEditFailure),
    Patch(crate::PatchFailure),
    PatchTarget {
        failure: crate::PatchFailure,
        path: crate::RepoPath,
    },
    GitFlow(crate::GitFlowFailure),
    BranchReview(crate::BranchReviewFailure),
    /// `detail` is the CLI's own message, for local display only; never logged.
    CommitDraft {
        provider: crate::DraftProvider,
        failure: crate::DraftFailure,
        detail: Option<String>,
    },
    /// Another Rocket process owns this repository (including linked worktrees).
    RepositoryBusy,
    WorktreeRequired,
    Cancelled,
    TimedOut,
    OutputLimit {
        stream: &'static str,
        limit: usize,
    },
    WorkerPanicked,
    StatePoisoned(&'static str),
}

impl Error {
    pub fn lfs_push_failure(&self) -> Option<crate::LfsPushFailure> {
        match self {
            Self::Git {
                operation: "lfs-push",
                stdout,
                stderr,
                ..
            } => Some(crate::LfsPushFailure::classify(stdout, stderr)),
            _ => None,
        }
    }
    pub fn lfs_lock_failure(&self) -> Option<crate::LfsLockFailure> {
        match self {
            Self::Git {
                operation: "lfs-locks" | "lfs-lock" | "lfs-unlock" | "lfs-force-unlock",
                stdout,
                stderr,
                ..
            } => Some(crate::LfsLockFailure::classify(stdout, stderr)),
            _ => None,
        }
    }
    pub fn clone_failure(&self) -> Option<crate::CloneFailure> {
        use crate::CloneFailure as C;
        match self {
            Self::Clone(failure) => Some(*failure),
            Self::Git {
                operation: "clone",
                stderr,
                ..
            } => Some(C::classify(stderr)),
            Self::Git {
                operation: "clone-verify",
                ..
            } => Some(C::VerificationFailed),
            Self::Git {
                operation: "clone-open",
                ..
            } => Some(C::OpenFailed),
            Self::Io {
                action: "prepare clone workspace",
                ..
            } => Some(C::WorkspaceUnavailable),
            Self::Io {
                action: "publish clone",
                ..
            } => Some(C::PublicationFailed),
            Self::Io {
                action: "clone-verify",
                ..
            } => Some(C::VerificationFailed),
            Self::Io {
                action: "clone-open",
                ..
            } => Some(C::OpenFailed),
            _ => None,
        }
    }
    /// Exact pull preflight decisions, or generic guidance for a failed branch
    /// update. Fetch transport errors retain their existing fetch classification.
    pub fn pull_failure(&self) -> Option<crate::PullFailure> {
        match self {
            Self::Pull(failure) => Some(*failure),
            Self::Git {
                operation: "pull-update",
                ..
            } => Some(crate::PullFailure::UpdateFailed),
            _ => None,
        }
    }

    /// Best-effort, payload-free guidance for a failed Git fetch. The original
    /// exit code/stdout/stderr remain available in `Error::Git` for local inspection.
    /// Other errors, including timeout/cancellation, are not authentication failures.
    pub fn fetch_failure(&self) -> Option<crate::FetchFailure> {
        match self {
            Self::Git {
                operation:
                    "fetch" | "submodule-fetch" | "submodule-init" | "lfs-fetch" | "remote-tags"
                    | "force-push-preview" | "remote-check",
                stderr,
                ..
            } => Some(crate::FetchFailure::classify(stderr)),
            _ => None,
        }
    }

    /// Payload-free push guidance from Git status/diagnostics. Cancellation and
    /// timeout remain separate; inspect PushReport::remote_outcome before retrying.
    pub fn push_failure(&self) -> Option<crate::PushFailure> {
        match self {
            Self::Git {
                operation: "push",
                stdout,
                stderr,
                ..
            } => Some(crate::PushFailure::classify(stdout, stderr)),
            _ => None,
        }
    }

    pub(crate) fn io(action: &'static str, source: io::Error) -> Self {
        Self::Io { action, source }
    }

    pub(crate) fn parse(operation: &'static str, detail: impl Into<String>) -> Self {
        Self::InvalidOutput {
            operation,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(failure) = self.clone_failure() {
            return write!(
                f,
                "Clone [{}]: {}\nNext: {}",
                failure.code(),
                failure.summary(),
                failure.guidance()
            );
        }
        match self {
            Self::Io { action, source } => write!(f, "{action}: {source}"),
            Self::Git {
                operation,
                code,
                stdout,
                stderr,
            } => {
                if let Some(failure) = self.lfs_lock_failure() {
                    return write!(
                        f,
                        "LFS locking [{}]: {}",
                        failure.code(),
                        failure.guidance()
                    );
                }
                if let Some(failure) = self.lfs_push_failure() {
                    write!(
                        f,
                        "LFS upload failed ({code:?}) [{}]: {}",
                        failure.code(),
                        failure.guidance()
                    )
                } else if let Some(failure) = self.fetch_failure() {
                    write!(
                        f,
                        "Git {operation} failed ({code:?}) [{}]: {}\nNext: {}",
                        failure.code(),
                        failure.summary(),
                        failure.guidance()
                    )
                } else if let Some(failure) = self.push_failure() {
                    write!(
                        f,
                        "Git push failed ({code:?}) [{}]: {}\nNext: {}",
                        failure.code(),
                        failure.summary(),
                        failure.guidance()
                    )
                } else if let Some(failure) = self.pull_failure() {
                    write!(
                        f,
                        "Git pull branch update failed ({code:?}) [{}]: {}\nNext: {}",
                        failure.code(),
                        failure.summary(),
                        failure.guidance()
                    )
                } else {
                    write!(
                        f,
                        "Git {operation} failed ({code:?}): {}",
                        String::from_utf8_lossy(if stderr.is_empty() { stdout } else { stderr })
                            .trim_end()
                    )
                }
            }
            Self::InvalidInput(s) => write!(f, "invalid input: {s}"),
            Self::InvalidOutput { operation, detail } => {
                write!(f, "invalid Git {operation} output: {detail}")
            }
            Self::Unsupported(s) => write!(f, "unsupported: {s}"),
            Self::SecretProtection(failure) => write!(f, "{failure}"),
            Self::ExternalTool(failure) => {
                write!(
                    f,
                    "External tool [{}]: {}",
                    failure.code(),
                    failure.guidance()
                )?;
                if let crate::ExternalToolFailure::Exit(code) = failure {
                    if let Some(code) = code {
                        write!(f, " Exit code: {code}.")?;
                    } else {
                        write!(f, " No exit code was available.")?;
                    }
                }
                Ok(())
            }
            Self::Rebase(failure) => {
                write!(f, "Rebase [{}]: {}", failure.code(), failure.guidance())
            }
            Self::Merge(failure) => {
                write!(f, "Merge [{}]: {}", failure.code(), failure.guidance())
            }
            Self::Bisect(failure) => {
                write!(f, "Bisect [{}]: {}", failure.code(), failure.guidance())
            }
            Self::Recovery(failure) => {
                write!(f, "Recovery [{}]: {}", failure.code(), failure.guidance())
            }
            Self::Ignore(failure) => write!(f, "Ignore rules [{}]: {}", failure.code(), failure.guidance()),
            Self::AtomicPush(failure) => write!(f, "Atomic push [{}]: {}", failure.code(), failure.guidance()),
            Self::ForcePush(failure) => write!(
                f,
                "Force-with-lease [{}]: {}",
                failure.code(),
                failure.guidance()
            ),
            Self::Tag(failure) => write!(f, "Tag [{}]: {}", failure.code(), failure.guidance()),
            Self::Signing(failure) => {
                write!(f, "Signing [{}]: {}", failure.code(), failure.guidance())
            }
            Self::Worktree(failure) => {
                write!(f, "Worktree [{}]: {}", failure.code(), failure.guidance())
            }
            Self::Submodule(failure) => {
                write!(f, "Submodule [{}]: {}", failure.code(), failure.guidance())
            }
            Self::Lfs(failure) => write!(f, "Git LFS [{}]: {}", failure.code(), failure.guidance()),
            Self::CommitAction(failure) => write!(
                f,
                "Commit operation [{}]: {}",
                failure.code(),
                failure.guidance()
            ),
            Self::FileEdit(failure) => write!(
                f,
                "File operation [{}]: {}",
                failure.code(),
                failure.guidance()
            ),
            Self::Patch(failure) => write!(f, "Patch [{}]: {}", failure.code(), failure.guidance()),
            Self::GitFlow(failure) => {
                write!(f, "Git Flow [{}]: {}", failure.code(), failure.guidance())
            }
            Self::BranchReview(failure) => write!(f, "Branch review [{}]: {}", failure.code(), failure.guidance()),
            Self::CommitDraft {
                provider,
                failure,
                detail,
            } => {
                let name = provider.label();
                write!(
                    f,
                    "{name} [{}]: {}",
                    failure.code(),
                    failure.guidance(*provider)
                )?;
                if let Some(detail) = detail {
                    write!(f, "\n{name} said: {detail}")?;
                }
                Ok(())
            }
            Self::PatchTarget { failure, path } => write!(
                f,
                "Patch [{}] · {}: {}",
                failure.code(),
                path.display_lossy(),
                failure.guidance()
            ),
            Self::Conflict(failure) => write!(
                f,
                "Conflict editor [{}]: {}",
                failure.code(),
                failure.guidance()
            ),
            Self::Forge(failure) => write!(f, "{}: {}", failure.code(), failure.guidance()),
            Self::Ssh(failure) => write!(f, "{}: {}", failure.code(), failure.guidance()),
            Self::Hosting(failure) => {
                write!(f, "GitHub [{}]: {}", failure.code(), failure.guidance())
            }
            Self::Clone(_) => unreachable!("clone failures are formatted above"),
            Self::Pull(failure) => write!(
                f,
                "Pull [{}]: {}\nNext: {}",
                failure.code(),
                failure.summary(),
                failure.guidance()
            ),
            Self::RepositoryBusy => write!(
                f,
                "Repository busy: another Rocket window or rocket-cli is writing to this repository or a linked worktree. Wait for it to finish, then refresh and retry. No write was started."
            ),
            Self::WorktreeRequired => write!(f, "this operation requires a working tree"),
            Self::Cancelled => write!(f, "operation cancelled"),
            Self::TimedOut => write!(f, "Git command timed out"),
            Self::OutputLimit { stream, limit } => write!(f, "Git {stream} exceeded {limit} bytes"),
            Self::WorkerPanicked => write!(f, "background worker panicked"),
            Self::StatePoisoned(state) => write!(f, "{state} was poisoned by a panic"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
