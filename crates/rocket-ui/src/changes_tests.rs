use crate::test_common as common;

use crate::app::{Intent, Rocket, View};
use eframe::egui::*;
use std::{
    thread,
    time::{Duration, Instant},
};

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
