use super::*;
use rocket_git::CancellationToken;
use std::{
    fs,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

fn fixture() -> (PathBuf, State) {
    let root = std::env::temp_dir().join(format!(
        "rocket-crash-view-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    let data = format!(
        "ROCKET-CRASH-1\t{}\tcli\trust-panic\t1793750000000\t1\t2\ttests.rs\t7\t8\t0\t0:0\t1:123\twindows\tx86_64\n",
        env!("CARGO_PKG_VERSION")
    );
    fs::write(root.join("crash-1-2-3.rcr"), data).unwrap();
    let mut state = State::new(root.clone());
    state.snapshot = reports::read(&root, &CancellationToken::new()).unwrap();
    state.selected = Some(state.snapshot.reports[0].id.clone());
    (root, state)
}
fn settle(state: &mut State) {
    let start = Instant::now();
    while state.read.is_some() || state.exporting() {
        state.poll();
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn report_export_enforces_fixture_scope_and_retains_errors() {
    let (root, mut state) = fixture();
    let target = root.join("报告.json");
    fs::write(&target, b"keep this").unwrap();
    state.export_path = target.display().to_string();
    state.save(Some(root.clone()), None);
    settle(&mut state);
    assert!(state.error.as_ref().unwrap().contains("already exists"));
    assert_eq!(fs::read(&target).unwrap(), b"keep this");
    state.export_path = root.join("../outside.json").display().to_string();
    state.save(Some(root.clone()), None);
    settle(&mut state);
    assert!(state.error.as_ref().unwrap().contains("absolute path"));
    state.export_path = root.join("new 报告.json").display().to_string();
    state.save(Some(root.clone()), None);
    settle(&mut state);
    assert!(state.error.is_none());
    assert!(state.notice.as_ref().unwrap().contains("saved"));
    let json: serde_json::Value =
        serde_json::from_slice(&fs::read(&state.export_path).unwrap()).unwrap();
    assert_eq!(json["kind"], "rust-panic");
    assert_eq!(json["line"], 7);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn refresh_stays_loading_from_request_through_background_read() {
    let (root, mut state) = fixture();
    state.refresh_requested = true;
    assert_eq!(state.snapshot()["loading"], true);
    state.load(&Context::default(), None);
    assert_eq!(state.snapshot()["loading"], true);
    settle(&mut state);
    assert_eq!(state.snapshot()["loading"], false);
    assert_eq!(state.snapshot.reports.len(), 1);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn cancelled_or_failed_read_cannot_replace_previous_reports() {
    let (root, mut state) = fixture();
    let id = state.selected.clone();
    state.read = Some(ReadTask::spawn(|_| Err(Error::Cancelled)).unwrap());
    settle(&mut state);
    assert_eq!(state.selected, id);
    assert_eq!(state.snapshot.reports.len(), 1);
    assert!(state.notice.as_ref().unwrap().contains("retained"));
    state.read = Some(
        ReadTask::spawn(|_| Err(Error::Unsupported("Local report read failed.".into()))).unwrap(),
    );
    settle(&mut state);
    assert_eq!(state.selected, id);
    assert_eq!(state.snapshot.reports.len(), 1);
    assert!(state.error.is_some());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn app_shutdown_observes_export_without_rendering_the_modal() {
    crate::test_common::isolated(
        "ROCKET_CRASH_EXPORT_QUIT_CASE",
        "crash_reports::tests::app_shutdown_observes_export_without_rendering_the_modal",
        || {
            let f = crate::test_common::Fixture::new();
            let ctx = Context::default();
            let mut app =
                Rocket::with_storage(&ctx, vec![], f.root.join("logs"), f.root.join("state"));
            let deadline = Instant::now() + Duration::from_secs(40);
            loop {
                app.poll(&ctx);
                assert!(app.startup_error.is_none(), "{:?}", app.startup_error);
                if app.backend.is_some() && !app.busy() {
                    break;
                }
                assert!(Instant::now() < deadline, "app startup did not settle");
                std::thread::sleep(Duration::from_millis(10));
            }
            let (started, observed) = std::sync::mpsc::channel();
            let (release, wait) = std::sync::mpsc::channel();
            app.crash_reports.open = true;
            app.crash_reports.write = Some(
                WriteTask::spawn(move |cancel| {
                    started.send(()).unwrap();
                    wait.recv().unwrap();
                    cancel.check()
                })
                .unwrap(),
            );
            observed.recv_timeout(Duration::from_secs(10)).unwrap();
            app.begin_quit();
            assert!(
                !app.crash_reports.open,
                "app quit must cancel the report panel"
            );
            assert!(app.crash_reports.exporting());
            assert!(app.busy(), "app must wait for the report worker");
            release.send(()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(40);
            while app.busy() {
                app.poll(&ctx);
                assert!(Instant::now() < deadline, "app quit did not settle");
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(!app.crash_reports.exporting());
            assert!(
                app.crash_reports
                    .notice
                    .as_ref()
                    .unwrap()
                    .contains("stopped")
            );
        },
    );
}
#[test]
fn shutdown_cancels_export_and_waits_for_its_observation() {
    let (root, mut state) = fixture();
    let destination = root.join("shutdown.json");
    let target = destination.clone();
    let report = state.selected().unwrap().clone();
    let (started, observed) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel();
    state.write = Some(
        WriteTask::spawn(move |cancel| {
            started.send(()).unwrap();
            wait.recv().unwrap();
            reports::export(&report, &target, &cancel)
        })
        .unwrap(),
    );
    observed.recv_timeout(Duration::from_secs(10)).unwrap();
    state.open = true;
    state.refresh_requested = true;
    state.shutdown();
    state.poll();
    assert!(!state.open);
    assert!(!state.refresh_requested);
    assert!(state.exporting());
    release.send(()).unwrap();
    settle(&mut state);
    assert!(!destination.exists());
    assert!(state.notice.as_ref().unwrap().contains("stopped"));
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn an_export_already_published_is_not_reported_as_cancelled() {
    let (root, mut state) = fixture();
    let destination = root.join("finished.json");
    let target = destination.clone();
    let report = state.selected().unwrap().clone();
    let (published, observed) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel();
    state.write = Some(
        WriteTask::spawn(move |cancel| {
            reports::export(&report, &target, &cancel)?;
            published.send(()).unwrap();
            wait.recv().unwrap();
            Ok(())
        })
        .unwrap(),
    );
    observed.recv_timeout(Duration::from_secs(10)).unwrap();
    state.write.as_ref().unwrap().cancel();
    state.stopping = true;
    release.send(()).unwrap();
    settle(&mut state);
    assert!(destination.exists());
    assert!(state.error.is_none());
    assert!(state.notice.as_ref().unwrap().contains("saved"));
    assert!(!state.stopping);
    fs::remove_dir_all(root).unwrap();
}
