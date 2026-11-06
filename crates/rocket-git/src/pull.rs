use crate::{
    model::bytes_to_os,
    remote::{branch_refspec, pattern_matches},
    runner::args,
    *,
};
use std::sync::mpsc::SyncSender;

impl Repository {
    /// Fetch the current branch's configured remote, then integrate only its
    /// freshly fetched upstream commit using merge --ff-only. Requires an existing
    /// local branch, clean tracked files/index/submodules, and no active operation.
    /// Untracked/ignored files are kept; Git refuses incoming path collisions.
    /// No autostash, rebase, reset, pruning, tag fetch, or submodule checkout.
    ///
    /// One write permit spans selection, fetch, update, and reconciliation. Other
    /// Git applications are not locked out: rechecks narrow, but cannot eliminate,
    /// races with external edits/config changes. Use WriteTask off the UI thread.
    pub fn pull_ff_only(
        &self,
        progress: Option<&SyncSender<FetchProgress>>,
        cancel: &CancellationToken,
    ) -> Result<PullReport> {
        self.pull_fast_forward(progress, false, cancel)
    }

    /// [`Repository::pull_ff_only`] for pulling many repositories at once: its fetch
    /// never asks for input. No askpass program runs and Git Credential Manager is told
    /// not to ask, so a remote that needs a sign-in fails this repository instead.
    pub fn pull_ff_only_quietly(&self, cancel: &CancellationToken) -> Result<PullReport> {
        self.pull_fast_forward(None, true, cancel)
    }

    fn pull_fast_forward(
        &self,
        progress: Option<&SyncSender<FetchProgress>>,
        quiet: bool,
        cancel: &CancellationToken,
    ) -> Result<PullReport> {
        self.require_worktree(cancel)?;
        let _permit = self.acquire_write(cancel)?;
        let (branch, before) = self
            .runner
            .step("pull", "validate_pull", || self.pull_state(cancel))?;
        let upstream = self.runner.step("pull", "resolve_upstream", || {
            self.pull_upstream(&branch, cancel)
        })?;
        let (arguments, input) = self.prepare_pull_fetch(&upstream, cancel)?;

        // From here onward, never return an outer error: fetch may have changed
        // objects/refs even if its process failed, was cancelled, or never spawned.
        let fetch = if quiet {
            // As automatic fetch: Git Credential Manager reads this, and the runner
            // removes every askpass route.
            let mut quiet_arguments = args(&["-c", "credential.interactive=never"]);
            quiet_arguments.extend(arguments);
            self.execute_quiet_fetch(&quiet_arguments, input, cancel)
        } else {
            self.execute_fetch(&arguments, input, progress, cancel)
        };
        let update = match &fetch {
            Ok(FetchOutput {
                updates: Ok(updates),
                ..
            }) => Some(self.runner.observe("pull_update", "pull", || {
                self.update_after_pull(&upstream, &before, updates, cancel)
            })),
            _ => None,
        };
        if let Some(log) = &self.runner.log {
            log.pull_outcome(&fetch, &update);
        }
        let status = self.runner.observe("refresh", "status", || {
            self.status(&CancellationToken::new())
        });
        let branches = self.runner.observe("refresh", "branches", || {
            self.branches(&CancellationToken::new())
        });
        let state = self.runner.observe("refresh", "operation", || {
            self.operation_state(&CancellationToken::new())
        });
        Ok(PullReport {
            upstream,
            fetch,
            update,
            status,
            branches,
            state,
        })
    }

    pub(crate) fn prepare_pull_fetch(
        &self,
        upstream: &PullUpstream,
        cancel: &CancellationToken,
    ) -> Result<(Vec<std::ffi::OsString>, Vec<u8>)> {
        let (prefix, remote) = self.validate_fetch_selection(&upstream.remote, cancel)?;
        self.runner
            .step("fetch", "validate_remote_and_refspecs", || {
                let prepared = self.prepare_fetch(&upstream.remote, &prefix, remote, cancel)?;
                // Validate the exact refspec bytes passed to Git, not a second config read.
                let mut included = false;
                let mut excluded = false;
                for spec in prepared.1.split(|b| *b == b'\n').filter(|s| !s.is_empty()) {
                    let (source, destination) = branch_refspec(spec, &prefix)?;
                    if pattern_matches(source, &upstream.remote_reference) {
                        if destination.is_some() {
                            included = true;
                        } else {
                            excluded = true;
                        }
                    }
                }
                if !included || excluded {
                    return Err(Error::Pull(PullFailure::UnsupportedUpstream));
                }
                self.require_direct_pull_ref(&upstream.tracking_reference, cancel)?;
                Ok(prepared)
            })
    }

    pub(crate) fn fetched_pull_target(
        &self,
        upstream: &PullUpstream,
        updates: &[FetchUpdate],
        cancel: &CancellationToken,
    ) -> Result<ObjectId> {
        self.runner.step("pull", "select_fetched_target", || {
            cancel.check()?;
            let mut selected = updates
                .iter()
                .filter(|u| u.reference == upstream.tracking_reference);
            let update = selected
                .next()
                .ok_or(Error::Pull(PullFailure::UpstreamNotFetched))?;
            if selected.next().is_some() {
                return Err(Error::Pull(PullFailure::UpstreamNotFetched));
            }
            // Never fall back to a stale ref after a deleted, skipped or excluded upstream.
            self.resolve_commit(update.new.as_str(), cancel)
        })
    }

    pub(crate) fn pull_state(&self, cancel: &CancellationToken) -> Result<(Vec<u8>, ObjectId)> {
        cancel.check()?;
        if !self.operation_state(cancel)?.is_idle() {
            return Err(Error::Pull(PullFailure::OperationInProgress));
        }
        let (branch, target) = match self.head(cancel)? {
            Head::Branch { name, target } => (name, target),
            Head::Detached { .. } => return Err(Error::Pull(PullFailure::DetachedHead)),
            Head::Unborn { .. } => return Err(Error::Pull(PullFailure::UnbornHead)),
        };
        self.verify_pull_head(&branch, &target, cancel)?;
        self.require_direct_pull_ref(&[b"refs/heads/", branch.as_slice()].concat(), cancel)?;
        let status = self.status(cancel)?;
        if status.head
            != (Head::Branch {
                name: branch.clone(),
                target: target.clone(),
            })
        {
            return Err(Error::Pull(PullFailure::StateChanged));
        }
        if status
            .entries
            .iter()
            .any(|e| !matches!(e.kind, EntryKind::Untracked | EntryKind::Ignored))
        {
            return Err(Error::Pull(PullFailure::DirtyWorktree));
        }
        Ok((branch, target))
    }

    pub(crate) fn pull_upstream(
        &self,
        branch: &[u8],
        cancel: &CancellationToken,
    ) -> Result<PullUpstream> {
        let remote = self.pull_config_value(branch, b".remote", cancel)?;
        let remote_reference = self.pull_config_value(branch, b".merge", cancel)?;
        self.checked_pull_upstream(branch, remote, remote_reference, cancel)
    }

    pub(crate) fn checked_pull_upstream(
        &self, branch: &[u8], remote: Vec<u8>, remote_reference: Vec<u8>,
        cancel: &CancellationToken,
    ) -> Result<PullUpstream> {
        if remote == b"." || !remote_reference.starts_with(b"refs/heads/") {
            return Err(Error::Pull(PullFailure::UnsupportedUpstream));
        }
        self.check_fetch_ref(&remote_reference, false, cancel)?;
        let tracking_reference = [
            b"refs/remotes/",
            remote.as_slice(),
            b"/",
            &remote_reference[b"refs/heads/".len()..],
        ]
        .concat();
        Ok(PullUpstream {
            branch: branch.to_vec(),
            remote,
            remote_reference,
            tracking_reference,
        })
    }

    fn pull_config_value(
        &self,
        branch: &[u8],
        suffix: &[u8],
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>> {
        let key = [b"branch.", branch, suffix].concat();
        let mut arguments = args(&["config", "--null", "--get-all"]);
        arguments.push(bytes_to_os(&key)?);
        let output = self
            .runner
            .run(self.cwd(), &arguments, Vec::new(), cancel)?;
        if output.status.code() == Some(1) && output.stdout.is_empty() {
            return Err(Error::Pull(PullFailure::MissingUpstream));
        }
        let bytes = output.success("read pull upstream")?;
        let value = bytes
            .strip_suffix(b"\0")
            .ok_or_else(|| Error::parse("pull upstream", "unterminated configuration value"))?;
        if value.is_empty() {
            return Err(Error::Pull(PullFailure::MissingUpstream));
        }
        if value.len() > 4096 || value.contains(&0) {
            return Err(Error::Pull(PullFailure::UnsupportedUpstream));
        }
        Ok(value.to_vec())
    }

    pub(crate) fn require_direct_pull_ref(
        &self,
        reference: &[u8],
        cancel: &CancellationToken,
    ) -> Result<()> {
        let mut arguments = args(&["symbolic-ref", "--quiet"]);
        arguments.push(bytes_to_os(reference)?);
        let output = self
            .runner
            .run(self.cwd(), &arguments, Vec::new(), cancel)?;
        match output.status.code() {
            Some(1) => Ok(()), // A missing ref is also allowed before the first fetch.
            Some(0) => Err(Error::Pull(PullFailure::UnsupportedUpstream)),
            _ => output.success("check pull reference").map(|_| ()),
        }
    }

    fn verify_pull_head(
        &self,
        branch: &[u8],
        target: &ObjectId,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let output = self.runner.run(
            self.cwd(),
            &args(&["symbolic-ref", "--quiet", "--no-recurse", "HEAD"]),
            Vec::new(),
            cancel,
        )?;
        if output.status.code() == Some(1) {
            return Err(Error::Pull(PullFailure::StateChanged));
        }
        let symbolic = output.success("check pull HEAD")?;
        if crate::parse::line(&symbolic) != [b"refs/heads/", branch].concat()
            || self.head(cancel)?
                != (Head::Branch {
                    name: branch.to_vec(),
                    target: target.clone(),
                })
        {
            return Err(Error::Pull(PullFailure::StateChanged));
        }
        Ok(())
    }

    fn update_after_pull(
        &self,
        upstream: &PullUpstream,
        before: &ObjectId,
        updates: &[FetchUpdate],
        cancel: &CancellationToken,
    ) -> Result<PullUpdate> {
        let target = self.fetched_pull_target(upstream, updates, cancel)?;
        let kind = self.runner.step("pull", "check_ancestry", || {
            if target == *before || self.pull_is_ancestor(&target, before, cancel)? {
                Ok(PullUpdateKind::UpToDate)
            } else if self.pull_is_ancestor(before, &target, cancel)? {
                Ok(PullUpdateKind::FastForward)
            } else {
                Err(Error::Pull(PullFailure::Diverged))
            }
        })?;
        self.runner.step("pull", "revalidate_pull", || {
            let (branch, current) = self.pull_state(cancel)?;
            if branch != upstream.branch
                || current != *before
                || self.pull_upstream(&branch, cancel)? != *upstream
            {
                return Err(Error::Pull(PullFailure::StateChanged));
            }
            self.require_direct_pull_ref(&upstream.tracking_reference, cancel)
        })?;
        let arguments = self.merge_commit_arguments(&target, MergeMode::FastForwardOnly, cancel)?;
        let output = self.runner.observe("write_result", "pull-update", || {
            let output = self
                .runner
                .write(self.cwd(), &arguments, Vec::new(), cancel)?;
            if !output.status.success() {
                return Err(Error::Git {
                    operation: "pull-update",
                    code: output.status.code(),
                    stdout: output.stdout,
                    stderr: output.stderr,
                });
            }
            Ok(WriteOutput {
                stdout: output.stdout,
                stderr: output.stderr,
            })
        })?;
        let expected = if kind == PullUpdateKind::FastForward {
            &target
        } else {
            before
        };
        let verification = self.runner.observe("verify", "pull-update", || {
            match self.verify_pull_head(&upstream.branch, expected, &CancellationToken::new()) {
                Err(Error::Pull(PullFailure::StateChanged)) => {
                    Err(Error::Pull(PullFailure::VerificationFailed))
                }
                result => result,
            }
        });
        Ok(PullUpdate {
            kind,
            before: before.clone(),
            fetched: target,
            error_diagnostics: crate::runner::has_error_diagnostics(&output.stderr),
            output,
            verification,
        })
    }

    fn pull_is_ancestor(
        &self,
        ancestor: &ObjectId,
        descendant: &ObjectId,
        cancel: &CancellationToken,
    ) -> Result<bool> {
        let output = self.runner.run(
            self.cwd(),
            &args(&[
                "merge-base",
                "--is-ancestor",
                ancestor.as_str(),
                descendant.as_str(),
            ]),
            Vec::new(),
            cancel,
        )?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => output
                .success("check pull ancestry")
                .map(|_| unreachable!()),
        }
    }
}
