//! The native panel renderer. A package describes a view; Rocket draws every part of it.
use super::*;
use eframe::egui::text::{LayoutJob, TextWrapping};
use eframe::egui::{Galley, UiBuilder};
use rocket_integrations::Cell;
use std::sync::Weak;

/// Below this width a split's two panes become tabs.
const SPLIT_MIN: f32 = 760.0;
/// Below this width, outside a pane, a table leaves its columns.
const TABLE_MIN: f32 = 700.0;
const ROW: f32 = 36.0;
const TWO_LINES: f32 = 54.0;
/// A table's inset from the edge of its card, and the space between its columns.
const EDGE: f32 = 14.0;
const GAP: f32 = 10.0;
/// A cell's width counts this much at most when columns share out the room.
const LONGEST: f32 = 420.0;

/// What a control in a panel asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PanelEvent {
    pub(crate) event: String,
    /// The key of the table row that was clicked.
    pub(crate) row: Option<String>,
}
impl From<&str> for PanelEvent {
    fn from(event: &str) -> Self {
        Self { event: event.to_owned(), row: None }
    }
}

/// What a panel draws from and reports to.
pub(crate) struct Panel<'a> {
    values: &'a mut HashMap<String, Value>,
    widgets: &'a mut HashMap<String, UiControl>,
    action: &'a mut Option<PanelEvent>,
    changed: &'a mut bool,
    prefix: &'a str,
    /// Inside a side-by-side pane, where a table keeps its columns.
    pane: bool,
    /// The room a row leaves its one input beside its buttons.
    field: Option<f32>,
    /// The result being drawn. Holding it keeps its address from going to a later one.
    view: Weak<Node>,
    /// Splits drawn so far, so each has its own tabs.
    splits: usize,
}
impl<'a> Panel<'a> {
    pub(crate) fn new(
        values: &'a mut HashMap<String, Value>,
        widgets: &'a mut HashMap<String, UiControl>,
        action: &'a mut Option<PanelEvent>,
        changed: &'a mut bool,
        prefix: &'a str,
    ) -> Self {
        Self { values, widgets, action, changed, prefix, pane: false, field: None, view: Weak::new(), splits: 0 }
    }
    /// The result this panel draws: a tab chosen by hand lasts as long as it does.
    pub(crate) fn showing(mut self, view: &Arc<Node>) -> Self {
        self.view = Arc::downgrade(view);
        self
    }
}

fn tone_color(tone: &str) -> Option<Color32> {
    match tone {
        "success" => Some(GREEN()),
        "warning" => Some(AMBER()),
        "error" => Some(RED()),
        "accent" => Some(BLUE()),
        _ => None,
    }
}

/// Draws a package's declared layout with Rocket's spacing: a 10-point rhythm, labels above
/// fields, cards with a title strip, the package's primary button in the accent and badges
/// as pills.
pub(crate) fn render_node(ui: &mut Ui, n: &Node, panel: &mut Panel, enabled: bool) {
    let key = n.id.as_ref().map(|id| format!("{}{id}", panel.prefix));
    let enabled = enabled && !n.disabled;
    ui.push_id(n.id.as_deref().unwrap_or(&n.kind), |ui| {
        match n.kind.as_str() {
            "page" | "column" => {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = if n.small { 4.0 } else { 10.0 };
                    if !n.label.is_empty() {
                        ui.label(RichText::new(&n.label).font(semibold(if n.kind == "page" { 16.0 } else { 14.0 })));
                    }
                    for c in &n.children {
                        render_node(ui, c, panel, enabled);
                    }
                });
            }
            "row" => {
                let inputs = n.children.iter().filter(|c| c.kind == "input").count();
                let padding = ui.spacing().button_padding.x * 2.0;
                let buttons: f32 = n
                    .children
                    .iter()
                    .filter(|c| c.kind == "button")
                    .map(|c| ui.painter().layout_no_wrap(c.label.clone(), FontId::proportional(13.0), TEXT()).size().x + padding + 12.0)
                    .sum();
                let outer = panel.field;
                panel.field = (inputs == 1).then(|| (ui.available_width() - buttons).max(160.0));
                // Fields carry their label above, so the row lines up on the bottom edge.
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(10.0, 8.0);
                    ui.with_layout(Layout::left_to_right(Align::Max).with_main_wrap(true), |ui| {
                        for c in &n.children {
                            render_node(ui, c, panel, enabled);
                        }
                    });
                });
                panel.field = outer;
            }
            "card" => card(ui, n, panel, enabled, true),
            "split" => split(ui, n, panel, enabled),
            "heading" => {
                ui.label(RichText::new(&n.label).font(semibold(14.0)));
            }
            "text" => {
                let color = tone_color(&n.tone).unwrap_or(if n.small { FAINT() } else { MUTED() });
                let text = RichText::new(&n.label).color(color);
                ui.add(Label::new(if n.small { text.size(12.0) } else { text }).wrap());
            }
            "code" => {
                Frame::new()
                    .fill(BG())
                    .stroke(Stroke::new(1.0, HAIRLINE()))
                    .corner_radius(8)
                    .inner_margin(Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.add(Label::new(RichText::new(&n.label).monospace().size(12.0).line_height(Some(19.0)).color(MUTED())).wrap());
                    });
            }
            "badge" => {
                pages::pill(ui, &n.label, tone_color(&n.tone).unwrap_or(MUTED()));
            }
            "button" => {
                let response = if n.primary {
                    pages::primary(ui, enabled, &n.label)
                } else {
                    ui.add_enabled(enabled, Button::new(&n.label))
                };
                pages::control(panel.widgets, key.as_deref().unwrap(), &response, ui.clip_rect());
                if response.clicked() {
                    *panel.action = n.id.as_deref().map(PanelEvent::from);
                }
            }
            "input" | "multiline" => {
                let id = n.id.as_ref().unwrap();
                *panel.changed |= !panel.values.contains_key(id);
                let value = panel.values.entry(id.clone()).or_insert_with(|| n.value.clone());
                let mut s = value.as_str().unwrap_or("").to_owned();
                let room = panel.field.unwrap_or_else(|| ui.available_width()).min(600.0);
                let submit = n.submit.as_ref().filter(|_| n.kind == "input");
                let mut submitted = false;
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    if !n.label.is_empty() {
                        ui.label(RichText::new(&n.label).size(12.0).color(MUTED()));
                    }
                    let mut edit = if n.kind == "multiline" {
                        TextEdit::multiline(&mut s).desired_rows(3)
                    } else {
                        TextEdit::singleline(&mut s)
                    }
                    .desired_width(room)
                    .margin(Margin { left: if submit.is_some() { 28 } else { 8 }, right: 8, top: 5, bottom: 5 })
                    .char_limit(4096);
                    if !n.placeholder.is_empty() {
                        edit = edit.hint_text(RichText::new(&n.placeholder).color(FAINT()));
                    }
                    let r = ui.add_enabled(enabled, edit);
                    *panel.changed |= r.changed();
                    pages::control(panel.widgets, key.as_deref().unwrap(), &r, ui.clip_rect());
                    if submit.is_some() {
                        let glass = Rect::from_center_size(pos2(r.rect.left() + 15.0, r.rect.center().y), vec2(14.0, 14.0));
                        icon(ui.painter(), glass, Icon::Search, HINT());
                        submitted = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    }
                });
                *panel.values.get_mut(id).unwrap() = json!(s);
                if submitted {
                    *panel.action = submit.map(|event| PanelEvent::from(event.as_str()));
                }
            }
            "checkbox" => {
                *panel.changed |= !panel.values.contains_key(n.id.as_ref().unwrap());
                let value = panel
                    .values
                    .entry(n.id.clone().unwrap())
                    .or_insert_with(|| n.value.clone());
                let mut checked = value.as_bool().unwrap_or(false);
                let r = ui.add_enabled(enabled, checkbox(&mut checked, &n.label));
                *panel.changed |= r.changed();
                pages::control(panel.widgets, key.as_deref().unwrap(), &r, ui.clip_rect());
                *value = json!(checked);
            }
            "select" => {
                *panel.changed |= !panel.values.contains_key(n.id.as_ref().unwrap());
                let value = panel
                    .values
                    .entry(n.id.clone().unwrap())
                    .or_insert_with(|| n.value.clone());
                let mut selected = value.as_str().unwrap_or(&n.options[0]).to_owned();
                let (widgets, changed) = (&mut *panel.widgets, &mut *panel.changed);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    if !n.label.is_empty() {
                        ui.label(RichText::new(&n.label).size(12.0).color(MUTED()));
                    }
                    ui.add_enabled_ui(enabled, |ui| {
                        let r = ComboBox::from_id_salt(key.as_deref().unwrap())
                            .selected_text(&selected)
                            .width(220.0)
                            .show_ui(ui, |ui| {
                                for (index, option) in n.options.iter().enumerate() {
                                    let r = ui.selectable_value(&mut selected, option.clone(), option);
                                    *changed |= r.changed();
                                    pages::control(
                                        widgets,
                                        format!("{}-{index}", key.as_deref().unwrap()),
                                        &r,
                                        ui.clip_rect(),
                                    );
                                }
                            });
                        pages::control(
                            widgets,
                            key.as_deref().unwrap(),
                            &r.response,
                            ui.clip_rect(),
                        );
                    });
                });
                *panel.values.get_mut(n.id.as_ref().unwrap()).unwrap() = json!(selected);
            }
            "segments" => {
                // The script owns what shows as chosen, so a callback can move it or clear it.
                let chosen = n.value.as_str().unwrap_or("");
                let items: Vec<(&str, bool)> = n.options.iter().map(|o| (o.as_str(), o == chosen)).collect();
                let mut picked = None;
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(enabled, |ui| {
                        for (i, r) in pages::segments(ui, &items).into_iter().enumerate() {
                            pages::control(panel.widgets, format!("{}-{i}", key.as_deref().unwrap()), &r, ui.clip_rect());
                            if r.clicked() {
                                picked = Some(i);
                            }
                        }
                    });
                });
                if let (Some(i), Some(id)) = (picked, &n.id) {
                    panel.values.insert(id.clone(), json!(n.options[i]));
                    *panel.changed = true;
                    *panel.action = Some(PanelEvent::from(id.as_str()));
                }
            }
            "table" => table(ui, n, panel, enabled, true),
            "separator" => {
                ui.add_space(2.0);
                preferences::divider(ui);
                ui.add_space(2.0);
            }
            "progress" => {
                let value = n.value.as_f64().unwrap_or(0.0).clamp(0.0, 1.0) as f32;
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 5.0;
                    let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
                    ui.painter().text(line.left_center(), Align2::LEFT_CENTER, &n.label, FontId::proportional(12.0), MUTED());
                    ui.painter().text(line.right_center(), Align2::RIGHT_CENTER, format!("{:.0}%", value * 100.0), FontId::proportional(12.0), HINT());
                    let (track, response) = ui.allocate_exact_size(vec2(ui.available_width(), 6.0), Sense::hover());
                    ui.painter().rect_filled(track, 3.0, RAISED());
                    let filled = Rect::from_min_size(track.min, vec2(track.width() * value, track.height()));
                    ui.painter().rect_filled(filled, 3.0, BLUE());
                    response.widget_info(|| WidgetInfo::labeled(Role::ProgressIndicator, true, &n.label));
                });
            }
            _ => {}
        }
    });
}

/// A card: its title strip, then its children. A table runs from edge to edge between the
/// margins of what surrounds it.
fn card(ui: &mut Ui, n: &Node, panel: &mut Panel, enabled: bool, title: bool) {
    Frame::new()
        .fill(PANEL())
        .stroke(Stroke::new(1.0, CARD_STROKE()))
        .corner_radius(8)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            if title && !n.label.is_empty() {
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
                ui.painter().rect_filled(rect, CornerRadius { nw: 7, ne: 7, sw: 0, se: 0 }, CARD());
                ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, CARD_LINE()));
                let mut job = LayoutJob::simple_singleline(n.label.clone(), semibold(13.0), TEXT());
                job.wrap = TextWrapping::truncate_at_width((rect.width() - EDGE * 2.0).max(0.0));
                let galley = ui.painter().layout_job(job);
                ui.painter().galley(pos2(rect.left() + EDGE, rect.center().y - galley.size().y / 2.0), galley, TEXT());
            }
            let wide = panel.pane || ui.available_width() >= TABLE_MIN;
            let flush = |c: &Node| c.kind == "table" && (wide || c.clickable());
            for (i, group) in n.children.chunk_by(|a, b| flush(a) == flush(b)).enumerate() {
                if i > 0 {
                    let top = ui.cursor().top();
                    ui.painter().hline(ui.max_rect().x_range(), top, Stroke::new(1.0, CARD_LINE()));
                }
                if flush(&group[0]) {
                    for c in group {
                        table(ui, c, panel, enabled && !c.disabled, false);
                    }
                    continue;
                }
                Frame::new().inner_margin(Margin::symmetric(14, 12)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 10.0;
                    for c in group {
                        render_node(ui, c, panel, enabled);
                    }
                });
            }
        });
}

/// Two panes side by side, or two tabs when the window is narrow. A tab chosen by hand lasts
/// until the next result, whose `focus` says which pane matters then.
fn split(ui: &mut Ui, n: &Node, panel: &mut Panel, enabled: bool) {
    let width = ui.available_width();
    let (first, second) = (&n.children[0], &n.children[1]);
    if panel.pane || width >= SPLIT_MIN {
        let left = (width * 0.41).round().clamp(160.0, 520.0);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            for (child, room) in [(first, left), (second, width - left - 12.0)] {
                ui.allocate_ui_with_layout(vec2(room, 0.0), Layout::top_down(Align::Min), |ui| {
                    ui.set_width(room);
                    let outer = std::mem::replace(&mut panel.pane, true);
                    render_node(ui, child, panel, enabled);
                    panel.pane = outer;
                });
            }
        });
        return;
    }
    let index = panel.splits;
    panel.splits += 1;
    let memory = Id::unique(("extension-panel-tab", panel.prefix, index));
    let chosen = ui
        .data(|d| d.get_temp::<(Weak<Node>, usize)>(memory))
        .filter(|(view, _)| view.ptr_eq(&panel.view))
        .map_or(n.focus, |(_, tab)| tab);
    let title = |child: &Node, fallback: &'static str| if child.label.is_empty() { fallback.to_owned() } else { child.label.clone() };
    let (one, two) = (title(first, "First"), title(second, "Second"));
    let keys = [0, 1].map(|tab| format!("{}pane-{index}-{tab}", panel.prefix));
    let items = [
        pages::TabItem { label: &one, count: None, selected: chosen == 0, enabled: true, key: &keys[0] },
        pages::TabItem { label: &two, count: None, selected: chosen == 1, enabled: true, key: &keys[1] },
    ];
    if let Some(tab) = pages::tabs(ui, &items, panel.widgets) {
        ui.data_mut(|d| d.insert_temp(memory, (panel.view.clone(), tab)));
    }
    let child = if chosen == 0 { first } else { second };
    if child.kind == "card" {
        card(ui, child, panel, enabled && !child.disabled, false);
    } else {
        render_node(ui, child, panel, enabled);
    }
}

fn table(ui: &mut Ui, n: &Node, panel: &mut Panel, enabled: bool, framed: bool) {
    let narrow = !panel.pane && ui.available_width() < TABLE_MIN;
    if narrow && !n.clickable() {
        for row in &n.rows {
            pages::titled_card(
                ui,
                |ui| {
                    ui.label(RichText::new(&row[0].text).font(semibold(13.0)));
                },
                |ui| {
                    for (label, cell) in n.options.iter().zip(row) {
                        pages::key_value(ui, label, 110.0, |ui| {
                            ui.add(Label::new(&cell.text).wrap());
                        });
                    }
                },
            );
        }
        return;
    }
    let mut body = |ui: &mut Ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        if narrow {
            two_line_rows(ui, n, panel, enabled);
        } else {
            column_rows(ui, n, panel, enabled);
        }
    };
    if framed {
        Frame::new()
            .fill(PANEL())
            .stroke(Stroke::new(1.0, CARD_STROKE()))
            .corner_radius(8)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                body(ui);
            });
    } else {
        body(ui);
    }
}

fn pill_width(ui: &Ui, text: &str) -> f32 {
    ui.painter().layout_no_wrap(text.to_owned(), semibold(11.0), TEXT()).size().x + 16.0
}
fn paint_pill(painter: &Painter, left: f32, y: f32, text: &str, tone: &str) -> f32 {
    let color = tone_color(tone).unwrap_or(MUTED());
    let galley = painter.layout_no_wrap(text.to_owned(), semibold(11.0), color);
    let rect = Rect::from_min_size(pos2(left, y - 10.0), vec2(galley.size().x + 16.0, 20.0));
    painter.rect_filled(rect, 10.0, color.gamma_multiply(0.14));
    painter.galley(pos2(rect.left() + 8.0, y - galley.size().y / 2.0), galley, color);
    rect.width()
}
fn cell_font(cell: &Cell) -> FontId {
    if cell.mono { FontId::monospace(12.5) } else { FontId::proportional(13.0) }
}

/// Column widths that fill `width`: the widest column takes what is left, or gives way first.
fn column_widths(ui: &Ui, n: &Node, width: f32) -> Vec<f32> {
    let mut natural: Vec<f32> = n
        .options
        .iter()
        .map(|c| ui.painter().layout_no_wrap(c.to_uppercase(), semibold(11.0), TEXT()).size().x)
        .collect();
    for row in &n.rows {
        for (cell, column) in row.iter().zip(&mut natural) {
            let wide = if cell.tone.is_some() {
                pill_width(ui, &cell.text)
            } else {
                ui.painter().layout_no_wrap(cell.text.clone(), cell_font(cell), TEXT()).size().x.min(LONGEST)
            };
            *column = column.max(wide);
        }
    }
    let room = (width - EDGE * 2.0 - GAP * natural.len().saturating_sub(1) as f32).max(natural.len() as f32 * 40.0);
    let widest = natural.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map_or(0, |(i, _)| i);
    let total: f32 = natural.iter().sum();
    if total <= room {
        natural[widest] += room - total;
        return natural;
    }
    // Lower a shared ceiling until the columns fit; narrow columns keep their width.
    let mut sorted = natural.clone();
    sorted.sort_by(f32::total_cmp);
    let mut used = 0.0;
    let mut ceiling = room / natural.len() as f32;
    for (i, wide) in sorted.iter().enumerate() {
        let share = (room - used) / (sorted.len() - i) as f32;
        if *wide >= share {
            ceiling = share;
            break;
        }
        used += wide;
    }
    natural.iter().map(|w| w.min(ceiling)).collect()
}

/// The chosen row's accent, or the hover band, under a row's text.
fn row_band(ui: &Ui, rect: Rect, chosen: bool, hot: bool) {
    if chosen {
        ui.painter().rect_filled(rect, 0.0, SELECTED());
        ui.painter().rect_filled(Rect::from_min_size(rect.min, vec2(3.0, rect.height())), 0.0, BLUE());
    } else if hot {
        ui.painter().rect_filled(rect, 0.0, HOVER());
    }
}

/// What a click on a row reports; automation reaches the row by the table's id and its key.
fn row_click(n: &Node, panel: &mut Panel, i: usize, response: &Response, clip: Rect) {
    let (Some(id), Some(key)) = (&n.id, n.keys.get(i)) else {
        return;
    };
    pages::control(panel.widgets, format!("{}{id}-{key}", panel.prefix), response, clip);
    if response.clicked() {
        *panel.action = Some(PanelEvent { event: id.clone(), row: Some(key.clone()) });
    }
}

/// A cell of a plain table stays a real label, which a screen reader reads and a mouse selects.
fn plain_cell(ui: &mut Ui, row: Rect, x: f32, room: f32, cell: &Cell, galley: Option<&Arc<Galley>>) {
    let rect = Rect::from_min_size(pos2(x, row.top()), vec2(room, row.height()));
    let mut ui = ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::left_to_right(Align::Center)));
    match galley {
        Some(galley) => {
            ui.add(Label::new(galley.clone()));
        }
        None => {
            let color = cell.tone.as_deref().and_then(tone_color).unwrap_or(MUTED());
            let pill = pages::pill(&mut ui, &cell.text, color);
            pill.widget_info(|| WidgetInfo::labeled(Role::Label, true, &cell.text));
        }
    }
}

fn column_rows(ui: &mut Ui, n: &Node, panel: &mut Panel, enabled: bool) {
    let width = ui.available_width();
    let widths = column_widths(ui, n, width);
    let clickable = n.clickable();
    let (head, _) = ui.allocate_exact_size(vec2(width, 30.0), Sense::hover());
    let mut x = head.left() + EDGE;
    for (column, room) in n.options.iter().zip(&widths) {
        let mut job = LayoutJob::simple_singleline(column.to_uppercase(), semibold(11.0), HINT());
        job.wrap = TextWrapping::truncate_at_width(*room);
        let galley = ui.painter().layout_job(job);
        ui.painter().galley(pos2(x, head.center().y - galley.size().y / 2.0), galley, HINT());
        x += room + GAP;
    }
    ui.painter().hline(head.x_range(), head.bottom() - 0.5, Stroke::new(1.0, CARD_LINE()));
    for (i, row) in n.rows.iter().enumerate() {
        let chosen = clickable && n.keys.get(i) == n.selected.as_ref();
        let galleys: Vec<_> = row
            .iter()
            .zip(&widths)
            .map(|(cell, room)| {
                cell.tone.is_none().then(|| {
                    let ink = if cell.mono && !chosen { MUTED() } else { TEXT() };
                    let mut job = LayoutJob::simple(cell.text.clone(), cell_font(cell), ink, *room);
                    if clickable {
                        job.wrap = TextWrapping::truncate_at_width(*room);
                    }
                    ui.painter().layout_job(job)
                })
            })
            .collect();
        let tallest = galleys.iter().flatten().map(|g| g.size().y).fold(0.0, f32::max);
        let sense = if clickable && enabled { Sense::click() } else { Sense::hover() };
        let (rect, response) = ui.allocate_exact_size(vec2(width, (tallest + 16.0).max(ROW)), sense);
        row_band(ui, rect, chosen, clickable && enabled && (response.hovered() || response.has_focus()));
        let painter = ui.painter_at(rect);
        let mut x = rect.left() + EDGE;
        for ((cell, galley), room) in row.iter().zip(&galleys).zip(&widths) {
            match galley {
                _ if !clickable => plain_cell(ui, rect, x, *room, cell, galley.as_ref()),
                Some(galley) => painter.galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley.clone(), TEXT()),
                None => {
                    paint_pill(&painter, x, rect.center().y, &cell.text, cell.tone.as_deref().unwrap_or("neutral"));
                }
            }
            x += room + GAP;
        }
        if i + 1 < n.rows.len() {
            ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, ROW_LINE()));
        }
        if clickable {
            let text = row.iter().map(|c| c.text.as_str()).collect::<Vec<_>>().join(" · ");
            response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, &text));
            row_click(n, panel, i, &response, ui.clip_rect());
            if galleys.iter().flatten().any(|g| g.elided) {
                response.on_hover_text(text);
            }
        }
    }
}

/// A narrow window: the first cell and any pills, with the rest on a line below.
fn two_line_rows(ui: &mut Ui, n: &Node, panel: &mut Panel, enabled: bool) {
    let width = ui.available_width();
    for (i, row) in n.rows.iter().enumerate() {
        let chosen = n.keys.get(i) == n.selected.as_ref();
        let sense = if enabled { Sense::click() } else { Sense::hover() };
        let (rect, response) = ui.allocate_exact_size(vec2(width, TWO_LINES), sense);
        row_band(ui, rect, chosen, enabled && (response.hovered() || response.has_focus()));
        let painter = ui.painter_at(rect);
        let top = rect.top() + 18.0;
        let mut x = rect.left() + EDGE;
        if let Some(first) = row.first().filter(|c| c.tone.is_none()) {
            let ink = if first.mono && !chosen { MUTED() } else { TEXT() };
            let galley = painter.layout_no_wrap(first.text.clone(), cell_font(first), ink);
            x += galley.size().x + 8.0;
            painter.galley(pos2(rect.left() + EDGE, top - galley.size().y / 2.0), galley, ink);
        }
        for cell in row.iter().filter(|c| c.tone.is_some()) {
            x += paint_pill(&painter, x, top, &cell.text, cell.tone.as_deref().unwrap_or("neutral")) + 6.0;
        }
        let rest: Vec<&str> = row.iter().skip(1).filter(|c| c.tone.is_none()).map(|c| c.text.as_str()).collect();
        let mut job = LayoutJob::simple_singleline(rest.join(" · "), FontId::proportional(13.0), TEXT());
        job.wrap = TextWrapping::truncate_at_width((rect.width() - EDGE * 2.0).max(0.0));
        let galley = painter.layout_job(job);
        painter.galley(pos2(rect.left() + EDGE, rect.bottom() - 18.0 - galley.size().y / 2.0), galley, TEXT());
        if i + 1 < n.rows.len() {
            ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, ROW_LINE()));
        }
        let text = row.iter().map(|c| c.text.as_str()).collect::<Vec<_>>().join(" · ");
        response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, &text));
        row_click(n, panel, i, &response, ui.clip_rect());
    }
}
