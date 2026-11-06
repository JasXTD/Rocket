//! Results grouped by disk folder, one line per repository; the rows scroll alone.
use super::{Row, State, View, relative};
use crate::{
    app::UiControl,
    pages::{self, Part},
    style::*,
};
use eframe::egui::*;
use std::{collections::HashMap, sync::atomic::Ordering};

type Widgets = HashMap<String, UiControl>;
const GROUP_HEIGHT: f32 = 30.0;
const ROW_HEIGHT: f32 = 32.0;

#[derive(Default)]
pub(super) struct Actions {
    pub start: bool,
    pub add: bool,
    pub browse: bool,
    pub change: bool,
    pub stop: bool,
    pub close: bool,
}
fn folder_name(s: &State) -> String {
    let root = s
        .scanned_root
        .as_deref()
        .map(crate::views::display_path)
        .unwrap_or_else(|| s.root.clone());
    std::path::Path::new(&root)
        .file_name()
        .map_or(root.clone(), |n| n.to_string_lossy().into_owned())
}
pub(super) fn show(ctx: &Context, s: &mut State, widgets: &mut Widgets) -> Actions {
    let mut actions = Actions::default();
    let name = folder_name(s);
    let title = if s.running() && !s.adding {
        format!("Scanning {name}")
    } else {
        format!("Add repositories from {name}")
    };
    let subtitle = s
        .scanned_root
        .as_deref()
        .map(crate::views::display_path)
        .unwrap_or_else(|| s.root.clone());
    let response = pages::review_at(
        ctx,
        "manager-scan",
        860.0,
        pages::ReviewHeader {
            image: Icon::Folder,
            title: &title,
            subtitle: &subtitle,
            danger: false,
        },
        widgets,
        "manager-scan-scroll",
        true,
        |ui, part, widgets| match part {
            Part::Body => body(ui, s, widgets, &mut actions),
            Part::Footer => footer(ui, s, widgets, &mut actions),
        },
    );
    actions.close |= response.should_close();
    actions
}

fn footer(ui: &mut Ui, s: &mut State, widgets: &mut Widgets, actions: &mut Actions) {
    if s.running() {
        let label = if s.stopped {
            "Stopping"
        } else if s.adding {
            "Stop adding"
        } else {
            "Stop"
        };
        let r = pages::ghost(ui, !s.stopped, label);
        pages::control(widgets, "manager-scan-stop", &r, ui.clip_rect());
        actions.stop = r.clicked();
        if s.adding {
            let total = s.selection().total;
            let checked = s.progress.checked.load(Ordering::Relaxed);
            ui.add_sized(
                [160.0, 6.0],
                ProgressBar::new(checked as f32 / total.max(1) as f32).desired_height(6.0),
            );
            // The bar shows the progress; the status needs no spinner of its own.
            pages::footer_status(ui, false, &format!("Checking {checked} of {total}"));
        } else {
            // The body shows the scan's spinner and counts; one is enough.
            pages::footer_status(ui, false, "Nothing is saved until you add");
        }
        return;
    }
    let eligible = s.rows.iter().filter(|r| !r.saved).count();
    if !s.completed || eligible == 0 {
        let text = if !s.completed {
            "Nothing was saved"
        } else if s.rows.is_empty() {
            ""
        } else {
            "Nothing to add"
        };
        pages::footer_status(ui, false, text);
        return;
    }
    let selection = s.selection();
    let label = if selection.total == 1 {
        "Add 1 repository".into()
    } else {
        format!("Add {} repositories", selection.total)
    };
    let r = pages::primary(ui, selection.total > 0, &label);
    pages::control(widgets, "manager-scan-add", &r, ui.clip_rect());
    actions.add = r.clicked();
    save_in(ui, s, widgets);
    ui.label(RichText::new("Save in").color(HINT()));
    let mut status = format!("{} selected", selection.total);
    let left_out = s.left_out();
    if left_out > 0 {
        status.push_str(&format!(" · {left_out} left out"));
    }
    if selection.hidden > 0 && s.view != View::Skipped {
        status.push_str(&format!(" · {} hidden by the filter", selection.hidden));
    }
    pages::footer_status(ui, false, &status);
}

fn save_in(ui: &mut Ui, s: &mut State, widgets: &mut Widgets) {
    let selected = if s.match_disk {
        "Match disk folders"
    } else {
        s.folder
            .and_then(|id| s.baseline.folders.iter().find(|f| f.id == id))
            .map_or("Ungrouped", |f| f.name.as_str())
    }
    .to_owned();
    let layer = ui.layer_id();
    let groups = s.has_groups();
    let mut chosen = None;
    let r = ComboBox::from_id_salt("manager-scan-folder")
        .selected_text(selected)
        .truncate()
        .width(170.0)
        .show_ui(ui, |ui| {
            ui.ctx().set_sublayer(layer, ui.layer_id());
            if groups {
                let made = s.new_folders();
                let r = ui.selectable_label(s.match_disk, "Match disk folders");
                let r = if made.is_empty() {
                    r.on_hover_text("Each repository goes in the library folder named after its disk folder")
                } else {
                    r.on_hover_text(format!("Makes {}", made.join(", ")))
                };
                pages::control(widgets, "manager-scan-folder-match", &r, ui.clip_rect());
                if r.clicked() {
                    chosen = Some((true, None));
                    ui.close();
                }
                ui.separator();
            }
            let r = ui.selectable_label(!s.match_disk && s.folder.is_none(), "Ungrouped");
            pages::control(widgets, "manager-scan-folder-none", &r, ui.clip_rect());
            if r.clicked() {
                chosen = Some((false, None));
                ui.close();
            }
            for f in &s.baseline.folders {
                let r = ui.selectable_label(!s.match_disk && s.folder == Some(f.id), &f.name);
                pages::control(
                    widgets,
                    format!("manager-scan-folder-{}", f.id),
                    &r,
                    ui.clip_rect(),
                );
                if r.clicked() {
                    chosen = Some((false, Some(f.id)));
                    ui.close();
                }
            }
        })
        .response;
    if let Some((matching, folder)) = chosen {
        s.match_disk = matching;
        s.folder = folder;
        s.save_in_chosen = true;
    }
    pages::control(widgets, "manager-scan-folder", &r, ui.clip_rect());
}

fn body(ui: &mut Ui, s: &mut State, widgets: &mut Widgets, actions: &mut Actions) {
    let start_y = ui.cursor().top();
    if s.running() && !s.adding {
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.add(Spinner::new().size(16.0));
            ui.label(RichText::new(if s.stopped { "Stopping" } else { "Looking for repositories" }).strong());
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 18.0;
            count(ui, s.progress.folders.load(Ordering::Relaxed), "folders checked");
            count(ui, s.progress.found.load(Ordering::Relaxed), "repositories found");
        });
        ui.add_space(10.0);
        return;
    }
    if !s.completed {
        if let Some(error) = &s.error {
            pages::callout(ui, RED(), error);
        } else if s.stopped {
            pages::callout(ui, HINT(), "Scan stopped. Nothing was saved.");
        }
        ui.horizontal(|ui| {
            let r = pages::primary(ui, !s.root.trim().is_empty(), "Scan again");
            pages::control(widgets, "manager-scan-start", &r, ui.clip_rect());
            actions.start |= r.clicked();
            let r = ui.button("Choose another folder");
            pages::control(widgets, "manager-scan-change", &r, ui.clip_rect());
            actions.change |= r.clicked();
        });
        return;
    }
    let eligible = s.rows.iter().filter(|r| !r.saved).count();
    let issues = s.summary.skipped + s.summary.invalid;
    if !s.rows.is_empty() || issues > 0 {
        toolbar(ui, s, widgets, actions, issues);
    }
    if let Some(error) = s.error.clone() {
        let retry = pages::notice_strip(ui, RED(), &error, Some("Scan again"));
        if let Some(r) = retry {
            pages::control(widgets, "manager-scan-start", &r, ui.clip_rect());
            actions.start |= r.clicked();
        }
    }
    if s.stopped && s.adding {
        pages::callout(
            ui,
            HINT(),
            "Adding stopped. Nothing was saved. Your selection is ready to retry.",
        );
    }
    if s.summary.limited {
        pages::callout(
            ui,
            AMBER(),
            "Scan limit reached. Choose a narrower folder to find the rest.",
        );
    }
    if s.rows.is_empty() && s.view != View::Skipped {
        empty(ui, |ui| {
            ui.label(RichText::new(format!("No repositories in {}", folder_name(s))).font(semibold(14.0)));
            ui.label(
                RichText::new(format!(
                    "Rocket looked in {} folders",
                    pages::number_text(s.progress.folders.load(Ordering::Relaxed))
                ))
                .color(MUTED()),
            );
            ui.add_space(4.0);
            let r = pages::primary(ui, true, "Choose another folder");
            pages::control(widgets, "manager-scan-change", &r, ui.clip_rect());
            actions.change |= r.clicked();
        });
        return;
    }
    if eligible == 0 && s.view != View::Skipped && s.view != View::Saved {
        empty(ui, |ui| {
            let (mark, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
            icon(ui.painter(), mark, Icon::Check, GREEN());
            let n = s.rows.len();
            ui.label(
                RichText::new(if n == 1 {
                    "It is already in your library".to_owned()
                } else {
                    format!("All {n} are already in your library")
                })
                .font(semibold(14.0)),
            );
            ui.label(RichText::new("Names and folders stay as they are").color(MUTED()));
        });
        return;
    }
    // Match the shared review body's height budget so only the rows scroll.
    let body_max = (ui.ctx().content_rect().height() - 250.0).max(100.0);
    let height = (body_max - 14.0 - (ui.cursor().top() - start_y) - 14.0).clamp(96.0, 520.0);
    let reset = std::mem::take(&mut s.reset_scroll);
    Frame::new()
        .fill(VIEW_BG())
        .stroke(Stroke::new(1.0, CARD_STROKE()))
        .corner_radius(8)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if s.view == View::Skipped {
                skipped(ui, s, widgets, height, reset);
            } else {
                list(ui, s, widgets, height, reset);
            }
        });
}

fn count(ui: &mut Ui, n: usize, what: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        ui.label(RichText::new(pages::number_text(n)).strong());
        ui.label(RichText::new(what).color(MUTED()));
    });
}

fn empty(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 170.0), Sense::hover());
    let mut inner = ui.new_child(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::top_down(Align::Center)),
    );
    inner.spacing_mut().item_spacing.y = 6.0;
    inner.add_space(34.0);
    content(&mut inner);
}

fn toolbar(ui: &mut Ui, s: &mut State, widgets: &mut Widgets, actions: &mut Actions, issues: usize) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let width = (ui.available_width() - 560.0).clamp(140.0, 230.0);
        let r = pages::search_box(ui, &mut s.filter, "manager-scan-filter", "Filter by name or path", width);
        pages::control(widgets, "manager-scan-filter", &r, ui.clip_rect());
        if r.changed() {
            s.reset_scroll = true;
        }
        let saved = s.rows.iter().filter(|r| r.saved).count();
        let chips = [
            (View::All, "All", s.rows.len()),
            (View::New, "New", s.rows.len() - saved),
            (View::Saved, "In library", saved),
            (View::Skipped, "Skipped", issues),
        ];
        for (view, label, n) in chips {
            if view == View::Skipped && n == 0 {
                continue;
            }
            let r = crate::activity_history::chip(ui, label, Some(n), s.view == view, CONTROL);
            pages::control(widgets, format!("manager-scan-view-{}", view.code()), &r, ui.clip_rect());
            if r.clicked() && s.view != view {
                s.view = view;
                s.reset_scroll = true;
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let r = crate::chrome::icon_button(ui, Icon::Folder, true, false, 28.0)
                .on_hover_text("Choose another folder");
            pages::control(widgets, "manager-scan-change", &r, ui.clip_rect());
            actions.change |= r.clicked();
            let r = crate::chrome::icon_button(ui, Icon::Refresh, true, false, 28.0)
                .on_hover_text("Scan again");
            pages::control(widgets, "manager-scan-start", &r, ui.clip_rect());
            actions.start |= r.clicked();
            let before = s.recursive;
            let r = switch(ui, &mut s.recursive, "Subfolders")
                .on_hover_text("Look in subfolders too; changing it scans again");
            pages::control(widgets, "manager-scan-recursive", &r, ui.clip_rect());
            if s.recursive != before {
                actions.start = true;
            }
            ui.label(RichText::new("Subfolders").color(MUTED()));
        });
    });
}

/// Rows and their group headings, top to bottom; only those in view are drawn.
enum Item {
    Group(usize),
    Row(usize),
}
struct Group {
    name: String,
    rows: Vec<usize>,
}

fn list(ui: &mut Ui, s: &mut State, widgets: &mut Widgets, height: f32, reset: bool) {
    let shown = s.visible();
    let mut groups: Vec<Group> = Vec::new();
    for &i in &shown {
        let name = &s.rows[i].group;
        match groups.iter_mut().find(|g| &g.name == name) {
            Some(g) => g.rows.push(i),
            None => groups.push(Group {
                name: name.clone(),
                rows: vec![i],
            }),
        }
    }
    // The scanned folder's own repositories come first, then folders by name.
    groups.sort_by_key(|g| (!g.name.is_empty(), g.name.to_lowercase()));
    let mut items = Vec::new();
    for (g, group) in groups.iter().enumerate() {
        items.push(Item::Group(g));
        items.extend(group.rows.iter().map(|&i| Item::Row(i)));
    }
    let tops: Vec<f32> = items
        .iter()
        .scan(0.0, |y, item| {
            let top = *y;
            *y += match item {
                Item::Group(_) => GROUP_HEIGHT,
                Item::Row(_) => ROW_HEIGHT,
            };
            Some(top)
        })
        .collect();
    let total = tops.last().map_or(0.0, |t| t + ROW_HEIGHT);
    let root = folder_name(s);
    let mut scroll = ScrollArea::vertical()
        .id_salt("manager-scan-results")
        .min_scrolled_height(height)
        .max_height(height)
        .auto_shrink([false, false]);
    if reset {
        scroll = scroll.vertical_scroll_offset(0.0);
    }
    let mut toggle_group = None;
    let output = scroll.show_viewport(ui, |ui, viewport| {
        ui.set_height(total);
        let left = ui.max_rect().left();
        let width = ui.available_width();
        for (n, item) in items.iter().enumerate() {
            let (top, size) = (tops[n], if matches!(item, Item::Group(_)) { GROUP_HEIGHT } else { ROW_HEIGHT });
            if top + size < viewport.top() || top > viewport.bottom() {
                continue;
            }
            let rect = Rect::from_min_size(pos2(left, ui.max_rect().top() + top), vec2(width, size));
            let mut child = ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::left_to_right(Align::Center)));
            match item {
                Item::Group(g) => {
                    let group = &groups[*g];
                    if let Some(value) = group_header(&mut child, s, group, &root, *g, widgets) {
                        toggle_group = Some((group.name.clone(), value));
                    }
                }
                Item::Row(i) => repository_row(&mut child, &mut s.rows[*i], *i, widgets),
            }
        }
    });
    if let Some((group, value)) = toggle_group {
        s.select_group(&group, value);
    }
    pages::region(
        widgets,
        "manager-scan-results",
        output.inner_rect.intersect(ui.clip_rect()),
        true,
    );
    if shown.is_empty() {
        let mut empty = ui.new_child(
            UiBuilder::new()
                .max_rect(output.inner_rect.shrink(14.0))
                .layout(Layout::top_down(Align::Min)),
        );
        empty.label(RichText::new("No matching repositories").strong());
        empty.label(RichText::new("Selected repositories stay selected when hidden.").color(HINT()));
        let r = empty.button("Show all");
        pages::control(widgets, "manager-scan-clear-filter", &r, empty.clip_rect());
        if r.clicked() {
            s.filter.clear();
            s.view = View::All;
        }
    }
}

/// A disk folder: its box selects or clears its new repositories in view.
fn group_header(ui: &mut Ui, s: &State, group: &Group, root: &str, index: usize, widgets: &mut Widgets) -> Option<bool> {
    let rect = ui.max_rect();
    ui.painter().rect_filled(rect, 0.0, SUNKEN());
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, CARD_LINE()));
    ui.add_space(10.0);
    let eligible: Vec<&Row> = group.rows.iter().map(|&i| &s.rows[i]).filter(|r| !r.saved).collect();
    let selected = eligible.iter().filter(|r| r.selected).count();
    let mut all = !eligible.is_empty() && selected == eligible.len();
    let r = ui.add_enabled(
        !eligible.is_empty(),
        Checkbox::without_text(&mut all).indeterminate(selected > 0 && selected < eligible.len()),
    );
    pages::control(widgets, format!("manager-scan-group-{index}"), &r, ui.clip_rect());
    let (mark, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
    icon(ui.painter(), mark, Icon::Folder, TEAL());
    let name = if group.name.is_empty() {
        root.to_owned()
    } else {
        group.name.replace('/', " › ")
    };
    ui.add(Label::new(RichText::new(name).font(semibold(13.0))).truncate());
    let new = eligible.len();
    let detail = if new == group.rows.len() {
        group.rows.len().to_string()
    } else {
        format!("{} · {new} new", group.rows.len())
    };
    ui.label(RichText::new(detail).size(12.0).color(FAINT()));
    r.changed().then_some(all)
}

fn repository_row(ui: &mut Ui, row: &mut Row, index: usize, widgets: &mut Widgets) {
    let rect = ui.max_rect();
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, ROW_LINE()));
    ui.add_space(32.0);
    let r = ui.add_enabled(!row.saved, Checkbox::without_text(&mut row.selected));
    let r = r.on_hover_text(crate::views::display_path(&row.repository.path));
    pages::control(widgets, format!("manager-scan-row-{index}"), &r, ui.clip_rect());
    let image = match row.repository.kind {
        "Linked worktree" => Icon::Worktree,
        "Bare repository" => Icon::Database,
        _ => Icon::Branch,
    };
    let (mark, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
    icon(ui.painter(), mark, image, if row.saved { DIM() } else { MUTED() });
    let name = RichText::new(&row.name);
    let name = if row.saved {
        name.color(MUTED())
    } else {
        name.font(semibold(13.0))
    };
    ui.add(Label::new(name).truncate())
        .on_hover_text(&row.relative_path);
    if row.saved {
        pages::pill(ui, "In library", HINT());
    } else if row.repository.kind == "Linked worktree" {
        pages::pill(ui, "Linked worktree", BLUE());
    } else if row.repository.kind == "Bare repository" {
        pages::pill(ui, "Bare", BLUE());
    }
    if !row.meta.is_empty() {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(12.0);
            ui.add(Label::new(RichText::new(&row.meta).size(12.0).color(FAINT())).truncate());
        });
    }
}

fn skipped(ui: &mut Ui, s: &State, widgets: &mut Widgets, height: f32, reset: bool) {
    let mut scroll = ScrollArea::vertical()
        .id_salt("manager-scan-issues")
        .min_scrolled_height(height)
        .max_height(height)
        .auto_shrink([false, false]);
    if reset {
        scroll = scroll.vertical_scroll_offset(0.0);
    }
    let output = scroll.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 0.0;
        for issue in &s.summary.issues {
            let path = s
                .scanned_root
                .as_deref()
                .map(|root| relative(&issue.path, root))
                .unwrap_or_default()
                .replace(['\\', '/'], " › ");
            ui.allocate_ui_with_layout(vec2(ui.available_width(), 44.0), Layout::left_to_right(Align::Center), |ui| {
                ui.add_space(12.0);
                let (mark, _) = ui.allocate_exact_size(vec2(15.0, 15.0), Sense::hover());
                icon(ui.painter(), mark, Icon::Warning, AMBER());
                ui.vertical(|ui| {
                    ui.add_space(5.0);
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.add(Label::new(RichText::new(path).font(semibold(13.0))).truncate())
                        .on_hover_text(crate::views::display_path(&issue.path));
                    ui.label(RichText::new(issue.reason).size(12.0).color(AMBER()));
                });
            });
            let r = ui.max_rect();
            ui.painter().hline(r.x_range(), ui.cursor().top() - 0.5, Stroke::new(1.0, ROW_LINE()));
        }
        let count = s.summary.skipped + s.summary.invalid;
        if count > s.summary.issues.len() {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.label(
                    RichText::new(format!("Showing {} of {count} skipped entries", s.summary.issues.len()))
                        .size(12.0)
                        .color(HINT()),
                );
            });
        }
    });
    pages::region(
        widgets,
        "manager-scan-issues",
        output.inner_rect.intersect(ui.clip_rect()),
        true,
    );
}
