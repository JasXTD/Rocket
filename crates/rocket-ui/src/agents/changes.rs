//! Read only the selected worktree. A dropped task cannot publish into another
//! session; directory notifications coalesce reads without polling Git while idle.
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
    pub patch: Option<Patch>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub stopping: bool,
    listing: Option<ReadTask<Snapshot>>,
    content: Option<ReadTask<Patch>>,
    watch: Option<Arc<RepositoryWatch>>,
    dirty_since: Option<Instant>,
    paused: bool,
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
        if self.owner.as_ref() != Some(owner) {
            *self = Self {
                owner: Some(owner.clone()),
                paused: true,
                ..Default::default()
            };
        }
    }
    pub fn suspend(&mut self) {
        self.listing = None;
        self.content = None;
        self.watch = None;
        if let Some(s) = &mut self.loaded {
            s.watch = None;
        }
        self.dirty_since = None;
        self.stopping = false;
        self.paused = true;
    }
    pub fn stop(&mut self) {
        if let Some(t) = &self.listing {
            t.cancel();
        }
        if let Some(t) = &self.content {
            t.cancel();
        }
        self.stopping = self.busy();
        self.watch = None;
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
        self.patch = None;
        self.error = None;
        self.stopping = false;
        let backend = backend.clone();
        match crate::read_task::spawn(ctx, move |c| read_patch(&backend, &owner, &file, &c)) {
            Ok(task) => self.content = Some(task),
            Err(e) => self.error = Some(user_error(&e)),
        }
    }
    pub fn poll(&mut self, backend: &Backend, ctx: &Context) {
        if self.paused {
            self.refresh(backend, ctx);
        }
        if let Some(result) = self.listing.as_mut().and_then(ReadTask::try_finish) {
            self.listing = None;
            match result {
                Ok(snapshot) if !self.stopping => {
                    let old = self.selected_file().cloned();
                    self.selected = old
                        .and_then(|f| snapshot.files.iter().position(|n| n == &f))
                        .or_else(|| (!snapshot.files.is_empty()).then_some(0));
                    self.watch = snapshot.watch.clone();
                    self.notice = snapshot.watch_error.clone();
                    self.loaded = Some(snapshot);
                    if let Some(index) = self.selected {
                        self.select(index, backend, ctx);
                    } else {
                        self.content = None;
                        self.patch = None;
                    }
                }
                Err(Error::Cancelled) | Ok(_) => {}
                Err(e) => {
                    self.error = Some(user_error(&e));
                    self.watch = None;
                }
            }
        }
        if let Some(result) = self.content.as_mut().and_then(ReadTask::try_finish) {
            self.content = None;
            match result {
                Ok(patch) if !self.stopping => self.patch = Some(patch),
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
