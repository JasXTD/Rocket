#![cfg(windows)]
#[path = "../../rocket-git/tests/common/mod.rs"]
mod common;
use common::Fixture;
use rocket_ui::app::{DialogKind, Intent, Rocket};
use std::{
    thread,
    time::{Duration, Instant},
};

fn until(app: &mut Rocket, ctx: &eframe::egui::Context, predicate: impl Fn(&Rocket) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        ctx.run_ui(Default::default(), |_| {})
            .drop_without_applying_deltas();
        app.poll(ctx);
        if predicate(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "controller did not settle: {:?}",
            app.tab()
                .map(|t| (&t.error, &t.search_state.error, t.search_state.scanned))
        );
        thread::sleep(Duration::from_millis(10));
    }
}
fn settled(app: &mut Rocket, ctx: &eframe::egui::Context) {
    until(app, ctx, |a| !a.busy());
}
fn search(app: &mut Rocket, text: &str) {
    app.tabs[0].search = text.into();
    app.dispatch(Intent::Search);
}
fn orphan(f: &Fixture, message: &str) -> String {
    let output = f
        .command(
            &f.repository,
            &["commit-tree", "HEAD^{tree}", "-m", message],
        )
        .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z")
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().into()
}

#[test]
fn all_branch_refresh_detects_nonfirst_tip_and_restarts_search_without_losing_draft() {
    common::isolated(
        "ROCKET_UI_MANAGEMENT_CASE",
        "all_branch_refresh_detects_nonfirst_tip_and_restarts_search_without_losing_draft",
        || {
            let f = Fixture::new();
            f.write("file", "baseline");
            f.commit("main baseline");
            let main_id = f
                .repo()
                .resolve_commit("HEAD", &rocket_git::CancellationToken::new())
                .unwrap();
            let side = orphan(&f, "side needle old");
            f.run(&["update-ref", "refs/heads/side", &side]);
            let ctx = eframe::egui::Context::default();
            let mut app =
                Rocket::with_log_directory(&ctx, vec![f.repository.clone()], f.root.join("logs"));
            settled(&mut app, &ctx);
            assert_eq!(app.tab().unwrap().commits().len(), 1);
            app.dispatch(Intent::AllBranches);
            settled(&mut app, &ctx);
            let first = app.tab().unwrap().commits()[0].id.clone();
            assert_eq!(first, main_id);
            assert_eq!(app.tab().unwrap().commits().len(), 2);
            app.tabs[0].draft = "Retain my draft".into();
            search(&mut app, "side needle");
            settled(&mut app, &ctx);
            assert_eq!(app.tab().unwrap().commits()[0].id.as_str(), side);
            let newer = orphan(&f, "side needle new");
            f.run(&["update-ref", "refs/heads/side", &newer]);
            until(&mut app, &ctx, |a| {
                !a.busy()
                    && a.tab()
                        .unwrap()
                        .commits()
                        .first()
                        .is_some_and(|c| c.id.as_str() == newer)
            });
            assert_eq!(
                app.tab()
                    .unwrap()
                    .snapshot
                    .as_ref()
                    .unwrap()
                    .history
                    .as_ref()
                    .unwrap()
                    .commits[0]
                    .id,
                first
            );
            assert_eq!(app.tab().unwrap().draft, "Retain my draft");
            app.dispatch(Intent::History(None));
            settled(&mut app, &ctx);
            assert!(app.tab().unwrap().search_state.complete);
            assert!(app.tab().unwrap().commits().is_empty());
            app.dispatch(Intent::AllBranches);
            settled(&mut app, &ctx);
            assert_eq!(app.tab().unwrap().commits().len(), 1);
            search(&mut app, "");
            settled(&mut app, &ctx);
            assert_eq!(app.tab().unwrap().commits().len(), 2);
        },
    );
}

#[test]
fn replacing_stopping_and_resuming_search_rejects_old_results() {
    common::isolated(
        "ROCKET_UI_MANAGEMENT_CASE",
        "replacing_stopping_and_resuming_search_rejects_old_results",
        || {
            let f = Fixture::new();
            f.write("file", "baseline");
            f.commit("baseline needle");
            let ctx = eframe::egui::Context::default();
            let mut app =
                Rocket::with_log_directory(&ctx, vec![f.repository.clone()], f.root.join("logs"));
            settled(&mut app, &ctx);
            search(&mut app, "does not exist");
            until(&mut app, &ctx, |a| {
                a.tab().unwrap().search_state.task.is_some()
            });
            search(&mut app, "baseline");
            settled(&mut app, &ctx);
            assert_eq!(app.tab().unwrap().commits().len(), 1);
            assert!(app.tab().unwrap().search_state.complete);
            search(&mut app, "needle");
            until(&mut app, &ctx, |a| {
                a.tab().unwrap().search_state.task.is_some()
            });
            app.dispatch(Intent::StopSearch);
            settled(&mut app, &ctx);
            assert!(app.tab().unwrap().search_state.paused);
            assert!(!app.tab().unwrap().search_state.complete);
            app.dispatch(Intent::More);
            settled(&mut app, &ctx);
            assert_eq!(app.tab().unwrap().commits().len(), 1);
            assert!(app.tab().unwrap().search_state.complete);
            search(&mut app, "pending");
            let id = app.tab().unwrap().id;
            app.remove_tab(id);
            settled(&mut app, &ctx);
            assert!(app.tabs.is_empty());
        },
    );
}

#[test]
fn deferred_refresh_is_idle_in_a_dialog_and_runs_after_dismissal() {
    common::isolated(
        "ROCKET_UI_MANAGEMENT_CASE",
        "deferred_refresh_is_idle_in_a_dialog_and_runs_after_dismissal",
        || {
            let f = Fixture::new();
            f.write("file", "baseline");
            f.commit("baseline");
            let ctx = eframe::egui::Context::default();
            let mut app =
                Rocket::with_log_directory(&ctx, vec![f.repository.clone()], f.root.join("logs"));
            settled(&mut app, &ctx);
            let before = app.tab().unwrap().refresh_count;
            app.dispatch(Intent::Dialog(DialogKind::AddRemote));
            f.write("file", "changed while dialog open");
            until(&mut app, &ctx, |a| {
                a.tab().unwrap().refresh_pending.is_some()
            });
            assert!(
                !app.busy(),
                "A deferred refresh cannot run until this dialog closes"
            );
            assert_eq!(app.tab().unwrap().refresh_count, before);
            app.dialog = None;
            until(&mut app, &ctx, |a| {
                !a.busy() && a.tab().unwrap().refresh_count > before
            });
            assert_eq!(
                app.tab()
                    .unwrap()
                    .snapshot
                    .as_ref()
                    .unwrap()
                    .status
                    .as_ref()
                    .unwrap()
                    .entries
                    .len(),
                1
            );
        },
    );
}

#[test]
fn concurrent_tab_search_completions_load_each_selected_diff() {
    common::isolated(
        "ROCKET_UI_MANAGEMENT_CASE",
        "concurrent_tab_search_completions_load_each_selected_diff",
        || {
            let f = Fixture::new();
            f.write("one", "one");
            f.commit("shared query one");
            let second = Fixture::new();
            second.write("two", "two");
            second.commit("shared query two");
            let ctx = eframe::egui::Context::default();
            let mut app = Rocket::with_log_directory(
                &ctx,
                vec![f.repository.clone(), second.repository.clone()],
                f.root.join("logs"),
            );
            settled(&mut app, &ctx);
            for index in 0..2 {
                app.active = index;
                app.tabs[index].search = "shared query".into();
                app.dispatch(Intent::Search);
                app.tabs[index].search_state.due = Some(Instant::now());
                app.poll(&ctx);
                assert!(app.tabs[index].search_state.task.is_some());
            }
            // Allow both real Git workers to finish before one controller poll.
            thread::sleep(Duration::from_secs(2));
            settled(&mut app, &ctx);
            for tab in &app.tabs {
                assert!(tab.search_state.complete);
                assert_eq!(tab.commits().len(), 1);
                assert!(
                    tab.patch.is_some(),
                    "each completed search must load its selected diff"
                );
            }
        },
    );
}
