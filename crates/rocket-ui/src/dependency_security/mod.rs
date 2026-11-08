//! Local dependency scans and an optional review before publishing pushed tips.
mod page;
mod scan_budget;
mod push;
mod push_page;
#[cfg(test)]
mod push_tests;
pub mod scan;
mod setup;
mod setup_page;
use crate::{app::Rocket, i18n::{tr, trf}, pages, style::*};
use eframe::egui::*;
use rocket_git::{Error, ReadTask};
use std::{
    collections::HashMap,
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Default)]
struct RepositoryScan {
    cached: Option<scan::Cached>,
    error: Option<String>,
    next: Option<Instant>,
}
enum JobResult {
    Scan(scan::Cached),
    Updated,
}
struct Job {
    owner: u64,
    path: PathBuf,
    update: bool,
    discard: bool,
    task: ReadTask<JobResult>,
}
pub struct State {
    pub enabled: bool,
    pub before_push: bool,
    pub editable: bool,
    push_review: push::State,
    repositories: HashMap<u64, RepositoryScan>,
    job: Option<Job>,
    pub update_notice: Option<String>,
    pub(crate) setup_page: bool,
    setup: setup::Setup,
    scan_budget: scan_budget::Edit,
    /// Set only by Rust fixture tests, never by repository content or MCP commands.
    #[cfg(test)]
    pub fixture_executable: Option<PathBuf>,
    #[cfg(test)]
    pub fixture_cache: Option<PathBuf>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            enabled: true,
            before_push: false,
            editable: true,
            push_review: Default::default(),
            repositories: HashMap::new(),
            job: None,
            update_notice: None,
            setup_page: false,
            setup: setup::Setup::default(),
            scan_budget: Default::default(),
            #[cfg(test)]
            fixture_executable: None,
            #[cfg(test)]
            fixture_cache: None,
        }
    }
}
impl State {
    pub fn running(&self) -> bool {
        self.job.is_some() || self.setup.running()
    }
    pub fn cancel(&self) {
        self.setup.cancel();
        if let Some(job) = &self.job {
            job.task.cancel();
        }
    }
    pub fn summary(&self, owner: Option<u64>) -> serde_json::Value {
        let state = owner.and_then(|id| self.repositories.get(&id));
        let report = state.and_then(|s| s.cached.as_ref()).map(|c| &c.report);
        serde_json::json!({"enabled":self.enabled,"user_control":self.editable,"running":self.running(),
            "status":if self.running() {"running"} else if self.setup.loaded && !self.setup.ready {"setup_required"} else if state.is_some_and(|s| s.error.is_some()) {"incomplete"} else if report.is_some() {"scanned"} else {"not_scanned"},
            "setup_loaded":self.setup.loaded,"scanner_ready":self.setup.ready,"setup_error":self.setup.error.is_some(),
            "packages":report.map(|r|r.packages),"findings":report.map(|r|r.findings.len()),"coverage_notes":report.map(|r|r.notes.len()),
            "checked":report.map(|r|r.checked),"blocking":self.push_review.open(),"before_push":self.before_push,"push_check":self.push_review.summary()})
    }
}
fn error_message(error: &Error) -> String {
    match error {
        Error::InvalidInput(message) => message.clone(),
        Error::Cancelled => "Scan cancelled. No current result is available.".into(),
        Error::TimedOut => "OSV exceeded its time limit. The scan is incomplete.".into(),
        _ => "The dependency scan could not finish. Check file access, scanner installation and advisory data.".into(),
    }
}

impl Rocket {
    fn dependency_paths(&self) -> (PathBuf, PathBuf, PathBuf) {
        let base = self.state_directory.join("dependency-security");
        let executable = self
            .dependencies
            .setup
            .program
            .clone()
            .unwrap_or_else(scan::executable);
        let cache = base.join("data");
        #[cfg(test)]
        let cache = self.dependencies.fixture_cache.clone().unwrap_or(cache);
        (executable, cache, base.join("work"))
    }
    fn start_dependency_scan(&mut self, force: bool, update: bool) {
        if self.dependencies.running()
            || !self.dependencies.setup.ready
            || self.quitting
            || (update && !self.dependencies.editable)
        {
            return;
        }
        let Some(tab) = self.tab() else { return };
        let owner = tab.id;
        let path = tab.path.clone();
        let job_path = path.clone();
        let (executable, cache, work) = self.dependency_paths();
        let state = self.dependencies.repositories.entry(owner).or_default();
        let previous = state.cached.clone();
        state.error = None;
        state.next = Some(Instant::now() + Duration::from_secs(30));
        self.dependencies.update_notice = None;
        match ReadTask::spawn(move |cancel| {
            if update {
                let inventory = scan::inventory(&path, &cancel)?;
                let ecosystems: std::collections::BTreeSet<_> = inventory
                    .files
                    .iter()
                    .map(|f| f.ecosystem.clone())
                    .collect();
                scan::update(
                    &executable,
                    &cache,
                    &work,
                    &ecosystems.into_iter().collect::<Vec<_>>(),
                    &cancel,
                )?;
                Ok(JobResult::Updated)
            } else {
                scan::scan(
                    &path,
                    &executable,
                    &cache,
                    &work,
                    previous.as_ref(),
                    force,
                    &cancel,
                )
                .map(JobResult::Scan)
            }
        }) {
            Ok(task) => {
                self.dependencies.job = Some(Job {
                    owner,
                    path: job_path,
                    update,
                    discard: false,
                    task,
                })
            }
            Err(error) => state.error = Some(error_message(&error)),
        }
    }
    pub(crate) fn poll_dependency_security(&mut self, ctx: &Context) {
        self.poll_dependency_setup(ctx);
        self.poll_dependency_push_check();
        let open: Vec<_> = self.tabs.iter().map(|t| t.id).collect();
        self.dependencies
            .repositories
            .retain(|id, _| open.contains(id));
        if self.quitting
            || self
                .dependencies
                .job
                .as_ref()
                .is_some_and(|j| !open.contains(&j.owner))
        {
            self.dependencies.cancel();
        }
        let completed = self
            .dependencies
            .job
            .as_mut()
            .and_then(|j| j.task.try_finish());
        if let Some(result) = completed {
            let job = self.dependencies.job.take().unwrap();
            if self
                .tabs
                .iter()
                .any(|t| t.id == job.owner && t.path == job.path)
                && !self.quitting
                && !job.discard
            {
                let state = self.dependencies.repositories.entry(job.owner).or_default();
                match result {
                    Ok(JobResult::Scan(cached)) => {
                        state.cached = Some(cached);
                        state.error = None;
                    }
                    Ok(JobResult::Updated) => {
                        self.dependencies.update_notice = Some(
                            "Public advisory data updated. The next scan uses the new data.".into(),
                        );
                        for state in self.dependencies.repositories.values_mut() {
                            state.next = None;
                        }
                    }
                    Err(error) => {
                        state.cached = None;
                        state.error = Some(error_message(&error));
                    }
                }
                if !job.update {
                    state_next(&mut self.dependencies.repositories, job.owner);
                }
            }
            ctx.request_repaint();
        }
        if self.dependencies.running() {
            ctx.request_repaint_after(Duration::from_millis(50));
            return;
        }
        if self.quitting
            || !self.dependencies.enabled
            || !self.dependencies.setup.ready
            || self
                .picker
                .as_ref()
                .is_some_and(|p| p.target == crate::folder_browser::PickTarget::DependencyScanner)
            || self.backend.is_none()
            || self.write.is_some()
        {
            return;
        }
        let Some(tab) = self.tab() else { return };
        if tab.snapshot.is_none() {
            return;
        }
        let due = self
            .dependencies
            .repositories
            .get(&tab.id)
            .and_then(|s| s.next)
            .is_none_or(|t| Instant::now() >= t);
        if due {
            self.start_dependency_scan(false, false);
        }
        ctx.request_repaint_after(Duration::from_secs(1));
    }
    pub(crate) fn dependency_status(&mut self, ui: &mut Ui) {
        if !self.dependencies.enabled || self.tab().is_none() {
            return;
        }
        let owner = self.tab().map(|t| t.id);
        let summary = self.dependencies.summary(owner);
        let count = summary["findings"].as_u64().unwrap_or(0);
        let text = if self.dependencies.running() {
            tr("Dependencies: checking").into()
        } else if !self.dependencies.setup.ready {
            tr("Dependencies: setup required").into()
        } else if summary["status"] != "scanned" {
            tr("Dependencies: not scanned").into()
        } else if count > 0 {
            trf!("Dependencies: {count} warnings", count)
        } else {
            tr("Dependencies").into()
        };
        let color = if count > 0
            || summary["status"] == "incomplete"
            || summary["status"] == "setup_required"
        {
            AMBER()
        } else {
            MUTED()
        };
        let r = ui.add(Button::new(RichText::new(text).size(11.0).color(color)).frame(false));
        pages::control(
            &mut self.frame_widgets,
            "dependency-status",
            &r,
            ui.clip_rect(),
        );
        if r.clicked() {
            self.show_preferences(crate::preferences::Page::Security);
        }
    }
}
fn state_next(states: &mut HashMap<u64, RepositoryScan>, owner: u64) {
    if let Some(state) = states.get_mut(&owner) {
        state.next = Some(Instant::now() + Duration::from_secs(30));
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod page_tests;
