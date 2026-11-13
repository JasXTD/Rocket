//! The selected session's uncommitted changes: files grouped as Git groups them, beside
//! the chosen file's diff. Read-only; staging and committing happen in the worktree's tab.
use super::*;
use crate::{
    agents::changes::{Area, State as Changes},
    diff::Kind,
    i18n::trn,
};

/// The order the groups show in: what blocks first, then what is staged.
const AREAS: [Area; 4] = [Area::Conflict, Area::Staged, Area::Unstaged, Area::Untracked];

pub(super) fn show(
    ui: &mut Ui,
    s: &Changes,
    w: &mut Widgets,
    destination: &std::path::Path,
    ready: bool,
    action: &mut Option<Action>,
) {
    let clip = ui.clip_rect();
    let pane = ui.max_rect();
    let area = Rect::from_min_max(pane.min + vec2(18.0, 10.0), pane.max - vec2(18.0, 16.0));
    let mut ui = child(ui, "agents-changes", area);
    let ui = &mut ui;
    ui.spacing_mut().item_spacing.y = 8.0;
    // One row: what was read, then Refresh (Stop while reading) and the way to the worktree.
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), CONTROL), Sense::hover());
    let mut row = ui.new_child(
        UiBuilder::new()
            .max_rect(bar)
            .layout(Layout::right_to_left(Align::Center)),
    );
    row.spacing_mut().item_spacing.x = 8.0;
    let r = button(&mut row, Look::Plain, Some(Icon::External), false, tr("Open worktree"), ready);
    pages::control(w, "agents-open", &r, clip);
    if r.clicked() {
        *action = Some(Action::Open(destination.into()));
    }
    if s.busy() {
        // The one sign of work under way is in the button that stops it.
        let r = button(&mut row, Look::Quiet, None, true, tr(if s.stopping { "Stopping" } else { "Stop" }), !s.stopping);
        pages::control(w, "agents-read-stop", &r, clip);
        if r.clicked() {
            *action = Some(Action::StopRead);
        }
    } else {
        let r = button(&mut row, Look::Quiet, Some(Icon::Refresh), false, tr("Refresh"), true);
        pages::control(w, "agents-refresh", &r, clip);
        if r.clicked() {
            *action = Some(Action::Refresh);
        }
    }
    if let Some(snapshot) = &s.loaded {
        row.with_layout(Layout::left_to_right(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(RichText::new(trn!(snapshot.files.len(), "{n} changed file", "{n} changed files")).font(semibold(13.0)));
            ui.add(Label::new(RichText::new(tr("not committed yet")).size(12.0).color(HINT())).truncate());
        });
    }
    if let Some(e) = &s.error {
        parts::strip(ui, RED(), Icon::Warning, e, None);
    }
    if let Some(n) = &s.notice {
        let text = if n == "Read cancelled. Refresh to check for newer changes." {
            tr("Read cancelled. Refresh to check for newer changes.")
        } else {
            n
        };
        ui.add(Label::new(RichText::new(text).size(12.5).color(MUTED())).wrap());
    }
    let Some(snapshot) = &s.loaded else {
        let rest = ui.available_rect_before_wrap();
        if s.busy() {
            pages::loading(&mut child(ui, "agents-changes-wait", rest), tr("Loading changes"));
        } else {
            parts::centered(ui, rest, Icon::Refresh, MUTED(), tr("Refresh to load changes"), "", 0.0, |_| ());
        }
        return;
    };
    if snapshot.limited {
        ui.add(
            Label::new(
                RichText::new(tr("Showing the first 2,000 changes. Open the worktree to review more."))
                    .size(12.5)
                    .color(MUTED()),
            )
            .wrap(),
        );
    }
    if snapshot.files.is_empty() {
        let rest = ui.available_rect_before_wrap();
        parts::centered(ui, rest, Icon::Check, GREEN(), tr("Worktree is clean"), "", 0.0, |_| ());
        return;
    }
    // In a narrow pane the bounded list sits above the diff. Both keep their own scroll
    // positions; neither can push the tabs or Stop out of view.
    let (rect, _) = ui.allocate_exact_size(ui.available_size().max(Vec2::ZERO), Sense::hover());
    let wide = rect.width() >= 620.0;
    let list = if wide {
        Rect::from_min_max(rect.min, pos2(rect.left() + 280.0, rect.bottom()))
    } else {
        Rect::from_min_max(
            rect.min,
            pos2(rect.right(), rect.top() + (rect.height() * 0.38).clamp(84.0, 170.0)),
        )
    };
    let patch = if wide {
        Rect::from_min_max(pos2(list.right() + 10.0, rect.top()), rect.max)
    } else {
        Rect::from_min_max(pos2(rect.left(), list.bottom() + 8.0), rect.max)
    };
    pages::card_back(ui, list);
    files(&mut child(ui, "agent-files", list.shrink(1.0)), s, snapshot, w, action);
    ui.painter()
        .rect(patch, 8.0, BG(), Stroke::new(1.0, CARD_STROKE()), StrokeKind::Inside);
    diff(&mut child(ui, "agent-diff", patch.shrink(1.0)), s, w);
}

/// A changed file's name and the folder it is in.
fn split(label: &str) -> (&str, &str) {
    match label.rfind('/') {
        Some(at) if at + 1 < label.len() => (&label[at + 1..], &label[..at]),
        _ => (label, ""),
    }
}

fn files(
    ui: &mut Ui,
    s: &Changes,
    snapshot: &crate::agents::changes::Snapshot,
    w: &mut Widgets,
    action: &mut Option<Action>,
) {
    let focus_key = ui.scope_id().with("revealed-selection");
    let focus = (
        s.owner.as_ref().map(|r| r.id),
        s.selected,
        ui.max_rect().height().to_bits(),
    );
    let reveal = ui.data(|d| d.get_temp::<(Option<u64>, Option<usize>, u32)>(focus_key)) != Some(focus);
    let out = ScrollArea::vertical()
        .id_salt(("agent-files", s.owner.as_ref().map(|r| r.id)))
        .auto_shrink([false, false])
        .min_scrolled_height(0.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add_space(4.0);
            for area in AREAS {
                let count = snapshot.files.iter().filter(|f| f.area == area).count();
                if count == 0 {
                    continue;
                }
                let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
                let name = ui
                    .painter()
                    .layout_no_wrap(tr(area.label()).to_uppercase(), semibold(11.0), HINT());
                let after = head.left() + 12.0 + name.size().x + 7.0;
                ui.painter()
                    .galley(pos2(head.left() + 12.0, head.center().y - name.size().y / 2.0), name, HINT());
                ui.painter()
                    .text(pos2(after, head.center().y), Align2::LEFT_CENTER, count, semibold(11.0), FAINT());
                for (i, file) in snapshot.files.iter().enumerate().filter(|(_, f)| f.area == area) {
                    let (r, response) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
                    if reveal && Some(i) == s.selected {
                        ui.scroll_to_rect(r, Some(Align::Center));
                    }
                    let row = r.shrink2(vec2(4.0, 0.0));
                    row_background(ui.painter(), row, Some(i) == s.selected, response.hovered() || response.has_focus());
                    icon(
                        ui.painter(),
                        Rect::from_center_size(pos2(row.left() + 15.0, row.center().y), vec2(13.0, 13.0)),
                        Icon::File,
                        MUTED(),
                    );
                    let label = crate::text::raw_path_label(file.path.as_bytes());
                    let (name, folder) = split(&label);
                    let text = Rect::from_min_max(pos2(row.left() + 30.0, row.top()), pos2(row.right() - 8.0, row.bottom()));
                    let width = ui
                        .painter()
                        .layout_no_wrap(name.to_owned(), FontId::proportional(13.0), TEXT())
                        .size()
                        .x
                        .min(text.width());
                    parts::line(ui, text, name, FontId::proportional(13.0), TEXT());
                    let rest = Rect::from_min_max(pos2(text.left() + width + 8.0, text.top()), text.max);
                    if !folder.is_empty() && rest.width() > 24.0 {
                        parts::line(ui, rest, folder, FontId::proportional(12.0), HINT());
                    }
                    pages::control(w, format!("agents-change-{i}"), &response, ui.clip_rect());
                    if let Some(c) = w.get_mut(&format!("agents-change-{i}")) {
                        c.enabled &= !s.loading() && !s.stopping;
                    }
                    if response.clicked() && !s.loading() && !s.stopping {
                        *action = Some(Action::File(i));
                    }
                    response.on_hover_text(label);
                }
                ui.add_space(4.0);
            }
        });
    pages::region(w, "agents-changes-scroll", out.inner_rect, ui.is_enabled());
    ui.data_mut(|d| d.insert_temp(focus_key, focus));
}

fn diff(ui: &mut Ui, s: &Changes, w: &mut Widgets) {
    ui.spacing_mut().item_spacing.y = 0.0;
    // The file's path with what was added and removed, as the card's title row.
    if let Some(file) = s.selected_file() {
        let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
        ui.painter()
            .rect_filled(bar, CornerRadius { nw: 7, ne: 7, sw: 0, se: 0 }, PANEL());
        ui.painter()
            .hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, CARD_LINE()));
        let mut right = bar.right() - 14.0;
        if let Some(p) = &s.patch {
            let lines = || p.files.iter().flat_map(|f| f.lines.iter());
            let added = lines().filter(|l| l.kind == Kind::Added).count();
            let removed = lines().filter(|l| l.kind == Kind::Removed).count();
            for (text, color) in [(format!("−{removed}"), RED()), (format!("+{added}"), GREEN())] {
                let galley = ui.painter().layout_no_wrap(text, FontId::monospace(12.0), color);
                right -= galley.size().x;
                ui.painter()
                    .galley(pos2(right, bar.center().y - galley.size().y / 2.0), galley, color);
                right -= 8.0;
            }
        }
        parts::line(
            ui,
            Rect::from_min_max(pos2(bar.left() + 14.0, bar.top()), pos2(right, bar.bottom())),
            &crate::text::raw_path_label(file.path.as_bytes()),
            FontId::monospace(12.0),
            TEXT_SOFT(),
        );
    }
    let Some(p) = &s.patch else {
        let rest = ui.available_rect_before_wrap();
        if s.busy() {
            pages::loading(&mut child(ui, "agent-diff-wait", rest), tr("Loading diff"));
        } else {
            parts::centered(ui, rest, Icon::File, MUTED(), tr("Select a file to view its diff"), "", 0.0, |_| ());
        }
        return;
    };
    let note = |ui: &mut Ui, text: &str| {
        Frame::new().inner_margin(Margin::symmetric(14, 6)).show(ui, |ui| {
            ui.add(Label::new(RichText::new(text).size(12.5).color(MUTED())).wrap());
        });
    };
    if let Some(n) = &p.notice {
        note(ui, n);
    }
    if p.limited {
        note(ui, tr("Diff preview is limited. Open the worktree to review more."));
    }
    if p.files.iter().all(|f| f.lines.is_empty()) {
        note(ui, tr("No text diff. Open the worktree for full review."));
        return;
    }
    ui.add_space(4.0);
    ui.spacing_mut().interact_size.y = 21.0;
    let total = p.files.iter().map(|f| f.lines.len()).sum();
    let out = ScrollArea::both()
        .id_salt(("agent-diff", s.owner.as_ref().map(|r| r.id), s.selected))
        .auto_shrink([false, false])
        .min_scrolled_height(0.0)
        .show_rows(ui, 21.0, total, |ui, range| {
            let mut offset = 0;
            for file in &p.files {
                for i in range.start.saturating_sub(offset)..range.end.saturating_sub(offset).min(file.lines.len()) {
                    let row = &file.lines[i];
                    let (ink, tint) = match row.kind {
                        Kind::Added => (GREEN(), Some(GREEN())),
                        Kind::Removed => (RED(), Some(RED())),
                        Kind::Hunk => (BLUE(), Some(BLUE())),
                        Kind::Header => (MUTED(), None),
                        Kind::Context => (TEXT(), None),
                    };
                    // Exact row geometry: no inherited spacing or wrapped labels.
                    let galley = ui
                        .painter()
                        .layout_no_wrap(row.text.clone(), FontId::monospace(12.0), ink);
                    let (r, _) = ui.allocate_exact_size(
                        vec2((galley.size().x + 72.0).max(ui.available_width()), 21.0),
                        Sense::hover(),
                    );
                    if let Some(tint) = tint {
                        ui.painter().rect_filled(r, 0.0, tint.gamma_multiply(0.08));
                    }
                    ui.painter().text(
                        pos2(r.left() + 32.0, r.center().y),
                        Align2::RIGHT_CENTER,
                        row.old.map_or(String::new(), |n| n.to_string()),
                        FontId::monospace(10.0),
                        HINT(),
                    );
                    ui.painter().text(
                        pos2(r.left() + 62.0, r.center().y),
                        Align2::RIGHT_CENTER,
                        row.new.map_or(String::new(), |n| n.to_string()),
                        FontId::monospace(10.0),
                        HINT(),
                    );
                    ui.painter()
                        .galley(pos2(r.left() + 72.0, r.center().y - galley.size().y / 2.0), galley, ink);
                }
                offset += file.lines.len();
            }
        });
    pages::region(w, "agents-diff-scroll", out.inner_rect, ui.is_enabled());
}
