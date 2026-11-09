//! The only bridge from widgets to Git. All calls run on backend tasks.
use rocket_git::*;
mod refresh;
mod benchmark;
pub use refresh::{NetworkRefresh, Refresh, WorktreeRefresh};
pub(crate) use refresh::{NetworkRead, RefreshMode};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, mpsc::SyncSender},
    time::Duration,
};
#[derive(Clone)]
pub struct Backend {
    client: GitClient,
    display_cache: Arc<crate::diff::cache::Cache>,
    pub executable: PathBuf,
    pub version: String,
    pub log: DiagnosticLog,
}
pub struct Snapshot {
    pub watcher: Option<Arc<RepositoryWatch>>,
    pub(crate) refresh_guard: Option<refresh::Guard>,
    pub info: RepositoryInfo,
    /// Reuse discovery for diff reads; mutable Git data is still read per request.
    pub repository: Repository,
    pub head: Option<Head>,
    pub status: Option<Status>,
    pub branches: Vec<Branch>,
    pub remotes: Vec<Remote>,
    pub stashes: Vec<StashEntry>,
    pub state: Option<OperationState>,
    /// Rocket's reviewed cherry-pick/revert sequence, read only while Git's
    /// sequencer is active.
    pub sequence: Option<CommitSequence>,
    pub history: Option<HistoryPage>,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug)]
pub enum Action {
    Stage(Vec<RepoPath>),
    StageAll,
    Unstage(Vec<RepoPath>),
    UnstageAll,
    ApplyReview(Arc<ChangeReview>, Vec<usize>),
    MarkResolved(RepoPath),
    SaveConflict {
        preview: Arc<ConflictPreview>,
        text: String,
        stage: bool,
    },
    KeepConflictSide {
        preview: Arc<WholeConflictPreview>,
        keep: ConflictKeep,
    },
    Commit(String),
    UndoCommit(Arc<UndoCommitPreview>),
    Rebase(Arc<RebasePreview>),
    MailImport(Arc<MailImportPreview>),
    Bisect(Arc<BisectPreview>),
    BisectAuto { preview: Arc<BisectAutoPreview>, progress: Arc<std::sync::Mutex<BisectAutoProgress>>, storage: PathBuf },
    Recovery(Arc<RecoveryPreview>),
    Tag(Arc<TagPreview>),
    RemoteTag(Arc<RemoteTagPreview>),
    /// Delete the reviewed local tags in order (only the ready ones).
    TagBatch(Arc<TagBatchReview<TagPreview>>),
    /// Publish the reviewed local tags in order (only the ready ones).
    RemoteTagBatch(Arc<TagBatchReview<RemoteTagPreview>>),
    Worktree(Arc<WorktreePreview>),
    Submodule(Arc<SubmodulePreview>),
    SubmoduleAction(Arc<SubmoduleActionPreview>),
    Lfs(Arc<LfsPreview>),
    LfsLock(Arc<LfsLockPreview>),
    FileEdit(Arc<FileEditPreview>),
    ApplyPatch(Arc<PatchPreview>),
    ExportPatch(Arc<PatchExportPreview>),
    ExportPatchSeries(Arc<PatchSeriesPreview>),
    CommitAction {
        preview: Arc<CommitPreview>,
        message: Option<String>,
    },
    SetSigning {
        expected: Arc<RepositorySigning>,
        options: SigningOptions,
    },
    SetIssueLinks {
        expected: IssueLinkSettings,
        rules: Vec<IssueLinkRule>,
    },
    SetIdentity {
        expected: RepositoryIdentity,
        identity: LocalIdentity,
    },
    CreateBranch(String),
    SwitchBranch(Vec<u8>),
    /// A new local branch `name` tracking the remote-tracking ref `upstream`, switched to.
    CheckOutRemote { name: Vec<u8>, upstream: Vec<u8> },
    /// After a branch switch; the path limits sources to the MCP fixture folder.
    UpdateSubmodules(Option<PathBuf>),
    RenameBranch(Vec<u8>, String),
    DeleteBranch(Vec<u8>),
    DeleteBranches(Vec<Vec<u8>>),
    /// A confirmed custom command: exactly this program and these arguments.
    RunCommand {
        name: String,
        program: PathBuf,
        arguments: Vec<std::ffi::OsString>,
        storage: PathBuf,
    },
    AddRemote {
        name: Vec<u8>,
        url: Vec<u8>,
    },
    EditRemote {
        expected: Remote,
        kind: RemoteUrlKind,
        url: Option<Vec<u8>>,
    },
    RemoveRemote(Remote),
    Fetch(Vec<u8>),
    FetchWithTags(Vec<u8>, FetchTags),
    Pull,
    PullFetch,
    PullReviewed(Arc<PullPreview>),
    ForcePush(Arc<ForcePushPreview>),
    AtomicPush(Arc<AtomicPushPreview>),
    Ignore(Arc<IgnorePreview>),
    Workflow { review: Arc<crate::workflows::Review>, selected:Vec<Vec<u8>> },
    Push {
        remote: Vec<u8>,
        local: Vec<u8>,
        destination: Vec<u8>,
        set_upstream: bool,
    },
    Stash {
        message: String,
        include_untracked: bool,
        /// Empty saves every change; otherwise only these changed files.
        paths: Vec<RepoPath>,
    },
    ApplyStash {
        entry: StashEntry,
        restore_index: bool,
    },
    DropStash(StashEntry),
    Merge {
        target: String,
        mode: MergeMode,
    },
    MergeReviewed(Arc<MergePreview>),
    GitFlowInit(Arc<GitFlowInitPreview>),
    GitFlowStart(Arc<GitFlowStartPreview>),
    GitFlowFinish(Arc<GitFlowFinishPreview>),
    ContinueMerge(String),
    AbortMerge,
    FinishMerge,
}
impl Action {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Stage(_) | Self::StageAll => "stage",
            Self::Unstage(_) | Self::UnstageAll => "unstage",
            Self::ApplyReview(review, _) => {
                if review.staged() {
                    "unstage-selection"
                } else {
                    "stage-selection"
                }
            }
            Self::MarkResolved(_) => "mark-resolved",
            Self::SaveConflict { .. } => "conflict-save",
            Self::KeepConflictSide { .. } => "conflict-keep",
            Self::Commit(_) => "commit",
            Self::UndoCommit(_) => "undo-commit",
            Self::CommitAction { preview, .. } => preview.request().label(),
            Self::Rebase(preview) => preview.action().label(),
            Self::MailImport(preview) => preview.action().label(),
            Self::Bisect(preview) => preview.action().label(),
            Self::BisectAuto { .. } => "bisect-auto",
            Self::Recovery(preview) => preview.action().label(),
            Self::Tag(preview) => preview.action().label(),
            Self::RemoteTag(preview) => preview.action().label(),
            Self::TagBatch(_) => "tag-delete-batch",
            Self::RemoteTagBatch(_) => "remote-tag-publish-batch",
            Self::Worktree(preview) => preview.label(),
            Self::Submodule(preview) => preview.label(),
            Self::SubmoduleAction(preview) => preview.label(),
            Self::Lfs(preview) => preview.action().label(),
            Self::LfsLock(preview) => preview.action().label(),
            Self::FileEdit(preview) => preview.source().label(),
            Self::ApplyPatch(preview) => preview.label(),
            Self::ExportPatch(_) => "patch-export",
            Self::ExportPatchSeries(_) => "patch-series-export",
            Self::SetSigning { .. } => "signing-save",
            Self::SetIdentity { .. } => "identity-save",
            Self::SetIssueLinks { .. } => "issue-links-save",
            Self::CreateBranch(_) => "branch-create",
            Self::SwitchBranch(_) => "branch-switch",
            Self::CheckOutRemote { .. } => "branch-checkout-remote",
            Self::UpdateSubmodules(_) => "submodule-checkout",
            Self::RenameBranch(..) => "branch-rename",
            Self::DeleteBranch(_) => "branch-delete",
            Self::DeleteBranches(_) => "branch-delete-batch",
            Self::RunCommand { .. } => "custom-command",
            Self::AddRemote { .. } => "remote-add",
            Self::EditRemote { .. } => "remote-edit",
            Self::RemoveRemote(_) => "remote-remove",
            Self::Fetch(_) | Self::FetchWithTags(..) => "fetch",
            Self::Pull => "pull",
            Self::PullFetch => "pull-fetch",
            Self::PullReviewed(_) => "pull-apply",
            Self::Push { .. } => "push",
            Self::ForcePush(_) => "force-push",
            Self::AtomicPush(_) => "atomic-push",
            Self::Ignore(_) => "ignore-save",
            Self::Workflow{review,..} => review.label(),
            Self::Stash { .. } => "stash-push",
            Self::ApplyStash { .. } => "stash-apply",
            Self::DropStash(_) => "stash-drop",
            Self::Merge { .. } | Self::MergeReviewed(_) => "merge",
            Self::ContinueMerge(_) => "merge-continue",
            Self::AbortMerge => "merge-abort",
            Self::FinishMerge => "merge-finish",
            Self::GitFlowInit(_) => "gitflow-init",
            Self::GitFlowStart(_) => "gitflow-start",
            Self::GitFlowFinish(_) => "gitflow-finish",
        }
    }
}
/// Command outcome is independent from every later refresh/verification outcome.
#[derive(Default, Debug)]
pub struct Outcome {
    #[doc(hidden)]
    pub network_refresh: Option<NetworkRefresh>,
    pub undo_commit: Option<Arc<UndoCommit>>,
    pub command_succeeded: bool,
    /// A rebase or import stopped to wait for you, which isn't a failure.
    pub paused: bool,
    /// Clear only the submitted draft when a composed push fails after commit.
    pub local_commit_created: bool,
    /// Machine-readable recovery observations for composed workflows. Keep raw
    /// command output, credentials and configuration out of this contract.
    pub workflow: Option<WorkflowOutcome>,
    pub recovery_pending: bool,
    pub summary: String,
    pub details: Vec<String>,
    pub raw_output: String,
    pub open_repository: Option<PathBuf>,
    pub retained_workspace: Option<PathBuf>,
    pub conflict: Option<ConflictOutcome>,
    pub whole_conflict: Option<WholeConflictOutcome>,
    pub lfs_locks: Option<LfsLocks>,
    pub pull_fetched: Option<Arc<PullFetched>>,
    /// Nothing happened worth a line: no notice, Activity entry or refresh.
    pub quiet: bool,
}
#[derive(Debug)]
pub enum WorkflowOutcome {
    CommitPush {
        phase: &'static str,
        commit: Option<ObjectId>,
        reviewed_commit_confirmed: bool,
        remote_outcome: &'static str,
        cancelled: bool,
    },
    Saved {
        phase: &'static str,
        stash: Option<ObjectId>,
        journal: PathBuf,
        cancelled: bool,
    },
}
#[derive(Debug)]
pub struct ConflictOutcome {
    pub saved: bool,
    pub staged: bool,
    pub preview: Option<Arc<ConflictPreview>>,
}
#[derive(Debug)]
pub struct WholeConflictOutcome {
    pub keep: ConflictKeep,
    pub file_change: Option<WholeFileChange>,
    pub staged: bool,
}
impl Outcome {
    fn lfs_lock(&mut self, report: LfsLockReport) {
        let command = self.observe("Send lock operation", report.result).is_some();
        let local = self
            .observe("Verify local file and staging", report.local)
            .is_some();
        let observed = self.observe("Refresh server locks", report.observed);
        let matches = self
            .observe("Confirm server result", report.verified)
            .is_some();
        self.lfs_locks = observed;
        self.command_succeeded = command && local && matches;
        self.recovery_pending = !self.command_succeeded;
        self.summary=match report.outcome {
            LfsLockOutcome::NotStarted=>"LFS lock write was not started. Refresh and review the reported error.",
            LfsLockOutcome::ConfirmedByHelper if self.command_succeeded=>"LFS lock operation completed; server state and local contents verified.",
            LfsLockOutcome::ConfirmedByHelper=>"The helper confirmed the lock operation, but verification needs review. Refresh locks before retrying.",
            LfsLockOutcome::Unknown=>"LFS lock request was interrupted or failed. Remote outcome is uncertain; refresh locks before retrying.",
        }.into();
        self.details.push(format!("Remote outcome: {}. No automatic retry or rollback. Staging and commits are not changed by locking.",report.outcome.code()));
        if command && !matches {
            self.details.push("The refreshed lock list did not confirm the requested result, or could not be read.".into());
        }
    }
    fn patch(&mut self, report: PatchReport, updates_modes: bool) {
        let command = self
            .observe("Apply reviewed patch", report.result)
            .is_some();
        let verified = self
            .observe("Verify patch files", report.verification)
            .is_some();
        self.command_succeeded = command && verified;
        self.recovery_pending = !verified || (!command && !report.changed.is_empty());
        self.summary = if command && verified {
            format!(
                "Updated and verified {} file(s); {}.",
                report.changed.len(),
                if updates_modes { "reviewed executable modes recorded in the index" } else { "staging unchanged" }
            )
        } else {
            format!(
                "Patch stopped after changing {} file(s). Review the result before retrying; recovery copies are retained.",
                report.changed.len()
            )
        };
        for path in report.changed {
            self.details
                .push(format!("Changed: {}", path.display_lossy()));
        }
        for path in report.recovery {
            self.details
                .push(format!("Recovery copies: {}", path.display()));
        }
        if updates_modes {
            self.details.push("Mode publication preserves existing staged blobs; new executable files are staged. The original index is retained with the recovery copies.".into());
        }
        self.details.push("Use Repository → History & Recovery → File Backups & Restore to recover completed copies. Interrupted preparations remain on disk for inspection.".into());
        self.observe("Refresh changes", report.status);
    }
    fn file_edit(&mut self, report: FileEditReport) {
        let command = self
            .observe("Apply file operation", report.result)
            .is_some();
        let verified = self
            .observe("Verify working file", report.verification)
            .is_some();
        self.command_succeeded = command && report.applied;
        self.recovery_pending = !verified || (report.applied && !command);
        self.summary = if command && report.applied && verified {
            "Working file updated and verified; staging unchanged."
        } else if report.applied {
            "File operation changed the path but needs review; recovery copies retained."
        } else {
            "File operation did not complete. Inspect the current file before retrying."
        }
        .into();
        if let Some(directory) = report.recovery_directory {
            self.details.push(format!("Recovery copies: {}. Open Repository → History & Recovery → File Backups & Restore to recover a completed copy. Incomplete preparations remain on disk for inspection.",directory.display()));
        }
        self.observe("Refresh changes", report.status);
    }
    fn worktree(&mut self, report: WorktreeWriteReport) {
        self.command(report.command);
        self.details
            .push(format!("Reviewed folder: {}", report.destination.display()));
        if let Some(f) = report.failure {
            self.details
                .insert(0, format!("[{}] {}", f.code(), f.guidance()));
        }
        if let Some(entries) = self.observe("Observe worktrees", report.observed) {
            let present = entries.iter().any(|e| e.path == report.destination);
            self.details.push(format!(
                "Reviewed folder registration: {}. Folder currently {}.",
                if present { "present" } else { "absent" },
                if report.destination.exists() {
                    "exists"
                } else {
                    "absent"
                }
            ));
        }
        let verified = self
            .observe("Verify worktree result", report.verification)
            .is_some();
        if let Some(path) = report.retained_workspace {
            self.details.push(format!("Retained preparation folder: {}. Inspect it and the worktree registration with Git; its checkout may already have moved to the reviewed folder. No recursive cleanup or retry was attempted.",path.display()));
            self.retained_workspace = Some(path);
        }
        if let Some(reference) = report.recovery_reference {
            self.details.push(format!("Committed tip retained at {reference}. This is not a backup of uncommitted files. The original branch is not deleted."));
        }
        if let Some(path) = report.retained_metadata {
            self.details.push(format!("The working folder is gone, but Git metadata remains at {}. Inspect it with Git before retrying; Rocket did not delete this metadata itself.", path.display()));
        }
        if let Some(path) = report.metadata_backup {
            self.details.push(format!("Retained Git metadata: {}. Inspect this recovery data and the current registration before retrying. It is not a backup of working files, and is never restored automatically.", path.display()));
        }
        if self.command_succeeded && verified {
            self.summary = "Worktree operation completed and verified.".into();
        } else {
            self.recovery_pending = true;
            self.summary =
                "Worktree operation needs review. Inspect the observed folder before retrying."
                    .into();
            if verified {
                self.details.push("The observed worktree already matches the reviewed result, although completion was interrupted. Do not repeat it blindly.".into());
            }
        }
    }
    fn lfs(&mut self, report: LfsWriteReport, action: &LfsAction) {
        let completed = self.observe("Apply LFS operation", report.result).is_some();
        let verified = self
            .observe("Verify LFS result", report.verification)
            .is_some();
        self.observe("Refresh LFS", report.observed);
        self.command_succeeded = completed && verified;
        self.recovery_pending = !completed || !verified;
        self.summary = if completed && verified { "LFS operation completed and verified; staging unchanged." } else { "LFS operation needs review. Inspect the refreshed state before retrying; cancellation is not rollback." }.into();
        if let Some(upload) = report.upload_outcome {
            self.summary = match upload {
                LfsUploadOutcome::NotStarted => "LFS upload was not started. Review the reported error before retrying.",
                LfsUploadOutcome::ConfirmedByHelper if verified => "The LFS helper confirmed the reviewed objects are available remotely; repository state is unchanged.",
                LfsUploadOutcome::ConfirmedByHelper => "The LFS helper confirmed the upload, but local verification needs review.",
                LfsUploadOutcome::Unknown => "LFS upload was interrupted or failed. Remote availability is uncertain; some objects may already have arrived.",
            }.into();
            self.details.push(format!("Upload outcome: {}. Git branches were not published. Publish the branch separately; an upload covers only the reviewed commit's file tree, not older history.", upload.code()));
            return;
        }
        let phase = if matches!(action, LfsAction::Track { .. } | LfsAction::Untrack { .. }) {
            match report.completed_steps {
                0 => {
                    "Attributes were not moved or published. Any prepared recovery data is retained."
                }
                1 => {
                    "Original attributes retained in recovery; replacement was not published. Inspect .gitattributes before retrying."
                }
                _ => {
                    "Root .gitattributes saved. Stage it and intended file conversions separately; existing pointers and history were not converted."
                }
            }
        } else if matches!(action, LfsAction::Checkout(_)) {
            match report.completed_steps {
                0 => "Working file was not moved; no prepared content was published.",
                1 => "Original pointer retained; prepared contents have not been published.",
                _ => "Contents published; the original pointer is retained for recovery.",
            }
        } else if report.completed_steps > 0 {
            "Download finished. Review verification and refresh results below."
        } else {
            "Download did not complete; any partial cache data is retained."
        };
        self.details.push(phase.into());
        if let Some(path) = report.recovery_directory {
            self.details.push(format!(
                "Recovery data: {}. Read RECOVERY.md before restoring anything.",
                path.display()
            ));
            self.retained_workspace = Some(path);
        }
    }
    fn submodule(&mut self, report: SubmoduleWriteReport) {
        let completed = self
            .observe("Run submodule checkout", report.result)
            .is_some();
        let verified = self
            .observe("Verify submodule checkout", report.verification)
            .is_some();
        self.observe("Refresh submodules", report.observed);
        self.command_succeeded = completed && verified;
        self.recovery_pending = !self.command_succeeded;
        self.summary = if self.command_succeeded { "Submodule checkout completed and verified; parent staging unchanged." } else { "Submodule operation needs review. Inspect the refreshed checkout before retrying; cancellation is not rollback." }.into();
        if let Some(reference) = report.recovery_reference {
            self.details.push(format!(
                "Previous submodule commit retained at {reference} inside the submodule."
            ));
        }
        if let Some(path) = report.retained_directory {
            self.details.push(format!("Preparation data retained at {}. Existing files were not replaced; no automatic cleanup was performed.",path.display()));
        }
    }
    fn submodule_action(&mut self, report: SubmoduleActionReport) {
        let completed = self
            .observe("Apply submodule action", report.result)
            .is_some();
        // A stopped command is not expected to match the intended final state.
        // Keep its actual error prominent instead of adding a redundant mismatch.
        let verified = completed
            && self
                .observe("Verify submodule action", report.verification)
                .is_some();
        self.observe("Refresh submodules", report.observed);
        self.command_succeeded = completed && verified;
        self.recovery_pending = !self.command_succeeded;
        self.summary = if self.command_succeeded {
            "Submodule action completed and verified."
        } else if report.completed_steps.is_empty() {
            "Submodule action stopped before publication was confirmed. Check the error and retained preparation before retrying."
        } else {
            "Submodule action needs review after completed steps. Inspect the refreshed state and recovery data; cancellation is not rollback."
        }.into();
        if !report.completed_steps.is_empty() {
            self.details
                .push(format!("Completed: {}.", report.completed_steps.join(", ")));
        }
        if let Some(path) = report.recovery_directory {
            self.details.push(format!("Recovery data: {}. Read RECOVERY.md (and MOVE-RECOVERY.md for moves) and compare newer edits before restoring anything. Removal retains the checkout here; no automatic cleanup runs.", path.display()));
        }
    }
    pub(crate) fn tag(&mut self, report: TagWriteReport) {
        self.command(report.result);
        if let Some(observed) = self.observe("Observe local tag", report.observed) {
            self.details.push(match observed {
                Some(tag) => format!(
                    "Observed local tag object: {} ({})",
                    tag.id, tag.object_type
                ),
                None => "Observed local tag: absent.".into(),
            });
        }
        let verified = self
            .observe("Verify tag result", report.verification)
            .is_some();
        if let Some(reference) = report.recovery_reference {
            self.details.push(format!("Exact deleted object retained at {reference}. Recover it with Git update-ref using this object; creating a new annotated tag would not preserve the original annotation/signature."));
        }
        if let Some(failure) = report.failure {
            self.details
                .push(format!("[{}] {}", failure.code(), failure.guidance()));
        }
        if self.command_succeeded && verified {
            self.summary =
                "Local tag operation completed and verified; remote tags unchanged.".into();
        } else {
            self.summary =
                "Tag operation needs review. Inspect the observed result before retrying.".into();
            self.recovery_pending = true;
            if verified {
                self.details.push("The tag already matches the reviewed result, although Git did not confirm completion. Do not repeat the operation blindly.".into());
            }
        }
    }
    fn atomic_push(&mut self, report: AtomicPushReport, p: &AtomicPushPreview) {
        self.command(report.result);
        let observed = self.observe("Observe remote branches", report.observed);
        let verified = self
            .observe("Verify atomic result", report.verification)
            .is_some();
        self.command_succeeded &= verified && !report.error_diagnostics;
        self.summary=if self.command_succeeded{"Atomic push completed and all selected tips verified."}else{match report.remote_state{AtomicPushRemoteState::Published=>"Every remote tip matches the review, but Git did not confirm completion. Do not retry blindly.",AtomicPushRemoteState::Unchanged=>"Selected remote branches are unchanged; this transaction was not published.",AtomicPushRemoteState::Different=>"The remote differs from the reviewed or requested state. Inspect every branch before retrying.",AtomicPushRemoteState::Unknown=>"Remote outcome is unknown. Inspect the destination before retrying."}}.into();
        if let Some(ids) = observed {
            for (e, id) in p.entries().iter().zip(ids) {
                self.details.push(format!(
                    "{}: {}",
                    String::from_utf8_lossy(&e.selection.remote_branch),
                    id.map_or("absent".into(), |id| id.to_string())
                ));
            }
        }
        if report.error_diagnostics {
            self.details
                .push("Git reported additional errors; inspect the local output.".into())
        }
    }
    fn ignore(&mut self, report: IgnoreReport) {
        self.command(report.result.map(|()| WriteOutput {
            stdout: vec![],
            stderr: vec![],
        }));
        self.observe("Refresh ignored files", report.status);
        if let Some(path) = report.recovery {
            self.details
                .push(format!("Recovery copies: {}", path.display()));
        }
        self.summary=if self.command_succeeded{".gitignore saved and verified. Nothing was staged or untracked."}else if report.published{"The file was published, but verification needs review. Inspect current rules and recovery copies."}else if report.restored{"Save interrupted. The original rules were restored."}else{"Save did not complete. Inspect current rules and any recovery copies before retrying."}.into();
    }

    fn force_push(&mut self, report: ForcePushReport, preview: &ForcePushPreview) {
        self.command(report.result);
        let observed = self.observe("Observe remote branch", report.observed);
        let verified = self
            .observe("Verify remote branch", report.verification)
            .is_some();
        self.command_succeeded &= verified && !report.error_diagnostics;
        self.summary = match &observed {
            Some(Some(id)) if id == preview.source() && self.command_succeeded => "Force-with-lease push completed and verified.",
            Some(Some(id)) if id == preview.source() => "The remote already matches the reviewed replacement, but completion needs inspection. Do not repeat this push blindly.",
            Some(Some(id)) if id == preview.expected() => "The remote still matches the reviewed original tip. This update is not confirmed.",
            Some(Some(_)) => "The remote branch has a different tip. Inspect the latest history and prepare a new review.",
            Some(None) => "The remote branch is now absent. Inspect the remote before taking another action.",
            None => "Remote state is unknown. Inspect the destination before retrying; an interrupted push may already have changed it.",
        }.into();
        if let Some(Some(id)) = observed {
            self.details.push(format!("Observed remote tip: {id}."));
        }
        if report.error_diagnostics {
            self.details
                .push("Git also reported an error; inspect the optional local Git output.".into());
        }
    }
    pub(crate) fn remote_tag(&mut self, report: RemoteTagWriteReport) {
        self.command(report.result);
        match self.observe("Observe remote tag", report.observed) {
            Some(Some(id)) => self
                .details
                .push(format!("Observed remote tag object: {id}.")),
            Some(None) => self.details.push("Observed remote tag: absent.".into()),
            None => self
                .details
                .push("Remote state is unknown. Refresh before deciding whether to retry.".into()),
        }
        let verified = self
            .observe("Verify remote tag", report.verification)
            .is_some();
        self.command_succeeded &= verified && !report.error_diagnostics;
        self.recovery_pending = !self.command_succeeded;
        self.summary = if self.command_succeeded {
            "Remote tag change completed and verified."
        } else if verified {
            "The remote already matches the reviewed result, but completion needs inspection. Do not repeat the operation blindly."
        } else {
            "Remote tag change was not confirmed. Inspect the observed state and errors before retrying."
        }.into();
        if report.error_diagnostics {
            self.details
                .push("Git also reported an error; inspect the optional local Git output.".into());
        }
    }
    fn conflict(&mut self, report: ConflictSaveReport, stage_requested: bool) {
        self.command_succeeded = self.observe("Save resolution", report.result).is_some();
        self.summary = if report.staged {
            "Resolution saved and staged; no commit created"
        } else if report.saved && self.command_succeeded {
            "File saved; conflict remains unstaged"
        } else if report.saved && stage_requested {
            "File saved; staging not confirmed. Review refreshed Changes before retrying"
        } else if report.saved {
            "File replacement completed but verification failed; inspect the working file and recovery copies"
        } else {
            "Conflict save not completed; draft retained"
        }
        .into();
        self.conflict = Some(ConflictOutcome {
            saved: report.saved,
            staged: report.staged,
            preview: report.preview.map(Arc::new),
        });
        if let Some(directory) = report.recovery_directory {
            self.details.push(format!("Recovery copies retained at {}. If publication failed, inspect the working file and these copies before retrying; a failed replacement may have moved the original.", directory.display()));
        }
        self.observe("Refresh changes", report.status);
        self.observe("Refresh operation", report.state);
    }
    fn whole_conflict(&mut self, report: WholeConflictReport, keep: ConflictKeep) {
        self.command_succeeded = self.observe("Keep version", report.result).is_some();
        let side = match keep {
            ConflictKeep::Ours => "ours",
            ConflictKeep::Theirs => "theirs",
            ConflictKeep::Deletion => "the deletion",
        };
        self.summary = match (report.staged, report.file_change) {
            (true, Some(WholeFileChange::Replaced)) => {
                format!("Kept {side}: working file replaced and staged; no commit created")
            }
            (true, Some(WholeFileChange::Created)) => {
                format!("Kept {side}: working file created and staged; no commit created")
            }
            (true, Some(WholeFileChange::Removed)) => "Kept the deletion: working file moved to recovery and the deletion staged; no commit created".into(),
            (true, None) if keep == ConflictKeep::Deletion => {
                "Kept the deletion: staged; no working path was removed".into()
            }
            (true, None) => {
                format!("Kept {side}: reviewed index entry staged; working contents left unchanged")
            }
            (false, Some(_)) => "Working file changed; staging not confirmed. Review refreshed Changes before retrying".into(),
            (false, None) => "Whole-file resolution not completed; nothing was staged".into(),
        };
        self.whole_conflict = Some(WholeConflictOutcome {
            keep,
            file_change: report.file_change,
            staged: report.staged,
        });
        if let Some(directory) = report.recovery_directory {
            self.details.push(format!("Recovery copies retained at {}. Check the resolution, then remove only copies you no longer need. If publication failed, inspect the working file and these copies before retrying.", directory.display()));
        }
        self.observe("Refresh changes", report.status);
        self.observe("Refresh operation", report.state);
    }
    pub(crate) fn observe<T>(&mut self, phase: &str, result: Result<T>) -> Option<T> {
        match result {
            Ok(value) => Some(value),
            Err(error) => {
                if let Error::SecretProtection(failure) = &error {
                    self.summary = failure.summary();
                }
                self.details
                    .push(format!("{phase}: {}", user_error(&error)));
                self.keep_raw(&error);
                None
            }
        }
    }
    fn keep_raw(&mut self, error: &Error) {
        if let Error::Git { stdout, stderr, .. } = error {
            self.raw_output.push_str(&String::from_utf8_lossy(stdout));
            self.raw_output.push_str(&String::from_utf8_lossy(stderr));
        }
    }
    fn command(&mut self, result: Result<WriteOutput>) {
        if let Some(output) = self.observe("Command", result) {
            self.command_succeeded = true;
            self.keep_output(output);
        }
    }
    fn keep_output(&mut self, output: WriteOutput) {
        self.raw_output
            .push_str(&String::from_utf8_lossy(&output.stdout));
        self.raw_output
            .push_str(&String::from_utf8_lossy(&output.stderr));
    }
    fn write(&mut self, report: WriteReport) {
        self.command(report.result);
        self.observe("Refresh changes", report.status);
    }
    pub(crate) fn branch(&mut self, report: BranchWriteReport) {
        self.write(report.write);
        self.observe("Refresh branches", report.branches);
    }
    fn stash(&mut self, report: StashWriteReport) {
        if report.write.operation == WriteOperation::ApplyStash
            && report
                .write
                .status
                .as_ref()
                .is_ok_and(|s| s.entries.iter().any(|e| e.kind == EntryKind::Unmerged))
        {
            self.recovery_pending = true;
            self.summary = "Stash apply left conflicts; saved stash retained".into();
            self.details.push("Review Changes, resolve conflicts, and stage the resolved files. There is no stash abort operation. The saved stash remains available; do not apply it again over unresolved changes.".into());
        } else if report.write.result.is_err() {
            self.details.push("Inspect the refreshed changes and stash list before retrying. Applying a stash never removes its saved entry.".into());
        }
        self.write(report.write);
        self.observe("Refresh stashes", report.stashes);
    }
    fn remote(&mut self, report: RemoteWriteReport) {
        self.command(report.result);
        self.observe("Refresh remotes", report.remotes);
        self.observe("Refresh branches", report.branches);
        if let Some(status) = report.status {
            self.observe("Refresh changes", status);
        }
    }
    pub(crate) fn merge(&mut self, report: MergeWriteReport) {
        let unfinished = report.state.as_ref().is_ok_and(|s| !s.is_idle());
        let unreadable =
            report.state.is_err() || report.write.status.is_err() || report.branches.is_err();
        self.recovery_pending = unfinished || unreadable;
        if let Some(failure) = report.failure {
            self.details
                .push(format!("[{}] {}", failure.code(), failure.guidance()));
        }
        if report.failure == Some(MergeFailure::AlreadyCommitted) {
            self.summary = "Merge commit exists; clear the leftover merge state in Changes. HEAD and local files will be kept.".into();
        } else if report.failure == Some(MergeFailure::Conflicts) {
            self.summary =
                "Merge paused with conflicts; resolve in Changes, then Continue or Abort.".into();
        } else if unfinished {
            self.summary =
                "Merge is unfinished. Review Changes and Continue or Abort; do not start it again."
                    .into();
        } else if unreadable {
            self.summary = "Merge outcome needs review because repository refresh was incomplete. Refresh before another operation.".into();
        } else if report.write.result.is_err() {
            self.details.push("The attempted operation may have changed files or HEAD. Inspect the refreshed state before retrying; cancellation is not rollback.".into());
        }
        self.write(report.write);
        self.observe("Refresh operation", report.state);
        self.observe("Refresh branches", report.branches);
    }
    /// What `--update-refs` did to the other branches, from the refreshed branch list.
    fn update_refs(&mut self, preview: &RebasePreview, report: &RebaseWriteReport) {
        if !preview.update_refs() {
            return;
        }
        let finished = report.state.as_ref().is_ok_and(|s| !s.rebase);
        let Ok(branches) = &report.branches else {
            return;
        };
        for dependent in preview.dependents() {
            let name = crate::style::short_ref(&dependent.reference);
            let now = branches
                .iter()
                .find(|b| b.reference == dependent.reference)
                .map(|b| &b.target);
            self.details.push(match (&dependent.checked_out_at, now) {
                (Some(path), _) => format!(
                    "{name} stays at {}: it is checked out in {}.",
                    &dependent.target.as_str()[..9],
                    crate::views::display_path(path)
                ),
                (None, _) if !finished => {
                    format!("{name} moves when the rebase finishes (--update-refs).")
                }
                (None, Some(now)) if *now != dependent.target => format!(
                    "{name} moved with the rebase: {} → {}.",
                    &dependent.target.as_str()[..9],
                    &now.as_str()[..9]
                ),
                (None, Some(_)) => {
                    format!("{name} did not move; it still points at its old commit.")
                }
                (None, None) => format!("{name} no longer exists."),
            });
        }
    }
    fn rebase(&mut self, report: RebaseWriteReport) {
        self.recovery_pending = report.state.as_ref().is_ok_and(|s| s.rebase);
        let paused = paused_summary("commit", &report);
        let branch_moved = report.failure == Some(RebaseFailure::RecoveryBranchMoved);
        // Git waiting for you, as opposed to a step that failed where it stood.
        let waiting = report.stop.is_some()
            || matches!(
                report.failure,
                Some(RebaseFailure::ConflictsRemain | RebaseFailure::Paused | RebaseFailure::EmptyChange | RebaseFailure::CommandStopped)
            );
        if let Some(failure) = report.failure {
            self.details
                .push(format!("[{}] {}", failure.code(), failure.guidance()));
        }
        self.details.push(format!(
            "Original branch tip retained at {}. This is a local operation; nothing was pushed.",
            report.recovery_reference
        ));
        self.write(report.write);
        self.observe("Refresh operation", report.state);
        self.observe("Refresh branches", report.branches);
        self.observe("Clean saved rebase plan", report.journal_cleanup);
        if branch_moved {
            self.summary =
                "Rebase branch changed; inspect history and finish recovery with Git.".into();
        } else if let Some(stop) = &report.stop {
            self.summary = match stop {
                RebaseStop::Edit(commit) => format!(
                    "Stopped to edit {}: stage changes and Continue to amend them into it, or Continue unchanged.",
                    &commit.as_str()[..9]
                ),
                RebaseStop::Break => {
                    "Stopped at a planned break. Inspect the result, then Continue the plan.".into()
                }
                RebaseStop::Command(_) => {
                    "Stopped after a planned command that failed or left changes. Review Changes, then Continue, Skip or Abort.".into()
                }
            };
        } else if self.recovery_pending && waiting {
            self.summary = paused;
        } else if self.recovery_pending {
            self.summary = "Rebase remains in progress. Review Changes and recovery options.".into();
        } else if self.command_succeeded {
            self.summary = "Rebase finished; review the refreshed branch history.".into();
        }
        self.paused = self.recovery_pending && waiting;
    }
    /// What finished, once a reviewed cherry-pick or revert is done. Counts and the
    /// branch only: commit subjects stay out of what's logged.
    fn commit_result(&mut self, preview: &CommitPreview) {
        let request = preview.request();
        let (word, done) = match request.operation {
            CommitOperation::Amend => return,
            CommitOperation::CherryPick => ("Cherry-pick", "Cherry-picked"),
            CommitOperation::Revert => ("Revert", "Reverted"),
        };
        // A failed Abort leaves Git where it was, which isn't Git waiting for you.
        if request.recovery == Some(CommitRecovery::Abort) && self.recovery_pending {
            self.paused = false;
            self.summary = format!("{word} remains in progress. Review Changes and recovery options.");
        }
        if !self.command_succeeded || self.recovery_pending {
            return;
        }
        let branch = match preview.head() {
            Head::Branch { name, .. } => crate::style::display(name),
            _ => "your checkout".to_owned(),
        };
        self.summary = match request.recovery {
            None if request.targets.len() == 1 => format!("{done} 1 commit on {branch}"),
            None => format!("{done} {} commits on {branch}", request.targets.len()),
            Some(CommitRecovery::Abort) if !preview.is_sequence() => format!("{word} aborted. {branch} didn't move"),
            Some(CommitRecovery::Abort) if !preview.rewinds() => format!("{word} ended. {branch} stayed where it was, because it moved by hand"),
            Some(CommitRecovery::Abort) => match preview.original_commit() {
                Some(commit) => format!("{word} aborted. {branch} is back at {}", &commit.id.as_str()[..7]),
                None => format!("{word} aborted"),
            },
            Some(_) => format!("{word} finished on {branch}"),
        };
    }
    /// What finished, by name, once a reviewed rebase is done.
    fn rebase_result(&mut self, preview: &RebasePreview) {
        if preview.action() == RebaseAction::Abort && self.recovery_pending {
            self.paused = false;
            self.summary = "Rebase remains in progress. Review Changes and recovery options.".into();
        }
        if !self.command_succeeded || self.recovery_pending {
            return;
        }
        let branch = String::from_utf8_lossy(preview.branch());
        let short = |id: &ObjectId| id.as_str()[..7].to_owned();
        self.summary = match preview.action() {
            RebaseAction::Abort => format!("Rebase aborted. {branch} is back at {}.", short(preview.original())),
            _ => match (preview.target(), preview.onto()) {
                ("", Some(onto)) => format!("{branch} is on top of {}.", short(onto)),
                (target, Some(_)) => format!("{branch} is on top of {}.", crate::rebase::target_name(target)),
                (_, None) => format!("{branch} is rewritten from its first commit."),
            },
        };
    }
    fn mail_import(&mut self, report: MailImportReport, preview: &MailImportPreview) {
        self.recovery_pending=report.state.as_ref().is_ok_and(|s|s.mail_import);
        let conflicts=report.write.status.as_ref().map_or(0,|s|s.entries.iter().filter(|e|e.kind==EntryKind::Unmerged).count());
        let stopped=report.state.as_ref().ok().and_then(|s|s.step.clone());
        self.details.push(format!("Original tip retained at {}. Nothing was pushed.",report.recovery_reference));
        if let Some(path)=report.backup{self.details.push(format!("Tracked resolution copies and the staged patch are retained in {}. Read manifest.log before restoring any file.",path.display()));}
        self.write(report.write);self.observe("Refresh import state",report.state);self.observe("Record import recovery",report.journal_result);
        let branch=preview.branch_display();
        // Git waiting at a patch, as opposed to a step that failed where it stood.
        let moved_on=stopped.as_ref().is_some_and(|s|s.at>preview.current());
        let waiting=preview.action()==MailImportAction::Start||(preview.action()!=MailImportAction::Abort&&(conflicts>0||moved_on));
        if self.recovery_pending&&waiting {
            self.paused=true;
            self.summary=paused_line("patch",stopped.as_ref(),conflicts);
        } else if self.recovery_pending {
            self.summary=stopped.as_ref().map_or_else(||"Git didn't finish this step.".to_owned(),|s|format!("Git didn't finish this step. Still at patch {} of {}.",s.at,s.total));
        } else if self.command_succeeded {
            self.summary=match preview.action() {
                MailImportAction::Start=>match preview.entries().len() {1=>format!("1 commit imported on {branch}"),n=>format!("{n} commits imported on {branch}")},
                MailImportAction::Abort=>format!("Import aborted. {branch} is back at {}.",&preview.original().as_str()[..7]),
                _=>format!("Import finished on {branch}"),
            };
        }
    }
    fn recovery(&mut self, report: RecoveryWriteReport, preview: &RecoveryPreview) {
        let branch = report.write.operation == WriteOperation::CreateBranch;
        if let Ok(status) = &report.write.status {
            match &status.head {
                Head::Branch { target, .. } | Head::Detached { target } => self
                    .details
                    .push(format!("Observed HEAD after the attempt: {target}")),
                Head::Unborn { .. } => self
                    .details
                    .push("Observed HEAD has no commit after the attempt.".into()),
            }
        }
        self.write(report.write);
        self.observe("Refresh branches", report.branches);
        let verified = self
            .observe("Verify recovery result", report.verification)
            .is_some();
        if verified && let RecoveryAction::Branch(name) = preview.action() {
            self.details.push(format!(
                "Verified recovery branch {name} at {}. The original checkout and tracked state match the preview.",
                preview.target().id
            ));
        }
        if let Some(reference) = report.recovery_reference {
            self.details.push(format!("Original committed tip retained at {reference}. Create a recovery branch there to inspect it; this does not restore uncommitted edits."));
        }
        if let Some(failure) = report.failure {
            self.details
                .push(format!("[{}] {}", failure.code(), failure.guidance()));
        }
        if self.command_succeeded && verified {
            self.summary = if branch {
                "Recovery branch created. Its target, original checkout and tracked state were verified."
            } else {
                "Recovery command completed and its resulting refs/index were verified."
            }.into();
        } else {
            self.summary = if branch && verified {
                "The recovery branch exists at the reviewed commit, although Git did not confirm completion. Inspect that branch instead of retrying creation."
            } else {
                "Recovery needs review. Inspect the refreshed history and files before retrying; cancellation is not rollback."
            }.into();
            self.recovery_pending = true;
            if verified && !branch {
                self.details.push("The observed refs/index already match the reviewed result, although Git did not confirm completion. Do not repeat the operation blindly.".into());
            }
        }
    }
    fn bisect(&mut self, report: BisectWriteReport) {
        let inspection_failed =
            report.session.is_err() || report.state.is_err() || report.status.is_err();
        self.recovery_pending =
            report.session.as_ref().is_ok_and(|s| s.is_some()) || inspection_failed;
        if let Some(failure) = report.failure {
            self.details
                .push(format!("[{}] {}", failure.code(), failure.guidance()));
        }
        let observed = report.session.as_ref().ok().and_then(|s| s.as_ref());
        let phase = observed.map(|s| s.phase);
        if let Some(s) = observed {
            self.details.push(format!(
                "{} candidates remain. Original committed checkout retained at {}.",
                s.candidates.len(),
                s.recovery_reference
            ));
        }
        self.command(report.result);
        self.observe("Refresh changes", report.status);
        self.observe("Refresh operation", report.state);
        self.observe("Inspect bisect session", report.session);
        self.summary = if inspection_failed {
            "Bisect outcome needs review because repository refresh was incomplete. Refresh the session before another action."
        } else if !self.command_succeeded {
            "Bisect step needs review. Refresh the session; a decision or checkout may already be recorded. Cancellation is not rollback."
        } else { match phase {
            Some(BisectPhase::Found) => "First bad commit found. Inspect it, then restore the original checkout.",
            Some(BisectPhase::Inconclusive) => "Bisect is inconclusive: skipped revisions leave multiple possible first bad commits.",
            Some(BisectPhase::Testing) => "Revision checked out. Test it, then review Good, Bad or Skip.",
            Some(_) => "Bisect requires recovery. Refresh and inspect the session.",
            None => "Original checkout restored and bisect state cleared.",
        }}.into();
    }
    fn commit_action(&mut self, report: CommitWriteReport) {
        self.recovery_pending = report.state.as_ref().is_ok_and(|state| !state.is_idle());
        let conflicts = report.write.status.as_ref().map_or(0, |s| s.entries.iter().filter(|e| e.kind == EntryKind::Unmerged).count());
        let stopped = report.state.as_ref().ok().and_then(|s| s.step.as_ref().filter(|step| step.total > 1).cloned());
        // Git waiting for you, as opposed to a step that failed where it stood.
        let waiting = matches!(report.failure, Some(CommitFailure::ConflictsRemain | CommitFailure::EmptyChange));
        if let Some(failure) = report.failure {
            self.details
                .push(format!("[{}] {}", failure.code(), failure.guidance()));
        }
        if let Some(reference) = report.recovery_reference {
            self.details.push(format!("Commit retained at {reference}. You can create a branch at this reference to inspect or recover it."));
        }
        self.write(report.write);
        self.observe("Refresh operation", report.state);
        self.observe("Refresh branches", report.branches);
        self.observe("Clean sequence review", report.journal_cleanup);
        self.paused = self.recovery_pending && waiting;
        if self.paused {
            self.summary = paused_line("commit", stopped.as_ref(), conflicts);
        }
        if self.recovery_pending {
            self.details.push("Operation remains in progress. Review Changes, resolve and stage conflicts, then Continue; use Skip or Abort only after reviewing the discarded changes.".into());
        }
    }
}
impl Backend {
    pub(crate) fn integration<T>(&self, operation: &str, run: impl FnOnce(&GitClient) -> Result<T>) -> Result<T> {
        self.request(operation, false, |client| run(&client))
    }
    pub fn bisect_session(
        &self,
        path: &Path,
        cancel: &CancellationToken,
    ) -> Result<Option<BisectSession>> {
        self.request("bisect", false, |git| {
            git.open(path, cancel)?.bisect_session(cancel)
        })
    }
    pub fn bisect_preview(
        &self,
        path: &Path,
        action: BisectAction,
        good: &str,
        bad: &str,
        cancel: &CancellationToken,
    ) -> Result<BisectPreview> {
        self.request("bisect-preview", false, |git| {
            git.open(path, cancel)?
                .preview_bisect(action, good, bad, cancel)
        })
    }
    pub fn bisect_auto_preview(&self,path:&Path,good:&str,bad:&str,command:&BisectTestCommand,c:&CancellationToken)->Result<BisectAutoPreview>{
        self.request("bisect-auto-preview",false,|git|git.open(path,c)?.preview_bisect_auto(good,bad,command,c))
    }
    pub fn bisect_tests(&self,path:&Path,c:&CancellationToken)->Result<Vec<BisectTestResult>>{
        self.request("bisect-tests",false,|git|git.open(path,c)?.bisect_test_history(c))
    }
    pub fn atomic_push_preview(&self,path:&Path,remote:&[u8],selection:&[AtomicPushSelection],c:&CancellationToken)->Result<AtomicPushPreview>{self.request("atomic-push-preview",true,|git|git.open(path,c)?.preview_atomic_push(remote,selection,c))}
    pub fn workflow_preview(&self,path:&Path,request:crate::workflows::Request,c:&CancellationToken)->Result<crate::workflows::Review>{
        use crate::workflows::{Request,Review};
        self.request("workflow-preview",true,|git|{let repo=git.open(path,c)?;Ok(match request{
            Request::StashApply(review, selected)=>Review::StashApply(repo.preview_stash_apply(&review,&selected,c)?),
            Request::StashSnapshot(message,untracked)=>Review::StashSnapshot(repo.preview_stash_snapshot(&message,untracked,c)?),
            Request::IgnoreTracked(paths)=>Review::IgnoreTracked(repo.preview_ignore_tracked(&paths,c)?),
            Request::FastForward=>Review::FastForward(repo.preview_fast_forward_branches(c)?),
            Request::RenameRemote(name,new)=>{let remote=repo.remotes(c)?.into_iter().find(|r|r.name==name).ok_or_else(||Error::InvalidInput("Remote changed; reopen the review.".into()))?;Review::RenameRemote(repo.preview_remote_rename(&remote,new.as_bytes(),c)?)},
            Request::StashControl(action)=>Review::StashControl(repo.preview_stash_control(action,c)?),
            Request::SubmoduleBatch(paths,root)=>Review::SubmoduleBatch(repo.preview_submodule_batch(&paths,root.as_deref(),c)?),
            Request::Branch(kind,base,options)=>Review::Branch(Box::new(repo.preview_branch_workflow_with_options(kind,&base,options,c)?)),
            Request::Prune(remote)=>Review::Prune(repo.preview_remote_cleanup(&remote,c)?),
            Request::Delete(remote,branch)=>Review::Delete(repo.preview_remote_branch_delete(&remote,&branch,c)?),
            Request::Rename(entry,name)=>Review::Rename(repo.preview_stash_rename(&entry,&name,c)?),
            Request::Saved(action)=>Review::Saved(repo.preview_saved_work(action,c)?),
            Request::CommitPush(message,remote,destination)=>Review::CommitPush(repo.preview_commit_push(&message,&remote,&destination,c)?),
            Request::Lfs(remote,scope)=>Review::Lfs(repo.preview_lfs_prune(&remote,scope.as_ref(),c)?),
        })})
    }
    pub fn stash_selection_preview(&self,path:&Path,review:&ChangeReview,selected:&[usize],message:&str,c:&CancellationToken)->Result<StashSelectionPreview>{
        self.request("stash-selection-preview",false,|git|git.open(path,c)?.preview_stash_selection(review,selected,message,c))
    }
    pub fn merge_preview_options(&self,path:&Path,target:&str,mode:MergeMode,unrelated:bool,c:&CancellationToken)->Result<MergePreview>{
        self.request("merge-preview",false,|git|git.open(path,c)?.preview_merge_allow_unrelated(target,mode,unrelated,c))
    }
    pub fn ignore_snapshot(&self,path:&Path,c:&CancellationToken)->Result<IgnoreSnapshot>{self.request("ignore-read",false,|git|git.open(path,c)?.ignore_snapshot(c))}
    pub fn ignore_preview(&self,path:&Path,s:&IgnoreSnapshot,text:&str,c:&CancellationToken)->Result<IgnorePreview>{self.request("ignore-preview",false,|git|git.open(path,c)?.preview_ignore(s,text,c))}
    pub fn force_push_preview(
        &self,
        path: &Path,
        remote: &[u8],
        local: &[u8],
        destination: &[u8],
        cancel: &CancellationToken,
    ) -> Result<ForcePushPreview> {
        self.request("force-push-preview", true, |git| {
            git.open(path, cancel)?
                .preview_force_push(remote, local, destination, cancel)
        })
    }
    pub fn pull_preview(
        &self,
        path: &Path,
        fetched: &PullFetched,
        strategy: PullStrategy,
        cancel: &CancellationToken,
    ) -> Result<PullPreview> {
        self.request("pull-preview", false, |git| {
            git.open(path, cancel)?
                .preview_pull(fetched, strategy, cancel)
        })
    }
    pub fn merge_preview(
        &self,
        path: &Path,
        target: &str,
        mode: MergeMode,
        cancel: &CancellationToken,
    ) -> Result<MergePreview> {
        self.request("merge-preview", false, |git| {
            git.open(path, cancel)?.preview_merge(target, mode, cancel)
        })
    }
    pub fn submodule_action_preview(
        &self,
        path: &Path,
        action: &SubmoduleAction,
        fixture_root: Option<&Path>,
        cancel: &CancellationToken,
    ) -> Result<SubmoduleActionPreview> {
        self.request("submodule-action-preview", true, |git| {
            let repo = git.open(path, cancel)?;
            if let Some(root) = fixture_root {
                repo.preview_submodule_action_local(action, root, cancel)
            } else {
                repo.preview_submodule_action(action, cancel)
            }
        })
    }
    pub fn worktrees(&self, path: &Path, cancel: &CancellationToken) -> Result<Vec<WorktreeEntry>> {
        self.request("worktrees", false, |git| {
            git.open(path, cancel)?.worktrees(cancel)
        })
    }
    pub fn lfs_status(&self, path: &Path, cancel: &CancellationToken) -> Result<LfsStatus> {
        self.request("lfs-status", false, |git| {
            git.open(path, cancel)?.lfs_status(cancel)
        })
    }
    pub fn lfs_locks(
        &self,
        path: &Path,
        remote: &str,
        scope: Option<&LfsSourceScope>,
        cancel: &CancellationToken,
    ) -> Result<LfsLocks> {
        self.request("lfs-locks", true, |git| {
            git.open(path, cancel)?.lfs_locks(remote, scope, cancel)
        })
    }
    pub fn lfs_lock_preview(
        &self,
        path: &Path,
        remote: &str,
        action: LfsLockAction,
        scope: Option<&LfsSourceScope>,
        cancel: &CancellationToken,
    ) -> Result<LfsLockPreview> {
        self.request("lfs-lock-preview", true, |git| {
            git.open(path, cancel)?
                .preview_lfs_lock(remote, action, scope, cancel)
        })
    }
    pub fn lfs_preview(
        &self,
        path: &Path,
        action: LfsAction,
        source_scope: Option<&LfsSourceScope>,
        cancel: &CancellationToken,
    ) -> Result<LfsPreview> {
        self.request("lfs-preview", false, |git| {
            git.open(path, cancel)?
                .preview_lfs_scoped(action, source_scope, cancel)
        })
    }
    pub fn submodules(
        &self,
        path: &Path,
        cancel: &CancellationToken,
    ) -> Result<Vec<SubmoduleEntry>> {
        self.request("submodules", false, |git| {
            git.open(path, cancel)?.submodules(cancel)
        })
    }
    pub fn submodule_preview(
        &self,
        path: &Path,
        entry: &SubmoduleEntry,
        cancel: &CancellationToken,
    ) -> Result<SubmodulePreview> {
        self.request("submodule-preview", false, |git| {
            git.open(path, cancel)?.preview_submodule(entry, cancel)
        })
    }
    pub fn submodule_open(
        &self,
        path: &Path,
        entry: &SubmoduleEntry,
        cancel: &CancellationToken,
    ) -> Result<PathBuf> {
        self.request("submodule-open", false, |git| {
            git.open(path, cancel)?.open_submodule(entry, cancel)
        })
    }
    pub fn worktree_create_preview(
        &self,
        path: &Path,
        destination: &Path,
        checkout: WorktreeCheckout,
        start: &str,
        cancel: &CancellationToken,
    ) -> Result<WorktreePreview> {
        self.request("worktree-preview", false, |git| {
            git.open(path, cancel)?
                .preview_create_worktree(destination, checkout, start, cancel)
        })
    }
    pub fn worktree_remove_preview(
        &self,
        path: &Path,
        entry: &WorktreeEntry,
        cancel: &CancellationToken,
    ) -> Result<WorktreePreview> {
        self.request("worktree-preview", false, |git| {
            git.open(path, cancel)?
                .preview_remove_worktree(entry, cancel)
        })
    }
    pub fn worktree_maintenance_preview(
        &self,
        path: &Path,
        kind: WorktreeMaintenance,
        entry: &WorktreeEntry,
        destination: &Path,
        cancel: &CancellationToken,
    ) -> Result<WorktreePreview> {
        self.request("worktree-preview", false, |git| {
            git.open(path, cancel)?
                .preview_maintain_worktree(kind, entry, destination, cancel)
        })
    }
    pub fn worktree_open(
        &self,
        path: &Path,
        entry: &WorktreeEntry,
        cancel: &CancellationToken,
    ) -> Result<PathBuf> {
        self.request("worktree-open", false, |git| {
            git.open(path, cancel)?.open_worktree(entry, cancel)
        })
    }
    pub fn signing(&self, path: &Path, cancel: &CancellationToken) -> Result<RepositorySigning> {
        self.request("signing-read", false, |git| {
            git.open(path, cancel)?.signing(cancel)
        })
    }
    pub fn verify_signature(
        &self,
        path: &Path,
        target: &SignatureTarget,
        cancel: &CancellationToken,
    ) -> Result<SignatureVerification> {
        self.request("signature-verify", false, |git| {
            git.open(path, cancel)?.verify_signature(target, cancel)
        })
    }
    pub fn create_signed_tag_preview(
        &self,
        path: &Path,
        name: &str,
        target: &str,
        message: &str,
        cancel: &CancellationToken,
    ) -> Result<TagPreview> {
        self.request("tag-preview", false, |git| {
            git.open(path, cancel)?
                .preview_signed_tag(name, target, message, cancel)
        })
    }
    pub fn tags(&self, path: &Path, limit: usize, cancel: &CancellationToken) -> Result<TagPage> {
        self.request("tags", false, |git| {
            git.open(path, cancel)?.tags(limit, cancel)
        })
    }
    pub fn tags_ordered(
        &self,
        path: &Path,
        limit: usize,
        order: TagOrder,
        cancel: &CancellationToken,
    ) -> Result<TagPage> {
        self.request("tags", false, |git| {
            git.open(path, cancel)?.tags_ordered(limit, order, cancel)
        })
    }
    pub fn remote_tags(
        &self,
        path: &Path,
        remote: &[u8],
        cancel: &CancellationToken,
    ) -> Result<RemoteTagSnapshot> {
        self.request("remote-tags", true, |git| {
            git.open(path, cancel)?.remote_tags(remote, cancel)
        })
    }
    pub fn remote_tag_preview(
        &self,
        path: &Path,
        snapshot: &RemoteTagSnapshot,
        name: &[u8],
        action: RemoteTagAction,
        cancel: &CancellationToken,
    ) -> Result<RemoteTagPreview> {
        self.request("remote-tag-preview", true, |git| {
            git.open(path, cancel)?
                .preview_remote_tag(snapshot, name, action, cancel)
        })
    }
    pub fn inspect_tag(
        &self,
        path: &Path,
        entry: &TagEntry,
        cancel: &CancellationToken,
    ) -> Result<TagDetails> {
        self.request("tag-inspect", false, |git| {
            git.open(path, cancel)?.inspect_tag(entry, cancel)
        })
    }
    pub fn create_tag_preview(
        &self,
        path: &Path,
        name: &str,
        target: &str,
        message: Option<&str>,
        cancel: &CancellationToken,
    ) -> Result<TagPreview> {
        self.request("tag-preview", false, |git| {
            git.open(path, cancel)?
                .preview_create_tag(name, target, message, cancel)
        })
    }
    /// Fetch every remote without prompting; see `Repository::fetch_all_quietly`.
    pub fn fetch_all_quietly(
        &self,
        path: &Path,
        tags: FetchTags,
        cancel: &CancellationToken,
    ) -> Result<QuietFetchReport> {
        self.request("auto-fetch", true, |git| {
            git.open(path, cancel)?.fetch_all_quietly(tags, cancel)
        })
    }
    pub fn delete_tags_preview(
        &self,
        path: &Path,
        entries: &[TagEntry],
        cancel: &CancellationToken,
    ) -> Result<TagBatchReview<TagPreview>> {
        self.request("tag-preview", false, |git| {
            git.open(path, cancel)?.preview_delete_tags(entries, cancel)
        })
    }
    pub fn publish_tags_preview(
        &self,
        path: &Path,
        snapshot: &RemoteTagSnapshot,
        names: &[Vec<u8>],
        cancel: &CancellationToken,
    ) -> Result<TagBatchReview<RemoteTagPreview>> {
        self.request("remote-tag-preview", true, |git| {
            git.open(path, cancel)?
                .preview_publish_tags(snapshot, names, cancel)
        })
    }
    pub fn delete_tag_preview(
        &self,
        path: &Path,
        entry: &TagEntry,
        cancel: &CancellationToken,
    ) -> Result<TagPreview> {
        self.request("tag-preview", false, |git| {
            git.open(path, cancel)?.preview_delete_tag(entry, cancel)
        })
    }
    pub fn file_edit_preview(
        &self,
        root: &Path,
        path: &RepoPath,
        source: FileEditSource,
        cancel: &CancellationToken,
    ) -> Result<FileEditPreview> {
        self.request("file-preview", false, |git| {
            git.open(root, cancel)?
                .preview_file_edit(path, source, cancel)
        })
    }
    pub fn patch_import_preview(
        &self,
        root: &Path,
        source: &Path,
        cancel: &CancellationToken,
    ) -> Result<PatchPreview> {
        self.request("patch-preview", false, |git| {
            git.open(root, cancel)?.preview_patch_import(source, cancel)
        })
    }
    /// Repository Statistics: commit activity of HEAD or of all branches.
    pub fn commit_activity(
        &self,
        root: &Path,
        scope: &ActivityScope,
        cancel: &CancellationToken,
    ) -> Result<CommitActivity> {
        self.request("commit-activity", false, |git| {
            git.open(root, cancel)?.commit_activity(scope, cancel)
        })
    }
    /// Repository Statistics: the files at HEAD with their changes since `since`.
    pub fn activity_treemap(
        &self,
        root: &Path,
        since: Option<i64>,
        cancel: &CancellationToken,
    ) -> Result<ActivityTreemap> {
        self.request("activity-treemap", false, |git| {
            git.open(root, cancel)?.activity_treemap(since, cancel)
        })
    }
    /// Review a patch pasted from the clipboard.
    pub fn patch_text_preview(
        &self,
        root: &Path,
        text: &[u8],
        cancel: &CancellationToken,
    ) -> Result<PatchPreview> {
        self.request("patch-preview", false, |git| {
            git.open(root, cancel)?.preview_patch_text(text, cancel)
        })
    }
    pub fn discard_selection_preview(
        &self,
        root: &Path,
        review: &ChangeReview,
        lines: &[usize],
        cancel: &CancellationToken,
    ) -> Result<PatchPreview> {
        self.request("discard-preview", false, |git| {
            git.open(root, cancel)?
                .preview_discard_selection(review, lines, cancel)
        })
    }
    pub fn patch_export_preview(
        &self,
        root: &Path,
        source: PatchExportSource,
        destination: &Path,
        cancel: &CancellationToken,
    ) -> Result<PatchExportPreview> {
        self.request("patch-export-preview", false, |git| {
            git.open(root, cancel)?
                .preview_patch_export(source, destination, cancel)
        })
    }
    pub fn file_backups(&self, root: &Path, cancel: &CancellationToken) -> Result<Vec<FileBackup>> {
        self.request("file-backups", false, |git| {
            git.open(root, cancel)?.file_backups(cancel)
        })
    }
    pub fn patch_series_preview(&self, root: &Path, first: &str, last: &str, destination: &Path, cancel: &CancellationToken) -> Result<PatchSeriesPreview> {
        self.request("patch-series-preview", false, |git| git.open(root, cancel)?.preview_patch_series_between(first, last, destination, cancel))
    }
    pub fn reflog(
        &self,
        path: &Path,
        limit: usize,
        cancel: &CancellationToken,
    ) -> Result<ReflogPage> {
        self.request("reflog", false, |git| {
            git.open(path, cancel)?.reflog(limit, cancel)
        })
    }
    pub fn reflog_scoped(
        &self,
        path: &Path,
        scope: ReflogScope,
        limit: usize,
        cancel: &CancellationToken,
    ) -> Result<ReflogPage> {
        // A connectivity check of every object can take minutes in a large repository.
        let long = scope == ReflogScope::Unreachable;
        self.request("reflog", long, |git| {
            git.open(path, cancel)?.reflog_scoped(scope, limit, cancel)
        })
    }
    pub fn lost_commits(
        &self,
        path: &Path,
        cancel: &CancellationToken,
        progress: &std::sync::mpsc::SyncSender<LostCommitProgress>,
    ) -> Result<LostCommitScan> {
        self.request("lost-commits", true, |git| {
            git.open(path, cancel)?
                .scan_lost_commits(cancel, Some(progress))
        })
    }
    pub fn lost_recovery_preview(
        &self,
        path: &Path,
        scan: &LostCommitScan,
        target: &ObjectId,
        name: &str,
        cancel: &CancellationToken,
    ) -> Result<RecoveryPreview> {
        self.request("lost-recovery-preview", false, |git| {
            git.open(path, cancel)?
                .preview_lost_recovery(scan, target, name, cancel)
        })
    }
    pub fn recovery_preview(
        &self,
        path: &Path,
        target: &ObjectId,
        action: RecoveryAction,
        cancel: &CancellationToken,
    ) -> Result<RecoveryPreview> {
        self.request("recovery-preview", false, |git| {
            git.open(path, cancel)?
                .preview_recovery(target, action, cancel)
        })
    }
    pub fn rebase_preview(
        &self,
        root: &Path,
        action: RebaseAction,
        target: &str,
        cancel: &CancellationToken,
    ) -> Result<RebasePreview> {
        self.request("rebase-preview", false, |git| {
            git.open(root, cancel)?
                .preview_rebase(action, target, cancel)
        })
    }
    /// Read-only review of a numbered mail folder or the retained import state.
    pub fn mail_import_preview(&self,root:&Path,action:MailImportAction,folder:&Path,cancel:&CancellationToken)->Result<MailImportPreview>{
        self.request("series-preview",false,|git|{let repo=git.open(root,cancel)?;if action==MailImportAction::Start{repo.preview_mail_import(folder,cancel)}else{repo.preview_mail_recovery(action,cancel)}})
    }
    pub fn preserving_rebase_preview(&self, root: &Path, target: &str, cancel: &CancellationToken) -> Result<RebasePreview> {
        self.request("rebase-preview", false, |git| git.open(root,cancel)?.preview_rebase_preserving_merges(target,cancel))
    }
    /// An interactive rebase of the whole current branch, from its root commit.
    pub fn root_rebase_preview(
        &self,
        root: &Path,
        cancel: &CancellationToken,
    ) -> Result<RebasePreview> {
        self.request("rebase-preview", false, |git| {
            git.open(root, cancel)?.preview_root_rebase(cancel)
        })
    }
    pub fn identity(&self, root: &Path, cancel: &CancellationToken) -> Result<RepositoryIdentity> {
        self.request("identity-read", false, |git| {
            git.open(root, cancel)?.identity(cancel)
        })
    }
    pub fn preview_undo(&self, root: &Path, record: &UndoCommit, cancel: &CancellationToken) -> Result<UndoCommitPreview> {
        self.request("undo-preview", false, |git| git.open(root, cancel)?.preview_undo_commit(record, cancel))
    }
    /// Delete branches: what Git's ordinary deletion would accept now.
    pub fn branch_merge_states(
        &self,
        root: &Path,
        names: &[Vec<u8>],
        cancel: &CancellationToken,
    ) -> Result<Vec<BranchMergeState>> {
        self.request("branch-merge-state", false, |git| {
            git.open(root, cancel)?.branch_merge_states(names, cancel)
        })
    }
    /// Push: what pushing `branch` to `remote`'s `destination` would send, as of the
    /// last fetch.
    pub fn push_outlook(
        &self,
        root: &Path,
        branch: &[u8],
        remote: &[u8],
        destination: &[u8],
        cancel: &CancellationToken,
    ) -> Result<PushOutlook> {
        self.request("push-outlook", false, |git| {
            git.open(root, cancel)?
                .push_outlook(branch, remote, destination, cancel)
        })
    }
    /// Reflog & recovery: the selected entry's commit, before any recovery is chosen.
    pub fn recovery_details(
        &self,
        root: &Path,
        target: &ObjectId,
        cancel: &CancellationToken,
    ) -> Result<RecoveryDetails> {
        self.request("recovery-preview", false, |git| {
            git.open(root, cancel)?.recovery_details(target, cancel)
        })
    }
    /// Worktrees: each working folder's changes and last commit.
    pub fn worktree_statuses(
        &self,
        root: &Path,
        entries: &[WorktreeEntry],
        cancel: &CancellationToken,
    ) -> Result<Vec<WorktreeStatus>> {
        self.request("worktrees", false, |git| {
            git.open(root, cancel)?.worktree_statuses(entries, cancel)
        })
    }
    /// Reflog & recovery: which of the listed commits no branch reaches.
    pub fn off_branches(
        &self,
        root: &Path,
        ids: &[ObjectId],
        cancel: &CancellationToken,
    ) -> Result<Option<std::collections::HashSet<ObjectId>>> {
        self.request("reflog", false, |git| {
            git.open(root, cancel)?.off_branches(ids, cancel)
        })
    }
    pub fn issue_link_settings(
        &self,
        root: &Path,
        cancel: &CancellationToken,
    ) -> Result<IssueLinkSettings> {
        self.request("issue-links-read", false, |git| {
            git.open(root, cancel)?.issue_link_settings(cancel)
        })
    }
    /// Display reads reuse the open repository's verified client. The rules
    /// themselves are still read fresh, including Git's effective includes.
    pub(crate) fn issue_link_settings_for(
        &self,
        repository: &Repository,
        cancel: &CancellationToken,
    ) -> Result<IssueLinkSettings> {
        self.log.request("issue-links-read", |log| {
            repository.with_log(log.clone()).issue_link_settings(cancel)
        })
    }
    /// Test connection for a remote URL typed in a repository's remote dialog.
    pub fn check_remote(
        &self,
        root: &Path,
        url: &[u8],
        cancel: &CancellationToken,
    ) -> Result<RemoteCheck> {
        self.request("remote-check", false, |git| {
            git.open(root, cancel)?.check_remote(url, cancel)
        })
    }
    /// Test connection for a clone source, outside every repository.
    pub fn check_clone_source(
        &self,
        source: &CloneSource,
        cancel: &CancellationToken,
    ) -> Result<RemoteCheck> {
        self.request("remote-check", false, |git| {
            git.check_clone_source(source, &std::env::temp_dir(), cancel)
        })
    }
    pub fn gitflow_state(&self, root: &Path, cancel: &CancellationToken) -> Result<GitFlowState> {
        self.request("gitflow-read", false, |git| {
            git.open(root, cancel)?.gitflow_state(cancel)
        })
    }
    pub fn gitflow_init_preview(
        &self,
        root: &Path,
        config: &GitFlowConfig,
        cancel: &CancellationToken,
    ) -> Result<GitFlowInitPreview> {
        self.request("gitflow-init-preview", false, |git| {
            git.open(root, cancel)?.preview_gitflow_init(config, cancel)
        })
    }
    pub fn gitflow_start_preview(
        &self,
        root: &Path,
        kind: GitFlowKind,
        name: &str,
        base: Option<&str>,
        cancel: &CancellationToken,
    ) -> Result<GitFlowStartPreview> {
        self.request("gitflow-start-preview", false, |git| {
            git.open(root, cancel)?
                .preview_gitflow_start(kind, name, base, cancel)
        })
    }
    pub fn gitflow_finish_preview(
        &self,
        root: &Path,
        branch: &str,
        options: &GitFlowFinishOptions,
        cancel: &CancellationToken,
    ) -> Result<GitFlowFinishPreview> {
        self.request("gitflow-finish-preview", false, |git| {
            git.open(root, cancel)?
                .preview_gitflow_finish(branch, options, cancel)
        })
    }
    /// A commit message draft from the staged changes, through Claude Code. Its own
    /// timeout applies, not the 30-second Git limit.
    pub fn commit_draft(
        &self,
        root: &Path,
        request: &DraftRequest,
        cancel: &CancellationToken,
    ) -> Result<CommitDraft> {
        self.request("commit-draft", false, |git| {
            git.open(root, cancel)?
                .draft_commit_message(request, cancel)
        })
    }
    pub fn commit_template(
        &self,
        root: &Path,
        cancel: &CancellationToken,
    ) -> Result<Option<CommitTemplate>> {
        self.request("commit-template", false, |git| {
            git.open(root, cancel)?.commit_template(cancel)
        })
    }
    pub fn forge_connect(&self, config: ForgeConfig, username: &str, token: &str, cancel: &CancellationToken) -> Result<ForgeSession> {
        self.request("hosting-connect", false, |git| git.forge_session(config, username, token, cancel))
    }
    pub fn forge_browser_connect(&self, config: ForgeConfig, cancel: &CancellationToken) -> Result<(ForgeSession,String)> {
        self.request("hosting-browser-sign-in", true, |git| git.forge_browser_session(config,cancel))
    }
    pub fn forge_request<T>(&self, operation: &str, session: &ForgeSession, read: impl FnOnce(&ForgeSession) -> Result<T>) -> Result<T> {
        self.log.request(operation, |log| read(&session.with_log(log.clone())))
    }
    pub fn publish_request<T>(&self, operation: &str, root: &Path, session: &ForgeSession, run: impl FnOnce(&Repository, &ForgeSession) -> Result<T>, cancel: &CancellationToken) -> Result<T> {
        self.log.request(operation, |log| {
            let git = self.client.for_request(CommandLimits { timeout: Duration::from_secs(600), ..Default::default() }, log.clone())?;
            run(&git.open(root, cancel)?, &session.with_log(log.clone()))
        })
    }

    /// Sign in with the github.com credential Git's credential helper stores.
    pub fn github_session(
        &self,
        endpoint: GitHubEndpoint,
        interactive: bool,
        cancel: &CancellationToken,
    ) -> Result<GitHubSession> {
        self.request("github-sign-in", false, |git| {
            git.github_session(endpoint, interactive, cancel)
        })
    }
    /// One read through a signed-in session, logged as its own request.
    pub fn github<T>(
        &self,
        operation: &str,
        session: &GitHubSession,
        read: impl FnOnce(&GitHubSession) -> Result<T>,
    ) -> Result<T> {
        self.log
            .request(operation, |log| read(&session.with_log(log.clone())))
    }

    /// Sign in on a preview's refused card: Git's credential helper for `host`.
    pub fn preview_sign_in(&self, host: &str, cancel: &CancellationToken) -> Result<()> {
        self.request("preview-sign-in", false, |git| git.preview_sign_in(host, cancel))
    }

    /// A GitHub session without sign-in, for searching public repositories.
    pub fn github_anonymous(&self, endpoint: GitHubEndpoint) -> Result<GitHubSession> {
        self.client.github_anonymous(endpoint)
    }

    /// Clone `source` as a read-only preview into `slot` and download the files of
    /// its default branch; see `GitClient::open_preview`.
    pub fn open_preview(
        &self,
        source: &CloneSource,
        slot: &PreviewSlot,
        limit: Option<u64>,
        meter: &PreviewDownloads,
        progress: &SyncSender<CloneProgress>,
        cancel: &CancellationToken,
    ) -> Result<PreviewOpened> {
        self.request("preview-open", true, |git| {
            git.open_preview(source, slot, limit, meter, Some(progress), cancel)
        })
    }

    /// Let reads of an opened preview download the file contents they need.
    pub fn register_preview(&self, repository: &Path, meter: Arc<PreviewDownloads>) -> Result<()> {
        self.client.register_preview(repository, meter)
    }

    pub fn unregister_preview(&self, repository: &Path) {
        self.client.unregister_preview(repository);
    }

    /// Keep a preview: download the rest, clone it to `destination` with the verified
    /// clone, and point origin at `url`. Reported like a clone.
    pub fn keep_preview(
        &self,
        preview: PathBuf,
        url: String,
        name: String,
        destination: PathBuf,
        progress: &SyncSender<CloneProgress>,
        cancel: &CancellationToken,
    ) -> Result<Outcome> {
        self.request("keep-preview", true, |git| {
            let kept = git.keep_preview(&preview, &url, &destination, Some(progress), cancel)?;
            let report = kept.clone;
            let mut out = Outcome::default();
            if let Some(output) = out.observe("Clone", report.command) {
                out.keep_output(output);
            }
            if let Some(verification) = report.verification {
                out.observe("Verify clone", verification);
            }
            let published = report
                .publication
                .is_some_and(|result| out.observe("Publish destination", result).is_some());
            if let Some(repo) = report.repository {
                out.observe("Open repository", repo);
            }
            if let Some(cleanup) = report.cleanup {
                out.observe("Clean empty workspace", cleanup);
            }
            let origin = kept
                .origin
                .is_some_and(|result| out.observe("Point origin at the original address", result).is_some());
            out.command_succeeded = published && origin;
            if published {
                out.summary = if origin {
                    format!("Kept {name} in {}", destination.display())
                } else {
                    format!("Kept {name} in {}, but origin still names the preview; set it in Remotes", destination.display())
                };
                out.open_repository = Some(destination);
            } else {
                out.retained_workspace = Some(report.staging_directory);
                out.summary = "The preview was not kept; existing destination files were preserved".into();
            }
            Ok(out)
        })
    }

    pub fn init_repository(
        &self,
        destination: &Path,
        branch: &str,
        cancel: &CancellationToken,
    ) -> Result<Outcome> {
        self.request("init", false, |git| {
            let report = git.init_repository(destination, branch, cancel)?;
            let mut out = Outcome::default();
            out.observe("Prepare repository", report.preparation);
            let published = report
                .publication
                .is_some_and(|result| out.observe("Create repository", result).is_some());
            if let Some(repository) = report.repository {
                out.observe("Open repository", repository);
            }
            if let Some(cleanup) = report.cleanup
                && out.observe("Clean empty workspace", cleanup).is_none()
            {
                out.retained_workspace = Some(report.staging_directory.clone());
            }
            out.command_succeeded = published;
            if published {
                out.open_repository = Some(report.destination);
                out.summary =
                    "Repository created; existing files preserved and nothing staged".into();
            } else {
                out.retained_workspace = Some(report.staging_directory);
                out.summary = "Repository was not created; existing files preserved".into();
            }
            Ok(out)
        })
    }

    /// Resolve a saved shortcut off the UI thread. Does not scan history or mutate Git.
    pub fn library_repository(&self, root: &Path, cancel: &CancellationToken) -> Result<PathBuf> {
        self.request("library-check", false, |client| {
            if !root.is_absolute() { return Err(Error::InvalidInput("Use an absolute repository folder path.".into())); }
            let repository = client.open(root, cancel)?;
            cancel.check()?;
            Ok(repository.info().worktree.clone().unwrap_or_else(|| repository.info().git_dir.clone()))
        })
    }

    /// A saved repository's row in the Repository manager: local reads only, each Git
    /// command limited to `limit`.
    pub fn repository_summary(&self, root: &Path, limit: Duration, cancel: &CancellationToken) -> Result<RepositorySummary> {
        self.log.request("repository-summary", |log| {
            let limits = CommandLimits { timeout: limit, ..Default::default() };
            let repository = self.client.for_request(limits, log.clone())?.open(root, cancel)?;
            repository.summary(SummaryReads { status: true, last_commit: false }, cancel)
        })
    }

    /// Fetch all: one repository fetched by automatic fetch's rules (every remote, no
    /// pruning or merging, never a prompt), logged as its own operation.
    pub fn fetch_all_listed(&self, root: &Path, tags: FetchTags, cancel: &CancellationToken) -> Result<QuietFetchReport> {
        self.request("fetch-all", true, |client| client.open(root, cancel)?.fetch_all_quietly(tags, cancel))
    }

    /// Pull all: one repository's fast-forward pull that never asks for a sign-in,
    /// and how many commits it brought when it moved the branch.
    pub fn pull_ff_only_quietly(&self, root: &Path, cancel: &CancellationToken) -> Result<(PullReport, Option<u64>)> {
        self.request("pull-all", true, |client| {
            let repository = client.open(root, cancel)?;
            let report = repository.pull_ff_only_quietly(cancel)?;
            let commits = match &report.update {
                Some(Ok(update)) if update.kind == PullUpdateKind::FastForward => repository
                    .commits_between(&update.before, &update.fetched, &CancellationToken::new())
                    .ok(),
                _ => None,
            };
            Ok((report, commits))
        })
    }

    pub fn discover(log_path: &Path) -> Result<Self> {
        Self::discover_scoped(log_path, None)
    }
    pub fn discover_scoped(log_path: &Path, scope: Option<TransportScope>) -> Result<Self> {
        Self::discover_selected(log_path, scope, None, &CancellationToken::new())
    }
    pub fn global_identity(&self, cancel: &CancellationToken) -> Result<GlobalIdentity> {
        self.request("global-identity-read", false, |git| git.global_identity(cancel))
    }
    pub fn save_global_identity(&self, expected: &GlobalIdentity, value: &LocalIdentity, cancel: &CancellationToken) -> Result<()> {
        self.request("global-identity-save", false, |git|git.set_global_identity(expected,value,cancel))
    }
    pub fn discover_selected(log_path: &Path, scope: Option<TransportScope>, selected: Option<&Path>, cancel: &CancellationToken) -> Result<Self> {
        let log = DiagnosticLog::open(log_path).map_err(|source| Error::Io {
            action: "open application diagnostics",
            source,
        })?;
        let selected = if scope.is_some() && let Some(requested) = selected {
            // MCP may use the pinned bundle or the first installed Git on PATH,
            // never an arbitrary program supplied by a profile. Resolve metadata
            // only here; client construction verifies the bundle before execution.
            let bundled = rocket_git::managed_git_path()?;
            let resolved = requested.canonicalize().ok();
            if requested == bundled || (resolved.is_some() && resolved == bundled.canonicalize().ok()) {
                // Use the known path, not an alias which could change after this
                // check or bypass bundled verification through a leaf link.
                Some(bundled)
            } else {
                let installed = GitClient::installed_executable().ok();
                if resolved.is_none() || resolved != installed {
                    return Err(Error::InvalidInput("Automation may use only bundled Git or the installed Git discovered on PATH".into()));
                }
                installed
            }
        } else {
            selected.map(Path::to_path_buf)
        };
        let client = GitClient::discover_with_options(selected.as_deref(), CommandLimits::default(), Some(log.clone()), cancel)?.with_transport_scope(scope);
        Ok(Self {
            executable: client.executable().into(),
            version: client.version().into(),
            client,
            display_cache: Default::default(),
            log,
        })
    }
    pub fn set_secret_protection(&self, enabled: bool) {
        self.client.set_secret_protection(enabled);
    }

    pub fn set_guard_ai_api_keys(&self, enabled: bool) {
        self.client.set_guard_ai_api_keys(enabled);
    }
    pub fn set_secret_scan_mib(&self, mib: u64) -> Result<()> {
        self.client.set_secret_scan_mib(mib)
    }
    pub(crate) fn set_push_check(&self, check: Option<Arc<dyn rocket_git::PushCheck>>) -> Result<()> {
        self.client.set_push_check(check)
    }
    /// Fresh welcome-screen HEAD without rediscovering the same Git executable.
    pub fn head(&self, root: &Path, cancel: &CancellationToken) -> Result<Head> {
        self.log.request("head", |log| {
            self.client
                .with_log(log.clone())
                .open(root, cancel)?
                .head(cancel)
        })
    }
    pub(crate) fn request<T>(
        &self,
        operation: &str,
        network: bool,
        run: impl FnOnce(GitClient) -> Result<T>,
    ) -> Result<T> {
        self.log.request(operation, |log| {
            let limits = CommandLimits {
                timeout: Duration::from_secs(if network { 600 } else { 30 }),
                ..Default::default()
            };
            run(self.client.for_request(limits, log.clone())?)
        })
    }
    pub fn snapshot(
        &self,
        root: &Path,
        start: HistoryStart,
        cancel: &CancellationToken,
    ) -> Result<Snapshot> {
        self.snapshot_observed(root, start, cancel, None, false)
    }
    pub fn snapshot_observed(
        &self,
        root: &Path,
        start: HistoryStart,
        cancel: &CancellationToken,
        wake: Option<Arc<dyn Fn() + Send + Sync>>,
        automatic: bool,
    ) -> Result<Snapshot> {
        self.snapshot_ordered(root, start, HistoryOrder::default(), cancel, wake, automatic)
    }
    pub fn snapshot_ordered(&self, root: &Path, start: HistoryStart, order: HistoryOrder,
        cancel: &CancellationToken, wake: Option<Arc<dyn Fn() + Send + Sync>>, automatic: bool,
    ) -> Result<Snapshot> {
        self.log
            .request(if automatic { "auto_refresh" } else { "status" }, |log| {
                let git = self.client.with_log(log.clone());
                let repo = git.open(root, cancel)?;
                let mut warnings = Vec::new();
                // Arm before reading. Changes during the snapshot cause a follow-up,
                // including the interval while the old watcher is being replaced.
                let watcher = wake.and_then(|wake| {
                    match log.request("watch", |_| RepositoryWatch::new(repo.info(), wake)) {
                        Ok(watch) => Some(Arc::new(watch)),
                        Err(error) => {
                            warnings.push(format!(
                                "Automatic refresh unavailable: {}. Use F5.",
                                user_error(&error)
                            ));
                            None
                        }
                    }
                });
                let refresh_guard = refresh::Guard::capture(&repo, watcher.as_ref(), cancel)?;
                fn read<T>(
                    result: Result<T>,
                    phase: &str,
                    warnings: &mut Vec<String>,
                ) -> Option<T> {
                    match result {
                        Ok(value) => Some(value),
                        Err(error) => {
                            warnings.push(format!("{phase}: {}", user_error(&error)));
                            None
                        }
                    }
                }
                // Three bounded read lanes; no Git data is cached or omitted.
                // Keep status + history together to reuse the observed HEAD,
                // while independent refs/config and operation markers run alongside.
                let all_branches = matches!(start, HistoryStart::AllBranches);
                let (status, head, history, branches, remotes, stashes, state, sequence) =
                    std::thread::scope(|scope| -> Result<_> {
                        let history_lane = std::thread::Builder::new()
                            .name("rocket-snapshot-history".into())
                            .spawn_scoped(scope, || {
                                let mut warnings = Vec::new();
                                let status = if repo.info().worktree.is_some() {
                                    read(repo.status(cancel), "Changes", &mut warnings)
                                } else {
                                    None
                                };
                                let head = status
                                    .as_ref()
                                    .map(|s| s.head.clone())
                                    .or_else(|| read(repo.head(cancel), "HEAD", &mut warnings));
                                // In the combined view this one branch read feeds
                                // both the sidebar and traversal. HEAD was already
                                // observed by status; do not rediscover it in Git.
                                let history_branches = all_branches.then(|| {
                                    read(repo.branches(cancel), "Branches", &mut warnings)
                                });
                                let history = match (&start, &head) {
                                    (HistoryStart::AllBranches, Some(head)) => {
                                        history_branches.as_ref().and_then(|branches| {
                                            branches.as_ref().and_then(|branches| {
                                                let mut tips: Vec<_> = branches
                                                    .iter()
                                                    .map(|b| b.target.clone())
                                                    .collect();
                                                if let Head::Branch { target, .. }
                                                | Head::Detached { target } = head
                                                {
                                                    tips.push(target.clone());
                                                }
                                                read(
                                                    repo.history_ordered(
                                                        HistoryStart::Tips(tips),
                                                        200,
                                                        order,
                                                        cancel,
                                                    ),
                                                    "History",
                                                    &mut warnings,
                                                )
                                            })
                                        })
                                    }
                                    (HistoryStart::AllBranches, None) => None,
                                    (HistoryStart::Head, Some(Head::Unborn { .. })) => {
                                        Some({ let mut page = HistoryPage::default(); page.order = order; page })
                                    }
                                    (
                                        HistoryStart::Head,
                                        Some(
                                            Head::Branch { target, .. } | Head::Detached { target },
                                        ),
                                    ) => read(
                                        repo.history_ordered(
                                            HistoryStart::Commit(target.clone()),
                                            200,
                                            order,
                                            cancel,
                                        ),
                                        "History",
                                        &mut warnings,
                                    ),
                                    _ => read(
                                        repo.history_ordered(start.clone(), 200, order, cancel),
                                        "History",
                                        &mut warnings,
                                    ),
                                };
                                (status, head, history, history_branches.flatten(), warnings)
                            })
                            .map_err(|source| Error::Io {
                                action: "start snapshot history worker",
                                source,
                            })?;
                        let refs_lane = std::thread::Builder::new()
                            .name("rocket-snapshot-refs".into())
                            .spawn_scoped(scope, || {
                                let mut warnings = Vec::new();
                                let branches = if all_branches {
                                    Vec::new()
                                } else {
                                    read(repo.branches(cancel), "Branches", &mut warnings)
                                        .unwrap_or_default()
                                };
                                let remotes = read(
                                    match &refresh_guard {
                                        Some(guard) => guard.remotes(cancel),
                                        None => repo.remotes(cancel),
                                    },
                                    "Remotes", &mut warnings,
                                )
                                    .unwrap_or_default();
                                // A bare repository has no working folder, so no stashes.
                                let stashes = if repo.info().worktree.is_some() {
                                    read(repo.stashes(cancel), "Stashes", &mut warnings)
                                        .unwrap_or_default()
                                } else {
                                    Vec::new()
                                };
                                (branches, remotes, stashes, warnings)
                            })
                            .map_err(|source| Error::Io {
                                action: "start snapshot reference worker",
                                source,
                            })?;
                        let state = if repo.info().worktree.is_some() {
                            read(
                                repo.operation_state(cancel),
                                "Operation state",
                                &mut warnings,
                            )
                        } else {
                            None
                        };
                        let sequence = if state.as_ref().is_some_and(|s| s.sequencer) {
                            read(
                                repo.commit_sequence(cancel),
                                "Commit sequence",
                                &mut warnings,
                            )
                            .flatten()
                        } else {
                            None
                        };
                        let (status, head, history, history_branches, history_warnings) =
                            history_lane.join().map_err(|_| Error::WorkerPanicked)?;
                        let (branches, remotes, stashes, refs_warnings) =
                            refs_lane.join().map_err(|_| Error::WorkerPanicked)?;
                        let branches = history_branches.unwrap_or(branches);
                        warnings.extend(history_warnings);
                        warnings.extend(refs_warnings);
                        Ok((
                            status, head, history, branches, remotes, stashes, state, sequence,
                        ))
                    })?;
                cancel.check()?;
                let refresh_guard = refresh_guard.filter(|guard| warnings.is_empty()
                    && head.is_some() && status.is_some()
                    && watcher.as_ref().and_then(|watch| watch.metadata_generation()) == Some(guard.generation)
                    && watcher.as_ref().is_some_and(|watch| watch.is_current().unwrap_or(false)));
                Ok(Snapshot {
                    watcher,
                    refresh_guard,
                    info: repo.info().clone(),
                    repository: repo,
                    head,
                    status,
                    branches,
                    remotes,
                    stashes,
                    state,
                    sequence,
                    history,
                    warnings,
                })
            })
    }
    pub fn history(
        &self,
        repository: &Repository,
        start: HistoryStart,
        cancel: &CancellationToken,
    ) -> Result<HistoryPage> {
        self.log.request("history", |log| {
            repository.with_log(log.clone()).history(start, 200, cancel)
        })
    }
    pub fn diff(
        &self,
        repository: &Repository,
        options: &DiffOptions,
        cancel: &CancellationToken,
    ) -> Result<crate::diff::Patch> {
        self.log.request("diff", |log| {
            let repository = repository.with_log(log.clone());
            if let DiffTarget::Commit { id } = &options.target {
                // Independent fresh reads overlap; both workers settle before the
                // result is published. No cached Git result or detached worker.
                let (mut patch, files) = std::thread::scope(|scope| -> Result<_> {
                    let metadata = std::thread::Builder::new()
                        .name("rocket-diff-identities".into())
                        .spawn_scoped(scope, || repository.file_comparisons(options, cancel))
                        .map_err(|source| Error::Io { action: "start diff identity worker", source })?;
                    let patch = repository.diff(options, cancel)
                        .map(|diff| self.display_cache.parse(&diff.patch));
                    let files = metadata.join().map_err(|_| Error::WorkerPanicked)?;
                    Ok((patch?, files?))
                })?;
                patch.bind_command_paths(id.clone(), &files);
                patch.load_submodules_for_files(&repository, &files, cancel)?;
                patch.bind_images(files.into_iter().filter(|f| rocket_git::image_path(f.path().as_bytes())).collect());
                cancel.check()?;
                return Ok(patch);
            }
            let mut patch = if options.paths.len() == 1
                && matches!(options.target, DiffTarget::Staged | DiffTarget::Unstaged)
            {
                let review = repository.review_change(
                    &options.paths[0],
                    matches!(options.target, DiffTarget::Staged),
                    cancel,
                )?;
                self.display_cache.parse(review.patch()).with_review(review)
            } else if options.paths.len() == 2
                && matches!(options.target, DiffTarget::Staged | DiffTarget::Unstaged)
            {
                // The Changes view includes both paths for a rename. Keep the
                // complete requested display, but enable selection only when a
                // sealed single-file review describes exactly that patch. An
                // arbitrary two-file diff must never acquire single-file writes.
                let requested = repository.diff(options, cancel)?.patch;
                let review = repository.review_change(
                    &options.paths[0],
                    matches!(options.target, DiffTarget::Staged),
                    cancel,
                )?;
                if review.patch() == requested {
                    self.display_cache.parse(review.patch()).with_review(review)
                } else {
                    self.display_cache.parse(&requested)
                }
            } else {
                self.display_cache.parse(&repository.diff(options, cancel)?.patch)
            };
            if patch
                .files
                .iter()
                .any(|f| rocket_git::image_path(f.label.as_bytes()))
                || patch.needs_submodules()
            {
                let files = repository.file_comparisons(options, cancel)?;
                patch.load_submodules_for_files(&repository, &files, cancel)?;
                patch.bind_images(files.into_iter().filter(|f| rocket_git::image_path(f.path().as_bytes())).collect());
            }
            cancel.check()?;
            Ok(patch)
        })
    }
    pub fn search_history(
        &self,
        repository: &Repository,
        start: HistorySearchStart,
        cancel: &CancellationToken,
    ) -> Result<HistorySearchPage> {
        self.log.request("history-search", |log| {
            repository
                .with_log(log.clone())
                .search_history(start, cancel)
        })
    }
    pub fn preview(
        &self,
        repository: &Repository,
        path: &RepoPath,
        cancel: &CancellationToken,
    ) -> Result<crate::diff::Patch> {
        self.log.request("file-preview", |log| {
            let repository = repository.with_log(log.clone());
            if rocket_git::image_path(path.as_bytes()) {
                return Ok(crate::diff::Patch::image_preview(
                    repository.untracked_image(path, cancel)?,
                ));
            }
            let preview = repository.preview_untracked(path, cancel)?;
            if preview.binary || preview.truncated { return Ok(crate::diff::Patch::preview(path, preview)); }
            Ok(crate::diff::Patch::from_review(repository.review_untracked(path, cancel)?))
        })
    }
    pub fn conflict_preview(
        &self,
        repository: &Repository,
        path: &RepoPath,
        stage: u8,
        cancel: &CancellationToken,
    ) -> Result<crate::diff::Patch> {
        self.log.request("conflict-preview", |log| {
            let repository = repository.with_log(log.clone());
            let preview = if stage == 0 { repository.preview_untracked(path, cancel)? } else { repository.conflict_preview(path, stage, cancel)? };
            let mut patch = crate::diff::Patch::preview(path, preview);
            patch.notice = Some("Conflict preview. Edit the working file in your editor or terminal, review it, then mark it resolved.".into());
            Ok(patch)
        })
    }
    pub fn commit_preview(
        &self,
        root: &Path,
        request: CommitActionRequest,
        cancel: &CancellationToken,
    ) -> Result<CommitPreview> {
        self.request("commit-preview", false, |git| {
            git.open(root, cancel)?
                .preview_commit_action(request, cancel)
        })
    }
    pub fn execute(
        &self,
        root: &Path,
        action: &Action,
        progress: &SyncSender<FetchProgress>,
        cancel: &CancellationToken,
    ) -> Result<Outcome> {
        self.execute_with_force_push_progress(root, action, progress, None, cancel)
    }

    pub(crate) fn execute_with_force_push_progress(
        &self,
        root: &Path,
        action: &Action,
        progress: &SyncSender<FetchProgress>,
        force_push: Option<&SyncSender<ForcePushProgress>>,
        cancel: &CancellationToken,
    ) -> Result<Outcome> {
        self.execute_with_network_refresh(root, action, progress, force_push, None, cancel)
    }

    pub(crate) fn execute_with_network_refresh(
        &self,
        root: &Path,
        action: &Action,
        progress: &SyncSender<FetchProgress>,
        force_push: Option<&SyncSender<ForcePushProgress>>,
        network: Option<&NetworkRead>,
        cancel: &CancellationToken,
    ) -> Result<Outcome> {
        // Network work, and rebase plans whose reviewed commands may run tests,
        // get the long command limit instead of the 30-second local one.
        let long = matches!(
            action,
            Action::Fetch(_) | Action::FetchWithTags(..)
                | Action::Pull
                | Action::PullFetch
                | Action::Push { .. }
                | Action::ForcePush(_)
                | Action::AtomicPush(_)
                | Action::RemoteTag(_)
                | Action::RemoteTagBatch(_)
                | Action::Submodule(_)
                | Action::SubmoduleAction(_)
                | Action::UpdateSubmodules(_)
                | Action::Lfs(_)
                | Action::LfsLock(_)
                | Action::Workflow{..}
        ) || matches!(action, Action::Rebase(preview) if preview.runs_commands());
        self.request(
            action.label(),
            long,
            |git| {
                let repo = git.open(root, cancel)?;
                let mut out = Outcome::default();
                if !matches!(action, Action::Bisect(_) | Action::BisectAuto { .. } | Action::RunCommand { .. })
                    && repo.is_bisecting(cancel)?
                {
                    return Err(Error::Bisect(BisectFailure::ActiveOperation));
                }
                match action {
                    Action::Stage(paths) => out.write(repo.stage(paths, cancel)?),
                    Action::StageAll => out.write(repo.stage_all(cancel)?),
                    Action::Unstage(paths) => out.write(repo.unstage(paths, cancel)?),
                    Action::UnstageAll => out.write(repo.unstage_all(cancel)?),
                    Action::ApplyReview(review, lines) => out.write(repo.apply_review(review, lines, cancel)?),
                    Action::MarkResolved(path) => out.write(repo.mark_resolved(path, cancel)?),
                    Action::SaveConflict { preview, text, stage } => out.conflict(repo.save_conflict(preview, text, *stage, cancel)?, *stage),
                    Action::KeepConflictSide { preview, keep } => out.whole_conflict(repo.keep_conflict_side(preview, *keep, cancel)?, *keep),
                    Action::Commit(message) => {
                        let report = repo.commit_with_undo(message, cancel)?;
                        out.undo_commit = report.undo.map(Arc::new);
                        out.write(report.write);
                        if let Some(note) = report.undo_unavailable { out.details.push(note); }
                    },
                    Action::UndoCommit(preview) => {
                        let report = repo.apply_undo_commit(preview, cancel)?;
                        let confirmed = report.verified.is_ok();
                        let command = out.observe("Undo command", report.result).is_some();
                        let verified = out.observe("Confirm Undo result", report.verified).is_some();
                        let refreshed = out.observe("Refresh after Undo", report.status).is_some();
                        out.command_succeeded = confirmed && verified && refreshed;
                        out.recovery_pending = !out.command_succeeded;
                        out.summary = if out.command_succeeded { "Commit undone; changes are staged and working files kept." }
                            else if command { "Undo ran; inspect the refreshed result before continuing." }
                            else { "Undo stopped or failed; inspect the branch and recovery details before retrying." }.into();
                        out.details.push(format!("Recovery reference (when applied): {}", report.recovery_reference));
                    },
                    Action::CommitAction { preview, message } => {
                        out.commit_action(repo.apply_commit_action(preview, message.as_deref(), cancel)?);
                        out.commit_result(preview);
                    }
                    Action::Rebase(preview) => {
                        let report = repo.apply_rebase(preview, cancel)?;
                        out.update_refs(preview, &report);
                        out.rebase(report);
                        out.rebase_result(preview);
                    }
                    Action::Bisect(preview) => out.bisect(repo.apply_bisect(preview,cancel)?),
                    Action::BisectAuto { preview, progress, storage } => crate::bisect::automatic::outcome(&mut out, repo.run_bisect_auto(preview,storage,Some(progress),cancel)?),
                    Action::MailImport(preview) => out.mail_import(repo.apply_mail_import(preview,cancel)?,preview),
                    Action::Recovery(preview) => out.recovery(repo.apply_recovery(preview, cancel)?, preview),
                    Action::Tag(preview) => out.tag(repo.apply_tag(preview, cancel)?),
                    Action::RemoteTag(preview) => out.remote_tag(repo.apply_remote_tag(preview, cancel)?),
                    Action::TagBatch(review) => crate::batch::tag_batch_outcome(&mut out, repo.apply_tags(&review.ready, cancel)?),
                    Action::RemoteTagBatch(review) => crate::batch::remote_tag_batch_outcome(&mut out, repo.apply_remote_tags(&review.ready, cancel)?),
                    Action::AtomicPush(preview) => out.atomic_push(repo.apply_atomic_push_with_progress(preview, force_push, cancel)?, preview),
                    Action::Ignore(preview) => out.ignore(repo.apply_ignore(preview, cancel)?),
                    Action::Workflow{review,selected} => out=review.apply(&repo,selected,cancel)?,
                    Action::ForcePush(preview) => out.force_push(repo.apply_force_push_with_progress(preview, force_push, cancel)?, preview),
                    Action::Worktree(preview) => out.worktree(repo.apply_worktree(preview, cancel)?),
                    Action::Submodule(preview) => out.submodule(repo.apply_submodule(preview,cancel)?),
                    Action::SubmoduleAction(preview) => out.submodule_action(repo.apply_submodule_action(preview,cancel)?),
                    Action::Lfs(preview) => out.lfs(repo.apply_lfs_with_progress(preview,Some(progress),cancel)?, preview.action()),
                    Action::LfsLock(preview) => out.lfs_lock(repo.apply_lfs_lock(preview,cancel)?),
                    Action::FileEdit(preview) => out.file_edit(repo.apply_file_edit(preview, cancel)?),
                    Action::ApplyPatch(preview) => out.patch(repo.apply_patch_preview(preview,cancel)?, !preview.mode_changes().is_empty()),
                    Action::ExportPatch(preview) => {
                        repo.export_patch(preview,cancel)?;
                        out.command_succeeded=true;
                        out.summary="Patch saved to a new file; existing files preserved.".into();
                        out.details.push(format!("Patch file: {}",preview.destination().display()));
                    },
                    Action::ExportPatchSeries(preview) => {
                        let report=repo.export_patch_series(preview,cancel)?;
                        out.command_succeeded=report.result.is_ok();
                        let folder=preview.destination().file_name().map_or_else(||preview.destination().display().to_string(),|n|n.to_string_lossy().into_owned());
                        let files=match preview.entries().len() {1=>"1 patch file".to_owned(),n=>format!("{n} patch files")};
                        out.summary=match (report.published,out.command_succeeded) {
                            (true,true)=>format!("{files} saved in {folder}"),
                            (true,false)=>format!("{files} saved in {folder}, but checking them failed. Look at the folder before using it."),
                            (false,_)=>"Commit series was not exported.".into(),
                        };
                        out.details.push(format!("{}: {}. Working files and staging didn't change.",if report.published {"Saved in"} else {"Nothing was saved in"},preview.destination().display()));
                        if let Err(error)=report.result {out.details.push(user_error(&error));}
                        if let Some(folder)=report.retained {out.details.push(format!("Preparation folder retained: {}. It may be incomplete; inspect before removing it.",folder.display()));}
                    },
                    Action::SetSigning { expected, options } => {
                        let report = repo.set_signing(expected, options, cancel)?;
                        out.command(report.result);
                        if let Some(observed) = out.observe("Reload signing settings", report.observed) && out.command_succeeded {
                            if observed.local != *options { out.command_succeeded=false;out.details.push("Observed local signing settings differ from the requested values. Reload before retrying.".into()); }
                            else { out.summary="Repository-local signing settings saved".into();
                                if (options.format.is_some() && observed.effective.format != options.format) || (options.key.is_some() && observed.effective.key != options.key) || (options.sign_commits.is_some() && observed.effective.sign_commits != options.sign_commits) || (options.allowed_signers.is_some() && observed.effective.allowed_signers != options.allowed_signers) { out.details.push("Another Git configuration overrides some saved local values. Reload signing settings to inspect effective values.".into()); }
                            }
                        }
                    },
                    Action::SetIdentity { expected, identity } => {
                        let report = repo.set_identity(expected, identity, cancel)?;
                        out.command(report.result);
                        if let Some(observed) = out.observe("Reload repository identity", report.identity)
                            && out.command_succeeded && ((identity.name.is_some() && observed.effective.name != identity.name)
                                || (identity.email.is_some() && observed.effective.email != identity.email)) {
                            out.details.push("Local identity saved, but another Git configuration overrides it. Reopen Repository Settings to review the effective values.".into());
                        }
                        if out.command_succeeded { out.summary = "Repository-local identity saved; global Git settings unchanged".into(); }
                    }
                    Action::SetIssueLinks { expected, rules } => {
                        let report = repo.set_issue_links(expected, rules, cancel)?;
                        out.command(report.result);
                        if let Some(observed) = out.observe("Reload issue link rules", report.settings)
                            && out.command_succeeded
                        {
                            if observed.local != *rules {
                                out.command_succeeded = false;
                                out.details.push("The saved rules differ from the requested ones. Reopen Repository Settings before retrying.".into());
                            } else {
                                out.summary = issue_link_summary(rules.len());
                            }
                        }
                    }
                    Action::CreateBranch(name) => out.branch(repo.create_branch(name, None, cancel)?),
                    Action::SwitchBranch(name) => out.branch(repo.switch_branch(name, cancel)?),
                    Action::CheckOutRemote { name, upstream } => {
                        out.branch(repo.check_out_remote_branch(name, upstream, cancel)?)
                    }
                    Action::UpdateSubmodules(root) => out = crate::general::update_submodules(&repo, root.as_deref(), cancel)?,
                    Action::RenameBranch(old, new) => out.branch(repo.rename_branch(old, new, cancel)?),
                    Action::DeleteBranch(name) => out.branch(repo.delete_branch(name, cancel)?),
                    Action::DeleteBranches(names) => crate::batch::branch_batch_outcome(&mut out, repo.delete_branches(names, cancel)?),
                    Action::RunCommand { name, program, arguments, storage } => crate::custom_commands::outcome(&mut out, name, repo.run_custom_command(program, arguments, storage, cancel)?),
                    Action::AddRemote { name, url } => out.remote(repo.add_remote(name, url, cancel)?),
                    Action::EditRemote { expected, kind, url } => out.remote(repo.set_remote_url(expected, *kind, url.as_deref(), cancel)?),
                    Action::RemoveRemote(expected) => out.remote(repo.remove_remote(expected, cancel)?),
                    Action::Stash {
                        message,
                        include_untracked,
                        paths,
                    } => out.stash(repo.push_stash_paths(
                        (!message.trim().is_empty()).then_some(message.as_str()),
                        *include_untracked,
                        paths,
                        cancel,
                    )?),
                    Action::ApplyStash { entry, restore_index } => {
                        out.stash(repo.apply_stash(&entry.id, *restore_index, cancel)?)
                    }
                    Action::DropStash(entry) => out.stash(repo.drop_stash(entry, cancel)?),
                    Action::Merge { target, mode } => out.merge(repo.merge(target, *mode, cancel)?),
                    Action::MergeReviewed(preview) => {
                        out.merge(repo.apply_merge(preview, cancel)?);
                        if preview.mode()==MergeMode::Squash && !out.command_succeeded {
                            out.details.push("Squash does not create MERGE_HEAD. Resolve any conflicts in Changes, then create an ordinary commit. Use reviewed discard/restore to undo prepared files; Continue/Abort merge does not apply.".into());
                        }
                        if out.command_succeeded {match preview.mode(){
                            MergeMode::Squash=>{out.summary="Squash prepared in Changes; no commit created. Review and commit it when ready.".into();out.recovery_pending=false;},
                            MergeMode::NoCommit=>out.summary="Merge prepared without committing. Review Changes, then Continue or Abort merge.".into(),
                            _=>(),
                        }}
                    },
                    Action::ContinueMerge(message) => out.merge(repo.continue_merge(message, cancel)?),
                    Action::AbortMerge => out.merge(repo.abort_merge(cancel)?),
                    Action::FinishMerge => out.merge(repo.finish_committed_merge(cancel)?),
                    Action::GitFlowInit(preview) => crate::gitflow::init_outcome(&mut out, repo.apply_gitflow_init(preview, cancel)?),
                    Action::GitFlowStart(preview) => crate::gitflow::start_outcome(&mut out, preview, repo.apply_gitflow_start(preview, cancel)?),
                    Action::GitFlowFinish(preview) => crate::gitflow::finish_outcome(&mut out, preview, repo.apply_gitflow_finish(preview, cancel)?),
                    Action::Fetch(name) | Action::FetchWithTags(name,_) => {
                        let tags=if let Action::FetchWithTags(_,tags)=action{*tags}else{FetchTags::None};
                        let (result, status, branches) = if let Some(view) = network {
                            let (result, refreshed) = repo.fetch_with_refresh(
                                name, tags, Some(progress), cancel, |_| view.reconcile(self, root),
                            )?;
                            out.reconciled(refreshed);
                            (result, None, None)
                        } else {
                            let report = repo.fetch_with_tags(name, tags, Some(progress), cancel)?;
                            (report.result, report.status, Some(report.branches))
                        };
                        if let Some(fetch) = out.observe("Fetch", result) {
                            out.command_succeeded = true;
                            out.keep_output(fetch.output);
                            out.observe("Interpret fetched refs", fetch.updates);
                        }
                        if let Some(status) = status {
                            out.observe("Refresh changes", status);
                        }
                        if let Some(branches) = branches { out.observe("Refresh branches", branches); }
                    }
                    Action::PullFetch => {
                        let (fetch, prepared, observations) = if let Some(view) = network {
                            let report = repo.fetch_for_pull_with_refresh(
                                Some(progress), cancel, |_| view.reconcile(self, root),
                            )?;
                            out.reconciled(report.refreshed);
                            (report.fetch, report.prepared, None)
                        } else {
                            let report = repo.fetch_for_pull(Some(progress), cancel)?;
                            (report.fetch, report.prepared, Some((report.status, report.branches, report.state)))
                        };
                        if let Some(fetch) = out.observe("Fetch upstream", fetch) {
                            out.command_succeeded = true;
                            out.keep_output(fetch.output);
                            out.observe("Interpret fetched refs", fetch.updates);
                        }
                        if let Some(prepared) = prepared {
                            out.pull_fetched = out.observe("Prepare fetched review", prepared).map(Arc::new);
                        }
                        if let Some((status, branches, state)) = observations {
                            out.observe("Refresh changes", status);
                            out.observe("Refresh branches", branches);
                            out.observe("Refresh operation", state);
                        }
                        out.summary = if out.pull_fetched.is_some() {
                            "Upstream fetched. Review the integration before applying."
                        } else {
                            "Pull review is unavailable. No integration was started; inspect the fetch result and refreshed state."
                        }.into();
                    }
                    Action::PullReviewed(preview) => match repo.apply_pull(preview, cancel)? {
                        PullApplyReport::Merge(report) => out.merge(*report),
                        PullApplyReport::Rebase(report) => out.rebase(*report),
                    },
                    Action::Pull => {
                        let report = repo.pull_ff_only(Some(progress), cancel)?;
                        if let Some(fetch) = out.observe("Fetch", report.fetch) {
                            out.keep_output(fetch.output);
                            out.observe("Interpret fetched refs", fetch.updates);
                        }
                        if let Some(update) = report.update
                            && let Some(update) = out.observe("Update branch", update)
                        {
                            out.command_succeeded = true;
                            out.keep_output(update.output);
                            out.observe("Verify branch tip", update.verification);
                            if update.error_diagnostics {
                                out.details
                                    .push("Git reported additional diagnostics after updating the branch.".into());
                            }
                        }
                        out.observe("Refresh changes", report.status);
                        out.observe("Refresh branches", report.branches);
                        out.observe("Refresh operation", report.state);
                    }
                    Action::Push {
                        remote,
                        local,
                        destination,
                        set_upstream,
                    } => {
                        let report = repo.push(
                            remote,
                            local,
                            destination,
                            PushOptions {
                                set_upstream: *set_upstream,
                            },
                            cancel,
                        )?;
                        out.summary = match report.remote_outcome {
                            PushRemoteOutcome::NotAttempted => "Push not attempted; no remote update was sent",
                            PushRemoteOutcome::Confirmed => "Remote update confirmed",
                            PushRemoteOutcome::Rejected => "Push rejected; remote branch was not updated by this push",
                            PushRemoteOutcome::Unknown => "Remote outcome unknown; inspect the remote before retrying",
                        }
                        .into();
                        out.command(report.result);
                        if let Some(update) = report.update {
                            out.observe("Interpret remote result", update);
                        }
                        if let Some(upstream) = report.upstream {
                            out.observe("Verify upstream", upstream);
                        }
                        if report.error_diagnostics {
                            out.details.push(
                                "Git reported additional diagnostics; local tracking/configuration may need attention."
                                    .into(),
                            );
                        }
                        if let Some(status) = report.status {
                            out.observe("Refresh changes", status);
                        }
                        out.observe("Refresh branches", report.branches);
                    }
                }
                if out.summary.is_empty() {
                    // Activity shows this, so the operation is named in words, not by key.
                    let name = crate::operation_names::done(action.label());
                    out.summary = if out.command_succeeded {
                        format!("{name} completed")
                    } else {
                        format!(
                            "{name} did not complete; review the refreshed repository before retrying"
                        )
                    };
                }
                Ok(out)
            },
        )
    }
    pub fn clone_repository(
        &self,
        source: CloneSource,
        destination: PathBuf,
        progress: &SyncSender<CloneProgress>,
        cancel: &CancellationToken,
    ) -> Result<Outcome> {
        self.request("clone", true, |git| {
            let report = git.clone_repository(&source, &destination, Some(progress), cancel)?;
            let mut out = Outcome::default();
            if let Some(output) = out.observe("Clone", report.command) {
                out.keep_output(output);
            }
            if let Some(verification) = report.verification {
                out.observe("Verify clone", verification);
            }
            let published = report
                .publication
                .is_some_and(|result| out.observe("Publish destination", result).is_some());
            if let Some(repo) = report.repository {
                out.observe("Open repository", repo);
            }
            if let Some(cleanup) = report.cleanup {
                out.observe("Clean empty workspace", cleanup);
            }
            if report.error_diagnostics {
                out.details
                    .push("Git reported additional diagnostics during clone.".into());
            }
            out.command_succeeded = published;
            if published {
                out.open_repository = Some(destination);
                out.summary = "Clone published successfully".into();
            } else {
                out.retained_workspace = Some(report.staging_directory);
                out.summary =
                    "Clone was not published; existing destination files were preserved".into();
            }
            Ok(out)
        })
    }
}
/// Raw helper output stays behind an explicit local disclosure in the activity panel.
/// It is never copied into the GUI activity summary or the diagnostic log.
/// The result line of a saved Issue links page.
fn issue_link_summary(rules: usize) -> String {
    match rules {
        0 => {
            "Issue link rules removed from this repository's config; global Git settings unchanged"
                .into()
        }
        1 => "1 issue link rule saved in this repository's config; global Git settings unchanged"
            .into(),
        n => format!(
            "{n} issue link rules saved in this repository's config; global Git settings unchanged"
        ),
    }
}
/// "Paused at patch 2 of 4: conflict in 1 file."
fn paused_line(noun: &str, step: Option<&OperationStep>, conflicts: usize) -> String {
    let place = step.map_or_else(|| "Paused".to_owned(), |s| format!("Paused at {noun} {} of {}", s.at, s.total));
    match conflicts {
        0 => format!("{place}."),
        1 => format!("{place}: conflict in 1 file."),
        n => format!("{place}: conflicts in {n} files."),
    }
}
fn paused_summary(noun: &str, report: &RebaseWriteReport) -> String {
    let conflicts = report.write.status.as_ref().map_or(0, |s| s.entries.iter().filter(|e| e.kind == EntryKind::Unmerged).count());
    paused_line(noun, report.state.as_ref().ok().and_then(|s| s.step.as_ref()), conflicts)
}
pub fn user_error(error: &Error) -> String {
    if let Some(failure) = error.signing_failure() {
        return format!("Signing [{}]: {}", failure.code(), failure.guidance());
    }
    if let Error::Git {
        operation: "remote-add" | "remote-edit" | "remote-remove",
        stderr,
        ..
    } = error
        && String::from_utf8_lossy(stderr).contains("could not lock config file")
    {
        return "Remote configuration is locked or not writable. Check other Git operations and file permissions, then refresh before retrying. Rocket leaves existing lock files in place.".into();
    }
    if let Error::Git {
        operation, code, ..
    } = error
        && error.fetch_failure().is_none()
        && error.push_failure().is_none()
        && error.pull_failure().is_none()
        && error.clone_failure().is_none()
        && error.lfs_push_failure().is_none()
        && error.lfs_lock_failure().is_none()
    {
        return format!(
            "Git {operation} failed (exit {}). Review changes and the optional local Git output for details.",
            code.map_or_else(|| "unknown".into(), |n| n.to_string())
        );
    }
    error.to_string()
}

#[cfg(test)]
mod remote_tag_report_tests {
    use super::*;
    fn output() -> Result<WriteOutput> {
        Ok(WriteOutput {
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }
    #[test]
    fn remote_tag_unknown_state_does_not_claim_success() {
        let mut outcome = Outcome::default();
        outcome.remote_tag(RemoteTagWriteReport {
            result: output(),
            observed: Err(Error::Cancelled),
            verification: Err(Error::Tag(TagFailure::VerificationFailed)),
            error_diagnostics: false,
        });
        assert!(!outcome.command_succeeded && outcome.recovery_pending);
        assert!(
            outcome
                .details
                .iter()
                .any(|s| s.contains("Remote state is unknown"))
        );
    }
    #[test]
    fn remote_tag_cancelled_but_verified_reports_completed_remote_effect() {
        let mut outcome = Outcome::default();
        outcome.remote_tag(RemoteTagWriteReport {
            result: Err(Error::Cancelled),
            observed: Ok(None),
            verification: Ok(()),
            error_diagnostics: false,
        });
        assert!(!outcome.command_succeeded && outcome.recovery_pending);
        assert!(outcome.summary.contains("already matches"));
        assert!(outcome.details.iter().any(|s| s.contains("absent")));
    }
    #[test]
    fn remote_tag_error_diagnostics_require_review_even_with_zero_exit() {
        let mut outcome = Outcome::default();
        outcome.remote_tag(RemoteTagWriteReport {
            result: output(),
            observed: Ok(None),
            verification: Ok(()),
            error_diagnostics: true,
        });
        assert!(!outcome.command_succeeded && outcome.recovery_pending);
        assert!(outcome.summary.contains("already matches"));
        assert!(
            outcome
                .details
                .iter()
                .any(|s| s.contains("Git also reported an error"))
        );
    }
}

#[cfg(test)]
mod bisect_report_tests {
    use super::*;
    #[test]
    fn successful_command_with_failed_inspection_does_not_claim_restoration() {
        let mut outcome = Outcome::default();
        outcome.bisect(BisectWriteReport {
            result: Ok(WriteOutput {
                stdout: Vec::new(),
                stderr: Vec::new(),
            }),
            status: Err(Error::Cancelled),
            session: Err(Error::Cancelled),
            state: Err(Error::Cancelled),
            failure: None,
        });
        assert!(outcome.command_succeeded);
        assert!(outcome.recovery_pending);
        assert!(outcome.summary.contains("refresh was incomplete"));
        assert!(!outcome.summary.contains("restored"));
    }
}
