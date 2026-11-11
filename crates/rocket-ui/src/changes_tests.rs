use crate::test_common as common;

use crate::app::{Intent, Rocket, View};
use eframe::egui::*;
use rocket_git::{Branch, BranchKind, Change, Commit, EntryKind, ObjectId, Remote, RepoPath, StatusEntry};
use std::{
    thread,
    time::{Duration, Instant},
};

/// A small repository whose status and branch list are swapped for `files` unstaged
/// files, `local` more local branches and `tracking` branches of a remote "origin", so
/// long lists can be driven and timed without writing that many files or refs.
fn large_lists(ctx: &Context, fixture: &common::Fixture, files: usize, local: usize, tracking: usize) -> Rocket {
    fixture.write("file.txt", "base\n");
    fixture.commit("base");
    let mut app = Rocket::with_log_directory(
        ctx,
        vec![fixture.repository.clone()],
        fixture.root.join("logs"),
    );
    let deadline = Instant::now() + Duration::from_secs(40);
    while app.busy() {
        app.poll(ctx);
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    app.auto_refresh = false;
    let snapshot = app.tabs[0].snapshot.as_mut().unwrap();
    snapshot.status.as_mut().unwrap().entries = (0..files)
        .map(|n| StatusEntry {
            path: RepoPath::new(format!("src/module{:03}/file{n:05}.rs", n % 200).into_bytes()).unwrap(),
            original_path: None,
            kind: EntryKind::Tracked,
            index: Change::Unchanged,
            worktree: Change::Modified,
            submodule: None,
            similarity: None,
        })
        .collect();
    let head = snapshot.branches.iter().find(|b| b.is_head).cloned().unwrap();
    if tracking > 0 {
        snapshot.remotes.push(Remote {
            name: b"origin".to_vec(),
            urls: vec![b"https://example.invalid/origin.git".to_vec()],
            push_urls: Vec::new(),
            fetch_refspecs: Vec::new(),
        });
    }
    for (kind, prefix, count) in [
        (BranchKind::Local, "refs/heads", local),
        (BranchKind::Remote, "refs/remotes/origin", tracking),
    ] {
        snapshot.branches.extend((0..count).map(|n| Branch {
            reference: format!("{prefix}/topic/{n:05}").into_bytes(),
            kind,
            is_head: false,
            upstream: None,
            checked_out_at: None,
            ahead_behind: None,
            ..head.clone()
        }));
    }
    app
}

/// One window frame without input: the interface pass, then turning its shapes into
/// triangles, as the app does before it hands them to the GPU.
fn window_frame(ctx: &Context, app: &mut Rocket, size: Vec2, events: Vec<Event>) -> (Duration, Duration) {
    app.frame_widgets.clear();
    let started = Instant::now();
    let mut output = ctx.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |ui| {
            let mut intents = Vec::new();
            app.shell(ui, &mut intents);
            for intent in intents {
                app.dispatch(intent);
            }
        },
    );
    let interface = started.elapsed();
    output.textures_delta.clear();
    let started = Instant::now();
    std::hint::black_box(ctx.tessellate(output.shapes, output.pixels_per_point));
    (interface, started.elapsed())
}

const WINDOW: Vec2 = vec2(1480.0, 920.0);

/// Turns the wheel over `over` until the control `key` is shown, goes on to the end of
/// that list, lets the scrolling rest and returns the control's rectangle.
fn scroll_to(ctx: &Context, app: &mut Rocket, over: Pos2, key: &str) -> Rect {
    let wheel = || {
        vec![
            Event::PointerMoved(over),
            Event::MouseWheel {
                unit: MouseWheelUnit::Point,
                delta: vec2(0.0, -3_000.0),
                modifiers: Modifiers::NONE,
                phase: TouchPhase::Move,
            },
        ]
    };
    for _ in 0..600 {
        window_frame(ctx, app, WINDOW, wheel());
        if app.frame_widgets.contains_key(key) {
            break;
        }
    }
    for _ in 0..5 {
        window_frame(ctx, app, WINDOW, wheel());
    }
    for _ in 0..40 {
        window_frame(ctx, app, WINDOW, vec![]);
    }
    app.frame_widgets
        .get(key)
        .unwrap_or_else(|| panic!("{key} was not reached by scrolling"))
        .rect
}

fn click_at(ctx: &Context, app: &mut Rocket, pos: Pos2) {
    let press = |pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    window_frame(ctx, app, WINDOW, vec![Event::PointerMoved(pos), press(true)]);
    window_frame(ctx, app, WINDOW, vec![press(false)]);
}

fn press_key(ctx: &Context, app: &mut Rocket, key: Key) {
    for pressed in [true, false] {
        window_frame(
            ctx,
            app,
            WINDOW,
            vec![Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE }],
        );
    }
}

fn change_key(entry: &StatusEntry) -> String {
    let path: String = entry.path.as_bytes().iter().map(|b| format!("{b:02x}")).collect();
    format!("change-unstaged-{path}")
}

#[test]
fn long_change_list_reaches_its_last_file_and_keeps_a_focused_row_working_out_of_view() {
    common::isolated(
        "ROCKET_CHANGES_LAYOUT_CASE",
        "changes_tests::long_change_list_reaches_its_last_file_and_keeps_a_focused_row_working_out_of_view",
        || {
            let fixture = common::Fixture::new();
            let ctx = Context::default();
            let mut app = large_lists(&ctx, &fixture, 5_000, 0, 0);
            app.tabs[0].view = View::Changes;
            for _ in 0..3 {
                window_frame(&ctx, &mut app, WINDOW, vec![]);
            }
            let entries = app.tabs[0].snapshot.as_ref().unwrap().status.as_ref().unwrap().entries.clone();
            let (first, last) = (change_key(&entries[0]), change_key(&entries[4_999]));
            assert!(!app.frame_widgets.contains_key(&last));
            // Keyboard focus reaches the first file row, which then scrolls out of view.
            let row = app.frame_widgets[&first].rect;
            let focused_row = |ctx: &Context| {
                ctx.memory(|memory| memory.focused())
                    .and_then(|id| ctx.read_response(id))
                    .is_some_and(|r| row.contains(r.rect.center()))
            };
            for _ in 0..400 {
                if focused_row(&ctx) {
                    break;
                }
                press_key(&ctx, &mut app, Key::Tab);
            }
            assert!(focused_row(&ctx), "Tab did not reach the first file row");
            let focused = ctx.memory(|memory| memory.focused());
            let over = app.frame_widgets["changes-scroll"].rect.center();
            let rect = scroll_to(&ctx, &mut app, over, &last);
            assert!(!app.frame_widgets.contains_key(&first));
            assert_eq!(ctx.memory(|memory| memory.focused()), focused, "the row out of view lost focus");
            press_key(&ctx, &mut app, Key::Enter);
            assert_eq!(app.tabs[0].selected_change, Some((entries[0].path.clone(), false)));
            // The last file is reached by scrolling and takes a click.
            click_at(&ctx, &mut app, rect.center());
            assert_eq!(app.tabs[0].selected_change, Some((entries[4_999].path.clone(), false)));
        },
    );
}

#[test]
fn long_branch_lists_reach_and_select_their_last_branch() {
    common::isolated(
        "ROCKET_CHANGES_LAYOUT_CASE",
        "changes_tests::long_branch_lists_reach_and_select_their_last_branch",
        || {
            // Branches page: the last remote-tracking branch, below every local one.
            let fixture = common::Fixture::new();
            let ctx = Context::default();
            let mut app = large_lists(&ctx, &fixture, 1, 1_500, 1_500);
            app.tabs[0].view = View::Branches;
            for _ in 0..3 {
                window_frame(&ctx, &mut app, WINDOW, vec![]);
            }
            let branches = &app.tabs[0].snapshot.as_ref().unwrap().branches;
            let (index, reference) = (branches.len() - 1, branches.last().unwrap().reference.clone());
            let key = format!("branch-row-{index}");
            assert!(!app.frame_widgets.contains_key(&key));
            let over = app
                .frame_widgets
                .iter()
                .find(|(name, _)| name.starts_with("branch-row-"))
                .expect("the page shows its first branches")
                .1
                .rect
                .center();
            let rect = scroll_to(&ctx, &mut app, over, &key);
            click_at(&ctx, &mut app, rect.center());
            let selected: Option<Vec<u8>> = ctx.data(|data| data.get_temp(Id::new(("branches-selected", app.tabs[0].id))));
            assert_eq!(selected, Some(reference));

            // Sidebar: the last local branch opens its history.
            let fixture = common::Fixture::new();
            let ctx = Context::default();
            let mut app = large_lists(&ctx, &fixture, 1, 3_000, 0);
            app.tabs[0].view = View::Changes;
            for _ in 0..3 {
                window_frame(&ctx, &mut app, WINDOW, vec![]);
            }
            let key = "sidebar-branch-refs/heads/topic/02999";
            assert!(!app.frame_widgets.contains_key(key));
            let over = app
                .frame_widgets
                .iter()
                .find(|(name, _)| name.starts_with("sidebar-branch-"))
                .expect("the sidebar shows its first branches")
                .1
                .rect
                .center();
            let rect = scroll_to(&ctx, &mut app, over, key);
            click_at(&ctx, &mut app, rect.center());
            assert_eq!(app.tabs[0].view, View::Commits);
        },
    );
}

#[test]
#[ignore = "manual before/after timing probe"]
fn measure_large_list_frames() {
    common::isolated(
        "ROCKET_CHANGES_LAYOUT_CASE",
        "changes_tests::measure_large_list_frames",
        || {
            for (label, files, local, tracking, view) in [
                ("small_repository", 1, 0, 0, View::Changes),
                ("changes_20000_files", 20_000, 0, 0, View::Changes),
                ("sidebar_5000_local_branches", 1, 5_000, 0, View::Changes),
                ("sidebar_5000_remote_branches", 1, 0, 5_000, View::Changes),
                ("branches_page_10000_branches", 1, 5_000, 5_000, View::Branches),
            ] {
                let fixture = common::Fixture::new();
                let ctx = Context::default();
                let mut app = large_lists(&ctx, &fixture, files, local, tracking);
                app.tabs[0].view = view;
                let (mut interface, mut triangles) = (Vec::new(), Vec::new());
                for iteration in 0..9 {
                    let (a, b) = window_frame(&ctx, &mut app, vec2(1480.0, 920.0), vec![]);
                    if iteration >= 2 {
                        interface.push(a.as_micros());
                        triangles.push(b.as_micros());
                    }
                }
                interface.sort_unstable();
                triangles.sort_unstable();
                println!(
                    "{label}_frame_median_us interface={} tessellation={}",
                    interface[interface.len() / 2],
                    triangles[triangles.len() / 2]
                );
            }
        },
    );
}

/// An object id that starts differently for each `n`, as real ones do.
fn object_id(n: usize) -> ObjectId {
    let a = (n as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let b = a.rotate_left(29) ^ 0xD6E8_FEB8_6659_FD93;
    ObjectId::parse(&format!("{a:016x}{b:016x}{:08x}", n as u32)).unwrap()
}

/// Appends `count` older commits to the loaded history, every tenth a merge, as
/// "Load more" pages would, and returns how long laying out the graph took.
fn older_commits(app: &mut Rocket, count: usize) -> Duration {
    let tab = &mut app.tabs[0];
    let history = tab.snapshot.as_mut().unwrap().history.as_mut().unwrap();
    let template = history.commits.last().unwrap().clone();
    if count > 0 {
        history.commits.last_mut().unwrap().parents = vec![object_id(0)];
    }
    history.commits.extend((0..count).map(|n| Commit {
        id: object_id(n),
        parents: if n % 10 == 9 {
            vec![object_id(n + 1), object_id(n + 4)]
        } else {
            vec![object_id(n + 1)]
        },
        message: format!("Change number {n}: adjust the layout of module {}\n", n % 200).into_bytes(),
        ..template.clone()
    }));
    let started = Instant::now();
    tab.relayout();
    started.elapsed()
}

/// Makes the shown commit's patch one that changes a line in each of `files` files.
fn many_file_commit(app: &mut Rocket, files: usize) {
    let patch: String = (0..files)
        .map(|n| {
            let path = format!("src/module{:03}/file{n:05}.rs", n % 200);
            format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -1 +1 @@\n-fn old() {{}}\n+fn changed() {{}}\n")
        })
        .collect();
    app.tabs[0].patch = Some(crate::diff::Patch::parse(patch.as_bytes()));
    app.tabs[0].patch_file = 0;
}

#[test]
fn commit_details_reach_and_select_the_last_of_many_changed_files() {
    common::isolated(
        "ROCKET_CHANGES_LAYOUT_CASE",
        "changes_tests::commit_details_reach_and_select_the_last_of_many_changed_files",
        || {
            let fixture = common::Fixture::new();
            let ctx = Context::default();
            let mut app = large_lists(&ctx, &fixture, 1, 0, 0);
            many_file_commit(&mut app, 3_000);
            app.tabs[0].view = View::Commits;
            for _ in 0..3 {
                window_frame(&ctx, &mut app, WINDOW, vec![]);
            }
            assert!(app.frame_widgets.contains_key("detail-files"), "the details pane lists the files");
            assert!(!app.frame_widgets.contains_key("patch-file-2999"));
            let over = app.frame_widgets["patch-file-0"].rect.center();
            let rect = scroll_to(&ctx, &mut app, over, "patch-file-2999");
            click_at(&ctx, &mut app, rect.center());
            assert_eq!(app.tabs[0].patch_file, 2_999);
        },
    );
}

#[test]
fn history_rows_get_the_branches_a_search_of_every_branch_finds() {
    common::isolated(
        "ROCKET_CHANGES_LAYOUT_CASE",
        "changes_tests::history_rows_get_the_branches_a_search_of_every_branch_finds",
        || {
            let fixture = common::Fixture::new();
            let ctx = Context::default();
            let mut app = large_lists(&ctx, &fixture, 1, 600, 600);
            older_commits(&mut app, 300);
            // Several branches share a commit, others sit on commits that are not loaded,
            // and one on a shown commit is a symbolic ref, which gets no chip.
            let branches = &mut app.tabs[0].snapshot.as_mut().unwrap().branches;
            for (n, branch) in branches.iter_mut().filter(|b| !b.is_head).enumerate() {
                branch.target = object_id(n % 400);
            }
            branches[31].symbolic_target = Some(b"refs/remotes/origin/main".to_vec());
            let symbolic = branches[31].target.clone();
            let tab = &app.tabs[0];
            let branches = &tab.snapshot.as_ref().unwrap().branches;
            let shown = &tab.commits()[20..80];
            assert!(shown.iter().any(|c| c.id == symbolic));
            let tips = crate::views::branch_tips(shown, branches);
            let mut shared = 0;
            for commit in shown {
                let expected: Vec<&Branch> = branches
                    .iter()
                    .filter(|b| b.target == commit.id && b.symbolic_target.is_none())
                    .collect();
                let found = tips.get(&commit.id).map_or(&[][..], Vec::as_slice);
                assert_eq!(found.len(), expected.len(), "branches on {}", commit.id);
                assert!(found.iter().zip(&expected).all(|(a, b)| std::ptr::eq(*a, *b)), "order on {}", commit.id);
                shared += usize::from(expected.len() > 1);
            }
            assert!(shared > 10, "only {shared} shown commits carry several branches");
            assert!(!tips.contains_key(&tab.commits()[100].id), "a commit that is not shown");
        },
    );
}

#[test]
fn sidebar_lists_and_filters_remote_branches_whatever_their_names() {
    common::isolated(
        "ROCKET_CHANGES_LAYOUT_CASE",
        "changes_tests::sidebar_lists_and_filters_remote_branches_whatever_their_names",
        || {
            let fixture = common::Fixture::new();
            let ctx = Context::default();
            let mut app = large_lists(&ctx, &fixture, 1, 0, 4);
            // A name that is not valid UTF-8 is shown with a replacement character.
            let branches = &mut app.tabs[0].snapshot.as_mut().unwrap().branches;
            let mut odd = branches.last().unwrap().clone();
            odd.reference = b"refs/remotes/origin/caf\xe9".to_vec();
            branches.push(odd);
            app.tabs[0].view = View::Changes;
            let key = |name: &str| format!("sidebar-branch-refs/remotes/origin/{name}");
            let shown = |app: &mut Rocket, filter: &str| {
                app.tabs[0].branch_filter = filter.into();
                for _ in 0..3 {
                    window_frame(&ctx, app, WINDOW, vec![]);
                }
                ["topic/00000", "topic/00002", "topic/00003", "caf\u{fffd}"]
                    .map(|name| app.frame_widgets.contains_key(&key(name)))
            };
            assert_eq!(shown(&mut app, ""), [true, true, true, true]);
            assert_eq!(shown(&mut app, "CAF"), [false, false, false, true]);
            assert_eq!(shown(&mut app, "topic/00002"), [false, true, false, false]);
            assert_eq!(shown(&mut app, "origin/topic"), [true, true, true, false]);
        },
    );
}

#[test]
#[ignore = "manual before/after timing probe"]
fn measure_history_frames() {
    common::isolated(
        "ROCKET_CHANGES_LAYOUT_CASE",
        "changes_tests::measure_history_frames",
        || {
            for (label, commits, local, tracking, files) in [
                ("history_one_commit", 0, 0, 0, 0),
                ("history_200_commits", 199, 0, 0, 0),
                ("history_50000_commits", 50_000, 0, 0, 0),
                ("history_2000_commits_10000_branches", 2_000, 5_000, 5_000, 0),
                ("history_200_commits_shown_commit_changes_8000_files", 199, 0, 0, 8_000),
            ] {
                let fixture = common::Fixture::new();
                let ctx = Context::default();
                let mut app = large_lists(&ctx, &fixture, 1, local, tracking);
                let layout = older_commits(&mut app, commits);
                if files > 0 {
                    many_file_commit(&mut app, files);
                }
                // Branches sit on commits five apart, most of them older than the loaded ones.
                let branches = &mut app.tabs[0].snapshot.as_mut().unwrap().branches;
                for (n, branch) in branches.iter_mut().filter(|b| !b.is_head).enumerate() {
                    branch.target = object_id(n * 5);
                }
                app.tabs[0].view = View::Commits;
                let times = |app: &mut Rocket, frames: usize, events: &dyn Fn() -> Vec<Event>| {
                    let mut all: Vec<u128> = (0..frames)
                        .map(|_| {
                            let (a, b) = window_frame(&ctx, app, WINDOW, events());
                            (a + b).as_micros()
                        })
                        .collect();
                    let slowest = *all.iter().max().unwrap();
                    all.sort_unstable();
                    (all[all.len() / 2], slowest)
                };
                times(&mut app, 3, &Vec::new);
                let (idle, _) = times(&mut app, 7, &Vec::new);
                let head = app.tabs[0].commits()[0].id.clone();
                let over = app.frame_widgets[&format!("commit-{head}")].rect.center();
                let (scrolling, slowest) = times(&mut app, 30, &|| {
                    vec![
                        Event::PointerMoved(over),
                        Event::MouseWheel {
                            unit: MouseWheelUnit::Point,
                            delta: vec2(0.0, -700.0),
                            modifiers: Modifiers::NONE,
                            phase: TouchPhase::Move,
                        },
                    ]
                });
                println!(
                    "{label} graph_layout_ms={} idle_frame_us={idle} scrolling_frame_median_us={scrolling} scrolling_frame_max_us={slowest} details_file_list_shown={}",
                    layout.as_millis(),
                    app.frame_widgets.contains_key("detail-files")
                );
            }
        },
    );
}

#[test]
fn image_comparison_keeps_both_previews_visible_above_terminal() {
    common::isolated(
        "ROCKET_CHANGES_LAYOUT_CASE",
        "changes_tests::image_comparison_keeps_both_previews_visible_above_terminal",
        || {
            let fixture = common::Fixture::new();
            let path = fixture.repository.join("画像 é.png");
            image::RgbaImage::from_pixel(320, 200, image::Rgba([30, 100, 200, 160]))
                .save(&path)
                .unwrap();
            fixture.commit("image baseline");
            image::RgbaImage::from_pixel(400, 260, image::Rgba([50, 140, 220, 200]))
                .save(&path)
                .unwrap();
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
            app.tabs[0].view = View::Changes;
            let entry = app.tabs[0]
                .snapshot
                .as_ref()
                .unwrap()
                .status
                .as_ref()
                .unwrap()
                .entries[0]
                .clone();
            app.dispatch(Intent::SelectChange(entry, false));
            let frame = |app: &mut Rocket, size| {
                app.poll(&ctx);
                app.frame_widgets.clear();
                let mut output = ctx.run_ui(
                    RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ui| app.shell(ui, &mut Vec::new()),
                );
                output.textures_delta.clear();
            };
            while !app.tabs[0].image.loaded {
                frame(&mut app, vec2(1480.0, 920.0));
                assert!(
                    Instant::now() < deadline,
                    "Image did not load: {:?}",
                    app.tabs[0].image.error
                );
                thread::sleep(Duration::from_millis(10));
            }
            app.open_terminal();
            for size in [vec2(1480.0, 920.0), vec2(780.0, 620.0), vec2(900.0, 700.0)] {
                for _ in 0..4 {
                    frame(&mut app, size);
                }
                // The drawer's arrow sits 5 px below the top of the terminal panel.
                let terminal_top = app.frame_widgets["drawer-collapse"].rect.top() - 5.0;
                for id in [
                    "image-fit",
                    "image-checker",
                    "image-reload",
                    "image-side-0",
                    "image-side-1",
                ] {
                    let control = app
                        .frame_widgets
                        .get(id)
                        .unwrap_or_else(|| panic!("{id} missing at {size:?}"));
                    assert!(
                        control.rect.bottom() <= terminal_top,
                        "{id} overlaps terminal at {size:?}: {:?}",
                        control.rect
                    );
                    if id.starts_with("image-side") {
                        assert!(
                            control.rect.height() >= 24.0,
                            "{id} has no usable thumbnail at {size:?}: {:?}",
                            control.rect
                        );
                    }
                }
            }
        },
    );
}

#[test]
fn commit_message_opens_at_seven_lines_follows_the_text_and_keeps_a_dragged_height() {
    common::isolated(
        "ROCKET_COMMIT_AREA_CASE",
        "changes_tests::commit_message_opens_at_seven_lines_follows_the_text_and_keeps_a_dragged_height",
        || {
            let fixture = common::Fixture::new();
            fixture.write("file.txt", "base\n");
            fixture.commit("base");
            fixture.write("file.txt", "changed\n");
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
            app.tabs[0].view = View::Changes;
            let message = |lines: usize| {
                let body: Vec<String> = (1..=lines).map(|i| format!("line {i}")).collect();
                format!("Summary\n\n{}", body.join("\n"))
            };
            let row = std::cell::Cell::new(0.0f32);
            // The description's height in lines of its font, after the area has
            // measured itself.
            let lines = |app: &mut Rocket, size: Vec2, draft: &str| -> f32 {
                app.tabs[0].draft = draft.into();
                for _ in 0..4 {
                    app.poll(&ctx);
                    app.frame_widgets.clear();
                    let mut output = ctx.run_ui(
                        RawInput {
                            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                            ..Default::default()
                        },
                        |ui| {
                            row.set(ui.text_style_height(&TextStyle::Body));
                            app.shell(ui, &mut Vec::new());
                        },
                    );
                    output.textures_delta.clear();
                }
                (app.frame_widgets["commit-message-scroll"].rect.height() / row.get()).round()
            };
            let wide = vec2(1480.0, 960.0);
            assert_eq!(lines(&mut app, wide, &message(1)), 7.0, "opens at 7 lines");
            assert_eq!(lines(&mut app, wide, &message(9)), 9.0, "follows the text");
            assert_eq!(lines(&mut app, wide, &message(40)), 12.0, "up to 12, then scrolls");
            assert!(app.frame_widgets.contains_key("commit-resize"), "the handle is there");
            assert!(app.frame_widgets["commit-message"].rect.height() >= 36.0);
            assert_eq!(lines(&mut app, vec2(780.0, 620.0), &message(1)), 4.0, "a short pane opens at 4");
            // A long message in a short pane takes the room left, in whole lines.
            let short = app.frame_widgets["commit-message-scroll"].rect.height();
            let long = lines(&mut app, vec2(780.0, 620.0), &message(9));
            let exact = app.frame_widgets["commit-message-scroll"].rect.height() / row.get();
            assert!(long > 4.0 && (exact - exact.round()).abs() < 0.05, "{long} lines, {exact} exactly, from {short}");
            // A dragged height stays, whatever the text.
            app.commit_height = Some(520.0);
            let dragged = lines(&mut app, wide, &message(1));
            assert!(dragged > 12.0, "{dragged}");
            assert_eq!(lines(&mut app, wide, &message(40)), dragged);
        },
    );
}

#[test]
fn changes_controls_stay_inside_short_repository_pane_with_long_drafts() {
    common::isolated(
        "ROCKET_CHANGES_LAYOUT_CASE",
        "changes_tests::changes_controls_stay_inside_short_repository_pane_with_long_drafts",
        || {
            let fixture = common::Fixture::new();
            fixture.write(
                "file.txt",
                "// baseline\nfn main() {\n    println!(\"base\");\n}\n",
            );
            fixture.commit("base");
            fixture.write(
                "file.txt",
                "// baseline\nfn main() {\n    println!(\"working 中文\");\n}\n",
            );
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
            app.tabs[0].view = View::Changes;
            let entry = app.tabs[0]
                .snapshot
                .as_ref()
                .unwrap()
                .status
                .as_ref()
                .unwrap()
                .entries[0]
                .clone();
            app.dispatch(Intent::SelectChange(entry, false));
            while app.busy() {
                app.poll(&ctx);
                assert!(Instant::now() < deadline);
                thread::sleep(Duration::from_millis(10));
            }
            app.tabs[0].draft = "A line of a long Unicode draft 中文 é\n".repeat(40);
            app.open_terminal();
            for size in [vec2(1480.0, 920.0), vec2(780.0, 620.0), vec2(900.0, 700.0)] {
                for _ in 0..3 {
                    app.poll(&ctx);
                    app.frame_widgets.clear();
                    let mut output = ctx.run_ui(
                        RawInput {
                            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                            ..Default::default()
                        },
                        |ui| {
                            app.shell(ui, &mut Vec::new());
                        },
                    );
                    output.textures_delta.clear();
                }
                // Compare with the actual terminal controls: panel sizes can
                // change with content, font metrics and the previous window size.
                // The drawer's arrow sits 5 px below the top of the terminal panel.
                let terminal_top = app.frame_widgets["drawer-collapse"].rect.top() - 5.0;
                assert!(
                    app.frame_widgets["diff-search"].rect.bottom() < terminal_top,
                    "Search clipped at {size:?}"
                );
                assert!(
                    app.frame_widgets
                        .iter()
                        .any(|(key, c)| key.starts_with("diff-line-")
                            && c.rect.height() >= 18.0
                            && c.rect.bottom() <= terminal_top),
                    "No usable diff row with terminal open at {size:?}; terminal top={terminal_top}; controls={:?}",
                    app.frame_widgets
                        .iter()
                        .filter(|(k, _)| k.starts_with("diff-") || k.starts_with("change-"))
                        .map(|(k, c)| (k, c.rect))
                        .collect::<Vec<_>>()
                );
                if size.x > 1000.0 {
                    assert!(app.frame_widgets["commit-message"].rect.height() >= 30.0);
                }
                for id in ["commit-message", "commit"] {
                    if let Some(control) = app.frame_widgets.get(id) {
                        assert!(
                            control.rect.bottom() <= terminal_top,
                            "{id} extends into terminal at {size:?}: {:?}",
                            control.rect
                        );
                    }
                }
            }
        },
    );
}
