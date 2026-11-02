//! Finish a git-flow branch: the reviewed switches, merges, version tag and branch
//! deletion run in order under one write gate. A merge that stops with conflicts ends
//! the run; finishing again once that merge is committed skips the completed steps.
use super::*;

/// What finishing may do besides git-flow's fixed merges.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitFlowFinishOptions {
    /// Features only: always create a merge commit. Otherwise a feature with exactly
    /// one commit may fast-forward develop, as git-flow does.
    pub no_ff: bool,
    /// Delete the finished branch with Git's safe delete (`branch -d`).
    pub delete_branch: bool,
    /// Releases and hotfixes: the annotated version tag's message.
    pub tag_message: String,
}
impl Default for GitFlowFinishOptions {
    fn default() -> Self {
        Self {
            no_ff: false,
            delete_branch: true,
            tag_message: String::new(),
        }
    }
}

/// One command of a reviewed finish, in run order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitFlowStep {
    Switch {
        branch: String,
    },
    /// Merge the reviewed `tip` of `branch` into `into`, which is checked out by then.
    Merge {
        branch: String,
        tip: ObjectId,
        into: String,
        mode: MergeMode,
        /// `into` only moves forward to `tip`; no merge commit is created.
        fast_forward: bool,
        message: String,
    },
    /// An annotated tag on the production branch once it contains the finished branch.
    Tag {
        name: String,
        message: String,
    },
    /// Git's safe delete of the finished branch.
    Delete {
        branch: String,
    },
}
impl GitFlowStep {
    /// A fixed diagnostic label; names and messages are never logged.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Switch { .. } => "switch_branch",
            Self::Merge { .. } => "merge_branch",
            Self::Tag { .. } => "create_tag",
            Self::Delete { .. } => "delete_branch",
        }
    }
}

/// Why the finished branch is kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitFlowKeep {
    /// Deleting it was not chosen.
    Requested,
    /// It is checked out in another worktree, where Git refuses to delete it.
    CheckedOut(PathBuf),
    /// Its upstream (named here) lacks some of its commits; Git's safe delete refuses it.
    Unpushed(String),
}

/// What a review found besides the steps it plans.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitFlowNote {
    /// The branch is merged into `into` already; that merge is skipped.
    Merged { into: String },
    /// The version tag exists and contains the branch; tagging is skipped.
    Tagged { name: String, target: ObjectId },
    /// Local commits that `origin/<branch>` lacks. Finishing never pushes.
    Unpushed { branch: String, commits: u64 },
    /// A hotfix while a release branch exists: classic git-flow merges it into the
    /// development branch only; merge it into the release branch separately if needed.
    ReleaseBranch { branch: String },
}

/// A reviewed finish, pinned to the checkout, index and every branch tip it uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitFlowFinishPreview {
    git_dir: PathBuf,
    worktree: Option<PathBuf>,
    kind: GitFlowKind,
    name: String,
    branch: String,
    tip: ObjectId,
    options: GitFlowFinishOptions,
    config: GitFlowConfig,
    head: Head,
    index: Vec<u8>,
    tips: Vec<(String, ObjectId)>,
    steps: Vec<GitFlowStep>,
    keep: Option<GitFlowKeep>,
    notes: Vec<GitFlowNote>,
    ends_on: Option<String>,
}
impl GitFlowFinishPreview {
    pub fn kind(&self) -> GitFlowKind {
        self.kind
    }
    /// The name after the prefix: a feature name or a version.
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn branch(&self) -> &str {
        &self.branch
    }
    pub fn tip(&self) -> &ObjectId {
        &self.tip
    }
    pub fn options(&self) -> &GitFlowFinishOptions {
        &self.options
    }
    pub fn config(&self) -> &GitFlowConfig {
        &self.config
    }
    pub fn head(&self) -> &Head {
        &self.head
    }
    pub fn steps(&self) -> &[GitFlowStep] {
        &self.steps
    }
    /// `Some` when the finished branch stays, and why.
    pub fn keep(&self) -> Option<&GitFlowKeep> {
        self.keep.as_ref()
    }
    pub fn notes(&self) -> &[GitFlowNote] {
        &self.notes
    }
    /// The branch checked out once every step succeeded.
    pub fn ends_on(&self) -> Option<&str> {
        self.ends_on.as_deref()
    }
}

/// One attempted step's own report, from Rocket's branch, merge or tag transaction.
#[derive(Debug)]
pub enum GitFlowStepReport {
    Branch(BranchWriteReport),
    Merge(MergeWriteReport),
    Tag(TagWriteReport),
}
impl GitFlowStepReport {
    pub fn succeeded(&self) -> bool {
        match self {
            Self::Branch(r) => r.write.result.is_ok(),
            Self::Merge(r) => r.write.result.is_ok() && r.failure.is_none(),
            Self::Tag(r) => r.result.is_ok() && r.verification.is_ok() && r.failure.is_none(),
        }
    }
    pub fn conflicts(&self) -> bool {
        matches!(self, Self::Merge(r) if r.failure == Some(MergeFailure::Conflicts))
    }
}

/// Steps run in order; the first that does not succeed ends the run.
#[derive(Debug)]
#[must_use = "inspect every attempted step and the refreshed state; cancellation is not rollback"]
pub struct GitFlowFinishReport {
    /// Attempted steps. An `Err` is a refusal or failure before the step's command.
    pub steps: Vec<(GitFlowStep, Result<GitFlowStepReport>)>,
    /// Steps not attempted because an earlier one failed or the run was cancelled.
    pub skipped: Vec<GitFlowStep>,
    pub cancelled: bool,
    /// `Conflicts` or `StepFailed` when a step did not succeed.
    pub failure: Option<GitFlowFailure>,
    pub state: Result<OperationState>,
    pub branches: Result<Vec<Branch>>,
}

/// Plans switches so each merge runs on its target and the finished branch is not
/// checked out when it is deleted.
struct Plan {
    current: Option<String>,
    steps: Vec<GitFlowStep>,
}
impl Plan {
    fn switch(&mut self, branch: &str) {
        if self.current.as_deref() != Some(branch) {
            self.steps.push(GitFlowStep::Switch {
                branch: branch.into(),
            });
            self.current = Some(branch.into());
        }
    }
}

impl Repository {
    /// Review finishing the local git-flow branch `branch` (for example `release/1.2`).
    /// No checkout, index, ref, tag or configuration writes, and nothing is fetched.
    pub fn preview_gitflow_finish(
        &self,
        branch: &str,
        options: &GitFlowFinishOptions,
        cancel: &CancellationToken,
    ) -> Result<GitFlowFinishPreview> {
        self.runner
            .step("gitflow-finish-preview", "review_gitflow_finish", || {
                self.require_worktree(cancel)?;
                let values = self.gitflow_values(false, cancel)?;
                let config = GitFlowConfig::from_values(&values)
                    .filter(|c| c.validate().is_ok())
                    .ok_or(Error::GitFlow(GitFlowFailure::NotInitialized))?;
                let (kind, name) = config
                    .classify(branch)
                    .ok_or(Error::GitFlow(GitFlowFailure::InvalidName))?;
                if !kind.finishes() {
                    return refused(GitFlowFailure::NotFinishable);
                }
                // The merge flow's own checks: idle, unlocked, clean tracked files and a
                // supported index. Its two common refusals get git-flow wording.
                self.check_merge_checkout(cancel).map_err(|e| match e {
                    Error::Merge(MergeFailure::DirtyWorktree) => {
                        Error::GitFlow(GitFlowFailure::DirtyWorktree)
                    }
                    Error::Merge(MergeFailure::ActiveOperation) => {
                        Error::GitFlow(GitFlowFailure::ActiveOperation)
                    }
                    other => other,
                })?;
                let head = self.head(cancel)?;
                let index = self
                    .runner
                    .run(
                        self.cwd(),
                        &args(&["ls-files", "--stage", "-v", "-z"]),
                        vec![],
                        cancel,
                    )?
                    .success("git-flow index")?;
                let branches = self.branches(cancel)?;
                let source = local_branch(&branches, branch)
                    .ok_or(Error::GitFlow(GitFlowFailure::BranchMissing))?;
                let tip = source.target.clone();
                let develop = local_branch(&branches, &config.develop)
                    .ok_or(Error::GitFlow(GitFlowFailure::DevelopMissing))?;
                let master = if kind.tagged() {
                    Some(
                        local_branch(&branches, &config.master)
                            .ok_or(Error::GitFlow(GitFlowFailure::ProductionMissing))?,
                    )
                } else {
                    None
                };
                let mut notes = Vec::new();
                let mut tips = vec![(branch.to_owned(), tip.clone())];
                tips.push((config.develop.clone(), develop.target.clone()));
                if let Some(master) = master {
                    tips.push((config.master.clone(), master.target.clone()));
                }
                for (name, _) in &tips {
                    match self.gitflow_origin(&branches, name, cancel)? {
                        Some((_, behind)) if behind > 0 => {
                            return refused(GitFlowFailure::BehindRemote);
                        }
                        Some((ahead, _)) if ahead > 0 => notes.push(GitFlowNote::Unpushed {
                            branch: name.clone(),
                            commits: ahead,
                        }),
                        _ => {}
                    }
                }
                let tag = if kind.tagged() {
                    let tag = config.tag_name(name);
                    if !self.gitflow_tag_valid(&tag, cancel)? {
                        return refused(GitFlowFailure::InvalidName);
                    }
                    match self.gitflow_tag_commit(&tag, cancel)? {
                        // Finishing again after a stopped run: the tag is done.
                        Some(target) if self.gitflow_ancestor(&tip, &target, cancel)? => {
                            notes.push(GitFlowNote::Tagged { name: tag, target });
                            None
                        }
                        Some(_) => return refused(GitFlowFailure::TagExists),
                        None => {
                            let message = &options.tag_message;
                            if message.trim().is_empty()
                                || message.len() > GITFLOW_MESSAGE_LIMIT
                                || message.contains('\0')
                            {
                                return refused(GitFlowFailure::TagMessage);
                            }
                            Some(tag)
                        }
                    }
                } else {
                    None
                };
                let mut plan = Plan {
                    current: match &head {
                        Head::Branch { name, .. } => String::from_utf8(name.clone()).ok(),
                        _ => None,
                    },
                    steps: Vec::new(),
                };
                let merge = |plan: &mut Plan, into: &Branch, name: &str, mode: MergeMode| {
                    let fast_forward = mode == MergeMode::AllowFastForward
                        && self.gitflow_ancestor(&into.target, &tip, cancel)?;
                    plan.switch(name);
                    plan.steps.push(GitFlowStep::Merge {
                        branch: branch.into(),
                        tip: tip.clone(),
                        into: name.into(),
                        mode,
                        fast_forward,
                        message: format!("Merge branch '{branch}' into {name}"),
                    });
                    Ok::<_, Error>(())
                };
                if let Some(master) = master {
                    if self.gitflow_ancestor(&tip, &master.target, cancel)? {
                        notes.push(GitFlowNote::Merged {
                            into: config.master.clone(),
                        });
                    } else {
                        merge(&mut plan, master, &config.master, MergeMode::NoFastForward)?;
                    }
                    if let Some(name) = tag {
                        plan.steps.push(GitFlowStep::Tag {
                            name,
                            message: options.tag_message.clone(),
                        });
                    }
                }
                if self.gitflow_ancestor(&tip, &develop.target, cancel)? {
                    notes.push(GitFlowNote::Merged {
                        into: config.develop.clone(),
                    });
                } else {
                    // git-flow lets a feature with exactly one commit fast-forward develop.
                    let mode = if kind == GitFlowKind::Feature
                        && !options.no_ff
                        && self.gitflow_count(&develop.target, &tip, cancel)? <= 1
                    {
                        MergeMode::AllowFastForward
                    } else {
                        MergeMode::NoFastForward
                    };
                    merge(&mut plan, develop, &config.develop, mode)?;
                }
                if kind == GitFlowKind::Hotfix
                    && let Some(release) = branches.iter().find_map(|b| {
                        let name = b.reference.strip_prefix(b"refs/heads/")?;
                        let name = std::str::from_utf8(name).ok()?;
                        (b.kind == BranchKind::Local
                            && config
                                .classify(name)
                                .is_some_and(|(k, _)| k == GitFlowKind::Release))
                        .then(|| name.to_owned())
                    })
                {
                    notes.push(GitFlowNote::ReleaseBranch { branch: release });
                }
                // An upstream that this finish merges into (develop, and the production
                // branch for releases and hotfixes) contains the branch once it is deleted.
                let merged_into = |reference: &[u8]| {
                    let name = reference.strip_prefix(b"refs/heads/").unwrap_or_default();
                    name == config.develop.as_bytes()
                        || (kind.tagged() && name == config.master.as_bytes())
                };
                let upstream = source
                    .upstream
                    .as_ref()
                    .filter(|reference| !merged_into(reference))
                    .and_then(|reference| branches.iter().find(|b| b.reference == *reference));
                let keep = if !options.delete_branch {
                    Some(GitFlowKeep::Requested)
                } else if let Some(path) =
                    source.checked_out_at.as_ref().filter(|_| !source.is_head)
                {
                    Some(GitFlowKeep::CheckedOut(path.clone()))
                } else if let Some(upstream) = upstream
                    && !self.gitflow_ancestor(&tip, &upstream.target, cancel)?
                {
                    // With an upstream, Git's safe delete checks the upstream, not HEAD.
                    let name = upstream
                        .reference
                        .strip_prefix(b"refs/remotes/")
                        .or_else(|| upstream.reference.strip_prefix(b"refs/heads/"))
                        .unwrap_or(&upstream.reference);
                    Some(GitFlowKeep::Unpushed(String::from_utf8_lossy(name).into()))
                } else {
                    None
                };
                if keep.is_none() {
                    // git-flow ends on develop; without an upstream, Git's safe delete
                    // then checks the branch against develop, which contains it.
                    plan.switch(&config.develop);
                    plan.steps.push(GitFlowStep::Delete {
                        branch: branch.into(),
                    });
                }
                if plan.steps.is_empty() {
                    return refused(GitFlowFailure::NothingToDo);
                }
                for step in &plan.steps {
                    if let GitFlowStep::Switch { branch } = step
                        && local_branch(&branches, branch)
                            .is_some_and(|b| b.checked_out_at.is_some() && !b.is_head)
                    {
                        return refused(GitFlowFailure::CheckedOutElsewhere);
                    }
                }
                Ok(GitFlowFinishPreview {
                    git_dir: self.info().git_dir.clone(),
                    worktree: self.info().worktree.clone(),
                    kind,
                    name: name.into(),
                    branch: branch.into(),
                    tip,
                    options: options.clone(),
                    config,
                    head,
                    index,
                    tips,
                    steps: plan.steps,
                    keep,
                    notes,
                    ends_on: plan.current,
                })
            })
    }

    /// Revalidate the whole review under the write gate, then run its steps in order
    /// with Rocket's branch, merge and tag transactions. Nothing is pushed.
    pub fn apply_gitflow_finish(
        &self,
        preview: &GitFlowFinishPreview,
        cancel: &CancellationToken,
    ) -> Result<GitFlowFinishReport> {
        self.require_worktree(cancel)?;
        let _permit = self.acquire_write(cancel)?;
        self.runner
            .step("gitflow-finish", "revalidate_gitflow_finish", || {
                if self.info().git_dir != preview.git_dir
                    || self.info().worktree != preview.worktree
                    || self.preview_gitflow_finish(&preview.branch, &preview.options, cancel)?
                        != *preview
                {
                    return refused(GitFlowFailure::StalePreview);
                }
                cancel.check()
            })?;
        let mut steps = Vec::new();
        let mut skipped = Vec::new();
        let mut cancelled = false;
        let mut failure = None;
        for (i, step) in preview.steps.iter().enumerate() {
            if cancel.is_cancelled() {
                cancelled = true;
                skipped = preview.steps[i..].to_vec();
                break;
            }
            let result = self.runner.step("gitflow-finish", step.label(), || {
                self.gitflow_step(preview, step, cancel)
            });
            let (succeeded, conflicts) = result
                .as_ref()
                .map_or((false, false), |r| (r.succeeded(), r.conflicts()));
            cancelled = matches!(result, Err(Error::Cancelled));
            steps.push((step.clone(), result));
            if !succeeded {
                failure = Some(if conflicts {
                    GitFlowFailure::Conflicts
                } else {
                    GitFlowFailure::StepFailed
                });
                skipped = preview.steps[i + 1..].to_vec();
                break;
            }
        }
        // A fresh token: cancellation must not suppress reconciliation.
        let fresh = CancellationToken::new();
        let state = self
            .runner
            .observe("refresh", "operation", || self.operation_state(&fresh));
        let branches = self
            .runner
            .observe("refresh", "branches", || self.branches(&fresh));
        Ok(GitFlowFinishReport {
            steps,
            skipped,
            cancelled,
            failure,
            state,
            branches,
        })
    }

    fn gitflow_step(
        &self,
        preview: &GitFlowFinishPreview,
        step: &GitFlowStep,
        cancel: &CancellationToken,
    ) -> Result<GitFlowStepReport> {
        match step {
            GitFlowStep::Switch { branch } => {
                let name = branch_os(self, branch, cancel)?;
                Ok(GitFlowStepReport::Branch(
                    self.switch_branch_to(name, cancel)?,
                ))
            }
            GitFlowStep::Merge {
                tip,
                into,
                mode,
                message,
                ..
            } => {
                // The previous switch made `into` current; never merge into anything else.
                if !matches!(self.head(cancel)?, Head::Branch { name, .. } if name == into.as_bytes())
                {
                    return refused(GitFlowFailure::StalePreview);
                }
                let arguments =
                    self.merge_commit_arguments_with_message(tip, *mode, Some(message), cancel)?;
                Ok(GitFlowStepReport::Merge(self.execute_merge_write(
                    WriteOperation::Merge,
                    &arguments,
                    Vec::new(),
                    cancel,
                )?))
            }
            GitFlowStep::Tag { name, message } => {
                let production = format!("refs/heads/{}", preview.config.master);
                let target = self.resolve_commit(&production, cancel)?;
                // Tag only a production commit that contains the finished branch.
                if !self.gitflow_ancestor(&preview.tip, &target, cancel)? {
                    return refused(GitFlowFailure::StalePreview);
                }
                let tag = self.preview_create_tag(name, target.as_str(), Some(message), cancel)?;
                Ok(GitFlowStepReport::Tag(self.apply_tag_locked(&tag, cancel)?))
            }
            GitFlowStep::Delete { branch } => {
                let name = branch_os(self, branch, cancel)?;
                Ok(GitFlowStepReport::Branch(
                    self.delete_branch_named(name, cancel)?,
                ))
            }
        }
    }
}
