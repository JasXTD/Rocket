//! An explicit stash/integrate/reapply workflow. The stash is never dropped.
//! A bounded snapshot is rechecked before the first mutation. Cancellation and
//! failures retain both the stash and a small durable phase record for recovery.
use crate::{runner::args, *};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SavedWorkAction {
    Pull(PullStrategy),
    Rebase(String),
    /// Save tracked changes, switch to an existing local branch, restore staging.
    /// Untracked/ignored files stay in place and may block the switch.
    Switch(String),
}
#[derive(Clone, Debug)]
pub struct SavedWorkPreview {
    repository: PathBuf,
    action: SavedWorkAction,
    head: Head,
    snapshot: Vec<u8>,
    config: Vec<u8>,
    target: ObjectId,
    reference: String,
    changed: usize,
}
impl SavedWorkPreview {
    pub fn action(&self) -> &SavedWorkAction {
        &self.action
    }
    pub fn target(&self) -> &ObjectId {
        &self.target
    }
    pub fn reference(&self) -> &str {
        &self.reference
    }
    pub fn changed_files(&self) -> usize {
        self.changed
    }
}
#[derive(Debug)]
pub struct SavedWorkReport {
    pub result: Result<()>,
    pub stash: Option<ObjectId>,
    pub phase: &'static str,
    pub journal: PathBuf,
}
fn invalid(s: &str) -> Error {
    Error::InvalidInput(s.into())
}
impl Repository {
    fn saved_read(&self, command: &[&str], c: &CancellationToken) -> Result<Vec<u8>> {
        self.runner
            .run(self.cwd(), &args(command), vec![], c)?
            .success("saved-work")
    }
    fn saved_snapshot(&self, c: &CancellationToken) -> Result<(Head, Vec<u8>, usize)> {
        self.require_worktree(c)?;
        self.require_idle(c)?;
        let status = self.status(c)?;
        if !matches!(status.head, Head::Branch { .. }) {
            return Err(invalid(
                "saved_work_branch: select a committed local branch first.",
            ));
        }
        if status
            .entries
            .iter()
            .any(|e| e.kind == EntryKind::Unmerged || e.submodule.is_some())
        {
            return Err(invalid(
                "saved_work_conflicts: resolve conflicts and handle dirty submodules first.",
            ));
        }
        let index = self.saved_read(&["ls-files", "--stage", "-v", "-z"], c)?;
        if index
            .split(|b| *b == 0)
            .filter_map(|r| r.first())
            .any(|b| b.is_ascii_lowercase() || *b == b'S')
        {
            return Err(invalid(
                "saved_work_index: hidden index entries must be handled first.",
            ));
        }
        let mut snapshot = Vec::new();
        for bytes in [
            index,
            self.saved_read(
                &[
                    "diff",
                    "--binary",
                    "--full-index",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--no-renames",
                    "HEAD",
                    "--",
                ],
                c,
            )?,
            self.saved_read(
                &[
                    "diff",
                    "--cached",
                    "--binary",
                    "--full-index",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--no-renames",
                    "--",
                ],
                c,
            )?,
            self.saved_read(
                &["status", "--porcelain=v2", "-z", "--untracked-files=all"],
                c,
            )?,
        ] {
            if snapshot.len() + bytes.len() > 32 * 1024 * 1024 {
                return Err(invalid(
                    "saved_work_limit: the review exceeds 32 MiB; stash manually first.",
                ));
            }
            snapshot.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
            snapshot.extend_from_slice(&bytes);
        }
        let changed = status
            .entries
            .iter()
            .filter(|e| !matches!(e.kind, EntryKind::Untracked | EntryKind::Ignored))
            .count();
        Ok((status.head, snapshot, changed))
    }
    /// Pull reviews explicitly fetch first. Working files and staging are unchanged.
    pub fn preview_saved_work(
        &self,
        action: SavedWorkAction,
        c: &CancellationToken,
    ) -> Result<SavedWorkPreview> {
        self.runner.step("saved-work-preview","review_saved_work",|| {
            let (head,snapshot,changed)=self.saved_snapshot(c)?;
            let config=self.saved_read(&["config","--null","--list"],c)?;
            let reference=match &action {
                SavedWorkAction::Rebase(target)=>target.clone(),
                SavedWorkAction::Switch(branch)=> {
                    self.branch_argument(branch.as_bytes(),c)?;
                    if matches!(&head,Head::Branch{name,..} if name==branch.as_bytes()) {
                        return Err(invalid("saved_work_same_branch: select a different local branch."));
                    }
                    format!("refs/heads/{branch}")
                }
                SavedWorkAction::Pull(_)=> {
                    let Head::Branch{name,..}=&head else {unreachable!()};
                    let upstream=self.pull_upstream(name,c)?;
                    let fetched=self.fetch(&upstream.remote,None,c)?.result?;
                    if !fetched.updates?.iter().any(|u|u.reference==upstream.tracking_reference) {
                        return Err(invalid("saved_work_upstream: the fetch did not update the configured upstream; inspect its fetch mapping before retrying."));
                    }
                    String::from_utf8(upstream.tracking_reference).map_err(|_|invalid("saved_work_upstream: unsupported upstream encoding"))?
                }
            };
            let target=self.resolve_commit(&reference,c)?;
            let fresh=self.saved_snapshot(c)?;
            if fresh.0!=head||fresh.1!=snapshot||self.saved_read(&["config","--null","--list"],c)?!=config {return Err(invalid("saved_work_stale: files, checkout or configuration changed during review."));}
            Ok(SavedWorkPreview {repository:self.info().git_dir.clone(),action,head,snapshot,config,target,reference,changed})
        })
    }
    pub fn apply_saved_work(
        &self,
        p: &SavedWorkPreview,
        c: &CancellationToken,
    ) -> Result<SavedWorkReport> {
        let _permit = self.acquire_write(c)?;
        self.apply_saved_work_locked(p, c)
    }
    /// The caller already owns the repository write permit.
    pub(crate) fn apply_saved_work_locked(
        &self,
        p: &SavedWorkPreview,
        c: &CancellationToken,
    ) -> Result<SavedWorkReport> {
        self.runner.step("saved-work","revalidate_saved_work",|| {
            let fresh=self.saved_snapshot(c)?;
            if p.repository!=self.info().git_dir||fresh.0!=p.head||fresh.1!=p.snapshot||self.saved_read(&["config","--null","--list"],c)?!=p.config||self.resolve_commit(&p.reference,c)?!=p.target {return Err(invalid("saved_work_stale: checkout, files or target changed; review again. Nothing was stashed."));} Ok(())
        })?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| invalid("Invalid system clock"))?
            .as_nanos();
        let directory = self.info().git_dir.join("rocket-saved-work");
        if !directory.exists() {
            fs::create_dir(&directory)
                .map_err(|e| Error::io("create saved-work recovery folder", e))?;
        }
        let _guards = crate::local_files::pin_directories(&self.info().git_dir, &directory)?;
        let journal = directory.join(format!("{}-{stamp}.log", std::process::id()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&journal)
            .map_err(|e| Error::io("create saved-work journal", e))?;
        let mut report = SavedWorkReport {
            result: Ok(()),
            stash: None,
            phase: "preparing",
            journal,
        };
        let result = self.runner.step("saved-work","apply_saved_work",|| {
            writeln!(file, "original-head\t{:?}\ntarget\t{}", p.head, p.target)
                .and_then(|_| file.sync_all())
                .map_err(|e| Error::io("save recovery checkout", e))?;
            record(&mut file, "preparing", None)?;
            if p.changed > 0 {
                self.runner.step("saved-work","save_local_work",|| {
                    let message=format!("Rocket saved work {stamp}");
                    let saved=self.push_stash_paths_locked(Some(&message),false,&[],c)?;
                    let observed=self.stashes(&CancellationToken::new())?;
                    if let Some(entry)=observed.first().filter(|e|String::from_utf8_lossy(&e.subject).contains(&message)) {report.stash=Some(entry.id.clone());}
                    record(&mut file,"saved",report.stash.as_ref())?;report.phase="saved";
                    saved.write.result?;
                    if report.stash.is_none() {return Err(invalid("saved_work_unknown: inspect Stashes before continuing; the saved object could not be confirmed."));} Ok(())
                })?;
            }
            c.check()?;
            if self.head(c)? != p.head
                || self.saved_read(&["config", "--null", "--list"], c)? != p.config
                || self.resolve_commit(&p.reference, c)? != p.target
            {
                return Err(invalid(
                    "saved_work_changed: checkout/configuration changed after saving. Inspect and reapply the retained stash; integration was not started.",
                ));
            }
            record(&mut file, "integrating", report.stash.as_ref())?;
            report.phase = "integrating";
            self.runner.step("saved-work","integrate_saved_work",|| {
                match &p.action {
                    SavedWorkAction::Switch(branch)=> {
                        let name=self.branch_argument(branch.as_bytes(),c)?;
                        self.switch_branch_to(name,c)?.write.result?;
                        if !matches!(self.head(c)?,Head::Branch{name,target} if name==branch.as_bytes() && target==p.target) {
                            return Err(invalid("saved_work_switch_changed: the checkout differs from the reviewed destination. Recovery stash retained; restore was not started."));
                        }
                    }
                    SavedWorkAction::Rebase(_) | SavedWorkAction::Pull(PullStrategy::Rebase)=> {
                        let preview=self.preview_rebase(RebaseAction::Start,p.target.as_str(),c)?;
                        let result=self.apply_rebase_locked(&preview,c)?;
                        result.write.result?;
                    }
                    SavedWorkAction::Pull(strategy)=> {
                        let mode=if *strategy==PullStrategy::FastForward {MergeMode::FastForwardOnly}else{MergeMode::AllowFastForward};
                        let preview=self.preview_merge(p.target.as_str(),mode,c)?;
                        self.apply_merge_locked(&preview,c)?.write.result?;
                    }
                }
                if !self.operation_state(c)?.is_idle() {return Err(invalid("saved_work_paused: finish or abort the Git operation, then inspect and reapply the retained stash."));} Ok(())
            })?;
            report.phase = "ready-to-reapply";
            record(&mut file, report.phase, report.stash.as_ref())?;
            c.check()?;
            if let Some(id) = &report.stash {
                if let SavedWorkAction::Switch(branch)=&p.action
                    && (!matches!(self.head(c)?,Head::Branch{name,target} if name==branch.as_bytes() && target==p.target)
                        || self.saved_read(&["config","--null","--list"],c)?!=p.config) {
                    return Err(invalid("saved_work_switch_changed: checkout/configuration changed before restore. Inspect the retained stash; nothing was reapplied."));
                }
                if self
                    .status(c)?
                    .entries
                    .iter()
                    .any(|e| !matches!(e.kind, EntryKind::Untracked | EntryKind::Ignored))
                {
                    return Err(invalid(
                        "saved_work_changed: local changes appeared before reapply. Saved stash retained; inspect it before applying.",
                    ));
                }
                report.phase = "reapplying";
                record(&mut file, report.phase, Some(id))?;
                self.runner.step("saved-work", "reapply_saved_work", || {
                    self.apply_stash_locked(id, true, c)?
                        .write
                        .result
                        .map(|_| ())
                })?;
            }
            report.phase = "reapplied";
            record(&mut file, report.phase, report.stash.as_ref())?;
            Ok(())
        });
        report.result = result;
        Ok(report)
    }
}
fn record(file: &mut std::fs::File, phase: &str, id: Option<&ObjectId>) -> Result<()> {
    writeln!(file, "{phase}\t{}", id.map_or("", ObjectId::as_str))
        .and_then(|_| file.sync_all())
        .map_err(|e| Error::io("save recovery phase", e))
}
