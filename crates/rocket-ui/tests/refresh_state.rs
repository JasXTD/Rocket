#![cfg(windows)]
#[path = "../../rocket-git/tests/common/mod.rs"]
mod common;
use common::Fixture;
use rocket_git::*;
use rocket_ui::{
    app::{Intent, Rocket, View},
    persistence::{Store, WORKTREE_VIEWS},
};
use std::{
    fs, thread,
    time::{Duration, Instant},
};

#[test]
fn full_snapshot_reduces_commands_and_reads_all_sections_fresh() {
    common::isolated(
        "ROCKET_UI_TEST_CASE",
        "full_snapshot_reduces_commands_and_reads_all_sections_fresh",
        || {
            let f = Fixture::new();
            f.write("a", "base\n");
            f.commit("base");
            let logs = f.root.join("snapshot-logs");
            let backend = rocket_ui::backend::Backend::discover(&logs).unwrap();
            let before = fs::read_to_string(logs.join("rocket.jsonl")).unwrap();
            let snap = backend
                .snapshot(&f.repository, HistoryStart::Head, &CancellationToken::new())
                .unwrap();
            assert!(snap.warnings.is_empty(), "{:?}", snap.warnings);
            let after = fs::read_to_string(logs.join("rocket.jsonl")).unwrap();
            assert_eq!(
                after[before.len()..]
                    .lines()
                    .filter(|line| line.contains("\"event\":\"git_start\""))
                    .count(),
                10
            );
            let combined = backend
                .snapshot(
                    &f.repository,
                    HistoryStart::AllBranches,
                    &CancellationToken::new(),
                )
                .unwrap();
            assert!(combined.warnings.is_empty(), "{:?}", combined.warnings);
            assert_eq!(
                combined.history.as_ref().unwrap().commits[0].id,
                snap.history.as_ref().unwrap().commits[0].id
            );
            let combined_log = fs::read_to_string(logs.join("rocket.jsonl")).unwrap();
            assert_eq!(
                combined_log[after.len()..]
                    .lines()
                    .filter(|line| line.contains("\"event\":\"git_start\""))
                    .count(),
                10
            );
            f.run(&["branch", "external"]);
            f.run(&["pack-refs", "--all", "--prune"]);
            f.run(&["remote", "add", "new-origin", "../local-only"]);
            f.write("a", "stashed\n");
            f.run(&["stash", "push", "-m", "synthetic stash"]);
            let head = f.run(&["rev-parse", "HEAD"]);
            fs::write(f.repository.join(".git/MERGE_HEAD"), &head).unwrap();
            f.run(&["branch", "CHERRY_PICK_HEAD"]); // Must not shadow the exact operation ref.
            f.write("a", "fresh\n");
            let snap = backend
                .snapshot(
                    &f.repository,
                    HistoryStart::AllBranches,
                    &CancellationToken::new(),
                )
                .unwrap();
            assert!(snap.warnings.is_empty(), "{:?}", snap.warnings);
            assert_eq!(snap.status.as_ref().unwrap().entries.len(), 1);
            assert!(
                snap.branches
                    .iter()
                    .any(|b| b.reference == b"refs/heads/external")
            );
            assert!(snap.remotes.iter().any(|r| r.name == b"new-origin"));
            assert_eq!(snap.stashes.len(), 1);
            let state = snap.state.unwrap();
            assert_eq!(state.merge_heads.len(), 1);
            assert!(!state.cherry_pick);
            assert_eq!(
                snap.history.unwrap().commits[0].id.as_str(),
                std::str::from_utf8(&head).unwrap().trim()
            );
        },
    );
}

#[test]
fn history_reuses_repository_setup_and_retains_pinned_tips() {
    common::isolated(
        "ROCKET_UI_TEST_CASE",
        "history_reuses_repository_setup_and_retains_pinned_tips",
        || {
            let f = Fixture::new();
            f.write("a", "first\n");
            f.commit("first");
            f.write("a", "second\n");
            f.commit("second");
            let logs = f.root.join("history-logs");
            let backend = rocket_ui::backend::Backend::discover(&logs).unwrap();
            let token = CancellationToken::new();
            let snapshot = backend
                .snapshot(&f.repository, HistoryStart::Head, &token)
                .unwrap();
            let pinned = snapshot.history.unwrap();
            f.write("a", "external\n");
            f.commit("external");
            let before = fs::read_to_string(logs.join("rocket.jsonl")).unwrap();
            let page = backend
                .history(&snapshot.repository, pinned.start(), &token)
                .unwrap();
            let after = fs::read_to_string(logs.join("rocket.jsonl")).unwrap();
            assert_eq!(
                page.commits.iter().map(|c| &c.id).collect::<Vec<_>>(),
                pinned.commits.iter().map(|c| &c.id).collect::<Vec<_>>()
            );
            assert_eq!(
                after[before.len()..]
                    .lines()
                    .filter(|line| line.contains("\"event\":\"git_start\""))
                    .count(),
                2
            );
            let fresh = backend
                .history(&snapshot.repository, HistoryStart::Head, &token)
                .unwrap();
            assert_ne!(fresh.commits[0].id, page.commits[0].id);
            assert_eq!(fresh.commits.len(), 3);
            let cancel = CancellationToken::new();
            cancel.cancel();
            assert!(matches!(
                backend.history(&snapshot.repository, pinned.start(), &cancel),
                Err(Error::Cancelled)
            ));
        },
    );
}

#[test]
fn native_watch_is_idle_without_writes() {
    let f = Fixture::new();
    f.write("a", "a");
    f.commit("a");
    let info = f.repo().info().clone();
    let watch =
        thread::spawn(move || RepositoryWatch::new(&info, std::sync::Arc::new(|| {})).unwrap())
            .join()
            .unwrap();
    thread::sleep(Duration::from_millis(300));
    assert!(
        !watch.take_changed().unwrap(),
        "watch changed without a write"
    );
    f.repo().status(&CancellationToken::new()).unwrap();
    thread::sleep(Duration::from_millis(300));
    assert!(
        !watch.take_changed().unwrap(),
        "read-only status generated a watch change"
    );
    let repo = f.repo();
    let token = CancellationToken::new();
    repo.branches(&token).unwrap();
    assert!(!watch.take_changed().unwrap(), "branches changed files");
    repo.remotes(&token).unwrap();
    assert!(!watch.take_changed().unwrap(), "remotes changed files");
    repo.stashes(&token).unwrap();
    assert!(!watch.take_changed().unwrap(), "stashes changed files");
    repo.operation_state(&token).unwrap();
    assert!(
        !watch.take_changed().unwrap(),
        "operation_state changed files"
    );
    repo.history(HistoryStart::Head, 200, &token).unwrap();
    assert!(!watch.take_changed().unwrap(), "history changed files");
    let ctx = eframe::egui::Context::default();
    let backend = rocket_ui::backend::Backend::discover(&f.root.join("watch-logs")).unwrap();
    let snap = backend
        .snapshot_observed(
            &f.repository,
            HistoryStart::Head,
            &token,
            Some(std::sync::Arc::new(move || ctx.request_repaint())),
            false,
        )
        .unwrap();
    thread::sleep(Duration::from_millis(300));
    assert!(
        !snap.watcher.as_ref().unwrap().take_changed().unwrap(),
        "snapshot generated notifications"
    );
    backend
        .diff(
            &snap.repository,
            &DiffOptions {
                target: DiffTarget::Commit {
                    id: snap.history.as_ref().unwrap().commits[0].id.clone(),
                },
                ..Default::default()
            },
            &token,
        )
        .unwrap();
    thread::sleep(Duration::from_millis(300));
    assert!(
        !snap.watcher.as_ref().unwrap().take_changed().unwrap(),
        "commit diff generated notifications"
    );
}

#[track_caller]
fn until(app: &mut Rocket, ctx: &eframe::egui::Context, predicate: impl Fn(&Rocket) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        ctx.run_ui(eframe::egui::RawInput::default(), |_| {})
            .drop_without_applying_deltas();
        app.poll(ctx);
        if predicate(app) {
            return;
        }
        if Instant::now() >= deadline {
            let logs = fs::read_to_string(app.log_path.join("rocket.jsonl")).unwrap_or_default();
            let evidence = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "../../work/refresh-failure-{}.log",
                std::process::id()
            ));
            fs::write(evidence, &logs).unwrap();
            panic!(
                "controller timeout: state_error={:?} startup={:?} save={} tabs={:?}",
                app.state_error,
                app.startup_error,
                app.state_pending(),
                app.tabs
                    .iter()
                    .map(|t| (
                        t.refresh_count,
                        t.refresh.is_some(),
                        t.refresh_pending.is_some(),
                        t.diff.is_some(),
                        t.error.clone()
                    ))
                    .collect::<Vec<_>>()
            );
        }
        thread::sleep(Duration::from_millis(10));
    }
}
#[track_caller]
fn settled(app: &mut Rocket, ctx: &eframe::egui::Context) {
    until(app, ctx, |a| !a.busy());
}
fn patch_contains(app: &Rocket, text: &str) -> bool {
    app.tab().and_then(|t| t.patch.as_ref()).is_some_and(|p| {
        p.files
            .iter()
            .any(|f| f.lines.iter().any(|l| l.text == text))
    })
}

fn last_auto_refresh_commands(app: &Rocket) -> usize {
    *auto_refresh_command_counts(app).last().expect("completed automatic refresh")
}
fn auto_refresh_command_counts(app: &Rocket) -> Vec<usize> {
    let records: Vec<serde_json::Value> = fs::read_to_string(app.log_path.join("rocket.jsonl"))
        .unwrap().lines().map(|line| serde_json::from_str(line).unwrap()).collect();
    assert!(records.iter().all(|r| r["dropped_before"] == 0));
    records.iter().filter(|r| r["event"] == "request_end" && r["operation"] == "auto_refresh").map(|end| {
        let request = &end["request"];
        assert!(request.as_u64().is_some_and(|id| id != 0));
        records.iter().filter(|r| r["event"] == "git_start" && &r["request"] == request).count()
    }).collect()
}

#[test]
fn network_followup_reuses_rules_and_detects_external_changes() {
    common::isolated("ROCKET_UI_TEST_CASE", "network_followup_reuses_rules_and_detects_external_changes", || {
        use rocket_ui::{app::ActiveWrite, backend::Outcome};
        use std::sync::{Arc, mpsc};
        let f = Fixture::new();
        f.write("文.rs", "base\n"); f.commit("base T-1");
        let include = f.root.join("links.config");
        let rules = |suffix| format!("[rocket.issuelink.test]\npattern = T-[0-9]+\nurl = https://example.invalid/{suffix}/{{id}}\n");
        fs::write(&include, rules("before")).unwrap();
        f.run(&["config", "include.path", include.to_str().unwrap()]);
        let ctx = eframe::egui::Context::default();
        let mut app = Rocket::with_log_directory(&ctx, vec![f.repository.clone()], f.root.join("logs"));
        settled(&mut app, &ctx);
        let records = |app: &Rocket| -> Vec<serde_json::Value> {
            fs::read_to_string(app.log_path.join("rocket.jsonl")).unwrap().lines()
                .map(|line| serde_json::from_str(line).unwrap()).collect()
        };
        let requests = |app: &Rocket| records(app).iter().filter(|r|
            r["event"] == "request_start" && r["operation"] == "issue-links-read").count();
        let first = records(&app);
        let request = &first.iter().find(|r| r["event"] == "request_start" && r["operation"] == "issue-links-read").unwrap()["request"];
        assert_eq!(first.iter().filter(|r| r["event"] == "git_start" && &r["request"] == request).count(), 2,
            "display rules need only their two config reads, not client/version/repository discovery");
        let original = app.tabs[0].issue_links.links.clone();
        assert!(original.find("T-1")[0].url.contains("/before/"));
        let index = fs::read(f.repository.join(".git/index")).unwrap();
        let head = f.repo().head(&CancellationToken::new()).unwrap();
        app.tabs[0].draft = "KEEP 文 draft".into();
        for label in ["fetch", "pull-fetch", "push"] {
            let before = requests(&app);
            // Real metadata/worktree edits while the controller waits for a
            // completed write. Native MCP covers the actual network cancellation.
            f.run(&["branch", &format!("external-{label}")]);
            f.write("文.rs", format!("external {label}\n"));
            app.write = Some(ActiveWrite {
                owner: Some(app.tabs[0].id), label: label.into(), title: "Synthetic cancelled network write".into(),
                task: WriteTask::spawn(|_| Ok(Outcome { summary: "Cancelled fixture write".into(), ..Default::default() })).unwrap(),
                fetch_progress: mpsc::channel().1, clone_progress: mpsc::channel().1,
                force_push_progress: mpsc::channel().1, force_push_phase: None,
                progress: String::new(), fraction: None, cancelling: true,
                started: Instant::now(), submitted_draft: None,
            });
            until(&mut app, &ctx, |a| a.write.is_none());
            assert!(app.tabs[0].background_refresh);
            assert!(!app.tabs[0].loading(), "background reconciliation must not block navigation");
            settled(&mut app, &ctx);
            let snapshot = app.tabs[0].snapshot.as_ref().unwrap();
            assert!(snapshot.branches.iter().any(|b| b.reference == format!("refs/heads/external-{label}").as_bytes()));
            assert_eq!(snapshot.status.as_ref().unwrap().entries.len(), 1);
            assert_eq!(requests(&app), before, "unchanged effective config must not launch another rule read");
            assert!(Arc::ptr_eq(&original, &app.tabs[0].issue_links.links));
        }
        // Only the included file changes; the repository config's length/mtime
        // cannot detect it. A worktree notification must refresh the effective rules.
        let before = app.tabs[0].refresh_count;
        fs::write(&include, rules("after")).unwrap();
        f.write("文.rs", "included-rule change\n");
        until(&mut app, &ctx, |a| a.tabs[0].refresh_count > before && !a.busy());
        assert!(app.tabs[0].issue_links.links.find("T-1")[0].url.contains("/after/"));
        let reads = requests(&app);
        app.refresh(app.tabs[0].id); settled(&mut app, &ctx);
        assert_eq!(requests(&app), reads + 1, "explicit refresh still forces a fresh rule read");
        assert_eq!(app.tabs[0].draft, "KEEP 文 draft");
        assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
        assert_eq!(f.repo().head(&CancellationToken::new()).unwrap(), head);
        assert!(app.tabs[0].error.is_none());
    });
}

#[test]
fn real_fetch_and_pull_reconcile_once_and_publish_external_changes() {
    common::isolated("ROCKET_UI_TEST_CASE", "real_fetch_and_pull_reconcile_once_and_publish_external_changes", || {
        use rocket_ui::backend::Action;
        let f = Fixture::new();
        f.write("文.rs", "base\n"); f.commit("base");
        let remote = f.root.join("remote.git");
        let peer = f.root.join("peer");
        f.run(&["clone", "--bare", ".", remote.to_str().unwrap()]);
        f.run(&["clone", remote.to_str().unwrap(), peer.to_str().unwrap()]);
        f.run_at(&peer, &["config", "user.name", "Synthetic"]);
        f.run_at(&peer, &["config", "user.email", "fixture@example.invalid"]);
        f.run(&["remote", "add", "origin", remote.to_str().unwrap()]);
        f.run(&["fetch", "origin"]);
        f.run(&["branch", "--set-upstream-to", "origin/main", "main"]);
        let ctx = eframe::egui::Context::default();
        let mut app = Rocket::with_log_directory(&ctx, vec![f.repository.clone()], f.root.join("logs"));
        settled(&mut app, &ctx);
        let head = f.run(&["rev-parse", "HEAD"]);
        let index = fs::read(f.repository.join(".git/index")).unwrap();
        app.tabs[0].draft = "KEEP draft 文".into();
        for (n, action) in [Action::Fetch(b"origin".to_vec()), Action::PullFetch].into_iter().enumerate() {
            fs::write(peer.join("文.rs"), format!("incoming {n}\n")).unwrap();
            f.run_at(&peer, &["add", "--all"]);
            f.run_at(&peer, &["commit", "-m", "incoming"]);
            f.run_at(&peer, &["push", "origin", "main"]);
            let tip = f.run_at(&peer, &["rev-parse", "HEAD"]);
            let counts = auto_refresh_command_counts(&app).len();
            app.dispatch(Intent::Execute(app.tabs[0].id, action));
            assert!(app.write.is_some());
            settled(&mut app, &ctx);
            let refreshes = auto_refresh_command_counts(&app);
            assert_eq!(&refreshes[counts..], &[10], "exactly one complete post-fetch snapshot");
            let log = fs::read_to_string(app.log_path.join("rocket.jsonl")).unwrap();
            let records: Vec<serde_json::Value> = log.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
            assert!(!records.iter().any(|r| r["event"] == "step_start" && r["operation"] == "refresh"
                && matches!(r["step"].as_str(), Some("status" | "branches" | "operation"))),
                "the UI must not discard standalone reconciliation reads then repeat them in the snapshot");
            let snapshot = app.tabs[0].snapshot.as_ref().unwrap();
            assert!(snapshot.warnings.is_empty(), "{:?}", snapshot.warnings);
            assert!(snapshot.branches.iter().any(|b| b.reference == b"refs/remotes/origin/main"
                && b.target.as_str().as_bytes() == tip.strip_suffix(b"\n").unwrap()));
            assert_eq!(snapshot.status.as_ref().unwrap().ahead_behind, Some((0, (n + 1) as u64)));
            assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
            assert_eq!(f.run(&["rev-parse", "HEAD"]), head);
            assert_eq!(app.tabs[0].draft, "KEEP draft 文");
        }
        let before = app.tabs[0].refresh_count;
        f.run(&["branch", "external-分支"]);
        f.write("文.rs", "external edit after pull\n");
        until(&mut app, &ctx, |a| a.tabs[0].refresh_count > before && !a.busy());
        let snapshot = app.tabs[0].snapshot.as_ref().unwrap();
        assert!(snapshot.branches.iter().any(|b| b.reference == "refs/heads/external-分支".as_bytes()));
        assert!(snapshot.status.as_ref().unwrap().entries.iter().any(|e| e.path.as_bytes() == "文.rs".as_bytes()));
        assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
        assert_eq!(f.run(&["rev-parse", "HEAD"]), head);
    });
}

#[test]
fn fetched_snapshot_handoff_does_not_overwrite_a_changed_history_view() {
    common::isolated("ROCKET_UI_TEST_CASE", "fetched_snapshot_handoff_does_not_overwrite_a_changed_history_view", || {
        use rocket_ui::backend::Action;
        let f = Fixture::new(); f.write("a", "base\n"); f.commit("base");
        let remote = f.root.join("remote.git");
        f.run(&["clone", "--bare", ".", remote.to_str().unwrap()]);
        f.run(&["remote", "add", "origin", remote.to_str().unwrap()]);
        let ctx = eframe::egui::Context::default();
        let mut app = Rocket::with_log_directory(&ctx, vec![f.repository.clone()], f.root.join("logs"));
        settled(&mut app, &ctx);
        app.dispatch(Intent::Execute(app.tabs[0].id, Action::Fetch(b"origin".to_vec())));
        // Hold the completed message, as if the event loop was busy. Changes in
        // this interval must not be overwritten by the earlier view's snapshot.
        let deadline = Instant::now() + Duration::from_secs(30);
        let outcome = loop {
            if let Some(result) = app.write.as_mut().unwrap().task.try_finish() { break result.unwrap(); }
            assert!(Instant::now() < deadline); thread::sleep(Duration::from_millis(10));
        };
        assert!(outcome.command_succeeded);
        app.write.as_mut().unwrap().task = WriteTask::spawn(move |_| Ok(outcome)).unwrap();
        f.run(&["checkout", "-b", "external-view"]);
        f.write("外部.txt", "new branch\n"); f.commit("external commit");
        f.run(&["checkout", "main"]);
        app.dispatch(Intent::AllBranches);
        settled(&mut app, &ctx);
        assert!(app.tabs[0].history_all);
        assert_eq!(app.tabs[0].commits().len(), 2);
        assert!(app.tabs[0].snapshot.as_ref().unwrap().branches.iter().any(|b| b.reference == b"refs/heads/external-view"));
        assert!(app.tabs[0].error.is_none());
    });
}

#[test]
fn fetched_snapshot_handoff_rearms_a_newly_enabled_watcher() {
    common::isolated("ROCKET_UI_TEST_CASE", "fetched_snapshot_handoff_rearms_a_newly_enabled_watcher", || {
        use rocket_ui::backend::Action;
        let f = Fixture::new(); f.write("文.rs", "base\n"); f.commit("base");
        let remote = f.root.join("remote.git");
        f.run(&["clone", "--bare", ".", remote.to_str().unwrap()]);
        f.run(&["remote", "add", "origin", remote.to_str().unwrap()]);
        let ctx = eframe::egui::Context::default();
        let mut app = Rocket::with_log_directory(&ctx, vec![f.repository.clone()], f.root.join("logs"));
        settled(&mut app, &ctx); app.auto_refresh = false;
        app.dispatch(Intent::Execute(app.tabs[0].id, Action::Fetch(b"origin".to_vec())));
        let deadline = Instant::now() + Duration::from_secs(30);
        let outcome = loop {
            if let Some(result) = app.write.as_mut().unwrap().task.try_finish() { break result.unwrap(); }
            assert!(Instant::now() < deadline); thread::sleep(Duration::from_millis(10));
        };
        assert!(outcome.command_succeeded);
        app.write.as_mut().unwrap().task = WriteTask::spawn(move |_| Ok(outcome)).unwrap();
        app.auto_refresh = true;
        settled(&mut app, &ctx);
        assert!(app.tabs[0].snapshot.as_ref().unwrap().watcher.is_some());
        let before = app.tabs[0].refresh_count;
        f.write("文.rs", "external after enabling refresh\n");
        until(&mut app, &ctx, |a| a.tabs[0].refresh_count > before && !a.busy());
        assert_eq!(app.tabs[0].snapshot.as_ref().unwrap().status.as_ref().unwrap().entries.len(), 1);
    });
}

#[test]
fn worktree_refresh_reuses_history_but_reads_changes_and_selected_diff_fresh() {
    common::isolated("ROCKET_UI_TEST_CASE", "worktree_refresh_reuses_history_but_reads_changes_and_selected_diff_fresh", || {
        let f = Fixture::new();
        f.write("a.rs", "base\n");
        f.commit("base");
        let ctx = eframe::egui::Context::default();
        let mut app = Rocket::with_log_directory(&ctx, vec![f.repository.clone()], f.root.join("logs"));
        settled(&mut app, &ctx);
        let history = app.tabs[0].commits().as_ptr();
        let index = fs::read(f.repository.join(".git/index")).unwrap();
        app.tabs[0].draft = "KEEP draft 文".into();
        for value in ["first", "other"] {
            let count = app.tabs[0].refresh_count;
            f.write("a.rs", format!("{value}\n"));
            until(&mut app, &ctx, |a| a.tabs[0].refresh_count > count && !a.busy());
            let commands = last_auto_refresh_commands(&app);
            println!("worktree_refresh_git_commands={commands}");
            assert_eq!(commands, 4, "one discovery, two configuration guards and fresh status");
            assert_eq!(app.tabs[0].commits().as_ptr(), history);
            assert_eq!(app.tabs[0].draft, "KEEP draft 文");
            app.dispatch(Intent::View(View::Changes));
            let change = app.tabs[0].snapshot.as_ref().unwrap().status.as_ref().unwrap().entries[0].clone();
            app.dispatch(Intent::SelectChange(change, false));
            settled(&mut app, &ctx);
            assert!(patch_contains(&app, &format!("+{value}")));
        }
        assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
    });
}

#[test]
fn native_watch_classifies_linked_metadata_and_rejects_replaced_directories() {
    common::isolated("ROCKET_UI_TEST_CASE", "native_watch_classifies_linked_metadata_and_rejects_replaced_directories", || {
        let f = Fixture::new(); f.write("a", "base"); f.commit("base");
        let linked = f.root.join("linked");
        f.run(&["worktree", "add", "-b", "linked", linked.to_str().unwrap()]);
        let repo = f.git.open(&linked, &CancellationToken::new()).unwrap();
        for (path, metadata) in [(linked.join("a"), false), (repo.info().git_dir.join("refresh-probe"), true), (repo.info().common_dir.join("refresh-probe"), true)] {
            let watch = RepositoryWatch::new(repo.info(), std::sync::Arc::new(|| {})).unwrap();
            assert!(watch.is_current().unwrap());
            fs::write(path, "changed").unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while !watch.take_changed().unwrap() {
                assert!(Instant::now() < deadline, "native notification missing");
                thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(watch.metadata_generation().unwrap() > 0, metadata);
        }
        let watch = RepositoryWatch::new(repo.info(), std::sync::Arc::new(|| {})).unwrap();
        fs::rename(&linked, f.root.join("retained-linked")).unwrap();
        fs::create_dir(&linked).unwrap();
        assert!(!watch.is_current().unwrap(), "an identical path is not the same watched directory");
    });
}

#[test]
fn metadata_and_included_configuration_force_complete_refreshes() {
    common::isolated("ROCKET_UI_TEST_CASE", "metadata_and_included_configuration_force_complete_refreshes", || {
        let f = Fixture::new(); f.write("a", "base\n"); f.commit("base");
        let include = f.root.join("shared.config");
        fs::write(&include, "[remote \"before\"]\nurl = ../local-only\n").unwrap();
        f.run(&["config", "include.path", include.to_str().unwrap()]);
        let ctx = eframe::egui::Context::default();
        let mut app = Rocket::with_log_directory(&ctx, vec![f.repository.clone()], f.root.join("logs"));
        settled(&mut app, &ctx);
        app.tabs[0].draft = "KEEP draft".into();
        let count = app.tabs[0].refresh_count;
        // This included file is outside every watched root. A worktree event
        // must compare effective configuration before reusing the remote list.
        fs::write(&include, "[remote \"after\"]\nurl = ../PRIVATE-local-only\n").unwrap();
        f.write("a", "included\n");
        until(&mut app, &ctx, |a| a.tabs[0].refresh_count > count && !a.busy());
        assert!(app.tabs[0].snapshot.as_ref().unwrap().remotes.iter().any(|r| r.name == b"after"));
        // Full refreshes reuse the guard's config for remotes and need 10 reads.
        assert!(last_auto_refresh_commands(&app) >= 10, "configuration changes require every section");
        f.run(&["branch", "external"]); f.run(&["pack-refs", "--all", "--prune"]);
        f.run(&["stash", "push", "-m", "synthetic stash"]);
        fs::write(f.repository.join(".git/MERGE_HEAD"), f.run(&["rev-parse", "HEAD"])).unwrap();
        f.write("a", "external state\n");
        until(&mut app, &ctx, |a| !a.busy() && a.tabs[0].snapshot.as_ref().is_some_and(|s|
            s.branches.iter().any(|b| b.reference == b"refs/heads/external") && s.stashes.len() == 1
            && s.state.as_ref().is_some_and(|s| s.merge_heads.len() == 1)));
        assert!(last_auto_refresh_commands(&app) >= 10);
        fs::remove_file(f.repository.join(".git/MERGE_HEAD")).unwrap();
        f.commit("external new HEAD");
        let expected = f.repo().head(&CancellationToken::new()).unwrap();
        let index = fs::read(f.repository.join(".git/index")).unwrap();
        until(&mut app, &ctx, |a| !a.busy() && a.tabs[0].snapshot.as_ref().unwrap().head.as_ref() == Some(&expected));
        assert_eq!(app.tabs[0].commits()[0].subject_lossy(), "external new HEAD");
        assert!(app.tabs[0].snapshot.as_ref().unwrap().state.as_ref().unwrap().merge_heads.is_empty());
        // An explicit refresh must cancel an in-flight automatic read and run
        // the complete path; it must not be swallowed by the busy-read guard.
        f.write("a", "manual refresh\n");
        until(&mut app, &ctx, |a| a.tabs[0].refresh.is_some());
        app.dispatch(Intent::Refresh);
        settled(&mut app, &ctx);
        assert!(!app.tabs[0].background_refresh);
        assert_eq!(app.tabs[0].draft, "KEEP draft");
        assert!(app.tabs[0].snapshot.as_ref().unwrap().warnings.is_empty());
        fs::write(&include, "[broken\n").unwrap();
        f.write("a", "invalid config\n");
        until(&mut app, &ctx, |a| !a.busy() && a.tabs[0].error.is_some());
        fs::write(&include, "[remote \"recovered\"]\nurl = ../local-only\n").unwrap();
        f.write("a", "recovered config\n");
        until(&mut app, &ctx, |a| !a.busy() && a.tabs[0].error.is_none() &&
            a.tabs[0].snapshot.as_ref().unwrap().remotes.iter().any(|r| r.name == b"recovered"));
        assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
        assert!(!fs::read_to_string(app.log_path.join("rocket.jsonl")).unwrap().contains("PRIVATE"));
    });
}

#[test]
fn late_metadata_change_requires_full_refresh_before_reuse() {
    common::isolated("ROCKET_UI_TEST_CASE", "late_metadata_change_requires_full_refresh_before_reuse", || {
        let f = Fixture::new(); f.write("a", "base"); f.commit("base");
        let index = fs::read(f.repository.join(".git/index")).unwrap();
        let ctx = eframe::egui::Context::default();
        let mut app = Rocket::with_log_directory(&ctx, vec![f.repository.clone()], f.root.join("logs"));
        settled(&mut app, &ctx);
        app.tabs[0].draft = "KEEP late draft".into();
        let initial_requests = auto_refresh_command_counts(&app).len();
        f.write("a", "automatic read");
        until(&mut app, &ctx, |a| a.tabs[0].refresh.is_some());
        let watch = app.tabs[0].snapshot.as_ref().unwrap().watcher.as_ref().unwrap().clone();
        let generation = watch.metadata_generation();
        f.run(&["branch", "late-branch"]);
        let deadline = Instant::now() + Duration::from_secs(5);
        while watch.metadata_generation() == generation {
            assert!(Instant::now() < deadline, "metadata notification missing");
            thread::sleep(Duration::from_millis(10));
        }
        until(&mut app, &ctx, |a| !a.busy() && a.tabs[0].snapshot.as_ref().unwrap()
            .branches.iter().any(|b| b.reference == b"refs/heads/late-branch"));
        let commands = auto_refresh_command_counts(&app);
        assert!(commands[initial_requests..].iter().any(|count| *count >= 10),
            "metadata requires a complete read; later coalesced worktree events may use the fast path: {commands:?}");
        assert_eq!(app.tabs[0].draft, "KEEP late draft");
        assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
        assert!(app.tabs[0].error.is_none());
    });
}

#[test]
fn native_refresh_preserves_selection_drafts_and_detects_repeated_edits() {
    common::isolated(
        "ROCKET_UI_TEST_CASE",
        "native_refresh_preserves_selection_drafts_and_detects_repeated_edits",
        || {
            let f = Fixture::new();
            f.write("a.rs", "base\n");
            f.commit("base");
            let ctx = eframe::egui::Context::default();
            let mut app =
                Rocket::with_log_directory(&ctx, vec![f.repository.clone()], f.root.join("logs"));
            settled(&mut app, &ctx);
            assert!(
                app.tab()
                    .unwrap()
                    .snapshot
                    .as_ref()
                    .unwrap()
                    .watcher
                    .is_some()
            );
            let selected = app.tab().unwrap().selected_commit.clone();
            let before_head = f.repo().head(&CancellationToken::new()).unwrap();
            let before_index = fs::read(f.repository.join(".git/index")).unwrap();
            app.tabs[0].draft = "PRIVATE saved draft 文\nsecond line".into();
            app.tabs[0].search = "base".into();
            app.dispatch(Intent::Search);
            for i in 0..20 {
                f.write("a.rs", format!("burst {i}\n"));
            }
            until(&mut app, &ctx, |a| {
                a.tab()
                    .unwrap()
                    .snapshot
                    .as_ref()
                    .unwrap()
                    .status
                    .as_ref()
                    .unwrap()
                    .entries
                    .len()
                    == 1
                    && !a.busy()
            });
            assert_eq!(app.tab().unwrap().selected_commit, selected);
            assert_eq!(app.tab().unwrap().search, "base");
            assert_eq!(
                app.tab().unwrap().draft,
                "PRIVATE saved draft 文\nsecond line"
            );
            app.dispatch(Intent::View(View::Changes));
            let entry = app
                .tab()
                .unwrap()
                .snapshot
                .as_ref()
                .unwrap()
                .status
                .as_ref()
                .unwrap()
                .entries[0]
                .clone();
            app.dispatch(Intent::SelectChange(entry, false));
            settled(&mut app, &ctx);
            assert!(patch_contains(&app, "+burst 19"));
            f.write("a.rs", "second edit\n");
            until(&mut app, &ctx, |a| {
                patch_contains(a, "+second edit") && !a.busy()
            });
            // A change while a snapshot is reading must not be consumed by that snapshot.
            f.write("new.txt", "new\n");
            until(&mut app, &ctx, |a| a.tab().unwrap().refresh.is_some());
            app.tabs[0].draft = "PRIVATE newest draft".into();
            f.write("a.rs", "during refresh\n");
            until(&mut app, &ctx, |a| {
                patch_contains(a, "+during refresh") && !a.busy()
            });
            assert_eq!(app.tab().unwrap().draft, "PRIVATE newest draft");
            assert_eq!(
                f.repo().head(&CancellationToken::new()).unwrap(),
                before_head
            );
            assert_eq!(
                fs::read(f.repository.join(".git/index")).unwrap(),
                before_index
            );
            let count = app.tab().unwrap().refresh_count;
            let idle = Instant::now() + Duration::from_secs(2);
            while Instant::now() < idle {
                app.poll(&ctx);
                thread::sleep(Duration::from_millis(20));
            }
            assert_eq!(
                app.tab().unwrap().refresh_count,
                count,
                "no idle refresh loop"
            );
            let log = fs::read_to_string(f.root.join("logs/rocket.jsonl")).unwrap();
            assert!(!log.contains("PRIVATE"));
            assert!(!log.contains("a.rs"));
            assert!(log.contains("\"operation\":\"auto_refresh\""));
        },
    );
}

#[test]
fn drafts_preferences_and_recent_repositories_survive_restart_and_close() {
    common::isolated(
        "ROCKET_UI_TEST_CASE",
        "drafts_preferences_and_recent_repositories_survive_restart_and_close",
        || {
            let f = Fixture::new();
            f.write("a", "base");
            f.commit("base");
            let ctx = eframe::egui::Context::default();
            let state = f.root.join("saved");
            let mut app = Rocket::with_storage(
                &ctx,
                vec![f.repository.clone()],
                f.root.join("logs1"),
                state.clone(),
            );
            settled(&mut app, &ctx);
            let draft = format!("PRIVATE recovered 文\n\n{}", "Details 文\n".repeat(18_000));
            assert!(draft.len() > 128 * 1024);
            app.tabs[0].draft = draft.clone();
            app.show_sidebar = false;
            app.row_height = 29.0;
            app.auto_refresh = false;
            ctx.set_zoom_factor(1.2);
            app.poll(&ctx);
            settled(&mut app, &ctx);
            app.open(f.repository.join("."));
            settled(&mut app, &ctx);
            assert_eq!(
                app.tabs.len(),
                1,
                "canonical repository aliases share one draft"
            );
            assert_eq!(app.tab().unwrap().draft, draft);
            let id = app.tab().unwrap().id;
            app.remove_tab(id);
            app.poll(&ctx);
            settled(&mut app, &ctx);
            drop(app);
            let ctx = eframe::egui::Context::default();
            let mut app = Rocket::with_storage(
                &ctx,
                vec![f.repository.clone()],
                f.root.join("logs2"),
                state.clone(),
            );
            settled(&mut app, &ctx);
            assert_eq!(app.tab().unwrap().draft, draft);
            assert!(!app.show_sidebar);
            assert!(!app.auto_refresh);
            assert_eq!(app.row_height, 29.0);
            assert_eq!(ctx.zoom_factor(), 1.2);
            assert_eq!(app.recent.len(), 1);
            app.tabs[0].draft.clear();
            app.begin_quit();
            settled(&mut app, &ctx);
            drop(app);
            let (_, saved) = Store::open(&state).unwrap();
            assert!(saved.drafts.is_empty());
        },
    );
}

#[test]
fn linked_worktree_observes_shared_refs_and_writes_supersede_auto_refresh() {
    common::isolated(
        "ROCKET_UI_TEST_CASE",
        "linked_worktree_observes_shared_refs_and_writes_supersede_auto_refresh",
        || {
            let f = Fixture::new();
            f.write("a", "base");
            f.commit("base");
            let linked = f.root.join("linked");
            f.run(&["worktree", "add", "-b", "linked", linked.to_str().unwrap()]);
            let ctx = eframe::egui::Context::default();
            let mut app =
                Rocket::with_log_directory(&ctx, vec![linked.clone()], f.root.join("logs"));
            settled(&mut app, &ctx);
            f.run(&["branch", "external"]);
            until(&mut app, &ctx, |a| {
                a.tab()
                    .unwrap()
                    .snapshot
                    .as_ref()
                    .unwrap()
                    .branches
                    .iter()
                    .any(|b| b.reference == b"refs/heads/external")
                    && !a.busy()
            });
            fs::write(linked.join("a"), "local").unwrap();
            until(&mut app, &ctx, |a| a.tab().unwrap().refresh.is_some());
            let id = app.tab().unwrap().id;
            assert!(app.can_write());
            app.execute(id, rocket_ui::backend::Action::StageAll);
            settled(&mut app, &ctx);
            assert!(app.activities.last().unwrap().outcome.command_succeeded);
            assert_eq!(
                app.tab()
                    .unwrap()
                    .snapshot
                    .as_ref()
                    .unwrap()
                    .status
                    .as_ref()
                    .unwrap()
                    .entries[0]
                    .index,
                Change::Modified
            );
        },
    );
}

#[test]
fn opening_repository_beyond_view_limit_keeps_saving_and_preserves_drafts() {
    common::isolated(
        "ROCKET_UI_TEST_CASE",
        "opening_repository_beyond_view_limit_keeps_saving_and_preserves_drafts",
        || {
            let f = Fixture::new();
            f.write("a", "base");
            f.commit("base");
            let directory = f.root.join("saved");
            let recent = f.root.join("past-repo-0");
            let old_draft = f.root.join("old-draft-repository");
            {
                let (store, mut state) = Store::open(&directory).unwrap();
                state.preferences.auto_refresh = false;
                state.preferences.dependency_warnings = false;
                for i in 0..WORKTREE_VIEWS {
                    state.worktree_views.insert(f.root.join(format!("past-repo-{i}")), Default::default());
                }
                state.recent.push(recent.clone());
                state.drafts.insert(old_draft.clone(), "PRIVATE historical draft".into());
                for i in 0..64 {
                    state.drafts.insert(f.root.join(format!("retained-draft-{i}")), format!("draft {i}").into());
                }
                store.save(&state).unwrap();
            }
            let ctx = eframe::egui::Context::default();
            let mut app = Rocket::with_storage(&ctx, vec![f.repository.clone()], f.root.join("logs"), directory.clone());
            settled(&mut app, &ctx);
            assert!(app.state_error.is_none(), "{:?}", app.state_error);
            let current = app.tabs[0].path.clone();
            app.tabs[0].branch_filter = "retained filter".into();
            app.tabs[0].draft = "PRIVATE current draft".into();
            app.show_sidebar = false;
            app.poll(&ctx);
            settled(&mut app, &ctx);
            let id = app.tabs[0].id;
            app.remove_tab(id);
            app.poll(&ctx);
            settled(&mut app, &ctx);
            assert!(app.state_error.is_none(), "later changes must remain saveable");
            drop(app);
            {
                let (_, state) = Store::open(&directory).unwrap();
                assert_eq!(state.worktree_views.len(), WORKTREE_VIEWS);
                assert_eq!(state.worktree_views[&current].branch_filter, "retained filter");
                assert!(state.worktree_views.contains_key(&recent));
                assert_eq!(state.drafts[&current].as_ref(), "PRIVATE current draft");
                assert_eq!(state.drafts[&old_draft].as_ref(), "PRIVATE historical draft");
                assert_eq!(state.drafts.len(), 66);
                for i in 0..64 {
                    assert_eq!(state.drafts[&f.root.join(format!("retained-draft-{i}"))].as_ref(), format!("draft {i}"));
                }
                assert!(!state.preferences.sidebar);
            }
            let mut reopened = Rocket::with_storage(&ctx, vec![f.repository.clone()], f.root.join("logs2"), directory);
            settled(&mut reopened, &ctx);
            assert!(reopened.state_error.is_none());
            assert_eq!(reopened.tabs[0].branch_filter, "retained filter");
            assert_eq!(reopened.tabs[0].draft, "PRIVATE current draft");
            assert!(!reopened.show_sidebar);
        },
    );
}

#[test]
fn save_failure_cancels_exit_and_preserves_previous_state() {
    common::isolated(
        "ROCKET_UI_TEST_CASE",
        "save_failure_cancels_exit_and_preserves_previous_state",
        || {
            use std::os::windows::fs::OpenOptionsExt;
            let f = Fixture::new();
            f.write("a", "base");
            f.commit("base");
            let ctx = eframe::egui::Context::default();
            let state = f.root.join("saved");
            let mut app = Rocket::with_storage(
                &ctx,
                vec![f.repository.clone()],
                f.root.join("logs"),
                state.clone(),
            );
            settled(&mut app, &ctx);
            let original = fs::read(state.join("state.bin")).unwrap();
            let held = fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(state.join("state.bin"))
                .unwrap();
            app.tabs[0].draft = "PRIVATE unsaved draft".into();
            app.begin_quit();
            until(&mut app, &ctx, |a| a.state_error.is_some() && !a.busy());
            assert!(!app.quitting, "a failed save must keep the window open");
            assert_eq!(app.tab().unwrap().draft, "PRIVATE unsaved draft");
            assert!(
                app.state_error
                    .as_ref()
                    .unwrap()
                    .contains("could not be saved")
            );
            drop(held);
            assert_eq!(fs::read(state.join("state.bin")).unwrap(), original);
            let log = fs::read_to_string(f.root.join("logs/rocket.jsonl")).unwrap();
            assert!(log.contains("\"operation\":\"save_state\""));
            assert!(log.contains("\"outcome\":\"io_error\""));
            assert!(!log.contains("PRIVATE"));
            assert!(app.retry_state_save());
            app.poll(&ctx);
            settled(&mut app, &ctx);
            assert!(app.state_error.is_none());
            assert!(!app.quitting, "retrying a save does not request exit");
            assert!(!app.retry_state_save(), "a successful save has nothing to retry");
            let saved_path = app.tabs[0].path.clone();
            drop(app);
            let (_, saved) = Store::open(&state).unwrap();
            assert_eq!(saved.drafts[&saved_path].as_ref(), "PRIVATE unsaved draft");
        },
    );
}

#[test]
fn a_failed_state_load_cannot_be_retried_as_a_write() {
    let f = Fixture::new();
    let directory = f.root.join("saved");
    fs::create_dir_all(&directory).unwrap();
    let unread = b"unsupported state that must be preserved";
    fs::write(directory.join("state.bin"), unread).unwrap();
    let ctx = eframe::egui::Context::default();
    let mut app = Rocket::with_storage(&ctx, vec![], f.root.join("logs"), directory.clone());
    settled(&mut app, &ctx);
    let error = app.state_error.clone();
    assert!(error.is_some());
    assert!(!app.retry_state_save());
    app.poll(&ctx);
    assert_eq!(app.state_error, error);
    assert_eq!(fs::read(directory.join("state.bin")).unwrap(), unread);
}

#[test]
fn state_replacement_corruption_and_writer_ownership_preserve_existing_data() {
    let f = Fixture::new();
    let dir = f.root.join("state");
    let (store, mut state) = Store::open(&dir).unwrap();
    state.recent.push(f.repository.clone());
    state.drafts.insert(f.repository.clone(), "first 文".into());
    store.save(&state).unwrap();
    assert!(
        Store::open(&dir).is_err(),
        "second writer must not overwrite"
    );
    let original = fs::read(dir.join("state.bin")).unwrap();
    // State v18 commits a pointer and an archive as one generation. A complete
    // backup includes both, rather than a pointer to an already retired archive.
    let original_drafts: Vec<_> = fs::read_dir(&dir).unwrap().map(|entry| entry.unwrap())
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("drafts-"))
        .map(|entry| (entry.file_name(), fs::read(entry.path()).unwrap())).collect();
    assert_eq!(original_drafts.len(), 1);
    {
        use std::os::windows::fs::OpenOptionsExt;
        let _held = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(dir.join("state.bin"))
            .unwrap();
        state.drafts.insert(f.repository.clone(), "second".into());
        assert!(store.save(&state).is_err());
    }
    assert_eq!(fs::read(dir.join("state.bin")).unwrap(), original);
    store.save(&state).unwrap();
    drop(store);
    let (store, recovered) = Store::open(&dir).unwrap();
    assert_eq!(state, recovered);
    drop(store);
    for truncated in [vec![], original[..10].to_vec(), vec![0; 200]] {
        fs::write(dir.join("state.bin"), &truncated).unwrap();
        assert!(Store::open(&dir).is_err());
        assert_eq!(fs::read(dir.join("state.bin")).unwrap(), truncated);
    }
    // A partial temporary file never replaces the last complete state.
    for (name, bytes) in original_drafts { fs::write(dir.join(name), bytes).unwrap(); }
    fs::write(dir.join("state.bin"), original).unwrap();
    fs::write(dir.join("state-abandoned.tmp"), "partial").unwrap();
    let (_, recovered) = Store::open(&dir).unwrap();
    assert_eq!(recovered.drafts[&f.repository].as_ref(), "first 文");
}

#[test]
fn bare_repository_snapshot_reads_without_a_stash_warning() {
    common::isolated(
        "ROCKET_UI_TEST_CASE",
        "bare_repository_snapshot_reads_without_a_stash_warning",
        || {
            let f = Fixture::new();
            f.write("a", "base\n");
            f.commit("base");
            let bare = f.root.canonicalize().unwrap().join("bare.git");
            f.run(&["clone", "--quiet", "--bare", ".", bare.to_str().unwrap().trim_start_matches(r"\\?\")]);
            let backend = rocket_ui::backend::Backend::discover(&f.root.join("bare-logs")).unwrap();
            let snap = backend
                .snapshot(&bare, HistoryStart::Head, &CancellationToken::new())
                .unwrap();
            // Stashes need a working folder, so a bare repository simply has none.
            assert!(snap.warnings.is_empty(), "{:?}", snap.warnings);
            assert!(snap.stashes.is_empty());
            assert_eq!(snap.history.unwrap().commits.len(), 1);
        },
    );
}
