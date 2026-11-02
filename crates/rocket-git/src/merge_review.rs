//! Read-only merge reviews, pinned to the checkout, index and selected commit.
use crate::{runner::args, *};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeFailure {
    UnbornHead,
    ActiveOperation,
    DirtyWorktree,
    UnsupportedIndex,
    TargetUnavailable,
    UnrelatedHistories,
    NotFastForward,
    StalePreview,
    IndexLocked,
    Conflicts,
    AlreadyCommitted,
    CleanupNotReady,
    CommandFailed,
}
impl MergeFailure {
    pub fn code(self) -> &'static str {
        match self {
            Self::UnbornHead => "merge_unborn_head",
            Self::ActiveOperation => "merge_active_operation",
            Self::DirtyWorktree => "merge_dirty_worktree",
            Self::UnsupportedIndex => "merge_unsupported_index",
            Self::TargetUnavailable => "merge_target_unavailable",
            Self::UnrelatedHistories => "merge_unrelated_histories",
            Self::NotFastForward => "merge_not_fast_forward",
            Self::StalePreview => "merge_stale_preview",
            Self::IndexLocked => "merge_index_locked",
            Self::Conflicts => "merge_conflicts",
            Self::AlreadyCommitted => "merge_already_committed",
            Self::CleanupNotReady => "merge_cleanup_not_ready",
            Self::CommandFailed => "merge_command_failed",
        }
    }
    pub fn guidance(self) -> &'static str {
        match self {
            Self::UnbornHead => "Create the first commit before merging.",
            Self::ActiveOperation => {
                "Finish or abort the active Git operation before preparing another merge."
            }
            Self::DirtyWorktree => {
                "Commit or stash staged and tracked working changes, including submodule changes, then review the merge again."
            }
            Self::UnsupportedIndex => {
                "The index has assume-unchanged or skip-worktree entries that can hide local edits. Review those flags and files with Git first; Rocket has not cleared them."
            }
            Self::TargetUnavailable => {
                "The selected branch or commit could not be resolved locally. Check the reference, fetch explicitly if needed, then review again."
            }
            Self::UnrelatedHistories => {
                "These commits have no common ancestor. Check the selected branch and shallow history first; combining complete unrelated histories requires the explicit Allow unrelated histories option and a new review."
            }
            Self::NotFastForward => {
                "Both sides have commits. Fast-forward only cannot merge them. Review again with a merge-commit mode, or rebase separately."
            }
            Self::StalePreview => {
                "The checkout, index or selected target changed since review. No merge was started. Refresh the preview and review the new state."
            }
            Self::IndexLocked => {
                "Git's index is locked. Let other Git work finish and review again. Rocket leaves existing lock files in place."
            }
            Self::Conflicts => {
                "The merge left conflicts. Resolve them in Changes, stage the resolutions and Continue merge, or review Abort merge. Do not start the merge again."
            }
            Self::AlreadyCommitted => {
                "HEAD already contains the merge commit, but Git's merge markers remain. Use Clear merge state in Changes. Continue and Abort are refused to protect the completed commit; clearing state keeps HEAD, index and working files."
            }
            Self::CleanupNotReady => {
                "No completed merge with leftover markers was found. Refresh Changes and review the current recovery options."
            }
            Self::CommandFailed => {
                "Git did not complete the merge operation. Inspect Changes and the operation state before retrying. Hooks, signing or file protection may have refused it; optional local Git output has details."
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeRelation {
    UpToDate,
    FastForward,
    Diverged,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Context {
    pub(crate) head: Head,
    index: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct MergePreview {
    git_dir: PathBuf,
    common_dir: PathBuf,
    worktree: Option<PathBuf>,
    context: Context,
    revision: String,
    target: Commit,
    mode: MergeMode,
    relation: MergeRelation,
    current_only: u64,
    incoming: u64,
    unrelated: bool,
}
impl MergePreview {
    pub fn unrelated(&self) -> bool { self.unrelated }
    pub fn head(&self) -> &Head {
        &self.context.head
    }
    pub fn revision(&self) -> &str {
        &self.revision
    }
    pub fn target(&self) -> &Commit {
        &self.target
    }
    pub fn mode(&self) -> MergeMode {
        self.mode
    }
    pub fn relation(&self) -> MergeRelation {
        self.relation
    }
    pub fn current_only(&self) -> u64 {
        self.current_only
    }
    pub fn incoming(&self) -> u64 {
        self.incoming
    }
    pub fn can_apply(&self) -> bool {
        self.mode != MergeMode::FastForwardOnly || self.relation != MergeRelation::Diverged
    }
}

impl Repository {
    /// No checkout, index, ref, configuration or object writes. A review describes
    /// ancestry, not a conflict-free guarantee; Git performs the actual merge.
    pub fn preview_merge(
        &self,
        revision: &str,
        mode: MergeMode,
        cancel: &CancellationToken,
    ) -> Result<MergePreview> {
        self.preview_merge_allow_unrelated(revision, mode, false, cancel)
    }
    /// Explicit opt-in applies only to this sealed review; ordinary merge/pull stays unchanged.
    pub fn preview_merge_allow_unrelated(
        &self, revision: &str, mode: MergeMode, allow_unrelated: bool, cancel: &CancellationToken,
    ) -> Result<MergePreview> {
        self.runner.step("merge-preview", "review_merge", || {
            self.require_worktree(cancel)?;
            let context = self.merge_context(cancel)?;
            let preview = self.describe_merge(context, revision, mode, allow_unrelated, cancel)?;
            self.validate_merge_context(&preview, &self.merge_context(cancel)?, cancel)?;
            Ok(preview)
        })
    }

    /// Read immutable ancestry using a freshly checked context. The caller must
    /// validate that context again before publishing this sealed preview.
    pub(crate) fn describe_merge(
        &self, context: Context, revision: &str, mode: MergeMode,
        allow_unrelated: bool, cancel: &CancellationToken,
    ) -> Result<MergePreview> {
        self.runner.step("merge-preview", "read_ancestry", || {
            let original = head_id(&context.head);
            let target = self.merge_target(revision, cancel)?;
            let base = self.runner.run(
                self.cwd(),
                &args(&["merge-base", original.as_str(), target.as_str()]),
                Vec::new(),
                cancel,
            )?;
            let unrelated = base.status.code() == Some(1);
            match base.status.code() {
                Some(0) => {
                    base.success("merge ancestry")?;
                }
                Some(1) if allow_unrelated => self.check_unrelated_history(cancel)?,
                Some(1) => return Err(Error::Merge(MergeFailure::UnrelatedHistories)),
                _ => {
                    base.success("merge ancestry")?;
                }
            }
            let range = format!("{original}...{target}");
            let counts = self.merge_read(
                &["rev-list", "--left-right", "--count", &range, "--"],
                cancel,
            )?;
            let counts: Vec<_> = std::str::from_utf8(&counts)
                .ok()
                .into_iter()
                .flat_map(str::split_whitespace)
                .collect();
            let invalid = || Error::parse("merge ancestry", "expected two commit counts");
            if counts.len() != 2 {
                return Err(invalid());
            }
            let current_only: u64 = counts[0].parse().map_err(|_| invalid())?;
            let incoming: u64 = counts[1].parse().map_err(|_| invalid())?;
            let relation = if incoming == 0 {
                MergeRelation::UpToDate
            } else if current_only == 0 {
                MergeRelation::FastForward
            } else {
                MergeRelation::Diverged
            };
            let objects = self
                .runner
                .run(
                    self.cwd(),
                    &args(&["cat-file", "--batch"]),
                    format!("{target}\n").into_bytes(),
                    cancel,
                )?
                .success("merge target")?;
            let target = parse::commits(&objects, &[target])?.remove(0);
            Ok(MergePreview {
                git_dir: self.info().git_dir.clone(),
                common_dir: self.info().common_dir.clone(),
                worktree: self.info().worktree.clone(),
                context,
                revision: revision.into(),
                target,
                mode,
                relation,
                current_only,
                incoming,
                unrelated,
            })
        })
    }

    pub(crate) fn validate_merge_context(
        &self, preview: &MergePreview, context: &Context, cancel: &CancellationToken,
    ) -> Result<()> {
        // Retain the target re-resolution as well as HEAD/index equality.
        if *context != preview.context
            || self.merge_target(&preview.revision, cancel)? != preview.target.id
        {
            return Err(Error::Merge(MergeFailure::StalePreview));
        }
        Ok(())
    }

    /// Pull already checks exact operation refs, direct HEAD, repeated HEAD
    /// observations and clean status. Add the remaining merge index guards once.
    pub(crate) fn pull_merge_context(&self, cancel: &CancellationToken) -> Result<Context> {
        let (name, target) = self.pull_state(cancel)?;
        Ok(Context { head: Head::Branch { name, target }, index: self.merge_index(cancel)? })
    }

    /// Revalidate under the common-directory write gate and pass only the reviewed
    /// object ID to Git. Cancellation is not rollback; inspect all report fields.
    pub fn apply_merge(
        &self,
        preview: &MergePreview,
        cancel: &CancellationToken,
    ) -> Result<MergeWriteReport> {
        self.require_worktree(cancel)?;
        let _permit = self.acquire_write(cancel)?;
        self.apply_merge_locked(preview, cancel)
    }

    /// Caller holds the common-directory write gate, including any enclosing workflow checks.
    pub(crate) fn apply_merge_locked(
        &self,
        preview: &MergePreview,
        cancel: &CancellationToken,
    ) -> Result<MergeWriteReport> {
        self.runner.step("merge", "revalidate_merge", || {
            if self.info().git_dir != preview.git_dir
                || self.info().common_dir != preview.common_dir
                || self.info().worktree != preview.worktree
                || self.merge_context(cancel)? != preview.context
                || self.merge_target(&preview.revision, cancel)? != preview.target.id
            {
                return Err(Error::Merge(MergeFailure::StalePreview));
            }
            if !preview.can_apply() {
                return Err(Error::Merge(MergeFailure::NotFastForward));
            }
            cancel.check()
        })?;
        let mut arguments = self.merge_commit_arguments(&preview.target.id, preview.mode, cancel)?;
        if preview.unrelated {
            self.check_unrelated_history(cancel)?;
            let separator = arguments.iter().position(|a| a == "--").expect("merge separator");
            arguments.insert(separator, "--allow-unrelated-histories".into());
        }
        self.execute_merge_write(WriteOperation::Merge, &arguments, Vec::new(), cancel)
    }

    fn check_unrelated_history(&self, c: &CancellationToken) -> Result<()> {
        if self.merge_read(&["rev-parse", "--is-shallow-repository"], c)? != b"false\n"
            || !self.merge_read(&["for-each-ref", "--format=%(refname)", "refs/replace/"], c)?.is_empty()
            || self.info().common_dir.join("info/grafts").try_exists().map_err(|e|Error::io("inspect grafted history",e))? {
            return Err(Error::Unsupported("merge_unrelated_history: fetch complete history and remove replacement/graft ambiguity before combining different roots.".into()));
        }
        Ok(())
    }

    pub(crate) fn check_merge_checkout(&self, cancel: &CancellationToken) -> Result<()> {
        self.merge_context(cancel).map(|_| ())
    }

    fn merge_context(&self, cancel: &CancellationToken) -> Result<Context> {
        if !self.operation_state(cancel)?.is_idle() {
            return Err(Error::Merge(MergeFailure::ActiveOperation));
        }
        if self
            .info()
            .git_dir
            .join("index.lock")
            .try_exists()
            .map_err(|e| Error::io("inspect merge index lock", e))?
        {
            return Err(Error::Merge(MergeFailure::IndexLocked));
        }
        let status = self.status(cancel)?;
        if matches!(status.head, Head::Unborn { .. }) {
            return Err(Error::Merge(MergeFailure::UnbornHead));
        }
        if status
            .entries
            .iter()
            .any(|e| !matches!(e.kind, EntryKind::Untracked | EntryKind::Ignored))
        {
            return Err(Error::Merge(MergeFailure::DirtyWorktree));
        }
        Ok(Context { head: status.head, index: self.merge_index(cancel)? })
    }

    fn merge_index(&self, cancel: &CancellationToken) -> Result<Vec<u8>> {
        if self.info().git_dir.join("index.lock").try_exists()
            .map_err(|e| Error::io("inspect merge index lock", e))?
        {
            return Err(Error::Merge(MergeFailure::IndexLocked));
        }
        let index = self.merge_read(&["ls-files", "--stage", "-v", "-z"], cancel)?;
        if index
            .split(|b| *b == 0)
            .filter_map(|e| e.first())
            .any(|tag| tag.is_ascii_lowercase() || *tag == b'S')
        {
            return Err(Error::Merge(MergeFailure::UnsupportedIndex));
        }
        Ok(index)
    }
    fn merge_target(&self, revision: &str, cancel: &CancellationToken) -> Result<ObjectId> {
        self.resolve_commit(revision, cancel).map_err(|e| match e {
            Error::Git { .. } => Error::Merge(MergeFailure::TargetUnavailable),
            other => other,
        })
    }
    fn merge_read(&self, arguments: &[&str], cancel: &CancellationToken) -> Result<Vec<u8>> {
        self.runner
            .run(self.cwd(), &args(arguments), Vec::new(), cancel)?
            .success("merge preview")
    }
}

fn head_id(head: &Head) -> &ObjectId {
    match head {
        Head::Branch { target, .. } | Head::Detached { target } => target,
        Head::Unborn { .. } => unreachable!("merge_context rejects unborn HEAD"),
    }
}
