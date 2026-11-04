//! Repository-manager discovery and one reviewed, atomic library update.
pub mod discovery;
mod view;
use crate::{
    app::*,
    backend::{Backend, user_error},
    folder_browser::PickTarget,
    repository_library::{Library, same_path},
    repository_manager::Selection,
};
use discovery::{Found, Progress, Report};
use eframe::egui::*;
use rocket_git::{Error, ReadTask, Result};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::Ordering},
};

pub struct Row {
    pub repository: Found,
    pub selected: bool,
    pub saved: bool,
    pub name: String,
    pub relative_path: String,
    search: String,
}
impl Row {
    fn new(repository: Found, root: &Path, saved: bool) -> Self {
        let name = repository
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let relative_path = relative(&repository.path, root);
        let search =
            format!("{name} {}", crate::views::display_path(&repository.path)).to_lowercase();
        Self {
            repository,
            selected: !saved,
            saved,
            name,
            relative_path,
            search,
        }
    }
}
fn relative(path: &Path, root: &Path) -> String {
    match path.strip_prefix(root) {
        Ok(p) if p.as_os_str().is_empty() => "Selected folder".into(),
        Ok(p) => p.to_string_lossy().into_owned(),
        Err(_) => crate::views::display_path(path),
    }
}
#[derive(Default)]
pub struct Summary {
    pub skipped: usize,
    pub invalid: usize,
    pub limited: bool,
    pub issues: Vec<discovery::Issue>,
}
pub struct BulkSelection {
    pub eligible: usize,
    pub selected: usize,
    pub total: usize,
    pub hidden: usize,
}
impl BulkSelection {
    pub fn checked(&self) -> bool {
        self.eligible > 0 && self.selected == self.eligible
    }
    pub fn partial(&self) -> bool {
        self.selected > 0 && self.selected < self.eligible
    }
}
enum Completion {
    Scanned(Report),
    Validated(Vec<Found>),
}
pub struct State {
    pub root: String,
    pub recursive: bool,
    pub folder: Option<u64>,
    pub rows: Vec<Row>,
    pub progress: Arc<Progress>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub stopped: bool,
    pub adding: bool,
    pub completed: bool,
    pub filter: String,
    pub hide_saved: bool,
    pub details: bool,
    pub summary: Summary,
    pub scanned_root: Option<PathBuf>,
    reset_scroll: bool,
    task: Option<ReadTask<Completion>>,
    baseline: Library,
}
impl State {
    fn new(root: String, folder: Option<u64>, baseline: Library) -> Self {
        Self {
            root,
            folder,
            baseline,
            recursive: true,
            rows: vec![],
            progress: Arc::default(),
            error: None,
            notice: None,
            stopped: false,
            adding: false,
            completed: false,
            filter: String::new(),
            hide_saved: false,
            details: false,
            summary: Summary::default(),
            scanned_root: None,
            reset_scroll: true,
            task: None,
        }
    }
    pub fn running(&self) -> bool {
        self.task.is_some()
    }
    pub fn phase(&self) -> &'static str {
        if self.running() {
            if self.stopped {
                "stopping"
            } else if self.adding {
                "validating"
            } else {
                "scanning"
            }
        } else if self.stopped {
            "cancelled"
        } else if self.completed {
            "complete"
        } else {
            "ready"
        }
    }
    pub fn selected(&self) -> Vec<PathBuf> {
        self.rows
            .iter()
            .filter(|r| r.selected && !r.saved)
            .map(|r| r.repository.path.clone())
            .collect()
    }
    pub fn visible(&self) -> Vec<usize> {
        let query = self.filter.trim().to_lowercase();
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                (!self.hide_saved || !r.saved) && (query.is_empty() || r.search.contains(&query))
            })
            .map(|(index, _)| index)
            .collect()
    }
    pub fn selection(&self) -> BulkSelection {
        let shown = self.visible();
        let eligible = shown.iter().filter(|&&i| !self.rows[i].saved).count();
        let selected = shown
            .iter()
            .filter(|&&i| !self.rows[i].saved && self.rows[i].selected)
            .count();
        let total = self.rows.iter().filter(|r| !r.saved && r.selected).count();
        BulkSelection {
            eligible,
            selected,
            total,
            hidden: total - selected,
        }
    }
    fn select_visible(&mut self, value: bool) {
        for i in self.visible() {
            if !self.rows[i].saved {
                self.rows[i].selected = value;
            }
        }
    }
    pub fn cancel(&mut self) {
        if let Some(task) = &self.task {
            task.cancel();
            self.stopped = true;
        }
    }
    fn clear(&mut self) {
        self.rows.clear();
        self.completed = false;
        self.error = None;
        self.notice = None;
        self.stopped = false;
        self.filter.clear();
        self.hide_saved = false;
        self.details = false;
        self.summary = Summary::default();
        self.scanned_root = None;
        self.reset_scroll = true;
    }
}
/// Build off a clone; failures never partially modify the saved library.
fn addition(
    current: &Library,
    baseline: &Library,
    found: Vec<Found>,
    folder: Option<u64>,
) -> std::result::Result<(Library, usize), String> {
    if current != baseline {
        return Err("Saved repositories changed. Close this scan and review again.".into());
    }
    let mut draft = current.clone();
    for item in found {
        let name = item
            .path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "Repository".into());
        draft
            .save_repository(item.path, name, folder)
            .map_err(str::to_owned)?;
    }
    draft.validate().map_err(str::to_owned)?;
    let count = draft.repositories.len() - current.repositories.len();
    Ok((draft, count))
}
impl Backend {
    pub(crate) fn scan_repositories(
        &self,
        root: &Path,
        recursive: bool,
        scope: Option<&Path>,
        progress: &Progress,
        cancel: &rocket_git::CancellationToken,
    ) -> Result<Report> {
        self.request("library-scan", false, |client| {
            discovery::scan(&client, root, recursive, scope, progress, cancel)
        })
    }
    fn check_scan_selection(
        &self,
        paths: &[PathBuf],
        scope: Option<&Path>,
        progress: &Progress,
        cancel: &rocket_git::CancellationToken,
    ) -> Result<Vec<Found>> {
        self.request("library-scan-add", false, |client| {
            paths
                .iter()
                .enumerate()
                .map(|(index, p)| {
                    let result = discovery::verify(&client, p, scope, cancel)?;
                    progress.checked.store(index + 1, Ordering::Relaxed);
                    Ok(result)
                })
                .collect()
        })
    }
}
impl Rocket {
    pub(crate) fn manager_scan(&mut self) {
        if self.manager.running()
            || self.manager.form.is_some()
            || self.manager.scan.is_some()
            || self.state_error.is_some()
        {
            return;
        }
        let root = self
            .tab()
            .and_then(|t| t.path.parent())
            .or(self.browse_root.as_deref())
            .map(crate::views::display_path)
            .unwrap_or_default();
        let folder = if let Selection::Folder(id) = self.manager.selection {
            Some(id)
        } else {
            None
        };
        self.manager.scan = Some(State::new(root, folder, self.library.clone()));
        if let Some(d) = &mut self.dialog {
            d.flag = true;
        }
    }
    pub(crate) fn manager_scan_picked(&mut self, path: &Path) {
        if let Some(s) = &mut self.manager.scan {
            s.root = crate::views::display_path(path);
            s.clear();
        }
    }
    fn start_scan(&mut self, adding: bool) {
        let Some(s) = &mut self.manager.scan else {
            return;
        };
        if s.running() {
            return;
        }
        let Some(backend) = self.backend.clone() else {
            return;
        };
        if self.library != s.baseline {
            s.error = Some("Saved repositories changed. Close this scan and review again.".into());
            return;
        }
        let scope = self.browse_root.clone();
        s.error = None;
        s.notice = None;
        s.stopped = false;
        s.adding = adding;
        let task = if adding {
            let paths = s.selected();
            if paths.is_empty() {
                return;
            }
            s.progress.checked.store(0, Ordering::Relaxed);
            let progress = s.progress.clone();
            ReadTask::spawn(move |cancel| {
                backend
                    .check_scan_selection(&paths, scope.as_deref(), &progress, &cancel)
                    .map(Completion::Validated)
            })
        } else {
            s.clear();
            s.progress = Arc::default();
            let progress = s.progress.clone();
            let root = PathBuf::from(s.root.trim());
            let recursive = s.recursive;
            ReadTask::spawn(move |cancel| {
                backend
                    .scan_repositories(&root, recursive, scope.as_deref(), &progress, &cancel)
                    .map(Completion::Scanned)
            })
        };
        match task {
            Ok(task) => s.task = Some(task),
            Err(e) => s.error = Some(user_error(&e)),
        }
    }
    pub(crate) fn poll_manager_scan(&mut self) {
        let Some(s) = &mut self.manager.scan else {
            return;
        };
        let Some(result) = s.task.as_mut().and_then(ReadTask::try_finish) else {
            return;
        };
        s.task = None;
        if s.stopped {
            s.notice = Some("Stopped. No repositories were added.".into());
            return;
        }
        match result {
            Err(Error::Cancelled) => {
                s.stopped = true;
                s.notice = Some("Stopped. No repositories were added.".into());
            }
            Err(e) => s.error = Some(format!("Nothing was added. {}", user_error(&e))),
            Ok(Completion::Scanned(report)) => {
                s.completed = true;
                s.scanned_root = Some(report.root.clone());
                s.rows = report
                    .repositories
                    .into_iter()
                    .map(|repository| {
                        let saved = self
                            .library
                            .repositories
                            .iter()
                            .any(|r| same_path(&r.path, &repository.path));
                        Row::new(repository, &report.root, saved)
                    })
                    .collect();
                s.notice = Some(format!(
                    "{} found · {} skipped entries (links or unreadable) · {} invalid repositories{}",
                    s.rows.len(),
                    report.skipped,
                    report.invalid,
                    if report.limited {
                        " · Scan limit reached; choose a narrower folder to find the rest."
                    } else {
                        ""
                    }
                ));
                s.summary = Summary {
                    skipped: report.skipped,
                    invalid: report.invalid,
                    limited: report.limited,
                    issues: report.issues,
                };
            }
            Ok(Completion::Validated(found)) => {
                match addition(&self.library, &s.baseline, found, s.folder) {
                    Ok((library, count)) => {
                        self.library = library;
                        self.manager.notice =
                            Some(format!("Added {count} repositories · no tabs opened"));
                        self.manager.error = None;
                        self.manager.selection = Selection::All;
                        self.manager.scan = None;
                        if let Some(d) = &mut self.dialog {
                            d.flag = false;
                        }
                    }
                    Err(e) => s.error = Some(e),
                }
            }
        }
    }
    pub(crate) fn manager_scan_dialog(&mut self, ctx: &Context) {
        let Some(s) = &mut self.manager.scan else {
            return;
        };
        let busy = s.running();
        let action = view::show(ctx, s, &mut self.frame_widgets);
        if action.close {
            if busy {
                s.cancel();
            } else {
                self.manager.scan = None;
                if let Some(d) = &mut self.dialog {
                    d.flag = false;
                }
            }
        } else if action.stop {
            s.cancel();
        } else if action.start {
            self.start_scan(false);
        } else if action.add {
            self.start_scan(true);
        } else if action.change {
            s.clear();
        } else if action.browse {
            let current = s.root.clone();
            self.browse(PickTarget::LibraryScanFolder, &current);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filtered_bulk_selection_preserves_hidden_rows_and_never_checks_empty_sets() {
        let root = std::env::current_dir().unwrap();
        let mut state = State::new(String::new(), None, Library::default());
        state.rows = vec![
            Row::new(found("saved"), &root, true),
            Row::new(found("日本語/資料"), &root, false),
            Row::new(found("hidden/PROJECT"), &root, false),
        ];
        assert!(state.selection().checked());
        assert!(!state.selection().partial());
        state.filter = "日本語".into();
        assert_eq!(state.visible(), vec![1]);
        assert_eq!(state.selection().hidden, 1);
        state.select_visible(false);
        assert!(!state.rows[1].selected);
        assert!(state.rows[2].selected);
        state.filter.clear();
        assert!(state.selection().partial());
        assert!(!state.selection().checked());
        state.filter = " PrOjEcT ".into();
        assert_eq!(state.visible(), vec![2]);
        state.filter = "no match".into();
        assert!(!state.selection().checked());
        state.select_visible(false);
        assert!(state.rows[2].selected);
        state.filter = "saved".into();
        assert!(!state.selection().checked());
        assert_eq!(state.selection().eligible, 0);
        state.hide_saved = true;
        assert!(state.visible().is_empty());
        assert_eq!(state.rows[1].name, "資料");
        assert_eq!(
            state.rows[1].relative_path,
            Path::new("日本語/資料").to_string_lossy()
        );
        assert_eq!(relative(&root, &root), "Selected folder");
    }
    fn found(name: &str) -> Found {
        Found {
            path: std::env::current_dir().unwrap().join(name),
            kind: "Repository",
        }
    }
    #[test]
    fn batch_preserves_duplicates_and_is_atomic_on_capacity_and_stale_library() {
        let mut library = Library::default();
        let folder = library.add_folder("Keep folder".into()).unwrap();
        library
            .save_repository(found("existing").path, "Keep label".into(), Some(folder))
            .unwrap();
        let (added, count) = addition(
            &library,
            &library,
            vec![found("existing"), found("日本語 🚀"), found("日本語 🚀")],
            None,
        )
        .unwrap();
        assert_eq!(count, 1);
        assert_eq!(added.repositories[0], library.repositories[0]);
        assert!(addition(&added, &library, vec![found("other")], None).is_err());
        assert!(addition(&library, &library, vec![found("other")], Some(u64::MAX)).is_err());
        for i in 1..crate::repository_library::MAX_REPOSITORIES {
            library
                .save_repository(found(&format!("repo-{i}")).path, format!("Repo {i}"), None)
                .unwrap();
        }
        let before = library.clone();
        assert!(addition(&library, &library, vec![found("overflow")], None).is_err());
        assert_eq!(library, before);
    }
    #[test]
    fn cancelled_validation_cannot_publish_a_completed_result() {
        let fixture = crate::test_common::Fixture::new();
        let mut app =
            Rocket::with_log_directory(&Context::default(), vec![], fixture.root.join("logs"));
        let before = app.library.clone();
        let mut scan = State::new(String::new(), None, before.clone());
        let (ready_rx_sender, ready_rx) = std::sync::mpsc::channel();
        let (go, wait) = std::sync::mpsc::channel();
        scan.adding = true;
        scan.task = Some(
            ReadTask::spawn(move |_| {
                ready_rx_sender.send(()).unwrap();
                wait.recv().unwrap();
                Ok(Completion::Validated(vec![found("cancelled")]))
            })
            .unwrap(),
        );
        ready_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        scan.cancel();
        assert_eq!(scan.phase(), "stopping");
        go.send(()).unwrap();
        app.manager.scan = Some(scan);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.manager.running() {
            app.poll_manager_scan();
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(app.library, before);
        assert_eq!(app.manager.scan.as_ref().unwrap().phase(), "cancelled");
    }
}
