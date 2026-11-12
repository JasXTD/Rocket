//! Display-only overview. Summaries are built when text/layout changes; painting
//! visits at most 1,024 buckets, regardless of the document's line count.
use crate::style::*;
use eframe::egui::*;

pub const WIDTH: f32 = 18.0;
pub const ADDED: u8 = 1;
pub const REMOVED: u8 = 2;
pub const CONFLICT: u8 = 4;
pub const RESOLVED: u8 = 8;
const BUCKETS: usize = 1024;

#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub rows: usize,
    pub marked_rows: usize,
    buckets: Vec<u8>,
}
impl Summary {
    pub fn from_rows(rows: impl ExactSizeIterator<Item = u8>) -> Self {
        let count = rows.len();
        let mut result = Self {
            rows: count,
            buckets: vec![0; count.min(BUCKETS)],
            ..Default::default()
        };
        for (row, flags) in rows.enumerate() {
            if flags != 0 {
                result.marked_rows += 1;
                let bucket = row * result.buckets.len() / count;
                result.buckets[bucket] |= flags;
            }
        }
        result
    }

    pub fn from_ranges(rows: usize, spans: &[(std::ops::Range<usize>, u8)]) -> Self {
        let mut result = Self {
            rows,
            buckets: vec![0; rows.min(BUCKETS)],
            ..Default::default()
        };
        if rows == 0 {
            return result;
        }
        for (range, flags) in spans {
            let start = range.start.min(rows - 1);
            let end = range.end.max(start + 1).min(rows);
            result.marked_rows += end - start;
            let first = start * result.buckets.len() / rows;
            let last = (end - 1) * result.buckets.len() / rows;
            for cell in &mut result.buckets[first..=last] {
                *cell |= flags;
            }
        }
        result
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct View {
    pub offset: f32,
    pub extent: f32,
    pub viewport: f32,
}
impl View {
    pub fn max(self) -> f32 {
        (self.extent - self.viewport).max(0.0)
    }
    pub fn scrollable(self) -> bool {
        self.max() > 1.0 && self.viewport > 0.0
    }
    pub fn seek(self, fraction: f32) -> f32 {
        (fraction.clamp(0.0, 1.0) * self.extent - self.viewport / 2.0).clamp(0.0, self.max())
    }
    fn thumb(self, rail: Rect) -> Rect {
        let height = (rail.height() * self.viewport / self.extent.max(1.0))
            .clamp(5.0_f32.min(rail.height()), rail.height());
        // Keep the minimum-size thumb inside the rail at either end.
        let top =
            self.offset.clamp(0.0, self.max()) / self.max().max(1.0) * (rail.height() - height);
        Rect::from_min_size(
            pos2(rail.left(), rail.top() + top),
            vec2(rail.width(), height),
        )
    }
}

/// Geometry only, for diagnostics/tests. No source text, search string or paths.
#[derive(Clone, Copy, Debug, Default)]
pub struct Probe {
    pub visible: bool,
    pub rows: usize,
    pub marked_rows: usize,
    pub view: View,
}
impl Probe {
    pub fn new(summary: &Summary, view: View) -> Self {
        Self {
            visible: summary.rows > 0 && view.scrollable(),
            rows: summary.rows,
            marked_rows: summary.marked_rows,
            view,
        }
    }
}

pub struct Output {
    pub response: Response,
    pub target: Option<f32>,
}

/// Click/drag moves the viewport; keyboard input is consumed only while this
/// control owns focus. It never selects patch lines or modifies editor text.
pub fn show(
    ui: &mut Ui,
    rect: Rect,
    id: Id,
    summary: &Summary,
    view: View,
    help: &str,
) -> Option<Output> {
    if !view.scrollable() || summary.rows == 0 || rect.height() < 8.0 {
        return None;
    }
    let response = ui
        .interact(rect, id, Sense::click_and_drag())
        .on_hover_text(help);
    let rail = rect.shrink2(vec2(2.0, 2.0));
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    painter.rect_filled(rail, 3.0, DEEP());
    let n = summary.buckets.len().max(1) as f32;
    for (i, flags) in summary.buckets.iter().copied().enumerate() {
        if flags == 0 {
            continue;
        }
        let top = rail.top() + i as f32 / n * rail.height();
        let bottom = (top + (rail.height() / n).max(2.0)).min(rail.bottom());
        let mark = Rect::from_min_max(
            pos2(rail.left() + 2.0, top),
            pos2(rail.right() - 2.0, bottom),
        );
        if flags & CONFLICT != 0 {
            painter.rect_filled(mark, 0.0, AMBER());
        } else if flags & RESOLVED != 0 {
            painter.rect_filled(mark, 0.0, GREEN());
        } else {
            if flags & REMOVED != 0 {
                painter.rect_filled(
                    Rect::from_min_max(
                        mark.min,
                        pos2(
                            if flags & ADDED != 0 {
                                mark.center().x
                            } else {
                                mark.right()
                            },
                            mark.bottom(),
                        ),
                    ),
                    0.0,
                    RED(),
                );
            }
            if flags & ADDED != 0 {
                painter.rect_filled(
                    Rect::from_min_max(
                        pos2(
                            if flags & REMOVED != 0 {
                                mark.center().x
                            } else {
                                mark.left()
                            },
                            mark.top(),
                        ),
                        mark.max,
                    ),
                    0.0,
                    GREEN(),
                );
            }
        }
    }
    let thumb = view.thumb(rail);
    painter.rect_filled(thumb, 2.0, BLUE().gamma_multiply(0.10));
    painter.rect_stroke(
        thumb,
        2.0,
        Stroke::new(if response.has_focus() { 1.5 } else { 1.0 }, BLUE()),
        StrokeKind::Inside,
    );
    let mut target = None;
    if (response.clicked() || response.dragged())
        && let Some(pos) = response.interact_pointer_pos()
    {
        response.request_focus();
        target = Some(view.seek((pos.y - rail.top()) / rail.height()));
    }
    if response.has_focus() {
        let step = view.extent / summary.rows.max(1) as f32;
        for (key, value) in [
            (Key::ArrowUp, view.offset - step),
            (Key::ArrowDown, view.offset + step),
            (Key::PageUp, view.offset - view.viewport),
            (Key::PageDown, view.offset + view.viewport),
            (Key::Home, 0.0),
            (Key::End, view.max()),
        ] {
            if ui.input_mut(|i| i.consume_key(Modifiers::NONE, key)) {
                target = Some(value.clamp(0.0, view.max()));
            }
        }
    }
    Some(Output { response, target })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dense_changes_keep_both_kinds_with_bounded_paint_work() {
        let summary =
            Summary::from_rows((0..50_000).map(|i| if i % 2 == 0 { ADDED } else { REMOVED }));
        assert_eq!(summary.rows, 50_000);
        assert_eq!(summary.marked_rows, 50_000);
        assert_eq!(summary.buckets.len(), BUCKETS);
        assert!(summary.buckets.iter().all(|f| *f == ADDED | REMOVED));
    }
    #[test]
    fn empty_and_short_documents_do_not_scroll() {
        assert!(!Probe::new(&Summary::default(), View::default()).visible);
        let s = Summary::from_rows([ADDED, 0, REMOVED].into_iter());
        assert!(
            !Probe::new(
                &s,
                View {
                    extent: 66.0,
                    viewport: 200.0,
                    offset: 0.0
                }
            )
            .visible
        );
    }
    #[test]
    fn seeks_and_minimum_thumb_stay_inside_document() {
        let v = View {
            extent: 1_000_000.0,
            viewport: 400.0,
            offset: 999_600.0,
        };
        assert_eq!(v.seek(-10.0), 0.0);
        assert_eq!(v.seek(20.0), v.max());
        let r = Rect::from_min_size(pos2(10.0, 10.0), vec2(18.0, 300.0));
        assert_eq!(v.thumb(r).bottom(), r.bottom());
        assert_eq!(View { offset: 0.0, ..v }.thumb(r).top(), r.top());
    }
    #[test]
    fn resolution_and_deleted_ranges_keep_visible_marks() {
        let s = Summary::from_ranges(100, &[(10..16, CONFLICT), (40..40, RESOLVED)]);
        assert_eq!(s.buckets[10], CONFLICT);
        assert_eq!(s.buckets[40], RESOLVED);
        assert_eq!(s.marked_rows, 7);
    }
    #[test]
    fn pointer_drag_seeks_without_mutating_the_summary() {
        let ctx = Context::default();
        let summary = Summary::from_rows((0..300).map(|_| ADDED));
        let mut target = None;
        for events in [
            vec![],
            vec![
                Event::PointerMoved(pos2(28.0, 40.0)),
                Event::PointerButton {
                    pos: pos2(28.0, 40.0),
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::NONE,
                },
            ],
            vec![Event::PointerMoved(pos2(28.0, 275.0))],
        ] {
            let mut output = ctx.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 400.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let o = show(
                        ui,
                        Rect::from_min_size(pos2(20.0, 20.0), vec2(WIDTH, 300.0)),
                        Id::unique("drag-overview"),
                        &summary,
                        View {
                            offset: 0.0,
                            extent: 6600.0,
                            viewport: 300.0,
                        },
                        "overview",
                    )
                    .unwrap();
                    if o.target.is_some() {
                        target = o.target;
                    }
                },
            );
            output.textures_delta.clear();
        }
        assert!(target.is_some_and(|v| v > 5000.0));
        assert_eq!(summary.marked_rows, 300);
    }
    #[test]
    fn keyboard_navigation_requires_overview_focus() {
        let ctx = Context::default();
        let s = Summary::from_rows((0..300).map(|_| ADDED));
        let id = Id::unique("overview-keyboard-test");
        let view = View {
            offset: 120.0,
            extent: 6600.0,
            viewport: 300.0,
        };
        for focused in [false, true] {
            if focused {
                ctx.memory_mut(|m| m.request_focus(id));
            }
            let mut target = None;
            let mut output = ctx.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 400.0))),
                    events: vec![Event::Key {
                        key: Key::End,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Modifiers::NONE,
                    }],
                    ..Default::default()
                },
                |ui| {
                    target = show(
                        ui,
                        Rect::from_min_size(pos2(20.0, 20.0), vec2(WIDTH, 300.0)),
                        id,
                        &s,
                        view,
                        "overview",
                    )
                    .unwrap()
                    .target;
                },
            );
            output.textures_delta.clear();
            assert_eq!(target, focused.then_some(view.max()));
        }
    }
}
