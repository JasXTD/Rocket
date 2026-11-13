//! A commit series as a list beside the chosen patch. Export and Import share it.
use crate::{app::UiControl, pages, patch_diff::{self, Shown}, style::*};
use eframe::egui::*;
use std::collections::HashMap;

/// Height of one row in the list.
const ROW: f32 = 46.0;

/// One commit or patch in the list.
pub(crate) struct Row {
    pub subject: String,
    /// Its short ID, author and date.
    pub meta: String,
    pub added: usize,
    pub removed: usize,
}

/// Control ids: `{row}-{i}` for each row and `scroll` for the list.
pub(crate) struct Keys<'a> {
    pub row: &'a str,
    pub scroll: &'a str,
}

/// The rows, then the chosen row's `files`. Returns nothing: `selected` follows clicks.
#[allow(clippy::too_many_arguments)]
pub(crate) fn split(
    ui: &mut Ui,
    title: &str,
    rows: &[Row],
    selected: &mut usize,
    files: &[Shown],
    keys: Keys,
    widgets: &mut HashMap<String, UiControl>,
    enabled: bool,
    height: f32,
) {
    *selected = (*selected).min(rows.len().saturating_sub(1));
    let index = *selected;
    Frame::new()
        .stroke(Stroke::new(1.0, CARD_STROKE()))
        .corner_radius(8)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            let list_width = (ui.available_width() * 0.36).clamp(220.0, 340.0);
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(vec2(list_width, height), Layout::top_down(Align::Min), |ui| {
                    ui.set_width(list_width);
                    ui.set_height(height);
                    let (head, _) = ui.allocate_exact_size(vec2(list_width, 33.0), Sense::hover());
                    ui.painter().rect_filled(
                        head,
                        CornerRadius { nw: 7, ne: 0, sw: 0, se: 0 },
                        CARD(),
                    );
                    ui.painter().hline(head.x_range(), head.bottom() - 0.5, Stroke::new(1.0, CARD_STROKE()));
                    let mut job = text::LayoutJob::simple_singleline(title.to_owned(), semibold(12.0), MUTED());
                    job.wrap = text::TextWrapping::truncate_at_width(list_width - 24.0);
                    let label = ui.painter().layout_job(job);
                    ui.painter().galley(pos2(head.left() + 12.0, head.center().y - label.size().y / 2.0), label, MUTED());
                    let list = ScrollArea::vertical()
                        .id_salt((keys.scroll, "rows"))
                        .max_height(height - 33.0)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            for (i, entry) in rows.iter().enumerate() {
                                let r = row(ui, i, entry, i == index, enabled);
                                pages::control(widgets, format!("{}-{i}", keys.row), &r, ui.clip_rect());
                                if r.clicked() {
                                    *selected = i;
                                }
                            }
                        });
                    pages::region(widgets, keys.scroll, list.inner_rect.intersect(ui.clip_rect()), enabled);
                });
                let (line, _) = ui.allocate_exact_size(vec2(1.0, height), Sense::hover());
                ui.painter().vline(line.center().x, line.y_range(), Stroke::new(1.0, CARD_STROKE()));
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width());
                    ui.set_height(height);
                    patch_diff::pane(ui, (keys.scroll, "diff", index), files, true, height);
                });
            });
        });
    cut_note(ui, files);
}

/// Says so when a diff shows only part of a long change.
fn cut_note(ui: &mut Ui, files: &[Shown]) {
    if files.iter().any(|f| f.limited) {
        pages::note(ui, crate::i18n::tr("This change is long, so only its start shows here. Nothing is left out of the commit."));
    }
}

/// Lines added and removed across `files`.
pub(crate) fn counts(files: &[Shown]) -> (usize, usize) {
    files.iter().fold((0, 0), |(a, r), f| (a + f.added, r + f.removed))
}

/// The edits a step throws away, one click away. Returns whether they're showing.
pub(crate) fn edits_section(ui: &mut Ui, id: &str, files: &[Shown], open: bool, widgets: &mut HashMap<String, UiControl>) -> bool {
    let (added, removed) = counts(files);
    let name = match files {
        [one] => one.label.rsplit('/').next().unwrap_or(&one.label).to_owned(),
        _ => crate::i18n::trn!(files.len(), "{n} file", "{n} files"),
    };
    let r = pages::disclosure(ui, open, &crate::i18n::trf!("Your edits to {name} go too  +{added} −{removed}", name, added, removed));
    pages::control(widgets, id, &r, ui.clip_rect());
    let open = open ^ r.clicked();
    if open {
        patch_diff::rows(ui, (id, "diff"), files, files.len() > 1, None, 150.0);
        cut_note(ui, files);
    }
    open
}

/// A titled diff in its own card, for a review that shows one set of changes.
pub(crate) fn diff_card(ui: &mut Ui, title: &str, id: &str, files: &[Shown], height: f32) {
    let (added, removed) = counts(files);
    pages::titled_card(
        ui,
        |ui| {
            ui.add(Label::new(RichText::new(title).size(12.0).color(MUTED()).strong()).truncate());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                ui.label(RichText::new(format!("−{removed}")).size(12.0).color(RED()));
                ui.label(RichText::new(format!("+{added}")).size(12.0).color(GREEN()));
            });
        },
        |ui| patch_diff::pane(ui, id, files, true, height),
    );
    cut_note(ui, files);
}

fn row(ui: &mut Ui, i: usize, entry: &Row, selected: bool, enabled: bool) -> Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), sense);
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 0.0, SELECTED());
        painter.rect_filled(Rect::from_min_size(rect.min, vec2(3.0, ROW)), 0.0, BLUE());
    } else if enabled && (response.hovered() || response.has_focus()) {
        painter.rect_filled(rect, 0.0, HOVER());
    }
    painter.text(
        pos2(rect.left() + 26.0, rect.center().y),
        Align2::RIGHT_CENTER,
        (i + 1).to_string(),
        FontId::proportional(12.0),
        HINT(),
    );
    let right = rect.right() - 12.0;
    let removed = painter.layout_no_wrap(format!("−{}", entry.removed), FontId::proportional(12.0), RED());
    let added = painter.layout_no_wrap(format!("+{}", entry.added), FontId::proportional(12.0), GREEN());
    let (removed_width, added_width) = (removed.size().x, added.size().x);
    let top = rect.top() + 7.0;
    // No lines to count: an empty commit, or one too big to read.
    if entry.added + entry.removed > 0 {
        painter.galley(pos2(right - removed_width, top + 1.0), removed, RED());
        painter.galley(pos2(right - removed_width - 5.0 - added_width, top + 1.0), added, GREEN());
    }
    let left = rect.left() + 36.0;
    let width = right - removed_width - added_width - 14.0 - left;
    let mut subject = text::LayoutJob::simple_singleline(entry.subject.clone(), FontId::proportional(13.0), TEXT());
    subject.wrap = text::TextWrapping::truncate_at_width(width.max(40.0));
    let subject = painter.layout_job(subject);
    painter.galley(pos2(left, top), subject, TEXT());
    let mut meta = text::LayoutJob::simple_singleline(entry.meta.clone(), FontId::proportional(12.0), HINT());
    meta.wrap = text::TextWrapping::truncate_at_width((right - left).max(40.0));
    let meta = painter.layout_job(meta);
    painter.galley(pos2(left, top + 19.0), meta, HINT());
    painter.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, ROW_LINE()));
    response.widget_info(|| WidgetInfo::selected(Role::Button, enabled, selected, &entry.subject));
    response
}

/// "31 Oct 2026" from a commit's ISO date or a mail's date, or the text as it came.
pub(crate) fn short_date(date: &str) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let mut parts = date.get(..10).unwrap_or_default().split('-');
    if let (Some(year), Some(month), Some(day)) = (parts.next(), parts.next(), parts.next())
        && year.len() == 4
        && let (Ok(month), Ok(day)) = (month.parse::<usize>(), day.parse::<u32>())
        && let Some(name) = month.checked_sub(1).and_then(|m| MONTHS.get(m))
    {
        return format!("{day} {} {year}", crate::i18n::tr(name));
    }
    // "Sat, 31 Oct 2026 19:31:09 +0800"
    let words: Vec<&str> = date.split_whitespace().collect();
    match words.as_slice() {
        [_, day, month, year, ..] if MONTHS.contains(month) => {
            let month = MONTHS.iter().find(|name| *name == month).expect("known month");
            format!("{day} {} {year}", crate::i18n::tr(month))
        },
        _ => date.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::short_date;

    #[test]
    fn dates_read_the_same_from_a_commit_and_from_a_mail() {
        assert_eq!(short_date("2026-10-31T19:31:09+08:00"), "31 Oct 2026");
        assert_eq!(short_date("2026-02-03T01:00:00Z"), "3 Feb 2026");
        assert_eq!(short_date("Sat, 31 Oct 2026 19:31:09 +0800"), "31 Oct 2026");
        assert_eq!(short_date("yesterday"), "yesterday");
        assert_eq!(short_date("2026-13-40T00:00:00Z"), "2026-13-40T00:00:00Z");
    }
}
