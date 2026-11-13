//! The page as it draws: where each kind of session shows, what each pane offers, and
//! the list as a switcher in a narrow window.
use super::*;
use crate::test_common as common;
use std::{path::Path, thread, time::Instant};

const CASE: &str = "ROCKET_AGENTS_VIEW_CASE";
const WIDE: Vec2 = vec2(1480.0, 920.0);
const SMALL: Vec2 = vec2(780.0, 620.0);

fn record(id: u64, source: &Path) -> Record {
    Record {
        id,
        name: format!("Task {id} 資料"),
        source: source.to_owned(),
        destination: source.parent().unwrap().join(format!("agent {id}")),
        branch: format!("agents/task-{id}"),
        base: "HEAD".into(),
        provider: Provider::Custom,
        command: Command {
            program: std::env::current_exe().unwrap().to_string_lossy().into_owned(),
            arguments: "--version".into(),
        },
        ..Default::default()
    }
}

/// The app on a small repository, showing Agents with `sessions` saved sessions.
fn open(ctx: &Context, fixture: &common::Fixture, sessions: u64) -> Rocket {
    fixture.write("file.txt", "base\n");
    fixture.commit("base");
    let mut app = Rocket::with_log_directory(ctx, vec![fixture.repository.clone()], fixture.root.join("logs"));
    let deadline = Instant::now() + Duration::from_secs(40);
    while app.busy() {
        app.poll(ctx);
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    app.auto_refresh = false;
    app.tabs[0].view = View::Agents;
    app.agents
        .load((1..=sessions).map(|id| record(id, &fixture.repository)).collect());
    app
}

fn frame(ctx: &Context, app: &mut Rocket, size: Vec2, events: Vec<Event>) {
    app.frame_widgets.clear();
    let mut output = ctx.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |ui| {
            let mut intents = Vec::new();
            app.shell(ui, &mut intents);
            for intent in intents {
                app.dispatch(intent);
            }
            let ctx = ui.ctx().clone();
            app.dialog_ui(&ctx);
            // As the app does: a Browse picker draws after, and above, the dialogs.
            app.picker_dialog(&ctx);
        },
    );
    output.textures_delta.clear();
}

/// Frames until `key` shows: the picker lists its folder on another thread.
fn wait_for(ctx: &Context, app: &mut Rocket, size: Vec2, key: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        frame(ctx, app, size, vec![]);
        if shown(app, key) {
            return;
        }
        assert!(Instant::now() < deadline, "{key} did not show");
        thread::sleep(Duration::from_millis(10));
    }
}

/// Polls until nothing runs and the settings are written.
fn idle(ctx: &Context, app: &mut Rocket) {
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        app.poll(ctx);
        if !app.busy() {
            return;
        }
        assert!(Instant::now() < deadline, "Rocket did not settle");
        thread::sleep(Duration::from_millis(10));
    }
}

/// Two frames: a popup or dialog that just opened is laid out once before it shows.
fn settle(ctx: &Context, app: &mut Rocket, size: Vec2) {
    frame(ctx, app, size, vec![]);
    frame(ctx, app, size, vec![]);
}

fn click(ctx: &Context, app: &mut Rocket, size: Vec2, key: &str) {
    settle(ctx, app, size);
    let control = app
        .frame_widgets
        .get(key)
        .unwrap_or_else(|| panic!("{key} is not shown"));
    assert!(control.enabled, "{key} is disabled");
    let pos = control.rect.center();
    let press = |pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    frame(ctx, app, size, vec![Event::PointerMoved(pos), press(true), press(false)]);
    settle(ctx, app, size);
}

fn shown(app: &Rocket, key: &str) -> bool {
    app.frame_widgets.contains_key(key)
}

fn top(app: &Rocket, key: &str) -> f32 {
    app.frame_widgets[key].rect.top()
}

#[test]
fn sessions_show_under_what_they_need_and_each_pane_offers_its_actions() {
    common::isolated(
        CASE,
        "agents::view::tests::sessions_show_under_what_they_need_and_each_pane_offers_its_actions",
        || {
            let fixture = common::Fixture::new();
            let ctx = Context::default();
            let mut app = open(&ctx, &fixture, 3);
            app.agents.sessions[1].attention = true;
            app.agents.sessions[1].activity_state = events::Activity::Waiting;
            // Failed and already seen: it is not running, and says why in red.
            app.agents.sessions[2].phase = Phase::Failed;
            settle(&ctx, &mut app, WIDE);
            for key in [
                "agents-new",
                "agents-search",
                "agents-group-need",
                "agents-group-idle",
                "agents-session-1",
                "agents-session-2",
                "agents-session-3",
                "agents-tab-terminal",
                "agents-tab-changes",
                "agents-tab-integrate",
                "agents-tab-details",
                "agents-restart",
            ] {
                assert!(shown(&app, key), "{key} is not shown");
            }
            assert!(!shown(&app, "agents-group-running"), "nothing runs, so no Running group");
            assert!(!shown(&app, "agents-switcher"), "a wide window lists sessions in a column");
            assert!(top(&app, "agents-group-need") < top(&app, "agents-session-2"));
            assert!(top(&app, "agents-session-2") < top(&app, "agents-group-idle"));
            assert!(top(&app, "agents-group-idle") < top(&app, "agents-session-1"));
            assert!(top(&app, "agents-group-idle") < top(&app, "agents-session-3"));
            let window = Rect::from_min_size(Pos2::ZERO, WIDE);
            assert!(window.contains_rect(ctx.globally_used_rect()), "the page fits its window");
            assert_eq!(parts::mark_of(&app.agents.sessions[0]), parts::Mark::Idle);
            assert_eq!(parts::mark_of(&app.agents.sessions[1]), parts::Mark::Need);
            assert_eq!(parts::mark_of(&app.agents.sessions[2]), parts::Mark::Failed);
            assert_eq!(parts::state_label(&app.agents.sessions[1]), "Needs your attention");
            assert_eq!(parts::state_label(&app.agents.sessions[2]), "Failed");

            // A folded group hides its rows and keeps the others.
            click(&ctx, &mut app, WIDE, "agents-group-idle");
            assert!(!shown(&app, "agents-session-1") && shown(&app, "agents-session-2"));
            click(&ctx, &mut app, WIDE, "agents-group-idle");
            assert!(shown(&app, "agents-session-1"));

            // The waiting session offers Mark as seen; once seen it is just not running.
            assert!(!shown(&app, "agents-acknowledge"));
            click(&ctx, &mut app, WIDE, "agents-session-2");
            assert_eq!(app.agents.selected, Some(2));
            click(&ctx, &mut app, WIDE, "agents-acknowledge");
            assert!(!app.agents.sessions[1].attention && !app.agents.sessions[1].record.attention);
            assert!(!shown(&app, "agents-group-need") && !shown(&app, "agents-acknowledge"));

            click(&ctx, &mut app, WIDE, "agents-tab-details");
            for key in ["agents-remove", "agents-forget", "agents-copy-branch", "agents-copy-folder", "agents-details-scroll"] {
                assert!(shown(&app, key), "{key} is not shown");
            }
            click(&ctx, &mut app, WIDE, "agents-tab-integrate");
            for key in ["agents-open", "agents-integration-check", "agents-integration-merge", "agents-integration-scroll"] {
                assert!(shown(&app, key), "{key} is not shown");
            }
            assert!(!app.frame_widgets["agents-integration-merge"].enabled, "nothing was checked yet");
            click(&ctx, &mut app, WIDE, "agents-tab-changes");
            assert!(shown(&app, "agents-open"));
            assert!(shown(&app, "agents-refresh") || shown(&app, "agents-read-stop"));
            assert!(window.contains_rect(ctx.globally_used_rect()));
        },
    );
}

#[test]
fn a_narrow_window_lists_sessions_in_a_switcher() {
    common::isolated(
        CASE,
        "agents::view::tests::a_narrow_window_lists_sessions_in_a_switcher",
        || {
            let fixture = common::Fixture::new();
            let ctx = Context::default();
            let mut app = open(&ctx, &fixture, 2);
            settle(&ctx, &mut app, SMALL);
            assert!(shown(&app, "agents-switcher") && shown(&app, "agents-tab-terminal"));
            assert!(!shown(&app, "agents-session-1"), "the list is closed until the switcher opens it");
            assert!(Rect::from_min_size(Pos2::ZERO, SMALL).contains_rect(ctx.globally_used_rect()));
            click(&ctx, &mut app, SMALL, "agents-switcher");
            assert!(shown(&app, "agents-session-1") && shown(&app, "agents-session-2") && shown(&app, "agents-search"));
            click(&ctx, &mut app, SMALL, "agents-session-2");
            assert_eq!(app.agents.selected, Some(2));
            assert!(!shown(&app, "agents-session-2"), "choosing a session closes the list");
        },
    );
}

#[test]
fn a_program_is_found_only_when_its_file_is_there() {
    assert!(!form::present(""));
    // Where Claude Code installs by default: a path is named whether or not it exists.
    let missing = std::env::temp_dir().join("rocket-no-such-folder").join("claude.exe");
    assert!(!form::present(&missing.to_string_lossy()));
    assert!(!form::present(&std::env::temp_dir().to_string_lossy()), "a folder is not a program");
    assert!(form::present(&std::env::current_exe().unwrap().to_string_lossy()));
}

#[test]
fn a_program_that_is_not_found_shows_the_field_for_its_path() {
    common::isolated(
        CASE,
        "agents::view::tests::a_program_that_is_not_found_shows_the_field_for_its_path",
        || {
            let fixture = common::Fixture::new();
            let ctx = Context::default();
            let mut app = open(&ctx, &fixture, 0);
            app.agents.new_form(fixture.repository.clone(), &app.drafting);
            // As Claude Code's preset does on a PC without it: a path that names no file.
            app.agents.form.as_mut().unwrap().record.command.program = fixture
                .root
                .join("not installed")
                .join("claude.exe")
                .to_string_lossy()
                .into_owned();
            // A dialog that opens tall takes a few frames to reach its size.
            settle(&ctx, &mut app, WIDE);
            settle(&ctx, &mut app, WIDE);
            let mut controls: Vec<_> = app.frame_widgets.keys().filter(|k| k.starts_with("agents-")).collect();
            controls.sort();
            assert!(
                shown(&app, "agents-program"),
                "the path can be typed without looking for the field; shown: {controls:?}"
            );
            assert!(!shown(&app, "agents-advanced"));
            // Typing the path of a program that is there keeps the field in view.
            app.agents.form.as_mut().unwrap().record.command.program =
                std::env::current_exe().unwrap().to_string_lossy().into_owned();
            settle(&ctx, &mut app, WIDE);
            assert!(shown(&app, "agents-program") && shown(&app, "agents-review"));
        },
    );
}

#[test]
fn browse_chooses_a_program_and_keeps_it_for_the_next_session() {
    common::isolated(
        CASE,
        "agents::view::tests::browse_chooses_a_program_and_keeps_it_for_the_next_session",
        || {
            use crate::folder_browser::PickTarget;
            use rocket_git::DraftProvider;
            let fixture = common::Fixture::new();
            let ctx = Context::default();
            let mut app = open(&ctx, &fixture, 0);
            // A program where Rocket does not look for one, alone in its folder.
            let tools = fixture.root.canonicalize().unwrap().join("tools 資料");
            std::fs::create_dir(&tools).unwrap();
            let name = |stem: &str| if cfg!(windows) { format!("{stem}.exe") } else { stem.to_owned() };
            let program = tools.join(name("copilot"));
            std::fs::write(&program, b"synthetic executable fixture").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
            }
            let same = |text: &str| {
                crate::views::display_path(Path::new(text)) == crate::views::display_path(&program)
            };
            // The dialog on a preset whose path names no file, whatever this PC has.
            let missing = tools.join(name("old")).to_string_lossy().into_owned();
            let lost = |app: &mut Rocket| {
                app.agents.form.as_mut().unwrap().record.command.program = missing.clone();
            };
            let pick = |ctx: &Context, app: &mut Rocket| {
                if shown(app, "agents-advanced") {
                    click(ctx, app, WIDE, "agents-advanced");
                }
                click(ctx, app, WIDE, "agents-program-browse");
                assert!(
                    app.picker.as_ref().is_some_and(|p| p.target == PickTarget::AgentProgram),
                    "Browse opens the program picker"
                );
                assert!(app.agents.form.is_some(), "the dialog stays under the picker");
                wait_for(ctx, app, WIDE, "picker-row-0");
                // The picker is above the dialog: the pointer reaches its rows.
                let row = app.frame_widgets["picker-row-0"].rect.center();
                assert_eq!(ctx.layer_id_at(row).map(|layer| layer.id), Some(Id::unique("rocket-picker")));
                click(ctx, app, WIDE, "picker-row-0");
                click(ctx, app, WIDE, "picker-choose");
                assert!(app.picker.is_none(), "choosing closes the picker");
            };
            app.agents.new_form(fixture.repository.clone(), &app.drafting);
            click(&ctx, &mut app, WIDE, "agents-provider-copilot");
            lost(&mut app);
            settle(&ctx, &mut app, WIDE);
            settle(&ctx, &mut app, WIDE);
            // Browse sits beside the path, level with it.
            let rect = |app: &Rocket, key: &str| app.frame_widgets[key].rect;
            let (path, browse) = (rect(&app, "agents-program"), rect(&app, "agents-program-browse"));
            assert!(browse.left() >= path.right(), "{browse:?} beside {path:?}");
            assert!((browse.center().y - path.center().y).abs() <= 1.0, "{browse:?} level with {path:?}");
            // The button grows with its label (German's is wider than the least width).
            // In every language the two stay side by side inside the column the fields
            // above them have, and so do the fields after them: a row that sticks out
            // would widen everything laid out below it.
            for language in crate::i18n::Language::ALL {
                crate::i18n::scoped(language, || {
                    settle(&ctx, &mut app, WIDE);
                    let (column, path, browse, below) = (
                        rect(&app, "agents-name"),
                        rect(&app, "agents-program"),
                        rect(&app, "agents-program-browse"),
                        rect(&app, "agents-arguments"),
                    );
                    let (first, second) = if path.left() < browse.left() { (path, browse) } else { (browse, path) };
                    assert!(first.right() <= second.left() + 0.5, "{language:?}: {path:?} over {browse:?}");
                    assert!(
                        first.left() >= column.left() - 0.5 && second.right() <= column.right() + 0.5,
                        "{language:?}: {path:?} and {browse:?} outside {column:?}"
                    );
                    assert!(
                        (below.left() - column.left()).abs() <= 0.5 && (below.right() - column.right()).abs() <= 0.5,
                        "{language:?}: {below:?} is not as wide as {column:?}"
                    );
                });
            }
            settle(&ctx, &mut app, WIDE);
            // Escape belongs to the picker: it closes, the dialog under it stays.
            click(&ctx, &mut app, WIDE, "agents-program-browse");
            assert!(app.picker.is_some());
            let escape = Event::Key {
                key: Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            };
            frame(&ctx, &mut app, WIDE, vec![escape]);
            settle(&ctx, &mut app, WIDE);
            assert!(app.picker.is_none(), "Escape closes the picker");
            assert!(app.agents.form.is_some(), "and leaves the dialog");
            // Choosing fills the field, and GitHub Copilot is found there from now on.
            pick(&ctx, &mut app);
            let form = app.agents.form.as_ref().unwrap();
            assert!(same(&form.record.command.program), "{}", form.record.command.program);
            assert!(same(&app.agents.copilot), "{}", app.agents.copilot);
            assert_eq!(form.found.map(|found| found[2]), Some(true), "its card says Found");
            // Claude Code's is the AI features' own setting.
            click(&ctx, &mut app, WIDE, "agents-provider-claude");
            lost(&mut app);
            pick(&ctx, &mut app);
            assert!(same(app.drafting.program_setting_of(DraftProvider::Claude)));
            assert!(app.drafting.program_setting_of(DraftProvider::Codex).is_empty());
            // Your own program has no preset to keep one for.
            click(&ctx, &mut app, WIDE, "agents-provider-custom");
            lost(&mut app);
            let kept = (app.agents.copilot.clone(), app.drafting.clone());
            pick(&ctx, &mut app);
            assert!(same(&app.agents.form.as_ref().unwrap().record.command.program));
            assert_eq!((app.agents.copilot.clone(), app.drafting.clone()), kept);
            // The next dialog starts from the kept paths.
            click(&ctx, &mut app, WIDE, "agents-cancel");
            assert!(app.agents.form.is_none());
            app.agents.new_form(fixture.repository.clone(), &app.drafting);
            assert!(same(&app.agents.form.as_ref().unwrap().record.command.program), "Claude Code");
            click(&ctx, &mut app, WIDE, "agents-provider-copilot");
            assert!(same(&app.agents.form.as_ref().unwrap().record.command.program), "GitHub Copilot");
            // And so does Rocket opened again on the same profile.
            click(&ctx, &mut app, WIDE, "agents-cancel");
            idle(&ctx, &mut app);
            drop(app);
            let mut reopened = Rocket::with_storage(
                &ctx,
                vec![fixture.repository.clone()],
                fixture.root.join("logs again"),
                fixture.root.join("logs").join("state"),
            );
            idle(&ctx, &mut reopened);
            assert!(reopened.state_error.is_none(), "{:?}", reopened.state_error);
            assert!(same(&reopened.agents.copilot), "{}", reopened.agents.copilot);
            assert!(same(reopened.drafting.program_setting_of(DraftProvider::Claude)));
        },
    );
}

#[test]
fn an_empty_page_offers_the_first_session() {
    common::isolated(
        CASE,
        "agents::view::tests::an_empty_page_offers_the_first_session",
        || {
            let fixture = common::Fixture::new();
            let ctx = Context::default();
            let mut app = open(&ctx, &fixture, 0);
            settle(&ctx, &mut app, WIDE);
            for key in ["agents-new", "agents-start", "agents-guide"] {
                assert!(shown(&app, key), "{key} is not shown");
            }
            assert!(!shown(&app, "agents-search") && !shown(&app, "agents-tab-terminal"));
        },
    );
}

#[test]
fn the_session_dialog_folds_program_fields_until_they_are_needed() {
    common::isolated(
        CASE,
        "agents::view::tests::the_session_dialog_folds_program_fields_until_they_are_needed",
        || {
            let fixture = common::Fixture::new();
            let ctx = Context::default();
            let mut app = open(&ctx, &fixture, 0);
            app.agents.new_form(fixture.repository.clone(), &app.drafting);
            // Whatever is installed on this machine, the chosen program has a path.
            app.agents.form.as_mut().unwrap().record.command.program =
                std::env::current_exe().unwrap().to_string_lossy().into_owned();
            settle(&ctx, &mut app, WIDE);
            for key in [
                "agents-name",
                "agents-provider-claude",
                "agents-provider-codex",
                "agents-provider-copilot",
                "agents-provider-custom",
                "agents-base",
                "agents-branch",
                "agents-destination",
                "agents-tracking",
                "agents-advanced",
                "agents-review",
                "agents-cancel",
            ] {
                assert!(shown(&app, key), "{key} is not shown");
            }
            assert!(!shown(&app, "agents-program"), "the program's path starts folded away");
            assert!(!app.frame_widgets["agents-new"].enabled, "the page waits under its dialog");
            click(&ctx, &mut app, WIDE, "agents-advanced");
            for key in ["agents-program", "agents-arguments", "agents-setup-enabled"] {
                assert!(shown(&app, key), "{key} is not shown");
            }
            assert!(!shown(&app, "agents-advanced"), "once open, the heading is only a caption");
            // Your own program has no path yet: its fields show without being asked for.
            click(&ctx, &mut app, WIDE, "agents-provider-custom");
            assert_eq!(app.agents.form.as_ref().unwrap().record.provider, Provider::Custom);
            assert!(shown(&app, "agents-program") && !shown(&app, "agents-advanced"));
            // Filling in the path, or choosing a program that has one, keeps them in view.
            click(&ctx, &mut app, WIDE, "agents-provider-claude");
            app.agents.form.as_mut().unwrap().record.command.program =
                std::env::current_exe().unwrap().to_string_lossy().into_owned();
            settle(&ctx, &mut app, WIDE);
            assert!(shown(&app, "agents-program") && !shown(&app, "agents-advanced"));
            click(&ctx, &mut app, WIDE, "agents-setup-enabled");
            assert!(shown(&app, "agents-setup-program") && shown(&app, "agents-setup-arguments"));
            let tracking = app.agents.form.as_ref().unwrap().record.tracking;
            click(&ctx, &mut app, WIDE, "agents-tracking");
            assert_eq!(app.agents.form.as_ref().unwrap().record.tracking, !tracking);
        },
    );
}
