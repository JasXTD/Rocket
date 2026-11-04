use super::*;
use std::collections::BTreeSet;

pub(super) fn layout(data: &RepositoryMap, visible: &[usize], alternate: bool) -> Vec<Pos2> {
    let mut positions = vec![Pos2::ZERO; data.nodes.len()];
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
            for (row, &i) in visible.iter().enumerate() {
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
        MapKind::People if !alternate => {
            let mut person = 0;
            let mut area = 0;
            for &i in visible {
                if data.nodes[i].kind == MapNodeKind::Person {
                    positions[i] = pos2(0., person as f32 * 96.);
                    person += 1;
                } else {
                    positions[i] = pos2(390., area as f32 * 96.);
                    area += 1;
                }
            }
        }
        MapKind::Code if !alternate => {
            let mut rows = HashMap::new();
            for &i in visible {
                let depth = data.nodes[i]
                    .path
                    .as_ref()
                    .map_or(0, |p| p.iter().filter(|b| **b == b'/').count())
                    .min(7);
                let row = rows.entry(depth).or_insert(0);
                positions[i] = pos2(depth as f32 * 270., *row as f32 * 90.);
                *row += 1;
            }
        }
        _ => {
            for (at, &i) in visible.iter().enumerate() {
                positions[i] = pos2((at % 3) as f32 * 285., (at / 3) as f32 * 150.);
            }
        }
    }
    positions
}
fn world_bounds(s: &MapPage) -> Rect {
    s.visible.iter().fold(Rect::NOTHING, |r, i| {
        r.union(Rect::from_min_size(s.positions[*i], vec2(214., 66.)))
    })
}
fn focus(s: &mut MapPage, rect: Rect) {
    if let Some(i) = s.selected.filter(|i| s.visible.contains(i)) {
        s.zoom = 0.95;
        s.pan = rect.size() / 2. - (s.positions[i].to_vec2() + vec2(107., 33.)) * s.zoom;
    }
}
pub(super) fn fit(s: &mut MapPage, rect: Rect) {
    let bounds = world_bounds(s);
    if bounds.is_finite() && bounds.is_positive() {
        s.zoom = ((rect.width() - 60.).max(1.) / bounds.width())
            .min((rect.height() - 90.).max(1.) / bounds.height())
            .clamp(0.00001, 1.1);
        s.pan = (rect.size() - bounds.size() * s.zoom) / 2. - bounds.min.to_vec2() * s.zoom;
    }
}
pub(super) fn title_job(title: String, zoom: f32, width: f32) -> text::LayoutJob {
    let title = if title.chars().count() > 160 {
        format!("{}…", title.chars().take(160).collect::<String>())
    } else {
        title
    };
    let mut job = text::LayoutJob::simple(
        title,
        FontId::proportional((13. * zoom).max(8.)),
        TEXT(),
        width,
    );
    job.wrap.max_rows = 2;
    job.wrap.break_anywhere = true;
    job
}
pub(super) fn canvas(
    ui: &mut Ui,
    s: &mut MapPage,
    rect: Rect,
    covered: bool,
    widgets: &mut HashMap<String, UiControl>,
) {
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    painter.rect_filled(rect, 0., BG());
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
        // the complete bounded graph, including tall directory listings.
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
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            if s.running() {
                "Reading the local repository…"
            } else if s.error.is_some() {
                "Map unavailable · check the message and retry"
            } else {
                "Choose Refresh to build this map"
            },
            FontId::proportional(13.),
            MUTED(),
        );
        return;
    };
    if s.visible.is_empty() {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            if !s.query.is_empty() {
                "No matching nodes · clear the search"
            } else if data.nodes.is_empty() {
                "No committed data in this scope"
            } else {
                "No connections of this type · change the filter"
            },
            FontId::proportional(13.),
            MUTED(),
        );
        return;
    }
    let visible = s.visible.iter().copied().collect::<HashSet<_>>();
    let at = |i: usize| rect.min + s.pan + s.positions[i].to_vec2() * s.zoom;
    for edge in s
        .projected_links
        .iter()
        .filter(|e| s.edge_kind(e.kind) && visible.contains(&e.from) && visible.contains(&e.to))
    {
        let a = at(edge.from) + vec2(107., 33.) * s.zoom;
        let b = at(edge.to) + vec2(107., 33.) * s.zoom;
        let active = s.selected == Some(edge.from) || s.selected == Some(edge.to);
        let color = if active { BLUE() } else { BORDER() };
        let width = if matches!(edge.kind, MapLinkKind::Changes | MapLinkKind::SharedFiles) {
            (edge.weight as f32).sqrt().clamp(1., 4.)
        } else if active {
            1.8
        } else {
            1.
        };
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let start = if dx.abs() > dy.abs() {
            a + vec2(dx.signum() * 107. * s.zoom, 0.)
        } else {
            a + vec2(0., dy.signum() * 33. * s.zoom)
        };
        let end = if dx.abs() > dy.abs() {
            b - vec2(dx.signum() * 107. * s.zoom, 0.)
        } else {
            b - vec2(0., dy.signum() * 33. * s.zoom)
        };
        let middle = pos2((start.x + end.x) / 2., (start.y + end.y) / 2.);
        let points = if dx.abs() > dy.abs() {
            [start, pos2(middle.x, start.y), pos2(middle.x, end.y), end]
        } else {
            [start, pos2(start.x, middle.y), pos2(end.x, middle.y), end]
        };
        painter.add(epaint::CubicBezierShape::from_points_stroke(
            points,
            false,
            Color32::TRANSPARENT,
            Stroke::new(width, color),
        ));
        if matches!(
            edge.kind,
            MapLinkKind::Parent | MapLinkKind::DependsOn | MapLinkKind::Contains
        ) {
            let direction = (end - points[2]).normalized();
            let cross = vec2(-direction.y, direction.x);
            painter.add(Shape::convex_polygon(
                vec![
                    end,
                    end - direction * 6. + cross * 3.,
                    end - direction * 6. - cross * 3.,
                ],
                color,
                Stroke::NONE,
            ));
        }
        if active
            && s.zoom >= 0.65
            && matches!(edge.kind, MapLinkKind::Changes | MapLinkKind::SharedFiles)
        {
            painter.rect_filled(Rect::from_center_size(middle, vec2(28., 16.)), 3., BG());
            painter.text(
                middle,
                Align2::CENTER_CENTER,
                edge.weight.to_string(),
                FontId::proportional(10.),
                MUTED(),
            );
        }
    }
    let mut picked = None;
    for &i in &s.visible {
        let n = &data.nodes[i];
        let node = Rect::from_min_size(at(i), vec2(214., 66.) * s.zoom);
        if !node.intersects(rect) {
            continue;
        }
        let selected = s.selected == Some(i);
        painter.rect_filled(node, 6., if selected { SELECTED() } else { RAISED() });
        painter.rect_stroke(
            node,
            6.,
            Stroke::new(
                if selected { 1.7 } else { 1. },
                if selected { BLUE() } else { BORDER() },
            ),
            StrokeKind::Inside,
        );
        let ink = painter.with_clip_rect(node.shrink(5.).intersect(rect));
        let title = label(&n.label);
        if s.zoom >= 0.4 {
            let title_galley = ink.layout_job(title_job(
                title.clone(),
                s.zoom,
                (node.width() - 18.).max(1.),
            ));
            ink.galley(
                node.left_top() + vec2(9., 8.) * s.zoom,
                title_galley,
                TEXT(),
            );
        }
        if s.zoom > 0.7 {
            ink.text(
                node.left_bottom() + vec2(9., -12.) * s.zoom,
                Align2::LEFT_CENTER,
                if let Some(id) = &n.commit {
                    format!("{} · {}", &id.as_str()[..8], label(&n.detail))
                } else if s.collapsed.contains(&n.key) {
                    format!("Collapsed group · {} files", n.files)
                } else {
                    label(&n.detail)
                },
                FontId::proportional(10. * s.zoom),
                MUTED(),
            );
        }
        let hit = node.intersect(rect.shrink2(vec2(0., 36.)));
        if !covered && hit.width() > 20. && hit.height() > 18. {
            let response = ui
                .interact(hit, ui.id().with(("map-node", i)), Sense::click())
                .on_hover_text(format!("{}\n{}", title, label(&n.detail)));
            register(ui, widgets, &format!("map-node-{i}"), &response);
            if response.clicked() {
                picked = Some(i);
            }
        }
    }
    if let Some(i) = picked {
        s.selected = Some(i);
        s.show_details = true;
    }
    let text = format!(
        "Graph: {} / {} nodes · {} records · {}{}",
        s.visible.len(),
        data.nodes.len(),
        data.scanned,
        data.tip
            .as_ref()
            .map(|id| &id.as_str()[..8])
            .unwrap_or("unborn"),
        if data.limited { " · partial map" } else { "" }
    );
    painter.rect_filled(
        Rect::from_min_size(rect.min, vec2(rect.width(), 30.)),
        0.,
        BG().gamma_multiply(0.95),
    );
    painter.text(
        rect.left_top() + vec2(12., 15.),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(10.),
        MUTED(),
    );
    if covered {
        return;
    }
    let bar = Rect::from_min_max(
        pos2(
            (rect.right() - 277.).max(rect.left() + 6.),
            rect.bottom() - 37.,
        ),
        rect.right_bottom() - vec2(6., 4.),
    );
    painter.rect_filled(bar, 5., PANEL());
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(bar.shrink(3.))
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            if button(ui, widgets, "map-zoom-out", "−", true) {
                zoom_at(s, 0.8, rect.size() / 2.);
            }
            ui.label(RichText::new(format!("{}%", (s.zoom * 100.).round())).size(10.));
            if button(ui, widgets, "map-zoom-in", "+", true) {
                zoom_at(s, 1.25, rect.size() / 2.);
            }
            if button(ui, widgets, "map-focus", "Focus", s.selected.is_some()) {
                s.focus = true;
            }
            if button(ui, widgets, "map-fit", "Fit", true) {
                fit(s, rect);
            }
        },
    );
}
fn zoom_at(s: &mut MapPage, factor: f32, point: Vec2) {
    let next = (s.zoom * factor).clamp(0.00001, 2.);
    s.pan = point - (point - s.pan) * (next / s.zoom);
    s.zoom = next;
}
