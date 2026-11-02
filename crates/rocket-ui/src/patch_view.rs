use crate::{
    app::{DialogKind, Intent, Tab, UiControl},
    backend::Action,
    diff::{FilePatch, Kind, Line},
    style::{self, *},
};
use eframe::egui::*;
use std::collections::{BTreeSet, HashMap};

pub(crate) fn patch_view(
    ui: &mut Ui,
    tab: &mut Tab,
    file_list: bool,
    writable: bool,
    widgets: &mut HashMap<String, UiControl>,
) -> Option<Intent> {
    tab.diff_tools.overview = Default::default();
    tab.diff_tools.follow(crate::general::options(ui.ctx()));
    // Placeholder states sit inside the pane's margins, not against its dividers.
    let placeholder = Frame::new().inner_margin(Margin::symmetric(14, 12));
    // After a background refresh the same selection's previous diff stays on screen,
    // read-only, until the reload arrives, instead of a spinner every time a file
    // changes. Image comparisons load their own previews, so they still wait.
    let reloading = tab.diff.is_some()
        && tab
            .patch
            .as_ref()
            .and_then(|p| p.files.get(tab.patch_file))
            .is_some_and(|f| f.image.is_none());
    let writable = writable && !reloading;
    if tab.diff.is_some() && !reloading {
        let mut stop = false;
        crate::pages::loading_with(ui, "Loading diff…", 34.0, |ui| {
            ui.add_space(6.0);
            let r = ui.button("Stop");
            widgets.insert("diff-cancel".into(), (&r).into());
            stop = r.clicked();
        });
        if stop {
            tab.diff = None;
            tab.patch_error = Some("Diff loading cancelled. Refresh or select a file to retry.".into());
        }
        return None;
    }
    if let Some(error) = &tab.patch_error {
        placeholder.show(ui, |ui| ui.colored_label(MUTED(), error));
        return None;
    }
    // While the repository is read, the selected diff loads right after it: one loading
    // state for both, not "select a file" in between.
    let selected = match tab.view {
        crate::app::View::Changes => tab.selected_change.is_some(),
        _ => tab.selected_commit.is_some(),
    };
    let waiting = tab.loading() && selected;
    let Some(patch) = &mut tab.patch else {
        if waiting {
            crate::pages::loading(ui, "Loading diff…");
        } else {
            placeholder.show(ui, |ui| ui.colored_label(MUTED(), "Select a file to view its diff."));
        }
        return None;
    };
    if patch.limited {
        ui.colored_label(
            AMBER(),
            if tab.view == crate::app::View::Compare {
                "Preview limit reached. The file list is complete; this diff is truncated."
            } else {
                "Preview limit reached. Review the complete file before staging."
            },
        );
    }
    if let Some(notice) = &patch.notice
        && !patch
            .files
            .get(tab.patch_file)
            .is_some_and(|f| f.image.is_some())
    {
        ui.colored_label(MUTED(), notice);
    }
    if patch.files.is_empty() {
        ui.colored_label(MUTED(), "No patch content for this selection.");
        return None;
    }
    tab.patch_file = tab.patch_file.min(patch.files.len() - 1);
    if file_list {
        Panel::left(Id::new(("patch-files", tab.id)))
            .default_size(220.0)
            .size_range(140.0..=(ui.available_width() * 0.5).max(140.0))
            .resizable(true)
            .frame(style::frame())
            .show(ui, |ui| {
                ui.label(format!("{} files changed", patch.files.len()));
                ui.separator();
                // Offscreen rows are absent, so carry keyboard focus into the
                // newly visible row on the next frame. Reject old diff requests.
                let focus_id = ui.id().with("patch-files-focus");
                let generation = patch.files[0].id;
                let focus = ui.data_mut(|data| data.remove_temp::<(u64, usize, bool)>(focus_id))
                    .filter(|(id, row, _)| *id == generation && *row < patch.files.len())
                    .map(|(_, row, scroll)| (row, scroll));
                let mut scroll = ScrollArea::vertical().id_salt("patch-files-scroll");
                if let Some((row, true)) = focus {
                    scroll = scroll.vertical_scroll_offset(row as f32 * (32.0 + ui.spacing().item_spacing.y));
                }
                scroll
                    .show_rows(ui, 32.0, patch.files.len(), |ui, rows| {
                        let visible_rows = rows.clone();
                        for i in rows {
                            let file = &patch.files[i];
                            let r = ui
                                .add_sized(
                                    [ui.available_width(), 32.0],
                                    Button::new(format!(
                                        "{}    +{}  −{}",
                                        file.label, file.added, file.removed
                                    ))
                                    .truncate()
                                    .selected(i == tab.patch_file),
                                )
                                .on_hover_text(&file.label);
                            if focus.is_some_and(|(row, _)| row == i) {
                                r.request_focus();
                                r.scroll_to_me(None);
                            }
                            if r.has_focus() {
                                let next = ui.input_mut(|input| {
                                    if input.consume_key(Modifiers::NONE, Key::Home) { Some(0) }
                                    else if input.consume_key(Modifiers::NONE, Key::End) { Some(patch.files.len() - 1) }
                                    else if input.consume_key(Modifiers::NONE, Key::ArrowUp) { Some(i.saturating_sub(1)) }
                                    else if input.consume_key(Modifiers::NONE, Key::ArrowDown) { Some((i + 1).min(patch.files.len() - 1)) }
                                    else { None }
                                });
                                if let Some(next) = next {
                                    tab.patch_file = next;
                                    ui.data_mut(|data| data.insert_temp(focus_id, (generation, next, !visible_rows.contains(&next))));
                                    ui.ctx().request_repaint();
                                }
                            }
                            if r.interact_rect.is_positive() {
                                widgets.insert(
                                    format!("patch-file-{i}"),
                                    UiControl {
                                        rect: r.interact_rect,
                                        enabled: r.enabled(),
                                    },
                                );
                            }
                            if r.clicked() {
                                tab.patch_file = i;
                                r.request_focus();
                            }
                        }
                    });
            });
    }
    let selectable = writable
        && !patch.limited
        && patch
            .review
            .as_ref()
            .is_some_and(|r| r.selection_unavailable().is_none());
    let mut action = None;
    CentralPanel::default()
        .frame(Frame::new().fill(VIEW_BG()).inner_margin(0))
        .show(ui, |ui| {
            let file = &mut patch.files[tab.patch_file];
            if let Some(summary) = &file.submodule {
                tab.image.clear();
                tab.diff_tools.invalidate();
                header_strip(ui, |ui| {
                    ui.label(RichText::new(&file.label).strong());
                    ui.small("Submodule commits · local history only");
                });
                ScrollArea::vertical().id_salt(("submodule-diff", file.id)).show(ui, |ui| {
                    Frame::new().inner_margin(14).show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.monospace(summary.before.as_ref().map_or("Absent", |id| id.as_str()));
                            ui.label("→");
                            ui.monospace(summary.after.as_ref().map_or("Absent", |id| id.as_str()));
                        });
                        if let Some(note) = &summary.notice { ui.colored_label(AMBER(), note); }
                        if summary.commits.is_empty() && summary.notice.is_none() { ui.label("No commit changes; the submodule may have local file edits."); }
                        for commit in &summary.commits {
                            ui.horizontal_wrapped(|ui| {
                                ui.colored_label(if commit.added { GREEN() } else { RED() }, if commit.added { "+ Added" } else { "− Removed" });
                                ui.monospace(&commit.id.as_str()[..9]);
                                ui.label(String::from_utf8_lossy(&commit.subject));
                            });
                            ui.separator();
                        }
                        ui.add_space(10.0);
                        ui.small("Original gitlink diff");
                        for line in file.lines.iter() { ui.monospace(&line.text); }
                    });
                });
                return;
            }
            if file.image.is_some() {
                tab.diff_tools.invalidate();
                header_strip(ui, |ui| {
                    ui.horizontal(|ui| {
                        if !tab.review_window { action = external_menu(ui, tab.view == crate::app::View::Changes, widgets); }
                        ui.add(Label::new(RichText::new(&file.label).strong()).truncate())
                            .on_hover_text(&file.label);
                    });
                });
                if let Some(snapshot) = &tab.snapshot
                    && tab.image.ui(ui, file, &snapshot.repository, widgets)
                {
                    action = Some(Intent::ReloadImage);
                }
                return;
            }
            tab.image.clear();
            let compact = ui.available_width() < 440.0 || ui.available_height() < 300.0;
            header_strip(ui, |ui| {
                if compact {
                    ui.spacing_mut().interact_size.y = 22.0;
                    ui.spacing_mut().button_padding = vec2(6.0, 2.0);
                    ui.spacing_mut().item_spacing = vec2(4.0, 3.0);
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            [(ui.available_width() - 112.0).max(40.0), 18.0],
                            Label::new(&file.label).truncate(),
                        )
                        .on_hover_text(&file.label);
                        ui.colored_label(GREEN(), format!("+{}", file.added));
                        ui.colored_label(RED(), format!("−{}", file.removed));
                        if !tab.review_window { action = external_menu(ui, tab.view == crate::app::View::Changes, widgets); }
                    });
                }
                ui.horizontal_wrapped(|ui| {
                    if !compact {
                        if !tab.review_window { action = external_menu(ui, tab.view == crate::app::View::Changes, widgets); }
                        ui.label(&file.label);
                        ui.colored_label(GREEN(), format!("+{}", file.added));
                        ui.colored_label(RED(), format!("−{}", file.removed));
                        ui.separator();
                    }
                    for (split, label, key) in [
                        (false, "Unified", "diff-unified"),
                        (
                            true,
                            if compact { "Split" } else { "Side by side" },
                            "diff-split",
                        ),
                    ] {
                        let r = ui.selectable_label(tab.split_diff == split, label);
                        widgets.insert(key.into(), (&r).into());
                        if r.clicked() {
                            tab.split_diff = split;
                        }
                    }
                    if let Some(review) = &patch.review {
                        let label = if review.staged() {
                            if compact {
                                "Unstage"
                            } else {
                                "Unstage selected"
                            }
                        } else {
                            if compact { "Stage" } else { "Stage selected" }
                        };
                        let r = ui.add_enabled(
                            selectable && !patch.selected.is_empty(),
                            Button::new(format!("{label} ({})", patch.selected.len())),
                        );
                        widgets.insert("diff-apply-selection".into(), (&r).into());
                        if r.clicked() {
                            action = Some(Intent::Execute(
                                tab.id,
                                Action::ApplyReview(
                                    review.clone(),
                                    patch.selected.iter().copied().collect(),
                                ),
                            ));
                        }
                        if !review.staged() && review.supports_worktree_selection() {
                            let r=ui.add_enabled(selectable&&!patch.selected.is_empty(),Button::new("Stash selected"));
                            widgets.insert("diff-stash-selection".into(),(&r).into());
                            if r.clicked(){action=Some(Intent::Dialog(DialogKind::Patch(crate::patches::Mode::Stash(review.clone(),patch.selected.iter().copied().collect()))));}
                            let r = ui.add_enabled(
                                selectable && !patch.selected.is_empty(),
                                Button::new(if compact {
                                    "Discard"
                                } else {
                                    "Discard selected"
                                }),
                            );
                            widgets.insert("diff-discard-selection".into(), (&r).into());
                            if r.clicked() {
                                action = Some(Intent::Dialog(DialogKind::Patch(
                                    crate::patches::Mode::Discard(
                                        review.clone(),
                                        patch.selected.iter().copied().collect(),
                                    ),
                                )));
                            }
                        }
                        if selectable && !compact {
                            ui.small("Click lines or a hunk to select.");
                        }
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    let r = ui.add(
                        TextEdit::singleline(&mut tab.diff_tools.query)
                            .hint_text(if compact {
                                "Find…"
                            } else {
                                "Find in this diff…"
                            })
                            .char_limit(512)
                            .desired_width(if compact { 80.0 } else { 180.0 }),
                    );
                    widgets.insert("diff-search".into(), (&r).into());
                    if !ui.ctx().egui_wants_keyboard_input()
                        && ui.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::F))
                    {
                        r.request_focus();
                    }
                    let r = ui
                        .add(checkbox(
                            &mut tab.diff_tools.syntax,
                            if compact { "Aa" } else { "Syntax" },
                        ))
                        .on_hover_text(format!("{} · lexical colors from available diff context.{}",
                            file.syntax_language.unwrap_or("Plain text"),
                            if file.syntax_limited { " Highlighting limit reached; remaining lines use plain colors." } else { "" }));
                    widgets.insert("diff-syntax".into(), (&r).into());
                    if tab.diff_tools.words_running() {
                        // Reuse the toggle's row so background work cannot push the
                        // patch out of a short pane while its highlights load.
                        let r = ui.button("Words ×").on_hover_text("Stop word highlights");
                        widgets.insert("diff-words-cancel".into(), (&r).into());
                        if r.clicked() {
                            tab.diff_tools.stop_words();
                        }
                    } else {
                        let r = ui
                            .add(checkbox(&mut tab.diff_tools.words, "Words"))
                            .on_hover_text("Highlight changed words within paired lines");
                        widgets.insert("diff-words".into(), (&r).into());
                    }
                    for (forward, label, key) in [
                        (false, "↑", "diff-search-prev"),
                        (true, "↓", "diff-search-next"),
                    ] {
                        let r = ui.add_enabled(!tab.diff_tools.hits.is_empty(), Button::new(label));
                        widgets.insert(key.into(), (&r).into());
                        if r.clicked() {
                            tab.diff_tools.navigate(forward);
                        }
                    }
                    let r = ui
                        .button(if compact { "×" } else { "Clear" })
                        .on_hover_text("Clear search");
                    widgets.insert("diff-search-clear".into(), (&r).into());
                    if r.clicked() {
                        tab.diff_tools.query.clear();
                    }
                    if !tab.diff_tools.query.is_empty() {
                        ui.small(if tab.diff_tools.running() {
                            "Searching…".into()
                        } else if tab.diff_tools.hits.is_empty() {
                            "No matches".into()
                        } else {
                            format!(
                                "{} / {}{}",
                                tab.diff_tools.current + 1,
                                tab.diff_tools.hits.len(),
                                if tab.diff_tools.limited {
                                    "+ (limit)"
                                } else {
                                    ""
                                }
                            )
                        });
                    }
                    if ui.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::F3)) {
                        tab.diff_tools.navigate(false);
                    } else if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F3)) {
                        tab.diff_tools.navigate(true);
                    }
                });
                tab.diff_tools.update(file, ui.ctx());
                if tab.diff_tools.syntax && file.syntax_limited {
                    ui.small("Syntax limit reached; remaining lines use plain colors.");
                }
                if tab.diff_tools.word_highlights.limited {
                    ui.small("Word detail limit reached; remaining changes use line highlights.");
                }
                if let Some(error) = &tab.diff_tools.word_error {
                    ui.colored_label(AMBER(), error);
                }
                if let Some(error) = &tab.diff_tools.error {
                    ui.colored_label(AMBER(), error);
                }
                if tab.diff_tools.running() {
                    let r = ui.button("Stop search");
                    widgets.insert("diff-search-cancel".into(), (&r).into());
                    if r.clicked() {
                        tab.diff_tools.cancel();
                        tab.diff_tools.error =
                            Some("Search cancelled. Change the query to search again.".into());
                    }
                }
            });
            ui.add_space(4.0);
            let header = tab.split_diff.then(|| {
                ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover())
                    .0
            });
            ui.spacing_mut().item_spacing.y = 0.0;
            let count = if tab.split_diff {
                file.split.len()
            } else {
                file.visible.len()
            };
            let mode = usize::from(tab.split_diff);
            let jump = if std::mem::take(&mut tab.diff_tools.jump) {
                tab.diff_tools
                    .hits
                    .get(tab.diff_tools.current)
                    .and_then(|hit| {
                        if tab.split_diff {
                            file.split
                                .iter()
                                .position(|(a, b)| *a == Some(hit.line) || *b == Some(hit.line))
                        } else {
                            file.visible.iter().position(|i| *i == hit.line)
                        }
                    })
            } else {
                None
            };
            let mut area = ScrollArea::both().id_salt((
                "patch-content",
                tab.id,
                tab.patch_file,
                tab.split_diff,
            ));
            let row_height = tab.diff_tools.row_height();
            if let Some(row) = jump {
                area = area.vertical_scroll_offset(row as f32 * row_height);
            }
            let available = ui.available_size();
            let mut rail = Rect::NOTHING;
            let mut scroll = ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let scroll = ui.allocate_ui_with_layout(
                    vec2((available.x - crate::overview::WIDTH - 4.0).max(40.0), available.y),
                    Layout::top_down(Align::Min), |ui| area
                .id_salt(("patch-content", tab.id, tab.patch_file, tab.split_diff))
                .auto_shrink([false, false])
                .show_rows(ui, row_height, count, |ui, range| {
                    let mut measured_width = file.widths[mode];
                    let min_width = file.widths[mode];
                    ui.set_min_width(min_width.max(ui.available_width()));
                    for row in range {
                        let (rect, _) = ui
                            .allocate_exact_size(vec2(ui.available_width(), row_height), Sense::hover());
                        if tab.split_diff {
                            let (old, new) = file.split[row];
                            if old == new
                                && old.is_some_and(|i| {
                                    matches!(file.lines[i].kind, Kind::Hunk | Kind::Header)
                                })
                            {
                                let width = draw(
                                    ui,
                                    rect,
                                    &file.lines[old.unwrap()],
                                    None,
                                    file,
                                    &mut patch.selected,
                                    selectable,
                                    widgets,
                                    &mut tab.diff_tools,
                                );
                                measured_width = measured_width.max(width);
                            } else {
                                let middle = rect.center().x;
                                let left =
                                    Rect::from_min_max(rect.min, pos2(middle, rect.bottom()));
                                let right = Rect::from_min_max(pos2(middle, rect.top()), rect.max);
                                if let Some(i) = old {
                                    let width = draw(
                                        ui,
                                        left,
                                        &file.lines[i],
                                        Some(false),
                                        file,
                                        &mut patch.selected,
                                        selectable,
                                        widgets,
                                        &mut tab.diff_tools,
                                    );
                                    measured_width = measured_width.max(width * 2.0);
                                }
                                if let Some(i) = new {
                                    let width = draw(
                                        ui,
                                        right,
                                        &file.lines[i],
                                        Some(true),
                                        file,
                                        &mut patch.selected,
                                        selectable,
                                        widgets,
                                        &mut tab.diff_tools,
                                    );
                                    measured_width = measured_width.max(width * 2.0);
                                }
                                ui.painter().vline(
                                    middle,
                                    rect.y_range(),
                                    Stroke::new(1.0, BORDER()),
                                );
                            }
                        } else {
                            let width = draw(
                                ui,
                                rect,
                                &file.lines[file.visible[row]],
                                None,
                                file,
                                &mut patch.selected,
                                selectable,
                                widgets,
                                &mut tab.diff_tools,
                            );
                            measured_width = measured_width.max(width);
                        }
                    }
                    if measured_width > file.widths[mode] {
                        // Fallback glyphs are not one ASCII cell wide. Use the
                        // layouts already drawn for visible rows, not a full
                        // patch scan on each frame. The next frame grows the
                        // horizontal scroll range to include the complete text.
                        file.widths[mode] = measured_width;
                        ui.ctx().request_repaint();
                    }
                })).inner;
                rail = ui.allocate_exact_size(vec2(crate::overview::WIDTH, scroll.inner_rect.height()), Sense::hover()).0;
                scroll
            }).inner;
            let view = crate::overview::View {
                offset: scroll.state.offset.y,
                extent: scroll.content_size.y,
                viewport: scroll.inner_rect.height(),
            };
            tab.diff_tools.overview = crate::overview::Probe::new(&file.overview[mode], view);
            if let Some(o) = crate::overview::show(ui, rail,
                Id::new(("diff-overview", tab.id, file.id, mode)), &file.overview[mode], view,
                "Loaded diff: green added, red removed. Click or drag to scroll. Focus here for arrows, Page Up/Down, Home/End.") {
                crate::pages::control(widgets, "diff-overview", &o.response, ui.clip_rect());
                if let Some(target) = o.target {
                    scroll.state.offset.y = target;
                    scroll.state.store(ui.ctx(), scroll.id);
                    ui.ctx().request_repaint();
                }
            }
            if let Some(header) = header {
                // Match the actual scrolling columns, rather than the viewport
                // midpoint, when wide glyphs/lines extend beyond the viewport.
                let left = scroll.inner_rect.left() - scroll.state.offset.x;
                let middle = left + scroll.content_size.x / 2.0;
                let painter = ui
                    .painter()
                    .with_clip_rect(header.intersect(ui.clip_rect()));
                for (x, label) in [(left, "Before"), (middle, "After")] {
                    painter.text(
                        pos2(x, header.center().y),
                        Align2::LEFT_CENTER,
                        label,
                        FontId::proportional(12.0),
                        TEXT(),
                    );
                }
            }
        });
    action
}

#[allow(clippy::too_many_arguments)]
fn draw(
    ui: &mut Ui,
    rect: Rect,
    line: &Line,
    side: Option<bool>,
    file: &FilePatch,
    selected: &mut BTreeSet<usize>,
    selectable: bool,
    widgets: &mut HashMap<String, UiControl>,
    tools: &mut crate::diff_tools::State,
) -> f32 {
    let changed = matches!(line.kind, Kind::Added | Kind::Removed);
    let hunk = line.kind == Kind::Hunk;
    let start = file
        .lines
        .partition_point(|l| l.raw_index <= line.raw_index);
    let hunk_lines = || {
        file.lines[start..]
            .iter()
            .take_while(|l| l.kind != Kind::Hunk)
            .filter(|l| matches!(l.kind, Kind::Added | Kind::Removed))
            .map(|l| l.raw_index)
    };
    let checked = if hunk {
        hunk_lines().all(|i| selected.contains(&i))
    } else {
        selected.contains(&line.raw_index)
    };
    let (fill, color) = match line.kind {
        Kind::Added => (ADDED_ROW(), GREEN()),
        Kind::Removed => (REMOVED_ROW(), TEXT()),
        Kind::Hunk => (VIEW_RAISED(), MUTED()),
        _ => (VIEW_PANEL(), TEXT()),
    };
    let mut hovered = false;
    if selectable && (changed || hunk) {
        let key = format!(
            "diff-{}-{}",
            if hunk { "hunk" } else { "line" },
            line.raw_index
        );
        let r = ui.interact(rect, ui.id().with(&key), Sense::click());
        let visible = r.interact_rect.intersect(ui.clip_rect());
        if visible.is_positive() {
            widgets.insert(
                key,
                UiControl {
                    rect: visible,
                    enabled: r.enabled(),
                },
            );
        }
        hovered = r.hovered();
        if r.clicked() {
            if hunk {
                for i in hunk_lines() {
                    if checked {
                        selected.remove(&i);
                    } else {
                        selected.insert(i);
                    }
                }
            } else if checked {
                selected.remove(&line.raw_index);
            } else {
                selected.insert(line.raw_index);
            }
        }
        r.on_hover_text(if hunk {
            "Select or clear every change in this hunk"
        } else {
            "Select or clear this changed line"
        });
    }
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    painter.rect_filled(
        rect,
        0.0,
        if checked {
            SELECTED()
        } else if hovered {
            RAISED()
        } else {
            fill
        },
    );
    let gutter = if side.is_some() { 80.0 } else { 125.0 };
    if selectable && (changed || hunk) {
        let box_rect =
            Rect::from_center_size(pos2(rect.left() + 12.0, rect.center().y), vec2(9.0, 9.0));
        painter.rect_stroke(box_rect, 1.0, Stroke::new(1.0, BLUE()), StrokeKind::Inside);
        if checked {
            painter.line_segment(
                [
                    box_rect.left_center() + vec2(1.0, 0.0),
                    box_rect.center_bottom() - vec2(4.0, 2.0),
                ],
                Stroke::new(1.3, BLUE()),
            );
            painter.line_segment(
                [
                    box_rect.center_bottom() - vec2(4.0, 2.0),
                    box_rect.right_top() + vec2(-1.0, 2.0),
                ],
                Stroke::new(1.3, BLUE()),
            );
        }
    }
    if !hunk {
        if side != Some(true) {
            painter.text(
                pos2(rect.left() + 60.0, rect.center().y),
                Align2::RIGHT_CENTER,
                line.old.map_or_else(String::new, |n| n.to_string()),
                FontId::monospace(12.0),
                MUTED(),
            );
        }
        if side != Some(false) {
            painter.text(
                pos2(
                    rect.left() + if side.is_some() { 60.0 } else { 104.0 },
                    rect.center().y,
                ),
                Align2::RIGHT_CENTER,
                line.new.map_or_else(String::new, |n| n.to_string()),
                FontId::monospace(12.0),
                MUTED(),
            );
        }
    }
    let index = file.lines.partition_point(|l| l.raw_index < line.raw_index);
    let job = tools.cached_layout(file, index, color);
    let galley = ui.fonts_mut(|f| f.layout_job((*job).clone()));
    let text_width = galley.size().x;
    painter.galley(
        pos2(
            rect.left() + if hunk { 28.0 } else { gutter },
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        color,
    );
    text_width + if hunk { 28.0 } else { gutter } + 8.0
}

/// The sample Preferences → General draws, in Rust so that it shows syntax colours.
const SAMPLE: &str = "diff --git a/src/greeter.rs b/src/greeter.rs
--- a/src/greeter.rs
+++ b/src/greeter.rs
@@ -18,5 +18,6 @@ impl Greeter
     fn greet(&self, name: &str) -> String {
-        let text = format!(\"Hello {name}\");
+        let word = self.word.trim();
+        let text = format!(\"{word} {name}\");
         text.to_uppercase()
     }
";

/// Preferences → General: a small diff drawn by the rows every diff uses, so the
/// options look exactly as they will.
pub(crate) struct Preview {
    file: FilePatch,
    tools: crate::diff_tools::State,
    selected: BTreeSet<usize>,
}
impl Default for Preview {
    fn default() -> Self {
        let mut patch = crate::diff::Patch::parse(SAMPLE.as_bytes());
        Self {
            file: patch.files.remove(0),
            tools: Default::default(),
            selected: BTreeSet::new(),
        }
    }
}
impl Preview {
    pub(crate) fn show(&mut self, ui: &mut Ui, marks: bool, font: u8, syntax: bool) {
        self.tools.marks = marks;
        self.tools.font = font;
        self.tools.syntax = syntax;
        self.tools.poll();
        self.tools.update(&self.file, ui.ctx());
        // Sized for the largest font, so the options below don't move as it changes.
        let largest = crate::general::DIFF_FONTS[crate::general::DIFF_FONTS.len() - 1];
        let height = self.file.visible.len() as f32 * crate::general::diff_row_height(largest) + 2.0;
        let (outer, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
        ui.painter().rect_filled(outer, 4.0, VIEW_PANEL());
        let mut rows = ui.new_child(
            UiBuilder::new()
                .max_rect(outer.shrink(1.0))
                .layout(Layout::top_down(Align::Min)),
        );
        rows.spacing_mut().item_spacing.y = 0.0;
        let row_height = self.tools.row_height();
        let width = rows.available_width();
        let mut widgets = HashMap::new();
        for i in self.file.visible.iter().copied() {
            let (rect, _) = rows.allocate_exact_size(vec2(width, row_height), Sense::hover());
            draw(&mut rows, rect, &self.file.lines[i], None, &self.file, &mut self.selected, false, &mut widgets, &mut self.tools);
        }
        ui.painter().rect_stroke(outer, 4.0, Stroke::new(1.0, HAIRLINE()), StrokeKind::Inside);
    }
}

/// Diff header: a full-width surface strip above the code, closed by a hairline.
fn header_strip(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    let response = Frame::new()
        .fill(VIEW_PANEL())
        .inner_margin(Margin::symmetric(12, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            content(ui);
        })
        .response;
    ui.painter().hline(
        response.rect.x_range(),
        response.rect.bottom() - 0.5,
        Stroke::new(1.0, HAIRLINE()),
    );
}

fn external_menu(
    ui: &mut Ui,
    editor: bool,
    widgets: &mut HashMap<String, UiControl>,
) -> Option<Intent> {
    let mut action = None;
    let menu = ui.menu_button("⋯", |ui| {
        for (id, label, kind, enabled) in [
            (
                "diff-external",
                "Open external diff",
                rocket_git::ExternalToolKind::Diff,
                true,
            ),
            (
                "diff-editor",
                "Open working file in editor",
                rocket_git::ExternalToolKind::Editor,
                editor,
            ),
        ] {
            let r = ui.add_enabled(enabled, Button::new(label));
            widgets.insert(id.into(), (&r).into());
            if r.clicked() {
                action = Some(Intent::Tool(crate::external_tools::ToolIntent::Prepare(
                    kind,
                )));
                ui.close();
            }
        }
    });
    widgets.insert("diff-tools-menu".into(), (&menu.response).into());
    action
}

#[cfg(test)]
mod performance_tests {
    use super::*;

    fn large_tab() -> Tab {
        let mut tab = Tab::new("render-fixture".into(), None);
        tab.diff_tools.words = false;
        let mut bytes = String::new();
        for n in 0..1000 {
            bytes.push_str(&format!("diff --git a/file{n}.rs b/file{n}.rs\n--- a/file{n}.rs\n+++ b/file{n}.rs\n@@ -1 +1 @@\n-fn old() {{}}\n+fn changed() {{ println!(\"你好\"); }}\n"));
        }
        tab.patch = Some(crate::diff::Patch::parse(bytes.as_bytes()));
        tab
    }

    fn frame(ctx: &Context, tab: &mut Tab, size: Vec2, events: Vec<Event>) -> HashMap<String, UiControl> {
        let mut widgets = HashMap::new();
        let mut output = ctx.run_ui(RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            events,
            ..Default::default()
        }, |ui| { patch_view(ui, tab, true, false, &mut widgets); });
        output.textures_delta.clear();
        widgets
    }

    #[test]
    fn virtual_file_list_keeps_last_file_reachable_and_selectable() {
        for size in [vec2(1280.0, 800.0), vec2(780.0, 620.0)] {
            let ctx = Context::default();
            let mut tab = large_tab();
            let mut widgets = frame(&ctx, &mut tab, size, vec![]);
            assert!(widgets.contains_key("patch-file-0"));
            assert!(!widgets.contains_key("patch-file-999"));
            let pos = widgets["patch-file-0"].rect.center();
            for _ in 0..60 {
                widgets = frame(&ctx, &mut tab, size, vec![
                    Event::PointerMoved(pos),
                    Event::MouseWheel { unit: MouseWheelUnit::Point, delta: vec2(0.0, -2000.0), modifiers: Modifiers::NONE, phase: TouchPhase::Move },
                ]);
                if widgets.contains_key("patch-file-999") { break; }
            }
            let last = widgets.get("patch-file-999").expect("last file remains reachable by scrolling").rect.center();
            for pressed in [true, false] {
                frame(&ctx, &mut tab, size, vec![Event::PointerMoved(last), Event::PointerButton { pos: last, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }]);
            }
            assert_eq!(tab.patch_file, 999);
            assert_eq!(tab.patch.as_ref().unwrap().files.len(), 1000);
            tab.split_diff = true;
            frame(&ctx, &mut tab, size, vec![]);
            assert_eq!(tab.patch_file, 999);
        }
    }

    #[test]
    fn virtual_file_list_keeps_keyboard_navigation_across_offscreen_rows() {
        for size in [vec2(1280.0, 800.0), vec2(780.0, 620.0)] {
            let ctx = Context::default();
            let mut tab = large_tab();
            let widgets = frame(&ctx, &mut tab, size, vec![]);
            let first = widgets["patch-file-0"].rect.center();
            for pressed in [true, false] {
                frame(&ctx, &mut tab, size, vec![Event::PointerMoved(first), Event::PointerButton { pos: first, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }]);
            }
            for (key, expected) in [(Key::End, 999), (Key::ArrowUp, 998), (Key::ArrowDown, 999), (Key::Home, 0), (Key::ArrowDown, 1)] {
                frame(&ctx, &mut tab, size, vec![Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]);
                let widgets = frame(&ctx, &mut tab, size, vec![Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE }]);
                assert_eq!(tab.patch_file, expected, "{key:?}");
                assert!(widgets.contains_key(&format!("patch-file-{expected}")));
                assert!(ctx.memory(|memory| memory.focused().is_some()));
            }
        }
    }

    #[test]
    #[ignore = "manual before/after timing probe"]
    fn measure_large_diff_frames() {
        let ctx = Context::default();
        let mut tab = large_tab();
        let mut times = Vec::new();
        for iteration in 0..30 {
            let started = std::time::Instant::now();
            frame(&ctx, &mut tab, vec2(1280.0, 800.0), vec![]);
            if iteration >= 5 { times.push(started.elapsed().as_micros()); }
        }
        times.sort_unstable();
        println!("large_diff_frame_median_us={}", times[times.len()/2]);
    }
}
