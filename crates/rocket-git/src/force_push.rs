//! A reviewed, single-branch replacement using an explicit expected-object lease.
//! Inspection never fetches or updates refs. A cancelled push may have reached the server.
use crate::{
    model::bytes_to_os,
    parse,
    runner::{args, has_error_diagnostics},
    *,
};
use std::{fmt, path::PathBuf, sync::mpsc::SyncSender, time::Duration};

mod wire;
mod atomic;
pub use atomic::*;
const PREVIEW_LIMIT: usize = 20;

/// Force-push phases are separate from transfer counters. CheckingRemote begins
/// only after the push command and its owned helper processes have finished.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForcePushProgress {
    CheckingLease,
    Pushing,
    CheckingRemote { push_cancelled: bool },
}
impl ForcePushProgress {
    pub fn code(self) -> &'static str {
        match self {
            Self::CheckingLease => "checking_lease",
            Self::Pushing => "pushing",
            Self::CheckingRemote { .. } => "checking_remote",
        }
    }
}
fn progress(sender: Option<&SyncSender<ForcePushProgress>>, value: ForcePushProgress) {
    // UI polling must never hold up Git or lose a mutation result on disconnect.
    if let Some(sender) = sender {
        let _ = sender.try_send(value);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForcePushFailure {
    StalePreview,
    ObservationChanged,
    RemoteChanged,
    RemoteMissing,
    SymbolicBranch,
    DestinationUnsupported,
    HistoryUnavailable,
    HistoryUnsupported,
    NoReplacement,
    VerificationFailed,
    ProtocolRequired,
}
impl ForcePushFailure {
    pub fn code(self) -> &'static str {
        match self {
            Self::StalePreview => "force_push_stale_preview",
            Self::ObservationChanged => "force_push_observation_changed",
            Self::RemoteChanged => "force_push_remote_changed",
            Self::RemoteMissing => "force_push_remote_missing",
            Self::SymbolicBranch => "force_push_symbolic_branch",
            Self::DestinationUnsupported => "force_push_destination_unsupported",
            Self::HistoryUnavailable => "force_push_history_unavailable",
            Self::HistoryUnsupported => "force_push_history_unsupported",
            Self::NoReplacement => "force_push_not_needed",
            Self::VerificationFailed => "force_push_unconfirmed",
            Self::ProtocolRequired => "force_push_protocol_required",
        }
    }
    pub fn guidance(self) -> &'static str {
        match self {
            Self::StalePreview => {
                "The source branch, repository or configuration changed since review. No push started. Inspect the new state and prepare a fresh review."
            }
            Self::ObservationChanged => {
                "Repository configuration changed during the push. Rocket did not follow the changed destination. The original remote may already have been updated; inspect it before retrying."
            }
            Self::RemoteChanged => {
                "The remote branch changed since review. No push started. Inspect the latest remote commits, fetch explicitly if needed, and prepare a fresh review."
            }
            Self::RemoteMissing => {
                "The remote branch does not exist. Use ordinary Push to create it; this review only replaces an existing branch."
            }
            Self::SymbolicBranch => {
                "This branch is a symbolic reference. Select a direct local and remote branch; indirect references cannot be safely replaced here."
            }
            Self::DestinationUnsupported => {
                "Select one configured push destination. Multiple URLs, self-pushes and ambiguous URL rewrites cannot be reviewed safely."
            }
            Self::HistoryUnavailable => {
                "The remote history is not fully available locally. Fetch this branch from the selected push destination, then review again. Rocket did not fetch automatically."
            }
            Self::HistoryUnsupported => {
                "A complete history is required for this review. Deepen the shallow repository or inspect legacy grafts with Git first, then review again."
            }
            Self::NoReplacement => {
                "No remote commits would be removed from this branch. Use ordinary Push for an up-to-date or fast-forward update."
            }
            Self::VerificationFailed => {
                "The requested update is not confirmed. Inspect the freshly observed remote state before any retry. Cancellation or a failed response does not roll back a remote write."
            }
            Self::ProtocolRequired => {
                "The server must support Git protocol v2 for safe inspection of branch aliases. Check the transport/server configuration; no push was started."
            }
        }
    }
}
fn refused<T>(failure: ForcePushFailure) -> Result<T> {
    Err(Error::ForcePush(failure))
}

#[derive(Clone)]
pub struct ForcePushPreview {
    repository: PathBuf,
    git_dir: PathBuf,
    remote: Vec<u8>,
    local_branch: Vec<u8>,
    remote_branch: Vec<u8>,
    destination: Vec<u8>,
    config: Vec<u8>,
    source: ObjectId,
    expected: ObjectId,
    removed_count: u64,
    added_count: u64,
    removed: Vec<Commit>,
    added: Vec<Commit>,
}
// Config and endpoint bytes may contain credentials; debug output is deliberately redacted.
impl fmt::Debug for ForcePushPreview {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ForcePushPreview")
            .field("removed_count", &self.removed_count)
            .field("added_count", &self.added_count)
            .finish_non_exhaustive()
    }
}
impl ForcePushPreview {
    pub fn remote(&self) -> &[u8] {
        &self.remote
    }
    pub fn local_branch(&self) -> &[u8] {
        &self.local_branch
    }
    pub fn remote_branch(&self) -> &[u8] {
        &self.remote_branch
    }
    /// For caller transport policy only. Never persist endpoint bytes in diagnostics.
    pub fn destination(&self) -> &[u8] {
        &self.destination
    }
    pub fn destination_display(&self) -> String {
        crate::tags::display_destination(&self.destination)
    }
    pub fn source(&self) -> &ObjectId {
        &self.source
    }
    pub fn expected(&self) -> &ObjectId {
        &self.expected
    }
    pub fn removed_count(&self) -> u64 {
        self.removed_count
    }
    pub fn added_count(&self) -> u64 {
        self.added_count
    }
    /// First 20 commits, newest first; counts describe the entire reachable difference.
    pub fn removed(&self) -> &[Commit] {
        &self.removed
    }
    pub fn added(&self) -> &[Commit] {
        &self.added
    }
    pub fn can_apply(&self) -> bool {
        self.removed_count > 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForcePushRemoteState {
    Replaced,
    Unchanged,
    Different,
    Missing,
    Unknown,
}
impl ForcePushRemoteState {
    pub fn code(self) -> &'static str {
        match self {
            Self::Replaced => "replaced",
            Self::Unchanged => "unchanged",
            Self::Different => "different",
            Self::Missing => "missing",
            Self::Unknown => "unknown",
        }
    }
}
#[derive(Debug)]
#[must_use = "inspect command, observation and verification; interruption is not rollback"]
pub struct ForcePushReport {
    pub result: Result<WriteOutput>,
    /// Fresh observation after the attempt, even if it failed or was cancelled.
    pub observed: Result<Option<ObjectId>>,
    pub verification: Result<()>,
    pub remote_state: ForcePushRemoteState,
    pub error_diagnostics: bool,
}

impl Repository {
    /// Resolve the effective push endpoint without opening a transport.
    pub fn force_push_destination(
        &self,
        remote: &[u8],
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>> {
        self.reviewed_push_destination(remote, cancel)
            .map_err(|error| match error {
                Error::Tag(_) => Error::ForcePush(ForcePushFailure::DestinationUnsupported),
                other => other,
            })
    }
    pub fn preview_force_push(
        &self,
        remote: &[u8],
        local_branch: &[u8],
        remote_branch: &[u8],
        cancel: &CancellationToken,
    ) -> Result<ForcePushPreview> {
        self.runner
            .step("force-push-preview", "review_force_push", || {
                self.branch_argument(local_branch, cancel)?;
                self.branch_argument(remote_branch, cancel)?;
                self.force_push_history(cancel)?;
                let config = self.force_push_config(cancel)?;
                let destination = self.force_push_destination(remote, cancel)?;
                let source = self.force_push_source(local_branch, cancel)?;
                let reference = [b"refs/heads/".as_slice(), remote_branch].concat();
                let expected =
                    self.runner
                        .step("force-push-preview", "inspect_force_push_remote", || {
                            wire::advertise(
                                &self.runner,
                                self.cwd(),
                                &destination,
                                &reference,
                                cancel,
                            )?
                            .ok_or(Error::ForcePush(ForcePushFailure::RemoteMissing))
                        })?;
                let (added_count, removed_count, added, removed) =
                    self.runner
                        .step("force-push-preview", "compare_force_push_history", || {
                            let object = self
                                .force_push_read(&["cat-file", "-t", expected.as_str()], cancel)?;
                            if parse::line(&object) != b"commit" {
                                return refused(ForcePushFailure::HistoryUnavailable);
                            }
                            let range = format!("{source}...{expected}");
                            let bytes = self.force_push_read(
                                &["rev-list", "--left-right", "--count", &range, "--"],
                                cancel,
                            )?;
                            let counts: Vec<_> = std::str::from_utf8(&bytes)
                                .unwrap_or_default()
                                .split_whitespace()
                                .collect();
                            let invalid =
                                || Error::parse("force-push-preview", "expected two commit counts");
                            if counts.len() != 2 {
                                return Err(invalid());
                            }
                            let added = counts[0].parse::<u64>().map_err(|_| invalid())?;
                            let removed = counts[1].parse::<u64>().map_err(|_| invalid())?;
                            Ok((
                                added,
                                removed,
                                self.force_push_commits(&expected, &source, cancel)?,
                                self.force_push_commits(&source, &expected, cancel)?,
                            ))
                        })?;
                let preview = ForcePushPreview {
                    repository: self.info().common_dir.clone(),
                    git_dir: self.info().git_dir.clone(),
                    remote: remote.to_vec(),
                    local_branch: local_branch.to_vec(),
                    remote_branch: remote_branch.to_vec(),
                    destination,
                    config,
                    source,
                    expected,
                    removed_count,
                    added_count,
                    removed,
                    added,
                };
                self.check_force_push_context(&preview, cancel)?;
                if let Some(log) = &self.runner.log {
                    log.force_push_review(preview.removed_count(), preview.added_count());
                }
                Ok(preview)
            })
    }
    pub fn apply_force_push(
        &self,
        preview: &ForcePushPreview,
        cancel: &CancellationToken,
    ) -> Result<ForcePushReport> {
        self.apply_force_push_with_progress(preview, None, cancel)
    }

    pub fn apply_force_push_with_progress(
        &self,
        preview: &ForcePushPreview,
        updates: Option<&SyncSender<ForcePushProgress>>,
        cancel: &CancellationToken,
    ) -> Result<ForcePushReport> {
        progress(updates, ForcePushProgress::CheckingLease);
        let _permit = self.acquire_write(cancel)?;
        let reference = [b"refs/heads/".as_slice(), preview.remote_branch()].concat();
        self.runner
            .step("force-push", "revalidate_force_push", || {
                self.check_force_push_context(preview, cancel)?;
                if !preview.can_apply() {
                    return refused(ForcePushFailure::NoReplacement);
                }
                let actual = wire::advertise(
                    &self.runner,
                    self.cwd(),
                    preview.destination(),
                    &reference,
                    cancel,
                )?;
                if actual.as_ref() != Some(preview.expected()) {
                    return refused(ForcePushFailure::RemoteChanged);
                }
                // Recheck local state after transport, then use only the pinned object and endpoint.
                self.check_force_push_context(preview, cancel)?;
                cancel.check()
            })?;
        let arguments = wire::push_arguments(preview, &reference)?;
        progress(updates, ForcePushProgress::Pushing);
        let result = self
            .runner
            .step("force-push", "push_with_explicit_lease", || {
                let output = self.runner.write(self.cwd(), &arguments, vec![], cancel)?;
                if !output.status.success() {
                    return Err(Error::Git {
                        operation: "push",
                        code: output.status.code(),
                        stdout: output.stdout,
                        stderr: output.stderr,
                    });
                }
                wire::validate_porcelain(&output.stdout, &reference, preview.source())?;
                Ok(WriteOutput {
                    stdout: output.stdout,
                    stderr: output.stderr,
                })
            });
        let result = match result {
            Err(error @ Error::SecretProtection(_)) => return Err(error),
            other => other,
        };
        let error_diagnostics = match &result {
            Ok(out) => has_error_diagnostics(&out.stderr),
            Err(Error::Git { stderr, .. }) => has_error_diagnostics(stderr),
            _ => false,
        };
        progress(
            updates,
            ForcePushProgress::CheckingRemote {
                push_cancelled: matches!(result, Err(Error::Cancelled)),
            },
        );
        let observed = self
            .runner
            .step("force-push", "observe_force_push_remote", || {
                let fresh = CancellationToken::new();
                let mut probe = self.clone();
                probe.runner.limits.timeout = Duration::from_secs(10);
                // Do not follow a changed config to another endpoint during reconciliation.
                if probe.force_push_config(&fresh)? != preview.config
                    || probe.force_push_destination(preview.remote(), &fresh)?
                        != preview.destination
                {
                    return refused(ForcePushFailure::ObservationChanged);
                }
                wire::advertise(
                    &probe.runner,
                    probe.cwd(),
                    preview.destination(),
                    &reference,
                    &fresh,
                )
            });
        let verification = self
            .runner
            .step("force-push", "verify_force_push_remote", || {
                if matches!(&observed, Ok(Some(id)) if id == preview.source()) {
                    Ok(())
                } else {
                    refused(ForcePushFailure::VerificationFailed)
                }
            });
        let remote_state = match &observed {
            Ok(Some(id)) if id == preview.source() => ForcePushRemoteState::Replaced,
            Ok(Some(id)) if id == preview.expected() => ForcePushRemoteState::Unchanged,
            Ok(Some(_)) => ForcePushRemoteState::Different,
            Ok(None) => ForcePushRemoteState::Missing,
            Err(_) => ForcePushRemoteState::Unknown,
        };
        if let Some(log) = &self.runner.log {
            log.force_push_result(
                remote_state,
                result.is_ok(),
                verification.is_ok(),
                error_diagnostics,
            );
        }
        Ok(ForcePushReport {
            result,
            observed,
            verification,
            remote_state,
            error_diagnostics,
        })
    }
    fn force_push_config(&self, cancel: &CancellationToken) -> Result<Vec<u8>> {
        self.runner
            .run(
                self.cwd(),
                &args(&["config", "--null", "--list"]),
                vec![],
                cancel,
            )?
            .success("read push configuration")
    }
    fn force_push_history(&self, cancel: &CancellationToken) -> Result<()> {
        let shallow = self
            .runner
            .run(
                self.cwd(),
                &args(&["rev-parse", "--is-shallow-repository"]),
                vec![],
                cancel,
            )?
            .success("inspect push history")?;
        if parse::line(&shallow) != b"false"
            || self
                .info()
                .common_dir
                .join("info/grafts")
                .try_exists()
                .map_err(|e| Error::io("inspect push history", e))?
        {
            return refused(ForcePushFailure::HistoryUnsupported);
        }
        Ok(())
    }
    fn force_push_source(&self, branch: &[u8], cancel: &CancellationToken) -> Result<ObjectId> {
        let reference = [b"refs/heads/".as_slice(), branch].concat();
        let mut arguments = args(&["symbolic-ref", "--quiet", "--"]);
        arguments.push(bytes_to_os(&reference)?);
        let output = self.runner.run(self.cwd(), &arguments, vec![], cancel)?;
        match output.status.code() {
            Some(0) => return refused(ForcePushFailure::SymbolicBranch),
            Some(1) => (),
            _ => {
                output.success("inspect push source")?;
            }
        }
        let mut arguments = args(&[
            "--no-replace-objects",
            "rev-parse",
            "--verify",
            "--end-of-options",
        ]);
        arguments.push(bytes_to_os(&[reference.as_slice(), b"^{commit}"].concat())?);
        let output = self.runner.run(self.cwd(), &arguments, vec![], cancel)?;
        if !output.status.success() {
            return refused(ForcePushFailure::StalePreview);
        }
        parse::oid(parse::line(&output.stdout), "force-push-preview")
    }
    fn check_force_push_context(
        &self,
        preview: &ForcePushPreview,
        cancel: &CancellationToken,
    ) -> Result<()> {
        if self.info().common_dir != preview.repository
            || self.info().git_dir != preview.git_dir
            || self.force_push_config(cancel)? != preview.config
            || self.force_push_destination(preview.remote(), cancel)? != preview.destination
            || self.force_push_source(preview.local_branch(), cancel)? != preview.source
        {
            return refused(ForcePushFailure::StalePreview);
        }
        self.force_push_history(cancel)
    }
    fn force_push_read(&self, options: &[&str], cancel: &CancellationToken) -> Result<Vec<u8>> {
        let mut arguments = args(&["--no-replace-objects"]);
        arguments.extend(args(options));
        let output = self.runner.run(self.cwd(), &arguments, vec![], cancel)?;
        if !output.status.success() {
            return refused(ForcePushFailure::HistoryUnavailable);
        }
        Ok(output.stdout)
    }
    fn force_push_commits(
        &self,
        exclude: &ObjectId,
        include: &ObjectId,
        cancel: &CancellationToken,
    ) -> Result<Vec<Commit>> {
        let bytes = self.force_push_read(
            &[
                "rev-list",
                "--topo-order",
                &format!("--max-count={PREVIEW_LIMIT}"),
                &format!("{exclude}..{include}"),
                "--",
            ],
            cancel,
        )?;
        let ids: Vec<_> = bytes
            .split(|b| *b == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| parse::oid(line, "force-push-preview"))
            .collect::<Result<_>>()?;
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let input = ids
            .iter()
            .map(|id| format!("{id}\n"))
            .collect::<String>()
            .into_bytes();
        let output = self
            .runner
            .run(
                self.cwd(),
                &args(&["--no-replace-objects", "cat-file", "--batch"]),
                input,
                cancel,
            )?
            .success("read push commits")?;
        parse::commits(&output, &ids)
    }
}
