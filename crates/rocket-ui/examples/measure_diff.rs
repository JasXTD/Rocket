//! Read-only controller benchmark: cargo run --release -p rocket-ui --example
//! measure_diff -- <repository> <new-log-directory>. Does not invoke UI widgets.
use eframe::egui::Context;
use rocket_ui::app::{Intent, Rocket, View};
use std::{
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

fn settle(app: &mut Rocket, ctx: &Context) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while app.busy() {
        app.poll(ctx);
        assert!(Instant::now() < deadline, "worker did not settle");
        thread::sleep(Duration::from_millis(1));
    }
    assert!(app.startup_error.is_none(), "{:?}", app.startup_error);
    let tab = app.tab().expect("repository tab");
    assert!(tab.error.is_none(), "{:?}", tab.error);
    assert!(tab.patch_error.is_none(), "{:?}", tab.patch_error);
}

fn sample(app: &mut Rocket, ctx: &Context, intent: Intent, kind: &str) {
    let started = Instant::now();
    app.dispatch(intent);
    settle(app, ctx);
    let elapsed = started.elapsed();
    let patch = app.tab().unwrap().patch.as_ref().expect("loaded patch");
    println!(
        "{kind}\t{:.3}\t{}\t{}",
        elapsed.as_secs_f64() * 1000.0,
        patch.files.len(),
        patch.files.iter().map(|f| f.lines.len()).sum::<usize>()
    );
}

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    assert_eq!(args.len(), 2, "expected repository and new log directory");
    let logs = PathBuf::from(&args[1]);
    assert!(
        !logs.exists(),
        "use a new log directory for each measurement"
    );
    let ctx = Context::default();
    let mut app = Rocket::with_log_directory(&ctx, vec![PathBuf::from(&args[0])], logs);
    settle(&mut app, &ctx);
    let commits: Vec<_> = app
        .tab()
        .unwrap()
        .commits()
        .iter()
        .take(5)
        .map(|c| c.id.clone())
        .collect();
    assert!(!commits.is_empty(), "fixture needs a commit");
    println!("kind\tms\tfiles\tlines");
    for i in 0..5 {
        sample(
            &mut app,
            &ctx,
            Intent::SelectCommit(commits[i % commits.len()].clone()),
            "commit",
        );
    }
    let change = app
        .tab()
        .unwrap()
        .snapshot
        .as_ref()
        .unwrap()
        .status
        .as_ref()
        .and_then(|s| {
            s.entries.iter().find(|e| {
                e.worktree != rocket_git::Change::Unchanged
                    && e.kind != rocket_git::EntryKind::Untracked
            })
        })
        .cloned();
    if let Some(change) = change {
        app.dispatch(Intent::View(View::Changes));
        for _ in 0..5 {
            sample(
                &mut app,
                &ctx,
                Intent::SelectChange(change.clone(), false),
                "unstaged",
            );
        }
    }
}
