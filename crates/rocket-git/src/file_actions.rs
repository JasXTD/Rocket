//! Reviewed whole-file working-tree changes. The index is never written.
use crate::{parse, repository::commit_revision_args, runner::args, *};
use std::{ffi::OsString, fmt, path::PathBuf};
mod disk;
pub const FILE_EDIT_LIMIT: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileEditFailure {
    Stale,
    WrongRepository,
    Unsupported,
    ActiveOperation,
    NoChange,
    MissingSource,
    InvalidRevision,
    Verification,
}
impl FileEditFailure {
    pub fn code(self) -> &'static str {
        match self {
            Self::Stale => "file_edit_stale",
            Self::WrongRepository => "file_edit_repository",
            Self::Unsupported => "file_edit_unsupported",
            Self::ActiveOperation => "file_edit_operation",
            Self::NoChange => "file_edit_no_change",
            Self::MissingSource => "file_edit_source_missing",
            Self::InvalidRevision => "file_edit_revision",
            Self::Verification => "file_edit_verification",
        }
    }
    pub fn guidance(self) -> &'static str {
        match self {
            Self::Stale => {
                "The file, index, branch or conversion settings changed after review. Review again before retrying, and check the operation summary for any files already changed."
            }
            Self::WrongRepository => {
                "This file preview or backup belongs to another worktree. Open a fresh review here."
            }
            Self::Unsupported => {
                "This operation supports regular files up to 16 MiB in existing normal folders. Links, junctions, submodules, hidden index flags, custom filters and encoding/ident conversions need Git or an external editor."
            }
            Self::ActiveOperation => {
                "Finish or abort the active Git operation before discarding or restoring files. Use the conflict editor for conflicts."
            }
            Self::NoChange => {
                "The working file already matches the reviewed result. No write or recovery copy is needed."
            }
            Self::MissingSource => {
                "The selected file version is unavailable locally. Check the exact path and revision; fetch missing objects explicitly before retrying."
            }
            Self::InvalidRevision => {
                "The revision could not be resolved locally. Enter an existing commit, branch or tag, then review again."
            }
            Self::Verification => {
                "The file operation needs review. Inspect the working file and retained recovery copies before retrying. A late external edit or interrupted replacement may already have changed the path; no automatic rollback was attempted."
            }
        }
    }
}
fn fail<T>(failure: FileEditFailure) -> Result<T> {
    Err(Error::FileEdit(failure))
}

#[derive(Clone, Debug)]
pub struct FileBackup {
    repository: PathBuf,
    id: String,
    path: RepoPath,
    present: bool,
}
impl FileBackup {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn path(&self) -> &RepoPath {
        &self.path
    }
    pub fn original_present(&self) -> bool {
        self.present
    }
}
#[derive(Clone, Debug)]
pub enum FileEditSource {
    Index,
    Commit(String),
    Backup(FileBackup),
    /// Issued only by the checked patch planner, never by a caller-supplied file write.
    Patch,
    Selection,
}
impl FileEditSource {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Index => "file-discard",
            Self::Commit(_) => "file-restore",
            Self::Backup(_) => "file-recover",
            Self::Patch => "patch-import",
            Self::Selection => "discard-selection",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Context {
    head: Head,
    index: Vec<u8>,
    flags: Vec<u8>,
    attrs: Vec<u8>,
    config: Vec<u8>,
}
#[derive(Clone)]
pub struct FileEditPreview {
    repository: PathBuf,
    path: RepoPath,
    source: FileEditSource,
    context: Context,
    before: Option<Vec<u8>>,
    after: Option<Vec<u8>>,
    /// The source entry's Git mode when it decides the executable bit (Linux): the
    /// index or commit version being restored. None keeps the file's own bits.
    #[cfg_attr(not(target_os = "linux"), expect(dead_code, reason = "only Linux files have an executable bit"))]
    executable: Option<bool>,
    commit: Option<ObjectId>,
    conversion: &'static str,
}
impl fmt::Debug for FileEditPreview {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileEditPreview")
            .field("operation", &self.source.label())
            .finish_non_exhaustive()
    }
}
impl FileEditPreview {
    pub fn path(&self) -> &RepoPath {
        &self.path
    }
    pub fn source(&self) -> &FileEditSource {
        &self.source
    }
    pub fn before(&self) -> Option<&[u8]> {
        self.before.as_deref()
    }
    pub fn after(&self) -> Option<&[u8]> {
        self.after.as_deref()
    }
    pub fn commit(&self) -> Option<&ObjectId> {
        self.commit.as_ref()
    }
    pub fn conversion(&self) -> &str {
        self.conversion
    }
}
pub struct FileEditReport {
    pub applied: bool,
    pub recovery_directory: Option<PathBuf>,
    pub result: Result<()>,
    pub verification: Result<()>,
    pub status: Result<Status>,
}
impl Repository {
    fn file_read(&self, arguments: &[OsString], cancel: &CancellationToken) -> Result<Vec<u8>> {
        let mut command = args(&["--no-replace-objects"]);
        command.extend_from_slice(arguments);
        self.runner
            .run(self.cwd(), &command, Vec::new(), cancel)?
            .success("file-preview")
    }
    fn file_context(&self, path: &RepoPath, cancel: &CancellationToken) -> Result<Context> {
        self.require_worktree(cancel)?;
        disk::validate_path(path)?;
        if !self.operation_state(cancel)?.is_idle() {
            return fail(FileEditFailure::ActiveOperation);
        }
        let read_path = |parts: &[&str]| {
            let mut a = args(parts);
            a.push(path.argument()?);
            self.file_read(&a, cancel)
        };
        let index = read_path(&["ls-files", "--stage", "-z", "--"])?;
        let flags = read_path(&["ls-files", "-v", "-z", "--"])?;
        if flags
            .split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .any(|s| !s.starts_with(b"H "))
        {
            return fail(FileEditFailure::Unsupported);
        }
        let (attrs, config) = self.conversion_settings(path, cancel)?;
        Ok(Context {
            head: self.head(cancel)?,
            index,
            flags,
            attrs,
            config,
        })
    }
    /// The attributes and core settings that decide how Git writes this path's
    /// working bytes. Compare both values before and after publishing.
    pub(crate) fn conversion_settings(
        &self,
        path: &RepoPath,
        cancel: &CancellationToken,
    ) -> Result<(Vec<u8>, Vec<u8>)> {
        let mut a = args(&[
            "check-attr",
            "-z",
            "text",
            "eol",
            "filter",
            "working-tree-encoding",
            "ident",
            "--",
        ]);
        a.push(path.argument()?);
        let attrs = self.file_read(&a, cancel)?;
        let out = self.runner.run(
            self.cwd(),
            &args(&[
                "config",
                "--null",
                "--get-regexp",
                "^core\\.(autocrlf|eol)$",
            ]),
            Vec::new(),
            cancel,
        )?;
        let config = if out.status.code() == Some(1) {
            Vec::new()
        } else {
            out.success("file-preview")?
        };
        Ok((attrs, config))
    }
    pub fn preview_file_edit(
        &self,
        path: &RepoPath,
        source: FileEditSource,
        cancel: &CancellationToken,
    ) -> Result<FileEditPreview> {
        self.runner.step("file-preview", "review-file", || {
            cancel.check()?;
            let context = self.file_context(path, cancel)?;
            let before = disk::read_working(self, path, cancel)?;
            let mut commit = None;
            let mut executable = None;
            let mut conversion = "Exact recovery bytes";
            let after = match &source {
                FileEditSource::Patch | FileEditSource::Selection => {
                    return fail(FileEditFailure::Unsupported);
                }
                FileEditSource::Backup(backup) => {
                    if backup.repository != self.info().git_dir || backup.path != *path {
                        return fail(FileEditFailure::WrongRepository);
                    }
                    disk::backup_contents(self, backup, cancel)?
                }
                FileEditSource::Index | FileEditSource::Commit(_) => {
                    let object = if let FileEditSource::Commit(revision) = &source {
                        let id = parse::oid(
                            parse::line(
                                &self
                                    .file_read(&commit_revision_args(revision)?, cancel)
                                    .map_err(|e| match e {
                                        Error::Git { .. } => {
                                            Error::FileEdit(FileEditFailure::InvalidRevision)
                                        }
                                        other => other,
                                    })?,
                            ),
                            "file-preview",
                        )?;
                        let mut a = args(&["ls-tree", "-z", "--full-tree", id.as_str(), "--"]);
                        a.push(path.argument()?);
                        let listing = self.file_read(&a, cancel)?;
                        commit = Some(id);
                        parse_object(&listing, path, true)?
                    } else {
                        parse_object(&context.index, path, false)?
                    };
                    match object {
                        None if matches!(source, FileEditSource::Commit(_)) => {
                            return fail(FileEditFailure::MissingSource);
                        }
                        None => {
                            conversion = "Remove from working folder; preserve a recovery copy";
                            None
                        }
                        Some((id, mode_executable)) => {
                            executable = Some(mode_executable);
                            let size = self
                                .file_read(&args(&["cat-file", "-s", id.as_str()]), cancel)
                                .map_err(|e| match e {
                                    Error::Git { .. } => {
                                        Error::FileEdit(FileEditFailure::MissingSource)
                                    }
                                    other => other,
                                })?;
                            let size = std::str::from_utf8(parse::line(&size))
                                .ok()
                                .and_then(|s| s.parse::<usize>().ok())
                                .ok_or_else(|| Error::parse("file-preview", "invalid blob size"))?;
                            if size > FILE_EDIT_LIMIT {
                                return fail(FileEditFailure::Unsupported);
                            }
                            let bytes =
                                self.file_read(&args(&["cat-file", "blob", id.as_str()]), cancel)?;
                            if bytes.len() != size {
                                return Err(Error::parse("file-preview", "blob size changed"));
                            }
                            let (bytes, rule) = working_bytes(bytes, &context)?;
                            conversion = rule;
                            Some(bytes)
                        }
                    }
                }
            };
            cancel.check()?;
            if before == after {
                return fail(FileEditFailure::NoChange);
            }
            if self.file_context(path, cancel)? != context
                || disk::read_working(self, path, cancel)? != before
            {
                return fail(FileEditFailure::Stale);
            }
            Ok(FileEditPreview {
                repository: self.info().git_dir.clone(),
                path: path.clone(),
                source,
                context,
                before,
                after,
                executable,
                commit,
                conversion,
            })
        })
    }
    pub(crate) fn recheck_file_edit(
        &self,
        p: &FileEditPreview,
        cancel: &CancellationToken,
    ) -> Result<()> {
        if p.repository != self.info().git_dir {
            return fail(FileEditFailure::WrongRepository);
        }
        if self.file_context(&p.path, cancel)? != p.context
            || disk::read_working(self, &p.path, cancel)? != p.before
        {
            return fail(FileEditFailure::Stale);
        }
        Ok(())
    }
    /// Snapshot a patch target without accepting replacement bytes from public callers.
    pub(crate) fn patch_file_base(
        &self,
        path: &RepoPath,
        source: FileEditSource,
        cancel: &CancellationToken,
    ) -> Result<FileEditPreview> {
        let context = self.file_context(path, cancel)?;
        parse_object(&context.index, path, false)?;
        working_bytes(Vec::new(), &context)?; // Refuse filters, encodings and ident before sandboxing.
        let before = disk::read_working(self, path, cancel)?;
        Ok(FileEditPreview {
            repository: self.info().git_dir.clone(),
            path: path.clone(),
            source,
            context,
            after: before.clone(),
            before,
            executable: None,
            commit: None,
            conversion: "Reviewed patch result; staging unchanged",
        })
    }
    pub(crate) fn finish_patch_file(
        &self,
        mut p: FileEditPreview,
        after: Option<Vec<u8>>,
        crlf: bool,
        cancel: &CancellationToken,
    ) -> Result<FileEditPreview> {
        p.after = match after {
            Some(bytes) if crlf || p.before.is_none() => Some(working_bytes(bytes, &p.context)?.0),
            other => other,
        };
        self.recheck_file_edit(&p, cancel)?;
        Ok(p)
    }
    pub(crate) fn finish_raw_patch_file(&self, mut p: FileEditPreview, after: Option<Vec<u8>>, c: &CancellationToken) -> Result<FileEditPreview> {
        p.after=after;
        self.recheck_file_edit(&p,c)?;
        Ok(p)
    }
    pub(crate) fn patch_input(p: &FileEditPreview) -> Result<(Option<Vec<u8>>, bool)> {
        let Some(before) = &p.before else {
            return Ok((None, false));
        };
        if before.contains(&0) || std::str::from_utf8(before).is_err() {
            return fail(FileEditFailure::Unsupported);
        }
        let converts = working_bytes(b"\n".to_vec(), &p.context)?.0 == b"\r\n";
        let crlf = converts && before.windows(2).any(|w| w == b"\r\n");
        if !crlf {
            return Ok((Some(before.clone()), false));
        }
        // Mixed endings cannot be normalized without changing unselected bytes.
        if before
            .iter()
            .enumerate()
            .any(|(i, b)| *b == b'\n' && (i == 0 || before[i - 1] != b'\r'))
        {
            return fail(FileEditFailure::Unsupported);
        }
        let bytes = before
            .iter()
            .enumerate()
            .filter_map(|(i, b)| {
                if *b == b'\r' && before.get(i + 1) == Some(&b'\n') {
                    None
                } else {
                    Some(*b)
                }
            })
            .collect();
        Ok((Some(bytes), true))
    }
    pub(crate) fn publish_patch_file(
        &self,
        p: &FileEditPreview,
        cancel: &CancellationToken,
        applied: &mut bool,
        recovery: &mut Option<PathBuf>,
    ) -> Result<()> {
        disk::publish(self, p, cancel, applied, recovery)
    }
    pub(crate) fn verify_patch_file(
        &self,
        p: &FileEditPreview,
        applied: bool,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let context = self.file_context(&p.path, cancel)?;
        if context.head != p.context.head
            || context.index != p.context.index
            || context.flags != p.context.flags
            || disk::read_working(self, &p.path, cancel)?
                != if applied {
                    p.after.clone()
                } else {
                    p.before.clone()
                }
        {
            return fail(FileEditFailure::Verification);
        }
        Ok(())
    }
    pub fn apply_file_edit(
        &self,
        p: &FileEditPreview,
        cancel: &CancellationToken,
    ) -> Result<FileEditReport> {
        let _permit = self.acquire_write(cancel)?;
        self.runner.step(p.source.label(), "revalidate-file", || {
            self.recheck_file_edit(p, cancel)
        })?;
        let mut applied = false;
        let mut recovery_directory = None;
        let result = disk::publish(self, p, cancel, &mut applied, &mut recovery_directory);
        let fresh = CancellationToken::new();
        let verification = self.runner.step(p.source.label(), "verify-file", || {
            let expected = if applied { &p.after } else { &p.before };
            let current = self.file_context(&p.path, &fresh)?;
            // Restoring .gitattributes can intentionally change conversion settings.
            // They were checked before publication; afterwards verify the pinned
            // result and unchanged Git identity/index, not the old attributes.
            let context_matches = if applied {
                current.head == p.context.head
                    && current.index == p.context.index
                    && current.flags == p.context.flags
            } else {
                current == p.context
            };
            if disk::read_working(self, &p.path, &fresh)? != *expected || !context_matches {
                return fail(FileEditFailure::Verification);
            }
            Ok(())
        });
        let status = self
            .runner
            .observe("refresh", "status", || self.status(&fresh));
        Ok(FileEditReport {
            applied,
            recovery_directory,
            result,
            verification,
            status,
        })
    }
    pub fn file_backups(&self, cancel: &CancellationToken) -> Result<Vec<FileBackup>> {
        self.require_worktree(cancel)?;
        self.runner
            .step("file-backups", "read-backups", || disk::list(self, cancel))
    }
}
/// Reserved device names, `.git` components and trailing dots/spaces are refused.
pub(crate) fn validate_working_path(path: &RepoPath) -> Result<()> {
    disk::validate_path(path)
}
/// The entry's object and whether its mode is executable (100755).
fn parse_object(bytes: &[u8], path: &RepoPath, tree: bool) -> Result<Option<(ObjectId, bool)>> {
    if bytes.is_empty() {
        return Ok(None);
    }
    let record = bytes
        .strip_suffix(&[0])
        .ok_or_else(|| Error::parse("file-preview", "unterminated entry"))?;
    let tab = record
        .iter()
        .position(|b| *b == b'\t')
        .ok_or_else(|| Error::parse("file-preview", "missing entry path"))?;
    if &record[tab + 1..] != path.as_bytes() {
        return fail(FileEditFailure::Unsupported);
    }
    let fields: Vec<_> = record[..tab].split(|b| *b == b' ').collect();
    if fields.len() != 3
        || !matches!(fields[0], b"100644" | b"100755")
        || fields[if tree { 1 } else { 2 }] != if tree { b"blob".as_slice() } else { b"0" }
    {
        return fail(FileEditFailure::Unsupported);
    }
    Ok(Some((
        parse::oid(fields[if tree { 2 } else { 1 }], "file-preview")?,
        fields[0] == b"100755",
    )))
}
fn working_bytes(bytes: Vec<u8>, context: &Context) -> Result<(Vec<u8>, &'static str)> {
    checkout_bytes(bytes, &context.attrs, &context.config)
}
/// Git blob bytes as a checkout would write them, from `conversion_settings`.
/// Filters, working-tree encodings and ident expansion are refused, never run.
pub(crate) fn checkout_bytes(
    bytes: Vec<u8>,
    attrs: &[u8],
    config: &[u8],
) -> Result<(Vec<u8>, &'static str)> {
    let records: Vec<_> = attrs.split(|b| *b == 0).filter(|s| !s.is_empty()).collect();
    if records.len() != 15 {
        return Err(Error::parse("file-preview", "invalid attributes"));
    }
    let attr = |name: &[u8]| -> &[u8] {
        records
            .as_chunks::<3>()
            .0
            .iter()
            .find(|a| a[1] == name)
            .map_or(b"unspecified", |a| a[2])
    };
    for name in [b"filter".as_slice(), b"working-tree-encoding", b"ident"] {
        if !matches!(attr(name), b"unspecified" | b"unset") {
            return fail(FileEditFailure::Unsupported);
        }
    }
    let config = |key: &[u8]| -> &[u8] {
        config
            .split(|b| *b == 0)
            .filter_map(|r| r.split_once_byte(b'\n'))
            .filter(|(k, _)| *k == key)
            .map(|(_, v)| v)
            .next_back()
            .unwrap_or(b"")
    };
    let text = attr(b"text");
    let eol = attr(b"eol");
    let auto_value = config(b"core.autocrlf").to_ascii_lowercase();
    let auto = match auto_value.as_slice() {
        b"yes" | b"on" | b"1" => b"true".as_slice(),
        b"no" | b"off" | b"0" => b"false",
        other => other,
    };
    let core_eol = config(b"core.eol").to_ascii_lowercase();
    if !matches!(auto, b"" | b"true" | b"false" | b"input")
        || !matches!(core_eol.as_slice(), b"" | b"lf" | b"crlf" | b"native")
    {
        return fail(FileEditFailure::Unsupported);
    }
    if !matches!(text, b"unspecified" | b"unset" | b"set" | b"auto")
        || !matches!(eol, b"unspecified" | b"unset" | b"lf" | b"crlf")
    {
        return fail(FileEditFailure::Unsupported);
    }
    let enabled = text != b"unset"
        && (text == b"set"
            || text == b"auto"
            || matches!(eol, b"lf" | b"crlf")
            || matches!(auto, b"true" | b"input"));
    let crlf = eol == b"crlf"
        || (eol != b"lf"
            && (auto == b"true"
                || (auto != b"input"
                    && (core_eol == b"crlf" || (core_eol != b"lf" && cfg!(windows))))));
    if !enabled || !crlf {
        return Ok((bytes, "Git blob bytes (no working-tree filter)"));
    }
    if (text == b"auto" || text == b"unspecified") && (bytes.contains(&0) || bytes.contains(&b'\r'))
    {
        return Ok((bytes, "Binary/existing CR bytes preserved"));
    }
    if std::str::from_utf8(&bytes).is_err() {
        return fail(FileEditFailure::Unsupported);
    }
    let mut result = Vec::with_capacity(bytes.len());
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'\n' && (i == 0 || bytes[i - 1] != b'\r') {
            result.push(b'\r')
        }
        result.push(*b);
        if result.len() > FILE_EDIT_LIMIT {
            return fail(FileEditFailure::Unsupported);
        }
    }
    Ok((result, "LF converted to CRLF for this working folder"))
}
trait SplitByte {
    fn split_once_byte(&self, b: u8) -> Option<(&[u8], &[u8])>;
}
impl SplitByte for [u8] {
    fn split_once_byte(&self, b: u8) -> Option<(&[u8], &[u8])> {
        self.iter()
            .position(|v| *v == b)
            .map(|i| (&self[..i], &self[i + 1..]))
    }
}
