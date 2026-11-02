//! Reviewed linear rebases, including bounded data-only interactive plans.
use crate::{commit_actions::WorkingFile, parse, runner::args, *};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const MAX_COMMITS: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RebaseAction {
    Start,
    Interactive,
    Continue,
    Skip,
    Abort,
}
impl RebaseAction {
    pub fn is_start(self) -> bool {
        matches!(self, Self::Start | Self::Interactive)
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Start => "rebase",
            Self::Interactive => "interactive-rebase",
            Self::Continue => "rebase-continue",
            Self::Skip => "rebase-skip",
            Self::Abort => "rebase-abort",
        }
    }
    fn operation(self) -> WriteOperation {
        match self {
            Self::Start | Self::Interactive => WriteOperation::Rebase,
            Self::Continue => WriteOperation::ContinueRebase,
            Self::Skip => WriteOperation::SkipRebase,
            Self::Abort => WriteOperation::AbortRebase,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RebaseFailure {
    InvalidPlan,
    EditorUnavailable,
    PlanJournal,
    SkipWouldFoldBase,
    MessageNotSaved,
    MessageOnlyStop,
    StalePreview,
    RecoveryBranchMoved,
    WrongRepository,
    BranchRequired,
    ActiveOperation,
    DirtyWorktree,
    UnsupportedIndex,
    ConflictsRemain,
    EmptyChange,
    UnsupportedSequence,
    MergeCommits,
    UnrelatedHistory,
    RootConflict,
    PreviewTooLarge,
    LocalFileCollision,
    RecoveryReferenceConflict,
    IndexLocked,
    IdentityMissing,
    SigningFailed,
    Paused,
    StagedAfterStop,
    CommandStopped,
    CommandFailed,
}
impl RebaseFailure {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidPlan => "invalid_rebase_plan",
            Self::EditorUnavailable => "rebase_editor_unavailable",
            Self::PlanJournal => "rebase_plan_journal",
            Self::SkipWouldFoldBase => "skip_would_fold_base",
            Self::MessageNotSaved => "reviewed_message_not_saved",
            Self::MessageOnlyStop => "message_only_stop",
            Self::StalePreview => "stale_preview",
            Self::RecoveryBranchMoved => "recovery_branch_moved",
            Self::WrongRepository => "wrong_repository",
            Self::BranchRequired => "branch_required",
            Self::ActiveOperation => "active_operation",
            Self::DirtyWorktree => "dirty_worktree",
            Self::UnsupportedIndex => "unsupported_index",
            Self::ConflictsRemain => "conflicts_remain",
            Self::EmptyChange => "empty_change",
            Self::UnsupportedSequence => "unsupported_rebase",
            Self::MergeCommits => "merge_commits",
            Self::UnrelatedHistory => "unrelated_history",
            Self::RootConflict => "root_conflict",
            Self::PreviewTooLarge => "preview_too_large",
            Self::LocalFileCollision => "local_file_collision",
            Self::RecoveryReferenceConflict => "recovery_ref_conflict",
            Self::IndexLocked => "index_locked",
            Self::IdentityMissing => "identity_missing",
            Self::SigningFailed => "signing_failed",
            Self::Paused => "rebase_paused",
            Self::StagedAfterStop => "staged_after_stop",
            Self::CommandStopped => "planned_command_stopped",
            Self::CommandFailed => "command_failed",
        }
    }
    pub fn guidance(self) -> &'static str {
        match self {
            Self::InvalidPlan => {
                "Review every original commit exactly once. Squash/fixup need a preceding kept commit. Reword/squash need a nonblank message (up to 64 KiB each, 512 KiB total). Stops and commands follow a kept commit outside a squash/fixup chain; a command is one line of up to 1,024 characters. The replay range must contain 1 to 200 commits."
            }
            Self::StagedAfterStop => {
                "This planned stop is between commits, so Git cannot continue with staged changes and Rocket does not create extra commits during a rebase. Unstage or discard them (Skip discards tracked changes and continues), or abort."
            }
            Self::CommandStopped => {
                "Git stopped after a planned command: it failed or left changes. Review Changes and the optional local Git output, fix or discard the changes, then Continue; Skip discards tracked changes and continues; Abort restores the original branch."
            }
            Self::EditorUnavailable => {
                "The bundled Rocket rebase editor is missing. Rebuild or restore rocket-rebase-editor beside the application before retrying; no rebase was started."
            }
            Self::PlanJournal => {
                "The saved interactive review is missing, changed, or left over from an interrupted setup. Inspect the rebase state and rocket-interactive-plan in the worktree Git directory with Git before retrying. Rocket will not replace that saved review or guess a plan."
            }
            Self::SkipWouldFoldBase => {
                "Skipping this commit would leave squash/fixup without a preceding replayed commit. Continue with a resolution, or abort and prepare a new plan; the commit underneath will not be folded into your changes."
            }
            Self::MessageNotSaved => {
                "The changes were replayed, but Git did not save the reviewed message. Check the local Console and commit hooks, then review Continue to retry that message on the pinned replayed commit. Do not skip a commit merely because its index is now clean."
            }
            Self::MessageOnlyStop => {
                "This stop is at the message-saving step after the changes were already replayed. Review Continue to finish the saved plan, or Abort to restore the original history. Skip cannot omit an already replayed commit here."
            }
            Self::StalePreview => {
                "The branch, what it goes on top of, the index, working files or the plan changed. Review it again; nothing was retried."
            }
            Self::RecoveryBranchMoved => {
                "Git still reports a rebase, but the original branch has moved from its saved tip. A late interruption may have happened after commits were applied, or another client moved the branch. Inspect the current history and original-tip recovery reference with Git, then finish or clear the remaining rebase state there. Rocket will not reset or retry that branch."
            }
            Self::WrongRepository => {
                "This preview belongs to another worktree. Prepare a new preview here."
            }
            Self::BranchRequired => {
                "Check out an existing local branch with a commit before starting a rebase."
            }
            Self::ActiveOperation => {
                "Finish or abort the current Git operation before starting a rebase."
            }
            Self::DirtyWorktree => {
                "Commit or stash tracked changes before starting. To continue a paused rebase, stage all intended resolutions and leave no unstaged tracked edits."
            }
            Self::UnsupportedIndex => {
                "The index contains assume-unchanged or skip-worktree entries, which can hide edits from the preview. Review those files and index flags with Git before rebasing here; Rocket has not cleared the flags or changed the files."
            }
            Self::ConflictsRemain => {
                "Fix the conflicts in Changes and stage them, then Continue. During a rebase, Ours is what your commit goes on top of and Theirs is your commit."
            }
            Self::EmptyChange => {
                "The stopped commit has no staged change. Review Skip to omit it, or finish it explicitly with Git. Rocket does not silently drop it."
            }
            Self::UnsupportedSequence => {
                "This is not an unchanged supported Rocket rebase. External, apply-backend, autostashed or modified command sequences must be reviewed and finished with Git."
            }
            Self::MergeCommits => {
                "The replay range contains merge commits. This first rebase flow supports linear history only; Rocket will not silently flatten merges."
            }
            Self::UnrelatedHistory => {
                "This branch and the one you chose share no history, so one can't go on top of the other here. To rewrite the whole branch, use an interactive rebase from the first commit."
            }
            Self::RootConflict => {
                "This is the first commit of a rebase from the root commit. Git would commit this resolution on top of its temporary empty base, which would stay in history as an empty first commit. Skip this commit (its resolution is discarded), or abort and plan a first commit that applies cleanly, such as the original first commit."
            }
            Self::PreviewTooLarge => {
                "This rebase exceeds the bounded review limit (200 commits or bounded metadata). Review a smaller range or use Git."
            }
            Self::LocalFileCollision => {
                "An untracked or ignored file overlaps paths this rebase may restore. Move or back it up before retrying; Rocket has not removed it."
            }
            Self::RecoveryReferenceConflict => {
                "The original-tip recovery reference has a different value or is missing during recovery. Inspect it with Git; Rocket will not overwrite it."
            }
            Self::IndexLocked => {
                "Git could not acquire the index lock. Let other Git work finish, then refresh. Existing lock files are left in place."
            }
            Self::IdentityMissing => {
                "Set the repository's name and email, then review the current rebase state before continuing."
            }
            Self::SigningFailed => {
                "Git could not sign a replayed commit. Check the signing key or agent, then review the current state before continuing."
            }
            Self::Paused => {
                "Rebase is still in progress. Review Changes and the stopped commit; use Continue, Skip or Abort after a fresh preview."
            }
            Self::CommandFailed => {
                "Git did not confirm completion. Review refreshed history and operation state, plus optional local Git output for hooks or configuration errors. Do not blindly retry."
            }
        }
    }
}
fn refused<T>(failure: RebaseFailure) -> Result<T> {
    Err(Error::Rebase(failure))
}

/// A stop the reviewed interactive plan asked for, as opposed to a conflict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RebaseStop {
    /// The original commit was replayed and can now be amended: Continue amends
    /// staged changes into it; a clean index continues unchanged.
    Edit(ObjectId),
    /// A planned break between commits.
    Break,
    /// Git stopped at or after this planned command: it failed or left changes.
    Command(String),
}

/// A local branch, other than the one being rebased, that points at one of the
/// replayed commits. With `--update-refs` it moves to that commit's rewritten
/// copy when the rebase finishes; Git never moves a branch that is checked out
/// in a worktree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DependentBranch {
    /// Full ref name, `refs/heads/…`.
    pub reference: Vec<u8>,
    pub target: ObjectId,
    /// The worktree that has it checked out; such a branch stays where it is.
    pub checked_out_at: Option<PathBuf>,
}
impl DependentBranch {
    pub fn moves(&self) -> bool {
        self.checked_out_at.is_none()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Context {
    head: Head,
    state: OperationState,
    index: Vec<u8>,
    branch_tip: Option<ObjectId>,
    files: Vec<(RepoPath, WorkingFile)>,
    /// Tracked files with changes, each with whether it still has a conflict.
    changes: Vec<(String, bool)>,
    metadata: BTreeMap<String, Vec<u8>>,
}
#[derive(Clone, Debug)]
pub struct RebasePreview {
    git_dir: PathBuf,
    action: RebaseAction,
    target: String,
    context: Context,
    branch: Vec<u8>,
    original: ObjectId,
    /// None for a rebase from the root commit.
    onto: Option<ObjectId>,
    /// A new rebase: the commit `onto` names, which the loaded history may not hold.
    onto_commit: Option<Commit>,
    commits: Vec<Commit>,
    prediction: Option<CommitPrediction>,
    prediction_guard: Option<crate::commit_prediction::PredictionGuard>,
    stopped: Option<ObjectId>,
    affected: Vec<Vec<u8>>,
    patch: Vec<u8>,
    fast_forward: bool,
    up_to_date: bool,
    plan: Option<crate::rebase_plan::Plan>,
    preserve_merges: bool,
    message_recovery: Option<(bool, String)>,
    planned_stop: Option<RebaseStop>,
    /// Branches pointing into the replayed commits; for a rebase in progress,
    /// the branches its `--update-refs` state will move.
    dependents: Vec<DependentBranch>,
    /// Move the dependent branches with the rebase (`--update-refs`).
    update_refs: bool,
}
impl RebasePreview {
    pub fn preserves_merges(&self) -> bool { self.preserve_merges }
    /// Content prediction for an ordinary start. Interactive plans and recovery
    /// actions are not simulated. No replay is needed for a fast-forward/no-op.
    pub fn prediction(&self) -> Option<&CommitPrediction> {
        self.prediction.as_ref()
    }

    /// Local branches other than this one that point at replayed commits. For a
    /// rebase in progress: those it will move when it finishes.
    pub fn dependents(&self) -> &[DependentBranch] {
        &self.dependents
    }
    /// Whether the dependent branches move with the rebase (`--update-refs`).
    pub fn update_refs(&self) -> bool {
        self.update_refs
    }
    /// Start the rebase with or without `--update-refs`. Only a new rebase can
    /// choose; one in progress keeps what it started with.
    pub fn with_update_refs(&self, on: bool) -> Result<Self> {
        if on == self.update_refs {
            return Ok(self.clone());
        }
        if !self.action.is_start() || (on && (self.preserve_merges || !self.dependents.iter().any(|b| b.moves()))) {
            return refused(RebaseFailure::InvalidPlan);
        }
        let mut result = self.clone();
        result.update_refs = on;
        let updates = result.planned_updates();
        if let Some(plan) = &mut result.plan {
            plan.update_refs = updates;
        }
        Ok(result)
    }
    /// The branches an interactive plan moves, with the commits they point at.
    fn planned_updates(&self) -> Vec<(ObjectId, Vec<u8>)> {
        if !self.update_refs {
            return Vec::new();
        }
        self.dependents
            .iter()
            .filter(|b| b.moves())
            .map(|b| (b.target.clone(), b.reference.clone()))
            .collect()
    }
    /// The planned stop this Continue/Skip/Abort resumes from, if any.
    pub fn planned_stop(&self) -> Option<&RebaseStop> {
        self.planned_stop.as_ref()
    }
    /// The reviewed plan runs commands; callers should allow a longer command limit.
    pub fn runs_commands(&self) -> bool {
        self.plan
            .as_ref()
            .is_some_and(crate::rebase_plan::Plan::runs_commands)
    }
    /// A clean-index pause while saving a reviewed message, not an empty replay.
    pub fn message_recovery(&self) -> Option<(bool, &str)> {
        self.message_recovery
            .as_ref()
            .map(|(retry, message)| (*retry, message.as_str()))
    }
    pub fn validate_plan(&self, entries: &[RebasePlanEntry]) -> Result<()> {
        if self.action != RebaseAction::Interactive {
            return refused(RebaseFailure::InvalidPlan);
        }
        crate::rebase_plan::Plan::validate_entries(
            entries,
            &self
                .commits
                .iter()
                .map(|c| c.id.clone())
                .collect::<Vec<_>>(),
        )
    }
    pub fn plan(&self) -> Option<&[RebasePlanEntry]> {
        self.plan.as_ref().filter(|p| !p.preserve_merges).map(|p| p.entries.as_slice())
    }
    pub fn with_plan(&self, entries: Vec<RebasePlanEntry>) -> Result<Self> {
        self.validate_plan(&entries)?;
        let plan = crate::rebase_plan::Plan {
            preserve_merges: false,
            original: self.original.clone(),
            onto: self.onto.clone(),
            branch: self.branch.clone(),
            entries,
            update_refs: self.planned_updates(),
        };
        plan.validate(
            &self
                .commits
                .iter()
                .map(|c| c.id.clone())
                .collect::<Vec<_>>(),
        )?;
        let mut result = self.clone();
        result.plan = Some(plan);
        Ok(result)
    }
    pub fn action(&self) -> RebaseAction {
        self.action
    }
    pub fn branch(&self) -> &[u8] {
        &self.branch
    }
    pub fn original(&self) -> &ObjectId {
        &self.original
    }
    /// The destination, or None when the rebase rewrites the branch from its root
    /// commit and the first kept commit becomes the new root.
    pub fn onto(&self) -> Option<&ObjectId> {
        self.onto.as_ref()
    }
    /// The commit a new rebase goes on top of. A rebase in progress has none.
    pub fn onto_commit(&self) -> Option<&Commit> {
        self.onto_commit.as_ref()
    }
    /// What a new rebase was asked to go on top of, as it was typed.
    pub fn target(&self) -> &str {
        &self.target
    }
    pub fn commits(&self) -> &[Commit] {
        &self.commits
    }
    /// The tracked files this review saw changed, each with whether it has a conflict.
    pub fn changes(&self) -> &[(String, bool)] {
        &self.context.changes
    }
    pub fn stopped(&self) -> Option<&ObjectId> {
        self.stopped.as_ref()
    }
    pub fn patch(&self) -> &[u8] {
        &self.patch
    }
    pub fn fast_forward(&self) -> bool {
        self.fast_forward
    }
    pub fn up_to_date(&self) -> bool {
        self.up_to_date
    }
    pub fn recovery_reference(&self) -> String {
        reference(&self.original)
    }
}
#[derive(Debug)]
#[must_use = "inspect command, refresh and recovery state; cancellation is not rollback"]
pub struct RebaseWriteReport {
    pub write: WriteReport,
    pub state: Result<OperationState>,
    pub branches: Result<Vec<Branch>>,
    pub failure: Option<RebaseFailure>,
    pub recovery_reference: String,
    pub journal_cleanup: Result<()>,
    /// The planned stop the rebase is paused at after this command, if any.
    pub stop: Option<RebaseStop>,
}

impl Repository {
    pub fn preview_rebase(
        &self,
        action: RebaseAction,
        target: &str,
        cancel: &CancellationToken,
    ) -> Result<RebasePreview> {
        self.preview_rebase_from(action, Some(target), false, cancel)
    }
    /// Preserve Git's merge topology with no-rebase-cousins semantics. Interactive
    /// row editing and dependent-branch updates remain separate linear workflows.
    pub fn preview_rebase_preserving_merges(&self, target: &str, cancel: &CancellationToken) -> Result<RebasePreview> {
        self.preview_rebase_from(RebaseAction::Start, Some(target), true, cancel)
    }

    /// Review an interactive rebase of the whole current branch (`git rebase
    /// --root`): the plan holds every commit, and the first kept commit becomes the
    /// new root commit. Continue, Skip and Abort are reviewed with `preview_rebase`.
    pub fn preview_root_rebase(&self, cancel: &CancellationToken) -> Result<RebasePreview> {
        self.preview_rebase_from(RebaseAction::Interactive, None, false, cancel)
    }

    /// `target` is None to start from the root commit; continuations ignore it.
    fn preview_rebase_from(
        &self,
        action: RebaseAction,
        target: Option<&str>,
        preserve_merges: bool,
        cancel: &CancellationToken,
    ) -> Result<RebasePreview> {
        self.runner.step("rebase-preview", "review_plan", || {
            self.require_worktree(cancel)?;
            let context = self.rebase_context(action, cancel)?;
            // `base` is the commit Git replays onto. A root rebase has no destination,
            // and Git creates its empty base only when the rebase starts.
            let (branch, original, onto, base, stopped) = if action.is_start() {
                let Head::Branch {
                    name,
                    target: original,
                } = &context.head
                else {
                    return refused(RebaseFailure::BranchRequired);
                };
                let onto = target
                    .map(|target| self.resolve_commit(target, cancel))
                    .transpose()?;
                (name.clone(), original.clone(), onto.clone(), onto, None)
            } else {
                let m = &context.metadata;
                let branch = value(m, "head-name")?
                    .strip_prefix(b"refs/heads/")
                    .ok_or(Error::Rebase(RebaseFailure::UnsupportedSequence))?
                    .to_vec();
                let original = parse::oid(value(m, "orig-head")?, "rebase original")?;
                let base = parse::oid(value(m, "onto")?, "rebase destination")?;
                // Git names the empty base of a root rebase in squash-onto as well.
                let onto = match m.get("squash-onto").map(|id| parse::line(id)) {
                    None | Some(b"") => Some(base.clone()),
                    Some(id) if id == base.as_str().as_bytes() => None,
                    Some(_) => return refused(RebaseFailure::UnsupportedSequence),
                };
                let stopped = m
                    .get("stopped-sha")
                    .map(|b| parse::oid(parse::line(b), "stopped commit"))
                    .transpose()?;
                self.verify_rebase_reference(&original, false, cancel)?;
                // A different client moving the original branch makes abort/finish unsafe.
                let mut full = b"refs/heads/".to_vec();
                full.extend(&branch);
                let mut arguments = args(&["show-ref", "--verify", "--hash", "--"]);
                arguments.push(model::bytes_to_os(&full)?);
                let actual = self
                    .runner
                    .run(self.cwd(), &arguments, Vec::new(), cancel)?
                    .success("rebase branch")?;
                if parse::oid(parse::line(&actual), "rebase branch")? != original {
                    return refused(RebaseFailure::RecoveryBranchMoved);
                }
                (branch, original, onto, Some(base), stopped)
            };
            let plan_saved = context
                .metadata
                .get(crate::rebase_plan::JOURNAL_FILE)
                .map(|b| crate::rebase_plan::Plan::decode(b))
                .transpose()?;
            let preserve_merges = preserve_merges || plan_saved.as_ref().is_some_and(|p|p.preserve_merges);
            // Rocket continues only the root rebases it started from a reviewed plan.
            if onto.is_none() && !action.is_start() && plan_saved.is_none() {
                return refused(RebaseFailure::UnsupportedSequence);
            }
            let interactive = action == RebaseAction::Interactive || plan_saved.is_some();
            let (mut up_to_date, mut fast_forward) = (false, false);
            // A root rebase replays every commit of the branch onto an empty base.
            if let Some(onto) = &onto {
                up_to_date = !interactive && self.rebase_ancestor(onto, &original, cancel)?;
                fast_forward = !up_to_date && self.rebase_ancestor(&original, onto, cancel)?;
                let common = self.runner.run(
                    self.cwd(),
                    &args(&["merge-base", original.as_str(), onto.as_str()]),
                    Vec::new(),
                    cancel,
                )?;
                if common.status.code() == Some(1) {
                    return refused(RebaseFailure::UnrelatedHistory);
                }
                common.success("rebase common ancestor")?;
            }
            let ids = if up_to_date {
                Vec::new()
            } else {
                let range = match &onto {
                    Some(onto) => format!("{onto}..{original}"),
                    None => original.to_string(),
                };
                let bytes = self.rebase_read(
                    &[
                        "rev-list",
                        "--reverse",
                        "--topo-order",
                        "--max-count=201",
                        &range,
                        "--",
                    ],
                    cancel,
                )?;
                bytes
                    .split(|b| *b == b'\n')
                    .filter(|s| !s.is_empty())
                    .map(|s| parse::oid(s, "rebase commits"))
                    .collect::<Result<Vec<_>>>()?
            };
            if ids.len() > MAX_COMMITS {
                return refused(RebaseFailure::PreviewTooLarge);
            }
            if !action.is_start() {
                if let Some(plan) = &plan_saved {
                    if plan.original != original || plan.onto != onto || plan.branch != branch {
                        return refused(RebaseFailure::UnsupportedSequence);
                    }
                    plan.validate(&ids)?;
                    if action == RebaseAction::Abort
                        && !context.metadata.contains_key(crate::rebase_plan::PLAN_FILE)
                    {
                        // Cancellation before the sequence editor: only allow abort of
                        // Git's untouched original pick list, never continue that list.
                        if context
                            .metadata
                            .get("done")
                            .is_some_and(|b| !b.trim_ascii().is_empty())
                        {
                            return refused(RebaseFailure::UnsupportedSequence);
                        }
                        if plan.preserve_merges {
                            let mut todo=context.metadata.get("done").cloned().unwrap_or_default();todo.push(b'\n');
                            todo.extend(context.metadata.get("git-rebase-todo").map_or(&[][..],Vec::as_slice));
                            crate::rebase_merges::canonical(&todo,&ids,true)?;
                        } else { verify_replay_order(&context.metadata, &ids)?; }
                    } else {
                        plan.verify_sequence(&context.metadata)?;
                    }
                    if action == RebaseAction::Skip
                        && let Some(stopped) = &stopped
                    {
                        if self
                            .rebase_message_recovery(&context.metadata, &context.head, cancel)?
                            .is_some()
                        {
                            return refused(RebaseFailure::MessageOnlyStop);
                        }
                        let stop = plan
                            .entries
                            .iter()
                            .position(|r| r.commit == *stopped)
                            .ok_or(Error::Rebase(RebaseFailure::UnsupportedSequence))?;
                        let preceding = match &context.head {
                            Head::Branch { target, .. } | Head::Detached { target } => {
                                Some(target) != base.as_ref()
                            }
                            Head::Unborn { .. } => false,
                        };
                        let following = plan.entries[stop + 1..]
                            .iter()
                            .find(|r| r.instruction != RebaseInstruction::Drop);
                        if !preceding
                            && following.is_some_and(|r| {
                                matches!(
                                    r.instruction,
                                    RebaseInstruction::Squash | RebaseInstruction::Fixup
                                )
                            })
                        {
                            return refused(RebaseFailure::SkipWouldFoldBase);
                        }
                    }
                } else {
                    verify_replay_order(&context.metadata, &ids)?;
                }
            }
            let commits = if ids.is_empty() {
                Vec::new()
            } else {
                let input = ids
                    .iter()
                    .map(|id| format!("{id}\n"))
                    .collect::<String>()
                    .into_bytes();
                let objects = self
                    .runner
                    .run(self.cwd(), &args(&["cat-file", "--batch"]), input, cancel)?
                    .success("rebase commits")?;
                parse::commits(&objects, &ids)?
            };
            if !preserve_merges && commits.iter().any(|c| c.parents.len() > 1) {
                return refused(RebaseFailure::MergeCommits);
            }
            let onto_commit = match onto.as_ref().filter(|_| action.is_start()) {
                Some(id) => {
                    let objects = self
                        .runner
                        .run(self.cwd(), &args(&["cat-file", "--batch"]), format!("{id}\n").into_bytes(), cancel)?
                        .success("rebase commits")?;
                    parse::commits(&objects, std::slice::from_ref(id))?.pop()
                }
                None => None,
            };
            let plan = if action == RebaseAction::Interactive || (action.is_start() && preserve_merges && !ids.is_empty()) {
                let entries = commits
                    .iter()
                    .map(|c| RebasePlanEntry::new(c.id.clone(), RebaseInstruction::Pick))
                    .collect();
                let p = crate::rebase_plan::Plan {
                    preserve_merges,
                    original: original.clone(),
                    onto: onto.clone(),
                    branch: branch.clone(),
                    entries,
                    update_refs: Vec::new(),
                };
                p.validate(&ids)?;
                Some(p)
            } else {
                plan_saved
            };
            let mut affected = Vec::new();
            let mut attributes_changing = false;
            for id in std::iter::once(&original).chain(&base) {
                affected.extend(paths(self.rebase_read(
                    &["ls-tree", "-r", "--name-only", "-z", id.as_str()],
                    cancel,
                )?));
            }
            if !ids.is_empty() {
                let input = ids
                    .iter()
                    .map(|id| format!("{id}\n"))
                    .collect::<String>()
                    .into_bytes();
                let bytes = self
                    .runner
                    .run(
                        self.cwd(),
                        &args(&[
                            "diff-tree",
                            "--stdin",
                            "--root",
                            "-r",
                            "--no-renames",
                            "--no-commit-id",
                            "--name-only",
                            "-z",
                        ]),
                        input,
                        cancel,
                    )?
                    .success("rebase paths")?;
                let changed = paths(bytes);
                attributes_changing = changed
                    .iter()
                    .any(|p| crate::commit_prediction::is_attributes(p));
                affected.extend(changed);
            }
            affected.sort();
            affected.dedup();
            self.protect_rebase_files(&affected, cancel)?;
            let mut patch = Vec::new();
            if !action.is_start() {
                // Show the index and working changes separately: their net diff
                // can be empty even though abort/skip would discard both edits.
                patch.extend_from_slice(b"Staged changes\n");
                patch.extend(self.rebase_read(
                    &[
                        "-c",
                        "core.quotepath=false",
                        "diff",
                        "--cached",
                        "--no-ext-diff",
                        "--no-textconv",
                        "--no-color",
                        "--no-renames",
                        "--binary",
                        "HEAD",
                        "--",
                    ],
                    cancel,
                )?);
                if action != RebaseAction::Continue {
                    patch.extend_from_slice(b"\nUnstaged changes\n");
                    patch.extend(self.rebase_read(
                        &[
                            "-c",
                            "core.quotepath=false",
                            "diff",
                            "--no-ext-diff",
                            "--no-textconv",
                            "--no-color",
                            "--no-renames",
                            "--binary",
                            "--",
                        ],
                        cancel,
                    )?);
                }
            }
            let (prediction, prediction_guard) = if action == RebaseAction::Start
                && !preserve_merges
                && !commits.is_empty()
            {
                let onto = onto
                    .as_ref()
                    .ok_or(Error::Rebase(RebaseFailure::UnsupportedSequence))?;
                // Attributes on the fetched/destination tree can differ from the
                // current checkout even if no replay commit edits those rules.
                let changed = paths(self.rebase_read(
                    &[
                        "diff-tree",
                        "-r",
                        "--no-renames",
                        "--no-commit-id",
                        "--name-only",
                        "-z",
                        original.as_str(),
                        onto.as_str(),
                        "--",
                    ],
                    cancel,
                )?);
                attributes_changing |= changed
                    .iter()
                    .any(|p| crate::commit_prediction::is_attributes(p));
                let (p, guard) =
                    self.predict_rebase(onto, &commits, &affected, attributes_changing, cancel)?;
                (Some(p), guard)
            } else {
                (None, None)
            };
            // Prediction is read-only and may take time. Discard its result if
            // the checkout or named destination changed while it was running.
            if context != self.rebase_context(action, cancel)? {
                return refused(RebaseFailure::StalePreview);
            }
            if action.is_start()
                && let (Some(target), Some(onto)) = (target, &onto)
                && self.resolve_commit(target, cancel)? != *onto
            {
                return refused(RebaseFailure::StalePreview);
            }
            let message_recovery = if action == RebaseAction::Continue {
                self.rebase_message_recovery(&context.metadata, &context.head, cancel)?
            } else {
                None
            };
            let planned_stop = if action.is_start() {
                None
            } else {
                planned_stop(&context.metadata, &context.head)?
            };
            let dependents = if action.is_start() {
                self.rebase_dependents(&branch, &ids, cancel)?
            } else {
                update_refs_state(&context.metadata)?
            };
            Ok(RebasePreview {
                git_dir: self.info().git_dir.clone(),
                action,
                target: target.unwrap_or_default().into(),
                context,
                branch,
                original,
                onto,
                onto_commit,
                commits,
                prediction,
                prediction_guard,
                stopped,
                affected,
                patch,
                fast_forward,
                up_to_date,
                plan,
                preserve_merges,
                message_recovery,
                planned_stop,
                update_refs: !action.is_start() && !dependents.is_empty(),
                dependents,
            })
        })
    }

    /// Local branches other than `branch` that point at one of `ids`, as
    /// `--update-refs` would find them.
    fn rebase_dependents(
        &self,
        branch: &[u8],
        ids: &[ObjectId],
        cancel: &CancellationToken,
    ) -> Result<Vec<DependentBranch>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut current = b"refs/heads/".to_vec();
        current.extend_from_slice(branch);
        Ok(self
            .branches(cancel)?
            .into_iter()
            .filter(|b| {
                b.reference.starts_with(b"refs/heads/")
                    && b.reference != current
                    && b.symbolic_target.is_none()
                    && ids.contains(&b.target)
                    && crate::rebase_plan::branch_ref(&b.reference)
            })
            .map(|b| DependentBranch {
                reference: b.reference,
                target: b.target,
                checked_out_at: b.checked_out_at,
            })
            .collect())
    }

    pub fn apply_rebase(
        &self,
        preview: &RebasePreview,
        cancel: &CancellationToken,
    ) -> Result<RebaseWriteReport> {
        self.require_worktree(cancel)?;
        let _permit = self.acquire_write(cancel)?;
        self.apply_rebase_locked(preview, cancel)
    }

    /// Caller holds the common-directory write gate, including any enclosing workflow checks.
    pub(crate) fn apply_rebase_locked(
        &self,
        preview: &RebasePreview,
        cancel: &CancellationToken,
    ) -> Result<RebaseWriteReport> {
        let validate = || -> Result<()> {
            if preview.git_dir != self.info().git_dir {
                return refused(RebaseFailure::WrongRepository);
            }
            if self.rebase_context(preview.action, cancel)? != preview.context {
                return refused(RebaseFailure::StalePreview);
            }
            if preview.action.is_start()
                && let Some(onto) = &preview.onto
                && self.resolve_commit(&preview.target, cancel)? != *onto
            {
                return refused(RebaseFailure::StalePreview);
            }
            // The branches `--update-refs` will move must still be the reviewed ones.
            if preview.action.is_start() && preview.update_refs {
                let ids: Vec<ObjectId> = preview.commits.iter().map(|c| c.id.clone()).collect();
                if self.rebase_dependents(&preview.branch, &ids, cancel)? != preview.dependents {
                    return refused(RebaseFailure::StalePreview);
                }
            }
            if let Some(guard) = &preview.prediction_guard
                && !self.prediction_is_current(guard, cancel)?
            {
                return refused(RebaseFailure::StalePreview);
            }
            self.protect_rebase_files(&preview.affected, cancel)
        };
        self.runner
            .step(preview.action.label(), "revalidate_preview", validate)?;
        if let (Some(log), Some(plan)) = (&self.runner.log, &preview.plan) {
            log.rebase_plan(&plan.entries);
        }
        let editor = self
            .runner
            .step(preview.action.label(), "prepare_plan_editor", || {
                preview
                    .plan
                    .as_ref()
                    .map(|plan| {
                        plan.validate(
                            &preview
                                .commits
                                .iter()
                                .map(|c| c.id.clone())
                                .collect::<Vec<_>>(),
                        )?;
                        crate::rebase_plan::Editor::new(
                            &self.info().git_dir,
                            preview.action.is_start().then_some(plan),
                        )
                    })
                    .transpose()
            })?;
        self.runner
            .step(preview.action.label(), "save_recovery_reference", || {
                self.verify_rebase_reference(&preview.original, preview.action.is_start(), cancel)
            })?;
        // Check again after the recovery-ref write. Other Git clients do not share our gate.
        validate()?;
        // An instructionFormat containing newlines can add todo commands. Pin a
        // single subject field; hooks/signing remain Git's configured behavior.
        let mut arguments = args(&[
            "-c",
            "submodule.recurse=false",
            "-c",
            "rebase.instructionFormat=%s",
            "-c",
            "rebase.abbreviateCommands=false",
            "-c",
            "rebase.updateRefs=false",
            "-c",
            "rebase.autoStash=false",
            "-c",
            "rerere.autoupdate=false",
            "-c",
            "commit.cleanup=whitespace",
            "rebase",
        ]);
        if preview.action.is_start() {
            if preview.action == RebaseAction::Interactive || (preview.preserve_merges && preview.plan.is_some()) {
                arguments.extend(args(&["--interactive", "--force-rebase"]));
            }
            arguments.extend(args(&[
                "--merge",
                "--no-stat",
                "--strategy=ort",
                "--no-autostash",
                if preview.update_refs {
                    "--update-refs"
                } else {
                    "--no-update-refs"
                },
                "--no-autosquash",
                if preview.preserve_merges { "--rebase-merges=no-rebase-cousins" } else { "--no-rebase-merges" },
                "--no-fork-point",
                "--no-rerere-autoupdate",
                "--no-reschedule-failed-exec",
                "--reapply-cherry-picks",
                "--keep-empty",
                "--empty=stop",
                "--verify",
            ]));
            match &preview.onto {
                Some(onto) => arguments.extend(args(&["--", onto.as_str()])),
                // Git creates an empty base; the first kept commit becomes the new root.
                None => arguments.extend(args(&["--root"])),
            }
        } else {
            arguments.push(
                match preview.action {
                    RebaseAction::Continue => "--continue",
                    RebaseAction::Skip => "--skip",
                    RebaseAction::Abort => "--abort",
                    _ => unreachable!(),
                }
                .into(),
            );
        }
        let message_write = if let Some((true, message)) = &preview.message_recovery {
            Some(
                self.runner
                    .step(preview.action.label(), "retry_reviewed_message", || {
                        self.execute_write(
                            WriteOperation::ContinueRebase,
                            &args(&[
                                "commit",
                                "--amend",
                                "--file=-",
                                "--no-edit",
                                "--cleanup=whitespace",
                            ]),
                            message.as_bytes().to_vec(),
                            cancel,
                        )
                    })?,
            )
        } else {
            None
        };
        let write = if let Some(report) = message_write.filter(|r| r.result.is_err()) {
            report
        } else {
            // A message amendment may already have succeeded. Even cancellation
            // before the following command must reconcile, never hide that write.
            match self.execute_write_with_editor(
                preview.action.operation(),
                &arguments,
                Vec::new(),
                cancel,
                editor.as_ref(),
            ) {
                Ok(report) => report,
                Err(error) => WriteReport {
                    operation: preview.action.operation(),
                    result: Err(error),
                    status: self.status(&CancellationToken::new()),
                },
            }
        };
        let state = self.runner.observe("refresh", "operation", || {
            self.operation_state(&CancellationToken::new())
        });
        let branches = self.runner.observe("refresh", "branches", || {
            self.branches(&CancellationToken::new())
        });
        let mut branch_ref = b"refs/heads/".to_vec();
        branch_ref.extend(&preview.branch);
        let branch_moved = state.as_ref().is_ok_and(|s| s.rebase)
            && branches.as_ref().is_ok_and(|branches| {
                !branches
                    .iter()
                    .any(|b| b.reference == branch_ref && b.target == preview.original)
            });
        let mut failure = if branch_moved {
            Some(RebaseFailure::RecoveryBranchMoved)
        } else {
            classify(&write, &state)
        };
        if failure == Some(RebaseFailure::Paused) && preview.plan.is_some() {
            let directory = self.info().git_dir.join("rebase-merge");
            if let (Ok(marker), Ok(stopped), Ok(status)) = (
                crate::rebase_plan::read(&directory.join(crate::rebase_plan::MESSAGE_FILE)),
                crate::rebase_plan::read(&directory.join("stopped-sha")),
                &write.status,
            ) {
                let mut lines = marker.split(|b| *b == b'\n');
                let original = lines.next().unwrap_or_default();
                let before = lines.next().unwrap_or_default();
                if original == stopped.trim_ascii()
                    && matches!(&status.head,Head::Detached{target} if target.as_str().as_bytes()==before)
                {
                    failure = Some(RebaseFailure::MessageNotSaved);
                }
            }
        }
        // A reviewed edit or break pauses successfully; a planned command stops
        // when it fails or leaves changes. Conflicts keep their own classification.
        let stop = match (&state, &write.status) {
            (Ok(s), Ok(status)) if s.rebase && preview.plan.is_some() && !branch_moved => metadata(
                &self.info().git_dir.join("rebase-merge"),
                &CancellationToken::new(),
            )
            .ok()
            .and_then(|m| planned_stop(&m, &status.head).ok().flatten()),
            _ => None,
        };
        match &stop {
            Some(RebaseStop::Edit(_) | RebaseStop::Break) if write.result.is_ok() => {
                failure = None;
            }
            Some(RebaseStop::Command(_)) if failure != Some(RebaseFailure::ConflictsRemain) => {
                failure = Some(RebaseFailure::CommandStopped);
            }
            _ => {}
        }
        self.runner
            .step(
                preview.action.label(),
                "classify_outcome",
                || match failure {
                    Some(f) => Err(Error::Rebase(f)),
                    None => Ok(()),
                },
            )
            .ok();
        let journal_cleanup =
            self.runner
                .step(preview.action.label(), "cleanup_plan_journal", || {
                    editor.as_ref().map_or(Ok(()), |editor| editor.cleanup())
                });
        Ok(RebaseWriteReport {
            write,
            state,
            branches,
            failure,
            recovery_reference: preview.recovery_reference(),
            journal_cleanup,
            stop,
        })
    }

    fn rebase_read(&self, arguments: &[&str], cancel: &CancellationToken) -> Result<Vec<u8>> {
        self.runner
            .run(self.cwd(), &args(arguments), Vec::new(), cancel)?
            .success("rebase preview")
    }
    fn rebase_message_recovery(
        &self,
        metadata: &BTreeMap<String, Vec<u8>>,
        head: &Head,
        cancel: &CancellationToken,
    ) -> Result<Option<(bool, String)>> {
        let Some(marker) = metadata.get(crate::rebase_plan::MESSAGE_FILE) else {
            return Ok(None);
        };
        let Some(plan) = metadata.get(crate::rebase_plan::JOURNAL_FILE) else {
            return Ok(None);
        };
        let plan = crate::rebase_plan::Plan::decode(plan)?;
        let mut lines = marker.split(|b| *b == b'\n');
        let original = parse::oid(lines.next().unwrap_or_default(), "rebase pending message")?;
        let before = parse::oid(lines.next().unwrap_or_default(), "rebase pending message")?;
        if lines.any(|line| !line.is_empty()) {
            return refused(RebaseFailure::UnsupportedSequence);
        }
        if metadata
            .get("stopped-sha")
            .is_none_or(|id| id.trim_ascii() != original.as_str().as_bytes())
        {
            return Ok(None);
        }
        let Head::Detached { target: current } = head else {
            return refused(RebaseFailure::UnsupportedSequence);
        };
        let Some(message) =
            plan.message_for_done(metadata.get("done").map(Vec::as_slice).unwrap_or_default())?
        else {
            return Ok(None);
        };
        if *current == before {
            return Ok(Some((true, message.into())));
        }
        // Cancellation in a commit hook may occur after the reviewed amendment.
        // Accept it only if the same tree/parents/author and exact cleaned message
        // prove the amendment already happened; do not amend it a second time.
        let ids = vec![before, current.clone()];
        let input = ids
            .iter()
            .map(|id| format!("{id}\n"))
            .collect::<String>()
            .into_bytes();
        let bytes = self
            .runner
            .run(self.cwd(), &args(&["cat-file", "--batch"]), input, cancel)?
            .success("rebase message recovery")?;
        let commits = parse::commits(&bytes, &ids)?;
        let cleaned = self
            .runner
            .run(
                self.cwd(),
                &args(&["stripspace"]),
                message.as_bytes().to_vec(),
                cancel,
            )?
            .success("rebase message recovery")?;
        if commits[0].tree == commits[1].tree
            && commits[0].parents == commits[1].parents
            && commits[0].author == commits[1].author
            && commits[1].message == cleaned
        {
            Ok(Some((false, message.into())))
        } else {
            refused(RebaseFailure::StalePreview)
        }
    }
    fn rebase_ancestor(
        &self,
        a: &ObjectId,
        b: &ObjectId,
        cancel: &CancellationToken,
    ) -> Result<bool> {
        let result = self.runner.run(
            self.cwd(),
            &args(&["merge-base", "--is-ancestor", a.as_str(), b.as_str()]),
            Vec::new(),
            cancel,
        )?;
        match result.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => result.success("rebase ancestry").map(|_| false),
        }
    }
    fn protect_rebase_files(&self, paths: &[Vec<u8>], cancel: &CancellationToken) -> Result<()> {
        self.protect_local_files(paths, cancel)
            .map_err(|e| match e {
                Error::CommitAction(CommitFailure::LocalFileCollision) => {
                    Error::Rebase(RebaseFailure::LocalFileCollision)
                }
                other => other,
            })
    }
    fn verify_rebase_reference(
        &self,
        original: &ObjectId,
        create: bool,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let name = reference(original);
        let output = self.runner.run(
            self.cwd(),
            &args(&["show-ref", "--exists", "--", &name]),
            Vec::new(),
            cancel,
        )?;
        match output.status.code() {
            Some(0) => {
                let bytes =
                    self.rebase_read(&["show-ref", "--verify", "--hash", "--", &name], cancel)?;
                if parse::oid(parse::line(&bytes), "rebase recovery")? != *original {
                    return refused(RebaseFailure::RecoveryReferenceConflict);
                }
            }
            Some(2) if create => {
                self.runner
                    .write(
                        self.cwd(),
                        &args(&[
                            "update-ref",
                            &name,
                            original.as_str(),
                            &"0".repeat(original.as_str().len()),
                        ]),
                        Vec::new(),
                        cancel,
                    )?
                    .success("rebase recovery reference")?;
            }
            Some(2) => return refused(RebaseFailure::RecoveryReferenceConflict),
            _ => {
                output.success("rebase recovery reference")?;
            }
        }
        Ok(())
    }
    fn rebase_context(&self, action: RebaseAction, cancel: &CancellationToken) -> Result<Context> {
        let state = self.operation_state(cancel)?;
        let metadata = if action.is_start() {
            if !state.is_idle() {
                return refused(RebaseFailure::ActiveOperation);
            }
            BTreeMap::new()
        } else {
            if !state.rebase
                || (!state.merge_heads.is_empty() && !self.info().git_dir.join(crate::rebase_merges::TODO_FILE).is_file())
                || state.revert
                || state.sequencer
                || state.merge_autostash.is_some()
                || self.info().git_dir.join("rebase-apply").exists()
            {
                return refused(RebaseFailure::UnsupportedSequence);
            }
            let mut metadata = metadata(&self.info().git_dir.join("rebase-merge"), cancel)?;
            let journal = self.info().git_dir.join(crate::rebase_plan::JOURNAL_FILE);
            if journal
                .try_exists()
                .map_err(|e| Error::io("rebase journal", e))?
            {
                let bytes = crate::rebase_plan::read(&journal)?;
                // Check the reviewed bytes before decoding: an altered journal
                // is a journal mismatch even when its replacement is malformed.
                if metadata
                    .get(crate::rebase_plan::PLAN_FILE)
                    .is_some_and(|saved| *saved != bytes)
                {
                    return refused(RebaseFailure::PlanJournal);
                }
                if action != RebaseAction::Abort
                    && !metadata.contains_key(crate::rebase_plan::PLAN_FILE)
                {
                    return refused(RebaseFailure::PlanJournal);
                }
                let saved = crate::rebase_plan::Plan::decode(&bytes)?;
                if saved.preserve_merges {
                    let path=self.info().git_dir.join(crate::rebase_merges::TODO_FILE);
                    if path.try_exists().map_err(|e|Error::io("rebase topology journal",e))? {
                        metadata.insert(crate::rebase_merges::TODO_FILE.into(),crate::rebase_plan::read(&path)?);
                    } else if action != RebaseAction::Abort { return refused(RebaseFailure::PlanJournal); }
                }
                metadata.insert(crate::rebase_plan::JOURNAL_FILE.into(), bytes);
            } else if metadata.contains_key(crate::rebase_plan::PLAN_FILE) {
                return refused(RebaseFailure::PlanJournal);
            }
            metadata
        };
        let status = self.status(cancel)?;
        let tracked = status
            .entries
            .iter()
            .filter(|e| !matches!(e.kind, EntryKind::Untracked | EntryKind::Ignored))
            .collect::<Vec<_>>();
        if action.is_start() && !tracked.is_empty() {
            return refused(RebaseFailure::DirtyWorktree);
        }
        if action == RebaseAction::Continue {
            // Stopped at the first commit of a root rebase, HEAD is still Git's empty
            // base. Git would commit a resolution on top of it and keep that commit.
            if metadata.contains_key("stopped-sha")
                && metadata.get("squash-onto").is_some_and(|base| {
                    matches!(&status.head, Head::Detached { target }
                        if target.as_str().as_bytes() == parse::line(base))
                })
            {
                return refused(RebaseFailure::RootConflict);
            }
            if tracked.iter().any(|e| e.kind == EntryKind::Unmerged) {
                return refused(RebaseFailure::ConflictsRemain);
            }
            if tracked.iter().any(|e| e.worktree != Change::Unchanged) {
                return refused(RebaseFailure::DirtyWorktree);
            }
            let staged = tracked.iter().any(|e| e.index != Change::Unchanged);
            match planned_stop(&metadata, &status.head)? {
                // Git amends staged changes into the stopped commit; a clean index
                // keeps it unchanged. Neither is an empty replay.
                Some(RebaseStop::Edit(_)) => {}
                // Between commits Git refuses staged changes and would ask for a commit.
                Some(RebaseStop::Break | RebaseStop::Command(_)) if staged => {
                    return refused(RebaseFailure::StagedAfterStop);
                }
                Some(_) => {}
                None if metadata.contains_key("stopped-sha")
                    && !staged
                    && self
                        .rebase_message_recovery(&metadata, &status.head, cancel)?
                        .is_none() =>
                {
                    return refused(RebaseFailure::EmptyChange);
                }
                None => {}
            }
        }
        let branch_tip = if action.is_start() {
            None
        } else {
            let name = value(&metadata, "head-name")?;
            if !name.starts_with(b"refs/heads/") {
                return refused(RebaseFailure::UnsupportedSequence);
            }
            let mut arguments = args(&["show-ref", "--verify", "--hash", "--"]);
            arguments.push(model::bytes_to_os(name)?);
            let bytes = self
                .runner
                .run(self.cwd(), &arguments, Vec::new(), cancel)?
                .success("rebase branch")?;
            Some(parse::oid(parse::line(&bytes), "rebase branch")?)
        };
        let files = self.snapshot_changed_files(&status, cancel)?;
        let changes = tracked
            .iter()
            .map(|e| (e.path.display_lossy().into_owned(), e.kind == EntryKind::Unmerged))
            .collect();
        // A clean status alone cannot protect files hidden by these index flags.
        let index = self.rebase_read(&["ls-files", "--stage", "-v", "-z"], cancel)?;
        if index
            .split(|b| *b == 0)
            .filter_map(|entry| entry.first())
            .any(|tag| tag.is_ascii_lowercase() || *tag == b'S')
        {
            return refused(RebaseFailure::UnsupportedIndex);
        }
        Ok(Context {
            head: status.head,
            state,
            index,
            files,
            changes,
            metadata,
            branch_tip,
        })
    }
}

fn reference(id: &ObjectId) -> String {
    format!("refs/rocket/recovery/rebase/{id}")
}
fn paths(bytes: Vec<u8>) -> Vec<Vec<u8>> {
    bytes
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(Vec::from)
        .collect()
}
/// The branches a rebase in progress will move: Git's `update-refs` state lists
/// each as its ref, the commit it pointed at, and the commit it will point at
/// (zeros until the rebase reaches it).
fn update_refs_state(metadata: &BTreeMap<String, Vec<u8>>) -> Result<Vec<DependentBranch>> {
    let Some(bytes) = metadata.get("update-refs") else {
        return Ok(Vec::new());
    };
    let lines: Vec<&[u8]> = bytes
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
        .collect();
    if !lines.len().is_multiple_of(3) || lines.len() > 3 * 200 {
        return refused(RebaseFailure::UnsupportedSequence);
    }
    lines
        .chunks(3)
        .map(|entry| {
            if !crate::rebase_plan::branch_ref(entry[0]) {
                return refused(RebaseFailure::UnsupportedSequence);
            }
            parse::oid(entry[2], "rebase update-refs")?;
            Ok(DependentBranch {
                reference: entry[0].to_vec(),
                target: parse::oid(entry[1], "rebase update-refs")?,
                checked_out_at: None,
            })
        })
        .collect()
}
fn verify_replay_order(metadata: &BTreeMap<String, Vec<u8>>, ids: &[ObjectId]) -> Result<()> {
    // Pick-only is insufficient: an external editor can replace or reorder picks.
    // Git moves consumed lines from todo to done, so their concatenation must
    // still describe exactly the original linear range, including skipped picks.
    let mut expected = ids.iter();
    for name in ["done", "git-rebase-todo"] {
        let Some(bytes) = metadata.get(name) else {
            continue;
        };
        for line in bytes.split(|b| *b == b'\n').map(<[u8]>::trim_ascii) {
            // Branch updates (`--update-refs`) sit between the picks.
            if line.is_empty()
                || line.starts_with(b"#")
                || line.starts_with(b"update-ref ")
                || line.starts_with(b"u ")
            {
                continue;
            }
            let abbreviated = line
                .split(|b| b.is_ascii_whitespace())
                .filter(|s| !s.is_empty())
                .nth(1)
                .ok_or(Error::Rebase(RebaseFailure::UnsupportedSequence))?;
            if expected.next().is_none_or(|id| {
                !id.as_str().as_bytes().starts_with(abbreviated)
                    || ids
                        .iter()
                        .filter(|candidate| candidate.as_str().as_bytes().starts_with(abbreviated))
                        .count()
                        != 1
            }) {
                return refused(RebaseFailure::UnsupportedSequence);
            }
        }
    }
    if expected.next().is_some() {
        return refused(RebaseFailure::UnsupportedSequence);
    }
    Ok(())
}
fn value<'a>(m: &'a BTreeMap<String, Vec<u8>>, name: &str) -> Result<&'a [u8]> {
    m.get(name)
        .map(|v| parse::line(v))
        .ok_or(Error::Rebase(RebaseFailure::UnsupportedSequence))
}
fn regular(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    !meta.file_type().is_symlink()
}
fn metadata(directory: &Path, cancel: &CancellationToken) -> Result<BTreeMap<String, Vec<u8>>> {
    let info = fs::symlink_metadata(directory).map_err(|e| Error::io("rebase metadata", e))?;
    if !info.is_dir() || !regular(&info) {
        return refused(RebaseFailure::UnsupportedSequence);
    }
    let mut out = BTreeMap::new();
    let mut remaining = 2 * 1024 * 1024;
    for entry in fs::read_dir(directory).map_err(|e| Error::io("rebase metadata", e))? {
        cancel.check()?;
        let entry = entry.map_err(|e| Error::io("rebase metadata", e))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| Error::Rebase(RebaseFailure::UnsupportedSequence))?;
        let info =
            fs::symlink_metadata(entry.path()).map_err(|e| Error::io("rebase metadata", e))?;
        if !info.is_file() || !regular(&info) || out.len() >= 64 {
            return refused(RebaseFailure::UnsupportedSequence);
        }
        let mut bytes = Vec::new();
        fs::File::open(entry.path())
            .map_err(|e| Error::io("rebase metadata", e))?
            .take(remaining as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| Error::io("rebase metadata", e))?;
        if bytes.len() > remaining {
            return refused(RebaseFailure::PreviewTooLarge);
        }
        remaining -= bytes.len();
        out.insert(name, bytes);
    }
    // squash-onto names the base of a root rebase; the preview checks it against
    // the reviewed plan.
    for name in ["autostash", "strategy_opts"] {
        if out.get(name).is_some_and(|v| !v.is_empty()) {
            return refused(RebaseFailure::UnsupportedSequence);
        }
    }
    // `--update-refs` state: only well-formed local branch entries.
    update_refs_state(&out)?;
    // Git marks rescheduling with an empty file; Rocket always disables it, so
    // its presence means another client changed how planned commands retry.
    if out.contains_key("reschedule-failed-exec") {
        return refused(RebaseFailure::UnsupportedSequence);
    }
    if out
        .get("strategy")
        .is_some_and(|v| parse::line(v) != b"ort")
    {
        return refused(RebaseFailure::UnsupportedSequence);
    }
    let interactive = out.contains_key(crate::rebase_plan::PLAN_FILE);
    let journal = directory.parent().map(|p|p.join(crate::rebase_plan::JOURNAL_FILE));
    let initial = if !interactive && let Some(path)=journal && path.try_exists().map_err(|e|Error::io("rebase journal",e))? {Some(crate::rebase_plan::read(&path)?)} else {None};
    let preserves = out.get(crate::rebase_plan::PLAN_FILE).or(initial.as_ref()).map(|b|crate::rebase_plan::Plan::decode(b)).transpose()?.is_some_and(|p|p.preserve_merges);
    for name in ["git-rebase-todo", "done"] {
        if let Some(bytes) = out.get(name) {
            if preserves { continue; } // Exact normalized topology is checked against its journal.
            // Commands and their exact text are verified against the reviewed plan
            // separately; an ordinary rebase permits only picks.
            for line in crate::rebase_plan::todo_lines(bytes)? {
                let permitted = match line {
                    crate::rebase_plan::TodoLine::Commit(RebaseInstruction::Pick, _)
                    | crate::rebase_plan::TodoLine::UpdateRef(_) => true,
                    _ => interactive,
                };
                if !permitted {
                    return refused(RebaseFailure::UnsupportedSequence);
                }
            }
        }
    }
    Ok(out)
}

/// Classify the current stop of a reviewed interactive plan from Git's state:
/// done ends with a break or command, or with an edit whose replay Git paused
/// for amending (`amend` names the current HEAD). Conflict stops are not planned.
fn planned_stop(metadata: &BTreeMap<String, Vec<u8>>, head: &Head) -> Result<Option<RebaseStop>> {
    use crate::rebase_plan::TodoLine;
    if !metadata.contains_key(crate::rebase_plan::PLAN_FILE) {
        return Ok(None);
    }
    if crate::rebase_plan::Plan::decode(&metadata[crate::rebase_plan::PLAN_FILE])?.preserve_merges { return Ok(None); }
    let done = crate::rebase_plan::todo_lines(metadata.get("done").map_or(&[][..], Vec::as_slice))?;
    Ok(match done.last() {
        Some(TodoLine::Break) => Some(RebaseStop::Break),
        Some(TodoLine::Exec(command)) => Some(RebaseStop::Command(
            String::from_utf8_lossy(command).into_owned(),
        )),
        Some(TodoLine::Commit(RebaseInstruction::Edit, id)) => {
            let head = match head {
                Head::Branch { target, .. } | Head::Detached { target } => Some(target),
                Head::Unborn { .. } => None,
            };
            let paused = head.is_some_and(|head| {
                metadata.get("amend").map(|a| a.trim_ascii()) == Some(head.as_str().as_bytes())
            });
            match metadata
                .get(crate::rebase_plan::JOURNAL_FILE)
                .or_else(|| metadata.get(crate::rebase_plan::PLAN_FILE))
                .map(|b| crate::rebase_plan::Plan::decode(b))
                .transpose()?
                .and_then(|plan| plan.commit_for(id).cloned())
            {
                Some(commit) if paused => Some(RebaseStop::Edit(commit)),
                _ => None,
            }
        }
        _ => None,
    })
}
fn classify(write: &WriteReport, state: &Result<OperationState>) -> Option<RebaseFailure> {
    if write.result.is_ok() {
        return None;
    }
    if let Err(Error::Git { stderr, .. }) = &write.result {
        let text = String::from_utf8_lossy(stderr).to_ascii_lowercase();
        if text.contains("index.lock") {
            return Some(RebaseFailure::IndexLocked);
        }
        if text.contains("unable to auto-detect email") || text.contains("identity unknown") {
            return Some(RebaseFailure::IdentityMissing);
        }
        if text.contains("failed to sign") || text.contains("gpg failed") {
            return Some(RebaseFailure::SigningFailed);
        }
        if text.contains("untracked") && text.contains("overwritten") {
            return Some(RebaseFailure::LocalFileCollision);
        }
    }
    if state.as_ref().is_ok_and(|s| s.rebase) {
        if write
            .status
            .as_ref()
            .is_ok_and(|s| s.entries.iter().any(|e| e.kind == EntryKind::Unmerged))
        {
            return Some(RebaseFailure::ConflictsRemain);
        }
        if let Err(Error::Git { stderr, .. }) = &write.result {
            let text = String::from_utf8_lossy(stderr).to_ascii_lowercase();
            if text.contains("empty") {
                return Some(RebaseFailure::EmptyChange);
            }
        }
        return Some(RebaseFailure::Paused);
    }
    Some(RebaseFailure::CommandFailed)
}
