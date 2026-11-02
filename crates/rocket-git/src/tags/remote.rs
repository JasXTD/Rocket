//! Advertise one push destination, pin a review, and push one exact tag with an
//! explicit expected-old-object lease. No checkout, fetch or local ref update.
use super::*;
use crate::runner::{Runner, has_error_diagnostics};
use std::{
    collections::{BTreeMap, HashMap},
    ffi::OsString,
    fmt,
    path::Path,
    time::Duration,
};

const TAG_LIMIT: usize = 10_000;
/// How many commits of remote tags one read names; the rest show none.
const COMMIT_NAME_LIMIT: usize = 2_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteTagEntry {
    pub name: Vec<u8>,
    pub remote_id: Option<ObjectId>,
    pub peeled: Option<ObjectId>,
    /// Indirect server refs are inspect-only: receive-pack can dereference them.
    pub symbolic_target: Option<Vec<u8>>,
    pub local: Option<TagEntry>,
    /// What the local tag names. Display only; reviews read the tag again.
    pub summary: Option<TagSummary>,
    /// The commit the remote's tag names, when this repository has it. Display only.
    pub remote_commit: Option<RemoteTagCommit>,
}
/// A commit a remote tag names, as this repository knows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteTagCommit {
    /// The first line of the commit's message.
    pub subject: Vec<u8>,
    /// The commit date, in Unix seconds.
    pub date: i64,
}
impl RemoteTagEntry {
    pub fn relation(&self) -> &'static str {
        if self.symbolic_target.is_some() {
            return "symbolic";
        }
        match (&self.remote_id, &self.local) {
            (Some(id), Some(local)) if *id == local.id => "published",
            (Some(_), Some(_)) => "different",
            (Some(_), None) => "remote_only",
            (None, _) => "local_only",
        }
    }
}

#[derive(Clone)]
pub struct RemoteTagSnapshot {
    pub remote: Vec<u8>,
    pub entries: Vec<RemoteTagEntry>,
    destination: Vec<u8>,
    config: Vec<u8>,
    repository: PathBuf,
}
impl fmt::Debug for RemoteTagSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RemoteTagSnapshot")
            .field("entries", &self.entries.len())
            .finish_non_exhaustive()
    }
}
impl RemoteTagSnapshot {
    /// Display only; credentials and query/fragment fields are omitted.
    pub fn destination_display(&self) -> String {
        display_destination(&self.destination)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteTagAction {
    Publish,
    Delete,
}
impl RemoteTagAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Publish => "remote-tag-publish",
            Self::Delete => "remote-tag-delete",
        }
    }
}

#[derive(Clone, Debug)]
pub struct RemoteTagPreview {
    snapshot: RemoteTagSnapshot,
    entry: RemoteTagEntry,
    action: RemoteTagAction,
}
impl RemoteTagPreview {
    pub fn action(&self) -> RemoteTagAction {
        self.action
    }
    pub fn entry(&self) -> &RemoteTagEntry {
        &self.entry
    }
    pub fn remote(&self) -> &[u8] {
        &self.snapshot.remote
    }
    pub fn destination_display(&self) -> String {
        self.snapshot.destination_display()
    }
    /// Endpoint bytes for caller transport policy; never put them in diagnostics.
    pub fn destination(&self) -> &[u8] {
        &self.snapshot.destination
    }
    pub fn expected_result(&self) -> Option<&ObjectId> {
        match self.action {
            RemoteTagAction::Publish => self.entry.local.as_ref().map(|e| &e.id),
            RemoteTagAction::Delete => None,
        }
    }
}

#[derive(Debug)]
pub struct RemoteTagWriteReport {
    pub result: Result<WriteOutput>,
    /// A bounded fresh read even after interruption; failure means unknown state.
    pub observed: Result<Option<ObjectId>>,
    pub verification: Result<()>,
    pub error_diagnostics: bool,
}

/// Several remote tag changes in the order requested; applying stops at the first
/// that is not verified, and the rest are `skipped` (not sent).
#[derive(Debug, Default)]
pub struct RemoteTagBatchReport {
    pub results: Vec<(Vec<u8>, Result<RemoteTagWriteReport>)>,
    pub skipped: Vec<Vec<u8>>,
}

impl Repository {
    /// Resolve the configured push destination without opening a transport.
    /// Used by local automation policy before allowing a network control.
    pub fn remote_tag_destination(
        &self,
        remote: &[u8],
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>> {
        self.reviewed_push_destination(remote, cancel)
    }
    /// Read advertised refs from the configured *push* destination. Remote-only
    /// annotations are not downloaded. All local refs/index/working files stay put.
    pub fn remote_tags(
        &self,
        remote: &[u8],
        cancel: &CancellationToken,
    ) -> Result<RemoteTagSnapshot> {
        self.runner.step("remote-tags", "read_remote_tags", || {
            let config = self.tag_config(cancel)?;
            let destination = self.reviewed_push_destination(remote, cancel)?;
            let advertised = advertise(&self.runner, self.cwd(), &destination, cancel)?;
            let local = self.tags(2000, cancel)?;
            if local.more {
                return refused(TagFailure::RemoteLimit);
            }
            let mut entries = advertised;
            for (tag, summary) in local.entries.into_iter().zip(local.summaries) {
                let name = tag.name.clone();
                let entry = entries
                    .entry(name.clone())
                    .or_insert_with(|| RemoteTagEntry {
                        name,
                        remote_id: None,
                        peeled: None,
                        symbolic_target: None,
                        local: None,
                        summary: None,
                        remote_commit: None,
                    });
                entry.local = Some(tag);
                entry.summary = Some(summary);
            }
            // Names are a convenience: only a cancelled read stops for them.
            match self.remote_commits(&entries, cancel) {
                Ok(commits) => {
                    for entry in entries.values_mut() {
                        let id = entry.peeled.as_ref().or(entry.remote_id.as_ref());
                        entry.remote_commit = id.and_then(|id| commits.get(id).cloned());
                    }
                }
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(_) => {}
            }
            let snapshot = RemoteTagSnapshot {
                remote: remote.to_vec(),
                entries: entries.into_values().collect(),
                destination,
                config,
                repository: self.info().common_dir.clone(),
            };
            self.check_tag_destination(&snapshot, cancel)?;
            Ok(snapshot)
        })
    }

    /// The commits of tags that differ or are only on the remote, among the objects
    /// this repository has. Nothing is fetched.
    fn remote_commits(
        &self,
        entries: &BTreeMap<Vec<u8>, RemoteTagEntry>,
        cancel: &CancellationToken,
    ) -> Result<HashMap<ObjectId, RemoteTagCommit>> {
        let wanted: Vec<&ObjectId> = entries
            .values()
            .filter(|e| !matches!(e.relation(), "published" | "local_only"))
            .filter_map(|e| e.peeled.as_ref().or(e.remote_id.as_ref()))
            .take(COMMIT_NAME_LIMIT)
            .collect();
        if wanted.is_empty() {
            return Ok(HashMap::new());
        }
        let list = |ids: &[&ObjectId]| {
            let mut all = Vec::with_capacity(ids.len() * 41);
            for id in ids {
                all.extend_from_slice(id.as_str().as_bytes());
                all.push(b'\n');
            }
            all
        };
        // Types first, so a tag on a large blob reads nothing.
        let types = self.tag_read(&["cat-file", "--batch-check"], list(&wanted), cancel)?;
        let commits: Vec<&ObjectId> = types
            .split(|b| *b == b'\n')
            .zip(&wanted)
            .filter(|(line, id)| line.starts_with(format!("{} commit ", id.as_str()).as_bytes()))
            .map(|(_, id)| *id)
            .collect();
        if commits.is_empty() {
            return Ok(HashMap::new());
        }
        let bytes = self.tag_read(&["cat-file", "--batch"], list(&commits), cancel)?;
        let ids: Vec<ObjectId> = commits.into_iter().cloned().collect();
        Ok(parse::commits(&bytes, &ids)?
            .into_iter()
            .map(|commit| {
                let subject = commit
                    .message
                    .split(|b| *b == b'\n')
                    .find(|line| line.iter().any(|b| !b.is_ascii_whitespace()))
                    .unwrap_or_default()
                    .to_vec();
                let date = commit.committer.timestamp;
                (commit.id, RemoteTagCommit { subject, date })
            })
            .collect())
    }

    pub fn preview_remote_tag(
        &self,
        snapshot: &RemoteTagSnapshot,
        name: &[u8],
        action: RemoteTagAction,
        cancel: &CancellationToken,
    ) -> Result<RemoteTagPreview> {
        self.runner
            .step("remote-tag-preview", "review_remote_tag", || {
                self.tag_reference(name, cancel)?;
                self.check_tag_destination(snapshot, cancel)?;
                let entry = self.remote_tag_candidate(snapshot, name, action, cancel)?;
                let current = advertise(&self.runner, self.cwd(), &snapshot.destination, cancel)?;
                let preview = pin_remote_tag(snapshot, entry, action, &current)?;
                self.check_tag_destination(snapshot, cancel)?;
                Ok(preview)
            })
    }

    /// Review publishing several local tags to the snapshot's remote. Each tag gets
    /// the checks of [`Repository::preview_remote_tag`]; the remote is read once
    /// for all of them.
    pub fn preview_publish_tags(
        &self,
        snapshot: &RemoteTagSnapshot,
        names: &[Vec<u8>],
        cancel: &CancellationToken,
    ) -> Result<TagBatchReview<RemoteTagPreview>> {
        if names.is_empty() || names.len() > MAX_TAG_BATCH {
            return Err(Error::InvalidInput(format!(
                "select 1..{MAX_TAG_BATCH} tags to publish"
            )));
        }
        if (1..names.len()).any(|i| names[..i].contains(&names[i])) {
            return Err(Error::InvalidInput("a tag is selected twice".into()));
        }
        let action = RemoteTagAction::Publish;
        self.runner
            .step("remote-tag-preview", "review_remote_tag", || {
                self.check_tag_destination(snapshot, cancel)?;
                let mut checked = TagBatchReview::new();
                for name in names {
                    cancel.check()?;
                    let candidate = self
                        .tag_reference(name, cancel)
                        .and_then(|_| self.remote_tag_candidate(snapshot, name, action, cancel));
                    checked.record(name, candidate)?;
                }
                let mut review = TagBatchReview::new();
                review.refused = checked.refused;
                if !checked.ready.is_empty() {
                    let current =
                        advertise(&self.runner, self.cwd(), &snapshot.destination, cancel)?;
                    for entry in checked.ready {
                        let name = entry.name.clone();
                        review.record(&name, pin_remote_tag(snapshot, entry, action, &current))?;
                    }
                }
                self.check_tag_destination(snapshot, cancel)?;
                Ok(review)
            })
    }

    /// The snapshot entry for one remote tag change, after the checks that need
    /// no network: it is direct, and the change fits it (a published tag must be
    /// absent remotely and still match locally; a deleted one must be present).
    fn remote_tag_candidate(
        &self,
        snapshot: &RemoteTagSnapshot,
        name: &[u8],
        action: RemoteTagAction,
        cancel: &CancellationToken,
    ) -> Result<RemoteTagEntry> {
        let entry = snapshot
            .entries
            .iter()
            .find(|e| e.name == name)
            .ok_or(Error::Tag(TagFailure::StalePreview))?
            .clone();
        if entry.symbolic_target.is_some() {
            return refused(TagFailure::SymbolicTag);
        }
        match action {
            RemoteTagAction::Publish => {
                if entry.remote_id.is_some() {
                    return refused(TagFailure::RemoteExists);
                }
                self.check_tag_entry(
                    entry
                        .local
                        .as_ref()
                        .ok_or(Error::Tag(TagFailure::TargetUnavailable))?,
                    cancel,
                )?;
            }
            RemoteTagAction::Delete if entry.remote_id.is_none() => {
                return refused(TagFailure::RemoteMissing);
            }
            RemoteTagAction::Delete => (),
        }
        Ok(entry)
    }

    /// Apply reviewed remote tag changes in order with
    /// [`Repository::apply_remote_tag`], stopping at the first that is not verified
    /// (a failure, a rejection or an unknown remote state). Later tags, and any after
    /// a cancellation, are reported as skipped and were not sent.
    pub fn apply_remote_tags(
        &self,
        previews: &[RemoteTagPreview],
        cancel: &CancellationToken,
    ) -> Result<RemoteTagBatchReport> {
        let mut report = RemoteTagBatchReport::default();
        let mut stopped = false;
        for preview in previews {
            let name = preview.entry.name.clone();
            if stopped || cancel.is_cancelled() {
                report.skipped.push(name);
                continue;
            }
            let result = self.apply_remote_tag(preview, cancel);
            stopped = !matches!(
                &result,
                Ok(r) if r.result.is_ok() && r.observed.is_ok() && r.verification.is_ok()
            );
            report.results.push((name, result));
        }
        Ok(report)
    }

    pub fn apply_remote_tag(
        &self,
        preview: &RemoteTagPreview,
        cancel: &CancellationToken,
    ) -> Result<RemoteTagWriteReport> {
        let _permit = self.acquire_write(cancel)?;
        let op = preview.action.label();
        let reference = self.runner.step(op, "revalidate_remote_tag", || {
            self.check_tag_destination(&preview.snapshot, cancel)?;
            let reference = self.tag_reference(&preview.entry.name, cancel)?;
            if let RemoteTagAction::Publish = preview.action {
                self.check_tag_entry(
                    preview
                        .entry
                        .local
                        .as_ref()
                        .ok_or(Error::Tag(TagFailure::TargetUnavailable))?,
                    cancel,
                )?;
            }
            let current = advertise(
                &self.runner,
                self.cwd(),
                &preview.snapshot.destination,
                cancel,
            )?;
            reject_indirect(current.get(&preview.entry.name))?;
            if current
                .get(&preview.entry.name)
                .and_then(|e| e.remote_id.as_ref())
                != preview.entry.remote_id.as_ref()
            {
                return refused(TagFailure::RemoteChanged);
            }
            self.check_tag_destination(&preview.snapshot, cancel)?;
            Ok(reference)
        })?;
        cancel.check()?;
        let arguments = push_arguments(preview, &reference)?;
        let result = self.runner.step(op, "push_selected_tag", || {
            let output = self.runner.write(self.cwd(), &arguments, vec![], cancel)?;
            if !output.status.success() {
                return Err(Error::Git {
                    operation: "push",
                    code: output.status.code(),
                    stdout: output.stdout,
                    stderr: output.stderr,
                });
            }
            // Require exactly the one requested ref in porcelain; don't infer
            // success just from an empty or unrecognized helper response.
            validate_porcelain(&output.stdout, &reference, preview.action)?;
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
        let observed = self.runner.step(op, "observe_remote_tag", || {
            let fresh = CancellationToken::new();
            self.check_tag_destination(&preview.snapshot, &fresh)?;
            let mut runner = self.runner.clone();
            runner.limits.timeout = Duration::from_secs(10);
            let refs = advertise(&runner, self.cwd(), &preview.snapshot.destination, &fresh)?;
            reject_indirect(refs.get(&preview.entry.name))?;
            Ok(refs
                .get(&preview.entry.name)
                .and_then(|e| e.remote_id.clone()))
        });
        let verification = self.runner.step(op, "verify_remote_tag", || {
            if matches!(&observed, Ok(id) if id.as_ref() == preview.expected_result()) {
                Ok(())
            } else {
                refused(TagFailure::VerificationFailed)
            }
        });
        Ok(RemoteTagWriteReport {
            result,
            observed,
            verification,
            error_diagnostics,
        })
    }

    fn tag_config(&self, cancel: &CancellationToken) -> Result<Vec<u8>> {
        self.runner
            .run(
                self.cwd(),
                &args(&["config", "--null", "--list"]),
                vec![],
                cancel,
            )?
            .success("read remote tag configuration")
    }
    fn check_tag_destination(
        &self,
        snapshot: &RemoteTagSnapshot,
        cancel: &CancellationToken,
    ) -> Result<()> {
        if snapshot.repository != self.info().common_dir
            || snapshot.config != self.tag_config(cancel)?
            || snapshot.destination != self.reviewed_push_destination(&snapshot.remote, cancel)?
        {
            return refused(TagFailure::RemoteDestinationChanged);
        }
        Ok(())
    }
    pub(crate) fn reviewed_push_destination(
        &self,
        name: &[u8],
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>> {
        self.validate_fetch_selection(name, cancel)?;
        self.ensure_no_remote_group(name, cancel)?;
        let remote = self
            .remotes(cancel)?
            .into_iter()
            .find(|r| r.name == name)
            .ok_or(Error::Tag(TagFailure::RemoteUnsupported))?;
        let urls = if remote.push_urls.is_empty() {
            &remote.urls
        } else {
            &remote.push_urls
        };
        if urls.len() != 1 || urls[0].is_empty() {
            return refused(TagFailure::RemoteUnsupported);
        }
        let mut arguments = args(&["remote", "get-url", "--push", "--all", "--"]);
        arguments.push(bytes_to_os(name)?);
        let output = self
            .runner
            .run(self.cwd(), &arguments, vec![], cancel)?
            .success("resolve remote tag destination")?;
        let mut destination = parse::line(&output).to_vec();
        if destination.is_empty()
            || destination.starts_with(b"-")
            || destination.iter().any(|b| matches!(b, 0 | b'\r' | b'\n'))
        {
            return refused(TagFailure::RemoteUnsupported);
        }
        // Anonymous literal destinations avoid configured mirror/refspec/tracking
        // side effects. Resolve local paths and reject any shared object database.
        if let Some(local) = local_destination(&destination)? {
            let path = if local.is_absolute() {
                local
            } else {
                self.cwd().join(local)
            };
            let path = std::fs::canonicalize(&path)
                .map_err(|_| Error::Tag(TagFailure::RemoteUnsupported))?;
            let out = self
                .runner
                .run(
                    &path,
                    &args(&["rev-parse", "--path-format=absolute", "--git-common-dir"]),
                    vec![],
                    cancel,
                )?
                .success("inspect local tag destination")?;
            let common = PathBuf::from(bytes_to_os(parse::line(&out))?);
            let common = std::fs::canonicalize(common)
                .map_err(|_| Error::Tag(TagFailure::RemoteUnsupported))?;
            let own = std::fs::canonicalize(&self.info().common_dir)
                .map_err(|_| Error::Tag(TagFailure::RemoteUnsupported))?;
            if common == own {
                return refused(TagFailure::RemoteUnsupported);
            }
            // Keep forward slashes on Windows; Git recognizes drive-letter paths.
            destination = path.to_string_lossy().replace('\\', "/").into_bytes();
            if destination.starts_with(b"//?/UNC/") {
                return refused(TagFailure::RemoteUnsupported);
            }
            if destination.starts_with(b"//?/") {
                destination.drain(..4);
            }
        }
        // An explicit pushurl can bypass pushInsteadOf when resolved by name,
        // while a later anonymous push applies it. Refuse this second rewrite
        // and any configured remote whose name aliases the literal destination.
        let alias = [b"remote.".as_slice(), &destination, b"."].concat();
        for item in self
            .tag_config(cancel)?
            .split(|b| *b == 0)
            .filter(|b| !b.is_empty())
        {
            let mut fields = item.splitn(2, |b| *b == b'\n');
            let key = fields.next().unwrap_or_default();
            let value = fields.next().unwrap_or_default();
            if key.starts_with(&alias)
                || (key.starts_with(b"url.")
                    && key.ends_with(b".pushinsteadof")
                    && destination.starts_with(value))
            {
                return refused(TagFailure::RemoteUnsupported);
            }
        }
        // Reject a second rewrite or name alias so inspection and push address
        // the same resolved endpoint. Detect configuration edits again on apply.
        let mut probe = args(&["ls-remote", "--get-url", "--"]);
        probe.push(bytes_to_os(&destination)?);
        let expanded = self
            .runner
            .run(self.cwd(), &probe, vec![], cancel)?
            .success("validate remote tag endpoint")?;
        if parse::line(&expanded) != destination {
            return refused(TagFailure::RemoteUnsupported);
        }
        Ok(destination)
    }
}

fn local_destination(url: &[u8]) -> Result<Option<PathBuf>> {
    let text = std::str::from_utf8(url).map_err(|_| Error::Tag(TagFailure::RemoteUnsupported))?;
    if let Some(path) = text.strip_prefix("file://") {
        let path = path
            .strip_prefix("localhost/")
            .map(|p| format!("/{p}"))
            .unwrap_or_else(|| path.to_owned());
        // Reject encoded/authority forms until decoded path equivalence can be
        // established. Ordinary local paths (including Unicode) work directly.
        if path.contains('%') || !path.starts_with('/') {
            return refused(TagFailure::RemoteUnsupported);
        }
        #[cfg(windows)]
        let path = if path.as_bytes().get(2) == Some(&b':') {
            &path[1..]
        } else {
            &path
        };
        return Ok(Some(PathBuf::from(path)));
    }
    if text.contains("://")
        || (text.contains(':')
            && !(text.as_bytes().get(1) == Some(&b':') && text.as_bytes()[0].is_ascii_alphabetic()))
    {
        return Ok(None);
    }
    Ok(Some(PathBuf::from(text)))
}

fn advertise(
    runner: &Runner,
    cwd: &Path,
    destination: &[u8],
    cancel: &CancellationToken,
) -> Result<BTreeMap<Vec<u8>, RemoteTagEntry>> {
    // v0/v1 hide non-HEAD symrefs. An explicit empty server option requires v2
    // (Git refuses a silent downgrade), where ls-refs exposes tag symref targets.
    let mut arguments = args(&[
        "-c",
        "protocol.version=2",
        "ls-remote",
        "--quiet",
        "--symref",
        "--server-option=",
        "--tags",
        "--",
    ]);
    arguments.push(bytes_to_os(destination)?);
    let output = runner.run(cwd, &arguments, vec![], cancel)?;
    if !output.status.success() && protocol_unsupported(&output.stderr) {
        return refused(TagFailure::RemoteProtocolRequired);
    }
    let bytes = output.success("remote-tags")?;
    parse_advertisement(&bytes)
}
/// Pin a checked candidate to the remote's current advertisement: the remote tag
/// must still be direct and exactly as the snapshot saw it.
fn pin_remote_tag(
    snapshot: &RemoteTagSnapshot,
    entry: RemoteTagEntry,
    action: RemoteTagAction,
    current: &BTreeMap<Vec<u8>, RemoteTagEntry>,
) -> Result<RemoteTagPreview> {
    reject_indirect(current.get(&entry.name))?;
    if current.get(&entry.name).and_then(|e| e.remote_id.as_ref()) != entry.remote_id.as_ref() {
        return refused(TagFailure::RemoteChanged);
    }
    Ok(RemoteTagPreview {
        snapshot: snapshot.clone(),
        entry,
        action,
    })
}
fn reject_indirect(entry: Option<&RemoteTagEntry>) -> Result<()> {
    if entry.is_some_and(|e| e.symbolic_target.is_some()) {
        return refused(TagFailure::SymbolicTag);
    }
    Ok(())
}
pub(crate) fn protocol_unsupported(stderr: &[u8]) -> bool {
    let text = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    text.contains("server options require protocol version")
        || text.contains("server does not support server options")
        || text.contains("server doesn't support server options")
        || text.contains("server doesn't support 'server-option'")
}
fn parse_advertisement(bytes: &[u8]) -> Result<BTreeMap<Vec<u8>, RemoteTagEntry>> {
    let invalid = || Error::parse("remote-tags", "malformed or duplicate tag advertisement");
    let mut entries = BTreeMap::<Vec<u8>, RemoteTagEntry>::new();
    let mut peeled = BTreeMap::new();
    let mut symbolic = BTreeMap::new();
    for line in bytes.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        let separator = line.iter().position(|b| *b == b'\t').ok_or_else(invalid)?;
        let (id, name) = (&line[..separator], &line[separator + 1..]);
        let name = name.strip_prefix(b"refs/tags/").ok_or_else(invalid)?;
        if let Some(target) = id.strip_prefix(b"ref: ") {
            if !target.starts_with(b"refs/")
                || target.iter().any(|b| *b < 32 || *b == 127)
                || name.is_empty()
                || name.ends_with(b"^{}")
                || symbolic.insert(name.to_vec(), target.to_vec()).is_some()
            {
                return Err(invalid());
            }
            if symbolic.len() > TAG_LIMIT {
                return refused(TagFailure::RemoteLimit);
            }
            continue;
        }
        let id = oid(id)?;
        if id.as_str().bytes().all(|b| b == b'0')
            || name.is_empty()
            || name.iter().any(|b| *b < 32 || *b == 127)
        {
            return Err(invalid());
        }
        if let Some(name) = name.strip_suffix(b"^{}") {
            if peeled.insert(name.to_vec(), id).is_some() {
                return Err(invalid());
            }
        } else if entries
            .insert(
                name.to_vec(),
                RemoteTagEntry {
                    name: name.to_vec(),
                    remote_id: Some(id),
                    peeled: None,
                    symbolic_target: None,
                    local: None,
                    summary: None,
                    remote_commit: None,
                },
            )
            .is_some()
        {
            return Err(invalid());
        }
        if entries.len() > TAG_LIMIT || peeled.len() > TAG_LIMIT {
            return refused(TagFailure::RemoteLimit);
        }
    }
    for (name, id) in peeled {
        entries.get_mut(&name).ok_or_else(invalid)?.peeled = Some(id);
    }
    for (name, target) in symbolic {
        entries.get_mut(&name).ok_or_else(invalid)?.symbolic_target = Some(target);
    }
    Ok(entries)
}
fn push_arguments(preview: &RemoteTagPreview, reference: &[u8]) -> Result<Vec<OsString>> {
    let mut arguments = args(&[
        "--no-replace-objects",
        "-c",
        "push.autoSetupRemote=false",
        "-c",
        "push.negotiate=false",
        "-c",
        "push.pushOption=",
        "push",
        "--porcelain",
        "--no-force",
        "--no-force-with-lease",
        "--no-force-if-includes",
        "--no-follow-tags",
        "--no-prune",
        "--no-recurse-submodules",
        "--no-mirror",
    ]);
    let expected = preview
        .entry
        .remote_id
        .as_ref()
        .map_or("", ObjectId::as_str);
    arguments.push(bytes_to_os(
        &[
            b"--force-with-lease=".as_slice(),
            reference,
            b":",
            expected.as_bytes(),
        ]
        .concat(),
    )?);
    arguments.push("--".into());
    arguments.push(bytes_to_os(&preview.snapshot.destination)?);
    let source = preview.expected_result().map_or("", ObjectId::as_str);
    arguments.push(bytes_to_os(&[source.as_bytes(), b":", reference].concat())?);
    Ok(arguments)
}
fn validate_porcelain(bytes: &[u8], reference: &[u8], action: RemoteTagAction) -> Result<()> {
    let mut count = 0;
    for row in bytes.split(|b| *b == b'\n') {
        if row.get(1) != Some(&b'\t') {
            continue;
        }
        let mut fields = row.split(|b| *b == b'\t');
        let flag = fields.next().unwrap_or_default();
        let mapping = fields.next().unwrap_or_default();
        let destination = mapping.rsplit(|b| *b == b':').next().unwrap_or_default();
        let wanted = match action {
            RemoteTagAction::Publish => b"*".as_slice(),
            RemoteTagAction::Delete => b"-".as_slice(),
        };
        if destination != reference || flag != wanted {
            return refused(TagFailure::VerificationFailed);
        }
        count += 1;
    }
    if count != 1 {
        return refused(TagFailure::VerificationFailed);
    }
    Ok(())
}
pub(crate) fn display_destination(bytes: &[u8]) -> String {
    let value = String::from_utf8_lossy(bytes);
    let value = value.split(['?', '#']).next().unwrap_or_default();
    if let Some((scheme, rest)) = value.split_once("://") {
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = authority.rsplit('@').next().unwrap_or_default();
        return format!(
            "{scheme}://{host}{}{}",
            if path.is_empty() { "" } else { "/" },
            path
        );
    }
    value.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn advertisement_is_complete_unambiguous_and_bounded() {
        let id = "1".repeat(40);
        let target = "2".repeat(40);
        let valid = format!("{id}\trefs/tags/v1\n{target}\trefs/tags/v1^{{}}\n");
        let tags = parse_advertisement(valid.as_bytes()).unwrap();
        assert_eq!(
            tags[b"v1".as_slice()].peeled.as_ref().unwrap().as_str(),
            target
        );
        for invalid in [
            format!("{valid}{valid}"),
            format!("{id}\trefs/tags/v1^{{}}\n"),
            format!("{id}\trefs/heads/main\n"),
            format!("{}\trefs/tags/v1\n", "0".repeat(40)),
        ] {
            assert!(parse_advertisement(invalid.as_bytes()).is_err());
        }
        assert!(parse_advertisement(b"").unwrap().is_empty());
        let symbolic = format!("ref: refs/heads/main\trefs/tags/alias\n{id}\trefs/tags/alias\n");
        let indirect = parse_advertisement(symbolic.as_bytes()).unwrap();
        assert_eq!(indirect[b"alias".as_slice()].relation(), "symbolic");
        assert!(matches!(
            reject_indirect(indirect.get(b"alias".as_slice())),
            Err(Error::Tag(TagFailure::SymbolicTag))
        ));
        assert!(parse_advertisement(b"ref: refs/heads/main\trefs/tags/alias\n").is_err());
        let large = (0..=TAG_LIMIT)
            .map(|n| format!("{id}\trefs/tags/tag{n}\n"))
            .collect::<String>();
        assert!(matches!(
            parse_advertisement(large.as_bytes()),
            Err(Error::Tag(TagFailure::RemoteLimit))
        ));
    }
    #[test]
    fn endpoint_display_omits_credentials_and_queries() {
        assert_eq!(
            display_destination(
                b"https://PRIVATE:SECRET@example.invalid/repo.git?token=SECRET#PRIVATE"
            ),
            "https://example.invalid/repo.git"
        );
    }
    #[test]
    fn old_protocol_has_specific_safe_guidance() {
        assert!(protocol_unsupported(
            b"fatal: server options require protocol version 2 or later\n"
        ));
        assert!(protocol_unsupported(
            b"fatal: server doesn't support server options\n"
        ));
        assert!(protocol_unsupported(
            b"fatal: server doesn't support 'server-option'\n"
        ));
        assert!(!protocol_unsupported(b"fatal: Authentication failed\n"));
        assert_eq!(
            TagFailure::RemoteProtocolRequired.code(),
            "remote_tag_protocol_required"
        );
    }
}
