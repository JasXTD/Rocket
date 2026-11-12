//! The session list: sessions grouped by what they need from you. In a wide window it
//! is the column left of the selected session; in a narrow one, a switcher above it.
//! MCP controls: `agents-search`, `agents-group-{need,running,idle}`,
//! `agents-session-{id}`, `agents-list-scroll` and, in a narrow window, `agents-switcher`.
use super::*;
use parts::{line, mark_of, paint_mark, short_time, state_label};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Group {
    Need,
    Running,
    Idle,
}

impl Group {
    /// In the order shown, which is also the order of `State::folded`.
    const ALL: [Self; 3] = [Self::Need, Self::Running, Self::Idle];

    fn of(s: &Session) -> Self {
        if s.attention {
            Self::Need
        } else if s.active() {
            Self::Running
        } else {
            Self::Idle
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Need => "Needs you",
            Self::Running => "Running",
            Self::Idle => "Not running",
        }
    }
    fn key(self) -> &'static str {
        match self {
            Self::Need => "need",
            Self::Running => "running",
            Self::Idle => "idle",
        }
    }
}

/// The list as a column: the search field over the groups.
pub(super) fn rail(ui: &mut Ui, state: &mut State, w: &mut Widgets) {
    ui.add_space(10.0);
    let width = ui.available_width();
    let r = pages::search_box(ui, &mut state.search, "agents-search", tr("Find sessions"), width);
    pages::control(w, "agents-search", &r, ui.clip_rect());
    ui.add_space(4.0);
    let out = ScrollArea::vertical()
        .id_salt("agents-list")
        .auto_shrink([false, false])
        .min_scrolled_height(0.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            groups(ui, state, w);
            ui.add_space(8.0);
        });
    pages::region(w, "agents-list-scroll", out.inner_rect, ui.is_enabled());
}

/// The list in a narrow window: one button naming the selected session, which opens the
/// groups under it.
pub(super) fn switcher(ui: &mut Ui, state: &mut State, w: &mut Widgets) {
    let Some((name, provider, mark)) = state
        .selected()
        .map(|s| (s.record.name.clone(), s.record.provider.label(), mark_of(s)))
    else {
        return;
    };
    let waiting = state.sessions.iter().filter(|s| s.attention).count();
    let total = state.sessions.len();
    ui.add_space(10.0);
    let button = ui
        .horizontal(|ui| {
            ui.add_space(12.0);
            ui.add(Button::new("").min_size(vec2((ui.available_width() - 12.0).max(120.0), 34.0)))
        })
        .inner;
    pages::control(w, "agents-switcher", &button, ui.clip_rect());
    let rect = button.rect;
    paint_mark(ui, pos2(rect.left() + 18.0, rect.center().y), mark);
    icon(
        ui.painter(),
        Rect::from_center_size(pos2(rect.right() - 16.0, rect.center().y), vec2(10.0, 10.0)),
        Icon::Chevron,
        MUTED(),
    );
    // How many sessions there are, in amber while one of them waits.
    let count = style_count(ui, pos2(rect.right() - 44.0, rect.center().y), total, waiting > 0);
    let text = Rect::from_min_max(pos2(rect.left() + 34.0, rect.top()), pos2(count.left() - 8.0, rect.bottom()));
    let title = ui.painter().layout_no_wrap(name.clone(), semibold(13.0), TEXT());
    let title_width = title.size().x.min(text.width());
    line(ui, text, &name, semibold(13.0), TEXT());
    let rest = Rect::from_min_max(pos2(text.left() + title_width + 8.0, text.top()), text.max);
    if rest.width() > 40.0 {
        line(ui, rest, tr(provider), FontId::proportional(12.0), HINT());
    }
    button.widget_info(|| WidgetInfo::labeled(WidgetType::ComboBox, true, &name));
    let room = (ui.ctx().content_rect().bottom() - rect.bottom() - 80.0).clamp(140.0, 420.0);
    Popup::menu(&button)
        // Typing in the search field must not close the list.
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .width(rect.width().min(440.0))
        .show(|ui| {
            let width = ui.available_width();
            let r = pages::search_box(ui, &mut state.search, "agents-search", tr("Find sessions"), width);
            pages::control(w, "agents-search", &r, ui.clip_rect());
            ui.add_space(2.0);
            let out = ScrollArea::vertical()
                .id_salt("agents-switch-list")
                .max_height(room)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    groups(ui, state, w)
                });
            pages::region(w, "agents-list-scroll", out.inner_rect, ui.is_enabled());
            if out.inner {
                ui.close();
            }
        });
}

/// The switcher's session count, right-aligned at `right`.
fn style_count(ui: &Ui, right: Pos2, count: usize, waiting: bool) -> Rect {
    let text = count.to_string();
    let width = (text.len() as f32 * 7.0 + 10.0).max(18.0);
    count_pill(
        ui.painter(),
        pos2(right.x - width / 2.0, right.y),
        &text,
        if waiting { AMBER() } else { MUTED() },
    )
}

/// Each group's heading and rows. Returns whether a session was chosen.
fn groups(ui: &mut Ui, state: &mut State, w: &mut Widgets) -> bool {
    ui.spacing_mut().item_spacing.y = 2.0;
    let needle = state.search.trim().to_lowercase();
    let listed = |s: &Session| {
        needle.is_empty()
            || format!("{} {} {}", s.record.name, s.record.branch, s.record.provider.label())
                .to_lowercase()
                .contains(&needle)
    };
    let mut chosen = None;
    let mut any = false;
    for (index, group) in Group::ALL.into_iter().enumerate() {
        let count = state
            .sessions
            .iter()
            .filter(|s| Group::of(s) == group && listed(s))
            .count();
        if count == 0 {
            continue;
        }
        any = true;
        // While searching, every match shows: a folded group would hide what was asked for.
        let open = !state.folded[index] || !needle.is_empty();
        let r = heading(ui, group, count, open);
        pages::control(w, format!("agents-group-{}", group.key()), &r, ui.clip_rect());
        if r.clicked() {
            state.folded[index] = !state.folded[index];
        }
        if !open {
            continue;
        }
        for s in state
            .sessions
            .iter()
            .filter(|s| Group::of(s) == group && listed(s))
        {
            let r = row(ui, s, state.selected == Some(s.record.id));
            pages::control(w, format!("agents-session-{}", s.record.id), &r, ui.clip_rect());
            if r.clicked() {
                chosen = Some(s.record.id);
            }
            r.on_hover_text(format!(
                "{}\n{}\n{}",
                s.record.name,
                s.record.branch,
                crate::views::display_path(&s.record.destination)
            ));
        }
    }
    if !any {
        pages::empty(ui, tr("No matching sessions"));
    }
    if let Some(id) = chosen {
        state.integration.clear();
        state.selected = Some(id);
        state.changes.suspend();
    }
    chosen.is_some()
}

/// A group's heading: a chevron, its name and how many sessions it holds. A click folds it.
fn heading(ui: &mut Ui, group: Group, count: usize, open: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
    if response.hovered() || response.has_focus() {
        ui.painter().rect_filled(rect, 5.0, HOVER());
    }
    let ink = if group == Group::Need { AMBER() } else { HINT() };
    icon(
        ui.painter(),
        Rect::from_center_size(pos2(rect.left() + 12.0, rect.center().y), vec2(10.0, 10.0)),
        if open { Icon::Chevron } else { Icon::ChevronRight },
        ink,
    );
    let label = tr(group.label());
    let name = ui
        .painter()
        .layout_no_wrap(label.to_uppercase(), semibold(11.0), ink);
    let after = rect.left() + 24.0 + name.size().x + 7.0;
    ui.painter()
        .galley(pos2(rect.left() + 24.0, rect.center().y - name.size().y / 2.0), name, ink);
    ui.painter().text(
        pos2(after, rect.center().y),
        Align2::LEFT_CENTER,
        count,
        semibold(11.0),
        FAINT(),
    );
    response.widget_info(|| WidgetInfo::selected(WidgetType::CollapsingHeader, true, open, label));
    response
}

/// One session: its state mark, its name with how long it has run, and under them the
/// program and where it stands.
fn row(ui: &mut Ui, s: &Session, selected: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 50.0), Sense::click());
    row_background(ui.painter(), rect, selected, response.hovered() || response.has_focus());
    if selected {
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min + vec2(0.0, 9.0), vec2(3.0, rect.height() - 18.0)),
            1.5,
            BLUE(),
        );
    }
    paint_mark(ui, pos2(rect.left() + 19.0, rect.center().y), mark_of(s));
    let (left, mut right) = (rect.left() + 36.0, rect.right() - 10.0);
    if let Some(time) = s.elapsed().filter(|_| s.active()) {
        let time = ui
            .painter()
            .layout_no_wrap(short_time(time), FontId::proportional(11.5), HINT());
        right -= time.size().x;
        ui.painter()
            .galley(pos2(right, rect.top() + 16.0 - time.size().y / 2.0), time, HINT());
        right -= 8.0;
    }
    line(
        ui,
        Rect::from_min_max(pos2(left, rect.top() + 7.0), pos2(right, rect.top() + 25.0)),
        &s.record.name,
        semibold(13.0),
        TEXT(),
    );
    line(
        ui,
        Rect::from_min_max(pos2(left, rect.top() + 26.0), pos2(rect.right() - 10.0, rect.top() + 44.0)),
        &format!("{} · {}", tr(s.record.provider.label()), tr(state_label(s))),
        FontId::proportional(12.0),
        if s.phase == Phase::Failed {
            RED()
        } else if s.attention {
            AMBER()
        } else {
            HINT()
        },
    );
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, selected, &s.record.name));
    response
}
