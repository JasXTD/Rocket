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
                let r = ui.interact(pill, ui.id().with(("map-tip", k)), Sense::click()).on_hover_text(text);
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
                let response = ui.interact(band, ui.id().with(("map-line", k)), Sense::click());
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
        register(ui, widgets, "map-canvas", &ui.interact(scroll.inner_rect, ui.id().with("map-topology-area"), Sense::hover()));
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
                let response = ui.interact(band, ui.id().with(("map-commit", i)), Sense::click());
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
        register(ui, widgets, "map-canvas", &ui.interact(scroll.inner_rect, ui.id().with("map-commits-area"), Sense::hover()));
    }
}

/// People table metrics, in points: a person's column, a folder's row, the people row,
/// and the room left of the folder column for a selection band to start in.
const PERSON_WIDTH: f32 = 72.;
const FOLDER_HEIGHT: f32 = 36.;
const PEOPLE_HEIGHT: f32 = 64.;
const INSET: f32 = 8.;

/// "1 person · 91 commits": the people in the table and their commits. A person's
/// total includes merges, which no cell counts.
pub(super) fn people_counts(people: usize, commits: usize) -> String {
    format!(
        "{} · {}",
        trn!(people, "{count} person", "{count} people", count = pages::number_text(people)),
        trn!(commits, "{found} commit", "{found} commits", found = pages::number_text(commits)),
    )
}

/// A cell's fill and ink: the larger its share of the largest cell, the stronger.
fn shade(weight: usize, most: usize) -> (Color32, Color32) {
    let share = weight as f32 / most as f32;
    if share > 0.75 {
        (BLUE(), ACCENT_INK())
    } else if share > 0.3 {
        (RAMP_3(), TEXT())
    } else if share > 0.12 {
        (RAMP_2(), TEXT())
    } else {
        (RAMP_1(), TEXT())
    }
}

/// The key beside the title: what a cell counts, then 1, the four shades and the
/// largest count.
struct Key {
    label: Arc<Galley>,
    one: Arc<Galley>,
    most: Arc<Galley>,
    width: f32,
}
impl Key {
    const SWATCH: Vec2 = vec2(16., 10.);
    fn new(painter: &Painter, most: usize) -> Self {
        let font = FontId::proportional(12.);
        let label = painter.layout_no_wrap(tr("Commits in a cell").into(), font.clone(), HINT());
        let one = painter.layout_no_wrap("1".into(), font.clone(), HINT());
        let most = painter.layout_no_wrap(pages::number_text(most), font, HINT());
        let width = label.size().x + one.size().x + most.size().x + 4. * Self::SWATCH.x + 3. * 2. + 3. * 7.;
        Self { label, one, most, width }
    }
    /// Paints the key from `at`, the left end of its centre line.
    fn paint(&self, painter: &Painter, at: Pos2) {
        let mut x = at.x;
        for galley in [&self.label, &self.one] {
            painter.galley(pos2(x, at.y - galley.size().y / 2.), galley.clone(), HINT());
            x += galley.size().x + 7.;
        }
        for color in [RAMP_1(), RAMP_2(), RAMP_3(), BLUE()] {
            painter.rect_filled(Rect::from_min_size(pos2(x, at.y - Self::SWATCH.y / 2.), Self::SWATCH), 2., color);
            x += Self::SWATCH.x + 2.;
        }
        painter.galley(pos2(x + 5., at.y - self.most.size().y / 2.), self.most.clone(), HINT());
    }
}

/// One line of text in `font`, cut with … at `width`.
fn single_line(text: String, font: FontId, color: Color32, width: f32) -> text::LayoutJob {
    let mut job = text::LayoutJob::simple_singleline(text, font, color);
    job.wrap = text::TextWrapping::truncate_at_width(width.max(1.));
    job
}

/// A folder's row label: the parent path faint and the folder's own name in full
/// colour, so crates/rocket-git says where rocket-git is. Files at the top of the
/// repository read "top folder".
fn folder_job(node: &MapNode, on: bool, width: f32) -> text::LayoutJob {
    let path = node.path.as_ref().map(|p| label(p)).unwrap_or_else(|| label(&node.label));
    let font = if on { semibold(13.) } else { FontId::proportional(13.) };
    let mut job = text::LayoutJob::default();
    if path == "/" {
        job.append(tr("top folder"), 0., text::TextFormat::simple(font, if on { BLUE() } else { MUTED() }));
    } else {
        let (parent, name) = path.rsplit_once('/').unwrap_or(("", path.as_str()));
        if !parent.is_empty() {
            job.append(&format!("{parent}/"), 0., text::TextFormat::simple(FontId::proportional(13.), FAINT()));
        }
        job.append(name, 0., text::TextFormat::simple(font, if on { BLUE() } else { TEXT() }));
    }
    job.wrap = text::TextWrapping::truncate_at_width(width.max(1.));
    job
}

/// A shadow along an edge that content scrolls under, fading away from `rect`'s left
/// (`across`) or top side.
fn fade(painter: &Painter, rect: Rect, color: Color32, across: bool) {
    let clear = Color32::TRANSPARENT;
    let mut mesh = epaint::Mesh::default();
    mesh.colored_vertex(rect.left_top(), color);
    mesh.colored_vertex(rect.right_top(), if across { clear } else { color });
    mesh.colored_vertex(rect.right_bottom(), clear);
    mesh.colored_vertex(rect.left_bottom(), if across { color } else { clear });
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(mesh);
}

/// People: one row a folder, one column a person, their commits touching the folder in
/// the cells. The folder column stays at the left while the table scrolls, and the
/// people row at the top when there is room for rows under it; pointing at a cell
/// lights its row and column.
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

    // The title and counts, then the key: on the same line when all three fit, else
    // on a line of its own. In a narrow card the title and counts end in … instead of
    // running past its edge.
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let left = rect.left() + 19.;
    let top = rect.top() + 15.;
    let room = rect.width() - 38.;
    let title = painter.layout_job(single_line(tr("Commits touching each folder").into(), semibold(14.), TEXT(), room));
    let commits: usize = people.iter().map(|&p| data.nodes[p].count).sum();
    let counts_room = room - title.size().x - 12.;
    let counts = (counts_room >= 40.).then(|| painter.layout_job(single_line(people_counts(people.len(), commits), FontId::proportional(12.), HINT(), counts_room)));
    let key = Key::new(&painter, most);
    let used = title.size().x + counts.as_ref().map_or(0., |c| 12. + c.size().x);
    let one_line = used + 24. + key.width <= room;
    let title_size = title.size();
    painter.galley(pos2(left, top), title, TEXT());
    if let Some(counts) = counts {
        painter.galley(pos2(left + title_size.x + 12., top + 2.), counts, HINT());
    }
    let table_top = if one_line {
        key.paint(&painter, pos2(rect.right() - 19. - key.width, top + title_size.y / 2.));
        rect.top() + 51.
    } else {
        key.paint(&painter, pos2(left, top + title_size.y + 13.));
        rect.top() + 73.
    };

    // The folder column is long enough for crates/rocket-integrations in a wide card.
    let side = INSET + if rect.width() >= 800. { 248. } else { 180. };
    let totals = people.len() > 1;
    let content = vec2(side + people.len() as f32 * PERSON_WIDTH, PEOPLE_HEIGHT + 4. + areas.len() as f32 * FOLDER_HEIGHT);
    let table = Rect::from_min_max(pos2(rect.left() + 11., table_top), pos2(rect.right() - 19., rect.bottom() - 6.));
    let mut child = ui.new_child(UiBuilder::new().max_rect(table).layout(Layout::top_down(Align::Min)));
    child.set_clip_rect(table.intersect(ui.clip_rect()));
    // egui's own scroll fades would wash over the fixed people row and folder column;
    // their edges get the shadows below instead.
    child.spacing_mut().scroll.fade.strength = 0.;
    // The node to select and, for a cell, the person and folder.
    let mut picked: Option<(usize, Option<(usize, usize)>)> = None;
    let scroll = ScrollArea::both()
        .id_salt("map-grid")
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            let (area, _) = ui.allocate_exact_size(content.max(ui.available_size()), Sense::hover());
            let view = ui.clip_rect();
            let origin = area.min;
            // The folder column sits at the view's left edge. The people row sits at its
            // top while two folder rows fit under it; in a shorter view it scrolls with
            // the rows, so every row can still be brought into view. Cells scroll under
            // both and are clipped to what is left.
            let pinned = view.height() >= PEOPLE_HEIGHT + 4. + 2. * FOLDER_HEIGHT;
            let head_top = if pinned { view.top() } else { origin.y };
            let body_top = if pinned { view.top() + PEOPLE_HEIGHT + 4. } else { view.top() };
            let head_clip = Rect::from_min_max(pos2(view.left() + side, head_top), pos2(view.right(), head_top + PEOPLE_HEIGHT)).intersect(view);
            let side_clip = Rect::from_min_max(pos2(view.left(), body_top), pos2(view.left() + side, view.bottom())).intersect(view);
            let body_clip = Rect::from_min_max(pos2(view.left() + side, body_top), view.max).intersect(view);
            let column_x = |c: usize| origin.x + side + c as f32 * PERSON_WIDTH;
            let row_y = |r: usize| origin.y + PEOPLE_HEIGHT + 4. + r as f32 * FOLDER_HEIGHT;
            let cell_rect = |c: usize, r: usize| Rect::from_min_size(pos2(column_x(c) + 2., row_y(r) + 3.), vec2(PERSON_WIDTH - 4., FOLDER_HEIGHT - 6.));
            // Only what can be seen is laid out: a repository can have thousands of people.
            let columns = {
                let last = (((view.right() - origin.x - side) / PERSON_WIDTH).ceil().max(0.) as usize).min(people.len());
                (((view.left() - origin.x) / PERSON_WIDTH).floor().max(0.) as usize).min(last)..last
            };
            let rows = {
                let last = (((view.bottom() - origin.y - PEOPLE_HEIGHT - 4.) / FOLDER_HEIGHT).ceil().max(0.) as usize).min(areas.len());
                (((body_top - origin.y - PEOPLE_HEIGHT - 4.) / FOLDER_HEIGHT).floor().max(0.) as usize).min(last)..last
            };
            let column_at = |x: f32| {
                let c = ((x - origin.x - side) / PERSON_WIDTH).floor();
                (c >= 0. && (c as usize) < people.len()).then_some(c as usize)
            };
            let row_at = |y: f32| {
                let r = ((y - origin.y - PEOPLE_HEIGHT - 4.) / FOLDER_HEIGHT).floor();
                (r >= 0. && (r as usize) < areas.len()).then_some(r as usize)
            };
            // What the pointer is over: nothing while the drawer covers the table, while
            // something is dragged (a scroll bar, say) or over the scroll bars' strips.
            let bars = ui.spacing().scroll.bar_width + ui.spacing().scroll.bar_outer_margin;
            let mut free = view;
            if content.y > view.height() {
                free.max.x -= bars;
            }
            if content.x > view.width() {
                free.max.y -= bars;
            }
            let pointer = if covered || ui.ctx().dragged_id().is_some() { None } else { ui.ctx().pointer_hover_pos().filter(|p| free.contains(*p)) };
            let over = |clip: Rect| pointer.filter(|p| clip.contains(*p) && ui.rect_contains_pointer(clip));
            let (mut hover_row, mut hover_column) = (None, None);
            if let Some(p) = over(body_clip) {
                if let (Some(r), Some(c)) = (row_at(p.y), column_at(p.x)) {
                    (hover_row, hover_column) = (Some(r), Some(c));
                }
            } else if let Some(p) = over(head_clip) {
                hover_column = column_at(p.x);
            } else if let Some(p) = over(side_clip) {
                hover_row = row_at(p.y);
            }

            // Clicks: a person heads a column, a folder starts a row, a cell is both.
            // While the details drawer covers the table, none of them takes a click.
            if !covered {
                for c in columns.clone() {
                    let p = people[c];
                    let hit = Rect::from_min_size(pos2(column_x(c), head_top), vec2(PERSON_WIDTH, PEOPLE_HEIGHT)).intersect(head_clip);
                    if !hit.is_positive() {
                        continue;
                    }
                    let r = ui.interact(hit, ui.id().with(("map-person", p)), Sense::click()).on_hover_text(label(&data.nodes[p].label));
                    register(ui, widgets, &format!("map-node-{p}"), &r);
                    if r.clicked() {
                        picked = Some((p, None));
                    }
                }
                for r in rows.clone() {
                    let a = areas[r];
                    let hit = Rect::from_min_size(pos2(view.left(), row_y(r)), vec2(side, FOLDER_HEIGHT)).intersect(side_clip);
                    if !hit.is_positive() {
                        continue;
                    }
                    let mut response = ui.interact(hit, ui.id().with(("map-area", a)), Sense::click());
                    if data.nodes[a].path.as_deref() != Some(b"/") {
                        response = response.on_hover_text(details::area_name(&data, a));
                    }
                    register(ui, widgets, &format!("map-node-{a}"), &response);
                    if response.clicked() {
                        picked = Some((a, None));
                    }
                }
                for r in rows.clone() {
                    let a = areas[r];
                    for c in columns.clone() {
                        let p = people[c];
                        let weight = cells.get(&(p, a)).copied().unwrap_or(0);
                        let hit = cell_rect(c, r).intersect(body_clip);
                        if weight == 0 || !hit.is_positive() {
                            continue;
                        }
                        let response = ui.interact(hit, ui.id().with(("map-cell", p, a)), Sense::click()).on_hover_ui(|ui| {
                            ui.label(format!(
                                "{} · {} · {}",
                                label(&data.nodes[p].label),
                                details::area_name(&data, a),
                                trn!(weight, "{found} commit", "{found} commits", found = pages::number_text(weight)),
                            ));
                        });
                        register(ui, widgets, &format!("map-cell-{p}-{a}"), &response);
                        if response.clicked() {
                            picked = Some((p, Some((p, a))));
                        }
                    }
                }
            }

            // Bands under the cells: the selected folder or person, then the pointed-at
            // row and column.
            let painter = ui.painter().clone();
            let band = TEXT().gamma_multiply(0.05);
            let right = origin.x + content.x;
            let bottom = origin.y + content.y;
            let selected_row = areas.iter().position(|&a| s.selected == Some(a));
            let selected_column = if s.cell.is_none() { people.iter().position(|&p| s.selected == Some(p)) } else { None };
            let rows_clip = Rect::from_min_max(pos2(view.left(), body_top), view.max).intersect(view);
            for (row, fill) in [(selected_row, ACCENT_SOFT()), (hover_row, band)] {
                if let Some(r) = row {
                    let y = row_y(r);
                    painter.with_clip_rect(rows_clip).rect_filled(Rect::from_min_max(pos2(view.left(), y + 1.), pos2(right, y + FOLDER_HEIGHT - 1.)), 6., fill);
                }
            }
            let columns_clip = Rect::from_min_max(pos2(view.left() + side, view.top()), view.max).intersect(view);
            for (column, fill) in [(selected_column, ACCENT_SOFT()), (hover_column, band)] {
                if let Some(c) = column {
                    let x = column_x(c);
                    painter.with_clip_rect(columns_clip).rect_filled(Rect::from_min_max(pos2(x + 2., head_top), pos2(x + PERSON_WIDTH - 2., bottom + 2.)), 6., fill);
                }
            }

            // Cells: an empty one is a faint box, a full one is shaded by its count.
            let ink = painter.with_clip_rect(body_clip);
            for r in rows.clone() {
                let a = areas[r];
                for c in columns.clone() {
                    let p = people[c];
                    let cell = cell_rect(c, r);
                    let weight = cells.get(&(p, a)).copied().unwrap_or(0);
                    if weight == 0 {
                        ink.rect_filled(cell, 6., VIEW_RAISED().gamma_multiply(0.5));
                        continue;
                    }
                    let (fill, text) = shade(weight, most);
                    ink.rect_filled(cell, 6., fill);
                    if s.cell == Some((p, a)) {
                        ink.rect_stroke(cell.expand(3.), 8., Stroke::new(2., TEXT()), StrokeKind::Inside);
                    } else if hover_row == Some(r) && hover_column == Some(c) {
                        ink.rect_stroke(cell.expand(1.5), 7.5, Stroke::new(1.5, TEXT()), StrokeKind::Inside);
                    }
                    ink.text(cell.center(), Align2::CENTER_CENTER, pages::number_text(weight), semibold(12.5), text);
                }
            }

            // The people row: initials, name and commits over each column.
            let heads = painter.with_clip_rect(head_clip);
            for c in columns.clone() {
                let n = &data.nodes[people[c]];
                let name = label(&n.label);
                let x = column_x(c);
                let on = s.selected == Some(people[c]);
                let center = pos2(x + PERSON_WIDTH / 2., head_top + 17.);
                heads.circle_filled(center, 12., AVATAR());
                if on {
                    heads.circle_stroke(center, 12.75, Stroke::new(1.5, BLUE()));
                }
                heads.text(center, Align2::CENTER_CENTER, details::initials(&name), semibold(10.5), TEXT());
                let color = if on { BLUE() } else if hover_column == Some(c) { TEXT() } else { MUTED() };
                let galley = heads.layout_job(single_line(name, if on { semibold(12.) } else { FontId::proportional(12.) }, color, PERSON_WIDTH - 6.));
                heads.galley(pos2(x + (PERSON_WIDTH - galley.size().x) / 2., head_top + 32.), galley, color);
                heads.text(pos2(x + PERSON_WIDTH / 2., head_top + 55.), Align2::CENTER_CENTER, pages::number_text(n.count), FontId::proportional(11.5), HINT());
            }

            // The folder column: icon, path and, with two or more people, commits.
            let names = painter.with_clip_rect(side_clip);
            for r in rows.clone() {
                let a = areas[r];
                let n = &data.nodes[a];
                let y = row_y(r);
                let on = s.selected == Some(a) || s.cell.map(|(_, b)| b) == Some(a);
                icon(&names, Rect::from_min_size(pos2(view.left() + INSET, y + (FOLDER_HEIGHT - 14.) / 2.), vec2(14., 14.)), Icon::Folder, TEAL());
                let mut room = side - INSET - 22. - 12.;
                if totals {
                    let total = names.layout_no_wrap(pages::number_text(n.count), FontId::proportional(12.), HINT());
                    room -= total.size().x + 8.;
                    names.galley(pos2(view.left() + side - 12. - total.size().x, y + (FOLDER_HEIGHT - total.size().y) / 2.), total, HINT());
                }
                let galley = names.layout_job(folder_job(n, on, room));
                names.galley(pos2(view.left() + INSET + 22., y + (FOLDER_HEIGHT - galley.size().y) / 2.), galley, TEXT());
            }

            // Column captions, the line under the people row, and shadows where the
            // table has scrolled under its fixed edges.
            let corner = Rect::from_min_max(pos2(view.left(), head_top), pos2(view.left() + side, head_top + PEOPLE_HEIGHT)).intersect(view);
            let captions = painter.with_clip_rect(corner);
            captions.text(pos2(view.left() + INSET, head_top + PEOPLE_HEIGHT - 15.), Align2::LEFT_CENTER, tr("FOLDER"), semibold(10.5), FAINT());
            if totals {
                captions.text(pos2(view.left() + side - 12., head_top + PEOPLE_HEIGHT - 15.), Align2::RIGHT_CENTER, tr("COMMITS"), semibold(10.5), FAINT());
            }
            let edges = painter.with_clip_rect(view);
            edges.hline(view.left()..=right.min(view.right()), head_top + PEOPLE_HEIGHT + 0.5, Stroke::new(1., CARD_LINE()));
            let shadow = Color32::from_black_alpha(if ui.visuals().dark_mode { 90 } else { 36 });
            if origin.x < view.left() - 0.5 {
                fade(&edges, Rect::from_min_size(pos2(view.left() + side, view.top()), vec2(10., view.height())), shadow, true);
            }
            if pinned && origin.y < view.top() - 0.5 {
                fade(&edges, Rect::from_min_size(pos2(view.left(), view.top() + PEOPLE_HEIGHT + 1.), vec2(view.width(), 8.)), shadow, false);
            }
        });
    if let Some((node, cell)) = picked {
        s.selected = Some(node);
        s.cell = cell;
        s.show_details = true;
    }
    if !covered {
        register(ui, widgets, "map-canvas", &ui.interact(scroll.inner_rect, ui.id().with("map-grid-area"), Sense::hover()));
    }
}
