use super::*;
use crate::{batch::flow_tests::open, preferences::Page, test_common as common};
use std::{fs, path::Path, thread, time::Duration};

const CASE: &str = "ROCKET_EXTENSIONS_UI_CASE";
const WIDE: Vec2 = vec2(1440.0, 900.0);
const NARROW: Vec2 = vec2(780.0, 620.0);
const LOW: Vec2 = vec2(1440.0, 620.0);

fn frame(ctx: &Context, app: &mut Rocket, size: Vec2, events: Vec<Event>) {
    app.frame_widgets.clear();
    let mut output = ctx.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |ui| {
            // As in the app frame: the window, its intents, then what sits above it.
            let mut intents = Vec::new();
            app.shell(ui, &mut intents);
            for intent in intents {
                app.dispatch(intent);
            }
            let ctx = ui.ctx().clone();
            app.dialog_ui(&ctx);
            app.extension_review_dialog(&ctx);
            app.picker_dialog(&ctx);
        },
    );
    output.textures_delta.clear();
}
/// Until the extensions' background work has ended and its result is on screen.
fn settle(ctx: &Context, app: &mut Rocket, size: Vec2) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        app.poll(ctx);
        frame(ctx, app, size, vec![]);
        if app.integrations.loaded && !app.integrations.running() {
            break;
        }
        assert!(Instant::now() < deadline, "extension work did not finish");
        thread::sleep(Duration::from_millis(10));
    }
    // A newly shown dialog is laid out invisibly first.
    frame(ctx, app, size, vec![]);
    frame(ctx, app, size, vec![]);
}
fn click(ctx: &Context, app: &mut Rocket, size: Vec2, key: &str) {
    settle(ctx, app, size);
    let control = *app.frame_widgets.get(key).unwrap_or_else(|| panic!("{key} is not shown"));
    assert!(control.enabled, "{key} is disabled");
    let pos = control.rect.center();
    let press = |pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame(ctx, app, size, vec![Event::PointerMoved(pos), press(true)]);
    frame(ctx, app, size, vec![press(false)]);
    settle(ctx, app, size);
}
fn package(root: &Path, folder: &str, id: &str, version: &str, permissions: Value, source: &str) -> PathBuf {
    let directory = root.join(folder);
    fs::create_dir_all(&directory).unwrap();
    let manifest = json!({"api":1,"id":id,"name":"Team dashboard","version":version,"permissions":permissions});
    fs::write(directory.join("integration.json"), manifest.to_string()).unwrap();
    fs::write(directory.join("main.lua"), source).unwrap();
    directory
}
fn state(app: &Rocket) -> Value {
    app.integrations.snapshot()
}
fn inside(app: &Rocket, id: &str, size: Vec2) {
    let rect = app.frame_widgets.get(id).unwrap_or_else(|| panic!("{id} at {size:?}")).rect;
    assert!(
        rect.min.x >= 0.0 && rect.min.y >= 0.0 && rect.max.x <= size.x && rect.max.y <= size.y && rect.height() >= 18.0,
        "{id} leaves the window: {rect:?} at {size:?}"
    );
}

#[test]
fn tiles_take_a_letter_and_versions_compare_as_numbers() {
    assert_eq!(extensions::letter("jira"), 'J');
    assert_eq!(extensions::letter("  (資料) board"), '資');
    assert_eq!(extensions::letter("···"), '?');
    assert!(extensions::later("1.10.0", "1.9.0"));
    assert!(!extensions::later("1.1.0", "1.2.0"));
    assert!(!extensions::later("1.1.0", "1.1.0"));
    assert!(extensions::later("2026-rc", "1.1.0"), "free text counts as different");
}

/// The first run lists bundled Jira off, with nothing on disk. Only its review puts it there,
/// and from then on it has its own sidebar entry and page.
#[test]
fn bundled_jira_goes_from_the_list_through_its_review_to_the_sidebar() {
    common::isolated(CASE, "integrations::tests::bundled_jira_goes_from_the_list_through_its_review_to_the_sidebar", || {
        let fixture = common::Fixture::new();
        let ctx = Context::default();
        let mut app = open(&ctx, &fixture);
        settle(&ctx, &mut app, WIDE);
        let folder = app.integrations.directory.join("integrations").join("jira");
        assert!(app.integrations.entries().is_empty());
        assert!(!app.frame_widgets.contains_key("collapse-extensions"), "no section with nothing on");

        app.dispatch(Intent::Preferences(Page::Integrations));
        settle(&ctx, &mut app, WIDE);
        assert!(app.frame_widgets.contains_key("int-package-jira"));
        assert!(!app.frame_widgets.contains_key("pref-ext-jira"));

        click(&ctx, &mut app, WIDE, "int-enable-jira");
        assert_eq!(state(&app)["prompt"]["kind"], "enable");
        assert!(!folder.exists(), "the review comes first");
        click(&ctx, &mut app, WIDE, "int-review-approve");
        assert!(folder.join("main.lua").exists());
        assert_eq!(state(&app)["prompt"], Value::Null);
        assert_eq!(state(&app)["details"], true, "the connection is the next step");
        assert_eq!(state(&app)["needs_connection"], true);
        assert!(app.frame_widgets.contains_key("pref-ext-jira"));
        assert!(app.frame_widgets.contains_key("int-server"));

        click(&ctx, &mut app, WIDE, "dialog-close");
        click(&ctx, &mut app, WIDE, "view-ext-jira");
        assert_eq!(app.page(), Some(crate::pages::Page::Integrations));
        assert!(app.frame_widgets.contains_key("int-add-connection"));
        assert!(!app.frame_widgets.contains_key("int-run-first"));
        click(&ctx, &mut app, WIDE, "int-add-connection");
        assert_eq!(app.preferences_page(), Some(Page::Integrations));
        assert_eq!(state(&app)["details"], true);

        // Off takes the entry away and keeps the approval, so on asks nothing.
        click(&ctx, &mut app, WIDE, "int-enable");
        assert!(app.integrations.entries().is_empty());
        assert!(folder.exists());
        click(&ctx, &mut app, WIDE, "int-enable");
        assert_eq!(state(&app)["prompt"], Value::Null);
        assert_eq!(app.integrations.entries().len(), 1);
    });
}

/// Save and Discard show only after an edit, and saving a connection approves nothing.
#[test]
fn a_connection_saves_inside_its_extension_without_approving_files() {
    common::isolated(CASE, "integrations::tests::a_connection_saves_inside_its_extension_without_approving_files", || {
        let fixture = common::Fixture::new();
        let ctx = Context::default();
        let mut app = open(&ctx, &fixture);
        settle(&ctx, &mut app, WIDE);
        let root = app.integrations.directory.join("integrations");
        let source = "return function() return ui.text('one') end";
        package(&root, "team", "team", "1.0.0", json!({"network":true,"write":true}), source);
        app.dispatch(Intent::Preferences(Page::Integrations));
        click(&ctx, &mut app, WIDE, "int-reload");
        click(&ctx, &mut app, WIDE, "int-package-team");
        assert!(app.frame_widgets.contains_key("int-new-connection"));
        assert!(!app.frame_widgets.contains_key("int-save"), "nothing edited yet");
        app.integrations.draft.origin = "https://team.example.invalid".into();
        settle(&ctx, &mut app, WIDE);
        assert!(app.frame_widgets.contains_key("int-discard"));
        click(&ctx, &mut app, WIDE, "int-discard");
        assert!(app.integrations.draft.origin.is_empty());
        app.integrations.draft.origin = "https://team.example.invalid".into();
        settle(&ctx, &mut app, WIDE);
        click(&ctx, &mut app, WIDE, "int-save");
        assert!(!app.frame_widgets.contains_key("int-save"));
        assert_eq!(state(&app)["extension_enabled"], false, "saving turns nothing on");

        click(&ctx, &mut app, WIDE, "int-enable");
        assert_eq!(state(&app)["prompt"]["kind"], "enable");
        click(&ctx, &mut app, WIDE, "int-review-approve");
        assert_eq!(state(&app)["enabled"], true);

        // A saved connection stays unapproved while files wait for their review.
        fs::write(root.join("team/main.lua"), "return function() return ui.text('two') end").unwrap();
        click(&ctx, &mut app, WIDE, "int-back");
        click(&ctx, &mut app, WIDE, "int-reload");
        click(&ctx, &mut app, WIDE, "int-package-team");
        assert_eq!(state(&app)["needs_review"], true);
        assert_eq!(state(&app)["enabled"], false);
        app.integrations.draft.origin = "https://other.example.invalid".into();
        settle(&ctx, &mut app, WIDE);
        click(&ctx, &mut app, WIDE, "int-save");
        assert_eq!(state(&app)["needs_review"], true);
        assert_eq!(state(&app)["enabled"], false);
    });
}

/// Edited files ask again and say how much changed. A file updates what it matches, keeps
/// its connections, and a file that can't be installed says why.
#[test]
fn reviews_show_what_changed_and_files_install_update_or_explain() {
    common::isolated(CASE, "integrations::tests::reviews_show_what_changed_and_files_install_update_or_explain", || {
        let fixture = common::Fixture::new();
        let ctx = Context::default();
        let mut app = open(&ctx, &fixture);
        settle(&ctx, &mut app, WIDE);
        let root = app.integrations.directory.join("integrations");
        let downloads = fixture.root.join("downloads");
        let source = "return function()\n  return ui.text('one')\nend\n";
        let v1 = package(&downloads, "v1", "team", "1.0.0", json!({"repository":true}), source);
        app.dispatch(Intent::Preferences(Page::Integrations));
        settle(&ctx, &mut app, WIDE);
        let backend = app.backend.clone().unwrap();

        app.integrations.pick_file(backend.clone(), v1.join("integration.json"));
        settle(&ctx, &mut app, WIDE);
        assert_eq!(state(&app)["prompt"]["kind"], "install");
        assert!(!root.join("team").exists(), "nothing lands before the review");
        click(&ctx, &mut app, WIDE, "int-review-approve");
        assert_eq!(fs::read_to_string(root.join("team/main.lua")).unwrap(), source);
        assert_eq!(state(&app)["extension_enabled"], true);
        assert_eq!(state(&app)["selected"], "team");

        // The same files again: nothing to do.
        app.integrations.pick_file(backend.clone(), v1.join("integration.json"));
        settle(&ctx, &mut app, WIDE);
        assert_eq!(state(&app)["prompt"]["kind"], "installed");
        click(&ctx, &mut app, WIDE, "int-review-show");
        assert_eq!(state(&app)["prompt"], Value::Null);

        // Edited on disk: the same review, with the changed lines.
        fs::write(root.join("team/main.lua"), "return function()\n  return ui.text('two')\nend\n-- note\n").unwrap();
        click(&ctx, &mut app, WIDE, "int-back");
        click(&ctx, &mut app, WIDE, "int-reload");
        assert!(app.integrations.entries()[0].attention);
        click(&ctx, &mut app, WIDE, "int-review-team");
        let prompt = &state(&app)["prompt"];
        assert_eq!(prompt["kind"], "again");
        assert_eq!((prompt["added"].as_u64(), prompt["removed"].as_u64()), (Some(2), Some(1)));
        click(&ctx, &mut app, WIDE, "int-review-code");
        assert_eq!(state(&app)["prompt"]["code_open"], true);
        assert!(app.frame_widgets.contains_key("int-package-code-scroll"), "{:?}", app.frame_widgets.keys().filter(|k| k.starts_with("int-")).collect::<Vec<_>>());
        click(&ctx, &mut app, WIDE, "int-review-approve");
        assert_eq!(state(&app)["needs_review"], false);
        assert!(!app.integrations.entries()[0].attention);

        // A newer file: an update that names the new permission.
        let v2 = package(&downloads, "v2", "team", "2.0.0", json!({"repository":true,"network":true}), source);
        app.integrations.pick_file(backend.clone(), v2.join("integration.json"));
        settle(&ctx, &mut app, WIDE);
        let prompt = &state(&app)["prompt"];
        assert_eq!(prompt["kind"], "update");
        assert_eq!(prompt["version"], "1.0.0 → 2.0.0");
        assert_eq!(prompt["new_permissions"], json!(["Network"]));
        click(&ctx, &mut app, WIDE, "int-review-approve");
        let installed = Package::load(&root.join("team")).unwrap();
        assert_eq!(installed.version, "2.0.0");
        assert_eq!(state(&app)["needs_review"], false);

        // An update leaves an extension that was switched off as it was, reviewed for later.
        click(&ctx, &mut app, WIDE, "int-enable");
        assert_eq!(state(&app)["extension_enabled"], false);
        let v3 = package(&downloads, "v3", "team", "3.0.0", json!({"repository":true,"network":true}), source);
        app.integrations.pick_file(backend.clone(), v3.join("integration.json"));
        settle(&ctx, &mut app, WIDE);
        assert_eq!(state(&app)["prompt"]["kind"], "update");
        click(&ctx, &mut app, WIDE, "int-review-approve");
        assert_eq!(Package::load(&root.join("team")).unwrap().version, "3.0.0");
        assert_eq!(state(&app)["extension_enabled"], false);
        click(&ctx, &mut app, WIDE, "int-enable");
        assert_eq!(state(&app)["prompt"], Value::Null);
        assert_eq!(state(&app)["extension_enabled"], true);

        // Bundled Jira's ID can't arrive from a file, so it can't take over saved tokens.
        let fake = package(&downloads, "fake", "jira", "9.0.0", json!({"network":true}), source);
        app.integrations.pick_file(backend.clone(), fake.join("integration.json"));
        settle(&ctx, &mut app, WIDE);
        assert_eq!(state(&app)["prompt"], json!({"kind":"failed","code":"integration_bundled"}));
        assert!(!root.join("jira").exists());
        click(&ctx, &mut app, WIDE, "dialog-close");
        assert_eq!(state(&app)["prompt"], Value::Null);

        // A file from a newer API says so.
        let newer = downloads.join("board");
        fs::create_dir_all(&newer).unwrap();
        fs::write(newer.join("integration.json"), json!({"api":2,"id":"board","name":"Board view","version":"1"}).to_string()).unwrap();
        fs::write(newer.join("main.lua"), source).unwrap();
        app.integrations.pick_file(backend.clone(), newer.join("integration.json"));
        settle(&ctx, &mut app, WIDE);
        assert_eq!(state(&app)["prompt"], json!({"kind":"failed","code":"integration_api"}));
        assert!(app.frame_widgets.contains_key("int-review-choose"));
        click(&ctx, &mut app, WIDE, "dialog-close");

        // Remove takes the folder and the saved connection.
        click(&ctx, &mut app, WIDE, "int-remove");
        assert_eq!(state(&app)["prompt"]["kind"], "remove");
        click(&ctx, &mut app, WIDE, "int-review-approve");
        assert!(!root.join("team").exists());
        assert_eq!(state(&app)["details"], false);
        assert!(app.integrations.entries().is_empty());
        assert!(profiles::connection(&app.integrations.store, "team", "default").is_null());
    });
}

/// Broken folders show as rows without stopping healthy extensions, and one copy of a
/// duplicate ID can be removed from the list.
#[test]
fn problem_folders_are_rows_and_one_duplicate_can_go() {
    common::isolated(CASE, "integrations::tests::problem_folders_are_rows_and_one_duplicate_can_go", || {
        let fixture = common::Fixture::new();
        let ctx = Context::default();
        let mut app = open(&ctx, &fixture);
        settle(&ctx, &mut app, WIDE);
        let root = app.integrations.directory.join("integrations");
        let source = "return function() return ui.text('one') end";
        package(&root, "copy-a", "team", "1.0.0", json!({}), source);
        package(&root, "copy-b", "team", "1.0.0", json!({}), source);
        package(&root, "other", "other", "1.0.0", json!({}), source);
        fs::create_dir_all(root.join("broken")).unwrap();
        fs::write(root.join("broken/integration.json"), "{}").unwrap();
        app.dispatch(Intent::Preferences(Page::Integrations));
        click(&ctx, &mut app, WIDE, "int-reload");
        assert_eq!(state(&app)["package_errors"], json!(["integration_files", "integration_duplicate", "integration_duplicate"]));
        assert!(app.frame_widgets.contains_key("int-package-other"));
        assert!(app.frame_widgets.contains_key("int-issue-folder-0"));
        click(&ctx, &mut app, WIDE, "int-issue-remove-2");
        assert_eq!(state(&app)["prompt"]["kind"], "remove");
        click(&ctx, &mut app, WIDE, "int-review-approve");
        assert!(root.join("copy-a").exists());
        assert!(!root.join("copy-b").exists());
        assert!(app.frame_widgets.contains_key("int-package-team"));
    });
}

/// With an extension enabled, the narrow window keeps the list, the details and the sidebar
/// entry inside it, the rail showing the extension's letter.
#[test]
fn extensions_fit_a_narrow_window() {
    common::isolated(CASE, "integrations::tests::extensions_fit_a_narrow_window", || {
        let fixture = common::Fixture::new();
        let ctx = Context::default();
        let mut app = open(&ctx, &fixture);
        settle(&ctx, &mut app, NARROW);
        app.dispatch(Intent::Preferences(Page::Integrations));
        settle(&ctx, &mut app, NARROW);
        for id in ["int-install-file", "int-reload", "int-package-jira", "int-enable-jira", "int-developer"] {
            inside(&app, id, NARROW);
        }
        click(&ctx, &mut app, NARROW, "int-enable-jira");
        inside(&app, "int-review-approve", NARROW);
        click(&ctx, &mut app, NARROW, "int-review-approve");
        for id in ["int-back", "int-enable", "int-connection", "int-new-connection"] {
            inside(&app, id, NARROW);
        }
        click(&ctx, &mut app, NARROW, "dialog-close");
        // Below the fold of the rail at this height, and brought into view when opened.
        assert!(!app.frame_widgets.contains_key("view-ext-jira"));
        app.dispatch(Intent::Extension("jira".into()));
        settle(&ctx, &mut app, NARROW);
        // The scroll is animated.
        for _ in 0..60 {
            frame(&ctx, &mut app, NARROW, vec![]);
        }
        for id in ["view-ext-jira", "int-configure", "int-run", "int-add-connection"] {
            inside(&app, id, NARROW);
        }
    });
}

/// One frame of a view at `width`, with what its controls reported.
fn draw_view(ctx: &Context, view: &Node, width: f32, values: &mut HashMap<String, Value>, events: Vec<Event>) -> (HashMap<String, UiControl>, Option<PanelEvent>) {
    let mut widgets = HashMap::new();
    let mut action = None;
    let mut changed = false;
    let mut output = ctx.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 800.0))),
            events,
            ..Default::default()
        },
        |ui| {
            let mut panel = Panel::new(values, &mut widgets, &mut action, &mut changed, "int-ui-");
            render_node(ui, view, &mut panel, true);
        },
    );
    output.textures_delta.clear();
    (widgets, action)
}
/// Clicks a control of a view and returns what it reported.
fn click_view(ctx: &Context, view: &Node, width: f32, values: &mut HashMap<String, Value>, key: &str) -> Option<PanelEvent> {
    let (widgets, _) = draw_view(ctx, view, width, values, vec![]);
    let pos = widgets.get(key).unwrap_or_else(|| panic!("{key} is not shown: {:?}", widgets.keys().collect::<Vec<_>>())).rect.center();
    let press = |pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    draw_view(ctx, view, width, values, vec![Event::PointerMoved(pos), press(true)]);
    draw_view(ctx, view, width, values, vec![press(false)]).1
}
fn view(value: Value) -> Node {
    Node::parse(&value).unwrap()
}
/// A context with the app's fonts, which panels draw with.
fn styled() -> Context {
    let ctx = Context::default();
    install(&ctx);
    ctx
}

#[test]
fn a_row_click_reports_its_key_and_a_narrow_window_keeps_the_rows() {
    let table = view(json!({"type":"table","id":"issues","columns":["Key","Summary","Status"],"keys":["A-1","A-2"],"selected":"A-1",
        "rows":[[{"text":"A-1","mono":true},"First",{"text":"Done","tone":"success"}],["A-2","Second",{"text":"To do","tone":"neutral"}]]}));
    for width in [1100.0, 500.0] {
        let ctx = styled();
        let mut values = HashMap::new();
        let event = click_view(&ctx, &table, width, &mut values, "int-ui-issues-A-2");
        assert_eq!(event, Some(PanelEvent { event: "issues".into(), row: Some("A-2".into()) }), "at {width}");
    }
    // A plain table is not a control, in a narrow window or a wide one.
    let plain = view(json!({"type":"table","columns":["A"],"rows":[["x"]]}));
    let ctx = styled();
    let (widgets, _) = draw_view(&ctx, &plain, 1100.0, &mut HashMap::new(), vec![]);
    assert!(widgets.is_empty());
}

#[test]
fn a_plain_table_keeps_its_text_where_assistive_tools_read_it() {
    let table = view(json!({"type":"table","columns":["Name","State"],"rows":[["First",{"text":"Done","tone":"success"}]]}));
    let ctx = styled();
    ctx.enable_accesskit();
    let mut output = ctx.run_ui(
        RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 800.0))), ..Default::default() },
        |ui| {
            let (mut values, mut widgets, mut action, mut changed) = (HashMap::new(), HashMap::new(), None, false);
            let mut panel = Panel::new(&mut values, &mut widgets, &mut action, &mut changed, "int-ui-");
            render_node(ui, &table, &mut panel, true);
        },
    );
    output.textures_delta.clear();
    let update = output.platform_output.accesskit_update.expect("a tree");
    let read: Vec<String> = update.nodes.iter().filter_map(|(_, node)| node.value().or(node.label()).map(str::to_owned)).collect();
    assert!(["First", "Done"].iter().all(|word| read.iter().any(|r| r == word)), "{read:?}");
}

#[test]
fn segments_report_the_choice_and_the_script_decides_what_shows() {
    let ctx = styled();
    let segments = view(json!({"type":"segments","id":"scope","options":["Mine","All"],"value":"Mine"}));
    let mut values = HashMap::new();
    let event = click_view(&ctx, &segments, 900.0, &mut values, "int-ui-scope-1");
    assert_eq!(event, Some(PanelEvent::from("scope")));
    assert_eq!(values["scope"], "All");
    assert!(segments.accepts_value("scope", &values["scope"]), "the choice can be saved with a draft");
}

#[test]
fn enter_in_a_search_field_runs_its_event_and_other_fields_ignore_it() {
    let enter = Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    let search = view(json!({"type":"input","id":"q","placeholder":"Key or words","submit":"search"}));
    let plain = view(json!({"type":"input","id":"q"}));
    for (input, expected) in [(&search, Some(PanelEvent::from("search"))), (&plain, None)] {
        let ctx = styled();
        let mut values = HashMap::new();
        click_view(&ctx, input, 900.0, &mut values, "int-ui-q");
        assert_eq!(draw_view(&ctx, input, 900.0, &mut values, vec![enter.clone()]).1, expected);
    }
}

#[test]
fn a_split_is_two_panes_or_two_tabs_and_focus_picks_the_tab() {
    let split = view(json!({"type":"split","focus":2,"children":[
        {"type":"card","title":"Issues","children":[{"type":"button","id":"one","label":"One"}]},
        {"type":"card","title":"ROCKET-7","children":[{"type":"button","id":"two","label":"Two"}]}]}));
    let ctx = styled();
    let mut values = HashMap::new();
    let (wide, _) = draw_view(&ctx, &split, 1100.0, &mut values, vec![]);
    assert_eq!(wide["int-ui-one"].rect.top(), wide["int-ui-two"].rect.top(), "side by side");
    assert!(wide["int-ui-one"].rect.right() < wide["int-ui-two"].rect.left());

    let ctx = styled();
    let (narrow, _) = draw_view(&ctx, &split, 600.0, &mut values, vec![]);
    assert!(narrow.contains_key("int-ui-two") && !narrow.contains_key("int-ui-one"), "the pane the script named");
    click_view(&ctx, &split, 600.0, &mut values, "int-ui-pane-0-0");
    let (narrow, _) = draw_view(&ctx, &split, 600.0, &mut values, vec![]);
    assert!(narrow.contains_key("int-ui-one") && !narrow.contains_key("int-ui-two"), "the tab chosen by hand");
}

/// One frame of a result, with what it drew.
fn draw_result(ctx: &Context, result: &Arc<Node>, events: Vec<Event>) -> HashMap<String, UiControl> {
    let (mut values, mut widgets, mut action, mut changed) = (HashMap::new(), HashMap::new(), None, false);
    let mut output = ctx.run_ui(
        RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 800.0))), events, ..Default::default() },
        |ui| {
            let mut panel = Panel::new(&mut values, &mut widgets, &mut action, &mut changed, "int-ui-").showing(result);
            render_node(ui, result, &mut panel, true);
        },
    );
    output.textures_delta.clear();
    widgets
}

#[test]
fn each_result_opens_on_its_focus_and_each_split_has_its_own_tabs() {
    let pair = |one: &str, two: &str| json!({"type":"split","focus":2,"children":[
        {"type":"card","title":"First","children":[{"type":"button","id":one,"label":"One"}]},
        {"type":"card","title":"Second","children":[{"type":"button","id":two,"label":"Two"}]}]});
    let ctx = styled();
    let first = Arc::new(view(json!({"type":"page","children":[pair("a", "b"), pair("c", "d")]})));
    let shown = draw_result(&ctx, &first, vec![]);
    assert!(shown.contains_key("int-ui-b") && shown.contains_key("int-ui-d"));

    let pos = shown["int-ui-pane-0-0"].rect.center();
    let press = |pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    draw_result(&ctx, &first, vec![Event::PointerMoved(pos), press(true)]);
    draw_result(&ctx, &first, vec![press(false)]);
    let shown = draw_result(&ctx, &first, vec![]);
    assert!(shown.contains_key("int-ui-a"), "the tab chosen by hand");
    assert!(shown.contains_key("int-ui-d"), "the other split stays where it was");

    let next = Arc::new(view(json!({"type":"page","children":[pair("a", "b"), pair("c", "d")]})));
    let shown = draw_result(&ctx, &next, vec![]);
    assert!(shown.contains_key("int-ui-b"), "the next result says again which pane matters");
}

/// A package with commands, in `root`, as Install from file would find it.
fn commands_package(root: &Path, folder: &str, version: &str, permissions: Value, commands: Value) -> PathBuf {
    let directory = root.join(folder);
    fs::create_dir_all(&directory).unwrap();
    let manifest = json!({"api":1,"id":"team","name":"Team dashboard","version":version,"permissions":permissions,"commands":commands});
    fs::write(directory.join("integration.json"), manifest.to_string()).unwrap();
    fs::write(directory.join("main.lua"), "return function(ctx) return ui.text(ctx.event) end").unwrap();
    directory
}

#[test]
fn a_review_marks_new_changed_and_removed_commands() {
    common::isolated(CASE, "integrations::tests::a_review_marks_new_changed_and_removed_commands", || {
        let fixture = common::Fixture::new();
        let ctx = Context::default();
        let mut app = open(&ctx, &fixture);
        settle(&ctx, &mut app, WIDE);
        let downloads = fixture.root.join("downloads");
        let first = json!([{"id":"keep","label":"Keep"},{"id":"rename","label":"Old name"},{"id":"drop","label":"Drop"}]);
        let v1 = commands_package(&downloads, "v1", "1.0.0", json!({"repository":true}), first);
        app.dispatch(Intent::Preferences(Page::Integrations));
        settle(&ctx, &mut app, WIDE);
        let backend = app.backend.clone().unwrap();
        app.integrations.pick_file(backend.clone(), v1.join("integration.json"));
        settle(&ctx, &mut app, WIDE);
        // A first review lists every command with nothing to compare.
        assert_eq!(state(&app)["prompt"]["new_commands"], json!([]));
        click(&ctx, &mut app, WIDE, "int-review-approve");
        assert_eq!(app.integrations.entries()[0].commands.len(), 3);

        let second = json!([{"id":"keep","label":"Keep"},{"id":"rename","label":"New name"},{"id":"add","label":"Add","target":"commit"}]);
        let v2 = commands_package(&downloads, "v2", "2.0.0", json!({"repository":true}), second);
        app.integrations.pick_file(backend.clone(), v2.join("integration.json"));
        settle(&ctx, &mut app, WIDE);
        let prompt = &state(&app)["prompt"];
        assert_eq!(prompt["kind"], "update");
        assert_eq!(prompt["new_commands"], json!(["add"]));
        assert_eq!(prompt["changed_commands"], json!(["rename"]));
        assert_eq!(prompt["removed_commands"], json!(["drop"]));
    });
}

#[test]
fn a_panel_stays_where_it_is_while_work_runs() {
    common::isolated(CASE, "integrations::tests::a_panel_stays_where_it_is_while_work_runs", || {
        let fixture = common::Fixture::new();
        let ctx = Context::default();
        let mut app = open(&ctx, &fixture);
        settle(&ctx, &mut app, WIDE);
        let root = app.integrations.directory.join("integrations");
        commands_package(&root, "team", "1.0.0", json!({}), json!([]));
        app.dispatch(Intent::Preferences(Page::Integrations));
        settle(&ctx, &mut app, WIDE);
        click(&ctx, &mut app, WIDE, "int-reload");
        click(&ctx, &mut app, WIDE, "int-enable-team");
        click(&ctx, &mut app, WIDE, "int-review-approve");
        click(&ctx, &mut app, WIDE, "dialog-close");
        app.open_extension("team".into());
        settle(&ctx, &mut app, WIDE);
        app.integrations.view = Some(Arc::new(view(json!({"type":"button","id":"go","label":"Go"}))));
        frame(&ctx, &mut app, WIDE, vec![]);
        let rest = app.frame_widgets["int-ui-go"].rect;

        let (release, wait) = std::sync::mpsc::channel::<()>();
        app.integrations.start("Run callback", move |_| {
            wait.recv().ok();
            Err(Error::Cancelled)
        });
        frame(&ctx, &mut app, WIDE, vec![]);
        assert!(app.frame_widgets.contains_key("int-cancel"), "Stop shows");
        assert_eq!(app.frame_widgets["int-ui-go"].rect, rest, "the page below it stays");
        release.send(()).unwrap();
        settle(&ctx, &mut app, WIDE);
    });
}

#[test]
fn a_changed_command_says_everything_it_was() {
    use rocket_integrations::Target;
    let old = Some("Old".to_owned());
    assert_eq!(review::was(&old, &Some(None)), " · Was “Old” in the Repository menu");
    assert_eq!(review::was(&old, &None), " · Was “Old”");
    assert_eq!(review::was(&None, &Some(Some(Target::Commit))), " · Was on commits");
    assert_eq!(review::was(&None, &None), "");
}

#[test]
fn a_saved_draft_that_no_longer_fits_leaves_one_row_that_leads_to_it() {
    common::isolated(CASE, "integrations::tests::a_saved_draft_that_no_longer_fits_leaves_one_row_that_leads_to_it", || {
        let fixture = common::Fixture::new();
        let ctx = Context::default();
        let mut app = open(&ctx, &fixture);
        settle(&ctx, &mut app, WIDE);
        let root = app.integrations.directory.join("integrations");
        commands_package(&root, "team", "1.0.0", json!({}), json!([{"id":"open","label":"Open board"}]));
        app.dispatch(Intent::Preferences(Page::Integrations));
        settle(&ctx, &mut app, WIDE);
        click(&ctx, &mut app, WIDE, "int-reload");
        click(&ctx, &mut app, WIDE, "int-enable-team");
        click(&ctx, &mut app, WIDE, "int-review-approve");
        assert_eq!(app.integrations.entries()[0].block, None);

        // A draft saved for other files, as after an update or a review of an edit.
        let mut older = app.integrations.packages[0].clone();
        older.source.push_str("-- older\n");
        let profile = profiles::connection(&app.integrations.store, "team", "default").clone();
        let shown = view(json!({"type":"text","text":"Saved"}));
        let draft = profiles::capture(&older, &profile, &shown, &HashMap::new(), &json!({})).unwrap();
        profiles::connection_mut(&mut app.integrations.store, "team", "default")["draft"] = draft;
        app.integrations.refresh_entries();
        let entry = app.integrations.entries()[0].clone();
        assert_eq!(entry.block, Some(Fix::Draft));
        let rows = app.palette_rows("Extensions");
        assert_eq!(rows[0].0, "Check Team dashboard’s saved draft");
        assert_eq!(rows[1].2, "Clear its saved draft first");

        // The row opens the page, and a row left over says what the page says.
        app.dispatch(Intent::ExtensionFix("team".into(), Fix::Draft));
        settle(&ctx, &mut app, WIDE);
        assert_eq!(app.page(), Some(crate::pages::Page::Integrations));
        assert_eq!(state(&app)["draft_blocked"], true);
        app.dispatch(Intent::ExtensionCommand(entry.run(&entry.commands[0], None)));
        settle(&ctx, &mut app, WIDE);
        assert_eq!(state(&app)["error"], "integration_draft_changed");
    });
}

#[test]
fn commands_wait_behind_the_row_that_clears_their_way() {
    use rocket_integrations::Target;
    common::isolated(CASE, "integrations::tests::commands_wait_behind_the_row_that_clears_their_way", || {
        let fixture = common::Fixture::new();
        fixture.write("a.txt", "base\n");
        fixture.commit("base");
        let ctx = Context::default();
        let mut app = open(&ctx, &fixture);
        settle(&ctx, &mut app, WIDE);
        let root = app.integrations.directory.join("integrations");
        let commands = json!([{"id":"open","label":"Open board"},{"id":"find","label":"Find builds","target":"commit"},{"id":"branch","label":"Branch builds","target":"branch"}]);
        commands_package(&root, "team", "1.0.0", json!({"network":true,"repository":true}), commands);
        app.dispatch(Intent::Preferences(Page::Integrations));
        settle(&ctx, &mut app, WIDE);
        click(&ctx, &mut app, WIDE, "int-reload");
        assert!(app.palette_rows("Extensions").is_empty(), "off means no rows");

        click(&ctx, &mut app, WIDE, "int-enable-team");
        click(&ctx, &mut app, WIDE, "int-review-approve");
        // No server yet: everything waits behind one row.
        let entry = app.integrations.entries()[0].clone();
        assert_eq!(entry.block, Some(Fix::Connect));
        assert_eq!(entry.fix_for(None), Some(Fix::Connect));
        let rows = app.palette_rows("Extensions");
        assert_eq!(rows[0], ("Connect Team dashboard".to_owned(), String::new(), ""));
        assert_eq!(rows[1].2, "Connect it first");

        // With a server, a command with no target runs; the others need sharing.
        click(&ctx, &mut app, WIDE, "int-server");
        frame(&ctx, &mut app, WIDE, vec![Event::Text("https://team.example.invalid".into())]);
        click(&ctx, &mut app, WIDE, "int-save");
        let entry = app.integrations.entries()[0].clone();
        assert_eq!(entry.block, None);
        assert_eq!(entry.fix_for(None), None);
        assert_eq!(entry.fix_for(Some(Target::Commit)), Some(Fix::Share));
        let rows = app.palette_rows("Extensions");
        assert_eq!(rows[0].0, "Share repository details with Team dashboard");
        assert_eq!(rows[1], ("Team dashboard: Open board".to_owned(), String::new(), ""));
        assert_eq!(rows[2].2, "Needs repository details");

        // Shared: a commit command needs a commit, and a branch command the checked-out branch.
        click(&ctx, &mut app, WIDE, "int-allow-repository");
        click(&ctx, &mut app, WIDE, "int-save");
        assert!(app.integrations.entries()[0].shares);
        let rows = app.palette_rows("Extensions");
        assert_eq!(rows.len(), 3, "{rows:?}");
        assert!(!rows[2].1.is_empty(), "the branch shows beside the command: {rows:?}");

        // Changed files: the approved commands stay, behind the review, and new ones don't show.
        let wider = json!([{"id":"open","label":"Open board"},{"id":"sneak","label":"Delete everything"}]);
        commands_package(&root, "team", "1.0.0", json!({"network":true,"repository":true}), wider);
        click(&ctx, &mut app, WIDE, "int-back");
        click(&ctx, &mut app, WIDE, "int-reload");
        assert_eq!(app.integrations.entries()[0].block, Some(Fix::Review));
        let manifest = root.join("team").join("integration.json");
        let renamed = fs::read_to_string(&manifest).unwrap().replace("Team dashboard", "Trust me");
        fs::write(&manifest, renamed).unwrap();
        click(&ctx, &mut app, WIDE, "int-reload");
        let rows = app.palette_rows("Extensions");
        assert_eq!(rows[0].0, "Review Team dashboard", "the name a review has seen");
        assert!(rows.iter().all(|r| !r.0.contains("Delete everything")), "{rows:?}");
        assert!(rows[1..].iter().all(|r| r.2 == "Review its files first"), "{rows:?}");
    });
}

#[test]
fn a_command_opens_its_page_and_refuses_what_it_cannot_run() {
    use rocket_integrations::Target;
    common::isolated(CASE, "integrations::tests::a_command_opens_its_page_and_refuses_what_it_cannot_run", || {
        let fixture = common::Fixture::new();
        fixture.write("a.txt", "base\n");
        fixture.commit("base");
        let ctx = Context::default();
        let mut app = open(&ctx, &fixture);
        settle(&ctx, &mut app, WIDE);
        let root = app.integrations.directory.join("integrations");
        let commands = json!([{"id":"open","label":"Open board"},{"id":"find","label":"Find builds","target":"commit"}]);
        commands_package(&root, "team", "1.0.0", json!({"repository":true}), commands);
        app.dispatch(Intent::Preferences(Page::Integrations));
        settle(&ctx, &mut app, WIDE);
        click(&ctx, &mut app, WIDE, "int-reload");
        click(&ctx, &mut app, WIDE, "int-enable-team");
        click(&ctx, &mut app, WIDE, "int-review-approve");
        click(&ctx, &mut app, WIDE, "dialog-close");
        let entry = app.integrations.entries()[0].clone();
        assert_eq!(entry.commands[1].target, Some(Target::Commit));
        let repository = app.tab().unwrap().path.clone();
        let target = CommandTarget { repository, chosen: Chosen::Commit("a".repeat(40)) };

        // A command the files no longer have, or on the wrong kind of target.
        let mut stale = entry.run(&entry.commands[0], None);
        stale.command = "gone".into();
        app.dispatch(Intent::ExtensionCommand(stale));
        settle(&ctx, &mut app, WIDE);
        assert_eq!(app.page(), Some(crate::pages::Page::Integrations), "the page opens either way");
        assert_eq!(state(&app)["error"], "integration_command");
        app.dispatch(Intent::ExtensionCommand(entry.run(&entry.commands[0], Some(target.clone()))));
        settle(&ctx, &mut app, WIDE);
        assert_eq!(state(&app)["error"], "integration_command");

        // Repository details stay off until they are shared, however the command was reached.
        app.dispatch(Intent::ExtensionCommand(entry.run(&entry.commands[1], Some(target.clone()))));
        settle(&ctx, &mut app, WIDE);
        assert_eq!(state(&app)["error"], "integration_permission");

        // A request review waits on its extension, and going back to it keeps it there.
        app.integrations.preview = Some(rocket_integrations::Request { method: "POST".into(), path: "/items".into(), body: None, check: None });
        app.open_extension("team".into());
        settle(&ctx, &mut app, WIDE);
        assert!(app.integrations.preview.is_some());
        let busy = entry.run(&entry.commands[0], None);
        app.dispatch(Intent::ExtensionCommand(busy));
        settle(&ctx, &mut app, WIDE);
        assert!(app.integrations.preview.is_some(), "a row left open can't replace it");
        app.integrations.preview = None;

        // The right-click row reads what is published for the frame.
        app.poll(&ctx);
        let mut intents = Vec::new();
        let mut widgets = HashMap::new();
        let mut output = ctx.run_ui(RawInput::default(), |ui| {
            context_rows(ui, &target, Divider::Before, &mut intents, &mut widgets);
        });
        output.textures_delta.clear();
        assert!(widgets.contains_key("ext-context-team"), "{:?}", widgets.keys().collect::<Vec<_>>());
    });
}

/// An extension opened from the menu or the palette brings its sidebar row into view, whole,
/// and a row already in view stays where it is.
#[test]
fn opening_an_extension_shows_its_sidebar_row() {
    common::isolated(CASE, "integrations::tests::opening_an_extension_shows_its_sidebar_row", || {
        let fixture = common::Fixture::new();
        let ctx = Context::default();
        let mut app = open(&ctx, &fixture);
        settle(&ctx, &mut app, LOW);
        app.dispatch(Intent::Preferences(Page::Integrations));
        settle(&ctx, &mut app, LOW);
        click(&ctx, &mut app, LOW, "int-enable-jira");
        click(&ctx, &mut app, LOW, "int-review-approve");
        click(&ctx, &mut app, LOW, "dialog-close");
        assert!(!app.frame_widgets.contains_key("view-ext-jira"), "below the fold at this height");
        app.dispatch(Intent::Extension("jira".into()));
        settle(&ctx, &mut app, LOW);
        // The scroll is animated.
        for _ in 0..60 {
            frame(&ctx, &mut app, LOW, vec![]);
        }
        let row = app.frame_widgets.get("view-ext-jira").expect("the row is in view").rect;
        assert_eq!(row.height(), 28.0, "the whole row shows: {row:?}");

        click(&ctx, &mut app, LOW, "view-ext-jira");
        for _ in 0..60 {
            frame(&ctx, &mut app, LOW, vec![]);
        }
        assert_eq!(app.frame_widgets["view-ext-jira"].rect, row);
    });
}
