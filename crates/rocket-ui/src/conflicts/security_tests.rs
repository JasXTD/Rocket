use super::*;
use std::time::Instant;

fn maximum_draft() -> State {
    let mut draft = "<<<<<<< ours\na\n=======\nb\n>>>>>>> theirs\n".repeat(3999);
    draft.extend(std::iter::repeat_n(
        'x',
        CONFLICT_TEXT_LIMIT - draft.len() - 1,
    ));
    draft.push('\n');
    assert_eq!(draft.len(), CONFLICT_TEXT_LIMIT);
    assert!(draft.lines().count() <= 20_000);
    let mut state = State {
        draft,
        ..Default::default()
    };
    state.changed();
    assert_eq!(state.blocks.blocks.len(), 3999);
    assert!(!state.blocks.malformed);
    state
}

fn rail_frame(
    state: &mut State,
    context: &Context,
    input: Vec<Event>,
    scrollable: bool,
) -> (HashMap<String, UiControl>, usize) {
    let mut widgets = HashMap::new();
    let view = crate::overview::View {
        offset: 0.0,
        extent: if scrollable {
            state.overview.rows as f32 * 20.0
        } else {
            360.0
        },
        viewport: 360.0,
    };
    let mut output = context.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(640.0, 480.0))),
            events: input,
            ..Default::default()
        },
        |ui| rail(ui, state, 7, 360.0, 20.0, view, &mut widgets),
    );
    let shapes = output.shapes.len();
    output.textures_delta.clear();
    (widgets, shapes)
}

#[test]
fn maximum_conflict_rail_has_bounded_frame_work() {
    let mut state = maximum_draft();
    let context = Context::default();
    let mut times = Vec::new();
    for _ in 0..64 {
        let start = Instant::now();
        let (widgets, shapes) = rail_frame(&mut state, &context, Vec::new(), true);
        times.push(start.elapsed());
        let markers = widgets
            .keys()
            .filter(|key| key.starts_with("conflict-marker-"))
            .count();
        assert!(
            markers <= 256,
            "per-frame marker controls grew with hostile input: {markers}"
        );
        assert!(
            shapes <= 1100,
            "per-frame painting exceeded the summary budget: {shapes}"
        );
    }
    times.sort();
    eprintln!(
        "512 KiB / 3999 conflicts / 64 rail frames: median={:?}, p95={:?}, max={:?}",
        times[32], times[60], times[63]
    );
    assert_eq!(state.draft.len(), CONFLICT_TEXT_LIMIT);
    assert_eq!(state.blocks.blocks.len(), 3999);
}

fn linear_hit(state: &State, rect: Rect, per_line: f32, point: Pos2) -> Option<(usize, usize)> {
    state
        .items
        .iter()
        .enumerate()
        .filter_map(|(index, (item, range))| {
            let Item::Open(block) = item else { return None };
            rail_mark(state, rect, per_line, index)
                .contains(point)
                .then_some((*block, range.start))
        })
        .next_back()
}

#[test]
fn indexed_hit_testing_preserves_overlap_edges_and_gaps() {
    let mut state = maximum_draft();
    let rect = Rect::from_min_size(pos2(17.0, 32.0), vec2(18.0, 360.0));
    for per_line in [20.0, (rect.height() - 4.0) / state.overview.rows as f32] {
        for step in 0..=512 {
            let point = pos2(
                rect.center().x,
                rect.top() - 1.0 + step as f32 * (rect.height() + 2.0) / 512.0,
            );
            assert_eq!(
                rail_hit(&state, rect, per_line, point),
                linear_hit(&state, rect, per_line, point)
            );
        }
        assert_eq!(
            rail_hit(
                &state,
                rect,
                per_line,
                pos2(rect.left() - 1.0, rect.center().y)
            ),
            None
        );
    }
    state.selected = 1;
    state.choose(2);
    let point = pos2(rect.center().x, rect.top() + 102.0);
    assert_eq!(
        rail_hit(&state, rect, 20.0, point),
        linear_hit(&state, rect, 20.0, point)
    );
}

#[test]
fn dense_click_selects_the_indexed_conflict_and_fallback_stays_bounded() {
    let mut state = maximum_draft();
    let context = Context::default();
    let (widgets, _) = rail_frame(&mut state, &context, Vec::new(), true);
    let rect = widgets["conflict-overview"].rect;
    let point = rect.center();
    let per_line = (rect.height() - 4.0) / state.overview.rows as f32;
    let expected = linear_hit(&state, rect, per_line, point).unwrap();
    for pressed in [true, false] {
        rail_frame(
            &mut state,
            &context,
            vec![
                Event::PointerMoved(point),
                Event::PointerButton {
                    pos: point,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ],
            true,
        );
    }
    assert_eq!((state.selected, state.jump.unwrap()), expected);
    assert!(
        state.overview_jump.is_none(),
        "a conflict click takes precedence over scroll seeking"
    );
    let (widgets, shapes) = rail_frame(&mut state, &context, Vec::new(), false);
    assert!(widgets.len() <= 256);
    assert!(shapes <= 1100);
}

#[test]
fn marker_cache_tracks_choices_undo_and_binding_clear() {
    let mut state = State {
        draft: "<<<<<<< ours\na\n=======\nb\n>>>>>>> theirs\n".repeat(2),
        ..Default::default()
    };
    state.changed();
    assert_eq!(state.overview_open, [0, 1]);
    state.choose(2);
    assert_eq!(state.overview_open, [1]);
    assert!(matches!(
        state.items[state.overview_open[0]].0,
        Item::Open(0)
    ));
    state.undo(0);
    assert_eq!(state.overview_open, [0, 1]);
    let context = Context::default();
    let (widgets, _) = rail_frame(&mut state, &context, Vec::new(), false);
    assert!(widgets.contains_key("conflict-marker-0") && widgets.contains_key("conflict-marker-1"));
    let point = widgets["conflict-marker-1"].rect.center();
    for pressed in [true, false] {
        rail_frame(
            &mut state,
            &context,
            vec![
                Event::PointerMoved(point),
                Event::PointerButton {
                    pos: point,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ],
            false,
        );
    }
    assert_eq!(state.selected, 1);
    state.forget_resolutions();
    assert!(state.overview_open.is_empty());
    let (widgets, _) = rail_frame(&mut state, &context, Vec::new(), false);
    assert!(
        widgets.is_empty(),
        "cleared bindings must not retain marker controls"
    );
}
