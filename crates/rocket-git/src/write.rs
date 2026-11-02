use crate::{runner::args, *};
use std::ffi::OsString;

const MAX_PATH_BYTES: usize = 8 * 1024 * 1024;
/// Largest UTF-8 commit message accepted by Rocket's commit writer and composer.
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

impl Repository {
    /// Stage exactly these literal paths (directories include their descendants).
    /// An empty selection is an error. Ignored files are not forced into the index.
    pub fn stage(&self, paths: &[RepoPath], cancel: &CancellationToken) -> Result<WriteReport> {
        let input = path_input(paths)?;
        self.stage_impl(Some(input), cancel)
    }

    /// Explicitly stage additions, modifications, and deletions across this worktree.
    pub fn stage_all(&self, cancel: &CancellationToken) -> Result<WriteReport> {
        self.stage_impl(None, cancel)
    }

    fn stage_impl(
        &self,
        paths: Option<Vec<u8>>,
        cancel: &CancellationToken,
    ) -> Result<WriteReport> {
        self.require_worktree(cancel)?;
        let _permit = self.acquire_write(cancel)?;
        let (arguments, input) = selection(args(&["add", "--all"]), paths);
        self.execute_write(WriteOperation::Stage, &arguments, input, cancel)
    }

    /// Reset these index paths to HEAD without changing working files.
    /// For a rename, pass both the old and new paths to unstage the entire rename.
    pub fn unstage(&self, paths: &[RepoPath], cancel: &CancellationToken) -> Result<WriteReport> {
        let input = path_input(paths)?;
        self.unstage_impl(Some(input), cancel)
    }

    pub fn unstage_all(&self, cancel: &CancellationToken) -> Result<WriteReport> {
        self.unstage_impl(None, cancel)
    }

    fn unstage_impl(
        &self,
        paths: Option<Vec<u8>>,
        cancel: &CancellationToken,
    ) -> Result<WriteReport> {
        self.require_worktree(cancel)?;
        let _permit = self.acquire_write(cancel)?;
        let arguments = match self.head(cancel)? {
            Head::Unborn { .. } => {
                // There is no HEAD tree to restore. Remove only index entries, even
                // if the staged bytes differ from disk. Never remove working files.
                args(&[
                    "rm",
                    "--cached",
                    "--force",
                    "--quiet",
                    "--ignore-unmatch",
                    "-r",
                ])
            }
            Head::Branch { target, .. } | Head::Detached { target } => {
                args(&["restore", "--staged", &format!("--source={target}")])
            }
        };
        let (arguments, input) = selection(arguments, paths);
        self.execute_write(WriteOperation::Unstage, &arguments, input, cancel)
    }

    /// Commit the current index using Git's identity, hooks, and signing configuration.
    /// Does not stage files, amend history, bypass hooks, or allow an empty commit.
    pub fn commit(&self, message: &str, cancel: &CancellationToken) -> Result<WriteReport> {
        validate_commit_message(message)?;
        self.require_worktree(cancel)?;
        let _permit = self.acquire_write(cancel)?;
        self.commit_locked(message, cancel)
    }

    /// Caller holds the repository write permit across the composed operation.
    pub(crate) fn commit_locked(&self, message: &str, cancel: &CancellationToken) -> Result<WriteReport> {
        validate_commit_message(message)?;
        self.require_worktree(cancel)?;
        // Message bytes go to stdin; they are never interpreted as arguments or shell text.
        // No editor is launched. Whitespace cleanup keeps literal # lines in the message.
        self.execute_write(
            WriteOperation::Commit,
            &args(&["commit", "--file=-", "--no-edit", "--cleanup=whitespace"]),
            message.as_bytes().to_vec(),
            cancel,
        )
    }

    pub(crate) fn require_worktree(&self, cancel: &CancellationToken) -> Result<()> {
        cancel.check()?;
        if self.info().worktree.is_none() {
            return Err(Error::WorktreeRequired);
        }
        Ok(())
    }

    pub(crate) fn acquire_write(
        &self,
        cancel: &CancellationToken,
    ) -> Result<crate::write_gate::WritePermit<'_>> {
        self.runner
            .observe("write_queue", "acquire", || self.write_gate.acquire(cancel))
    }

    pub(crate) fn execute_write(
        &self,
        operation: WriteOperation,
        arguments: &[OsString],
        input: Vec<u8>,
        cancel: &CancellationToken,
    ) -> Result<WriteReport> {
        self.execute_write_with_editor(operation, arguments, input, cancel, None)
    }

    pub(crate) fn execute_write_with_editor(
        &self,
        operation: WriteOperation,
        arguments: &[OsString],
        input: Vec<u8>,
        cancel: &CancellationToken,
        editor: Option<&crate::rebase_plan::Editor>,
    ) -> Result<WriteReport> {
        // Outer errors are preflight/queue failures. After this point, conservatively
        // report the attempted operation and reconcile, including spawn/I/O failures.
        if self.is_bisecting(cancel)? {
            return Err(Error::Bisect(BisectFailure::ActiveOperation));
        }
        cancel.check()?;
        let label = match operation {
            WriteOperation::Stage => "stage",
            WriteOperation::Unstage => "unstage",
            WriteOperation::Commit => "commit",
            WriteOperation::CreateBranch => "create branch",
            WriteOperation::SwitchBranch => "switch branch",
            WriteOperation::RenameBranch => "rename branch",
            WriteOperation::DeleteBranch => "delete branch",
            WriteOperation::PushStash => "save stash",
            WriteOperation::ApplyStash => "apply stash",
            WriteOperation::DropStash => "drop stash",
            WriteOperation::Merge => "merge",
            WriteOperation::ContinueMerge => "continue merge",
            WriteOperation::AbortMerge => "abort merge",
            WriteOperation::FinishMerge => "clear completed merge state",
            WriteOperation::Amend => "amend",
            WriteOperation::CherryPick => "cherry-pick",
            WriteOperation::Revert => "revert",
            WriteOperation::ContinueCherryPick => "cherry-pick-continue",
            WriteOperation::SkipCherryPick => "cherry-pick-skip",
            WriteOperation::AbortCherryPick => "cherry-pick-abort",
            WriteOperation::ContinueRevert => "revert-continue",
            WriteOperation::SkipRevert => "revert-skip",
            WriteOperation::AbortRevert => "revert-abort",
            WriteOperation::Rebase => "rebase",
            WriteOperation::ContinueRebase => "rebase-continue",
            WriteOperation::SkipRebase => "rebase-skip",
            WriteOperation::AbortRebase => "rebase-abort",
            WriteOperation::ImportSeries => "series-import",
            WriteOperation::ContinueSeries => "series-continue",
            WriteOperation::SkipSeries => "series-skip",
            WriteOperation::AbortSeries => "series-abort",
            WriteOperation::ResetSoft => "reset-soft",
            WriteOperation::ResetMixed => "reset-mixed",
            WriteOperation::ResetHard => "reset-hard",
        };
        let result = self.runner.observe("write_result", label, || {
            let output = if let Some(editor) = editor {
                self.runner
                    .interactive_rebase(self.cwd(), arguments, input, cancel, editor)
            } else if matches!(
                operation,
                WriteOperation::Amend
                    | WriteOperation::Rebase
                    | WriteOperation::ContinueRebase
                    | WriteOperation::SkipRebase
                    | WriteOperation::AbortRebase
                    | WriteOperation::CherryPick
                    | WriteOperation::Revert
                    | WriteOperation::ContinueCherryPick
                    | WriteOperation::SkipCherryPick
                    | WriteOperation::AbortCherryPick
                    | WriteOperation::ContinueRevert
                    | WriteOperation::SkipRevert
                    | WriteOperation::AbortRevert
            ) {
                self.runner
                    .commit_action(self.cwd(), arguments, input, cancel)
            } else {
                self.runner.write(self.cwd(), arguments, input, cancel)
            };
            output.and_then(|output| {
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
        // A fresh token is intentional: cancellation must not suppress reconciliation.
        // Keep the shared write permit until this bounded read completes. Do not retry a
        // commit, roll back, or delete Git lock files on an uncertain outcome.
        let status = self.runner.observe("refresh", "status", || {
            self.status(&CancellationToken::new())
        });
        Ok(WriteReport {
            operation,
            result,
            status,
        })
    }
}

pub(crate) fn validate_commit_message(message: &str) -> Result<()> {
    if message.trim().is_empty() || message.contains('\0') || message.len() > MAX_MESSAGE_BYTES {
        return Err(Error::InvalidInput(
            "commit message must be nonblank, contain no NUL, and fit in 1 MiB".into(),
        ));
    }
    Ok(())
}

fn path_input(paths: &[RepoPath]) -> Result<Vec<u8>> {
    if paths.is_empty() {
        return Err(Error::InvalidInput(
            "select at least one path; use the explicit *_all method for every path".into(),
        ));
    }
    let mut input = Vec::new();
    for path in paths {
        // Match the platform's representability checks used by the read API as well.
        path.argument()?;
        if path.as_bytes().len() >= MAX_PATH_BYTES.saturating_sub(input.len()) {
            return Err(Error::InvalidInput("selected paths exceed 8 MiB".into()));
        }
        input.extend_from_slice(path.as_bytes());
        input.push(0);
    }
    Ok(input)
}

fn selection(mut arguments: Vec<OsString>, paths: Option<Vec<u8>>) -> (Vec<OsString>, Vec<u8>) {
    if let Some(input) = paths {
        arguments.extend(args(&[
            "--pathspec-from-file=-",
            "--pathspec-file-nul",
            "--",
        ]));
        (arguments, input)
    } else {
        arguments.extend(args(&["--", "."]));
        (arguments, Vec::new())
    }
}
