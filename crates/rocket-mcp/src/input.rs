use eframe::egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput};
use std::collections::VecDeque;

/// Published controls that are egui sliders; a click moves one to the clicked point,
/// its centre. Not `image-mix`: that control also covers its value box and label, and
/// image-mix-less/more move it.
pub(crate) fn slider_target(target: &str) -> bool {
    target == "theme-visibility"
}

/// A click at `pos`: the first frame's events, then the frames after it. egui never
/// takes a press and release in one frame as a drag, and its Slider senses drags only,
/// so a slider gets the release a frame later, as from a person. Other controls keep
/// press and release in one frame.
pub(crate) fn click_events(
    pos: Pos2,
    button: PointerButton,
    modifiers: Modifiers,
    slider: bool,
) -> (Vec<Event>, VecDeque<Vec<Event>>) {
    let press = |pressed| Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers,
    };
    let mut events = vec![Event::PointerMoved(pos), press(true)];
    let mut later = VecDeque::new();
    if slider {
        later.push_back(vec![press(false)]);
    } else {
        events.push(press(false));
    }
    (events, later)
}

pub(crate) fn prepare_fixture_input(input: &mut RawInput) {
    // MCP owns this fixture's input frame, even while its OS window is in the
    // background. egui otherwise ignores keys despite retaining widget focus.
    // This changes only synthetic frame input, never desktop/foreground focus.
    input.focused = true;
}

pub(crate) fn key_event(key: Key, pressed: bool) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: Modifiers::NONE,
    }
}

pub(crate) fn replacement_events(text: String) -> Vec<Event> {
    let mut select = key_event(Key::A, true);
    if let Event::Key { modifiers, .. } = &mut select {
        *modifiers = Modifiers::CTRL | Modifiers::COMMAND;
    }
    let mut events = vec![select, key_event(Key::A, false)];
    if text.is_empty() {
        // egui ignores empty text events. Delete the selected text with a real key.
        events.extend([
            key_event(Key::Backspace, true),
            key_event(Key::Backspace, false),
        ]);
    } else {
        events.push(Event::Text(text));
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Context, Id, RawInput, Rect, ScrollArea, Slider, TextEdit};

    // Exercise the real egui text widget, not a model of its input behavior.
    #[test]
    fn replacement_input_can_clear_and_retype_a_focused_editor() {
        let ctx = Context::default();
        let mut text = "Original draft".to_owned();
        let render = |text: &mut String, events: Vec<Event>| {
            let mut input = RawInput {
                events,
                focused: false,
                ..Default::default()
            };
            prepare_fixture_input(&mut input);
            let mut output = ctx.run_ui(input, |ui| {
                ui.add(TextEdit::multiline(text).id(Id::unique("test-editor")))
                    .request_focus();
            });
            // This headless widget test has no renderer to apply texture updates.
            output.textures_delta.clear();
        };
        render(&mut text, vec![]);
        render(&mut text, replacement_events("Replacement draft".into()));
        assert_eq!(text, "Replacement draft");
        render(&mut text, replacement_events(String::new()));
        assert!(
            text.is_empty(),
            "An empty replacement must delete the selection"
        );
        render(&mut text, replacement_events(String::new()));
        assert!(text.is_empty());
        render(&mut text, replacement_events("A new draft".into()));
        assert_eq!(text, "A new draft");
    }

    /// One frame of a slider and a button in a scroll area, as on Preferences →
    /// Appearance: the slider's and the button's rects, and whether the button was clicked.
    fn slider_page(ctx: &Context, events: Vec<Event>, value: &mut u8) -> (Rect, Rect, bool) {
        let mut input = RawInput {
            events,
            ..Default::default()
        };
        prepare_fixture_input(&mut input);
        let mut shown = (Rect::NOTHING, Rect::NOTHING, false);
        let mut output = ctx.run_ui(input, |ui| {
            ScrollArea::vertical().show(ui, |ui| {
                shown.0 = ui
                    .add(Slider::new(&mut *value, 10..=60).show_value(false))
                    .rect;
                let next = ui.button("Next");
                shown.1 = next.rect;
                shown.2 = next.clicked();
            });
        });
        output.textures_delta.clear();
        shown
    }

    // The real egui Slider, which senses drags only, as theme-visibility does.
    #[test]
    fn a_slider_click_moves_the_slider_and_other_clicks_keep_one_frame() {
        let ctx = Context::default();
        let mut value = 30;
        let (slider, next, _) = slider_page(&ctx, vec![], &mut value);
        let click = |rect: Rect, slider| {
            click_events(
                rect.center(),
                PointerButton::Primary,
                Modifiers::NONE,
                slider,
            )
        };
        let (events, later) = click(slider, false);
        assert!(
            later.is_empty(),
            "a control's press and release share one frame"
        );
        slider_page(&ctx, events, &mut value);
        assert_eq!(
            value, 30,
            "one frame's press and release never move a slider"
        );

        let (events, later) = click(slider, slider_target("theme-visibility"));
        assert_eq!(later.len(), 1);
        slider_page(&ctx, events, &mut value);
        assert_eq!(value, 35, "the press moves the slider to its centre");
        for frame in later {
            slider_page(&ctx, frame, &mut value);
        }
        assert_eq!(value, 35);
        assert!(!ctx.input(|i| i.pointer.any_down()), "the release came");

        // The next click is an ordinary one-frame click, and it does not drag the slider.
        let (events, later) = click(next, slider_target("theme-dark"));
        assert!(later.is_empty());
        let (.., clicked) = slider_page(&ctx, events, &mut value);
        assert!(clicked);
        assert_eq!(value, 35);
    }
}
