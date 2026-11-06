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

fn line(component: &str, kind: &str, ms: u64, code: u32, fault: &str) -> String {
    format!(
        "ROCKET-CRASH-1\t{}\t{component}\t{kind}\t{ms}\t11\t12\t-\t0\t0\t{code}\t{fault}\t\twindows\tx86_64\n",
        env!("CARGO_PKG_VERSION")
    )
}
#[test]
fn next_start_offers_only_a_window_app_report_from_just_before_this_start() {
    let (root, _) = fixture();
    let start = 1_793_800_000_000u64;
    fs::write(root.join("crash-10-1-1.rcr"), line("desktop", "windows-exception", start - 300_000, 0xC000_0005, "6:52112")).unwrap();
    fs::write(root.join("crash-11-1-1.rcr"), line("cli", "rust-panic", start - 60_000, 0, "0:0")).unwrap();
    fs::write(root.join("crash-12-1-1.rcr"), line("mcp", "rust-panic", start - 20 * 60_000, 0, "0:0")).unwrap();
    fs::write(root.join("crash-13-1-1.rcr"), line("desktop", "rust-panic", start + 1_000, 0, "0:0")).unwrap();
    let snapshot = reports::read(&root, &CancellationToken::new()).unwrap();
    // The CLI report is newer, a report after the start is this session's own.
    assert_eq!(recent(&snapshot, start).as_deref(), Some("crash-10-1-1-1"));
    assert_eq!(recent(&snapshot, start + 60 * 60_000), None);
    // The startup read sets the offer once, and never while the dialog is open.
    let mut state = State::new(root.clone());
    state.started_ms = start;
    state.check_startup(&Context::default(), None);
    settle(&mut state);
    assert_eq!(state.recent.as_deref(), Some("crash-10-1-1-1"));
    assert!(state.error.is_none() && state.notice.is_none());
    state.recent = None;
    state.check_startup(&Context::default(), None);
    assert!(state.read.is_none(), "the startup read runs once per process");
    let mut open = State::new(root.clone());
    open.started_ms = start;
    open.open = true;
    open.check_startup(&Context::default(), None);
    settle(&mut open);
    assert_eq!(open.recent, None);
    open.show(Some("crash-12-1-1-1".into()));
    assert_eq!(open.selected.as_deref(), Some("crash-12-1-1-1"));
    assert!(open.refresh_requested && !open.export_open);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn save_as_needs_a_json_name_and_publishes_into_the_chosen_folder() {
    let (root, mut state) = fixture();
    state.start_export(Some(&root));
    assert!(state.export_open);
    assert_eq!(state.export_dir, crate::views::display_path(&root));
    assert!(state.export_name.starts_with("rocket-crash-") && state.export_name.ends_with(".json"));
    for bad in ["", "nested/report.json", r"nested\report.json", "report.txt"] {
        state.export_name = bad.into();
        state.save_as(Some(root.clone()), None);
        assert!(!state.exporting());
        assert!(state.error.as_ref().unwrap().contains(".json"), "{bad}");
    }
    state.export_name = "新しい report.JSON".into();
    state.save_as(Some(root.clone()), None);
    settle(&mut state);
    assert!(state.error.is_none(), "{:?}", state.error);
    assert!(root.join("新しい report.JSON").is_file());
    assert!(!state.export_open);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn reports_use_plain_names_for_apps_kinds_and_error_codes() {
    let root = std::env::temp_dir().join(format!("rocket-crash-names-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir(&root).unwrap();
    fs::write(root.join("crash-1-1-1.rcr"), line("desktop", "windows-exception", 1_793_800_000_000, 0xC000_0005, "6:52112")).unwrap();
    fs::write(root.join("crash-2-1-1.rcr"), line("cli", "rust-panic", 1_793_700_000_000, 0, "0:0")).unwrap();
    fs::write(root.join("crash-3-1-1.rcr"), line("mcp", "windows-exception", 1_793_600_000_000, 0xE042_5243, "4:152425")).unwrap();
    let snapshot = reports::read(&root, &CancellationToken::new()).unwrap();
    let titles: Vec<_> = snapshot.reports.iter().map(title).collect();
    assert_eq!(titles, ["Rocket stopped", "Rocket CLI internal error", "MCP server stopped"]);
    assert_eq!(exception_name(0xC000_0005), Some("Access violation"));
    assert_eq!(exception_name(0xE042_5243), None);
    let now = 1_793_800_000_000 + 60_000;
    assert!(detail_line(&snapshot.reports[0], now).starts_with("Access violation · "));
    assert!(detail_line(&snapshot.reports[2], now).starts_with("Windows exception · "));
    assert_eq!(system(&snapshot.reports[0]), "Windows · x64");
    assert_eq!(offsets(&snapshot.reports[0]), ["opengl32 + 0xcb90"]);
    fs::remove_dir_all(root).unwrap();
}
