//! Per-push worker/UI rendezvous. Approval belongs to one pinned push only.
use super::{Limits, error_message, scan};
use crate::app::Rocket;
use eframe::egui::Context;
use rocket_git::{CancellationToken, Error, ObjectId, PushCheck, Repository, Result};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender},
    },
    time::Duration,
};

#[derive(Clone, PartialEq, Eq)]
struct Config {
    executable: PathBuf,
    cache: PathBuf,
    work: PathBuf,
    /// A push started after Apply checks with the new limits.
    limits: Limits,
}
pub(super) struct TipReport {
    pub source: ObjectId,
    pub report: Arc<scan::Report>,
}
pub(super) enum Phase {
    Scanning,
    Continuing,
    Review {
        reports: Vec<TipReport>,
        decision: SyncSender<()>,
    },
    Failed(String),
}
pub(super) struct Active {
    pub(super) id: u64,
    pub repository: PathBuf,
    pub cancel: CancellationToken,
    pub phase: Phase,
}
enum Event {
    Started {
        id: u64,
        repository: PathBuf,
        cancel: CancellationToken,
    },
    Review {
        id: u64,
        reports: Vec<TipReport>,
        decision: SyncSender<()>,
    },
    Failed {
        id: u64,
        message: String,
    },
    Finished(u64),
}
pub(super) struct State {
    pub active: Option<Active>,
    sender: Sender<Event>,
    receiver: Receiver<Event>,
    guard: Option<(Config, Arc<Guard>)>,
}
impl Default for State {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            active: None,
            sender,
            receiver,
            guard: None,
        }
    }
}
impl State {
    pub fn cancel(&mut self) {
        if let Some(active) = self.active.take() {
            active.cancel.cancel();
        }
    }
    pub fn open(&self) -> bool {
        self.active.is_some()
    }
    pub fn summary(&self) -> serde_json::Value {
        serde_json::json!({"phase":match self.active.as_ref().map(|a| &a.phase) {
            None => "idle", Some(Phase::Scanning) => "scanning", Some(Phase::Continuing) => "continuing", Some(Phase::Review { .. }) => "review", Some(Phase::Failed(_)) => "failed"
        },"findings":self.active.as_ref().and_then(|a| match &a.phase { Phase::Review { reports, .. } => Some(reports.iter().map(|r|r.report.findings.len()).sum::<usize>()), _ => None })})
    }
    fn poll(&mut self, quitting: bool) {
        while let Ok(event) = self.receiver.try_recv() {
            match event {
                Event::Started {
                    id,
                    repository,
                    cancel,
                } => {
                    self.cancel();
                    self.active = Some(Active {
                        id,
                        repository,
                        cancel,
                        phase: Phase::Scanning,
                    });
                }
                Event::Review {
                    id,
                    reports,
                    decision,
                } => {
                    if let Some(active) = &mut self.active
                        && active.id == id
                    {
                        active.phase = Phase::Review { reports, decision };
                    }
                }
                Event::Failed { id, message } => {
                    if let Some(active) = &mut self.active
                        && active.id == id
                    {
                        active.phase = Phase::Failed(message);
                    }
                }
                Event::Finished(id) => {
                    if self
                        .active
                        .as_ref()
                        .is_some_and(|a| a.id == id && !matches!(a.phase, Phase::Failed(_)))
                    {
                        self.active = None;
                    }
                }
            }
        }
        if quitting
            || self
                .active
                .as_ref()
                .is_some_and(|a| a.cancel.is_cancelled() && !matches!(a.phase, Phase::Failed(_)))
        {
            self.cancel();
        }
    }
}
impl Drop for State {
    fn drop(&mut self) {
        self.cancel();
    }
}

struct Guard {
    config: Config,
    sender: Sender<Event>,
    context: Context,
}
impl Guard {
    fn send(&self, event: Event) -> Result<()> {
        self.sender.send(event).map_err(|_| {
            Error::InvalidInput("The dependency review is unavailable. No push was started.".into())
        })?;
        self.context.request_repaint();
        Ok(())
    }
    fn scan(
        &self,
        repo: &Repository,
        sources: &[ObjectId],
        cancel: &CancellationToken,
    ) -> Result<Vec<(ObjectId, scan::Cached)>> {
        if sources.len() > 32 {
            return Err(Error::InvalidInput(
                "Review at most 32 pushed tips in one dependency check.".into(),
            ));
        }
        let limits = &self.config.limits;
        // Every tip is read and the limits checked before OSV sees any of them, so a
        // limit stop sends nothing. All files counts every pushed tip together.
        let mut inventories = Vec::with_capacity(sources.len());
        let mut bytes = 0u64;
        for source in sources {
            cancel.check()?;
            let inventory = scan::committed_inventory(repo, source, limits, cancel)?;
            bytes += inventory.files.iter().map(|f| f.bytes.len() as u64).sum::<u64>();
            if bytes > limits.total_bytes() {
                return Err(scan::stop(format!(
                    "Dependency files in the pushed commits add up to over {} MiB. Scan limits allow {} MiB in all.",
                    limits.total_mib, limits.total_mib
                )));
            }
            inventories.push((source, inventory));
        }
        let mut results = Vec::new();
        let mut findings = 0usize;
        for (source, inventory) in inventories {
            cancel.check()?;
            let cached = if inventory.files.is_empty() {
                let mut notes = inventory.notes.clone();
                notes.push("No supported dependency files were found at this pushed tip. Dependency coverage is unavailable.".into());
                scan::Cached {
                    inventory: inventory.fingerprint()?,
                    database: vec![],
                    report: Arc::new(scan::Report {
                        packages: 0,
                        files: 0,
                        findings: vec![],
                        notes,
                        checked: scan::now(),
                        oldest_database: scan::now(),
                    }),
                }
            } else {
                scan::scan_inventory(
                    inventory,
                    &self.config.executable,
                    &self.config.cache,
                    &self.config.work,
                    None,
                    true,
                    cancel,
                )?
            };
            findings += cached.report.findings.len();
            if findings > 5_000 {
                return Err(Error::InvalidInput("Combined dependency warnings exceed the 5,000-finding review limit. Push fewer refs at a time.".into()));
            }
            results.push((source.clone(), cached));
        }
        Ok(results)
    }
    fn check_inner(
        &self,
        id: u64,
        repo: &Repository,
        sources: &[ObjectId],
        cancel: &CancellationToken,
    ) -> Result<()> {
        let results = self.scan(repo, sources, cancel)?;
        if results.iter().any(|(_, c)| {
            !c.report.findings.is_empty() || !c.report.notes.is_empty() || c.report.packages == 0
        }) {
            let (decision, answer) = mpsc::sync_channel(1);
            let reports = results
                .iter()
                .map(|(source, c)| TipReport {
                    source: source.clone(),
                    report: c.report.clone(),
                })
                .collect();
            self.send(Event::Review {
                id,
                reports,
                decision,
            })?;
            loop {
                cancel.check()?;
                match answer.recv_timeout(Duration::from_millis(50)) {
                    Ok(()) => break,
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return Err(Error::Cancelled),
                }
            }
        }
        cancel.check()?;
        for (_, cached) in &results {
            if !cached.inventory.is_empty()
                && scan::databases(&self.config.cache, &cached.inventory.ecosystems(), cancel)?
                    != cached.database
            {
                return Err(Error::InvalidInput("Advisory data changed during the push review. Run Push again to review the current result.".into()));
            }
        }
        Ok(())
    }
}
impl PushCheck for Guard {
    fn check(
        &self,
        repo: &Repository,
        sources: &[ObjectId],
        cancel: &CancellationToken,
    ) -> Result<()> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let repository = repo
            .info()
            .worktree
            .clone()
            .unwrap_or_else(|| repo.info().git_dir.clone());
        self.send(Event::Started {
            id,
            repository,
            cancel: cancel.clone(),
        })?;
        struct Finished<'a>(&'a Guard, u64);
        impl Drop for Finished<'_> {
            fn drop(&mut self) {
                let _ = self.0.send(Event::Finished(self.1));
            }
        }
        let _finished = Finished(self, id);
        let result = self.check_inner(id, repo, sources, cancel);
        if let Err(error) = &result
            && !matches!(error, Error::Cancelled)
        {
            self.send(Event::Failed {
                id,
                message: error_message(error),
            })?;
        }
        result
    }
}

impl Rocket {
    pub(crate) fn sync_dependency_push_check(&mut self) {
        let Some(backend) = &self.backend else { return };
        let check: Option<Arc<dyn PushCheck>> = if self.dependencies.before_push {
            let (executable, cache, work) = self.dependency_paths();
            let config = Config {
                executable,
                cache,
                work,
                limits: self.dependencies.limits,
            };
            let state = &mut self.dependencies.push_review;
            if state.guard.as_ref().is_none_or(|(old, _)| old != &config) {
                state.guard = Some((
                    config.clone(),
                    Arc::new(Guard {
                        config,
                        sender: state.sender.clone(),
                        context: self.context.clone(),
                    }),
                ));
            }
            Some(state.guard.as_ref().unwrap().1.clone())
        } else {
            None
        };
        if let Err(error) = backend.set_push_check(check) {
            self.notice = Some(error_message(&error));
        }
    }
    pub(crate) fn poll_dependency_push_check(&mut self) {
        self.sync_dependency_push_check();
        self.dependencies.push_review.poll(self.quitting);
    }
    pub fn dependency_push_review_open(&self) -> bool {
        self.dependencies.push_review.open()
    }
    pub(crate) fn cancel_dependency_push_review(&mut self) {
        self.dependencies.push_review.cancel();
    }
}

#[cfg(test)]
mod limit_tests {
    use super::*;
    use crate::test_common::{Fixture, isolated};

    #[test]
    fn pushed_tips_over_all_files_stop_before_any_tip_is_scanned() {
        isolated(
            "ROCKET_SCAN_LIMITS_CASE",
            "dependency_security::push::limit_tests::pushed_tips_over_all_files_stop_before_any_tip_is_scanned",
            || {
                let f = Fixture::new();
                let tip = |f: &Fixture| ObjectId::parse(String::from_utf8(f.run(&["rev-parse", "HEAD"])).unwrap().trim()).unwrap();
                f.write("Cargo.lock", vec![b'#'; 600 * 1024]);
                f.commit("first tip");
                let first = tip(&f);
                f.write("Cargo.lock", vec![b'%'; 600 * 1024]);
                f.commit("second tip");
                let second = tip(&f);
                // No scanner or advisory data: scanning the first tip would fail on them,
                // not on the limit.
                let (sender, _receiver) = mpsc::channel();
                let guard = Guard {
                    config: Config {
                        executable: f.root.join("inert.exe"),
                        cache: f.root.join("no-data"),
                        work: f.root.join("work"),
                        limits: Limits { files: 256, file_mib: 1, total_mib: 1 },
                    },
                    sender,
                    context: Context::default(),
                };
                let cancel = CancellationToken::new();
                let error = guard.scan(&f.repo(), &[first.clone(), second], &cancel).err().expect("over All files");
                assert_eq!(
                    error_message(&error),
                    "Dependency files in the pushed commits add up to over 1 MiB. Scan limits allow 1 MiB in all."
                );
                assert!(scan::stopped_at_limit(&error));
                // One tip alone fits, and then fails on the missing scanner, not a limit.
                let error = guard.scan(&f.repo(), &[first], &cancel).err().expect("no advisory data");
                assert!(!scan::stopped_at_limit(&error), "{}", error_message(&error));
            },
        );
    }
}
