//! Local tags: immutable previews, exact ref identity, and checked ref transactions.
use crate::{model::bytes_to_os, parse, runner::args, *};
use std::path::PathBuf;
mod remote;
pub use remote::*;
pub(crate) use remote::{display_destination, protocol_unsupported};

const REF_FORMAT: &str = "--format=%(refname)%00%(objectname)%00%(objecttype)%00%(symref)%00";
/// `REF_FORMAT`, then what the list shows: date, peeled object and its type, and the
/// subject of the commit the tag names (the subject comes last; Git folds it to one line).
const LIST_FORMAT: &str = "--format=%(refname)%00%(objectname)%00%(objecttype)%00%(symref)%00%(creatordate:unix)%00%(*objectname)%00%(*objecttype)%00%(if)%(*objectname)%(then)%(*subject)%(else)%(subject)%(end)";
const MESSAGE_LIMIT: usize = 64 * 1024;
/// Lines that open a signature block in a tag message, as Git recognises them.
const SIGNATURE_STARTS: [&[u8]; 4] = [
    b"-----BEGIN PGP SIGNATURE-----",
    b"-----BEGIN PGP MESSAGE-----",
    b"-----BEGIN SIGNED MESSAGE-----",
    b"-----BEGIN SSH SIGNATURE-----",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagEntry {
    pub name: Vec<u8>,
    /// The tag object itself for annotated tags, never the peeled commit.
    pub id: ObjectId,
    pub object_type: String,
    pub symbolic: bool,
    repository: PathBuf,
}
impl TagEntry {
    pub fn annotated(&self) -> bool {
        self.object_type == "tag"
    }
}
#[derive(Clone, Debug, Default)]
pub struct TagPage {
    pub entries: Vec<TagEntry>,
    /// What the list shows beside each entry, in the same order.
    pub summaries: Vec<TagSummary>,
    pub more: bool,
}
/// The commit (or other object) a listed tag finally names, and when it was made.
/// Display data only; reviews re-read the tag itself.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TagSummary {
    pub target: Option<ObjectId>,
    pub target_type: String,
    /// The first line of the commit's message, as Git folds it.
    pub subject: Vec<u8>,
    /// Tagger date of an annotated tag, else the commit date, in Unix seconds.
    pub date: Option<i64>,
}
/// The order a tag list is read in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TagOrder {
    #[default]
    Name,
    /// Newest first by tagger date (commit date for lightweight tags), then by name.
    Newest,
}
/// The parts of an annotated tag object that the Tags page shows. A signature
/// block is only detected here, never verified.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TagAnnotation {
    pub tagger: Option<Signature>,
    /// The message without its signature block.
    pub message: Vec<u8>,
    /// Whether the message ends in a signature block (OpenPGP, X.509 or SSH).
    pub signed: bool,
}
impl TagAnnotation {
    /// Read a tag object as `git cat-file tag` prints it.
    pub fn parse(object: &[u8]) -> Self {
        let (headers, message) = match object.windows(2).position(|w| w == b"\n\n") {
            Some(end) => (&object[..end], &object[end + 2..]),
            None => (object, &[][..]),
        };
        let tagger = headers
            .split(|b| *b == b'\n')
            .find_map(|line| line.strip_prefix(b"tagger "))
            .and_then(|value| parse::signature(value).ok());
        // Like Git, the block starts at the last line that opens one.
        let mut start = message.len();
        let mut offset = 0;
        for line in message.split_inclusive(|b| *b == b'\n') {
            if SIGNATURE_STARTS.iter().any(|s| line.starts_with(s)) {
                start = offset;
            }
            offset += line.len();
        }
        Self {
            tagger,
            message: message[..start].to_vec(),
            signed: start < message.len(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct TagDetails {
    pub entry: TagEntry,
    pub target: ObjectId,
    pub target_type: String,
    pub commit: Option<Commit>,
    /// Exact outer tag object, including tagger, timestamp, message and any signature.
    /// Displayed as data only; signatures are not verified by this read.
    pub annotation: Option<Vec<u8>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TagAction {
    Create,
    Delete,
}
impl TagAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Create => "tag-create",
            Self::Delete => "tag-delete",
        }
    }
}
#[derive(Clone, Debug)]
pub struct TagPreview {
    action: TagAction,
    details: TagDetails,
    signing: Option<RepositorySigning>,
}
impl TagPreview {
    pub fn signing(&self) -> Option<&RepositorySigning> {
        self.signing.as_ref()
    }
    pub fn action(&self) -> TagAction {
        self.action
    }
    pub fn details(&self) -> &TagDetails {
        &self.details
    }
    pub fn recovery_reference(&self) -> Option<String> {
        (self.action == TagAction::Delete)
            .then(|| format!("refs/rocket/deleted-tags/{}", self.details.entry.id))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TagFailure {
    InvalidName,
    InvalidMessage,
    AlreadyExists,
    StalePreview,
    SymbolicTag,
    IdentityUnavailable,
    TargetUnavailable,
    RecoveryReferenceConflict,
    ReferenceLocked,
    CommandFailed,
    VerificationFailed,
    RemoteChanged,
    RemoteExists,
    RemoteMissing,
    RemoteDestinationChanged,
    RemoteUnsupported,
    RemoteLimit,
    RemoteProtocolRequired,
}
impl TagFailure {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidName => "tag_invalid_name",
            Self::InvalidMessage => "tag_invalid_message",
            Self::AlreadyExists => "tag_exists",
            Self::StalePreview => "tag_stale_preview",
            Self::SymbolicTag => "tag_symbolic_ref",
            Self::IdentityUnavailable => "tag_identity_unavailable",
            Self::TargetUnavailable => "tag_target_unavailable",
            Self::RecoveryReferenceConflict => "tag_recovery_ref_conflict",
            Self::ReferenceLocked => "tag_ref_locked",
            Self::CommandFailed => "tag_command_failed",
            Self::VerificationFailed => "tag_verification_failed",
            Self::RemoteChanged => "remote_tag_changed",
            Self::RemoteExists => "remote_tag_exists",
            Self::RemoteMissing => "remote_tag_missing",
            Self::RemoteDestinationChanged => "remote_tag_destination_changed",
            Self::RemoteUnsupported => "remote_tag_destination_unsupported",
            Self::RemoteLimit => "remote_tag_limit",
            Self::RemoteProtocolRequired => "remote_tag_protocol_required",
        }
    }
    pub fn guidance(self) -> &'static str {
        match self {
            Self::RemoteProtocolRequired => {
                "The server cannot provide the protocol-v2 tag inspection Rocket requires to detect indirect references. Use a server supporting Git protocol v2 and server options; no tag write was started."
            }
            Self::RemoteChanged => {
                "The remote tag no longer matches the reviewed object. Refresh and review again. Rocket will not replace a different tag."
            }
            Self::RemoteExists => {
                "That name is already present on the remote. Inspect its object; publication only creates an absent tag and never replaces an existing one."
            }
            Self::RemoteMissing => {
                "The selected remote tag is absent. Refresh the remote list; no deletion was started."
            }
            Self::RemoteDestinationChanged => {
                "The repository, remote destination or Git configuration changed after review. Refresh and review again before changing a remote tag."
            }
            Self::RemoteUnsupported => {
                "Select one configured remote with exactly one push destination (or one fetch URL). Remote groups, ambiguous URL rewrites and this repository itself are not supported."
            }
            Self::RemoteLimit => {
                "The remote tag list exceeds the supported 10,000 tags, or the local tag list exceeds 2,000. Use Git to inspect this repository; Rocket will not treat a partial list as complete."
            }
            Self::InvalidName => {
                "Enter a literal tag name without spaces, a leading dash, refs/ prefix, HEAD, @, or Git revision syntax."
            }
            Self::InvalidMessage => {
                "An annotation needs 1–65,536 UTF-8 bytes, some non-whitespace text and no NUL. Use a lightweight tag for no annotation."
            }
            Self::AlreadyExists => {
                "That tag already exists. Choose another name; Rocket never replaces existing tags."
            }
            Self::StalePreview => {
                "The selected tag or repository changed. Refresh the tag list and review again; no tag write was started."
            }
            Self::SymbolicTag => {
                "This tag is a symbolic reference. Inspect it with Git; Rocket will not edit or delete an indirect tag."
            }
            Self::IdentityUnavailable => {
                "Git could not determine the tagger identity. Set your repository name/email, then review again."
            }
            Self::TargetUnavailable => {
                "The target does not resolve to an available commit. Choose a commit ID or existing branch and review again."
            }
            Self::RecoveryReferenceConflict => {
                "The tag recovery reference has unexpected content. Inspect it with Git; Rocket will not overwrite it."
            }
            Self::ReferenceLocked => {
                "Git could not lock the tag or recovery reference. Finish other Git writers and check permissions, then refresh. Existing locks are left in place."
            }
            Self::CommandFailed => {
                "Git did not confirm completion. Inspect the observed tag and optional local Git output before retrying. Cancellation does not undo completed writes."
            }
            Self::VerificationFailed => {
                "The observed tag or recovery reference does not match the reviewed result. Refresh and inspect before retrying."
            }
        }
    }
}
fn refused<T>(failure: TagFailure) -> Result<T> {
    Err(Error::Tag(failure))
}
#[derive(Debug)]
#[must_use = "inspect command completion and observed refs independently; cancellation is not rollback"]
pub struct TagWriteReport {
    pub result: Result<WriteOutput>,
    pub observed: Result<Option<TagEntry>>,
    pub verification: Result<()>,
    pub failure: Option<TagFailure>,
    /// Present only when the recovery ref was observed with the expected tag object.
    pub recovery_reference: Option<String>,
}

/// Most tags one batch reviews.
pub const MAX_TAG_BATCH: usize = 200;

/// Several tag changes in the order requested. Applying stops at the first change
/// that is not verified; the rest are `skipped` and unchanged.
#[derive(Debug, Default)]
pub struct TagBatchReport {
    pub results: Vec<(Vec<u8>, Result<TagWriteReport>)>,
    pub skipped: Vec<Vec<u8>>,
}

/// Reviews of several tag changes, in the order requested. A tag refused while
/// reviewing (it changed, is symbolic, already exists remotely, …) is listed in
/// `refused` with the reason instead of a preview.
#[derive(Debug)]
pub struct TagBatchReview<P> {
    pub ready: Vec<P>,
    pub refused: Vec<(Vec<u8>, Error)>,
}
impl<P> TagBatchReview<P> {
    fn new() -> Self {
        Self {
            ready: Vec::new(),
            refused: Vec::new(),
        }
    }
    /// Record one item's review: a refusal belongs to that tag; any other error
    /// (cancellation, a failed Git process) ends the whole review.
    fn record(&mut self, name: &[u8], result: Result<P>) -> Result<()> {
        match result {
            Ok(preview) => self.ready.push(preview),
            Err(error @ (Error::Tag(_) | Error::InvalidInput(_))) => {
                self.refused.push((name.to_vec(), error))
            }
            Err(error) => return Err(error),
        }
        Ok(())
    }
}

impl Repository {
    pub fn preview_signed_tag(
        &self,
        name: &str,
        target: &str,
        message: &str,
        cancel: &CancellationToken,
    ) -> Result<TagPreview> {
        let mut preview = self.preview_create_tag(name, target, Some(message), cancel)?;
        let signing = self.signing(cancel)?;
        if !["ssh", "openpgp"].contains(&signing.format()) {
            return Err(Error::Signing(SigningFailure::UnsupportedFormat));
        }
        preview.signing = Some(signing);
        Ok(preview)
    }
    pub fn tags(&self, limit: usize, cancel: &CancellationToken) -> Result<TagPage> {
        self.tags_ordered(limit, TagOrder::Name, cancel)
    }
    pub fn tags_ordered(
        &self,
        limit: usize,
        order: TagOrder,
        cancel: &CancellationToken,
    ) -> Result<TagPage> {
        self.runner.step("tags", "read_tags", || {
            if !(1..=2000).contains(&limit) {
                return Err(Error::InvalidInput("tag limit must be 1..2000".into()));
            }
            let count = format!("--count={}", limit + 1);
            // The last sort key is the primary one.
            let mut arguments = vec!["for-each-ref", LIST_FORMAT, "--sort=refname"];
            if order == TagOrder::Newest {
                arguments.push("--sort=-creatordate");
            }
            arguments.extend([count.as_str(), "refs/tags/"]);
            let bytes = self.tag_read(&arguments, vec![], cancel)?;
            let (mut entries, mut summaries) =
                parse_listing(&bytes, &self.info().common_dir)?;
            let more = entries.len() > limit;
            entries.truncate(limit);
            summaries.truncate(limit);
            Ok(TagPage {
                entries,
                summaries,
                more,
            })
        })
    }
    /// Look up one exact tag, including tags beyond the bounded list page.
    /// A prefix must never select a descendant tag.
    pub fn find_tag(&self, name: &[u8], cancel: &CancellationToken) -> Result<Option<TagEntry>> {
        self.runner.step("tag-inspect", "inspect_tag", || {
            let reference = self.tag_reference(name, cancel)?;
            self.tag_ref(&reference, cancel)
        })
    }
    pub fn inspect_tag(&self, entry: &TagEntry, cancel: &CancellationToken) -> Result<TagDetails> {
        self.runner.step("tag-inspect", "inspect_tag", || {
            self.check_tag_entry(entry, cancel)?;
            let target = oid(&self.tag_read(
                &[
                    "rev-parse",
                    "--verify",
                    "--end-of-options",
                    &format!("{}^{{}}", entry.id),
                ],
                vec![],
                cancel,
            )?)?;
            let target_type = object_type(parse::line(&self.tag_read(
                &["cat-file", "-t", target.as_str()],
                vec![],
                cancel,
            )?))?;
            let annotation = if entry.annotated() {
                Some(self.tag_read(&["cat-file", "tag", entry.id.as_str()], vec![], cancel)?)
            } else {
                None
            };
            let commit = if target_type == "commit" {
                Some(self.tag_commit(&target, cancel)?)
            } else {
                None
            };
            self.check_tag_entry(entry, cancel)?;
            Ok(TagDetails {
                entry: entry.clone(),
                target,
                target_type,
                commit,
                annotation,
            })
        })
    }
    /// Resolve once. The final write uses the reviewed commit even if HEAD moves.
    /// Annotated tag identity/time/message are pinned in the preview, without signing.
    pub fn preview_create_tag(
        &self,
        name: &str,
        target: &str,
        message: Option<&str>,
        cancel: &CancellationToken,
    ) -> Result<TagPreview> {
        self.runner.step("tag-preview", "review_create_tag", || {
            let reference = self.tag_reference(name.as_bytes(), cancel)?;
            if self.tag_ref(&reference, cancel)?.is_some() {
                return refused(TagFailure::AlreadyExists);
            }
            if message
                .is_some_and(|m| m.trim().is_empty() || m.len() > MESSAGE_LIMIT || m.contains('\0'))
            {
                return refused(TagFailure::InvalidMessage);
            }
            let target = self.resolve_commit(target, cancel).map_err(|e| match e {
                Error::Git { .. } | Error::InvalidInput(_) => {
                    Error::Tag(TagFailure::TargetUnavailable)
                }
                other => other,
            })?;
            let commit = self.tag_commit(&target, cancel)?;
            let annotation = if let Some(message) = message {
                let ident = self
                    .tag_read(&["var", "GIT_COMMITTER_IDENT"], vec![], cancel)
                    .map_err(|e| match e {
                        Error::Git { .. } => Error::Tag(TagFailure::IdentityUnavailable),
                        other => other,
                    })?;
                let ident = parse::line(&ident);
                if ident.is_empty()
                    || ident.contains(&b'\n')
                    || ident.contains(&b'\r')
                    || ident.contains(&0)
                {
                    return refused(TagFailure::IdentityUnavailable);
                }
                let mut bytes =
                    format!("object {target}\ntype commit\ntag {name}\ntagger ").into_bytes();
                bytes.extend_from_slice(ident);
                bytes.extend_from_slice(b"\n\n");
                bytes.extend_from_slice(message.as_bytes());
                if !bytes.ends_with(b"\n") {
                    bytes.push(b'\n');
                }
                Some(bytes)
            } else {
                None
            };
            let id = if let Some(bytes) = &annotation {
                oid(&self.tag_read(
                    &["hash-object", "-t", "tag", "--stdin"],
                    bytes.clone(),
                    cancel,
                )?)?
            } else {
                target.clone()
            };
            Ok(TagPreview {
                action: TagAction::Create,
                signing: None,
                details: TagDetails {
                    entry: TagEntry {
                        name: name.as_bytes().to_vec(),
                        id,
                        object_type: if annotation.is_some() {
                            "tag"
                        } else {
                            "commit"
                        }
                        .into(),
                        symbolic: false,
                        repository: self.info().common_dir.clone(),
                    },
                    target,
                    target_type: "commit".into(),
                    commit: Some(commit),
                    annotation,
                },
            })
        })
    }
    pub fn preview_delete_tag(
        &self,
        entry: &TagEntry,
        cancel: &CancellationToken,
    ) -> Result<TagPreview> {
        self.runner.step("tag-preview", "review_delete_tag", || {
            if entry.symbolic {
                return refused(TagFailure::SymbolicTag);
            }
            Ok(TagPreview {
                action: TagAction::Delete,
                signing: None,
                details: self.inspect_tag(entry, cancel)?,
            })
        })
    }
    /// Review deleting several tags: one [`Repository::preview_delete_tag`] each.
    pub fn preview_delete_tags(
        &self,
        entries: &[TagEntry],
        cancel: &CancellationToken,
    ) -> Result<TagBatchReview<TagPreview>> {
        if entries.is_empty() || entries.len() > MAX_TAG_BATCH {
            return Err(Error::InvalidInput(format!(
                "select 1..{MAX_TAG_BATCH} tags to delete"
            )));
        }
        if (1..entries.len()).any(|i| entries[..i].iter().any(|e| e.name == entries[i].name)) {
            return Err(Error::InvalidInput("a tag is selected twice".into()));
        }
        let mut review = TagBatchReview::new();
        for entry in entries {
            cancel.check()?;
            review.record(&entry.name, self.preview_delete_tag(entry, cancel))?;
        }
        Ok(review)
    }
    /// Apply reviewed tag changes in order with [`Repository::apply_tag`]. The batch
    /// stops at the first change that is not verified (each applied deletion keeps
    /// its recovery reference); later tags, and any after a cancellation, are
    /// reported as skipped.
    pub fn apply_tags(
        &self,
        previews: &[TagPreview],
        cancel: &CancellationToken,
    ) -> Result<TagBatchReport> {
        let mut report = TagBatchReport::default();
        let mut stopped = false;
        for preview in previews {
            let name = preview.details.entry.name.clone();
            if stopped || cancel.is_cancelled() {
                report.skipped.push(name);
                continue;
            }
            let result = self.apply_tag(preview, cancel);
            stopped = !matches!(
                &result,
                Ok(r) if r.result.is_ok() && r.verification.is_ok() && r.failure.is_none()
            );
            report.results.push((name, result));
        }
        Ok(report)
    }
    pub fn apply_tag(
        &self,
        preview: &TagPreview,
        cancel: &CancellationToken,
    ) -> Result<TagWriteReport> {
        let _permit = self.acquire_write(cancel)?;
        self.apply_tag_locked(preview, cancel)
    }
    /// Caller holds the common-directory write gate, including any enclosing workflow checks.
    pub(crate) fn apply_tag_locked(
        &self,
        preview: &TagPreview,
        cancel: &CancellationToken,
    ) -> Result<TagWriteReport> {
        let mut signed_preview;
        let preview = if let Some(settings) = &preview.signing {
            let reference = self.tag_reference(&preview.details.entry.name, cancel)?;
            if self.tag_ref(&reference, cancel)?.is_some() {
                return refused(TagFailure::AlreadyExists);
            }
            let signed = self.sign_tag_object(&preview.details, settings, cancel)?;
            let id = oid(&self.tag_read(
                &["hash-object", "-t", "tag", "--stdin"],
                signed.clone(),
                cancel,
            )?)?;
            signed_preview = preview.clone();
            signed_preview.details.entry.id = id;
            signed_preview.details.annotation = Some(signed);
            &signed_preview
        } else {
            preview
        };
        let entry = &preview.details.entry;
        let label = preview.action.label();
        let reference = self.tag_reference(&entry.name, cancel)?;
        self.runner.step(label, "revalidate_tag", || {
            if entry.repository != self.info().common_dir {
                return refused(TagFailure::StalePreview);
            }
            if preview.action == TagAction::Delete {
                self.check_tag_entry(entry, cancel)
            } else if self.tag_ref(&reference, cancel)?.is_some() {
                refused(TagFailure::AlreadyExists)
            } else {
                Ok(())
            }
        })?;
        let recovery = preview.recovery_reference();
        let mut transaction = b"start\0".to_vec();
        if let Some(recovery) = &recovery {
            let existing = self.tag_ref(recovery.as_bytes(), cancel)?;
            if existing
                .as_ref()
                .is_some_and(|r| r.symbolic || r.id != entry.id)
            {
                return refused(TagFailure::RecoveryReferenceConflict);
            }
            transaction.extend_from_slice(if existing.is_some() {
                b"verify "
            } else {
                b"create "
            });
            transaction.extend_from_slice(recovery.as_bytes());
            transaction.push(0);
            transaction.extend_from_slice(entry.id.as_str().as_bytes());
            transaction.push(0);
        }
        transaction.extend_from_slice(if preview.action == TagAction::Create {
            b"create "
        } else {
            b"delete "
        });
        transaction.extend_from_slice(&reference);
        transaction.push(0);
        transaction.extend_from_slice(entry.id.as_str().as_bytes());
        transaction.push(0);
        transaction.extend_from_slice(b"prepare\0commit\0");
        cancel.check()?;
        let result = self.runner.observe("write_result", label, || {
            if preview.action == TagAction::Create
                && let Some(bytes) = &preview.details.annotation
            {
                self.runner.step(label, "write_tag_object", || {
                    let created = self
                        .runner
                        .write(
                            self.cwd(),
                            &args(&["--no-replace-objects", "mktag"]),
                            bytes.clone(),
                            cancel,
                        )?
                        .success("tag-create")?;
                    if oid(&created)? != entry.id {
                        return refused(TagFailure::VerificationFailed);
                    }
                    Ok(())
                })?;
            }
            self.runner.step(label, "commit_tag_transaction", || {
                let output = self.runner.write(
                    self.cwd(),
                    &args(&["update-ref", "--no-deref", "--stdin", "-z"]),
                    transaction,
                    cancel,
                )?;
                if output.status.success() {
                    Ok(WriteOutput {
                        stdout: output.stdout,
                        stderr: output.stderr,
                    })
                } else {
                    Err(Error::Git {
                        operation: label,
                        code: output.status.code(),
                        stdout: output.stdout,
                        stderr: output.stderr,
                    })
                }
            })
        });
        // Cancellation can arrive after Git committed. Always reconcile with a fresh token.
        let fresh = CancellationToken::new();
        let observed = self
            .runner
            .step(label, "observe_tag", || self.tag_ref(&reference, &fresh));
        let mut recovery_reference = None;
        let verification = self.runner.step(label, "verify_tag", || {
            if let Some(recovery) = recovery {
                let retained = self.tag_ref(recovery.as_bytes(), &fresh)?;
                if !retained.is_some_and(|r| !r.symbolic && r.id == entry.id) {
                    return refused(TagFailure::VerificationFailed);
                }
                recovery_reference = Some(recovery);
            }
            let matches = match (&observed, preview.action) {
                (Ok(Some(actual)), TagAction::Create) => actual == entry,
                (Ok(None), TagAction::Delete) => true,
                _ => false,
            };
            if matches {
                Ok(())
            } else {
                refused(TagFailure::VerificationFailed)
            }
        });
        let failure = match &result {
            Err(Error::Git { stderr, .. })
                if String::from_utf8_lossy(stderr).contains("cannot lock ref") =>
            {
                Some(TagFailure::ReferenceLocked)
            }
            Err(_) => Some(TagFailure::CommandFailed),
            Ok(_) if verification.is_err() => Some(TagFailure::VerificationFailed),
            _ => None,
        };
        let _ = self.runner.observe("tag_outcome", label, || match failure {
            Some(f) => refused::<()>(f),
            None => Ok(()),
        });
        Ok(TagWriteReport {
            result,
            observed,
            verification,
            failure,
            recovery_reference,
        })
    }
    fn tag_commit(&self, id: &ObjectId, cancel: &CancellationToken) -> Result<Commit> {
        let bytes = self.tag_read(
            &["cat-file", "--batch"],
            format!("{id}\n").into_bytes(),
            cancel,
        )?;
        parse::commits(&bytes, std::slice::from_ref(id))?
            .into_iter()
            .next()
            .ok_or(Error::Tag(TagFailure::TargetUnavailable))
    }
    fn tag_read(
        &self,
        arguments: &[&str],
        input: Vec<u8>,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>> {
        let mut raw = args(&["--no-replace-objects"]);
        raw.extend(args(arguments));
        self.runner
            .run(self.cwd(), &raw, input, cancel)?
            .success("tag read")
    }
    fn tag_ref(&self, reference: &[u8], cancel: &CancellationToken) -> Result<Option<TagEntry>> {
        let mut arguments = args(&[
            "--no-replace-objects",
            "for-each-ref",
            REF_FORMAT,
            "--sort=refname",
            "--count=1",
            "--",
        ]);
        arguments.push(bytes_to_os(reference)?);
        let bytes = self
            .runner
            .run(self.cwd(), &arguments, vec![], cancel)?
            .success("tag ref read")?;
        // for-each-ref also matches descendants. A prefix must never select another ref.
        let exact = bytes
            .split(|b| *b == b'\n')
            .find(|row| row.split(|b| *b == 0).next() == Some(reference));
        exact
            .map(|row| parse_ref(row, &self.info().common_dir))
            .transpose()
    }
    fn check_tag_entry(&self, entry: &TagEntry, cancel: &CancellationToken) -> Result<()> {
        let reference = self.tag_reference(&entry.name, cancel)?;
        if entry.repository != self.info().common_dir
            || self.tag_ref(&reference, cancel)?.as_ref() != Some(entry)
        {
            return refused(TagFailure::StalePreview);
        }
        if entry.symbolic {
            return refused(TagFailure::SymbolicTag);
        }
        Ok(())
    }
    fn tag_reference(&self, name: &[u8], cancel: &CancellationToken) -> Result<Vec<u8>> {
        if name.is_empty()
            || name.len() > 4096
            || name.contains(&0)
            || name.starts_with(b"-")
            || name.starts_with(b"refs/")
            || matches!(name, b"HEAD" | b"@")
        {
            return refused(TagFailure::InvalidName);
        }
        let mut reference = b"refs/tags/".to_vec();
        reference.extend_from_slice(name);
        let mut arguments = args(&["check-ref-format"]);
        arguments.push(bytes_to_os(&reference)?);
        let output = self.runner.run(self.cwd(), &arguments, vec![], cancel)?;
        match output.status.code() {
            Some(0) => Ok(reference),
            Some(1) => refused(TagFailure::InvalidName),
            _ => output.success("validate tag name").map(|_| reference),
        }
    }
}
fn oid(bytes: &[u8]) -> Result<ObjectId> {
    ObjectId::parse(
        std::str::from_utf8(parse::line(bytes))
            .map_err(|_| Error::parse("tags", "non-ASCII object ID"))?,
    )
}
fn object_type(bytes: &[u8]) -> Result<String> {
    match bytes {
        b"tag" | b"commit" | b"tree" | b"blob" => Ok(String::from_utf8(bytes.to_vec()).unwrap()),
        _ => Err(Error::parse("tags", "unknown object type")),
    }
}
fn parse_ref(row: &[u8], repository: &std::path::Path) -> Result<TagEntry> {
    let fields: Vec<_> = row.split(|b| *b == 0).collect();
    if fields.len() != 5 || !fields[4].is_empty() {
        return Err(Error::parse("tags", "incomplete ref record"));
    }
    Ok(TagEntry {
        name: fields[0]
            .strip_prefix(b"refs/tags/")
            .unwrap_or(fields[0])
            .to_vec(),
        id: oid(fields[1])?,
        object_type: object_type(fields[2])?,
        symbolic: !fields[3].is_empty(),
        repository: repository.to_owned(),
    })
}
/// Rows in `LIST_FORMAT`: the exact entry, then its display summary. A summary
/// field Git leaves empty or that does not parse stays empty.
fn parse_listing(
    bytes: &[u8],
    repository: &std::path::Path,
) -> Result<(Vec<TagEntry>, Vec<TagSummary>)> {
    let mut entries = Vec::new();
    let mut summaries = Vec::new();
    for row in bytes.split(|b| *b == b'\n').filter(|r| !r.is_empty()) {
        if !row.starts_with(b"refs/tags/") {
            return Err(Error::parse("tags", "unexpected ref namespace"));
        }
        let end = row
            .iter()
            .enumerate()
            .filter(|(_, b)| **b == 0)
            .nth(3)
            .map(|(i, _)| i + 1)
            .ok_or_else(|| Error::parse("tags", "incomplete ref record"))?;
        let entry = parse_ref(&row[..end], repository)?;
        let mut rest = row[end..].splitn(4, |b| *b == 0);
        let mut field = || rest.next().unwrap_or_default();
        let date = std::str::from_utf8(field()).ok().and_then(|d| d.parse().ok());
        let (peeled, peeled_type) = (field(), field());
        let (target, target_type) = if peeled.is_empty() {
            (Some(entry.id.clone()), entry.object_type.clone())
        } else {
            (oid(peeled).ok(), object_type(peeled_type).unwrap_or_default())
        };
        summaries.push(TagSummary {
            target,
            target_type,
            subject: field().to_vec(),
            date,
        });
        entries.push(entry);
    }
    Ok((entries, summaries))
}
