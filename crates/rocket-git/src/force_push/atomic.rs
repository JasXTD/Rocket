//! One reviewed transaction containing only normal branch updates. Never retries sequentially.
use super::*;
use std::collections::BTreeSet;

pub const MAX_ATOMIC_BRANCHES: usize = 32;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtomicPushSelection {
    pub local_branch: Vec<u8>,
    pub remote_branch: Vec<u8>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtomicPushKind {
    Create,
    FastForward,
    UpToDate,
    NonFastForward,
    FetchFirst,
}
impl AtomicPushKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::FastForward => "fast_forward",
            Self::UpToDate => "up_to_date",
            Self::NonFastForward => "non_fast_forward",
            Self::FetchFirst => "fetch_first",
        }
    }
    pub fn allowed(self) -> bool {
        matches!(self, Self::Create | Self::FastForward | Self::UpToDate)
    }
}
#[derive(Clone, Debug)]
pub struct AtomicPushEntry {
    pub selection: AtomicPushSelection,
    pub source: ObjectId,
    pub expected: Option<ObjectId>,
    pub kind: AtomicPushKind,
}
#[derive(Clone)]
pub struct AtomicPushPreview {
    repository: PathBuf,
    git_dir: PathBuf,
    remote: Vec<u8>,
    destination: Vec<u8>,
    config: Vec<u8>,
    entries: Vec<AtomicPushEntry>,
}
impl fmt::Debug for AtomicPushPreview {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AtomicPushPreview")
            .field("entries", &self.entries.len())
            .finish_non_exhaustive()
    }
}
impl AtomicPushPreview {
    pub fn remote(&self) -> &[u8] {
        &self.remote
    }
    /// Transport policy only; endpoint bytes can contain credentials.
    pub fn destination(&self) -> &[u8] {
        &self.destination
    }
    pub fn destination_display(&self) -> String {
        crate::tags::display_destination(&self.destination)
    }
    pub fn entries(&self) -> &[AtomicPushEntry] {
        &self.entries
    }
    pub fn can_apply(&self) -> bool {
        !self.entries.is_empty() && self.entries.iter().all(|e| e.kind.allowed())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtomicPushFailure {
    Selection,
    StalePreview,
    RemoteChanged,
    Blocked,
    Unsupported,
    Unconfirmed,
    ObservationChanged,
    Destination,
    History,
    Symbolic,
    Protocol,
}
impl AtomicPushFailure {
    pub fn code(self) -> &'static str {
        match self {
            Self::Selection => "atomic_push_selection",
            Self::StalePreview => "atomic_push_stale_preview",
            Self::RemoteChanged => "atomic_push_remote_changed",
            Self::Blocked => "atomic_push_blocked",
            Self::Unsupported => "atomic_push_unsupported",
            Self::Unconfirmed => "atomic_push_unconfirmed",
            Self::ObservationChanged => "atomic_push_observation_changed",
            Self::Destination => "atomic_push_destination",
            Self::History => "atomic_push_history",
            Self::Symbolic => "atomic_push_symbolic",
            Self::Protocol => "atomic_push_protocol",
        }
    }
    pub fn guidance(self) -> &'static str {
        match self {
            Self::Selection => {
                "Select 1 to 32 direct local branches and distinct remote branch names on one configured remote."
            }
            Self::StalePreview => {
                "The selected source branches or push configuration changed. No push started. Review again."
            }
            Self::RemoteChanged => {
                "A selected remote branch changed after review. No push started. Inspect the new tips and review again."
            }
            Self::Blocked => {
                "At least one branch needs a fetch or integration first. Nothing was pushed. Atomic push never replaces remote history."
            }
            Self::Unsupported => {
                "The server does not support atomic push. No branches were published by this attempt. There is no sequential fallback."
            }
            Self::Unconfirmed => {
                "The requested tips are not all confirmed. Inspect each observed remote result before retrying; cancellation is not rollback."
            }
            Self::ObservationChanged => {
                "Push configuration changed during the operation. Rocket did not follow the new destination; inspect the original remote before retrying."
            }
            Self::Destination => {
                "Choose exactly one configured push destination. Multiple URLs, self-pushes and ambiguous rewrites cannot be reviewed safely."
            }
            Self::History => {
                "Complete local history is required to prove these updates are fast-forwards. Deepen shallow history and inspect legacy grafts first."
            }
            Self::Symbolic => {
                "Select direct branches. A symbolic local or remote branch cannot be included in an atomic review."
            }
            Self::Protocol => {
                "The server must support protocol v2 to inspect branch aliases safely. No push was started."
            }
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtomicPushRemoteState {
    Published,
    Unchanged,
    Different,
    Unknown,
}
impl AtomicPushRemoteState {
    pub fn code(self) -> &'static str {
        match self {
            Self::Published => "published",
            Self::Unchanged => "unchanged",
            Self::Different => "different",
            Self::Unknown => "unknown",
        }
    }
}
#[derive(Debug)]
#[must_use = "inspect command and observation: a cancelled push may have been accepted"]
pub struct AtomicPushReport {
    pub result: Result<WriteOutput>,
    pub observed: Result<Vec<Option<ObjectId>>>,
    pub verification: Result<()>,
    pub remote_state: AtomicPushRemoteState,
    pub error_diagnostics: bool,
}
fn refusal<T>(f: AtomicPushFailure) -> Result<T> {
    Err(Error::AtomicPush(f))
}
fn map_error(e: Error) -> Error {
    match e {
        Error::ForcePush(f) => Error::AtomicPush(match f {
            ForcePushFailure::SymbolicBranch => AtomicPushFailure::Symbolic,
            ForcePushFailure::ProtocolRequired => AtomicPushFailure::Protocol,
            ForcePushFailure::HistoryUnsupported => AtomicPushFailure::History,
            ForcePushFailure::DestinationUnsupported => AtomicPushFailure::Destination,
            _ => AtomicPushFailure::StalePreview,
        }),
        other => other,
    }
}
fn refs(entries: &[AtomicPushEntry]) -> Vec<Vec<u8>> {
    entries
        .iter()
        .map(|e| [b"refs/heads/".as_slice(), &e.selection.remote_branch].concat())
        .collect()
}
impl Repository {
    pub fn preview_atomic_push(
        &self,
        remote: &[u8],
        selection: &[AtomicPushSelection],
        cancel: &CancellationToken,
    ) -> Result<AtomicPushPreview> {
        self.runner
            .step("atomic-push-preview", "review_atomic_push", || {
                cancel.check()?;
                if selection.is_empty() || selection.len() > MAX_ATOMIC_BRANCHES {
                    return refusal(AtomicPushFailure::Selection);
                }
                let mut names = BTreeSet::new();
                for s in selection {
                    self.branch_argument(&s.local_branch, cancel)?;
                    self.branch_argument(&s.remote_branch, cancel)?;
                    if !names.insert(&s.remote_branch) {
                        return refusal(AtomicPushFailure::Selection);
                    }
                }
                self.force_push_history(cancel).map_err(map_error)?;
                let config = self.force_push_config(cancel)?;
                let destination = self
                    .force_push_destination(remote, cancel)
                    .map_err(map_error)?;
                let mut entries = Vec::new();
                for s in selection {
                    entries.push(AtomicPushEntry {
                        selection: s.clone(),
                        source: self
                            .force_push_source(&s.local_branch, cancel)
                            .map_err(map_error)?,
                        expected: None,
                        kind: AtomicPushKind::Create,
                    })
                }
                let references = refs(&entries);
                let expected = wire::advertise_many(
                    &self.runner,
                    self.cwd(),
                    &destination,
                    &references,
                    cancel,
                )
                .map_err(map_error)?;
                for (e, old) in entries.iter_mut().zip(expected) {
                    e.expected = old;
                    if let Some(old) = &e.expected {
                        e.kind = if *old == e.source {
                            AtomicPushKind::UpToDate
                        } else {
                            let object = self.runner.run(
                                self.cwd(),
                                &args(&["--no-replace-objects", "cat-file", "-t", old.as_str()]),
                                vec![],
                                cancel,
                            )?;
                            if !object.status.success() {
                                AtomicPushKind::FetchFirst
                            } else if parse::line(&object.stdout) != b"commit" {
                                return refusal(AtomicPushFailure::History);
                            } else {
                                let output = self.runner.run(
                                    self.cwd(),
                                    &args(&[
                                        "--no-replace-objects",
                                        "merge-base",
                                        "--is-ancestor",
                                        old.as_str(),
                                        e.source.as_str(),
                                    ]),
                                    vec![],
                                    cancel,
                                )?;
                                match output.status.code() {
                                    Some(0) => AtomicPushKind::FastForward,
                                    Some(1) => AtomicPushKind::NonFastForward,
                                    _ => {
                                        output.success("atomic-push-preview")?;
                                        unreachable!()
                                    }
                                }
                            }
                        };
                    }
                }
                let p = AtomicPushPreview {
                    repository: self.info().common_dir.clone(),
                    git_dir: self.info().git_dir.clone(),
                    remote: remote.to_vec(),
                    destination,
                    config,
                    entries,
                };
                self.check_atomic_context(&p, cancel)?;
                Ok(p)
            })
    }
    fn check_atomic_context(&self, p: &AtomicPushPreview, c: &CancellationToken) -> Result<()> {
        if self.info().common_dir != p.repository
            || self.info().git_dir != p.git_dir
            || self.force_push_config(c)? != p.config
            || self
                .force_push_destination(p.remote(), c)
                .map_err(map_error)?
                != p.destination
        {
            return refusal(AtomicPushFailure::StalePreview);
        }
        for e in &p.entries {
            if self
                .force_push_source(&e.selection.local_branch, c)
                .map_err(map_error)?
                != e.source
            {
                return refusal(AtomicPushFailure::StalePreview);
            }
        }
        self.force_push_history(c).map_err(map_error)
    }
    pub fn apply_atomic_push(
        &self,
        p: &AtomicPushPreview,
        c: &CancellationToken,
    ) -> Result<AtomicPushReport> {
        self.apply_atomic_push_with_progress(p, None, c)
    }
    pub fn apply_atomic_push_with_progress(
        &self,
        p: &AtomicPushPreview,
        updates: Option<&SyncSender<ForcePushProgress>>,
        c: &CancellationToken,
    ) -> Result<AtomicPushReport> {
        progress(updates, ForcePushProgress::CheckingLease);
        let _permit = self.acquire_write(c)?;
        let references = refs(&p.entries);
        self.runner
            .step("atomic-push", "revalidate_atomic_push", || {
                self.check_atomic_context(p, c)?;
                if !p.can_apply() {
                    return refusal(AtomicPushFailure::Blocked);
                }
                let actual =
                    wire::advertise_many(&self.runner, self.cwd(), p.destination(), &references, c)
                        .map_err(map_error)?;
                if actual
                    != p.entries
                        .iter()
                        .map(|e| e.expected.clone())
                        .collect::<Vec<_>>()
                {
                    return refusal(AtomicPushFailure::RemoteChanged);
                }
                self.check_atomic_context(p, c)?;
                c.check()
            })?;
        let arguments = push_arguments(p)?;
        progress(updates, ForcePushProgress::Pushing);
        let result = self
            .runner
            .step("atomic-push", "publish_atomic_branches", || {
                let output = self.runner.write(self.cwd(), &arguments, vec![], c)?;
                if !output.status.success() {
                    if String::from_utf8_lossy(&output.stderr)
                        .to_ascii_lowercase()
                        .contains("does not support --atomic push")
                    {
                        return refusal(AtomicPushFailure::Unsupported);
                    }
                    return Err(Error::Git {
                        operation: "push",
                        code: output.status.code(),
                        stdout: output.stdout,
                        stderr: output.stderr,
                    });
                }
                validate_porcelain(&output.stdout, p)?;
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
            .step("atomic-push", "observe_atomic_remote", || {
                let fresh = CancellationToken::new();
                let mut probe = self.clone();
                probe.runner.limits.timeout = Duration::from_secs(10);
                if probe.force_push_config(&fresh)? != p.config
                    || probe
                        .force_push_destination(p.remote(), &fresh)
                        .map_err(map_error)?
                        != p.destination
                {
                    return refusal(AtomicPushFailure::ObservationChanged);
                }
                wire::advertise_many(
                    &probe.runner,
                    probe.cwd(),
                    p.destination(),
                    &references,
                    &fresh,
                )
                .map_err(map_error)
            });
        let remote_state = match &observed {
            Ok(ids)
                if ids
                    .iter()
                    .zip(&p.entries)
                    .all(|(id, e)| id.as_ref() == Some(&e.source)) =>
            {
                AtomicPushRemoteState::Published
            }
            Ok(ids) if ids.iter().zip(&p.entries).all(|(id, e)| *id == e.expected) => {
                AtomicPushRemoteState::Unchanged
            }
            Ok(_) => AtomicPushRemoteState::Different,
            Err(_) => AtomicPushRemoteState::Unknown,
        };
        let verification = self.runner.step("atomic-push", "verify_atomic_remote", || {
            if remote_state == AtomicPushRemoteState::Published {
                Ok(())
            } else {
                refusal(AtomicPushFailure::Unconfirmed)
            }
        });
        if let Some(log) = &self.runner.log {
            log.atomic_push_result(
                remote_state,
                p.entries.len(),
                result.is_ok(),
                verification.is_ok(),
            );
        }
        Ok(AtomicPushReport {
            result,
            observed,
            verification,
            remote_state,
            error_diagnostics,
        })
    }
}
fn push_arguments(p: &AtomicPushPreview) -> Result<Vec<std::ffi::OsString>> {
    let mut a = args(&[
        "--no-replace-objects",
        "-c",
        "push.autoSetupRemote=false",
        "-c",
        "push.negotiate=false",
        "-c",
        "push.pushOption=",
        "push",
        "--porcelain",
        "--atomic",
        "--no-force",
        "--no-force-with-lease",
        "--no-force-if-includes",
        "--no-follow-tags",
        "--no-prune",
        "--recurse-submodules=check",
        "--no-mirror",
    ]);
    // Exact leases pin the reviewed old tips; ancestry was separately proved. A lease
    // is never used to authorize a non-fast-forward, and the source is a pinned OID.
    for e in &p.entries {
        a.push(bytes_to_os(
            &[
                b"--force-with-lease=refs/heads/".as_slice(),
                &e.selection.remote_branch,
                b":",
                e.expected
                    .as_ref()
                    .map_or(b"".as_slice(), |id| id.as_str().as_bytes()),
            ]
            .concat(),
        )?)
    }
    a.push("--".into());
    a.push(bytes_to_os(p.destination())?);
    for e in &p.entries {
        a.push(bytes_to_os(
            &[
                e.source.as_str().as_bytes(),
                b":refs/heads/",
                &e.selection.remote_branch,
            ]
            .concat(),
        )?)
    }
    Ok(a)
}
fn validate_porcelain(bytes: &[u8], p: &AtomicPushPreview) -> Result<()> {
    let mut found = BTreeSet::new();
    for row in bytes
        .split(|b| *b == b'\n')
        .filter(|r| r.get(1) == Some(&b'\t'))
    {
        let parts: Vec<_> = row.split(|b| *b == b'\t').collect();
        if parts.len() != 3 || !matches!(parts[0], b" " | b"*" | b"=") {
            return refusal(AtomicPushFailure::Unconfirmed);
        }
        let Some(index) = p.entries.iter().position(|e| {
            parts[1]
                == [
                    e.source.as_str().as_bytes(),
                    b":refs/heads/",
                    &e.selection.remote_branch,
                ]
                .concat()
        }) else {
            return refusal(AtomicPushFailure::Unconfirmed);
        };
        if !found.insert(index) {
            return refusal(AtomicPushFailure::Unconfirmed);
        }
    }
    if found.len() != p.entries.len() {
        return refusal(AtomicPushFailure::Unconfirmed);
    }
    Ok(())
}
