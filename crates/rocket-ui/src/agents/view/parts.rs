//! Drawn parts the Agents page shares: a session's state mark and words, buttons with
//! an icon, cards, strips and centred states.
use super::*;
use crate::i18n::trf;

/// What a session's mark shows. Each has its own shape, so colour is not the only cue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mark {
    /// Waiting for the user: a filled amber dot in a halo.
    Need,
    /// Starting, stopping or working on a turn: a turning ring.
    Busy,
    /// Its process runs and no turn is under way: a green dot.
    Live,
    /// No process: a hollow ring.
    Idle,
    /// Ended with an error: a red cross.
    Failed,
}

/// Phases that are on their way somewhere; the provider's activity says nothing yet.
fn passing(phase: Phase) -> bool {
    matches!(phase, Phase::Saving | Phase::Starting | Phase::Setup | Phase::Stopping)
}

pub(super) fn mark_of(s: &Session) -> Mark {
    if s.phase == Phase::Failed {
        Mark::Failed
    } else if s.attention {
        Mark::Need
    } else if !s.active() {
        Mark::Idle
    } else if passing(s.phase) || s.activity_state == events::Activity::Working {
        Mark::Busy
    } else {
        Mark::Live
    }
}

/// Where a session stands, in words: what its provider reports while that is known and
/// current, otherwise what its process is doing.
pub(super) fn state_label(s: &Session) -> &'static str {
    let reported = s.activity_state != events::Activity::Unknown && (s.active() || s.attention);
    if s.phase != Phase::Failed && reported && !passing(s.phase) {
        s.activity_state.label()
    } else {
        s.phase.label()
    }
}

/// The colour of [`state_label`] where it stands alone, as in the header's pill.
pub(super) fn state_color(s: &Session) -> Color32 {
    match mark_of(s) {
        Mark::Failed => RED(),
        Mark::Need => AMBER(),
        Mark::Busy => ACCENT_TEXT(),
        Mark::Live => GREEN_TEXT(),
        Mark::Idle => MUTED(),
    }
}

pub(super) fn paint_mark(ui: &Ui, center: Pos2, mark: Mark) {
    let painter = ui.painter();
    match mark {
        Mark::Need => {
            painter.circle_filled(center, 8.0, AMBER().gamma_multiply(0.22));
            painter.circle_filled(center, 5.0, AMBER());
        }
        Mark::Busy => Spinner::new()
            .size(12.0)
            .color(BLUE())
            .paint_at(ui, Rect::from_center_size(center, vec2(12.0, 12.0))),
        Mark::Live => {
            painter.circle_filled(center, 4.0, GREEN());
        }
        Mark::Idle => {
            painter.circle_stroke(center, 4.0, Stroke::new(1.5, FAINT()));
        }
        Mark::Failed => {
            let stroke = Stroke::new(2.0, RED());
            painter.line_segment([center + vec2(-4.0, -4.0), center + vec2(4.0, 4.0)], stroke);
            painter.line_segment([center + vec2(-4.0, 4.0), center + vec2(4.0, -4.0)], stroke);
        }
    }
}

/// How long a session has run, at the precision a glance needs.
pub(super) fn short_time(d: Duration) -> String {
    let seconds = d.as_secs();
    match seconds {
        0..60 => trf!("{n} s", n = seconds),
        60..3_600 => trf!("{n} min", n = seconds / 60),
        _ => trf!("{h} h {m} min", h = seconds / 3_600, m = seconds / 60 % 60),
    }
}

/// One line of text cut to `rect`'s width and centred on its height.
pub(super) fn line(ui: &Ui, rect: Rect, text: &str, font: FontId, color: Color32) {
    let mut job = text::LayoutJob::simple_singleline(text.into(), font, color);
    job.wrap = text::TextWrapping::truncate_at_width(rect.width().max(0.0));
    let galley = ui.painter().layout_job(job);
    ui.painter()
        .galley(pos2(rect.left(), rect.center().y - galley.size().y / 2.0), galley, color);
}

/// A child laid out top down inside `rect` and clipped to it.
pub(super) fn child(ui: &mut Ui, id: &'static str, rect: Rect) -> Ui {
    let mut out = ui.new_child(
        UiBuilder::new()
            .id_salt(id)
            .max_rect(rect)
            .layout(Layout::top_down(Align::Min)),
    );
    out.set_clip_rect(rect.intersect(ui.clip_rect()));
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Look {
    /// The one main action: accent fill.
    Primary,
    Plain,
    /// Stops or removes something: a red outline.
    Danger,
    /// Framed only under the pointer.
    Quiet,
}

/// A button `CONTROL` tall with an optional icon before its label. A `None` image with
/// `busy` shows a spinner in the icon's place.
pub(super) fn action(ui: &mut Ui, look: Look, image: Option<Icon>, busy: bool, label: &str, enabled: bool) -> Response {
    let font = if look == Look::Primary { semibold(13.0) } else { FontId::proportional(13.0) };
    let ink = match look {
        Look::Primary if enabled => ACCENT_INK(),
        // Over the dimmed accent fill of a disabled main action.
        Look::Primary => ACCENT_INK().gamma_multiply(0.55),
        _ if !enabled => FAINT(),
        Look::Danger => RED(),
        Look::Quiet => MUTED(),
        Look::Plain => TEXT(),
    };
    let text = ui.painter().layout_no_wrap(label.to_owned(), font, ink);
    let lead = if image.is_some() || busy { 20.0 } else { 0.0 };
    let width = (lead + text.size().x + 22.0).ceil();
    let button = Button::new("").min_size(vec2(width, CONTROL));
    let response = match look {
        Look::Primary => ui.add_enabled(enabled, button.fill(BLUE())),
        Look::Danger => ui.add_enabled(
            enabled,
            button
                .fill(Color32::TRANSPARENT)
                .stroke(Stroke::new(1.0, RED().gamma_multiply(0.45))),
        ),
        Look::Plain => ui.add_enabled(enabled, button),
        Look::Quiet => ui.add_enabled(enabled, quiet(button)),
    };
    let rect = response.rect;
    let left = rect.center().x - (lead + text.size().x) / 2.0;
    let slot = Rect::from_center_size(pos2(left + 7.0, rect.center().y), vec2(14.0, 14.0));
    match image {
        Some(image) => icon(ui.painter(), slot, image, ink),
        None if busy => Spinner::new().size(14.0).paint_at(ui, slot),
        None => {}
    }
    ui.painter()
        .galley(pos2(left + lead, rect.center().y - text.size().y / 2.0), text, ink);
    response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, label));
    response
}

/// A card that fills the width it is given: a title row with `extra` at its right end,
/// then `body`.
pub(super) fn card(ui: &mut Ui, title: &str, extra: impl FnOnce(&mut Ui), body: impl FnOnce(&mut Ui)) {
    Frame::new()
        .fill(PANEL())
        .stroke(Stroke::new(1.0, CARD_STROKE()))
        .corner_radius(8)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
            ui.painter()
                .hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, CARD_LINE()));
            let inner = Rect::from_min_max(bar.min + vec2(14.0, 0.0), bar.max - vec2(12.0, 0.0));
            let mut row = ui.new_child(
                UiBuilder::new()
                    .max_rect(inner)
                    .layout(Layout::right_to_left(Align::Center)),
            );
            extra(&mut row);
            row.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add(Label::new(RichText::new(title).font(semibold(13.0))).truncate());
            });
            Frame::new().inner_margin(Margin::symmetric(14, 12)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 8.0;
                body(ui)
            });
        });
}

/// A tinted line across the pane: `image`, the text, and a × when `close` names its
/// control. Returns whether the × was clicked.
pub(super) fn strip(ui: &mut Ui, color: Color32, image: Icon, text: &str, close: Option<(&mut Widgets, &str)>) -> bool {
    let mut closed = false;
    Frame::new()
        .fill(color.gamma_multiply(0.08))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.35)))
        .corner_radius(8)
        .inner_margin(Margin { left: 12, right: 6, top: 6, bottom: 6 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let (mark, _) = ui.allocate_exact_size(vec2(14.0, 22.0), Sense::hover());
                icon(ui.painter(), Rect::from_center_size(mark.center(), vec2(14.0, 14.0)), image, color);
                let room = ui.available_width() - if close.is_some() { 32.0 } else { 6.0 };
                ui.allocate_ui_with_layout(vec2(room.max(40.0), 22.0), Layout::left_to_right(Align::Center), |ui| {
                    ui.set_width(room.max(40.0));
                    ui.add(Label::new(RichText::new(text).size(12.5).color(color)).wrap());
                });
                if let Some((widgets, id)) = close {
                    let r = crate::chrome::icon_button(ui, Icon::Close, true, false, 22.0);
                    pages::control(widgets, id, &r, ui.clip_rect());
                    closed = r.clicked();
                }
            });
        });
    closed
}

/// A pane with nothing to list yet: an icon tile, a title, a line under it and `below`
/// (`extra` points tall), centred in `rect`.
#[allow(clippy::too_many_arguments)]
pub(super) fn centered(
    ui: &mut Ui,
    rect: Rect,
    image: Icon,
    tint: Color32,
    title: &str,
    text: &str,
    extra: f32,
    below: impl FnOnce(&mut Ui),
) {
    let width = (rect.width() - 48.0).clamp(80.0, 420.0);
    let about = ui
        .painter()
        .layout(text.to_owned(), FontId::proportional(13.0), MUTED(), width);
    let group = 44.0 + 12.0 + 20.0 + 6.0 + about.size().y + extra;
    let column = Rect::from_center_size(rect.center(), vec2(width, group.min(rect.height())));
    let mut ui = ui.new_child(
        UiBuilder::new()
            .max_rect(column)
            .layout(Layout::top_down(Align::Center)),
    );
    ui.set_clip_rect(rect.intersect(ui.clip_rect()));
    ui.spacing_mut().item_spacing.y = 0.0;
    let (tile, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
    ui.painter().rect_filled(tile, 11.0, RAISED());
    icon(ui.painter(), tile.shrink(12.0), image, tint);
    ui.add_space(12.0);
    ui.label(RichText::new(title).font(semibold(15.0)));
    ui.add_space(6.0);
    ui.add(Label::new(RichText::new(text).size(13.0).color(MUTED())).wrap().halign(Align::Center));
    below(&mut ui);
}

/// A small copy button beside a value. Returns its response for the caller to name.
pub(super) fn copy(ui: &mut Ui, value: &str) -> Response {
    let r = crate::chrome::icon_button(ui, Icon::Copy, true, false, 20.0).on_hover_text(tr("Copy"));
    if r.clicked() {
        ui.ctx().copy_text(value.to_owned());
    }
    r
}
