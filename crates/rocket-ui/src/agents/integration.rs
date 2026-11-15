//! Review committed work against the original checkout; never stash, commit,
//! switch branches or delete worktrees as a side effect of integration. A session that
//! started from a pull request is checked against that pull request's branch instead,
//! to push back to it (section 48).
use super::{model::Record, runtime};
use crate::backend::{Backend, user_error};
use rocket_git::*;
use std::sync::Arc;

pub struct Review {
    pub owner: Record,
    /// Bringing the work into the original checkout: its merge review.
    pub merge: Option<Arc<MergePreview>>,
    /// A pull request's session: its branch on the remote, as a fast-forward.
    pub push: Option<PushCheck>,
    pub comparison: Comparison,
}
/// What pushing a pull request session's branch back would do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushCheck {
    pub remote: String,
    pub branch: String,
    /// Commits the pull request's branch gains.
    pub ahead: u64,
}
impl Review {
    /// Commits the step after the check brings: incoming for a merge, ahead for a push.
    pub fn incoming(&self) -> u64 {
        match (&self.merge, &self.push) {
            (Some(m), _) => m.incoming(),
            (None, Some(p)) => p.ahead,
            _ => 0,
        }
    }
}
#[derive(Default)]
pub struct State {
    pub task: Option<ReadTask<Review>>,
    pub review: Option<Review>,
    pub error: Option<String>,
    pub stopping: bool,
}
pub(super) fn read(backend: &Backend, r: &Record, cancel: &CancellationToken) -> Result<Review> {
    backend.request("agent-integration-preview",false,|git|{
        let source=git.open(&r.source,cancel)?;
        let entry=runtime::entry(&source,r,cancel)?;
        let agent=git.open(&r.destination,cancel)?;
        if !agent.operation_state(cancel)?.is_idle() || !agent.status(cancel)?.entries.is_empty() {
            return Err(super::model::invalid("Review and commit or stash the agent worktree changes first. Integration only brings committed work into the original checkout; all local files are kept."));
        }
        if let Some(p)=r.origin.as_ref().and_then(|o|o.push.as_ref()) {
            return push_check(&source,r,p,entry.head.as_ref(),cancel);
        }
        let status=source.status(cancel)?;
        if !matches!(status.head,Head::Branch{..}) || !status.entries.is_empty() {
            return Err(super::model::invalid("The original checkout needs a named branch and clean working files. Commit or stash its local changes first; Rocket has left them untouched."));
        }
        let revision=format!("refs/heads/{}",r.branch);
        let merge=source.preview_merge(&revision,MergeMode::AllowFastForward,cancel)?;
        if entry.head.as_ref()!=Some(&merge.target().id) {return Err(super::model::invalid("Agent branch changed during review. Check again."));}
        let comparison=source.compare_revisions("HEAD",merge.target().id.as_str(),cancel)?;
        // The source HEAD must be the same one described by the sealed merge preview.
        if !matches!(merge.head(),Head::Branch{target,..} if target==&comparison.base) {return Err(super::model::invalid("Original checkout changed during review. Check again."));}
        cancel.check()?;
        Ok(Review{owner:r.clone(),merge:Some(Arc::new(merge)),push:None,comparison})
    })
}
/// A pull request's session pushes its branch to the pull request's branch only as a
/// fast-forward of the commit it started on, as the remote last showed it. A fork's
/// branch is not on this remote, so it never passes.
fn push_check(source: &Repository, r: &Record, p: &super::model::PushBack, head: Option<&ObjectId>, cancel: &CancellationToken) -> Result<Review> {
    let outlook = source.push_outlook(r.branch.as_bytes(), p.remote.as_bytes(), p.branch.as_bytes(), cancel)?;
    let (remote, branch) = (&p.remote, &p.branch);
    match &outlook.tracked {
        None => return Err(Error::InvalidInput(crate::i18n::trf!(
            "{remote} has no branch {branch}. Fetch and check again; a pull request from a fork can't be pushed to from here.",
            remote, branch
        ))),
        Some(tracked) if tracked.as_str() != p.head => return Err(Error::InvalidInput(crate::i18n::trf!(
            "{remote}/{branch} moved since this session started. Fetch and look at what changed before pushing.",
            remote, branch
        ))),
        Some(_) => {}
    }
    if outlook.behind > 0 {
        return Err(Error::InvalidInput(crate::i18n::trf!("{remote}/{branch} has commits the agent's branch lacks. Pushing would be refused.", remote, branch)));
    }
    if outlook.ahead == 0 {
        return Err(Error::InvalidInput(crate::i18n::tr("Nothing to push yet. Commit in the worktree first.").into()));
    }
    let target = format!("refs/heads/{}", r.branch);
    let comparison = source.compare_revisions(&p.head, &target, cancel)?;
    if head != Some(&comparison.target) {
        return Err(super::model::invalid("Agent branch changed during review. Check again."));
    }
    cancel.check()?;
    Ok(Review { owner: r.clone(), merge: None, push: Some(PushCheck { remote: p.remote.clone(), branch: p.branch.clone(), ahead: outlook.ahead }), comparison })
}
impl State {
    pub fn start(
        &mut self,
        backend: Backend,
        r: Record,
        ctx: &eframe::egui::Context,
    ) -> Result<()> {
        self.clear();
        let ctx = ctx.clone();
        self.task = Some(ReadTask::spawn_with_wake(
            move |c| read(&backend, &r, &c),
            move || ctx.request_repaint(),
        )?);
        Ok(())
    }
    pub fn clear(&mut self) {
        self.task = None;
        self.review = None;
        self.error = None;
        self.stopping = false;
    }
    pub fn stop(&mut self) {
        if let Some(t) = &self.task {
            t.cancel();
            self.stopping = true;
        }
        self.review = None;
    }
    pub fn poll(&mut self) {
        if let Some(result) = self.task.as_mut().and_then(ReadTask::try_finish) {
            self.task = None;
            if self.stopping {
                self.error =
                    Some("Integration check cancelled. No files or branches changed.".into());
            } else {
                match result {
                    Ok(r) => self.review = Some(r),
                    Err(e) => self.error = Some(user_error(&e)),
                }
            }
            self.stopping = false;
        }
    }
}
