//! Local branch helpers with explicit saved-work and merge policies. No network or deletion.
use crate::{runner::args, *};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BranchWorkflowKind {
    Sync,
    Finish,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BranchWorkflowOptions {
    /// Sync only: save tracked edits and restore staging, retaining the recovery stash.
    pub save_changes: bool,
    /// Finish only: allow a fast-forward; diverged branches still create a merge commit.
    pub allow_fast_forward: bool,
}
impl BranchWorkflowKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Sync => "branch-sync",
            Self::Finish => "branch-finish",
        }
    }
}
#[derive(Clone, PartialEq, Eq)]
struct State {
    git_dir: PathBuf,
    head: Head,
    feature: String,
    tip: ObjectId,
    base: String,
    base_tip: ObjectId,
    config: Vec<u8>,
    index: Vec<u8>,
}
#[derive(Clone)]
pub struct BranchWorkflowPreview {
    kind: BranchWorkflowKind,
    state: State,
    rebase: Option<RebasePreview>,
    commits: u64,
    options: BranchWorkflowOptions,
    saved: Option<SavedWorkPreview>,
    can_fast_forward: bool,
}
impl std::fmt::Debug for BranchWorkflowPreview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BranchWorkflowPreview")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}
impl BranchWorkflowPreview {
    pub fn kind(&self) -> BranchWorkflowKind {
        self.kind
    }
    pub fn feature(&self) -> &str {
        &self.state.feature
    }
    pub fn base(&self) -> &str {
        &self.state.base
    }
    pub fn tip(&self) -> &ObjectId {
        &self.state.tip
    }
    pub fn base_tip(&self) -> &ObjectId {
        &self.state.base_tip
    }
    pub fn commits(&self) -> u64 {
        self.commits
    }
    pub fn options(&self) -> BranchWorkflowOptions { self.options }
    pub fn saved_changes(&self) -> usize { self.saved.as_ref().map_or(0, SavedWorkPreview::changed_files) }
    pub fn can_fast_forward(&self) -> bool { self.can_fast_forward }
}
#[derive(Debug)]
pub struct BranchWorkflowReport {
    pub result: Result<()>,
    pub phase: &'static str,
    pub recovery_prefix: String,
    pub journal: PathBuf,
    pub state: Result<OperationState>,
    pub head: Result<Head>,
    pub recovery_stash: Option<ObjectId>,
    pub saved_work_journal: Option<PathBuf>,
}
fn refusal(message: &str) -> Error {
    Error::InvalidInput(format!("branch_workflow: {message}"))
}
impl Repository {
    fn workflow_read(&self, a: &[&str], c: &CancellationToken) -> Result<Vec<u8>> {
        self.runner
            .run(self.cwd(), &args(a), vec![], c)?
            .success("branch-workflow")
    }
    fn workflow_clean(&self, c: &CancellationToken) -> Result<()> {
        self.workflow_ready(false, c)
    }
    fn workflow_ready(&self, save_changes: bool, c: &CancellationToken) -> Result<()> {
        self.require_worktree(c)?;
        self.require_idle(c)?;
        if !save_changes && !self.status(c)?.entries.is_empty() {
            return Err(refusal(
                "commit or stash local changes (including untracked files) first; no changes were discarded.",
            ));
        }
        let entries = self.workflow_read(&["ls-files", "-v", "-z"], c)?;
        if entries
            .split(|b| *b == 0)
            .filter_map(|e| e.first())
            .any(|b| b.is_ascii_lowercase() || *b == b'S')
        {
            return Err(refusal(
                "handle sparse/skip-worktree or assume-unchanged entries before using this helper.",
            ));
        }
        Ok(())
    }
    fn workflow_state(&self, base: &str, save_changes: bool, c: &CancellationToken) -> Result<State> {
        self.workflow_ready(save_changes, c)?;
        self.branch_argument(base.as_bytes(), c)?;
        let head = self.head(c)?;
        let Head::Branch { name, target } = &head else {
            return Err(refusal("check out a committed local feature branch first."));
        };
        let feature = String::from_utf8(name.clone())
            .map_err(|_| refusal("this branch name cannot be displayed as Unicode."))?;
        if feature == base {
            return Err(refusal("choose a different local base branch."));
        }
        let tip = target.clone();
        let base_tip = self.resolve_commit(&format!("refs/heads/{base}"), c)?;
        let config = self.workflow_read(&["config", "--null", "--list"], c)?;
        let index = self.workflow_read(&["ls-files", "--stage", "-v", "-z"], c)?;
        Ok(State {
            git_dir: self.info().git_dir.clone(),
            head,
            feature,
            tip,
            base: base.into(),
            base_tip,
            config,
            index,
        })
    }
    pub fn preview_branch_workflow(
        &self,
        kind: BranchWorkflowKind,
        base: &str,
        c: &CancellationToken,
    ) -> Result<BranchWorkflowPreview> {
        self.preview_branch_workflow_with_options(kind, base, BranchWorkflowOptions::default(), c)
    }
    pub fn preview_branch_workflow_with_options(
        &self,
        kind: BranchWorkflowKind,
        base: &str,
        options: BranchWorkflowOptions,
        c: &CancellationToken,
    ) -> Result<BranchWorkflowPreview> {
        self.runner.step(kind.label(),"review_branch_workflow",||{
            if (kind==BranchWorkflowKind::Finish && options.save_changes) || (kind==BranchWorkflowKind::Sync && options.allow_fast_forward) {
                return Err(refusal("save changes is a Sync option; fast-forward is a Finish option."));
            }
            let state=self.workflow_state(base,options.save_changes,c)?;
            if kind==BranchWorkflowKind::Finish {
                let branch=format!("refs/heads/{base}");
                if self.worktrees(c)?.iter().any(|w|w.branch.as_deref()==Some(branch.as_bytes()) && w.path!=self.cwd()){
                    return Err(refusal("the base branch is checked out in another worktree; finish there or switch that worktree first."));
                }
            }
            let saved=if options.save_changes {
                Some(self.preview_saved_work(SavedWorkAction::Rebase(format!("refs/heads/{base}")),c)?)
            } else { None };
            let rebase=if kind==BranchWorkflowKind::Sync && saved.is_none() {
                Some(self.preview_rebase(RebaseAction::Start,&format!("refs/heads/{base}"),c)?)
            }else{None};
            let count=self.workflow_read(&["rev-list","--count",&format!("{}..{}",state.base_tip,state.tip)],c)?;
            let commits=std::str::from_utf8(&count).ok().and_then(|s|s.trim().parse().ok()).ok_or_else(||refusal("could not count the reviewed commits."))?;
            let behind=self.workflow_read(&["rev-list","--count",&format!("{}..{}",state.tip,state.base_tip)],c)?;
            let can_fast_forward=behind==b"0\n";
            if self.workflow_state(base,options.save_changes,c)?!=state{return Err(refusal("repository changed during review; review again."));}
            Ok(BranchWorkflowPreview{kind,state,rebase,commits,options,saved,can_fast_forward})
        })
    }
    pub fn apply_branch_workflow(
        &self,
        p: &BranchWorkflowPreview,
        c: &CancellationToken,
    ) -> Result<BranchWorkflowReport> {
        let _permit = self.acquire_write(c)?;
        let validate = || {
            self.runner.step(p.kind.label(),"revalidate_branch_workflow",||{
            if self.workflow_state(&p.state.base,p.options.save_changes,c)?!=p.state {return Err(refusal("checkout, base, feature, files or configuration changed; review again. No branch operation started."));}
            Ok(())
        })
        };
        validate()?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| refusal("invalid system clock."))?
            .as_nanos();
        let prefix = format!("refs/rocket/branch-workflow/{}-{stamp}", std::process::id());
        let directory = self.info().git_dir.join("rocket-branch-workflow");
        if !directory.exists() {
            fs::create_dir(&directory)
                .map_err(|e| Error::io("create branch workflow recovery folder", e))?;
        }
        let _pins = crate::local_files::pin_directories(&self.info().git_dir, &directory)?;
        let journal = directory.join(format!("{}-{stamp}.log", std::process::id()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&journal)
            .map_err(|e| Error::io("create branch workflow journal", e))?;
        writeln!(
            file,
            "operation\t{}\noriginal-branch\t{}\nbase-branch\t{}\nrecovery-prefix\t{}",
            p.kind.label(),
            p.feature(),
            p.base(),
            prefix
        )
        .and_then(|_| file.sync_all())
        .map_err(|e| Error::io("record branch workflow recovery", e))?;
        let mut phase = "preparing";
        let mut recovery_stash = None;
        let mut saved_work_journal = None;
        let result=self.runner.step(p.kind.label(),"apply_branch_workflow",||{
            let input=format!("start\ncreate {prefix}/feature {}\ncreate {prefix}/base {}\nprepare\ncommit\n",p.tip(),p.base_tip());
            self.runner.step(p.kind.label(),"backup_branch_tips",||self.runner.write(self.cwd(),&args(&["update-ref","--stdin"]),input.into_bytes(),c)?.success("branch-workflow").map(|_|()))?;
            validate()?; c.check()?;
            if p.kind==BranchWorkflowKind::Sync {
                if let Some(saved)=&p.saved {
                    phase="saving-and-syncing";record(&mut file,phase)?;
                    let r=self.apply_saved_work_locked(saved,c)?;
                    phase=r.phase;recovery_stash=r.stash;saved_work_journal=Some(r.journal);
                    record(&mut file,phase)?;
                    r.result?;
                } else {
                    phase="rebasing"; record(&mut file,phase)?;
                    let r=self.apply_rebase_locked(p.rebase.as_ref().ok_or_else(||refusal("missing rebase review."))?,c)?;
                    r.write.result?;
                    r.journal_cleanup?;
                    if let Some(f)=r.failure{return Err(Error::Rebase(f));}
                }
            }else {
                phase="switching-base";record(&mut file,phase)?;
                let name=self.branch_argument(p.base().as_bytes(),c)?;
                self.switch_branch_to(name,c)?.write.result?;
                phase="base-checked-out";record(&mut file,phase)?;
                c.check()?;self.workflow_clean(c)?;
                if !matches!(self.head(c)?,Head::Branch{name,target} if name==p.base().as_bytes() && target==*p.base_tip())
                    || self.resolve_commit(&format!("refs/heads/{}",p.feature()),c)?!=*p.tip()
                    || self.workflow_read(&["config","--null","--list"],c)?!=p.state.config {
                    return Err(refusal("branch tips, checkout or configuration changed after switching. Merge did not start; inspect the current checkout."));
                }
                phase="merging";record(&mut file,phase)?;
                let mode=if p.options.allow_fast_forward {MergeMode::AllowFastForward} else {MergeMode::NoFastForward};
                let arguments=self.merge_commit_arguments_with_message(p.tip(),mode,Some(&format!("Merge branch '{}' into {}",p.feature(),p.base())),c)?;
                let r=self.execute_merge_write(WriteOperation::Merge,&arguments,vec![],c)?;
                r.write.result?;
                if let Some(f)=r.failure{return Err(Error::Merge(f));}
            }
            if !self.operation_state(c)?.is_idle(){return Err(refusal("Git paused. Use Changes to continue or abort; recovery tips and both branches are retained."));}
            phase="finished";record(&mut file,phase)?;
            Ok(())
        });
        let fresh = CancellationToken::new();
        let state = self
            .runner
            .step(p.kind.label(), "observe_branch_workflow", || {
                self.operation_state(&fresh)
            });
        let head = self.head(&fresh);
        Ok(BranchWorkflowReport {
            result,
            phase,
            recovery_prefix: prefix,
            journal,
            state,
            head,
            recovery_stash,
            saved_work_journal,
        })
    }
}
fn record(file: &mut std::fs::File, phase: &str) -> Result<()> {
    writeln!(file, "phase\t{phase}")
        .and_then(|_| file.sync_all())
        .map_err(|e| Error::io("record branch workflow phase", e))
}
