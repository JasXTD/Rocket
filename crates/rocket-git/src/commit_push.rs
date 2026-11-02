//! Reviewed commit followed by an ordinary push. A remote failure never rolls
//! back a local commit. The write permit spans both phases.
use crate::{runner::args, *};
use std::path::PathBuf;

#[derive(Clone)]
pub struct CommitPushPreview {
    repository: PathBuf,
    head: Head,
    index: Vec<u8>,
    config: Vec<u8>,
    url: Vec<u8>,
    remote: Vec<u8>,
    branch: Vec<u8>,
    destination: Vec<u8>,
    message: String,
    staged_files: usize,
}
impl std::fmt::Debug for CommitPushPreview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommitPushPreview")
            .field("staged_files", &self.staged_files)
            .finish_non_exhaustive()
    }
}
impl CommitPushPreview {
    pub fn destination_display(&self) -> String {
        crate::tags::display_destination(&self.url)
    }
    pub fn remote(&self) -> &[u8] {
        &self.remote
    }
    pub fn branch(&self) -> &[u8] {
        &self.branch
    }
    pub fn destination(&self) -> &[u8] {
        &self.destination
    }
    pub fn message(&self) -> &str {
        &self.message
    }
    pub fn staged_files(&self) -> usize {
        self.staged_files
    }
}
#[derive(Debug)]
#[must_use = "inspect both the retained local commit and remote push outcome before retrying"]
pub struct CommitPushReport {
    pub result: Result<()>,
    /// Observed new commit even if the commit command was interrupted after writing it.
    pub commit: Option<ObjectId>,
    pub reviewed_commit_confirmed: bool,
    pub phase: &'static str,
    pub push: Option<PushReport>,
}
fn invalid(message: &str) -> Error {
    Error::InvalidInput(message.into())
}
impl Repository {
    fn commit_push_read(&self, command: &[&str], c: &CancellationToken) -> Result<Vec<u8>> {
        self.runner
            .run(self.cwd(), &args(command), vec![], c)?
            .success("commit-push")
    }
    fn commit_push_index(&self, c: &CancellationToken) -> Result<Vec<u8>> {
        let bytes = self.commit_push_read(&["ls-files", "--stage", "-v", "-z"], c)?;
        if bytes.len() > 32 * 1024 * 1024 {
            return Err(invalid("commit_push_limit: index review exceeds 32 MiB."));
        }
        Ok(bytes)
    }
    fn commit_push_url(&self, remote: &[u8], c: &CancellationToken) -> Result<Vec<u8>> {
        let selected = self
            .remotes(c)?
            .into_iter()
            .find(|r| r.name == remote)
            .ok_or_else(|| invalid("commit_push_remote: choose a configured remote."))?;
        self.ensure_no_remote_group(remote, c)?;
        let urls = if selected.push_urls.is_empty() {
            &selected.urls
        } else {
            &selected.push_urls
        };
        if urls.len() != 1 {
            return Err(invalid(
                "commit_push_remote: exactly one push destination is required.",
            ));
        }
        let mut command = args(&["remote", "get-url", "--push", "--all", "--"]);
        command.push(crate::model::bytes_to_os(remote)?);
        let bytes = self
            .runner
            .run(self.cwd(), &command, vec![], c)?
            .success("commit-push destination")?;
        let url = bytes.strip_suffix(b"\n").unwrap_or(&bytes);
        if url.is_empty() || url.iter().any(|b| matches!(b, 0 | b'\n' | b'\r')) {
            return Err(invalid("commit_push_remote: choose one valid destination."));
        }
        Ok(url.to_vec())
    }
    pub fn preview_commit_push(
        &self,
        message: &str,
        remote: &[u8],
        destination: &[u8],
        c: &CancellationToken,
    ) -> Result<CommitPushPreview> {
        self.runner.step(
            "commit-push-preview",
            "review_commit_and_destination",
            || {
                crate::write::validate_commit_message(message)?;
                self.require_worktree(c)?;
                self.require_idle(c)?;
                self.branch_argument(destination, c)?;
                let status = self.status(c)?;
                let branch = match &status.head {
                    Head::Branch { name, .. } => name.clone(),
                    Head::Unborn { branch } => branch.clone(),
                    _ => return Err(invalid("commit_push_branch: select a local branch first.")),
                };
                if status.entries.iter().any(|e| e.kind == EntryKind::Unmerged) {
                    return Err(invalid(
                        "commit_push_conflicts: resolve conflicts before committing.",
                    ));
                }
                let staged_files = status
                    .entries
                    .iter()
                    .filter(|e| {
                        !matches!(e.kind, EntryKind::Untracked | EntryKind::Ignored)
                            && e.index != Change::Unchanged
                    })
                    .count();
                if staged_files == 0 {
                    return Err(invalid(
                        "commit_push_empty: stage changes before reviewing Commit & Push.",
                    ));
                }
                let p = CommitPushPreview {
                    repository: self.info().git_dir.clone(),
                    head: status.head,
                    index: self.commit_push_index(c)?,
                    config: self.commit_push_read(&["config", "--null", "--list"], c)?,
                    url: self.commit_push_url(remote, c)?,
                    remote: remote.to_vec(),
                    branch,
                    destination: destination.to_vec(),
                    message: message.into(),
                    staged_files,
                };
                self.check_commit_push(&p, c)?;
                Ok(p)
            },
        )
    }
    fn check_commit_push(&self, p: &CommitPushPreview, c: &CancellationToken) -> Result<()> {
        c.check()?;
        self.require_idle(c)?;
        if self.info().git_dir != p.repository
            || self.head(c)? != p.head
            || self.commit_push_index(c)? != p.index
            || self.commit_push_read(&["config", "--null", "--list"], c)? != p.config
            || self.commit_push_url(&p.remote, c)? != p.url
        {
            return Err(invalid(
                "commit_push_stale: staging, checkout or destination changed. Review again; nothing committed or pushed.",
            ));
        }
        Ok(())
    }
    pub fn apply_commit_push(
        &self,
        p: &CommitPushPreview,
        c: &CancellationToken,
    ) -> Result<CommitPushReport> {
        let _permit = self.acquire_write(c)?;
        self.runner
            .step("commit-push", "revalidate_commit_review", || {
                self.check_commit_push(p, c)
            })?;
        let tree = self.commit_push_read(&["write-tree"], c)?;
        let mut report = CommitPushReport {
            result: Ok(()),
            commit: None,
            reviewed_commit_confirmed: false,
            phase: "committing",
            push: None,
        };
        report.result = self.runner.step("commit-push", "commit_then_push", || {
            let commit = self.runner.step("commit-push", "commit_local", || self.commit_locked(&p.message, c))?;
            // Reconcile with a fresh token: cancellation is not rollback. Do not
            // publish commits that a hook or external Git writer changed.
            let probe = CancellationToken::new();
            let head = self.head(&probe)?;
            if let Head::Branch { name, target } = &head
                && name == &p.branch && head != p.head { report.commit = Some(target.clone()); }
            if report.commit.is_none() {commit.result?;return Err(invalid("commit_push_unconfirmed: no new local commit was confirmed. Inspect history before retrying."));}
            let id = report.commit.as_ref().ok_or_else(|| invalid("commit_push_unconfirmed: no new local commit was confirmed. Inspect history before retrying."))?;
            let history = self.history(HistoryStart::Commit(id.clone()), 1, &probe)?;
            let parents_ok = history.commits.first().is_some_and(|commit| match &p.head {
                Head::Branch { target, .. } => commit.parents.as_slice() == [target.clone()],
                Head::Unborn { .. } => commit.parents.is_empty(),
                _ => false,
            });
            let tree_ok=self.commit_push_read(&["rev-parse", "HEAD^{tree}"], &probe)? == tree;
            report.reviewed_commit_confirmed=parents_ok && tree_ok && history.commits.first().is_some_and(|entry|String::from_utf8_lossy(&entry.message).trim()==p.message.trim());
            commit.result?;
            if !parents_ok || !tree_ok {
                return Err(invalid("commit_push_commit_changed: the new commit differs from the reviewed staging or parent. Local history retained; push was not started."));
            }
            report.phase = "committed";
            c.check()?;
            if self.head(c)? != head || self.commit_push_read(&["config", "--null", "--list"], c)? != p.config
                || self.commit_push_url(&p.remote, c)? != p.url {
                return Err(invalid("commit_push_destination_changed: checkout or destination changed after committing. Local commit retained; review Push separately."));
            }
            report.phase = "pushing";
            report.push = Some(self.runner.step("commit-push", "push_committed_tip", || self.push_guarded_locked(&p.remote, &p.branch, &p.destination, PushOptions::default(), None, Some((id, &p.url)), c))?);
            report.phase = "push-finished";
            let push=report.push.as_ref().expect("push report just assigned");
            if push.result.is_err() || push.remote_outcome!=PushRemoteOutcome::Confirmed
                || push.error_diagnostics || push.update.as_ref().is_some_and(Result::is_err)
                || push.branches.is_err() {
                return Err(if c.is_cancelled(){Error::Cancelled}else{invalid("commit_push_incomplete: local commit retained; push or its verification did not complete. Inspect the remote outcome before retrying Push.")});
            }
            Ok(())
        });
        Ok(report)
    }
}
