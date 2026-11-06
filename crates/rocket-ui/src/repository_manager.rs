//! Global repository library. Opening adds tabs; organization never moves files.
//!
//! The page has its own sidebar (Library, Folders, Workspaces) in the app sidebar's place, the
//! selection's repositories in a header-bar card and the selected item in a details card.
use crate::{
    app::*,
    backend::user_error,
    folder_browser::PickTarget,
    pages::{self, Part},
    preferences::card_foot,
    repository_library::*,
    repository_status::{BatchKind, Cell, Row, chip, icon_button, paint_cell},
    style::*,
};
use eframe::egui::*;
use rocket_git::ReadTask;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    path::{Path, PathBuf},
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Selection {
    #[default]
    All,
    Ungrouped,
    Folder(u64),
    Workspace(u64),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormKind {
    Repository(Option<u64>),
    Folder(Option<u64>),
    Workspace(Option<u64>),
    RemoveRepository(u64),
    RemoveFolder(u64),
    RemoveWorkspace(u64),
}
impl FormKind {
    fn removal(self) -> bool {
        matches!(
            self,
            Self::RemoveRepository(_) | Self::RemoveFolder(_) | Self::RemoveWorkspace(_)
        )
    }
}
pub struct Form {
    pub kind: FormKind,
    pub name: String,
    pub path: String,
    pub folder: Option<u64>,
    pub parent: Option<u64>,
    pub members: Vec<u64>,
    pub active: Option<u64>,
    pub error: Option<String>,
    baseline: Library,
    draft: Library,
}
struct Opening {
    pending: VecDeque<(u64, PathBuf)>,
    active: Option<PathBuf>,
    opened: usize,
    failed: usize,
}
/// How the last opening ended, for the colour of its result line.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Outcome {
    #[default]
    Opened,
    Partial,
    Stopped,
}
#[derive(Default)]
pub struct State {
    pub selection: Selection,
    pub selected: Option<u64>,
    pub filter: String,
    pub include_nested: bool,
    pub collapsed: BTreeSet<u64>,
    pub form: Option<Form>,
    pub scan: Option<crate::repository_scan::State>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub unavailable: BTreeMap<u64, String>,
    check: Option<ReadTask<PathBuf>>,
    checking_id: Option<u64>,
    opening: Option<Opening>,
    outcome: Outcome,
    /// The details card's full height last frame, so a list stacked above it leaves it room.
    details_height: f32,
    /// Each repository's status, and Fetch all / Pull all.
    pub status: crate::repository_status::Status,
}
impl State {
    fn select(&mut self, selection: Selection) {
        self.selection = selection;
        self.selected = None;
        self.include_nested = false;
        self.status.result = None;
        self.clear_result();
    }
    /// Select a repository row. The last result belonged to the previous selection.
    fn pick(&mut self, id: u64) {
        self.status.result = None;
        if self.selected != Some(id) {
            self.selected = Some(id);
            self.clear_result();
        }
    }
    pub(crate) fn clear_result(&mut self) {
        self.error = None;
        self.notice = None;
    }
    pub fn running(&self) -> bool {
        self.check.is_some()
            || self.opening.is_some()
            || self.status.batch.is_some()
            || self.scan.as_ref().is_some_and(|s| s.running())
    }
    pub fn opening(&self) -> bool {
        self.opening.is_some()
    }
    pub fn remaining(&self) -> usize {
        self.opening.as_ref().map_or(0, |o| o.pending.len())
            + usize::from(self.checking_id.is_some())
    }
    pub fn cancel(&mut self) {
        if let Some(scan)=&mut self.scan {scan.cancel();}
        self.check = None;
        self.checking_id = None;
        if let Some(o) = self.opening.take() {
            self.notice = Some(format!("Stopped · {} opened", o.opened));
            self.outcome = Outcome::Stopped;
        }
    }
    pub fn cancel_form(&mut self) {
        self.check = None;
        self.form = None;
        self.scan = None;
    }
}
/// The details rows' and the forms' label column.
const LABEL: f32 = 92.0;
/// The Active tab column of a workspace form's repository list.
const ACTIVE: f32 = 84.0;
/// "1 repository", "3 repositories".
fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}
/// The repositories a selection lists, in list order: a workspace's members (with its
/// nested workspaces' when asked), or the saved ones in a folder, ungrouped or all.
pub(crate) fn selection_repositories(
    library: &Library,
    selection: Selection,
    include_nested: bool,
) -> Result<Vec<&SavedRepository>, &'static str> {
    if let Selection::Workspace(id) = selection {
        if !library.workspaces.iter().any(|w| w.id == id) {
            return Ok(Vec::new());
        }
        return Ok(library
            .workspace_members(id, include_nested)?
            .iter()
            .filter_map(|id| library.repositories.iter().find(|r| r.id == *id))
            .collect());
    }
    Ok(library
        .repositories
        .iter()
        .filter(|r| match selection {
            Selection::Ungrouped => r.folder.is_none(),
            Selection::Folder(id) => r.folder == Some(id),
            _ => true,
        })
        .collect())
}
/// The selection's name, as the list's bar shows it.
pub(crate) fn selection_title(library: &Library, selection: Selection) -> String {
    match selection {
        Selection::All => "All saved".to_owned(),
        Selection::Ungrouped => "Ungrouped".to_owned(),
        Selection::Folder(id) => library
            .folders
            .iter()
            .find(|f| f.id == id)
            .map_or_else(|| "Folder".to_owned(), |f| f.name.clone()),
        Selection::Workspace(id) => library
            .workspaces
            .iter()
            .find(|w| w.id == id)
            .map_or_else(|| "Workspace".to_owned(), |w| w.name.clone()),
    }
}
/// A failed library check: a missing repository in plain words, anything else as Git said it.
pub(crate) fn check_error(error: &rocket_git::Error) -> String {
    match error {
        rocket_git::Error::Git { operation: "discovery", code: Some(128), .. } => {
            "No Git repository at this location (Git discovery, exit 128).".into()
        }
        rocket_git::Error::Io { action: "resolve repository directory", .. } => {
            "This location doesn't exist or can't be read.".into()
        }
        _ => user_error(error),
    }
}
/// What the page was asked to do this frame; applied once it is drawn.
#[derive(Default)]
struct Asked {
    scan: bool,
    form: Option<FormKind>,
    /// A new empty workspace under this parent (`None`: the top level).
    workspace: Option<Option<u64>>,
    open: bool,
    stop: bool,
    close: bool,
    /// Check again: read the listed repositories' status now.
    check: bool,
    /// Fetch all or Pull all.
    batch: Option<BatchKind>,
    /// The result card's Open N in tabs, and its ×.
    result_open: bool,
    result_close: bool,
}
/// What the page's parts read and report to.
struct Parts<'a> {
    library: &'a Library,
    tabs: &'a [Tab],
    s: &'a mut State,
    widgets: &'a mut HashMap<String, UiControl>,
    busy: bool,
    editable: bool,
    /// A Git write runs in a tab: Fetch all and Pull all wait for it.
    writing: bool,
    asked: Asked,
}
fn add_actions(ui: &mut Ui, p: &mut Parts<'_>) {
    let r = pages::primary(ui, p.editable, "Add repository");
    pages::control(p.widgets, "manager-add", &r, ui.clip_rect());
    if r.clicked() { p.asked.form = Some(FormKind::Repository(None)); }
    let r = ui.add_enabled(p.editable, Button::new("Scan folder"));
    pages::control(p.widgets, "manager-scan", &r, ui.clip_rect());
    p.asked.scan = r.clicked();
    let r = ui.add_enabled(p.editable, Button::new("Save open tabs"))
        .on_hover_text("Save the open tabs as a new workspace");
    pages::control(p.widgets, "manager-workspace-save", &r, ui.clip_rect());
    if r.clicked() { p.asked.form = Some(FormKind::Workspace(None)); }
}
/// A sidebar heading; with `add` (its tooltip, automation id and whether it is enabled), a +
/// at its right edge. Returns whether + was clicked.
fn caption(
    ui: &mut Ui,
    text: &str,
    add: Option<(&str, &str, bool)>,
    widgets: &mut HashMap<String, UiControl>,
) -> bool {
    ui.add_space(8.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
    ui.painter().text(
        pos2(rect.left() + 10.0, rect.center().y),
        Align2::LEFT_CENTER,
        text.to_uppercase(),
        semibold(11.0),
        HINT(),
    );
    let Some((tip, id, enabled)) = add else {
        return false;
    };
    let slot = Rect::from_center_size(pos2(rect.right() - 12.0, rect.center().y), vec2(24.0, 24.0));
    let mut slot = ui.new_child(UiBuilder::new().max_rect(slot));
    let r = crate::chrome::icon_button(&mut slot, Icon::Plus, enabled, false, 24.0).on_hover_text(tip);
    pages::control(widgets, id, &r, ui.clip_rect());
    r.clicked()
}
/// A sidebar row: icon, name and count, `indent` points in; the selection is filled with a bar.
/// `attention` puts an amber dot before the count.
fn nav_row(ui: &mut Ui, image: Icon, label: &str, count: usize, indent: f32, selected: bool, attention: bool) -> Response {
    let enabled = ui.is_enabled();
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    let hot = enabled && (response.hovered() || response.has_focus());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 6.0, SELECTED());
        painter.rect_filled(
            Rect::from_min_size(rect.min + vec2(0.0, 8.0), vec2(2.5, rect.height() - 16.0)),
            1.0,
            BLUE(),
        );
    } else if hot {
        painter.rect_filled(rect, 6.0, HOVER());
    }
    let left = rect.left() + indent;
    icon(
        painter,
        Rect::from_center_size(pos2(left + 18.0, rect.center().y), vec2(15.0, 15.0)),
        image,
        if selected { BLUE() } else { HINT() },
    );
    // The count sits at the right edge and the name is cut before it.
    let number = painter.layout_no_wrap(count.to_string(), FontId::proportional(12.0), FAINT());
    let right = rect.right() - 10.0;
    let mut width = number.size().x;
    painter.galley(pos2(right - width, rect.center().y - number.size().y / 2.0), number, FAINT());
    if attention {
        painter.circle_filled(pos2(right - width - 9.5, rect.center().y), 3.5, AMBER());
        width += 13.0;
    }
    let color = if !enabled {
        MUTED()
    } else if selected || hot {
        TEXT()
    } else {
        TEXT_SOFT()
    };
    let font = if selected { semibold(13.0) } else { FontId::proportional(13.0) };
    let mut job = text::LayoutJob::simple_singleline(label.into(), font, color);
    job.wrap = text::TextWrapping::truncate_at_width((right - width - 8.0 - (left + 34.0)).max(0.0));
    let name = painter.layout_job(job);
    painter.galley(pos2(left + 34.0, rect.center().y - name.size().y / 2.0), name, color);
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, enabled, selected, label));
    response
}
/// An empty sidebar section.
fn none_yet(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
    ui.painter().text(
        pos2(rect.left() + 34.0, rect.center().y),
        Align2::LEFT_CENTER,
        "None yet",
        FontId::proportional(12.0),
        FAINT(),
    );
}
/// The library's sidebar: Library, Folders and Workspaces, each row with its count.
fn navigation(ui: &mut Ui, p: &mut Parts) {
    let (library, editable) = (p.library, p.editable);
    // A dot on each folder and workspace where a repository already checked needs attention.
    let needs = |r: &SavedRepository| p.s.status.attention(r.id, &r.path);
    let dots: (BTreeSet<u64>, BTreeSet<u64>) = (
        library
            .folders
            .iter()
            .filter(|f| library.repositories.iter().any(|r| r.folder == Some(f.id) && needs(r)))
            .map(|f| f.id)
            .collect(),
        library
            .workspaces
            .iter()
            .filter(|w| {
                w.repositories
                    .iter()
                    .filter_map(|id| library.repositories.iter().find(|r| r.id == *id))
                    .any(needs)
            })
            .map(|w| w.id)
            .collect(),
    );
    let output = ScrollArea::vertical()
        .id_salt("manager-navigation")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // While repositories open, the selection whose foot shows Stop stays put.
            ui.add_enabled_ui(!p.busy, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.add_space(4.0);
                caption(ui, "Library", None, p.widgets);
                let ungrouped = library.repositories.iter().filter(|r| r.folder.is_none()).count();
                for (selection, key, image, label, n) in [
                    (Selection::All, "manager-all", Icon::Database, "All saved", library.repositories.len()),
                    (Selection::Ungrouped, "manager-ungrouped", Icon::Layout, "Ungrouped", ungrouped),
                ] {
                    let r = nav_row(ui, image, label, n, 0.0, p.s.selection == selection, false);
                    pages::control(p.widgets, key, &r, ui.clip_rect());
                    if r.clicked() {
                        p.s.select(selection);
                    }
                }
                if caption(ui, "Folders", Some(("New folder", "manager-folder-new", editable)), p.widgets) {
                    p.asked.form = Some(FormKind::Folder(None));
                }
                for folder in &library.folders {
                    let n = library.repositories.iter().filter(|r| r.folder == Some(folder.id)).count();
                    let selected = p.s.selection == Selection::Folder(folder.id);
                    let dot = dots.0.contains(&folder.id);
                    let r = nav_row(ui, Icon::Folder, &folder.name, n, 0.0, selected, dot);
                    pages::control(p.widgets, format!("manager-folder-{}", folder.id), &r, ui.clip_rect());
                    if r.clicked() {
                        p.s.select(Selection::Folder(folder.id));
                    }
                }
                if library.folders.is_empty() {
                    none_yet(ui);
                }
                if caption(ui, "Workspaces", Some(("New workspace", "manager-workspace-new", editable)), p.widgets) {
                    p.asked.workspace = Some(None);
                }
                let mut hidden_below = None;
                for (id, depth) in library.workspace_tree() {
                    if hidden_below.is_some_and(|hidden| depth > hidden) {
                        continue;
                    }
                    hidden_below = None;
                    let Some(w) = library.workspaces.iter().find(|w| w.id == id) else {
                        continue;
                    };
                    // Workspace icons sit right of a chevron column; each level indents 16 points.
                    let indent = 10.0 + depth as f32 * 16.0;
                    let selected = p.s.selection == Selection::Workspace(id);
                    let dot = dots.1.contains(&id);
                    let r = nav_row(ui, Icon::Modules, &w.name, w.repositories.len(), indent, selected, dot)
                        .on_hover_text(library.workspace_path(id).unwrap_or_else(|| w.name.clone()));
                    pages::control(p.widgets, format!("manager-workspace-{id}"), &r, ui.clip_rect());
                    let collapsed = p.s.collapsed.contains(&id);
                    let mut toggled = false;
                    if library.workspaces.iter().any(|w| w.parent == Some(id)) {
                        // The chevron is its own target over the row's left edge.
                        let rect = Rect::from_center_size(
                            pos2(r.rect.left() + indent - 2.0, r.rect.center().y),
                            vec2(18.0, 26.0),
                        );
                        let t = ui.interact(rect, ui.id().with(("manager-toggle", id)), Sense::click());
                        let tip = if collapsed { "Expand workspace" } else { "Collapse workspace" };
                        let ink = if ui.is_enabled() && t.hovered() { TEXT() } else { HINT() };
                        icon(
                            ui.painter(),
                            Rect::from_center_size(rect.center(), vec2(11.0, 11.0)),
                            if collapsed { Icon::ChevronRight } else { Icon::Chevron },
                            ink,
                        );
                        t.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), tip));
                        let t = t.on_hover_text(tip);
                        pages::control(p.widgets, format!("manager-workspace-toggle-{id}"), &t, ui.clip_rect());
                        if t.clicked() {
                            toggled = true;
                            if !p.s.collapsed.remove(&id) {
                                p.s.collapsed.insert(id);
                                if let Selection::Workspace(selected) = p.s.selection
                                    && library.workspace_contains(id, selected)
                                {
                                    p.s.select(Selection::Workspace(id));
                                }
                            }
                        }
                    }
                    if r.clicked() && !toggled {
                        p.s.select(Selection::Workspace(id));
                    }
                    if p.s.collapsed.contains(&id) {
                        hidden_below = Some(depth);
                    }
                }
                if library.workspaces.is_empty() {
                    none_yet(ui);
                }
                ui.add_space(12.0);
            });
        });
    pages::region(p.widgets, "manager-navigation", output.inner_rect.intersect(ui.clip_rect()), true);
}
/// A card on the page: the panel fill, its outline and rounded corners. `fill` stretches it
/// to the height it was given. Returns where it was drawn.
fn card(ui: &mut Ui, fill: bool, content: impl FnOnce(&mut Ui)) -> Rect {
    Frame::new()
        .fill(PANEL())
        .stroke(Stroke::new(1.0, CARD_STROKE()))
        .corner_radius(8)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if fill {
                ui.set_min_height(ui.available_height());
            }
            ui.spacing_mut().item_spacing.y = 0.0;
            content(ui);
        })
        .response
        .rect
}
/// A card's 40-point header bar: an icon tile, the title and a quieter detail on the left, and
/// `side` laid out from the right edge.
fn bar(ui: &mut Ui, image: Option<Icon>, title: &str, detail: &str, side: impl FnOnce(&mut Ui)) {
    let mut row = bar_row(ui);
    side(&mut row);
    bar_title(&mut row, image, title, detail);
}
/// [`bar`]'s row, laid out from the right edge for its controls; [`bar_title`] fills the rest.
fn bar_row(ui: &mut Ui) -> Ui {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius { nw: 7, ne: 7, sw: 0, se: 0 }, CARD());
    ui.painter()
        .hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, CARD_LINE()));
    // Drawn into the rectangle already taken, so the rows below start after the bar.
    let mut row = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink2(vec2(12.0, 0.0)))
            .layout(Layout::right_to_left(Align::Center)),
    );
    row.spacing_mut().item_spacing.x = 8.0;
    row
}
fn bar_title(bar: &mut Ui, image: Option<Icon>, title: &str, detail: &str) {
    bar.with_layout(Layout::left_to_right(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if let Some(image) = image {
            let (tile, _) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::hover());
            ui.painter().rect_filled(tile, 6.0, RAISED());
            icon(ui.painter(), tile.shrink(5.5), image, BLUE());
        }
        if !title.is_empty() {
            ui.add(Label::new(RichText::new(title).font(semibold(13.0))).truncate());
        }
        if !detail.is_empty() {
            ui.add(Label::new(RichText::new(detail).size(12.0).color(FAINT())).truncate());
        }
    });
}
/// A tag beside a repository's name, such as Open or Active tab: its text and colour.
type Pill = (String, Color32);
/// A 50-point repository row: its tile; the name with its tags over the branch and path;
/// then the Changes and Upstream columns at the right edge.
fn repo_row(ui: &mut Ui, repo: &SavedRepository, tags: &[Pill], status: &Row, selected: bool, click: bool) -> Response {
    let enabled = ui.is_enabled();
    let sense = if click { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 50.0), sense);
    let hot = click && enabled && (response.hovered() || response.has_focus());
    if selected {
        ui.painter().rect_filled(rect, 0.0, SELECTED());
        ui.painter().rect_filled(Rect::from_min_size(rect.min, vec2(3.0, rect.height())), 0.0, BLUE());
    } else if hot {
        ui.painter().rect_filled(rect, 0.0, HOVER());
    }
    ui.painter()
        .hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, ROW_LINE()));
    let tile = Rect::from_min_size(pos2(rect.left() + 14.0, rect.center().y - 15.0), vec2(30.0, 30.0));
    ui.painter().rect_filled(tile, 7.0, RAISED());
    let (image, ink) = if status.problem {
        (Icon::Folder, RED())
    } else if selected {
        (Icon::Branch, BLUE())
    } else {
        (Icon::Branch, MUTED())
    };
    icon(ui.painter(), tile.shrink(8.0), image, ink);
    // Two columns at the right edge, narrower in a narrow window, so their contents line
    // up from row to row.
    let (changes, upstream, gap) = if rect.width() >= 600.0 { (104.0, 116.0, 12.0) } else { (80.0, 92.0, 10.0) };
    let right = rect.right() - 14.0;
    let upstream_column = Rect::from_min_max(pos2(right - upstream, rect.top()), pos2(right, rect.bottom()));
    let changes_column = Rect::from_min_max(
        pos2(upstream_column.left() - gap - changes, rect.top()),
        pos2(upstream_column.left() - gap, rect.bottom()),
    );
    for (column, cell) in [(changes_column, &status.changes), (upstream_column, &status.upstream)] {
        if let Some(cell) = cell {
            paint_cell(ui, column, cell);
        }
    }
    let left = tile.right() + 12.0;
    let width = (changes_column.left() - 12.0 - left).max(0.0);
    let color = if enabled { TEXT() } else { MUTED() };
    // The tags follow the name; the name is cut first.
    let tag_galleys: Vec<_> = tags
        .iter()
        .map(|(text, color)| (ui.painter().layout_no_wrap(text.clone(), semibold(10.5), *color), *color))
        .collect();
    let tags_width: f32 = tag_galleys.iter().map(|(g, _)| g.size().x + 14.0 + 6.0).sum();
    let mut job = text::LayoutJob::simple_singleline(repo.name.clone(), semibold(13.0), color);
    job.wrap = text::TextWrapping::truncate_at_width((width - tags_width).max(width.min(60.0)));
    let name = ui.painter().layout_job(job);
    let line = rect.center().y - 9.0;
    let mut x = left + name.size().x + 7.0;
    ui.painter().galley(pos2(left, line - name.size().y / 2.0), name, color);
    for (galley, tint) in tag_galleys {
        let tag = Rect::from_min_size(pos2(x, line - 9.0), vec2(galley.size().x + 14.0, 18.0));
        if tag.right() > left + width {
            break;
        }
        ui.painter().rect_filled(tag, 9.0, tint.gamma_multiply(0.14));
        ui.painter().galley(pos2(tag.left() + 7.0, line - galley.size().y / 2.0), galley, tint);
        x = tag.right() + 6.0;
    }
    // The branch, then the path, on the second line.
    let line = rect.center().y + 9.0;
    let mut job = text::LayoutJob::default();
    let mut at = left;
    if let Some(branch) = &status.branch {
        icon(ui.painter(), Rect::from_center_size(pos2(left + 5.5, line), vec2(11.0, 11.0)), Icon::Branch, MUTED());
        at += 15.0;
        let soft = TextFormat { font_id: FontId::proportional(11.5), color: TEXT_SOFT(), ..Default::default() };
        let faint = TextFormat { font_id: FontId::proportional(11.5), color: FAINT(), ..Default::default() };
        job.append(branch, 0.0, soft);
        job.append(" · ", 0.0, faint);
    }
    let faint = TextFormat { font_id: FontId::proportional(11.5), color: FAINT(), ..Default::default() };
    job.append(&crate::views::display_path(&repo.path), 0.0, faint);
    job.wrap = text::TextWrapping::truncate_at_width((left + width - at).max(0.0));
    let second = ui.painter().layout_job(job);
    ui.painter().galley(pos2(at, line - second.size().y / 2.0), second, FAINT());
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, enabled, selected, &repo.name));
    response
}
/// A card body with nothing to list: an optional icon, the text centred, and actions under it
/// (`actions` lays them out left to right; `width` is their total width, to centre them).
fn empty(ui: &mut Ui, image: Option<Icon>, text: &str, width: f32, actions: impl FnOnce(&mut Ui)) {
    let height = ui.available_height().clamp(120.0, 4000.0);
    let full = ui.available_width();
    ui.allocate_ui_with_layout(vec2(full, height), Layout::top_down(Align::Center), |ui| {
        ui.set_min_size(vec2(full, height));
        let group = (if image.is_some() { 50.0 } else { 0.0 }) + 20.0 + (if width > 0.0 { 40.0 } else { 0.0 });
        ui.add_space(((height - group) / 2.0).max(12.0));
        if let Some(image) = image {
            let (rect, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
            ui.painter().circle_filled(rect.center(), 20.0, BLUE().gamma_multiply(0.14));
            icon(ui.painter(), rect.shrink(11.0), image, BLUE());
            ui.add_space(4.0);
        }
        ui.add(Label::new(RichText::new(text).size(14.0).color(TEXT())).wrap());
        if width > 0.0 {
            ui.add_space(6.0);
            ui.allocate_ui_with_layout(vec2(width, CONTROL), Layout::left_to_right(Align::Center), actions);
        }
    });
}
/// The width of buttons labelled `labels`, with the gaps between them.
fn buttons_width(ui: &Ui, labels: &[&str]) -> f32 {
    let padding = ui.spacing().button_padding.x * 2.0;
    let gaps = ui.spacing().item_spacing.x * labels.len().saturating_sub(1) as f32;
    labels
        .iter()
        .map(|label| ui.painter().layout_no_wrap((*label).into(), FontId::proportional(13.0), TEXT()).size().x + padding)
        .sum::<f32>()
        + gaps
        + 4.0
}
/// The selection's repositories: the header bar with its name, count and Filter (a folder
/// adds Rename and Remove), then the rows or what to do when there are none.
fn repository_list(ui: &mut Ui, p: &mut Parts) {
    let library = p.library;
    let workspace = match p.s.selection {
        Selection::Workspace(id) => library.workspaces.iter().find(|w| w.id == id),
        _ => None,
    };
    let membership = selection_repositories(library, p.s.selection, p.s.include_nested);
    let in_selection: Vec<&SavedRepository> = membership.as_ref().map_or_else(|_| Vec::new(), Clone::clone);
    // Each listed repository's status is read once a visit, a few at a time.
    p.s.status.want(in_selection.iter().map(|r| (r.id, r.path.as_path())));
    // A workspace's nested workspaces in tree order: where a repository it opens comes from.
    let nested: Vec<&Workspace> = workspace.map_or_else(Vec::new, |w| {
        library
            .workspace_tree()
            .iter()
            .filter(|(id, _)| *id != w.id && library.workspace_contains(w.id, *id))
            .filter_map(|(id, _)| library.workspaces.iter().find(|x| x.id == *id))
            .collect()
    });
    let active = workspace.and_then(|w| {
        w.active
            .filter(|a| w.repositories.contains(a))
            .or_else(|| w.repositories.first().copied())
    });
    let title = selection_title(library, p.s.selection);
    let editable = p.editable;
    card(ui, true, |ui| {
        // The filter and folder tools come first, so the rows below use what was just typed.
        let mut row = bar_row(ui);
        {
            let ui = &mut row;
            let width = (ui.available_width() * 0.42).clamp(120.0, 240.0);
            let typed = !p.s.filter.is_empty();
            let r = ui.add_sized(
                [width, CONTROL],
                TextEdit::singleline(&mut p.s.filter)
                    .id_salt("manager-filter")
                    .hint_text("Filter by name or path")
                    .margin(Margin { left: 26, right: if typed { 24 } else { 8 }, top: 0, bottom: 0 })
                    .vertical_align(Align::Center),
            );
            pages::control(p.widgets, "manager-filter", &r, ui.clip_rect());
            icon(
                ui.painter(),
                Rect::from_center_size(pos2(r.rect.left() + 14.0, r.rect.center().y), vec2(13.0, 13.0)),
                Icon::Search,
                FAINT(),
            );
            if typed {
                let x = Rect::from_center_size(pos2(r.rect.right() - 13.0, r.rect.center().y), vec2(18.0, 18.0));
                let clear = ui.interact(x, ui.id().with("manager-filter-x"), Sense::click());
                let ink = if clear.hovered() { TEXT() } else { HINT() };
                icon(ui.painter(), Rect::from_center_size(x.center(), vec2(10.0, 10.0)), Icon::Close, ink);
                if clear.on_hover_text("Clear filter").clicked() {
                    p.s.filter.clear();
                }
            }
            if let Selection::Folder(id) = p.s.selection {
                let r = remove_tool(ui, editable);
                pages::control(p.widgets, "manager-folder-remove", &r, ui.clip_rect());
                if r.clicked() {
                    p.asked.form = Some(FormKind::RemoveFolder(id));
                }
                let r = pages::tool(ui, Icon::Pencil, "Rename", editable);
                pages::control(p.widgets, "manager-folder-edit", &r, ui.clip_rect());
                if r.clicked() {
                    p.asked.form = Some(FormKind::Folder(Some(id)));
                }
            }
        }
        let query = p.s.filter.to_lowercase();
        let status = &p.s.status;
        let attention = in_selection.iter().filter(|r| status.attention(r.id, &r.path)).count();
        let pullable = status.pullable(in_selection.iter().map(|r| (r.id, r.path.as_path())));
        let attention_only = status.attention_only;
        let shown: Vec<&SavedRepository> = in_selection
            .iter()
            .filter(|r| {
                query.is_empty()
                    || r.name.to_lowercase().contains(&query)
                    || r.path.to_string_lossy().to_lowercase().contains(&query)
            })
            .filter(|r| !attention_only || status.attention(r.id, &r.path))
            .copied()
            .collect();
        let counted = if !query.is_empty() {
            format!("{} of {}", shown.len(), in_selection.len())
        } else if workspace.is_some() && p.s.include_nested && !nested.is_empty() {
            format!("{} with nested", in_selection.len())
        } else {
            in_selection.len().to_string()
        };
        bar_title(&mut row, None, &title, &counted);
        if library.repositories.is_empty() {
            let labels = ["Add repository", "Scan folder"];
            let width = buttons_width(ui, &labels);
            empty(ui, Some(Icon::Database), "No saved repositories", width, |ui| {
                let r = pages::primary(ui, editable, labels[0]);
                pages::control(p.widgets, "manager-empty-add", &r, ui.clip_rect());
                if r.clicked() {
                    p.asked.form = Some(FormKind::Repository(None));
                }
                let r = ui.add_enabled(editable, Button::new(labels[1]));
                pages::control(p.widgets, "manager-empty-scan", &r, ui.clip_rect());
                if r.clicked() {
                    p.asked.scan = true;
                }
            });
            return;
        }
        if let Err(error) = &membership {
            Frame::new().inner_margin(14).show(ui, |ui| pages::callout(ui, AMBER(), error));
            return;
        }
        if !in_selection.is_empty() {
            status_strip(ui, p, in_selection.len(), attention, pullable);
        }
        if shown.is_empty() {
            if attention_only && in_selection.iter().any(|r| {
                query.is_empty()
                    || r.name.to_lowercase().contains(&query)
                    || r.path.to_string_lossy().to_lowercase().contains(&query)
            }) {
                let width = buttons_width(ui, &["Show all"]);
                empty(ui, None, "Nothing here needs attention", width, |ui| {
                    let r = pages::ghost(ui, true, "Show all");
                    pages::control(p.widgets, "manager-attention-clear", &r, ui.clip_rect());
                    if r.clicked() {
                        p.s.status.attention_only = false;
                        ui.ctx().request_repaint();
                    }
                });
            } else if !query.is_empty() {
                let width = buttons_width(ui, &["Clear filter"]);
                empty(ui, None, &format!("No repositories match “{}”", p.s.filter), width, |ui| {
                    let r = pages::ghost(ui, true, "Clear filter");
                    pages::control(p.widgets, "manager-filter-clear", &r, ui.clip_rect());
                    if r.clicked() {
                        p.s.filter.clear();
                        ui.ctx().request_repaint();
                    }
                });
            } else {
                let text = match p.s.selection {
                    Selection::Folder(_) => format!("No repositories in {title}"),
                    Selection::Ungrouped => "Every repository is in a folder".to_owned(),
                    Selection::Workspace(_) if !nested.is_empty() && !p.s.include_nested => {
                        "No repositories of its own".to_owned()
                    }
                    _ => "No repositories".to_owned(),
                };
                empty(ui, None, &text, 0.0, |_| {});
            }
            return;
        }
        let output = ScrollArea::vertical()
            .id_salt("manager-repositories")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_enabled_ui(!p.busy, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let checking = p.s.opening.as_ref().and(p.s.checking_id);
                    let waiting: Vec<u64> = p
                        .s
                        .opening
                        .as_ref()
                        .map_or_else(Vec::new, |o| o.pending.iter().map(|(id, _)| *id).collect());
                    let mut picked = None;
                    for repo in &shown {
                        let mut tags: Vec<Pill> = Vec::new();
                        if let Some(w) = workspace {
                            if !w.repositories.contains(&repo.id) {
                                if let Some(from) = nested.iter().find(|x| x.repositories.contains(&repo.id)) {
                                    tags.push((from.name.clone(), pages::NEUTRAL()));
                                }
                            } else if active == Some(repo.id) {
                                tags.push(("Active tab".into(), pages::INFO()));
                            }
                        }
                        if p.tabs.iter().any(|t| same_path(&t.path, &repo.path)) {
                            tags.push(("Open".into(), pages::INFO()));
                        }
                        let mut status = p.s.status.row(repo.id, &repo.path);
                        // Opening repositories one by one shows in the Upstream column.
                        if checking == Some(repo.id) {
                            status.upstream = Some(Cell::Busy("Opening".into()));
                        } else if waiting.contains(&repo.id) {
                            status.upstream = Some(Cell::Pill("Waiting".into(), pages::NEUTRAL()));
                        }
                        // A workspace lists what it opens; its card on the right is the selection.
                        if workspace.is_some() {
                            repo_row(ui, repo, &tags, &status, false, false);
                            continue;
                        }
                        let selected = p.s.selected == Some(repo.id);
                        let r = repo_row(ui, repo, &tags, &status, selected, true);
                        pages::control(p.widgets, format!("manager-repo-{}", repo.id), &r, ui.clip_rect());
                        if r.clicked() {
                            picked = Some(repo.id);
                        }
                    }
                    if let Some(id) = picked {
                        p.s.pick(id);
                        // The row was painted before its click; show the selection next frame.
                        ui.ctx().request_repaint();
                    }
                });
            });
        pages::region(p.widgets, "manager-repositories", output.inner_rect.intersect(ui.clip_rect()), !p.busy);
    });
}
/// Under the list's bar: the All and Needs attention chips, then the status reads'
/// progress or the last run's result, Check again, Fetch all and Pull all. While a run
/// goes, its progress and Stop instead.
fn status_strip(ui: &mut Ui, p: &mut Parts, all: usize, attention: usize, pullable: bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, FOOTER());
    ui.painter()
        .hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, CARD_LINE()));
    let mut row = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink2(vec2(12.0, 0.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    row.spacing_mut().item_spacing.x = 8.0;
    let on = p.s.status.attention_only;
    let r = chip(&mut row, "All", all, !on, false);
    pages::control(p.widgets, "manager-chip-all", &r, row.clip_rect());
    if r.clicked() {
        p.s.status.attention_only = false;
        row.ctx().request_repaint();
    }
    let r = chip(&mut row, "Needs attention", attention, on, attention > 0);
    pages::control(p.widgets, "manager-chip-attention", &r, row.clip_rect());
    if r.clicked() {
        p.s.status.attention_only = true;
        row.ctx().request_repaint();
    }
    let busy_line = |ui: &mut Ui, text: &str| {
        ui.add(Label::new(RichText::new(text).size(12.5).color(TEXT_SOFT())).truncate());
        ui.add(Spinner::new().size(13.0));
    };
    row.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if let Some(batch) = &p.s.status.batch {
            let r = pages::danger_ghost(ui, !batch.stopping, "Stop");
            pages::control(p.widgets, "manager-stop", &r, ui.clip_rect());
            p.asked.stop |= r.clicked();
            busy_line(ui, &batch.progress());
            return;
        }
        let (ready, writing) = (p.editable && !p.writing, p.writing);
        let wait = |r: Response| {
            if writing { r.on_disabled_hover_text("Waiting for the current Git operation") } else { r }
        };
        let r = wait(icon_button(ui, Icon::Pull, "Pull all", ready && pullable));
        let r = if pullable { r } else { r.on_disabled_hover_text("No repository here has an upstream") };
        pages::control(p.widgets, "manager-pull-all", &r, ui.clip_rect());
        if r.clicked() {
            p.asked.batch = Some(BatchKind::Pull);
        }
        let r = wait(icon_button(ui, Icon::Fetch, "Fetch all", ready));
        pages::control(p.widgets, "manager-fetch-all", &r, ui.clip_rect());
        if r.clicked() {
            p.asked.batch = Some(BatchKind::Fetch);
        }
        let r = crate::chrome::icon_button(ui, Icon::Refresh, p.editable && !p.s.status.checking(), false, 28.0)
            .on_hover_text("Check again");
        pages::control(p.widgets, "manager-check", &r, ui.clip_rect());
        p.asked.check |= r.clicked();
        // What the reads or the last run say, in the room that is left.
        if let Some(text) = p.s.status.progress() {
            busy_line(ui, &text);
        } else if let Some(result) = p.s.status.result.as_ref().filter(|r| r.selection == p.s.selection) {
            let (short, _, tone) = result.summary();
            ui.add(Label::new(RichText::new(&short).size(12.5).color(tone.color())).truncate())
                .on_hover_text(&short);
            let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
            icon(ui.painter(), rect, tone.icon(), tone.color());
        } else if let Some(at) = p.s.status.checked
            && ui.available_width() > 150.0
        {
            ui.add(Label::new(RichText::new(crate::repository_status::checked_text(at)).size(12.0).color(HINT())).truncate());
        }
    });
}
/// [`pages::tool`] with a trash icon and red ink, for removing the selected folder.
fn remove_tool(ui: &mut Ui, enabled: bool) -> Response {
    let font = FontId::proportional(12.5);
    let width = ui.painter().layout_no_wrap("Remove".into(), font.clone(), RED()).size().x;
    let response = ui.add_enabled(enabled, Button::new("").frame(false).min_size(vec2(width + 38.0, 28.0)));
    let rect = response.rect;
    let hot = enabled && (response.hovered() || response.has_focus());
    if hot {
        ui.painter()
            .rect(rect, 6.0, HOVER(), Stroke::new(1.0, EDGE_HOT()), StrokeKind::Inside);
    }
    let ink = if enabled { RED() } else { RED().gamma_multiply(0.5) };
    icon(
        ui.painter(),
        Rect::from_center_size(pos2(rect.left() + 15.5, rect.center().y), vec2(13.0, 13.0)),
        Icon::Trash,
        ink,
    );
    ui.painter()
        .text(pos2(rect.left() + 28.0, rect.center().y), Align2::LEFT_CENTER, "Remove", font, ink);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, "Remove"));
    response
}
/// A details row: the label in a fixed column and the value beside it, wrapping under itself.
fn detail(ui: &mut Ui, label: &str, value: &str, color: Color32) {
    Frame::new().inner_margin(Margin::symmetric(14, 6)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            let (rect, _) = ui.allocate_exact_size(vec2(LABEL, 18.0), Sense::hover());
            ui.painter()
                .text(rect.left_center(), Align2::LEFT_CENTER, label, FontId::proportional(13.0), HINT());
            ui.add(Label::new(RichText::new(value).size(13.0).color(color)).wrap());
        });
    });
}
/// The last opening's result, or what went wrong, in the space left in the foot.
fn result_line(ui: &mut Ui, s: &State) {
    let (text, image, color) = match (&s.notice, &s.error) {
        (Some(notice), _) => match s.outcome {
            Outcome::Opened => (notice, Icon::Check, GREEN()),
            Outcome::Partial => (notice, Icon::Warning, AMBER()),
            Outcome::Stopped => (notice, Icon::Info, HINT()),
        },
        (None, Some(error)) => (error, Icon::Warning, AMBER()),
        (None, None) => return,
    };
    ui.spacing_mut().item_spacing.x = 6.0;
    let (rect, _) = ui.allocate_exact_size(vec2(13.0, 13.0), Sense::hover());
    icon(ui.painter(), rect, image, color);
    ui.add(Label::new(RichText::new(text).size(12.5).color(color)).truncate())
        .on_hover_text(text);
}
/// The foot while repositories open: Stop, and which one is opening of how many.
fn opening_foot(ui: &mut Ui, p: &mut Parts) -> bool {
    let Some(o) = &p.s.opening else {
        return false;
    };
    let r = pages::danger_ghost(ui, true, "Stop");
    pages::control(p.widgets, "manager-stop", &r, ui.clip_rect());
    p.asked.stop = r.clicked();
    let done = o.opened + o.failed;
    let total = done + o.pending.len() + usize::from(p.s.checking_id.is_some());
    let name = p
        .s
        .checking_id
        .and_then(|id| p.library.repositories.iter().find(|r| r.id == id))
        .map_or_else(String::new, |r| format!(" · {}", r.name));
    pages::footer_status(ui, true, &format!("Opening {} of {}{name}", (done + 1).min(total.max(1)), total.max(1)));
    true
}
/// After Fetch all or Pull all, in the details card's place: what happened to each
/// repository, failures and skips first, and Open N in tabs for those. Returns the card's
/// natural height.
fn result_card(ui: &mut Ui, p: &mut Parts, room: f32) -> f32 {
    use crate::repository_status::Outcome;
    let Some(result) = p.s.status.result.as_ref() else {
        return 0.0;
    };
    let library = p.library;
    let status = &p.s.status;
    let (_, long, tone) = result.summary();
    let kind = result.kind;
    let detail = format!(
        "{} · {}",
        result.scope,
        crate::repository_status::plural(result.total, "repository", "repositories")
    );
    let took = format!("Took {} s", result.took.as_secs().max(1));
    // Each group's rows: the name, what happened in words and colour, and a pill.
    type Line = (String, String, Color32, Option<(String, Color32)>);
    let mut groups: Vec<(&str, Vec<Line>)> = Vec::new();
    let order: [(&str, &str); 6] = match kind {
        BatchKind::Pull => [("Failed", "failed"), ("Skipped", "skipped"), ("Pulled", "pulled"), ("Nothing to pull", "up_to_date"), ("Stopped", "stopped"), ("", "fetched")],
        BatchKind::Fetch => [("Failed", "failed"), ("Skipped", "skipped"), ("Fetched", "fetched"), ("Stopped", "stopped"), ("", "pulled"), ("", "up_to_date")],
    };
    for (label, code) in order.into_iter().filter(|(label, _)| !label.is_empty()) {
        let mut lines = Vec::new();
        for (id, outcome) in result.outcomes.iter().filter(|(_, o)| o.code() == code) {
            let Some(repo) = library.repositories.iter().find(|r| r.id == *id) else {
                continue;
            };
            // The status read after the run: its Upstream pill says where it stands now.
            let now = match status.row(repo.id, &repo.path).upstream {
                Some(Cell::Pill(text, color)) if text.starts_with('↑') || text.starts_with('↓') => Some((text, color)),
                _ => None,
            };
            let line = match outcome {
                Outcome::Failed(why) => (why.clone(), RED(), None),
                Outcome::Skipped(why) if why == "Diverged" => (why.clone(), TEXT_SOFT(), now),
                Outcome::Skipped(why) => (why.clone(), TEXT_SOFT(), None),
                Outcome::Pulled { commits, head } => (
                    format!("now at {head}"),
                    FAINT(),
                    commits.map(|n| (format!("↓{n}"), GREEN())),
                ),
                Outcome::UpToDate => match now {
                    Some((text, _)) if text.starts_with('↑') => (format!("Up to date, {text} to push"), FAINT(), None),
                    _ => ("Up to date".into(), FAINT(), None),
                },
                Outcome::Fetched { changed: true } => ("Updated".into(), TEXT_SOFT(), now),
                Outcome::Fetched { changed: false } => ("Nothing new".into(), FAINT(), None),
                Outcome::Stopped => ("Stopped".into(), FAINT(), None),
            };
            lines.push((repo.name.clone(), line.0, line.1, line.2));
        }
        if !lines.is_empty() {
            groups.push((label, lines));
        }
    }
    let need_you = {
        let ids = result.need_you();
        library
            .repositories
            .iter()
            .filter(|r| ids.contains(&r.id) && !matches!(r.path.try_exists(), Ok(false)))
            .count()
    };
    let mut natural = 0.0;
    card(ui, false, |ui| {
        let image = if kind == BatchKind::Pull { Icon::Pull } else { Icon::Fetch };
        bar(ui, Some(image), kind.title(), &detail, |ui| {
            let r = crate::chrome::icon_button(ui, Icon::Close, true, false, 24.0).on_hover_text("Close");
            pages::control(p.widgets, "manager-result-close", &r, ui.clip_rect());
            p.asked.result_close |= r.clicked();
        });
        Frame::new().inner_margin(Margin { left: 14, right: 14, top: 10, bottom: 2 }).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                icon(ui.painter(), rect, tone.icon(), tone.color());
                ui.add(Label::new(RichText::new(&long).size(12.5).color(tone.color())).truncate());
            });
        });
        let output = ScrollArea::vertical()
            .id_salt("manager-result")
            .max_height((room - 40.0 - 34.0 - 46.0 - 2.0).max(48.0))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (label, lines) in &groups {
                    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
                    let caption = ui.painter().text(
                        pos2(rect.left() + 14.0, rect.center().y + 2.0),
                        Align2::LEFT_CENTER,
                        label.to_uppercase(),
                        semibold(11.0),
                        HINT(),
                    );
                    ui.painter().text(
                        pos2(caption.right() + 8.0, rect.center().y + 2.0),
                        Align2::LEFT_CENTER,
                        lines.len().to_string(),
                        FontId::proportional(12.0),
                        FAINT(),
                    );
                    for (name, what, color, pill) in lines {
                        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
                        let column = 132.0_f32.min(rect.width() * 0.4);
                        let mut job = text::LayoutJob::simple_singleline(name.clone(), semibold(13.0), TEXT());
                        job.wrap = text::TextWrapping::truncate_at_width(column - 8.0);
                        let galley = ui.painter().layout_job(job);
                        ui.painter().galley(pos2(rect.left() + 14.0, rect.center().y - galley.size().y / 2.0), galley, TEXT());
                        let mut x = rect.left() + 14.0 + column;
                        if let Some((text, tint)) = pill {
                            let cell = Rect::from_min_max(pos2(x, rect.top()), pos2(x + 60.0, rect.bottom()));
                            let galley = ui.painter().layout_no_wrap(text.clone(), semibold(12.5), *tint);
                            let pill = Rect::from_min_size(pos2(cell.left(), rect.center().y - 10.0), vec2(galley.size().x + 18.0, 20.0));
                            ui.painter().rect_filled(pill, 10.0, tint.gamma_multiply(0.14));
                            ui.painter().galley(pos2(pill.left() + 9.0, rect.center().y - galley.size().y / 2.0), galley, *tint);
                            x = pill.right() + 8.0;
                        }
                        let room = (rect.right() - 14.0 - x).max(0.0);
                        let font = FontId::proportional(12.5);
                        let cut = ui.painter().layout_no_wrap(what.clone(), font.clone(), *color).size().x > room;
                        let mut job = text::LayoutJob::simple_singleline(what.clone(), font, *color);
                        job.wrap = text::TextWrapping::truncate_at_width(room);
                        let galley = ui.painter().layout_job(job);
                        ui.painter().galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, *color);
                        // A reason cut short shows in full on hover.
                        if cut {
                            ui.interact(rect, ui.id().with(("manager-result-row", name.as_str())), Sense::hover())
                                .on_hover_text(what.as_str());
                        }
                    }
                }
                ui.add_space(8.0);
            });
        pages::region(p.widgets, "manager-result-scroll", output.inner_rect.intersect(ui.clip_rect()), true);
        natural = 40.0 + 34.0 + output.content_size.y + 46.0 + 2.0;
        card_foot(ui, None, |ui| {
            if need_you > 0 {
                let label = if need_you == 1 { "Open 1 in a tab".to_owned() } else { format!("Open {need_you} in tabs") };
                let r = icon_button(ui, Icon::Folder, &label, p.editable && !p.writing);
                pages::control(p.widgets, "manager-result-open", &r, ui.clip_rect());
                p.asked.result_open |= r.clicked();
            }
            pages::footer_status(ui, false, &took);
        });
    });
    natural
}
/// The selected repository or workspace: a header bar, its rows and the foot with Open, or a
/// placeholder while nothing is selected. After Fetch all or Pull all, its result instead.
/// `stacked`: under the list in a narrow window. Returns the card's natural height.
fn details(ui: &mut Ui, p: &mut Parts, room: f32, stacked: bool) -> f32 {
    if p.s.status.result.as_ref().is_some_and(|r| r.selection == p.s.selection) {
        return result_card(ui, p, room);
    }
    let library = p.library;
    let workspace = match p.s.selection {
        Selection::Workspace(id) => library.workspaces.iter().find(|w| w.id == id),
        _ => None,
    };
    let repository = if workspace.is_some() || matches!(p.s.selection, Selection::Workspace(_)) {
        None
    } else {
        p.s.selected.and_then(|id| library.repositories.iter().find(|r| r.id == id))
    };
    let editable = p.editable;
    let missing = repository.is_some_and(|r| {
        matches!(p.s.status.entry(r.id, &r.path).map(|e| &e.known), Some(crate::repository_status::Known::Missing))
    });
    let mut natural = 0.0;
    card(ui, false, |ui| {
        if let Some(w) = workspace {
            bar(ui, Some(Icon::Modules), &w.name, "Workspace", |_| {});
        } else if let Some(repo) = repository {
            let status = if missing {
                Some(("Folder missing", RED()))
            } else if p.tabs.iter().any(|t| same_path(&t.path, &repo.path)) {
                Some(("Open", pages::INFO()))
            } else {
                None
            };
            bar(ui, Some(Icon::Branch), &repo.name, "", |ui| {
                if let Some((text, color)) = status {
                    pages::pill(ui, text, color);
                }
            });
        } else if stacked {
            // Under the list in a narrow window, an empty card keeps to its bar.
            bar(ui, None, "", "No selection", |ui| {
                ui.label(RichText::new("Select a repository").size(12.0).color(FAINT()));
            });
            natural = 40.0 + 2.0;
            return;
        } else {
            bar(ui, None, "", "No selection", |_| {});
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 96.0), Sense::hover());
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                "Select a repository",
                FontId::proportional(13.0),
                FAINT(),
            );
            natural = 40.0 + 96.0 + 2.0;
            return;
        }
        // The rows scroll on their own so the foot and its buttons stay in view.
        let output = ScrollArea::vertical()
            .id_salt("manager-details")
            .max_height((room - 40.0 - 46.0 - 2.0).max(48.0))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.add_space(4.0);
                if let Some(w) = workspace {
                    workspace_rows(ui, p, w);
                } else if let Some(repo) = repository {
                    let folder = repo.folder.and_then(|id| library.folders.iter().find(|f| f.id == id));
                    let workspaces: Vec<&str> = library
                        .workspaces
                        .iter()
                        .filter(|w| w.repositories.contains(&repo.id))
                        .map(|w| w.name.as_str())
                        .collect();
                    // Where it stands first: branch, upstream, changes and last fetch,
                    // two to a line when the card is wide.
                    let (rows, note) = p.s.status.details(repo.id, &repo.path);
                    if ui.available_width() >= 520.0 && rows.len() > 1 {
                        for pair in rows.chunks(2) {
                            ui.columns(2, |columns| {
                                for (column, (label, value, color)) in columns.iter_mut().zip(pair) {
                                    detail(column, label, value, *color);
                                }
                            });
                        }
                    } else {
                        for (label, value, color) in &rows {
                            detail(ui, label, value, *color);
                        }
                    }
                    if !rows.is_empty() {
                        ui.add_space(4.0);
                        let top = ui.cursor().top();
                        ui.painter()
                            .hline(ui.max_rect().x_range(), top, Stroke::new(1.0, ROW_LINE()));
                        ui.add_space(4.0);
                    }
                    detail(ui, "Location", &crate::views::display_path(&repo.path), TEXT_SOFT());
                    detail(ui, "Folder", folder.map_or("Ungrouped", |f| f.name.as_str()), TEXT());
                    if workspaces.is_empty() {
                        detail(ui, "Workspaces", "None", MUTED());
                    } else {
                        detail(ui, "Workspaces", &workspaces.join(", "), TEXT());
                    }
                    // The status read's note, or why the last Open failed.
                    if let Some((color, text)) = note.or_else(|| p.s.unavailable.get(&repo.id).map(|e| (AMBER(), e.clone()))) {
                        Frame::new()
                            .inner_margin(Margin { left: 14, right: 14, top: 6, bottom: 4 })
                            .show(ui, |ui| pages::callout(ui, color, &text));
                    }
                }
                ui.add_space(6.0);
            });
        pages::region(p.widgets, "page-details-scroll", output.inner_rect.intersect(ui.clip_rect()), !p.busy);
        natural = 40.0 + output.content_size.y + 46.0 + 2.0;
        card_foot(ui, None, |ui| {
            if opening_foot(ui, p) {
                return;
            }
            let (open, label, edit, remove) = if let Some(w) = workspace {
                let ready = library
                    .workspace_members(w.id, p.s.include_nested)
                    .is_ok_and(|ids| !ids.is_empty());
                (ready, "Open workspace", FormKind::Workspace(Some(w.id)), FormKind::RemoveWorkspace(w.id))
            } else {
                let id = repository.map_or(0, |r| r.id);
                (!missing, "Open", FormKind::Repository(Some(id)), FormKind::RemoveRepository(id))
            };
            let r = pages::primary(ui, open && !p.busy, label);
            pages::control(p.widgets, "manager-open", &r, ui.clip_rect());
            p.asked.open |= r.clicked();
            let r = ui.add_enabled(editable, Button::new("Edit"));
            pages::control(p.widgets, "manager-edit", &r, ui.clip_rect());
            if r.clicked() {
                p.asked.form = Some(edit);
            }
            if let Some(w) = workspace {
                let r = pages::tool(ui, Icon::Plus, "New child", editable);
                pages::control(p.widgets, "manager-workspace-child", &r, ui.clip_rect());
                if r.clicked() {
                    p.asked.workspace = Some(Some(w.id));
                }
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let r = pages::danger_ghost(ui, editable, "Remove");
                pages::control(p.widgets, "manager-remove", &r, ui.clip_rect());
                if r.clicked() {
                    p.asked.form = Some(remove);
                }
                result_line(ui, p.s);
            });
        });
    });
    natural
}
/// A workspace's rows: parent, how many repositories it opens, its children, its active tab and
/// the Include nested workspaces switch.
fn workspace_rows(ui: &mut Ui, p: &mut Parts, w: &Workspace) {
    let library = p.library;
    let descendants = library
        .workspaces
        .iter()
        .filter(|o| o.id != w.id && library.workspace_contains(w.id, o.id))
        .count();
    let children: Vec<&str> = library
        .workspaces
        .iter()
        .filter(|o| o.parent == Some(w.id))
        .map(|o| o.name.as_str())
        .collect();
    let parent = w.parent.and_then(|id| library.workspace_path(id));
    let own = w.repositories.len();
    let repositories = if descendants == 0 {
        own.to_string()
    } else {
        match library.workspace_members(w.id, true) {
            Ok(ids) => format!("{own} here · {} with nested", ids.len()),
            Err(_) => format!("{own} here · more than {MAX_WORKSPACE_REPOSITORIES} with nested"),
        }
    };
    let members = library.workspace_members(w.id, p.s.include_nested);
    let active = w
        .active
        .filter(|a| w.repositories.contains(a))
        .or_else(|| members.as_ref().ok().and_then(|ids| ids.first().copied()))
        .and_then(|id| library.repositories.iter().find(|r| r.id == id));
    let rows = [
        ("Parent", parent.unwrap_or_else(|| "Top level".into()), TEXT()),
        ("Repositories", repositories, TEXT()),
        if children.is_empty() {
            ("Nested", "None".into(), MUTED())
        } else {
            ("Nested", children.join(", "), TEXT())
        },
        match active {
            Some(repo) => ("Active tab", repo.name.clone(), TEXT()),
            None => ("Active tab", "None".into(), MUTED()),
        },
    ];
    // Stacked under the list, the card is wide but short: two rows to a line keep the switch
    // in view.
    if ui.available_width() >= 520.0 {
        for pair in rows.chunks(2) {
            ui.columns(2, |columns| {
                for (column, (label, value, color)) in columns.iter_mut().zip(pair) {
                    detail(column, label, value, *color);
                }
            });
        }
    } else {
        for (label, value, color) in &rows {
            detail(ui, label, value, *color);
        }
    }
    if let Err(error) = &members {
        Frame::new()
            .inner_margin(Margin { left: 14, right: 14, top: 6, bottom: 4 })
            .show(ui, |ui| pages::callout(ui, AMBER(), error));
    }
    ui.add_space(4.0);
    let top = ui.cursor().top();
    ui.painter()
        .hline(ui.max_rect().x_range(), top, Stroke::new(1.0, ROW_LINE()));
    Frame::new().inner_margin(Margin::symmetric(14, 9)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new("Include nested workspaces").size(13.0));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let enabled = !p.busy && descendants > 0;
                let r = ui
                    .add_enabled_ui(enabled, |ui| switch(ui, &mut p.s.include_nested, "Include nested workspaces"))
                    .inner;
                pages::control(p.widgets, "manager-include-nested", &r, ui.clip_rect());
                // The list above already showed this frame's members.
                if r.changed() {
                    ui.ctx().request_repaint();
                }
            });
        });
    });
}
/// A form row: the label in a fixed column, then the field at full height.
fn labelled(ui: &mut Ui, label: &str, content: impl FnOnce(&mut Ui)) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        let (rect, _) = ui.allocate_exact_size(vec2(LABEL, CONTROL), Sense::hover());
        ui.painter()
            .text(rect.left_center(), Align2::LEFT_CENTER, label, FontId::proportional(13.0), MUTED());
        content(ui);
    });
}
impl Rocket {
    pub fn manager_open_paths(&self) -> Result<Vec<PathBuf>, &'static str> {
        if let Selection::Workspace(id) = self.manager.selection {
            self.library.workspace_members(id, self.manager.include_nested)?.iter()
                .map(|id| self.library.repositories.iter().find(|r| r.id == *id)
                    .map(|r| r.path.clone()).ok_or("A workspace repository no longer exists. Review its membership."))
                .collect()
        } else {
            Ok(self.manager
                .selected
                .and_then(|id| self.library.repositories.iter().find(|r| r.id == id))
                .map_or_else(Vec::new, |r| vec![r.path.clone()]))
        }
    }
    pub(crate) fn manager_form(&mut self, kind: FormKind) {
        self.begin_manager_form(kind, None);
    }
    fn begin_manager_form(&mut self, kind: FormKind, empty_parent: Option<Option<u64>>) {
        // None captures open tabs; Some(parent) starts an empty workspace group.
        if self.manager.running() || self.state_error.is_some() {
            return;
        }
        let mut f = Form {
            kind,
            name: String::new(),
            path: String::new(),
            folder: None,
            parent: empty_parent.flatten(),
            members: vec![],
            active: None,
            error: None,
            baseline: self.library.clone(),
            draft: self.library.clone(),
        };
        match kind {
            FormKind::Repository(Some(id)) | FormKind::RemoveRepository(id) => {
                if let Some(r) = self.library.repositories.iter().find(|r| r.id == id) {
                    f.name = r.name.clone();
                    f.path = crate::views::display_path(&r.path);
                    f.folder = r.folder;
                }
            }
            FormKind::Repository(None) => {
                f.folder = if let Selection::Folder(id) = self.manager.selection {
                    Some(id)
                } else {
                    None
                };
                if let Some(tab) = self.tab() {
                    f.path = crate::views::display_path(&tab.path);
                    f.name = tab.title.clone();
                }
            }
            FormKind::Folder(Some(id)) | FormKind::RemoveFolder(id) => {
                if let Some(folder) = self.library.folders.iter().find(|r| r.id == id) {
                    f.name = folder.name.clone();
                }
            }
            FormKind::Workspace(Some(id)) | FormKind::RemoveWorkspace(id) => {
                if let Some(w) = self.library.workspaces.iter().find(|w| w.id == id) {
                    f.name = w.name.clone();
                    f.members = w.repositories.clone();
                    f.active = w.active;
                    f.parent = w.parent;
                }
            }
            FormKind::Workspace(None) if empty_parent.is_none() => {
                for tab in &self.tabs {
                    if let Some(s) = &tab.snapshot {
                        let path = s
                            .info
                            .worktree
                            .clone()
                            .unwrap_or_else(|| s.info.git_dir.clone());
                        match f.draft.save_repository(path, tab.title.clone(), None) {
                            Ok(id) => {
                                if !f.members.contains(&id) {
                                    f.members.push(id);
                                }
                                if self.tab().map(|t| t.id) == Some(tab.id) {
                                    f.active = Some(id);
                                }
                            }
                            Err(e) => f.error = Some(e.into()),
                        }
                    }
                }
            }
            FormKind::Folder(None) | FormKind::Workspace(None) => {}
        }
        self.manager.form = Some(f);
        if let Some(d) = &mut self.dialog {
            d.flag = true;
        }
    }
    fn apply_manager_form(&mut self) {
        let Some(f) = self.manager.form.as_mut() else {
            return;
        };
        if f.baseline != self.library {
            f.error = Some("Saved selections changed. Close this form and review again.".into());
            return;
        }
        let mut draft = f.draft.clone();
        let name = f.name.trim().to_owned();
        let result: Result<Option<u64>, &str> = match f.kind {
            FormKind::Repository(None) => draft
                .save_repository(PathBuf::from(&f.path), name, f.folder)
                .map(Some),
            FormKind::Repository(Some(id)) => draft
                .edit_repository(id, PathBuf::from(&f.path), name, f.folder)
                .map(|_| Some(id)),
            FormKind::Folder(None) => draft.add_folder(name).map(Some),
            FormKind::Folder(Some(id)) => draft.rename_folder(id, name).map(|_| Some(id)),
            FormKind::Workspace(id) => draft
                .save_workspace_in(
                    id,
                    name,
                    f.members.clone(),
                    f.active
                        .filter(|id| f.members.contains(id))
                        .or_else(|| f.members.first().copied()),
                    f.parent,
                )
                .map(Some),
            FormKind::RemoveRepository(id) => draft.remove_repository(id).map(|_| None),
            FormKind::RemoveFolder(id) => draft.remove_folder(id).map(|_| None),
            FormKind::RemoveWorkspace(id) => draft.remove_workspace(id).map(|_| None),
        };
        match result.and_then(|id| draft.validate().map(|_| id)) {
            Ok(id) => {
                match f.kind {
                    FormKind::Repository(_) => {
                        self.manager.selected = id;
                        if let Some(id) = id {
                            self.manager.unavailable.remove(&id);
                        }
                    }
                    FormKind::Folder(_) => {
                        if let Some(id) = id {
                            // A new folder is empty: a repository chosen elsewhere is not in it.
                            if self.manager.selection != Selection::Folder(id) {
                                self.manager.selected = None;
                            }
                            self.manager.selection = Selection::Folder(id);
                        }
                    }
                    FormKind::Workspace(_) => {
                        if let Some(id) = id {
                            self.manager.selection = Selection::Workspace(id);
                            self.manager.include_nested = false;
                            self.manager.collapsed.retain(|parent| !draft.workspace_contains(*parent, id));
                        }
                    }
                    FormKind::RemoveRepository(id) => {
                        self.manager.selected = None;
                        self.manager.unavailable.remove(&id);
                    }
                    FormKind::RemoveFolder(_) | FormKind::RemoveWorkspace(_) => {
                        self.manager.selection = Selection::All;
                        self.manager.selected = None;
                        self.manager.include_nested = false;
                    }
                }
                self.manager.collapsed.retain(|id| draft.workspaces.iter().any(|w| w.id == *id));
                self.library = draft;
                // The change shows in the page; an earlier result no longer applies.
                self.manager.clear_result();
                self.manager.form = None;
                if let Some(d) = &mut self.dialog {
                    d.flag = false;
                }
            }
            Err(e) => f.error = Some(e.into()),
        }
    }
    fn submit_manager_form(&mut self) {
        if self.manager.running() || self.state_error.is_some() {
            return;
        }
        let Some(f) = self.manager.form.as_mut() else {
            return;
        };
        f.error = None;
        if let FormKind::Repository(id) = f.kind {
            let path = PathBuf::from(f.path.trim());
            // Label/folder changes to an unavailable shortcut must still be possible.
            let unchanged = id
                .and_then(|id| self.library.repositories.iter().find(|r| r.id == id))
                .is_some_and(|r| same_path(&r.path, &path));
            if !unchanged {
                let Some(backend) = self.backend.clone() else {
                    return;
                };
                match ReadTask::spawn(move |cancel| backend.library_repository(&path, &cancel)) {
                    Ok(task) => self.manager.check = Some(task),
                    Err(e) => f.error = Some(user_error(&e)),
                };
                return;
            }
        }
        self.apply_manager_form();
    }
    fn start_manager_open(&mut self) {
        if self.manager.running() || self.write.is_some() || self.quitting {
            return;
        }
        let paths = match self.manager_open_paths() {
            Ok(paths) => paths,
            Err(error) => { self.manager.error = Some(error.into()); return; }
        };
        if paths.is_empty() {
            self.manager.error = Some("Nothing to open.".into());
            return;
        }
        let active = if let Selection::Workspace(id) = self.manager.selection {
            self.library
                .workspaces
                .iter()
                .find(|w| w.id == id)
                .and_then(|w| w.active)
                .and_then(|id| self.library.repositories.iter().find(|r| r.id == id))
                .map(|r| r.path.clone())
                .or_else(|| paths.first().cloned())
        } else {
            paths.first().cloned()
        };
        let pending = paths
            .into_iter()
            .filter_map(|path| {
                self.library
                    .repositories
                    .iter()
                    .find(|r| same_path(&r.path, &path))
                    .map(|r| (r.id, path))
            })
            .collect();
        self.start_opening(pending, active);
    }
    /// Open these saved repositories one by one, then make `active` the active tab.
    fn start_opening(&mut self, pending: VecDeque<(u64, PathBuf)>, active: Option<PathBuf>) {
        if let Some(d) = &mut self.dialog {
            d.flag = true;
        }
        self.manager.opening = Some(Opening {
            pending,
            active,
            opened: 0,
            failed: 0,
        });
        self.manager.clear_result();
    }
    /// The result card's Open N in tabs: the repositories Fetch all or Pull all left for
    /// someone, opened like a workspace.
    fn open_result(&mut self) {
        if self.manager.running() || self.write.is_some() || self.quitting {
            return;
        }
        let pending: VecDeque<(u64, PathBuf)> = self.manager_result_paths().into();
        if pending.is_empty() {
            return;
        }
        let active = pending.front().map(|(_, path)| path.clone());
        self.manager.status.result = None;
        self.start_opening(pending, active);
    }
    pub(crate) fn poll_manager(&mut self, ctx: &Context) {
        if self.quitting {
            self.manager.cancel();
            self.manager.cancel_form();
            self.poll_repository_status(ctx);
            return;
        }
        self.poll_manager_scan();
        self.poll_repository_status(ctx);
        if let Some(result) = self.manager.check.as_mut().and_then(ReadTask::try_finish) {
            self.manager.check = None;
            if let Some(id) = self.manager.checking_id.take() {
                match result {
                    Ok(path) => {
                        self.manager.unavailable.remove(&id);
                        if let Some(o) = &mut self.manager.opening {
                            o.opened += 1;
                        }
                        self.open(path);
                    }
                    Err(e) => {
                        let error = check_error(&e);
                        let saved = self.library.repositories.iter().find(|r| r.id == id);
                        let name = saved.map_or("A saved repository", |r| r.name.as_str());
                        self.manager.error = Some(format!("Couldn't open {name}: {error}"));
                        // The row says so too: Folder missing, or Couldn't check.
                        if let Some(path) = saved.map(|r| r.path.clone()) {
                            self.manager.status.record_failure(id, &path, &error);
                        }
                        self.manager.unavailable.insert(id, error);
                        if let Some(o) = &mut self.manager.opening {
                            o.failed += 1;
                        }
                    }
                }
            } else if self.manager.form.is_some() {
                match result {
                    Ok(path) => {
                        self.manager.form.as_mut().unwrap().path =
                            path.to_string_lossy().into_owned();
                        self.apply_manager_form();
                    }
                    Err(e) => self.manager.form.as_mut().unwrap().error = Some(check_error(&e)),
                }
            }
        }
        if self.manager.opening.is_some()
            && self.manager.check.is_none()
            && self.write.is_none()
            && !self.tabs.iter().any(Tab::loading)
        {
            let next = self.manager.opening.as_mut().unwrap().pending.pop_front();
            if let Some((id, path)) = next {
                if let Some(index) = self
                    .tabs
                    .iter()
                    .position(|t| same_path(&t.path, &path) && t.snapshot.is_some())
                {
                    self.active = index;
                    self.manager.opening.as_mut().unwrap().opened += 1;
                } else if let Some(backend) = self.backend.clone() {
                    match ReadTask::spawn(move |cancel| backend.library_repository(&path, &cancel))
                    {
                        Ok(task) => {
                            self.manager.check = Some(task);
                            self.manager.checking_id = Some(id);
                        }
                        Err(e) => {
                            self.manager.opening.as_mut().unwrap().failed += 1;
                            self.manager.error = Some(user_error(&e));
                        }
                    }
                }
            } else {
                let o = self.manager.opening.take().unwrap();
                if let Some(d) = &mut self.dialog {
                    d.flag = false;
                }
                if let Some(path) = o.active
                    && let Some(index) = self.tabs.iter().position(|t| same_path(&t.path, &path))
                {
                    self.active = index;
                }
                self.manager.notice = Some(if o.failed == 0 {
                    format!("Opened {}", o.opened)
                } else {
                    format!("Opened {} · {} unavailable", o.opened, o.failed)
                });
                self.manager.outcome = if o.failed == 0 { Outcome::Opened } else { Outcome::Partial };
            }
        }
        if self.manager.running() {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
    }
    pub(crate) fn repository_manager_page(&mut self, ui: &mut Ui) {
        let busy = self.manager.running();
        let full = ui.available_rect_before_wrap();
        let nav = Rect::from_min_size(full.min, vec2(if full.width() < 1000.0 { 196.0 } else { 240.0 }, full.height()));
        let main = Rect::from_min_max(pos2(nav.right(), full.top()), full.max);
        let mut p = Parts {
            library: &self.library,
            tabs: &self.tabs,
            s: &mut self.manager,
            widgets: &mut self.frame_widgets,
            busy,
            editable: self.backend.is_some() && self.state_error.is_none() && !busy,
            writing: self.write.is_some(),
            asked: Asked::default(),
        };
        // The library's sidebar, in the app sidebar's place.
        ui.painter().rect_filled(nav, 0.0, PANE_PANEL());
        ui.painter()
            .vline(nav.right() - 0.5, nav.y_range(), Stroke::new(1.0, HAIRLINE()));
        let mut side = ui.new_child(
            UiBuilder::new()
                .id_salt("manager-sidebar")
                .max_rect(nav.shrink2(vec2(8.0, 0.0)))
                .layout(Layout::top_down(Align::Min)),
        );
        navigation(&mut side, &mut p);
        let mut body = ui.new_child(
            UiBuilder::new()
                .id_salt("manager-body")
                .max_rect(main)
                .layout(Layout::top_down(Align::Min)),
        );
        let library = p.library;
        let summary = format!(
            "{} · {} · {}",
            count(library.repositories.len(), "repository", "repositories"),
            count(library.folders.len(), "folder", "folders"),
            count(library.workspaces.len(), "workspace", "workspaces"),
        );
        let compact = main.width() < 1000.0;
        pages::header(&mut body, Icon::Database, "Repository manager", &summary, |ui| {
            let r = pages::ghost(ui, !busy, "Close");
            pages::control(p.widgets, "manager-close", &r, ui.clip_rect());
            p.asked.close = r.clicked();
            if !compact { add_actions(ui, &mut p); }
        });
        if compact {
            Frame::new().inner_margin(Margin {left:24,right:20,top:0,bottom:12})
                .show(&mut body, |ui| { ui.horizontal_wrapped(|ui| add_actions(ui, &mut p)); });
        }
        if let Some(error) = &self.state_error {
            Frame::new()
                .inner_margin(Margin { left: 24, right: 24, top: 0, bottom: 12 })
                .show(&mut body, |ui| pages::callout(ui, RED(), error));
        }
        let area = body.available_rect_before_wrap();
        let area = Rect::from_min_max(area.min + vec2(24.0, 0.0), area.max - vec2(24.0, 18.0));
        let shown = !library.repositories.is_empty() || matches!(p.s.selection, Selection::Workspace(_));
        // Beside the list when there is room; otherwise under it at its own height, which
        // last frame measured, so its foot and buttons stay in view.
        let (list, side_card) = if !shown {
            (area, None)
        } else if area.width() >= 700.0 {
            let width = (area.width() * 0.35).clamp(340.0, 400.0);
            (
                Rect::from_min_max(area.min, pos2(area.right() - width - 16.0, area.bottom())),
                Some(Rect::from_min_max(pos2(area.right() - width, area.top()), area.max)),
            )
        } else {
            let wanted = if p.s.details_height > 0.0 { p.s.details_height } else { 190.0 };
            let height = wanted.min(area.height() * 0.62).max(96.0);
            (
                Rect::from_min_max(area.min, pos2(area.right(), (area.bottom() - height - 12.0).max(area.top() + 96.0))),
                Some(Rect::from_min_max(pos2(area.left(), area.bottom() - height), area.max)),
            )
        };
        let mut list_ui = body.new_child(
            UiBuilder::new()
                .id_salt("manager-list")
                .max_rect(list)
                .layout(Layout::top_down(Align::Min)),
        );
        repository_list(&mut list_ui, &mut p);
        if let Some(rect) = side_card {
            let mut card_ui = body.new_child(
                UiBuilder::new()
                    .id_salt("manager-details")
                    .max_rect(rect)
                    .layout(Layout::top_down(Align::Min)),
            );
            let natural = details(&mut card_ui, &mut p, rect.height(), area.width() < 700.0);
            p.s.details_height = natural;
        }
        ui.advance_cursor_after_rect(full);
        let asked = p.asked;
        if asked.scan { self.manager_scan(); ui.ctx().request_repaint(); }
        // Applied after drawing: the next frame shows what they changed.
        if asked.form.is_some()
            || asked.workspace.is_some()
            || asked.stop
            || asked.open
            || asked.close
            || asked.check
            || asked.batch.is_some()
            || asked.result_open
            || asked.result_close
        {
            ui.ctx().request_repaint();
        }
        if let Some(kind) = asked.form {
            self.manager_form(kind);
        }
        if let Some(parent) = asked.workspace {
            self.begin_manager_form(FormKind::Workspace(None), Some(parent));
        }
        if asked.stop {
            if self.manager.status.batch.is_some() {
                // Stop skips what hasn't started; a running pull finishes first.
                self.stop_batch();
            } else {
                self.manager.cancel();
                if let Some(d) = &mut self.dialog {
                    d.flag = false;
                }
            }
        }
        if asked.open {
            self.start_manager_open();
        }
        if asked.check {
            let rows = self.manager_batch_rows();
            self.manager.status.check_again(rows.iter().map(|(id, path)| (*id, path.as_path())));
        }
        if let Some(kind) = asked.batch {
            self.start_batch(kind);
        }
        if asked.result_close {
            self.manager.status.result = None;
        }
        if asked.result_open {
            self.open_result();
        }
        if asked.close {
            self.dialog = None;
        }
    }
    pub(crate) fn repository_manager_dialog(&mut self, ctx: &Context) {
        if self.manager.scan.is_some() { self.manager_scan_dialog(ctx); return; }
        let busy = self.manager.check.is_some();
        let mut submit = false;
        let mut browse = false;
        let Some(f) = self.manager.form.as_mut() else {
            return;
        };
        let (image, title) = match f.kind {
            FormKind::Repository(None) => (Icon::Branch, "Add repository".to_owned()),
            FormKind::Repository(Some(_)) => (Icon::Branch, "Edit repository".to_owned()),
            FormKind::Folder(None) => (Icon::Folder, "New folder".to_owned()),
            FormKind::Folder(Some(_)) => (Icon::Folder, "Rename folder".to_owned()),
            FormKind::Workspace(None) => (Icon::Modules, "New workspace".to_owned()),
            FormKind::Workspace(Some(_)) => (Icon::Modules, "Edit workspace".to_owned()),
            FormKind::RemoveRepository(_) => (Icon::Trash, format!("Remove {} from Rocket", f.name)),
            FormKind::RemoveFolder(_) => (Icon::Trash, format!("Remove folder {}", f.name)),
            FormKind::RemoveWorkspace(_) => (Icon::Trash, format!("Remove workspace {}", f.name)),
        };
        let removal = f.kind.removal();
        let response = pages::review(
            ctx,
            "repository-manager-form",
            pages::ReviewHeader {
                image,
                title: &title,
                subtitle: "",
                danger: removal,
            },
            &mut self.frame_widgets,
            "manager-form-scroll",
            true,
            |ui, part, widgets| match part {
                Part::Body => {
                    ui.add_enabled_ui(!busy, |ui| match f.kind {
                        FormKind::RemoveRepository(_) | FormKind::RemoveFolder(_) | FormKind::RemoveWorkspace(_) => {
                            removal_rows(ui, f);
                        }
                        FormKind::Repository(_) => {
                            labelled(ui, "Location", |ui| {
                                let width = (ui.available_width() - 76.0 - ui.spacing().item_spacing.x).max(80.0);
                                let r = ui.add_sized(
                                    [width, CONTROL],
                                    TextEdit::singleline(&mut f.path)
                                        .id_salt("manager-path")
                                        .vertical_align(Align::Center),
                                );
                                pages::control(widgets, "manager-path", &r, ui.clip_rect());
                                let r = ui.add_sized([76.0, CONTROL], Button::new("Browse"));
                                pages::control(widgets, "manager-browse", &r, ui.clip_rect());
                                browse = r.clicked();
                            });
                            name_row(ui, widgets, &mut f.name);
                            labelled(ui, "Folder", |ui| {
                                let selected = f
                                    .folder
                                    .and_then(|id| f.draft.folders.iter().find(|x| x.id == id))
                                    .map_or_else(|| "Ungrouped".to_owned(), |x| x.name.clone());
                                let width = ui.available_width().min(320.0);
                                let layer = ui.layer_id();
                                let r = ComboBox::from_id_salt("manager-folder")
                                    .selected_text(selected)
                                    .truncate()
                                    .width(width)
                                    .show_ui(ui, |ui| {
                                        // Keep popup painting and pointer input above the modal review.
                                        ui.ctx().set_sublayer(layer, ui.layer_id());
                                        ui.set_max_width(width);
                                        let r = ui.selectable_value(&mut f.folder, None, "Ungrouped");
                                        if r.clicked() {
                                            ui.close();
                                        }
                                        pages::control(widgets, "manager-folder-choice-none", &r, ui.clip_rect());
                                        for folder in &f.draft.folders {
                                            let r = ui.selectable_value(&mut f.folder, Some(folder.id), &folder.name);
                                            if r.clicked() {
                                                ui.close();
                                            }
                                            pages::control(widgets, format!("manager-folder-choice-{}", folder.id), &r, ui.clip_rect());
                                        }
                                    })
                                    .response;
                                pages::control(widgets, "manager-folder", &r, ui.clip_rect());
                            });
                        }
                        FormKind::Folder(_) => name_row(ui, widgets, &mut f.name),
                        FormKind::Workspace(current) => {
                            name_row(ui, widgets, &mut f.name);
                            labelled(ui, "Parent", |ui| {
                                let parent = f.parent.and_then(|id| f.draft.workspace_path(id)).unwrap_or_else(|| "Top level".into());
                                let parent_width = ui.available_width().min(320.0);
                                let dialog_layer = ui.layer_id();
                                let r = ComboBox::from_id_salt("manager-parent").selected_text(parent).truncate().width(parent_width).show_ui(ui, |ui| {
                                    // Keep popup painting and pointer input above the modal review.
                                    ui.ctx().set_sublayer(dialog_layer, ui.layer_id());
                                    ui.set_max_width(parent_width);
                                    let r = ui.selectable_value(&mut f.parent, None, "Top level");
                                    if r.clicked() { ui.close(); }
                                    pages::control(widgets, "manager-parent-none", &r, ui.clip_rect());
                                    for (id, _) in f.draft.workspace_tree() {
                                        if current.is_some_and(|current| f.draft.workspace_contains(current, id)) { continue; }
                                        if let Some(name) = f.draft.workspace_path(id) {
                                            let r = ui.add_sized(vec2(parent_width, 24.0), Button::selectable(f.parent == Some(id), &name).truncate()).on_hover_text(&name);
                                            if r.clicked() { f.parent = Some(id); ui.close(); }
                                            pages::control(widgets, format!("manager-parent-{id}"), &r, ui.clip_rect());
                                        }
                                    }
                                    pages::region(widgets, "manager-parent-options", ui.min_rect().intersect(ui.clip_rect()), ui.is_enabled());
                                }).response;
                                pages::control(widgets, "manager-parent", &r, ui.clip_rect());
                            });
                            labelled(ui, "Repositories", |ui| member_list(ui, widgets, f));
                        }
                    });
                    if let Some(error) = &f.error {
                        pages::callout(ui, RED(), error);
                    }
                }
                Part::Footer => {
                    let label = match f.kind {
                        FormKind::Repository(None) => "Add",
                        FormKind::Folder(None) | FormKind::Workspace(None) => "Create",
                        kind if kind.removal() => "Remove",
                        _ => "Save",
                    };
                    let r = if removal {
                        pages::danger(ui, !busy, label)
                    } else {
                        pages::primary(ui, !busy, label)
                    };
                    pages::control(widgets, "manager-save", &r, ui.clip_rect());
                    submit = r.clicked();
                    if busy {
                        pages::footer_status(ui, true, "Checking the folder");
                    }
                }
            },
        );
        // Automation's old id for the footer's Cancel.
        if let Some(close) = self.frame_widgets.get("dialog-close").copied() {
            self.frame_widgets.insert("manager-cancel".into(), close);
        }
        if response.should_close() {
            self.manager.cancel_form();
            if let Some(d) = &mut self.dialog {
                d.flag = false;
            }
        } else if submit {
            self.submit_manager_form();
        } else if browse && let Some(f) = &self.manager.form {
            let current = f.path.clone();
            self.browse(PickTarget::LibraryRepository, &current);
        }
    }
    /// Browse's choice: the repository's folder, and its name while the name was still the old
    /// folder's (or empty).
    pub(crate) fn library_repository_picked(&mut self, path: &Path) {
        let Some(f) = self.manager.form.as_mut() else {
            return;
        };
        let previous = Path::new(f.path.trim())
            .file_name()
            .map(|n| n.to_string_lossy().into_owned());
        if (f.name.trim().is_empty() || previous.as_deref() == Some(f.name.as_str()))
            && let Some(name) = path.file_name()
        {
            f.name = name.to_string_lossy().into_owned();
        }
        f.path = crate::views::display_path(path);
        f.error = None;
    }
}
/// The Name row of every form that names something.
fn name_row(ui: &mut Ui, widgets: &mut HashMap<String, UiControl>, name: &mut String) {
    labelled(ui, "Name", |ui| {
        let r = ui.add_sized(
            [ui.available_width(), CONTROL],
            TextEdit::singleline(name)
                .id_salt("manager-name")
                .vertical_align(Align::Center),
        );
        pages::control(widgets, "manager-name", &r, ui.clip_rect());
    });
}
/// A workspace form's repositories: a box with a tick per repository and, for the ticked ones,
/// a circle in the Active tab column.
fn member_list(ui: &mut Ui, widgets: &mut HashMap<String, UiControl>, f: &mut Form) {
    ui.vertical(|ui| {
        ui.set_width(ui.available_width());
        Frame::new()
            .fill(PANEL())
            .stroke(Stroke::new(1.0, CARD_STROKE()))
            .corner_radius(8)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 0.0;
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
                ui.painter()
                    .rect_filled(rect, CornerRadius { nw: 7, ne: 7, sw: 0, se: 0 }, CARD());
                ui.painter()
                    .hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, CARD_LINE()));
                ui.painter().text(
                    pos2(rect.left() + 12.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    format!("{} of {}", f.members.len(), f.draft.repositories.len()),
                    FontId::proportional(12.0),
                    HINT(),
                );
                ui.painter().text(
                    pos2(rect.right() - ACTIVE / 2.0, rect.center().y),
                    Align2::CENTER_CENTER,
                    "Active tab",
                    FontId::proportional(12.0),
                    HINT(),
                );
                let active = f
                    .active
                    .filter(|a| f.members.contains(a))
                    .or_else(|| f.members.first().copied());
                let last = f.draft.repositories.len().saturating_sub(1);
                for (index, repo) in f.draft.repositories.iter().enumerate() {
                    let width = ui.available_width();
                    let row = ui
                        .allocate_ui_with_layout(vec2(width, 34.0), Layout::left_to_right(Align::Center), |ui| {
                            ui.set_min_size(vec2(width, 34.0));
                            ui.add_space(10.0);
                            let mut checked = f.members.contains(&repo.id);
                            let r = ui.add(checkbox(&mut checked, repo.name.as_str()));
                            pages::control(widgets, format!("manager-member-{}", repo.id), &r, ui.clip_rect());
                            if r.changed() {
                                if checked {
                                    f.members.push(repo.id);
                                } else {
                                    f.members.retain(|id| *id != repo.id);
                                    if f.active == Some(repo.id) {
                                        f.active = f.members.first().copied();
                                    }
                                }
                            }
                            // The path fills the space before the Active tab column.
                            let room = (ui.available_width() - ACTIVE).max(0.0);
                            ui.allocate_ui_with_layout(vec2(room, 34.0), Layout::left_to_right(Align::Center), |ui| {
                                ui.set_min_width(room);
                                ui.add(
                                    Label::new(
                                        RichText::new(crate::views::display_path(&repo.path))
                                            .size(11.5)
                                            .color(FAINT()),
                                    )
                                    .truncate(),
                                );
                            });
                            ui.allocate_ui_with_layout(
                                vec2(ACTIVE, 34.0),
                                Layout::left_to_right(Align::Center).with_main_align(Align::Center),
                                |ui| {
                                    if checked {
                                        let r = ui.add(RadioButton::new(active == Some(repo.id), ""));
                                        pages::control(widgets, format!("manager-active-{}", repo.id), &r, ui.clip_rect());
                                        if r.clicked() {
                                            f.active = Some(repo.id);
                                        }
                                    }
                                },
                            );
                        })
                        .response
                        .rect;
                    if index != last {
                        ui.painter()
                            .hline(row.x_range(), row.bottom() - 0.5, Stroke::new(1.0, ROW_LINE()));
                    }
                }
                if f.draft.repositories.is_empty() {
                    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
                    ui.painter().text(
                        pos2(rect.left() + 12.0, rect.center().y),
                        Align2::LEFT_CENTER,
                        "No saved repositories",
                        FontId::proportional(12.5),
                        FAINT(),
                    );
                }
            });
    });
}
/// What a removal takes away and what it leaves, as labelled rows in a well.
fn removal_rows(ui: &mut Ui, f: &Form) {
    let draft = &f.draft;
    let rows: Vec<(&str, String)> = match f.kind {
        FormKind::RemoveRepository(id) => {
            let workspaces: Vec<&str> = draft
                .workspaces
                .iter()
                .filter(|w| w.repositories.contains(&id))
                .map(|w| w.name.as_str())
                .collect();
            vec![
                ("Removed", format!("{} from the library", f.name)),
                (
                    "Workspaces",
                    if workspaces.is_empty() { "None".into() } else { format!("Leaves {}", workspaces.join(", ")) },
                ),
                ("Location", f.path.clone()),
                ("Files and tabs", "Unchanged".into()),
            ]
        }
        FormKind::RemoveFolder(id) => {
            let n = draft.repositories.iter().filter(|r| r.folder == Some(id)).count();
            vec![
                ("Removed", format!("The folder {}", f.name)),
                (
                    "Repositories",
                    if n == 0 { "None in it".into() } else { format!("{} stay saved, Ungrouped", count(n, "repository", "repositories")) },
                ),
                ("Workspaces", "Unchanged".into()),
                ("Files and tabs", "Unchanged".into()),
            ]
        }
        FormKind::RemoveWorkspace(id) => {
            let children: Vec<&str> = draft
                .workspaces
                .iter()
                .filter(|w| w.parent == Some(id))
                .map(|w| w.name.as_str())
                .collect();
            let parent = f
                .parent
                .and_then(|id| draft.workspace_path(id))
                .unwrap_or_else(|| "the top level".into());
            let moved = match children.len() {
                0 => "None".to_owned(),
                1 => format!("{} moves to {parent}", children[0]),
                _ => format!("{} move to {parent}", children.join(", ")),
            };
            vec![
                ("Removed", format!("The workspace {}", f.name)),
                ("Child workspaces", moved),
                ("Repositories", "Stay saved".into()),
                ("Files and tabs", "Unchanged".into()),
            ]
        }
        _ => Vec::new(),
    };
    pages::well(ui, |ui| {
        for (label, value) in rows {
            pages::key_value(ui, label, 120.0, |ui| {
                ui.add(Label::new(RichText::new(value).size(13.0)).wrap());
            });
        }
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_common as common;
    use std::{
        thread,
        time::{Duration, Instant},
    };

    #[test]
    fn counts_and_check_errors_read_plainly() {
        assert_eq!(count(1, "repository", "repositories"), "1 repository");
        assert_eq!(count(3, "repository", "repositories"), "3 repositories");
        assert_eq!(count(0, "folder", "folders"), "0 folders");
        let git = |code| rocket_git::Error::Git { operation: "discovery", code, stdout: vec![], stderr: vec![] };
        assert_eq!(check_error(&git(Some(128))), "No Git repository at this location (Git discovery, exit 128).");
        assert_eq!(check_error(&git(Some(1))), user_error(&git(Some(1))));
        let missing = rocket_git::Error::Io {
            action: "resolve repository directory",
            source: std::io::Error::from(std::io::ErrorKind::NotFound),
        };
        assert_eq!(check_error(&missing), "This location doesn't exist or can't be read.");
    }

    // The details card's foot (Open, Edit, Remove) stays inside the window: beside the list when
    // it is wide, under it at its own height when it is narrow. The forms keep their fields in view
    // and Browse's choice fills Location, and Name while it was the old folder's.
    #[test]
    fn details_foot_stays_in_view_and_browse_fills_the_form() {
        common::isolated(
            "ROCKET_MANAGER_UI_CASE",
            "repository_manager::tests::details_foot_stays_in_view_and_browse_fills_the_form",
            || {
                let fixture = common::Fixture::new();
                fixture.write("file", "first");
                fixture.commit("first");
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(
                    &ctx,
                    vec![fixture.repository.clone()],
                    fixture.root.join("logs"),
                );
                let deadline = Instant::now() + Duration::from_secs(40);
                while app.busy() {
                    app.poll(&ctx);
                    assert!(Instant::now() < deadline);
                    thread::sleep(Duration::from_millis(10));
                }
                app.auto_refresh = false;
                let mut library = Library::default();
                let work = library.add_folder("Client work".into()).unwrap();
                let names = ["rocket-app", "api-server", "design-tokens", "docs-site", "検索 service", "legacy-billing", "notes"];
                let ids: Vec<u64> = names
                    .iter()
                    .enumerate()
                    .map(|(n, name)| {
                        let folder = (n == 1 || n == 2).then_some(work);
                        library.save_repository(fixture.root.join(format!("saved-{n}")), (*name).into(), folder).unwrap()
                    })
                    .collect();
                let acme = library.save_workspace_in(None, "Acme".into(), vec![ids[1], ids[2]], Some(ids[1]), None).unwrap();
                library.save_workspace_in(None, "Backend".into(), vec![ids[1], ids[4]], None, Some(acme)).unwrap();
                app.library = library;
                app.dispatch(Intent::Dialog(DialogKind::RepositoryManager));
                let input = |app: &mut Rocket, size: Vec2, events: Vec<Event>| {
                    app.frame_widgets.clear();
                    let mut output = ctx.run_ui(
                        RawInput {
                            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                            events,
                            ..Default::default()
                        },
                        |ui| {
                            app.shell(ui, &mut Vec::new());
                            let ctx = ui.ctx().clone();
                            app.repository_manager_dialog(&ctx);
                        },
                    );
                    output.textures_delta.clear();
                };
                let frame = |app: &mut Rocket, size: Vec2| input(app, size, Vec::new());
                let inside = |app: &Rocket, id: &str, size: Vec2| {
                    let rect = app.frame_widgets.get(id).unwrap_or_else(|| panic!("{id} at {size:?}")).rect;
                    assert!(
                        rect.min.x >= 0.0 && rect.min.y >= 0.0 && rect.max.x <= size.x && rect.max.y <= size.y,
                        "{id} leaves the window: {rect:?} at {size:?}"
                    );
                    assert!(rect.height() >= 20.0, "{id} is cut: {rect:?} at {size:?}");
                    rect
                };
                for size in [vec2(1440.0, 900.0), vec2(780.0, 620.0)] {
                    for selection in [Selection::All, Selection::Workspace(acme)] {
                        app.manager.selection = selection;
                        app.manager.selected = Some(ids[0]);
                        for _ in 0..3 {
                            frame(&mut app, size);
                        }
                        for id in ["manager-edit", "manager-remove", "manager-add", "manager-workspace-save", "manager-close", "manager-folder-new", "manager-workspace-new"] {
                            inside(&app, id, size);
                        }
                        let open = inside(&app, "manager-open", size);
                        let list = app.frame_widgets["manager-repositories"].rect;
                        if size.x < 1000.0 {
                            assert!(list.bottom() <= open.top(), "the list stops above the details: {list:?} {open:?}");
                        } else {
                            assert!(list.right() <= open.left(), "the details sit beside the list: {list:?} {open:?}");
                        }
                        if selection == Selection::All {
                            inside(&app, &format!("manager-repo-{}", ids[0]), size);
                        } else {
                            inside(&app, "manager-include-nested", size);
                            inside(&app, "manager-workspace-child", size);
                            assert!(!app.frame_widgets.contains_key(&format!("manager-repo-{}", ids[1])), "a workspace lists what it opens");
                        }
                    }
                }
                // Typing into the filter lists only the matches in that same frame: the filter in
                // the card's bar is drawn before the rows.
                let wide = vec2(1440.0, 900.0);
                app.manager.selection = Selection::All;
                for _ in 0..3 {
                    frame(&mut app, wide);
                }
                let at = app.frame_widgets["manager-filter"].rect.center();
                let button = |pressed| Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                };
                input(&mut app, wide, vec![Event::PointerMoved(at), button(true)]);
                input(&mut app, wide, vec![button(false)]);
                input(&mut app, wide, vec![Event::Text("検索".into())]);
                assert_eq!(app.manager.filter, "検索");
                assert!(app.frame_widgets.contains_key(&format!("manager-repo-{}", ids[4])));
                assert!(
                    !app.frame_widgets.contains_key(&format!("manager-repo-{}", ids[0])),
                    "the frame that took the typing lists only matches"
                );
                app.manager.filter.clear();
                // A new folder becomes the selection with nothing selected in it; renaming keeps
                // the selected repository.
                app.manager.selected = Some(ids[0]);
                app.manager_form(FormKind::Folder(None));
                app.manager.form.as_mut().unwrap().name = "Archive".into();
                app.submit_manager_form();
                let archive = app.library.folders.iter().find(|f| f.name == "Archive").unwrap().id;
                assert_eq!((app.manager.selection, app.manager.selected), (Selection::Folder(archive), None));
                app.manager.selected = Some(ids[0]);
                app.manager_form(FormKind::Folder(Some(archive)));
                app.manager.form.as_mut().unwrap().name = "Old work".into();
                app.submit_manager_form();
                assert_eq!((app.manager.selection, app.manager.selected), (Selection::Folder(archive), Some(ids[0])));
                let narrow = vec2(780.0, 620.0);
                app.manager.selection = Selection::Workspace(acme);
                app.manager_form(FormKind::Workspace(Some(acme)));
                for _ in 0..3 {
                    frame(&mut app, narrow);
                }
                for id in ["manager-name", "manager-parent", "manager-save", "manager-cancel"] {
                    inside(&app, id, narrow);
                }
                assert!(app.frame_widgets.contains_key(&format!("manager-active-{}", ids[1])), "a ticked repository can be the active tab");
                assert!(!app.frame_widgets.contains_key(&format!("manager-active-{}", ids[0])), "an unticked one cannot");
                app.manager.cancel_form();
                app.manager_form(FormKind::Repository(None));
                for _ in 0..3 {
                    frame(&mut app, narrow);
                }
                for id in ["manager-path", "manager-browse", "manager-name", "manager-folder", "manager-save"] {
                    inside(&app, id, narrow);
                }
                let form = app.manager.form.as_mut().unwrap();
                form.path = fixture.root.join("old-folder").display().to_string();
                form.name = "old-folder".into();
                let picked = fixture.root.join("picked-repo");
                app.library_repository_picked(&picked);
                let form = app.manager.form.as_ref().unwrap();
                assert_eq!((form.name.as_str(), form.path.clone()), ("picked-repo", crate::views::display_path(&picked)));
                app.manager.form.as_mut().unwrap().name = "My name".into();
                app.library_repository_picked(&fixture.root.join("another"));
                assert_eq!(app.manager.form.as_ref().unwrap().name, "My name", "a typed name stays");
            },
        );
    }

    // Opening the page reads each listed repository in the background and its row says where it
    // stands; Needs attention lists only the ones that need it; the strip and a result card keep
    // their controls in the window at both sizes; Fetch all skips what it can't fetch.
    #[test]
    fn status_rows_strip_and_results_stay_in_view() {
        use crate::repository_status::{BatchResult, Outcome};
        common::isolated(
            "ROCKET_MANAGER_STATUS_UI_CASE",
            "repository_manager::tests::status_rows_strip_and_results_stay_in_view",
            || {
                let fixture = common::Fixture::new();
                fixture.write("file", "first");
                fixture.commit("first");
                // A clone of it with an edit it hasn't committed, and a folder that is gone.
                let edited = fixture.root.join("edited");
                fixture.run(&["clone", "--quiet", fixture.repository.to_str().unwrap(), edited.to_str().unwrap()]);
                std::fs::write(edited.join("file"), "changed").unwrap();
                let gone = fixture.root.join("gone");
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(&ctx, vec![fixture.repository.clone()], fixture.root.join("logs"));
                let deadline = Instant::now() + Duration::from_secs(40);
                while app.busy() {
                    app.poll(&ctx);
                    assert!(Instant::now() < deadline);
                    thread::sleep(Duration::from_millis(10));
                }
                app.auto_refresh = false;
                let mut library = Library::default();
                let ids = [
                    library.save_repository(fixture.repository.clone(), "clean".into(), None).unwrap(),
                    library.save_repository(edited.clone(), "edited".into(), None).unwrap(),
                    library.save_repository(gone.clone(), "gone".into(), None).unwrap(),
                ];
                let paths = [fixture.repository.clone(), edited.clone(), gone.clone()];
                app.library = library;
                app.dispatch(Intent::Dialog(DialogKind::RepositoryManager));
                let frame = |app: &mut Rocket, size: Vec2| {
                    app.frame_widgets.clear();
                    let mut output = ctx.run_ui(
                        RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), ..Default::default() },
                        |ui| app.shell(ui, &mut Vec::new()),
                    );
                    output.textures_delta.clear();
                };
                let inside = |app: &Rocket, id: &str, size: Vec2| {
                    let rect = app.frame_widgets.get(id).unwrap_or_else(|| panic!("{id} at {size:?}")).rect;
                    assert!(
                        rect.min.x >= 0.0 && rect.min.y >= 0.0 && rect.max.x <= size.x && rect.max.y <= size.y,
                        "{id} leaves the window: {rect:?} at {size:?}"
                    );
                };
                let wide = vec2(1440.0, 900.0);
                let settle = |app: &mut Rocket, done: &dyn Fn(&Rocket) -> bool| {
                    let deadline = Instant::now() + Duration::from_secs(40);
                    loop {
                        frame(app, wide);
                        app.poll(&ctx);
                        if done(app) {
                            break;
                        }
                        assert!(Instant::now() < deadline, "the manager's background work");
                        thread::sleep(Duration::from_millis(20));
                    }
                };
                settle(&mut app, &|app| {
                    !app.manager.status.checking() && (0..3).all(|n| app.manager.status.entry(ids[n], &paths[n]).is_some())
                });
                let upstream = |app: &Rocket, n: usize| app.manager.status.row(ids[n], &paths[n]).upstream.unwrap();
                assert_eq!(upstream(&app, 0), Cell::Pill("No upstream".into(), pages::NEUTRAL()));
                assert_eq!(upstream(&app, 1), Cell::Quiet("Up to date".into()));
                assert_eq!(app.manager.status.row(ids[1], &paths[1]).changes, Some(Cell::Pill("1 change".into(), AMBER())));
                assert_eq!(upstream(&app, 2), Cell::Pill("Folder missing".into(), RED()));
                assert!(app.manager.status.checked.is_some());
                // Needs attention: the edited clone and the missing folder only.
                app.manager.status.attention_only = true;
                frame(&mut app, wide);
                let listed = |app: &Rocket| ids.map(|id| app.frame_widgets.contains_key(&format!("manager-repo-{id}")));
                assert_eq!(listed(&app), [false, true, true]);
                app.manager.status.attention_only = false;
                for size in [wide, vec2(780.0, 620.0)] {
                    for _ in 0..3 {
                        frame(&mut app, size);
                    }
                    for id in ["manager-chip-all", "manager-chip-attention", "manager-check", "manager-fetch-all", "manager-pull-all"] {
                        inside(&app, id, size);
                    }
                    inside(&app, &format!("manager-repo-{}", ids[1]), size);
                }
                // Fetch all: the clone fetches from its origin; nothing to fetch for the others.
                app.start_batch(crate::repository_status::BatchKind::Fetch);
                assert!(app.manager.running(), "a run keeps the page busy");
                settle(&mut app, &|app| app.manager.status.result.is_some() && !app.manager.status.checking());
                let result = app.manager.status.result.as_ref().unwrap();
                assert!(!app.manager.running());
                assert_eq!(
                    result.outcomes,
                    [
                        (ids[0], Outcome::Skipped("No remotes".into())),
                        (ids[1], Outcome::Fetched { changed: false }),
                        (ids[2], Outcome::Skipped("Folder missing".into())),
                    ]
                );
                // The result card takes the details card's place, with Open 2 in tabs in its foot.
                app.manager.status.result = Some(BatchResult {
                    kind: crate::repository_status::BatchKind::Pull,
                    selection: Selection::All,
                    scope: "All saved".into(),
                    total: 3,
                    outcomes: vec![
                        (ids[0], Outcome::Skipped("No upstream".into())),
                        (ids[1], Outcome::Skipped("1 change uncommitted".into())),
                        (ids[2], Outcome::Skipped("Folder missing".into())),
                    ],
                    took: Duration::from_secs(2),
                    stopped: false,
                });
                assert_eq!(app.manager_result_paths().len(), 2, "a missing folder isn't opened");
                for size in [wide, vec2(780.0, 620.0)] {
                    for _ in 0..3 {
                        frame(&mut app, size);
                    }
                    inside(&app, "manager-result-close", size);
                    inside(&app, "manager-result-open", size);
                }
                // Picking a row closes the result.
                app.manager.pick(ids[1]);
                assert!(app.manager.status.result.is_none());
                // Exit names a running run, stops it as it stops any write, and stays open
                // until its Git has reported back.
                app.start_batch(crate::repository_status::BatchKind::Fetch);
                assert!(app.quit_rows().iter().any(|(_, _, text)| text == "Fetch all is still running. Exiting stops it first."));
                app.begin_quit();
                assert!(app.working(), "Rocket stays open while the run reports back");
                let deadline = Instant::now() + Duration::from_secs(40);
                while app.manager.status.batch.is_some() {
                    app.poll(&ctx);
                    assert!(Instant::now() < deadline, "the stopped run reports back");
                    thread::sleep(Duration::from_millis(10));
                }
                assert!(app.manager.status.result.as_ref().is_some_and(|r| r.stopped));
            },
        );
    }
}
