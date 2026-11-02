#![cfg(windows)]
#[path = "../../rocket-git/tests/common/mod.rs"]
mod common;

use eframe::{App, Frame, egui::*};
use rocket_git::{Error, ReadTask};
use rocket_ui::{
    app::{DialogKind, Intent, Rocket},
    persistence::Store,
};
use std::{
    fs,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

fn fixture() -> (common::Fixture, Context, Rocket) {
    let f = common::Fixture::new();
    f.write("tracked.txt", "unchanged\n");
    f.commit("initial");
    let directory = f.root.join("state");
    let (store, mut state) = Store::open(&directory).unwrap();
    state.preferences.auto_refresh = false;
    state.preferences.dependency_warnings = false;
    store.save(&state).unwrap();
    drop(store);
    let ctx = Context::default();
    let mut app = Rocket::with_storage(&ctx, vec![], f.root.join("logs"), directory);
    settle(&mut app, &ctx);
    (f, ctx, app)
}

fn settle(app: &mut Rocket, ctx: &Context) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        app.poll(ctx);
        assert!(app.startup_error.is_none(), "{:?}", app.startup_error);
        assert!(app.state_error.is_none(), "{:?}", app.state_error);
        if app.backend.is_some()
            && !app.busy()
            && !app.integrations.running()
            && app.integrations.snapshot()["loaded"] == true
        {
            return;
        }
        assert!(Instant::now() < deadline, "Application did not settle");
        thread::sleep(Duration::from_millis(10));
    }
}

fn hidden_input(close: bool) -> RawInput {
    let mut input = RawInput::default();
    let viewport = input.viewports.get_mut(&ViewportId::ROOT).unwrap();
    viewport.minimized = Some(true);
    if close {
        viewport.events.push(ViewportEvent::Close);
    }
    input
}

#[test]
fn native_close_is_handled_during_logic_only_frames() {
    common::isolated(
        "ROCKET_SHUTDOWN_CASE",
        "native_close_is_handled_during_logic_only_frames",
        || {
            let (_f, ctx, mut app) = fixture();
            let output = ctx.run_logic(&hidden_input(true), |ctx| {
                app.logic(ctx, &mut Frame::_new_kittest());
            });
            assert!(
                app.quitting,
                "Native close must enter the same shutdown path while minimized"
            );
            assert!(
                output.viewport_commands[&ViewportId::ROOT].contains(&ViewportCommand::CancelClose),
                "The first close must wait for draft and cleanup checks"
            );
        },
    );
}

#[test]
fn quitting_does_not_wait_for_an_unanswered_clipboard_request() {
    common::isolated(
        "ROCKET_SHUTDOWN_CASE",
        "quitting_does_not_wait_for_an_unanswered_clipboard_request",
        || {
            let (_f, ctx, mut app) = fixture();
            app.dispatch(Intent::PastePatch);
            assert!(app.busy());
            app.request_close();
            assert!(app.quitting);
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let output = ctx.run_logic(&hidden_input(false), |ctx| {
                    app.logic(ctx, &mut Frame::_new_kittest());
                });
                if output
                    .viewport_commands
                    .get(&ViewportId::ROOT)
                    .is_some_and(|commands| commands.contains(&ViewportCommand::Close))
                {
                    assert!(app.paste_patch.is_none());
                    assert!(!app.busy());
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "Shutdown waited forever for clipboard polling that needs a drawn frame"
                );
                thread::sleep(Duration::from_millis(10));
            }
        },
    );
}

#[test]
fn minimized_close_preserves_confirmation_and_commit_drafts() {
    common::isolated(
        "ROCKET_SHUTDOWN_CASE",
        "minimized_close_preserves_confirmation_and_commit_drafts",
        || {
            let (f, ctx, mut app) = fixture();
            let original = f
                .repo()
                .head(&rocket_git::CancellationToken::new())
                .unwrap();
            app.open(f.repository.clone());
            settle(&mut app, &ctx);
            app.tabs[0].draft = "Keep this uncommitted draft 資料".into();
            let output = ctx.run_logic(&hidden_input(true), |ctx| {
                app.logic(ctx, &mut Frame::_new_kittest());
            });
            assert!(!app.quitting);
            assert!(matches!(
                app.dialog.as_ref().map(|d| &d.kind),
                Some(DialogKind::Quit)
            ));
            let commands = &output.viewport_commands[&ViewportId::ROOT];
            assert!(commands.contains(&ViewportCommand::CancelClose));
            assert!(
                commands.contains(&ViewportCommand::Minimized(false)),
                "Make the confirmation visible"
            );
            assert!(!commands.contains(&ViewportCommand::Close));
            // Cancelling the confirmation leaves the message and repository intact.
            app.dialog = None;
            assert_eq!(app.tabs[0].draft, "Keep this uncommitted draft 資料");
            app.begin_quit();
            settle(&mut app, &ctx);
            let repository = app.tabs[0].path.clone();
            drop(app);
            assert_eq!(
                Store::open(&f.root.join("state")).unwrap().1.drafts[&repository].as_ref(),
                "Keep this uncommitted draft 資料"
            );
            assert_eq!(
                f.repo()
                    .head(&rocket_git::CancellationToken::new())
                    .unwrap(),
                original
            );
            assert_eq!(
                fs::read_to_string(f.repository.join("tracked.txt")).unwrap(),
                "unchanged\n"
            );
        },
    );
}

#[test]
fn close_waits_for_cancelled_read_cleanup_without_blocking_logic() {
    common::isolated(
        "ROCKET_SHUTDOWN_CASE",
        "close_waits_for_cancelled_read_cleanup_without_blocking_logic",
        || {
            let (f, ctx, mut app) = fixture();
            app.open(f.repository.clone());
            settle(&mut app, &ctx);
            let (started, ready) = mpsc::channel();
            let (release, gate) = mpsc::channel();
            let (cancelled, observed) = mpsc::channel();
            app.tabs[0].diff = Some(
                ReadTask::spawn(move |cancel| {
                    started.send(()).unwrap();
                    gate.recv_timeout(Duration::from_secs(10)).unwrap();
                    cancelled
                        .send(matches!(cancel.check(), Err(Error::Cancelled)))
                        .unwrap();
                    cancel.check()?;
                    Err(Error::InvalidInput("read was not cancelled".into()))
                })
                .unwrap(),
            );
            ready.recv_timeout(Duration::from_secs(5)).unwrap();
            let started = Instant::now();
            let output = ctx.run_logic(&hidden_input(true), |ctx| {
                app.logic(ctx, &mut Frame::_new_kittest());
            });
            assert!(
                started.elapsed() < Duration::from_secs(1),
                "Close blocked on a worker"
            );
            assert!(app.quitting && app.busy());
            assert!(
                output.viewport_commands[&ViewportId::ROOT].contains(&ViewportCommand::CancelClose)
            );
            assert!(!output.viewport_commands[&ViewportId::ROOT].contains(&ViewportCommand::Close));
            // A second X cannot bypass the cleanup barrier either.
            let output = ctx.run_logic(&hidden_input(true), |ctx| {
                app.logic(ctx, &mut Frame::_new_kittest());
            });
            assert!(
                output.viewport_commands[&ViewportId::ROOT].contains(&ViewportCommand::CancelClose)
            );
            release.send(()).unwrap();
            assert!(observed.recv_timeout(Duration::from_secs(5)).unwrap());
            settle(&mut app, &ctx);
            assert!(app.quitting && !app.busy());
            assert!(app.tabs[0].diff.is_none());
        },
    );
}

#[test]
fn failed_save_restores_the_window_and_can_be_retried() {
    use std::os::windows::fs::OpenOptionsExt;
    common::isolated(
        "ROCKET_SHUTDOWN_CASE",
        "failed_save_restores_the_window_and_can_be_retried",
        || {
            let (f, ctx, mut app) = fixture();
            let state_path = f.root.join("state/state.bin");
            let original = fs::read(&state_path).unwrap();
            let held = fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(&state_path)
                .unwrap();
            app.show_sidebar = !app.show_sidebar;
            let expected = app.show_sidebar;
            app.request_close();
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let output = ctx.run_logic(&hidden_input(true), |ctx| {
                    app.logic(ctx, &mut Frame::_new_kittest());
                });
                let commands = output.viewport_commands.get(&ViewportId::ROOT);
                assert!(
                    commands.is_some_and(|c| c.contains(&ViewportCommand::CancelClose)),
                    "Repeated native close must respect newly started saves and save errors"
                );
                assert!(
                    !commands.is_some_and(|c| c.contains(&ViewportCommand::Close)),
                    "Save failure must not close the app"
                );
                if app.state_error.is_some() {
                    assert!(!app.quitting);
                    assert!(
                        commands.is_some_and(|c| c.contains(&ViewportCommand::Minimized(false)))
                    );
                    break;
                }
                assert!(Instant::now() < deadline, "Save did not finish");
                thread::sleep(Duration::from_millis(10));
            }
            drop(held);
            assert_eq!(fs::read(&state_path).unwrap(), original);
            assert!(app.retry_state_save());
            app.request_close();
            settle(&mut app, &ctx);
            assert!(app.quitting);
            drop(app);
            assert_eq!(
                Store::open(&f.root.join("state"))
                    .unwrap()
                    .1
                    .preferences
                    .sidebar,
                expected
            );
        },
    );
}
