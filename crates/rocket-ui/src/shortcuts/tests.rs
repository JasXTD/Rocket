use super::*;

#[cfg(target_os = "macos")]
#[test]
fn mac_raw_command_key_dispatches_and_control_and_ime_pass_through() {
    let mut state = State::default();
    let mut input = raw(vec![key("Ctrl+B", true), Event::Text("資料 e\u{301}".into()), key("Cmd+C", true)]);
    state.filter(&mut input, &[Action::Sidebar], true);
    assert_eq!(input.events.len(), 3);
    assert!(state.pending.is_none());
    let mut input = raw(vec![key("Cmd+B", true), key("Cmd+B", false)]);
    state.filter(&mut input, &[Action::Sidebar], true);
    assert!(input.events.is_empty()); assert_eq!(state.pending, Some(Action::Sidebar));
    state.record(Action::Sidebar, 0);
    let mut input = raw(vec![key("Cmd+F8", true), key("Cmd+F8", false)]);
    state.filter(&mut input, &[], true);
    let edit = state.edit.as_ref().unwrap();
    assert_eq!(edit.candidate.map(Binding::text).as_deref(), Some("Cmd+F8"));
    assert!(edit.error.is_none());
    assert!(!edit.recording);
}

fn key(chord: &str, pressed: bool) -> Event {
    let b = Binding::parse(chord).unwrap();
    let Trigger::Key(key) = b.trigger else {
        panic!("key")
    };
    Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: b.egui_modifiers(),
    }
}
fn pointer(chord: &str, pressed: bool) -> Event {
    let b = Binding::parse(chord).unwrap();
    let Trigger::Mouse(button) = b.trigger else {
        panic!("mouse")
    };
    Event::PointerButton {
        pos: pos2(40.0, 40.0),
        button,
        pressed,
        modifiers: b.egui_modifiers(),
    }
}
fn raw(events: Vec<Event>) -> RawInput {
    RawInput {
        focused: true,
        events,
        ..Default::default()
    }
}

#[test]
fn shortcut_recording_does_not_run_actions_or_change_bindings_and_escape_cancels() {
    let mut state = State::default();
    let original = state.bindings.clone();
    state.record(Action::Sidebar, 0);
    let open = Bindings::default().label(Action::Open);
    let mut input = raw(vec![
        key(&open, true),
        Event::Text("o".into()),
        key(&open, false),
    ]);
    state.filter(&mut input, &[Action::Open], true);
    assert!(input.events.is_empty());
    assert!(state.pending.is_none());
    assert_eq!(state.bindings, original);
    assert!(
        state
            .edit
            .as_ref()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("Open repository")
    );
    let mut input = raw(vec![key("Escape", true), key("Escape", false)]);
    state.filter(&mut input, &[], true);
    assert!(input.events.is_empty());
    assert!(state.edit.is_none());
    state.record(Action::Sidebar, 1);
    let mut input = raw(vec![
        pointer("MouseBack", true),
        pointer("MouseBack", false),
    ]);
    state.filter(&mut input, &[Action::Sidebar], false);
    assert!(input.events.is_empty());
    assert_eq!(
        state.edit.as_ref().unwrap().candidate.unwrap().text(),
        "MouseBack"
    );
    assert_eq!(state.bindings, original);
}

#[test]
fn shortcut_mouse_consumption_prevents_underlying_click_and_preserves_plain_input() {
    let mut state = State::default();
    state
        .bindings
        .set(
            Action::Sidebar,
            vec![Binding::parse("Ctrl+MouseLeft").unwrap()],
        )
        .unwrap();
    let ctx = Context::default();
    let mut clicks = 0;
    let mut render = |state: &mut State, events: Vec<Event>| {
        let mut input = raw(events);
        input.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(200.0, 100.0)));
        state.filter(&mut input, &[Action::Sidebar], true);
        let mut output = ctx.run_ui(input, |ui| {
            if ui
                .put(
                    Rect::from_min_max(pos2(20.0, 20.0), pos2(100.0, 60.0)),
                    Button::new("Underlying action"),
                )
                .clicked()
            {
                clicks += 1
            }
        });
        output.textures_delta.clear();
    };
    render(&mut state, vec![Event::PointerMoved(pos2(40.0, 40.0))]);
    render(&mut state, vec![pointer("Ctrl+MouseLeft", true)]);
    assert_eq!(state.pending, Some(Action::Sidebar));
    render(&mut state, vec![pointer("Ctrl+MouseLeft", false)]);
    render(&mut state, vec![pointer("MouseLeft", true)]);
    render(&mut state, vec![pointer("MouseLeft", false)]);
    assert_eq!(clicks, 1, "only the ordinary click reaches the widget");
}

#[test]
fn shortcut_unavailable_commands_and_terminal_pointer_inputs_are_not_stolen() {
    let mut state = State::default();
    state
        .bindings
        .set(Action::Sidebar, vec![Binding::parse("MouseBack").unwrap()])
        .unwrap();
    let mut input = raw(vec![
        pointer("MouseBack", true),
        pointer("MouseBack", false),
    ]);
    state.filter(&mut input, &[], true);
    assert_eq!(input.events.len(), 2);
    assert!(state.pending.is_none());
    state.filter(&mut input, &[Action::Sidebar], false);
    assert_eq!(input.events.len(), 2);
    assert!(state.pending.is_none());
    state.filter(&mut input, &[Action::Sidebar], true);
    assert!(input.events.is_empty());
    assert_eq!(state.pending, Some(Action::Sidebar));
    let mut input = raw(vec![]);
    state.filter(&mut input, &[], true);
    assert!(state.pending.is_none());
    let mut input = raw(vec![pointer("MouseBack", true)]);
    state.filter(&mut input, &[Action::Sidebar], true);
    let mut release = raw(vec![pointer("MouseBack", false)]);
    state.filter(&mut release, &[], false);
    assert!(
        release.events.is_empty(),
        "release stays consumed across context changes"
    );
}

#[test]
fn shortcut_wheel_and_repeat_are_bounded_and_modifiers_are_exact() {
    let mut state = State::default();
    state
        .bindings
        .set(
            Action::Sidebar,
            vec![
                Binding::parse("Ctrl+WheelUp").unwrap(),
                Binding::parse("Ctrl+B").unwrap(),
            ],
        )
        .unwrap();
    let wheel = |amount| Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: vec2(0.0, amount),
        modifiers: Modifiers::CTRL,
        phase: TouchPhase::Move,
        source: MouseWheelSource::Unknown,
    };
    let mut input = raw(vec![wheel(20.0)]);
    state.filter(&mut input, &[Action::Sidebar], true);
    assert!(input.events.is_empty());
    assert!(state.pending.is_none());
    let mut input = raw(vec![wheel(21.0), wheel(80.0)]);
    state.filter(&mut input, &[Action::Sidebar], true);
    assert_eq!(state.pending, Some(Action::Sidebar));
    let mut input = raw(vec![key("Ctrl+Shift+B", true)]);
    state.filter(&mut input, &[Action::Sidebar], true);
    assert_eq!(input.events.len(), 1);
    assert!(state.pending.is_none());
    let mut repeat = key("Ctrl+B", true);
    if let Event::Key { repeat, .. } = &mut repeat {
        *repeat = true
    };
    let mut input = raw(vec![key("Ctrl+B", true), repeat, key("Ctrl+B", false)]);
    state.filter(&mut input, &[Action::Sidebar], true);
    assert!(input.events.is_empty());
    assert_eq!(state.pending, Some(Action::Sidebar));
}

#[test]
fn shortcut_typing_clipboard_and_unmodified_navigation_pass_through() {
    let mut state = State::default();
    let mut input = raw(vec![
        Event::Text("資料 e\u{301} 🦀".into()),
        key("Ctrl+C", true),
        key("Tab", true),
        key("Enter", true),
        pointer("MouseLeft", true),
    ]);
    state.filter(
        &mut input,
        &COMMANDS.iter().map(|c| c.action).collect::<Vec<_>>(),
        true,
    );
    assert_eq!(input.events.len(), 5);
    assert!(state.pending.is_none());
}
