//! Apply immutable saved patches through the existing sandbox and file backups.
//! The stash and index are retained; display strings never become patch input.
use crate::{runner::args, *};
use std::{collections::BTreeSet, path::PathBuf};

#[derive(Clone, Debug)]
pub struct StashApplyFile {
    path: RepoPath,
    original: Option<RepoPath>,
    review: ChangeReview,
    hunks: Vec<(String, Vec<usize>)>,
    mode_change: bool,
}
impl StashApplyFile {
    pub fn path(&self) -> &RepoPath {
        &self.path
    }
    pub fn original_path(&self) -> Option<&RepoPath> {
        self.original.as_ref()
    }
    pub fn patch(&self) -> &[u8] {
        self.review.patch()
    }
    pub fn hunk_headers(&self) -> Vec<&str> {
        self.hunks.iter().map(|h| h.0.as_str()).collect()
    }
    pub fn hunk_count(&self) -> usize {
        self.hunks.len()
    }
    /// Exact raw patch line IDs for this hunk; only changed lines are selectable.
    pub fn hunk_line_ids(&self, hunk: usize) -> &[usize] {
        self.hunks.get(hunk).map_or(&[], |h| h.1.as_slice())
    }
    pub fn changed_line_ids(&self) -> Vec<usize> {
        self.hunks
            .iter()
            .flat_map(|h| h.1.iter().copied())
            .collect()
    }
    pub fn whole_only(&self) -> bool {
        self.hunks.is_empty()
    }
    pub fn mode_change(&self) -> bool {
        self.mode_change
    }
}
#[derive(Clone)]
pub struct StashApplyReview {
    repository: PathBuf,
    entry: StashEntry,
    part: StashPart,
    files: Vec<StashApplyFile>,
}
impl std::fmt::Debug for StashApplyReview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StashApplyReview")
            .field("files", &self.files.len())
            .finish_non_exhaustive()
    }
}
impl StashApplyReview {
    pub fn entry(&self) -> &StashEntry {
        &self.entry
    }
    pub fn part(&self) -> StashPart {
        self.part
    }
    pub fn files(&self) -> &[StashApplyFile] {
        &self.files
    }
}
/// Empty hunk AND line lists select the whole file. Otherwise choose either
/// hunks or raw changed-line IDs (not both). All IDs refer to the sealed review.
#[derive(Clone, Debug)]
pub struct StashApplySelection {
    pub file: usize,
    pub hunks: Vec<usize>,
    pub lines: Vec<usize>,
}
#[derive(Clone, Debug)]
pub struct StashApplyPreview {
    review: StashApplyReview,
    patch: PatchPreview,
    head: Head,
}
impl StashApplyPreview {
    pub fn entry(&self) -> &StashEntry {
        self.review.entry()
    }
    pub fn files(&self) -> &[FileEditPreview] {
        self.patch.files()
    }
    pub fn patch(&self) -> &[u8] {
        self.patch.patch()
    }
}

impl Repository {
    pub fn review_stash_apply(
        &self,
        entry: &StashEntry,
        part: StashPart,
        c: &CancellationToken,
    ) -> Result<StashApplyReview> {
        self.runner
            .step("stash-apply-preview", "read_saved_selection", || {
                self.require_worktree(c)?;
                let target = self.stash_diff_target(entry, part, c)?;
                let mut files = Vec::new();
                if let Some(target) = target {
                    let mut a = self.diff_arguments(
                        &DiffOptions {
                            target,
                            context_lines: 3,
                            ..Default::default()
                        },
                        c,
                    )?;
                    a.splice(
                        1..1,
                        args(&["--binary", "--full-index", "--inter-hunk-context=0"]),
                    );
                    a.insert(0, "--no-replace-objects".into());
                    let bytes = self.read_diff("stash-apply-preview", &a, c)?;
                    if !bytes.is_empty() {
                        // Validate the complete patch, including duplicate/aliased paths.
                        let parsed = crate::patches::parse::parse(&bytes, c)?;
                        let lines: Vec<_> = bytes.split_inclusive(|b| *b == b'\n').collect();
                        let starts: Vec<_> = lines
                            .iter()
                            .enumerate()
                            .filter_map(|(i, l)| l.starts_with(b"diff --git ").then_some(i))
                            .chain(Some(lines.len()))
                            .collect();
                        if parsed.len() + 1 != starts.len() {
                            return Err(Error::Patch(PatchFailure::Format));
                        }
                        for (file, range) in parsed.into_iter().zip(starts.windows(2)) {
                            c.check()?;
                            let review = ChangeReview::saved_patch(
                                self,
                                file.path.clone(),
                                lines[range[0]..range[1]].concat(),
                            );
                            let hunks = review.saved_hunks();
                            files.push(StashApplyFile {
                                path: file.path,
                                original: file.original,
                                review,
                                hunks,
                                mode_change: file.mode.is_some(),
                            });
                        }
                    }
                }
                Ok(StashApplyReview {
                    repository: self.info().git_dir.clone(),
                    entry: entry.clone(),
                    part,
                    files,
                })
            })
    }

    pub fn preview_stash_apply(
        &self,
        review: &StashApplyReview,
        selected: &[StashApplySelection],
        c: &CancellationToken,
    ) -> Result<StashApplyPreview> {
        self.runner.step("stash-apply-preview", "review_saved_selection", || {
            if review.repository != self.info().git_dir { return Err(Error::FileEdit(FileEditFailure::WrongRepository)); }
            self.require_idle(c)?;
            if selected.is_empty() { return Err(Error::InvalidInput("Select at least one saved file, hunk or changed line.".into())); }
            let mut seen = BTreeSet::new();
            let mut bytes = Vec::new();
            for selection in selected {
                c.check()?;
                let file = review.files.get(selection.file).filter(|_| seen.insert(selection.file)).ok_or_else(|| Error::InvalidInput("The saved-file selection is invalid. Read the stash again.".into()))?;
                // The patch engine publishes executable-mode changes to the index.
                // This workflow promises to preserve staging, so refuse that case.
                if file.mode_change { return Err(Error::InvalidInput("Selected stash application preserves the index. Apply executable-mode changes with the full-stash workflow instead.".into())); }
                if !selection.hunks.is_empty() && !selection.lines.is_empty() { return Err(Error::InvalidInput("Choose hunks or individual lines for each saved file, not both.".into())); }
                if selection.hunks.is_empty() && selection.lines.is_empty() { bytes.extend_from_slice(file.patch()); }
                else if !selection.lines.is_empty() {
                    let unique: BTreeSet<_> = selection.lines.iter().copied().collect();
                    let allowed: BTreeSet<_> = file.changed_line_ids().into_iter().collect();
                    if unique.len() != selection.lines.len() || !unique.is_subset(&allowed) {
                        return Err(Error::InvalidInput("Choose valid, unique changed lines from this stash review.".into()));
                    }
                    bytes.extend(file.review.selected_patch(&selection.lines)?);
                }
                else {
                    let mut lines = Vec::new();
                    let mut seen_hunks = BTreeSet::new();
                    for &i in &selection.hunks {
                        let h = file.hunks.get(i).filter(|_| seen_hunks.insert(i)).ok_or_else(|| Error::InvalidInput("Choose valid text hunks from this stash review, or apply the whole file.".into()))?;
                        lines.extend_from_slice(&h.1);
                    }
                    bytes.extend(file.review.selected_patch(&lines)?);
                }
            }
            let head = self.head(c)?;
            let patch = self.preview_patch_text(&bytes, c)?;
            if self.head(c)? != head { return Err(Error::FileEdit(FileEditFailure::Stale)); }
            Ok(StashApplyPreview { review: review.clone(), patch, head })
        })
    }

    pub fn apply_stash_selection(
        &self,
        p: &StashApplyPreview,
        c: &CancellationToken,
    ) -> Result<PatchReport> {
        let _permit = self.acquire_write(c)?;
        self.runner
            .step("stash-apply-selection", "apply_saved_selection", || {
                self.require_idle(c)?;
                if p.review.repository != self.info().git_dir {
                    return Err(Error::FileEdit(FileEditFailure::WrongRepository));
                }
                if self.head(c)? != p.head {
                    return Err(Error::FileEdit(FileEditFailure::Stale));
                }
                self.apply_patch_preview_locked(&p.patch, c)
            })
    }
}
