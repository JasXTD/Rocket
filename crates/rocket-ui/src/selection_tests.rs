use crate::test_common as common;

use crate::app::{Intent, Rocket};
use eframe::egui::Context;
use rocket_git::{CommitOperation, ObjectId};
use std::{
    thread,
    time::{Duration, Instant},
};

#[test]
fn branch_toolbar_opens_existing_reviews_and_gates_integration() {
    common::isolated(
        "ROCKET_SELECTION_CASE",
        "selection_tests::branch_toolbar_opens_existing_reviews_and_gates_integration",
        || {
            use crate::{app::DialogKind, batch::flow_tests::{click, frame, open}};
            use rocket_git::{BranchWorkflowKind, Head};
            let f = common::Fixture::new();
            f.write("branch.txt", "original\n");
            f.commit("original");
            let before = f.run(&["rev-parse", "HEAD"]);
            let ctx = Context::default();
            let mut app = open(&ctx, &f);
            click(&ctx, &mut app, "toolbar-branch");
            assert!(app.dialog.is_none(), "Opening the dropdown must not open a review");
            for id in ["toolbar-branch-new", "toolbar-branch-sync", "toolbar-branch-finish"] {
                assert!(app.frame_widgets[id].enabled);
            }
            click(&ctx, &mut app, "toolbar-branch-new");
            assert!(matches!(app.dialog.as_ref().unwrap().kind, DialogKind::NewBranch));
            click(&ctx, &mut app, "dialog-cancel");
            for (id, mode) in [("toolbar-branch-sync", 6), ("toolbar-branch-finish", 7)] {
                click(&ctx, &mut app, "toolbar-branch");
                click(&ctx, &mut app, id);
                let workflow = app.dialog.as_ref().unwrap().workflow.as_ref().unwrap();
                assert_eq!(workflow.mode, mode);
                assert!(workflow.preview.is_none());
                assert!(!workflow.save_changes && !workflow.allow_fast_forward);
                click(&ctx, &mut app, "dialog-cancel");
            }
            let head = app.tabs[0].snapshot.as_ref().unwrap().head.clone();
            let target = app.tabs[0].commits()[0].id.clone();
            app.tabs[0].snapshot.as_mut().unwrap().head = Some(Head::Detached { target });
            frame(&ctx, &mut app, vec![]);
            click(&ctx, &mut app, "toolbar-branch");
            assert!(app.frame_widgets["toolbar-branch-new"].enabled);
            assert!(!app.frame_widgets["toolbar-branch-sync"].enabled);
            assert!(!app.frame_widgets["toolbar-branch-finish"].enabled);
            app.dispatch(Intent::BranchWorkflow(BranchWorkflowKind::Sync));
            assert!(app.dialog.is_none());
            app.tabs[0].snapshot.as_mut().unwrap().head = head;
            app.tabs[0].snapshot.as_mut().unwrap().state.as_mut().unwrap().rebase = true;
            assert!(!app.can_branch_workflow());
            app.dispatch(Intent::BranchWorkflow(BranchWorkflowKind::Finish));
            assert!(app.dialog.is_none());
            assert_eq!(f.run(&["rev-parse", "HEAD"]), before);
            assert!(f.run(&["status", "--porcelain"]).is_empty());
        },
    );
}

/// Ctrl/Shift-click history rows through the real dispatcher and check the order
/// a reviewed cherry-pick or revert sequence would use.
#[test]
fn ctrl_and_shift_clicks_build_ordered_commit_sequences() {
    common::isolated(
        "ROCKET_SELECTION_CASE",
        "selection_tests::ctrl_and_shift_clicks_build_ordered_commit_sequences",
        || {
            let fixture = common::Fixture::new();
            for name in ["A", "B", "C", "D", "E"] {
                fixture.write(&format!("{name}.txt"), name);
                fixture.commit(name);
            }
            let ctx = Context::default();
            let mut app = Rocket::with_log_directory(
                &ctx,
                vec![fixture.repository.clone()],
                fixture.root.join("logs"),
            );
            let deadline = Instant::now() + Duration::from_secs(40);
            while app.busy() {
                app.poll(&ctx);
                assert!(Instant::now() < deadline);
                thread::sleep(Duration::from_millis(10));
            }
            app.auto_refresh = false;
            // Newest first, as the history list shows them: E, D, C, B, A.
            let listed: Vec<ObjectId> =
                app.tabs[0].commits().iter().map(|c| c.id.clone()).collect();
            assert_eq!(listed.len(), 5);
            let ids =
                |range: &[usize]| range.iter().map(|&i| listed[i].clone()).collect::<Vec<_>>();
            fn tab(app: &Rocket) -> &crate::app::Tab {
                &app.tabs[0]
            }

            app.dispatch(Intent::SelectCommit(listed[1].clone()));
            app.dispatch(Intent::ToggleCommit(listed[3].clone()));
            assert_eq!(tab(&app).selected_commits, ids(&[1, 3]));
            assert_eq!(tab(&app).selected_commit, Some(listed[3].clone()));
            // Cherry-pick applies oldest first; revert undoes newest first.
            assert_eq!(
                tab(&app).commit_targets(CommitOperation::CherryPick),
                ids(&[3, 1])
            );
            assert_eq!(
                tab(&app).commit_targets(CommitOperation::Revert),
                ids(&[1, 3])
            );

            // Removing one of two leaves a single selection, shown in details.
            app.dispatch(Intent::ToggleCommit(listed[3].clone()));
            assert!(tab(&app).selected_commits.is_empty());
            assert_eq!(tab(&app).selected_commit, Some(listed[1].clone()));
            assert_eq!(tab(&app).commit_targets(CommitOperation::Revert), ids(&[1]));

            // Shift-click ranges keep the plain-click anchor while they grow or shrink.
            app.dispatch(Intent::SelectCommit(listed[1].clone()));
            app.dispatch(Intent::SelectCommitRange(listed[3].clone()));
            assert_eq!(tab(&app).selected_commits, ids(&[1, 2, 3]));
            app.dispatch(Intent::SelectCommitRange(listed[0].clone()));
            assert_eq!(tab(&app).selected_commits, ids(&[0, 1]));
            assert_eq!(tab(&app).selected_commit, Some(listed[0].clone()));
            assert_eq!(
                tab(&app).commit_targets(CommitOperation::CherryPick),
                ids(&[1, 0])
            );

            // Ctrl-click moves the anchor; a plain click clears the selection.
            app.dispatch(Intent::ToggleCommit(listed[4].clone()));
            assert_eq!(tab(&app).selected_commits, ids(&[0, 1, 4]));
            app.dispatch(Intent::SelectCommitRange(listed[2].clone()));
            assert_eq!(tab(&app).selected_commits, ids(&[2, 3, 4]));
            app.dispatch(Intent::SelectCommit(listed[2].clone()));
            assert!(tab(&app).selected_commits.is_empty());
            assert_eq!(
                tab(&app).commit_targets(CommitOperation::CherryPick),
                ids(&[2])
            );
        },
    );
}

/// Keyboard use of the commit list: a clicked row keeps focus, Up and Down select
/// the neighbouring commit, Space toggles the details popup and Tab leaves the list.
#[test]
fn the_commit_list_follows_arrow_keys_space_and_tab() {
    use crate::batch::flow_tests::{click, frame, open, wait};
    use eframe::egui::{Event, Key, Modifiers};
    common::isolated(
        "ROCKET_SELECTION_CASE",
        "selection_tests::the_commit_list_follows_arrow_keys_space_and_tab",
        || {
            let fixture = common::Fixture::new();
            for name in ["A", "B", "C", "D"] {
                fixture.write(&format!("{name}.txt"), name);
                fixture.commit(name);
            }
            let ctx = Context::default();
            let mut app = open(&ctx, &fixture);
            let listed: Vec<ObjectId> =
                app.tabs[0].commits().iter().map(|c| c.id.clone()).collect();
            // One key per frame, then a quiet frame, as typing does.
            let key = |app: &mut Rocket, modifiers, key| {
                let event = Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                };
                frame(&ctx, app, vec![event]);
                frame(&ctx, app, vec![]);
            };
            let selected = |app: &Rocket| app.tabs[0].selected_commit.clone();
            click(&ctx, &mut app, &format!("commit-{}", listed[1]));
            wait(&ctx, &mut app);
            assert_eq!(selected(&app), Some(listed[1].clone()));
            key(&mut app, Modifiers::NONE, Key::ArrowDown);
            assert_eq!(selected(&app), Some(listed[2].clone()));
            key(&mut app, Modifiers::NONE, Key::ArrowUp);
            key(&mut app, Modifiers::NONE, Key::ArrowUp);
            assert_eq!(selected(&app), Some(listed[0].clone()));
            // Up at the newest commit stays there.
            key(&mut app, Modifiers::NONE, Key::ArrowUp);
            assert_eq!(selected(&app), Some(listed[0].clone()));
            key(&mut app, Modifiers::NONE, Key::Space);
            assert_eq!(app.tabs[0].peek, Some(listed[0].clone()));
            assert!(app.frame_widgets.contains_key("commit-peek"));
            // Space again closes it; it never re-selects through a click.
            key(&mut app, Modifiers::NONE, Key::Space);
            assert_eq!(app.tabs[0].peek, None);
            assert!(!app.frame_widgets.contains_key("commit-peek"));
            assert_eq!(selected(&app), Some(listed[0].clone()));
            // Tab leaves the list: the arrows no longer move the selection.
            key(&mut app, Modifiers::NONE, Key::Tab);
            key(&mut app, Modifiers::NONE, Key::ArrowDown);
            assert_eq!(selected(&app), Some(listed[0].clone()));
            wait(&ctx, &mut app);
        },
    );
}

/// Collapsing a merge in the commit list: Left and Right on its row, the "+N"
/// badge, the button in its details and Collapse all merges, which hands the
/// selection of a hidden commit to the merge that hides it.
#[test]
fn merges_collapse_and_expand_in_the_commit_list() {
    use crate::batch::flow_tests::{click, frame, open, wait};
    use eframe::egui::{Event, Key, Modifiers};
    common::isolated(
        "ROCKET_SELECTION_CASE",
        "selection_tests::merges_collapse_and_expand_in_the_commit_list",
        || {
            let f = common::Fixture::new();
            f.write("base.txt", "base");
            f.commit("base");
            f.run(&["switch", "-c", "side"]);
            for name in ["S1", "S2"] {
                f.write(&format!("{name}.txt"), name);
                f.commit(name);
            }
            f.run(&["switch", "main"]);
            f.write("M1.txt", "M1");
            f.commit("M1");
            f.run(&["merge", "--no-ff", "-m", "Merge side", "side"]);
            f.write("T.txt", "T");
            f.commit("T");
            let rev = |name: &str| {
                ObjectId::parse(
                    String::from_utf8(f.run(&["rev-parse", name]))
                        .unwrap()
                        .trim(),
                )
                .unwrap()
            };
            let (merge, side_one, tip) = (rev("HEAD~1"), rev("side~1"), rev("HEAD"));
            let ctx = Context::default();
            let mut app = open(&ctx, &f);
            let key = |app: &mut Rocket, key| {
                let event = Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                };
                frame(&ctx, app, vec![event]);
                frame(&ctx, app, vec![]);
            };
            let rows = |app: &Rocket| app.tabs[0].commits().len();
            assert_eq!(rows(&app), 6);
            // Left on an ordinary commit changes nothing.
            click(&ctx, &mut app, &format!("commit-{tip}"));
            key(&mut app, Key::ArrowLeft);
            assert_eq!(rows(&app), 6);
            // Left on the merge hides S1 and S2; its row shows +2; Right shows them.
            click(&ctx, &mut app, &format!("commit-{merge}"));
            wait(&ctx, &mut app);
            key(&mut app, Key::ArrowLeft);
            assert_eq!(rows(&app), 4);
            assert_eq!(app.tabs[0].behind.get(&merge), Some(&2));
            assert!(
                app.frame_widgets
                    .contains_key(&format!("commit-expand-{merge}"))
            );
            key(&mut app, Key::ArrowRight);
            assert_eq!(rows(&app), 6);
            assert!(app.tabs[0].behind.is_empty());
            // The badge expands it again.
            key(&mut app, Key::ArrowLeft);
            assert_eq!(rows(&app), 4);
            click(&ctx, &mut app, &format!("commit-expand-{merge}"));
            assert_eq!(rows(&app), 6);
            // The row's right-click menu collapses it, and every merge.
            let right_click = |app: &mut Rocket, key: &str| {
                let pos = app.frame_widgets[key].rect.center();
                let press = |pressed| Event::PointerButton {
                    pos,
                    button: eframe::egui::PointerButton::Secondary,
                    pressed,
                    modifiers: Modifiers::NONE,
                };
                frame(
                    &ctx,
                    app,
                    vec![Event::PointerMoved(pos), press(true), press(false)],
                );
                frame(&ctx, app, vec![]);
            };
            right_click(&mut app, &format!("commit-{merge}"));
            click(&ctx, &mut app, "commit-menu-fold");
            assert_eq!(rows(&app), 4);
            right_click(&mut app, &format!("commit-{merge}"));
            click(&ctx, &mut app, "commit-menu-expand-all");
            assert_eq!(rows(&app), 6);
            // Collapse all with a side commit selected: the merge gets the selection.
            app.dispatch(Intent::SelectCommit(side_one.clone()));
            app.dispatch(Intent::FoldAllMerges(true));
            assert_eq!(rows(&app), 4);
            assert_eq!(app.tabs[0].selected_commit, Some(merge.clone()));
            wait(&ctx, &mut app);
            // The merge's details offer to expand it; the others stay collapsed.
            click(&ctx, &mut app, "detail-merge-fold");
            assert_eq!(rows(&app), 6);
            assert!(app.tabs[0].folding.all);
            app.dispatch(Intent::FoldAllMerges(false));
            assert!(!app.tabs[0].folding.active());
            assert_eq!(rows(&app), 6);
        },
    );
}
