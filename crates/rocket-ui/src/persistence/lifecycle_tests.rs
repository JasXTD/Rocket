use super::*;
use crate::{
    app::{Intent, Rocket},
    test_common as common,
};
use eframe::egui::Context;
use std::{
    thread,
    time::{Duration, Instant},
};

fn settle(app: &mut Rocket, ctx: &Context) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        app.poll(ctx);
        assert!(app.startup_error.is_none(), "{:?}", app.startup_error);
        assert!(app.state_error.is_none(), "{:?}", app.state_error);
        if app.backend.is_some() && app.forge.drafts.loaded && !app.busy() {
            break;
        }
        assert!(Instant::now() < deadline, "application did not settle");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn closing_unloaded_tabs_preserves_saved_drafts() {
    common::isolated(
        "ROCKET_UNLOADED_DRAFT_CASE",
        "persistence::lifecycle_tests::closing_unloaded_tabs_preserves_saved_drafts",
        || {
            for failed_load in [false, true] {
                let fixture = common::Fixture::new();
                let path = if failed_load {
                    fixture.root.join("missing-repository")
                } else {
                    fixture.repo().info().worktree.clone().unwrap()
                };
                let directory = fixture.root.join("state");
                let text = "Retain the saved commit draft 資料";
                let (store, mut saved) = Store::open(&directory).unwrap();
                saved.preferences.auto_refresh = false;
                saved.preferences.dependency_warnings = false;
                saved.drafts.insert(path.clone(), text.into());
                store.save(&saved).unwrap();
                drop(store);

                let ctx = Context::default();
                let mut app = Rocket::with_storage(
                    &ctx,
                    vec![],
                    fixture.root.join("logs"),
                    directory.clone(),
                );
                settle(&mut app, &ctx);
                app.open(path.clone());
                if failed_load {
                    settle(&mut app, &ctx);
                    assert!(app.tabs[0].error.is_some());
                }
                assert!(!app.tabs[0].draft_loaded);
                assert!(app.tabs[0].draft.is_empty());
                app.dispatch(Intent::CloseTab(app.tabs[0].id));
                assert!(app.tabs.is_empty());
                assert!(app.dialog.is_none());
                app.begin_quit();
                settle(&mut app, &ctx);
                drop(app);
                assert_eq!(
                    Store::open(&directory).unwrap().1.drafts[&path].as_ref(),
                    text
                );

                if !failed_load {
                    // Loaded drafts can still be deliberately cleared; preserving
                    // an unloaded draft must not resurrect an intentional deletion.
                    let mut app = Rocket::with_storage(
                        &ctx,
                        vec![path.clone()],
                        fixture.root.join("reopen-logs"),
                        directory.clone(),
                    );
                    settle(&mut app, &ctx);
                    assert!(app.tabs[0].draft_loaded);
                    assert_eq!(app.tabs[0].draft, text);
                    app.tabs[0].draft.clear();
                    app.dispatch(Intent::CloseTab(app.tabs[0].id));
                    app.begin_quit();
                    settle(&mut app, &ctx);
                    drop(app);
                    assert!(
                        !Store::open(&directory)
                            .unwrap()
                            .1
                            .drafts
                            .contains_key(&path)
                    );
                }
            }
        },
    );
}
