//! Reviewed cleanup and saved-work workflows. Workers own backend values; edits
//! invalidate the sealed preview. Only the normal write task applies it.
use crate::{
    app::*,
    backend::{Action, Outcome, user_error},
    pages::{self, control},
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use std::{collections::HashMap, sync::Arc};

#[derive(Clone)]
pub enum Request {
    CommitPush(String, Vec<u8>, Vec<u8>),
    StashApply(StashApplyReview, Vec<StashApplySelection>),
    StashSnapshot(String, bool),
    IgnoreTracked(Vec<RepoPath>),
    FastForward,
    RenameRemote(Vec<u8>,String),
    StashControl(StashControl),
    SubmoduleBatch(Vec<RepoPath>,Option<std::path::PathBuf>),
    Branch(BranchWorkflowKind, String, BranchWorkflowOptions),
    Prune(Vec<u8>),
    Delete(Vec<u8>, Vec<u8>),
    Rename(StashEntry, String),
    Saved(SavedWorkAction),
    Lfs(String, Option<LfsSourceScope>),
}
#[derive(Clone, Debug)]
pub enum Review {
    CommitPush(CommitPushPreview),
    StashApply(StashApplyPreview),
    StashSnapshot(StashSnapshotPreview),
    IgnoreTracked(IgnoreTrackedPreview),
    FastForward(FastForwardPreview),
    RenameRemote(RemoteRenamePreview),
    StashControl(StashControlPreview),
    SubmoduleBatch(SubmoduleBatchPreview),
    StashSelection(StashSelectionPreview),
    Branch(Box<BranchWorkflowPreview>),
    Prune(RemoteCleanupPreview),
    Delete(RemoteBranchDeletePreview),
    Rename(StashRenamePreview),
    Saved(SavedWorkPreview),
    Lfs(LfsPrunePreview),
}
impl Review {
    pub fn label(&self) -> &'static str {
        match self {
            Self::CommitPush(_) => "commit-push",
            Self::StashApply(_) => "stash-apply-selection",
            Self::StashSnapshot(_) => "stash-snapshot",
            Self::IgnoreTracked(_) => "ignore-tracked",
            Self::FastForward(_) => "branch-fast-forward",
            Self::RenameRemote(_) => "remote-rename",
            Self::StashControl(_) => "stash-control",
            Self::SubmoduleBatch(_) => "submodule-batch",
            Self::StashSelection(_) => "stash-selection",
            Self::Branch(p) => p.kind().label(),
            Self::Prune(_) => "remote-prune",
            Self::Delete(_) => "remote-branch-delete",
            Self::Rename(_) => "stash-rename",
            Self::Saved(p) if matches!(p.action(), SavedWorkAction::Switch(_)) => "saved-switch",
            Self::Saved(_) => "saved-work",
            Self::Lfs(_) => "lfs-prune",
        }
    }
    pub(crate) fn apply(
        &self,
        repo: &Repository,
        selected: &[Vec<u8>],
        c: &CancellationToken,
    ) -> rocket_git::Result<Outcome> {
        let mut out = Outcome::default();
        let result = match self {
            Self::CommitPush(p) => {
                let r=repo.apply_commit_push(p,c)?;
                out.local_commit_created=r.reviewed_commit_confirmed;
                out.summary=if let Some(id)=&r.commit {format!("Local commit {id} retained.")} else {"No new local commit confirmed; inspect history before retrying.".into()};
                out.details.push(format!("Last phase: {}.",r.phase));
                if let Some(push)=r.push {
                    if let Err(error)=&push.result {out.details.push(user_error(error));}
                    out.details.push(match push.remote_outcome {
                        PushRemoteOutcome::Confirmed=>"Remote update confirmed.",
                        PushRemoteOutcome::Rejected=>"Push rejected. Fetch and review before retrying Push; do not create another commit.",
                        PushRemoteOutcome::NotAttempted=>"Push was not started. Review and retry Push separately.",
                        PushRemoteOutcome::Unknown=>"Remote outcome unknown. Check the remote before retrying Push.",
                    }.into());
                    let mut result=r.result.and(push.result.map(|_|()));
                    if let Some(update)=push.update {result=result.and(update.map(|_|()));}
                    result=result.and(push.branches.map(|_|()));
                    if push.error_diagnostics {result=result.and(Err(Error::InvalidInput("Push reported extra diagnostics. Inspect remote and tracking state before retrying.".into())));}
                    if result.is_ok() {out.summary="Local commit created and remote update confirmed.".into();}
                    result
                } else {
                    out.details.push("Push was not started. Inspect the local commit, then use Push separately.".into());
                    r.result
                }
            }
            Self::StashApply(p) => {
                let r = repo.apply_stash_selection(p, c)?;
                out.summary = "Selected saved changes applied. The stash and staging are kept.".into();
                out.details.push(format!("{} working files changed. The stash was not removed.", r.changed.len()));
                for path in &r.changed { out.details.push(format!("Applied: {}", path.display_lossy())); }
                for dir in r.recovery { out.details.push(format!("Working-file recovery: {}", dir.display())); }
                r.result.and(r.verification).and(r.status.map(|_| ()))
            }
            Self::StashSnapshot(p) => {
                let r=repo.stash_snapshot(p,c)?;
                out.summary="Stash snapshot saved. Working files and staging kept.".into();
                out.details.push(format!("Snapshot publication: {}. Recovery ref: {}. Object: {}.",match r.published{Some(true)=>"confirmed",Some(false)=>"not published",None=>"unknown; refresh before retrying"},r.recovery,r.object.as_ref().map(|id|id.as_str()).unwrap_or("not created")));
                r.result.and_then(|()|r.stashes.map(|_|()))
            }
            Self::IgnoreTracked(p) => {
                let r=repo.ignore_tracked(p,c)?;
                out.summary="Selected files are ignored and removed from the index; local copies kept.".into();
                out.details.push(format!("Rules saved: {}. Index replacement published: {}. .gitignore was not staged automatically.",r.rules_saved,r.index_published));
                if let Some(dir)=r.recovery{out.details.push(format!("Recovery copies: {}. Compare current staging before restoring an index backup.",dir.display()));}
                r.result.and_then(|()|r.status.map(|_|()))
            }
            Self::StashSelection(p) => {
                let r=repo.stash_selection(p,c)?;out.summary="Selected changes saved; other hunks and staging kept.".into();
                out.details.push(format!("Saved object: {}. Recovery ref: {}/0.",r.stash.as_ref().map(|s|s.as_str()).unwrap_or("not created"),r.recovery));
                if let Some(cleanup)=r.cleanup{for dir in cleanup.recovery{out.details.push(format!("Working-file recovery: {}",dir.display()));}}
                r.result
            }
            Self::FastForward(p) => {
                let r=repo.fast_forward_branches(p,selected,c)?;
                out.summary="Selected local branches fast-forwarded together.".into();
                out.details.push(format!("Recovery refs: {}. Working files and index retained.",r.recovery));
                r.result.and_then(|()|r.observed.map(|_|()))
            }
            Self::RenameRemote(p) => {
                let r=repo.rename_remote(p,c)?;out.summary="Remote renamed; URLs and upstream relationships retained.".into();
                out.details.push(format!("Config backup: {}. Tracking recovery refs: {}.",r.config_backup.display(),r.recovery_refs));r.report.result.map(|_|())
            }
            Self::StashControl(p) => {
                let r=repo.apply_stash_control(p,c)?;
                out.summary=if matches!(p.action(),StashControl::Clear){"Reviewed stashes cleared; recovery refs retained."}else{"Stash applied and removed from the list."}.into();
                out.details.push(format!("Entry removal confirmed: {}. Recovery refs: {}. If interrupted or conflicted, inspect Stashes and Changes before retrying.",r.removed,r.recovery));
                r.result.and_then(|()|r.stashes.map(|_|())).and_then(|()|r.status.map(|_|()))
            }
            Self::SubmoduleBatch(p) => {
                let r=repo.apply_submodule_batch(p,c)?;let mut result=Ok(());
                out.summary="Reviewed submodule updates finished.".into();
                for (path,entry) in r.results {let done=entry.and_then(|r|r.result.and(r.verification));out.details.push(format!("{}: {}",path.display_lossy(),if done.is_ok(){"updated and verified"}else{"stopped; inspect this checkout"}));if let Err(e)=done{result=Err(e);}}
                if !r.skipped.is_empty(){out.details.push(format!("{} remaining updates were not started.",r.skipped.len()));if result.is_ok(){result=Err(Error::Cancelled);}}result
            }
            Self::Branch(p) => {
                let r = repo.apply_branch_workflow(p, c)?;
                out.details.push(format!("Last phase: {}. Journal: {}. Recovery namespace: {}. Verify its feature/base refs before restoring; an interrupted backup may not have completed.", r.phase, r.journal.display(), r.recovery_prefix));
                out.details.push(format!("Original checkout: {}. Base: {}. Both branches are retained.", p.feature(), p.base()));
                if let Some(stash)=&r.recovery_stash {
                    out.details.push(format!("Recovery stash retained: {stash}. If Sync paused, finish or abort the rebase before inspecting and applying this stash. It is never dropped automatically."));
                }
                if let Some(journal)=&r.saved_work_journal { out.details.push(format!("Saved-work journal: {}.",journal.display())); }
                if let Ok(head) = &r.head { out.details.push(match head {
                    Head::Branch { name, target } => format!("Observed checkout: {} at {}.", String::from_utf8_lossy(name), target.as_str()),
                    Head::Detached { target } => format!("Observed checkout: detached at {}.", target.as_str()),
                    Head::Unborn { branch } => format!("Observed checkout: unborn {}.", String::from_utf8_lossy(branch)),
                }); }
                out.details.push("If Git paused, use Changes to continue or abort. After aborting a finish merge, switch back to the original feature branch if desired. Do not reset over local edits.".into());
                out.summary = if p.kind() == BranchWorkflowKind::Sync {
                    if p.options().save_changes {"Feature synchronized; tracked edits and staging restored. Recovery stash kept when created."} else {"Feature branch synchronized with its reviewed local base."}
                } else if p.commits()==0 {"Base already contains the feature; base checked out, feature kept."}
                else if p.options().allow_fast_forward && p.can_fast_forward() {"Base fast-forwarded to the reviewed feature; feature branch kept."}
                else {"Feature merged into its reviewed local base; feature branch kept."}.into();
                r.result.and_then(|()| r.state.map(|_|())).and_then(|()| r.head.map(|_|()))
            }
            Self::Prune(p) => {
                let r = repo.prune_tracking_refs(p, selected, c)?;
                if let Some(path) = r.recovery {
                    out.details.push(format!("Recovery ref prefix: {path}. The ref transaction creates backups and deletes selected tracking refs together."));
                }
                out.summary =
                    "Selected tracking refs pruned. Local branches and files kept.".into();
                r.result
            }
            Self::Delete(p) => {
                let r = repo.delete_remote_branch(p, c)?;
                out.details.push(
                    match r.remote_deleted {
                        Some(true) => "Observed remote branch: absent.",
                        Some(false) => "Observed remote branch: still present.",
                        None => {
                            "Remote result is unknown. Refresh before deciding whether to retry."
                        }
                    }
                    .into(),
                );
                out.summary = "Remote branch deletion confirmed; local branch kept.".into();
                r.result.and_then(|()|if r.remote_deleted==Some(true){Ok(())}else{Err(Error::InvalidInput("remote_branch_unconfirmed: deletion could not be verified; do not retry blindly.".into()))})
            }
            Self::Rename(p) => {
                let backup = repo.rename_stash(p, c)?;
                out.details.push(format!(
                    "Previous stash metadata retained at {}.",
                    backup.display()
                ));
                out.summary =
                    "Stash renamed. Commit, position, index and working files kept.".into();
                Ok(())
            }
            Self::Saved(p) => {
                let r = repo.apply_saved_work(p, c)?;
                let switching=matches!(p.action(),SavedWorkAction::Switch(_));
                out.details.push(format!(
                    "Last recovery phase: {}. Journal: {}.",
                    r.phase,
                    r.journal.display()
                ));
                if let Some(id) = r.stash {
                    out.details.push(format!("Recovery stash retained: {id}. Inspect Stashes before applying or dropping it. If integration paused, finish or abort it before reapplying."));
                    out.summary = if switching {"Branch switched and tracked changes restored with staging. Recovery stash retained."} else {"Integration finished and saved changes reapplied. Recovery stash retained."}.into();
                } else {
                    out.summary = if switching {"Branch switched. No tracked changes needed saving; untracked files kept."} else {"Integration finished. No tracked changes needed saving; no new stash was created."}.into();
                }
                r.result
            }
            Self::Lfs(p) => {
                let r = repo.apply_lfs_prune(p, c)?;
                out.details.push(format!("Removed {} verified cache objects ({} bytes). Remaining objects and working files kept.",r.removed,r.removed_bytes));
                out.summary = "Reviewed LFS cache cleanup finished.".into();
                r.result
            }
        };
        out.command_succeeded = result.is_ok();
        if let Err(error) = result {
            out.recovery_pending = true;
            if !matches!(self,Self::CommitPush(_)) { out.summary =
                "Operation stopped or needs review. Inspect the observed state before retrying."
                    .into(); }
            out.details.push(user_error(&error));
        }
        Ok(out)
    }
}
#[derive(Default)]
pub struct DialogState {
    pub stash_apply: Option<crate::stash_apply::State>,
    /// 0 prune refs, 1 delete remote branch, 2 rename stash, 3 pull/reapply,
    /// 4 rebase/reapply, 5 prune LFS cache.
    pub mode: usize,
    pub stash: usize,
    pub stashes: Vec<StashEntry>,
    pub strategy: usize,
    pub include_untracked: bool,
    pub save_changes: bool,
    pub allow_fast_forward: bool,
    pub reveal_mode: bool,
    pub preview: Option<Arc<Review>>,
    pub selected: Vec<Vec<u8>>,
    pub task: Option<ReadTask<Review>>,
    pub applying: bool,
    pub stopping: bool,
    pub notice: Option<String>,
    /// The Apply button's label from the last apply, for the footer's "Done" line.
    /// Cleared by a new review or a changed input.
    pub applied: Option<String>,
}
impl DialogState {
    pub fn running(&self) -> bool {
        self.task.is_some() || self.applying || self.stash_apply.as_ref().is_some_and(|s| s.task.is_some())
    }
}

/// The operations by `mode`: name, one line for the list and the line under the name.
/// What is checked again and kept lives in the guide.
const MODES: [(&str, &str, &str); 18] = [
    (
        "Tracking refs",
        "Remove stale remote refs",
        "Remote refs whose branch is gone from the server. Their tips are kept under refs/rocket/remote-prune.",
    ),
    (
        "Remote branch",
        "Delete one branch on a remote",
        "Deletes one branch on the remote, only if its tip is still the one you reviewed. Local branches stay.",
    ),
    (
        "Rename stash",
        "Give a stash a clearer name",
        "Changes the stash name only; its changes and position stay.",
    ),
    (
        "Stash + pull",
        "Save, pull, then reapply",
        "Saves tracked changes, pulls, then reapplies them with staging restored. The recovery stash is kept.",
    ),
    (
        "Stash + rebase",
        "Save, rebase, then reapply",
        "Saves tracked changes, rebases on top of a branch or commit, then reapplies them. The recovery stash is kept.",
    ),
    (
        "LFS cache",
        "Free space from old LFS objects",
        "Deletes local LFS objects the remote already has. Current, stashed, recent and unpushed objects stay.",
    ),
    ("Sync branch", "Rebase on top of a local base", "Rebases the feature on top of a reviewed local base. Optionally save tracked changes and restore staging afterward. Recovery refs and any saved stash are kept. Fetch separately."),
    ("Finish branch", "Merge into a local base", "Switches to the reviewed base and integrates the feature. Choose a merge commit or allow fast-forward. A clean checkout is required; both branches stay. Publishing and deletion are separate."),
    ("Fast-forward branches", "Update several local branches", "Fetch first, then review local upstream tips. Checked-out and diverged branches are blocked. Selected refs update atomically with recovery refs."),
    ("Rename remote", "Keep URLs and tracking relationships", "Review the new name. Config and tracking tips are backed up before Git renames the remote. No network access."),
    ("Pop stash", "Apply, then remove on success", "Requires a clean checkout. Conflicts, cancellation or a changed list retain the stash; recovery refs are kept."),
    ("Clear stashes", "Remove reviewed entries with recovery", "Clears the reviewed stash list after retaining every object under refs/rocket/stash. Working files and staging stay."),
    ("Update submodules", "Review several clean checkouts", "Enter one submodule path per line. All selections are checked before updates begin; a failure or cancellation stops remaining updates. Review parents and children separately."),
    ("Stash snapshot", "Save without cleaning local work", "Save staged and working versions separately. Working files and the index stay unchanged; a stash entry and recovery ref are added. Normal files only, up to 256 files / 64 MiB (16 MiB per file)."),
    ("Ignore tracked files", "Keep local and stop tracking", "Append exact root ignore rules and stage removal of reviewed files. Local copies and unrelated staging stay. Selected staged changes are blocked. The root .gitignore is not staged automatically."),
    ("Selected stash", "", ""), // mode 15 has its own existing dialog.
    ("Commit & Push", "Commit, then publish", "Commit only the reviewed staging, then push the new commit normally. Rejection or cancellation keeps the local commit. Upstream settings stay unchanged."),
    ("Save + switch", "Save, switch, then restore", "Save tracked changes, switch to an existing local branch, then restore staging and working changes. The recovery stash is kept. Untracked and ignored files stay in place; overwriting them is refused."),
];
/// The list's groups: what the operations touch.
const GROUPS: [(&str, &[usize]); 4] = [
    ("Branches", &[17, 6, 7, 8]),
    ("Remote", &[16, 0, 1, 9]),
    ("Saved work", &[13, 2, 3, 4, 10, 11]),
    ("Storage", &[14, 5,12]),
];

impl Rocket {
    pub(crate) fn can_branch_workflow(&self) -> bool {
        self.can_write()
            && self.tab().and_then(|t| t.snapshot.as_ref()).is_some_and(|s| {
                matches!(s.head, Some(Head::Branch { .. }))
                    && s.state.as_ref().is_some_and(OperationState::is_idle)
            })
    }

    pub(crate) fn open_branch_workflow(&mut self, kind: BranchWorkflowKind) {
        if self.dialog.is_some() || !self.can_branch_workflow() {
            return;
        }
        // Opening the menu only selects an existing workflow. The user still
        // chooses its local base and reviews the sealed preview before writing.
        self.dispatch(Intent::Dialog(DialogKind::Workflows));
        if let Some(d) = &mut self.dialog && let Some(s) = &mut d.workflow {
            s.mode = match kind {
                BranchWorkflowKind::Sync => 6,
                BranchWorkflowKind::Finish => 7,
            };
            s.reveal_mode = true;
        }
    }

    pub(crate) fn open_commit_switch(&mut self, target:Option<String>) {
        if self.dialog.is_some() || !self.can_write() {return;}
        let draft=self.tab().map(|t|t.draft.clone()).unwrap_or_default();
        let branch=self.tab().and_then(|t|t.snapshot.as_ref()).and_then(|s|s.head.as_ref()).and_then(|h|match h {Head::Branch{name,..}|Head::Unborn{branch:name}=>Some(String::from_utf8_lossy(name).into_owned()),_=>None}).unwrap_or_default();
        self.dispatch(Intent::Dialog(DialogKind::Workflows));
        if let Some(d)=&mut self.dialog && let Some(s)=&mut d.workflow {
            s.mode=if target.is_some(){17}else{16};s.reveal_mode=true;
            d.first=target.unwrap_or(branch);d.second=draft;
        }
    }
    pub(crate) fn open_keep_local(&mut self, snapshot:bool, paths:Vec<RepoPath>) {
        if self.dialog.is_some() || !self.can_write() { return; }
        self.dispatch(Intent::Dialog(DialogKind::Workflows));
        self.start_workflow_dialog();
        if let Some(d)=&mut self.dialog
            && matches!(d.kind,DialogKind::Workflows)
            && let Some(s)=&mut d.workflow {
            s.mode=if snapshot{13}else{14};
            s.reveal_mode=true;
            d.first=if snapshot{"Rocket snapshot".into()}else{paths.iter().map(|p|p.display_lossy()).collect::<Vec<_>>().join("\n")};
        }
    }
    pub(crate) fn start_workflow_dialog(&mut self) {
        if !self
            .dialog
            .as_ref()
            .is_some_and(|d| matches!(d.kind, DialogKind::Workflows))
        {
            return;
        }
        let stashes = self
            .tab()
            .and_then(|t| t.snapshot.as_ref())
            .map(|s| s.stashes.clone())
            .unwrap_or_default();
        if let Some(d) = &mut self.dialog {
            d.workflow = Some(DialogState {
                stashes,
                ..Default::default()
            });
        }
    }
    pub(crate) fn start_workflow_review(&mut self) {
        let path = self.tab().map(|t| t.path.clone());
        let backend = self.backend.clone();
        let scope = self.lfs_source_scope.clone();
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.workflow else { return };
        if s.running() {
            return;
        }
        s.preview = None;
        s.selected.clear();
        s.notice = None;
        d.error = None;
        let remote = d.remotes.get(d.remote).cloned().unwrap_or_default();
        let request = match s.mode {
            16 => Request::CommitPush(d.second.clone(),remote,d.first.as_bytes().to_vec()),
            17 => Request::Saved(SavedWorkAction::Switch(d.first.clone())),
            15 => {
                let Some(saved) = &s.stash_apply else { return; };
                let Some(catalog) = &saved.review else { return; };
                Request::StashApply(catalog.clone(), saved.selection())
            }
            13 => Request::StashSnapshot(d.first.clone(),s.include_untracked),
            14 => {
                let paths=d.first.lines().filter(|s|!s.is_empty()).map(|s|RepoPath::new(s.trim_end_matches('/').as_bytes().to_vec())).collect::<rocket_git::Result<Vec<_>>>();
                match paths{Ok(paths)=>Request::IgnoreTracked(paths),Err(e)=>{d.error=Some(user_error(&e));return;}}
            }
            8 => Request::FastForward,
            9 => Request::RenameRemote(remote,d.first.trim().into()),
            10 => {let Some(entry)=s.stashes.get(s.stash)else{d.error=Some("Choose a stash first.".into());return;};Request::StashControl(StashControl::Pop(entry.clone()))},
            11 => Request::StashControl(StashControl::Clear),
            12 => {let paths=d.first.lines().filter(|s|!s.trim().is_empty()).map(|s|RepoPath::new(s.trim().as_bytes().to_vec())).collect::<rocket_git::Result<Vec<_>>>();match paths{Ok(paths)=>Request::SubmoduleBatch(paths,self.submodule_fixture_root.clone()),Err(e)=>{d.error=Some(user_error(&e));return;}}},
            0 => Request::Prune(remote),
            1 => Request::Delete(remote, d.first.as_bytes().to_vec()),
            2 => {
                let Some(entry) = s.stashes.get(s.stash) else {
                    d.error = Some("Create a stash first, then reopen this dialog.".into());
                    return;
                };
                Request::Rename(entry.clone(), d.first.clone())
            }
            3 => Request::Saved(SavedWorkAction::Pull(match s.strategy {
                1 => PullStrategy::Merge,
                2 => PullStrategy::Rebase,
                _ => PullStrategy::FastForward,
            })),
            4 => Request::Saved(SavedWorkAction::Rebase(d.first.clone())),
            6 => Request::Branch(BranchWorkflowKind::Sync, d.first.trim().into(), BranchWorkflowOptions{save_changes:s.save_changes,..Default::default()}),
            7 => Request::Branch(BranchWorkflowKind::Finish, d.first.trim().into(), BranchWorkflowOptions{allow_fast_forward:s.allow_fast_forward,..Default::default()}),
            _ => Request::Lfs(
                String::from_utf8_lossy(&remote).into_owned(),
                scope,
            ),
        };
        if let (Some(path), Some(backend)) = (path, backend) {
            match ReadTask::spawn(move |c| backend.workflow_preview(&path, request, &c)) {
                Ok(t) => s.task = Some(t),
                Err(e) => d.error = Some(user_error(&e)),
            }
        }
    }
    pub(crate) fn poll_workflow(&mut self) {
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.workflow else { return };
        if let Some(saved) = &mut s.stash_apply
            && let Some(result) = saved.task.as_mut().and_then(ReadTask::try_finish) {
            saved.task = None;
            match result {
                Ok(review) if !s.stopping => saved.review = Some(review),
                Ok(_) | Err(Error::Cancelled) => s.notice = Some("Reading saved changes stopped. Close and reopen to retry.".into()),
                Err(error) => d.error = Some(user_error(&error)),
            }
            s.stopping = false;
        }
        if let Some(r) = s.task.as_mut().and_then(ReadTask::try_finish) {
            s.task = None;
            let r = if s.stopping { Err(Error::Cancelled) } else { r };
            s.stopping = false;
            match r {Ok(p)=>{s.preview=Some(Arc::new(p));},Err(Error::Cancelled)=>s.notice=Some(if s.mode==15{"Review cancelled. No saved changes were applied."}else if matches!(s.mode,13|14){"Review cancelled. No snapshot, ignore-rule or index changes were applied."}else{"Review cancelled. No cleanup or integration was applied. A pull review may already have fetched remote refs."}.into()),Err(e)=>d.error=Some(user_error(&e))}
        }
    }
    pub(crate) fn finish_workflow_dialog(&mut self, owner: Option<u64>, out: &Outcome) {
        let Some(d) = &mut self.dialog else { return };
        if d.owner != owner {
            return;
        }
        let Some(s) = &mut d.workflow else { return };
        if !s.applying {
            return;
        }
        s.applying = false;
        s.stopping = false;
        // The dialog stays open after Apply. Keep its observed entry in sync
        // with our completed rename so a second review doesn't use the old name.
        if out.command_succeeded
            && let Some(Review::Rename(p)) = s.preview.as_deref()
            && let Some(entry) = s.stashes.iter_mut().find(|entry| **entry == *p.entry())
        {
            entry.subject = p.name().as_bytes().to_vec();
        }
        s.preview = None;
        s.selected.clear();
        // Applied stash changes are done; the selection starts empty again.
        if out.command_succeeded
            && let Some(a) = &mut s.stash_apply
        {
            a.chosen.clear();
        }
        let message = format!("{}\n{}", out.summary, out.details.join("\n"));
        if out.command_succeeded {
            s.notice = Some(message);
            d.error = None
        } else {
            d.error = Some(message);
            s.notice = None
        }
    }
    /// Operations grouped on the left, the chosen one on the right with its inputs and
    /// review, and a footer whose buttons keep one order: Review (again), then the
    /// reviewed operation by name. Stop shows only while something runs.
    pub(crate) fn workflow_dialog(&mut self, ctx: &Context) {
        if self.dialog.as_ref().and_then(|d|d.workflow.as_ref()).is_some_and(|s|s.mode==15) {
            self.stash_apply_dialog(ctx);
            return;
        }
        let writable = self.can_write();
        let idle = self.write.is_none();
        let Some(mut d) = self.dialog.take() else {
            return;
        };
        let Some(s) = &mut d.workflow else {
            self.dialog = Some(d);
            return;
        };
        if d.owner != self.tab().map(|t| t.id) && !s.applying {
            return;
        }
        let busy = s.running();
        let (mut close, mut review, mut apply, mut cancel, mut changed) =
            (false, false, false, false, false);
        let mut applied = None;
        let write = self.write.as_ref().filter(|w| w.owner == d.owner);
        let widgets = &mut self.frame_widgets;
        let width = (ctx.content_rect().width() - 48.0).clamp(480.0, 900.0);
        let height = (ctx.content_rect().height() - 96.0).clamp(360.0, 560.0);
        let response = Modal::new(Id::new("rocket-workflows"))
            .backdrop_color(Color32::from_black_alpha(120))
            .frame(pages::modal_frame())
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.set_height(height);
                let outer = ui.max_rect();
                let header = Rect::from_min_size(outer.min, vec2(outer.width(), 74.0));
                let footer =
                    Rect::from_min_max(pos2(outer.left(), outer.bottom() - 56.0), outer.max);
                let nav = Rect::from_min_max(
                    pos2(outer.left(), header.bottom()),
                    pos2(outer.left() + (width * 0.27).clamp(170.0, 236.0), footer.top()),
                );
                let main = Rect::from_min_max(
                    pos2(nav.right(), header.bottom()),
                    pos2(outer.right(), footer.top()),
                );
                let line = Stroke::new(1.0, HAIRLINE());
                ui.painter().rect_filled(nav, 0.0, NAV());
                ui.painter().vline(nav.right() - 0.5, nav.y_range(), line);
                ui.painter()
                    .hline(header.x_range(), header.bottom() - 0.5, line);
                ui.painter().rect_filled(
                    footer,
                    CornerRadius { nw: 0, ne: 0, sw: 11, se: 11 },
                    FOOTER(),
                );
                ui.painter().hline(footer.x_range(), footer.top(), line);

                // Header: what the dialog is for, the guide and close.
                let inner =
                    Rect::from_min_max(header.min + vec2(20.0, 18.0), header.max - vec2(16.0, 12.0));
                ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
                    ui.horizontal_top(|ui| {
                        let (badge, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
                        ui.painter().rect_filled(badge, 8.0, ACCENT_SOFT());
                        icon(ui.painter(), badge.shrink(9.0), Icon::Branch, BLUE());
                        ui.add_space(6.0);
                        ui.vertical(|ui| {
                            ui.set_max_width(ui.available_width() - 70.0);
                            ui.spacing_mut().item_spacing.y = 3.0;
                            ui.label(RichText::new("Branch workflows & cleanup").font(semibold(16.0)));
                            ui.add(
                                Label::new(
                                    RichText::new(
                                        "Sync or finish a branch, manage saved work and clean up refs. Review before applying.",
                                    )
                                    .color(MUTED()),
                                )
                                .truncate(),
                            );
                        });
                        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                            let x = crate::chrome::icon_button(ui, Icon::Close, !s.applying, false, 28.0)
                                .on_hover_text("Close (Esc)");
                            control(widgets, "dialog-close", &x, ui.clip_rect());
                            // Automation's old id for the footer's Close.
                            control(widgets, "dialog-cancel", &x, ui.clip_rect());
                            close |= x.clicked();
                            if crate::guide::available(ui.ctx()) {
                                crate::guide::header_button(ui);
                            }
                        });
                    });
                });

                // The operations, grouped by what they touch.
                ui.scope_builder(UiBuilder::new().max_rect(nav), |ui| {
                    ScrollArea::vertical()
                        .id_salt("workflow-nav")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            Frame::new().inner_margin(Margin::symmetric(10, 10)).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.spacing_mut().item_spacing.y = 2.0;
                                // Each operation's line only while the list has room for it.
                                let lines = width >= 640.0;
                                ui.add_enabled_ui(!busy, |ui| {
                                    for (group, modes) in GROUPS {
                                        ui.add_space(6.0);
                                        ui.horizontal(|ui| {
                                            ui.add_space(10.0);
                                            ui.label(
                                                RichText::new(group.to_uppercase())
                                                    .font(semibold(11.0))
                                                    .color(HINT()),
                                            );
                                        });
                                        ui.add_space(2.0);
                                        for &i in modes {
                                            let (title, about, _) = MODES[i];
                                            let r = nav_row(ui, title, if lines { about } else { "" }, s.mode == i);
                                            if s.reveal_mode && s.mode==i {r.scroll_to_me(Some(Align::Center));s.reveal_mode=false;}
                                            control(widgets, format!("workflow-mode-{i}"), &r, ui.clip_rect());
                                            if r.clicked() && s.mode != i {
                                                s.mode = i;
                                                changed = true;
                                            }
                                        }
                                    }
                                });
                            });
                        });
                });

                // The chosen operation: its inputs, then what the review found.
                pages::region(widgets,"dialog-workflow-nav",nav,true);
                ui.scope_builder(UiBuilder::new().max_rect(main), |ui| {
                    let scroll = ScrollArea::vertical()
                        .id_salt("workflow-main")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            Frame::new()
                                .inner_margin(Margin { left: 22, right: 22, top: 18, bottom: 18 })
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.spacing_mut().item_spacing.y = 6.0;
                                    let (title, _, about) = MODES[s.mode.min(MODES.len() - 1)];
                                    ui.label(RichText::new(title).font(semibold(15.0)));
                                    let size = TextStyle::Body.resolve(ui.style()).size;
                                    pages::with_guide_link(ui, &[about], size, MUTED(), widgets, "workflow-guide");
                                    ui.add_space(8.0);
                                    ui.add_enabled_ui(!busy, |ui| {
                                        if matches!(s.mode, 0 | 1 | 5 | 9 | 16) {
                                            input_label(ui, "Remote");
                                            ui.horizontal_wrapped(|ui| {
                                                ui.spacing_mut().item_spacing.x = 6.0;
                                                for (i, name) in d.remotes.iter().enumerate() {
                                                    let r = chip(ui, &String::from_utf8_lossy(name), d.remote == i);
                                                    control(widgets, format!("workflow-remote-{i}"), &r, ui.clip_rect());
                                                    if r.clicked() && d.remote != i {
                                                        d.remote = i;
                                                        changed = true;
                                                    }
                                                }
                                                if d.remotes.is_empty() {
                                                    ui.label(RichText::new("This repository has no remotes.").color(HINT()));
                                                }
                                            });
                                            ui.add_space(8.0);
                                        }
                                        if matches!(s.mode,2|10) {
                                            input_label(ui, "Stash");
                                            let shown = s.stashes.len().min(100);
                                            if shown == 0 {
                                                ui.label(
                                                    RichText::new("No stashes. Save one first, then open this dialog again.")
                                                        .color(HINT()),
                                                );
                                            } else {
                                                Frame::new()
                                                    .stroke(Stroke::new(1.0, CARD_STROKE()))
                                                    .corner_radius(8)
                                                    .show(ui, |ui| {
                                                        ui.set_width(ui.available_width());
                                                        ui.spacing_mut().item_spacing.y = 0.0;
                                                        for (i, entry) in s.stashes.iter().enumerate().take(shown) {
                                                            let r = stash_row(ui, entry, s.stash == i, i + 1 < shown);
                                                            control(widgets, format!("workflow-stash-{i}"), &r, ui.clip_rect());
                                                            if r.clicked() && s.stash != i {
                                                                s.stash = i;
                                                                changed = true;
                                                            }
                                                        }
                                                    });
                                            }
                                            ui.add_space(8.0);
                                        }
                                        if s.mode == 3 {
                                            input_label(ui, "Pull with");
                                            let items = [
                                                ("Fast-forward only", s.strategy == 0),
                                                ("Merge", s.strategy == 1),
                                                ("Rebase", s.strategy == 2),
                                            ];
                                            let ids: Vec<String> =
                                                (0..items.len()).map(|i| format!("workflow-strategy-{i}")).collect();
                                            ui.horizontal(|ui| {
                                                if let Some(i) = crate::preferences::choices(ui, widgets, &items, &ids)
                                                    && i != s.strategy
                                                {
                                                    s.strategy = i;
                                                    changed = true;
                                                }
                                            });
                                            ui.add_space(8.0);
                                        }
                                        if matches!(s.mode, 1 | 2 | 4 | 6 | 7 | 9 | 12 | 13 | 14 | 16 | 17) {
                                            input_label(
                                                ui,
                                                match s.mode {
                                                    16 => "Destination branch on the remote",
                                                    17 => "Existing local branch to switch to",
                                                    6 | 7 => "Local base branch (for example main)",
                                                    9 => "New remote name",
                                                    12 => "Submodule paths (one per line)",
                                                    13 => "Snapshot name",
                                                    14 => "Files or folders to stop tracking (one per line)",
                                                    1 => "Branch on the remote",
                                                    2 => "New name",
                                                    _ => "Target branch or commit",
                                                },
                                            );
                                            let mut edit = (if matches!(s.mode,12|14) {TextEdit::multiline(&mut d.first).desired_rows(4)}else{TextEdit::singleline(&mut d.first)})
                                                .desired_width(ui.available_width().min(420.0));
                                            // Ref names in monospace; a stash name is prose.
                                            if !matches!(s.mode,2|13) {
                                                edit = edit.font(TextStyle::Monospace);
                                            }
                                            let r = ui.add(edit);
                                            control(widgets, "dialog-first", &r, ui.clip_rect());
                                            changed |= r.changed();
                                            ui.add_space(8.0);
                                        }
                                        if s.mode==6 {
                                            let r=ui.checkbox(&mut s.save_changes,"Save tracked changes and restore after Sync");
                                            control(widgets,"workflow-save-changes",&r,ui.clip_rect());changed|=r.changed();
                                            if s.save_changes {ui.add(Label::new("Staging is restored and the recovery stash is kept. Untracked and ignored files stay in place; overwriting them is refused.").wrap());}
                                        }
                                        if s.mode==7 {
                                            let r=ui.radio_value(&mut s.allow_fast_forward,false,"Always create a merge commit");
                                            control(widgets,"workflow-finish-merge",&r,ui.clip_rect());changed|=r.changed();
                                            let r=ui.radio_value(&mut s.allow_fast_forward,true,"Allow fast-forward when possible");
                                            control(widgets,"workflow-finish-ff",&r,ui.clip_rect());changed|=r.changed();
                                        }
                                        if s.mode==16 {
                                            input_label(ui,"Commit message (staged changes only)");
                                            let r=ui.add(TextEdit::multiline(&mut d.second).desired_rows(3).desired_width(ui.available_width()));
                                            control(widgets,"workflow-message",&r,ui.clip_rect());changed|=r.changed();
                                        }
                                        if s.mode==13 {
                                            let r=ui.checkbox(&mut s.include_untracked,"Include untracked files");
                                            control(widgets,"workflow-untracked",&r,ui.clip_rect());changed|=r.changed();
                                        }
                                    });
                                    if !changed && let Some(p) = s.preview.clone() {
                                        reviewed(ui, &p, &mut s.selected, widgets, busy);
                                    }
                                    if let Some(error) = &d.error {
                                        pages::callout(ui, RED(), error)
                                    }
                                    if let Some(n) = &s.notice {
                                        pages::callout(ui, BLUE(), n)
                                    }
                                });
                        });
                    pages::region(
                        widgets,
                        "dialog-workflow-scroll",
                        scroll.inner_rect.intersect(ui.clip_rect()),
                        !s.applying,
                    );
                });

                // Footer: where things stand, then the buttons in a fixed order.
                let ready = !changed && s.preview.is_some();
                let (working, status) = if s.task.is_some() {
                    let text = if s.stopping {
                        "Stopping the review"
                    } else {
                        "Reviewing targets and checking the current state"
                    };
                    (true, text.to_owned())
                } else if s.applying {
                    let progress = write.map(|w| w.progress.clone()).filter(|p| !p.is_empty());
                    let text = if s.stopping {
                        "Stopping the operation".to_owned()
                    } else {
                        progress.unwrap_or_else(|| "Applying the reviewed operation".to_owned())
                    };
                    (true, text)
                } else if ready {
                    (false, "Reviewed · nothing has changed yet".to_owned())
                } else if d.error.is_some() {
                    (false, "Nothing else changes until you review again".to_owned())
                } else if let Some(label) = &s.applied {
                    (false, format!("Done · {} finished", lower_first(label)))
                } else {
                    (false, "Nothing changes until you review and apply.".to_owned())
                };
                ui.scope_builder(
                    UiBuilder::new()
                        .max_rect(footer.shrink2(vec2(20.0, 12.0)))
                        .layout(Layout::right_to_left(Align::Center)),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        // Added right to left: Stop, the operation, Review.
                        if busy {
                            let r = pages::danger_ghost(ui, !s.stopping, "Stop");
                            control(widgets, "workflow-stop", &r, ui.clip_rect());
                            cancel = r.clicked();
                        } else {
                            if let Some(p) = s.preview.as_deref().filter(|_| ready) {
                                let (label, danger) = apply_label(s.mode, p, s.selected.len());
                                let targets = match p {
                                    Review::Prune(_) | Review::FastForward(_) => !s.selected.is_empty(),
                                    Review::Lfs(p) => !p.objects().is_empty(),
                                    _ => true,
                                };
                                let enabled = writable && idle && targets;
                                let r = if danger {
                                    pages::danger(ui, enabled, &label)
                                } else {
                                    pages::primary(ui, enabled, &label)
                                };
                                control(widgets, "dialog-submit", &r, ui.clip_rect());
                                if r.clicked() {
                                    apply = true;
                                    applied = Some(label);
                                }
                            }
                            let r = if ready {
                                ui.add_enabled(idle, Button::new("Review again"))
                            } else {
                                pages::primary(ui, idle, "Review")
                            };
                            control(widgets, "workflow-review", &r, ui.clip_rect());
                            review = r.clicked();
                        }
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            if working {
                                ui.add(Spinner::new().size(14.0));
                            }
                            ui.add(Label::new(RichText::new(status).size(12.5).color(HINT())).truncate());
                        });
                    },
                );
            });
        close |= response.should_close() && !s.applying;
        if changed {
            s.preview = None;
            s.notice = None;
            s.selected.clear();
            d.error = None;
        }
        if changed || review {
            s.applied = None;
        }
        if cancel {
            if s.applying {
                self.cancel_write()
            } else if let Some(t) = &s.task {
                t.cancel();
            }
            s.stopping = true;
            s.preview = None;
        }
        let action = if apply {
            s.applying = true;
            s.notice = None;
            s.applied = applied;
            d.error = None;
            s.preview.clone().map(|review| Action::Workflow {
                review,
                selected: s.selected.clone(),
            })
        } else {
            None
        };
        let owner = d.owner;
        if !close {
            self.dialog = Some(d)
        }
        if review {
            self.start_workflow_review()
        }
        if let (Some(action), Some(owner)) = (action, owner) {
            self.dispatch(Intent::Execute(owner, action));
            if self.write.is_none()
                && let Some(d) = &mut self.dialog
                && let Some(s) = &mut d.workflow
            {
                s.applying = false;
                d.error = Some(
                    "Could not start; wait for repository work to finish, then review again."
                        .into(),
                );
            }
        }
    }
}

/// One operation in the list: its name and, when there is room, one line on it.
pub(crate) fn nav_row(ui: &mut Ui, title: &str, about: &str, selected: bool) -> Response {
    let height = if about.is_empty() { 32.0 } else { 46.0 };
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    let enabled = ui.is_enabled();
    if selected {
        ui.painter().rect_filled(rect, 6.0, SELECTED());
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min + vec2(0.0, 8.0), vec2(2.5, rect.height() - 16.0)),
            1.0,
            BLUE(),
        );
    } else if enabled && (response.hovered() || response.has_focus()) {
        ui.painter().rect_filled(rect, 6.0, CHIP());
    }
    let ink = if selected || enabled { TEXT() } else { FAINT() };
    let font = if selected { semibold(13.0) } else { FontId::proportional(13.0) };
    let y = if about.is_empty() { rect.center().y } else { rect.top() + 15.0 };
    ui.painter()
        .text(pos2(rect.left() + 12.0, y), Align2::LEFT_CENTER, title, font, ink);
    if !about.is_empty() {
        let mut job = text::LayoutJob::simple_singleline(about.into(), FontId::proportional(12.0), HINT());
        job.wrap = text::TextWrapping::truncate_at_width(rect.width() - 22.0);
        let galley = ui.painter().layout_job(job);
        ui.painter().galley(pos2(rect.left() + 12.0, rect.top() + 24.0), galley, HINT());
    }
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, enabled, selected, title));
    response
}

/// The name of an input, above it.
fn input_label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(MUTED()));
}

/// A remote to act on, as a pill; the chosen one is filled.
fn chip(ui: &mut Ui, name: &str, selected: bool) -> Response {
    let edge = EDGE_ACCENT();
    let text = RichText::new(name).size(13.0).color(if selected { TEXT() } else { MUTED() });
    let r = ui.add(
        Button::new(text)
            .selected(selected)
            .fill(if selected { SELECTED() } else { Color32::TRANSPARENT })
            .stroke(Stroke::new(1.0, if selected { edge } else { BORDER() }))
            .corner_radius(14)
            .min_size(vec2(0.0, 28.0)),
    );
    if !selected && ui.is_enabled() && r.hovered() {
        ui.painter()
            .rect_stroke(r.rect, 14.0, Stroke::new(1.0, edge), StrokeKind::Inside);
    }
    r
}

/// A stash to rename, as a radio row.
fn stash_row(ui: &mut Ui, entry: &StashEntry, selected: bool, line: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
    let enabled = ui.is_enabled();
    if selected {
        ui.painter()
            .rect_filled(rect.shrink(1.0), 6.0, CHIP());
    } else if enabled && (response.hovered() || response.has_focus()) {
        ui.painter().rect_filled(rect.shrink(1.0), 6.0, HOVER());
    }
    let ring = pos2(rect.left() + 18.0, rect.center().y);
    let ring_color = if selected { BLUE() } else { DIM() };
    ui.painter()
        .circle_stroke(ring, 5.5, Stroke::new(2.0, ring_color));
    if selected {
        ui.painter().circle_filled(ring, 2.5, BLUE());
    }
    let name = format!("stash@{{{}}}", entry.index);
    let galley = ui
        .painter()
        .layout_no_wrap(name.clone(), FontId::monospace(12.0), FAINT());
    let left = rect.left() + 34.0;
    let subject_left = left + galley.size().x + 10.0;
    ui.painter()
        .galley(pos2(left, rect.center().y - galley.size().y / 2.0), galley, FAINT());
    let subject = String::from_utf8_lossy(&entry.subject).into_owned();
    let mut job = text::LayoutJob::simple_singleline(
        subject.clone(),
        FontId::proportional(13.0),
        if enabled { TEXT() } else { MUTED() },
    );
    job.wrap = text::TextWrapping::truncate_at_width((rect.right() - 12.0 - subject_left).max(0.0));
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(
        pos2(subject_left, rect.center().y - galley.size().y / 2.0),
        galley,
        TEXT(),
    );
    if line {
        ui.painter()
            .hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, CARD_LINE()));
    }
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::RadioButton, enabled, selected, format!("{name} · {subject}"))
    });
    response
}

/// What the review found: a header line, the exact targets, and what is kept.
fn reviewed(
    ui: &mut Ui,
    review: &Review,
    selected: &mut Vec<Vec<u8>>,
    widgets: &mut HashMap<String, UiControl>,
    busy: bool,
) {
    let short = |id: &ObjectId| id.as_str().chars().take(7).collect::<String>();
    let count = |n: usize, one: &str, many: &str| {
        if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
    };
    let lossy = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
    let (title, meta, note) = match review {
        Review::CommitPush(p) => (
            format!("Commit {}", count(p.staged_files(),"staged file","staged files")),
            format!("{} → {}/{}",lossy(p.branch()),lossy(p.remote()),lossy(p.destination())),
            format!("Destination: {}\nNormal push only. A rejected or cancelled push keeps the local commit; retry Push separately.",p.destination_display()),
        ),
        Review::StashSnapshot(p) => ("Save a stash snapshot".into(),format!("{} reviewed paths",p.files().len()),"Working files, staging and checkout are retained. External edits invalidate this review.".into()),
        Review::IgnoreTracked(p) => ("Keep local and stop tracking".into(),format!("{} reviewed files",p.files().len()),"Stage deletions, keep local files, append .gitignore rules. Index and rule recovery copies are retained.".into()),
        Review::StashSelection(p) => ("Save selected hunks".into(),format!("{} changed lines",p.selected_count()),"Unselected changes and staging are retained.".into()),
        Review::StashApply(p) => ("Apply selected saved changes".into(),format!("{} working files",p.files().len()),"The stash and index are retained; file recovery copies are kept.".into()),
        Review::FastForward(p) => ("Fast-forward selected branches".into(),format!("{} local branches",p.entries().len()),"Only eligible branches can be selected; no checkout or file changes.".into()),
        Review::RenameRemote(p) => (format!("{} → {}",lossy(&p.remote().name),lossy(p.name())),format!("{} upstream relationships",p.affected_branches().len()),"URLs remain unchanged; config and tracking tips are backed up.".into()),
        Review::StashControl(p) => (if matches!(p.action(),StashControl::Clear){"Clear reviewed stashes"}else{"Pop selected stash"}.into(),format!("{} entries guarded",p.entries().len()),"Recovery objects are retained. Conflicts or an interrupted pop keep the entry.".into()),
        Review::SubmoduleBatch(p) => ("Update reviewed submodules".into(),format!("{} checkouts",p.entries().count()),"Stops remaining updates on the first failure or cancellation. Dirty children are refused.".into()),
        Review::Branch(p) => (
            format!("{} → {}", p.feature(), p.base()),
            format!("{} commits · feature {} · base {}", p.commits(), short(p.tip()), short(p.base_tip())),
            if p.kind() == BranchWorkflowKind::Sync {
                if p.options().save_changes {format!("Save {} tracked files, sync, then restore staging and edits. Recovery stash retained. Conflicts pause before restore; no fetch or push.",p.saved_changes())}
                else {"Rewrites feature commits on the local base. Recovery references retained; no fetch or push.".into()}
            } else {
                let action=if p.commits()==0 {"Base already contains this feature."} else if p.options().allow_fast_forward && p.can_fast_forward() {"Base will fast-forward to the feature tip."} else {"A merge commit will be created."};
                format!("Switch to the base. {action} Both branches and recovery tips remain; conflicts stop for recovery.")
            },
        ),
        Review::Prune(p) => {
            let kept: Vec<String> = p.stale_local_branches().iter().map(|b| lossy(b)).collect();
            let title = match p.obsolete().len() {
                0 => "No obsolete tracking refs".to_owned(),
                n => count(n, "obsolete tracking ref", "obsolete tracking refs"),
            };
            let note = match kept.len() {
                0 => String::new(),
                1 => format!("Kept: local branch {} has no upstream any more; it is not deleted.", kept[0]),
                _ => format!(
                    "Kept: local branches {} have no upstream any more; they are not deleted.",
                    kept.join(", ")
                ),
            };
            (title, lossy(p.remote()), note)
        }
        Review::Delete(p) => (
            format!("{}/{}", lossy(p.remote()), lossy(p.branch())),
            format!("tip {}", short(p.expected())),
            format!("Deleted only if the remote still points at {}.", short(p.expected())),
        ),
        Review::Rename(p) => (
            format!("stash@{{{}}} → {}", p.entry().index, p.name()),
            short(&p.entry().id),
            String::new(),
        ),
        Review::Saved(p) => (
            match p.changed_files() {
                0 => "No tracked changes to save".to_owned(),
                n => format!("{} will be saved", count(n, "tracked file", "tracked files")),
            },
            format!("{} · {}", p.reference(), short(p.target())),
            "Untracked files stay in place.".to_owned(),
        ),
        Review::Lfs(p) => (
            match p.objects().len() {
                0 => "No cached objects to remove".to_owned(),
                n => format!(
                    "{} · {} to reclaim",
                    count(n, "object", "objects"),
                    crate::tree::size(p.bytes() as usize)
                ),
            },
            p.remote().to_owned(),
            if p.objects().len() > 100 {
                format!("First 100 of {} shown; the review covers all of them.", p.objects().len())
            } else {
                String::new()
            },
        ),
    };
    Frame::new()
        .stroke(Stroke::new(1.0, CARD_STROKE()))
        .corner_radius(8)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            Frame::new()
                .fill(CARD())
                .corner_radius(CornerRadius { nw: 7, ne: 7, sw: 0, se: 0 })
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let (mark, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                        ui.painter()
                            .circle_stroke(mark.center(), 6.0, Stroke::new(1.4, GREEN()));
                        icon(ui.painter(), mark.shrink(3.5), Icon::Check, GREEN());
                        ui.label(RichText::new(&title).font(semibold(13.0)));
                        ui.add(Label::new(RichText::new(&meta).size(12.0).color(HINT())).truncate());
                    });
                });
            match review {
                Review::FastForward(p) => {for (i,e) in p.entries().iter().enumerate(){card_row(ui,|ui|{
                    let mut yes=selected.contains(&e.branch);let r=ui.add_enabled(!busy&&e.blocked.is_none(),checkbox(&mut yes,short_ref(&e.branch)));control(widgets,format!("workflow-ref-{i}"),&r,ui.clip_rect());
                    if r.changed(){if yes{selected.push(e.branch.clone());}else{selected.retain(|n|n!=&e.branch);}}
                    ui.label(RichText::new(e.blocked.map(str::to_owned).unwrap_or_else(||format!("{} → {}",short(&e.before),e.after.as_ref().map(short).unwrap_or_default()))).size(12.0).color(HINT()));
                });}},
                Review::StashSnapshot(p) => {for f in p.files(){card_row(ui,|ui|{ui.label(format!("{} · {}",f.path().display_lossy(),if f.before().is_some(){"local copy kept"}else{"working deletion saved"}));});}},
                Review::IgnoreTracked(p) => {
                    for f in p.files(){card_row(ui,|ui|{ui.label(format!("{} · keep local, stage deletion",f.path().display_lossy()));});}
                    pages::caption(ui,"Reviewed .gitignore (existing content retained)");
                    ui.label(RichText::new(String::from_utf8_lossy(p.rules().after()).chars().take(6000).collect::<String>()).monospace());
                    if p.rules().after().len()>6000{pages::note(ui,"Preview text is capped at 6,000 characters; the complete reviewed file is saved.");}
                },
                Review::StashControl(p) => {for e in p.entries().iter().filter(|e|matches!(p.action(),StashControl::Clear)||matches!(p.action(),StashControl::Pop(selected) if selected==*e)){card_row(ui,|ui|{ui.label(format!("stash@{{{}}} · {} · {}",e.index,short(&e.id),lossy(&e.subject)));});}},
                Review::SubmoduleBatch(p) => {for e in p.entries(){card_row(ui,|ui|{ui.label(e.path.display_lossy());});}},
                Review::Prune(p) => {
                    for (i, entry) in p.obsolete().iter().enumerate() {
                        card_row(ui, |ui| {
                            let mut yes = selected.contains(&entry.reference);
                            let name = RichText::new(short_ref(&entry.reference)).monospace().size(12.5);
                            let r = ui.add_enabled(!busy, checkbox(&mut yes, name));
                            control(widgets, format!("workflow-ref-{i}"), &r, ui.clip_rect());
                            if r.changed() {
                                if yes {
                                    selected.push(entry.reference.clone())
                                } else {
                                    selected.retain(|n| n != &entry.reference)
                                }
                            }
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(RichText::new(short(&entry.id)).monospace().size(12.0).color(HINT()));
                            });
                        });
                    }
                }
                Review::Lfs(p) => {
                    for object in p.objects().iter().take(100) {
                        card_row(ui, |ui| {
                            let oid = &object.oid;
                            let name = if oid.len() > 12 {
                                format!("{}…{}", &oid[..8], &oid[oid.len() - 4..])
                            } else {
                                oid.clone()
                            };
                            ui.label(RichText::new(name).monospace().size(12.5))
                                .on_hover_text(oid);
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(
                                    RichText::new(crate::tree::size(object.size as usize))
                                        .monospace()
                                        .size(12.0)
                                        .color(HINT()),
                                );
                            });
                        });
                    }
                }
                _ => {}
            }
            if !note.is_empty() {
                card_row(ui, |ui| {
                    ui.add(Label::new(RichText::new(&note).size(12.5).color(MUTED())).wrap());
                });
            }
        });
}

/// A line of the review card, under a hairline.
fn card_row(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    let top = ui.cursor().top();
    ui.painter()
        .hline(ui.max_rect().x_range(), top, Stroke::new(1.0, CARD_LINE()));
    Frame::new()
        .inner_margin(Margin::symmetric(12, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.set_min_height(20.0);
                content(ui)
            });
        });
}

/// The Apply button names the reviewed operation; deletes are red.
fn apply_label(mode: usize, review: &Review, selected: usize) -> (String, bool) {
    match review {
        Review::CommitPush(_) => ("Commit & Push".into(),false),
        Review::Saved(p) if matches!(p.action(),SavedWorkAction::Switch(_)) => ("Save, switch & restore".into(),false),
        Review::StashSnapshot(_) => ("Save snapshot".into(),false),
        Review::IgnoreTracked(_) => ("Keep local and stop tracking".into(),false),
        Review::StashSelection(_) => ("Save selected hunks".into(),false),
        Review::StashApply(_) => ("Apply selected changes".into(),false),
        Review::FastForward(_) => (format!("Update {selected} branches"),false),
        Review::RenameRemote(_) => ("Rename remote".into(),false),
        Review::StashControl(p) => (if matches!(p.action(),StashControl::Clear){"Clear reviewed stashes"}else{"Pop stash"}.into(),matches!(p.action(),StashControl::Clear)),
        Review::SubmoduleBatch(_) => ("Update submodules".into(),false),
        Review::Branch(p) => ((if p.kind() == BranchWorkflowKind::Sync {if p.options().save_changes {"Save, sync & restore"} else {"Sync branch"}} else {"Finish branch"}).into(), false),
        Review::Prune(_) => (
            match selected {
                0 => "Remove tracking refs".to_owned(),
                1 => "Remove 1 tracking ref".to_owned(),
                n => format!("Remove {n} tracking refs"),
            },
            false,
        ),
        Review::Delete(p) => {
            let name = format!(
                "{}/{}",
                String::from_utf8_lossy(p.remote()),
                String::from_utf8_lossy(p.branch())
            );
            // A long name stays in the review card; the button keeps to one short line.
            let label = if name.chars().count() <= 32 {
                format!("Delete {name}")
            } else {
                "Delete remote branch".to_owned()
            };
            (label, true)
        }
        Review::Rename(_) => ("Rename stash".to_owned(), false),
        Review::Saved(_) if mode == 4 => ("Save, rebase and reapply".to_owned(), false),
        Review::Saved(_) => ("Save, pull and reapply".to_owned(), false),
        Review::Lfs(p) => (
            match p.objects().len() {
                0 => "Delete objects".to_owned(),
                1 => "Delete 1 object".to_owned(),
                n => format!("Delete {n} objects"),
            },
            true,
        ),
    }
}

fn lower_first(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map_or_else(String::new, |c| c.to_lowercase().chain(chars).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_common as common;
    use std::{
        thread,
        time::{Duration, Instant},
    };

    fn wait(app: &mut Rocket, ctx: &Context) {
        let deadline = Instant::now() + Duration::from_secs(45);
        while app.busy() {
            app.poll(ctx);
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn repeated_stash_rename_uses_the_updated_entry_without_reopening() {
        common::isolated(
            "ROCKET_WORKFLOW_UI_CASE",
            "workflows::tests::repeated_stash_rename_uses_the_updated_entry_without_reopening",
            || {
                let f = common::Fixture::new();
                f.write("文件.txt", "base\n");
                f.commit("base");
                f.write("文件.txt", "draft\n");
                f.run(&["stash", "push", "-m", "original"]);
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(
                    &ctx,
                    vec![f.repository.clone()],
                    f.root.join("logs"),
                );
                wait(&mut app, &ctx);
                app.auto_refresh = false;
                app.dispatch(Intent::Dialog(DialogKind::Workflows));
                let owner = app.dialog.as_ref().unwrap().owner;
                for name in ["保存 one", "保存 two"] {
                    let d = app.dialog.as_mut().unwrap();
                    d.first = name.into();
                    d.workflow.as_mut().unwrap().mode = 2;
                    app.start_workflow_review();
                    wait(&mut app, &ctx);
                    let d = app.dialog.as_mut().unwrap();
                    assert!(d.error.is_none(), "{:?}", d.error);
                    let state = d.workflow.as_mut().unwrap();
                    state.applying = true;
                    let result = state
                        .preview
                        .as_ref()
                        .unwrap()
                        .apply(&f.repo(), &[], &CancellationToken::new())
                        .unwrap();
                    assert!(
                        result.command_succeeded,
                        "{} {:?}",
                        result.summary, result.details
                    );
                    app.finish_workflow_dialog(owner, &result);
                    assert_eq!(
                        f.repo().stashes(&CancellationToken::new()).unwrap()[0].subject,
                        name.as_bytes()
                    );
                }
            },
        );
    }

    fn frame(ctx: &Context, app: &mut Rocket) {
        frame_with(ctx, app, vec![]);
    }

    fn frame_with(ctx: &Context, app: &mut Rocket, events: Vec<Event>) {
        app.frame_widgets.clear();
        let mut output = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 860.0))),
                events,
                ..Default::default()
            },
            |ui| {
                let ctx = ui.ctx().clone();
                app.dialog_ui(&ctx);
            },
        );
        output.textures_delta.clear();
    }

    fn click(ctx: &Context, app: &mut Rocket, id: &str) {
        let at = app.frame_widgets[id].rect.center();
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        frame_with(ctx, app, vec![Event::PointerMoved(at), button(true)]);
        frame_with(ctx, app, vec![button(false)]);
    }

    fn reveal_mode(ctx: &Context, app: &mut Rocket, mode: usize) {
        let id = format!("workflow-mode-{mode}");
        for step in 0..20 {
            if app.frame_widgets.get(&id).is_some_and(|w| w.rect.height() >= 20.0) {
                return;
            }
            let at = app.frame_widgets["dialog-workflow-nav"].rect.center();
            frame_with(ctx, app, vec![
                Event::PointerMoved(at),
                Event::MouseWheel {
                    unit: MouseWheelUnit::Point,
                    delta: vec2(0.0, if step == 0 { 2000.0 } else { -140.0 }),
                    phase: TouchPhase::Move,
                    modifiers: Modifiers::NONE,
                },
            ]);
            for _ in 0..12 { frame(ctx, app); }
        }
        panic!("{id} must be reachable by scrolling the workflow navigation");
    }

    #[test]
    fn the_buttons_keep_one_order_and_appear_only_when_they_can_act() {
        common::isolated(
            "ROCKET_WORKFLOW_UI_CASE",
            "workflows::tests::the_buttons_keep_one_order_and_appear_only_when_they_can_act",
            || {
                let f = common::Fixture::new();
                f.write("a.txt", "base\n");
                f.commit("base");
                f.write("a.txt", "draft\n");
                f.run(&["stash", "push", "-m", "original"]);
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(
                    &ctx,
                    vec![f.repository.clone()],
                    f.root.join("logs"),
                );
                wait(&mut app, &ctx);
                app.auto_refresh = false;
                app.dispatch(Intent::Dialog(DialogKind::Workflows));
                // A new modal is laid out for a frame before its controls are drawn.
                frame(&ctx, &mut app);
                frame(&ctx, &mut app);
                // The larger list scrolls independently of the inputs and fixed footer.
                let review_rect = app.frame_widgets["workflow-review"].rect;
                // Selected-stash mode 15 is contextual and uses its own dialog;
                // every ordinary workflow must still be reachable in this list.
                for i in (0..MODES.len()).filter(|i| *i != 15) {
                    reveal_mode(&ctx, &mut app, i);
                    assert_eq!(app.frame_widgets["workflow-review"].rect, review_rect);
                }
                assert!(!app.frame_widgets.contains_key("workflow-mode-15"));
                for id in ["dialog-close", "dialog-cancel", "workflow-review", "dialog-workflow-scroll"] {
                    assert!(app.frame_widgets.contains_key(id), "{id} is shown");
                }
                for id in ["dialog-submit", "workflow-stop"] {
                    assert!(!app.frame_widgets.contains_key(id), "no {id} before a review");
                }

                reveal_mode(&ctx, &mut app, 2);
                click(&ctx, &mut app, "workflow-mode-2");
                frame(&ctx, &mut app);
                assert_eq!(app.dialog.as_ref().unwrap().workflow.as_ref().unwrap().mode, 2);
                assert!(app.frame_widgets.contains_key("workflow-stash-0"));
                assert!(app.frame_widgets.contains_key("dialog-first"));
                app.dialog.as_mut().unwrap().first = "renamed".into();
                click(&ctx, &mut app, "workflow-review");
                wait(&mut app, &ctx);
                frame(&ctx, &mut app);
                let d = app.dialog.as_ref().unwrap();
                assert!(d.error.is_none(), "{:?}", d.error);
                assert!(d.workflow.as_ref().unwrap().preview.is_some());
                // Review again, then the reviewed operation, left to right; no Close
                // beside them: the old Close id is the ×.
                let x = |id: &str| app.frame_widgets[id].rect.center().x;
                assert!(x("workflow-review") < x("dialog-submit"));
                assert_eq!(app.frame_widgets["dialog-cancel"].rect, app.frame_widgets["dialog-close"].rect);
                assert!(app.frame_widgets["dialog-submit"].enabled);
                assert!(!app.frame_widgets.contains_key("workflow-stop"));

                // While the operation runs only Stop is offered, and × waits too.
                app.dialog.as_mut().unwrap().workflow.as_mut().unwrap().applying = true;
                frame(&ctx, &mut app);
                assert!(app.frame_widgets["workflow-stop"].enabled);
                for id in ["workflow-review", "dialog-submit"] {
                    assert!(!app.frame_widgets.contains_key(id), "no {id} while applying");
                }
                assert!(!app.frame_widgets["dialog-close"].enabled);
                assert!(!app.frame_widgets["dialog-cancel"].enabled);
                app.dialog.as_mut().unwrap().workflow.as_mut().unwrap().applying = false;

                frame(&ctx, &mut app);
                click(&ctx, &mut app, "dialog-close");
                assert!(app.dialog.is_none(), "× closes the dialog");

                // Mode 15 remains reachable through the stash's contextual
                // action, with its own selection/review controls.
                let stash = f.repo().stashes(&CancellationToken::new()).unwrap().remove(0);
                app.open_stash_apply(stash, StashPart::Tracked);
                wait(&mut app, &ctx);
                frame(&ctx, &mut app);
                frame(&ctx, &mut app);
                assert_eq!(app.dialog.as_ref().unwrap().workflow.as_ref().unwrap().mode, 15);
                for id in ["dialog-stash-file-0", "dialog-stash-show-0", "dialog-stash-all", "dialog-submit", "dialog-cancel"] {
                    assert!(app.frame_widgets.contains_key(id), "contextual stash control {id}");
                }
                assert!(!app.frame_widgets.contains_key("dialog-workflow-nav"));
                assert_eq!(std::fs::read(f.repository.join("a.txt")).unwrap(), b"base\n");

                // Choosing a file checks the selection at once; there is no review step.
                assert!(!app.frame_widgets["dialog-submit"].enabled, "nothing to apply before a choice");
                click(&ctx, &mut app, "dialog-stash-file-0");
                wait(&mut app, &ctx);
                frame(&ctx, &mut app);
                let d = app.dialog.as_ref().unwrap();
                assert!(d.error.is_none(), "{:?}", d.error);
                assert!(d.workflow.as_ref().unwrap().preview.is_some(), "checked without a review button");
                assert!(app.frame_widgets["dialog-submit"].enabled);
                assert!(!app.frame_widgets.contains_key("dialog-preview-refresh"));

                // A changed selection drops the old check at once; with nothing chosen
                // there is nothing to check or apply.
                click(&ctx, &mut app, "dialog-stash-file-0");
                frame(&ctx, &mut app);
                let s = app.dialog.as_ref().unwrap().workflow.as_ref().unwrap();
                assert!(s.preview.is_none() && s.task.is_none());
                assert!(!app.frame_widgets["dialog-submit"].enabled);

                // A single line can be chosen by its box, and is checked the same way.
                let line = app
                    .frame_widgets
                    .keys()
                    .filter(|k| k.starts_with("dialog-stash-line-0-"))
                    .min()
                    .cloned()
                    .expect("line boxes for the shown file");
                click(&ctx, &mut app, &line);
                wait(&mut app, &ctx);
                frame(&ctx, &mut app);
                let d = app.dialog.as_ref().unwrap();
                let s = d.workflow.as_ref().unwrap();
                assert_eq!(s.stash_apply.as_ref().unwrap().selection()[0].lines.len(), 1);
                assert!(s.preview.is_some(), "{:?}", d.error);
            },
        );
    }
}
