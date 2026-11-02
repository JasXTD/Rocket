//! Start a git-flow branch: create it at its reviewed base commit, then switch to it.
use super::*;

/// A reviewed start. Feature branches take tracked changes along, as `git checkout -b`
/// does; releases, hotfixes and support branches need a clean checkout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitFlowStartPreview {
    git_dir: PathBuf,
    worktree: Option<PathBuf>,
    kind: GitFlowKind,
    name: String,
    branch: String,
    base: String,
    base_id: ObjectId,
    tag: Option<String>,
    head: Head,
    carried: usize,
    unpushed: Option<u64>,
    config: GitFlowConfig,
}
impl GitFlowStartPreview {
    pub fn kind(&self) -> GitFlowKind {
        self.kind
    }
    /// The name after the prefix: a feature name or a version.
    pub fn name(&self) -> &str {
        &self.name
    }
    /// The new local branch, for example `feature/login`.
    pub fn branch(&self) -> &str {
        &self.branch
    }
    /// The base branch (or, for support branches, the revision) as entered.
    pub fn base(&self) -> &str {
        &self.base
    }
    pub fn base_id(&self) -> &ObjectId {
        &self.base_id
    }
    /// The version tag that must not exist yet (releases and hotfixes).
    pub fn tag(&self) -> Option<&str> {
        self.tag.as_deref()
    }
    pub fn head(&self) -> &Head {
        &self.head
    }
    /// Tracked changes that come along to a new feature branch.
    pub fn carried(&self) -> usize {
        self.carried
    }
    /// Commits on the base that `origin/<base>` lacks, when that branch exists.
    pub fn unpushed(&self) -> Option<u64> {
        self.unpushed
    }
    pub fn config(&self) -> &GitFlowConfig {
        &self.config
    }
}

/// Switching is attempted only after the branch was created.
#[derive(Debug)]
#[must_use = "inspect both commands and their refreshed state; cancellation is not rollback"]
pub struct GitFlowStartReport {
    pub create: BranchWriteReport,
    /// `None` when creating the branch failed.
    pub switch: Option<Result<BranchWriteReport>>,
}

impl Repository {
    /// Review starting a git-flow branch. `base` overrides the default base: develop for
    /// features and releases, the production branch for hotfixes and support branches.
    pub fn preview_gitflow_start(
        &self,
        kind: GitFlowKind,
        name: &str,
        base: Option<&str>,
        cancel: &CancellationToken,
    ) -> Result<GitFlowStartPreview> {
        self.runner
            .step("gitflow-start-preview", "review_gitflow_start", || {
                self.require_worktree(cancel)?;
                let state = self.gitflow_state(cancel)?;
                let config = state
                    .config
                    .clone()
                    .ok_or(Error::GitFlow(GitFlowFailure::NotInitialized))?;
                if name.is_empty()
                    || name.len() > GITFLOW_NAME_LIMIT
                    || name.chars().any(|c| c.is_control() || c.is_whitespace())
                {
                    return refused(GitFlowFailure::InvalidName);
                }
                let branch = config.branch_name(kind, name);
                branch_os(self, &branch, cancel)?;
                let tag = kind.tagged().then(|| config.tag_name(name));
                if let Some(tag) = &tag
                    && !self.gitflow_tag_valid(tag, cancel)?
                {
                    return refused(GitFlowFailure::InvalidName);
                }
                if !self.operation_state(cancel)?.is_idle() {
                    return refused(GitFlowFailure::ActiveOperation);
                }
                let status = self.status(cancel)?;
                let branches = self.branches(cancel)?;
                if local_branch(&branches, &branch).is_some() {
                    return refused(GitFlowFailure::BranchExists);
                }
                // git-flow keeps one release and one hotfix at a time.
                let open = |k| state.branches.iter().any(|b| b.kind == k);
                match kind {
                    GitFlowKind::Release if open(kind) => {
                        return refused(GitFlowFailure::ReleaseInProgress);
                    }
                    GitFlowKind::Hotfix if open(kind) => {
                        return refused(GitFlowFailure::HotfixInProgress);
                    }
                    _ => {}
                }
                if let Some(tag) = &tag
                    && self.gitflow_tag_commit(tag, cancel)?.is_some()
                {
                    return refused(GitFlowFailure::TagExists);
                }
                let (default, missing) = match kind {
                    GitFlowKind::Feature | GitFlowKind::Release => {
                        (&config.develop, GitFlowFailure::DevelopMissing)
                    }
                    GitFlowKind::Hotfix | GitFlowKind::Support => {
                        (&config.master, GitFlowFailure::ProductionMissing)
                    }
                };
                let base_name = base.unwrap_or(default);
                let base_id = match local_branch(&branches, base_name) {
                    Some(branch) => branch.target.clone(),
                    None if base.is_none() => return refused(missing),
                    None => self
                        .resolve_commit(base_name, cancel)
                        .map_err(|e| match e {
                            Error::Git { .. } | Error::InvalidInput(_) => {
                                Error::GitFlow(GitFlowFailure::BaseMissing)
                            }
                            other => other,
                        })?,
                };
                let unpushed = match self.gitflow_origin(&branches, base_name, cancel)? {
                    Some((_, behind)) if behind > 0 => {
                        return refused(GitFlowFailure::BehindRemote);
                    }
                    Some((ahead, _)) => Some(ahead),
                    None => None,
                };
                let carried = status
                    .entries
                    .iter()
                    .filter(|e| !matches!(e.kind, EntryKind::Untracked | EntryKind::Ignored))
                    .count();
                if carried > 0 && kind != GitFlowKind::Feature {
                    return refused(GitFlowFailure::DirtyWorktree);
                }
                Ok(GitFlowStartPreview {
                    git_dir: self.info().git_dir.clone(),
                    worktree: self.info().worktree.clone(),
                    kind,
                    name: name.into(),
                    branch,
                    base: base_name.into(),
                    base_id,
                    tag,
                    head: status.head,
                    carried,
                    unpushed,
                    config,
                })
            })
    }

    /// Revalidate under the write gate, create the branch at the reviewed commit and
    /// switch to it. Git refuses a switch that would overwrite local changes.
    pub fn apply_gitflow_start(
        &self,
        preview: &GitFlowStartPreview,
        cancel: &CancellationToken,
    ) -> Result<GitFlowStartReport> {
        self.require_worktree(cancel)?;
        let _permit = self.acquire_write(cancel)?;
        self.runner
            .step("gitflow-start", "revalidate_gitflow_start", || {
                if self.info().git_dir != preview.git_dir
                    || self.info().worktree != preview.worktree
                    || self.preview_gitflow_start(
                        preview.kind,
                        &preview.name,
                        Some(&preview.base),
                        cancel,
                    )? != *preview
                {
                    return refused(GitFlowFailure::StalePreview);
                }
                cancel.check()
            })?;
        let name = branch_os(self, &preview.branch, cancel)?;
        let create = self.create_branch_at(name.clone(), &preview.base_id, cancel)?;
        let switch = create
            .write
            .result
            .is_ok()
            .then(|| self.switch_branch_to(name, cancel));
        Ok(GitFlowStartReport { create, switch })
    }
}
