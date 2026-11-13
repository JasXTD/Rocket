use super::*;
use crate::{
    backend::Action,
    test_common::{Fixture, isolated},
};
use std::{fs, path::Path};

fn wait(app: &mut Rocket, ctx: &Context, condition: impl Fn(&Rocket) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        app.poll(ctx);
        if condition(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "push did not settle: {} / {:?} / {:?}",
            app.dependencies.summary(app.tab().map(|t| t.id)),
            app.notice,
            failure(app)
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn settled(app: &Rocket) -> bool {
    app.backend.is_some() && !app.busy() && !app.dependencies.running()
}
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
            app.dependency_push_review_ui(ui.ctx());
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
fn click(ctx: &Context, app: &mut Rocket, key: &str) {
    let size = vec2(1280.0, 860.0);
    for _ in 0..3 {
        frame(ctx, app, size, vec![]);
    }
    let control = app.frame_widgets[key];
    assert!(
        control.enabled && control.rect.height() > 12.0,
        "{key}: enabled={} rect={:?} busy={}",
        control.enabled,
        control.rect,
        app.busy()
    );
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
}
fn phase(app: &Rocket) -> String {
    app.dependencies.push_review.summary()["phase"]
        .as_str()
        .unwrap()
        .into()
}
fn failure(app: &Rocket) -> Option<&str> {
    app.dependencies
        .push_review
        .active
        .as_ref()
        .and_then(|a| match &a.phase {
            push::Phase::Failed(error) => Some(error.as_str()),
            _ => None,
        })
}
fn start(app: &mut Rocket) {
    app.execute(
        app.tab().unwrap().id,
        Action::Push {
            remote: b"origin".to_vec(),
            local: b"main".to_vec(),
            destination: b"main".to_vec(),
            set_upstream: false,
        },
    );
    assert!(app.busy());
}
fn refs(f: &Fixture) -> Vec<u8> {
    f.run_at(
        &f.root.join("remote.git"),
        &["for-each-ref", "--format=%(refname) %(objectname)"],
    )
}
fn prepare(f: &Fixture) {
    f.commit("dependency fixture");
    let remote = f.root.join("remote.git");
    f.run(&[
        "init",
        "--bare",
        "--initial-branch=main",
        remote.to_str().unwrap(),
    ]);
    f.run(&["remote", "add", "origin", remote.to_str().unwrap()]);
}
fn app(f: &Fixture, ctx: &Context) -> Rocket {
    let mut app = Rocket::with_storage(
        ctx,
        vec![f.repository.clone()],
        f.root.join("logs"),
        f.root.join("state"),
    );
    app.dependencies.fixture_executable = Some(f.root.join("not-installed.exe"));
    wait(&mut app, ctx, settled);
    assert!(!app.dependencies.before_push);
    app.dependencies.enabled = false;
    app.dependencies.before_push = true;
    app
}
fn paths() -> (PathBuf, PathBuf) {
    (
        std::env::var_os("ROCKET_OSV_TEST_EXE")
            .expect("approved scanner")
            .into(),
        std::env::var_os("ROCKET_OSV_TEST_FIXTURES")
            .expect("synthetic offline fixtures")
            .into(),
    )
}

#[test]
fn review_layout_keeps_actions_visible_and_escape_cancels() {
    isolated(
        "ROCKET_OSV_PUSH_CASE",
        "dependency_security::push_tests::review_layout_keeps_actions_visible_and_escape_cancels",
        || {
            let f = Fixture::new();
            let ctx = Context::default();
            let mut app = app(&f, &ctx);
            let token = rocket_git::CancellationToken::new();
            app.dependencies.push_review.active = Some(push::Active {
                id: 9001,
                repository: f.repository.clone(),
                cancel: token.clone(),
                phase: push::Phase::Scanning,
            });
            for size in [vec2(1280.0, 860.0), vec2(780.0, 620.0)] {
                for (secrets, ai_keys) in [(true, true), (true, false), (false, true), (false, false)] {
                    app.secret_protection = secrets;
                    app.guard_ai_api_keys = ai_keys;
                    for _ in 0..3 {
                        frame(&ctx, &mut app, size, vec![]);
                    }
                    let text = frame(&ctx, &mut app, size, vec![]);
                    assert!(text.contains("Checking dependencies before push"));
                    assert!(text.contains("The push has not started"));
                    assert_eq!(text.contains("No secrets detected"), secrets);
                    assert_eq!(text.contains("No supported AI API-key formats detected"), ai_keys);
                    assert!(!app.frame_widgets.contains_key("dependency-push-continue"));
                    assert!(
                        Rect::from_min_size(Pos2::ZERO, size)
                            .contains_rect(app.frame_widgets["dependency-push-cancel"].rect)
                    );
                }
            }
            let (decision, _answer) = std::sync::mpsc::sync_channel(1);
            let report = scan::Report {
                packages: 1,
                files: 1,
                notes: vec![],
                checked: 1,
                oldest_database: 1,
                findings: vec![scan::Finding {
                    package: format!("long-{}", "package".repeat(24)),
                    version: "1.0.0".into(),
                    ecosystem: "npm".into(),
                    file: "nested/package-lock.json".into(),
                    ids: vec!["DEMO-1".into()],
                    severity: "High".into(),
                    summary: "Long advisory detail ".repeat(100),
                    fixed: vec!["2.0.0".into()],
                }],
            };
            app.dependencies.push_review.active.as_mut().unwrap().phase = push::Phase::Review {
                reports: vec![push::TipReport {
                    source: rocket_git::ObjectId::parse(&"a".repeat(40)).unwrap(),
                    report: std::sync::Arc::new(report),
                }],
                decision,
            };
            click(&ctx, &mut app, "dependency-push-package-0-0");
            for size in [vec2(1280.0, 860.0), vec2(780.0, 620.0)] {
                for _ in 0..3 {
                    frame(&ctx, &mut app, size, vec![]);
                }
                for key in [
                    "dependency-push-cancel",
                    "dependency-push-continue",
                    "dependency-push-close",
                ] {
                    let control = *app.frame_widgets.get(key).unwrap_or_else(|| {
                        panic!(
                            "{key} missing at {size:?}: {:?}",
                            app.frame_widgets.keys().collect::<Vec<_>>()
                        )
                    });
                    assert!(
                        control.enabled && control.rect.height() > 20.0,
                        "{key} at {size:?}: enabled={}, rect={:?}",
                        control.enabled,
                        control.rect
                    );
                    assert!(
                        Rect::from_min_size(Pos2::ZERO, size).contains_rect(control.rect),
                        "{key}: {:?}",
                        control.rect
                    );
                }
            }
            frame(
                &ctx,
                &mut app,
                vec2(780.0, 620.0),
                vec![Event::Key {
                    key: Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }],
            );
            assert!(token.is_cancelled());
            assert!(!app.dependency_push_review_open());
        },
    );
}

#[test]
#[ignore = "requires approved OSV binary and synthetic offline databases"]
fn actual_osv_scanning_cancel_keeps_remote_unchanged() {
    isolated(
        "ROCKET_OSV_PUSH_CASE",
        "dependency_security::push_tests::actual_osv_scanning_cancel_keeps_remote_unchanged",
        || {
            let (exe, fixtures) = paths();
            let f = Fixture::new();
            locks(&f, &fixtures, "vulnerable");
            prepare(&f);
            let ctx = Context::default();
            let mut app = app(&f, &ctx);
            scanner(&mut app, &f, exe, &fixtures);
            start(&mut app);
            wait(&mut app, &ctx, |a| phase(a) == "scanning");
            for _ in 0..3 {
                frame(&ctx, &mut app, vec2(780.0, 620.0), vec![]);
            }
            assert_eq!(phase(&app), "scanning");
            let pos = app.frame_widgets["dependency-push-cancel"].rect.center();
            frame(
                &ctx,
                &mut app,
                vec2(780.0, 620.0),
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
            wait(&mut app, &ctx, settled);
            assert!(refs(&f).is_empty());
            assert_eq!(phase(&app), "idle");
        },
    );
}

#[test]
fn committed_inventory_peels_tags_preserves_limits_and_ignores_working_edits() {
    isolated(
        "ROCKET_OSV_PUSH_CASE",
        "dependency_security::push_tests::committed_inventory_peels_tags_preserves_limits_and_ignores_working_edits",
        || {
            let f = Fixture::new();
            let contents = format!("version = 3\n# {}\n", "x".repeat(3 * 1024 * 1024));
            f.write("Cargo.lock", &contents);
            f.write("child/requirements.txt", "example==1.0.0\n");
            f.write("vendor/Cargo.lock", "ignored generated dependency\n");
            f.commit("immutable dependency inventory");
            f.run(&["tag", "-a", "v1", "-m", "fixture"]);
            f.write("Cargo.lock", "uncommitted replacement\n");
            let id = rocket_git::ObjectId::parse(
                String::from_utf8(f.run(&["rev-parse", "v1"]))
                    .unwrap()
                    .trim(),
            )
            .unwrap();
            let repo = f.repo();
            let cancel = rocket_git::CancellationToken::new();
            let inventory = scan::committed_inventory(&repo, &id, &cancel).unwrap();
            assert_eq!(inventory.files.len(), 2);
            assert_eq!(inventory.files[0].bytes, contents.as_bytes());
            assert!(inventory.notes.iter().any(|n| n.contains("Manifest-only")));
            let tree = repo.repository_tree(id.as_str(), &cancel).unwrap();
            let entry = tree
                .entries
                .iter()
                .find(|e| e.name() == b"Cargo.lock")
                .unwrap();
            assert!(
                repo.tree_file(entry, &cancel)
                    .unwrap_err()
                    .to_string()
                    .contains("2 MiB preview")
            );
            assert!(
                repo.tree_file_with_limit(entry, 9 * 1024 * 1024, &cancel)
                    .is_err()
            );
        },
    );
}
fn locks(f: &Fixture, fixtures: &Path, version: &str) {
    for file in ["Cargo.lock", "package-lock.json"] {
        fs::copy(fixtures.join(version).join(file), f.repository.join(file)).unwrap();
    }
}
fn scanner(app: &mut Rocket, f: &Fixture, executable: PathBuf, fixtures: &Path) {
    let cache = f.root.join("data");
    for ecosystem in ["crates.io", "npm"] {
        let target = cache.join("osv-scalibr").join(ecosystem);
        fs::create_dir_all(&target).unwrap();
        fs::copy(
            fixtures
                .join("cache/osv-scalibr")
                .join(ecosystem)
                .join("all.zip"),
            target.join("all.zip"),
        )
        .unwrap();
    }
    app.dependencies.setup.program = Some(executable);
    // The fixture root contains ../ segments. OSV's Go cache path uses slash
    // joining, so give it the same canonical base as normal profile storage.
    app.dependencies.fixture_cache = Some(cache.canonicalize().unwrap());
}

#[test]
fn pointer_push_preference_defaults_off_and_persists_independently() {
    isolated(
        "ROCKET_OSV_PUSH_CASE",
        "dependency_security::push_tests::pointer_push_preference_defaults_off_and_persists_independently",
        || {
            let f = Fixture::new();
            let ctx = Context::default();
            let mut app =
                Rocket::with_storage(&ctx, vec![], f.root.join("logs"), f.root.join("state"));
            wait(&mut app, &ctx, settled);
            assert!(!app.dependencies.before_push);
            app.dependencies.enabled = false;
            app.show_preferences(crate::preferences::Page::Security);
            frame(&ctx, &mut app, vec2(1280.0, 860.0), vec![]);
            wait(&mut app, &ctx, settled);
            crate::preferences::reveal_test_control(&ctx, &mut app, vec2(1280.0, 860.0), "pref-dependency-push", frame);
            click(&ctx, &mut app, "pref-dependency-push");
            assert!(app.dependencies.before_push);
            assert!(!app.dependencies.enabled);
            app.begin_quit();
            wait(&mut app, &ctx, settled);
            drop(app);
            let mut reopened =
                Rocket::with_storage(&ctx, vec![], f.root.join("logs2"), f.root.join("state"));
            wait(&mut reopened, &ctx, settled);
            assert!(reopened.dependencies.before_push);
            assert!(!reopened.dependencies.enabled);
        },
    );
}

#[test]
fn missing_coverage_requires_review_and_shutdown_cancels_without_publishing() {
    isolated(
        "ROCKET_OSV_PUSH_CASE",
        "dependency_security::push_tests::missing_coverage_requires_review_and_shutdown_cancels_without_publishing",
        || {
            let f = Fixture::new();
            f.write("readme.md", "no dependencies\n");
            prepare(&f);
            let ctx = Context::default();
            let mut app = app(&f, &ctx);
            start(&mut app);
            wait(&mut app, &ctx, |a| phase(a) == "review");
            assert!(refs(&f).is_empty());
            for size in [vec2(1280.0, 860.0), vec2(780.0, 620.0)] {
                for _ in 0..3 {
                    frame(&ctx, &mut app, size, vec![]);
                }
                for key in ["dependency-push-continue", "dependency-push-cancel"] {
                    let control = app.frame_widgets[key];
                    assert!(control.enabled && control.rect.height() >= 18.0);
                    assert!(Rect::from_min_size(Pos2::ZERO, size).contains_rect(control.rect));
                }
            }
            app.dependencies.editable = false;
            frame(&ctx, &mut app, vec2(1280.0, 860.0), vec![]);
            assert!(!app.frame_widgets["dependency-push-continue"].enabled);
            app.begin_quit();
            wait(&mut app, &ctx, settled);
            assert!(!app.dependency_push_review_open());
            assert!(refs(&f).is_empty());
        },
    );
}

#[test]
#[ignore = "requires approved OSV binary and synthetic offline databases"]
fn actual_osv_warns_on_committed_versions_cancel_then_approve_the_same_tip() {
    isolated(
        "ROCKET_OSV_PUSH_CASE",
        "dependency_security::push_tests::actual_osv_warns_on_committed_versions_cancel_then_approve_the_same_tip",
        || {
            let (exe, fixtures) = paths();
            let f = Fixture::new();
            locks(&f, &fixtures, "vulnerable");
            prepare(&f);
            let committed = f.run(&["rev-parse", "HEAD"]);
            locks(&f, &fixtures, "fixed"); // Uncommitted fixes cannot hide the pushed version.
            let ctx = Context::default();
            let mut app = app(&f, &ctx);
            scanner(&mut app, &f, exe, &fixtures);
            start(&mut app);
            wait(&mut app, &ctx, |a| phase(a) == "review");
            assert_eq!(app.dependencies.push_review.summary()["findings"], 2);
            assert!(refs(&f).is_empty());
            click(&ctx, &mut app, "dependency-push-package-0-0");
            let text = frame(&ctx, &mut app, vec2(1280.0, 860.0), vec![]);
            assert!(text.contains("Synthetic regression fixture"));
            click(&ctx, &mut app, "dependency-push-cancel");
            wait(&mut app, &ctx, settled);
            assert!(refs(&f).is_empty());
            start(&mut app);
            wait(&mut app, &ctx, |a| phase(a) == "review");
            assert!(refs(&f).is_empty());
            click(&ctx, &mut app, "dependency-push-continue");
            wait(&mut app, &ctx, settled);
            assert_eq!(
                f.run_at(&f.root.join("remote.git"), &["rev-parse", "main"]),
                committed
            );
            assert!(!app.dependency_push_review_open());
        },
    );
}

#[test]
#[ignore = "requires approved OSV binary and synthetic offline databases"]
fn actual_osv_clean_tip_continues_but_data_changes_and_scanner_failure_stop_push() {
    isolated(
        "ROCKET_OSV_PUSH_CASE",
        "dependency_security::push_tests::actual_osv_clean_tip_continues_but_data_changes_and_scanner_failure_stop_push",
        || {
            let (exe, fixtures) = paths();
            for case in ["clean", "changed-data", "missing-scanner", "missing-data"] {
                let f = Fixture::new();
                locks(
                    &f,
                    &fixtures,
                    if case == "clean" {
                        "fixed"
                    } else {
                        "vulnerable"
                    },
                );
                prepare(&f);
                if case == "clean" {
                    locks(&f, &fixtures, "vulnerable");
                }
                let ctx = Context::default();
                let mut app = app(&f, &ctx);
                scanner(
                    &mut app,
                    &f,
                    if case == "missing-scanner" {
                        f.root.join("not-installed.exe")
                    } else {
                        exe.clone()
                    },
                    &fixtures,
                );
                if case == "missing-data" {
                    fs::remove_file(f.root.join("data/osv-scalibr/npm/all.zip")).unwrap();
                }
                start(&mut app);
                if case == "clean" {
                    wait(&mut app, &ctx, |a| {
                        assert_ne!(
                            phase(a),
                            "review",
                            "uncommitted vulnerable edits are not pushed"
                        );
                        settled(a)
                    });
                    assert!(
                        !refs(&f).is_empty(),
                        "clean push failed: {:?} / {:?}",
                        failure(&app),
                        app.notice
                    );
                    assert!(!app.dependency_push_review_open());
                    continue;
                }
                if case == "changed-data" {
                    wait(&mut app, &ctx, |a| phase(a) == "review");
                    let file = f.root.join("data/osv-scalibr/npm/all.zip");
                    let mut bytes = fs::read(&file).unwrap();
                    bytes.push(0);
                    fs::write(file, bytes).unwrap();
                    click(&ctx, &mut app, "dependency-push-continue");
                }
                wait(&mut app, &ctx, |a| settled(a) && phase(a) == "failed");
                assert!(refs(&f).is_empty());
                for _ in 0..3 {
                    frame(&ctx, &mut app, vec2(780.0, 620.0), vec![]);
                }
                assert!(!app.frame_widgets.contains_key("dependency-push-continue"));
                assert!(app.frame_widgets["dependency-push-setup"].enabled);
                click(&ctx, &mut app, "dependency-push-cancel");
                app.dependencies.before_push = false;
                start(&mut app);
                wait(&mut app, &ctx, settled);
                assert!(
                    !refs(&f).is_empty(),
                    "disabling the optional check restores ordinary push: {case}"
                );
            }
        },
    );
}

#[test]
#[ignore = "requires approved OSV binary and synthetic offline databases"]
fn actual_osv_malformed_source_stops_push_before_publication() {
    isolated(
        "ROCKET_OSV_PUSH_CASE",
        "dependency_security::push_tests::actual_osv_malformed_source_stops_push_before_publication",
        || {
            let (exe, fixtures) = paths();
            let f = Fixture::new();
            locks(&f, &fixtures, "vulnerable");
            let lockfile = f.repository.join("package-lock.json");
            let mut lock: serde_json::Value =
                serde_json::from_slice(&fs::read(&lockfile).unwrap()).unwrap();
            for package in lock["packages"].as_object_mut().unwrap().values_mut() {
                package["resolved"] = serde_json::json!("garbage");
            }
            fs::write(lockfile, serde_json::to_vec(&lock).unwrap()).unwrap();
            prepare(&f);
            // A clean working tree lockfile must not hide the malformed pushed tip.
            locks(&f, &fixtures, "fixed");
            let ctx = Context::default();
            let mut app = app(&f, &ctx);
            scanner(&mut app, &f, exe, &fixtures);
            start(&mut app);
            wait(&mut app, &ctx, |a| settled(a) && phase(a) == "failed");
            assert!(failure(&app).unwrap().contains("incomplete"));
            assert!(refs(&f).is_empty());
            for _ in 0..3 {
                frame(&ctx, &mut app, vec2(780.0, 620.0), vec![]);
            }
            assert!(!app.frame_widgets.contains_key("dependency-push-continue"));
            click(&ctx, &mut app, "dependency-push-cancel");
            wait(&mut app, &ctx, settled);
            assert!(refs(&f).is_empty());
        },
    );
}
