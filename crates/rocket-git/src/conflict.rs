//! Repository-bound conflict resolution: text editing and keeping one whole
//! version. Saves are checked, recoverable writes; they never commit or finish
//! the surrounding Git operation.
use crate::{runner::args, *};
use std::{fmt, path::PathBuf};
mod file;
mod markers;
mod side;
mod index;
pub use markers::{ConflictBlock, ConflictBlocks, conflict_blocks};
pub use side::{
    CONFLICT_FILE_LIMIT, ConflictKeep, ConflictVersion, WholeConflictPreview, WholeConflictReport,
    WholeFileChange, ConflictKind,
};

pub const CONFLICT_TEXT_LIMIT: usize = 512 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictFailure {
    FileChanged,
    ContextChanged,
    NotConflicted,
    MarkersRemain,
    UnsupportedFile,
    WrongRepository,
    UnsupportedVersion,
    VersionUnavailable,
    ReviewStale,
    ResolutionUnverified,
}
impl ConflictFailure {
    pub fn code(self) -> &'static str {
        match self {
            Self::FileChanged => "conflict_file_changed",
            Self::ContextChanged => "conflict_context_changed",
            Self::NotConflicted => "not_conflicted",
            Self::MarkersRemain => "conflict_markers_remain",
            Self::UnsupportedFile => "unsupported_conflict_file",
            Self::WrongRepository => "wrong_repository",
            Self::UnsupportedVersion => "unsupported_conflict_version",
            Self::VersionUnavailable => "conflict_version_unavailable",
            Self::ReviewStale => "conflict_review_stale",
            Self::ResolutionUnverified => "conflict_resolution_unverified",
        }
    }
    pub fn guidance(self) -> &'static str {
        match self {
            Self::FileChanged => {
                "The working file changed outside this editor. Your draft is retained. Compare the current file or reload before saving again."
            }
            Self::ContextChanged => {
                "The branch, conflict stages or Git operation changed. Your draft is retained. Reload and review the current conflict before saving."
            }
            Self::NotConflicted => {
                "This file is no longer conflicted. Refresh Changes; your editor draft is retained."
            }
            Self::MarkersRemain => {
                "Conflict markers remain or are incomplete. Resolve them before Save and stage; Save file can retain unfinished work."
            }
            Self::UnsupportedFile => {
                "The conflict editor edits regular UTF-8 text up to 512 KiB and 20,000 lines, with LF or CRLF endings. For a binary or deleted file, keep one whole version instead. Mixed-ending, symbolic-link or special working files require an external editor or Git."
            }
            Self::WrongRepository => {
                "This conflict preview belongs to another repository or worktree. Open a new preview here."
            }
            Self::UnsupportedVersion => {
                "Keep a regular file or verified local Git LFS object up to 16 MiB, or stage a submodule commit without changing its checkout. Other filters, encodings, ident expansion, symbolic links and mixed file/submodule types need Git."
            }
            Self::VersionUnavailable => {
                "That version is not part of this conflict. Keep an available side; the deletion is offered only when one side deleted the file."
            }
            Self::ReviewStale => {
                "The working file, conflict stages, branch, Git operation or conversion settings changed after this review. Nothing was written. Reload and review the conflict again."
            }
            Self::ResolutionUnverified => {
                "The working file or index did not end exactly as reviewed. Inspect Changes, the working file and the retained recovery copies before retrying; nothing was rolled back."
            }
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Context {
    head: Head,
    stages: Vec<u8>,
    state: OperationState,
}
#[derive(Clone)]
pub struct ConflictPreview {
    git_dir: PathBuf,
    path: RepoPath,
    context: Context,
    original: Vec<u8>,
    working: String,
    sides: [Option<String>; 3],
    crlf: bool,
    bom: bool,
}
impl fmt::Debug for ConflictPreview {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConflictPreview")
            .field("bytes", &self.original.len())
            .finish_non_exhaustive()
    }
}
impl ConflictPreview {
    pub fn path(&self) -> &RepoPath {
        &self.path
    }
    pub fn working(&self) -> &str {
        &self.working
    }
    /// The working file's bytes as reviewed; a save requires them unchanged.
    pub fn original(&self) -> &[u8] {
        &self.original
    }
    pub fn side(&self, stage: u8) -> Option<&str> {
        self.sides.get(stage.checked_sub(1)? as usize)?.as_deref()
    }
    pub fn crlf(&self) -> bool {
        self.crlf
    }
    pub fn bom(&self) -> bool {
        self.bom
    }
    fn encoded(&self, text: &str) -> Result<Vec<u8>> {
        let (mut text, _, leading_bom) = decode(text.as_bytes())?;
        // The editor has already removed the original BOM. A newly typed FEFF
        // is literal result content and must not silently disappear.
        if leading_bom {
            text.insert(0, '\u{feff}');
        }
        let mut bytes = if self.bom {
            vec![0xef, 0xbb, 0xbf]
        } else {
            Vec::new()
        };
        bytes.extend_from_slice(
            if self.crlf {
                text.replace('\n', "\r\n")
            } else {
                text
            }
            .as_bytes(),
        );
        if bytes.len() > CONFLICT_TEXT_LIMIT {
            return Err(Error::Conflict(ConflictFailure::UnsupportedFile));
        }
        Ok(bytes)
    }
}
pub struct ConflictSaveReport {
    /// True once publication is confirmed; later cancellation never turns it false.
    pub saved: bool,
    /// True only after Git confirms success. False may mean staging never ran,
    /// failed, or was interrupted after an index write; inspect refreshed status.
    pub staged: bool,
    /// Updated binding after a confirmed file-only save. Later external edits
    /// still fail the next save's byte/context checks.
    pub preview: Option<ConflictPreview>,
    pub recovery_directory: Option<PathBuf>,
    pub result: Result<()>,
    pub status: Result<Status>,
    pub state: Result<OperationState>,
}

pub(crate) fn decode(bytes: &[u8]) -> Result<(String, bool, bool)> {
    let unsupported = || Error::Conflict(ConflictFailure::UnsupportedFile);
    if bytes.len() > CONFLICT_TEXT_LIMIT || bytes.contains(&0) {
        return Err(unsupported());
    }
    let bom = bytes.starts_with(b"\xef\xbb\xbf");
    let text =
        std::str::from_utf8(if bom { &bytes[3..] } else { bytes }).map_err(|_| unsupported())?;
    let crlf = text.contains("\r\n");
    let normalized = text.replace("\r\n", "\n");
    if normalized.contains('\r')
        || normalized.lines().count() > 20_000
        || (crlf && text.replace("\r\n", "").contains('\n'))
    {
        return Err(unsupported());
    }
    Ok((normalized, crlf, bom))
}
impl Repository {
    /// Check a retained external resolution against the original binding without writing.
    pub fn validate_conflict_preview(
        &self,
        preview: &ConflictPreview,
        cancel: &CancellationToken,
    ) -> Result<()> {
        cancel.check()?;
        if preview.git_dir != self.info().git_dir {
            return Err(Error::Conflict(ConflictFailure::WrongRepository));
        }
        if self.conflict_context(&preview.path, cancel)? != preview.context {
            return Err(Error::Conflict(ConflictFailure::ContextChanged));
        }
        if file::read(self, &preview.path)? != preview.original {
            return Err(Error::Conflict(ConflictFailure::FileChanged));
        }
        cancel.check()
    }
    fn conflict_context(&self, path: &RepoPath, cancel: &CancellationToken) -> Result<Context> {
        let mut arguments = args(&["ls-files", "--unmerged", "-z", "--"]);
        arguments.push(path.argument()?);
        let stages = self
            .runner
            .run(self.cwd(), &arguments, vec![], cancel)?
            .success("read conflict stages")?;
        if stages.is_empty() {
            return Err(Error::Conflict(ConflictFailure::NotConflicted));
        }
        let head = self.head(cancel)?;
        Ok(Context {
            head,
            stages,
            state: self.operation_state(cancel)?,
        })
    }
    pub fn edit_conflict(
        &self,
        path: &RepoPath,
        cancel: &CancellationToken,
    ) -> Result<ConflictPreview> {
        self.require_worktree(cancel)?;
        self.runner.step("conflict-edit", "read_conflict", || {
            let context = self.conflict_context(path, cancel)?;
            if context.stages.split(|b| *b == 0).any(|record| record.starts_with(b"160000 ")) {
                return Err(Error::Conflict(ConflictFailure::UnsupportedFile));
            }
            if side::lfs_attributes(&self.conversion_settings(path, cancel)?.0)? {
                return Err(Error::Conflict(ConflictFailure::UnsupportedFile));
            }
            let original = file::read(self, path)?;
            let (working, crlf, bom) = decode(&original)?;
            let mut sides = [None, None, None];
            for record in context.stages.split(|b| *b == 0).filter(|r| !r.is_empty()) {
                let tab = record
                    .iter()
                    .position(|b| *b == b'\t')
                    .ok_or_else(|| Error::parse("conflict stages", "missing path"))?;
                if &record[tab + 1..] != path.as_bytes() {
                    return Err(Error::parse("conflict stages", "unexpected path"));
                }
                let fields: Vec<_> = record[..tab].split(|b| *b == b' ').collect();
                if fields.len() != 3
                    || !matches!(fields[0], b"100644" | b"100755")
                    || !matches!(fields[2], b"1" | b"2" | b"3")
                {
                    return Err(Error::Conflict(ConflictFailure::UnsupportedFile));
                }
                let stage = (fields[2][0] - b'1') as usize;
                if sides[stage].is_some() {
                    return Err(Error::parse("conflict stages", "duplicate stage"));
                }
                let id = crate::parse::oid(fields[1], "conflict object")?;
                let mut runner = self.runner.clone();
                runner.limits.stdout_bytes = CONFLICT_TEXT_LIMIT + 1;
                let bytes = runner
                    .run(
                        self.cwd(),
                        &args(&["cat-file", "blob", id.as_str()]),
                        vec![],
                        cancel,
                    )?
                    .success("read conflict side")?;
                sides[stage] = Some(decode(&bytes)?.0);
            }
            cancel.check()?;
            if file::read(self, path)? != original {
                return Err(Error::Conflict(ConflictFailure::FileChanged));
            }
            if self.conflict_context(path, cancel)? != context {
                return Err(Error::Conflict(ConflictFailure::ContextChanged));
            }
            Ok(ConflictPreview {
                git_dir: self.info().git_dir.clone(),
                path: path.clone(),
                context,
                original,
                working,
                sides,
                crlf,
                bom,
            })
        })
    }
    pub fn save_conflict(
        &self,
        preview: &ConflictPreview,
        text: &str,
        stage: bool,
        cancel: &CancellationToken,
    ) -> Result<ConflictSaveReport> {
        self.require_worktree(cancel)?;
        if preview.git_dir != self.info().git_dir {
            return Err(Error::Conflict(ConflictFailure::WrongRepository));
        }
        let _permit = self.acquire_write(cancel)?;
        let bytes = preview.encoded(text)?;
        if stage && !conflict_blocks(text).is_resolved() {
            return Err(Error::Conflict(ConflictFailure::MarkersRemain));
        }
        let mut saved = false;
        let mut staged = false;
        let mut recovery_directory = None;
        let result = self.runner.observe("write_result", "conflict-save", || {
            self.runner
                .step("conflict-save", "revalidate_conflict", || {
                    cancel.check()?;
                    if self.conflict_context(&preview.path, cancel)? != preview.context {
                        return Err(Error::Conflict(ConflictFailure::ContextChanged));
                    }
                    if file::read(self, &preview.path)? != preview.original {
                        return Err(Error::Conflict(ConflictFailure::FileChanged));
                    }
                    Ok(())
                })?;
            self.runner
                .step("conflict-save", "publish_resolution", || {
                    file::publish(
                        self,
                        preview,
                        &bytes,
                        cancel,
                        &mut saved,
                        &mut recovery_directory,
                    )
                })?;
            if stage {
                self.runner.step("conflict-save", "stage_resolution", || {
                    cancel.check()?;
                    if self.conflict_context(&preview.path, cancel)? != preview.context {
                        return Err(Error::Conflict(ConflictFailure::ContextChanged));
                    }
                    // Prevent both replacement and in-place edits while Git reads
                    // the saved file. Git takes its own index lock for staging.
                    let opened = file::for_staging(self, &preview.path)?;
                    if file::bytes(&opened.file)? != bytes {
                        return Err(Error::Conflict(ConflictFailure::FileChanged));
                    }
                    let mut input = preview.path.as_bytes().to_vec();
                    input.push(0);
                    self.runner
                        .write(
                            self.cwd(),
                            &args(&["add", "--pathspec-from-file=-", "--pathspec-file-nul", "--"]),
                            input,
                            cancel,
                        )?
                        .success("stage conflict resolution")?;
                    staged = true;
                    Ok(())
                })?;
            }
            Ok(())
        });
        let status = self.runner.observe("refresh", "status", || {
            self.status(&CancellationToken::new())
        });
        let state = self.runner.observe("refresh", "operation", || {
            self.operation_state(&CancellationToken::new())
        });
        let next = if result.is_ok() && saved && !stage {
            let mut next = preview.clone();
            next.original = bytes;
            next.working = text.replace("\r\n", "\n");
            Some(next)
        } else {
            None
        };
        Ok(ConflictSaveReport {
            saved,
            staged,
            preview: next,
            recovery_directory,
            result,
            status,
            state,
        })
    }
}
