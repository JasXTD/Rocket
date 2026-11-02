use super::*;
use crate::{
    app::UiControl,
    pages,
    preferences::card as preferences_card,
    style::*,
};
use std::collections::HashMap;

fn card<R>(ui: &mut Ui, caption: &str, content: impl FnOnce(&mut Ui) -> R) -> R {
    preferences_card(ui, caption, |ui| {
        Frame::new()
            .inner_margin(12)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 7.0;
                content(ui)
            })
            .inner
    })
}

/// What one shortcut cell shows.
enum Cell {
    Bound(Binding),
    /// The first free slot: clicking it adds a shortcut.
    Add,
    /// The second slot while the first is free.
    Empty,
    Recording,
    /// A captured input that needs a decision.
    Problem(Binding),
}

/// The choices a row's problem line offers.
enum Problem {
    /// The input belongs to another command.
    Clash(Action),
    Invalid(String),
}

/// What the table asked for this frame; applied after it is drawn.
#[derive(Default)]
struct Asked {
    record: Option<(Action, usize)>,
    reset: Option<Action>,
    take_over: bool,
    again: bool,
    keep: bool,
}

const ROW: f32 = 36.0;

impl State {
    /// A finished capture saves at once. Delete or Backspace alone clears the cell; a
    /// clash or an input the model refuses waits on its row for a decision.
    fn settle(&mut self) {
        let k = self;
        let Some(edit) = &k.edit else { return };
        let (Some(candidate), false) = (edit.candidate, edit.recording) else { return };
        let (action, index) = (edit.action, edit.index);
        let label = action.command().label;
        let mut values = k.bindings.get(action);
        if candidate.modifiers == 0 && matches!(candidate.trigger, Trigger::Key(Key::Delete | Key::Backspace)) {
            if index < values.len() {
                values.remove(index);
                k.error = k.bindings.set(action, values).err();
                k.notice = k.error.is_none().then(|| format!("{label} shortcut removed."));
            }
            k.edit = None;
            return;
        }
        if edit.error.is_some() {
            return;
        }
        if index < values.len() {
            values[index] = candidate;
        } else {
            values.push(candidate);
        }
        match k.bindings.set(action, values) {
            Ok(()) => {
                k.edit = None;
                k.notice = Some(format!("{label}: {} saved.", candidate.text()));
            }
            Err(error) => {
                if let Some(edit) = &mut k.edit {
                    edit.error = Some(error);
                }
            }
        }
    }

    /// Use for this command: the clashing input leaves the other command first.
    fn take_over(&mut self) {
        let k = self;
        let Some(edit) = &k.edit else { return };
        let Some(candidate) = edit.candidate else { return };
        let (action, index) = (edit.action, edit.index);
        let Some(other) = k.bindings.find(candidate).filter(|a| *a != action) else { return };
        let mut next = k.bindings.clone();
        let mut theirs = next.get(other);
        theirs.retain(|b| *b != candidate);
        let result = next.set(other, theirs).and_then(|()| {
            let mut mine = next.get(action);
            if index < mine.len() {
                mine[index] = candidate;
            } else {
                mine.push(candidate);
            }
            next.set(action, mine)
        });
        match result {
            Ok(()) => {
                k.bindings = next;
                k.edit = None;
                k.notice = Some(format!(
                    "{} now runs {} instead of {}.",
                    candidate.text(),
                    action.command().label,
                    other.command().label
                ));
            }
            Err(error) => {
                if let Some(edit) = &mut k.edit {
                    edit.error = Some(error);
                }
            }
        }
    }

}

impl Rocket {
    fn shortcut_button(&mut self, ui: &mut Ui, id: &str, label: &str, enabled: bool) -> bool {
        let response = ui.add_enabled(enabled, Button::new(label));
        pages::control(&mut self.frame_widgets, id, &response, ui.clip_rect());
        response.clicked()
    }

    pub(crate) fn shortcut_preferences(&mut self, ui: &mut Ui) {
        self.keybindings.settle();
        let custom = self.keybindings.bindings.0.len();
        let capturing = self.keybindings.capturing();
        let finding = self.keybindings.finding.is_some();
        // An input found by pressing keys is the search text until that text is edited.
        if let Some(found) = self.keybindings.found.map(Binding::text)
            && self.preferences.shortcut_filter != found
        {
            self.preferences.shortcut_filter = found;
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let width = (ui.available_width() - 260.0).clamp(160.0, 340.0);
            let (field, find) = find_field(ui, &mut self.preferences.shortcut_filter, width, finding);
            if let Some(r) = field {
                pages::control(&mut self.frame_widgets, "pref-shortcuts-filter", &r, ui.clip_rect());
                if r.changed() {
                    self.keybindings.found = None;
                }
            }
            pages::control(&mut self.frame_widgets, "pref-shortcuts-find", &find, ui.clip_rect());
            if find.clicked() {
                if finding {
                    self.keybindings.finding = None;
                } else {
                    self.keybindings.find();
                    if let Some(id) = ui.memory(|m| m.focused()) {
                        ui.memory_mut(|m| m.surrender_focus(id));
                    }
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let r = ui.add_enabled(!capturing && custom > 0, Button::new("Restore defaults"));
                pages::control(&mut self.frame_widgets, "pref-shortcut-reset-all", &r, ui.clip_rect());
                if r.clicked() {
                    self.keybindings.reset_all = true;
                    self.keybindings.edit = None;
                }
                if custom > 0 {
                    pages::pill(ui, &format!("{custom} changed"), BLUE());
                }
            });
        });
        ui.add_space(8.0);
        if self.keybindings.reset_all {
            card(ui, "Restore default shortcuts?", |ui| {
                ui.label("This removes your custom keyboard and mouse bindings.");
                ui.horizontal_wrapped(|ui| {
                    if self.shortcut_button(ui, "pref-shortcut-reset-confirm", "Restore all", true) {
                        self.keybindings.bindings = Bindings::default();
                        self.keybindings.reset_all = false;
                        self.keybindings.error = None;
                        self.keybindings.notice = Some("Default shortcuts restored.".into());
                    }
                    if self.shortcut_button(ui, "pref-shortcut-reset-cancel", "Keep my shortcuts", true) {
                        self.keybindings.reset_all = false;
                    }
                });
            });
        }
        if let Some(edit) = self.keybindings.edit.as_ref().filter(|e| e.recording) {
            pages::callout(
                ui,
                BLUE(),
                &format!(
                    "Recording for {} · Esc cancels · Delete clears · mouse side buttons work on their own",
                    edit.action.command().label
                ),
            );
            ui.add_space(6.0);
        }
        if let Some(error) = &self.keybindings.error {
            ui.colored_label(AMBER(), error);
        }
        if let Some(notice) = &self.keybindings.notice {
            ui.label(RichText::new(notice).size(12.0).color(HINT()));
        }
        let mut asked = Asked::default();
        self.shortcut_table(ui, &mut asked);
        if let Some((action, index)) = asked.record {
            self.keybindings.record(action, index);
            if let Some(id) = ui.memory(|m| m.focused()) {
                ui.memory_mut(|m| m.surrender_focus(id));
            }
        }
        if let Some(action) = asked.reset {
            self.keybindings.error = self.keybindings.bindings.reset(action).err();
            self.keybindings.notice = self
                .keybindings
                .error
                .is_none()
                .then(|| format!("{} reset.", action.command().label));
        }
        if asked.take_over {
            self.keybindings.take_over();
        }
        if asked.again
            && let Some(edit) = &self.keybindings.edit
        {
            let (action, index) = (edit.action, edit.index);
            self.keybindings.record(action, index);
        }
        if asked.keep {
            self.keybindings.edit = None;
        }
    }

    /// Command, Shortcut and Alternate for each command, grouped; a cell is clicked to
    /// record into it, and a captured input that needs a decision gets a line below.
    fn shortcut_table(&mut self, ui: &mut Ui, asked: &mut Asked) {
        let query = self.preferences.shortcut_filter.trim().to_lowercase();
        let k = &self.keybindings;
        let (found, finding) = (k.found, k.finding.is_some());
        let widgets = &mut self.frame_widgets;
        let editing = k.edit.as_ref().map(|e| (e.action, e.index, e.recording, e.candidate, e.error.clone()));
        let open = !k.reset_all;
        let scroll = std::mem::take(&mut self.keybindings.scroll_editor);
        let k = &self.keybindings;
        let width = ui.available_width();
        let cell = ((width - 60.0) * 0.28).clamp(120.0, 220.0);
        let mut shown = false;
        preferences_card(ui, "", |ui| {
            row_frame(ui, |ui| {
                let label = (ui.available_width() - 2.0 * (cell + 8.0) - 36.0).max(80.0);
                // The value columns line up with the cells' content: 8 spacing + 8 padding.
                for (text, w, inset) in [("Command", label, 0.0), ("Shortcut", cell + 8.0, 16.0), ("Alternate", cell + 8.0, 16.0)] {
                    let (rect, _) = ui.allocate_exact_size(vec2(w, 28.0), Sense::hover());
                    ui.painter().text(rect.left_center() + vec2(inset, 0.0), Align2::LEFT_CENTER, text.to_uppercase(), semibold(11.0), HINT());
                }
            });
            for group in ["Repositories", "Panels", "Working"] {
                let commands: Vec<_> = COMMANDS
                    .iter()
                    .filter(|c| c.group == group && listed(c, &k.bindings, &query, found))
                    .collect();
                if commands.is_empty() {
                    continue;
                }
                shown = true;
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
                ui.painter().rect_filled(rect, 0.0, PANEL());
                ui.painter().text(
                    pos2(rect.left() + 14.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    group.to_uppercase(),
                    semibold(11.0),
                    HINT(),
                );
                for c in commands {
                    let bindings = k.bindings.get(c.action);
                    let changed = k.bindings.0.contains_key(c.id);
                    let mine = editing.as_ref().filter(|e| e.0 == c.action);
                    row_frame(ui, |ui| {
                        let label = (ui.available_width() - 2.0 * (cell + 8.0) - 36.0).max(80.0);
                        let (rect, _) = ui.allocate_exact_size(vec2(label, ROW), Sense::hover());
                        let mut x = rect.left();
                        if changed {
                            ui.painter().circle_filled(pos2(x + 3.0, rect.center().y), 3.0, BLUE());
                            x += 12.0;
                        }
                        let mut job = text::LayoutJob::simple_singleline(c.label.to_owned(), FontId::proportional(13.0), TEXT());
                        job.wrap = text::TextWrapping::truncate_at_width(rect.right() - x - 8.0);
                        let galley = ui.painter().layout_job(job);
                        ui.painter().galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, TEXT());
                        for index in 0..2 {
                            let state = match mine {
                                Some(e) if e.1 == index && e.2 => Cell::Recording,
                                Some(e) if e.1 == index && e.4.is_some() && e.3.is_some() => Cell::Problem(e.3.unwrap()),
                                _ => match bindings.get(index) {
                                    Some(b) => Cell::Bound(*b),
                                    None if index == bindings.len() => Cell::Add,
                                    None => Cell::Empty,
                                },
                            };
                            ui.add_space(8.0);
                            let busy = finding || editing.as_ref().is_some_and(|e| e.2 || e.4.is_some());
                            let r = shortcut_cell(ui, vec2(cell, 30.0), &state, open && !busy, c.label);
                            let key = match state {
                                Cell::Add => Some(format!("pref-shortcut-{}-add", c.id)),
                                Cell::Empty => None,
                                _ => Some(format!("pref-shortcut-{}-edit-{index}", c.id)),
                            };
                            if let Some(key) = key {
                                pages::control(widgets, key, &r, ui.clip_rect());
                            }
                            if matches!(state, Cell::Recording) && scroll {
                                r.scroll_to_me(Some(Align::Center));
                            }
                            if r.clicked() && !matches!(state, Cell::Empty) {
                                asked.record = Some((c.action, index));
                            }
                        }
                        ui.add_space(4.0);
                        if changed {
                            let default = c.default;
                            let hint = if default.is_empty() { "Reset: no shortcut".to_owned() } else { format!("Reset to {default}") };
                            let r = crate::chrome::icon_button(ui, Icon::Refresh, open && editing.is_none(), false, 28.0).on_hover_text(hint);
                            pages::control(widgets, format!("pref-shortcut-{}-reset", c.id), &r, ui.clip_rect());
                            if r.clicked() {
                                asked.reset = Some(c.action);
                            }
                        }
                    });
                    // A captured input that needs a decision, under its row.
                    if let Some((_, _, false, Some(candidate), Some(error))) = mine {
                        let problem = match k.bindings.find(*candidate).filter(|a| *a != c.action) {
                            Some(other) => Problem::Clash(other),
                            None => Problem::Invalid(error.clone()),
                        };
                        problem_line(ui, widgets, asked, &problem, *candidate, c.label, bindings.get(mine.map_or(0, |e| e.1)).copied());
                    }
                    divider(ui);
                }
            }
            if !shown {
                let text = found.map_or_else(|| "No commands match this search.".to_owned(), |b| format!("No command uses {}.", b.text()));
                Frame::new().inner_margin(14).show(ui, |ui| {
                    ui.label(RichText::new(text).color(HINT()));
                });
            }
        });
    }
}

/// Whether the table lists a command: it uses the input found by pressing keys, exactly,
/// or its name or shortcuts contain the typed text.
fn listed(c: &model::Command, bindings: &Bindings, query: &str, found: Option<Binding>) -> bool {
    match found {
        Some(input) => bindings.get(c.action).contains(&input),
        None => format!("{} {}", c.label, bindings.label(c.action)).to_lowercase().contains(query),
    }
}

/// The search box, with a keyboard button at its end that waits for keys or a gesture
/// to find the command using them. While it waits, the box says so instead of the text.
fn find_field(ui: &mut Ui, value: &mut String, width: f32, finding: bool) -> (Option<Response>, Response) {
    Frame::new()
        .fill(RAISED())
        .stroke(Stroke::new(1.0, if finding { BLUE() } else { BORDER() }))
        .corner_radius(6)
        .inner_margin(Margin { left: 10, right: 4, top: 7, bottom: 7 })
        .show(ui, |ui| {
            ui.set_width((width - 16.0).max(30.0));
            ui.spacing_mut().interact_size.y = 22.0;
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                icon(ui.painter(), rect, Icon::Search, MUTED());
                // The keyboard button is 22 px wide, after the usual spacing.
                let text = (ui.available_width() - 22.0 - ui.spacing().item_spacing.x).max(20.0);
                let field = if finding {
                    let (rect, _) = ui.allocate_exact_size(vec2(text, 22.0), Sense::hover());
                    clipped_text(ui, rect, "Press keys or a gesture · Esc cancels", 12.5, ACCENT_TEXT());
                    None
                } else {
                    Some(ui.add(
                        TextEdit::singleline(value)
                            .frame(Frame::NONE)
                            .min_size(vec2(0.0, 22.0))
                            .hint_text("Find a command or key")
                            .desired_width(text),
                    ))
                };
                let find = crate::chrome::icon_button(ui, Icon::Keyboard, true, finding, 22.0);
                find.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, "Find by pressing keys"));
                (field, find.on_hover_text("Press keys to find the command that uses them"))
            })
            .inner
        })
        .inner
}

/// A table row's padding; its content is laid out left to right.
fn row_frame(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    Frame::new().inner_margin(Margin::symmetric(14, 0)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            content(ui)
        });
    });
}

fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.center().y, Stroke::new(1.0, CARD_LINE()));
}

/// One shortcut cell: its keys or gesture, the recording state, or a free slot.
fn shortcut_cell(ui: &mut Ui, size: Vec2, cell: &Cell, enabled: bool, command: &str) -> Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let hot = enabled && (response.hovered() || response.has_focus());
    let painter = ui.painter();
    let (fill, line) = match cell {
        Cell::Recording => (SELECTED(), BLUE()),
        Cell::Problem(_) => (AMBER().gamma_multiply(0.10), AMBER()),
        _ if hot => (HOVER(), BORDER()),
        _ => (Color32::TRANSPARENT, Color32::TRANSPARENT),
    };
    painter.rect(rect, 6.0, fill, Stroke::new(1.0, line), StrokeKind::Inside);
    let inner = rect.shrink2(vec2(8.0, 0.0));
    let clip = painter.with_clip_rect(inner);
    let described = match cell {
        Cell::Bound(b) | Cell::Problem(b) => {
            paint_binding(&clip, inner, *b, if matches!(cell, Cell::Problem(_)) { AMBER() } else { BORDER() });
            b.text()
        }
        Cell::Recording => {
            clip.text(inner.left_center(), Align2::LEFT_CENTER, "Press keys or a gesture", FontId::proportional(12.5), ACCENT_TEXT());
            "Recording".to_owned()
        }
        Cell::Add => {
            let (text, color) = if hot { ("+ Add shortcut", MUTED()) } else { ("—", FAINT()) };
            clip.text(inner.left_center(), Align2::LEFT_CENTER, text, FontId::proportional(12.5), color);
            "No shortcut".to_owned()
        }
        Cell::Empty => {
            clip.text(inner.left_center(), Align2::LEFT_CENTER, "—", FontId::proportional(12.5), FAINT());
            "No shortcut".to_owned()
        }
    };
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, format!("{command} shortcut: {described}")));
    if enabled && !matches!(cell, Cell::Empty) {
        response.on_hover_text("Click, then press the keys or mouse gesture")
    } else {
        response
    }
}

/// Keys as keycaps; a mouse button or the wheel as a violet chip after any modifiers.
fn paint_binding(painter: &Painter, rect: Rect, binding: Binding, edge: Color32) {
    let text = binding.text();
    let mut pieces: Vec<&str> = text.split('+').collect();
    let gesture = match binding.trigger {
        Trigger::Key(_) => None,
        Trigger::Mouse(button) => Some(match button {
            PointerButton::Primary => "Left click",
            PointerButton::Secondary => "Right click",
            PointerButton::Middle => "Middle click",
            PointerButton::Extra1 => "Mouse back",
            PointerButton::Extra2 => "Mouse forward",
        }),
        Trigger::Wheel(wheel) => Some(match wheel {
            Wheel::Up => "Wheel up",
            Wheel::Down => "Wheel down",
            Wheel::Left => "Wheel left",
            Wheel::Right => "Wheel right",
        }),
    };
    if gesture.is_some() {
        pieces.pop();
    }
    let font = FontId::proportional(11.5);
    let mut x = rect.left();
    let y = rect.center().y;
    for piece in pieces {
        let galley = painter.layout_no_wrap(piece.to_owned(), font.clone(), TEXT());
        let cap = Rect::from_min_size(pos2(x, y - 10.0), vec2(galley.size().x.max(8.0) + 12.0, 20.0));
        painter.rect(cap, 4.0, RAISED(), Stroke::new(1.0, edge), StrokeKind::Inside);
        painter.galley(pos2(cap.center().x - galley.size().x / 2.0, y - galley.size().y / 2.0), galley, TEXT());
        x = cap.right() + 3.0;
    }
    if let Some(gesture) = gesture {
        let ink = VIOLET_TEXT();
        let galley = painter.layout_no_wrap(gesture.to_owned(), font, ink);
        let chip = Rect::from_min_size(pos2(x, y - 10.0), vec2(galley.size().x + 24.0, 20.0));
        painter.rect(chip, 4.0, VIOLET().gamma_multiply(0.12), Stroke::new(1.0, VIOLET().gamma_multiply(0.5)), StrokeKind::Inside);
        // A small mouse: its body and wheel.
        let mouse = Rect::from_center_size(pos2(chip.left() + 10.0, y), vec2(7.0, 11.0));
        painter.rect_stroke(mouse, 3.5, Stroke::new(1.2, ink), StrokeKind::Inside);
        painter.line_segment([pos2(mouse.center().x, mouse.top() + 2.0), pos2(mouse.center().x, mouse.top() + 4.5)], Stroke::new(1.2, ink));
        painter.galley(pos2(chip.left() + 18.0, y - galley.size().y / 2.0), galley, ink);
    }
}

/// Under a row: whose the input is, or why it cannot be used, and the choices.
fn problem_line(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    asked: &mut Asked,
    problem: &Problem,
    candidate: Binding,
    command: &str,
    current: Option<Binding>,
) {
    Frame::new()
        .fill(AMBER().gamma_multiply(0.06))
        .inner_margin(Margin { left: 26, right: 14, top: 4, bottom: 8 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let mut button = |ui: &mut Ui, key: &str, label: &str| {
                        let r = ui.button(label);
                        pages::control(widgets, key, &r, ui.clip_rect());
                        r.clicked()
                    };
                    let text = match problem {
                        Problem::Clash(other) => {
                            let keep = current.map_or_else(|| "Keep it free".to_owned(), |b| format!("Keep {}", b.text()));
                            asked.keep |= button(ui, "pref-shortcut-cancel", &keep);
                            asked.take_over |= button(ui, "pref-shortcut-move", &format!("Use for {command}"));
                            format!("{} already runs {}.", candidate.text(), other.command().label)
                        }
                        Problem::Invalid(error) => {
                            asked.keep |= button(ui, "pref-shortcut-cancel", "Cancel");
                            asked.again |= button(ui, "pref-shortcut-again", "Record again");
                            error.clone()
                        }
                    };
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.add(Label::new(RichText::new(text).size(12.5).color(AMBER_TEXT())).wrap());
                    });
                });
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn capture(state: &mut State, action: Action, index: usize, chord: &str) {
        state.record(action, index);
        state.capture(Binding::parse(chord).unwrap());
        state.settle();
    }
    fn inputs(state: &State, action: Action) -> Vec<String> {
        state.bindings.get(action).iter().map(|b| b.text()).collect()
    }

    #[test]
    fn a_captured_shortcut_saves_at_once_delete_clears_and_a_clash_waits() {
        let mut state = State::default();
        capture(&mut state, Action::Sidebar, 0, "Ctrl+F8");
        assert!(state.edit.is_none(), "saved without a Save step");
        assert_eq!(inputs(&state, Action::Sidebar), ["Ctrl+F8"]);
        capture(&mut state, Action::Sidebar, 0, "Delete");
        assert!(state.edit.is_none());
        assert!(inputs(&state, Action::Sidebar).is_empty(), "Delete clears the cell");
        // Another command's input waits for a choice; Use for this moves it.
        capture(&mut state, Action::Sidebar, 0, "Ctrl+O");
        assert!(state.edit.as_ref().unwrap().error.as_ref().unwrap().contains("Open repository"));
        assert!(inputs(&state, Action::Sidebar).is_empty());
        state.take_over();
        assert!(state.edit.is_none());
        assert_eq!(inputs(&state, Action::Sidebar), ["Ctrl+O"]);
        assert!(!inputs(&state, Action::Open).contains(&"Ctrl+O".to_owned()));
        // An input the model refuses waits too and changes nothing.
        let before = state.bindings.clone();
        capture(&mut state, Action::Sidebar, 1, "MouseLeft");
        assert!(state.edit.as_ref().unwrap().error.is_some());
        assert_eq!(state.bindings, before);
    }

    #[test]
    fn finding_by_keys_lists_only_the_command_using_them_and_saves_nothing() {
        let key = |chord: &str, pressed: bool| {
            let b = Binding::parse(chord).unwrap();
            let Trigger::Key(key) = b.trigger else { panic!("key") };
            Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: b.egui_modifiers() }
        };
        let click = |pressed| Event::PointerButton { pos: pos2(40.0, 40.0), button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        let raw = |events| RawInput { focused: true, events, ..Default::default() };
        let rows = |state: &State, query: &str| -> Vec<&str> {
            COMMANDS.iter().filter(|c| listed(c, &state.bindings, query, state.found)).map(|c| c.id).collect()
        };
        let mut state = State::default();
        let original = state.bindings.clone();
        state.find();
        let mut input = raw(vec![key("Ctrl+K", true), Event::Text("k".into()), key("Ctrl+K", false)]);
        state.filter(&mut input, &[Action::Palette], true);
        assert!(input.events.is_empty(), "the keys neither type nor run anything");
        assert!(state.finding.is_none() && state.edit.is_none() && state.pending.is_none());
        assert_eq!(state.found.map(Binding::text).as_deref(), Some("Ctrl+K"));
        assert_eq!(rows(&state, "ctrl+k"), ["palette"]);
        assert_eq!(state.bindings, original, "finding binds nothing");
        // The match is exact: Ctrl+F1 does not find a command bound to Ctrl+F12, as text would.
        state.bindings.set(Action::Sidebar, vec![Binding::parse("Ctrl+F12").unwrap()]).unwrap();
        state.found = Some(Binding::parse("Ctrl+F1").unwrap());
        assert!(rows(&state, "ctrl+f1").is_empty());
        state.found = None;
        assert_eq!(rows(&state, "ctrl+f1"), ["sidebar"]);
        // Esc and a plain click leave finding with no result.
        for leave in [vec![key("Escape", true), key("Escape", false)], vec![click(true), click(false)]] {
            state.find();
            let mut input = raw(leave);
            state.filter(&mut input, &[], true);
            assert!(input.events.is_empty() && state.finding.is_none() && state.found.is_none());
        }
    }
}
