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
    /// The disk folder holding it, relative to the scanned folder, with `/` between
    /// names. Empty for repositories directly in the scanned folder.
    pub group: String,
    /// The branch and the date of its latest commit, when the scan could read them.
    pub meta: String,
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
        let group = group(&repository.path, root);
        let meta = meta(&repository);
        let search =
            format!("{name} {}", crate::views::display_path(&repository.path)).to_lowercase();
        Self {
            repository,
            selected: !saved,
            saved,
            name,
            relative_path,
            group,
            meta,
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
fn group(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .ok()
        .and_then(Path::parent)
        .map(|p| {
            p.components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/")
        })
        .unwrap_or_default()
}
fn meta(found: &Found) -> String {
    let Some(tip) = &found.tip else {
        return String::new();
    };
    let now = crate::activity_history::now_ms();
    let branch = tip
        .branch
        .as_ref()
        .map_or_else(|| "detached".into(), |b| String::from_utf8_lossy(b).into_owned());
    format!("{branch} · {}", crate::activity_history::short_date(tip.time, now))
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
/// Which results the list shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    #[default]
    All,
    New,
    Saved,
    Skipped,
}
impl View {
    pub fn code(self) -> &'static str {
        match self {
            View::All => "all",
            View::New => "new",
            View::Saved => "saved",
            View::Skipped => "skipped",
        }
    }
}
/// Where an added repository goes in the library.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Placement {
    Folder(Option<u64>),
    /// A library folder named after the disk folder, made if it doesn't exist.
    Named(String),
}
enum Completion {
    Scanned(Report),
    Validated(Vec<Found>),
}
pub struct State {
    pub root: String,
    pub recursive: bool,
    pub folder: Option<u64>,
    /// Save in: one library folder per disk folder, instead of `folder`.
    pub match_disk: bool,
    pub rows: Vec<Row>,
    pub progress: Arc<Progress>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub stopped: bool,
    pub adding: bool,
    pub completed: bool,
    pub filter: String,
    pub view: View,
    pub summary: Summary,
    pub scanned_root: Option<PathBuf>,
    /// Save in follows the results until it is chosen by hand.
    save_in_chosen: bool,
    reset_scroll: bool,
    task: Option<ReadTask<Completion>>,
    placements: Vec<(PathBuf, Placement)>,
    baseline: Library,
}
impl State {
    fn new(root: String, folder: Option<u64>, baseline: Library) -> Self {
        Self {
            root,
            folder,
            baseline,
            recursive: true,
            match_disk: false,
            save_in_chosen: folder.is_some(),
            rows: vec![],
            progress: Arc::default(),
            error: None,
            notice: None,
            stopped: false,
            adding: false,
            completed: false,
            filter: String::new(),
            view: View::All,
            summary: Summary::default(),
            scanned_root: None,
            reset_scroll: true,
            task: None,
            placements: vec![],
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
    /// Rows the list shows: the chosen view and the search. Skipped shows none.
    pub fn visible(&self) -> Vec<usize> {
        let query = self.filter.trim().to_lowercase();
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| match self.view {
                View::All => true,
                View::New => !r.saved,
                View::Saved => r.saved,
                View::Skipped => false,
            })
            .filter(|(_, r)| query.is_empty() || r.search.contains(&query))
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
    /// New repositories the user left unselected, shown or not.
    pub fn left_out(&self) -> usize {
        self.rows.iter().filter(|r| !r.saved && !r.selected).count()
    }
    /// A group's box changes only its rows the list shows.
    fn select_group(&mut self, group: &str, value: bool) {
        for i in self.visible() {
            if !self.rows[i].saved && self.rows[i].group == group {
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
        self.view = View::All;
        self.summary = Summary::default();
        self.scanned_root = None;
        self.reset_scroll = true;
    }
    /// Where each selected repository goes, from Save in.
    fn placement(&self, row: &Row) -> Placement {
        if self.match_disk && !row.group.is_empty() {
            Placement::Named(row.group.clone())
        } else if self.match_disk {
            Placement::Folder(None)
        } else {
            Placement::Folder(self.folder)
        }
    }
    /// Library folders Match disk folders would make, in list order.
    pub fn new_folders(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for row in self.rows.iter().filter(|r| r.selected && !r.saved && !r.group.is_empty()) {
            let exists = self
                .baseline
                .folders
                .iter()
                .any(|f| f.name.to_lowercase() == row.group.to_lowercase());
            if !exists && !names.iter().any(|n| n.to_lowercase() == row.group.to_lowercase()) {
                names.push(row.group.clone());
            }
        }
        names
    }
    pub fn has_groups(&self) -> bool {
        self.rows.iter().any(|r| !r.group.is_empty())
    }
}
/// Build off a clone; failures never partially modify the saved library.
fn addition(
    current: &Library,
    baseline: &Library,
    found: Vec<(Found, Placement)>,
) -> std::result::Result<(Library, usize), String> {
    if current != baseline {
        return Err("Saved repositories changed. Close this scan and review again.".into());
    }
    let mut draft = current.clone();
    for (item, placement) in found {
        let folder = match placement {
            Placement::Folder(folder) => folder,
            Placement::Named(name) => match draft
                .folders
                .iter()
                .find(|f| f.name.to_lowercase() == name.to_lowercase())
            {
                Some(f) => Some(f.id),
                None => Some(draft.add_folder(name).map_err(str::to_owned)?),
            },
        };
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
                    let result = discovery::verify(&client, p, scope, false, cancel)?;
                    progress.checked.store(index + 1, Ordering::Relaxed);
                    Ok(result)
                })
                .collect()
        })
    }
}
impl Rocket {
    /// Scan folder starts with the folder picker; choosing a folder starts the scan.
    pub(crate) fn manager_scan(&mut self) {
        if self.manager.running()
            || self.manager.form.is_some()
            || self.manager.scan.is_some()
            || self.state_error.is_some()
        {
            return;
        }
        let start = self
            .general
            .source_folder
            .clone()
            .filter(|p| p.is_dir())
            .or_else(|| self.tab().and_then(|t| t.path.parent()).map(Path::to_path_buf))
            .or_else(|| self.browse_root.clone())
            .map(|p| crate::views::display_path(&p))
            .unwrap_or_default();
        self.browse(PickTarget::LibraryScanFolder, &start);
    }
    pub(crate) fn manager_scan_picked(&mut self, path: &Path) {
        let root = crate::views::display_path(path);
        match &mut self.manager.scan {
            Some(s) => {
                if s.running() {
                    return;
                }
                s.root = root;
                s.clear();
            }
            None => {
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
        }
        self.start_scan(false);
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
            s.placements = s
                .rows
                .iter()
                .filter(|r| r.selected && !r.saved)
                .map(|r| (r.repository.path.clone(), s.placement(r)))
                .collect();
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
                // Disk folders become library folders unless one was chosen.
                if !s.save_in_chosen {
                    s.match_disk = s.has_groups();
                }
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
                let placements = std::mem::take(&mut s.placements);
                let found = found
                    .into_iter()
                    .map(|f| {
                        let placement = placements
                            .iter()
                            .find(|(p, _)| same_path(p, &f.path))
                            .map_or(Placement::Folder(s.folder), |(_, p)| p.clone());
                        (f, placement)
                    })
                    .collect();
                match addition(&self.library, &s.baseline, found) {
                    Ok((library, count)) => {
                        self.library = library;
                        self.manager.notice = Some(if count == 1 {
                            "Added 1 repository · no tabs opened".into()
                        } else {
                            format!("Added {count} repositories · no tabs opened")
                        });
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
        } else if action.change || action.browse {
            let current = s
                .scanned_root
                .as_deref()
                .map(crate::views::display_path)
                .unwrap_or_else(|| s.root.clone());
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
        state.select_group("日本語", false);
        assert!(!state.rows[1].selected);
        assert!(state.rows[2].selected);
        state.filter.clear();
        assert!(state.selection().partial());
        assert!(!state.selection().checked());
        assert_eq!(state.left_out(), 1);
        state.filter = " PrOjEcT ".into();
        assert_eq!(state.visible(), vec![2]);
        state.filter = "no match".into();
        assert!(!state.selection().checked());
        state.select_group("hidden", false);
        assert!(state.rows[2].selected);
        state.filter = "saved".into();
        assert!(!state.selection().checked());
        assert_eq!(state.selection().eligible, 0);
        state.view = View::New;
        assert!(state.visible().is_empty());
        state.filter.clear();
        assert_eq!(state.visible(), vec![1, 2]);
        state.view = View::Saved;
        assert_eq!(state.visible(), vec![0]);
        state.view = View::Skipped;
        assert!(state.visible().is_empty());
        assert_eq!(state.rows[1].name, "資料");
        assert_eq!(
            state.rows[1].relative_path,
            Path::new("日本語/資料").to_string_lossy()
        );
        assert_eq!(relative(&root, &root), "Selected folder");
    }
    #[test]
    fn groups_follow_disk_folders_and_toggle_only_shown_new_rows() {
        let root = std::env::current_dir().unwrap();
        let mut state = State::new(String::new(), None, Library::default());
        state.rows = vec![
            Row::new(found("top"), &root, false),
            Row::new(found("client/north/shop"), &root, false),
            Row::new(found("client/north/admin"), &root, false),
            Row::new(found("client/north/old"), &root, true),
            Row::new(found("infra/terraform"), &root, false),
        ];
        let groups: Vec<_> = state.rows.iter().map(|r| r.group.as_str()).collect();
        assert_eq!(groups, ["", "client/north", "client/north", "client/north", "infra"]);
        assert!(state.has_groups());
        state.filter = "shop".into();
        state.select_group("client/north", false);
        assert!(!state.rows[1].selected && state.rows[2].selected && state.rows[3].saved);
        state.filter.clear();
        state.select_group("client/north", false);
        assert!(!state.rows[2].selected);
        state.select_group("client/north", true);
        assert!(state.rows[1].selected && state.rows[2].selected && !state.rows[3].selected);
        // Match disk folders names only folders that don't exist yet.
        state.baseline.add_folder("INFRA".into()).unwrap();
        state.match_disk = true;
        assert_eq!(state.new_folders(), ["client/north"]);
        assert_eq!(state.placement(&state.rows[0]), Placement::Folder(None));
        assert_eq!(state.placement(&state.rows[4]), Placement::Named("infra".into()));
        state.match_disk = false;
        state.folder = Some(7);
        assert_eq!(state.placement(&state.rows[4]), Placement::Folder(Some(7)));
    }
    fn found(name: &str) -> Found {
        Found {
            path: std::env::current_dir().unwrap().join(name),
            kind: "Repository",
            tip: None,
        }
    }
    #[test]
    fn batch_preserves_duplicates_and_is_atomic_on_capacity_and_stale_library() {
        let mut library = Library::default();
        let folder = library.add_folder("Keep folder".into()).unwrap();
        library
            .save_repository(found("existing").path, "Keep label".into(), Some(folder))
            .unwrap();
        let none = |f: Found| (f, Placement::Folder(None));
        let (added, count) = addition(
            &library,
            &library,
            vec![none(found("existing")), none(found("日本語 🚀")), none(found("日本語 🚀"))],
        )
        .unwrap();
        assert_eq!(count, 1);
        assert_eq!(added.repositories[0], library.repositories[0]);
        assert!(addition(&added, &library, vec![none(found("other"))]).is_err());
        assert!(
            addition(
                &library,
                &library,
                vec![(found("other"), Placement::Folder(Some(u64::MAX)))]
            )
            .is_err()
        );
        for i in 1..crate::repository_library::MAX_REPOSITORIES {
            library
                .save_repository(found(&format!("repo-{i}")).path, format!("Repo {i}"), None)
                .unwrap();
        }
        let before = library.clone();
        assert!(addition(&library, &library, vec![none(found("overflow"))]).is_err());
        assert_eq!(library, before);
    }
    #[test]
    fn matched_disk_folders_reuse_names_and_are_made_once() {
        let mut library = Library::default();
        let infra = library.add_folder("Infra".into()).unwrap();
        let (added, count) = addition(
            &library,
            &library,
            vec![
                (found("infra/terraform"), Placement::Named("infra".into())),
                (found("client/north/shop"), Placement::Named("client/north".into())),
                (found("client/north/admin"), Placement::Named("client/north".into())),
                (found("top"), Placement::Folder(None)),
            ],
        )
        .unwrap();
        assert_eq!(count, 4);
        assert_eq!(added.folders.len(), 2);
        let north = added.folders.iter().find(|f| f.name == "client/north").unwrap().id;
        let folder_of = |name: &str| {
            added
                .repositories
                .iter()
                .find(|r| r.name == name)
                .unwrap()
                .folder
        };
        assert_eq!(folder_of("terraform"), Some(infra));
        assert_eq!(folder_of("shop"), Some(north));
        assert_eq!(folder_of("admin"), Some(north));
        assert_eq!(folder_of("top"), None);
        // A disk folder name the library can't hold fails the whole batch.
        assert!(
            addition(
                &library,
                &library,
                vec![(found("bad"), Placement::Named(" padded ".into()))]
            )
            .is_err()
        );
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
