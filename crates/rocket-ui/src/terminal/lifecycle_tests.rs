use super::*;
use crate::{persistence::Store, test_common as common};
use std::{
    fs, thread,
    time::{Duration, Instant},
};

fn settle(app: &mut Rocket, ctx: &Context) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        app.poll(ctx);
        assert!(app.startup_error.is_none(), "{:?}", app.startup_error);
        assert!(app.state_error.is_none(), "{:?}", app.state_error);
        if app.backend.is_some()
            && app.forge.drafts.loaded
            && !app.busy()
            && !app.terminal_log.busy()
        {
            break;
        }
        assert!(Instant::now() < deadline, "application did not settle");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn contended_terminal_logging_keeps_poll_responsive_and_drains_on_exit() {
    common::isolated(
        "ROCKET_TERMINAL_LOG_LIFECYCLE_CASE",
        "terminal::lifecycle_tests::contended_terminal_logging_keeps_poll_responsive_and_drains_on_exit",
        || {
            let fixture = common::Fixture::new();
            let directory = fixture.root.join("state");
            let (store, mut saved) = Store::open(&directory).unwrap();
            saved.preferences.auto_refresh = false;
            saved.preferences.dependency_warnings = false;
            store.save(&saved).unwrap();
            drop(store);
            let ctx = Context::default();
            let mut app = Rocket::with_storage(
                &ctx,
                vec![fixture.repository.clone()],
                fixture.root.join("logs"),
                directory,
            );
            app.terminal_history_file = Some(fixture.root.join("history.txt"));
            settle(&mut app, &ctx);
            app.open_terminal();
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                app.poll(&ctx);
                let terminal = app.tabs[0].terminal.as_ref().unwrap();
                assert!(
                    terminal.status.error.is_none(),
                    "{:?}",
                    terminal.status.error
                );
                if terminal.status.running {
                    break;
                }
                assert!(Instant::now() < deadline, "terminal did not start");
                thread::sleep(Duration::from_millis(10));
            }
            settle(&mut app, &ctx);
            let log = app.backend.as_ref().unwrap().log.clone();
            let lock = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(log.directory().join("rocket.lock"))
                .unwrap();
            lock.try_lock().unwrap();
            app.begin_quit();
            let started = Instant::now();
            app.poll(&ctx);
            let elapsed = started.elapsed();
            // The old UI-thread logger waits at least 500 ms for this OS lock.
            // Leave ample scheduler headroom while detecting that actual wait.
            assert!(
                elapsed < Duration::from_millis(250),
                "UI poll blocked on logging: {elapsed:?}"
            );
            assert!(app.terminal_log.busy());
            assert!(app.busy(), "exit must wait for the queued terminal records");
            drop(lock);
            settle(&mut app, &ctx);
            assert!(app.quitting);
            assert!(app.tabs[0].terminal.as_ref().unwrap().status.closed);
            let records = fs::read_to_string(log.directory().join("rocket.jsonl")).unwrap();
            for operation in ["terminal_stop", "terminal_exit"] {
                assert!(
                    records.lines().any(|line| {
                        let row: serde_json::Value = serde_json::from_str(line).unwrap();
                        row["event"] == "request_end" && row["operation"] == operation
                    }),
                    "missing drained event: {operation}"
                );
            }
            assert_eq!(log.health().dropped_events, 0);
            drop(app);
        },
    );
}
