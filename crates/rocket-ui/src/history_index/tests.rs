//! The Speed up history notice and dialog, driven through the app against real
//! fixture repositories: what is on screen, what a click starts, and what Git has
//! on disk afterwards.
use super::*;
use crate::test_common as common;
use std::{fs, path::Path, thread};

const CASE: &str = "ROCKET_HISTORY_INDEX_CASE";
const WINDOW: Vec2 = vec2(1480.0, 920.0);

/// One window frame as the app draws it: the window, its intents, then the dialog.
fn frame(ctx: &Context, app: &mut Rocket, events: Vec<Event>) {
    app.frame_widgets.clear();
    let mut output = ctx.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, WINDOW)),
            events,
            ..Default::default()
        },
        |ui| {
            let mut intents = Vec::new();
            app.shell(ui, &mut intents);
            for intent in intents {
                app.dispatch(intent);
            }
            app.dialog_ui(&ui.ctx().clone());
        },
    );
    output.textures_delta.clear();
}

fn frames(ctx: &Context, app: &mut Rocket) {
    for _ in 0..3 {
        frame(ctx, app, vec![]);
    }
}

fn click(ctx: &Context, app: &mut Rocket, key: &str) {
    frames(ctx, app);
    let control = *app.frame_widgets.get(key).unwrap_or_else(|| panic!("{key} is not on screen"));
    assert!(control.enabled, "{key} is disabled");
    let pos = control.rect.center();
    let press = |pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    frame(ctx, app, vec![Event::PointerMoved(pos), press(true)]);
    frame(ctx, app, vec![press(false)]);
}

/// Polls as the app does between frames until `done`, with a limit.
fn until(ctx: &Context, app: &mut Rocket, what: &str, done: impl Fn(&Rocket) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        app.poll(ctx);
        if done(app) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting until {what}");
        thread::sleep(Duration::from_millis(10));
    }
}

fn opened(ctx: &Context, fixture: &common::Fixture, commits: usize) -> Rocket {
    for n in 0..commits {
        fixture.write("file.txt", format!("{n}\n"));
        fixture.commit(&format!("commit {n}"));
    }
    let mut app = Rocket::with_log_directory(ctx, vec![fixture.repository.clone()], fixture.root.join("logs"));
    until(ctx, &mut app, "the repository is read", |app| !app.busy());
    app.auto_refresh = false;
    app.tabs[0].view = View::Commits;
    app
}

fn state(app: &Rocket) -> &DialogState {
    app.dialog.as_ref().expect("the dialog is open").history_index.as_ref().expect("its state")
}

/// The dialog's reads have landed and nothing is running.
fn settled(app: &Rocket) -> bool {
    let s = state(app);
    s.task.is_none() && s.count.is_none() && s.applying.is_none() && !app.busy()
}

fn shown(app: &Rocket, keys: &[&str]) {
    for key in keys {
        assert!(app.frame_widgets.contains_key(*key), "{key} is not on screen");
    }
}

fn hidden(app: &Rocket, keys: &[&str]) {
    for key in keys {
        assert!(!app.frame_widgets.contains_key(*key), "{key} is on screen");
    }
}

fn index_files(repository: &Path) -> (bool, bool) {
    let info = repository.join(".git/objects/info");
    (info.join("commit-graph").is_file(), info.join("commit-graphs/commit-graph-chain").is_file())
}

#[test]
fn the_notice_opens_the_dialog_and_its_close_hides_it_for_that_repository() {
    common::isolated(CASE, "history_index::tests::the_notice_opens_the_dialog_and_its_close_hides_it_for_that_repository", || {
        let fixture = common::Fixture::new();
        let ctx = Context::default();
        let mut app = opened(&ctx, &fixture, 3);
        // Three commits order in milliseconds: nothing is offered.
        let snapshot = app.tabs[0].snapshot.as_ref().unwrap();
        assert!(snapshot.ordering.is_some_and(|took| took < SLOW) && !snapshot.history_notice);
        frames(&ctx, &mut app);
        hidden(&app, &["history-speed-notice", "history-speed-open", "history-speed-hide"]);

        // As loads mark a repository whose ordering was slow and that has no index.
        let slow = |app: &mut Rocket, loads: u8| {
            let snapshot = app.tabs[0].snapshot.as_mut().unwrap();
            snapshot.ordering = Some(Duration::from_millis(3_100));
            snapshot.history_notice = true;
            snapshot.slow_loads = loads;
        };
        // One slow load can be a busy moment; the notice waits for a second in a row.
        slow(&mut app, 1);
        frames(&ctx, &mut app);
        hidden(&app, &["history-speed-notice", "history-speed-open"]);
        assert_eq!(slow_loads(None, true), 1);
        assert_eq!(slow_loads(app.tabs[0].snapshot.as_ref(), true), 2);
        assert_eq!(slow_loads(app.tabs[0].snapshot.as_ref(), false), 0, "a fast load starts the count over");
        slow(&mut app, u8::MAX);
        assert_eq!(slow_loads(app.tabs[0].snapshot.as_ref(), true), u8::MAX);
        slow(&mut app, SLOW_LOADS);
        frames(&ctx, &mut app);
        shown(&app, &["history-speed-notice", "history-speed-open", "history-speed-hide"]);
        // The notice sits between the History bar and the list, across the pane.
        let (notice, list) = (app.frame_widgets["history-speed-notice"].rect, app.frame_widgets["history-scope"].rect);
        assert!(notice.top() >= list.bottom() && notice.width() > 600.0 && notice.height() < 48.0, "{notice:?}");

        click(&ctx, &mut app, "history-speed-open");
        assert!(matches!(app.dialog.as_ref().unwrap().kind, DialogKind::HistoryIndex));
        until(&ctx, &mut app, "the dialog has read the index", settled);
        frames(&ctx, &mut app);
        shown(&app, &["dialog-submit", "dialog-history-index-keep", "dialog-history-index-facts"]);
        assert_eq!(state(&app).commits, Some(3));
        assert!(state(&app).index.as_ref().is_some_and(|index| index.file.is_none() && index.unused.is_none()));
        app.dialog = None;

        // The × hides it now, is remembered for this repository, and holds for later refreshes.
        click(&ctx, &mut app, "history-speed-hide");
        frames(&ctx, &mut app);
        hidden(&app, &["history-speed-notice"]);
        let path = app.tabs[0].path.clone();
        assert!(app.history_notice_closed(&path));
        assert!(!offered(&app.tabs[0], &[path.clone()].into()));
        assert!(offered(&app.tabs[0], &Default::default()));
        // A pinned review can write nothing, so it is offered nothing either.
        app.tabs[0].review_window = true;
        assert!(!offered(&app.tabs[0], &Default::default()));
    });
}

#[test]
fn the_dialog_writes_removes_and_updates_the_index_and_stores_the_fetch_choice() {
    common::isolated(CASE, "history_index::tests::the_dialog_writes_removes_and_updates_the_index_and_stores_the_fetch_choice", || {
        let fixture = common::Fixture::new();
        let ctx = Context::default();
        let mut app = opened(&ctx, &fixture, 3);
        let references = fixture.run(&["for-each-ref"]);
        app.show_dialog(DialogKind::HistoryIndex);
        until(&ctx, &mut app, "the dialog has read the index", settled);

        // Review: the write, the checkbox, and nothing to remove yet.
        frames(&ctx, &mut app);
        shown(&app, &["dialog-submit", "dialog-history-index-keep"]);
        hidden(&app, &["dialog-history-index-remove", "dialog-history-index-state", "dialog-history-index-written"]);
        assert_eq!(index_files(&fixture.repository), (false, false));

        click(&ctx, &mut app, "dialog-submit");
        assert!(matches!(state(&app).applying, Some(HistoryIndexChange::Write { on_fetch: true, commits: Some(3) })));
        // While it runs the footer is Stop alone.
        frames(&ctx, &mut app);
        if state(&app).applying.is_some() {
            shown(&app, &["cancel-operation"]);
            hidden(&app, &["dialog-submit"]);
        }
        until(&ctx, &mut app, "the write and the refresh after it are done", settled);

        // Done: what was written, the time with the index, and Remove.
        frames(&ctx, &mut app);
        shown(&app, &["dialog-history-index-written", "dialog-history-index-after", "dialog-history-index-remove"]);
        hidden(&app, &["dialog-submit", "dialog-history-index-keep"]);
        let s = state(&app);
        assert!(s.written.is_some() && s.failure.is_none());
        let index = s.index.clone().unwrap();
        assert_eq!(index.file.as_ref().unwrap().commits, Some(3));
        assert!(index.on_fetch);
        assert_eq!(index_files(&fixture.repository), (true, false));
        assert_eq!(fixture.run(&["config", "--local", "--get", "rocket.commitGraphOnFetch"]), b"true\n");
        assert_eq!(fixture.run(&["for-each-ref"]), references);
        // The refresh after the write found the index, so the notice has nothing to offer.
        assert!(!app.tabs[0].snapshot.as_ref().unwrap().history_notice);

        // Remove returns to the review, with the earlier choice still ticked.
        click(&ctx, &mut app, "dialog-history-index-remove");
        until(&ctx, &mut app, "the index is removed", settled);
        frames(&ctx, &mut app);
        shown(&app, &["dialog-submit", "dialog-history-index-keep"]);
        hidden(&app, &["dialog-history-index-written", "dialog-history-index-remove"]);
        assert_eq!(index_files(&fixture.repository), (false, false));
        assert!(state(&app).index.as_ref().is_some_and(|index| index.file.is_none() && index.on_fetch));

        // Written again and opened later: the index as it is, Update and Remove.
        click(&ctx, &mut app, "dialog-submit");
        until(&ctx, &mut app, "the second write is done", settled);
        app.dialog = None;
        fixture.write("file.txt", "later\n");
        fixture.commit("later");
        app.show_dialog(DialogKind::HistoryIndex);
        until(&ctx, &mut app, "the reopened dialog has read the index", settled);
        frames(&ctx, &mut app);
        shown(&app, &["dialog-history-index-state", "dialog-history-index-keep", "dialog-submit", "dialog-history-index-remove"]);
        hidden(&app, &["dialog-history-index-written"]);
        let s = state(&app);
        assert_eq!(s.commits, Some(4));
        assert_eq!(s.index.as_ref().unwrap().file.as_ref().unwrap().commits, Some(3));

        // Update adds the commit made since, as a second layer.
        click(&ctx, &mut app, "dialog-submit");
        until(&ctx, &mut app, "the update is done", settled);
        assert_eq!(state(&app).index.as_ref().unwrap().file.as_ref().unwrap().commits, Some(4));
        assert_eq!(index_files(&fixture.repository), (false, true));

        // Reopened, the checkbox stores the choice at once.
        app.dialog = None;
        app.show_dialog(DialogKind::HistoryIndex);
        until(&ctx, &mut app, "the dialog has read the index again", settled);
        click(&ctx, &mut app, "dialog-history-index-keep");
        assert!(matches!(state(&app).applying, Some(HistoryIndexChange::OnFetch(false))));
        until(&ctx, &mut app, "the setting is saved", settled);
        assert!(!state(&app).index.as_ref().unwrap().on_fetch);
        let stored = fixture.output(&fixture.repository, &["config", "--local", "--get", "rocket.commitGraphOnFetch"]);
        assert_eq!(stored.status.code(), Some(1));
        fixture.run(&["commit-graph", "verify"]);
    });
}

#[test]
fn a_refused_write_says_why_keeps_its_details_and_can_be_tried_again() {
    common::isolated(CASE, "history_index::tests::a_refused_write_says_why_keeps_its_details_and_can_be_tried_again", || {
        let fixture = common::Fixture::new();
        let ctx = Context::default();
        let mut app = opened(&ctx, &fixture, 2);
        let lock = fixture.repository.join(".git/objects/info/commit-graph.lock");
        fs::write(&lock, b"another writer").unwrap();
        app.show_dialog(DialogKind::HistoryIndex);
        until(&ctx, &mut app, "the dialog has read the index", settled);
        click(&ctx, &mut app, "dialog-submit");
        until(&ctx, &mut app, "the refused write is back", settled);

        frames(&ctx, &mut app);
        shown(&app, &["dialog-history-index-failure", "dialog-history-index-copy", "dialog-submit"]);
        hidden(&app, &["dialog-history-index-keep", "dialog-history-index-written"]);
        let failure = state(&app).failure.as_ref().unwrap();
        assert!(matches!(failure.change, HistoryIndexChange::Write { .. }));
        assert!(failure.reason.contains("commit-graph.lock"), "{}", failure.reason);
        assert!(!failure.reason.starts_with(WRITE_PHASE), "{}", failure.reason);
        assert!(failure.details.contains("commit-graph.lock") && failure.details.contains(WRITE_PHASE));
        // Nothing was written, and the other writer's lock is where it was.
        assert_eq!(index_files(&fixture.repository), (false, false));
        assert_eq!(fs::read(&lock).unwrap(), b"another writer");

        // With the lock gone, Try again writes the index.
        fs::remove_file(&lock).unwrap();
        click(&ctx, &mut app, "dialog-submit");
        until(&ctx, &mut app, "the retried write is done", settled);
        frames(&ctx, &mut app);
        shown(&app, &["dialog-history-index-written", "dialog-history-index-remove"]);
        hidden(&app, &["dialog-history-index-failure"]);
        assert!(state(&app).failure.is_none());
        assert_eq!(index_files(&fixture.repository), (true, false));
    });
}

#[test]
fn a_shallow_clone_explains_itself_and_offers_no_write() {
    common::isolated(CASE, "history_index::tests::a_shallow_clone_explains_itself_and_offers_no_write", || {
        let fixture = common::Fixture::new();
        let ctx = Context::default();
        for n in 0..2 {
            fixture.write("file.txt", format!("{n}\n"));
            fixture.commit(&format!("commit {n}"));
        }
        fs::write(fixture.repository.join(".git/shallow"), fixture.run(&["rev-parse", "HEAD"])).unwrap();
        let mut app = opened(&ctx, &fixture, 0);
        app.show_dialog(DialogKind::HistoryIndex);
        until(&ctx, &mut app, "the dialog has read the index", settled);
        frames(&ctx, &mut app);
        assert_eq!(state(&app).index.as_ref().unwrap().unused, Some(HistoryIndexUnused::Shallow));
        assert!(!app.frame_widgets["dialog-submit"].enabled, "a shallow clone offers no write");
        assert_eq!(index_files(&fixture.repository), (false, false));
    });
}

#[test]
fn times_sizes_and_estimates_read_as_the_dialog_shows_them() {
    assert_eq!(seconds(Duration::from_millis(3_140)), "3.1 s");
    assert_eq!(seconds(Duration::from_millis(180)), "0.2 s");
    assert_eq!(seconds(Duration::from_millis(12_700)), "12 s");
    // Coarse steps: five seconds up to a minute and a half, then whole minutes.
    assert_eq!(estimate(3), "About 5 s here");
    assert_eq!(estimate(100_500), "About 10 s here");
    assert_eq!(estimate(482_117), "About 35 s here");
    assert_eq!(estimate(1_500_000), "About 2 min here");
    assert_eq!(crate::explore::size(482_117 * BYTES_A_COMMIT), "28 MB");
    assert_eq!(commits_text(1), "1 commit");
    assert_eq!(commits_text(482_117), "482,117 commits");
    // Git's own line is the reason; without one, the step's error without its name.
    let mut outcome = Outcome {
        summary: "The commit index was not written.".into(),
        details: vec![format!("{WRITE_PHASE}: Git write commit index failed (exit 128). Review the output.")],
        raw_output: "warning: something else\nfatal: unable to create 'x/commit-graph.lock': Permission denied\n".into(),
        ..Default::default()
    };
    assert_eq!(reason(&outcome), "unable to create 'x/commit-graph.lock': Permission denied");
    outcome.raw_output.clear();
    assert_eq!(reason(&outcome), "Git write commit index failed (exit 128). Review the output.");
    outcome.details.clear();
    assert_eq!(reason(&outcome), "The commit index was not written.");
}
