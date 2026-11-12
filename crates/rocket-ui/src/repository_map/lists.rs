//! Maps read as lists: history's branches and merges, every commit, and who works where.
use super::*;

/// History along the first parents of the snapshot's tip: commits worth naming, with
/// runs of plain commits folded into one line.
pub(crate) struct Topology {
    pub lines: Vec<Line>,
    /// Branch tips the main line doesn't contain, with their commits ahead of it.
    pub tips: Vec<(usize, Vec<usize>)>,
}
pub(crate) enum Line {
    /// The tip, a merge, a decorated commit or where a branch starts. `merged` holds
    /// what a merge brought in through its other parents.
    Commit { node: usize, merged: Vec<usize> },
    Run { nodes: Vec<usize> },
}
impl Line {
    pub fn first(&self) -> usize {
        match self {
            Line::Commit { node, .. } => *node,
            Line::Run { nodes } => nodes[0],
        }
    }
    fn nodes(&self) -> Vec<usize> {
        match self {
            Line::Commit { node, merged } => std::iter::once(*node).chain(merged.iter().copied()).collect(),
            Line::Run { nodes } => nodes.clone(),
        }
    }
}
pub(crate) fn build(data: &RepositoryMap) -> Topology {
    let n = data.nodes.len();
    let mut parents = vec![Vec::new(); n];
    for e in data.links.iter().filter(|e| e.kind == MapLinkKind::Parent) {
        parents[e.to].push(e.from);
    }
    let start = data
        .tip
        .as_ref()
        .and_then(|tip| data.nodes.iter().position(|node| node.commit.as_ref() == Some(tip)))
        .unwrap_or(0);
    let mut main = Vec::new();
    let mut on_main = HashSet::new();
    let mut at = (n > 0).then_some(start);
    while let Some(i) = at {
        if !on_main.insert(i) {
            break;
        }
        main.push(i);
        at = parents[i].first().copied();
    }
    // A merge's other parents, along their first parents until the main line.
    let mut claimed = HashSet::new();
    let walk = |from: usize, claimed: &mut HashSet<usize>| {
        let mut out = Vec::new();
        let mut at = Some(from);
        while let Some(i) = at {
            if on_main.contains(&i) || !claimed.insert(i) || out.len() >= MAP_HISTORY_MAX {
                break;
            }
            out.push(i);
            at = parents[i].first().copied();
        }
        out
    };
    let mut merged = HashMap::new();
    for &m in &main {
        let mut list = Vec::new();
        for &p in parents[m].iter().skip(1) {
            list.extend(walk(p, &mut claimed));
        }
        if !list.is_empty() {
            merged.insert(m, list);
        }
    }
    let branch = |i: usize| data.nodes[i].refs.iter().any(|r| r.starts_with(b"refs/heads/"));
    let mut forks = HashSet::new();
    let mut tips = Vec::new();
    for i in 0..n {
        if on_main.contains(&i) || claimed.contains(&i) || !branch(i) {
            continue;
        }
        let ahead = walk(i, &mut claimed);
        if let Some(&last) = ahead.last()
            && let Some(&fork) = parents[last].first()
            && on_main.contains(&fork)
        {
            forks.insert(fork);
        }
        tips.push((i, ahead));
    }
    let key = |pos: usize, i: usize| {
        pos == 0
            || pos + 1 == main.len()
            || parents[i].len() > 1
            || !data.nodes[i].refs.is_empty()
            || forks.contains(&i)
    };
    let mut lines = Vec::new();
    let mut run = Vec::new();
    for (pos, &i) in main.iter().enumerate() {
        if key(pos, i) {
            if !run.is_empty() {
                lines.push(Line::Run { nodes: std::mem::take(&mut run) });
            }
            lines.push(Line::Commit { node: i, merged: merged.remove(&i).unwrap_or_default() });
        } else {
            run.push(i);
        }
    }
    if !run.is_empty() {
        lines.push(Line::Run { nodes: run });
    }
    Topology { lines, tips }
}
/// The branch a merge commit's subject names, or the subject.
pub(super) fn merge_name(subject: &str) -> String {
    let s = subject.trim();
    if let Some(rest) = s.strip_prefix("Merge pull request ")
        && let Some((_, from)) = rest.split_once(" from ")
    {
        return from.split_whitespace().next().unwrap_or(from).to_owned();
    }
    for prefix in ["Merge remote-tracking branch '", "Merge branch '"] {
        if let Some(rest) = s.strip_prefix(prefix)
            && let Some((name, _)) = rest.split_once('\'')
        {
            return name.to_owned();
        }
    }
    if let Some(rest) = s.strip_prefix("Merge ")
        && !rest.contains(' ')
    {
        return rest.to_owned();
    }
    s.to_owned()
}
/// "Sofia Reyes", "Ada and Ben", "Ada, Ben and 3 more".
pub(super) fn authors(data: &RepositoryMap, nodes: &[usize]) -> String {
    let mut names: Vec<String> = Vec::new();
    for &i in nodes {
        let name = label(&data.nodes[i].detail);
        if !names.contains(&name) {
            names.push(name);
        }
    }
    match names.len() {
        0 => String::new(),
        1 => names[0].clone(),
        2 => trf!("{first} and {second}", first = &names[0], second = &names[1]),
        n => trf!("{first}, {second} and {more} more", first = &names[0], second = &names[1], more = n - 2),
    }
}
pub(super) fn date(data: &RepositoryMap, i: usize) -> String {
    let time = data.nodes[i].time;
    if time <= 0 {
        return String::new();
    }
    crate::activity_history::short_date(time, crate::activity_history::now_ms())
}
fn ref_pills(ui: &mut Ui, refs: &[Vec<u8>]) {
    for r in refs.iter().take(3) {
        if let Some(tag) = r.strip_prefix(b"refs/tags/") {
            pages::pill(ui, &label(tag), AMBER());
        } else {
            pages::pill(ui, &label(r.strip_prefix(b"refs/heads/").unwrap_or(r)), BLUE());
        }
    }
}
const ROW: f32 = 44.;
const MAIN: f32 = 34.;
const SIDE: f32 = 80.;

pub(super) fn topology(
    ui: &mut Ui,
    s: &mut MapPage,
    rect: Rect,
    covered: bool,
    widgets: &mut HashMap<String, UiControl>,
) {
    let Some(data) = s.data.clone() else {
        drawing_message(ui, rect, s);
        return;
    };
    if s.topology.is_none() {
        s.topology = Some(Arc::new(build(&data)));
    }
    let topo = s.topology.clone().unwrap();
    // Search keeps the lines with a match, as the other maps keep matching nodes.
    let shown: Vec<usize> = (0..topo.lines.len())
        .filter(|&k| s.query.is_empty() || topo.lines[k].nodes().iter().any(|i| s.visible.contains(i)))
        .collect();
    if shown.is_empty() {
        let painter = ui.painter().with_clip_rect(rect);
        painter.text(rect.center(), Align2::CENTER_CENTER, if s.query.is_empty() { tr("No commits in this scope") } else { tr("No match in this history") }, FontId::proportional(13.), MUTED());
        return;
    }
    // A branch tip picked above the main line keeps the selection without a line.
    let tip = s.selected.is_some_and(|i| topo.tips.iter().any(|(t, _)| *t == i));
    if s.line.is_some_and(|l| !shown.contains(&l)) || s.line.is_none() && !tip {
        s.line = shown.first().copied();
        s.selected = s.line.map(|l| topo.lines[l].first());
    }
    let text_x = if rect.width() < 640. { 200. } else { 300. };
    let tips: Vec<&(usize, Vec<usize>)> = if s.query.is_empty() { topo.tips.iter().take(4).collect() } else { Vec::new() };
    let head = if tips.is_empty() { 8. } else { 34. };
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min)));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    let scroll = ScrollArea::vertical()
        .id_salt("map-topology")
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            let width = ui.available_width();
            let (area, _) = ui.allocate_exact_size(vec2(width, head + shown.len() as f32 * ROW + 16.), Sense::hover());
            let painter = ui.painter().clone();
            let x = |dx: f32| area.left() + dx;
            let y = |row: usize| area.top() + head + row as f32 * ROW + ROW / 2.;
            // Main line.
            painter.line_segment([pos2(x(MAIN), y(0) - ROW / 2. + 4.), pos2(x(MAIN), y(shown.len() - 1))], Stroke::new(2.2, BLUE().gamma_multiply(0.85)));
            // Branches that are ahead of the main line, above it.
            for (k, (tip, ahead)) in tips.iter().enumerate() {
                let lane = x(MAIN + 22. * (k as f32 + 1.));
                let color = [VIOLET(), ORANGE(), PINK(), TEAL()][k % 4];
                let dot = pos2(lane, area.top() + 12.);
                painter.line_segment([dot, pos2(lane, y(0) - 10.)], Stroke::new(2., color));
                painter.line_segment([pos2(lane, y(0) - 10.), pos2(x(MAIN), y(0))], Stroke::new(2., color));
                painter.circle_filled(dot, 5., color);
                let name = data.nodes[*tip]
                    .refs
                    .iter()
                    .find_map(|r| r.strip_prefix(b"refs/heads/"))
                    .map(label)
                    .unwrap_or_default();
                let text = trf!("{name} · {count} ahead", name = name, count = ahead.len());
                let at = pos2(x(text_x) + k as f32 * 210., area.top() + 3.);
                let galley = painter.layout_no_wrap(text.clone(), semibold(11.5), color);
                let pill = Rect::from_min_size(at, galley.size() + vec2(16., 6.));
                painter.rect_filled(pill, 9., color.gamma_multiply(0.14));
                painter.galley(at + vec2(8., 3.), galley, color);
                let r = ui.interact(pill, ui.scope_id().with(("map-tip", k)), Sense::click()).on_hover_text(text);
                register(ui, widgets, &format!("map-tip-{k}"), &r);
                if r.clicked() && !covered {
                    s.selected = Some(*tip);
                    s.line = None;
                    s.show_details = true;
                }
            }
            for (row, &k) in shown.iter().enumerate() {
                let line = &topo.lines[k];
                let cy = y(row);
                let band = Rect::from_min_max(pos2(area.left(), cy - ROW / 2.), pos2(area.right(), cy + ROW / 2.));
                let selected = s.line == Some(k);
                if selected {
                    painter.rect_filled(band, 0., ACCENT_SOFT());
                }
                let response = ui.interact(band, ui.scope_id().with(("map-line", k)), Sense::click());
                register(ui, widgets, &format!("map-line-{k}"), &response);
                if response.clicked() && !covered {
                    s.line = Some(k);
                    s.selected = Some(line.first());
                    s.show_details = true;
                }
                let title_at = x(text_x);
                let right = area.right() - 12.;
                match line {
                    Line::Commit { node, merged } => {
                        let n = &data.nodes[*node];
                        let merge = n.count > 1;
                        if merge {
                            painter.circle_filled(pos2(x(MAIN), cy), 6., VIEW_BG());
                            painter.circle_stroke(pos2(x(MAIN), cy), 6., Stroke::new(2.4, BLUE()));
                        } else {
                            painter.circle_filled(pos2(x(MAIN), cy), if row == 0 { 7. } else { 5. }, BLUE());
                        }
                        if !merged.is_empty() {
                            let bend = Stroke::new(if selected { 2. } else { 1.6 }, if selected { BLUE() } else { BORDER() });
                            let side = x(MAIN + 46.);
                            let top = pos2(x(MAIN), cy);
                            let bottom = pos2(x(MAIN), cy + ROW);
                            painter.add(epaint::CubicBezierShape::from_points_stroke([top, pos2(side, cy + 2.), pos2(side, cy + 6.), pos2(side, cy + 14.)], false, Color32::TRANSPARENT, bend));
                            painter.line_segment([pos2(side, cy + 14.), pos2(side, cy + ROW - 14.)], bend);
                            painter.add(epaint::CubicBezierShape::from_points_stroke([pos2(side, cy + ROW - 14.), pos2(side, cy + ROW - 6.), pos2(side, cy + ROW - 2.), bottom], false, Color32::TRANSPARENT, bend));
                            let chip = format!("{} · {}", trn!(merged.len(), "{n} commit", "{n} commits"), authors(&data, merged));
                            let room = (title_at - x(SIDE) - 16.).max(40.);
                            let galley = painter.layout_job(line_job(chip, 12., if selected { TEXT() } else { MUTED() }, room - 18.));
                            let r = Rect::from_min_size(pos2(x(SIDE), cy + 2.), galley.size() + vec2(18., 6.));
                            painter.rect(r, 11., if selected { SELECTED() } else { RAISED() }, Stroke::new(1., if selected { BLUE() } else { BORDER() }), StrokeKind::Inside);
                            painter.galley(r.min + vec2(9., 3.), galley, MUTED());
                        }
                        let mut child = ui.new_child(UiBuilder::new().max_rect(Rect::from_min_max(pos2(title_at, cy - 11.), pos2(right, cy + 11.))).layout(Layout::left_to_right(Align::Center)));
                        child.spacing_mut().item_spacing.x = 7.;
                        let (mark, _) = child.allocate_exact_size(vec2(14., 14.), Sense::hover());
                        icon(child.painter(), mark, if merge { Icon::Merge } else { Icon::Commit }, HINT());
                        let title = if merge { merge_name(&label(&n.label)) } else { label(&n.label) };
                        ref_pills(&mut child, &n.refs);
                        let date = date(&data, *node);
                        let who = if merge { String::new() } else { label(&n.detail) };
                        child.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(date).size(12.).color(HINT()));
                            if !who.is_empty() {
                                ui.label(RichText::new(who).size(12.).color(HINT()));
                            }
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                let text = RichText::new(title);
                                ui.add(Label::new(if selected { text.strong() } else { text }).truncate());
                            });
                        });
                    }
                    Line::Run { nodes } => {
                        let count = nodes.len();
                        let chip = Rect::from_center_size(pos2(x(MAIN), cy), vec2(34., 20.));
                        painter.rect(chip, 10., RAISED(), Stroke::new(1., BORDER()), StrokeKind::Inside);
                        painter.text(chip.center(), Align2::CENTER_CENTER, pages::number_text(count), FontId::proportional(11.), MUTED());
                        let mut child = ui.new_child(UiBuilder::new().max_rect(Rect::from_min_max(pos2(title_at, cy - 11.), pos2(right, cy + 11.))).layout(Layout::left_to_right(Align::Center)));
                        child.spacing_mut().item_spacing.x = 7.;
                        let (mark, _) = child.allocate_exact_size(vec2(14., 14.), Sense::hover());
                        icon(child.painter(), mark, Icon::Commit, FAINT());
                        let what = if count == 1 { label(&data.nodes[nodes[0]].label) } else { trn!(count, "{n} commit", "{n} commits") };
                        let who = authors(&data, nodes);
                        let date = date(&data, nodes[0]);
                        child.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(date).size(12.).color(HINT()));
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                ui.label(RichText::new(what).color(MUTED()));
                                ui.add(Label::new(RichText::new(who).size(12.).color(FAINT())).truncate());
                            });
                        });
                    }
                }
            }
        });
    if !covered {
        register(ui, widgets, "map-canvas", &ui.interact(scroll.inner_rect, ui.scope_id().with("map-topology-area"), Sense::hover()));
    }
}
fn line_job(text: String, size: f32, color: Color32, width: f32) -> text::LayoutJob {
    let mut job = text::LayoutJob::simple_singleline(text, FontId::proportional(size), color);
    job.wrap = text::TextWrapping::truncate_at_width(width.max(1.));
    job
}
fn drawing_message(ui: &mut Ui, rect: Rect, s: &MapPage) {
    let painter = ui.painter().with_clip_rect(rect);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        if s.running() {
            tr("Reading the repository")
        } else if s.error.is_some() {
            tr("No map yet. Check the message above and retry")
        } else {
            tr("Choose Refresh to read this map")
        },
        FontId::proportional(13.),
        MUTED(),
    );
}

/// Every commit in compact rows, lanes as in Commits.
pub(super) fn commits(
    ui: &mut Ui,
    s: &mut MapPage,
    rect: Rect,
    covered: bool,
    widgets: &mut HashMap<String, UiControl>,
) {
    let Some(data) = s.data.clone() else {
        drawing_message(ui, rect, s);
        return;
    };
    if s.visible.is_empty() {
        let painter = ui.painter().with_clip_rect(rect);
        painter.text(rect.center(), Align2::CENTER_CENTER, if s.query.is_empty() { tr("No commits in this scope") } else { tr("No match in this history") }, FontId::proportional(13.), MUTED());
        return;
    }
    const RH: f32 = 30.;
    const LANE: f32 = 16.;
    let rows: Vec<usize> = s.visible.clone();
    let row_of: HashMap<usize, usize> = rows.iter().enumerate().map(|(r, &i)| (i, r)).collect();
    let lane = |i: usize| (s.positions.get(i).map_or(0., |p| p.x / 260.)).min(24.);
    let lanes = rows.iter().map(|&i| lane(i)).fold(0f32, f32::max);
    let text_x = 18. + (lanes + 1.) * LANE + 10.;
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min)));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    let mut picked = None;
    let scroll = ScrollArea::vertical()
        .id_salt("map-commits")
        .auto_shrink([false, false])
        .show_rows(&mut child, RH, rows.len(), |ui, range| {
            let width = ui.available_width();
            let top = ui.cursor().top() - range.start as f32 * RH;
            let left = ui.max_rect().left();
            let painter = ui.painter().clone();
            let center = |r: usize, i: usize| pos2(left + 18. + lane(i) * LANE, top + r as f32 * RH + RH / 2.);
            for e in data.links.iter().filter(|e| e.kind == MapLinkKind::Parent) {
                let (Some(&child_row), Some(&parent_row)) = (row_of.get(&e.to), row_of.get(&e.from)) else { continue };
                if parent_row < range.start.saturating_sub(40) || child_row > range.end + 40 {
                    continue;
                }
                let (a, b) = (center(child_row, e.to), center(parent_row, e.from));
                let color = if s.selected == Some(e.to) || s.selected == Some(e.from) { BLUE() } else { BORDER() };
                if (a.x - b.x).abs() < 0.5 {
                    painter.line_segment([a, b], Stroke::new(1.6, color));
                } else {
                    let mid = a.y + RH * 0.6;
                    painter.add(epaint::CubicBezierShape::from_points_stroke([a, pos2(a.x, mid), pos2(b.x, mid), pos2(b.x, (mid + RH).min(b.y))], false, Color32::TRANSPARENT, Stroke::new(1.6, color)));
                    if b.y > mid + RH {
                        painter.line_segment([pos2(b.x, mid + RH), b], Stroke::new(1.6, color));
                    }
                }
            }
            for r in range {
                let i = rows[r];
                let n = &data.nodes[i];
                let band = Rect::from_min_size(pos2(left, top + r as f32 * RH), vec2(width, RH));
                let selected = s.selected == Some(i);
                if selected {
                    painter.rect_filled(band, 0., ACCENT_SOFT());
                }
                let dot = center(r, i);
                if n.count > 1 {
                    painter.circle_filled(dot, 5., VIEW_BG());
                    painter.circle_stroke(dot, 5., Stroke::new(2.2, BLUE()));
                } else {
                    painter.circle_filled(dot, 4.5, BLUE());
                }
                let response = ui.interact(band, ui.scope_id().with(("map-commit", i)), Sense::click());
                register(ui, widgets, &format!("map-node-{i}"), &response);
                if response.clicked() && !covered {
                    picked = Some(i);
                }
                let mut row = ui.new_child(UiBuilder::new().max_rect(Rect::from_min_max(pos2(left + text_x, band.top()), pos2(band.right() - 12., band.bottom()))).layout(Layout::left_to_right(Align::Center)));
                row.spacing_mut().item_spacing.x = 7.;
                ref_pills(&mut row, &n.refs);
                let subject = label(&n.label);
                let who = label(&n.detail);
                let id = n.commit.as_ref().map(|c| short(c).to_owned()).unwrap_or_default();
                let date = date(&data, i);
                row.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(date).size(12.).color(HINT()));
                    ui.label(RichText::new(id).monospace().size(11.5).color(FAINT()));
                    ui.add(Label::new(RichText::new(who).size(12.).color(HINT())).truncate());
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let text = RichText::new(subject);
                        ui.add(Label::new(if selected { text.strong() } else { text }).truncate());
                    });
                });
            }
        });
    if let Some(i) = picked {
        s.selected = Some(i);
        s.show_details = true;
    }
    if !covered {
        register(ui, widgets, "map-canvas", &ui.interact(scroll.inner_rect, ui.scope_id().with("map-commits-area"), Sense::hover()));
    }
}

/// People: one row a person, one column a folder, commits in the cells.
pub(super) fn grid(
    ui: &mut Ui,
    s: &mut MapPage,
    rect: Rect,
    covered: bool,
    widgets: &mut HashMap<String, UiControl>,
) {
    let Some(data) = s.data.clone() else {
        drawing_message(ui, rect, s);
        return;
    };
    let mut people: Vec<usize> = s.visible.iter().copied().filter(|&i| data.nodes[i].kind == MapNodeKind::Person).collect();
    people.sort_by(|a, b| data.nodes[*b].count.cmp(&data.nodes[*a].count).then_with(|| a.cmp(b)));
    let mut areas: Vec<usize> = (0..data.nodes.len()).filter(|&i| data.nodes[i].kind == MapNodeKind::Area).collect();
    if !s.query.is_empty() {
        areas.retain(|i| s.visible.contains(i));
    }
    areas.sort_by(|a, b| data.nodes[*b].count.cmp(&data.nodes[*a].count).then_with(|| a.cmp(b)));
    if people.is_empty() || areas.is_empty() {
        let painter = ui.painter().with_clip_rect(rect);
        painter.text(rect.center(), Align2::CENTER_CENTER, if s.query.is_empty() { tr("No commits in this period") } else { tr("No match among these people") }, FontId::proportional(13.), MUTED());
        return;
    }
    let mut cells: HashMap<(usize, usize), usize> = HashMap::new();
    for e in data.links.iter().filter(|e| e.kind == MapLinkKind::Changes) {
        *cells.entry((e.from, e.to)).or_default() += e.weight;
    }
    let most = cells.values().copied().max().unwrap_or(1).max(1);
    const NAMES: f32 = 210.;
    const CELL: Vec2 = vec2(46., 40.);
    const HEAD: f32 = 120.;
    let width = NAMES + areas.len() as f32 * CELL.x + 24.;
    let height = 44. + HEAD + people.len() as f32 * CELL.y + 48.;
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min)));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    let scroll = ScrollArea::both()
        .id_salt("map-grid")
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            let (area, _) = ui.allocate_exact_size(vec2(width.max(ui.available_width()), height), Sense::hover());
            let painter = ui.painter().clone();
            let origin = area.left_top() + vec2(20., 16.);
            painter.text(origin, Align2::LEFT_TOP, tr("Commits touching each folder"), semibold(14.), TEXT());
            let commits: usize = people.iter().map(|&p| data.nodes[p].count).sum();
            painter.text(
                origin + vec2(232., 2.),
                Align2::LEFT_TOP,
                // A person's total includes merges, which no cell counts.
                trf!("People {people} · commits {commits}", people = people.len(), commits = pages::number_text(commits)),
                FontId::proportional(12.),
                HINT(),
            );
            let grid = origin + vec2(NAMES - 20., 44. + HEAD);
            let selected_area = s.cell.map(|(_, a)| a);
            for (c, &a) in areas.iter().enumerate() {
                let n = &data.nodes[a];
                let full = n.path.as_ref().map(|p| label(p)).unwrap_or_else(|| label(&n.label));
                let name = if full == "/" { tr("top folder").to_owned() } else { full.rsplit('/').next().unwrap_or(&full).to_owned() };
                let on = selected_area == Some(a);
                let anchor = grid + vec2(c as f32 * CELL.x + 14., -8.);
                let galley = painter.layout_no_wrap(name, if on { semibold(12.) } else { FontId::proportional(12.) }, if on { BLUE() } else { MUTED() });
                painter.add(epaint::TextShape::new(anchor, galley, MUTED()).with_angle(-0.9));
                let head = Rect::from_min_size(grid + vec2(c as f32 * CELL.x, -HEAD + 10.), vec2(CELL.x - 4., HEAD - 14.));
                let r = ui.interact(head, ui.scope_id().with(("map-area", a)), Sense::click()).on_hover_text(&full);
                register(ui, widgets, &format!("map-node-{a}"), &r);
                if r.clicked() && !covered {
                    s.selected = Some(a);
                    s.cell = None;
                    s.show_details = true;
                }
            }
            for (row, &p) in people.iter().enumerate() {
                let n = &data.nodes[p];
                let y = grid.y + row as f32 * CELL.y;
                let on = s.selected == Some(p) || s.cell.map(|(q, _)| q) == Some(p);
                let names = Rect::from_min_size(pos2(origin.x, y), vec2(NAMES - 32., CELL.y - 4.));
                let r = ui.interact(names, ui.scope_id().with(("map-person", p)), Sense::click());
                register(ui, widgets, &format!("map-node-{p}"), &r);
                if r.clicked() && !covered {
                    s.selected = Some(p);
                    s.cell = None;
                    s.show_details = true;
                }
                if on || r.hovered() {
                    painter.rect_filled(names, 6., if on { ACCENT_SOFT() } else { HOVER() });
                }
                let initials: String = label(&n.label).split_whitespace().filter_map(|w| w.chars().next()).take(2).collect();
                let center = pos2(names.left() + 16., names.center().y);
                painter.circle_filled(center, 12., AVATAR());
                painter.text(center, Align2::CENTER_CENTER, initials.to_uppercase(), semibold(10.5), TEXT());
                let galley = painter.layout_job(line_job(label(&n.label), 13., if on { BLUE() } else { TEXT() }, NAMES - 110.));
                painter.galley(pos2(names.left() + 36., names.center().y - galley.size().y / 2.), galley, TEXT());
                painter.text(pos2(names.right() - 4., names.center().y), Align2::RIGHT_CENTER, pages::number_text(n.count), FontId::proportional(12.), HINT());
                for (c, &a) in areas.iter().enumerate() {
                    let cell = Rect::from_min_size(pos2(grid.x + c as f32 * CELL.x, y), CELL - vec2(4., 4.));
                    let weight = cells.get(&(p, a)).copied().unwrap_or(0);
                    let chosen = s.cell == Some((p, a));
                    if weight == 0 {
                        painter.rect_filled(cell, 6., VIEW_RAISED().gamma_multiply(0.5));
                        continue;
                    }
                    let share = weight as f32 / most as f32;
                    let (fill, ink) = if share > 0.75 {
                        (BLUE(), ACCENT_INK())
                    } else if share > 0.3 {
                        (RAMP_3(), TEXT())
                    } else if share > 0.12 {
                        (RAMP_2(), TEXT())
                    } else {
                        (RAMP_1(), TEXT())
                    };
                    painter.rect_filled(cell, 6., fill);
                    if chosen {
                        painter.rect_stroke(cell.expand(3.), 8., Stroke::new(2., TEXT()), StrokeKind::Inside);
                    }
                    painter.text(cell.center(), Align2::CENTER_CENTER, pages::number_text(weight), semibold(12.5), ink);
                    let r = ui.interact(cell, ui.scope_id().with(("map-cell", p, a)), Sense::click());
                    let full = data.nodes[a].path.as_ref().map(|x| label(x)).unwrap_or_default();
                    let r = r.on_hover_text(trf!("{person} · {folder} · commits {count}", person = label(&n.label), folder = full, count = weight));
                    register(ui, widgets, &format!("map-cell-{p}-{a}"), &r);
                    if r.clicked() && !covered {
                        s.cell = Some((p, a));
                        s.selected = Some(p);
                        s.show_details = true;
                    }
                }
            }
            // A key for the shades, and where the folders come from.
            let key = pos2(origin.x, grid.y + people.len() as f32 * CELL.y + 18.);
            painter.text(key, Align2::LEFT_CENTER, "1", FontId::proportional(12.), HINT());
            for (k, color) in [RAMP_1(), RAMP_2(), RAMP_3(), BLUE()].into_iter().enumerate() {
                painter.rect_filled(Rect::from_min_size(key + vec2(14. + k as f32 * 26., -5.), vec2(26., 10.)), 0., color);
            }
            painter.text(key + vec2(126., 0.), Align2::LEFT_CENTER, trn!(most, "{n} commit", "{n} commits"), FontId::proportional(12.), HINT());
            if areas.iter().any(|&a| data.nodes[a].path.as_ref().is_some_and(|p| p.contains(&b'/'))) {
                painter.text(key + vec2(230., 0.), Align2::LEFT_CENTER, tr("A folder holding most changes is split into its subfolders"), FontId::proportional(12.), HINT());
            }
        });
    if !covered {
        register(ui, widgets, "map-canvas", &ui.interact(scroll.inner_rect, ui.scope_id().with("map-grid-area"), Sense::hover()));
    }
}
