//! Code → Modules: one Rust package's modules. The list beside the picture names every
//! module, most used first. The picture puts the chosen module between the modules that
//! use it and the ones it uses; a small package can also be drawn whole, in layers.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

/// Cards a side before the rest are counted on one card.
pub(super) const SIDE: usize = 8;
/// A package with this many top-level modules or fewer can be drawn whole.
pub(super) const WHOLE: usize = 40;
const LIST_WIDTH: f32 = 220.;
const LIST_ROW: f32 = 28.;
/// Columns and rows of the picture around one module, at 100% zoom.
const COLUMN: f32 = drawing::CARD.x + 64.;
const PITCH: f32 = drawing::CARD.y + 10.;
const CAPTION: f32 = 28.;
/// Outline rows Details shows before "N more".
const OUTLINE_ROWS: usize = 8;

/// Words drawn on the picture, translated where they are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptionKind {
    UsedBy,
    Uses,
    NotConnected,
}
#[derive(Clone, Debug)]
pub struct Caption {
    pub at: Pos2,
    pub what: CaptionKind,
    pub count: usize,
}
/// Which side a "N more modules" card counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum More {
    Users,
    Uses,
}

/// The module tree of one read, worked out once per read.
pub(crate) struct Units {
    pub package: Option<usize>,
    pub root: Option<usize>,
    /// Each node's line from the crate root down to it (itself alone when outside it).
    chains: Vec<Vec<usize>>,
    children: HashMap<usize, Vec<usize>>,
    /// The crate root's modules, by name.
    pub top: Vec<usize>,
    /// Modules that use each module, counted as Details lists them.
    pub used_by: HashMap<usize, usize>,
    /// A module with the modules inside it: files and lines.
    size: HashMap<usize, (usize, usize)>,
    /// Connections between the crate root's modules.
    pub top_links: usize,
}
/// One module beside the chosen one, and the names joining them.
pub(super) struct Neighbor {
    pub unit: usize,
    pub names: BTreeSet<Vec<u8>>,
}

impl Units {
    pub(super) fn new(data: &RepositoryMap) -> Self {
        let n = data.nodes.len();
        let module = |i: usize| data.nodes[i].kind == MapNodeKind::Module;
        let mut parent = vec![None; n];
        let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
        for e in data.links.iter().filter(|e| e.kind == MapLinkKind::Contains && module(e.from) && module(e.to)) {
            parent[e.to] = Some(e.from);
            children.entry(e.from).or_default().push(e.to);
        }
        let name = |i: &usize| label(&data.nodes[*i].label).to_lowercase();
        for list in children.values_mut() {
            list.sort_by_key(name);
        }
        let details = data.modules.as_ref();
        let root = details.and_then(|d| d.root).filter(|&r| r < n);
        let top = root.and_then(|r| children.get(&r).cloned()).unwrap_or_default();
        let chains = (0..n)
            .map(|mut i| {
                let mut line = vec![i];
                while let Some(p) = parent[i] {
                    if line.len() > 64 {
                        break;
                    }
                    line.push(p);
                    i = p;
                }
                line.reverse();
                line
            })
            .collect();
        let mut units = Self {
            package: details.map(|d| d.package).filter(|&p| p < n),
            root,
            chains,
            children,
            top,
            used_by: HashMap::new(),
            size: HashMap::new(),
            top_links: 0,
        };
        // Sizes from the leaves up.
        let modules: Vec<usize> = (0..n).filter(|&i| module(i)).collect();
        let mut order = modules.clone();
        order.sort_by_key(|&i| std::cmp::Reverse(units.depth(i)));
        for &i in &order {
            let own = (1, data.nodes[i].count);
            let inside = if Some(i) == units.root {
                (0, 0)
            } else {
                units.children.get(&i).map_or((0, 0), |kids| {
                    kids.iter().fold((0, 0), |sum, k| {
                        let (f, l) = units.size.get(k).copied().unwrap_or_default();
                        (sum.0 + f, sum.1 + l)
                    })
                })
            };
            units.size.insert(i, (own.0 + inside.0, own.1 + inside.1));
        }
        // Users of every module in one pass, counted as neighbors() lists them: a link
        // into a module or anything inside it, from outside it.
        let mut users: HashMap<usize, HashSet<usize>> = HashMap::new();
        for e in data.links.iter().filter(|e| e.kind == MapLinkKind::DependsOn && e.to < n && e.from < n) {
            for &x in &units.chains[e.to] {
                if Some(x) == root && x != e.to || units.within(e.from, x) {
                    continue;
                }
                users.entry(x).or_default().insert(units.unit_of(e.from, x));
            }
        }
        for &i in &modules {
            units.used_by.insert(i, users.get(&i).map_or(0, HashSet::len));
        }
        let mut pairs = BTreeSet::new();
        for e in data.links.iter().filter(|e| e.kind == MapLinkKind::DependsOn) {
            let (a, b) = (units.top_of(e.from), units.top_of(e.to));
            if a != b && Some(a) != root && Some(b) != root {
                pairs.insert((a, b));
            }
        }
        units.top_links = pairs.len();
        units
    }
    fn depth(&self, i: usize) -> usize {
        self.chains[i].len() - 1
    }
    /// The crate root's module that holds `i`; the root for the root itself.
    fn top_of(&self, i: usize) -> usize {
        let chain = &self.chains[i];
        chain.get(1).copied().unwrap_or(chain[0])
    }
    /// What the picture shows for `f` beside `chosen`: an ancestor as itself, anything
    /// else as the module on its side of the nearest module they share.
    fn unit_of(&self, f: usize, chosen: usize) -> usize {
        let (theirs, ours) = (&self.chains[f], &self.chains[chosen]);
        if ours.contains(&f) {
            return f;
        }
        let shared = theirs.iter().zip(ours).take_while(|(a, b)| a == b).count();
        theirs.get(shared).copied().unwrap_or(f)
    }
    /// Whether `x` is `chosen` or inside it. The crate root stands for its own file only.
    fn within(&self, x: usize, chosen: usize) -> bool {
        x == chosen || Some(chosen) != self.root && self.chains[x].contains(&chosen)
    }
    pub(super) fn children(&self, i: usize) -> &[usize] {
        self.children.get(&i).map(Vec::as_slice).unwrap_or_default()
    }
    pub(super) fn size(&self, i: usize) -> (usize, usize) {
        self.size.get(&i).copied().unwrap_or((1, 0))
    }
    /// The modules that use `chosen` and the ones it uses, most names first.
    pub(super) fn neighbors(&self, data: &RepositoryMap, chosen: usize) -> (Vec<Neighbor>, Vec<Neighbor>) {
        let mut users: BTreeMap<usize, BTreeSet<Vec<u8>>> = BTreeMap::new();
        let mut uses: BTreeMap<usize, BTreeSet<Vec<u8>>> = BTreeMap::new();
        for e in data.links.iter().filter(|e| e.kind == MapLinkKind::DependsOn) {
            let (from, to) = (self.within(e.from, chosen), self.within(e.to, chosen));
            if from && !to {
                uses.entry(self.unit_of(e.to, chosen)).or_default().extend(e.evidence.iter().cloned());
            } else if to && !from {
                users.entry(self.unit_of(e.from, chosen)).or_default().extend(e.evidence.iter().cloned());
            }
        }
        let order = |map: BTreeMap<usize, BTreeSet<Vec<u8>>>| {
            let mut list: Vec<Neighbor> = map.into_iter().map(|(unit, names)| Neighbor { unit, names }).collect();
            list.sort_by(|a, b| {
                b.names
                    .len()
                    .cmp(&a.names.len())
                    .then_with(|| name(data, a.unit).to_lowercase().cmp(&name(data, b.unit).to_lowercase()))
            });
            list
        };
        (order(users), order(uses))
    }
}

/// A module as the map names it: its path in the crate, or the root's file name.
pub(super) fn name(data: &RepositoryMap, i: usize) -> String {
    let n = &data.nodes[i];
    match n.key.strip_prefix(b"module:") {
        Some(path) if !path.is_empty() => label(path),
        _ => label(&n.label),
    }
}
/// "2 files · 2,169 lines", or "1,251 lines" for one file.
pub(super) fn size_text(units: &Units, i: usize) -> String {
    let (files, lines) = units.size(i);
    let lines = trn!(lines, "{found} line", "{found} lines", found = pages::number_text(lines));
    if files > 1 {
        format!("{} · {lines}", trn!(files, "{n} file", "{n} files"))
    } else {
        lines
    }
}
/// Names in a short line: "PickTarget, pin_directory +1".
pub(super) fn names_text(names: &[&Vec<u8>], fits: usize) -> String {
    let shown: Vec<String> = names.iter().take(fits).map(|n| label(n)).collect();
    let mut text = shown.join(", ");
    if names.len() > fits {
        text.push_str(&format!(" +{}", names.len() - fits));
    }
    text
}

impl MapPage {
    /// The module tree for this read, built on first use.
    pub(super) fn units(&mut self) -> Option<Arc<Units>> {
        let data = self.data.as_ref()?;
        if data.kind != MapKind::Modules {
            return None;
        }
        if self.units.is_none() {
            self.units = Some(Arc::new(Units::new(data)));
        }
        self.units.clone()
    }
    /// After a read: keep what was chosen when the same package was read again, else
    /// start on the whole package when it can be drawn, or on the most used module.
    pub(super) fn start_modules(&mut self, previous: Option<Vec<u8>>, was_whole: bool) {
        self.units = None;
        let Some(units) = self.units() else { return };
        let data = self.data.clone().expect("modules data");
        self.package = data.scope.package.clone();
        self.expanded.retain(|key| data.nodes.iter().any(|n| &n.key == key));
        let kept = previous.and_then(|key| data.nodes.iter().position(|n| n.key == key && n.kind == MapNodeKind::Module));
        let small = units.top.len() <= WHOLE;
        if small && (was_whole || kept.is_none()) {
            self.whole = true;
            self.selected = None;
        } else {
            self.whole = false;
            self.selected = kept.or_else(|| most_used(&units).first().copied()).or(units.root);
        }
        self.list_jump = true;
    }
}
fn most_used(units: &Units) -> Vec<usize> {
    let mut list = units.top.clone();
    list.sort_by_key(|i| std::cmp::Reverse(units.used_by.get(i).copied().unwrap_or(0)));
    list
}

/// Visible cards, links, positions and captions for the picture.
pub(super) fn rebuild(s: &mut MapPage) {
    let key = (s.query.clone(), s.alternate, s.selected, s.whole);
    if s.filter_key.as_ref() == Some(&key) {
        return;
    }
    let Some(units) = s.units() else { return };
    let data = s.data.clone().expect("modules data");
    // Search picks the first module that matches when the chosen one doesn't.
    let query = s.query.to_lowercase();
    // Only when the search changes: a module chosen while it is on stays chosen.
    let searched = s.filter_key.as_ref().is_none_or(|key| key.0 != s.query);
    if searched && !query.is_empty() && !s.whole {
        let matches = |i: usize| name(&data, i).to_lowercase().contains(&query);
        if !s.selected.is_some_and(matches) {
            let rows = rows(&data, &units, s, &query);
            if let Some(row) = rows.iter().find(|r| matches(r.node)) {
                s.selected = Some(row.node);
                s.list_jump = true;
            }
        }
    }
    s.filter_key = Some((s.query.clone(), s.alternate, s.selected, s.whole));
    s.positions = vec![Pos2::ZERO; data.nodes.len()];
    s.file_groups.clear();
    s.captions.clear();
    s.projected_links.clear();
    s.visible.clear();
    s.display_offset = 0;
    if s.whole {
        whole(s, &data, &units);
    } else if let Some(chosen) = s.selected.filter(|&i| data.nodes.get(i).is_some_and(|n| n.kind == MapNodeKind::Module)) {
        around(s, &data, &units, chosen);
    }
    s.view_total = s.visible.len();
    s.topology = None;
    s.fit = true;
}

/// The chosen module in the middle, its users on the left, what it uses on the right.
fn around(s: &mut MapPage, data: &RepositoryMap, units: &Units, chosen: usize) {
    let (users, uses) = units.neighbors(data, chosen);
    // A module that uses the chosen one and is used by it is drawn once, among the
    // users, with a connection each way; the captions still count it on both sides.
    let mutual: HashSet<usize> = users.iter().map(|n| n.unit).filter(|u| uses.iter().any(|n| n.unit == *u)).collect();
    let right: Vec<&Neighbor> = uses.iter().filter(|n| !mutual.contains(&n.unit)).collect();
    let left: Vec<&Neighbor> = users.iter().collect();
    let shown = |count: usize| count.min(SIDE) + usize::from(count > SIDE);
    let rows = shown(left.len()).max(shown(right.len())).max(1);
    let height = rows as f32 * PITCH - 10.;
    let top = |count: usize| CAPTION + (height - (count as f32 * PITCH - 10.)) / 2.;
    s.visible.push(chosen);
    s.positions[chosen] = pos2(COLUMN, CAPTION + (height - drawing::CARD.y) / 2.);
    for (list, total, x, side, what) in [(&left, users.len(), 0., More::Users, CaptionKind::UsedBy), (&right, uses.len(), 2. * COLUMN, More::Uses, CaptionKind::Uses)] {
        let y0 = top(shown(list.len()).max(1));
        s.captions.push(Caption {
            at: pos2(x, y0 - 24.),
            what,
            count: total,
        });
        for (k, n) in list.iter().take(SIDE).enumerate() {
            s.visible.push(n.unit);
            s.positions[n.unit] = pos2(x, y0 + k as f32 * PITCH);
        }
        if list.len() > SIDE {
            s.file_groups.push(FileGroup {
                folder: Some(chosen),
                files: list.len() - SIDE,
                about: String::new(),
                at: pos2(x, y0 + SIDE as f32 * PITCH),
                more: Some(side),
            });
        }
    }
    let shown: HashSet<usize> = s.visible.iter().copied().collect();
    for (list, incoming) in [(&users, true), (&uses, false)] {
        for n in list.iter().filter(|n| shown.contains(&n.unit)) {
            let (from, to) = if incoming { (n.unit, chosen) } else { (chosen, n.unit) };
            s.projected_links.push(MapLink {
                from,
                to,
                kind: MapLinkKind::DependsOn,
                weight: n.names.len(),
                evidence: n.names.iter().cloned().collect(),
            });
        }
    }
}

/// A small package's modules in layers: what a module uses sits below it. Modules with
/// no connection sit apart, under the rest.
fn whole(s: &mut MapPage, data: &RepositoryMap, units: &Units) {
    if units.top.len() > WHOLE {
        return;
    }
    let mut names: BTreeMap<(usize, usize), BTreeSet<Vec<u8>>> = BTreeMap::new();
    for e in data.links.iter().filter(|e| e.kind == MapLinkKind::DependsOn) {
        let (a, b) = (units.top_of(e.from), units.top_of(e.to));
        if a != b && Some(a) != units.root && Some(b) != units.root {
            names.entry((a, b)).or_default().extend(e.evidence.iter().cloned());
        }
    }
    let linked: BTreeSet<usize> = names.keys().flat_map(|(a, b)| [*a, *b]).collect();
    let (connected, apart): (Vec<usize>, Vec<usize>) = units.top.iter().partition(|i| linked.contains(i));
    let edges: Vec<(usize, usize)> = names.keys().copied().collect();
    drawing::layers(data, &connected, &edges, &mut s.positions);
    wrap(&connected, &mut s.positions);
    let bottom = connected.iter().map(|&i| s.positions[i].y).fold(None, |m: Option<f32>, y| Some(m.map_or(y, |m| m.max(y))));
    if !apart.is_empty() {
        let y = bottom.map_or(CAPTION, |b| b + drawing::CARD.y + 76.);
        s.captions.push(Caption {
            at: pos2(0., y - 24.),
            what: CaptionKind::NotConnected,
            count: apart.len(),
        });
        for (k, &i) in apart.iter().enumerate() {
            let (row, column) = (k / ROW_CARDS, k % ROW_CARDS);
            s.positions[i] = pos2(column as f32 * (drawing::CARD.x + 26.), y + row as f32 * (drawing::CARD.y + 30.));
        }
    }
    s.visible = units.top.clone();
    s.projected_links = names
        .into_iter()
        .map(|((from, to), names)| MapLink {
            from,
            to,
            kind: MapLinkKind::DependsOn,
            weight: names.len(),
            evidence: names.into_iter().collect(),
        })
        .collect();
}

/// Cards a row of the whole package holds; a wider layer goes on to the next row, so a
/// package with many modules side by side still draws them large enough to read.
const ROW_CARDS: usize = 4;
fn wrap(nodes: &[usize], positions: &mut [Pos2]) {
    let mut layers: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
    for &i in nodes {
        layers.entry(positions[i].y.round() as i64).or_default().push(i);
    }
    let step = drawing::CARD.x + 26.;
    let widest = layers.values().map(|l| l.len().min(ROW_CARDS)).max().unwrap_or(1) as f32;
    let mut y = 0.;
    for row in layers.values_mut() {
        row.sort_by(|a, b| positions[*a].x.total_cmp(&positions[*b].x));
        for chunk in row.chunks(ROW_CARDS) {
            let offset = (widest - chunk.len() as f32) * step / 2.;
            for (k, &i) in chunk.iter().enumerate() {
                positions[i] = pos2(offset + k as f32 * step, y);
            }
            y += drawing::CARD.y + 30.;
        }
        // Layers keep the wider gap between them that their connections run through.
        y += 46.;
    }
}

/// One row of the list.
pub(super) struct Row {
    pub node: usize,
    pub depth: usize,
    pub folder: bool,
    pub open: bool,
}
/// The list's rows: the crate root's modules, and the inside of open ones. A search
/// keeps a module that matches or holds one, and opens what holds one.
pub(super) fn rows(data: &RepositoryMap, units: &Units, s: &MapPage, query: &str) -> Vec<Row> {
    let matches = |i: usize| query.is_empty() || name(data, i).to_lowercase().contains(query);
    fn holds(units: &Units, i: usize, matches: &dyn Fn(usize) -> bool, depth: usize) -> bool {
        depth < 64 && units.children(i).iter().any(|&k| matches(k) || holds(units, k, matches, depth + 1))
    }
    let mut top: Vec<usize> = units.top.iter().copied().chain(units.root).collect();
    if s.by_name {
        top.sort_by_key(|&i| name(data, i).to_lowercase());
    } else {
        top.sort_by(|a, b| {
            let used = |i: &usize| units.used_by.get(i).copied().unwrap_or(0);
            used(b).cmp(&used(a)).then_with(|| name(data, *a).to_lowercase().cmp(&name(data, *b).to_lowercase()))
        });
    }
    let mut out = Vec::new();
    #[allow(clippy::too_many_arguments)]
    fn add(out: &mut Vec<Row>, data: &RepositoryMap, units: &Units, s: &MapPage, i: usize, depth: usize, query: &str, matches: &dyn Fn(usize) -> bool) {
        // The crate root holds every module, so the list never opens it.
        let kids = if Some(i) == units.root { &[][..] } else { units.children(i) };
        let inside = !query.is_empty() && kids.iter().any(|&k| matches(k) || holds(units, k, matches, 1));
        if !matches(i) && !inside {
            return;
        }
        let open = !kids.is_empty() && (s.expanded.contains(&data.nodes[i].key) || inside);
        out.push(Row {
            node: i,
            depth,
            folder: !kids.is_empty(),
            open,
        });
        if open && depth < 32 {
            for &k in kids {
                add(out, data, units, s, k, depth + 1, query, matches);
            }
        }
    }
    for i in top {
        add(&mut out, data, units, s, i, 0, query, &matches);
    }
    out
}

/// What a click in the list changes, once the list is drawn.
enum Pick {
    Whole,
    Module(usize),
    Toggle(usize),
    Order,
}
fn apply(s: &mut MapPage, pick: Pick) {
    match pick {
        Pick::Whole => {
            s.whole = true;
            s.selected = None;
        }
        Pick::Module(i) => {
            s.whole = false;
            s.selected = Some(i);
            s.show_details = true;
        }
        Pick::Toggle(i) => {
            if let Some(key) = s.data.as_ref().map(|d| d.nodes[i].key.clone())
                && !s.expanded.remove(&key)
            {
                s.expanded.insert(key);
            }
        }
        Pick::Order => s.by_name = !s.by_name,
    }
}

/// The list in its own column, or folded into a button above the picture in a narrow
/// window; then the picture.
pub(super) fn body(ui: &mut Ui, s: &mut MapPage, rect: Rect, covered: bool, compact: bool, intents: &mut Vec<Intent>, widgets: &mut HashMap<String, UiControl>) {
    let Some(data) = s.data.clone() else {
        drawing::canvas(ui, s, rect, covered, widgets);
        return;
    };
    let Some(units) = s.units() else { return };
    if units.package.is_none() {
        let mut card = ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Center)));
        card.add_space((rect.height() / 2. - 52.).max(8.));
        icon(card.painter(), Rect::from_center_size(card.cursor().center_top() + vec2(0., 11.), vec2(22., 22.)), Icon::Braces, FAINT());
        card.add_space(28.);
        card.label(RichText::new(tr("No Rust package in this snapshot")).font(semibold(14.)).color(TEXT()));
        card.label(RichText::new(tr("Modules reads Rust packages.")).color(MUTED()));
        card.add_space(6.);
        if button(&mut card, widgets, "map-modules-show-folders", crate::i18n::tr("Show folders"), !covered) {
            emit(intents, MapIntent::CodeView(false));
        }
        return;
    }
    let picture = if compact {
        let bar = Rect::from_min_max(rect.min, pos2(rect.right(), rect.top() + 46.));
        switcher(ui, s, &data, &units, bar, covered, widgets);
        Rect::from_min_max(pos2(rect.left(), bar.bottom()), rect.max)
    } else {
        let list = Rect::from_min_max(rect.min, pos2(rect.left() + LIST_WIDTH, rect.bottom()));
        ui.painter().with_clip_rect(rect).rect_filled(list, CornerRadius { nw: 7, sw: 7, ne: 0, se: 0 }, VIEW_PANEL());
        ui.painter().with_clip_rect(rect).vline(list.right(), list.y_range(), Stroke::new(1., CARD_STROKE()));
        let mut child = ui.new_child(UiBuilder::new().max_rect(list).layout(Layout::top_down(Align::Min)));
        child.set_clip_rect(list.intersect(ui.clip_rect()));
        if let Some(pick) = list_panel(&mut child, s, &data, &units, covered, widgets) {
            apply(s, pick);
        }
        Rect::from_min_max(pos2(list.right() + 1., rect.top()), rect.max)
    };
    let root_missing = units.root.is_none();
    if root_missing || s.whole && units.top.len() > WHOLE {
        let painter = ui.painter().with_clip_rect(picture);
        let text = if root_missing {
            tr("This package has no src/lib.rs or src/main.rs").to_owned()
        } else {
            trf!("{count} modules are too many to draw at once. Choose one from the list", count = pages::number_text(units.top.len()))
        };
        let galley = painter.layout(text, FontId::proportional(13.), MUTED(), (picture.width() - 48.).max(40.));
        painter.galley(picture.center() - galley.size() / 2., galley, MUTED());
        if !covered {
            register(ui, widgets, "map-canvas", &ui.interact(picture, ui.id().with("map-modules-message"), Sense::hover()));
        }
        return;
    }
    drawing::canvas(ui, s, picture, covered, widgets);
}

/// The list: a header with the count and the order, Whole package, then the modules.
fn list_panel(ui: &mut Ui, s: &mut MapPage, data: &RepositoryMap, units: &Units, covered: bool, widgets: &mut HashMap<String, UiControl>) -> Option<Pick> {
    let mut pick = None;
    let width = ui.available_width();
    let (head, _) = ui.allocate_exact_size(vec2(width, 38.), Sense::hover());
    let painter = ui.painter().clone();
    let caption = painter.layout_no_wrap(tr("MODULES").to_owned(), semibold(11.), HINT());
    let caption_width = caption.size().x;
    painter.galley(pos2(head.left() + 12., head.center().y - caption.size().y / 2.), caption, HINT());
    painter.text(
        pos2(head.left() + 18. + caption_width, head.center().y),
        Align2::LEFT_CENTER,
        pages::number_text(units.top.len()),
        FontId::proportional(12.),
        FAINT(),
    );
    let order = if s.by_name { tr("A to Z") } else { tr("Most used") };
    let order_rect = Rect::from_min_max(pos2(head.right() - 8. - text_width(ui, order) - 16., head.top() + 7.), pos2(head.right() - 8., head.bottom() - 7.));
    let r = ui
        .scope_builder(UiBuilder::new().max_rect(order_rect), |ui| {
            ui.add(Button::new(RichText::new(order).size(12.).color(MUTED())).frame(false).min_size(order_rect.size()))
        })
        .inner
        .on_hover_text(tr("Order the modules by how many use them, or by name"));
    register(ui, widgets, "map-module-order", &r);
    if r.clicked() && !covered {
        pick = Some(Pick::Order);
    }
    let (columns, _) = ui.allocate_exact_size(vec2(width, 20.), Sense::hover());
    painter.text(pos2(columns.left() + 26., columns.center().y), Align2::LEFT_CENTER, tr("Module"), FontId::proportional(11.), FAINT());
    painter.text(pos2(columns.right() - 12., columns.center().y), Align2::RIGHT_CENTER, tr("Used by"), FontId::proportional(11.), FAINT());
    painter.hline(columns.x_range(), columns.bottom() - 0.5, Stroke::new(1., CARD_STROKE()));
    // Always listed: a package too large to draw whole still shows its facts in Details.
    {
        let (r, response) = ui.allocate_exact_size(vec2(width, 32.), Sense::click());
        if s.whole {
            painter.rect_filled(r, 0., SELECTED());
            painter.rect_filled(Rect::from_min_size(r.min, vec2(3., r.height())), 0., BLUE());
        } else if response.hovered() {
            painter.rect_filled(r, 0., HOVER());
        }
        icon(&painter, Rect::from_center_size(pos2(r.left() + 15., r.center().y), vec2(12., 12.)), Icon::Package, if s.whole { BLUE() } else { FAINT() });
        painter.text(pos2(r.left() + 30., r.center().y), Align2::LEFT_CENTER, tr("Whole package"), if s.whole { semibold(13.) } else { FontId::proportional(13.) }, TEXT());
        painter.hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1., CARD_STROKE()));
        let response = response.on_hover_text(tr("Every module of the package and how they connect"));
        register(ui, widgets, "map-module-whole", &response);
        if response.clicked() && !covered {
            pick = Some(Pick::Whole);
        }
    }
    let query = s.query.to_lowercase();
    let list = rows(data, units, s, &query);
    if list.is_empty() {
        ui.add_space(12.);
        ui.vertical_centered(|ui| ui.label(RichText::new(tr("No module matches")).size(12.5).color(HINT())));
        return pick;
    }
    let jump = std::mem::take(&mut s.list_jump);
    let mut area = ScrollArea::vertical().id_salt("map-modules-list").auto_shrink([false, false]);
    if jump && let Some(at) = list.iter().position(|r| Some(r.node) == s.selected) {
        let view = ui.available_height();
        area = area.vertical_scroll_offset((at as f32 * LIST_ROW - view / 2. + LIST_ROW).max(0.));
    }
    let output = area.show_rows(ui, LIST_ROW, list.len(), |ui, range| {
        for row in &list[range] {
            let (r, response) = ui.allocate_exact_size(vec2(ui.available_width(), LIST_ROW), Sense::click());
            let painter = ui.painter();
            let on = Some(row.node) == s.selected && !s.whole;
            if on {
                painter.rect_filled(r, 0., SELECTED());
                painter.rect_filled(Rect::from_min_size(r.min, vec2(3., r.height())), 0., BLUE());
            } else if response.hovered() {
                painter.rect_filled(r, 0., HOVER());
            }
            let left = r.left() + 8. + row.depth as f32 * 16.;
            let twisty = Rect::from_center_size(pos2(left + 6., r.center().y), vec2(14., 14.));
            if row.folder {
                icon(painter, twisty.shrink(3.), if row.open { Icon::Chevron } else { Icon::ChevronRight }, FAINT());
            }
            let n = &data.nodes[row.node];
            let count = units.used_by.get(&row.node).copied().unwrap_or(0);
            let count_galley = painter.layout_no_wrap(pages::number_text(count), FontId::proportional(12.), if on { ACCENT_TEXT() } else { FAINT() });
            let count_x = r.right() - 12. - count_galley.size().x;
            let mut end = count_x - 8.;
            let test = n.detail == b"Test module";
            if test {
                let pill = painter.layout_no_wrap(tr("test").to_owned(), FontId::proportional(10.), MUTED());
                let rect = Rect::from_min_size(pos2(end - pill.size().x - 12., r.center().y - 8.), vec2(pill.size().x + 12., 16.));
                painter.rect_filled(rect, 8., HOVER());
                painter.galley(rect.center() - pill.size() / 2., pill, MUTED());
                end = rect.left() - 6.;
            }
            let font = if on { semibold(13.) } else { FontId::proportional(13.) };
            let mut job = text::LayoutJob::simple_singleline(label(&n.label), font, if row.depth > 0 && !on { MUTED() } else { TEXT() });
            job.wrap = text::TextWrapping::truncate_at_width((end - left - 16.).max(10.));
            let galley = painter.layout_job(job);
            painter.galley(pos2(left + 16., r.center().y - galley.size().y / 2.), galley, TEXT());
            painter.galley(pos2(count_x, r.center().y - 7.), count_galley, FAINT());
            let response = response.on_hover_text(format!("{}\n{}", name(data, row.node), trn!(count, "Used by {n} module", "Used by {n} modules")));
            register(ui, widgets, &format!("map-module-{}", row.node), &response);
            if row.folder {
                let toggle = ui.interact(twisty.expand(4.).intersect(r), ui.id().with(("map-module-toggle", row.node)), Sense::click());
                register(ui, widgets, &format!("map-module-toggle-{}", row.node), &toggle);
                if toggle.clicked() && !covered {
                    pick = Some(Pick::Toggle(row.node));
                    continue;
                }
            }
            if response.clicked() && !covered {
                pick = Some(Pick::Module(row.node));
            }
        }
    });
    pages::region(widgets, "map-modules-list", output.inner_rect.intersect(ui.clip_rect()), !covered);
    pick
}

/// A narrow window's list: a button naming the chosen module that opens the list.
fn switcher(ui: &mut Ui, s: &mut MapPage, data: &RepositoryMap, units: &Units, bar: Rect, covered: bool, widgets: &mut HashMap<String, UiControl>) {
    let chosen = if s.whole {
        tr("Whole package").to_owned()
    } else {
        s.selected.map(|i| name(data, i)).unwrap_or_default()
    };
    let used = s.selected.filter(|_| !s.whole).and_then(|i| units.used_by.get(&i).copied());
    let detail = used.map(|n| trf!("used by {count}", count = n)).unwrap_or_default();
    let width = (text_width(ui, &chosen) + text_width(ui, &detail) + 64.).clamp(140., (bar.width() - 24.).max(140.));
    let button_rect = Rect::from_min_size(bar.left_top() + vec2(12., 9.), vec2(width, CONTROL));
    let mut child = ui.new_child(UiBuilder::new().max_rect(bar).layout(Layout::left_to_right(Align::Center)));
    let r = child
        .scope_builder(UiBuilder::new().max_rect(button_rect).id_salt("map-module-switcher"), |ui| ui.add_enabled(!covered, Button::new("").min_size(button_rect.size())))
        .inner
        .on_hover_text(tr("Choose a module"));
    let painter = ui.painter().with_clip_rect(bar);
    icon(&painter, Rect::from_center_size(pos2(r.rect.left() + 15., r.rect.center().y), vec2(13., 13.)), if s.whole { Icon::Package } else { Icon::Braces }, TEAL());
    let text = painter.layout_job(drawing::title_job(chosen, 1., (r.rect.width() - 72.).max(20.)));
    let text_size = text.size();
    painter.galley(pos2(r.rect.left() + 29., r.rect.center().y - text_size.y / 2.), text, TEXT());
    painter.text(pos2(r.rect.left() + 35. + text_size.x, r.rect.center().y), Align2::LEFT_CENTER, &detail, FontId::proportional(12.), FAINT());
    icon(&painter, Rect::from_center_size(pos2(r.rect.right() - 14., r.rect.center().y), vec2(11., 11.)), Icon::Chevron, FAINT());
    let order = if s.by_name { tr("A to Z") } else { tr("most used first") };
    let note = trf!("{count} modules, {order}", count = pages::number_text(units.top.len()), order = order);
    if r.rect.right() + 12. + text_width(ui, &note) < bar.right() - 12. {
        painter.text(pos2(r.rect.right() + 12., r.rect.center().y), Align2::LEFT_CENTER, note, FontId::proportional(12.), FAINT());
    }
    register(ui, widgets, "map-module-switcher", &r);
    let mut pick = None;
    Popup::menu(&r).close_behavior(PopupCloseBehavior::CloseOnClickOutside).width(300.).show(|ui| {
        ui.set_max_height(340.);
        pick = list_panel(ui, s, data, units, covered, widgets);
        if matches!(pick, Some(Pick::Whole | Pick::Module(_))) {
            ui.close();
        }
    });
    if let Some(pick) = pick {
        apply(s, pick);
    }
}

/// The package button and its list of the snapshot's Rust packages.
pub(super) fn package_picker(ui: &mut Ui, s: &MapPage, packages: &[(String, Vec<u8>, Option<usize>)], intents: &mut Vec<Intent>, widgets: &mut HashMap<String, UiControl>) {
    let current = packages.iter().find(|(_, folder, _)| Some(folder) == s.package.as_ref());
    let text = current.map_or_else(|| tr("Package").to_owned(), |(name, ..)| name.clone());
    let width = (text_width(ui, &text) + 58.).min(240.);
    room(ui, width);
    let control_id = ui.make_persistent_id(("map-control", "map-module-package"));
    let r = ui
        .scope_builder(UiBuilder::new().id(control_id), |ui| ui.add_enabled(!packages.is_empty(), Button::new("").min_size(vec2(width, CONTROL))))
        .inner
        .on_hover_text(tr("The Rust package to map"));
    let painter = ui.painter();
    icon(painter, Rect::from_center_size(pos2(r.rect.left() + 15., r.rect.center().y), vec2(13., 13.)), Icon::Package, TEAL());
    painter.with_clip_rect(r.rect.shrink2(vec2(26., 0.))).text(pos2(r.rect.left() + 28., r.rect.center().y), Align2::LEFT_CENTER, &text, FontId::proportional(12.5), TEXT());
    icon(painter, Rect::from_center_size(pos2(r.rect.right() - 13., r.rect.center().y), vec2(11., 11.)), Icon::Chevron, FAINT());
    register(ui, widgets, "map-module-package", &r);
    Popup::menu(&r).width(280.).show(|ui| {
        ScrollArea::vertical().id_salt("map-module-packages").max_height(360.).show(ui, |ui| {
            for (k, (name, folder, files)) in packages.iter().enumerate() {
                let on = Some(folder) == s.package.as_ref();
                // Before Modules reads, the files are not counted yet.
                let usable = *files != Some(0);
                let r = ui.add_enabled(usable, Button::selectable(on, "").min_size(vec2(ui.available_width(), 30.)));
                let painter = ui.painter();
                icon(painter, Rect::from_center_size(pos2(r.rect.left() + 14., r.rect.center().y), vec2(13., 13.)), Icon::Package, if usable { TEAL() } else { FAINT() });
                painter.text(pos2(r.rect.left() + 30., r.rect.center().y), Align2::LEFT_CENTER, name, FontId::proportional(12.5), if usable { TEXT() } else { FAINT() });
                let count = match *files {
                    Some(0) => tr("No Rust files").to_owned(),
                    Some(n) => trn!(n, "{n} Rust file", "{n} Rust files"),
                    None => String::new(),
                };
                painter.text(pos2(r.rect.right() - 10., r.rect.center().y), Align2::RIGHT_CENTER, count, FontId::proportional(12.), FAINT());
                let r = r.on_hover_text(if folder.is_empty() { tr("the top folder").to_owned() } else { label(folder) });
                register(ui, widgets, &format!("map-module-package-{k}"), &r);
                if r.clicked() && !on {
                    emit(intents, MapIntent::Modules(Some(folder.clone())));
                    ui.close();
                }
            }
        });
    });
}

/// The package's folder as the Modules read names it: a package at the top of the
/// repository is its Cargo.toml in Packages and Folders.
pub(super) fn package_folder(n: &MapNode) -> Vec<u8> {
    match n.path.as_deref() {
        Some(b"Cargo.toml") | None => Vec::new(),
        Some(path) => path.to_vec(),
    }
}

/// A module's card subtitle: its size in the middle and in the whole package, else
/// the names that join it to the chosen module.
pub(super) fn card_detail(s: &MapPage, data: &RepositoryMap, i: usize) -> String {
    let Some(units) = s.units.as_ref() else { return String::new() };
    if s.whole || s.selected == Some(i) {
        return size_text(units, i);
    }
    s.projected_links
        .iter()
        .find(|e| e.from == i || e.to == i)
        .map(|e| names_text(&e.evidence.iter().collect::<Vec<_>>(), 2))
        .unwrap_or_else(|| label(&data.nodes[i].detail))
}
/// "folder_browser uses 7 names from style", and the names.
pub(super) fn hover(s: &MapPage, data: &RepositoryMap, i: usize) -> String {
    if s.whole || s.selected == Some(i) {
        return format!("{}\n{}", name(data, i), card_detail(s, data, i));
    }
    let Some(e) = s.projected_links.iter().find(|e| e.from == i || e.to == i) else {
        return name(data, i);
    };
    let names: Vec<String> = e.evidence.iter().map(|n| label(n)).collect();
    let mut text = trn!(e.weight, "{user} uses {n} name from {module}", "{user} uses {n} names from {module}", user = name(data, e.from), module = name(data, e.to));
    text.push('\n');
    text.push_str(&names.join(", "));
    if e.weight > names.len() {
        text.push_str(&format!(" +{}", e.weight - names.len()));
    }
    text
}

/// What a "N more modules" card says.
pub(super) fn more_text(group: &FileGroup) -> (String, String) {
    (trn!(group.files, "{n} more module", "{n} more modules"), tr("listed in Details").into())
}
pub(super) fn caption_text(c: &Caption) -> String {
    let title = match c.what {
        CaptionKind::UsedBy => tr("USED BY"),
        CaptionKind::Uses => tr("USES"),
        CaptionKind::NotConnected => tr("NOT CONNECTED"),
    };
    format!("{title}  {}", pages::number_text(c.count))
}

// ---- Details -------------------------------------------------------------------

/// Details for one module: its file and size, what is in the file, what it uses and
/// what uses it.
pub(super) fn module_details(ui: &mut Ui, s: &MapPage, data: &RepositoryMap, i: usize, select: &mut Option<usize>, intents: &mut Vec<Intent>, widgets: &mut HashMap<String, UiControl>) {
    let Some(units) = s.units.clone() else { return };
    let n = &data.nodes[i];
    let file = n.path.clone().unwrap_or_default();
    let (users, uses) = units.neighbors(data, i);
    details::fact_mono(ui, crate::i18n::tr("File"), &label(&file));
    details::fact(ui, tr("Size"), &size_text(&units, i));
    let count = |n: usize| if n == 0 { tr("None").to_owned() } else { trn!(n, "{n} module", "{n} modules") };
    details::fact(ui, tr("Uses"), &count(uses.len()));
    details::fact(ui, tr("Used by"), &count(users.len()));
    details::actions(ui, |ui| {
        let folder = file.rsplitn(2, |b| *b == b'/').nth(1).unwrap_or_default().to_vec();
        if button(ui, widgets, "map-show-folder", tr("Show folder"), !folder.is_empty()) {
            emit(intents, MapIntent::Code(folder));
        }
        if button(ui, widgets, "map-history", tr("Commits here"), !file.is_empty()) {
            emit(intents, MapIntent::Path(file.clone(), String::new()));
        }
    });
    if let Some(outline) = data.modules.as_ref().and_then(|d| d.outline.get(&i)) {
        outline_section(ui, data, i, outline, widgets);
    }
    for (title, list, id, image) in [(tr("USES"), &uses, "map-module-section-uses", Icon::ArrowDown), (tr("USED BY"), &users, "map-module-section-used-by", Icon::ArrowUp)] {
        if list.is_empty() {
            continue;
        }
        if !section(ui, title, list.len(), id, false, widgets) {
            continue;
        }
        for (k, nb) in list.iter().take(100).enumerate() {
            let names: Vec<&Vec<u8>> = nb.names.iter().collect();
            let hover = format!("{}\n{}", name(data, nb.unit), names_text(&names, 12));
            if details::row(ui, widgets, &format!("{id}-{k}"), image, BLUE(), &name(data, nb.unit), &pages::number_text(nb.names.len()), false, &hover) {
                *select = Some(nb.unit);
            }
        }
        details::more(ui, list.len().min(100), list.len());
    }
}

/// Where a folded section keeps whether it is open, in egui's memory.
fn section_key(id: &str) -> Id {
    Id::new(("map-module-section", id))
}
/// A folded section heading; true while it is open.
fn section(ui: &mut Ui, title: &str, count: usize, id: &str, open_at_first: bool, widgets: &mut HashMap<String, UiControl>) -> bool {
    ui.add_space(6.);
    let key = section_key(id);
    let open = ui.data(|d| d.get_temp::<bool>(key)).unwrap_or(open_at_first);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 24.), Sense::click());
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, 5., HOVER());
    }
    icon(painter, Rect::from_center_size(pos2(rect.left() + 6., rect.center().y), vec2(9., 9.)), if open { Icon::Chevron } else { Icon::ChevronRight }, FAINT());
    painter.text(pos2(rect.left() + 16., rect.center().y), Align2::LEFT_CENTER, title, semibold(11.), HINT());
    painter.text(rect.right_center() - vec2(4., 0.), Align2::RIGHT_CENTER, pages::number_text(count), FontId::proportional(11.5), FAINT());
    register(ui, widgets, id, &response);
    if response.clicked() {
        ui.data_mut(|d| d.insert_temp(key, !open));
    }
    open != response.clicked()
}
/// A "N more modules" card opens the Details section that lists them all.
pub(super) fn open_section(ctx: &Context, more: More) {
    let id = match more {
        More::Users => "map-module-section-used-by",
        More::Uses => "map-module-section-uses",
    };
    ctx.data_mut(|d| d.insert_temp(section_key(id), true));
}

/// "In this file": public items first, each type with its own functions counted.
fn outline_section(ui: &mut Ui, data: &RepositoryMap, i: usize, outline: &[OutlineItem], widgets: &mut HashMap<String, UiControl>) {
    let inherent: HashMap<&[u8], usize> = outline.iter().filter(|o| o.kind == OutlineKind::Impl && o.of_trait.is_none()).fold(HashMap::new(), |mut m, o| {
        *m.entry(o.name.as_slice()).or_default() += o.functions;
        m
    });
    let declared: HashSet<&[u8]> = outline.iter().filter(|o| o.kind != OutlineKind::Impl).map(|o| o.name.as_slice()).collect();
    let mut shown: Vec<&OutlineItem> = outline
        .iter()
        .filter(|o| match o.kind {
            // Trait impls are left out; an inherent impl counts on its type's row.
            OutlineKind::Impl => o.of_trait.is_none() && !declared.contains(o.name.as_slice()),
            _ => true,
        })
        .collect();
    shown.sort_by_key(|o| o.visibility.is_empty());
    if shown.is_empty() {
        return;
    }
    if !section(ui, tr("IN THIS FILE"), shown.len(), "map-module-section-outline", true, widgets) {
        return;
    }
    let all_key = ui.make_persistent_id(("map-outline-all", data.nodes[i].key.clone()));
    let all = ui.data(|d| d.get_temp::<bool>(all_key)).unwrap_or(false);
    let limit = if all { shown.len().min(400) } else { OUTLINE_ROWS };
    for (k, o) in shown.iter().take(limit).enumerate() {
        let keyword = match o.kind {
            OutlineKind::Function => "fn",
            OutlineKind::Struct => "struct",
            OutlineKind::Enum => "enum",
            OutlineKind::Union => "union",
            OutlineKind::Trait => "trait",
            OutlineKind::Type => "type",
            OutlineKind::Const => "const",
            OutlineKind::Static => "static",
            OutlineKind::Macro => "macro_rules!",
            OutlineKind::Impl => "impl",
        };
        let functions = if o.kind == OutlineKind::Impl { o.functions } else { inherent.get(o.name.as_slice()).copied().unwrap_or(0) };
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 24.), Sense::hover());
        let painter = ui.painter();
        let mut job = text::LayoutJob::default();
        let mono = |size: f32| FontId::monospace(size);
        if !o.visibility.is_empty() {
            job.append(&format!("{} ", label(&o.visibility)), 0., TextFormat::simple(mono(12.), SYNTAX_KEYWORD()));
        }
        job.append(&format!("{keyword} "), 0., TextFormat::simple(mono(12.), SYNTAX_KEYWORD()));
        job.append(&label(&o.name), 0., TextFormat::simple(mono(12.), TEXT()));
        let mut end = rect.right() - 6.;
        if functions > 0 {
            let galley = painter.layout_no_wrap(trn!(functions, "{n} function", "{n} functions"), FontId::proportional(12.), HINT());
            end -= galley.size().x;
            painter.galley(pos2(end, rect.center().y - galley.size().y / 2.), galley, HINT());
            end -= 8.;
        }
        job.wrap = text::TextWrapping::truncate_at_width((end - rect.left() - 6.).max(10.));
        let galley = painter.layout_job(job);
        painter.galley(pos2(rect.left() + 6., rect.center().y - galley.size().y / 2.), galley, TEXT());
        register(ui, widgets, &format!("map-outline-{k}"), &response);
    }
    if shown.len() > limit {
        let r = ui.add(Button::new(RichText::new(trf!("{count} more", count = pages::number_text(shown.len() - limit))).size(12.).color(ACCENT_TEXT())).frame(false));
        register(ui, widgets, "map-outline-more", &r);
        if r.clicked() {
            ui.data_mut(|d| d.insert_temp(all_key, true));
        }
    }
}

/// Details for the whole package: its folder and counts, the most used modules, those
/// with no connection and the files no module is read from.
pub(super) fn package_details(ui: &mut Ui, s: &MapPage, data: &RepositoryMap, select: &mut Option<usize>, intents: &mut Vec<Intent>, widgets: &mut HashMap<String, UiControl>) {
    let Some(units) = s.units.clone() else { return };
    let Some(package) = units.package else { return };
    let n = &data.nodes[package];
    let folder = n.path.clone().unwrap_or_default();
    details::fact_mono(ui, crate::i18n::tr("Folder"), &if folder.is_empty() { tr("the top folder").to_owned() } else { label(&folder) });
    details::fact(ui, crate::i18n::tr("Files"), &trn!(n.count, "{n} Rust file", "{n} Rust files"));
    details::fact(ui, tr("Modules"), &pages::number_text(units.top.len()));
    details::fact(ui, tr("Links"), &pages::number_text(units.top_links));
    details::actions(ui, |ui| {
        if button(ui, widgets, "map-show-folders", crate::i18n::tr("Show folders"), true) {
            emit(intents, MapIntent::Code(folder.clone()));
        }
        if button(ui, widgets, "map-history", tr("Commits here"), !folder.is_empty()) {
            emit(intents, MapIntent::Path(folder.clone(), String::new()));
        }
    });
    let unlinked = data.modules.as_ref().map(|d| d.unlinked.as_slice()).unwrap_or_default();
    if !unlinked.is_empty() {
        ui.add_space(4.);
        pages::notice_strip(ui, AMBER(), &trn!(unlinked.len(), "{n} file is not linked", "{n} files are not linked"), None);
    }
    let used: Vec<usize> = most_used(&units).into_iter().filter(|i| units.used_by.get(i).copied().unwrap_or(0) > 0).collect();
    if !used.is_empty() {
        details::section(ui, tr("MOST USED"), used.len());
        for (k, &i) in used.iter().take(5).enumerate() {
            let count = trf!("used by {count}", count = pages::number_text(units.used_by.get(&i).copied().unwrap_or(0)));
            if details::row(ui, widgets, &format!("map-module-most-{k}"), Icon::Braces, TEAL(), &name(data, i), &count, false, "") {
                *select = Some(i);
            }
        }
        details::more(ui, used.len().min(5), used.len());
    }
    let mut connected = HashSet::new();
    for e in data.links.iter().filter(|e| e.kind == MapLinkKind::DependsOn) {
        let (a, b) = (units.top_of(e.from), units.top_of(e.to));
        if a != b {
            connected.extend([a, b]);
        }
    }
    let apart: Vec<usize> = units.top.iter().copied().filter(|i| !connected.contains(i)).collect();
    if !apart.is_empty() {
        details::section(ui, tr("NOT CONNECTED"), apart.len());
        for (k, &i) in apart.iter().take(100).enumerate() {
            if details::row(ui, widgets, &format!("map-module-apart-{k}"), Icon::Braces, TEAL(), &name(data, i), "", false, "") {
                *select = Some(i);
            }
        }
        details::more(ui, apart.len().min(100), apart.len());
    }
    if !unlinked.is_empty() {
        details::section(ui, tr("NOT LINKED"), unlinked.len());
        let src = if folder.is_empty() { b"src/".to_vec() } else { [folder.as_slice(), b"/src/"].concat() };
        for (k, (path, reason)) in unlinked.iter().take(100).enumerate() {
            let why = match reason {
                UnlinkedReason::Program => tr("its own program"),
                UnlinkedReason::NotDeclared => tr("not declared"),
                UnlinkedReason::TooLarge => tr("over 1 MiB"),
                UnlinkedReason::NotUtf8 => tr("not UTF-8"),
                UnlinkedReason::Limit => tr("over the read limits"),
            };
            let shown = path.strip_prefix(src.as_slice()).unwrap_or(path);
            let hover = format!("{}\n{}", label(path), tr("Commits that changed this file"));
            if details::row(ui, widgets, &format!("map-path-{k}"), Icon::File, MUTED(), &label(shown), why, false, &hover) {
                emit(intents, MapIntent::Path(path.clone(), String::new()));
            }
        }
        details::more(ui, unlinked.len().min(100), unlinked.len());
    }
}
