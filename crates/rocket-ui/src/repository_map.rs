//! Per-repository immutable map reads and presentation. A dropped/superseded
//! receiver cannot publish a stale result, and Stop wins even if completion races.
use crate::{
    app::*,
    backend::{Backend, user_error},
    folder_browser::PickTarget,
    pages,
    statistics::Period,
    style::*,
};
use eframe::egui::*;
use crate::i18n::{tr, trf, trn};
use rocket_git::*;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

mod details;
mod drawing;
mod export;
mod lists;
mod navigation;
#[cfg(test)]
mod tests;
const DISPLAY_NODES: usize = 240;
/// The details card beside the map.
const DETAILS_WIDTH: f32 = 300.;

pub enum MapIntent {
    Open,
    Load,
    More,
    Export,
    SaveExport,
    BrowseExport,
    Cancel,
    Switch(MapKind),
    History(String),
    People(String),
    /// Code → Folders, opened at this path.
    Code(Vec<u8>),
    /// History of commits touching this literal path, searched for the text.
    Path(Vec<u8>, String),
    InspectCommit(ObjectId),
}
#[derive(Default)]
pub struct State {
    pub kind: MapKind,
    pub pages: [MapPage; 3],
}
pub struct MapPage {
    pub revision: String,
    pub query: String,
    pub all_branches: bool,
    /// People: only commits from this period.
    pub period: Period,
    pub base: String,
    pub path_scope: String,
    pub show_scope: bool,
    pub collapsed: HashSet<Vec<u8>>,
    pub neighbor_anchor: Option<Vec<u8>>,
    pub projected_links: Vec<MapLink>,
    pub export: Option<export::Export>,
    /// Code: packages instead of folders. People: shared files instead of the grid.
    pub alternate: bool,
    /// History: branches and merges with runs of commits folded, instead of every commit.
    pub condensed: bool,
    pub data: Option<Arc<RepositoryMap>>,
    pub selected: Option<usize>,
    /// People grid: the selected person and folder.
    pub cell: Option<(usize, usize)>,
    /// Condensed history: the selected line.
    pub line: Option<usize>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub show_details: bool,
    pub cancelling: bool,
    pub loaded_refresh: u64,
    pub visible: Vec<usize>,
    pub display_offset: usize,
    pub view_total: usize,
    pub positions: Vec<Pos2>,
    /// Folders: where each folder's own files are summed up, by folder node.
    pub file_groups: Vec<FileGroup>,
    pub zoom: f32,
    pub pan: Vec2,
    pub fit: bool,
    pub focus: bool,
    pub stale: bool,
    task: Option<ReadTask<RepositoryMap>>,
    load_refresh: u64,
    filter_key: Option<(String, bool)>,
    last_canvas: Vec2,
    loaded_inputs: Option<(String, String, String, bool, Period)>,
    pending_inputs: Option<(String, String, String, bool, Period)>,
    loading_more: bool,
    /// Code → Folders opens at this path once its data is read.
    pending_path: Option<Vec<u8>>,
    topology: Option<Arc<lists::Topology>>,
}
impl Default for MapPage {
    fn default() -> Self {
        Self {
            revision: "HEAD".into(),
            query: String::new(),
            all_branches: false,
            period: Period::All,
            base: String::new(),
            path_scope: String::new(),
            show_scope: false,
            collapsed: HashSet::new(),
            neighbor_anchor: None,
            projected_links: Vec::new(),
            export: None,
            alternate: false,
            condensed: true,
            data: None,
            selected: None,
            cell: None,
            line: None,
            error: None,
            notice: None,
            show_details: false,
            cancelling: false,
            loaded_refresh: 0,
            visible: Vec::new(),
            display_offset: 0,
            view_total: 0,
            positions: Vec::new(),
            file_groups: Vec::new(),
            zoom: 1.,
            pan: Vec2::ZERO,
            fit: true,
            focus: false,
            stale: false,
            task: None,
            load_refresh: 0,
            filter_key: None,
            last_canvas: Vec2::ZERO,
            loaded_inputs: None,
            pending_inputs: None,
            loading_more: false,
            pending_path: None,
            topology: None,
        }
    }
}
/// A folder's own files summed up on one card beside its subfolders.
pub struct FileGroup {
    /// None for the files at the top of the repository.
    pub folder: Option<usize>,
    pub files: usize,
    /// Raw file names when there are a few; the counted folder caption is drawn live.
    pub about: String,
    /// Where the card sits at 100% zoom.
    pub at: Pos2,
}
fn index(kind: MapKind) -> usize {
    match kind {
        MapKind::Code => 0,
        MapKind::History => 1,
        MapKind::People => 2,
    }
}
impl State {
    pub fn page(&self) -> &MapPage {
        &self.pages[index(self.kind)]
    }
    fn page_mut(&mut self) -> &mut MapPage {
        &mut self.pages[index(self.kind)]
    }
    pub fn running(&self) -> bool {
        self.pages
            .iter()
            .any(|p| p.running() || p.export.as_ref().is_some_and(|e| e.running()))
    }
    pub fn stop(&mut self) {
        for p in &mut self.pages {
            p.stop();
        }
    }
}
impl MapPage {
    fn finish(&mut self, result: Result<RepositoryMap>) {
        self.task = None;
        if self.cancelling || matches!(result, Err(Error::Cancelled)) {
            self.notice = Some(tr("Map read stopped. Previous results retained.").into());
        } else {
            match result {
                Ok(data) => {
                    let previous = self
                        .selected
                        .and_then(|i| self.data.as_ref()?.nodes.get(i))
                        .map(|n| n.key.clone());
                    self.selected =
                        previous.and_then(|key| data.nodes.iter().position(|n| n.key == key));
                    let first = self.data.is_none();
                    self.collapsed.retain(|key| {
                        data.nodes
                            .iter()
                            .position(|n| &n.key == key)
                            .is_some_and(|i| navigation::is_group(&data, i))
                    });
                    if self
                        .neighbor_anchor
                        .as_ref()
                        .is_some_and(|key| !data.nodes.iter().any(|n| &n.key == key))
                    {
                        self.neighbor_anchor = None;
                    }
                    if first && data.kind == MapKind::Code {
                        self.alternate = data.nodes.iter().any(|n| n.kind == MapNodeKind::Crate);
                        // Folders open one at a time: start with every folder closed.
                        self.collapsed = (0..data.nodes.len())
                            .filter(|&i| navigation::is_group(&data, i))
                            .map(|i| data.nodes[i].key.clone())
                            .collect();
                    }
                    self.data = Some(Arc::new(data));
                    // The next visible frame rebuilds these caches. MCP must not
                    // observe indices from the prior snapshot in the meantime.
                    self.visible.clear();
                    if !self.loading_more {
                        self.display_offset = 0;
                    }
                    self.positions.clear();
                    self.file_groups.clear();
                    self.projected_links.clear();
                    self.topology = None;
                    self.cell = None;
                    self.line = None;
                    self.loaded_inputs = self.pending_inputs.take();
                    self.loaded_refresh = self.load_refresh;
                    self.filter_key = None;
                    self.notice = None;
                    if !self.loading_more {
                        self.stale = false;
                    }
                    if let Some(path) = self.pending_path.take() {
                        self.open_path(&path);
                    }
                }
                Err(e) => self.error = Some(map_error(&e)),
            }
        }
        self.cancelling = false;
        self.loading_more = false;
    }
    pub fn running(&self) -> bool {
        self.task.is_some()
    }
    fn stop(&mut self) {
        if let Some(e) = &mut self.export {
            e.stop();
        }
        if let Some(t) = &self.task {
            t.cancel();
            self.cancelling = true;
            self.notice = Some(tr("Stopping map read…").into());
        }
    }
    fn load(
        &mut self,
        ctx: &Context,
        kind: MapKind,
        backend: Backend,
        repo: Repository,
        refresh: u64,
        more: bool,
    ) {
        if more && !self.can_more() {
            return;
        }
        self.task = None;
        self.error = None;
        self.notice = None;
        self.cancelling = false;
        self.loading_more = more;
        self.load_refresh = if more { self.loaded_refresh } else { refresh };
        self.pending_inputs = Some(self.inputs());
        let path = if self.path_scope.is_empty() {
            None
        } else {
            match RepoPath::new(self.path_scope.as_bytes().to_vec()) {
                Ok(p) => Some(p),
                Err(e) => {
                    self.error = Some(map_error(&e));
                    return;
                }
            }
        };
        let options = MapOptions {
            kind,
            revision: self.revision.clone(),
            all_branches: self.all_branches,
            since: if kind == MapKind::People {
                self.period.since()
            } else {
                None
            },
            base: self.base.clone(),
            path,
            page_size: MAP_COMMITS,
        };
        let previous = more.then(|| self.data.clone().unwrap());
        match crate::read_task::spawn(ctx, move |cancel| {
            backend.log.request(kind.operation(), |log| {
                let repo = repo.with_log(log.clone());
                match previous {
                    Some(previous) => repo.repository_map_more(&previous, &cancel),
                    None => repo.repository_map(&options, &cancel),
                }
            })
        }) {
            Ok(task) => self.task = Some(task),
            Err(e) => self.error = Some(map_error(&e)),
        }
    }
    fn inputs(&self) -> (String, String, String, bool, Period) {
        (
            self.revision.clone(),
            self.base.clone(),
            self.path_scope.clone(),
            self.all_branches,
            self.period,
        )
    }
    pub fn can_more(&self) -> bool {
        !self.running()
            && self.data.as_ref().is_some_and(|d| d.continuation.is_some())
            && self.loaded_inputs.as_ref() == Some(&self.inputs())
    }
    pub fn edge_kind(&self, kind: MapLinkKind) -> bool {
        match self.data.as_ref().map(|d| d.kind) {
            Some(MapKind::Code) => {
                if self.alternate {
                    kind == MapLinkKind::DependsOn
                } else {
                    matches!(kind, MapLinkKind::Contains | MapLinkKind::DependsOn)
                }
            }
            Some(MapKind::People) => {
                if self.alternate {
                    kind == MapLinkKind::SharedFiles
                } else {
                    kind == MapLinkKind::Changes
                }
            }
            _ => kind == MapLinkKind::Parent,
        }
    }
    /// Code → Folders at `path`: its folders open, the node selected.
    fn open_path(&mut self, path: &[u8]) {
        let Some(data) = self.data.clone() else {
            self.pending_path = Some(path.to_vec());
            return;
        };
        self.alternate = false;
        self.query.clear();
        let found = data.nodes.iter().position(|n| n.path.as_deref() == Some(path));
        let Some(found) = found else { return };
        // Open every folder on the way, and the folder itself.
        let mut prefix = Vec::new();
        for part in path.split(|b| *b == b'/') {
            if !prefix.is_empty() {
                prefix.push(b'/');
            }
            prefix.extend_from_slice(part);
            if let Some(n) = data.nodes.iter().find(|n| n.path.as_deref() == Some(&prefix[..])) {
                self.collapsed.remove(&n.key);
            }
        }
        self.selected = Some(found);
        self.filter_key = None;
        self.focus = true;
    }
}
fn label(bytes: &[u8]) -> String {
    crate::tree::path_label(bytes)
}
fn map_error(e: &Error) -> String {
    match e {Error::Git{..}=>tr("Could not read the map's local Git objects. Check the revision and object availability, then refresh. Previous results retained.").into(),_=>user_error(e)}
}
impl Rocket {
    pub(crate) fn map_intent(&mut self, intent: MapIntent) {
        if let MapIntent::InspectCommit(id) = intent {
            if self.tab().is_some_and(|t| {
                t.commits()
                    .iter()
                    .chain(t.loaded_commits())
                    .any(|c| c.id == id)
            }) {
                self.dispatch(Intent::SelectCommit(id));
            } else {
                self.open_review(crate::reviews::Request {
                    commit: id,
                    path: None,
                    blame: false,
                });
            }
            return;
        }
        if let MapIntent::BrowseExport = intent {
            let current = self
                .tab()
                .and_then(|t| t.map.page().export.as_ref())
                .map(|e| e.dir.clone())
                .unwrap_or_default();
            self.browse(PickTarget::MapExportFolder, &current);
            return;
        }
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let scope = self.submodule_fixture_root.clone();
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let Some(repo) = tab.snapshot.as_ref().map(|s| s.repository.clone()) else {
            return;
        };
        tab.view = View::Map;
        let mut load = false;
        let mut more = false;
        match intent {
            MapIntent::Open => load = tab.map.page().data.is_none() && !tab.map.page().running(),
            MapIntent::Load => load = true,
            MapIntent::More => {
                load = true;
                more = true;
            }
            MapIntent::Export => {
                let kind = tab.map.kind;
                let p = tab.map.page_mut();
                p.rebuild();
                if let Some(data) = &p.data {
                    match map_mermaid(
                        data,
                        &p.visible,
                        &p.projected_links,
                        &CancellationToken::new(),
                    ) {
                        Ok(text) => {
                            p.export = Some(export::Export::new(
                                text,
                                export::default_folder(scope.as_deref()),
                                kind,
                                p.alternate,
                                p.visible.len(),
                                p.projected_links
                                    .iter()
                                    .filter(|e| {
                                        p.edge_kind(e.kind)
                                            && p.visible.contains(&e.from)
                                            && p.visible.contains(&e.to)
                                    })
                                    .count(),
                            ))
                        }
                        Err(e) => p.error = Some(map_error(&e)),
                    }
                }
            }
            MapIntent::SaveExport => {
                if let Some(e) = &mut tab.map.page_mut().export {
                    e.save(&self.context, scope, backend.log.clone());
                }
            }
            MapIntent::BrowseExport | MapIntent::InspectCommit(_) => unreachable!(),
            MapIntent::Cancel => tab.map.page_mut().stop(),
            MapIntent::Switch(kind) => {
                tab.map.page_mut().stop();
                tab.map.kind = kind;
                load = tab.map.page().data.is_none() && !tab.map.page().running();
            }
            MapIntent::History(query) => {
                tab.map.page_mut().stop();
                tab.map.kind = MapKind::History;
                let p = tab.map.page_mut();
                p.query = query;
                p.filter_key = None;
                load = p.data.is_none();
            }
            MapIntent::People(query) => {
                tab.map.page_mut().stop();
                tab.map.kind = MapKind::People;
                let p = tab.map.page_mut();
                p.query = query;
                p.alternate = false;
                p.filter_key = None;
                load = p.data.is_none();
            }
            MapIntent::Code(path) => {
                tab.map.page_mut().stop();
                tab.map.kind = MapKind::Code;
                let p = tab.map.page_mut();
                p.open_path(&path);
                load = p.data.is_none() && !p.running();
            }
            MapIntent::Path(path, query) => {
                tab.map.page_mut().stop();
                tab.map.kind = MapKind::History;
                let p = tab.map.page_mut();
                p.path_scope = label(&path);
                p.show_scope = true;
                p.query = query;
                p.filter_key = None;
                load = true;
            }
        }
        if load {
            let kind = tab.map.kind;
            tab.map
                .page_mut()
                .load(&self.context, kind, backend, repo, tab.refresh_count, more);
        }
    }
    /// The map's Export dialog is open over the active repository.
    pub fn map_export_open(&self) -> bool {
        self.tab()
            .is_some_and(|t| t.view == View::Map && t.map.page().export.is_some())
    }
    pub(crate) fn map_export_folder_picked(&mut self, path: &std::path::Path) {
        if let Some(e) = self
            .tabs
            .get_mut(self.active)
            .and_then(|t| t.map.page_mut().export.as_mut())
        {
            e.dir = crate::views::display_path(path);
            e.error = None;
        }
    }
    pub(crate) fn poll_repository_map(&mut self, ctx: &Context) {
        for tab in &mut self.tabs {
            for p in &mut tab.map.pages {
                if let Some(e) = &mut p.export {
                    e.poll();
                    if e.running() {
                        ctx.request_repaint_after(std::time::Duration::from_millis(30));
                    }
                }
                if let Some(result) = p.task.as_mut().and_then(ReadTask::try_finish) {
                    p.finish(result);
                }
                if p.data.is_some() && p.loaded_refresh != tab.refresh_count {
                    p.stale = true;
                }
                if p.running() {
                    ctx.request_repaint_after(std::time::Duration::from_millis(30));
                }
            }
        }
    }
}
fn register(ui: &Ui, widgets: &mut HashMap<String, UiControl>, id: &str, r: &Response) {
    let rect = r.interact_rect.intersect(ui.clip_rect());
    if rect.is_positive() {
        widgets.insert(
            id.into(),
            UiControl {
                rect,
                enabled: r.enabled(),
            },
        );
    }
}
fn button(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    id: &str,
    text: &str,
    enabled: bool,
) -> bool {
    // Paging and scope rows appear/disappear before other controls. Keep the
    // interaction ID independent of their allocation order, including while a
    // pointer press is in flight.
    let control_id = ui.make_persistent_id(("map-control", id));
    let r = ui
        .scope_builder(UiBuilder::new().id(control_id), |ui| {
            ui.add_enabled(enabled, Button::new(text))
        })
        .inner;
    register(ui, widgets, id, &r);
    r.clicked()
}
/// In the wrapping toolbar, starts a new row when a control of this width would not
/// fit on the current one. A segmented control or chip is drawn as one block, which
/// egui's wrapping layout cannot move to the next row by itself.
fn room(ui: &mut Ui, width: f32) {
    // What is left of this row; available_width() is a whole row in a wrapping layout.
    let rest = ui.available_rect_before_wrap();
    if rest.width() < width && rest.left() > ui.max_rect().left() + 1. {
        ui.end_row();
    }
}
/// A label's width in the toolbar's 12.5 pt buttons.
fn text_width(ui: &Ui, text: &str) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_owned(), FontId::proportional(12.5), TEXT())
        .size()
        .x
}
/// A segmented control whose options keep their own automation names.
fn segments(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    items: &[(&str, bool, &str)],
) -> Option<usize> {
    // Each option is its label plus 10 points either side, 2 apart, in a framed row.
    let width = items.iter().map(|(t, _, _)| text_width(ui, t) + 20.).sum::<f32>()
        + 2. * items.len().saturating_sub(1) as f32
        + 6.;
    room(ui, width);
    let labels: Vec<(&str, bool)> = items.iter().map(|(t, on, _)| (*t, *on)).collect();
    let mut clicked = None;
    for (i, r) in pages::segments(ui, &labels).iter().enumerate() {
        register(ui, widgets, items[i].2, r);
        if r.clicked() {
            clicked = Some(i);
        }
    }
    clicked
}
fn chip(ui: &mut Ui, widgets: &mut HashMap<String, UiControl>, id: &str, text: &str, on: bool) -> bool {
    room(ui, text_width(ui, text) + 22.);
    let r = crate::activity_history::chip(ui, text, None, on, CONTROL);
    register(ui, widgets, id, &r);
    r.clicked()
}
fn emit(out: &mut Vec<Intent>, intent: MapIntent) {
    out.push(Intent::Map(intent));
}
fn short(id: &ObjectId) -> &str {
    &id.as_str()[..8]
}
/// What the header says about the snapshot on screen.
fn snapshot_line(s: &MapPage, kind: MapKind) -> String {
    let what = if s.all_branches && kind != MapKind::Code {
        crate::i18n::tr("all branches").to_owned()
    } else {
        s.revision.clone()
    };
    if s.running() {
        return if s.cancelling {
            crate::i18n::tr("Stopping").into()
        } else {
            trf!("Reading {what}", what = what)
        };
    }
    let Some(d) = &s.data else {
        return tr("How the code, history and people in this repository connect").into();
    };
    let mut line = trf!("Snapshot of {what}", what = what);
    if let Some(tip) = &d.tip {
        line.push_str(&format!(" · {}", short(tip)));
    }
    if kind != MapKind::Code {
        line.push_str(&format!(" · {}", trn!(d.scanned, "{n} commit", "{n} commits")));
    }
    line
}

pub(crate) fn view(
    ui: &mut Ui,
    tab: &mut Tab,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
) {
    ui.set_clip_rect(ui.clip_rect().intersect(ui.available_rect_before_wrap()));
    let compact = ui.available_width() < 850.;
    let low = ui.available_height() < 420.;
    let kind = tab.map.kind;
    let branches = revision_choices(tab);
    let downloads = crate::preview_tab::downloads(tab);
    let s = tab.map.page_mut();
    s.rebuild();
    let line = snapshot_line(s, kind);
    if low {
        Frame::new()
            .inner_margin(Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    header_actions(ui, s, kind, compact, intents, widgets);
                });
            });
    } else {
        pages::header_with(
            ui,
            |painter, tile| icon(painter, tile.shrink(11.), Icon::Map, BLUE()),
            crate::i18n::tr("Repository map"),
            &line,
            |ui| header_actions(ui, s, kind, compact, intents, widgets),
        );
    }
    toolbar(ui, s, kind, compact, &branches, intents, widgets);
    strips(ui, s, kind, intents, widgets);
    s.rebuild();
    body(ui, s, kind, compact, intents, widgets, downloads.as_ref());
    if s.export.is_some() {
        export::dialog(ui.ctx(), s, intents, widgets);
    }
}

/// Right to left: Export, Refresh (Stop while reading), the three maps.
fn header_actions(
    ui: &mut Ui,
    s: &mut MapPage,
    kind: MapKind,
    compact: bool,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
) {
    ui.spacing_mut().item_spacing.x = 8.0;
    let ready = s.data.is_some() && !s.visible.is_empty() && !s.running();
    let r = crate::chrome::icon_button(ui, Icon::External, ready, false, 28.)
        .on_hover_text(tr("Export as Mermaid"));
    register(ui, widgets, "map-export", &r);
    if r.clicked() {
        emit(intents, MapIntent::Export);
    }
    if s.running() {
        if button(
            ui,
            widgets,
            "map-cancel",
            if s.cancelling { crate::i18n::tr("Stopping") } else { crate::i18n::tr("Stop") },
            !s.cancelling,
        ) {
            emit(intents, MapIntent::Cancel);
        }
    } else if button(ui, widgets, "map-load", crate::i18n::tr("Refresh"), !s.revision.trim().is_empty()) {
        emit(intents, MapIntent::Load);
    }
    let items = [
        (crate::i18n::tr("Code"), kind == MapKind::Code, "map-view-code"),
        (crate::i18n::tr("History"), kind == MapKind::History, "map-view-history"),
        (tr("People"), kind == MapKind::People, "map-view-people"),
    ];
    if let Some(i) = segments(ui, widgets, &items) {
        let next = [MapKind::Code, MapKind::History, MapKind::People][i];
        if next != kind {
            emit(intents, MapIntent::Switch(next));
        }
    }
    if compact {
        ui.add_space(4.0);
    }
}

fn toolbar(
    ui: &mut Ui,
    s: &mut MapPage,
    kind: MapKind,
    compact: bool,
    branches: &[(String, bool)],
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
) {
    let margin = if compact { 12 } else { 24 };
    Frame::new()
        .inner_margin(Margin {
            left: margin,
            right: margin,
            top: 0,
            bottom: 10,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // The revision keeps its own column on the right; everything else wraps
            // into rows beside it, so a narrow window never stacks the toolbar
            // over the map.
            let folders = kind == MapKind::Code && !s.alternate;
            let field = if compact { 110. } else { 150. };
            let right = field + if folders { 108. } else { 0. };
            ui.horizontal_top(|ui| {
                let left = (ui.available_width() - right - 12.).max(120.);
                ui.allocate_ui_with_layout(
                    vec2(left, CONTROL),
                    Layout::left_to_right(Align::Center).with_main_wrap(true),
                    |ui| {
                        ui.set_max_width(left);
                        ui.spacing_mut().item_spacing = vec2(8., 8.);
                        // Controls move to the next row instead of breaking their labels.
                        ui.style_mut().wrap_mode = Some(TextWrapMode::Extend);
                        controls(ui, s, kind, compact, intents, widgets);
                    },
                );
                ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.;
                    revision_picker(ui, s, branches, field, intents, widgets);
                    if folders
                        && button(
                            ui,
                            widgets,
                            "map-collapse-all",
                            if s.collapsed.is_empty() { tr("Collapse all") } else { tr("Expand all") },
                            s.data.is_some(),
                        )
                    {
                        if s.collapsed.is_empty() {
                            if let Some(d) = &s.data {
                                s.collapsed.extend(
                                    d.nodes
                                        .iter()
                                        .enumerate()
                                        .filter(|(i, _)| navigation::is_group(d, *i))
                                        .map(|(_, n)| n.key.clone()),
                                );
                            }
                        } else {
                            s.collapsed.clear();
                        }
                        s.display_offset = 0;
                        s.filter_key = None;
                    }
                });
            });
        });
}

/// Branches for the revision picker: the checked-out branch first, then the other
/// local branches, then remote-tracking ones. Symbolic refs such as origin/HEAD are
/// left out. The bool marks the checked-out branch.
fn revision_choices(tab: &Tab) -> Vec<(String, bool)> {
    let Some(snapshot) = &tab.snapshot else {
        return Vec::new();
    };
    let (mut local, remote): (Vec<_>, Vec<_>) = snapshot
        .branches
        .iter()
        .filter(|b| b.symbolic_target.is_none())
        .map(|b| (b.reference.starts_with(b"refs/heads/"), (short_ref(&b.reference), b.is_head)))
        .partition(|(local, _)| *local);
    // A stable sort keeps the rest in the order Git listed them.
    local.sort_by_key(|(_, (_, head))| !*head);
    local.into_iter().chain(remote).map(|(_, choice)| choice).collect()
}

/// The revision to map, on a button with a branch icon. Its list has HEAD and the
/// branches; the box above them takes any commit, branch or tag, and Enter maps it.
/// MCP controls: `map-revision-pick` (the button), `map-revision` (the box),
/// `map-revision-head` and `map-revision-branch-{i}`.
fn revision_picker(
    ui: &mut Ui,
    s: &mut MapPage,
    branches: &[(String, bool)],
    width: f32,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
) {
    let control_id = ui.make_persistent_id(("map-control", "map-revision-pick"));
    let r = ui
        .scope_builder(UiBuilder::new().id(control_id), |ui| ui.add_sized([width, CONTROL], Button::new("")))
        .inner
        .on_hover_text(tr("The commit, branch or tag to map"));
    let rect = r.rect;
    let painter = ui.painter();
    icon(painter, Rect::from_center_size(pos2(rect.left() + 16., rect.center().y), vec2(14., 14.)), Icon::Branch, MUTED());
    icon(painter, Rect::from_center_size(pos2(rect.right() - 14., rect.center().y), vec2(12., 12.)), Icon::Chevron, FAINT());
    let empty = s.revision.trim().is_empty();
    let text = painter.layout_no_wrap(
        if empty { tr("Revision").to_owned() } else { s.revision.trim().to_owned() },
        FontId::proportional(12.5),
        if empty { FAINT() } else { TEXT() },
    );
    let room = Rect::from_min_max(pos2(rect.left() + 29., rect.top()), pos2(rect.right() - 24., rect.bottom()));
    painter.with_clip_rect(room.intersect(ui.clip_rect())).galley(
        pos2(room.left(), rect.center().y - text.size().y / 2.),
        text,
        TEXT(),
    );
    register(ui, widgets, "map-revision-pick", &r);
    Popup::menu(&r)
        // Clicking into the box must not close the list.
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .width(300.)
        .show(|ui| {
            let field = ui.add(
                TextEdit::singleline(&mut s.revision)
                    .id_salt("map-revision")
                    .char_limit(4096)
                    .hint_text(crate::i18n::tr("Commit, branch or tag"))
                    .desired_width(f32::INFINITY),
            );
            register(ui, widgets, "map-revision", &field);
            if field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && !s.revision.trim().is_empty() {
                emit(intents, MapIntent::Load);
                ui.close();
            }
            let checked_out = branches.iter().find(|(_, head)| *head).map_or(crate::i18n::tr("detached"), |(name, _)| name.as_str());
            let mut chosen = None;
            let row = |ui: &mut Ui, name: &str, detail: &str, on: bool| {
                let r = ui.add(Button::selectable(on, "").min_size(vec2(ui.available_width(), 28.)));
                ui.painter().text(pos2(r.rect.left() + 8., r.rect.center().y), Align2::LEFT_CENTER, name, FontId::proportional(12.5), TEXT());
                ui.painter().text(pos2(r.rect.right() - 8., r.rect.center().y), Align2::RIGHT_CENTER, detail, FontId::proportional(12.), FAINT());
                r
            };
            let r = row(ui, "HEAD", checked_out, s.revision.trim() == "HEAD");
            register(ui, widgets, "map-revision-head", &r);
            if r.clicked() {
                chosen = Some("HEAD".to_owned());
            }
            if !branches.is_empty() {
                ui.label(RichText::new(crate::i18n::tr("BRANCHES")).size(11.).color(FAINT()));
                ScrollArea::vertical().id_salt("map-revision-branches").max_height(260.).show(ui, |ui| {
                    for (i, (name, head)) in branches.iter().take(200).enumerate() {
                        let r = row(ui, name, if *head { crate::i18n::tr("checked out") } else { "" }, s.revision.trim() == name);
                        register(ui, widgets, &format!("map-revision-branch-{i}"), &r);
                        if r.clicked() {
                            chosen = Some(name.clone());
                        }
                    }
                });
            }
            ui.label(RichText::new(tr("Enter maps what you type.")).size(12.).color(FAINT()));
            if let Some(name) = chosen {
                s.revision = name;
                emit(intents, MapIntent::Load);
                ui.close();
            }
        });
}

/// Search, the view's modes and its filters, left to right.
fn controls(
    ui: &mut Ui,
    s: &mut MapPage,
    kind: MapKind,
    compact: bool,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
) {
    let hint = match kind {
        MapKind::Code => tr("Find a package, folder or file"),
        MapKind::History => tr("Find a commit, author or branch"),
        MapKind::People => tr("Find a person or folder"),
    };
    let search = if compact { 190. } else { 260. };
    room(ui, search);
    let r = pages::search_box(ui, &mut s.query, "map-search", hint, search);
    register(ui, widgets, "map-search", &r);
    if !s.query.is_empty() {
        room(ui, 26.);
        let r = crate::chrome::icon_button(ui, Icon::Close, true, false, 26.).on_hover_text(crate::i18n::tr("Clear search"));
        register(ui, widgets, "map-clear", &r);
        if r.clicked() {
            s.query.clear();
        }
    }
    let data = s.data.clone();
    match kind {
        MapKind::Code => {
            let crates = data
                .as_ref()
                .map_or(0, |d| d.nodes.iter().filter(|n| n.kind == MapNodeKind::Crate).count());
            let folders = data.as_ref().map_or(0, |d| {
                d.nodes
                    .iter()
                    .filter(|n| n.kind == MapNodeKind::Directory || n.kind == MapNodeKind::Crate)
                    .count()
            });
            if crates > 0 {
                let a = trf!("Packages {count}", count = crates);
                let b = trf!("Folders {count}", count = folders);
                if let Some(i) = segments(ui, widgets, &[(&a, s.alternate, "map-mode-packages"), (&b, !s.alternate, "map-mode-folders")]) {
                    let alternate = i == 0;
                    if alternate != s.alternate {
                        s.alternate = alternate;
                        s.filter_key = None;
                        s.fit = true;
                    }
                }
            }
        }
        MapKind::History => {
            if let Some(i) = segments(
                ui,
                widgets,
                &[(tr("Branches & merges"), s.condensed, "map-mode-topology"), (tr("Every commit"), !s.condensed, "map-mode-commits")],
            ) {
                s.condensed = i == 0;
                s.alternate = false;
                s.filter_key = None;
            }
            scope_chips(ui, s, compact, intents, widgets);
        }
        MapKind::People => {
            if let Some(i) = segments(
                ui,
                widgets,
                &[(tr("Who works where"), !s.alternate, "map-mode-grid"), (tr("Shared files"), s.alternate, "map-mode-shared")],
            ) {
                let alternate = i == 1;
                if alternate != s.alternate {
                    s.alternate = alternate;
                    s.filter_key = None;
                    s.fit = true;
                }
            }
            let periods = [
                (tr("All time"), Period::All, "map-period-all"),
                (crate::i18n::tr("90 days"), Period::Days90, "map-period-90"),
                (crate::i18n::tr("30 days"), Period::Days30, "map-period-30"),
            ];
            let items: Vec<_> = periods.iter().map(|(t, p, id)| (*t, s.period == *p, *id)).collect();
            if let Some(i) = segments(ui, widgets, &items)
                && s.period != periods[i].1
            {
                s.period = periods[i].1;
                emit(intents, MapIntent::Load);
            }
            scope_chips(ui, s, compact, intents, widgets);
        }
    }
    let graph = matches!(
        (kind, s.alternate, s.condensed),
        (MapKind::Code, _, _) | (MapKind::People, true, _) | (MapKind::History, _, false)
    );
    if graph {
        room(ui, text_width(ui, tr("Connected only")) + 22.);
        let on = s.neighbor_anchor.is_some();
        let enabled = s.selected.is_some();
        let r = ui
            .add_enabled_ui(enabled || on, |ui| {
                crate::activity_history::chip(ui, tr("Connected only"), None, on, CONTROL)
            })
            .inner
            .on_hover_text(tr("Show the selection and what it connects to"));
        register(ui, widgets, "map-neighbors", &r);
        if r.clicked() {
            s.neighbors();
        }
    }
}

/// All branches, and the scope chips that open the scope row. A narrow window keeps
/// one: Path opens the same row, which has both fields.
fn scope_chips(
    ui: &mut Ui,
    s: &mut MapPage,
    compact: bool,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
) {
    if chip(ui, widgets, "map-all-branches", crate::i18n::tr("All branches"), s.all_branches) {
        s.all_branches = !s.all_branches;
        emit(intents, MapIntent::Load);
    }
    // The chips name the last part of a path and the start of a commit ID; the
    // scope row shows them whole.
    let path = match s.path_scope.rsplit(['/', '\\']).find(|p| !p.is_empty()) {
        Some(name) => trf!("Path: {name}", name = name),
        None => tr("Path: any").to_owned(),
    };
    if chip(ui, widgets, "map-scope", &path, !s.path_scope.is_empty()) {
        s.show_scope = !s.show_scope;
    }
    if compact && s.base.is_empty() {
        return;
    }
    let base = match s.base.trim() {
        "" => tr("Leave out: none").to_owned(),
        b if b.len() > 16 && b.bytes().all(|c| c.is_ascii_hexdigit()) => trf!("Leave out: {reference}", reference = &b[..12]),
        b => trf!("Leave out: {reference}", reference = b),
    };
    if chip(ui, widgets, "map-scope-base", &base, !s.base.is_empty()) {
        s.show_scope = !s.show_scope;
    }
}

/// The scope row and one-line notices under the toolbar.
fn strips(
    ui: &mut Ui,
    s: &mut MapPage,
    kind: MapKind,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
) {
    let compact = ui.available_width() < 850.;
    let margin = if compact { 12 } else { 24 };
    Frame::new()
        .inner_margin(Margin {
            left: margin,
            right: margin,
            top: 0,
            bottom: 0,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 8.;
            if s.show_scope && kind != MapKind::Code {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(crate::i18n::tr("Path")).color(HINT()));
                    let r = ui.add(
                        TextEdit::singleline(&mut s.path_scope)
                            .id_salt("map-path-scope")
                            .char_limit(4096)
                            .desired_width(220.)
                            .hint_text(tr("Literal file or folder path")),
                    );
                    register(ui, widgets, "map-path-scope", &r);
                    ui.label(RichText::new(tr("Leave out")).color(HINT()));
                    let r = ui.add(
                        TextEdit::singleline(&mut s.base)
                            .id_salt("map-base")
                            .char_limit(4096)
                            .desired_width(170.)
                            .hint_text(tr("Commit or branch")),
                    );
                    let r = r.on_hover_text(tr("Leave out commits this commit or branch already has"));
                    register(ui, widgets, "map-base", &r);
                    if button(ui, widgets, "map-apply-scope", crate::i18n::tr("Apply"), !s.revision.is_empty()) {
                        emit(intents, MapIntent::Load);
                    }
                });
                ui.add_space(2.);
            }
            if let Some(error) = s.error.clone() {
                if let Some(r) = pages::notice_strip(ui, RED(), &error, Some(crate::i18n::tr("Retry"))) {
                    register(ui, widgets, "map-retry", &r);
                    if r.clicked() {
                        emit(intents, MapIntent::Load);
                    }
                }
            } else if let Some(notice) = s.notice.clone() {
                pages::notice_strip(ui, HINT(), &notice, None);
            } else if s.stale
                && !s.running()
                && let Some(r) = pages::notice_strip(ui, BLUE(), tr("The repository changed since this map was read"), Some(crate::i18n::tr("Refresh")))
            {
                register(ui, widgets, "map-refresh-stale", &r);
                if r.clicked() {
                    emit(intents, MapIntent::Load);
                }
            }
            if let Some(d) = s.data.clone()
                && kind != MapKind::Code
                && d.has_more
            {
                let text = trn!(d.scanned, "Showing the newest {n} commit", "Showing the newest {n} commits");
                let r = pages::notice_strip(ui, AMBER(), &text, Some(crate::i18n::tr("Load older")));
                if let Some(r) = r {
                    let enabled = s.can_more();
                    register(ui, widgets, "map-more", &r);
                    if enabled && r.clicked() {
                        emit(intents, MapIntent::More);
                    }
                }
            } else if s.data.as_ref().is_some_and(|d| d.limited) {
                pages::notice_strip(ui, AMBER(), tr("This map is partial. Narrow the scope to see the rest."), None);
            }
            // Branches & merges folds runs of commits and lists them all.
            if s.view_total > DISPLAY_NODES && !(kind == MapKind::History && s.condensed) {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(trf!(
                            "Nodes {start}–{end} of {total}",
                            start = s.display_offset + 1,
                            end = (s.display_offset + DISPLAY_NODES).min(s.view_total),
                            total = s.view_total
                        ))
                        .color(HINT()),
                    );
                    if button(ui, widgets, "map-previous-nodes", crate::i18n::tr("Previous"), s.display_offset > 0) {
                        s.display_offset = s.display_offset.saturating_sub(DISPLAY_NODES);
                        s.filter_key = None;
                    }
                    if button(
                        ui,
                        widgets,
                        "map-next-nodes",
                        crate::i18n::tr("Next"),
                        s.display_offset + DISPLAY_NODES < s.view_total,
                    ) {
                        s.display_offset += DISPLAY_NODES;
                        s.filter_key = None;
                    }
                });
            }
            ui.add_space(2.);
        });
}

fn body(
    ui: &mut Ui,
    s: &mut MapPage,
    kind: MapKind,
    compact: bool,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
    // A preview's downloads, for the files a map at another revision needs.
    preview: Option<&crate::preview_tab::Downloads>,
) {
    let rect = ui.available_rect_before_wrap();
    let margin = if compact { 12. } else { 24. };
    let inner = Rect::from_min_max(
        rect.min + vec2(margin, 0.),
        rect.max - vec2(margin, if compact { 12. } else { 16. }),
    );
    if inner.height() < 60. || inner.width() < 120. {
        ui.allocate_rect(rect, Sense::hover());
        return;
    }
    let wide = !compact && s.data.is_some();
    let canvas = if wide {
        Rect::from_min_max(inner.min, pos2(inner.right() - DETAILS_WIDTH - 16., inner.bottom()))
    } else {
        inner
    };
    let covered = compact && s.show_details && s.data.is_some();
    ui.painter().rect(canvas, 8., VIEW_BG(), Stroke::new(1., CARD_STROKE()), StrokeKind::Inside);
    let content = canvas.shrink(1.);
    match kind {
        MapKind::History if s.condensed => lists::topology(ui, s, content, covered, widgets),
        MapKind::History => lists::commits(ui, s, content, covered, widgets),
        MapKind::People if !s.alternate => lists::grid(ui, s, content, covered, widgets),
        _ => drawing::canvas(ui, s, content, covered, widgets),
    }
    let downloading = s.running() && preview.is_some_and(|d| d.status.running);
    let failed = !s.running() && s.data.is_none() && s.error.is_some() && preview.is_some_and(|d| d.status.failure.is_some());
    if downloading || failed {
        ui.painter().rect_filled(content, 7., VIEW_BG());
        let mut card = ui.new_child(UiBuilder::new().max_rect(content).layout(Layout::top_down(Align::Center)));
        if downloading {
            let revision = s.revision.trim();
            let title = trf!("Downloading the files of {revision}", revision = revision.strip_prefix("refs/heads/").unwrap_or(revision));
            if let Some(true) = crate::preview_tab::downloading(&mut card, preview, &title, "map-download-stop", widgets) {
                emit(intents, MapIntent::Cancel);
            }
        } else if let Some(true) = crate::preview_tab::not_downloaded(&mut card, preview, "map-download-retry", widgets) {
            emit(intents, MapIntent::Load);
        }
    }
    if wide {
        let panel = Rect::from_min_max(pos2(inner.right() - DETAILS_WIDTH, inner.top()), inner.max);
        details::card(ui, s, kind, panel, false, intents, widgets);
    } else if covered {
        ui.painter().rect_filled(canvas, 8., Color32::from_black_alpha(90));
        let width = (canvas.width() - 40.).min(DETAILS_WIDTH);
        let panel = Rect::from_min_max(pos2(canvas.right() - width, canvas.top()), canvas.max);
        let backdrop = Rect::from_min_max(canvas.min, pos2(panel.left(), canvas.bottom()));
        if ui.interact(backdrop, ui.id().with("map-backdrop"), Sense::click()).clicked() {
            s.show_details = false;
        }
        details::card(ui, s, kind, panel, true, intents, widgets);
    }
    ui.allocate_rect(rect, Sense::hover());
}
