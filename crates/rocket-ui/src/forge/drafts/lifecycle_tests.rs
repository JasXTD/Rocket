use super::*;
use crate::{persistence::Store, test_common as common};
use std::thread;

fn scope(number: u64) -> Scope {
    Scope {
        provider: ForgeProvider::GitHub,
        server: "https://github.com".into(),
        api: "7.1".into(),
        account: "synthetic-local-test".into(),
        repository: "fixture/local-only".into(),
        number,
    }
}

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
fn full_store_blocks_exit_until_the_active_draft_is_saved() {
    common::isolated(
        "ROCKET_PR_CAPACITY_CASE",
        "forge::drafts::lifecycle_tests::full_store_blocks_exit_until_the_active_draft_is_saved",
        || {
            let fixture = common::Fixture::new();
            let directory = fixture.root.join("state");
            let (store, mut saved) = Store::open(&directory).unwrap();
            saved.preferences.dependency_warnings = false;
            store.save(&saved).unwrap();
            drop(store);
            let ctx = Context::default();
            let mut app =
                Rocket::with_storage(&ctx, vec![], fixture.root.join("logs"), directory.clone());
            settle(&mut app, &ctx);
            for number in 1..=RECORDS as u64 {
                app.forge.drafts.active = Some(scope(number));
                app.forge.pull.body = format!("Saved draft {number}");
                capture(&mut app.forge);
            }
            settle(&mut app, &ctx);
            assert!(app.forge.drafts.error.is_none());
            let original = load(&directory).unwrap().records;
            assert_eq!(original.len(), RECORDS);

            let new_scope = scope(RECORDS as u64 + 1);
            let text = "New unsaved PR draft 資料";
            app.forge.drafts.active = Some(new_scope.clone());
            app.forge.pull.body = text.into();
            // No explicit capture: quitting must scan even within the debounce.
            app.request_close();
            assert!(app.dialog.is_some());
            app.begin_quit();
            settle(&mut app, &ctx);
            assert!(!app.quitting, "capacity failure must keep the editor open");
            assert_eq!(app.forge.pull.body, text);
            assert!(app.notice.as_ref().unwrap().contains("limit reached"));
            assert!(load(&directory).unwrap().records == original);

            // The existing Saved drafts panel deletes a chosen record and offers
            // an explicit storage retry. Neither action discards this active text.
            app.forge.drafts.records.remove(0);
            app.forge.drafts.error = None;
            app.begin_quit();
            settle(&mut app, &ctx);
            assert!(app.quitting);
            let recovered = load(&directory).unwrap().records;
            assert_eq!(recovered.len(), RECORDS);
            assert!(
                recovered
                    .iter()
                    .any(|r| r.scope == new_scope && r.data["body"] == text)
            );
            assert!(original[1..].iter().all(|r| recovered.contains(r)));
            drop(app);
        },
    );
}

#[test]
fn capacity_tracking_clears_when_the_active_text_is_discarded_or_captured() {
    let mut state = super::super::State::default();
    state.drafts.loaded = true;
    for number in 1..=RECORDS as u64 {
        state.drafts.active = Some(scope(number));
        state.pull.body = format!("Saved draft {number}");
        capture(&mut state);
    }
    state.drafts.written = state.drafts.records.clone();
    state.drafts.active = Some(scope(RECORDS as u64 + 1));
    state.pull.body = "Extra draft".into();
    capture(&mut state);
    assert!(state.drafts.unsaved());
    state.pull.body.clear();
    capture(&mut state);
    assert!(
        !state.drafts.unsaved(),
        "explicitly clearing text must allow exit"
    );

    state.drafts.error = None;
    state.drafts.active = Some(scope(1));
    state.pull.body = "Updated existing draft".into();
    capture(&mut state);
    assert!(state.drafts.error.is_none());
    assert!(state.drafts.unsaved());
    assert_eq!(state.drafts.records.len(), RECORDS);
    assert_eq!(
        state.drafts.records[0].data["body"],
        "Updated existing draft"
    );
}
