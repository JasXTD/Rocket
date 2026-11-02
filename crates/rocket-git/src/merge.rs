use crate::{model::bytes_to_os, runner::args, write::validate_commit_message, *};
use std::ffi::OsString;

impl Repository {
    /// Merge one pinned commit with Git's ort strategy. Requires clean tracked
    /// files/index and no active operation. Untracked and ignored collisions are
    /// protected by Git. Hooks/signing still apply; no autostash or editor is used.
    pub fn merge(
        &self,
        target: &str,
        mode: MergeMode,
        cancel: &CancellationToken,
    ) -> Result<MergeWriteReport> {
        self.require_worktree(cancel)?;
        let _permit = self.acquire_write(cancel)?;
        self.require_idle(cancel)?;
        let status = self.status(cancel)?;
        if matches!(status.head, Head::Unborn { .. }) {
            return Err(Error::InvalidInput(
                "create the first commit before merging".into(),
            ));
        }
        if status
            .entries
            .iter()
            .any(|e| !matches!(e.kind, EntryKind::Untracked | EntryKind::Ignored))
        {
            return Err(Error::InvalidInput(
                "commit or stash tracked changes before merging".into(),
            ));
        }
        let target = self.resolve_commit(target, cancel)?;
        let arguments = self.merge_commit_arguments(&target, mode, cancel)?;
        self.execute_merge_write(WriteOperation::Merge, &arguments, Vec::new(), cancel)
    }

    pub(crate) fn merge_commit_arguments(
        &self,
        target: &ObjectId,
        mode: MergeMode,
        cancel: &CancellationToken,
    ) -> Result<Vec<OsString>> {
        self.merge_commit_arguments_with_message(target, mode, None, cancel)
    }

    /// [`Repository::merge_commit_arguments`]; with a message, Git uses it for the merge
    /// commit instead of its default text for an object ID ("Merge commit '…'").
    pub(crate) fn merge_commit_arguments_with_message(
        &self,
        target: &ObjectId,
        mode: MergeMode,
        message: Option<&str>,
        cancel: &CancellationToken,
    ) -> Result<Vec<OsString>> {
        let mut arguments = self.merge_arguments(cancel)?;
        arguments.extend(args(&[
            "merge",
            "--no-edit",
            "--no-autostash",
            "--no-overwrite-ignore",
            "--no-rerere-autoupdate",
            if mode == MergeMode::Squash { "--squash" } else { "--no-squash" },
            if matches!(mode, MergeMode::Squash | MergeMode::NoCommit) { "--no-commit" } else { "--commit" },
            "--verify",
            "--strategy=ort",
            "--no-stat",
            match mode {
                MergeMode::FastForwardOnly => "--ff-only",
                MergeMode::AllowFastForward => "--ff",
                MergeMode::NoFastForward => "--no-ff",
                MergeMode::Squash => "--ff",
                MergeMode::NoCommit => "--no-ff",
            },
        ]));
        if let Some(message) = message {
            arguments.extend(["-m".into(), message.into()]);
        }
        arguments.extend(args(&["--", target.as_str()]));
        Ok(arguments)
    }

    /// Finish an existing merge by committing the resolved index with this message.
    /// Git refuses unresolved conflicts and preserves its hooks/signing behavior.
    pub fn continue_merge(
        &self,
        message: &str,
        cancel: &CancellationToken,
    ) -> Result<MergeWriteReport> {
        validate_commit_message(message)?;
        self.require_worktree(cancel)?;
        let _permit = self.acquire_write(cancel)?;
        if self.require_merge(cancel)?.merge_committed {
            return Err(Error::Merge(MergeFailure::AlreadyCommitted));
        }
        self.execute_merge_write(
            WriteOperation::ContinueMerge,
            &args(&[
                "-c",
                "submodule.recurse=false",
                "commit",
                "--file=-",
                "--no-edit",
                "--cleanup=whitespace",
            ]),
            message.as_bytes().to_vec(),
            cancel,
        )
    }

    /// Explicitly abandon the current merge and its staged conflict resolutions.
    /// Uses Git's merge-abort recovery, never a hard reset or lock-file deletion.
    pub fn abort_merge(&self, cancel: &CancellationToken) -> Result<MergeWriteReport> {
        self.require_worktree(cancel)?;
        let _permit = self.acquire_write(cancel)?;
        if self.require_merge(cancel)?.merge_committed {
            return Err(Error::Merge(MergeFailure::AlreadyCommitted));
        }
        let mut arguments = self.merge_arguments(cancel)?;
        arguments.extend(args(&["merge", "--abort"]));
        self.execute_merge_write(WriteOperation::AbortMerge, &arguments, Vec::new(), cancel)
    }

    /// Clear leftover markers after a completed merge. Git --quit does not reset
    /// HEAD or restore files/index. Autostash is refused by require_merge.
    pub fn finish_committed_merge(&self, cancel: &CancellationToken) -> Result<MergeWriteReport> {
        self.require_worktree(cancel)?;
        let _permit = self.acquire_write(cancel)?;
        self.runner
            .step("merge-finish", "check_committed_merge", || {
                if !self.require_merge(cancel)?.merge_committed {
                    return Err(Error::Merge(MergeFailure::CleanupNotReady));
                }
                cancel.check()
            })?;
        let mut arguments = self.merge_arguments(cancel)?;
        arguments.extend(args(&["merge", "--quit"]));
        self.execute_merge_write(WriteOperation::FinishMerge, &arguments, Vec::new(), cancel)
    }

    fn require_merge(&self, cancel: &CancellationToken) -> Result<OperationState> {
        let state = self.operation_state(cancel)?;
        if state.merge_heads.is_empty()
            || state.rebase
            || state.mail_import
            || state.cherry_pick
            || state.revert
            || state.sequencer
        {
            return Err(Error::InvalidInput(
                "no standalone merge is in progress".into(),
            ));
        }
        // Rocket never creates an autostash. Taking over one requires an explicit
        // recovery workflow, otherwise finishing can unexpectedly apply other edits.
        if state.merge_autostash.is_some() {
            return Err(Error::Unsupported(
                "finish this externally autostashed merge with Git first".into(),
            ));
        }
        Ok(state)
    }

    fn merge_arguments(&self, cancel: &CancellationToken) -> Result<Vec<OsString>> {
        let mut arguments = args(&["-c", "submodule.recurse=false"]);
        if let Head::Branch { name, .. } = self.head(cancel)? {
            // Branch mergeOptions can inject strategies, squash, no-verify, or
            // auto-stashing. Empty that default for this invocation, not on disk.
            // Unlike `-c key=value`, --config-env splits at the last '='. Branch
            // names may contain '='; keep their entire config subsection literal.
            let mut setting = b"--config-env=branch.".to_vec();
            setting.extend(name);
            setting.extend_from_slice(b".mergeOptions=");
            setting.extend_from_slice(crate::runner::EMPTY_CONFIG_ENV.as_bytes());
            arguments.push(bytes_to_os(&setting)?);
        }
        Ok(arguments)
    }

    pub(crate) fn execute_merge_write(
        &self,
        operation: WriteOperation,
        arguments: &[OsString],
        input: Vec<u8>,
        cancel: &CancellationToken,
    ) -> Result<MergeWriteReport> {
        let write = self.execute_write(operation, arguments, input, cancel)?;
        let state = self.runner.observe("refresh", "operation", || {
            self.operation_state(&CancellationToken::new())
        });
        let branches = self.runner.observe("refresh", "branches", || {
            self.branches(&CancellationToken::new())
        });
        let failure = if state.as_ref().is_ok_and(|s| s.merge_committed) {
            Some(MergeFailure::AlreadyCommitted)
        } else if write
            .status
            .as_ref()
            .is_ok_and(|s| s.entries.iter().any(|e| e.kind == EntryKind::Unmerged))
            && state.as_ref().is_ok_and(|s| !s.merge_heads.is_empty())
        {
            Some(MergeFailure::Conflicts)
        } else if let Err(Error::Git { stderr, .. }) = &write.result {
            Some(if String::from_utf8_lossy(stderr).contains("index.lock") {
                MergeFailure::IndexLocked
            } else {
                MergeFailure::CommandFailed
            })
        } else {
            None
        };
        self.runner
            .step("merge", "inspect_merge_result", || {
                failure.map_or(Ok(()), |failure| Err(Error::Merge(failure)))
            })
            .ok();
        Ok(MergeWriteReport {
            write,
            state,
            branches,
            failure,
        })
    }
}
