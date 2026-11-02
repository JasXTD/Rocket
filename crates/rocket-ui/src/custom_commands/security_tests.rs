use super::*;
use crate::{
    batch::flow_tests::{click, enabled, frame, open},
    test_common as common,
};

fn editor(app: &mut Rocket) -> &mut Editor {
    app.dialog.as_mut().unwrap().commands.as_mut().unwrap()
}
fn no_probe(app: &mut Rocket) {
    let e = editor(app);
    assert!(e.program_requested.is_none());
    assert!(e.program.wanted.is_none() && e.program.task.is_none() && e.program.checked.is_none());
}

#[test]
fn imported_programs_are_unprobed_until_check_and_edits_reset_consent() {
    common::isolated(
        "ROCKET_COMMANDS_UI_CASE",
        "custom_commands::security_tests::imported_programs_are_unprobed_until_check_and_edits_reset_consent",
        || {
            let f = common::Fixture::new();
            f.write("base", "base\n");
            f.commit("base");
            let ctx = Context::default();
            let mut app = open(&ctx, &f);
            let program = app
                .backend
                .as_ref()
                .unwrap()
                .executable
                .to_string_lossy()
                .into_owned();
            app.dispatch(Intent::Preferences(crate::preferences::Page::Commands));
            frame(&ctx, &mut app, vec![]);
            let incoming = CustomCommand {
                name: "Imported local fixture".into(),
                program: program.clone(),
                arguments: "--version".into(),
                ..Default::default()
            };
            // Exercise the real decoder and acceptance path, not a fabricated probe result.
            let bytes = crate::command_definitions::encode(&[incoming]).unwrap();
            editor(&mut app).step = Step::Import;
            editor(&mut app).imported = Some(crate::command_definitions::decode(&bytes).unwrap());
            click(&ctx, &mut app, "dialog-commands-import-accept");
            no_probe(&mut app);
            frame(&ctx, &mut app, vec![]);
            no_probe(&mut app);
            assert!(enabled(&ctx, &mut app, "dialog-commands-program-check"));
            no_probe(&mut app);
            click(&ctx, &mut app, "dialog-commands-program-check");
            // Move the debounce deadline, then await the real local filesystem check.
            editor(&mut app).program.wanted = Some((program.clone(), Instant::now() - SETTLE));
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                frame(&ctx, &mut app, vec![]);
                if matches!(
                    editor(&mut app).program.checked,
                    Some((_, Ok(ProgramCheck::Found)))
                ) {
                    break;
                }
                assert!(Instant::now() < deadline, "explicit check did not finish");
                std::thread::yield_now();
            }
            editor(&mut app).list[0].program =
                f.root.join("missing.exe").to_string_lossy().into_owned();
            frame(&ctx, &mut app, vec![]);
            no_probe(&mut app);
            // Invalid legacy definitions remain editable, but neither Check nor Save can use them.
            editor(&mut app).list[0].program = if cfg!(windows) {
                r"\\example.invalid\share\tool.exe"
            } else {
                "relative/tool"
            }
            .into();
            assert!(!enabled(&ctx, &mut app, "dialog-commands-program-check"));
            no_probe(&mut app);
            assert!(!enabled(&ctx, &mut app, "dialog-commands-save"));
            no_probe(&mut app);
            assert!(app.activities.is_empty());
        },
    );
}

#[test]
fn imported_json_rejects_network_paths_and_hidden_text_in_every_schema() {
    let ordinary = CustomCommand {
        name: "Review 文 🚀".into(),
        program: if cfg!(windows) {
            r"C:\Tools\fixture.exe"
        } else {
            "/tools/fixture"
        }
        .into(),
        arguments: "--name={input:text}".into(),
        group: "Tools / Review".into(),
        inputs: vec![CommandInput {
            key: "text".into(),
            label: "Text".into(),
            default_value: "ordinary".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    for version in 1..=3 {
        let mut base: serde_json::Value = serde_json::from_slice(
            &crate::command_definitions::encode(std::slice::from_ref(&ordinary)).unwrap(),
        )
        .unwrap();
        base["version"] = version.into();
        if version >= 2 {
            base["commands"][0]["scope"] = "repository".into();
        }
        if version == 3 {
            base["commands"][0]["action"] = "program".into();
        }
        assert_eq!(
            crate::command_definitions::decode(&serde_json::to_vec(&base).unwrap()).unwrap().as_slice(),
            std::slice::from_ref(&ordinary)
        );
        for pointer in [
            "/commands/0/name",
            "/commands/0/group",
            "/commands/0/program",
            "/commands/0/arguments",
            "/commands/0/inputs/0/label",
            "/commands/0/inputs/0/default",
            "/commands/0/inputs/0/checked",
        ] {
            for hidden in ['\u{202e}', '\u{2066}', '\u{200b}'] {
                let mut data = base.clone();
                let value = data.pointer_mut(pointer).unwrap();
                *value = format!("{}{hidden}", value.as_str().unwrap()).into();
                assert!(
                    crate::command_definitions::decode(&serde_json::to_vec(&data).unwrap())
                        .is_err(),
                    "{version}: {pointer}"
                );
            }
        }
        #[cfg(windows)]
        {
            base["commands"][0]["program"] = r"\\example.invalid\share\tool.exe".into();
            assert!(
                crate::command_definitions::decode(&serde_json::to_vec(&base).unwrap()).is_err()
            );
        }
    }
}

fn visible(output: &FullOutput) -> String {
    fn gather(shape: &Shape, text: &mut String) {
        match shape {
            Shape::Text(t) => {
                text.push_str(&t.galley.job.text);
                text.push('\n');
            }
            Shape::Vec(shapes) => {
                for shape in shapes {
                    gather(shape, text)
                }
            }
            _ => {}
        }
    }
    let mut text = String::new();
    for shape in &output.shapes {
        gather(&shape.shape, &mut text);
    }
    text
}

#[test]
fn confirmation_escapes_context_but_returns_exact_original_arguments() {
    let command = CustomCommand {
        name: "Review".into(),
        program: if cfg!(windows) {
            r"C:\Tools\fixture.exe"
        } else {
            "/tools/fixture"
        }
        .into(),
        arguments: "{branch}".into(),
        ..Default::default()
    };
    let original = "feature/文\u{202e}\u{200b}\\u{202e}";
    let context = CommandContext {
        repository: PathBuf::from(if cfg!(windows) { r"C:\repo" } else { "/repo" }),
        branch: Some(original.into()),
        ..Default::default()
    };
    let ctx = Context::default();
    crate::style::install(&ctx);
    let mut returned = None;
    let mut output = ctx.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 860.0))),
            ..Default::default()
        },
        |ui| {
            returned = confirmation(ui, Some(&command), Some(&context), &BTreeMap::new());
        },
    );
    output.textures_delta.clear();
    assert_eq!(returned.unwrap().1, [OsString::from(original)]);
    let text = visible(&output);
    assert!(!text.contains(['\u{202e}', '\u{200b}']), "{text}");
    assert!(text.contains(r"\u{202e}\u{200b}\\u{202e}"), "{text}");
}

#[test]
fn legacy_command_labels_are_escaped_and_invalid_menu_actions_are_disabled() {
    let c = CustomCommand {
        name: "Review\u{202e}".into(),
        program: if cfg!(windows) {
            r"C:\Tools\fixture.exe"
        } else {
            "/tools/fixture"
        }
        .into(),
        arguments: "--value=\u{200b}".into(),
        inputs: vec![CommandInput {
            key: "flag".into(),
            label: "Flag\u{2066}".into(),
            kind: CommandInputKind::Checkbox,
            checked_value: "--flag\u{200b}".into(),
            default_value: "false".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let ctx = Context::default();
    crate::style::install(&ctx);
    let mut widgets = HashMap::new();
    let mut intents = Vec::new();
    let mut output = ctx.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 860.0))),
            ..Default::default()
        },
        |ui| {
            menu_entries(
                ui,
                std::slice::from_ref(&c),
                CommandScope::Repository,
                None,
                true,
                &mut intents,
                &mut widgets,
            );
            form(ui, &c, &mut BTreeMap::new(), &[], &mut widgets);
            command_chips(ui, &c.program, &[vec![("--value=\u{200b}".into(), false)]]);
        },
    );
    output.textures_delta.clear();
    assert!(!widgets["command-run-0"].enabled);
    assert!(intents.is_empty());
    let text = visible(&output);
    assert!(
        !text.contains(['\u{202e}', '\u{2066}', '\u{200b}']),
        "{text}"
    );
    for expected in [
        r"Review\u{202e}",
        r"Flag\u{2066}",
        r"--flag\u{200b}",
        r"--value=\u{200b}",
    ] {
        assert!(text.contains(expected), "{expected}: {text}");
    }
    assert!(!definition(&c).contains(['\u{202e}', '\u{2066}', '\u{200b}']));
    // The table row's Runs text, which replaced the dialog list's summary.
    assert!(!runs_job(&c).text.contains(['\u{202e}', '\u{2066}', '\u{200b}']));
}

#[test]
fn default_preview_refuses_input_amplification_before_rendering() {
    let c = CustomCommand {
        name: "Expanded defaults".into(),
        program: if cfg!(windows) {
            r"C:\Tools\fixture.exe"
        } else {
            "/tools/fixture"
        }
        .into(),
        arguments: "{input:text}".repeat(1000),
        inputs: vec![CommandInput {
            key: "text".into(),
            label: "Text".into(),
            default_value: "x".repeat(4096),
            ..Default::default()
        }],
        ..Default::default()
    };
    c.validate().unwrap();
    assert!(preview(&c).unwrap_err().contains("32000"));
    let mut boundary = c;
    boundary.inputs[0].default_value = "x".repeat(4000);
    boundary.arguments = format!("{}{}", "{input:text}".repeat(7), "y".repeat(3999));
    assert!(boundary.expand(&CommandContext::default()).is_ok());
    assert!(
        preview(&boundary).is_ok(),
        "exactly 32000 bytes including the separator"
    );
}
