//! The card beside the map: what the selection is, what it connects to, and where to
//! go from it.
use super::lists::Line;
use super::*;
use std::collections::BTreeSet;

/// Width of the fact names.
const KEY: f32 = 78.;
/// Rows a list shows before "N more".
const ROWS: usize = 100;

enum Pick {
    Empty,
    Node(usize),
    /// People grid: a person in a folder.
    Cell(usize, usize),
    /// Branches & merges: one line.
    Line(usize),
    /// Modules: the whole package.
    Package(usize),
    /// Code map: a folder's own files on one card; None for the top folder.
    Files(Option<usize>),
}
enum Mark {
    Icon(Icon, Color32),
    Person(String),
}
/// What a click in the card changes once the card is drawn.
enum Act {
    Select(usize),
    /// Open a folder and select one of its folders.
    Open(usize, usize),
    Toggle(usize),
    Cell(usize, usize),
    /// Show this package among the packages.
    Package(usize),
    /// Modules: put this module in the middle.
    Module(usize),
    /// Code map: choose a card or box.
    Item(code_map::Item),
}

pub(super) fn card(
    ui: &mut Ui,
    s: &mut MapPage,
    kind: MapKind,
    rect: Rect,
    drawer: bool,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
) {
    let clip = rect.intersect(ui.clip_rect());
    ui.painter()
        .with_clip_rect(clip)
        .rect(rect, 8., VIEW_PANEL(), Stroke::new(1., CARD_STROKE()), StrokeKind::Inside);
    let Some(data) = s.data.clone() else { return };
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink(1.))
            .layout(Layout::top_down(Align::Min)),
    );
    child.set_clip_rect(clip.shrink(1.));
    let ui = &mut child;
    let pick = pick(s, &data, kind);
    let (mark, title, about) = heading(s, &data, &pick);
    if header(ui, &mark, &title, &about, drawer, widgets) {
        s.show_details = false;
    }
    let mut act = None;
    let view: &MapPage = s;
    let scroll = ScrollArea::vertical()
        .id_salt("map-details-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            Frame::new()
                .inner_margin(Margin {
                    left: 14,
                    right: 12,
                    top: 12,
                    bottom: 14,
                })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 8.;
                    let mut out = Out {
                        widgets,
                        intents,
                        act: &mut act,
                        rows: 0,
                    };
                    match pick {
                        Pick::Empty => {
                            ui.label(RichText::new(tr("Select a card to see what it connects to")).color(HINT()));
                        }
                        Pick::Cell(p, a) => cell(ui, &data, p, a, &mut out),
                        Pick::Line(l) => line(ui, view, &data, l, &mut out),
                        Pick::Node(i) => node(ui, view, &data, i, &mut out),
                        Pick::Files(folder) => files_card(ui, view, &data, folder, &mut out),
                        Pick::Package(_) => {
                            let mut chosen = None;
                            modules::package_details(ui, view, &data, &mut chosen, out.intents, out.widgets);
                            if let Some(k) = chosen {
                                *out.act = Some(Act::Module(k));
                            }
                        }
                    }
                    notes(ui, &data, out.widgets);
                });
        });
    pages::region(widgets, "map-details-scroll", scroll.inner_rect.intersect(clip), true);
    match act {
        Some(Act::Select(i)) => select(s, i),
        Some(Act::Open(folder, child)) => {
            if let Some(key) = s.data.as_ref().map(|d| d.nodes[folder].key.clone()) {
                s.collapsed.remove(&key);
            }
            s.filter_key = None;
            select(s, child);
        }
        Some(Act::Toggle(i)) => s.toggle_group(i),
        Some(Act::Cell(p, a)) => {
            s.cell = Some((p, a));
            s.selected = Some(p);
        }
        Some(Act::Package(i)) => {
            s.alternate = true;
            s.selected = Some(i);
            s.neighbor_anchor = None;
            s.filter_key = None;
            s.fit = true;
            s.focus = true;
        }
        Some(Act::Module(i)) => {
            s.whole = false;
            s.selected = Some(i);
            s.list_jump = true;
        }
        Some(Act::Item(code_map::Item::Node(i))) => {
            s.code.group = None;
            select(s, i);
        }
        Some(Act::Item(code_map::Item::Files(folder))) => {
            s.code.group = Some(folder);
            s.focus = true;
        }
        None => {}
    }
}
/// Select a node, bringing it into view if search or Connected only hid it.
fn select(s: &mut MapPage, i: usize) {
    s.selected = Some(i);
    s.cell = None;
    if !s.visible.contains(&i) {
        s.query.clear();
        s.neighbor_anchor = None;
        s.filter_key = None;
    } else if s.neighbor_anchor.is_some()
        && let Some(n) = s.data.as_ref().and_then(|d| d.nodes.get(i))
    {
        s.neighbor_anchor = Some(n.key.clone());
        s.filter_key = None;
    }
    s.focus = true;
}
struct Out<'a> {
    widgets: &'a mut HashMap<String, UiControl>,
    intents: &'a mut Vec<Intent>,
    act: &'a mut Option<Act>,
    /// Rows drawn so far; each row's automation name is `map-link-<n>`.
    rows: usize,
}
impl Out<'_> {
    fn emit(&mut self, intent: MapIntent) {
        emit(self.intents, intent);
    }
    /// The next connection row's automation name.
    fn link(&mut self) -> String {
        self.rows += 1;
        format!("map-link-{}", self.rows - 1)
    }
}

fn pick(s: &MapPage, data: &RepositoryMap, kind: MapKind) -> Pick {
    let n = data.nodes.len();
    if kind == MapKind::History
        && s.condensed
        && let (Some(l), Some(topology)) = (s.line, &s.topology)
        && l < topology.lines.len()
    {
        return Pick::Line(l);
    }
    if kind == MapKind::People
        && !s.alternate
        && let Some((p, a)) = s.cell
        && p < n
        && a < n
    {
        return Pick::Cell(p, a);
    }
    if kind == MapKind::Modules
        && s.whole
        && let Some(p) = s.units.as_ref().and_then(|u| u.package)
    {
        return Pick::Package(p);
    }
    if kind == MapKind::Code
        && !s.alternate
        && let Some(folder) = s.code.group
    {
        return Pick::Files(folder);
    }
    match s.selected {
        Some(i) if i < n => Pick::Node(i),
        _ => Pick::Empty,
    }
}
pub(super) fn initials(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase()
}
pub(super) fn area_name(data: &RepositoryMap, a: usize) -> String {
    match data.nodes[a].path.as_deref() {
        Some(b"/") => tr("the top folder").into(),
        Some(path) => label(path),
        None => label(&data.nodes[a].label),
    }
}
fn heading(s: &MapPage, data: &RepositoryMap, pick: &Pick) -> (Mark, String, String) {
    match *pick {
        Pick::Empty => (Mark::Icon(Icon::Map, BLUE()), crate::i18n::tr("Details").into(), tr("Nothing selected").into()),
        Pick::Cell(p, a) => {
            let name = label(&data.nodes[p].label);
            (Mark::Person(initials(&name)), name, trf!("in {folder}", folder = area_name(data, a)))
        }
        Pick::Line(l) => match &s.topology.as_ref().expect("picked line").lines[l] {
            Line::Commit { node, merged } if !merged.is_empty() => (
                Mark::Icon(Icon::Merge, VIOLET()),
                lists::merge_name(&label(&data.nodes[*node].label)),
                tr("Merged branch").into(),
            ),
            Line::Commit { node, .. } => node_heading(s, data, *node),
            Line::Run { nodes } => (
                Mark::Icon(Icon::Commit, BLUE()),
                trn!(nodes.len(), "{n} commit", "{n} commits"),
                tr("Commits in a row").into(),
            ),
        },
        Pick::Node(i) => node_heading(s, data, i),
        Pick::Files(folder) => {
            let count = s.code.facts.as_ref().map_or(0, |f| code_map::own_files(f, folder));
            let about = match folder {
                Some(c) => trf!("in {folder}", folder = drawing::folder_name(&data.nodes[c])),
                None => tr("in the top folder").to_owned(),
            };
            (Mark::Icon(Icon::File, MUTED()), trn!(count, "{n} file", "{n} files"), about)
        }
        Pick::Package(p) => {
            let manifest = s.units.as_ref().and_then(|u| u.info.as_ref()).is_none_or(|i| i.manifest);
            (Mark::Icon(Icon::Package, TEAL()), modules::package_name(&data.nodes[p].label), modules::Words::of(s).package_kind(manifest).into())
        }
    }
}
fn node_heading(s: &MapPage, data: &RepositoryMap, i: usize) -> (Mark, String, String) {
    let n = &data.nodes[i];
    let name = label(&n.label);
    match n.kind {
        MapNodeKind::Crate if s.alternate => (Mark::Icon(Icon::Package, TEAL()), name, tr("Cargo package").into()),
        MapNodeKind::Crate => (Mark::Icon(Icon::Folder, TEAL()), drawing::folder_name(n), tr("Folder · Cargo package").into()),
        MapNodeKind::Directory => (Mark::Icon(Icon::Folder, TEAL()), drawing::folder_name(n), crate::i18n::tr("Folder").into()),
        MapNodeKind::File => (Mark::Icon(Icon::File, MUTED()), name, crate::i18n::tr("File").into()),
        MapNodeKind::Person => (Mark::Person(initials(&name)), name, tr("Contributor").into()),
        MapNodeKind::Area => {
            let title = match n.path.as_deref() {
                Some(b"/") => tr("Top folder").to_owned(),
                _ => area_name(data, i),
            };
            (Mark::Icon(Icon::Folder, TEAL()), title, crate::i18n::tr("Folder").into())
        }
        MapNodeKind::Commit if n.count > 1 => (Mark::Icon(Icon::Merge, BLUE()), name, tr("Merge commit").into()),
        MapNodeKind::Commit => (Mark::Icon(Icon::Commit, BLUE()), name, crate::i18n::tr("Commit").into()),
        MapNodeKind::Module => {
            let package = s.units.as_ref().and_then(|u| u.package).map(|p| modules::package_name(&data.nodes[p].label)).unwrap_or_default();
            let about = match n.detail.as_slice() {
                b"Crate root" => trf!("Crate root · {package}", package = package),
                b"Test module" => trf!("Test module · {package}", package = package),
                b"Package root" => trf!("Package root · {package}", package = package),
                b"Folder" => trf!("Folder · {package}", package = package),
                b"Go package" | b"Python package" | b"Java package" => trf!("Package · {package}", package = package),
                b"C# namespace" => trf!("Namespace · {package}", package = package),
                b"C source" | b"C header" | b"C++ source" | b"C++ header" | b"Objective-C source" | b"Objective-C++ source" | b"PowerShell script" | b"Bash script" | b"Lua script" | b"JSON file" | b"GitHub Actions workflow" | b"Referenced file" | b"HTML page" | b"Stylesheet" => trf!("File · {package}", package = package),
                _ => trf!("Module · {package}", package = package),
            };
            let mark = match n.detail.as_slice() { b"Folder" => Icon::Folder, b"Referenced file" => Icon::File, _ => Icon::Braces };
            (Mark::Icon(mark, TEAL()), modules::name(data, i), about)
        }
    }
}
fn single(text: &str, font: FontId, color: Color32, width: f32) -> text::LayoutJob {
    let mut job = text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = text::TextWrapping::truncate_at_width(width.max(1.));
    job
}
/// The card's title bar; true when the drawer's × was clicked.
fn header(
    ui: &mut Ui,
    mark: &Mark,
    title: &str,
    about: &str,
    drawer: bool,
    widgets: &mut HashMap<String, UiControl>,
) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 60.), Sense::hover());
    let painter = ui.painter().clone();
    let tile = Rect::from_center_size(pos2(rect.left() + 30., rect.center().y), vec2(32., 32.));
    match mark {
        Mark::Icon(image, tint) => {
            painter.rect_filled(tile, 8., tint.gamma_multiply(0.14));
            icon(&painter, tile.shrink(8.), *image, *tint);
        }
        Mark::Person(initials) => {
            painter.circle_filled(tile.center(), 16., AVATAR());
            painter.text(tile.center(), Align2::CENTER_CENTER, initials, semibold(11.), TEXT());
        }
    }
    let left = tile.right() + 12.;
    let right = rect.right() - if drawer { 46. } else { 14. };
    let title_job = painter.layout_job(single(title, semibold(14.), TEXT(), right - left));
    let cut = title_job.size().x >= right - left - 1.;
    painter.galley(pos2(left, rect.top() + 12.), title_job, TEXT());
    painter.galley(
        pos2(left, rect.top() + 32.),
        painter.layout_job(single(about, FontId::proportional(12.), HINT(), right - left)),
        HINT(),
    );
    if cut {
        let _ = response.on_hover_text(title);
    }
    painter.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1., CARD_STROKE()));
    if !drawer {
        return false;
    }
    let button = Rect::from_center_size(pos2(rect.right() - 24., rect.center().y), vec2(28., 28.));
    ui.scope_builder(UiBuilder::new().max_rect(button), |ui| {
        let r = crate::chrome::icon_button(ui, Icon::Close, true, false, 28.).on_hover_text(tr("Close details"));
        register(ui, widgets, "map-close-details", &r);
        r.clicked()
    })
    .inner
}
pub(super) fn fact(ui: &mut Ui, key: &str, value: &str) {
    pages::key_value(ui, key, KEY, |ui| {
        ui.add(Label::new(RichText::new(value).color(TEXT())).wrap());
    });
}
pub(super) fn fact_mono(ui: &mut Ui, key: &str, value: &str) {
    pages::key_value(ui, key, KEY, |ui| {
        ui.add(Label::new(RichText::new(value).monospace().size(12.5).color(TEXT())).wrap());
    });
}
pub(super) fn actions(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    ui.add_space(4.);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8., 8.);
        content(ui);
    });
}
pub(super) fn section(ui: &mut Ui, title: &str, count: usize) {
    ui.add_space(8.);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.), Sense::hover());
    ui.painter().text(rect.left_center(), Align2::LEFT_CENTER, title, semibold(11.), HINT());
    ui.painter().text(
        rect.right_center() - vec2(4., 0.),
        Align2::RIGHT_CENTER,
        pages::number_text(count),
        FontId::proportional(11.5),
        FAINT(),
    );
}
/// One clickable row: icon, name, an optional figure on the right and a chevron.
#[allow(clippy::too_many_arguments)]
pub(super) fn row(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    id: &str,
    image: Icon,
    tint: Color32,
    text: &str,
    right: &str,
    mono: bool,
    hover: &str,
) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 30.), Sense::click());
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, 6., HOVER());
    }
    icon(
        painter,
        Rect::from_center_size(pos2(rect.left() + 12., rect.center().y), vec2(13., 13.)),
        image,
        tint,
    );
    icon(
        painter,
        Rect::from_center_size(pos2(rect.right() - 10., rect.center().y), vec2(8., 8.)),
        Icon::ChevronRight,
        FAINT(),
    );
    let mut end = rect.right() - 22.;
    if !right.is_empty() {
        let font = if mono { FontId::monospace(11.5) } else { FontId::proportional(12.) };
        let galley = painter.layout_no_wrap(right.to_owned(), font, HINT());
        end -= galley.size().x;
        painter.galley(pos2(end, rect.center().y - galley.size().y / 2.), galley, HINT());
        end -= 10.;
    }
    let left = rect.left() + 28.;
    let galley = painter.layout_job(single(text, FontId::proportional(13.), TEXT(), end - left));
    painter.galley(pos2(left, rect.center().y - galley.size().y / 2.), galley, TEXT());
    let response = response.on_hover_text(if hover.is_empty() { text } else { hover });
    register(ui, widgets, id, &response);
    response.clicked()
}
/// A connection row named `map-link-<n>`.
#[allow(clippy::too_many_arguments)]
fn row_link(ui: &mut Ui, out: &mut Out, image: Icon, tint: Color32, text: &str, right: &str, mono: bool, hover: &str) -> bool {
    let id = out.link();
    row(ui, out.widgets, &id, image, tint, text, right, mono, hover)
}
pub(super) fn more(ui: &mut Ui, shown: usize, total: usize) {
    if total > shown {
        ui.label(RichText::new(trf!("{count} more", count = pages::number_text(total - shown))).size(12.).color(HINT()));
    }
}
/// A list of paths; a click shows the commits that changed one.
fn paths(ui: &mut Ui, out: &mut Out, title: &str, list: &[&Vec<u8>], total: usize, strip: &[u8], to_code: bool) {
    if list.is_empty() {
        return;
    }
    section(ui, title, total);
    for (k, path) in list.iter().take(ROWS).enumerate() {
        let shown = path
            .strip_prefix(strip)
            .and_then(|p| p.strip_prefix(b"/"))
            .unwrap_or(path);
        let hover = if to_code { tr("Show it in the folder map") } else { tr("Commits that changed this file") };
        let hover = format!("{}\n{hover}", label(path));
        if row(ui, out.widgets, &format!("map-path-{k}"), Icon::File, MUTED(), &label(shown), "", false, &hover) {
            out.emit(if to_code {
                MapIntent::Code(path.to_vec())
            } else {
                MapIntent::Path(path.to_vec(), String::new())
            });
        }
    }
    more(ui, list.len().min(ROWS), total);
}

fn node(ui: &mut Ui, s: &MapPage, data: &RepositoryMap, i: usize, out: &mut Out) {
    match data.nodes[i].kind {
        MapNodeKind::Crate if s.alternate => package(ui, s, data, i, out),
        MapNodeKind::Crate | MapNodeKind::Directory => folder(ui, s, data, i, out),
        MapNodeKind::File => {
            let path = data.nodes[i].path.clone().unwrap_or_default();
            fact_mono(ui, crate::i18n::tr("Path"), &label(&path));
            let folders = data.kind == MapKind::Code && !s.alternate;
            if folders {
                code_facts(ui, s, data, code_map::Item::Node(i));
            }
            actions(ui, |ui| {
                if button(ui, out.widgets, "map-history", tr("Commits here"), !path.is_empty()) {
                    out.emit(MapIntent::Path(path.clone(), String::new()));
                }
            });
            if folders {
                code_lists(ui, s, data, code_map::Item::Node(i), out);
            }
        }
        MapNodeKind::Person => person(ui, s, data, i, out),
        MapNodeKind::Area => area(ui, data, i, out),
        MapNodeKind::Commit => commit(ui, s, data, i, out),
        MapNodeKind::Module => {
            let mut chosen = None;
            modules::module_details(ui, s, data, i, &mut chosen, out.intents, out.widgets);
            if let Some(k) = chosen {
                *out.act = Some(Act::Module(k));
            }
        }
    }
}

fn package(ui: &mut Ui, s: &MapPage, data: &RepositoryMap, i: usize, out: &mut Out) {
    let n = &data.nodes[i];
    let path = n.path.clone().unwrap_or_default();
    let name = |k: &usize| label(&data.nodes[*k].label).to_lowercase();
    let collect = |outgoing: bool| {
        let mut list: Vec<usize> = s
            .projected_links
            .iter()
            .filter(|e| e.kind == MapLinkKind::DependsOn && e.from != e.to)
            .filter_map(|e| match outgoing {
                true if e.from == i => Some(e.to),
                false if e.to == i => Some(e.from),
                _ => None,
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        list.sort_by_key(name);
        list
    };
    let (uses, used) = (collect(true), collect(false));
    fact_mono(ui, crate::i18n::tr("Folder"), &label(&path));
    fact(ui, crate::i18n::tr("Files"), &pages::number_text(n.files));
    fact(ui, tr("Uses"), &if uses.is_empty() { tr("No local packages").into() } else { trn!(uses.len(), "{n} package", "{n} packages") });
    fact(ui, tr("Used by"), &if used.is_empty() { tr("No local packages").into() } else { trn!(used.len(), "{n} package", "{n} packages") });
    actions(ui, |ui| {
        if button(ui, out.widgets, "map-show-folders", crate::i18n::tr("Show folders"), !path.is_empty()) {
            out.emit(MapIntent::Code(path.clone()));
        }
        if button(ui, out.widgets, "map-show-modules", tr("Show modules"), true) {
            out.emit(MapIntent::Modules(Some(PackageRef {
                folder: modules::package_folder(n),
                language: ModuleLanguage::Rust,
            })));
        }
        if button(ui, out.widgets, "map-history", tr("Commits here"), !path.is_empty()) {
            out.emit(MapIntent::Path(path.clone(), String::new()));
        }
    });
    for (title, list, image) in [(tr("USES"), &uses, Icon::ArrowDown), (tr("USED BY"), &used, Icon::ArrowUp)] {
        if list.is_empty() {
            continue;
        }
        section(ui, title, list.len());
        for &k in list {
            if row_link(ui, out, image, BLUE(), &label(&data.nodes[k].label), "", false, "") {
                *out.act = Some(Act::Select(k));
            }
        }
    }
}

fn folder(ui: &mut Ui, s: &MapPage, data: &RepositoryMap, i: usize, out: &mut Out) {
    let n = &data.nodes[i];
    let path = n.path.clone().unwrap_or_default();
    let mut folders = Vec::new();
    let mut files = Vec::new();
    for e in data.links.iter().filter(|e| e.kind == MapLinkKind::Contains && e.from == i) {
        if data.nodes[e.to].kind == MapNodeKind::File {
            files.push(e.to);
        } else {
            folders.push(e.to);
        }
    }
    let name = |k: &usize| drawing::folder_name(&data.nodes[*k]).to_lowercase();
    folders.sort_by_key(name);
    files.sort_by_key(name);
    fact_mono(ui, crate::i18n::tr("Path"), &label(&path));
    let code = data.kind == MapKind::Code && !s.alternate;
    fact(
        ui,
        crate::i18n::tr("Files"),
        &if folders.is_empty() {
            pages::number_text(n.files)
        } else {
            trf!(
                "{total} · {here} here, {nested} in folders",
                total = pages::number_text(n.files),
                here = pages::number_text(files.len()),
                nested = pages::number_text(n.files.saturating_sub(files.len()))
            )
        },
    );
    if n.kind == MapNodeKind::Crate {
        pages::key_value(ui, tr("Package"), KEY, |ui| {
            let r = ui
                .add(Label::new(RichText::new(label(&n.label)).color(BLUE())).sense(Sense::click()))
                .on_hover_text(tr("Show it among the packages"));
            register(ui, out.widgets, "map-package", &r);
            if r.clicked() {
                *out.act = Some(Act::Package(i));
            }
        });
    }
    if code {
        code_facts(ui, s, data, code_map::Item::Node(i));
    }
    actions(ui, |ui| {
        if button(ui, out.widgets, "map-history", tr("Commits here"), !path.is_empty()) {
            out.emit(MapIntent::Path(path.clone(), String::new()));
        }
        if !folders.is_empty() || !files.is_empty() {
            let open = !s.collapsed.contains(&n.key);
            if button(ui, out.widgets, "map-group", if open { tr("Close folder") } else { crate::i18n::tr("Open folder") }, true) {
                *out.act = Some(Act::Toggle(i));
            }
        }
    });
    if code {
        code_lists(ui, s, data, code_map::Item::Node(i), out);
    }
    if !folders.is_empty() {
        section(ui, tr("FOLDERS"), folders.len());
        for &k in folders.iter().take(ROWS) {
            let count = pages::number_text(data.nodes[k].files);
            if row_link(ui, out, Icon::Folder, TEAL(), &drawing::folder_name(&data.nodes[k]), &count, false, "") {
                *out.act = Some(Act::Open(i, k));
            }
        }
        more(ui, folders.len().min(ROWS), folders.len());
    }
    let list: Vec<&Vec<u8>> = files.iter().filter_map(|&k| data.nodes[k].path.as_ref()).collect();
    paths(ui, out, tr("FILES HERE"), &list, list.len(), &path, false);
}

fn person(ui: &mut Ui, s: &MapPage, data: &RepositoryMap, i: usize, out: &mut Out) {
    let n = &data.nodes[i];
    let name = label(&n.label);
    fact(ui, crate::i18n::tr("Commits"), &pages::number_text(n.count));
    fact(ui, crate::i18n::tr("Files"), &pages::number_text(n.files));
    actions(ui, |ui| {
        let r = button(ui, out.widgets, "map-history", tr("Show commits"), true);
        if r {
            out.emit(MapIntent::History(name.clone()));
        }
    });
    let kind = if s.alternate { MapLinkKind::SharedFiles } else { MapLinkKind::Changes };
    let mut list: Vec<(usize, usize, String)> = data
        .links
        .iter()
        .filter(|e| e.kind == kind && (e.from == i || e.to == i))
        .map(|e| {
            let other = if e.from == i { e.to } else { e.from };
            let evidence = e.evidence.iter().map(|p| label(p)).collect::<Vec<_>>().join("\n");
            (other, e.weight, evidence)
        })
        .collect();
    list.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    if list.is_empty() {
        return;
    }
    section(ui, if s.alternate { tr("SHARES FILES WITH") } else { tr("FOLDERS") }, list.len());
    for (other, weight, evidence) in list.iter().take(ROWS) {
        if s.alternate {
            let hover = trf!("Files both changed (up to 24):\n{evidence}", evidence = evidence);
            if row_link(ui, out, Icon::User, VIOLET(), &label(&data.nodes[*other].label), &trn!(*weight, "{n} file", "{n} files"), false, &hover) {
                *out.act = Some(Act::Select(*other));
            }
        } else if row_link(ui, out, Icon::Folder, TEAL(), &area_name(data, *other), &pages::number_text(*weight), false, "") {
            *out.act = Some(Act::Cell(i, *other));
        }
    }
    more(ui, list.len().min(ROWS), list.len());
}

fn area(ui: &mut Ui, data: &RepositoryMap, a: usize, out: &mut Out) {
    let n = &data.nodes[a];
    let path = n.path.clone().unwrap_or_default();
    let top = path == b"/";
    let mut people: Vec<(usize, usize)> = data
        .links
        .iter()
        .filter(|e| e.kind == MapLinkKind::Changes && e.to == a)
        .map(|e| (e.from, e.weight))
        .collect();
    people.sort_by(|x, y| y.1.cmp(&x.1).then_with(|| x.0.cmp(&y.0)));
    fact(ui, crate::i18n::tr("Commits"), &pages::number_text(n.count));
    fact(ui, crate::i18n::tr("Files"), &pages::number_text(n.files));
    fact(ui, tr("People"), &pages::number_text(people.len()));
    if !top {
        actions(ui, |ui| {
            if button(ui, out.widgets, "map-history", tr("Commits here"), true) {
                out.emit(MapIntent::Path(path.clone(), String::new()));
            }
            if button(ui, out.widgets, "map-show-folder", tr("Show folder"), true) {
                out.emit(MapIntent::Code(path.clone()));
            }
        });
    }
    if people.is_empty() {
        return;
    }
    section(ui, tr("PEOPLE"), people.len());
    for &(p, weight) in people.iter().take(ROWS) {
        if row_link(ui, out, Icon::User, VIOLET(), &label(&data.nodes[p].label), &pages::number_text(weight), false, "") {
            *out.act = Some(Act::Cell(p, a));
        }
    }
}
/// A changed file inside an area of the People grid.
pub(super) fn in_area(file: &[u8], area: &[u8]) -> bool {
    if area == b"/" {
        !file.contains(&b'/')
    } else {
        file.len() > area.len() && file.starts_with(area) && file[area.len()] == b'/'
    }
}

fn cell(ui: &mut Ui, data: &RepositoryMap, p: usize, a: usize, out: &mut Out) {
    let person = &data.nodes[p];
    let name = label(&person.label);
    let path = data.nodes[a].path.clone().unwrap_or_default();
    let top = path == b"/";
    let here: usize = data
        .links
        .iter()
        .filter(|e| e.kind == MapLinkKind::Changes && e.from == p && e.to == a)
        .map(|e| e.weight)
        .sum();
    let files: Vec<&Vec<u8>> = person.evidence.iter().filter(|f| in_area(f, &path)).collect();
    fact(
        ui,
        crate::i18n::tr("Commits"),
        &trf!("{here} here · {total} in total", here = pages::number_text(here), total = pages::number_text(person.count)),
    );
    fact(ui, crate::i18n::tr("Files"), &trf!("{count} in {folder}", count = pages::number_text(files.len()), folder = area_name(data, a)));
    actions(ui, |ui| {
        if button(ui, out.widgets, "map-history", tr("Show commits"), true) {
            out.emit(if top {
                MapIntent::History(name.clone())
            } else {
                MapIntent::Path(path.clone(), name.clone())
            });
        }
        if !top && button(ui, out.widgets, "map-show-folder", tr("Show folder"), true) {
            out.emit(MapIntent::Code(path.clone()));
        }
    });
    let strip: &[u8] = if top { b"" } else { &path };
    paths(ui, out, tr("FILES CHANGED HERE"), &files, files.len(), strip, false);
}

fn commit(ui: &mut Ui, s: &MapPage, data: &RepositoryMap, i: usize, out: &mut Out) {
    let n = &data.nodes[i];
    let author = label(&n.detail);
    fact(ui, crate::i18n::tr("Author"), &author);
    let when = lists::date(data, i);
    if !when.is_empty() {
        fact(ui, tr("Date"), &when);
    }
    if let Some(id) = &n.commit {
        fact_mono(ui, crate::i18n::tr("Commit"), &id.as_str()[..12]);
    }
    if n.count > 1 {
        fact(ui, tr("Parents"), &pages::number_text(n.count));
    }
    if let Some((_, ahead)) = s.topology.as_ref().and_then(|t| t.tips.iter().find(|(tip, _)| *tip == i)) {
        fact(ui, tr("Ahead"), &trn!(ahead.len(), "{n} commit ahead of the main line", "{n} commits ahead of the main line"));
    }
    if !n.refs.is_empty() {
        pages::key_value(ui, tr("Refs"), KEY, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6., 6.);
                for r in n.refs.iter().take(12) {
                    if let Some(tag) = r.strip_prefix(b"refs/tags/") {
                        pages::pill(ui, &label(tag), AMBER());
                    } else {
                        pages::pill(ui, &label(r.strip_prefix(b"refs/heads/").unwrap_or(r)), BLUE());
                    }
                }
                if n.refs.len() > 12 {
                    ui.label(RichText::new(trf!("{count} more", count = n.refs.len() - 12)).color(HINT()));
                }
            });
        });
    }
    actions(ui, |ui| {
        if button(ui, out.widgets, "map-inspect-commit", tr("Open commit"), n.commit.is_some())
            && let Some(id) = &n.commit
        {
            out.emit(MapIntent::InspectCommit(id.clone()));
        }
        if button(ui, out.widgets, "map-person", tr("Find contributor"), !author.is_empty()) {
            out.emit(MapIntent::People(author.clone()));
        }
    });
    let list: Vec<&Vec<u8>> = n.evidence.iter().collect();
    paths(ui, out, tr("CHANGED PATHS"), &list, n.files.max(list.len()), b"", true);
}

/// Branches & merges: a merged branch, a commit worth naming, or a run of commits.
fn line(ui: &mut Ui, s: &MapPage, data: &RepositoryMap, l: usize, out: &mut Out) {
    let Some(topology) = s.topology.clone() else { return };
    match &topology.lines[l] {
        Line::Commit { node, merged } if !merged.is_empty() => {
            let n = &data.nodes[*node];
            fact(ui, tr("Into"), &main_name(s, data));
            let when = lists::date(data, *node);
            let by = label(&n.detail);
            fact(ui, tr("Merged"), &if when.is_empty() { trf!("by {author}", author = by) } else { trf!("{when} by {author}", when = when, author = by) });
            fact(
                ui,
                crate::i18n::tr("Commits"),
                &format!("{} · {}", pages::number_text(merged.len()), lists::authors(data, merged)),
            );
            let files: BTreeSet<&Vec<u8>> = merged.iter().flat_map(|&c| data.nodes[c].evidence.iter()).collect();
            fact(ui, crate::i18n::tr("Changes"), &trn!(files.len(), "{n} file", "{n} files"));
            if let Some(id) = &n.commit {
                fact_mono(ui, crate::i18n::tr("Merge"), &id.as_str()[..12]);
            }
            actions(ui, |ui| {
                if button(ui, out.widgets, "map-inspect-commit", tr("Open merge"), n.commit.is_some())
                    && let Some(id) = &n.commit
                {
                    out.emit(MapIntent::InspectCommit(id.clone()));
                }
            });
            commits(ui, data, merged, out);
        }
        Line::Commit { node, .. } => commit(ui, s, data, *node, out),
        Line::Run { nodes } => {
            fact(ui, crate::i18n::tr("Authors"), &lists::authors(data, nodes));
            let newest = lists::date(data, nodes[0]);
            let oldest = lists::date(data, nodes[nodes.len() - 1]);
            if !newest.is_empty() {
                fact(ui, tr("When"), &if newest == oldest { newest } else { trf!("{oldest} to {newest}", oldest = oldest, newest = newest) });
            }
            commits(ui, data, nodes, out);
        }
    }
}
fn commits(ui: &mut Ui, data: &RepositoryMap, list: &[usize], out: &mut Out) {
    section(ui, tr("COMMITS"), list.len());
    for (k, &c) in list.iter().take(ROWS).enumerate() {
        let n = &data.nodes[c];
        let id = n.commit.as_ref().map(|id| short(id).to_owned()).unwrap_or_default();
        let clicked = row(ui, out.widgets, &format!("map-commit-{k}"), Icon::Commit, BLUE(), &label(&n.label), &id, true, "");
        if clicked && let Some(id) = &n.commit {
            out.emit(MapIntent::InspectCommit(id.clone()));
        }
    }
    more(ui, list.len().min(ROWS), list.len());
}
/// The branch the main line follows: the mapped revision when it names one.
fn main_name(s: &MapPage, data: &RepositoryMap) -> String {
    let branches: Vec<String> = data
        .tip
        .as_ref()
        .and_then(|tip| data.nodes.iter().find(|n| n.commit.as_ref() == Some(tip)))
        .map(|n| n.refs.iter().filter_map(|r| r.strip_prefix(b"refs/heads/")).map(label).collect())
        .unwrap_or_default();
    if branches.contains(&s.revision) || branches.is_empty() {
        s.revision.clone()
    } else {
        branches[0].clone()
    }
}

/// What the read could not cover, folded away at the bottom.
fn notes(ui: &mut Ui, data: &RepositoryMap, widgets: &mut HashMap<String, UiControl>) {
    if data.notes.is_empty() {
        return;
    }
    ui.add_space(10.);
    let id = ui.make_persistent_id("map-notes-open");
    let open = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    let r = pages::disclosure(ui, open, tr("About this map"));
    register(ui, widgets, "map-notes", &r);
    if r.clicked() {
        ui.data_mut(|d| d.insert_temp(id, !open));
    }
    if open {
        for note in &data.notes {
            ui.add(Label::new(RichText::new(note).size(11.5).color(HINT())).wrap());
        }
    }
}

/// Code map: a folder's own files on one card.
fn files_card(ui: &mut Ui, s: &MapPage, data: &RepositoryMap, folder: Option<usize>, out: &mut Out) {
    let Some(f) = s.code.facts.clone() else { return };
    let list: Vec<usize> = match folder {
        Some(c) => f.files[c].clone(),
        None => f.top_files.clone(),
    };
    let item = code_map::Item::Files(folder);
    code_facts(ui, s, data, item);
    code_lists(ui, s, data, item, out);
    let paths_list: Vec<&Vec<u8>> = list.iter().filter_map(|&k| data.nodes[k].path.as_ref()).collect();
    let strip = folder.and_then(|c| data.nodes[c].path.clone()).unwrap_or_default();
    paths(ui, out, tr("FILES HERE"), &paths_list, paths_list.len(), &strip, false);
}

/// Code map facts that follow the colour: kinds, commits, owner or changes.
fn code_facts(ui: &mut Ui, s: &MapPage, data: &RepositoryMap, item: code_map::Item) {
    let (Some(picture), Some(f)) = (s.code.layout.as_ref(), s.code.facts.as_ref()) else { return };
    let placed = picture.index(item).map(|i| &picture.placed[i]);
    let path = code_map::item_path(data, item);
    let file = matches!(item, code_map::Item::Node(i) if code_map::is_file(&data.nodes[i]));
    match s.code.colour {
        code_map::Colour::Kind => {
            let sizes = match item {
                code_map::Item::Node(i) => f.sizes[i],
                code_map::Item::Files(Some(c)) => f.own[c],
                code_map::Item::Files(None) => f.top_own,
            };
            let total: u64 = sizes.iter().sum::<u64>().max(1);
            let mut parts: Vec<(u64, String)> = FileKind::ALL
                .iter()
                .enumerate()
                .filter(|(k, _)| sizes[*k] > 0)
                .map(|(k, kind)| (sizes[k], format!("{} {}%", code_map::kind_name(*kind), ((sizes[k] as f64 / total as f64) * 100.).round().max(1.))))
                .collect();
            parts.sort_by_key(|p| std::cmp::Reverse(p.0));
            if !parts.is_empty() {
                fact(ui, tr("Kind"), &parts.into_iter().map(|(_, t)| t).collect::<Vec<_>>().join(" · "));
            }
            if file && let Some(lines) = s.code.links.as_ref().and_then(|l| l.lines.get(&path)) {
                fact(ui, tr("Lines"), &pages::number_text(*lines));
            }
        }
        code_map::Colour::Activity => {
            let Some(h) = &s.code.history else { return };
            let since = s.code.period.since();
            let count = match item {
                code_map::Item::Node(_) if file => h.file_commits(&path, since),
                code_map::Item::Node(_) => h.folder_commits(&path, false, since),
                code_map::Item::Files(_) => h.folder_commits(&path, true, since),
            };
            let period = match s.code.period {
                Period::Days30 => crate::i18n::tr("30 days"),
                Period::Days90 => crate::i18n::tr("90 days"),
                Period::Year => tr("1 year"),
                Period::All => tr("All time"),
            };
            let (count, total) = (pages::number_text(count), pages::number_text(h.commits.len()));
            let text = if s.code.period == Period::All {
                trf!("{count} of {total}", count = count, total = total)
            } else {
                trf!("{count} in {period}, of {total}", count = count, period = period, total = total)
            };
            fact(ui, crate::i18n::tr("Commits"), &text);
            if let Some(last) = h.last(&path, !file) {
                fact(ui, tr("Last commit"), &format!("{} · {}", crate::style::date(last.time, 0), &last.id.as_str()[..8]));
            }
        }
        code_map::Colour::Owner => {
            if let Some(p) = placed {
                fact(ui, tr("Owner"), &p.paint.sub);
            }
            match s.code.links.as_ref().and_then(|l| l.owners.as_ref()) {
                Some(rules) => fact(ui, tr("Owners from"), &label(&rules.file)),
                None if file => {
                    if let Some(h) = &s.code.history
                        && let Some((_, commits)) = h.owner(&path)
                    {
                        fact(ui, tr("Owners from"), &trf!("Most commits: {count}", count = pages::number_text(commits)));
                    }
                }
                None => fact(ui, tr("Owners from"), tr("Most commits")),
            }
        }
        code_map::Colour::Changes => {
            let Some(changes) = &s.code.changes else { return };
            if file {
                if let Some(c) = changes.files.iter().find(|c| c.path.as_bytes() == path.as_slice()) {
                    let text = match c.change {
                        Change::Added => tr("Added"),
                        Change::Renamed => tr("Renamed"),
                        Change::Deleted => tr("Deleted"),
                        _ => tr("Changed"),
                    };
                    fact(ui, tr("Change"), text);
                }
                if let Some((a, r)) = changes.lines.get(&path) {
                    fact(ui, tr("Lines"), &format!("+{} \u{2212}{}", pages::number_text(*a), pages::number_text(*r)));
                }
            } else if let Some(p) = placed {
                fact(ui, tr("Change"), &p.paint.sub);
            }
            if let Some(split) = &changes.split {
                fact_mono(ui, tr("Since"), &format!("{} · {}", changes.base, &split.as_str()[..8]));
            }
        }
    }
}

/// Code map lists: what the card uses and what uses it, with the files on the using
/// side; in Activity, the most changed files under it.
fn code_lists(ui: &mut Ui, s: &MapPage, data: &RepositoryMap, item: code_map::Item, out: &mut Out) {
    let (Some(picture), Some(f)) = (s.code.layout.clone(), s.code.facts.clone()) else { return };
    if s.code.colour == code_map::Colour::Activity
        && let Some(h) = &s.code.history
    {
        // The files under the card, from the tree: only those, not every node.
        let mut files = Vec::new();
        if !matches!(item, code_map::Item::Node(i) if code_map::is_file(&data.nodes[i])) {
            code_map::under(&f, data, item, &mut files);
        }
        let since = s.code.period.since();
        let mut ranked: Vec<(usize, usize)> = files
            .into_iter()
            .map(|k| (h.file_commits(data.nodes[k].path.as_deref().unwrap_or_default(), since), k))
            .filter(|(c, _)| *c > 0)
            .collect();
        ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        if !ranked.is_empty() {
            section(ui, tr("MOST CHANGED"), ranked.len());
            for (count, k) in ranked.iter().take(5) {
                let hover = label(data.nodes[*k].path.as_deref().unwrap_or_default());
                if row_link(ui, out, Icon::File, MUTED(), &label(&data.nodes[*k].label), &trn!(*count, "{n} commit", "{n} commits"), false, &hover) {
                    *out.act = Some(Act::Item(code_map::Item::Node(*k)));
                }
            }
        }
    }
    let Some(at) = picture.index(item) else { return };
    let (uses, used) = picture.connections(at);
    for (title, list, forward) in [(tr("USES"), uses, true), (tr("USED BY"), used, false)] {
        if list.is_empty() {
            continue;
        }
        section(ui, title, list.len());
        for a in list.iter().take(ROWS) {
            let target = picture.placed[if forward { a.to } else { a.from }].item;
            let (image, tint, name) = code_map::title(data, &f, target);
            let name = match target {
                code_map::Item::Files(Some(c)) => trf!("{files} in {folder}", files = name, folder = drawing::folder_name(&data.nodes[c])),
                code_map::Item::Files(None) => trf!("{files} in the top folder", files = name),
                _ => name,
            };
            let hover = a.names.join(", ");
            if row_link(ui, out, image, tint, &name, &trn!(a.files, "{n} file", "{n} files"), false, &hover) {
                *out.act = Some(Act::Item(target));
            }
        }
        more(ui, list.len().min(ROWS), list.len());
    }
}
