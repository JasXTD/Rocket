use crate::{model::bytes_to_os, runner::args, *};
use std::ffi::OsString;

impl Repository {
    /// Create a local branch without switching or configuring an upstream.
    /// `start` is a commit-ish, resolved under the write gate; None means HEAD.
    /// A first commit must exist when starting from HEAD. Names are literal short
    /// names (for example `feature/login`), passed as UTF-8 strings or raw Git bytes.
    pub fn create_branch(
        &self,
        name: impl AsRef<[u8]>,
        start: Option<&str>,
        cancel: &CancellationToken,
    ) -> Result<BranchWriteReport> {
        self.require_worktree(cancel)?;
        let name = self.branch_argument(name.as_ref(), cancel)?;
        let _permit = self.acquire_write(cancel)?;
        // Pin the requested commit so a moving start ref cannot change the creation
        // point between resolution and the branch command. Never reset an existing ref.
        let start = self.resolve_commit(start.unwrap_or("HEAD"), cancel)?;
        self.create_branch_at(name, &start, cancel)
    }

    /// [`Repository::create_branch`] at a pinned commit, for a name already checked by
    /// `branch_argument`. The caller holds the write gate.
    pub(crate) fn create_branch_at(
        &self,
        name: OsString,
        start: &ObjectId,
        cancel: &CancellationToken,
    ) -> Result<BranchWriteReport> {
        let mut arguments = args(&["branch", "--no-track", "--"]);
        arguments.extend([name, start.as_str().into()]);
        self.execute_branch_write(WriteOperation::CreateBranch, &arguments, cancel)
    }

    /// Check out a remote-tracking branch as a new local branch: create `name` at the
    /// commit `upstream` (a `refs/remotes/` ref) points to, set `upstream` as its
    /// upstream, then switch to it, all under one write gate. All or nothing: when Git
    /// refuses the upstream (no remote's fetch refspec maps it, or two do) or the switch
    /// (local changes in the way), or a step is stopped, the new branch is deleted again
    /// and that step's failure is reported. An existing local branch is never replaced,
    /// and the switch keeps `switch_branch`'s protections.
    ///
    /// The steps stay separate on purpose: Git's own `switch -c <name> --track` updates
    /// the working tree before it checks the upstream, so a refused upstream left HEAD
    /// on the old branch over the new branch's files.
    pub fn check_out_remote_branch(
        &self,
        name: impl AsRef<[u8]>,
        upstream: &[u8],
        cancel: &CancellationToken,
    ) -> Result<BranchWriteReport> {
        self.require_worktree(cancel)?;
        let upstream = std::str::from_utf8(upstream)
            .ok()
            .filter(|u| u.starts_with("refs/remotes/") && !u.contains(['\0', '\n']))
            .ok_or_else(|| Error::InvalidInput("Choose a remote-tracking branch to check out.".into()))?;
        let name = self.branch_argument(name.as_ref(), cancel)?;
        let _permit = self.acquire_write(cancel)?;
        let start = self.resolve_commit(upstream, cancel)?;
        let created = self.create_branch_at(name.clone(), &start, cancel)?;
        if created.write.result.is_err() {
            // Refused (the name is taken) or stopped while Git ran: nothing of ours to undo.
            return Ok(created);
        }
        let mut arguments = args(&["branch"]);
        arguments.push(format!("--set-upstream-to={upstream}").into());
        arguments.push("--".into());
        arguments.push(name.clone());
        match self.execute_branch_write(WriteOperation::CreateBranch, &arguments, cancel) {
            Ok(tracked) if tracked.write.result.is_ok() => {}
            failed => return self.remove_created_branch(name, WriteOperation::CreateBranch, failed),
        }
        match self.switch_branch_to(name.clone(), cancel) {
            Ok(switched) if switched.write.result.is_ok() => Ok(switched),
            failed => self.remove_created_branch(name, WriteOperation::SwitchBranch, failed),
        }
    }

    /// [`Repository::check_out_remote_branch`] after a later step failed: delete the
    /// branch it created (and the upstream it may have set), then report the failed
    /// step with the state read after the removal. Git refuses to delete a branch that
    /// is checked out, so a stopped switch that got as far as HEAD keeps it, and the
    /// branch list then shows it. The caller holds the write gate.
    fn remove_created_branch(
        &self,
        name: OsString,
        operation: WriteOperation,
        failed: Result<BranchWriteReport>,
    ) -> Result<BranchWriteReport> {
        let failure = match failed {
            Ok(report) => match report.write.result {
                Err(error) => error,
                Ok(_) => return Ok(report),
            },
            Err(error) => error,
        };
        let mut arguments = args(&["branch", "--delete", "--force", "--"]);
        arguments.push(name);
        // A fresh token: the Stop that ended the step must not also stop the removal.
        let removed = self.execute_branch_write(
            WriteOperation::DeleteBranch,
            &arguments,
            &CancellationToken::new(),
        )?;
        Ok(BranchWriteReport {
            write: WriteReport {
                operation,
                result: Err(failure),
                status: removed.write.status,
            },
            branches: removed.branches,
        })
    }

    /// Switch to an existing local branch. Git rejects a switch that would overwrite
    /// local changes or a branch checked out in another worktree. Ignored files are
    /// also protected from overwrite. No remote guessing, forced discard, automatic
    /// stash/merge, or recursive submodule checkout is used.
    pub fn switch_branch(
        &self,
        name: impl AsRef<[u8]>,
        cancel: &CancellationToken,
    ) -> Result<BranchWriteReport> {
        self.require_worktree(cancel)?;
        let name = self.branch_argument(name.as_ref(), cancel)?;
        let _permit = self.acquire_write(cancel)?;
        self.switch_branch_to(name, cancel)
    }

    /// [`Repository::switch_branch`] for a name already checked by `branch_argument`.
    /// The caller holds the write gate.
    pub(crate) fn switch_branch_to(
        &self,
        name: OsString,
        cancel: &CancellationToken,
    ) -> Result<BranchWriteReport> {
        let mut arguments = args(&[
            "switch",
            "--no-guess",
            "--no-recurse-submodules",
            "--no-overwrite-ignore",
            "--",
        ]);
        arguments.push(name);
        self.execute_branch_write(WriteOperation::SwitchBranch, &arguments, cancel)
    }

    /// Rename one local branch, preserving Git's config/reflog migration. An existing
    /// destination is not overwritten. Git also updates HEAD in worktrees using the
    /// renamed branch, including this worktree and any linked worktree.
    pub fn rename_branch(
        &self,
        old: impl AsRef<[u8]>,
        new: impl AsRef<[u8]>,
        cancel: &CancellationToken,
    ) -> Result<BranchWriteReport> {
        self.require_worktree(cancel)?;
        let old = self.branch_argument(old.as_ref(), cancel)?;
        let new = self.branch_argument(new.as_ref(), cancel)?;
        let _permit = self.acquire_write(cancel)?;
        let mut arguments = args(&["branch", "--move", "--"]);
        arguments.extend([old, new]);
        self.execute_branch_write(WriteOperation::RenameBranch, &arguments, cancel)
    }

    /// Delete one local branch using Git's non-forced deletion. Git requires it to
    /// be merged into its configured upstream, or HEAD when it has no upstream,
    /// and refuses deletion while it is checked out in a worktree.
    pub fn delete_branch(
        &self,
        name: impl AsRef<[u8]>,
        cancel: &CancellationToken,
    ) -> Result<BranchWriteReport> {
        self.require_worktree(cancel)?;
        let name = self.branch_argument(name.as_ref(), cancel)?;
        let _permit = self.acquire_write(cancel)?;
        self.delete_branch_named(name, cancel)
    }

    /// [`Repository::delete_branch`] for a name already checked by `branch_argument`.
    /// The caller holds the write gate.
    pub(crate) fn delete_branch_named(
        &self,
        name: OsString,
        cancel: &CancellationToken,
    ) -> Result<BranchWriteReport> {
        let mut arguments = args(&["branch", "--delete", "--"]);
        arguments.push(name);
        self.execute_branch_write(WriteOperation::DeleteBranch, &arguments, cancel)
    }

    /// Delete several local branches, one at a time, each with
    /// [`Repository::delete_branch`]: Git refuses (and keeps) a branch that is not
    /// merged or is checked out, and the remaining branches are still attempted.
    /// Cancellation stops before the next branch; those are reported as skipped.
    pub fn delete_branches(
        &self,
        names: &[Vec<u8>],
        cancel: &CancellationToken,
    ) -> Result<BranchBatchReport> {
        if names.is_empty() || names.len() > MAX_BRANCH_BATCH {
            return Err(Error::InvalidInput(format!(
                "select 1..{MAX_BRANCH_BATCH} branches to delete"
            )));
        }
        if (1..names.len()).any(|i| names[..i].contains(&names[i])) {
            return Err(Error::InvalidInput("a branch is selected twice".into()));
        }
        let mut report = BranchBatchReport::default();
        for name in names {
            if cancel.is_cancelled() {
                report.skipped.push(name.clone());
                continue;
            }
            let result = self.delete_branch(name, cancel);
            report.results.push((name.clone(), result));
        }
        Ok(report)
    }

    pub(crate) fn branch_argument(
        &self,
        name: &[u8],
        cancel: &CancellationToken,
    ) -> Result<OsString> {
        if name.is_empty()
            || name.len() > 4096
            || name.contains(&0)
            || name.starts_with(b"-")
            || name.starts_with(b"refs/")
            || matches!(name, b"HEAD" | b"@")
        {
            return Err(Error::InvalidInput(
                "expected a literal local branch name of 1..4096 bytes, without a refs/ prefix, HEAD, @, a leading dash, or NUL".into(),
            ));
        }
        let argument = bytes_to_os(name)?;
        let mut full_ref = b"refs/heads/".to_vec();
        full_ref.extend_from_slice(name);
        let mut arguments = args(&["check-ref-format"]);
        arguments.push(bytes_to_os(&full_ref)?);
        // Check the fully qualified ref without --normalize or --branch: the latter
        // expands @{-n}. A branch name must never silently select a different branch.
        let output = self
            .runner
            .run(self.cwd(), &arguments, Vec::new(), cancel)?;
        match output.status.code() {
            Some(0) => Ok(argument),
            Some(1) => Err(Error::InvalidInput("invalid local branch name".into())),
            _ => output.success("validate branch name").map(|_| argument),
        }
    }

    fn execute_branch_write(
        &self,
        operation: WriteOperation,
        arguments: &[OsString],
        cancel: &CancellationToken,
    ) -> Result<BranchWriteReport> {
        // The caller holds the shared write gate through both refreshes. Reuse the
        // write contract: cancellation is not rollback, and refresh errors never
        // hide the attempted command's outcome. Status includes the observed HEAD.
        // Override this for the command, not on disk. With propagateBranches enabled,
        // Git can otherwise propagate branch operations into a separate repository.
        let mut command = args(&["-c", "submodule.recurse=false"]);
        command.extend_from_slice(arguments);
        let write = self.execute_write(operation, &command, Vec::new(), cancel)?;
        let branches = self.runner.observe("refresh", "branches", || {
            self.branches(&CancellationToken::new())
        });
        Ok(BranchWriteReport { write, branches })
    }
}
