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
    before_executable: Option<bool>,
    after: Option<Vec<u8>>,
    /// The executable bit to restore from the index, commit or displaced file.
    /// None keeps the working file's permissions.
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
    fn matches_before(&self, (bytes, mode): &disk::FileState) -> bool {
        *bytes == self.before && *mode == self.before_executable
    }
    fn matches_result(&self, state: &disk::FileState, applied: bool) -> bool {
        if !applied {
            return self.matches_before(state);
        }
        let (bytes, mode) = state;
        *bytes == self.after
            && mode
                .zip(self.executable.or(self.before_executable))
                .is_none_or(|(actual, expected)| actual == expected)
    }
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
        let mut a = args(&["check-attr", "-z"]);
        a.extend(args(&CONVERSION_ATTRIBUTES));
        a.push("--".into());
        a.push(path.argument()?);
        let attrs = self.file_read(&a, cancel)?;
        let out = self.runner.run(
            self.cwd(),
            &args(&[
                "config",
                "--null",
                // Git canonicalizes implicit, numeric and spelled booleans, while
                // retaining the non-boolean modes "input", "warn", "lf" and "crlf".
                "--type=bool-or-str",
                "--get-regexp",
                "^core\\.(autocrlf|eol|filemode|safecrlf)$",
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
            let working = disk::read_working(self, path, cancel)?;
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
                    let (bytes, mode) = disk::backup_contents(self, backup, cancel)?;
                    executable = mode;
                    bytes
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
            #[cfg(windows)]
            if let Some(wanted) = executable
                && working.0.is_some()
                && self.file_read(
                    &args(&["config", "--bool", "--default=true", "--get", "core.filemode"]),
                    cancel,
                )?.trim_ascii() == b"true"
            {
                // A Windows checkout cannot chmod a regular file. When Git is configured
                // to track mode anyway, ask it which mode it observes for this path.
                let mut args = args(&["diff-files", "--raw", "-z", "--"]);
                args.push(path.argument()?);
                let diff = self.file_read(&args, cancel)?;
                let working_mode = if diff.is_empty() {
                    parse_object(&context.index, path, false)?.map(|(_, mode)| mode)
                } else {
                    match diff.split(|b| *b == b' ').nth(1) {
                        Some(b"100755") => Some(true),
                        Some(b"100644") => Some(false),
                        _ => return fail(FileEditFailure::Unsupported),
                    }
                };
                if working_mode.is_some_and(|actual| actual != wanted) {
                    return Err(Error::Unsupported("Windows cannot change the executable bit of this file. Restore its mode from a Unix working tree.".into()));
                }
            }
            conversion = describe_change(&working, after.as_deref(), executable, conversion)?;
            let (before, before_executable) = working;
            let preview = FileEditPreview {
                repository: self.info().git_dir.clone(),
                path: path.clone(),
                source,
                context,
                before,
                before_executable,
                after,
                executable,
                commit,
                conversion,
            };
            self.recheck_file_edit(&preview, cancel)?;
            Ok(preview)
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
            || !p.matches_before(&disk::read_working(self, &p.path, cancel)?)
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
        let (before, before_executable) = disk::read_working(self, path, cancel)?;
        Ok(FileEditPreview {
            repository: self.info().git_dir.clone(),
            path: path.clone(),
            source,
            context,
            after: before.clone(),
            before,
            before_executable,
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
            || !p.matches_result(&disk::read_working(self, &p.path, cancel)?, applied)
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
            let state = disk::read_working(self, &p.path, &fresh)?;
            if !context_matches || !p.matches_result(&state, applied) {
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
fn describe_change(
    working: &disk::FileState,
    after: Option<&[u8]>,
    executable: Option<bool>,
    conversion: &'static str,
) -> Result<&'static str> {
    if working.0.as_deref() != after {
        return Ok(conversion);
    }
    match (working.1, executable) {
        (Some(false), Some(true)) => Ok("Restore executable permission"),
        (Some(true), Some(false)) => Ok("Remove executable permission"),
        _ => fail(FileEditFailure::NoChange),
    }
}

fn working_bytes(bytes: Vec<u8>, context: &Context) -> Result<(Vec<u8>, &'static str)> {
    checkout_bytes(bytes, &context.attrs, &context.config)
}
const CONVERSION_ATTRIBUTES: [&str; 6] = [
    "text",
    "crlf",
    "eol",
    "filter",
    "working-tree-encoding",
    "ident",
];

pub(crate) fn conversion_attributes<'a>(attrs: &'a [u8]) -> Result<impl Fn(&[u8]) -> &'a [u8]> {
    let fields = attrs.strip_suffix(&[0]).unwrap_or(attrs);
    let records: Vec<_> = fields.split(|b| *b == 0).collect();
    if records.len() != CONVERSION_ATTRIBUTES.len() * 3 {
        return Err(Error::parse("file-preview", "invalid attributes"));
    }
    Ok(move |name: &[u8]| {
        records
            .as_chunks::<3>()
            .0
            .iter()
            .find(|a| a[1] == name)
            .map_or(b"unspecified".as_slice(), |a| a[2])
    })
}

pub(crate) struct LineEndings {
    normalize: bool,
    auto_detect: bool,
    checkout_crlf: bool,
    safe_crlf: bool,
}

pub(crate) fn line_endings(attrs: &[u8], config: &[u8]) -> Result<LineEndings> {
    let attr = conversion_attributes(attrs)?;
    for name in [b"filter".as_slice(), b"working-tree-encoding", b"ident"] {
        if !matches!(attr(name), b"unspecified" | b"unset") {
            return fail(FileEditFailure::Unsupported);
        }
    }
    // Git consults the legacy crlf attribute only when text is unspecified.
    let text = match attr(b"text") {
        b"unspecified" => attr(b"crlf"),
        text => text,
    };
    let eol = attr(b"eol");
    let auto = config_value(config, b"core.autocrlf").to_ascii_lowercase();
    let auto = auto.as_slice();
    let core_eol = config_value(config, b"core.eol").to_ascii_lowercase();
    let safe = config_value(config, b"core.safecrlf").to_ascii_lowercase();
    if !matches!(auto, b"" | b"true" | b"false" | b"input")
        || !matches!(core_eol.as_slice(), b"" | b"lf" | b"crlf" | b"native")
        || !matches!(safe.as_slice(), b"" | b"true" | b"false" | b"warn")
    {
        return fail(FileEditFailure::Unsupported);
    }
    if !matches!(text, b"unspecified" | b"unset" | b"set" | b"auto" | b"input")
        || !matches!(eol, b"unspecified" | b"unset" | b"lf" | b"crlf")
    {
        return fail(FileEditFailure::Unsupported);
    }
    let enabled = text != b"unset"
        && (matches!(text, b"set" | b"auto" | b"input")
            || matches!(eol, b"lf" | b"crlf")
            || matches!(auto, b"true" | b"input"));
    let crlf = eol == b"crlf"
        || (eol != b"lf"
            && text != b"input"
            && (auto == b"true"
                || (auto != b"input"
                    && (core_eol == b"crlf" || (core_eol != b"lf" && cfg!(windows))))));
    Ok(LineEndings {
        normalize: enabled,
        auto_detect: text == b"auto" || (text == b"unspecified" && !matches!(eol, b"lf" | b"crlf")),
        checkout_crlf: crlf,
        safe_crlf: safe == b"true",
    })
}

/// Git's automatic EOL detection counts control bytes separately from line
/// endings. One trailing DOS end-of-file marker is discounted.
#[derive(Default)]
struct TextStats {
    lf: bool,
    crlf: bool,
    bare_cr: bool,
    binary: bool,
}

impl TextStats {
    fn read(bytes: &[u8]) -> Self {
        let mut stats = Self::default();
        let (mut printable, mut controls) = (0usize, 0usize);
        let mut input = bytes.strip_suffix(&[0x1a]).unwrap_or(bytes).iter().copied().peekable();
        while let Some(byte) = input.next() {
            match byte {
                b'\r' if input.next_if_eq(&b'\n').is_some() => stats.crlf = true,
                b'\r' => stats.bare_cr = true,
                b'\n' => stats.lf = true,
                0 => stats.binary = true,
                8 | 9 | 12 | 27 => printable += 1,
                1..=31 | 127 => controls += 1,
                _ => printable += 1,
            }
        }
        stats.binary |= stats.bare_cr || controls > printable / 128;
        stats
    }
}

impl LineEndings {
    pub(crate) fn clean_untracked(&self, bytes: &mut Vec<u8>) -> std::result::Result<(), &'static str> {
        if !self.normalize { return Ok(()); }
        let stats = TextStats::read(bytes);
        if stats.bare_cr {
            return Err("Mixed or bare CR line endings require whole-file staging.");
        }
        if self.auto_detect && stats.binary { return Ok(()); }
        let changes_on_checkout = if self.checkout_crlf { stats.lf } else { stats.crlf };
        if self.safe_crlf && changes_on_checkout {
            return Err("Partial staging would change this file's line endings on checkout (core.safecrlf).");
        }
        if stats.crlf {
            let mut write = 0;
            for read in 0..bytes.len() {
                if bytes[read] == b'\r' && bytes.get(read + 1) == Some(&b'\n') { continue; }
                bytes[write] = bytes[read];
                write += 1;
            }
            bytes.truncate(write);
        }
        Ok(())
    }
}

/// Git blob bytes as a checkout would write them, from `conversion_settings`.
/// Filters, working-tree encodings and ident expansion are refused, never run.
pub(crate) fn checkout_bytes(
    bytes: Vec<u8>,
    attrs: &[u8],
    config: &[u8],
) -> Result<(Vec<u8>, &'static str)> {
    let rules = line_endings(attrs, config)?;
    if !rules.normalize || !rules.checkout_crlf {
        return Ok((bytes, "Git blob bytes (no working-tree filter)"));
    }
    if rules.auto_detect {
        let stats = TextStats::read(&bytes);
        if stats.binary || stats.crlf {
            return Ok((bytes, "Binary/existing CR bytes preserved"));
        }
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
fn config_value<'a>(config: &'a [u8], key: &[u8]) -> &'a [u8] {
    config.rsplit(|&b| b == 0)
        .find_map(|record| record.strip_prefix(key)?.strip_prefix(b"\n"))
        .unwrap_or_default()
}
