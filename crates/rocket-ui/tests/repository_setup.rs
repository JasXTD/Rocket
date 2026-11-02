#![cfg(windows)]
#[path = "../../rocket-git/tests/common/mod.rs"]
mod common;
use common::Fixture;
use rocket_ui::app::{DialogKind, Rocket};
use std::{
    thread,
    time::{Duration, Instant},
};

fn settle(app: &mut Rocket, ctx: &eframe::egui::Context) {
    let until = Instant::now() + Duration::from_secs(45);
    loop {
        ctx.run_ui(Default::default(), |_| {})
            .drop_without_applying_deltas();
        app.poll(ctx);
        if !app.busy() {
            return;
        }
        assert!(Instant::now() < until, "UI controller did not settle");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn creation_from_welcome_opens_unborn_repository_and_loads_settings_on_worker() {
    common::isolated(
        "ROCKET_UI_SETUP_CASE",
        "creation_from_welcome_opens_unborn_repository_and_loads_settings_on_worker",
        || {
            let f = Fixture::new();
            f.write("existing-file", "preserve existing project");
            f.commit("Existing repository");
            let original_head = f
                .repo()
                .head(&rocket_git::CancellationToken::new())
                .unwrap();
            let ctx = eframe::egui::Context::default();
            let mut app = Rocket::with_log_directory(&ctx, vec![], f.root.join("logs"));
            settle(&mut app, &ctx);
            assert!(app.tabs.is_empty());
            let destination = f.root.canonicalize().unwrap().join("new project");
            app.init_repository(destination.clone(), "main".into());
            assert!(app.write.is_some());
            settle(&mut app, &ctx);
            assert!(app.activities.last().unwrap().outcome.command_succeeded);
            assert_eq!(app.tabs.len(), 1);
            assert!(matches!(
                app.tab().unwrap().snapshot.as_ref().unwrap().head,
                Some(rocket_git::Head::Unborn { .. })
            ));
            assert!(!destination.join(".git/index").exists());
            assert_eq!(
                f.repo()
                    .head(&rocket_git::CancellationToken::new())
                    .unwrap(),
                original_head
            );
            app.show_dialog(DialogKind::RepositorySettings);
            assert!(app.dialog.as_ref().unwrap().identity_task.is_some());
            settle(&mut app, &ctx);
            let dialog = app.dialog.as_ref().unwrap();
            assert!(dialog.identity.is_some());
            assert!(dialog.first.is_empty());
            assert!(dialog.second.is_empty());
            app.dialog = None;
            app.show_dialog(DialogKind::RepositorySettings);
            app.dialog = None;
            settle(&mut app, &ctx);
            assert!(app.activities.last().unwrap().outcome.command_succeeded);
        },
    );
}
