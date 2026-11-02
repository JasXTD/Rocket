//! Whole-version resolution: keep ours, theirs or the deletion of one conflicted
//! path. A review pins the conflict stages, conversion settings and working
//! bytes; applying rechecks them under the write gate, publishes with the text
//! editor's recovery copies and stages exactly the reviewed index entry. Nothing
//! is committed and the surrounding operation is never continued.
use super::{
    file::{Bounds, Change},
    *,
};
use crate::{
    file_actions::{checkout_bytes, validate_working_path},
    parse,
};

/// Each kept version and the observed working file are bounded by this size.
pub const CONFLICT_FILE_LIMIT: usize = crate::FILE_EDIT_LIMIT;
const WHOLE: Bounds = Bounds {
    limit: CONFLICT_FILE_LIMIT,
    unsupported: ConflictFailure::UnsupportedVersion,
};
const READONLY: &str = "The working file is read-only. Change its permissions explicitly before resolving; nothing was written.";

/// Which complete version of a conflicted path becomes the resolution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictKeep {
    /// Index stage 2.
    Ours,
    /// Index stage 3.
    Theirs,
    /// Remove the path; offered only when one side deleted it.
    Deletion,
}
impl ConflictKeep {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ours => "ours",
            Self::Theirs => "theirs",
            Self::Deletion => "deletion",
        }
    }
    fn stage(self) -> Option<u8> {
        match self {
            Self::Ours => Some(2),
            Self::Theirs => Some(3),
            Self::Deletion => None,
        }
    }
}
/// One index stage of the conflict. Its content is not retained.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictVersion {
    pub id: ObjectId,
    pub executable: bool,
    pub size: usize,
    /// Git's own merge test: a NUL byte within the first 8,000 bytes.
    pub binary: bool,
    pub gitlink: bool,
    pub lfs: Option<LfsPointer>,
    /// Missing/corrupt local LFS objects are shown but cannot be selected.
    pub unavailable: Option<LfsFailure>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictKind {
    File,
    Lfs,
    Submodule,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WholeFileChange {
    Replaced,
    Created,
    Removed,
}
impl WholeFileChange {
    pub fn label(self) -> &'static str {
        match self {
            Self::Replaced => "replaced",
            Self::Created => "created",
            Self::Removed => "removed",
        }
    }
}
#[derive(Clone)]
pub struct WholeConflictPreview {
    git_dir: PathBuf,
    path: RepoPath,
    context: Context,
    settings: (Vec<u8>, Vec<u8>),
    versions: [Option<ConflictVersion>; 3],
    working: Option<Vec<u8>>,
    /// Whether the working file already holds each stage as a checkout writes it.
    matches: [bool; 3],
}
impl fmt::Debug for WholeConflictPreview {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WholeConflictPreview")
            .field(
                "stages",
                &self
                    .versions
                    .iter()
                    .map(Option::is_some)
                    .collect::<Vec<_>>(),
            )
            .field("working_bytes", &self.working.as_ref().map(Vec::len))
            .finish_non_exhaustive()
    }
}
impl WholeConflictPreview {
    pub fn kind(&self) -> ConflictKind {
        if self.versions.iter().flatten().any(|v| v.gitlink) {
            ConflictKind::Submodule
        } else if self.versions.iter().flatten().any(|v| v.lfs.is_some()) {
            ConflictKind::Lfs
        } else {
            ConflictKind::File
        }
    }
    pub fn path(&self) -> &RepoPath {
        &self.path
    }
    /// 1 = base, 2 = ours, 3 = theirs; None when that side has no file.
    pub fn version(&self, stage: u8) -> Option<&ConflictVersion> {
        self.versions.get(stage.checked_sub(1)? as usize)?.as_ref()
    }
    pub fn working_size(&self) -> Option<usize> {
        self.working.as_ref().map(Vec::len)
    }
    pub fn working_binary(&self) -> bool {
        self.working.as_deref().is_some_and(binary)
    }
    /// True when a version or the working file is binary.
    pub fn binary(&self) -> bool {
        self.working_binary() || self.versions.iter().flatten().any(|v| v.binary)
    }
    /// True when ours or theirs deleted the path (modify/delete, add-by-one-side).
    pub fn deleted(&self) -> bool {
        self.versions[1].is_none() || self.versions[2].is_none()
    }
    pub fn can_keep(&self, keep: ConflictKeep) -> bool {
        match keep.stage() {
            Some(stage) => self.version(stage).is_some_and(|v| v.unavailable.is_none()),
            None => self.deleted(),
        }
    }
    /// True when the working file already holds this stage's version.
    pub fn working_matches(&self, stage: u8) -> bool {
        (1..=3).contains(&stage) && self.matches[stage as usize - 1]
    }
    /// The working-file change keeping this version needs, judged from the review;
    /// None when only the index changes.
    pub fn planned_change(&self, keep: ConflictKeep) -> Option<WholeFileChange> {
        if self.kind() == ConflictKind::Submodule {
            return None;
        }
        if keep
            .stage()
            .is_some_and(|stage| self.working_matches(stage))
        {
            return None;
        }
        match (self.working.is_some(), keep) {
            (false, ConflictKeep::Deletion) => None,
            (false, _) => Some(WholeFileChange::Created),
            (true, ConflictKeep::Deletion) => Some(WholeFileChange::Removed),
            (true, _) => Some(WholeFileChange::Replaced),
        }
    }
}
pub struct WholeConflictReport {
    /// The confirmed working-file change. None when the file already matched the
    /// kept version or publication did not complete; later cancellation never
    /// clears a confirmed change.
    pub file_change: Option<WholeFileChange>,
    /// True only after the index entry is verified to be exactly the kept version.
    pub staged: bool,
    pub recovery_directory: Option<PathBuf>,
    pub result: Result<()>,
    pub status: Result<Status>,
    pub state: Result<OperationState>,
}

fn binary(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(8000)].contains(&0)
}
fn refuse(failure: ConflictFailure) -> Error {
    Error::Conflict(failure)
}
/// Conversion and path checks are shared with whole-file restore; report their
/// refusals with this operation's own code.
fn whole(error: Error) -> Error {
    match error {
        Error::FileEdit(FileEditFailure::Unsupported) => {
            refuse(ConflictFailure::UnsupportedVersion)
        }
        other => other,
    }
}
fn stage_entries(stages: &[u8], path: &RepoPath) -> Result<Vec<(u8, bool, bool, ObjectId)>> {
    let mut entries: Vec<(u8, bool, bool, ObjectId)> = Vec::new();
    for record in stages.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let tab = record
            .iter()
            .position(|b| *b == b'\t')
            .ok_or_else(|| Error::parse("conflict stages", "missing path"))?;
        if &record[tab + 1..] != path.as_bytes() {
            return Err(Error::parse("conflict stages", "unexpected path"));
        }
        let fields: Vec<_> = record[..tab].split(|b| *b == b' ').collect();
        if fields.len() != 3 || !matches!(fields[2], b"1" | b"2" | b"3") {
            return Err(Error::parse("conflict stages", "invalid stage"));
        }
        // Gitlinks are index-only; links and other special modes need Git.
        if !matches!(fields[0], b"100644" | b"100755" | b"160000") {
            return Err(refuse(ConflictFailure::UnsupportedVersion));
        }
        let stage = fields[2][0] - b'0';
        if entries.iter().any(|(s, ..)| *s == stage) {
            return Err(Error::parse("conflict stages", "duplicate stage"));
        }
        entries.push((
            stage,
            fields[0] == b"100755",
            fields[0] == b"160000",
            parse::oid(fields[1], "conflict object")?,
        ));
    }
    Ok(entries)
}

pub(super) fn lfs_attributes(attrs: &[u8]) -> Result<bool> {
    let records: Vec<_> = attrs.split(|b| *b == 0).filter(|s| !s.is_empty()).collect();
    if records.len() != 15 {
        return Err(Error::parse("conflict attributes", "invalid attributes"));
    }
    let attr = |name: &[u8]| {
        records
            .as_chunks::<3>()
            .0
            .iter()
            .find(|r| r[1] == name)
            .map_or(b"unspecified".as_slice(), |r| r[2])
    };
    let lfs = attr(b"filter") == b"lfs";
    if lfs
        && (!matches!(attr(b"working-tree-encoding"), b"unspecified" | b"unset")
            || !matches!(attr(b"ident"), b"unspecified" | b"unset")
            || attr(b"text") != b"unset")
    {
        return Err(refuse(ConflictFailure::UnsupportedVersion));
    }
    Ok(lfs)
}

impl Repository {
    fn version_size(&self, id: &ObjectId, cancel: &CancellationToken) -> Result<usize> {
        let output = self
            .runner
            .run(
                self.cwd(),
                &args(&["--no-replace-objects", "cat-file", "-s", id.as_str()]),
                Vec::new(),
                cancel,
            )?
            .success("read conflict version size")?;
        std::str::from_utf8(parse::line(&output))
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| Error::parse("conflict version", "invalid blob size"))
    }
    fn version_bytes(
        &self,
        version: &ConflictVersion,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>> {
        let bytes = self
            .runner
            .run(
                self.cwd(),
                &args(&[
                    "--no-replace-objects",
                    "cat-file",
                    "blob",
                    version.id.as_str(),
                ]),
                Vec::new(),
                cancel,
            )?
            .success("read conflict version")?;
        if bytes.len() != version.size {
            return Err(Error::parse("conflict version", "blob size changed"));
        }
        Ok(bytes)
    }
    /// Stages, HEAD, operation and conversion settings must be exactly as reviewed.
    fn recheck_versions(
        &self,
        preview: &WholeConflictPreview,
        failure: ConflictFailure,
        cancel: &CancellationToken,
    ) -> Result<()> {
        if self.conflict_context(&preview.path, cancel)? != preview.context
            || self.conversion_settings(&preview.path, cancel)? != preview.settings
        {
            return Err(refuse(failure));
        }
        Ok(())
    }
    /// Read every side of a conflicted path for keeping one complete version.
    /// Nothing is written.
    pub fn review_whole_conflict(
        &self,
        path: &RepoPath,
        cancel: &CancellationToken,
    ) -> Result<WholeConflictPreview> {
        self.require_worktree(cancel)?;
        self.runner
            .step("conflict-keep-review", "read_conflict_versions", || {
                validate_working_path(path).map_err(whole)?;
                let context = self.conflict_context(path, cancel)?;
                let settings = self.conversion_settings(path, cancel)?;
                let entries = stage_entries(&context.stages, path)?;
                let gitlink = entries.iter().any(|e| e.2);
                if gitlink && entries.iter().any(|e| !e.2) {
                    return Err(refuse(ConflictFailure::UnsupportedVersion));
                }
                let lfs = !gitlink && lfs_attributes(&settings.0)?;
                if gitlink || lfs { super::index::check_layout(self, cancel)?; }
                if !gitlink && !lfs {
                    checkout_bytes(Vec::new(), &settings.0, &settings.1).map_err(whole)?;
                }
                let working = if gitlink {
                    None
                } else {
                    file::read_optional(self, path, WHOLE)?
                };
                let mut versions = [None, None, None];
                let mut matches = [false; 3];
                for (stage, executable, gitlink, id) in entries {
                    let size = if gitlink {
                        0
                    } else {
                        self.version_size(&id, cancel)?
                    };
                    if size > CONFLICT_FILE_LIMIT {
                        return Err(refuse(ConflictFailure::UnsupportedVersion));
                    }
                    let mut version = ConflictVersion {
                        id,
                        executable,
                        size,
                        binary: false,
                        gitlink,
                        lfs: None,
                        unavailable: None,
                    };
                    if gitlink {
                        versions[stage as usize - 1] = Some(version);
                        continue;
                    }
                    let bytes = self.version_bytes(&version, cancel)?;
                    version.binary = binary(&bytes);
                    if lfs {
                        let pointer = if bytes.is_empty() { LfsPointer { oid: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".into(), size: 0 } } else { LfsPointer::parse(&bytes)? };
                        match self
                            .runner
                            .step("conflict-keep-review", "verify-lfs-object", || {
                                self.conflict_lfs_content(&pointer, cancel)
                            }) {
                            Ok(content) => {
                                matches[stage as usize - 1] = working.as_ref() == Some(&content)
                            }
                            Err(Error::Lfs(f)) => version.unavailable = Some(f),
                            Err(e) => return Err(e),
                        }
                        version.lfs = Some(pointer);
                    } else {
                        // Content-dependent conversion refusals surface when keeping it.
                        matches[stage as usize - 1] = working.as_ref().is_some_and(|working| {
                            checkout_bytes(bytes, &settings.0, &settings.1)
                                .is_ok_and(|(bytes, _)| bytes == *working)
                        });
                    }
                    versions[stage as usize - 1] = Some(version);
                }
                cancel.check()?;
                let preview = WholeConflictPreview {
                    git_dir: self.info().git_dir.clone(),
                    path: path.clone(),
                    context,
                    settings,
                    versions,
                    working,
                    matches,
                };
                self.recheck_versions(&preview, ConflictFailure::ReviewStale, cancel)?;
                if !gitlink && file::read_optional(self, path, WHOLE)? != preview.working {
                    return Err(refuse(ConflictFailure::ReviewStale));
                }
                Ok(preview)
            })
    }
    /// Keep one reviewed version: publish it to the working file with recovery
    /// copies, then stage exactly that index entry (or the deletion).
    pub fn keep_conflict_side(
        &self,
        preview: &WholeConflictPreview,
        keep: ConflictKeep,
        cancel: &CancellationToken,
    ) -> Result<WholeConflictReport> {
        self.require_worktree(cancel)?;
        if preview.git_dir != self.info().git_dir {
            return Err(refuse(ConflictFailure::WrongRepository));
        }
        if !preview.can_keep(keep) {
            return Err(refuse(ConflictFailure::VersionUnavailable));
        }
        let _permit = self.acquire_write(cancel)?;
        let path = &preview.path;
        let kept = keep.stage().and_then(|stage| preview.version(stage));
        let gitlink = preview.kind() == ConflictKind::Submodule;
        let mut published = false;
        let mut file_change = None;
        let mut staged = false;
        let mut recovery_directory = None;
        let result = self.runner.observe("write_result", "conflict-keep", || {
            let target = self
                .runner
                .step("conflict-keep", "revalidate_versions", || {
                    cancel.check()?;
                    self.recheck_versions(preview, ConflictFailure::ReviewStale, cancel)?;
                    if preview.kind() != ConflictKind::File {
                        super::index::check_layout(self, cancel)?;
                    }
                    if !gitlink && file::read_optional(self, path, WHOLE)? != preview.working {
                        return Err(refuse(ConflictFailure::ReviewStale));
                    }
                    let Some(version) = kept else {
                        return Ok(None);
                    };
                    if gitlink {
                        return Ok(None);
                    }
                    if let Some(pointer) = &version.lfs {
                        return self.conflict_lfs_content(pointer, cancel).map(Some);
                    }
                    let bytes = self.version_bytes(version, cancel)?;
                    let (bytes, _) =
                        checkout_bytes(bytes, &preview.settings.0, &preview.settings.1)
                            .map_err(whole)?;
                    if bytes.len() > CONFLICT_FILE_LIMIT {
                        return Err(refuse(ConflictFailure::UnsupportedVersion));
                    }
                    Ok(Some(bytes))
                })?;
            if target != preview.working {
                self.runner.step("conflict-keep", "publish_version", || {
                    let change = Change {
                        path,
                        observed: preview.working.as_deref(),
                        result: target.as_deref(),
                        bounds: WHOLE,
                        stale: ConflictFailure::ReviewStale,
                        unverified: ConflictFailure::ResolutionUnverified,
                        readonly: READONLY,
                    };
                    let recheck =
                        || self.recheck_versions(preview, ConflictFailure::ReviewStale, cancel);
                    let published_now = file::publish_change(
                        self,
                        &change,
                        recheck,
                        cancel,
                        &mut published,
                        &mut recovery_directory,
                    );
                    if published {
                        file_change = preview.planned_change(keep);
                    }
                    published_now
                })?;
            }
            self.runner.step("conflict-keep", "stage_version", || {
                cancel.check()?;
                // The file is now the kept version; the index must still hold the
                // reviewed stages, so a concurrent resolution is never overwritten.
                // Conversion settings were checked before publication: keeping a
                // version of `.gitattributes` may intentionally change its own.
                if self.conflict_context(path, cancel)? != preview.context {
                    return Err(refuse(ConflictFailure::ResolutionUnverified));
                }
                let any = preview
                    .versions
                    .iter()
                    .flatten()
                    .next()
                    .ok_or_else(|| Error::parse("conflict stages", "no stages"))?;
                let mode = |v: &ConflictVersion| {
                    if v.gitlink {
                        "160000"
                    } else if v.executable {
                        "100755"
                    } else {
                        "100644"
                    }
                };
                let mut input = match kept {
                    Some(v) => format!("{} {} 0\t", mode(v), v.id.as_str()),
                    // Mode 0 removes every stage of the path.
                    None => format!("0 {}\t", any.id.as_str()),
                }
                .into_bytes();
                input.extend_from_slice(path.as_bytes());
                input.push(0);
                if preview.kind() != ConflictKind::File {
                    let recheck = || {
                        self.recheck_versions(
                            preview,
                            ConflictFailure::ResolutionUnverified,
                            cancel,
                        )?;
                        if !gitlink && file::read_optional(self, path, WHOLE)? != target {
                            return Err(refuse(ConflictFailure::ResolutionUnverified));
                        }
                        Ok(())
                    };
                    super::index::stage(
                        self,
                        &preview.context.stages,
                        input.clone(),
                        recheck,
                        cancel,
                        &mut staged,
                        &mut recovery_directory,
                    )?;
                } else {
                    self.runner
                        .write(
                            self.cwd(),
                            &args(&["update-index", "-z", "--index-info"]),
                            input,
                            cancel,
                        )?
                        .success("stage conflict version")?;
                }
                let mut arguments = args(&["ls-files", "--stage", "-z", "--"]);
                arguments.push(path.argument()?);
                let index = self
                    .runner
                    .run(self.cwd(), &arguments, Vec::new(), cancel)?
                    .success("verify conflict resolution")?;
                let expected = kept.map(|v| {
                    let mut line = format!("{} {} 0\t", mode(v), v.id.as_str()).into_bytes();
                    line.extend_from_slice(path.as_bytes());
                    line.push(0);
                    line
                });
                if index != expected.unwrap_or_default() {
                    return Err(refuse(ConflictFailure::ResolutionUnverified));
                }
                staged = true;
                Ok(())
            })
        });
        let status = self.runner.observe("refresh", "status", || {
            self.status(&CancellationToken::new())
        });
        let state = self.runner.observe("refresh", "operation", || {
            self.operation_state(&CancellationToken::new())
        });
        Ok(WholeConflictReport {
            file_change,
            staged,
            recovery_directory,
            result,
            status,
            state,
        })
    }
}
