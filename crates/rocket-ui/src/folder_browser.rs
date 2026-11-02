//! The folder browser behind File → Open and the Browse buttons in Preferences:
//! places on the left, folders on the right with Git repositories marked by their
//! branch, and a typed path for anything else. Folder reads run on a worker and only
//! read. In the MCP test host browsing stays inside the fixture folder.
use crate::{
    app::{DialogKind, Rocket, UiControl},
    pages,
    style::*,
};
use eframe::egui::*;
use rocket_git::{CancellationToken, Error, ExternalToolKind, ReadTask, Result};
use std::{
    collections::HashMap,
    path::{Component, Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Folders and programs listed per folder; a larger folder shows the first ones.
const LIMIT: usize = 2_000;
/// `.git` pointer files and `HEAD` are tiny; reads stop here.
const POINTER_LIMIT: u64 = 4_096;
const ROW: f32 = 36.0;
const PLACE_ROW: f32 = 30.0;
pub(crate) mod scope;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// File → Open: a Git repository.
    Repository,
    /// Any folder, such as the MCP test repositories folder.
    Folder,
    /// A program (`.exe`) for external tools and AI drafts.
    Program,
    File,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    /// A program file rather than a folder (Program mode only).
    pub program: bool,
    /// For a folder holding a Git repository: its branch, `detached at …`, or empty
    /// when `HEAD` could not be read.
    pub repository: Option<String>,
    pub modified: Option<SystemTime>,
}

#[derive(Clone, Debug)]
pub struct Listing {
    pub folder: PathBuf,
    pub entries: Vec<Entry>,
    /// The folder itself holds a Git repository: its branch.
    pub repository: Option<String>,
    pub truncated: bool,
}

/// Read one folder for the browser: subfolders (and programs in Program mode), sorted
/// by name with folders first. Hidden and system entries and `.git` itself are left
/// out; a scope also validates the target and metadata before any directory read.
pub fn read(folder: &Path, mode: Mode, scope: Option<&Path>, cancel: &CancellationToken) -> Result<Listing> {
    read_limited(folder, mode, scope, LIMIT, cancel)
}

fn read_limited(
    folder: &Path,
    mode: Mode,
    scope: Option<&Path>,
    limit: usize,
    cancel: &CancellationToken,
) -> Result<Listing> {
    cancel.check()?;
    let _guards = scope.map(|root| scope::pin_directory(folder, root)).transpose()?;
    let items = std::fs::read_dir(folder).map_err(|source| Error::Io {
        action: "read the folder",
        source,
    })?;
    let mut entries = Vec::new();
    let mut truncated = false;
    for item in items {
        cancel.check()?;
        let Ok(item) = item else { continue };
        // A directory entry's metadata describes the entry itself, not a link target.
        let Ok(metadata) = item.metadata() else { continue };
        let name = item.file_name().to_string_lossy().into_owned();
        let link = linked(&metadata);
        if hidden(&name, &metadata) || (scope.is_some() && link) {
            continue;
        }
        let path = item.path();
        let directory = if link { path.is_dir() } else { metadata.is_dir() };
        let entry = if directory && !name.eq_ignore_ascii_case(".git") {
            Entry {
                repository: repository_head(&path, scope),
                program: false,
                modified: metadata.modified().ok(),
                name,
                path,
            }
        } else if !directory && (mode == Mode::File || mode == Mode::Program && is_program(&name)) {
            Entry {
                repository: None,
                program: true,
                modified: metadata.modified().ok(),
                name,
                path,
            }
        } else {
            continue;
        };
        if entries.len() == limit {
            truncated = true;
            break;
        }
        entries.push(entry);
    }
    entries.sort_by(|a, b| {
        a.program
            .cmp(&b.program)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(Listing {
        folder: folder.to_path_buf(),
        repository: repository_head(folder, scope),
        entries,
        truncated,
    })
}

fn linked(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0 // FILE_ATTRIBUTE_REPARSE_POINT
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

/// Hidden or system entries, as File Explorer hides them by default.
fn hidden(name: &str, metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        let _ = name;
        metadata.file_attributes() & 0x6 != 0 // FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM
    }
    #[cfg(not(windows))]
    {
        let _ = metadata;
        name.starts_with('.')
    }
}

fn is_program(name: &str) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("exe"))
}

/// The branch of the Git repository whose working tree is `folder`, if it is one:
/// the branch name, `detached at …`, or empty when `HEAD` cannot be read.
fn repository_head(folder: &Path, scope: Option<&Path>) -> Option<String> {
    let _guards = scope.map(|root| scope::pin_directory(folder, root)).transpose().ok()?;
    let dot_git = folder.join(".git");
    if scope.is_some() && linked(&std::fs::symlink_metadata(&dot_git).ok()?) { return None; }
    let metadata = std::fs::metadata(&dot_git).ok()?;
    let git_dir = if metadata.is_dir() {
        dot_git
    } else if metadata.is_file() {
        // Linked worktrees and submodules: a `.git` file names the Git directory.
        let pointer = read_small(&dot_git, scope)?;
        let target = PathBuf::from(pointer.lines().next()?.strip_prefix("gitdir:")?.trim());
        if target.is_absolute() {
            target
        } else {
            folder.join(target)
        }
    } else {
        return None;
    };
    let git_dir = if let Some(root) = scope {
        let resolved = git_dir.canonicalize().ok()?;
        if !inside(&resolved, root) { return None; }
        resolved
    } else { git_dir };
    Some(read_small(&git_dir.join("HEAD"), scope).map_or_else(String::new, |head| branch_label(&head)))
}

fn read_small(path: &Path, scope: Option<&Path>) -> Option<String> {
    if let Some(root) = scope { return scope::read_small(path, root, POINTER_LIMIT); }
    use std::io::Read;
    let mut text = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(POINTER_LIMIT)
        .read_to_string(&mut text)
        .ok()?;
    Some(text)
}

/// `ref: refs/heads/main` → `main`; a commit ID → `detached at a1b2c3d`.
pub(crate) fn branch_label(head: &str) -> String {
    let head = head.trim();
    if let Some(reference) = head.strip_prefix("ref:") {
        let reference = reference.trim();
        return reference
            .strip_prefix("refs/heads/")
            .unwrap_or(reference)
            .to_owned();
    }
    if head.len() >= 40 && head.bytes().all(|b| b.is_ascii_hexdigit()) {
        return format!("detached at {}", &head[..7]);
    }
    String::new()
}

/// Clickable segments of a folder path: the drive (or share) and each folder.
pub(crate) fn crumbs(folder: &Path) -> Vec<(String, PathBuf)> {
    let mut crumbs = Vec::new();
    let mut path = PathBuf::new();
    for component in folder.components() {
        match component {
            Component::Prefix(prefix) => {
                path.push(prefix.as_os_str());
                // Canonical paths spell C: as \\?\C:; show the usual form.
                let label = match prefix.kind() {
                    std::path::Prefix::Disk(d) | std::path::Prefix::VerbatimDisk(d) => {
                        format!("{}:", char::from(d))
                    }
                    std::path::Prefix::UNC(server, share)
                    | std::path::Prefix::VerbatimUNC(server, share) => format!(
                        r"\\{}\{}",
                        server.to_string_lossy(),
                        share.to_string_lossy()
                    ),
                    _ => prefix.as_os_str().to_string_lossy().into_owned(),
                };
                crumbs.push((label, path.clone()));
            }
            Component::RootDir => {
                path.push(Component::RootDir.as_os_str());
                match crumbs.last_mut() {
                    Some(last) => last.1 = path.clone(),
                    None => crumbs.push((path.to_string_lossy().into_owned(), path.clone())),
                }
            }
            Component::Normal(name) => {
                path.push(name);
                crumbs.push((name.to_string_lossy().into_owned(), path.clone()));
            }
            Component::CurDir | Component::ParentDir => {}
        }
    }
    crumbs
}

/// Existing ancestors must resolve inside the root and contain no links/junctions.
/// Missing descendants remain valid for New/Clone previews, not directory reads.
pub(crate) fn inside(path: &Path, scope: &Path) -> bool {
    scope::inside(path, scope)
}

/// A typed folder: absolute, without `.` or `..`, and inside the scope when there is one.
fn typed_folder(text: &str, scope: Option<&Path>) -> std::result::Result<PathBuf, String> {
    let path = PathBuf::from(text.trim().trim_matches('"'));
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::CurDir | Component::ParentDir))
    {
        return Err("Enter a full folder path, such as C:\\Projects\\app.".into());
    }
    if scope.is_some_and(|s| !inside(&path, s)) {
        return Err("This test window can only browse its test repositories folder.".into());
    }
    Ok(path)
}

pub(crate) fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

#[derive(Clone)]
pub struct Place {
    pub label: String,
    pub path: PathBuf,
    pub icon: Icon,
}

/// Places beside the folder list: the user's folders and the drives, or only the
/// fixture folder in the MCP test host. Runs on the worker: it checks folders exist.
fn places(scope: Option<&Path>) -> (Vec<Place>, Vec<Place>) {
    if let Some(scope) = scope {
        let place = Place {
            label: "Test repositories".into(),
            path: scope.to_path_buf(),
            icon: Icon::Folder,
        };
        return (vec![place], Vec::new());
    }
    let mut folders = Vec::new();
    if let Some(home) = home() {
        let label = home
            .file_name()
            .map_or_else(|| "Home".into(), |n| n.to_string_lossy().into_owned());
        folders.push(Place {
            label,
            path: home.clone(),
            icon: Icon::Home,
        });
        for (name, icon) in [
            ("Desktop", Icon::Monitor),
            ("Documents", Icon::File),
            ("Downloads", Icon::Pull),
        ] {
            // Folders redirected to OneDrive live under it instead.
            if let Some(path) = [home.join(name), home.join("OneDrive").join(name)]
                .into_iter()
                .find(|p| p.is_dir())
            {
                folders.push(Place {
                    label: name.into(),
                    path,
                    icon,
                });
            }
        }
    }
    let drives = rocket_git::drive_letters()
        .into_iter()
        .map(|letter| Place {
            label: format!("{letter}:"),
            path: PathBuf::from(format!("{letter}:\\")),
            icon: Icon::Drive,
        })
        .collect();
    (folders, drives)
}

/// A recent repository for the places column.
pub struct Recent {
    pub path: PathBuf,
    pub branch: Option<String>,
}

/// What the browser asks its dialog to do.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    /// Double-click or Enter on something this mode picks.
    Choose(PathBuf),
    /// Repository mode: start a repository in this plain folder.
    CreateHere(PathBuf),
}

pub struct Browser {
    pub mode: Mode,
    /// Set only in the MCP test host: browsing never leaves this folder.
    scope: Option<PathBuf>,
    folder: PathBuf,
    listing: Option<Listing>,
    error: Option<String>,
    /// The selected entry's name in the current listing.
    selected: Option<String>,
    pub filter: String,
    /// The path being typed, while the path bar is a text field.
    editing: Option<String>,
    focus_path: bool,
    history: Vec<PathBuf>,
    read: Option<ReadTask<Listing>>,
    places: (Vec<Place>, Vec<Place>),
    places_read: Option<ReadTask<(Vec<Place>, Vec<Place>)>>,
    scroll_to: Option<usize>,
}

impl Browser {
    pub fn new(mode: Mode, scope: Option<PathBuf>, start: PathBuf, select: Option<String>) -> Self {
        let start = match &scope {
            Some(scope) if !inside(&start, scope) => scope.clone(),
            _ => start,
        };
        let place_scope = scope.clone();
        let mut browser = Self {
            mode,
            scope,
            folder: start.clone(),
            listing: None,
            error: None,
            selected: None,
            filter: String::new(),
            editing: None,
            focus_path: false,
            history: Vec::new(),
            read: None,
            places: (Vec::new(), Vec::new()),
            places_read: ReadTask::spawn(move |_| Ok(places(place_scope.as_deref()))).ok(),
            scroll_to: None,
        };
        browser.load(start, select);
        browser
    }

    pub fn folder(&self) -> &Path {
        &self.folder
    }

    pub fn loading(&self) -> bool {
        self.read.is_some()
    }

    fn within(&self, path: &Path) -> bool {
        self.scope.as_ref().is_none_or(|s| inside(path, s))
    }

    /// Show `folder`, selecting `select` once it is listed. A newer read replaces an
    /// older one; dropping the older task cancels it.
    fn load(&mut self, folder: PathBuf, select: Option<String>) {
        self.folder = folder.clone();
        self.listing = None;
        self.error = None;
        self.selected = select;
        self.filter.clear();
        self.editing = None;
        self.scroll_to = None;
        let (mode, scope) = (self.mode, self.scope.clone());
        match ReadTask::spawn(move |cancel| read(&folder, mode, scope.as_deref(), &cancel)) {
            Ok(task) => self.read = Some(task),
            Err(error) => {
                self.read = None;
                self.error = Some(crate::backend::user_error(&error));
            }
        }
    }

    pub fn go(&mut self, folder: PathBuf, select: Option<String>) {
        if !self.within(&folder) || folder == self.folder {
            return;
        }
        self.history.push(self.folder.clone());
        self.load(folder, select);
    }

    pub fn up(&mut self) {
        let Some(parent) = self.folder.parent().map(Path::to_path_buf) else {
            return;
        };
        let name = self
            .folder
            .file_name()
            .map(|n| n.to_string_lossy().into_owned());
        self.go(parent, name);
    }

    pub fn back(&mut self) {
        if let Some(previous) = self.history.pop() {
            self.load(previous, None);
        }
    }

    pub fn can_go_up(&self) -> bool {
        self.folder.parent().is_some_and(|p| self.within(p))
    }

    pub fn poll(&mut self, ctx: &Context) {
        if let Some(result) = self.read.as_mut().and_then(ReadTask::try_finish) {
            self.read = None;
            match result {
                Ok(listing) if listing.folder == self.folder => {
                    if self
                        .selected
                        .as_ref()
                        .is_some_and(|s| !listing.entries.iter().any(|e| &e.name == s))
                    {
                        self.selected = None;
                    }
                    self.scroll_to = self
                        .selected
                        .as_ref()
                        .and_then(|s| listing.entries.iter().position(|e| &e.name == s));
                    self.listing = Some(listing);
                }
                Ok(_) => {}
                Err(error) => self.error = Some(crate::backend::user_error(&error)),
            }
        }
        if let Some(result) = self.places_read.as_mut().and_then(ReadTask::try_finish) {
            self.places_read = None;
            if let Ok(places) = result {
                self.places = places;
            }
        }
        if self.read.is_some() || self.places_read.is_some() {
            ctx.request_repaint_after(Duration::from_millis(30));
        }
    }

    fn entry(&self, name: &str) -> Option<&Entry> {
        self.listing.as_ref()?.entries.iter().find(|e| e.name == name)
    }

    fn selected_entry(&self) -> Option<&Entry> {
        self.selected.as_deref().and_then(|name| self.entry(name))
    }

    /// The repository this browser would open: the selected repository, or the
    /// current folder when it is one and no plain folder is selected.
    pub fn repository(&self) -> Option<(PathBuf, String)> {
        match self.selected_entry() {
            Some(entry) => entry
                .repository
                .clone()
                .map(|branch| (entry.path.clone(), branch)),
            None => self
                .listing
                .as_ref()
                .and_then(|l| l.repository.clone())
                .map(|branch| (self.folder.clone(), branch)),
        }
    }

    /// What Choose picks in Folder and Program mode.
    pub fn choice(&self) -> Option<PathBuf> {
        match self.mode {
            Mode::Repository => self.repository().map(|(path, _)| path),
            Mode::Folder => Some(
                self.selected_entry()
                    .map_or_else(|| self.folder.clone(), |e| e.path.clone()),
            ),
            Mode::Program | Mode::File => self
                .selected_entry()
                .filter(|e| e.program)
                .map(|e| e.path.clone()),
        }
    }

    fn visible(&self) -> Vec<&Entry> {
        let filter = self.filter.trim().to_lowercase();
        self.listing.as_ref().map_or_else(Vec::new, |l| {
            l.entries
                .iter()
                .filter(|e| filter.is_empty() || e.name.to_lowercase().contains(&filter))
                .collect()
        })
    }

    /// Double-click or Enter on an entry.
    fn activate(&mut self, entry: &Entry) -> Option<Action> {
        let picks = match self.mode {
            Mode::Repository => entry.repository.is_some(),
            Mode::Folder => false,
            Mode::Program | Mode::File => entry.program,
        };
        if picks {
            return Some(Action::Choose(entry.path.clone()));
        }
        if !entry.program {
            self.go(entry.path.clone(), None);
        }
        None
    }

    fn start_editing(&mut self) {
        self.editing = Some(self.folder.display().to_string());
        self.focus_path = true;
    }

    fn apply_path(&mut self) {
        let Some(text) = self.editing.take() else {
            return;
        };
        match typed_folder(&text, self.scope.as_deref()) {
            Ok(path) => {
                self.go(path, None);
            }
            Err(problem) => {
                self.error = Some(problem);
                self.editing = Some(text);
            }
        }
    }

    /// Esc while typing a path cancels the edit instead of closing the dialog.
    pub fn take_escape(&mut self, ctx: &Context) {
        if self.editing.is_some() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.editing = None;
            self.error = None;
        }
    }

    /// The navigation row: back, up, the path bar and the filter.
    pub fn nav(&mut self, ui: &mut Ui, widgets: &mut HashMap<String, UiControl>, ids: Ids) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let back = crate::chrome::icon_button(ui, Icon::ArrowLeft, !self.history.is_empty(), false, 30.0)
                .on_hover_text("Back (Alt+Left)");
            pages::control(widgets, format!("{}-back", ids.prefix), &back, ui.clip_rect());
            if back.clicked() {
                self.back();
            }
            let up = crate::chrome::icon_button(ui, Icon::ArrowUp, self.can_go_up(), false, 30.0)
                .on_hover_text("Up one folder (Backspace)");
            pages::control(widgets, format!("{}-up", ids.prefix), &up, ui.clip_rect());
            if up.clicked() {
                self.up();
            }
            let filter_width = 200.0_f32.min(ui.available_width() * 0.3);
            let bar = (ui.available_width() - filter_width - 6.0).max(120.0);
            ui.allocate_ui_with_layout(vec2(bar, 32.0), Layout::left_to_right(Align::Center), |ui| {
                self.path_bar(ui, widgets, ids);
            });
            field(ui, filter_width, |ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                icon(ui.painter(), rect, Icon::Search, HINT());
                let edit = ui.add(
                    TextEdit::singleline(&mut self.filter)
                        .frame(Frame::NONE)
                        .hint_text(if self.mode == Mode::Program { "Filter" } else { "Filter folders" })
                        .char_limit(128)
                        .desired_width(ui.available_width()),
                );
                pages::control(widgets, format!("{}-filter", ids.prefix), &edit, ui.clip_rect());
            });
        });
        if let Some(error) = &self.error {
            ui.add_space(4.0);
            ui.label(RichText::new(error).size(12.0).color(RED()));
        }
    }

    fn path_bar(&mut self, ui: &mut Ui, widgets: &mut HashMap<String, UiControl>, ids: Ids) {
        if let Some(text) = &mut self.editing {
            let mut apply = false;
            field(ui, ui.available_width(), |ui| {
                let edit = ui.add(
                    TextEdit::singleline(text)
                        .frame(Frame::NONE)
                        .font(FontId::monospace(12.5))
                        .char_limit(32 * 1024)
                        .desired_width(ui.available_width() - 40.0),
                );
                if std::mem::take(&mut self.focus_path) {
                    edit.request_focus();
                }
                pages::control(widgets, ids.path, &edit, ui.clip_rect());
                apply = edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                let go = ui.add(quiet(Button::new(RichText::new("Go").size(12.5))));
                pages::control(widgets, format!("{}-go", ids.prefix), &go, ui.clip_rect());
                apply |= go.clicked();
            });
            if apply {
                self.apply_path();
            }
            return;
        }
        let crumbs = crumbs(&self.folder);
        let mut target = None;
        let mut edit = false;
        field(ui, ui.available_width(), |ui| {
            // Right to left: the current folder stays visible; long paths lose their start.
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 1.0;
                let pencil = crate::chrome::icon_button(ui, Icon::Pencil, true, false, 24.0)
                    .on_hover_text("Type or paste a path (Ctrl+L)");
                pages::control(widgets, format!("{}-edit-path", ids.prefix), &pencil, ui.clip_rect());
                edit = pencil.clicked();
                ui.add_space(4.0);
                for (i, (label, path)) in crumbs.iter().enumerate().rev() {
                    // Clipping paint alone does not constrain egui's measured
                    // width. Stop allocating ancestors when the field is full.
                    if ui.available_width() < 24.0 { break; }
                    let last = i + 1 == crumbs.len();
                    let r = crumb(ui, label, last);
                    pages::control(widgets, format!("{}-crumb-{i}", ids.prefix), &r, ui.clip_rect());
                    if r.clicked() && !last {
                        target = Some(path.clone());
                    }
                    if i > 0 && ui.available_width() >= 12.0 {
                        let (rect, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                        icon(ui.painter(), rect, Icon::ChevronRight, FAINT());
                    }
                }
                if ui.available_width() >= 16.0 {
                    let (rect, _) = ui.allocate_exact_size(vec2(16.0, 14.0), Sense::hover());
                    icon(ui.painter(), rect.shrink2(vec2(1.0, 0.0)), Icon::Drive, HINT());
                }
            });
        });
        if edit {
            self.start_editing();
        }
        if let Some(path) = target {
            self.go(path, None);
        }
    }

    /// Places on the left, the folder list on the right.
    pub fn body(
        &mut self,
        ui: &mut Ui,
        widgets: &mut HashMap<String, UiControl>,
        ids: Ids,
        recent: &[Recent],
    ) -> Option<Action> {
        let rect = ui.available_rect_before_wrap();
        ui.allocate_rect(rect, Sense::hover());
        let narrow = rect.width() < 560.0;
        let places_rect = Rect::from_min_size(rect.min, vec2(if narrow { 0.0 } else { 210.0 }, rect.height()));
        let list_rect = Rect::from_min_max(pos2(places_rect.right(), rect.top()), rect.max);
        let line = Stroke::new(1.0, HAIRLINE());
        ui.painter().hline(rect.x_range(), rect.top() + 0.5, line);
        if !narrow {
            ui.painter().rect_filled(places_rect, 0.0, SUNKEN());
            ui.painter().vline(places_rect.right() - 0.5, places_rect.y_range(), line);
            ui.scope_builder(UiBuilder::new().max_rect(places_rect.shrink2(vec2(8.0, 10.0))), |ui| {
                ScrollArea::vertical()
                    .id_salt((ids.prefix, "places"))
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.places_column(ui, widgets, ids, recent));
            });
        }
        let mut action = None;
        ui.scope_builder(UiBuilder::new().max_rect(list_rect), |ui| {
            action = self.list(ui, widgets, ids);
        });
        action
    }

    fn places_column(
        &mut self,
        ui: &mut Ui,
        widgets: &mut HashMap<String, UiControl>,
        ids: Ids,
        recent: &[Recent],
    ) {
        ui.spacing_mut().item_spacing.y = 2.0;
        let mut index = 0;
        let mut target = None;
        let mut place = |ui: &mut Ui, image: Icon, label: &str, detail: &str, path: &Path, select: Option<String>| {
            let current = (select.is_none() && path == self.folder)
                || (select.is_some()
                    && path.parent() == Some(self.folder.as_path())
                    && self.selected.as_deref() == select.as_deref());
            let r = place_row(ui, image, label, detail, current);
            pages::control(widgets, format!("{}-place-{index}", ids.prefix), &r, ui.clip_rect());
            index += 1;
            if r.clicked() {
                target = Some((path.to_path_buf(), select));
            }
        };
        let recent: Vec<&Recent> = recent.iter().filter(|r| self.within(&r.path)).take(5).collect();
        if !recent.is_empty() && self.mode == Mode::Repository {
            section(ui, "Recent");
            for r in recent {
                let name = r
                    .path
                    .file_name()
                    .map_or_else(|| r.path.display().to_string(), |n| n.to_string_lossy().into_owned());
                place(ui, Icon::Branch, &name, r.branch.as_deref().unwrap_or(""), &r.path, Some(name.clone()));
            }
            ui.add_space(8.0);
        }
        let (folders, drives) = self.places.clone();
        if !folders.is_empty() {
            section(ui, if self.scope.is_some() { "Folders" } else { "This PC" });
            for p in &folders {
                place(ui, p.icon, &p.label, "", &p.path, None);
            }
            ui.add_space(8.0);
        }
        if !drives.is_empty() {
            section(ui, "Drives");
            for p in &drives {
                place(ui, p.icon, &p.label, "", &p.path, None);
            }
        }
        if let Some((path, select)) = target {
            match select {
                // A recent repository: show it in its parent folder, selected.
                Some(name) => match path.parent() {
                    Some(parent) if parent != self.folder => self.go(parent.to_path_buf(), Some(name)),
                    _ => self.selected = Some(name),
                },
                None => self.go(path, None),
            }
        }
    }

    fn list(&mut self, ui: &mut Ui, widgets: &mut HashMap<String, UiControl>, ids: Ids) -> Option<Action> {
        let width = ui.available_width();
        let line = Stroke::new(1.0, HAIRLINE());
        let repository_column = self.mode == Mode::Repository && width > 440.0;
        let modified_column = width > 520.0;
        // Column header.
        let (header, _) = ui.allocate_exact_size(vec2(width, 30.0), Sense::hover());
        ui.painter().hline(header.x_range(), header.bottom() - 0.5, line);
        let caption = |x: f32, text: &str, align: Align2| {
            ui.painter().text(
                pos2(x, header.center().y),
                align,
                text.to_uppercase(),
                semibold(11.0),
                HINT(),
            );
        };
        caption(header.left() + 50.0, "Name", Align2::LEFT_CENTER);
        let modified_right = header.right() - 34.0;
        let repository_left = modified_right - if modified_column { 120.0 } else { 0.0 } - 200.0;
        if repository_column {
            caption(repository_left, "Repository", Align2::LEFT_CENTER);
        }
        if modified_column {
            caption(modified_right, "Modified", Align2::RIGHT_CENTER);
        }
        // This folder is itself a repository.
        if self.mode == Mode::Repository
            && let Some(branch) = self.listing.as_ref().and_then(|l| l.repository.clone())
        {
            ui.add_space(8.0);
            Frame::new()
                .fill(GREEN().gamma_multiply(0.08))
                .stroke(Stroke::new(1.0, GREEN().gamma_multiply(0.3)))
                .corner_radius(8)
                .inner_margin(Margin::symmetric(12, 8))
                .outer_margin(Margin::symmetric(14, 0))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                        icon(ui.painter(), rect, Icon::Check, GREEN());
                        let label = if branch.is_empty() {
                            "This folder is a Git repository. Open it, or pick a folder inside.".to_owned()
                        } else {
                            format!("This folder is a Git repository on {branch}. Open it, or pick a folder inside.")
                        };
                        ui.add(Label::new(RichText::new(label).size(12.5).color(TEXT())).truncate());
                    });
                });
            ui.add_space(4.0);
        }
        let rows: Vec<Entry> = self.visible().into_iter().cloned().collect();
        let mut action = None;
        // Keyboard: arrows move the selection, Enter activates, Backspace goes up.
        if !ui.ctx().text_edit_focused() && self.editing.is_none() {
            let position = self
                .selected
                .as_ref()
                .and_then(|s| rows.iter().position(|e| &e.name == s));
            let (down, up, enter, parent, back, path) = ui.input_mut(|i| {
                (
                    i.consume_key(Modifiers::NONE, Key::ArrowDown),
                    i.consume_key(Modifiers::NONE, Key::ArrowUp),
                    i.consume_key(Modifiers::NONE, Key::Enter),
                    i.consume_key(Modifiers::NONE, Key::Backspace),
                    i.consume_key(Modifiers::ALT, Key::ArrowLeft),
                    i.consume_key(Modifiers::COMMAND, Key::L),
                )
            });
            let step = |index: usize| {
                rows.get(index).map(|e| (e.name.clone(), index))
            };
            let moved = if down {
                step(position.map_or(0, |p| (p + 1).min(rows.len().saturating_sub(1))))
            } else if up {
                step(position.map_or(0, |p| p.saturating_sub(1)))
            } else {
                None
            };
            if let Some((name, index)) = moved {
                self.selected = Some(name);
                self.scroll_to = Some(index);
            }
            if enter && let Some(entry) = position.and_then(|p| rows.get(p)) {
                action = self.activate(entry);
            }
            if parent {
                self.up();
            } else if back {
                self.back();
            } else if path {
                self.start_editing();
            }
        }
        let scroll_to = self.scroll_to.take();
        let mut area = ScrollArea::vertical()
            .id_salt((ids.prefix, "list", self.folder.as_os_str()))
            .auto_shrink([false, false]);
        if let Some(index) = scroll_to {
            let view = (ui.available_height() - ROW).max(0.0);
            area = area.vertical_scroll_offset((index as f32 * ROW - view / 2.0).max(0.0));
        }
        let mut clicked = None;
        let mut entered = None;
        let output = area.show_rows(ui, ROW, rows.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for index in range {
                let entry = &rows[index];
                let selected = self.selected.as_deref() == Some(entry.name.as_str());
                let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
                let chevron = Rect::from_min_max(pos2(rect.right() - 30.0, rect.top()), rect.max);
                let into = (!entry.program).then(|| {
                    ui.interact(chevron, ui.id().with(("enter", index)), Sense::click())
                        .on_hover_text(format!("Go into {}", entry.name))
                });
                paint_row(ui, rect, entry, selected, response.hovered() || response.has_focus(), repository_column, modified_column);
                if let Some(into) = &into {
                    if into.hovered() {
                        ui.painter().rect_filled(chevron.shrink2(vec2(2.0, 6.0)), 5.0, RAISED());
                    }
                    icon(
                        ui.painter(),
                        Rect::from_center_size(chevron.center(), vec2(12.0, 12.0)),
                        Icon::ChevronRight,
                        if into.hovered() { TEXT() } else { FAINT() },
                    );
                    pages::control(widgets, format!("{}-row-{index}-enter", ids.prefix), into, ui.clip_rect());
                }
                response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, selected, &entry.name));
                pages::control(widgets, format!("{}-row-{index}", ids.prefix), &response, ui.clip_rect());
                if into.as_ref().is_some_and(Response::clicked) {
                    entered = Some(entry.path.clone());
                } else if response.double_clicked() {
                    clicked = Some((index, true));
                } else if response.clicked() {
                    clicked = Some((index, false));
                }
            }
        });
        pages::region(widgets, &format!("{}-list-scroll", ids.prefix), output.inner_rect.intersect(ui.clip_rect()), true);
        if let Some(path) = entered {
            self.go(path, None);
        } else if let Some((index, double)) = clicked {
            let entry = rows[index].clone();
            self.selected = Some(entry.name.clone());
            if double {
                action = self.activate(&entry);
            }
        }
        // Loading, errors and empty folders.
        let notice = if self.read.is_some() && self.listing.is_none() {
            Some("Reading folder…".to_owned())
        } else if self.listing.is_none() {
            None
        } else if rows.is_empty() && !self.filter.trim().is_empty() {
            Some(format!("No folders match “{}”.", self.filter.trim()))
        } else if rows.is_empty() {
            Some(match self.mode {
                Mode::Program => "No subfolders or programs here.".to_owned(),
                Mode::File => "No files or subfolders here.".to_owned(),
                _ => "This folder has no subfolders.".to_owned(),
            })
        } else if self.listing.as_ref().is_some_and(|l| l.truncated) {
            Some(format!("Showing the first {LIMIT} items. Filter or type a path to narrow it."))
        } else {
            None
        };
        if let Some(text) = notice {
            let area = output.inner_rect;
            let y = if rows.is_empty() { area.top() + 40.0 } else { area.bottom() - 14.0 };
            ui.painter().text(pos2(area.center().x, y), Align2::CENTER_CENTER, text, FontId::proportional(12.5), HINT());
        }
        action
    }
}

/// Widget ids for one browser, so the Open dialog keeps its automation names.
#[derive(Clone, Copy)]
pub struct Ids {
    pub prefix: &'static str,
    pub path: &'static str,
}

fn section(ui: &mut Ui, text: &str) {
    ui.add_space(2.0);
    ui.label(RichText::new(text.to_uppercase()).font(semibold(11.0)).color(HINT()));
    ui.add_space(2.0);
}

/// A sunken input row, like the path bar and the filter.
fn field<R>(ui: &mut Ui, width: f32, content: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    Frame::new()
        .fill(BG())
        .stroke(Stroke::new(1.0, BORDER()))
        .corner_radius(6)
        .inner_margin(Margin {
            left: 9,
            right: 3,
            top: 0,
            bottom: 0,
        })
        .show(ui, |ui| {
            ui.set_width((width - 14.0).max(40.0));
            ui.set_height(30.0);
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                content(ui)
            })
            .inner
        })
}

fn crumb(ui: &mut Ui, label: &str, last: bool) -> Response {
    let font = if last { semibold(13.0) } else { FontId::proportional(13.0) };
    let width = ui.painter().layout_no_wrap(label.into(), font.clone(), TEXT()).size().x;
    let (rect, response) = ui.allocate_exact_size(vec2((width + 12.0).min(ui.available_width().max(0.0)), 24.0), Sense::click());
    let hot = !last && (response.hovered() || response.has_focus());
    if hot {
        ui.painter().rect_filled(rect, 4.0, HOVER());
    }
    truncated(ui, pos2(rect.left() + 6.0, rect.center().y), rect.width() - 12.0, label, font, if last || hot { TEXT() } else { MUTED() });
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, label));
    response.on_hover_text(label)
}

fn place_row(ui: &mut Ui, image: Icon, label: &str, detail: &str, current: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), PLACE_ROW), Sense::click());
    if current {
        ui.painter().rect_filled(rect, 6.0, ACCENT_SOFT());
    } else if response.hovered() || response.has_focus() {
        ui.painter().rect_filled(rect, 6.0, HOVER());
    }
    let tint = if matches!(image, Icon::Branch) { BLUE() } else { MUTED() };
    icon(ui.painter(), Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), vec2(14.0, 14.0)), image, tint);
    let detail_width = if detail.is_empty() {
        0.0
    } else {
        ui.painter().layout_no_wrap(detail.into(), FontId::proportional(11.5), HINT()).size().x.min(70.0)
    };
    truncated(ui, pos2(rect.left() + 32.0, rect.center().y), rect.width() - 44.0 - detail_width, label, FontId::proportional(13.0), TEXT());
    if !detail.is_empty() {
        truncated(ui, pos2(rect.right() - 8.0 - detail_width, rect.center().y), detail_width, detail, FontId::proportional(11.5), HINT());
    }
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, current, label));
    response
}

#[allow(clippy::too_many_arguments)]
fn paint_row(
    ui: &Ui,
    rect: Rect,
    entry: &Entry,
    selected: bool,
    hovered: bool,
    repository_column: bool,
    modified_column: bool,
) {
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 0.0, SELECTED());
    } else if hovered {
        painter.rect_filled(rect, 0.0, HOVER());
    }
    painter.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, ROW_LINE()));
    let tile = Rect::from_center_size(pos2(rect.left() + 27.0, rect.center().y), vec2(22.0, 22.0));
    let (image, tint) = if entry.program {
        (Icon::Console, MUTED())
    } else if entry.repository.is_some() {
        painter.rect_filled(tile, 5.0, ACCENT_SOFT());
        (Icon::Branch, BLUE())
    } else {
        (Icon::Folder, HINT())
    };
    icon(painter, tile.shrink(4.0), image, tint);
    let modified_right = rect.right() - 34.0;
    let repository_left = modified_right - if modified_column { 120.0 } else { 0.0 } - 200.0;
    let name_right = if repository_column {
        repository_left - 12.0
    } else if modified_column {
        modified_right - 132.0
    } else {
        rect.right() - 34.0
    };
    let font = if entry.repository.is_some() { semibold(13.0) } else { FontId::proportional(13.0) };
    truncated(ui, pos2(rect.left() + 50.0, rect.center().y), name_right - rect.left() - 50.0, &entry.name, font, TEXT());
    if repository_column {
        match &entry.repository {
            Some(branch) if !branch.is_empty() => {
                let galley = painter.layout_no_wrap(branch.clone(), FontId::proportional(11.5), BLUE());
                let width = (galley.size().x + 16.0).min(180.0);
                let pill = Rect::from_min_size(pos2(repository_left, rect.center().y - 10.0), vec2(width, 20.0));
                painter.rect_filled(pill, 10.0, ACCENT_SOFT());
                truncated(ui, pos2(pill.left() + 8.0, pill.center().y), width - 16.0, branch, FontId::proportional(11.5), BLUE());
            }
            Some(_) => {
                painter.text(pos2(repository_left, rect.center().y), Align2::LEFT_CENTER, "Repository", FontId::proportional(12.0), BLUE());
            }
            None => {
                painter.text(pos2(repository_left, rect.center().y), Align2::LEFT_CENTER, "—", FontId::proportional(12.0), FAINT());
            }
        }
    }
    if modified_column && let Some(modified) = entry.modified {
        let now = SystemTime::now();
        let age = now.duration_since(modified).unwrap_or_default().as_secs();
        let unix = modified.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        painter.text(
            pos2(modified_right, rect.center().y),
            Align2::RIGHT_CENTER,
            ago(age, unix),
            FontId::proportional(12.0),
            HINT(),
        );
    }
}

/// One line of text starting at `left_center`, cut with "…" to `width`.
fn truncated(ui: &Ui, left_center: Pos2, width: f32, text: &str, font: FontId, color: Color32) {
    let mut job = text::LayoutJob::simple_singleline(text.into(), font, color);
    job.wrap = text::TextWrapping::truncate_at_width(width.max(0.0));
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(pos2(left_center.x, left_center.y - galley.size().y / 2.0), galley, color);
}

/// Where File → Open starts: a typed folder, else beside the most recent repository
/// (selected), else the user's folder.
fn open_start(typed: &str, recent: &[PathBuf]) -> (PathBuf, Option<String>) {
    let typed = Path::new(typed.trim());
    if typed.is_absolute() {
        return (typed.to_path_buf(), None);
    }
    if let Some(last) = recent.first()
        && let (Some(parent), Some(name)) = (last.parent(), last.file_name())
    {
        return (parent.to_path_buf(), Some(name.to_string_lossy().into_owned()));
    }
    (home().unwrap_or_else(|| PathBuf::from("C:\\")), None)
}

/// What a Browse button fills in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickTarget {
    GitProgram,
    /// Preferences → Security: the checksum-verified optional OSV scanner.
    DependencyScanner,
    CommandInput(usize),
    /// Custom commands: the edited command's program.
    CommandProgram(usize),
    /// Custom commands: a definitions file to import.
    CommandImport,
    /// Custom commands: the folder for a new definitions file.
    CommandExport,
    /// Submodules: a local repository to add, or to fetch from.
    SubmoduleSource,
    /// The AI draft program (claude.exe or codex.exe).
    DraftProgram,
    /// One CLI's program, from a page that names the CLI itself.
    AiProgram(rocket_git::DraftProvider),
    /// An external tool profile's program.
    ToolProgram(ExternalToolKind, usize),
    /// Preferences → Terminal: a custom shell.
    ShellProgram,
    /// The MCP test repositories folder.
    McpRoot,
    /// Preferences → General: where Clone and New repository start.
    SourceFolder,
    /// Clone: a local repository to clone.
    CloneSource,
    /// Repository manager: the repository a saved entry points to.
    LibraryRepository,
    /// Clone: the folder the new folder goes into.
    CloneParent,
    /// New repository: the project folder.
    InitFolder,
    /// New worktree: its new folder.
    WorktreeFolder,
    /// Import commit series: the folder of patches.
    SeriesFolder,
    /// Export commit series: where its new folder goes.
    SeriesSave,
    /// Preferences → Appearance: a picture for the Custom theme.
    ThemePicture,
    /// Preferences → Extensions: the integration.json of a package to install or update.
    ExtensionFile,
}

pub struct Picker {
    pub target: PickTarget,
    /// Frames left of bringing it above the dialog it opened from.
    raise: u8,
    pub browser: Browser,
}

impl Rocket {
    /// Recent repositories with their branch when the welcome screen has read it.
    fn recent_places(&self) -> Vec<Recent> {
        self.recent
            .iter()
            .map(|path| Recent {
                path: path.clone(),
                branch: self
                    .recent_heads
                    .get(path)
                    .flatten()
                    .and_then(|head| match head {
                        rocket_git::Head::Branch { name, .. } | rocket_git::Head::Unborn { branch: name } => {
                            Some(String::from_utf8_lossy(name).into_owned())
                        }
                        rocket_git::Head::Detached { .. } => None,
                    }),
            })
            .collect()
    }

    /// File → Open: browse to a repository and open it.
    pub(crate) fn open_dialog(&mut self, ctx: &Context) {
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        if dialog.folder.is_none() {
            let (start, select) = match &self.browse_root {
                Some(root) => (root.clone(), None),
                None => open_start(&dialog.first, &self.recent),
            };
            dialog.folder = Some(Browser::new(Mode::Repository, self.browse_root.clone(), start, select));
        }
        let recent = self.recent_places();
        let browser = dialog.folder.as_mut().expect("browser created above");
        browser.poll(ctx);
        browser.take_escape(ctx);
        // The MCP host checks this path before an automated Open.
        let target = browser.repository();
        dialog.first = target
            .as_ref()
            .map_or_else(String::new, |(path, _)| path.display().to_string());
        let widgets = &mut self.frame_widgets;
        let ids = Ids {
            prefix: "open",
            path: "dialog-path",
        };
        let mut action = None;
        let mut close = false;
        let response = browse_modal(ctx, "rocket-open", Icon::Folder, "Open repository", "Choose a folder that contains a Git repository.", |ui, part| match part {
            Part::Nav => browser.nav(ui, widgets, ids),
            Part::Body => action = browser.body(ui, widgets, ids, &recent),
            Part::Status => repository_status(ui, browser, widgets, &mut action),
            Part::Buttons => {
                let open = pages::primary(ui, target.is_some(), "Open");
                pages::control(widgets, "dialog-submit", &open, ui.clip_rect());
                if open.clicked() {
                    action = target.clone().map(|(path, _)| Action::Choose(path));
                }
            }
            Part::Close(x) => {
                // Automation's old id for the footer's Cancel.
                pages::control(widgets, "dialog-cancel", &x, ui.clip_rect());
                close |= x.clicked();
            }
        });
        close |= response.should_close();
        // Ctrl+Enter takes the primary action, as in the other dialogs.
        if action.is_none()
            && let Some((path, _)) = target
            && ctx.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::Enter))
        {
            action = Some(Action::Choose(path));
        }
        match action {
            Some(Action::Choose(path)) => {
                self.open(path);
            }
            Some(Action::CreateHere(path)) => {
                self.show_dialog(DialogKind::NewRepository);
                if let Some(d) = self.dialog.as_mut() {
                    d.first = path.display().to_string();
                }
            }
            None if !close => self.dialog = Some(dialog),
            None => {}
        }
    }

    /// Start a Browse picker for a Preferences or dialog field.
    pub(crate) fn browse(&mut self, target: PickTarget, current: &str) {
        let mode = match target {
            PickTarget::CommandInput(_) | PickTarget::CommandImport | PickTarget::ThemePicture | PickTarget::ExtensionFile => Mode::File,
            PickTarget::McpRoot
            | PickTarget::SourceFolder
            | PickTarget::CloneParent
            | PickTarget::InitFolder
            | PickTarget::WorktreeFolder
            | PickTarget::SeriesFolder
            | PickTarget::SeriesSave
            | PickTarget::CommandExport => Mode::Folder,
            PickTarget::CloneSource | PickTarget::SubmoduleSource | PickTarget::LibraryRepository => Mode::Repository,
            PickTarget::GitProgram
            | PickTarget::DependencyScanner
            | PickTarget::DraftProgram
            | PickTarget::AiProgram(_)
            | PickTarget::ToolProgram(..)
            | PickTarget::ShellProgram
            | PickTarget::CommandProgram(_) => Mode::Program,
        };
        let current = Path::new(current.trim());
        let (start, select) = if current.is_absolute() {
            match target {
                // A program, a repository or a project folder may not exist yet:
                // start beside it, with it selected when it does.
                PickTarget::GitProgram
                | PickTarget::DependencyScanner
                | PickTarget::DraftProgram
                | PickTarget::AiProgram(_)
                | PickTarget::CommandInput(_)
                | PickTarget::CommandProgram(_)
                | PickTarget::CommandImport
                | PickTarget::ToolProgram(..)
                | PickTarget::ShellProgram
                | PickTarget::CloneSource
                | PickTarget::SubmoduleSource
                | PickTarget::LibraryRepository
                | PickTarget::InitFolder
                | PickTarget::WorktreeFolder
                | PickTarget::SeriesFolder
                | PickTarget::ThemePicture
                | PickTarget::ExtensionFile => (
                    current.parent().map_or_else(|| current.to_path_buf(), Path::to_path_buf),
                    current.file_name().map(|n| n.to_string_lossy().into_owned()),
                ),
                // An export path names a file that does not exist yet: start in its folder.
                PickTarget::CommandExport | PickTarget::SeriesSave => (
                    current.parent().map_or_else(|| current.to_path_buf(), Path::to_path_buf),
                    None,
                ),
                PickTarget::McpRoot | PickTarget::SourceFolder | PickTarget::CloneParent => (current.to_path_buf(), None),
            }
        } else if mode == Mode::Program {
            (
                std::env::var_os("ProgramFiles")
                    .map(PathBuf::from)
                    .or_else(home)
                    .unwrap_or_else(|| PathBuf::from("C:\\")),
                None,
            )
        } else if target == PickTarget::ThemePicture {
            let pictures = home().map(|h| h.join("Pictures")).filter(|p| p.is_dir());
            (pictures.or_else(home).unwrap_or_else(|| PathBuf::from("C:\\")), None)
        } else {
            (home().unwrap_or_else(|| PathBuf::from("C:\\")), None)
        };
        self.picker = Some(Picker {
            raise: 2,
            target,
            browser: Browser::new(mode, self.browse_root.clone(), start, select),
        });
    }

    /// The Browse picker, drawn above Preferences.
    pub(crate) fn picker_dialog(&mut self, ctx: &Context) {
        let Some(mut picker) = self.picker.take() else {
            return;
        };
        if picker.raise > 0 {
            picker.raise -= 1;
            pages::raise(ctx, "rocket-picker");
        }
        let browser = &mut picker.browser;
        browser.poll(ctx);
        browser.take_escape(ctx);
        let choice = browser.choice();
        let picture = picker.target == PickTarget::ThemePicture;
        let (image, title, subtitle) = if picture {
            (Icon::Image, "Choose a picture", "A PNG, JPG or WebP picture for the Custom theme.")
        } else if picker.target == PickTarget::ExtensionFile {
            (Icon::Puzzle, "Choose a package", "Pick its integration.json. main.lua must be in the same folder.")
        } else {
            match browser.mode {
            Mode::Program => (Icon::Console, "Choose a program", "Pick the tool's .exe file."),
            Mode::File => (Icon::File, "Choose a file", "Pick a file for the command. It will appear as one literal argument."),
            Mode::Repository => (Icon::Folder, "Choose a repository", "Pick a local Git repository."),
            Mode::Folder => (Icon::Folder, "Choose a folder", "Pick a folder, or open one and choose it."),
            }
        };
        let widgets = &mut self.frame_widgets;
        let ids = Ids {
            prefix: "picker",
            path: "picker-path",
        };
        let mut action = None;
        let mut close = false;
        let response = browse_modal(ctx, "rocket-picker", image, title, subtitle, |ui, part| match part {
            Part::Nav => browser.nav(ui, widgets, ids),
            Part::Body => action = browser.body(ui, widgets, ids, &[]),
            Part::Status => {
                let text = choice.as_ref().map_or_else(
                    || match browser.mode {
                        Mode::Program => "Select a program (.exe).".to_owned(),
                        _ => String::new(),
                    },
                    |path| crate::views::display_path(path),
                );
                ui.add(Label::new(RichText::new(text).size(12.0).monospace().color(HINT())).truncate());
            }
            Part::Buttons => {
                let label = match browser.mode {
                    Mode::Program => "Choose",
                    Mode::File if picture => "Choose picture",
                    Mode::File => "Choose file",
                    Mode::Repository => "Choose repository",
                    Mode::Folder => "Choose folder",
                };
                let choose = pages::primary(ui, choice.is_some(), label);
                pages::control(widgets, "picker-choose", &choose, ui.clip_rect());
                if choose.clicked() {
                    action = choice.clone().map(Action::Choose);
                }
            }
            Part::Close(x) => {
                // Automation's old id for the footer's Cancel. Not `dialog-cancel`:
                // that stays with the dialog under the picker.
                pages::control(widgets, "picker-cancel", &x, ui.clip_rect());
                close |= x.clicked();
            }
        });
        close |= response.should_close();
        match action {
            Some(Action::Choose(path)) => self.apply_pick(picker.target, &path),
            _ if !close => self.picker = Some(picker),
            _ => {}
        }
    }

    fn apply_pick(&mut self, target: PickTarget, path: &Path) {
        let text = path.display().to_string();
        match target {
            PickTarget::CommandInput(index) => {
                if let Some(dialog) = &mut self.dialog
                    && let DialogKind::RunCommand(command) = dialog.kind
                    && let Some(input) = self.commands.get(command).and_then(|c| c.inputs.get(index))
                    && input.kind == rocket_git::CommandInputKind::File
                { dialog.command_values.insert(input.key.clone(), text); }
            }
            PickTarget::CommandProgram(index) => {
                if let Some(command) = self
                    .dialog
                    .as_mut()
                    .and_then(|d| d.commands.as_mut())
                    .and_then(|e| e.list.get_mut(index))
                {
                    command.program = text;
                }
            }
            PickTarget::CommandImport | PickTarget::CommandExport => {
                if let Some(editor) = self.dialog.as_mut().and_then(|d| d.commands.as_mut()) {
                    editor.picked(target == PickTarget::CommandExport, path);
                }
            }
            PickTarget::SeriesFolder => {
                if let Some(d) = self.dialog.as_mut()
                    && matches!(d.kind, DialogKind::MailImport(_))
                {
                    d.first = text;
                    self.start_mail_preview();
                }
            }
            PickTarget::SeriesSave => {
                let name = self
                    .tab()
                    .and_then(|t| t.path.file_name())
                    .map_or_else(|| "commit".to_owned(), |n| n.to_string_lossy().into_owned());
                if let Some(s) = self.dialog.as_mut().and_then(|d| d.patch.as_mut()) {
                    s.path = path.join(format!("{name}-series")).display().to_string();
                    s.preview = None;
                    s.notice = None;
                    s.saved = None;
                }
            }
            PickTarget::SubmoduleSource => {
                if let Some(d) = self.dialog.as_mut()
                    && let Some(s) = d.submodules.as_mut()
                {
                    s.source = text;
                    s.action_task = None;
                    s.action_preview = None;
                    d.error = None;
                }
            }
            PickTarget::GitProgram => self.git_settings.picked(text),
            PickTarget::DependencyScanner => self.choose_dependency_scanner(path.to_path_buf()),
            PickTarget::ThemePicture => self.import_theme_picture(path),
            PickTarget::ExtensionFile => {
                if let Some(backend) = self.backend.clone() {
                    self.integrations.pick_file(backend, path.to_path_buf());
                }
            }
            PickTarget::DraftProgram => *self.drafting.program_setting_mut() = text,
            PickTarget::AiProgram(provider) => *self.drafting.program_setting_of_mut(provider) = text,
            PickTarget::McpRoot => self.mcp.root = text,
            PickTarget::SourceFolder => self.source_folder_picked(path),
            PickTarget::LibraryRepository => self.library_repository_picked(path),
            PickTarget::ToolProgram(kind, index) => {
                if let Some(profile) = self.external.editing.0[kind.index()].entries.get_mut(index) {
                    profile.config.executable = text;
                }
            }
            PickTarget::ShellProgram => self.terminal_shell.program_picked(text),
            PickTarget::CloneSource | PickTarget::InitFolder => {
                if let Some(dialog) = self.dialog.as_mut() {
                    dialog.first = text;
                }
            }
            PickTarget::WorktreeFolder => {
                if let Some(dialog) = self.dialog.as_mut() {
                    dialog.first = text;
                    // A reviewed folder no longer matches the new one.
                    if let Some(s) = dialog.worktrees.as_mut() {
                        s.task = None;
                        s.preview = None;
                        s.suggested = None;
                    }
                    dialog.error = None;
                }
            }
            PickTarget::CloneParent => {
                if let Some(form) = self.dialog.as_mut().and_then(|d| d.create.as_mut()) {
                    form.parent = text;
                }
            }
        }
    }
}

/// The Open dialog's footer status: what Open would do.
fn repository_status(
    ui: &mut Ui,
    browser: &Browser,
    widgets: &mut HashMap<String, UiControl>,
    action: &mut Option<Action>,
) {
    let badge = |ui: &mut Ui, image: Icon, color: Color32| {
        let (rect, _) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
        ui.painter().circle_filled(rect.center(), 13.0, color.gamma_multiply(0.14));
        icon(ui.painter(), rect.shrink(7.0), image, color);
    };
    let selected = browser.selected_entry().cloned();
    if let Some((path, branch)) = browser.repository() {
        badge(ui, Icon::Check, GREEN());
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            let name = path
                .file_name()
                .map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label(RichText::new(name).font(semibold(13.0)));
                if branch.is_empty() {
                    ui.label(RichText::new("· Git repository").color(MUTED()));
                } else {
                    ui.label(RichText::new("· Git repository on").color(MUTED()));
                    ui.add(Label::new(RichText::new(branch).color(BLUE())).truncate());
                }
            });
            ui.add(Label::new(RichText::new(crate::views::display_path(&path)).size(11.5).monospace().color(HINT())).truncate());
        });
    } else if let Some(entry) = selected.filter(|e| !e.program) {
        badge(ui, Icon::Info, AMBER());
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.add(Label::new(RichText::new(&entry.name).font(semibold(13.0))).truncate());
                ui.label(RichText::new("is not a Git repository").color(MUTED()));
            });
            let create = ui.add(
                Button::new(RichText::new("Create a repository here").size(12.0).color(BLUE())).frame(false),
            );
            pages::control(widgets, "open-create-here", &create, ui.clip_rect());
            if create.clicked() {
                *action = Some(Action::CreateHere(entry.path.clone()));
            }
        });
    } else {
        ui.label(
            RichText::new("Select a repository, or double-click a folder to look inside.")
                .size(12.5)
                .color(HINT()),
        );
    }
}

enum Part {
    Nav,
    Body,
    Status,
    Buttons,
    /// The header's close button.
    Close(Response),
}

/// The shared frame of the Open dialog and the Browse picker: header, navigation,
/// the browser and a footer with the status left and the buttons right.
fn browse_modal(
    ctx: &Context,
    id: &str,
    image: Icon,
    title: &str,
    subtitle: &str,
    mut content: impl FnMut(&mut Ui, Part),
) -> ModalResponse<()> {
    let width = (ctx.content_rect().width() - 48.0).clamp(360.0, 880.0);
    let height = (ctx.content_rect().height() - 48.0).clamp(380.0, 600.0);
    Modal::new(Id::new(id))
        .backdrop_color(Color32::from_black_alpha(120))
        .frame(pages::modal_frame())
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.set_height(height);
            ui.spacing_mut().item_spacing.y = 0.0;
            // Header: badge, title and subtitle, the guide and close.
            Frame::new()
                .inner_margin(Margin {
                    left: 20,
                    right: 16,
                    top: 18,
                    bottom: 12,
                })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_top(|ui| {
                        let (badge, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
                        ui.painter().rect_filled(badge, 8.0, ACCENT_SOFT());
                        icon(ui.painter(), badge.shrink(9.0), image, BLUE());
                        ui.add_space(6.0);
                        ui.vertical(|ui| {
                            ui.set_max_width(ui.available_width() - 70.0);
                            ui.spacing_mut().item_spacing.y = 3.0;
                            ui.label(RichText::new(title).font(semibold(16.0)));
                            ui.add(Label::new(RichText::new(subtitle).color(MUTED())).truncate());
                        });
                        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                            let close = crate::chrome::icon_button(ui, Icon::Close, true, false, 28.0)
                                .on_hover_text("Close (Esc)");
                            content(ui, Part::Close(close));
                            if crate::guide::available(ui.ctx()) {
                                crate::guide::header_button(ui);
                            }
                        });
                    });
                });
            Frame::new()
                .inner_margin(Margin {
                    left: 20,
                    right: 20,
                    top: 0,
                    bottom: 12,
                })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    content(ui, Part::Nav);
                });
            let footer_height = 58.0;
            let body = Rect::from_min_max(
                ui.cursor().min,
                pos2(ui.max_rect().right(), ui.max_rect().bottom() - footer_height),
            );
            ui.scope_builder(UiBuilder::new().max_rect(body), |ui| content(ui, Part::Body));
            let footer = Rect::from_min_max(pos2(ui.max_rect().left(), body.bottom()), ui.max_rect().max);
            ui.painter().rect_filled(
                footer,
                CornerRadius { nw: 0, ne: 0, sw: 11, se: 11 },
                FOOTER(),
            );
            ui.painter().hline(footer.x_range(), footer.top(), Stroke::new(1.0, HAIRLINE()));
            ui.scope_builder(
                UiBuilder::new()
                    .max_rect(footer.shrink2(vec2(20.0, 12.0)))
                    .layout(Layout::right_to_left(Align::Center)),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    content(ui, Part::Buttons);
                    ui.add_space(8.0);
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        content(ui, Part::Status);
                    });
                },
            );
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_common::Fixture;

    #[test]
    fn long_breadcrumbs_stay_inside_the_file_picker_width() {
        let fixture = scoped_fixture();
        let folder = fixture.root.join("long-parent-folder-".repeat(6)).join("Unicode 文 folder-".repeat(6));
        std::fs::create_dir_all(&folder).unwrap();
        let mut browser = Browser::new(Mode::File, Some(fixture.root.clone()), folder, None);
        let ctx = Context::default();
        crate::style::install(&ctx);
        let mut widgets = HashMap::new();
        for width in [840.0, 500.0] {
            let mut output = ctx.run_ui(RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 860.0))),
                ..Default::default()
            }, |ui| {
                let row = ui.allocate_ui_with_layout(vec2(width, 50.0), Layout::top_down(Align::Min), |ui| {
                    browser.nav(ui, &mut widgets, Ids { prefix: "picker", path: "picker-path" });
                });
                assert!(row.response.rect.width() <= width + 1.0, "{} exceeds {width}", row.response.rect.width());
            });
            output.textures_delta.clear();
        }
    }

    fn scoped_fixture() -> Fixture {
        let mut fixture = Fixture::new();
        fixture.root = PathBuf::from(crate::views::display_path(&fixture.root.canonicalize().unwrap()));
        fixture.repository = PathBuf::from(crate::views::display_path(&fixture.repository.canonicalize().unwrap()));
        fixture
    }

    fn frame(ctx: &Context, app: &mut Rocket, events: Vec<Event>) {
        app.frame_widgets.clear();
        let mut output = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 860.0))),
                events,
                ..Default::default()
            },
            |ui| {
                let ctx = ui.ctx().clone();
                app.dialog_ui(&ctx);
            },
        );
        output.textures_delta.clear();
    }

    fn click(ctx: &Context, app: &mut Rocket, id: &str) {
        let at = app.frame_widgets[id].rect.center();
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        frame(ctx, app, vec![Event::PointerMoved(at), button(true)]);
        frame(ctx, app, vec![button(false)]);
    }

    /// In the MCP test host the browser lists only the fixture folder; picking its
    /// repository enables Open, which opens it.
    #[test]
    fn open_shows_the_test_folder_and_opens_the_picked_repository() {
        crate::test_common::isolated(
            "ROCKET_FOLDER_UI_CASE",
            "folder_browser::tests::open_shows_the_test_folder_and_opens_the_picked_repository",
            || {
                let fixture = scoped_fixture();
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(&ctx, vec![], fixture.root.join("logs"));
                app.browse_root = Some(fixture.root.clone());
                app.show_dialog(DialogKind::Open);
                let deadline = std::time::Instant::now() + Duration::from_secs(10);
                while !app
                    .dialog
                    .as_ref()
                    .and_then(|d| d.folder.as_ref())
                    .is_some_and(|b| b.listing.is_some() && b.places_read.is_none())
                {
                    assert!(std::time::Instant::now() < deadline, "the folder read finished");
                    frame(&ctx, &mut app, vec![]);
                    std::thread::sleep(Duration::from_millis(5));
                }
                frame(&ctx, &mut app, vec![]);
                assert!(!app.frame_widgets["dialog-submit"].enabled, "nothing to open yet");
                assert!(app.frame_widgets.contains_key("open-place-0"), "the test folder is a place");
                assert!(!app.frame_widgets.contains_key("open-place-1"), "no other places or drives");
                let browser = app.dialog.as_ref().unwrap().folder.as_ref().unwrap();
                let index = browser
                    .visible()
                    .iter()
                    .position(|e| e.path == fixture.repository)
                    .expect("the fixture repository is listed");
                click(&ctx, &mut app, &format!("open-row-{index}"));
                frame(&ctx, &mut app, vec![]);
                assert!(app.frame_widgets["dialog-submit"].enabled);
                assert_eq!(
                    app.dialog.as_ref().unwrap().first,
                    fixture.repository.display().to_string(),
                    "the MCP host checks this path before Open"
                );
                click(&ctx, &mut app, "dialog-submit");
                assert!(app.dialog.is_none(), "Open closes the dialog");
            },
        );
    }

    #[test]
    fn head_names_branches_and_detached_commits() {
        assert_eq!(branch_label("ref: refs/heads/main\n"), "main");
        assert_eq!(branch_label("ref: refs/heads/feature/folder-browser"), "feature/folder-browser");
        assert_eq!(branch_label(&format!("{}\n", "a".repeat(40))), "detached at aaaaaaa");
        assert_eq!(branch_label("ref: refs/remotes/origin/main"), "refs/remotes/origin/main");
        assert_eq!(branch_label("not a head"), "");
    }

    #[test]
    #[cfg(windows)]
    fn scoped_paths_reject_junctions_before_listing() {
        use std::os::windows::process::CommandExt;
        let fixture = scoped_fixture(); let outside = scoped_fixture();
        let junction = fixture.root.join("outside-junction");
        let status = std::process::Command::new("cmd.exe").args(["/d", "/c", "mklink", "/J"])
            .arg(&junction).arg(&outside.root).creation_flags(0x0800_0000).output().unwrap();
        assert!(status.status.success(), "{status:?}");
        let accepted = inside(&junction, &fixture.root);
        let typed = typed_folder(junction.to_str().unwrap(), Some(&fixture.root));
        let read = read(&junction, Mode::Program, Some(&fixture.root), &CancellationToken::new());
        let nested = inside(&junction.join("new-child"), &fixture.root);
        std::fs::remove_dir(&junction).unwrap();
        assert!(!accepted, "junction outside the scope was accepted");
        assert!(typed.is_err());
        assert!(read.is_err(), "worker followed an outside junction");
        assert!(!nested, "missing child concealed a redirected ancestor");
    }

    #[test]
    fn crumbs_name_the_drive_and_each_folder() {
        let crumbs = crumbs(Path::new(r"C:\Users\dev\Projects"));
        let labels: Vec<_> = crumbs.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, ["C:", "Users", "dev", "Projects"]);
        assert_eq!(crumbs[0].1, PathBuf::from(r"C:\"));
        assert_eq!(crumbs[3].1, PathBuf::from(r"C:\Users\dev\Projects"));
        // Canonical paths show their drive the usual way.
        assert_eq!(super::crumbs(Path::new(r"\\?\C:\Users"))[0].0, "C:");
        assert_eq!(super::crumbs(Path::new(r"\\server\share\repo"))[0].0, r"\\server\share");
    }

    #[test]
    fn typed_paths_must_be_full_and_stay_in_scope() {
        let fixture = scoped_fixture();
        let scope = fixture.root.as_path();
        let inside = fixture.repository.to_string_lossy();
        assert!(typed_folder(&inside, Some(scope)).is_ok());
        assert!(typed_folder(&format!("\"{inside}\""), Some(scope)).is_ok());
        assert!(typed_folder(r"C:\Users", Some(scope)).is_err());
        assert!(typed_folder(&format!("{inside}\\..\\Users"), Some(scope)).is_err());
        assert!(typed_folder(r"fixtures\repo", None).is_err());
        assert!(typed_folder(r"D:\work", None).is_ok());
        // The test host's scope is canonical; a usual spelling inside it is accepted.
        let canonical = fixture.root.canonicalize().unwrap();
        assert!(typed_folder(&inside, Some(&canonical)).is_ok());
        assert!(typed_folder(r"C:\elsewhere", Some(&canonical)).is_err());
        assert!(super::inside(&fixture.root.join("new-repository"), scope), "creation preview accepts a missing leaf");
    }

    #[test]
    fn scoped_metadata_does_not_follow_outside_git_pointers() {
        let fixture = scoped_fixture(); let outside = scoped_fixture();
        let linked = fixture.root.join("linked"); std::fs::create_dir(&linked).unwrap();
        let pointer = linked.join(".git");
        std::fs::write(&pointer, format!("gitdir: {}\n", outside.repository.join(".git").display())).unwrap();
        let cancel = CancellationToken::new();
        let scoped = read(&linked, Mode::Repository, Some(&fixture.root), &cancel).unwrap();
        assert_eq!(scoped.repository, None);
        assert_eq!(read(&linked, Mode::Repository, None, &cancel).unwrap().repository.as_deref(), Some("main"));
        std::fs::write(&pointer, format!("gitdir: {}\n", fixture.repository.join(".git").display())).unwrap();
        assert_eq!(read(&linked, Mode::Repository, Some(&fixture.root), &cancel).unwrap().repository.as_deref(), Some("main"));
    }

    #[test]
    fn folders_list_repositories_with_their_branch() {
        let fixture = scoped_fixture();
        let root = &fixture.root;
        std::fs::create_dir(root.join("plain")).unwrap();
        std::fs::create_dir(root.join("Alpha")).unwrap();
        // A linked worktree's `.git` file points at the real Git directory.
        let linked = root.join("linked");
        std::fs::create_dir(&linked).unwrap();
        std::fs::write(linked.join(".git"), format!("gitdir: {}\n", fixture.repository.join(".git").display())).unwrap();
        std::fs::write(root.join("tool.exe"), b"").unwrap();
        std::fs::write(root.join("notes.txt"), b"").unwrap();
        let cancel = CancellationToken::new();

        let listing = read(root, Mode::Repository, None, &cancel).unwrap();
        let names: Vec<_> = listing.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.starts_with(&["Alpha", "linked", "no-hooks", "plain"]), "{names:?}");
        assert!(!names.contains(&"tool.exe"), "files are not listed for repositories");
        let repository = listing.entries.iter().find(|e| e.path == fixture.repository).unwrap();
        assert_eq!(repository.repository.as_deref(), Some("main"));
        let linked = listing.entries.iter().find(|e| e.name == "linked").unwrap();
        assert_eq!(linked.repository.as_deref(), Some("main"));
        assert_eq!(listing.entries.iter().find(|e| e.name == "plain").unwrap().repository, None);
        assert_eq!(listing.repository, None);

        let inside = read(&fixture.repository, Mode::Repository, None, &cancel).unwrap();
        assert_eq!(inside.repository.as_deref(), Some("main"));
        assert!(inside.entries.iter().all(|e| e.name != ".git"));

        let programs = read(root, Mode::Program, None, &cancel).unwrap();
        let last = programs.entries.last().unwrap();
        assert!(last.program && last.name == "tool.exe", "programs follow the folders");
        assert!(programs.entries.iter().all(|e| e.name != "notes.txt"));

        let files = read(root, Mode::File, Some(root), &cancel).unwrap();
        assert!(files.entries.iter().any(|e| e.program && e.name == "notes.txt"));
        assert!(files.entries.iter().all(|e| e.name != ".git"));

        let limited = read_limited(root, Mode::Repository, None, 2, &cancel).unwrap();
        assert!(limited.truncated);
        assert_eq!(limited.entries.len(), 2);
    }

    #[test]
    fn cancelled_reads_stop() {
        let fixture = Fixture::new();
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(read(&fixture.root, Mode::Repository, None, &cancel).is_err());
    }

    #[test]
    fn a_scoped_browser_never_leaves_its_folder() {
        let fixture = scoped_fixture();
        let mut browser = Browser::new(Mode::Repository, Some(fixture.root.clone()), PathBuf::from(r"C:\"), None);
        assert_eq!(browser.folder(), fixture.root.as_path(), "a start outside the scope moves into it");
        assert!(!browser.can_go_up());
        browser.up();
        assert_eq!(browser.folder(), fixture.root.as_path());
        browser.go(PathBuf::from(r"C:\Windows"), None);
        assert_eq!(browser.folder(), fixture.root.as_path());
        browser.go(fixture.repository.clone(), None);
        assert_eq!(browser.folder(), fixture.repository.as_path());
        assert!(browser.can_go_up());
        browser.back();
        assert_eq!(browser.folder(), fixture.root.as_path());
    }

    #[test]
    fn the_target_is_the_selected_repository_or_the_open_folder() {
        let fixture = Fixture::new();
        std::fs::create_dir(fixture.root.join("plain")).unwrap();
        let mut browser = Browser::new(Mode::Repository, None, fixture.root.clone(), None);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while browser.listing.is_none() {
            assert!(std::time::Instant::now() < deadline, "folder read finished");
            browser.poll(&Context::default());
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(browser.repository(), None);
        let name = fixture.repository.file_name().unwrap().to_string_lossy().into_owned();
        browser.selected = Some(name);
        assert_eq!(browser.repository(), Some((fixture.repository.clone(), "main".into())));
        browser.selected = Some("plain".into());
        assert_eq!(browser.repository(), None, "a plain folder is selected");
        assert_eq!(
            browser.activate(&browser.selected_entry().unwrap().clone()),
            None,
            "double-clicking a plain folder goes into it"
        );
        assert_eq!(browser.folder(), fixture.root.join("plain").as_path());
    }
}
