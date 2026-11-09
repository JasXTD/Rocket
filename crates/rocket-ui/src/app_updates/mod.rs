//! Once at launch, then only on explicit request. Download and restart are separate choices.
pub mod install;
mod package;
mod view;
use crate::{app::Rocket, backend::user_error};
use eframe::egui::*;
use rocket_git::{
    ReadTask, Result,
    app_update::{AppRelease, UpdateCheck, UpdateClient, UpdateEndpoint},
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};

enum Outcome {
    Checked(UpdateCheck),
    Prepared(package::Prepared, bool),
    Handoff(install::Handoff),
    Receipt(String),
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Operation {
    #[default]
    Check,
    Download,
    Install,
}
pub struct State {
    pub endpoint: Option<UpdateEndpoint>,
    pub open: bool,
    pub started: bool,
    pub status: &'static str,
    pub message: Option<String>,
    pub error: Option<String>,
    pub release: Option<AppRelease>,
    pub last_result: Option<String>,
    pub result_directory: Option<PathBuf>,
    /// Only the native test host sets this to a packaged fixture inside its root.
    pub fixture_install: Option<PathBuf>,
    prepared: Option<package::Prepared>,
    installable: bool,
    task: Option<ReadTask<Outcome>>,
    progress: Arc<AtomicU8>,
    handoff: Option<install::Handoff>,
    notice: bool,
    notes_expanded: bool,
    details_expanded: bool,
    operation: Operation,
    check_requested: bool,
    install_task: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            endpoint: None,
            open: false,
            started: false,
            status: "idle",
            message: None,
            error: None,
            release: None,
            last_result: None,
            result_directory: None,
            fixture_install: None,
            prepared: None,
            installable: false,
            task: None,
            progress: Arc::new(AtomicU8::new(0)),
            handoff: None,
            notice: false,
            notes_expanded: false,
            details_expanded: false,
            operation: Operation::Check,
            check_requested: false,
            install_task: false,
        }
    }
}
impl State {
    pub fn running(&self) -> bool {
        self.task.is_some()
    }
    pub fn ready(&self) -> bool {
        self.prepared.is_some()
    }
    pub fn installable(&self) -> bool {
        self.installable
    }
    pub fn pending_restart(&self) -> bool {
        self.handoff.is_some()
    }
    pub fn open(&mut self) {
        self.reveal();
        if !self.running() && self.prepared.is_none() {
            self.check_requested = true;
        }
    }
    /// Reopen a known result without arming another network check.
    fn reveal(&mut self) {
        self.open = true;
        self.notice = false;
    }
    pub fn notes_expanded(&self) -> bool {
        self.notes_expanded
    }
    pub fn details_expanded(&self) -> bool {
        self.details_expanded
    }
    /// The download worker reports phases, not a byte count or percentage.
    pub fn phase(&self) -> &'static str {
        if self.status == "downloading" {
            match self.progress.load(Ordering::Acquire) {
                0 | 1 => "downloading",
                2 => "verifying_archive",
                _ => "verifying_files",
            }
        } else {
            self.status
        }
    }
    pub fn stop(&mut self) {
        if let Some(task) = &self.task {
            task.cancel();
        }
        if self.running() {
            self.status = "stopping";
        }
    }
    pub fn shutdown(&mut self) {
        if let Some(task) = &self.task {
            task.cancel();
        }
    }
    pub fn approve_restart(&mut self) -> Result<()> {
        if let Some(handoff) = &mut self.handoff {
            handoff.approve()?;
        }
        Ok(())
    }
    fn target(&self) -> Result<PathBuf> {
        #[cfg(target_os = "macos")]
        if self.fixture_install.is_none() {
            let exe = std::env::current_exe().map_err(package::io)?;
            let app = exe.parent().and_then(std::path::Path::parent).and_then(std::path::Path::parent)
                .filter(|p| p.extension().is_some_and(|s| s == "app") && p.join("Contents/MacOS").join(exe.file_name().unwrap()) == exe)
                .ok_or_else(|| package::error("Install Rocket.app in a writable folder before updating. Development executables cannot be replaced in place."))?;
            return Ok(app.to_path_buf());
        }
        self.fixture_install.clone().map(Ok).unwrap_or_else(|| {
            std::env::current_exe()
                .map_err(package::io)?
                .parent()
                .map(PathBuf::from)
                .ok_or_else(|| package::error("Installation directory is unavailable."))
        })
    }
}
impl Rocket {
    fn start_update_check(&mut self, ctx: &Context) {
        self.updates.check_requested = false;
        self.updates.started = true;
        let Some(endpoint) = self.updates.endpoint else {
            self.updates.status = "disabled";
            self.updates.message = Some("App updates are disabled in this isolated window.".into());
            return;
        };
        if self.updates.running() {
            return;
        }
        self.updates.status = "checking";
        self.updates.operation = Operation::Check;
        self.updates.message = None;
        self.updates.error = None;
        self.updates.release = None;
        self.updates.prepared = None;
        self.updates.installable = false;
        self.updates.notice = false;
        self.updates.notes_expanded = false;
        self.updates.details_expanded = false;
        let cache = self.state_directory.join("updates");
        let log = self.backend.as_ref().map(|b| b.log.clone());
        let task = crate::read_task::spawn(ctx, move |cancel| {
            package::plain(&cache)?;
            #[cfg(not(windows))]
            rocket_git::local_storage::create_protected(&cache).map_err(package::io)?;
            #[cfg(windows)]
            std::fs::create_dir_all(&cache).map_err(package::io)?;
            let read = |log| {
                UpdateClient::new(endpoint, log)?
                    .check(env!("CARGO_PKG_VERSION"), &cache, &cancel)
                    .map(Outcome::Checked)
            };
            if let Some(log) = log {
                log.request("app-update-check", |context| read(Some(context.clone())))
            } else {
                read(None)
            }
        });
        self.assign_update_task(task);
    }
    fn assign_update_task(&mut self, result: Result<ReadTask<Outcome>>) {
        match result {
            Ok(task) => self.updates.task = Some(task),
            Err(e) => {
                self.updates.status = "error";
                self.updates.error = Some(user_error(&e));
            }
        }
    }
    fn download_update(&mut self, ctx: &Context) {
        let (Some(endpoint), Some(release)) = (self.updates.endpoint, self.updates.release.clone())
        else {
            return;
        };
        if self.updates.running() {
            return;
        }
        self.updates.operation = Operation::Download;
        self.updates.details_expanded = false;
        let target = match self.updates.target() {
            Ok(p) => p,
            Err(e) => {
                self.updates.status = "error";
                self.updates.error = Some(user_error(&e));
                return;
            }
        };
        self.updates.status = "downloading";
        self.updates.progress.store(0, Ordering::Release);
        self.updates.error = None;
        self.updates.message = None;
        let cache = self.state_directory.join("updates");
        let phase = self.updates.progress.clone();
        let log = self.backend.as_ref().map(|b| b.log.clone());
        let task = crate::read_task::spawn(ctx, move |cancel| {
            let run = |log| {
                let client = UpdateClient::new(endpoint, log)?;
                let p = package::prepare(&client, &release, &cache, &phase, &cancel)?;
                Ok(Outcome::Prepared(
                    p,
                    package::is_packaged(&target),
                ))
            };
            if let Some(log) = log {
                log.request("app-update-download", |context| run(Some(context.clone())))
            } else {
                run(None)
            }
        });
        self.assign_update_task(task);
    }
    fn install_update(&mut self, ctx: &Context) {
        if self.write.is_some()
            || self.external.running()
            || self.forge.running()
            || self.ssh.running()
            || self.integrations.running()
            || self.updates.running()
        {
            return;
        }
        let Some(prepared) = self.updates.prepared.clone() else {
            return;
        };
        self.updates.operation = Operation::Install;
        self.updates.details_expanded = false;
        let target = match self.updates.target() {
            Ok(p) => p,
            Err(e) => {
                self.updates.status = "error";
                self.updates.error = Some(user_error(&e));
                return;
            }
        };
        let helper = match std::env::current_exe() {
            Ok(path) => path.parent().unwrap().join(if cfg!(target_os = "macos") { "rocket" } else { package::ENTRYPOINT }),
            Err(e) => {
                self.updates.status = "error";
                self.updates.error = Some(e.to_string());
                return;
            }
        };
        let repositories: Vec<_> = self.tabs.iter().map(|t| t.path.clone()).collect();
        self.updates.status = "preparing_restart";
        self.updates.install_task = true;
        self.updates.error = None;
        self.updates.message = None;
        let log = self.backend.as_ref().map(|b| b.log.clone());
        let task = crate::read_task::spawn(ctx, move |cancel| {
            let run = || {
                install::handoff(&prepared, &target, &helper, &repositories, &cancel)
                    .map(Outcome::Handoff)
            };
            if let Some(log) = log {
                log.request("app-update-install", |_| run())
            } else {
                run()
            }
        });
        self.assign_update_task(task);
    }
    pub(crate) fn poll_app_updates(&mut self, ctx: &Context) {
        if self.updates.handoff.is_some()
            && !self.quitting
            && !self
                .dialog
                .as_ref()
                .is_some_and(|d| matches!(d.kind, crate::app::DialogKind::Quit))
        {
            let pending = self.updates.handoff.take().unwrap();
            install::cancel(&pending);
            self.updates.status = "cancelled";
            self.updates.open = true;
            self.updates.message =
                Some("Restart cancelled. The existing installation is unchanged.".into());
            self.updates.prepared = None;
            self.updates.installable = false;
        }
        if let Some(result) = self.updates.task.as_mut().and_then(ReadTask::try_finish) {
            self.updates.task = None;
            if std::mem::take(&mut self.updates.install_task) && result.is_err() {
                // Preparation may have reserved a staging directory. A fresh
                // download creates a new transaction without deleting old files.
                self.updates.prepared = None;
                self.updates.installable = false;
            }
            match result {
                Ok(Outcome::Checked(UpdateCheck::Available(release))) => {
                    self.updates.status = "available";
                    self.updates.release = Some(release);
                    self.updates.notice = !self.updates.open;
                }
                Ok(Outcome::Checked(UpdateCheck::Current)) => {
                    self.updates.status = "current";
                    self.updates.message =
                        Some("You're up to date. No newer stable release is available.".into());
                }
                Ok(Outcome::Checked(UpdateCheck::Unavailable)) => {
                    self.updates.status = "unavailable";
                    self.updates.message=Some("No public release is available yet. The repository may still be private or have no published stable release.".into());
                }
                Ok(Outcome::Prepared(prepared, installable)) => {
                    self.updates.status = "ready";
                    self.updates.installable = installable;
                    self.updates.prepared = Some(prepared);
                }
                Ok(Outcome::Handoff(handoff)) => {
                    self.updates.status = "restarting";
                    self.updates.handoff = Some(handoff);
                    self.updates.open = false;
                    self.request_close();
                }
                Ok(Outcome::Receipt(message)) => {
                    if self.notice.is_none() {
                        self.notice = Some(message.clone());
                    }
                    self.updates.last_result = Some(message);
                }
                Err(rocket_git::Error::Cancelled) => {
                    self.updates.status = "cancelled";
                    self.updates.message =
                        Some("Update cancelled. Your current installation is unchanged.".into());
                }
                Err(e) => {
                    self.updates.status = "error";
                    self.updates.error = Some(user_error(&e));
                }
            }
        }
        if self.quitting {
            return;
        }
        if !self.updates.running()
            && let Some(job) = self.updates.result_directory.take()
        {
            let task = crate::read_task::spawn(ctx, move |_| {
                let result = package::json_file(&job.join("result.json"))?;
                let text = match result["code"].as_str() {
                    Some("installed") => {
                        "Rocket was updated successfully. The previous installation is retained beside the app."
                    }
                    Some("rolled_back") => {
                        "The update failed. Rocket restored the previous installation."
                    }
                    Some("recovery_required") => {
                        "The update needs recovery. Both installation directories were retained; inspect the update transaction before retrying."
                    }
                    _ => "The update did not finish. Your previous files were retained.",
                };
                Ok(Outcome::Receipt(text.into()))
            });
            self.assign_update_task(task);
        }
        // No time-based retry. Backend startup may finish later, but it never arms
        // a second automatic check in this process. Explicit checks reuse one task.
        if !self.updates.running()
            && (self.updates.check_requested
                || (!self.updates.started
                    && self.updates.endpoint.is_some()
                    && (self.backend.is_some() || self.startup_error.is_some())))
        {
            self.start_update_check(ctx);
        }
        if self.updates.running() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }
}
