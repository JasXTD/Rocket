//! Review committed work against the original checkout; never stash, commit,
//! switch branches or delete worktrees as a side effect of integration.
use super::{model::Record, runtime};
use crate::backend::{Backend, user_error};
use rocket_git::*;
use std::sync::Arc;

pub struct Review {
    pub owner: Record,
    pub merge: Arc<MergePreview>,
    pub comparison: Comparison,
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
        Ok(Review{owner:r.clone(),merge:Arc::new(merge),comparison})
    })
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
