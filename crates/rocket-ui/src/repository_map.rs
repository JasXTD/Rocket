//! Per-repository immutable map reads and presentation. A dropped/superseded
//! receiver cannot publish a stale result, and Stop wins even if completion races.
use crate::{
    app::*,
    backend::{Backend, user_error},
    pages,
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

mod drawing;
mod export;
mod navigation;
#[cfg(test)]
mod tests;
const DISPLAY_NODES: usize = 240;

pub enum MapIntent {
    Open,
    Load,
    More,
    Export,
    SaveExport,
    Cancel,
    Switch(MapKind),
    History(String),
    People(String),
    Code(Vec<u8>),
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
    pub recent: bool,
    pub base: String,
    pub path_scope: String,
    pub show_scope: bool,
    pub collapsed: HashSet<Vec<u8>>,
    pub neighbor_anchor: Option<Vec<u8>>,
    pub projected_links: Vec<MapLink>,
    pub export: Option<export::Export>,
    pub alternate: bool,
    pub data: Option<Arc<RepositoryMap>>,
    pub selected: Option<usize>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub show_details: bool,
    hide_wide_details: bool,
    pub cancelling: bool,
    pub loaded_refresh: u64,
    pub visible: Vec<usize>,
    pub display_offset: usize,
    pub view_total: usize,
    pub positions: Vec<Pos2>,
    pub zoom: f32,
    pub pan: Vec2,
    pub fit: bool,
    pub focus: bool,
    pub stale: bool,
    task: Option<ReadTask<RepositoryMap>>,
    load_refresh: u64,
    filter_key: Option<(String, bool)>,
    last_canvas: Vec2,
    loaded_inputs: Option<(String, String, String, bool, bool)>,
    pending_inputs: Option<(String, String, String, bool, bool)>,
    loading_more: bool,
}
impl Default for MapPage {
    fn default() -> Self {
        Self {
            revision: "HEAD".into(),
            query: String::new(),
            all_branches: false,
            recent: false,
            base: String::new(),
            path_scope: String::new(),
            show_scope: false,
            collapsed: HashSet::new(),
            neighbor_anchor: None,
            projected_links: Vec::new(),
            export: None,
            alternate: false,
            data: None,
            selected: None,
            error: None,
            notice: None,
            show_details: false,
            hide_wide_details: false,
            cancelling: false,
            loaded_refresh: 0,
            visible: Vec::new(),
            display_offset: 0,
            view_total: 0,
            positions: Vec::new(),
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
        }
    }
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
            self.notice = Some("Map read stopped. Previous results retained.".into());
        } else {
            match result {
                Ok(data) => {
                    let previous = self
                        .selected
                        .and_then(|i| self.data.as_ref()?.nodes.get(i))
                        .map(|n| n.key.clone());
                    self.selected =
                        previous.and_then(|key| data.nodes.iter().position(|n| n.key == key));
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
                    if self.data.is_none() && data.kind == MapKind::Code {
                        self.alternate = data.nodes.iter().any(|n| n.kind == MapNodeKind::Crate);
                    }
                    self.data = Some(Arc::new(data));
                    // The next visible frame rebuilds these caches. MCP must not
                    // observe indices from the prior snapshot in the meantime.
                    self.visible.clear();
                    if !self.loading_more {
                        self.display_offset = 0;
                    }
                    self.positions.clear();
                    self.projected_links.clear();
                    self.loaded_inputs = self.pending_inputs.take();
                    self.loaded_refresh = self.load_refresh;
                    self.filter_key = None;
                    self.notice = None;
                    if !self.loading_more {
                        self.stale = false;
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
            self.notice = Some("Stopping map read…".into());
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
            since: if self.recent {
                crate::statistics::Period::Days30.since()
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
    fn inputs(&self) -> (String, String, String, bool, bool) {
        (
            self.revision.clone(),
            self.base.clone(),
            self.path_scope.clone(),
            self.all_branches,
            self.recent,
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
}
fn label(bytes: &[u8]) -> String {
    crate::tree::path_label(bytes)
}
fn map_error(e: &Error) -> String {
    match e {Error::Git{..}=>"Could not read the map's local Git objects. Check the revision and object availability, then refresh. Previous results retained.".into(),_=>user_error(e)}
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
        let Some(backend) = self.backend.clone() else {
            return;
        };
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
                let p = tab.map.page_mut();
                p.rebuild();
                if let Some(data) = &p.data {
                    match map_mermaid(
                        data,
                        &p.visible,
                        &p.projected_links,
                        &CancellationToken::new(),
                    ) {
                        Ok(text) => p.export = Some(export::Export::new(text)),
                        Err(e) => p.error = Some(map_error(&e)),
                    }
                }
            }
            MapIntent::SaveExport => {
                if let Some(e) = &mut tab.map.page_mut().export {
                    e.save(&self.context, self.browse_root.clone(), backend.log.clone());
                }
            }
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
                p.alternate = false;
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
                p.query = label(&path);
                p.alternate = false;
                p.filter_key = None;
                load = p.data.is_none();
            }
            MapIntent::InspectCommit(_) => unreachable!(),
        }
        if load {
            let kind = tab.map.kind;
            tab.map
                .page_mut()
                .load(&self.context, kind, backend, repo, tab.refresh_count, more);
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
fn emit(out: &mut Vec<Intent>, intent: MapIntent) {
    out.push(Intent::Map(intent));
}

pub(crate) fn view(
    ui: &mut Ui,
    tab: &mut Tab,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
) {
    ui.set_clip_rect(ui.clip_rect().intersect(ui.available_rect_before_wrap()));
    let compact = ui.available_width() < 850.;
    let short = ui.available_height() < 370.;
    let kind = tab.map.kind;
    let s = tab.map.page_mut();
    s.rebuild();
    if s.export.is_some() {
        export::panel(ui, s, intents, widgets);
        return;
    }
    if !short {
        pages::header(
            ui,
            Icon::Branch,
            "Repository map",
            "Code structure, Git ancestry and contributions · local, read only",
            |_| {},
        );
        pages::divider(ui);
    }
    Frame::new()
        .inner_margin(Margin::symmetric(12, 7))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (k, text, id) in [
                    (MapKind::Code, "Code", "code"),
                    (MapKind::History, "Git history", "history"),
                    (MapKind::People, "People", "people"),
                ] {
                    let r = ui.selectable_label(kind == k, text);
                    register(ui, widgets, &format!("map-view-{id}"), &r);
                    if r.clicked() {
                        emit(intents, MapIntent::Switch(k));
                    }
                }
                ui.separator();
                if button(ui, widgets, "map-load", "Refresh", !s.revision.is_empty()) {
                    emit(intents, MapIntent::Load);
                }
                if button(
                    ui,
                    widgets,
                    "map-cancel",
                    if s.cancelling { "Stopping" } else { "Stop" },
                    s.running() && !s.cancelling,
                ) {
                    emit(intents, MapIntent::Cancel);
                }
                if button(ui, widgets, "map-details", "Details", s.selected.is_some()) {
                    if compact {
                        s.show_details = !s.show_details;
                    } else {
                        s.hide_wide_details = !s.hide_wide_details;
                    }
                }
                if button(
                    ui,
                    widgets,
                    "map-export",
                    "Export Mermaid",
                    s.data.is_some() && !s.visible.is_empty() && !s.running(),
                ) {
                    emit(intents, MapIntent::Export);
                }
            });
            ui.horizontal_wrapped(|ui| {
                let r = ui.add_sized(
                    [if compact { 120. } else { 180. }, 26.],
                    TextEdit::singleline(&mut s.revision)
                        .id_salt("map-revision")
                        .char_limit(4096)
                        .hint_text("Commit, branch or tag"),
                );
                register(ui, widgets, "map-revision", &r);
                if kind != MapKind::Code {
                    let r = ui.checkbox(&mut s.all_branches, "All branches");
                    register(ui, widgets, "map-all-branches", &r);
                    if r.changed() {
                        emit(intents, MapIntent::Load);
                    }
                }
                if kind == MapKind::People {
                    let r = ui.checkbox(&mut s.recent, "Last 30 days");
                    register(ui, widgets, "map-recent", &r);
                    if r.changed() {
                        emit(intents, MapIntent::Load);
                    }
                }
                let text = match kind {
                    MapKind::Code => "Cargo packages",
                    MapKind::History => "Merge context",
                    MapKind::People => "Shared files",
                };
                let r = ui.checkbox(&mut s.alternate, text);
                register(ui, widgets, "map-mode", &r);
            });
            ui.horizontal_wrapped(|ui| {
                if kind == MapKind::Code
                    && !s.alternate
                    && button(
                        ui,
                        widgets,
                        "map-collapse-all",
                        if s.collapsed.is_empty() {
                            "Collapse folders"
                        } else {
                            "Expand all"
                        },
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
                if button(
                    ui,
                    widgets,
                    "map-neighbors",
                    if s.neighbor_anchor.is_some() {
                        "All connections"
                    } else {
                        "Neighbors"
                    },
                    s.selected.is_some(),
                ) {
                    s.neighbors();
                }
                if kind != MapKind::Code {
                    if button(ui, widgets, "map-scope", "Scope", true) {
                        s.show_scope = !s.show_scope;
                    }
                    if button(ui, widgets, "map-more", "Load more", s.can_more()) {
                        emit(intents, MapIntent::More);
                    }
                    if let Some(d) = &s.data {
                        ui.label(
                            RichText::new(format!(
                                "{} loaded · {} outside parents{}",
                                d.scanned,
                                d.boundary_parents,
                                if d.has_more {
                                    " · older history available"
                                } else {
                                    ""
                                }
                            ))
                            .size(10.)
                            .color(MUTED()),
                        );
                    }
                }
            });
            if s.view_total > DISPLAY_NODES {
                ui.horizontal_wrapped(|ui| {
                    if button(
                        ui,
                        widgets,
                        "map-previous-nodes",
                        "Previous nodes",
                        s.display_offset > 0,
                    ) {
                        s.display_offset = s.display_offset.saturating_sub(DISPLAY_NODES);
                        s.filter_key = None;
                    }
                    if button(
                        ui,
                        widgets,
                        "map-next-nodes",
                        "Next nodes",
                        s.display_offset + DISPLAY_NODES < s.view_total,
                    ) {
                        s.display_offset += DISPLAY_NODES;
                        s.filter_key = None;
                    }
                    ui.label(
                        RichText::new(format!(
                            "Nodes {}–{} of {}",
                            s.display_offset + 1,
                            (s.display_offset + DISPLAY_NODES).min(s.view_total),
                            s.view_total
                        ))
                        .size(10.)
                        .color(MUTED()),
                    );
                });
            }
            if s.show_scope && kind != MapKind::Code {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Exclude base");
                    let r = ui.add(
                        TextEdit::singleline(&mut s.base)
                            .id_salt("map-base")
                            .char_limit(4096)
                            .desired_width(150.)
                            .hint_text("Optional commit or branch"),
                    );
                    register(ui, widgets, "map-base", &r);
                    ui.label("Commits touching");
                    let r = ui.add(
                        TextEdit::singleline(&mut s.path_scope)
                            .id_salt("map-path-scope")
                            .char_limit(4096)
                            .desired_width(170.)
                            .hint_text("Literal file/folder path"),
                    );
                    register(ui, widgets, "map-path-scope", &r);
                    if button(
                        ui,
                        widgets,
                        "map-apply-scope",
                        "Apply scope",
                        !s.revision.is_empty(),
                    ) {
                        emit(intents, MapIntent::Load);
                    }
                });
            }
            ui.horizontal(|ui| {
                let r = ui.add_sized(
                    [(ui.available_width() - 165.).max(75.), 26.],
                    TextEdit::singleline(&mut s.query)
                        .id_salt("map-search")
                        .char_limit(1024)
                        .hint_text("Find name, path, author or commit"),
                );
                register(ui, widgets, "map-search", &r);
                if button(ui, widgets, "map-clear", "Clear", !s.query.is_empty()) {
                    s.query.clear();
                }
                let message = if s.running() {
                    if s.cancelling {
                        "Stopping…"
                    } else {
                        "Reading local objects…"
                    }
                } else if s.stale {
                    "Refresh suggested"
                } else {
                    "Pinned snapshot"
                };
                ui.add(
                    Label::new(RichText::new(message).size(11.).color(if s.stale {
                        AMBER()
                    } else {
                        MUTED()
                    }))
                    .truncate(),
                );
            });
            if let Some(message) = s.error.as_ref().or(s.notice.as_ref()) {
                ui.add(
                    Label::new(
                        RichText::new(message)
                            .size(11.)
                            .color(if s.error.is_some() { RED() } else { AMBER() }),
                    )
                    .truncate(),
                )
                .on_hover_text(message);
            }
        });
    s.rebuild();
    let rect = ui.available_rect_before_wrap();
    if rect.height() < 45. {
        return;
    }
    let footer = 24.;
    let body = Rect::from_min_max(rect.min, rect.right_bottom() - vec2(0., footer));
    let wide_details = !compact && !s.hide_wide_details;
    let details = (wide_details || compact && s.show_details) && s.data.is_some();
    let canvas = if wide_details && details {
        Rect::from_min_max(body.min, body.right_bottom() - vec2(284., 0.))
    } else {
        body
    };
    drawing::canvas(ui, s, canvas, compact && s.show_details, widgets);
    if details {
        let panel = Rect::from_min_max(
            pos2((body.right() - 280.).max(body.left() + 10.), body.top()),
            body.right_bottom(),
        );
        if compact {
            ui.painter()
                .rect_filled(body, 0., Color32::from_black_alpha(90));
            let backdrop = Rect::from_min_max(body.min, pos2(panel.left(), body.bottom()));
            let r = ui.interact(backdrop, ui.id().with("map-backdrop"), Sense::click());
            if r.clicked() {
                s.show_details = false;
            }
        }
        ui.painter().rect_filled(panel, 0., PANEL());
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(panel.shrink(10.))
                .layout(Layout::top_down(Align::Min)),
            |ui| {
                ui.set_clip_rect(panel.intersect(ui.clip_rect()));
                inspector(ui, s, compact, intents, widgets);
            },
        );
    }
    ui.painter().text(
        pos2(rect.left() + 12., rect.bottom() - 12.),
        Align2::LEFT_CENTER,
        if kind == MapKind::People {
            "Shared files show overlapping changes, not a productivity score."
        } else {
            "Drag to pan · scroll to zoom · Focus for readable labels · Fit for overview"
        },
        FontId::proportional(10.),
        MUTED(),
    );
    ui.allocate_rect(rect, Sense::hover());
}

fn inspector(
    ui: &mut Ui,
    s: &mut MapPage,
    compact: bool,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("DETAILS").size(10.).color(MUTED()));
        if compact && button(ui, widgets, "map-close-details", "Close", true) {
            s.show_details = false;
        }
    });
    let Some(data) = s.data.clone() else { return };
    let Some(i) = s.selected else {
        ui.label("Select a node to inspect its connections.");
        return;
    };
    let n = &data.nodes[i];
    let scroll = ScrollArea::vertical()
        .id_salt("map-details-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add(Label::new(RichText::new(label(&n.label)).size(17.).strong()).wrap());
            ui.add(Label::new(RichText::new(label(&n.detail)).size(11.).color(MUTED())).wrap());
            if let Some(path) = &n.path {
                ui.add(Label::new(RichText::new(label(path)).monospace().size(10.)).wrap());
            }
            if let Some(id) = &n.commit {
                ui.label(RichText::new(&id.as_str()[..12]).monospace().color(BLUE()));
            }
            ui.add_space(6.);
            if navigation::is_group(&data, i)
                && button(
                    ui,
                    widgets,
                    "map-group",
                    if s.collapsed.contains(&n.key) {
                        "Expand group"
                    } else {
                        "Collapse group"
                    },
                    true,
                )
            {
                s.toggle_group(i);
            }
            ui.label(match n.kind {
                MapNodeKind::Commit => format!("{} parents · {} changed paths", n.count, n.files),
                MapNodeKind::Person | MapNodeKind::Area => {
                    format!("{} commits · {} changed paths", n.count, n.files)
                }
                MapNodeKind::File => "Committed file".into(),
                _ => format!("{} committed files", n.files),
            });
            for reference in n.refs.iter().take(16) {
                ui.add(Label::new(RichText::new(label(reference)).size(11.).color(BLUE())).wrap());
            }
            if n.refs.len() > 16 {
                ui.label(format!("{} more references", n.refs.len() - 16));
            }
            ui.add_space(6.);
            if n.kind == MapNodeKind::Commit {
                if button(ui, widgets, "map-inspect-commit", "Open commit", true)
                    && let Some(id) = &n.commit
                {
                    emit(intents, MapIntent::InspectCommit(id.clone()));
                }
                if button(ui, widgets, "map-person", "Find contributor", true) {
                    emit(intents, MapIntent::People(label(&n.detail)));
                }
            } else if n.kind == MapNodeKind::Person {
                if button(ui, widgets, "map-history", "Find author in history", true) {
                    emit(intents, MapIntent::History(label(&n.label)));
                }
            } else if n.path.as_deref() != Some(b"/")
                && button(ui, widgets, "map-history", "Find path in history", true)
            {
                emit(
                    intents,
                    MapIntent::History(n.path.as_ref().map(|p| label(p)).unwrap_or_default()),
                );
            }
            ui.separator();
            ui.label(RichText::new("Connections").strong().size(12.));
            let connections = s
                .projected_links
                .iter()
                .enumerate()
                .filter(|(_, e)| (e.from == i || e.to == i) && s.edge_kind(e.kind))
                .take(30)
                .map(|(i, e)| (i, e.clone()))
                .collect::<Vec<_>>();
            for (ei, e) in connections {
                let other = if e.from == i { e.to } else { e.from };
                let direction = match e.kind {
                    MapLinkKind::Contains => {
                        if e.from == i {
                            "contains"
                        } else {
                            "inside"
                        }
                    }
                    MapLinkKind::DependsOn => {
                        if e.from == i {
                            "uses"
                        } else {
                            "used by"
                        }
                    }
                    MapLinkKind::Parent => {
                        if e.from == i {
                            "child"
                        } else {
                            "parent"
                        }
                    }
                    MapLinkKind::Changes => "changed",
                    MapLinkKind::SharedFiles => "shared",
                };
                let r = ui.add(
                    Button::new(format!(
                        "{} · {}{}",
                        label(&data.nodes[other].label),
                        direction,
                        if matches!(e.kind, MapLinkKind::Changes | MapLinkKind::SharedFiles) {
                            format!(" {}", e.weight)
                        } else {
                            String::new()
                        }
                    ))
                    .wrap(),
                );
                let r = r.on_hover_text(if e.evidence.is_empty() {
                    direction.into()
                } else {
                    format!(
                        "Recorded paths (up to 24):\n{}",
                        e.evidence
                            .iter()
                            .map(|p| label(p))
                            .collect::<Vec<_>>()
                            .join("\n")
                    )
                });
                register(ui, widgets, &format!("map-link-{ei}"), &r);
                if r.clicked() {
                    s.selected = Some(other);
                    if s.neighbor_anchor.is_some() {
                        s.neighbor_anchor = Some(data.nodes[other].key.clone());
                        s.filter_key = None;
                    }
                    if !s.visible.contains(&other) {
                        if s.visible.len() == DISPLAY_NODES {
                            s.visible.pop();
                        }
                        s.visible.push(other);
                        s.visible.sort_unstable();
                        s.positions = drawing::layout(&data, &s.visible, s.alternate);
                    }
                    s.focus = true;
                }
            }
            if !n.evidence.is_empty() {
                ui.separator();
                ui.label(RichText::new("Changed paths (up to 24)").size(12.).strong());
                for (ei, path) in n.evidence.iter().take(24).enumerate() {
                    let r = ui.add(Button::new(label(path)).wrap());
                    register(ui, widgets, &format!("map-path-{ei}"), &r);
                    if r.clicked() {
                        emit(intents, MapIntent::Code(path.clone()));
                    }
                }
            }
            ui.separator();
            for note in &data.notes {
                ui.add(Label::new(RichText::new(note).size(10.).color(MUTED())).wrap());
            }
        });
    widgets.insert(
        "map-details-scroll".into(),
        UiControl {
            rect: scroll.inner_rect.intersect(ui.clip_rect()),
            enabled: true,
        },
    );
}
