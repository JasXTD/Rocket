//! Bounded local reflog reads and reviewed recovery. Reflog positions are display
//! data only: every write uses an immutable commit ID from its preview.
use crate::{commit_actions::WorkingFile, parse, runner::args, *};
use std::{collections::HashSet, fs, io::Read, path::PathBuf};

mod lost;
pub use lost::{LostCommitProgress, LostCommitScan, MAX_LOST_COMMITS};

/// The HEAD log read for deleted branches; larger logs are left to Git.
const MAX_HEAD_LOG: u64 = 32 * 1024 * 1024;
/// Dangling, retained or deleted-branch commits read before sorting by date.
const MAX_COMMIT_CANDIDATES: usize = 10_000;

/// Where recovery candidates come from. Every scope yields commit IDs only; writes
/// still go through the reviewed recovery-branch and reset flow below.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReflogScope {
    #[default]
    Head,
    /// Retained logs of all refs plus this worktree's HEAD. This does not scan
    /// unreachable objects or recreate reflogs removed by Git branch deletion.
    AllRefs,
    /// Branches this worktree left (`checkout: moving from …`) that no longer exist,
    /// at the tip they had when HEAD moved away. Requires the files ref backend.
    DeletedBranches,
    /// Commits retained under `refs/rocket/` by reset, rebase, amend, aborted
    /// sequences, tag deletion and worktree removal.
    Retained,
    /// Commits no ref or reflog reaches, including ancestors of dangling tips.
    /// These can include dropped stashes. Git's pruning can remove these at any time.
    Unreachable,
}

#[derive(Clone, Debug)]
pub struct ReflogEntry {
    pub id: ObjectId,
    /// Reflog selector, or the retained ref name; empty for unreachable commits.
    pub selector: Vec<u8>,
    /// Reflog message, or the commit subject for retained/unreachable commits.
    pub subject: Vec<u8>,
    /// Subject of the commit the entry points to.
    pub commit_subject: Vec<u8>,
    /// Reflog identity name, or the commit author for retained/unreachable commits.
    pub actor: Vec<u8>,
    /// The deleted branch name, or the full retained ref name, for those scopes.
    pub reference: Option<Vec<u8>>,
}
#[derive(Clone, Debug, Default)]
pub struct ReflogPage {
    pub entries: Vec<ReflogEntry>,
    pub more: bool,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResetMode {
    #[default]
    Soft,
    Mixed,
    Hard,
}
impl ResetMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Soft => "reset-soft",
            Self::Mixed => "reset-mixed",
            Self::Hard => "reset-hard",
        }
    }
    fn flag(self) -> &'static str {
        match self {
            Self::Soft => "--soft",
            Self::Mixed => "--mixed",
            Self::Hard => "--hard",
        }
    }
    fn operation(self) -> WriteOperation {
        match self {
            Self::Soft => WriteOperation::ResetSoft,
            Self::Mixed => WriteOperation::ResetMixed,
            Self::Hard => WriteOperation::ResetHard,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryAction {
    Branch(String),
    Reset(ResetMode),
}
impl RecoveryAction {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Branch(_) => "recover-branch",
            Self::Reset(mode) => mode.label(),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryFailure {
    LostScanFailed,
    LostScanLimit,
    LostCommitUnavailable,
    LostHistoryUnsupported,
    StalePreview,
    BranchRequired,
    ActiveOperation,
    StagedChanges,
    DirtyWorktree,
    UnsupportedIndex,
    LocalFileCollision,
    RecoveryReferenceConflict,
    IndexLocked,
    BranchExists,
    VerificationFailed,
    CommandFailed,
}
impl RecoveryFailure {
    pub fn code(self) -> &'static str {
        match self {
            Self::LostScanFailed => "lost_scan_failed",
            Self::LostScanLimit => "lost_scan_limit",
            Self::LostCommitUnavailable => "lost_commit_unavailable",
            Self::LostHistoryUnsupported => "lost_history_unsupported",
            Self::StalePreview => "stale_preview",
            Self::BranchRequired => "branch_required",
            Self::ActiveOperation => "active_operation",
            Self::StagedChanges => "staged_changes",
            Self::DirtyWorktree => "dirty_worktree",
            Self::UnsupportedIndex => "unsupported_index",
            Self::LocalFileCollision => "local_file_collision",
            Self::RecoveryReferenceConflict => "recovery_ref_conflict",
            Self::IndexLocked => "index_locked",
            Self::BranchExists => "branch_exists",
            Self::VerificationFailed => "recovery_verification_failed",
            Self::CommandFailed => "recovery_command_failed",
        }
    }
    pub fn guidance(self) -> &'static str {
        match self {
            Self::LostScanFailed => {
                "Git could not complete the lost-commit scan. This is not an empty result. Check repository integrity with Git, then scan again; no recovery branch was created."
            }
            Self::LostScanLimit => {
                "The scan exceeded 10,000 lost commits. Rocket has not presented a partial result. Inspect this repository with Git; no recovery branch was created."
            }
            Self::LostCommitUnavailable => {
                "The selected commit or its content is no longer readable. Git may have pruned it. Scan again and review an available commit; no recovery branch was created."
            }
            Self::LostHistoryUnsupported => {
                "Lost-commit recovery requires complete history without replacement refs or grafts. Inspect this repository with Git first; no recovery branch was created."
            }
            Self::StalePreview => {
                "The repository changed after this preview. Refresh and review again; the reviewed action was not started."
            }
            Self::BranchRequired => {
                "Reset requires an existing local branch. Create or switch to a branch first."
            }
            Self::ActiveOperation => {
                "Finish or safely restore the active Git operation before recovery."
            }
            Self::StagedChanges => {
                "Mixed and hard reset require no staged changes. Commit or stash them first; soft reset preserves the index and working files."
            }
            Self::DirtyWorktree => {
                "Hard reset requires clean tracked files. Commit or stash your edits first; Rocket does not discard uncommitted work here."
            }
            Self::UnsupportedIndex => {
                "Review sparse/hidden index flags or submodules with Git first. Rocket will not reset an index or worktree it cannot fully protect."
            }
            Self::LocalFileCollision => {
                "An untracked or ignored path would be replaced. Move or save that local content, then refresh the preview."
            }
            Self::RecoveryReferenceConflict => {
                "The original-tip recovery reference exists with unexpected content. Inspect it with Git; Rocket will not overwrite it."
            }
            Self::IndexLocked => {
                "Git's index is locked. Finish other Git operations, then refresh before retrying. Rocket leaves the lock file in place."
            }
            Self::BranchExists => {
                "That branch already exists. Choose a new recovery branch name; existing branches are never overwritten."
            }
            Self::VerificationFailed => {
                "The observed repository does not match the reviewed result. Inspect refreshed history and files before retrying; cancellation is not rollback."
            }
            Self::CommandFailed => {
                "Git did not confirm completion. Inspect refreshed history and the optional local Git output before retrying. The requested branch or recovery reference may already exist."
            }
        }
    }
}
fn refused<T>(f: RecoveryFailure) -> Result<T> {
    Err(Error::Recovery(f))
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Context {
    head: Head,
    state: OperationState,
    index: Vec<u8>,
    status: Vec<u8>,
    files: Vec<(RepoPath, WorkingFile)>,
}
#[derive(Clone, Debug)]
pub struct RecoveryPreview {
    repository: PathBuf,
    action: RecoveryAction,
    context: Context,
    target: Commit,
    patch: Vec<u8>,
    affected: Vec<Vec<u8>>,
    lost: bool,
    relation: Option<(u64, u64)>,
}
impl RecoveryPreview {
    /// Commits only the selected commit has, and only the current tip has. None for a
    /// lost commit, whose review shows its own change instead.
    pub fn relation(&self) -> Option<(u64, u64)> {
        self.relation
    }
    pub fn is_lost_commit(&self) -> bool {
        self.lost
    }
    pub fn action(&self) -> &RecoveryAction {
        &self.action
    }
    pub fn head(&self) -> &Head {
        &self.context.head
    }
    pub fn target(&self) -> &Commit {
        &self.target
    }
    pub fn patch(&self) -> &[u8] {
        &self.patch
    }
    pub fn recovery_reference(&self) -> Option<String> {
        match (&self.action, &self.context.head) {
            (RecoveryAction::Reset(_), Head::Branch { target, .. }) => {
                Some(format!("refs/rocket/recovery/reset/{target}"))
            }
            _ => None,
        }
    }
}
/// What a reflog entry points to, shown before any recovery is chosen.
#[derive(Clone, Debug)]
pub struct RecoveryDetails {
    pub commit: Commit,
    /// Commits only the selected commit has, and only the current tip has; None
    /// without a current commit.
    pub relation: Option<(u64, u64)>,
    /// The current tip compared with the selected commit; empty without a tip.
    pub patch: Vec<u8>,
}
#[derive(Debug)]
#[must_use = "inspect command, verification and refresh independently; cancellation is not rollback"]
pub struct RecoveryWriteReport {
    pub write: WriteReport,
    pub branches: Result<Vec<Branch>>,
    pub verification: Result<()>,
    pub failure: Option<RecoveryFailure>,
    pub recovery_reference: Option<String>,
}
impl Repository {
    /// Reload a growing window, not mutable ordinal cursors. Callers should retain
    /// selections by object ID and explicitly review them before any write.
    pub fn reflog(&self, limit: usize, cancel: &CancellationToken) -> Result<ReflogPage> {
        self.reflog_scoped(ReflogScope::Head, limit, cancel)
    }

    pub fn reflog_scoped(
        &self,
        scope: ReflogScope,
        limit: usize,
        cancel: &CancellationToken,
    ) -> Result<ReflogPage> {
        let step = match scope {
            ReflogScope::Head | ReflogScope::AllRefs => "read_reflog",
            ReflogScope::DeletedBranches => "find_deleted_branches",
            ReflogScope::Retained => "read_retained_refs",
            ReflogScope::Unreachable => "find_unreachable_commits",
        };
        self.runner.step("reflog", step, || {
            self.require_worktree(cancel)?;
            if !(1..=2000).contains(&limit) {
                return Err(Error::InvalidInput("reflog limit must be 1..2000".into()));
            }
            match scope {
                ReflogScope::DeletedBranches => return self.deleted_branches(limit, cancel),
                ReflogScope::Retained => return self.retained_commits(limit, cancel),
                ReflogScope::Unreachable => return self.unreachable_commits(limit, cancel),
                ReflogScope::Head | ReflogScope::AllRefs => {}
            }
            let unborn = matches!(self.head(cancel)?, Head::Unborn { .. });
            if scope == ReflogScope::Head && unborn {
                return Ok(ReflogPage::default());
            }
            let count = format!("--max-count={}", limit + 1);
            let mut arguments = vec![
                "reflog",
                "show",
                "-z",
                "--format=%H%x00%gD%x00%gs%x00%gn%x00%s",
                "--date=iso-strict",
                "--no-color",
                "--no-decorate",
                "--no-show-signature",
                "--no-patch",
                "--encoding=none",
                &count,
            ];
            if scope == ReflogScope::AllRefs {
                arguments.extend(["--single-worktree", "--all"]);
            }
            if !unborn {
                arguments.push("HEAD");
            }
            arguments.push("--");
            let bytes = self.recovery_read(&arguments, cancel)?;
            parse_reflog(&bytes, limit)
        })
    }
    /// Of `ids`, the commits no local or remote-tracking branch reaches: the ones
    /// worth a recovery branch. None when more than 100,000 commits are off every
    /// branch, which this check does not walk.
    pub fn off_branches(
        &self,
        ids: &[ObjectId],
        cancel: &CancellationToken,
    ) -> Result<Option<HashSet<ObjectId>>> {
        const LIMIT: usize = 100_000;
        self.runner.step("reflog", "find_off_branch", || {
            if ids.is_empty() {
                return Ok(Some(HashSet::new()));
            }
            // Revisions on stdin stay positive; --not before --stdin covers only the
            // branch patterns. Only validated object IDs are written.
            let input = ids.iter().flat_map(|id| format!("{id}\n").into_bytes()).collect();
            let output = self
                .runner
                .run(
                    self.cwd(),
                    &args(&[
                        "rev-list",
                        &format!("--max-count={}", LIMIT + 1),
                        "--not",
                        "--branches",
                        "--remotes",
                        "--stdin",
                        "--",
                    ]),
                    input,
                    cancel,
                )?
                .success("recovery read")?;
            let lines: Vec<&[u8]> = output
                .split(|b| *b == b'\n')
                .filter(|line| !line.is_empty())
                .collect();
            if lines.len() > LIMIT {
                return Ok(None);
            }
            let mut off = HashSet::new();
            for line in lines {
                let id = parse::oid(line, "reflog")?;
                if ids.contains(&id) {
                    off.insert(id);
                }
            }
            Ok(Some(off))
        })
    }
    /// Read-only. The commit an entry points to, how it relates to the current tip,
    /// and their difference, for looking before choosing a recovery.
    pub fn recovery_details(
        &self,
        target: &ObjectId,
        cancel: &CancellationToken,
    ) -> Result<RecoveryDetails> {
        self.runner.step("recovery-preview", "read_recovery_details", || {
            self.require_worktree(cancel)?;
            let commit = self
                .history(HistoryStart::Commit(target.clone()), 1, cancel)?
                .commits
                .into_iter()
                .next()
                .filter(|c| c.id == *target)
                .ok_or_else(|| {
                    Error::InvalidInput("selected commit is no longer available".into())
                })?;
            let (relation, patch) = match self.head(cancel)? {
                Head::Branch { target: tip, .. } | Head::Detached { target: tip } => {
                    let patch = self
                        .diff(
                            &DiffOptions {
                                target: DiffTarget::Between {
                                    base: tip.to_string(),
                                    target: target.to_string(),
                                },
                                ..Default::default()
                            },
                            cancel,
                        )?
                        .patch;
                    (Some(self.relation_to(&tip, target, cancel)?), patch)
                }
                Head::Unborn { .. } => (None, Vec::new()),
            };
            Ok(RecoveryDetails {
                commit,
                relation,
                patch,
            })
        })
    }
    /// Commits only `target` has, and only `tip` has.
    fn relation_to(
        &self,
        tip: &ObjectId,
        target: &ObjectId,
        cancel: &CancellationToken,
    ) -> Result<(u64, u64)> {
        let range = format!("{tip}...{target}");
        let counts =
            self.recovery_read(&["rev-list", "--left-right", "--count", &range, "--"], cancel)?;
        std::str::from_utf8(parse::line(&counts))
            .ok()
            .and_then(|line| line.split_once('\t'))
            .and_then(|(left, right)| Some((right.parse().ok()?, left.parse().ok()?)))
            .ok_or_else(|| Error::parse("recovery read", "invalid commit counts"))
    }
    pub fn preview_recovery(
        &self,
        target: &ObjectId,
        action: RecoveryAction,
        cancel: &CancellationToken,
    ) -> Result<RecoveryPreview> {
        self.runner.step("recovery-preview", "review_recovery", || {
            self.require_worktree(cancel)?;
            let context = self.recovery_context(&action, cancel)?;
            if let RecoveryAction::Branch(name) = &action {
                self.check_recovery_branch(name, cancel)?;
            }
            let commit = self
                .history(HistoryStart::Commit(target.clone()), 1, cancel)?
                .commits
                .into_iter()
                .next()
                .filter(|c| c.id == *target)
                .ok_or_else(|| {
                    Error::InvalidInput("selected commit is no longer available".into())
                })?;
            let original = match &context.head {
                Head::Branch { target, .. } | Head::Detached { target } => target,
                Head::Unborn { .. } => return refused(RecoveryFailure::BranchRequired),
            };
            let patch = self
                .diff(
                    &DiffOptions {
                        target: DiffTarget::Between {
                            base: original.to_string(),
                            target: target.to_string(),
                        },
                        ..Default::default()
                    },
                    cancel,
                )?
                .patch;
            let affected = if action == RecoveryAction::Reset(ResetMode::Hard) {
                let tree =
                    self.recovery_read(&["ls-tree", "-r", "-z", target.as_str(), "--"], cancel)?;
                let mut paths = Vec::new();
                for entry in tree.split(|b| *b == 0).filter(|e| !e.is_empty()) {
                    if entry.starts_with(b"160000 ") {
                        return refused(RecoveryFailure::UnsupportedIndex);
                    }
                    let tab = entry
                        .iter()
                        .position(|b| *b == b'\t')
                        .ok_or_else(|| Error::parse("reset tree", "missing path"))?;
                    let p = &entry[tab + 1..];
                    paths.push(RepoPath::new(p.to_vec())?.as_bytes().to_vec());
                }
                self.protect_reset_files(&paths, cancel)?;
                paths
            } else {
                Vec::new()
            };
            let relation = self.relation_to(original, target, cancel)?;
            if self.recovery_context(&action, cancel)? != context {
                return refused(RecoveryFailure::StalePreview);
            }
            Ok(RecoveryPreview {
                repository: self.info().git_dir.clone(),
                action,
                context,
                target: commit,
                patch,
                affected,
                lost: false,
                relation: Some(relation),
            })
        })
    }
    pub fn apply_recovery(
        &self,
        preview: &RecoveryPreview,
        cancel: &CancellationToken,
    ) -> Result<RecoveryWriteReport> {
        self.require_worktree(cancel)?;
        let _permit = self.acquire_write(cancel)?;
        let label = preview.action.label();
        let validate = || -> Result<()> {
            if preview.lost {
                self.lost_target_matches(&preview.target, cancel)?;
                // A live commit object alone does not guarantee its tree/blob
                // content survived concurrent pruning. Re-read the reviewed diff.
                self.runner.step(label, "revalidate_lost_content", || {
                    if self.lost_patch(&preview.target.id, cancel)? != preview.patch {
                        return refused(RecoveryFailure::StalePreview);
                    }
                    Ok(())
                })?;
            }
            if preview.repository != self.info().git_dir
                || self.recovery_context(&preview.action, cancel)? != preview.context
            {
                return refused(RecoveryFailure::StalePreview);
            }
            if let RecoveryAction::Branch(name) = &preview.action {
                self.check_recovery_branch(name, cancel)?;
            }
            self.protect_reset_files(&preview.affected, cancel)
        };
        self.runner.step(label, "revalidate_preview", validate)?;
        let recovery_reference = preview.recovery_reference();
        if let (Some(reference), Head::Branch { target, .. }) =
            (&recovery_reference, &preview.context.head)
        {
            self.runner.step(label, "save_recovery_reference", || {
                self.save_reset_reference(reference, target, cancel)
            })?;
            self.runner
                .step(label, "revalidate_before_reset", validate)?;
        }
        cancel.check()?;
        let write = match &preview.action {
            RecoveryAction::Reset(mode) => self.execute_write(
                mode.operation(),
                &args(&[
                    "-c",
                    "submodule.recurse=false",
                    "reset",
                    mode.flag(),
                    "--no-recurse-submodules",
                    preview.target.id.as_str(),
                    "--",
                ]),
                Vec::new(),
                cancel,
            )?,
            RecoveryAction::Branch(name) => {
                let mut arguments = args(&[
                    "-c",
                    "submodule.recurse=false",
                    "branch",
                    "--no-track",
                    "--",
                ]);
                arguments.push(self.branch_argument(name.as_bytes(), cancel)?);
                arguments.push(preview.target.id.as_str().into());
                self.execute_write(WriteOperation::CreateBranch, &arguments, Vec::new(), cancel)?
            }
        };
        // Never reuse the caller's cancelled token for reconciliation.
        let fresh = CancellationToken::new();
        let branches = self
            .runner
            .observe("refresh", "branches", || self.branches(&fresh));
        let verification = self.runner.step(label, "verify_recovery", || {
            self.verify_recovery(preview, &fresh)
        });
        let failure = match &write.result {
            Err(Error::Git { stderr, .. })
                if String::from_utf8_lossy(stderr).contains("index.lock") =>
            {
                Some(RecoveryFailure::IndexLocked)
            }
            Err(_) => Some(RecoveryFailure::CommandFailed),
            Ok(_) if verification.is_err() => Some(RecoveryFailure::VerificationFailed),
            _ => None,
        };
        // Log only fixed failure codes, independently of raw local Git output.
        let _ = self
            .runner
            .observe("recovery_outcome", label, || match failure {
                Some(f) => refused::<()>(f),
                None => Ok(()),
            });
        Ok(RecoveryWriteReport {
            write,
            branches,
            verification,
            failure,
            recovery_reference,
        })
    }
    fn recovery_read(&self, arguments: &[&str], cancel: &CancellationToken) -> Result<Vec<u8>> {
        self.runner
            .run(self.cwd(), &args(arguments), Vec::new(), cancel)?
            .success("recovery read")
    }
    fn recovery_context(
        &self,
        action: &RecoveryAction,
        cancel: &CancellationToken,
    ) -> Result<Context> {
        let state = self.operation_state(cancel)?;
        if !state.is_idle() {
            return refused(RecoveryFailure::ActiveOperation);
        }
        let status = self.status(cancel)?;
        let tracked = status
            .entries
            .iter()
            .filter(|e| !matches!(e.kind, EntryKind::Untracked | EntryKind::Ignored))
            .collect::<Vec<_>>();
        if let RecoveryAction::Reset(mode) = action {
            if !matches!(status.head, Head::Branch { .. }) {
                return refused(RecoveryFailure::BranchRequired);
            }
            if *mode != ResetMode::Soft && tracked.iter().any(|e| e.index != Change::Unchanged) {
                return refused(RecoveryFailure::StagedChanges);
            }
            if *mode == ResetMode::Hard && !tracked.is_empty() {
                return refused(RecoveryFailure::DirtyWorktree);
            }
        }
        let index = self.recovery_read(&["ls-files", "--stage", "-v", "-z"], cancel)?;
        if matches!(
            action,
            RecoveryAction::Reset(ResetMode::Mixed | ResetMode::Hard)
        ) && index.split(|b| *b == 0).any(|e| {
            e.first()
                .is_some_and(|b| b.is_ascii_lowercase() || *b == b'S')
                || (matches!(action, RecoveryAction::Reset(ResetMode::Hard))
                    && e.get(2..8) == Some(b"160000"))
        }) {
            return refused(RecoveryFailure::UnsupportedIndex);
        }
        let files = self.snapshot_changed_files(&status, cancel)?;
        let bytes = self.recovery_read(
            &[
                "status",
                "--porcelain=v2",
                "-z",
                "--untracked-files=no",
                "--ignore-submodules=none",
            ],
            cancel,
        )?;
        Ok(Context {
            head: status.head,
            state,
            index,
            status: bytes,
            files,
        })
    }
    fn check_recovery_branch(&self, name: &str, cancel: &CancellationToken) -> Result<()> {
        self.branch_argument(name.as_bytes(), cancel)?;
        let result = self.runner.run(
            self.cwd(),
            &args(&["show-ref", "--exists", "--", &format!("refs/heads/{name}")]),
            Vec::new(),
            cancel,
        )?;
        match result.status.code() {
            Some(2) => Ok(()),
            Some(0) => refused(RecoveryFailure::BranchExists),
            _ => result.success("check recovery branch").map(|_| ()),
        }
    }
    fn protect_reset_files(&self, paths: &[Vec<u8>], cancel: &CancellationToken) -> Result<()> {
        self.protect_local_files(paths, cancel)
            .map_err(|e| match e {
                Error::CommitAction(CommitFailure::LocalFileCollision) => {
                    Error::Recovery(RecoveryFailure::LocalFileCollision)
                }
                other => other,
            })
    }
    fn save_reset_reference(
        &self,
        name: &str,
        id: &ObjectId,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let exists = self.runner.run(
            self.cwd(),
            &args(&["show-ref", "--exists", "--", name]),
            Vec::new(),
            cancel,
        )?;
        match exists.status.code() {
            Some(2) => {
                self.runner
                    .write(
                        self.cwd(),
                        &args(&[
                            "update-ref",
                            "--no-deref",
                            name,
                            id.as_str(),
                            &"0".repeat(id.as_str().len()),
                        ]),
                        Vec::new(),
                        cancel,
                    )?
                    .success("save reset recovery reference")?;
            }
            Some(0) => {
                let symbolic = self.runner.run(
                    self.cwd(),
                    &args(&["symbolic-ref", "-q", name]),
                    Vec::new(),
                    cancel,
                )?;
                if symbolic.status.code() != Some(1) || self.resolve_commit(name, cancel)? != *id {
                    return refused(RecoveryFailure::RecoveryReferenceConflict);
                }
            }
            _ => {
                exists.success("check reset recovery reference")?;
            }
        }
        Ok(())
    }
    fn verify_recovery(&self, p: &RecoveryPreview, cancel: &CancellationToken) -> Result<()> {
        let head = self.head(cancel)?;
        let expected = match (&p.action, &p.context.head) {
            (RecoveryAction::Reset(_), Head::Branch { name, .. }) => Head::Branch {
                name: name.clone(),
                target: p.target.id.clone(),
            },
            _ => p.context.head.clone(),
        };
        if head != expected {
            return refused(RecoveryFailure::VerificationFailed);
        }
        match &p.action {
            RecoveryAction::Branch(name) => {
                if self.resolve_commit(&format!("refs/heads/{name}"), cancel)? != p.target.id {
                    return refused(RecoveryFailure::VerificationFailed);
                }
                if self.recovery_context(&p.action, cancel)? != p.context {
                    return refused(RecoveryFailure::VerificationFailed);
                }
            }
            RecoveryAction::Reset(ResetMode::Soft) => {
                if self.recovery_read(&["ls-files", "--stage", "-v", "-z"], cancel)?
                    != p.context.index
                {
                    return refused(RecoveryFailure::VerificationFailed);
                }
            }
            RecoveryAction::Reset(_) => {
                let diff = self.runner.run(
                    self.cwd(),
                    &args(&[
                        "diff",
                        "--cached",
                        "--quiet",
                        "--no-ext-diff",
                        "--no-textconv",
                        p.target.id.as_str(),
                        "--",
                    ]),
                    Vec::new(),
                    cancel,
                )?;
                if !diff.status.success() {
                    return refused(RecoveryFailure::VerificationFailed);
                }
                if p.action == RecoveryAction::Reset(ResetMode::Hard)
                    && self
                        .status(cancel)?
                        .entries
                        .iter()
                        .any(|e| !matches!(e.kind, EntryKind::Untracked | EntryKind::Ignored))
                {
                    return refused(RecoveryFailure::VerificationFailed);
                }
            }
        }
        Ok(())
    }
}
fn parse_reflog(bytes: &[u8], limit: usize) -> Result<ReflogPage> {
    if bytes.is_empty() {
        return Ok(ReflogPage::default());
    }
    let fields = bytes
        .strip_suffix(&[0])
        .ok_or_else(|| Error::parse("reflog", "missing record terminator"))?
        .split(|b| *b == 0)
        .collect::<Vec<_>>();
    if fields.len() % 5 != 0 {
        return Err(Error::parse("reflog", "incomplete record"));
    }
    let more = fields.len() / 5 > limit;
    let entries = fields
        .as_chunks::<5>()
        .0
        .iter()
        .take(limit)
        .map(|f| {
            Ok(ReflogEntry {
                id: parse::oid(f[0], "reflog")?,
                selector: f[1].to_vec(),
                subject: f[2].to_vec(),
                actor: f[3].to_vec(),
                commit_subject: f[4].to_vec(),
                reference: None,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(ReflogPage { entries, more })
}

impl Repository {
    /// Branch names this worktree checked out and left, which no longer exist. The
    /// raw files-backend log keeps the exact tip each one had when HEAD moved away.
    fn deleted_branches(&self, limit: usize, cancel: &CancellationToken) -> Result<ReflogPage> {
        if parse::line(&self.recovery_read(&["rev-parse", "--show-ref-format"], cancel)?)
            != b"files"
        {
            return Err(Error::Unsupported(
                "Deleted-branch discovery reads Git's files reflog. This repository stores refs in another format; use the HEAD or all-refs history instead."
                    .into(),
            ));
        }
        let Some(log) = bounded_file(&self.info().git_dir.join("logs").join("HEAD"))? else {
            return Ok(ReflogPage::default());
        };
        let existing: HashSet<Vec<u8>> = self
            .recovery_read(
                &["for-each-ref", "--format=%(refname)", "refs/heads/"],
                cancel,
            )?
            .split(|b| *b == b'\n')
            .filter_map(|l| l.strip_prefix(b"refs/heads/"))
            .map(<[u8]>::to_vec)
            .collect();
        let mut seen = HashSet::new();
        let mut entries = Vec::new();
        // Newest first; Git itself skips reflog lines it cannot parse.
        for line in log.split(|b| *b == b'\n').rev().filter(|l| !l.is_empty()) {
            cancel.check()?;
            let Some(entry) = LogLine::parse(line) else {
                continue;
            };
            let Some(name) = entry
                .message
                .strip_prefix(b"checkout: moving from ")
                .and_then(|rest| {
                    let end = rest.windows(4).position(|w| w == b" to ")?;
                    Some(&rest[..end])
                })
            else {
                continue;
            };
            if !plausible_branch(name)
                || existing.contains(name)
                || entry.old.as_str().bytes().all(|b| b == b'0')
                || !seen.insert(name.to_vec())
            {
                continue;
            }
            entries.push(ReflogEntry {
                selector: format!("HEAD@{{{}}}", entry.date()).into_bytes(),
                id: entry.old,
                subject: entry.message.to_vec(),
                actor: entry.name.to_vec(),
                commit_subject: Vec::new(),
                reference: Some(name.to_vec()),
            });
            if entries.len() == MAX_COMMIT_CANDIDATES {
                break;
            }
        }
        // Garbage collection may already have pruned an old tip.
        let present = self.existing_commits(entries.iter().map(|e| &e.id), cancel)?;
        entries.retain(|e| present.contains(&e.id));
        let more = entries.len() > limit;
        entries.truncate(limit);
        let commits = self.recovery_commits(entries.iter().map(|e| &e.id), cancel)?;
        for entry in &mut entries {
            if let Some(commit) = commits.iter().find(|c| c.id == entry.id) {
                entry.commit_subject = commit.subject_lossy().into_owned().into_bytes();
            }
        }
        Ok(ReflogPage { entries, more })
    }

    /// Commits retained by Rocket's own recovery references, newest first.
    fn retained_commits(&self, limit: usize, cancel: &CancellationToken) -> Result<ReflogPage> {
        let bytes = self.recovery_read(
            &[
                "for-each-ref",
                "--format=%(refname)%00%(objectname)%00%(objecttype)%00%(*objectname)%00%(*objecttype)",
                "refs/rocket/",
            ],
            cancel,
        )?;
        let mut retained = Vec::new();
        for record in bytes.split(|b| *b == b'\n').filter(|r| !r.is_empty()) {
            cancel.check()?;
            let fields: Vec<_> = record.split(|b| *b == 0).collect();
            if fields.len() != 5 {
                return Err(Error::parse("retained refs", "malformed ref record"));
            }
            // Retained tag objects are recoverable when they peel to a commit.
            let id = match (fields[2], fields[4]) {
                (b"commit", _) => fields[1],
                (b"tag", b"commit") => fields[3],
                _ => continue,
            };
            retained.push((fields[0].to_vec(), parse::oid(id, "retained refs")?));
            if retained.len() == MAX_COMMIT_CANDIDATES {
                break;
            }
        }
        let commits = self.recovery_commits(retained.iter().map(|(_, id)| id), cancel)?;
        let mut entries: Vec<_> = retained
            .into_iter()
            .filter_map(|(name, id)| {
                let commit = commits.iter().find(|c| c.id == id)?;
                Some((
                    commit.committer.timestamp,
                    commit_entry(commit, name.clone(), Some(name)),
                ))
            })
            .collect();
        entries.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.selector.cmp(&b.1.selector)));
        let more = entries.len() > limit;
        Ok(ReflogPage {
            entries: entries.into_iter().take(limit).map(|(_, e)| e).collect(),
            more,
        })
    }

    /// History unreachable from refs and reflogs, newest commit first.
    fn unreachable_commits(&self, limit: usize, cancel: &CancellationToken) -> Result<ReflogPage> {
        self.scan_lost_commits(cancel, None)?.page(limit)
    }

    fn existing_commits<'a>(
        &self,
        ids: impl Iterator<Item = &'a ObjectId>,
        cancel: &CancellationToken,
    ) -> Result<HashSet<ObjectId>> {
        let input: Vec<u8> = ids.flat_map(|id| format!("{id}\n").into_bytes()).collect();
        if input.is_empty() {
            return Ok(HashSet::new());
        }
        let output = self
            .runner
            .run(
                self.cwd(),
                &args(&["cat-file", "--batch-check"]),
                input,
                cancel,
            )?
            .success("recovery read")?;
        let mut present = HashSet::new();
        for line in output.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
            let fields: Vec<_> = line.split(|b| *b == b' ').collect();
            if fields.len() == 3 && fields[1] == b"commit" {
                present.insert(parse::oid(fields[0], "recovery read")?);
            }
        }
        Ok(present)
    }

    fn recovery_commits<'a>(
        &self,
        ids: impl Iterator<Item = &'a ObjectId>,
        cancel: &CancellationToken,
    ) -> Result<Vec<Commit>> {
        let mut unique = Vec::new();
        let mut seen = HashSet::new();
        for id in ids {
            if seen.insert(id.clone()) {
                unique.push(id.clone());
            }
        }
        if unique.is_empty() {
            return Ok(Vec::new());
        }
        let input = unique
            .iter()
            .flat_map(|id| format!("{id}\n").into_bytes())
            .collect();
        let objects = self
            .runner
            .run(self.cwd(), &args(&["cat-file", "--batch"]), input, cancel)?
            .success("recovery read")?;
        parse::commits(&objects, &unique)
    }
}

fn commit_entry(commit: &Commit, selector: Vec<u8>, reference: Option<Vec<u8>>) -> ReflogEntry {
    let subject = commit
        .message
        .split(|b| *b == b'\n')
        .next()
        .unwrap_or_default()
        .to_vec();
    ReflogEntry {
        id: commit.id.clone(),
        selector,
        commit_subject: subject.clone(),
        subject,
        actor: commit.author.name.clone(),
        reference,
    }
}

/// One files-backend reflog line:
/// `<old> <new> <name> <<email>> <seconds> <±hhmm>\t<message>`.
struct LogLine<'a> {
    old: ObjectId,
    name: &'a [u8],
    timestamp: i64,
    offset_minutes: i32,
    message: &'a [u8],
}
impl<'a> LogLine<'a> {
    fn parse(line: &'a [u8]) -> Option<Self> {
        let (head, message) = match line.iter().position(|b| *b == b'\t') {
            Some(tab) => (&line[..tab], &line[tab + 1..]),
            None => (line, &b""[..]),
        };
        let first = head.iter().position(|b| *b == b' ')?;
        let old = parse::oid(&head[..first], "reflog").ok()?;
        let rest = &head[first + 1..];
        let second = rest.iter().position(|b| *b == b' ')?;
        parse::oid(&rest[..second], "reflog").ok()?;
        let ident = &rest[second + 1..];
        let close = ident.iter().rposition(|b| *b == b'>')?;
        let open = ident[..close].iter().rposition(|b| *b == b'<')?;
        let name = ident[..open].strip_suffix(b" ").unwrap_or(&ident[..open]);
        let when: Vec<_> = ident[close + 1..]
            .trim_ascii()
            .split(|b| *b == b' ')
            .collect();
        let [seconds, zone] = when.as_slice() else {
            return None;
        };
        if zone.len() != 5
            || !matches!(zone[0], b'+' | b'-')
            || !zone[1..].iter().all(u8::is_ascii_digit)
        {
            return None;
        }
        let digit = |i: usize| i32::from(zone[i] - b'0');
        let offset = (digit(1) * 10 + digit(2)) * 60 + digit(3) * 10 + digit(4);
        Some(Self {
            old,
            name,
            timestamp: std::str::from_utf8(seconds).ok()?.parse().ok()?,
            offset_minutes: if zone[0] == b'-' { -offset } else { offset },
            message,
        })
    }
    fn date(&self) -> String {
        iso_strict(self.timestamp, self.offset_minutes)
    }
}

/// Git's `--date=iso-strict` layout, used for display selectors only.
fn iso_strict(timestamp: i64, offset_minutes: i32) -> String {
    let local = timestamp + i64::from(offset_minutes) * 60;
    let (year, month, day) = civil(local.div_euclid(86_400));
    let seconds = local.rem_euclid(86_400);
    let offset = offset_minutes.unsigned_abs();
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}{}{:02}:{:02}",
        seconds / 3600,
        seconds % 3600 / 60,
        seconds % 60,
        if offset_minutes < 0 { '-' } else { '+' },
        offset / 60,
        offset % 60
    )
}

/// Days since 1970-01-01 to a proleptic Gregorian date (Hinnant's algorithm).
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

/// A checkout message names the branch left, or a commit when HEAD was detached.
/// Only a valid-looking branch name, never HEAD or an object ID, is a candidate.
fn plausible_branch(name: &[u8]) -> bool {
    !name.is_empty()
        && name.len() <= 1024
        && name != b"HEAD"
        && !(matches!(name.len(), 40 | 64) && name.iter().all(u8::is_ascii_hexdigit))
        && !name.starts_with(b"-")
        && !name.starts_with(b"/")
        && !name.ends_with(b"/")
        && !name.ends_with(b".")
        && !name.ends_with(b".lock")
        && !name
            .windows(2)
            .any(|w| w == b".." || w == b"@{" || w == b"//")
        && name
            .iter()
            .all(|b| *b > b' ' && *b != 0x7f && !b"~^:?*[\\".contains(b))
}

/// The bounded raw HEAD reflog; absent is None, links are refused.
fn bounded_file(path: &std::path::Path) -> Result<Option<Vec<u8>>> {
    let meta = match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error::io("read HEAD reflog", error)),
        Ok(meta) => meta,
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(Error::Unsupported(
                "The HEAD reflog is redirected. Review it with Git.".into(),
            ));
        }
    }
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(Error::Unsupported(
            "The HEAD reflog is not a regular file. Review it with Git.".into(),
        ));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| Error::io("read HEAD reflog", e))?
        .take(MAX_HEAD_LOG + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io("read HEAD reflog", e))?;
    if bytes.len() as u64 > MAX_HEAD_LOG {
        return Err(Error::Unsupported(
            "The HEAD reflog exceeds 32 MiB. Review older checkouts with Git.".into(),
        ));
    }
    Ok(Some(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reflog_parser_preserves_unicode_and_refuses_incomplete_records() {
        let id = "a".repeat(40);
        let entry = format!("{id}\0HEAD@{{2026-10-18T10:00:00+08:00}}\0commit: 検索 🚀\0Author\0元の件名 📄\0");
        let page = parse_reflog(entry.repeat(2).as_bytes(), 1).unwrap();
        assert!(page.more);
        assert_eq!(page.entries[0].subject, "commit: 検索 🚀".as_bytes());
        assert_eq!(page.entries[0].commit_subject, "元の件名 📄".as_bytes());
        assert!(parse_reflog(&entry.as_bytes()[..entry.len() - 1], 1).is_err());
        assert!(parse_reflog(b"not-an-object\0HEAD@{0}\0message\0actor\0subject\0", 1).is_err());
        assert!(parse_reflog(format!("{id}\0HEAD@{{0}}\0message\0actor\0").as_bytes(), 1).is_err());
        assert!(parse_reflog(b"truncated\0selector\0", 1).is_err());
    }
    #[test]
    fn raw_reflog_lines_keep_the_old_tip_and_skip_malformed_lines() {
        let (old, new) = ("a".repeat(40), "b".repeat(64));
        let line = format!(
            "{old} {new} 林 安 <x@example.invalid> 1700000000 +0530\tcheckout: moving from 功能 to main"
        );
        let entry = LogLine::parse(line.as_bytes()).unwrap();
        assert_eq!(entry.old.as_str(), old);
        assert_eq!(entry.name, "林 安".as_bytes());
        assert_eq!(
            entry.message,
            "checkout: moving from 功能 to main".as_bytes()
        );
        assert_eq!(entry.date(), "2023-11-15T03:43:20+05:30");
        for bad in [
            format!("{old} {new} name <e> 1700000000\tmessage"),
            format!("{old} {new} name <e> 1700000000 +5\tmessage"),
            format!("{old} not-an-id name <e> 1 +0000\tmessage"),
            "short".into(),
        ] {
            assert!(LogLine::parse(bad.as_bytes()).is_none(), "{bad}");
        }
        assert_eq!(iso_strict(0, 0), "1970-01-01T00:00:00+00:00");
        assert_eq!(iso_strict(-1, 0), "1969-12-31T23:59:59+00:00");
        assert_eq!(iso_strict(951_782_400, 0), "2000-02-29T00:00:00+00:00");
        assert_eq!(iso_strict(1_700_000_000, -420), "2023-11-14T15:13:20-07:00");
    }
    #[test]
    fn only_branch_like_names_are_deleted_branch_candidates() {
        for name in ["feature/検索", "fix-1", "a.b"] {
            assert!(plausible_branch(name.as_bytes()), "{name}");
        }
        for name in [
            "",
            "HEAD",
            &"c".repeat(40),
            &"d".repeat(64),
            "-option",
            "a..b",
            "a@{1}",
            "trailing/",
            "x.lock",
            "with space",
            "tilde~1",
            "glob*",
        ] {
            assert!(!plausible_branch(name.as_bytes()), "{name}");
        }
    }
}
