//! Reviewed commit operations: amend, cherry-pick/revert of one commit or of a
//! reviewed sequence, and Continue/Skip/Abort. Previews are repository-bound,
//! read-only observations; execution rechecks them under Rocket's shared write gate.
use crate::{parse, runner::args, sequence, write::validate_commit_message, *};
use std::{collections::HashSet, fs, io::Read, path::PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitOperation {
    Amend,
    CherryPick,
    Revert,
}
impl CommitOperation {
    pub fn label(self) -> &'static str {
        match self {
            Self::Amend => "amend",
            Self::CherryPick => "cherry-pick",
            Self::Revert => "revert",
        }
    }
    fn marker(self) -> &'static str {
        match self {
            Self::CherryPick => "CHERRY_PICK_HEAD",
            Self::Revert => "REVERT_HEAD",
            Self::Amend => "HEAD",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitRecovery {
    Continue,
    Skip,
    Abort,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitActionRequest {
    pub operation: CommitOperation,
    /// Full object IDs in application order, never revision ranges. One commit
    /// may be a merge with an explicit mainline; 2..=100 form a reviewed sequence.
    /// Amend and recovery requests name none: they act on HEAD or the stopped work.
    pub targets: Vec<ObjectId>,
    /// Explicit one-based parent for replaying/reverting a single merge commit.
    pub mainline: Option<usize>,
    pub recovery: Option<CommitRecovery>,
}
impl CommitActionRequest {
    pub fn label(&self) -> &'static str {
        match (self.operation, self.recovery) {
            (CommitOperation::Amend, _) => "amend",
            (CommitOperation::CherryPick, None) => "cherry-pick",
            (CommitOperation::Revert, None) => "revert",
            (CommitOperation::CherryPick, Some(CommitRecovery::Continue)) => "cherry-pick-continue",
            (CommitOperation::CherryPick, Some(CommitRecovery::Skip)) => "cherry-pick-skip",
            (CommitOperation::CherryPick, Some(CommitRecovery::Abort)) => "cherry-pick-abort",
            (CommitOperation::Revert, Some(CommitRecovery::Continue)) => "revert-continue",
            (CommitOperation::Revert, Some(CommitRecovery::Skip)) => "revert-skip",
            (CommitOperation::Revert, Some(CommitRecovery::Abort)) => "revert-abort",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitFailure {
    StalePreview,
    RecoveryReferenceConflict,
    WrongRepository,
    MissingHead,
    ActiveOperation,
    DirtyWorktree,
    ConflictsRemain,
    EmptyChange,
    MainlineRequired,
    InvalidMainline,
    UnsupportedSequence,
    SequenceMerge,
    SequenceJournal,
    NothingToSkip,
    LocalFileCollision,
    IndexLocked,
    IdentityMissing,
    SigningFailed,
    CommandFailed,
}
impl CommitFailure {
    pub fn code(self) -> &'static str {
        match self {
            Self::StalePreview => "stale_preview",
            Self::RecoveryReferenceConflict => "recovery_ref_conflict",
            Self::WrongRepository => "wrong_repository",
            Self::MissingHead => "missing_head",
            Self::ActiveOperation => "active_operation",
            Self::DirtyWorktree => "dirty_worktree",
            Self::ConflictsRemain => "conflicts_remain",
            Self::EmptyChange => "empty_change",
            Self::MainlineRequired => "mainline_required",
            Self::InvalidMainline => "invalid_mainline",
            Self::UnsupportedSequence => "unsupported_sequence",
            Self::SequenceMerge => "sequence_merge",
            Self::SequenceJournal => "sequence_review_changed",
            Self::NothingToSkip => "nothing_to_skip",
            Self::LocalFileCollision => "local_file_collision",
            Self::IndexLocked => "index_locked",
            Self::IdentityMissing => "identity_missing",
            Self::SigningFailed => "signing_failed",
            Self::CommandFailed => "command_failed",
        }
    }
    pub fn guidance(self) -> &'static str {
        match self {
            Self::StalePreview => {
                "Something changed since this review, so nothing ran. Review it again."
            }
            Self::RecoveryReferenceConflict => {
                "The amend recovery reference already points to a different commit. Inspect and back up that reference with Git before retrying. Rocket has not replaced it or amended HEAD."
            }
            Self::WrongRepository => {
                "This review belongs to another repository or worktree. Review it again here."
            }
            Self::MissingHead => "Create the first commit before using this operation.",
            Self::ActiveOperation => {
                "Finish or abort the active Git operation before starting another one."
            }
            Self::DirtyWorktree => {
                "Commit or stash your changes first. They were left in place."
            }
            Self::ConflictsRemain => {
                "Fix the conflicts in Changes and stage them, then Continue."
            }
            Self::EmptyChange => {
                "Nothing is staged, so there's nothing to save. Stage a fix, or Abort. Rocket never makes or drops an empty commit by itself."
            }
            Self::MainlineRequired => {
                "This is a merge. Choose which side stays, then review again."
            }
            Self::InvalidMainline => {
                "Choose one of this merge's sides. Other commits don't have sides to choose."
            }
            Self::UnsupportedSequence => {
                "This is not a cherry-pick or revert that Rocket reviewed. Finish the external rebase or sequence with Git before continuing here."
            }
            Self::SequenceMerge => {
                "A merge can't be part of a range. Do the merge on its own, then the other commits."
            }
            Self::SequenceJournal => {
                "Git's sequence no longer matches the commits Rocket reviewed (or that review is missing). Inspect the sequencer state with Git and finish it there; Rocket will not guess the remaining commits."
            }
            Self::NothingToSkip => {
                "No commit is stopped right now. Continue does the rest, or Abort goes back to the start."
            }
            Self::LocalFileCollision => {
                "An untracked or ignored file overlaps this commit's paths. Move or back up that file before retrying; Rocket has not overwritten it."
            }
            Self::IndexLocked => {
                "Git could not acquire its index lock. Let other Git work finish, then refresh. Rocket leaves existing lock files in place."
            }
            Self::IdentityMissing => {
                "Configure the repository's name and email, then review the current operation before retrying or continuing."
            }
            Self::SigningFailed => {
                "Git could not sign the commit. Check your configured signing key or agent, then review the current operation before retrying or continuing."
            }
            Self::CommandFailed => {
                "Git did not confirm completion. Check the refreshed history and operation state, and the optional local Git output for hook or configuration errors. Do not blindly retry."
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum WorkingFile {
    Missing,
    Link(PathBuf),
    Bytes(Vec<u8>),
}
/// A journaled sequence in progress: exact review and sequencer bytes for staleness.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SequenceContext {
    journal: Vec<u8>,
    sequencer: Vec<u8>,
    safety: Vec<u8>,
    original: ObjectId,
    commits: Vec<ObjectId>,
    remaining: Vec<ObjectId>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct CommitContext {
    head: Head,
    state: OperationState,
    operation_target: Option<ObjectId>,
    operation_message: Option<Vec<u8>>,
    index: Vec<u8>,
    files: Vec<(RepoPath, WorkingFile)>,
    sequence: Option<SequenceContext>,
}
#[derive(Clone, Debug)]
pub struct CommitPreview {
    git_dir: PathBuf,
    request: CommitActionRequest,
    context: CommitContext,
    source: Commit,
    sources: Vec<Commit>,
    applied: Vec<Commit>,
    patch: Vec<u8>,
    patches: Vec<Vec<u8>>,
    parents: Vec<Commit>,
    reviewed: Vec<Commit>,
    original: Option<Commit>,
    affected: Vec<Vec<u8>>,
    prediction: Option<CommitPrediction>,
    prediction_guard: Option<crate::commit_prediction::PredictionGuard>,
}
impl CommitPreview {
    pub fn request(&self) -> &CommitActionRequest {
        &self.request
    }
    /// The commit being applied, amended or recovered; the first of a sequence.
    pub fn source(&self) -> &Commit {
        &self.source
    }
    /// Every reviewed commit, in application order.
    pub fn commits(&self) -> &[Commit] {
        &self.sources
    }
    /// Starting or recovering a multi-commit sequence.
    pub fn is_sequence(&self) -> bool {
        self.sources.len() > 1 || self.context.sequence.is_some()
    }
    /// Reviewed commits Git has not finished, including a stopped one.
    pub fn remaining(&self) -> &[ObjectId] {
        self.context
            .sequence
            .as_ref()
            .map_or(&[], |s| s.remaining.as_slice())
    }
    /// Commits the sequence has created so far, newest first (recovery only).
    pub fn applied(&self) -> &[Commit] {
        &self.applied
    }
    pub fn head(&self) -> &Head {
        &self.context.head
    }
    /// Display text: a patch, or each sequence commit's changed paths.
    pub fn patch(&self) -> &[u8] {
        &self.patch
    }
    /// Starting a sequence: each commit's change, in `commits()` order, or nothing if
    /// they were too big to read. Display only.
    pub fn patches(&self) -> &[Vec<u8>] {
        &self.patches
    }
    /// Starting from a merge: its parents, in order.
    pub fn parent_commits(&self) -> &[Commit] {
        &self.parents
    }
    /// Recovering a sequence: every reviewed commit, in application order.
    pub fn reviewed(&self) -> &[Commit] {
        &self.reviewed
    }
    /// Recovering a sequence: the commit Git has reached, from one, and how many there are.
    pub fn position(&self) -> (usize, usize) {
        let total = self.reviewed.len();
        (total.saturating_sub(self.remaining().len()) + 1, total)
    }
    /// The commit Git stopped at. A sequence between two commits has none.
    pub fn stopped_commit(&self) -> Option<&Commit> {
        self.context.operation_target.as_ref().map(|_| &self.source)
    }
    /// Abort: the commit the branch ends up on.
    pub fn original_commit(&self) -> Option<&Commit> {
        self.original.as_ref()
    }
    /// Abort of a sequence moves the branch back. Git leaves it alone if it moved since
    /// Git's last commit, as when someone committed a fix by hand.
    pub fn rewinds(&self) -> bool {
        self.context.sequence.as_ref().is_some_and(rewinds(&self.context.head))
    }
    /// Content prediction for a new cherry-pick/revert; amend and recovery have none.
    pub fn prediction(&self) -> Option<&CommitPrediction> {
        self.prediction.as_ref()
    }
    pub fn recovery_reference(&self) -> Option<String> {
        self.recovery_target().map(|(reference, _)| reference)
    }
    /// Amend keeps the original commit; aborting a sequence that already created
    /// commits keeps the current tip, since abort returns to the starting commit.
    fn recovery_target(&self) -> Option<(String, ObjectId)> {
        if self.request.operation == CommitOperation::Amend {
            return Some((
                format!("refs/rocket/recovery/amend/{}", self.source.id),
                self.source.id.clone(),
            ));
        }
        let head = head_id(&self.context.head).ok()?;
        (self.request.recovery == Some(CommitRecovery::Abort) && !self.applied.is_empty()).then(
            || {
                (
                    format!("refs/rocket/recovery/sequence/{head}"),
                    head.clone(),
                )
            },
        )
    }
}
#[derive(Debug)]
#[must_use = "inspect the command result and independent refreshed state before retrying"]
pub struct CommitWriteReport {
    pub write: WriteReport,
    pub state: Result<OperationState>,
    pub branches: Result<Vec<Branch>>,
    pub failure: Option<CommitFailure>,
    pub recovery_reference: Option<String>,
    /// Removal of the sequence review once Git has finished; Ok when there is none.
    pub journal_cleanup: Result<()>,
}

/// A multi-commit cherry-pick/revert that Rocket reviewed and Git is still applying.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitSequence {
    pub operation: CommitOperation,
    /// Every reviewed commit, in application order.
    pub commits: Vec<ObjectId>,
    /// Reviewed commits Git has not finished, including a stopped one; None when
    /// Git's sequencer state no longer matches the review.
    pub remaining: Option<Vec<ObjectId>>,
}

/// Whether Git's abort of `sequence` will rewind a branch that's at `head`.
fn rewinds(head: &Head) -> impl Fn(&SequenceContext) -> bool {
    let id = head_id(head).ok().cloned();
    move |sequence| id.as_ref().is_some_and(|id| id.as_str().as_bytes() == sequence.safety)
}

/// `diff-tree --stdin` output cut at each commit's ID. A commit with no changes prints
/// nothing, and a patch line starts with a sign or a keyword, so it can't read as an ID.
fn commit_patches(output: &[u8], ids: &[ObjectId]) -> Vec<Vec<u8>> {
    let mut patches = vec![Vec::new(); ids.len()];
    let mut current = None;
    for line in output.split_inclusive(|b| *b == b'\n') {
        match ids.iter().position(|id| id.as_str().as_bytes() == line.trim_ascii_end()) {
            Some(i) => current = Some(i),
            None => {
                if let Some(i) = current {
                    patches[i].extend_from_slice(line);
                }
            }
        }
    }
    patches
}

impl Repository {
    /// The reviewed sequence in this worktree, if Rocket started one. Read-only;
    /// recovery previews revalidate it against Git's sequencer state again.
    pub fn commit_sequence(&self, cancel: &CancellationToken) -> Result<Option<CommitSequence>> {
        self.runner.step("commit-preview", "read_sequence", || {
            self.require_worktree(cancel)?;
            let git_dir = &self.info().git_dir;
            let Some((journal, _)) = sequence::read_journal(git_dir)? else {
                return Ok(None);
            };
            // A sequencer Rocket cannot read is reported as not matching, like a
            // sequence started or edited outside Rocket.
            let remaining = match sequence::read_sequencer(git_dir) {
                Ok(sequencer) => sequencer.and_then(|s| sequence::remaining(&journal, &s)),
                Err(Error::CommitAction(_)) => None,
                Err(error) => return Err(error),
            };
            Ok(Some(CommitSequence {
                operation: journal.operation,
                commits: journal.commits,
                remaining,
            }))
        })
    }

    pub fn preview_commit_action(
        &self,
        request: CommitActionRequest,
        cancel: &CancellationToken,
    ) -> Result<CommitPreview> {
        self.runner.step("commit-preview", "validate", || {
            self.require_worktree(cancel)?;
            let starting =
                request.recovery.is_none() && request.operation != CommitOperation::Amend;
            if starting {
                let unique: HashSet<_> = request.targets.iter().collect();
                if request.targets.is_empty() || request.targets.len() > sequence::MAX_SEQUENCE {
                    return Err(Error::InvalidInput("Select 1 to 100 commits.".into()));
                }
                if unique.len() != request.targets.len() {
                    return Err(Error::InvalidInput("Select each commit only once.".into()));
                }
                if request.targets.len() > 1 && request.mainline.is_some() {
                    return Err(Error::CommitAction(CommitFailure::InvalidMainline));
                }
            } else if !request.targets.is_empty() {
                return Err(Error::InvalidInput(
                    "Amend and Continue/Skip/Abort act on HEAD or the stopped operation; select no commits."
                        .into(),
                ));
            }
            let context = self.commit_context(&request, cancel)?;
            if starting && request.targets.len() > 1 {
                return self.preview_sequence(request, context, cancel);
            }
            let id = if request.recovery.is_some() {
                // After a manual commit, a sequence has no stopped commit to show.
                match &context.operation_target {
                    Some(id) => id.clone(),
                    None => head_id(&context.head)?.clone(),
                }
            } else if request.operation == CommitOperation::Amend {
                head_id(&context.head)?.clone()
            } else {
                request.targets[0].clone()
            };
            let objects = self
                .runner
                .run(
                    self.cwd(),
                    &args(&["cat-file", "--batch"]),
                    format!("{id}\n").into_bytes(),
                    cancel,
                )?
                .success("commit preview")?;
            let source = parse::commits(&objects, &[id])?.remove(0);
            if request.recovery.is_none() && request.operation != CommitOperation::Amend {
                match (source.parents.len(), request.mainline) {
                    (2.., None) => {
                        return Err(Error::CommitAction(CommitFailure::MainlineRequired));
                    }
                    (n, Some(parent)) if n < 2 || parent == 0 || parent > n => {
                        return Err(Error::CommitAction(CommitFailure::InvalidMainline));
                    }
                    _ => {}
                }
            }
            let mut arguments = args(&[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "--no-relative",
                "--no-renames",
                "--binary",
                "--full-index",
                "--ignore-submodules=none",
            ]);
            let mut names = args(&[
                "diff-tree",
                "--no-commit-id",
                "--name-only",
                "--no-renames",
                "-r",
                "-z",
            ]);
            if request.recovery.is_some() {
                if request.recovery == Some(CommitRecovery::Continue) {
                    arguments.push("--cached".into());
                }
                arguments.push(head_id(&context.head)?.as_str().into());
            } else if request.operation == CommitOperation::Amend {
                arguments.extend(args(&["--cached", head_id(&context.head)?.as_str()]));
            } else if let Some(parent) = source.parents.get(request.mainline.unwrap_or(1) - 1) {
                arguments.extend(args(&[parent.as_str(), source.id.as_str()]));
                names.extend(args(&[parent.as_str(), source.id.as_str()]));
            } else {
                arguments[0] = "show".into();
                arguments.extend(args(&[
                    "--format=",
                    "--root",
                    "--no-show-signature",
                    "--no-notes",
                    "--no-decorate",
                    source.id.as_str(),
                ]));
                names.extend(args(&["--root", source.id.as_str()]));
            }
            if request.operation == CommitOperation::Revert && request.recovery.is_none() {
                arguments.push("-R".into());
            }
            arguments.push("--".into());
            let patch = self
                .runner
                .run(self.cwd(), &arguments, Vec::new(), cancel)?
                .success("commit preview")?;
            let affected = if matches!(
                request.recovery,
                Some(CommitRecovery::Abort | CommitRecovery::Skip)
            ) {
                // A rollback can restore a path deleted from the index by the
                // failed pick. Protect files created there after the failure too.
                // Aborting a sequence also returns to its starting commit.
                let mut trees = vec![head_id(&context.head)?.clone()];
                if request.recovery == Some(CommitRecovery::Abort)
                    && let Some(s) = &context.sequence
                {
                    trees.push(s.original.clone());
                }
                let mut paths = Vec::new();
                for tree in &trees {
                    let bytes = self
                        .action_read(&["ls-tree", "-r", "--name-only", "-z", tree.as_str()], cancel)?;
                    paths.extend(
                        bytes
                            .split(|b| *b == 0)
                            .filter(|p| !p.is_empty())
                            .map(Vec::from),
                    );
                }
                paths.sort();
                paths.dedup();
                self.protect_local_files(&paths, cancel)?;
                paths
            } else if request.recovery.is_none() && request.operation != CommitOperation::Amend {
                names.push("--".into());
                let bytes = self
                    .runner
                    .run(self.cwd(), &names, Vec::new(), cancel)?
                    .success("commit paths")?;
                let paths = bytes
                    .split(|b| *b == 0)
                    .filter(|p| !p.is_empty())
                    .map(Vec::from)
                    .collect::<Vec<_>>();
                self.protect_local_files(&paths, cancel)?;
                paths
            } else {
                Vec::new()
            };
            let applied = match &context.sequence {
                Some(s) => self.commits_since(&s.original, cancel)?,
                None => Vec::new(),
            };
            let parents = if starting && source.parents.len() > 1 {
                self.read_commits(&source.parents, cancel)?
            } else {
                Vec::new()
            };
            let reviewed = match &context.sequence {
                Some(s) => self.read_commits(&s.commits, cancel)?,
                None => Vec::new(),
            };
            let original = if request.recovery == Some(CommitRecovery::Abort) {
                let id = match context.sequence.as_ref().filter(|s| rewinds(&context.head)(s)) {
                    Some(s) => s.original.clone(),
                    None => head_id(&context.head)?.clone(),
                };
                self.read_commits(&[id], cancel)?.pop()
            } else {
                None
            };
            let (prediction, prediction_guard) = if starting {
                let (prediction, guard) = self.predict_commits(&request, head_id(&context.head)?, std::slice::from_ref(&source), &affected, cancel)?;
                (Some(prediction), guard)
            } else { (None, None) };
            if context != self.commit_context(&request, cancel)? {
                return Err(Error::CommitAction(CommitFailure::StalePreview));
            }
            Ok(CommitPreview {
                git_dir: self.info().git_dir.clone(),
                request,
                context,
                sources: vec![source.clone()],
                source,
                applied,
                patch,
                patches: Vec::new(),
                parents,
                reviewed,
                original,
                affected,
                prediction,
                prediction_guard,
            })
        })
    }

    /// 2..=100 non-merge commits applied in the reviewed order by one Git sequence.
    fn preview_sequence(
        &self,
        request: CommitActionRequest,
        context: CommitContext,
        cancel: &CancellationToken,
    ) -> Result<CommitPreview> {
        let input = request
            .targets
            .iter()
            .map(|id| format!("{id}\n"))
            .collect::<String>()
            .into_bytes();
        let objects = self
            .runner
            .run(
                self.cwd(),
                &args(&["cat-file", "--batch"]),
                input.clone(),
                cancel,
            )?
            .success("commit preview")?;
        let sources = parse::commits(&objects, &request.targets)?;
        if sources.iter().any(|c| c.parents.len() > 1) {
            return Err(Error::CommitAction(CommitFailure::SequenceMerge));
        }
        // Display only: each commit ID followed by the paths it changes. Paths for
        // protection come from the separate NUL-delimited listing below.
        let patch = self
            .runner
            .run(
                self.cwd(),
                &args(&[
                    "-c",
                    "core.quotepath=false",
                    "diff-tree",
                    "--stdin",
                    "--root",
                    "-r",
                    "--no-renames",
                    "--name-status",
                    "--no-ext-diff",
                    "--no-textconv",
                ]),
                input.clone(),
                cancel,
            )?
            .success("commit preview")?;
        // Display only, like the listing above, so a change too big to read refuses nothing.
        let mut arguments = args(&[
            "diff-tree",
            "--stdin",
            "--root",
            "-r",
            "--no-renames",
            "--patch",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
        ]);
        if request.operation == CommitOperation::Revert {
            arguments.push("-R".into());
        }
        let patches = self
            .runner
            .run(self.cwd(), &arguments, input.clone(), cancel)
            .and_then(|output| output.success("commit preview"));
        if matches!(patches, Err(Error::Cancelled)) {
            return Err(Error::Cancelled);
        }
        let patches = patches.map_or_else(|_| Vec::new(), |bytes| commit_patches(&bytes, &request.targets));
        let names = self
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
            .success("commit paths")?;
        let mut affected = names
            .split(|b| *b == 0)
            .filter(|p| !p.is_empty())
            .map(Vec::from)
            .collect::<Vec<_>>();
        affected.sort();
        affected.dedup();
        self.protect_local_files(&affected, cancel)?;
        let (prediction, prediction_guard) = self.predict_commits(
            &request,
            head_id(&context.head)?,
            &sources,
            &affected,
            cancel,
        )?;
        if context != self.commit_context(&request, cancel)? {
            return Err(Error::CommitAction(CommitFailure::StalePreview));
        }
        Ok(CommitPreview {
            git_dir: self.info().git_dir.clone(),
            request,
            context,
            source: sources[0].clone(),
            sources,
            applied: Vec::new(),
            patch,
            patches,
            parents: Vec::new(),
            reviewed: Vec::new(),
            original: None,
            affected,
            prediction: Some(prediction),
            prediction_guard,
        })
    }

    /// Commits created since a sequence started, newest first.
    fn commits_since(
        &self,
        original: &ObjectId,
        cancel: &CancellationToken,
    ) -> Result<Vec<Commit>> {
        let range = format!("{original}..HEAD");
        let bytes = self.action_read(
            &[
                "rev-list",
                &format!("--max-count={}", sequence::MAX_SEQUENCE + 1),
                &range,
                "--",
            ],
            cancel,
        )?;
        let ids = bytes
            .split(|b| *b == b'\n')
            .filter(|l| !l.is_empty())
            .map(|l| parse::oid(l, "commit preview"))
            .collect::<Result<Vec<_>>>()?;
        self.read_commits(&ids, cancel)
    }

    fn read_commits(&self, ids: &[ObjectId], cancel: &CancellationToken) -> Result<Vec<Commit>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let input = ids
            .iter()
            .map(|id| format!("{id}\n"))
            .collect::<String>()
            .into_bytes();
        let objects = self
            .runner
            .run(self.cwd(), &args(&["cat-file", "--batch"]), input, cancel)?
            .success("commit preview")?;
        parse::commits(&objects, ids)
    }

    pub fn apply_commit_action(
        &self,
        preview: &CommitPreview,
        message: Option<&str>,
        cancel: &CancellationToken,
    ) -> Result<CommitWriteReport> {
        self.require_worktree(cancel)?;
        let _permit = self.acquire_write(cancel)?;
        self.runner
            .step(preview.request.label(), "revalidate_preview", || {
                if self.info().git_dir != preview.git_dir {
                    return Err(Error::CommitAction(CommitFailure::WrongRepository));
                }
                if self.commit_context(&preview.request, cancel)? != preview.context {
                    return Err(Error::CommitAction(CommitFailure::StalePreview));
                }
                self.protect_local_files(&preview.affected, cancel)?;
                if let Some(guard) = &preview.prediction_guard {
                    self.validate_prediction(guard, cancel)?;
                }
                if preview.request.operation == CommitOperation::Amend {
                    validate_commit_message(message.unwrap_or(""))?;
                } else if message.is_some() {
                    return Err(Error::InvalidInput(
                        "Only amend accepts a replacement message.".into(),
                    ));
                }
                Ok(())
            })?;
        let recovery = preview.recovery_target();
        let recovery_reference = recovery.as_ref().map(|(reference, _)| reference.clone());
        if let Some((reference, target)) = &recovery {
            self.runner
                .step(preview.request.label(), "save_recovery_reference", || {
                    // Transactional create/verify: never replace another recovery ref.
                    let existing = self.runner.run(
                        self.cwd(),
                        &args(&["show-ref", "--verify", "--hash", "--", reference]),
                        Vec::new(),
                        cancel,
                    )?;
                    if existing.status.success() {
                        if parse::oid(parse::line(&existing.stdout), "recovery ref")? != *target {
                            return Err(Error::CommitAction(
                                CommitFailure::RecoveryReferenceConflict,
                            ));
                        }
                    } else {
                        let zero = "0".repeat(target.as_str().len());
                        self.runner
                            .write(
                                self.cwd(),
                                &args(&["update-ref", reference, target.as_str(), &zero]),
                                Vec::new(),
                                cancel,
                            )?
                            .success("save recovery ref")?;
                    }
                    Ok(())
                })?;
        }
        let git_dir = &self.info().git_dir;
        // A new sequence records the reviewed commits before Git starts. A review
        // left while idle belongs to a sequence that already ended, so it is stale.
        let journal = if preview.sources.len() > 1 {
            Some(
                self.runner
                    .step(preview.request.label(), "save_sequence_review", || {
                        if let Some((_, stale)) = sequence::read_journal(git_dir)? {
                            sequence::remove_journal(git_dir, &stale)?;
                        }
                        sequence::write_journal(
                            git_dir,
                            &sequence::Journal {
                                operation: preview.request.operation,
                                original: head_id(&preview.context.head)?.clone(),
                                commits: preview.request.targets.clone(),
                            },
                        )
                    })?,
            )
        } else {
            preview.context.sequence.as_ref().map(|s| s.journal.clone())
        };
        let request = &preview.request;
        let mut arguments = args(&["-c", "submodule.recurse=false"]);
        let mut input = Vec::new();
        let operation = if request.operation == CommitOperation::Amend {
            arguments.extend(args(&[
                "commit",
                "--amend",
                "--allow-empty",
                "--file=-",
                "--no-edit",
                "--cleanup=whitespace",
            ]));
            input = message.unwrap().as_bytes().to_vec();
            WriteOperation::Amend
        } else {
            arguments.push(request.operation.label().into());
            if let Some(recovery) = request.recovery {
                arguments.push(
                    match recovery {
                        CommitRecovery::Continue => "--continue",
                        CommitRecovery::Skip => "--skip",
                        CommitRecovery::Abort => "--abort",
                    }
                    .into(),
                );
            } else {
                arguments.extend(args(&[
                    "--no-edit",
                    "--no-rerere-autoupdate",
                    "--strategy=ort",
                ]));
                if request.operation == CommitOperation::CherryPick {
                    arguments.push("--empty=stop".into());
                }
                if let Some(parent) = request.mainline {
                    arguments.extend(args(&["--mainline", &parent.to_string()]));
                }
                // Git applies explicit commits in command-line order.
                arguments.push("--".into());
                arguments.extend(preview.sources.iter().map(|c| c.id.as_str().into()));
            }
            match (request.operation, request.recovery) {
                (CommitOperation::CherryPick, None) => WriteOperation::CherryPick,
                (CommitOperation::Revert, None) => WriteOperation::Revert,
                (CommitOperation::CherryPick, Some(CommitRecovery::Continue)) => {
                    WriteOperation::ContinueCherryPick
                }
                (CommitOperation::CherryPick, Some(CommitRecovery::Skip)) => {
                    WriteOperation::SkipCherryPick
                }
                (CommitOperation::CherryPick, Some(CommitRecovery::Abort)) => {
                    WriteOperation::AbortCherryPick
                }
                (CommitOperation::Revert, Some(CommitRecovery::Continue)) => {
                    WriteOperation::ContinueRevert
                }
                (CommitOperation::Revert, Some(CommitRecovery::Skip)) => WriteOperation::SkipRevert,
                (CommitOperation::Revert, Some(CommitRecovery::Abort)) => {
                    WriteOperation::AbortRevert
                }
                _ => unreachable!(),
            }
        };
        let write = self.execute_write(operation, &arguments, input, cancel)?;
        let state = self.runner.observe("refresh", "operation", || {
            self.operation_state(&CancellationToken::new())
        });
        let branches = self.runner.observe("refresh", "branches", || {
            self.branches(&CancellationToken::new())
        });
        let failure = classify(&write, &state);
        if let Some(log) = &self.runner.log {
            log.commit_action_outcome(request.label(), failure, &state);
        }
        // The review is kept while Git's sequence is in progress, for recovery.
        let journal_cleanup = match (&journal, &state) {
            (Some(bytes), Ok(s)) if !s.sequencer && !s.cherry_pick && !s.revert => self
                .runner
                .step(request.label(), "clean_sequence_review", || {
                    sequence::remove_journal(git_dir, bytes)
                }),
            _ => Ok(()),
        };
        Ok(CommitWriteReport {
            write,
            state,
            branches,
            failure,
            recovery_reference,
            journal_cleanup,
        })
    }

    fn action_read(&self, arguments: &[&str], cancel: &CancellationToken) -> Result<Vec<u8>> {
        self.runner
            .run(self.cwd(), &args(arguments), Vec::new(), cancel)?
            .success("commit preview")
    }
    fn commit_context(
        &self,
        request: &CommitActionRequest,
        cancel: &CancellationToken,
    ) -> Result<CommitContext> {
        let state = self.operation_state(cancel)?;
        // CHERRY_PICK_HEAD/REVERT_HEAD: a stopped commit. A sequence can lack it
        // after the user committed the resolution themselves.
        let stopped = state.cherry_pick || state.revert;
        let mut sequence = None;
        if request.recovery.is_some() {
            let (own, other) = match request.operation {
                CommitOperation::CherryPick => (state.cherry_pick, state.revert),
                CommitOperation::Revert => (state.revert, state.cherry_pick),
                CommitOperation::Amend => (false, true),
            };
            if other
                || state.rebase
                || state.mail_import
                || !state.merge_heads.is_empty()
                || state.merge_autostash.is_some()
                || (!own && !state.sequencer)
            {
                return Err(Error::CommitAction(CommitFailure::UnsupportedSequence));
            }
            if state.sequencer {
                // Only a sequence Rocket reviewed, still matching that review.
                let git_dir = &self.info().git_dir;
                let unsupported = || Error::CommitAction(CommitFailure::UnsupportedSequence);
                let (journal, bytes) = sequence::read_journal(git_dir)?.ok_or_else(unsupported)?;
                let sequencer = sequence::read_sequencer(git_dir)?.ok_or_else(unsupported)?;
                if journal.operation != request.operation {
                    return Err(unsupported());
                }
                let remaining = sequence::remaining(&journal, &sequencer)
                    .ok_or(Error::CommitAction(CommitFailure::SequenceJournal))?;
                if request.recovery == Some(CommitRecovery::Skip) && !own {
                    return Err(Error::CommitAction(CommitFailure::NothingToSkip));
                }
                sequence = Some(SequenceContext {
                    journal: bytes,
                    sequencer: sequencer.raw,
                    safety: sequencer.abort_safety,
                    original: journal.original,
                    commits: journal.commits,
                    remaining,
                });
            }
        } else if !state.is_idle() {
            return Err(Error::CommitAction(CommitFailure::ActiveOperation));
        }
        let status = self.status(cancel)?;
        head_id(&status.head)?;
        let tracked =
            |e: &&StatusEntry| !matches!(e.kind, EntryKind::Untracked | EntryKind::Ignored);
        if request.recovery.is_none()
            && request.operation != CommitOperation::Amend
            && status.entries.iter().any(|e| tracked(&e))
        {
            return Err(Error::CommitAction(CommitFailure::DirtyWorktree));
        }
        if (request.operation == CommitOperation::Amend
            || request.recovery == Some(CommitRecovery::Continue))
            && status.entries.iter().any(|e| e.kind == EntryKind::Unmerged)
        {
            return Err(Error::CommitAction(CommitFailure::ConflictsRemain));
        }
        if request.recovery == Some(CommitRecovery::Continue) {
            let staged = status
                .entries
                .iter()
                .filter(tracked)
                .any(|e| e.index != Change::Unchanged);
            if stopped && !staged {
                return Err(Error::CommitAction(CommitFailure::EmptyChange));
            }
            // With no stopped commit, Git only resumes the remaining commits.
            if !stopped && status.entries.iter().any(|e| tracked(&e)) {
                return Err(Error::CommitAction(CommitFailure::DirtyWorktree));
            }
        }
        let index = self.action_read(&["ls-files", "--stage", "--debug", "-z"], cancel)?;
        let operation_target = if request.recovery.is_some() && stopped {
            Some(parse::oid(
                parse::line(&self.action_read(
                    &[
                        "show-ref",
                        "--verify",
                        "--hash",
                        "--",
                        request.operation.marker(),
                    ],
                    cancel,
                )?),
                "operation target",
            )?)
        } else {
            None
        };
        let operation_message = if request.recovery.is_some() {
            let path = self.info().git_dir.join("MERGE_MSG");
            match fs::symlink_metadata(&path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(Error::io("read operation message", error)),
                Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() && meta.len() <= 1024 * 1024 => {
                    let mut bytes = Vec::new();
                    fs::File::open(path).map_err(|e| Error::io("read operation message", e))?.take(1024 * 1024 + 1).read_to_end(&mut bytes).map_err(|e| Error::io("read operation message", e))?;
                    if bytes.len() > 1024 * 1024 { return Err(Error::CommitAction(CommitFailure::StalePreview)); }
                    Some(bytes)
                }
                _ => return Err(Error::Unsupported("The operation message is not a bounded regular file. Review this operation with Git.".into())),
            }
        } else {
            None
        };
        let files = if request.recovery.is_some() {
            self.snapshot_changed_files(&status, cancel)?
        } else {
            Vec::new()
        };
        Ok(CommitContext {
            head: status.head,
            state,
            operation_target,
            operation_message,
            index,
            files,
            sequence,
        })
    }

    pub(crate) fn snapshot_changed_files(
        &self,
        status: &Status,
        cancel: &CancellationToken,
    ) -> Result<Vec<(RepoPath, WorkingFile)>> {
        let mut files = Vec::new();
        let mut remaining = 16 * 1024 * 1024;
        for entry in status
            .entries
            .iter()
            .filter(|e| !matches!(e.kind, EntryKind::Untracked | EntryKind::Ignored))
        {
            cancel.check()?;
            let path = self
                .info()
                .worktree
                .as_ref()
                .unwrap()
                .join(entry.path.argument()?);
            let file = match fs::symlink_metadata(&path) {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => WorkingFile::Missing,
                    Err(e) => return Err(Error::io("preview working file", e)),
                    Ok(meta) if meta.file_type().is_symlink() => WorkingFile::Link(fs::read_link(&path).map_err(|e| Error::io("preview symlink", e))?),
                    Ok(meta) if meta.is_file() => {
                        let mut bytes = Vec::new();
                        fs::File::open(&path).map_err(|e| Error::io("preview working file", e))?.take(remaining as u64 + 1).read_to_end(&mut bytes).map_err(|e| Error::io("preview working file", e))?;
                        if bytes.len() > remaining { return Err(Error::Unsupported("Recovery preview exceeds 16 MiB of working changes. Review and finish this operation with Git.".into())); }
                        remaining -= bytes.len();
                        WorkingFile::Bytes(bytes)
                    }
                    _ => return Err(Error::Unsupported("Review changed submodules or special files with Git before recovering this operation.".into())),
                };
            files.push((entry.path.clone(), file));
        }
        Ok(files)
    }
    pub(crate) fn protect_local_files(
        &self,
        affected: &[Vec<u8>],
        cancel: &CancellationToken,
    ) -> Result<()> {
        if affected.is_empty() {
            return Ok(());
        }
        for ignored in [false, true] {
            let mut arguments = vec![
                "ls-files",
                "--others",
                "--exclude-standard",
                "--directory",
                "-z",
            ];
            if ignored {
                arguments.push("--ignored");
            }
            let files = self.action_read(&arguments, cancel)?;
            for file in files.split(|b| *b == 0).filter(|p| !p.is_empty()) {
                let directory = file.ends_with(b"/");
                let file = file.strip_suffix(b"/").unwrap_or(file);
                for path in affected.iter().filter(|path| paths_overlap(path, file)) {
                    // --directory avoids walking large ignored build trees. A
                    // directory sharing a prefix is safe if the actual affected
                    // path is absent; unrelated files in it need not be moved.
                    if directory {
                        let target = self
                            .info()
                            .worktree
                            .as_ref()
                            .unwrap()
                            .join(RepoPath::new(path.clone())?.argument()?);
                        match fs::symlink_metadata(target) {
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                            Err(error) => {
                                return Err(Error::io("check local file collision", error));
                            }
                            Ok(_) => {}
                        }
                    }
                    return Err(Error::CommitAction(CommitFailure::LocalFileCollision));
                }
            }
        }
        Ok(())
    }
}
fn head_id(head: &Head) -> Result<&ObjectId> {
    match head {
        Head::Branch { target, .. } | Head::Detached { target } => Ok(target),
        Head::Unborn { .. } => Err(Error::CommitAction(CommitFailure::MissingHead)),
    }
}
fn paths_overlap(a: &[u8], b: &[u8]) -> bool {
    // Git for Windows operates on a case-insensitive filesystem by default.
    #[cfg(windows)]
    let (a, b) = (
        String::from_utf8_lossy(a).to_uppercase().into_bytes(),
        String::from_utf8_lossy(b).to_uppercase().into_bytes(),
    );
    a == b
        || (a.starts_with(&b) && a.get(b.len()) == Some(&b'/'))
        || (b.starts_with(&a) && b.get(a.len()) == Some(&b'/'))
}
fn classify(write: &WriteReport, state: &Result<OperationState>) -> Option<CommitFailure> {
    let Err(error) = &write.result else {
        return None;
    };
    // A recovery command can fail on a lock/signature while conflicts are
    // already present. Report that command failure before the existing state.
    if let Error::Git { stderr, .. } = error {
        let text = String::from_utf8_lossy(stderr).to_ascii_lowercase();
        if text.contains("index.lock") {
            return Some(CommitFailure::IndexLocked);
        }
        if text.contains("unable to auto-detect email")
            || text.contains("author identity unknown")
            || text.contains("committer identity unknown")
            || text.contains("no email was given")
        {
            return Some(CommitFailure::IdentityMissing);
        }
        if text.contains("failed to sign") || text.contains("signing failed") {
            return Some(CommitFailure::SigningFailed);
        }
    }
    if let Ok(status) = &write.status {
        if status.entries.iter().any(|e| e.kind == EntryKind::Unmerged) {
            return Some(CommitFailure::ConflictsRemain);
        }
        // A revert with nothing to undo stops a range with no marker at all.
        if state.as_ref().is_ok_and(|s| s.cherry_pick || s.revert || s.sequencer)
            && !status.entries.iter().any(|e| {
                !matches!(e.kind, EntryKind::Untracked | EntryKind::Ignored)
                    && e.index != Change::Unchanged
            })
        {
            return Some(CommitFailure::EmptyChange);
        }
    }
    Some(CommitFailure::CommandFailed)
}
