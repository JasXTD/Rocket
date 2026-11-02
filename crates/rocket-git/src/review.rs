//! Byte-preserving, pinned change reviews. Display text is never accepted as a patch.
use crate::{runner::args, *};
use std::{collections::BTreeSet, fmt, io::Read, path::PathBuf};

const REVIEW_LIMIT: usize = 2 * 1024 * 1024;
mod selection;
mod untracked;
use selection::{ChangeKind, parse_hunks, selection_patch};

#[derive(Clone)]
pub struct ChangeReview {
    git_dir: PathBuf,
    path: RepoPath,
    staged: bool,
    patch: Vec<u8>,
    hunks: Vec<Hunk>,
    unavailable: Option<&'static str>,
    kind: ChangeKind,
    paths: Vec<RepoPath>,
    // Raw working bytes and conversion settings pin untracked previews even when
    // normalization produces the same patch. No intent-to-add writes are needed.
    untracked: Option<(Vec<u8>, Vec<u8>, Vec<u8>)>,
}
impl fmt::Debug for ChangeReview {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChangeReview")
            .field("staged", &self.staged)
            .field("hunks", &self.hunks.len())
            .field("unavailable", &self.unavailable)
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Debug)]
struct Hunk {
    old: usize,
    new: usize,
    lines: std::ops::Range<usize>,
}
impl ChangeReview {
    pub(crate) fn saved_patch(repository: &Repository, path: RepoPath, patch: Vec<u8>) -> Self {
        let (hunks, kind, unavailable) = match parse_hunks(&patch) {
            Ok((hunks, ChangeKind::Modified)) => (hunks, ChangeKind::Modified, None),
            _ => (
                Vec::new(),
                ChangeKind::Modified,
                Some(
                    "Choose the whole saved file for additions, deletions, renames or unsupported text.",
                ),
            ),
        };
        Self {
            git_dir: repository.info().git_dir.clone(),
            paths: vec![path.clone()],
            path,
            staged: false,
            patch,
            hunks,
            unavailable,
            kind,
            untracked: None,
        }
    }
    pub(crate) fn saved_hunks(&self) -> Vec<(String, Vec<usize>)> {
        let lines: Vec<_> = self.patch.split(|b| *b == b'\n').collect();
        self.hunks
            .iter()
            .map(|h| {
                (
                    String::from_utf8_lossy(lines[h.lines.start - 1]).into_owned(),
                    h.lines
                        .clone()
                        .filter(|&i| matches!(lines[i].first(), Some(b'+' | b'-')))
                        .collect(),
                )
            })
            .collect()
    }
    pub(crate) fn belongs_to(&self, repository: &Repository) -> bool {
        self.git_dir == repository.info().git_dir
    }
    pub(crate) fn path(&self) -> &RepoPath {
        &self.path
    }
    pub(crate) fn discard_patch(&self, selected: &[usize]) -> Result<Vec<u8>> {
        if self.staged || self.unavailable.is_some() || !self.supports_worktree_selection() {
            return Err(Error::Patch(crate::PatchFailure::Unsupported));
        }
        let chosen: BTreeSet<_> = selected.iter().copied().collect();
        let allowed: BTreeSet<_> = self.changed_lines().into_iter().collect();
        if chosen.is_empty() || !chosen.is_subset(&allowed) {
            return Err(Error::Patch(crate::PatchFailure::Selection));
        }
        let mut reverse = self.clone();
        reverse.staged = true; // The same line-selection transform used for unstage, applied to a sandbox copy.
        selection_patch(&reverse, &chosen)
    }
    pub(crate) fn selected_patch(&self, selected: &[usize]) -> Result<Vec<u8>> {
        // Reuse the validated line IDs and selection transform, never display text.
        self.discard_patch(selected)?;
        selection_patch(self, &selected.iter().copied().collect())
    }
    pub fn patch(&self) -> &[u8] {
        &self.patch
    }
    pub fn staged(&self) -> bool {
        self.staged
    }
    pub fn selection_unavailable(&self) -> Option<&'static str> {
        self.unavailable
    }
    /// Partial working-file replacement stays restricted to regular modifications.
    /// Added/deleted/renamed reviews support index edits only.
    pub fn supports_worktree_selection(&self) -> bool {
        self.kind == ChangeKind::Modified && self.untracked.is_none()
    }
    pub fn selection_note(&self) -> Option<&'static str> {
        match (self.kind, self.staged) {
            (ChangeKind::Added(_), false) => Some(
                "Selected lines create the staged file; working files stay unchanged. Stage a removed old path separately if this is a rename.",
            ),
            (ChangeKind::Added(_), true) => Some(
                "Unstage selected additions; remaining lines stay staged. Selecting all removes this index entry.",
            ),
            (ChangeKind::Deleted(_), false) => Some(
                "Stage selected removals; remaining lines stay in the index. Selecting all stages the file deletion.",
            ),
            (ChangeKind::Deleted(_), true) => {
                Some("Restore selected lines to the index. The working file stays deleted.")
            }
            (ChangeKind::Renamed, true) => Some(
                "Unstage selected text changes at the new path. The rename stays staged; unstage the whole file to undo it.",
            ),
            (ChangeKind::Renamed, false) => {
                Some("Staging selected text also stages this rename. Working files stay unchanged.")
            }
            _ => None,
        }
    }
    /// Zero-based indexes into the original raw patch, including its headers.
    pub fn changed_lines(&self) -> Vec<usize> {
        let lines: Vec<_> = self.patch.split(|b| *b == b'\n').collect();
        self.hunks
            .iter()
            .flat_map(|h| h.lines.clone())
            .filter(|&i| matches!(lines[i].first(), Some(b'+' | b'-')))
            .collect()
    }
}

impl Repository {
    /// Read one exact index stage of a conflicted path. 1=base, 2=ours, 3=theirs.
    pub fn conflict_preview(
        &self,
        path: &RepoPath,
        stage: u8,
        cancel: &CancellationToken,
    ) -> Result<FilePreview> {
        self.require_worktree(cancel)?;
        if !(1..=3).contains(&stage) {
            return Err(Error::InvalidInput(
                "Conflict stage must be base, ours or theirs.".into(),
            ));
        }
        let mut arguments = args(&["ls-files", "--unmerged", "-z", "--"]);
        arguments.push(path.argument()?);
        let output = self
            .runner
            .run(self.cwd(), &arguments, Vec::new(), cancel)?
            .success("read conflict stages")?;
        for record in output.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            let Some(tab) = record.iter().position(|b| *b == b'\t') else {
                continue;
            };
            if record[tab + 1..] != *path.as_bytes() {
                continue;
            }
            let fields: Vec<_> = record[..tab].split(|b| *b == b' ').collect();
            if fields.len() != 3 || fields[2] != [b'0' + stage] {
                continue;
            }
            if !matches!(fields[0], b"100644" | b"100755") {
                return Err(Error::InvalidInput(
                    "This conflict side is not a regular file. Resolve it with Git.".into(),
                ));
            }
            let id = crate::parse::oid(fields[1], "conflict object")?;
            let mut bytes = self
                .runner
                .run(
                    self.cwd(),
                    &args(&["cat-file", "blob", id.as_str()]),
                    Vec::new(),
                    cancel,
                )?
                .success("read conflict side")?;
            let truncated = bytes.len() > REVIEW_LIMIT;
            bytes.truncate(REVIEW_LIMIT);
            return Ok(FilePreview {
                binary: bytes.contains(&0),
                bytes,
                truncated,
            });
        }
        Err(Error::InvalidInput("This side is absent (for example, an add/delete conflict), or the conflict has already been resolved.".into()))
    }

    /// Mark an edited working file resolved, refusing leftover standard conflict
    /// marker blocks. No file content is replaced and no merge is committed.
    pub fn mark_resolved(
        &self,
        path: &RepoPath,
        cancel: &CancellationToken,
    ) -> Result<WriteReport> {
        self.require_worktree(cancel)?;
        let _permit = self.acquire_write(cancel)?;
        let status = self.status(cancel)?;
        if !status
            .entries
            .iter()
            .any(|e| e.path == *path && e.kind == EntryKind::Unmerged)
        {
            return Err(Error::InvalidInput(
                "This path is no longer conflicted. Refresh its status.".into(),
            ));
        }
        let preview = self.preview_untracked(path, cancel)?;
        if preview.truncated {
            return Err(Error::InvalidInput(
                "This file exceeds the resolution preview limit. Review and stage it with Git."
                    .into(),
            ));
        }
        // Allow marker-like ordinary text; reject complete ordered marker blocks.
        let mut marker = 0;
        for line in preview.bytes.split(|b| *b == b'\n') {
            if line.starts_with(b"<<<<<<< ") {
                marker = 1;
            } else if marker == 1 && (line == b"=======" || line == b"=======\r") {
                marker = 2;
            } else if marker == 2 && line.starts_with(b">>>>>>> ") {
                return Err(Error::InvalidInput("Conflict markers remain in this file. Edit the working file, review it, then mark it resolved.".into()));
            }
        }
        let mut input = path.as_bytes().to_vec();
        input.push(0);
        self.execute_write(
            WriteOperation::Stage,
            &args(&["add", "--pathspec-from-file=-", "--pathspec-file-nul", "--"]),
            input,
            cancel,
        )
    }

    /// Read a single tracked file. Unsupported patch types remain viewable, but
    /// must be staged as a whole file; they are never partially reinterpreted.
    pub fn review_change(
        &self,
        path: &RepoPath,
        staged: bool,
        cancel: &CancellationToken,
    ) -> Result<ChangeReview> {
        let mut options = review_options(path, staged);
        let mut patch = self.diff(&options, cancel)?.patch;
        // A destination-only pathspec hides the source of a staged rename. Only
        // additions need this extra status query; ordinary diffs stay fast.
        if staged
            && patch
                .split(|b| *b == b'\n')
                .any(|l| l.starts_with(b"new file mode "))
            && let Some(original) = self
                .status(cancel)?
                .entries
                .iter()
                .find(|e| e.path == *path && e.index == Change::Renamed)
                .and_then(|e| e.original_path.clone())
            {
                options.paths.push(original);
                patch = self.diff(&options, cancel)?.patch;
            }
        let (hunks, kind, unavailable) = match parse_hunks(&patch) {
            Ok((hunks, kind)) => (hunks, kind, None),
            Err(reason) => (Vec::new(), ChangeKind::Modified, Some(reason)),
        };
        Ok(ChangeReview {
            git_dir: self.info().git_dir.clone(),
            path: path.clone(),
            staged,
            patch,
            hunks,
            unavailable,
            kind,
            paths: options.paths,
            untracked: None,
        })
    }

    /// Stage/unstage selected changed lines from a review issued by this repository.
    /// Only the index is written. Git's index lock and patch matching protect
    /// concurrent index writes; working files are never overwritten.
    pub fn apply_review(
        &self,
        review: &ChangeReview,
        selected: &[usize],
        cancel: &CancellationToken,
    ) -> Result<WriteReport> {
        self.require_worktree(cancel)?;
        if review.git_dir != self.info().git_dir {
            return Err(Error::InvalidInput(
                "This review belongs to another repository. Refresh the diff.".into(),
            ));
        }
        if let Some(reason) = review.unavailable {
            return Err(Error::InvalidInput(reason.into()));
        }
        let selected: BTreeSet<_> = selected.iter().copied().collect();
        let allowed: BTreeSet<_> = review.changed_lines().into_iter().collect();
        if selected.is_empty() || !selected.is_subset(&allowed) {
            return Err(Error::InvalidInput(
                "Select one or more added or removed lines from the current review.".into(),
            ));
        }
        let patch = selection_patch(review, &selected)?;
        let _permit = self.acquire_write(cancel)?;
        self.runner.observe("review", "verify current patch", || {
            let matches = if review.untracked.is_some() {
                let current = self.review_untracked(&review.path, cancel)?;
                current.unavailable.is_none() && current.patch == review.patch && current.untracked == review.untracked
            } else {
                let mut options = review_options(&review.path, review.staged);
                options.paths = review.paths.clone();
                self.diff(&options, cancel)?.patch == review.patch
            };
            if !matches {
                return Err(Error::InvalidInput("The file or index changed since this diff was loaded. Refresh and select the lines again.".into()));
            }
            Ok(())
        })?;
        self.execute_write(
            if review.staged {
                WriteOperation::Unstage
            } else {
                WriteOperation::Stage
            },
            &args(&[
                "apply",
                "--cached",
                "--unidiff-zero",
                "--whitespace=nowarn",
                "-",
            ]),
            patch,
            cancel,
        )
    }

    /// A bounded preview, without running content filters or following a final
    /// symlink. On Windows the opened handle's final path is checked as well,
    /// including parent junctions. Bytes are for display only, never a write source.
    pub fn preview_untracked(
        &self,
        path: &RepoPath,
        cancel: &CancellationToken,
    ) -> Result<FilePreview> {
        self.require_worktree(cancel)?;
        let root = self
            .info()
            .worktree
            .as_ref()
            .ok_or(Error::WorktreeRequired)?;
        let full = root.join(path.argument()?);
        let metadata = std::fs::symlink_metadata(&full).map_err(preview_io)?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err(Error::InvalidInput("Preview is available for regular files only; links and directories are not followed.".into()));
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
        }
        let file = options.open(&full).map_err(preview_io)?;
        verify_preview_handle(&file, root, &full)?;
        if !file.metadata().map_err(preview_io)?.is_file() {
            return Err(Error::InvalidInput(
                "Preview requires a regular file.".into(),
            ));
        }
        let mut bytes = Vec::new();
        file.take((REVIEW_LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(preview_io)?;
        cancel.check()?;
        let truncated = bytes.len() > REVIEW_LIMIT;
        bytes.truncate(REVIEW_LIMIT);
        let binary = bytes.contains(&0);
        Ok(FilePreview {
            bytes,
            truncated,
            binary,
        })
    }
}

#[derive(Clone, Debug)]
pub struct FilePreview {
    pub bytes: Vec<u8>,
    pub truncated: bool,
    pub binary: bool,
}
fn preview_io(source: std::io::Error) -> Error {
    Error::Io {
        action: "read file preview",
        source,
    }
}
fn review_options(path: &RepoPath, staged: bool) -> DiffOptions {
    DiffOptions {
        target: if staged {
            DiffTarget::Staged
        } else {
            DiffTarget::Unstaged
        },
        paths: vec![path.clone()],
        context_lines: 3,
    }
}

#[cfg(windows)]
pub(crate) fn verify_preview_handle(
    file: &std::fs::File,
    root: &std::path::Path,
    _path: &std::path::Path,
) -> Result<()> {
    use std::os::windows::{ffi::OsStringExt, fs::MetadataExt, io::AsRawHandle};
    unsafe extern "system" {
        fn GetFinalPathNameByHandleW(
            handle: *mut std::ffi::c_void,
            path: *mut u16,
            size: u32,
            flags: u32,
        ) -> u32;
    }
    if file.metadata().map_err(preview_io)?.file_attributes() & 0x400 != 0 {
        return Err(Error::InvalidInput(
            "Preview does not follow reparse points.".into(),
        ));
    }
    let mut buffer = vec![0_u16; 32768];
    // SAFETY: live file handle and writable buffer of the stated length.
    let size = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle(),
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            0,
        )
    } as usize;
    if size == 0 {
        return Err(preview_io(std::io::Error::last_os_error()));
    }
    if size >= buffer.len() {
        return Err(Error::InvalidInput("Preview path is too long.".into()));
    }
    let opened = PathBuf::from(std::ffi::OsString::from_wide(&buffer[..size]));
    let root = std::fs::canonicalize(root).map_err(preview_io)?;
    if !opened.starts_with(&root) {
        return Err(Error::InvalidInput(
            "Preview path resolves outside this working tree.".into(),
        ));
    }
    Ok(())
}
#[cfg(not(windows))]
pub(crate) fn verify_preview_handle(
    file: &std::fs::File,
    root: &std::path::Path,
    path: &std::path::Path,
) -> Result<()> {
    #[cfg(target_os = "linux")]
    let opened = {
        use std::os::fd::AsRawFd;
        let _ = path;
        std::fs::canonicalize(format!("/proc/self/fd/{}", file.as_raw_fd())).map_err(preview_io)?
    };
    #[cfg(not(target_os = "linux"))]
    let opened = {
        use std::os::unix::fs::MetadataExt;
        let actual = file.metadata().map_err(preview_io)?;
        let expected = std::fs::symlink_metadata(path).map_err(preview_io)?;
        if actual.dev() != expected.dev()
            || actual.ino() != expected.ino()
            || expected.file_type().is_symlink()
        {
            return Err(Error::InvalidInput(
                "Preview path changed. Refresh and try again.".into(),
            ));
        }
        std::fs::canonicalize(path).map_err(preview_io)?
    };
    if !opened.starts_with(std::fs::canonicalize(root).map_err(preview_io)?) {
        return Err(Error::InvalidInput(
            "Preview path resolves outside this working tree.".into(),
        ));
    }
    Ok(())
}
