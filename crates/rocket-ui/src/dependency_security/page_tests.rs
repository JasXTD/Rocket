use super::*;
use crate::test_common::{self, Fixture};
use std::sync::Arc;

fn frame(ctx: &Context, app: &mut Rocket, size: Vec2, events: Vec<Event>) -> String {
    app.poll(ctx);
    app.frame_widgets.clear();
    let mut output = ctx.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |ui| {
            app.dialog_ui(ui.ctx());
            app.picker_dialog(ui.ctx());
        },
    );
    fn text(shape: &Shape, values: &mut Vec<String>) {
        match shape {
            Shape::Text(shape) => values.push(shape.galley.job.text.clone()),
            Shape::Vec(shapes) => {
                for shape in shapes {
                    text(shape, values);
                }
            }
            _ => {}
        }
    }
    let mut values = vec![];
    for shape in output.shapes {
        text(&shape.shape, &mut values);
    }
    output.textures_delta.clear();
    values.join("\n")
}
fn settle(ctx: &Context, app: &mut Rocket) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        app.poll(ctx);
        if app.backend.is_some() && !app.busy() && !app.dependencies.running() {
            return;
        }
        assert!(Instant::now() < deadline, "Security test did not settle");
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn click(ctx: &Context, app: &mut Rocket, size: Vec2, key: &str) {
    for _ in 0..12 {
        frame(ctx, app, size, vec![]);
        let control = app.frame_widgets.get(key);
        if let Some(control) = control.filter(|control| control.rect.height() >= 18.0) {
            assert!(control.enabled, "{key} disabled");
            let pos = control.rect.center();
            frame(
                ctx,
                app,
                size,
                vec![
                    Event::PointerMoved(pos),
                    Event::PointerButton {
                        pos,
                        button: PointerButton::Primary,
                        pressed: true,
                        modifiers: Modifiers::NONE,
                    },
                    Event::PointerButton {
                        pos,
                        button: PointerButton::Primary,
                        pressed: false,
                        modifiers: Modifiers::NONE,
                    },
                ],
            );
            frame(ctx, app, size, vec![]);
            return;
        }
        let scroll = app.frame_widgets["prefs-scroll"].rect;
        let toward_top = control.is_some_and(|c| c.rect.center().y < scroll.center().y)
            || matches!(
                key,
                "security-overview" | "dependency-scanner-choose" | "pref-secret-protection"
            );
        let direction = if toward_top { 150.0 } else { -150.0 };
        frame(
            ctx,
            app,
            size,
            vec![
                Event::PointerMoved(scroll.center()),
                Event::MouseWheel {
                    unit: MouseWheelUnit::Point,
                    delta: vec2(0.0, direction),
                    phase: TouchPhase::Move,
                    modifiers: Modifiers::NONE,
                },
            ],
        );
    }
    panic!("{key} could not be revealed at {size:?}");
}

#[test]
fn overview_setup_navigation_and_empty_profile_actions_work_at_both_sizes() {
    test_common::isolated(
        "ROCKET_SECURITY_OVERVIEW_NAV",
        "dependency_security::page_tests::overview_setup_navigation_and_empty_profile_actions_work_at_both_sizes",
        || {
            let fixture = Fixture::new();
            let ctx = Context::default();
            let mut app = Rocket::with_storage(
                &ctx,
                vec![],
                fixture.root.join("logs"),
                fixture.root.join("state"),
            );
            app.dependencies.fixture_executable = Some(fixture.root.join("absent-scanner.exe"));
            settle(&ctx, &mut app);
            for size in [vec2(1280.0, 900.0), vec2(780.0, 620.0)] {
                app.show_preferences(crate::preferences::Page::Security);
                frame(&ctx, &mut app, size, vec![]);
                frame(&ctx, &mut app, size, vec![]);
                assert!(!app.frame_widgets.contains_key("dependency-scanner-choose"));
                click(&ctx, &mut app, size, "dependency-setup-open");
                assert!(app.dependencies.setup_page);
                assert!(!app.frame_widgets["dependencies-update"].enabled);
                assert!(!app.frame_widgets["dependencies-scan"].enabled);
                click(&ctx, &mut app, size, "dependency-scanner-choose");
                assert!(app.picker.is_some());
                click(&ctx, &mut app, size, "picker-cancel");
                assert!(app.picker.is_none());
                assert!(app.dependencies.setup_page);
                click(&ctx, &mut app, size, "security-overview");
                assert!(!app.dependencies.setup_page);
                // Overview retains its previous scroll position. The additional
                // pre-push row can leave the secret card above a compact viewport.
                let scroll = app.frame_widgets["prefs-scroll"].rect;
                for _ in 0..3 {
                    frame(&ctx, &mut app, size, vec![Event::PointerMoved(scroll.center()), Event::MouseWheel { unit: MouseWheelUnit::Point, delta: vec2(0.0, 300.0), phase: TouchPhase::Move, modifiers: Modifiers::NONE }]);
                }
                assert!(app.frame_widgets.contains_key("pref-secret-protection"));
            }
            assert!(
                !fixture
                    .root
                    .join("state/dependency-security/scanner-selection")
                    .exists()
            );
            assert!(!fixture.root.join("state/dependency-security/data").exists());
        },
    );
}

#[test]
fn overview_findings_expand_and_coverage_or_failures_never_look_clean() {
    test_common::isolated(
        "ROCKET_SECURITY_OVERVIEW_RESULTS",
        "dependency_security::page_tests::overview_findings_expand_and_coverage_or_failures_never_look_clean",
        || {
            let fixture = Fixture::new();
            let ctx = Context::default();
            let size = vec2(1280.0, 900.0);
            let mut app = Rocket::with_storage(
                &ctx,
                vec![fixture.repository.clone()],
                fixture.root.join("logs"),
                fixture.root.join("state"),
            );
            app.dependencies.enabled = false;
            app.dependencies.fixture_executable = Some(fixture.root.join("absent-scanner.exe"));
            settle(&ctx, &mut app);
            // Startup loads the persisted preference. Freeze this synthetic report
            // only after that load so the normal background worker cannot replace it.
            app.dependencies.enabled = false;
            app.dependencies.setup.ready = true;
            let owner = app.tab().unwrap().id;
            let mut report = scan::Report {
                packages: 1,
                files: 1,
                findings: vec![scan::Finding {
                    package: "synthetic-parser".into(),
                    version: "1.0.0".into(),
                    ecosystem: "crates.io".into(),
                    file: "Cargo.lock".into(),
                    ids: vec!["DEMO-001".into()],
                    severity: "High".into(),
                    summary: "Synthetic detail that starts collapsed".into(),
                    fixed: vec!["2.0.0".into()],
                }],
                notes: vec![],
                checked: scan::now(),
                oldest_database: scan::now() - 12 * 86_400,
            };
            let cached = |report: scan::Report| scan::Cached {
                inventory: scan::Inventory {
                    files: vec![],
                    notes: vec![],
                },
                database: vec![],
                report: Arc::new(report),
            };
            app.dependencies.repositories.insert(
                owner,
                RepositoryScan {
                    cached: Some(cached(report.clone())),
                    ..Default::default()
                },
            );
            app.show_preferences(crate::preferences::Page::Security);
            frame(&ctx, &mut app, size, vec![]);
            let text = frame(&ctx, &mut app, size, vec![]);
            assert!(
                text.contains("1 advisory needs attention"),
                "Rendered text: {text}; state: {}",
                app.dependencies.summary(Some(owner))
            );
            assert!(text.contains("12 days old"));
            assert!(!app.frame_widgets.contains_key("dependency-advisory-0-0"));
            click(&ctx, &mut app, size, "dependency-finding-0");
            assert!(app.frame_widgets.contains_key("dependency-advisory-0-0"));
            report.findings.clear();
            report
                .notes
                .push("Synthetic unsupported dependency format".into());
            app.dependencies
                .repositories
                .get_mut(&owner)
                .unwrap()
                .cached = Some(cached(report));
            let text = frame(&ctx, &mut app, size, vec![]);
            assert!(text.contains("coverage limited"));
            assert!(!text.contains("No known advisories found"));
            app.dependencies.repositories.get_mut(&owner).unwrap().error =
                Some("Synthetic failed scan; previous report retained".into());
            let text = frame(&ctx, &mut app, size, vec![]);
            assert!(text.contains("Scan incomplete"));
            assert!(!text.contains("Coverage does not guarantee safety"));
            assert_eq!(app.dependencies.summary(Some(owner))["blocking"], false);
            assert!(app.can_write());
        },
    );
}

#[test]
fn moving_to_setup_and_cancelling_keeps_git_available_and_drains_the_job() {
    test_common::isolated(
        "ROCKET_SECURITY_OVERVIEW_CANCEL",
        "dependency_security::page_tests::moving_to_setup_and_cancelling_keeps_git_available_and_drains_the_job",
        || {
            let fixture = Fixture::new();
            let ctx = Context::default();
            let size = vec2(780.0, 620.0);
            let mut app = Rocket::with_storage(
                &ctx,
                vec![fixture.repository.clone()],
                fixture.root.join("logs"),
                fixture.root.join("state"),
            );
            app.dependencies.enabled = false;
            app.dependencies.fixture_executable = Some(fixture.root.join("absent-scanner.exe"));
            settle(&ctx, &mut app);
            app.dependencies.enabled = false;
            app.dependencies.setup.ready = true;
            let tab = app.tab().unwrap();
            let (release, receiver) = std::sync::mpsc::channel();
            app.dependencies.job = Some(Job {
                owner: tab.id,
                path: tab.path.clone(),
                update: false,
                discard: false,
                task: ReadTask::spawn(move |cancel| {
                    receiver.recv_timeout(Duration::from_secs(20)).unwrap();
                    cancel.check()?;
                    Ok(JobResult::Updated)
                })
                .unwrap(),
            });
            app.show_preferences(crate::preferences::Page::Security);
            frame(&ctx, &mut app, size, vec![]);
            frame(&ctx, &mut app, size, vec![]);
            assert!(app.can_write());
            click(&ctx, &mut app, size, "dependency-setup-open");
            assert!(app.dependencies.setup_page);
            click(&ctx, &mut app, size, "dependencies-cancel");
            release.send(()).unwrap();
            settle(&ctx, &mut app);
            assert!(!app.dependencies.running());
            assert_eq!(
                app.dependencies.summary(app.tab().map(|t| t.id))["status"],
                "incomplete"
            );
            assert!(app.can_write());
        },
    );
}
