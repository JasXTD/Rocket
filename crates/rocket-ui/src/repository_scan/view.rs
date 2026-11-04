//! Review controls remain above the independently scrolling repository list.
use super::{Row, State, relative};
use crate::{
    app::UiControl,
    pages::{self, Part},
    style::*,
};
use eframe::egui::*;
use std::{collections::HashMap, sync::atomic::Ordering};

type Widgets = HashMap<String, UiControl>;
#[derive(Default)]
pub(super) struct Actions {
    pub start: bool,
    pub add: bool,
    pub browse: bool,
    pub change: bool,
    pub stop: bool,
    pub close: bool,
}
pub(super) fn show(ctx: &Context, s: &mut State, widgets: &mut Widgets) -> Actions {
    let mut actions = Actions::default();
    let busy = s.running();
    let response = pages::review_at(
        ctx,
        "manager-scan",
        820.0,
        pages::ReviewHeader {
            image: Icon::Folder,
            title: "Scan folder",
            subtitle: "Choose repositories to add to your library",
            danger: false,
        },
        widgets,
        "manager-scan-scroll",
        true,
        |ui, part, widgets| match part {
            Part::Body => body(ui, s, widgets, &mut actions),
            Part::Footer => {
                let selection = s.selection();
                if busy {
                    let label = if s.stopped {
                        "Stopping"
                    } else if s.adding {
                        "Stop adding"
                    } else {
                        "Stop scan"
                    };
                    let r = pages::ghost(ui, !s.stopped, label);
                    pages::control(widgets, "manager-scan-stop", &r, ui.clip_rect());
                    actions.stop = r.clicked();
                } else {
                    if !s.completed {
                        let r = pages::primary(ui, !s.root.trim().is_empty(), "Scan folder");
                        pages::control(widgets, "manager-scan-start", &r, ui.clip_rect());
                        actions.start |= r.clicked();
                    } else if s.rows.is_empty() {
                        let r = pages::primary(ui, true, "Change folder");
                        pages::control(widgets, "manager-scan-change", &r, ui.clip_rect());
                        actions.change |= r.clicked();
                    } else if s.rows.iter().all(|r| r.saved) {
                        let r = pages::primary(ui, true, "Done");
                        pages::control(widgets, "manager-scan-done", &r, ui.clip_rect());
                        actions.close |= r.clicked();
                    } else {
                        let label = if selection.total == 1 {
                            "Add 1 repository".into()
                        } else {
                            format!("Add {} repositories", selection.total)
                        };
                        let r = pages::primary(ui, selection.total > 0, &label);
                        pages::control(widgets, "manager-scan-add", &r, ui.clip_rect());
                        actions.add = r.clicked();
                    }
                    let r = pages::ghost(ui, true, "Cancel");
                    pages::control(widgets, "manager-scan-cancel", &r, ui.clip_rect());
                    actions.close |= r.clicked();
                }
                let status = if busy {
                    "Nothing saved yet".into()
                } else if !s.completed {
                    "Saves shortcuts; opens no tabs".into()
                } else if selection.hidden > 0 {
                    format!(
                        "{} selected · {} hidden by filter",
                        selection.total, selection.hidden
                    )
                } else {
                    format!("{} selected", selection.total)
                };
                pages::footer_status(ui, busy, &status);
            }
        },
    );
    actions.close |= response.should_close();
    actions
}

fn body(ui: &mut Ui, s: &mut State, widgets: &mut Widgets, actions: &mut Actions) {
    let start_y = ui.cursor().top();
    let busy = s.running();
    if !s.completed && !busy {
        ui.label("Parent folder");
        ui.horizontal(|ui| {
            let r = ui.add_sized(
                [(ui.available_width() - 90.0).max(80.0), CONTROL],
                TextEdit::singleline(&mut s.root)
                    .id_salt("manager-scan-root")
                    .hint_text("Choose a folder containing your repositories"),
            );
            pages::control(widgets, "manager-scan-root", &r, ui.clip_rect());
            if r.changed() {
                s.clear();
            }
            let r = ui.button("Browse");
            pages::control(widgets, "manager-scan-browse", &r, ui.clip_rect());
            actions.browse = r.clicked();
        });
        let r = ui.add(checkbox(&mut s.recursive, "Include subfolders"));
        pages::control(widgets, "manager-scan-recursive", &r, ui.clip_rect());
        hint(
            ui,
            if s.recursive {
                "Find repositories in this folder and its subfolders."
            } else {
                "Check this folder and its immediate children only."
            },
        );
        hint(
            ui,
            "Your files stay unchanged. Git metadata and folder links are skipped.",
        );
    } else {
        Frame::new()
            .fill(RAISED())
            .corner_radius(6)
            .inner_margin(8)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if !busy && !s.rows.is_empty() {
                            let r = ui.button("Change folder");
                            pages::control(widgets, "manager-scan-change", &r, ui.clip_rect());
                            actions.change = r.clicked();
                        }
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            hint(ui, if busy { "Scanning in" } else { "Folder" });
                            ui.add(Label::new(&s.root).truncate())
                                .on_hover_text(&s.root);
                        });
                    });
                });
            });
    }
    if let Some(error) = &s.error {
        pages::callout(ui, RED(), error);
    }
    if s.stopped && !busy {
        pages::callout(
            ui,
            HINT(),
            if s.adding {
                "Adding stopped. Nothing was saved. Your selection is ready to retry."
            } else {
                "Scan stopped. Nothing was saved. You can scan again."
            },
        );
    }
    if busy {
        ui.add_space(12.0);
        let label = if s.stopped {
            "Stopping"
        } else if s.adding {
            "Checking selected repositories"
        } else {
            "Looking for repositories"
        };
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(RichText::new(label).strong());
        });
        if s.adding {
            let total = s.selection().total;
            let checked = s.progress.checked.load(Ordering::Relaxed);
            ui.add(
                ProgressBar::new(checked as f32 / total.max(1) as f32)
                    .text(format!("{checked} of {total} checked")),
            );
            hint(
                ui,
                "The library updates only after every selected repository passes.",
            );
        } else {
            hint(
                ui,
                &format!(
                    "{} folders checked · {} repositories found",
                    s.progress.folders.load(Ordering::Relaxed),
                    s.progress.found.load(Ordering::Relaxed)
                ),
            );
        }
        return;
    }
    if !s.completed {
        return;
    }
    let saved = s.rows.iter().filter(|r| r.saved).count();
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let r = pages::ghost(ui, true, "Rescan");
            pages::control(widgets, "manager-scan-start", &r, ui.clip_rect());
            actions.start = r.clicked();
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                pages::pill(ui, &format!("{} found", s.rows.len()), TEXT());
                pages::pill(ui, &format!("{} new", s.rows.len() - saved), BLUE());
                if saved > 0 {
                    pages::pill(ui, &format!("{saved} already saved"), HINT());
                }
            });
        });
    });
    if s.rows.is_empty() {
        ui.add_space(16.0);
        ui.label(RichText::new("No repositories found").strong());
        hint(
            ui,
            "Choose another folder, or include subfolders when scanning.",
        );
        issues(ui, s, widgets, 92.0);
        return;
    }
    if saved == s.rows.len() {
        hint(
            ui,
            "These repositories are already in your library. Names and folders are unchanged.",
        );
    }
    let mut reset_scroll = std::mem::take(&mut s.reset_scroll);
    ui.horizontal(|ui| {
        let width = (ui.available_width() - 215.0).max(70.0);
        let r = ui.add_sized(
            [width, CONTROL],
            TextEdit::singleline(&mut s.filter)
                .id_salt("manager-scan-filter")
                .hint_text("Filter by name or path"),
        );
        pages::control(widgets, "manager-scan-filter", &r, ui.clip_rect());
        reset_scroll |= r.changed();
        hint(ui, "Save in");
        folder(ui, s, widgets);
    });
    ui.horizontal(|ui| {
        let selection = s.selection();
        let mut all = selection.checked();
        let r = ui
            .scope(|ui| {
                let widgets = &mut ui.visuals_mut().widgets;
                for v in [
                    &mut widgets.noninteractive,
                    &mut widgets.inactive,
                    &mut widgets.hovered,
                    &mut widgets.active,
                ] {
                    v.corner_radius = CornerRadius::same(3);
                }
                ui.add_enabled(
                    selection.eligible > 0,
                    Checkbox::new(&mut all, "Select shown").indeterminate(selection.partial()),
                )
            })
            .inner;
        pages::control(widgets, "manager-scan-all", &r, ui.clip_rect());
        if r.changed() {
            s.select_visible(all);
        }
        let r = ui.add(checkbox(&mut s.hide_saved, "Hide already saved"));
        pages::control(widgets, "manager-scan-hide-saved", &r, ui.clip_rect());
        reset_scroll |= r.changed();
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            hint(
                ui,
                &format!("{} of {} shown", s.visible().len(), s.rows.len()),
            );
        });
    });
    // Match the shared review body's height budget. Only rows scroll in ordinary
    // compact states; the outer scroller still handles unusually long error text.
    let body_max = (ui.ctx().content_rect().height() - 250.0).max(100.0);
    // Body padding is 10 + 4; the list frame adds 6 + 6 padding and two strokes.
    let remaining = body_max - 14.0 - (ui.cursor().top() - start_y);
    let gap = ui.spacing().item_spacing.y;
    let issue_line = ui.spacing().interact_size.y;
    let detail_height = (remaining - 14.0 - gap - issue_line - gap - 60.0).clamp(24.0, 92.0);
    let reserve = if s.summary.limited || s.summary.skipped + s.summary.invalid > 0 {
        gap + issue_line + if s.details { gap + detail_height } else { 0.0 }
    } else {
        0.0
    };
    let height = (remaining - 14.0 - reserve).clamp(60.0, 400.0);
    let shown = s.visible();
    Frame::new()
        .stroke(Stroke::new(1.0, HAIRLINE()))
        .corner_radius(6)
        .inner_margin(6)
        .show(ui, |ui| {
            let mut scroll = ScrollArea::vertical()
                .id_salt("manager-scan-results")
                .min_scrolled_height(height)
                .max_height(height)
                .auto_shrink([false, false]);
            if reset_scroll {
                scroll = scroll.vertical_scroll_offset(0.0);
            }
            let output = scroll.show_rows(ui, 47.0, shown.len(), |ui, range| {
                for index in range {
                    let index = shown[index];
                    row(ui, &mut s.rows[index], index, widgets);
                }
            });
            pages::region(
                widgets,
                "manager-scan-results",
                output.inner_rect.intersect(ui.clip_rect()),
                true,
            );
            if shown.is_empty() {
                // Use the viewport for the empty message, without growing the list.
                let mut empty = ui.new_child(
                    UiBuilder::new()
                        .max_rect(output.inner_rect.shrink(12.0))
                        .layout(Layout::top_down(Align::Min)),
                );
                empty.label(RichText::new("No matching repositories").strong());
                hint(
                    &mut empty,
                    "Selected repositories stay selected when hidden.",
                );
                let r = empty.button("Clear filters");
                pages::control(widgets, "manager-scan-clear-filter", &r, empty.clip_rect());
                if r.clicked() {
                    s.filter.clear();
                    s.hide_saved = false;
                }
            }
        });
    issues(ui, s, widgets, detail_height);
}

fn folder(ui: &mut Ui, s: &mut State, widgets: &mut Widgets) {
    let selected = s
        .folder
        .and_then(|id| s.baseline.folders.iter().find(|f| f.id == id))
        .map_or("Ungrouped", |f| f.name.as_str());
    let layer = ui.layer_id();
    let r = ComboBox::from_id_salt("manager-scan-folder")
        .selected_text(selected)
        .truncate()
        .width(150.0)
        .show_ui(ui, |ui| {
            ui.ctx().set_sublayer(layer, ui.layer_id());
            let r = ui.selectable_value(&mut s.folder, None, "Ungrouped");
            pages::control(widgets, "manager-scan-folder-none", &r, ui.clip_rect());
            if r.clicked() {
                ui.close();
            }
            for f in &s.baseline.folders {
                let r = ui.selectable_value(&mut s.folder, Some(f.id), &f.name);
                pages::control(
                    widgets,
                    format!("manager-scan-folder-{}", f.id),
                    &r,
                    ui.clip_rect(),
                );
                if r.clicked() {
                    ui.close();
                }
            }
        })
        .response;
    pages::control(widgets, "manager-scan-folder", &r, ui.clip_rect());
}
fn row(ui: &mut Ui, row: &mut Row, index: usize, widgets: &mut Widgets) {
    ui.push_id(index, |ui| {
        ui.allocate_ui_with_layout(
            vec2(ui.available_width(), 47.0),
            Layout::right_to_left(Align::Center),
            |ui| {
                pages::pill(
                    ui,
                    if row.saved {
                        "Already saved"
                    } else {
                        row.repository.kind
                    },
                    if row.saved { HINT() } else { BLUE() },
                );
                ui.with_layout(Layout::top_down(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.style_mut().wrap_mode = Some(TextWrapMode::Truncate);
                    let r = ui.add_enabled(!row.saved, checkbox(&mut row.selected, &row.name));
                    pages::control(
                        widgets,
                        format!("manager-scan-row-{index}"),
                        &r,
                        ui.clip_rect(),
                    );
                    ui.add(
                        Label::new(RichText::new(&row.relative_path).size(11.5).color(HINT()))
                            .truncate(),
                    )
                    .on_hover_text(crate::views::display_path(&row.repository.path));
                });
            },
        );
    });
}
fn issues(ui: &mut Ui, s: &mut State, widgets: &mut Widgets, max_height: f32) {
    let count = s.summary.skipped + s.summary.invalid;
    if count == 0 && !s.summary.limited {
        return;
    }
    let label = if s.summary.limited {
        "Scan limit reached · Review details".into()
    } else {
        format!("{count} entries skipped · Review details")
    };
    // The list above was sized using this frame's expansion state. Apply a click
    // to the next frame rather than briefly shrinking only half of the layout.
    let expanded = s.details;
    let r = ui.selectable_label(expanded, RichText::new(label).size(12.0).color(AMBER()));
    pages::control(widgets, "manager-scan-details", &r, ui.clip_rect());
    if r.clicked() {
        s.details = !s.details;
    }
    if !expanded {
        return;
    }
    let output = ScrollArea::vertical()
        .id_salt("manager-scan-issues")
        .max_height(max_height)
        .show(ui, |ui| {
            if s.summary.limited {
                hint(
                    ui,
                    "Choose a narrower folder to find the remaining repositories.",
                );
            }
            for issue in &s.summary.issues {
                let path = s
                    .scanned_root
                    .as_deref()
                    .map(|root| relative(&issue.path, root))
                    .unwrap_or_default();
                ui.add(
                    Label::new(
                        RichText::new(format!("{path} — {}", issue.reason))
                            .size(12.0)
                            .color(HINT()),
                    )
                    .truncate(),
                )
                .on_hover_text(crate::views::display_path(&issue.path));
            }
            if count > s.summary.issues.len() {
                hint(
                    ui,
                    &format!(
                        "Showing {} of {count} skipped entries.",
                        s.summary.issues.len()
                    ),
                );
            }
        });
    pages::region(
        widgets,
        "manager-scan-issues",
        output.inner_rect.intersect(ui.clip_rect()),
        true,
    );
}
fn hint(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(HINT()));
}
