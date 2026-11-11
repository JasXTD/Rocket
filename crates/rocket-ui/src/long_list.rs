//! Long lists build only the rows near the visible area. The other rows keep their room
//! in the layout and their place in egui's automatic widget ids, so the scroll bar,
//! keyboard focus, open menus and drags behave as if every row had been built.
use eframe::egui::*;

/// Shows `count` rows of `height` points, one under another with the current item
/// spacing between them, and calls `row` only for the rows that can be seen or used:
///
/// - the rows inside the clip rectangle;
/// - the first and the last row, where keyboard focus enters the list;
/// - the rows that asked to stay, and the row before and after each, where keyboard
///   focus moves next.
///
/// `row` returns whether its row must stay built after it scrolls out of view: it holds
/// keyboard focus, is being dragged or has a menu open. Each row is built in a scope of
/// its own, so it may add any number of widgets, and must be exactly `height` high.
/// `id` names the list; two lists shown in the same frame need different ones.
pub(crate) fn rows(ui: &mut Ui, id: Id, count: usize, height: f32, mut row: impl FnMut(&mut Ui, usize) -> bool) {
    if count == 0 {
        return;
    }
    let pitch = height + ui.spacing().item_spacing.y;
    let top = ui.cursor().top();
    let clip = ui.clip_rect();
    // Float-to-integer casts saturate, so an unbounded clip rectangle shows every row.
    let first = (((clip.top() - top) / pitch).floor().max(0.0) as usize).min(count);
    let last = (((clip.bottom() - top) / pitch).ceil().max(0.0) as usize).min(count);
    let mut build: Vec<usize> = (first..last).collect();
    build.extend([0, count - 1]);
    let stayed: Vec<usize> = ui.data_mut(|data| data.remove_temp(id)).unwrap_or_default();
    for index in stayed {
        build.extend([index.saturating_sub(1), index, index.saturating_add(1)]);
    }
    build.retain(|index| *index < count);
    build.sort_unstable();
    build.dedup();
    let mut stay = Vec::new();
    let mut next = 0;
    for index in build {
        skip(ui, index - next, pitch);
        let built = ui.push_id(index, |ui| row(ui, index));
        debug_assert!(
            (built.response.rect.height() - height).abs() < 0.5,
            "row {index} is {} points high, not {height}",
            built.response.rect.height()
        );
        if built.inner {
            stay.push(index);
        }
        next = index + 1;
    }
    skip(ui, count - next, pitch);
    if !stay.is_empty() {
        ui.data_mut(|data| data.insert_temp(id, stay));
    }
}

/// Leaves the room of `rows` rows that are not built. A scope takes one automatic
/// widget id from its parent, as one allocation does, so the skipped rows take as many
/// ids as they would have built: widgets after them keep their identity while the list
/// scrolls.
fn skip(ui: &mut Ui, rows: usize, pitch: f32) {
    if rows == 0 {
        return;
    }
    let spacing = ui.spacing().item_spacing.y;
    ui.allocate_space(vec2(ui.available_width(), rows as f32 * pitch - spacing));
    ui.skip_ahead_auto_ids(rows - 1);
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROW: f32 = 24.0;

    /// What one frame of a list of buttons, followed by one more button, showed.
    #[derive(Default)]
    struct Shown {
        /// Each built row with its button's id and rectangle.
        rows: Vec<(usize, Id, Rect)>,
        after: Option<(Id, Rect)>,
        /// The rows whose button was clicked this frame.
        clicked: Vec<usize>,
        content_height: f32,
    }

    fn frame(ctx: &Context, count: usize, window: f32, virtual_rows: bool, events: Vec<Event>) -> Shown {
        let mut shown = Shown::default();
        let mut output = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(300.0, window))),
                events,
                ..Default::default()
            },
            |ui| {
                let output = ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let mut one = |ui: &mut Ui, index: usize| {
                        let r = ui.add(Button::new(format!("row {index}")).min_size(vec2(ui.available_width(), ROW)));
                        shown.rows.push((index, r.id, r.rect));
                        if r.clicked() {
                            shown.clicked.push(index);
                        }
                        r.has_focus()
                    };
                    if virtual_rows {
                        rows(ui, Id::new("long-list-test"), count, ROW, one);
                    } else {
                        for index in 0..count {
                            ui.push_id(index, |ui| one(ui, index));
                        }
                    }
                    let r = ui.button("after");
                    shown.after = Some((r.id, r.rect));
                });
                shown.content_height = output.content_size.y;
            },
        );
        output.textures_delta.clear();
        shown
    }

    fn wheel(delta: f32) -> Vec<Event> {
        vec![
            Event::PointerMoved(pos2(100.0, 100.0)),
            Event::MouseWheel {
                unit: MouseWheelUnit::Point,
                delta: vec2(0.0, delta),
                modifiers: Modifiers::NONE,
                phase: TouchPhase::Move,
            },
        ]
    }

    fn key(key: Key, modifiers: Modifiers) -> Vec<Event> {
        [true, false]
            .into_iter()
            .map(|pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers })
            .collect()
    }

    /// Whether a row past `index` is wholly inside a 300-point window.
    fn in_view_past(shown: &Shown, index: usize) -> bool {
        shown.rows.iter().any(|(row, _, rect)| *row > index && rect.top() >= 0.0 && rect.bottom() <= 300.0)
    }

    /// Scrolls a 300-point window down until a row past `index` is in view, then lets
    /// the scrolling come to rest.
    fn scroll_past(ctx: &Context, count: usize, index: usize) -> Shown {
        for _ in 0..400 {
            if in_view_past(&frame(ctx, count, 300.0, true, wheel(-2_000.0)), index) {
                break;
            }
        }
        for _ in 0..40 {
            frame(ctx, count, 300.0, true, vec![]);
        }
        let shown = frame(ctx, count, 300.0, true, vec![]);
        assert!(in_view_past(&shown, index), "the list did not scroll past row {index}");
        shown
    }

    fn same_place(a: Rect, b: Rect) -> bool {
        (a.min - b.min).abs().max_elem() < 0.1 && (a.max - b.max).abs().max_elem() < 0.1
    }

    #[test]
    fn rows_keep_the_place_size_and_identity_of_a_fully_built_list() {
        let count = 400;
        // A window tall enough for every row, where every row is built either way.
        let whole = frame(&Context::default(), count, 12_000.0, false, vec![]);
        assert_eq!(whole.rows.len(), count);
        let tall = frame(&Context::default(), count, 12_000.0, true, vec![]);
        assert_eq!(tall.rows.len(), count);
        let ctx = Context::default();
        let top = frame(&ctx, count, 300.0, true, vec![]);
        assert!(top.rows.len() < 20, "{} rows built for a 300-point window", top.rows.len());
        assert!((top.content_height - whole.content_height).abs() < 0.1);
        let scrolled = scroll_past(&ctx, count, 100);
        assert!(scrolled.rows.len() < 20, "{} rows built after scrolling", scrolled.rows.len());
        for shown in [&tall, &top, &scrolled] {
            let offset = shown.rows[0].2.top() - whole.rows[0].2.top();
            for (index, id, rect) in &shown.rows {
                let (_, whole_id, whole_rect) = whole.rows[*index];
                assert_eq!(*id, whole_id, "row {index} changed identity");
                assert!(same_place(rect.translate(vec2(0.0, -offset)), whole_rect), "row {index} moved to {rect:?}");
            }
            let (id, rect) = shown.after.unwrap();
            let (whole_id, whole_rect) = whole.after.unwrap();
            assert_eq!(id, whole_id, "the widget after the list changed identity");
            assert!(same_place(rect.translate(vec2(0.0, -offset)), whole_rect), "the widget after the list moved");
        }
    }

    #[test]
    fn last_row_is_reached_by_scrolling_and_takes_a_click() {
        let ctx = Context::default();
        let count = 5_000;
        let shown = scroll_past(&ctx, count, count - 2);
        let last = shown.rows.iter().find(|(index, ..)| *index == count - 1).unwrap().2;
        let pos = last.center();
        let press = |pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        frame(&ctx, count, 300.0, true, vec![Event::PointerMoved(pos), press(true)]);
        let released = frame(&ctx, count, 300.0, true, vec![press(false)]);
        assert_eq!(released.clicked, [count - 1]);
    }

    #[test]
    fn keyboard_focus_walks_every_row_and_stays_with_its_row_out_of_view() {
        let ctx = Context::default();
        let count = 500;
        frame(&ctx, count, 300.0, true, vec![]);
        let focused = |shown: &Shown| {
            let id = ctx.memory(|memory| memory.focused());
            shown.rows.iter().find(|(_, row, _)| Some(*row) == id).map(|(index, ..)| *index)
        };
        // Focus enters at the first row and moves one row a press, past the rows in view.
        for expected in 0..40 {
            frame(&ctx, count, 300.0, true, key(Key::Tab, Modifiers::NONE));
            assert_eq!(focused(&frame(&ctx, count, 300.0, true, vec![])), Some(expected));
        }
        // The focused row stays built, and focused, while the list scrolls away from it.
        assert_eq!(focused(&scroll_past(&ctx, count, 200)), Some(39));
        frame(&ctx, count, 300.0, true, key(Key::Tab, Modifiers::SHIFT));
        assert_eq!(focused(&frame(&ctx, count, 300.0, true, vec![])), Some(38));
        // Backwards from the widget after the list, focus enters at the last row.
        let ctx = Context::default();
        let after = frame(&ctx, count, 300.0, true, vec![]).after.unwrap().0;
        ctx.memory_mut(|memory| memory.request_focus(after));
        frame(&ctx, count, 300.0, true, vec![]);
        frame(&ctx, count, 300.0, true, key(Key::Tab, Modifiers::SHIFT));
        let shown = frame(&ctx, count, 300.0, true, vec![]);
        let id = ctx.memory(|memory| memory.focused());
        assert_eq!(
            shown.rows.iter().find(|(_, row, _)| Some(*row) == id).map(|(index, ..)| *index),
            Some(count - 1)
        );
    }
}
