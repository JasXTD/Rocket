//! The pannable map: packages in layers, folders as a tree, shared files in a ring.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

/// One card at 100% zoom.
pub(super) const CARD: Vec2 = vec2(184., 58.);
const PACKAGE_GAP: Vec2 = vec2(26., 76.);
const FOLDER_COLUMN: f32 = 218.;
const FOLDER_ROW: f32 = 66.;

/// A folder's own name; a Cargo package's folder can be named unlike its package.
pub(super) fn folder_name(n: &MapNode) -> String {
    n.path
        .as_deref()
        .and_then(|p| p.rsplit(|b| *b == b'/').find(|part| !part.is_empty()))
        .map(label)
        .unwrap_or_else(|| label(&n.label))
}

/// Positions at 100% zoom, and the "N files" cards a folder tree shows.
pub(super) fn layout(
    data: &RepositoryMap,
    nodes: &[usize],
    alternate: bool,
    collapsed: &HashSet<Vec<u8>>,
) -> (Vec<Pos2>, Vec<FileGroup>) {
    let mut positions = vec![Pos2::ZERO; data.nodes.len()];
    let mut groups = Vec::new();
    match data.kind {
        MapKind::History => {
            // Index parent edges once. Retain first-parent order and the leftmost
            // free lane without rescanning every edge or active lane per node.
            let mut parents = vec![Vec::new(); data.nodes.len()];
            for edge in &data.links {
                if edge.kind == MapLinkKind::Parent {
                    parents[edge.to].push(edge.from);
                }
            }
            let mut node_lanes = vec![None; data.nodes.len()];
            let mut free_lanes = (0..data.nodes.len()).collect::<BTreeSet<_>>();
            for (row, &i) in nodes.iter().enumerate() {
                // Each active node owns at most one lane, so an unassigned node
                // always has a free slot among the snapshot's node-count slots.
                let lane = node_lanes[i].take().unwrap_or_else(|| {
                    free_lanes
                        .pop_first()
                        .expect("free lane for unassigned node")
                });
                positions[i] = pos2(lane as f32 * 260., row as f32 * 94.);
                free_lanes.insert(lane);
                for (p, &parent) in parents[i].iter().enumerate() {
                    if node_lanes[parent].is_some() {
                        continue;
                    }
                    let parent_lane = if p == 0 {
                        free_lanes.remove(&lane);
                        lane
                    } else {
                        free_lanes
                            .pop_first()
                            .expect("free lane for unassigned parent")
                    };
                    node_lanes[parent] = Some(parent_lane);
                }
            }
        }
        MapKind::People if alternate => {
            let n = nodes.len().max(1) as f32;
            let radius = (n * CARD.x / std::f32::consts::TAU * 1.25).max(170.);
            for (k, &i) in nodes.iter().enumerate() {
                let angle = std::f32::consts::TAU * k as f32 / n - std::f32::consts::FRAC_PI_2;
                positions[i] = pos2(radius + radius * angle.cos(), radius + radius * angle.sin());
            }
        }
        MapKind::People => {
            let mut person = 0;
            let mut area = 0;
            for &i in nodes {
                if data.nodes[i].kind == MapNodeKind::Person {
                    positions[i] = pos2(0., person as f32 * 96.);
                    person += 1;
                } else {
                    positions[i] = pos2(390., area as f32 * 96.);
                    area += 1;
                }
            }
        }
        MapKind::Code if alternate => packages(data, nodes, &mut positions),
        MapKind::Code => groups = folders(data, nodes, collapsed, &mut positions),
        // Modules lays out its own picture (modules::rebuild).
        MapKind::Modules => {}
    }
    (positions, groups)
}

fn packages(data: &RepositoryMap, nodes: &[usize], positions: &mut [Pos2]) {
    let set: HashSet<usize> = nodes.iter().copied().collect();
    let edges: Vec<(usize, usize)> = data
        .links
        .iter()
        .filter(|e| e.kind == MapLinkKind::DependsOn && set.contains(&e.from) && set.contains(&e.to) && e.from != e.to)
        .map(|e| (e.from, e.to))
        .collect();
    layers(data, nodes, &edges, positions);
}

/// Connections that close a circle (modules can use each other; packages cannot) are
/// left out of the layering. Depth first from the biggest users, a connection back to
/// a card still being walked closes one.
fn without_circles(data: &RepositoryMap, nodes: &[usize], edges: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut out: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut net: HashMap<usize, i64> = HashMap::new();
    for &(a, b) in edges {
        out.entry(a).or_default().push(b);
        *net.entry(a).or_default() += 1;
        *net.entry(b).or_default() -= 1;
    }
    let name = |i: usize| label(&data.nodes[i].label).to_lowercase();
    let mut order = nodes.to_vec();
    order.sort_by(|a, b| net.get(b).copied().unwrap_or(0).cmp(&net.get(a).copied().unwrap_or(0)).then_with(|| name(*a).cmp(&name(*b))));
    for list in out.values_mut() {
        list.sort_by_key(|&i| name(i));
    }
    // 0: not reached, 1: on the walk, 2: done.
    let mut state: HashMap<usize, u8> = HashMap::new();
    let mut kept = Vec::new();
    for &start in &order {
        if state.contains_key(&start) {
            continue;
        }
        let mut stack = vec![(start, 0usize)];
        state.insert(start, 1);
        while let Some(&mut (node, ref mut next)) = stack.last_mut() {
            let targets = out.get(&node).map(Vec::as_slice).unwrap_or_default();
            if let Some(&to) = targets.get(*next) {
                *next += 1;
                match state.get(&to) {
                    Some(1) => {}
                    Some(_) => kept.push((node, to)),
                    None => {
                        kept.push((node, to));
                        state.insert(to, 1);
                        stack.push((to, 0));
                    }
                }
            } else {
                state.insert(node, 2);
                stack.pop();
            }
        }
    }
    kept
}

/// Layers so that what a card uses sits below it: longest path from cards nothing
/// uses. Within a layer, cards sit under the ones that use them.
pub(super) fn layers(data: &RepositoryMap, nodes: &[usize], edges: &[(usize, usize)], positions: &mut [Pos2]) {
    let all = edges;
    let edges = without_circles(data, nodes, edges);
    let mut layer: HashMap<usize, usize> = nodes.iter().map(|&i| (i, 0)).collect();
    // Bounded relaxation over what is left, which no longer closes a circle.
    for _ in 0..nodes.len().min(64) {
        let mut changed = false;
        for &(user, used) in &edges {
            let next = layer[&user] + 1;
            if layer[&used] < next && next <= nodes.len() {
                layer.insert(used, next);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut layers: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for &i in nodes {
        layers.entry(layer[&i]).or_default().push(i);
    }
    let name = |i: usize| label(&data.nodes[i].label).to_lowercase();
    let mut order: HashMap<usize, f32> = HashMap::new();
    for row in layers.values_mut() {
        row.sort_by_key(|&i| name(i));
        // Barycenter of the cards that use each one, from layers above.
        let weight = |i: usize| {
            let users: Vec<f32> = all.iter().filter(|(_, used)| *used == i).filter_map(|(user, _)| order.get(user).copied()).collect();
            if users.is_empty() {
                f32::MAX
            } else {
                users.iter().sum::<f32>() / users.len() as f32
            }
        };
        let mut keyed: Vec<(f32, usize)> = row.iter().map(|&i| (weight(i), i)).collect();
        keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| name(a.1).cmp(&name(b.1))));
        *row = keyed.into_iter().map(|(_, i)| i).collect();
        let count = row.len() as f32;
        for (k, &i) in row.iter().enumerate() {
            order.insert(i, k as f32 - (count - 1.) / 2.);
        }
    }
    let widest = layers.values().map(Vec::len).max().unwrap_or(1) as f32;
    let step = CARD.x + PACKAGE_GAP.x;
    for (depth, row) in &layers {
        let offset = (widest - row.len() as f32) * step / 2.;
        for (k, &i) in row.iter().enumerate() {
            positions[i] = pos2(offset + k as f32 * step, *depth as f32 * (CARD.y + PACKAGE_GAP.y));
        }
    }
}

/// A tree read left to right. A closed folder takes one row; an open one centres on
/// its subfolders and a card counting its own files.
fn folders(
    data: &RepositoryMap,
    nodes: &[usize],
    collapsed: &HashSet<Vec<u8>>,
    positions: &mut [Pos2],
) -> Vec<FileGroup> {
    let set: HashSet<usize> = nodes.iter().copied().collect();
    let mut parent = vec![None; data.nodes.len()];
    // Each folder's own files: how many, and the names of the first few.
    let mut files: Vec<(usize, Vec<String>)> = vec![(0, Vec::new()); data.nodes.len()];
    let mut top: (usize, Vec<String>) = (0, Vec::new());
    let mut has_parent = vec![false; data.nodes.len()];
    let count = |own: &mut (usize, Vec<String>), file: usize| {
        own.0 += 1;
        if own.1.len() < 4 {
            own.1.push(label(&data.nodes[file].label));
        }
    };
    for e in data.links.iter().filter(|e| e.kind == MapLinkKind::Contains) {
        has_parent[e.to] = true;
        if data.nodes[e.to].kind == MapNodeKind::File {
            count(&mut files[e.from], e.to);
        } else if set.contains(&e.from) && set.contains(&e.to) {
            parent[e.to] = Some(e.from);
        }
    }
    for (i, n) in data.nodes.iter().enumerate() {
        if n.kind == MapNodeKind::File && !has_parent[i] {
            count(&mut top, i);
        }
    }
    // Up to three files are named; more are counted in their folder.
    let about = |own: &(usize, Vec<String>)| {
        if own.0 <= 3 {
            let mut names = own.1.clone();
            names.sort_by_key(|n| n.to_lowercase());
            names.join(", ")
        } else {
            String::new()
        }
    };
    let own: Vec<(usize, String)> = files
        .iter()
        .map(|f| (f.0, about(f)))
        .collect();
    let mut children: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    let mut roots = Vec::new();
    for &i in nodes {
        match parent[i] {
            Some(p) => children.entry(p).or_default().push(i),
            None => roots.push(i),
        }
    }
    let name = |i: &usize| folder_name(&data.nodes[*i]).to_lowercase();
    roots.sort_by_key(name);
    for list in children.values_mut() {
        list.sort_by_key(name);
    }
    let mut groups = Vec::new();
    let mut next = 0f32;
    #[allow(clippy::too_many_arguments)]
    fn place(
        i: usize,
        depth: usize,
        data: &RepositoryMap,
        children: &BTreeMap<usize, Vec<usize>>,
        files: &[(usize, String)],
        collapsed: &HashSet<Vec<u8>>,
        positions: &mut [Pos2],
        groups: &mut Vec<FileGroup>,
        next: &mut f32,
    ) -> f32 {
        let open = !collapsed.contains(&data.nodes[i].key);
        let kids = children.get(&i).map(Vec::as_slice).unwrap_or_default();
        let x = depth as f32 * FOLDER_COLUMN;
        if !open || kids.is_empty() && files[i].0 == 0 {
            let y = *next * FOLDER_ROW;
            *next += 1.;
            positions[i] = pos2(x, y);
            return y;
        }
        let mut ys = Vec::new();
        for &k in kids {
            ys.push(place(k, depth + 1, data, children, files, collapsed, positions, groups, next));
        }
        if files[i].0 > 0 {
            let y = *next * FOLDER_ROW;
            *next += 1.;
            groups.push(FileGroup {
                folder: Some(i),
                files: files[i].0,
                about: files[i].1.clone(),
                at: pos2(x + FOLDER_COLUMN, y),
                more: None,
            });
            ys.push(y);
        }
        let y = (ys[0] + ys[ys.len() - 1]) / 2.;
        positions[i] = pos2(x, y);
        y
    }
    for &r in &roots {
        place(r, 0, data, &children, &own, collapsed, positions, &mut groups, &mut next);
    }
    if top.0 > 0 {
        groups.push(FileGroup {
            folder: None,
            files: top.0,
            about: about(&top),
            at: pos2(0., next * FOLDER_ROW),
            more: None,
        });
    }
    groups
}

/// Keep cached layout language-neutral. Switching Preferences must update the
/// caption on the next frame without another Git read or moving the graph.
pub(super) fn group_about(group: &FileGroup, data: &RepositoryMap) -> String {
    if group.files <= 3 {
        group.about.clone()
    } else if let Some(folder) = group.folder {
        trf!("in {folder}", folder = folder_name(&data.nodes[folder]))
    } else {
        tr("in the top folder").into()
    }
}

fn world_bounds(s: &MapPage) -> Rect {
    let cards = s.visible.iter().fold(Rect::NOTHING, |r, i| {
        r.union(Rect::from_min_size(s.positions[*i], CARD))
    });
    let groups = s
        .file_groups
        .iter()
        .fold(cards, |r, g| r.union(Rect::from_min_size(g.at, CARD)));
    s.captions
        .iter()
        .fold(groups, |r, c| r.union(Rect::from_min_size(c.at, vec2(CARD.x, 18.))))
}
fn focus(s: &mut MapPage, rect: Rect) {
    if let Some(i) = s.selected.filter(|i| s.visible.contains(i)) {
        s.zoom = 0.95;
        s.pan = rect.size() / 2. - (s.positions[i].to_vec2() + CARD / 2.) * s.zoom;
    }
}
pub(super) fn fit(s: &mut MapPage, rect: Rect) {
    let bounds = world_bounds(s);
    if bounds.is_finite() && bounds.is_positive() {
        s.zoom = ((rect.width() - 60.).max(1.) / bounds.width())
            .min((rect.height() - 90.).max(1.) / bounds.height())
            .clamp(0.00001, 1.);
        s.pan = (rect.size() - bounds.size() * s.zoom) / 2. - bounds.min.to_vec2() * s.zoom;
    }
}
/// One line, cut with an ellipsis; never broken inside a word.
pub(super) fn title_job(title: String, zoom: f32, width: f32) -> text::LayoutJob {
    let title = if title.chars().count() > 160 {
        format!("{}…", title.chars().take(160).collect::<String>())
    } else {
        title
    };
    let mut job = text::LayoutJob::simple_singleline(title, semibold((13. * zoom).max(7.)), TEXT());
    job.wrap = text::TextWrapping::truncate_at_width(width);
    job
}
fn line_job(text: String, size: f32, color: Color32, width: f32) -> text::LayoutJob {
    let mut job = text::LayoutJob::simple_singleline(text, FontId::proportional(size), color);
    job.wrap = text::TextWrapping::truncate_at_width(width);
    job
}
/// Kind icon, its tint, title and the line of numbers under it.
pub(super) fn card_text(s: &MapPage, data: &RepositoryMap, i: usize) -> (Icon, Color32, String, String) {
    let n = &data.nodes[i];
    match n.kind {
        MapNodeKind::Crate if s.alternate => {
            let uses = s.projected_links.iter().filter(|e| e.kind == MapLinkKind::DependsOn && e.from == i).count();
            let used = s.projected_links.iter().filter(|e| e.kind == MapLinkKind::DependsOn && e.to == i).count();
            let mut parts = vec![trn!(n.files, "{n} file", "{n} files")];
            if uses > 0 {
                parts.push(trf!("uses {count}", count = uses));
            }
            if used > 0 {
                parts.push(trf!("used by {count}", count = used));
            }
            (Icon::Package, TEAL(), label(&n.label), parts.join(" · "))
        }
        MapNodeKind::Directory | MapNodeKind::Crate => (Icon::Folder, TEAL(), folder_name(n), trn!(n.files, "{n} file", "{n} files")),
        MapNodeKind::File => (Icon::File, MUTED(), label(&n.label), n.path.as_ref().map(|p| label(p)).unwrap_or_default()),
        MapNodeKind::Person => (Icon::User, VIOLET(), label(&n.label), format!("{} · {}", trn!(n.count, "{n} commit", "{n} commits"), trn!(n.files, "{n} file", "{n} files"))),
        MapNodeKind::Area => (Icon::Folder, TEAL(), label(&n.label), trn!(n.count, "{n} commit", "{n} commits")),
        MapNodeKind::Commit => (
            Icon::Commit,
            BLUE(),
            label(&n.label),
            n.commit.as_ref().map(|id| format!("{} · {}", short(id), label(&n.detail))).unwrap_or_default(),
        ),
        MapNodeKind::Module => (Icon::Braces, TEAL(), modules::name(data, i), modules::card_detail(s, data, i)),
    }
}
fn arrow(painter: &Painter, end: Pos2, direction: Vec2, color: Color32) {
    let direction = direction.normalized();
    let cross = vec2(-direction.y, direction.x);
    painter.add(Shape::convex_polygon(
        vec![end, end - direction * 7. + cross * 3.5, end - direction * 7. - cross * 3.5],
        color,
        Stroke::NONE,
    ));
}
fn curve(painter: &Painter, points: [Pos2; 4], stroke: Stroke, dashed: bool) {
    if dashed {
        let shape = epaint::CubicBezierShape::from_points_stroke(points, false, Color32::TRANSPARENT, stroke);
        let samples: Vec<Pos2> = (0..=24).map(|k| shape.sample(k as f32 / 24.)).collect();
        painter.extend(Shape::dashed_line(&samples, stroke, 5., 4.));
    } else {
        painter.add(epaint::CubicBezierShape::from_points_stroke(points, false, Color32::TRANSPARENT, stroke));
    }
}

pub(super) fn canvas(
    ui: &mut Ui,
    s: &mut MapPage,
    rect: Rect,
    covered: bool,
    widgets: &mut HashMap<String, UiControl>,
) {
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let canvas = ui.interact(
        rect,
        ui.id().with("repository-map-canvas"),
        if covered {
            Sense::hover()
        } else {
            Sense::click_and_drag()
        },
    );
    if !covered {
        register(ui, widgets, "map-canvas", &canvas);
    }
    let size = rect.size();
    if s.fit || s.last_canvas != size {
        fit(s, rect);
        // Start large maps at a readable selection; explicit Fit still shows
        // the complete bounded graph, including tall folder lists.
        if s.zoom < 0.45 || size.x < 700. && s.zoom < 0.75 {
            focus(s, rect);
        }
        s.fit = false;
        s.last_canvas = size;
    }
    if s.focus {
        focus(s, rect);
        s.focus = false;
    }
    if !covered {
        if canvas.dragged() {
            s.pan += ui.input(|i| i.pointer.delta());
        }
        if canvas.hovered() {
            let delta = ui.input(|i| i.smooth_scroll_delta.y);
            if delta != 0. {
                zoom_at(
                    s,
                    (delta * 0.002).exp(),
                    ui.input(|i| i.pointer.hover_pos()).unwrap_or(rect.center()) - rect.min,
                );
            }
        }
        if canvas.clicked() {
            canvas.request_focus();
        }
        if canvas.has_focus() {
            ui.input(|i| {
                if i.key_pressed(Key::ArrowLeft) {
                    s.pan.x += 30.;
                }
                if i.key_pressed(Key::ArrowRight) {
                    s.pan.x -= 30.;
                }
                if i.key_pressed(Key::ArrowUp) {
                    s.pan.y += 30.;
                }
                if i.key_pressed(Key::ArrowDown) {
                    s.pan.y -= 30.;
                }
                if i.key_pressed(Key::F) {
                    fit(s, rect);
                }
            });
        }
    }
    for x in (rect.left() as i32..rect.right() as i32).step_by(24) {
        for y in (rect.top() as i32..rect.bottom() as i32).step_by(24) {
            painter.circle_filled(pos2(x as f32, y as f32), 0.7, BORDER().gamma_multiply(0.5));
        }
    }
    let Some(data) = s.data.clone() else {
        empty(&painter, rect, if s.running() {
            tr("Reading the repository")
        } else if s.error.is_some() {
            tr("No map yet. Check the message above and retry")
        } else {
            tr("Choose Refresh to read this map")
        });
        return;
    };
    if s.visible.is_empty() && s.file_groups.is_empty() {
        empty(&painter, rect, if data.kind == MapKind::Modules {
            tr("Choose a module from the list")
        } else if !s.query.is_empty() {
            tr("No match. Search looks at names, paths, authors and commit IDs")
        } else if data.nodes.is_empty() {
            tr("Nothing committed in this scope")
        } else {
            tr("Nothing to show here. Try the other view or a wider scope")
        });
        return;
    }
    let modules = data.kind == MapKind::Modules;
    // Modules drawn whole read top to bottom like packages; around one module they
    // read left to right.
    let packages = data.kind == MapKind::Code && s.alternate || modules && s.whole;
    let around = modules && !s.whole;
    let tree = data.kind == MapKind::Code && !s.alternate;
    let visible = s.visible.iter().copied().collect::<HashSet<_>>();
    let at = |i: usize| rect.min + s.pan + s.positions[i].to_vec2() * s.zoom;
    let card = CARD * s.zoom;
    // What the selection connects to stays bright; the rest dims.
    let mut related = HashSet::new();
    if let Some(sel) = s.selected.filter(|i| visible.contains(i)) {
        related.insert(sel);
        for e in s.projected_links.iter().filter(|e| s.edge_kind(e.kind)) {
            if e.from == sel {
                related.insert(e.to);
            }
            if e.to == sel {
                related.insert(e.from);
            }
        }
    }
    let dimmed = |i: usize| !related.is_empty() && !related.contains(&i) && !tree;
    for edge in s
        .projected_links
        .iter()
        .filter(|e| s.edge_kind(e.kind) && visible.contains(&e.from) && visible.contains(&e.to))
    {
        let (a, b) = (at(edge.from), at(edge.to));
        let active = s.selected == Some(edge.from) || s.selected == Some(edge.to);
        let color = if active {
            BLUE()
        } else if !related.is_empty() && !tree {
            BORDER().gamma_multiply(0.5)
        } else {
            BORDER()
        };
        if matches!(edge.kind, MapLinkKind::Changes | MapLinkKind::SharedFiles) {
            let (from, to) = (a + card / 2., b + card / 2.);
            let width = (edge.weight as f32).sqrt().clamp(1., 4.);
            painter.line_segment([from, to], Stroke::new(width, color));
            if s.zoom >= 0.6 {
                let middle = from + (to - from) / 2.;
                let text = trn!(edge.weight, "{n} file", "{n} files");
                let galley = painter.layout_no_wrap(text, FontId::proportional(10.), if active { BLUE() } else { MUTED() });
                painter.rect_filled(Rect::from_center_size(middle, galley.size() + vec2(12., 4.)), 8., VIEW_BG());
                painter.galley(middle - galley.size() / 2., galley, MUTED());
            }
            continue;
        }
        if tree {
            if edge.kind != MapLinkKind::Contains {
                continue;
            }
            // Parent's right edge to the child's left edge.
            let start = a + vec2(card.x, card.y / 2.);
            let end = b + vec2(0., card.y / 2.);
            let mid = (start.x + end.x) / 2.;
            curve(&painter, [start, pos2(mid, start.y), pos2(mid, end.y), end], Stroke::new(if active { 1.8 } else { 1.2 }, color), false);
            continue;
        }
        if packages {
            // From the user's bottom to the top of what it uses.
            let start = a + vec2(card.x / 2., card.y);
            let end = b + vec2(card.x / 2., 0.);
            let mid = (start.y + end.y) / 2.;
            let incoming = s.selected == Some(edge.to);
            curve(&painter, [start, pos2(start.x, mid), pos2(end.x, mid), end], Stroke::new(if active { 1.8 } else { 1.2 }, color), incoming);
            arrow(&painter, end, vec2(0., 1.), color);
            continue;
        }
        if around {
            // Between facing edges, the arrow at what is used: left to right, or back
            // to a user on the left that the chosen module also uses.
            let forward = a.x < b.x;
            let start = a + vec2(if forward { card.x } else { 0. }, card.y / 2.);
            let end = b + vec2(if forward { 0. } else { card.x }, card.y / 2.);
            let mid = (start.x + end.x) / 2.;
            curve(&painter, [start, pos2(mid, start.y), pos2(mid, end.y), end], Stroke::new(1.4, BLUE()), false);
            arrow(&painter, end, vec2(if forward { 1. } else { -1. }, 0.), BLUE());
            continue;
        }
        let (start, end) = (a + card / 2., b + card / 2.);
        painter.line_segment([start, end], Stroke::new(1.2, color));
        arrow(&painter, end - (end - start).normalized() * card.y / 2., end - start, color);
    }
    // Modules: the words over the columns.
    for c in &s.captions {
        if s.zoom >= 0.4 {
            painter.text(rect.min + s.pan + c.at.to_vec2() * s.zoom, Align2::LEFT_TOP, modules::caption_text(c), semibold(11. * s.zoom.min(1.2)), HINT());
        }
    }
    // Tree: a folder's own files, summed up on one card. Modules: the rest of a side.
    let mut opened = None;
    for group in &s.file_groups {
        let (parent, count) = (group.folder, group.files);
        let r = Rect::from_min_size(rect.min + s.pan + group.at.to_vec2() * s.zoom, card);
        if !r.intersects(rect) {
            continue;
        }
        if let Some(side) = group.more {
            if let Some(p) = parent.filter(|p| visible.contains(p)) {
                let (start, end) = match side {
                    modules::More::Users => (r.right_center(), at(p) + vec2(0., card.y / 2.)),
                    modules::More::Uses => (at(p) + vec2(card.x, card.y / 2.), r.left_center()),
                };
                let mid = (start.x + end.x) / 2.;
                curve(&painter, [start, pos2(mid, start.y), pos2(mid, end.y), end], Stroke::new(1.2, BLUE()), true);
                arrow(&painter, end, vec2(1., 0.), BLUE());
            }
            painter.rect_filled(r, 8. * s.zoom.min(1.), VIEW_RAISED().gamma_multiply(0.6));
            let dash = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
            painter.extend(Shape::dashed_line(&dash, Stroke::new(1., BORDER()), 5., 3.));
            let (title, line) = modules::more_text(group);
            if s.zoom >= 0.4 {
                let ink = painter.with_clip_rect(r.shrink(4.).intersect(rect));
                ink.galley(r.left_top() + vec2(10., 8.) * s.zoom, ink.layout_job(title_job(title.clone(), s.zoom, r.width() - 20. * s.zoom)), MUTED());
                if s.zoom > 0.7 {
                    ink.galley(r.left_top() + vec2(10., 35.) * s.zoom, ink.layout_job(line_job(line, 11. * s.zoom, HINT(), r.width() - 20. * s.zoom)), HINT());
                }
            }
            let hit = r.intersect(rect.shrink2(vec2(0., 36.)));
            if !covered && hit.width() > 20. && hit.height() > 18. {
                let id = match side {
                    modules::More::Users => "map-module-more-users",
                    modules::More::Uses => "map-module-more-uses",
                };
                let response = ui.interact(hit, ui.id().with(id), Sense::click()).on_hover_text(tr("Show them all in Details"));
                register(ui, widgets, id, &response);
                if response.clicked() {
                    opened = Some(side);
                }
            }
            continue;
        }
        if let Some(p) = parent.filter(|p| visible.contains(p)) {
            let from = at(p) + vec2(card.x, card.y / 2.);
            let to = r.left_center();
            let mid = (from.x + to.x) / 2.;
            curve(&painter, [from, pos2(mid, from.y), pos2(mid, to.y), to], Stroke::new(1.2, BORDER()), false);
        }
        painter.rect_filled(r, 8. * s.zoom.min(1.), VIEW_RAISED().gamma_multiply(0.6));
        let dash = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
        painter.extend(Shape::dashed_line(&dash, Stroke::new(1., BORDER()), 5., 3.));
        if s.zoom >= 0.4 {
            let ink = painter.with_clip_rect(r.shrink(4.).intersect(rect));
            let icon_rect = Rect::from_min_size(r.left_top() + vec2(9., 10.) * s.zoom, vec2(13., 13.) * s.zoom);
            icon(&ink, icon_rect, Icon::File, HINT());
            let title = trn!(count, "{n} file", "{n} files");
            ink.galley(r.left_top() + vec2(28., 8.) * s.zoom, ink.layout_job(title_job(title, s.zoom, r.width() - 36. * s.zoom)), TEXT());
            if s.zoom > 0.7 {
                ink.galley(r.left_top() + vec2(9., 36.) * s.zoom, ink.layout_job(line_job(group_about(group, &data), 11. * s.zoom, HINT(), r.width() - 18. * s.zoom)), HINT());
            }
        }
    }
    let mut picked = None;
    let mut toggled = None;
    for &i in &s.visible {
        let n = &data.nodes[i];
        let node = Rect::from_min_size(at(i), card);
        if !node.intersects(rect) {
            continue;
        }
        let selected = s.selected == Some(i);
        let fade = if dimmed(i) { 0.45 } else { 1. };
        painter.rect_filled(node, 8. * s.zoom.min(1.), if selected { SELECTED() } else { RAISED() }.gamma_multiply(fade));
        painter.rect_stroke(
            node,
            8. * s.zoom.min(1.),
            Stroke::new(if selected { 1.7 } else { 1. }, if selected { BLUE() } else { BORDER() }.gamma_multiply(fade)),
            StrokeKind::Inside,
        );
        let group = tree && navigation::is_group(&data, i);
        let (kind_icon, tint, title, detail) = card_text(s, &data, i);
        if s.zoom >= 0.4 {
            let ink = painter.with_clip_rect(node.shrink(4.).intersect(rect));
            let mut left = 9.;
            if group {
                let chevron = Rect::from_min_size(node.left_top() + vec2(8., 11.) * s.zoom, vec2(10., 10.) * s.zoom);
                icon(&ink, chevron, if s.collapsed.contains(&n.key) { Icon::ChevronRight } else { Icon::Chevron }, HINT().gamma_multiply(fade));
                left += 14.;
            }
            let kind_rect = Rect::from_min_size(node.left_top() + vec2(left, 9.) * s.zoom, vec2(14., 14.) * s.zoom);
            icon(&ink, kind_rect, kind_icon, if selected { BLUE() } else { tint }.gamma_multiply(fade));
            let title_at = node.left_top() + vec2(left + 20., 7.) * s.zoom;
            let width = (node.right() - title_at.x - 8. * s.zoom).max(1.);
            ink.galley(title_at, ink.layout_job(title_job(title.clone(), s.zoom, width)), TEXT().gamma_multiply(fade));
            if s.zoom > 0.7 {
                let width = (node.width() - 18. * s.zoom).max(1.);
                ink.galley(
                    node.left_top() + vec2(9., 35.) * s.zoom,
                    ink.layout_job(line_job(detail.clone(), 11.5 * s.zoom, HINT(), width)),
                    HINT().gamma_multiply(fade),
                );
            }
        }
        let hit = node.intersect(rect.shrink2(vec2(0., 36.)));
        if !covered && hit.width() > 20. && hit.height() > 18. {
            let hover = if modules { modules::hover(s, &data, i) } else { format!("{title}\n{detail}") };
            let response = ui
                .interact(hit, ui.id().with(("map-node", i)), Sense::click())
                .on_hover_text(hover);
            register(ui, widgets, &format!("map-node-{i}"), &response);
            if response.clicked() {
                picked = Some(i);
            }
            if group {
                let chevron = Rect::from_min_size(node.left_top(), vec2(26., CARD.y) * s.zoom).intersect(hit);
                if chevron.width() > 8. {
                    let r = ui.interact(chevron, ui.id().with(("map-toggle", i)), Sense::click());
                    register(ui, widgets, &format!("map-toggle-{i}"), &r);
                    if r.clicked() {
                        toggled = Some(i);
                    }
                }
            }
        }
    }
    // A node picked here is not a People cell, so a cell picked earlier no longer
    // decides what Details shows.
    if let Some(i) = toggled {
        s.toggle_group(i);
        s.selected = Some(i);
        s.cell = None;
    } else if let Some(i) = picked {
        s.selected = Some(i);
        s.cell = None;
        s.show_details = true;
        // Modules: the picked card moves to the middle, and the list shows it.
        if modules {
            s.whole = false;
            s.list_jump = true;
        }
    }
    if let Some(side) = opened {
        modules::open_section(ui.ctx(), side);
        s.show_details = true;
    }
    if covered {
        return;
    }
    // A fixed English-width bar clips longer translated labels. Measure the
    // actual button text/padding and reserve the same space for the legend.
    let fit_label = tr("Fit");
    let focus_label = tr("Focus");
    let buttons_width: f32 = ["−", "+", fit_label, focus_label].into_iter().map(|text| {
        let galley = WidgetText::from(text).into_galley(ui, Some(TextWrapMode::Extend), f32::INFINITY, TextStyle::Button);
        (galley.size().x + 2. * ui.spacing().button_padding.x).max(ui.spacing().interact_size.x)
    }).sum();
    let bar_width = buttons_width + 40. + 4. * 4. + 6.;
    let bar = Rect::from_min_max(
        pos2((rect.right() - 10. - bar_width).max(rect.left() + 6.), rect.bottom() - 40.),
        rect.right_bottom() - vec2(10., 8.),
    );
    legend(ui, s, &data, rect, bar.left() - 8.);
    painter.rect(bar, 8., PANEL(), Stroke::new(1., CARD_STROKE()), StrokeKind::Inside);
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(bar.shrink(3.))
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = 4.;
            ui.style_mut().wrap_mode = Some(TextWrapMode::Extend);
            if button(ui, widgets, "map-zoom-out", "−", true) {
                zoom_at(s, 0.8, rect.size() / 2.);
            }
            ui.add_sized([40., 20.], Label::new(RichText::new(format!("{}%", (s.zoom * 100.).round())).size(11.).color(MUTED())));
            if button(ui, widgets, "map-zoom-in", "+", true) {
                zoom_at(s, 1.25, rect.size() / 2.);
            }
            if button(ui, widgets, "map-fit", fit_label, true) {
                fit(s, rect);
            }
            if button(ui, widgets, "map-focus", focus_label, s.selected.is_some()) {
                s.focus = true;
            }
        },
    );
}
fn empty(painter: &Painter, rect: Rect, text: &str) {
    painter.text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(13.), MUTED());
}
/// What the map shows, in the corner.
fn legend(ui: &Ui, s: &MapPage, data: &RepositoryMap, rect: Rect, max_right: f32) {
    let text = match data.kind {
        MapKind::Code if s.alternate => {
            let links = s.projected_links.iter().filter(|e| e.kind == MapLinkKind::DependsOn).count();
            trf!("Packages {packages} · dependencies {links} · arrows point to what is used", packages = s.visible.len(), links = links)
        }
        MapKind::Code => {
            let total = data.nodes.iter().filter(|n| matches!(n.kind, MapNodeKind::Directory | MapNodeKind::Crate)).count();
            trf!("Folders {shown} of {total} shown · files are listed in Details", shown = s.visible.len(), total = total)
        }
        MapKind::People => trf!("People {count} · lines join people who changed the same files", count = s.visible.len()),
        MapKind::History => trn!(s.visible.len(), "{n} commit", "{n} commits"),
        MapKind::Modules => {
            let (count, links) = s.units.as_ref().map_or((0, 0), |u| (u.top.len(), u.top_links));
            trf!("Modules {modules} · links {links} · arrows point to what is used", modules = pages::number_text(count), links = pages::number_text(links))
        }
    };
    let painter = ui.painter().with_clip_rect(rect);
    let galley = painter.layout_no_wrap(text, FontId::proportional(12.), MUTED());
    let r = Rect::from_min_size(rect.left_bottom() + vec2(10., -40.), vec2(galley.size().x + 22., 32.));
    if r.right() > max_right {
        return;
    }
    painter.rect(r, 8., PANEL(), Stroke::new(1., CARD_STROKE()), StrokeKind::Inside);
    painter.galley(pos2(r.left() + 11., r.center().y - galley.size().y / 2.), galley, MUTED());
}
fn zoom_at(s: &mut MapPage, factor: f32, point: Vec2) {
    let next = (s.zoom * factor).clamp(0.00001, 2.);
    s.pan = point - (point - s.pan) * (next / s.zoom);
    s.zoom = next;
}
