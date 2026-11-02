//! Worktree-only refreshes retain repository-wide state only behind fresh guards.
use super::*;

pub enum Refresh {
    Full(Box<Snapshot>),
    Worktree(Box<WorktreeRefresh>),
}

pub(crate) enum RefreshMode {
    Manual,
    Automatic(Option<RefreshBase>),
}

/// The view requested when a network write started. Reconciliation always reads
/// fresh data; the result can only be handed to that same repository/view.
#[derive(Clone)]
pub(crate) struct NetworkRead {
    pub start: HistoryStart,
    pub order: HistoryOrder,
    pub wake: Option<Arc<dyn Fn() + Send + Sync>>,
}

#[doc(hidden)]
pub struct NetworkRefresh {
    root: PathBuf,
    view: NetworkRead,
    snapshot: Box<Snapshot>,
}
impl std::fmt::Debug for NetworkRefresh {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Effective configuration and repository contents never enter Debug logs.
        f.debug_struct("NetworkRefresh").finish_non_exhaustive()
    }
}
impl NetworkRead {
    pub(super) fn reconcile(&self, backend: &Backend, root: &Path) -> Result<NetworkRefresh> {
        let snapshot = backend.snapshot_ordered(
            root, self.start.clone(), self.order, &CancellationToken::new(),
            self.wake.clone(), true,
        )?;
        Ok(NetworkRefresh { root: root.into(), view: self.clone(), snapshot: Box::new(snapshot) })
    }
}
impl NetworkRefresh {
    pub(crate) fn into_matching(
        self, root: &Path, start: &HistoryStart, order: HistoryOrder, watched: bool,
    ) -> Option<Box<Snapshot>> {
        let same_start = match (&self.view.start, start) {
            (HistoryStart::Head, HistoryStart::Head)
            | (HistoryStart::AllBranches, HistoryStart::AllBranches) => true,
            (HistoryStart::Revision(a), HistoryStart::Revision(b)) => a == b,
            _ => false,
        };
        (self.root == root && self.view.order == order && same_start
            && self.view.wake.is_some() == watched).then_some(self.snapshot)
    }
}

impl Outcome {
    pub(super) fn reconciled(&mut self, result: Result<NetworkRefresh>) {
        if let Some(refreshed) = self.observe("Refresh repository", result) {
            self.details.extend(refreshed.snapshot.warnings.iter().cloned());
            self.network_refresh = Some(refreshed);
        }
    }
}

#[derive(Clone)]
pub(crate) struct Guard {
    pub(super) generation: u64,
    configuration: Arc<ConfigurationSnapshot>,
}

impl Guard {
    pub(super) fn remotes(&self, cancel: &CancellationToken) -> Result<Vec<Remote>> {
        self.configuration.remotes(cancel)
    }

    pub(super) fn capture(
        repo: &Repository,
        watch: Option<&Arc<RepositoryWatch>>,
        cancel: &CancellationToken,
    ) -> Result<Option<Self>> {
        if repo.info().worktree.is_none() {
            return Ok(None);
        }
        let Some(generation) = watch.and_then(|watch| watch.metadata_generation()) else {
            return Ok(None);
        };
        if !watch.is_some_and(|watch| watch.is_current().unwrap_or(false)) {
            return Ok(None);
        }
        // This optional guard is not required for a full read. A failure disables
        // reuse; the regular snapshot still reports its own read errors normally.
        match repo.configuration_snapshot(cancel) {
            Ok(configuration) => Ok(Some(Self {
                generation,
                configuration: Arc::new(configuration),
            })),
            Err(Error::Cancelled) => Err(Error::Cancelled),
            Err(_) => Ok(None),
        }
    }
}

pub(crate) struct RefreshBase {
    info: RepositoryInfo,
    head: Head,
    watcher: Arc<RepositoryWatch>,
    guard: Guard,
}

impl Snapshot {
    /// Opaque effective config already read for this snapshot; never log it.
    pub(crate) fn configuration(&self) -> Option<Arc<ConfigurationSnapshot>> {
        self.refresh_guard.as_ref().map(|guard| guard.configuration.clone())
    }

    pub(crate) fn refresh_base(&self) -> Option<RefreshBase> {
        let guard = self.refresh_guard.as_ref()?;
        let watcher = self.watcher.as_ref()?;
        if !self.warnings.is_empty() || watcher.metadata_generation() != Some(guard.generation) {
            return None;
        }
        Some(RefreshBase {
            info: self.info.clone(),
            head: self.head.clone()?,
            watcher: watcher.clone(),
            guard: guard.clone(),
        })
    }
}

pub struct WorktreeRefresh {
    repository: Repository,
    status: Status,
    generation: u64,
}

impl WorktreeRefresh {
    /// A late metadata notification requires a full follow-up, not publication
    /// of a partial snapshot whose unchanged sections can no longer be proven.
    pub(crate) fn matches(&self, snapshot: &Snapshot) -> bool {
        snapshot.info == *self.repository.info()
            && snapshot.head.as_ref() == Some(&self.status.head)
            && snapshot
                .watcher
                .as_ref()
                .is_none_or(|watch| watch.metadata_generation() == Some(self.generation))
    }
    pub(crate) fn apply(self, snapshot: &mut Snapshot) {
        snapshot.repository = self.repository;
        snapshot.head = Some(self.status.head.clone());
        snapshot.status = Some(self.status);
    }
}

impl Backend {
    pub(crate) fn refresh_ordered(
        &self,
        root: &Path,
        start: HistoryStart,
        order: HistoryOrder,
        cancel: &CancellationToken,
        wake: Option<Arc<dyn Fn() + Send + Sync>>,
        mode: RefreshMode,
    ) -> Result<Refresh> {
        let (automatic, base) = match mode {
            RefreshMode::Manual => (false, None),
            RefreshMode::Automatic(base) => (true, base),
        };
        if let Some(base) = base
            && base.watcher.metadata_generation() == Some(base.guard.generation)
        {
            let read = self
                .log
                .request("auto_refresh", |log| -> Result<Option<WorktreeRefresh>> {
                    if !base.watcher.is_current()? {
                        return Ok(None);
                    }
                    let repo = self.client.with_log(log.clone()).open(root, cancel)?;
                    if repo.info() != &base.info
                        || repo.configuration_snapshot(cancel)? != *base.guard.configuration
                    {
                        return Ok(None);
                    }
                    let status = repo.status(cancel)?;
                    if status.head != base.head
                        || repo.configuration_snapshot(cancel)? != *base.guard.configuration
                        || base.watcher.metadata_generation() != Some(base.guard.generation)
                        || !base.watcher.is_current()?
                    {
                        return Ok(None);
                    }
                    cancel.check()?;
                    Ok(Some(WorktreeRefresh {
                        repository: repo,
                        status,
                        generation: base.guard.generation,
                    }))
                });
            match read {
                Ok(Some(update)) => return Ok(Refresh::Worktree(Box::new(update))),
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                // An uncertain/failed fast read never supplies partial data.
                // Run the complete existing read and preserve its error behavior.
                Ok(None) | Err(_) => (),
            }
        }
        self.snapshot_ordered(root, start, order, cancel, wake, automatic)
            .map(|snapshot| Refresh::Full(Box::new(snapshot)))
    }
}
