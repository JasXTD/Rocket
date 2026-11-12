//! Read only the selected worktree. A dropped task cannot publish into another
//! session; directory notifications coalesce reads without polling Git while idle.
//!
//! What was read is kept so it is not read again for nothing. A diff read since the
//! worktree last changed shows at once when its file is chosen again. Leaving the pane
//! keeps the watch, so coming back reads nothing unless something changed. When the
//! worktree does change, the list and the chosen file are read again while the old diff
//! stays on screen. A session's list and diffs are kept while another session shows,
//! shown again when it is chosen, and checked against the worktree right then.
use super::{model::Record, runtime};
use crate::{
    backend::{Backend, user_error},
    diff::Patch,
};
use eframe::egui::Context;
use rocket_git::*;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

const MAX_FILES: usize = 2_000;
/// Diffs kept ready for one session, and sessions whose list and diffs are kept.
const KEPT_PATCHES: usize = 24;
const KEPT_SESSIONS: usize = 6;
/// Back in view after this long, the list is read again even if no change was
/// reported: a watch can go quiet when its folder is replaced.
const TRUSTED: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Area {
    Staged,
    Unstaged,
    Untracked,
    Conflict,
}
impl Area {
    pub fn label(self) -> &'static str {
        match self {
            Self::Staged => "Staged",
            Self::Unstaged => "Unstaged",
            Self::Untracked => "Untracked",
            Self::Conflict => "Conflicted",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    pub path: RepoPath,
    pub original: Option<RepoPath>,
    pub area: Area,
}
pub struct Snapshot {
    pub files: Vec<File>,
    pub limited: bool,
    pub at: Instant,
    watch: Option<Arc<RepositoryWatch>>,
    watch_error: Option<String>,
}
#[derive(Default)]
pub struct State {
    pub owner: Option<Record>,
    pub loaded: Option<Snapshot>,
    pub selected: Option<usize>,
    /// The chosen file's diff. While that file is read again it is the earlier diff.
    pub patch: Option<Arc<Patch>>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub stopping: bool,
    listing: Option<ReadTask<Snapshot>>,
    content: Option<ReadTask<(File, Patch)>>,
    watch: Option<Arc<RepositoryWatch>>,
    dirty_since: Option<Instant>,
    paused: bool,
    /// Diffs read since the worktree last changed, oldest first.
    ready: Vec<(File, Arc<Patch>)>,
    /// What other sessions showed last, oldest first.
    kept: Vec<Kept>,
}

/// A session's list and diffs while another session shows. It is not watched, so it
/// is read again when the session shows next; until that answers it shows as it was.
struct Kept {
    owner: Record,
    loaded: Snapshot,
    selected: Option<usize>,
    patch: Option<Arc<Patch>>,
    ready: Vec<(File, Arc<Patch>)>,
}

/// Whether two records read the same worktree. A session's other fields (its activity,
/// whether it waits for the user) change while it runs and do not change what is read.
fn same_worktree(a: &Record, b: &Record) -> bool {
    a.id == b.id && a.source == b.source && a.destination == b.destination && a.branch == b.branch
}

fn open(backend: &Backend, record: &Record, cancel: &CancellationToken) -> Result<Repository> {
    backend.request("agent-session-changes", false, |git| {
        let source = git.open(&record.source, cancel)?;
        let path = source.open_worktree(&runtime::entry(&source, record, cancel)?, cancel)?;
        git.open(path, cancel)
    })
}
pub(super) fn read(
    backend: &Backend,
    record: &Record,
    cancel: &CancellationToken,
    previous: Option<Arc<RepositoryWatch>>,
    wake: Arc<dyn Fn() + Send + Sync>,
) -> Result<Snapshot> {
    cancel.check()?;
    let repo = open(backend, record, cancel)?;
    // Subscribe before reading status so edits during the read trigger another read.
    let (watch, watch_error) = match previous.filter(|w| w.is_current().unwrap_or(false)) {
        Some(w) => (Some(w), None),
        None => match RepositoryWatch::new(repo.info(), wake) {
            Ok(w) => (Some(Arc::new(w)), None),
            Err(e) => (None, Some(user_error(&e))),
        },
    };
    let status = backend.log.request("agent-session-changes", |log| {
        repo.with_log(log.clone()).status(cancel)
    })?;
    let mut files = Vec::new();
    for entry in status.entries {
        cancel.check()?;
        let mut add = |area| {
            files.push(File {
                path: entry.path.clone(),
                original: entry.original_path.clone(),
                area,
            })
        };
        match entry.kind {
            EntryKind::Ignored => {}
            EntryKind::Untracked => add(Area::Untracked),
            EntryKind::Unmerged => add(Area::Conflict),
            _ => {
                if entry.index != Change::Unchanged {
                    add(Area::Staged);
                }
                if entry.worktree != Change::Unchanged
                    || entry.submodule.is_some_and(|s| {
                        s.tracked_changes || s.untracked_changes || s.commit_changed
                    })
                {
                    add(Area::Unstaged);
                }
            }
        }
    }
    let limited = files.len() > MAX_FILES;
    files.truncate(MAX_FILES);
    cancel.check()?;
    Ok(Snapshot {
        files,
        limited,
        at: Instant::now(),
        watch,
        watch_error,
    })
}
pub(super) fn read_patch(
    backend: &Backend,
    record: &Record,
    file: &File,
    cancel: &CancellationToken,
) -> Result<Patch> {
    let repo = open(backend, record, cancel)?;
    match file.area {
        Area::Untracked => backend.log.request("agent-session-diff", |log| {
            // This panel is read-only. It needs the bounded bytes, not a staging
            // review with an index snapshot and instructions to apply lines.
            let preview = repo
                .with_log(log.clone())
                .preview_untracked(&file.path, cancel)?;
            cancel.check()?;
            let mut patch = Patch::preview(&file.path, preview);
            patch.notice = None;
            Ok(patch)
        }),
        Area::Conflict => backend.conflict_preview(&repo, &file.path, 0, cancel),
        _ => backend.log.request("agent-session-diff", |log| {
            let mut paths = vec![file.path.clone()];
            if let Some(old) = &file.original {
                paths.push(old.clone());
            }
            let diff = repo.with_log(log.clone()).diff(
                &DiffOptions {
                    target: if file.area == Area::Staged {
                        DiffTarget::Staged
                    } else {
                        DiffTarget::Unstaged
                    },
                    paths,
                    ..Default::default()
                },
                cancel,
            )?;
            cancel.check()?;
            let patch = Patch::parse(&diff.patch);
            cancel.check()?;
            Ok(patch)
        }),
    }
}
impl State {
    pub fn busy(&self) -> bool {
        self.listing.is_some() || self.content.is_some()
    }
    pub fn loading(&self) -> bool {
        self.listing.is_some()
    }
    pub fn selected_file(&self) -> Option<&File> {
        self.loaded.as_ref()?.files.get(self.selected?)
    }
    pub fn select_owner(&mut self, owner: &Record) {
        if let Some(current) = &mut self.owner
            && same_worktree(current, owner)
        {
            if current != owner {
                *current = owner.clone();
            }
            return;
        }
        // Another session: set aside what this one showed, and bring back what that one
        // showed last. Only the session in view is watched.
        let old = std::mem::take(self);
        let mut kept = old.kept;
        if let (Some(record), Some(mut loaded)) = (old.owner, old.loaded) {
            loaded.watch = None;
            kept.retain(|k| !same_worktree(&k.owner, &record));
            kept.push(Kept {
                owner: record,
                loaded,
                selected: old.selected,
                patch: old.patch,
                ready: old.ready,
            });
            if kept.len() > KEPT_SESSIONS {
                kept.remove(0);
            }
        }
        let back = kept
            .iter()
            .position(|k| same_worktree(&k.owner, owner))
            .map(|i| kept.remove(i));
        *self = Self {
            owner: Some(owner.clone()),
            paused: true,
            kept,
            ..Default::default()
        };
        if let Some(back) = back {
            self.loaded = Some(back.loaded);
            self.selected = back.selected;
            self.patch = back.patch;
            self.ready = back.ready;
        }
    }
    /// Out of view: reads stop, the watch stays. A read that was cut short, or a change
    /// already noticed, is remembered as a change, so coming back reads again.
    pub fn suspend(&mut self) {
        if self.listing.is_some() || self.content.is_some() {
            self.dirty_since.get_or_insert_with(Instant::now);
        }
        self.listing = None;
        self.content = None;
        self.stopping = false;
        self.paused = true;
    }
    /// Out of view for something that may move or remove the worktree (a session dialog,
    /// another page, quitting): reads stop and the folder is let go of, since nothing
    /// here may hold it then. What shows is kept, and read again once back in view.
    pub fn release(&mut self) {
        self.suspend();
        self.watch = None;
        if let Some(s) = &mut self.loaded {
            s.watch = None;
        }
        self.ready.clear();
    }
    pub fn stop(&mut self) {
        if let Some(t) = &self.listing {
            t.cancel();
        }
        if let Some(t) = &self.content {
            t.cancel();
        }
        self.stopping = self.busy();
        // No longer watched: the diff on screen stays, and nothing else is trusted.
        self.watch = None;
        self.ready.clear();
        self.dirty_since = None;
        if let Some(s) = &mut self.loaded {
            s.watch = None;
        }
        self.notice = Some("Read cancelled. Refresh to check for newer changes.".into());
    }
    pub fn refresh(&mut self, backend: &Backend, ctx: &Context) {
        let Some(owner) = self.owner.clone() else {
            return;
        };
        self.listing = None;
        self.content = None;
        self.stopping = false;
        self.error = None;
        self.notice = None;
        self.paused = false;
        self.dirty_since = None;
        let backend = backend.clone();
        let watch = self.watch.clone();
        let wake = ctx.clone();
        match crate::read_task::spawn(ctx, move |c| {
            read(
                &backend,
                &owner,
                &c,
                watch,
                Arc::new(move || wake.request_repaint()),
            )
        }) {
            Ok(task) => self.listing = Some(task),
            Err(e) => self.error = Some(user_error(&e)),
        }
    }
    pub fn select(&mut self, index: usize, backend: &Backend, ctx: &Context) {
        if self.loading() || self.stopping {
            return;
        }
        let Some(file) = self
            .loaded
            .as_ref()
            .and_then(|s| s.files.get(index))
            .cloned()
        else {
            return;
        };
        let Some(owner) = self.owner.clone() else {
            return;
        };
        self.selected = Some(index);
        self.content = None;
        self.error = None;
        self.stopping = false;
        // Read since the worktree last changed: it is still right, so show it again.
        // Without a watch nothing says when it stops being right, so nothing is kept.
        if self.watch.is_some()
            && let Some((_, patch)) = self.ready.iter().find(|(f, _)| f == &file)
        {
            self.patch = Some(patch.clone());
            return;
        }
        self.patch = None;
        self.read_diff(owner, file, backend, ctx);
    }
    fn read_diff(&mut self, owner: Record, file: File, backend: &Backend, ctx: &Context) {
        let backend = backend.clone();
        match crate::read_task::spawn(ctx, move |c| {
            let patch = read_patch(&backend, &owner, &file, &c)?;
            Ok((file, patch))
        }) {
            Ok(task) => self.content = Some(task),
            Err(e) => self.error = Some(user_error(&e)),
        }
    }
    pub fn poll(&mut self, backend: &Backend, ctx: &Context) {
        if self.paused {
            // Back in view. Watched all along with nothing reported, and read not long
            // ago: what shows is still right, and nothing is read.
            let fresh = self.loaded.as_ref().is_some_and(|s| s.at.elapsed() < TRUSTED);
            let quiet = self.dirty_since.is_none()
                && self.watch.as_ref().is_some_and(|w| matches!(w.take_changed(), Ok(false)));
            if fresh && quiet {
                self.paused = false;
                if self.patch.is_none()
                    && let Some(index) = self.selected
                {
                    self.select(index, backend, ctx);
                }
            } else {
                self.refresh(backend, ctx);
            }
        }
        if let Some(result) = self.listing.as_mut().and_then(ReadTask::try_finish) {
            self.listing = None;
            match result {
                Ok(snapshot) if !self.stopping => {
                    let old = self.selected_file().cloned();
                    self.selected = old
                        .as_ref()
                        .and_then(|f| snapshot.files.iter().position(|n| n == f))
                        .or_else(|| (!snapshot.files.is_empty()).then_some(0));
                    self.watch = snapshot.watch.clone();
                    self.notice = snapshot.watch_error.clone();
                    self.loaded = Some(snapshot);
                    // A new list means the worktree may have changed: no diff read before
                    // can be trusted. The chosen file is read again, and until that answers
                    // its earlier diff stays on screen.
                    self.ready.clear();
                    self.content = None;
                    let chosen = self.selected_file().cloned();
                    if chosen != old {
                        self.patch = None;
                    }
                    if let (Some(file), Some(owner)) = (chosen, self.owner.clone()) {
                        self.error = None;
                        self.read_diff(owner, file, backend, ctx);
                    }
                }
                Err(Error::Cancelled) | Ok(_) => {}
                Err(e) => {
                    self.error = Some(user_error(&e));
                    self.watch = None;
                    self.ready.clear();
                }
            }
        }
        if let Some(result) = self.content.as_mut().and_then(ReadTask::try_finish) {
            self.content = None;
            match result {
                Ok((file, patch)) if !self.stopping => {
                    let patch = Arc::new(patch);
                    self.ready.retain(|(f, _)| f != &file);
                    if self.watch.is_some() {
                        self.ready.push((file, patch.clone()));
                        if self.ready.len() > KEPT_PATCHES {
                            self.ready.remove(0);
                        }
                    }
                    self.patch = Some(patch);
                }
                Err(Error::Cancelled) | Ok(_) => {}
                Err(e) => self.error = Some(user_error(&e)),
            }
        }
        if !self.busy() {
            self.stopping = false;
        }
        if let Some(watch) = &self.watch {
            match watch.take_changed() {
                Ok(true) => {
                    self.dirty_since.get_or_insert_with(Instant::now);
                }
                Ok(false) => {}
                Err(e) => {
                    self.notice = Some(user_error(&e));
                    self.watch = None;
                    self.ready.clear();
                }
            }
        }
        if let Some(since) = self.dirty_since {
            if !self.busy() && since.elapsed() >= Duration::from_millis(250) {
                self.refresh(backend, ctx);
            } else {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
    }
}

#[cfg(test)]
mod tests;
