use super::*;
use crate::{agents::changes::State as Changes, diff::Kind};

pub(super) fn show(
    ui: &mut Ui,
    s: &Changes,
    w: &mut Widgets,
    destination: &std::path::Path,
    ready: bool,
    action: &mut Option<Action>,
) {
    ui.horizontal_wrapped(|ui| {
        if button(ui, w, "agents-refresh", "Refresh", !s.busy()) {
            *action = Some(Action::Refresh);
        }
        if s.busy() {
            ui.spinner();
            if button(
                ui,
                w,
                "agents-read-stop",
                if s.stopping { "Stopping" } else { "Stop" },
                !s.stopping,
            ) {
                *action = Some(Action::StopRead);
            }
        }
        if button(ui, w, "agents-open", "Review changes", ready) {
            *action = Some(Action::Open(destination.into()));
        }
    });
    if let Some(e) = &s.error {
        ui.add(Label::new(RichText::new(e).color(RED())).wrap());
    }
    if let Some(n) = &s.notice {
        ui.add(
            Label::new(
                if n == "Read cancelled. Refresh to check for newer changes." {
                    tr("Read cancelled. Refresh to check for newer changes.")
                } else {
                    n
                },
            )
            .wrap(),
        );
    }
    let Some(snapshot) = &s.loaded else {
        ui.label(tr(if s.busy() {
            "Loading changes"
        } else {
            "Refresh to load changes"
        }));
        return;
    };
    if snapshot.limited {
        ui.label(tr(
            "Showing the first 2,000 changes. Open the worktree to review more.",
        ));
    }
    if snapshot.files.is_empty() {
        ui.add_space(20.0);
        ui.heading(tr("Worktree is clean"));
        return;
    }
    // Stack the bounded file list above the diff in compact windows. Both keep
    // independent scroll positions; neither can push Stop or the tabs offscreen.
    let (rect, _) = ui.allocate_exact_size(ui.available_size().max(vec2(0.0, 0.0)), Sense::hover());
    let wide = rect.width() >= 620.0;
    let list = if wide {
        Rect::from_min_max(rect.min, pos2(rect.left() + 230.0, rect.bottom()))
    } else {
        Rect::from_min_max(
            rect.min,
            pos2(
                rect.right(),
                rect.top() + (rect.height() * 0.35).clamp(70.0, 145.0),
            ),
        )
    };
    let patch = if wide {
        Rect::from_min_max(pos2(list.right() + 10.0, rect.top()), rect.max)
    } else {
        Rect::from_min_max(pos2(rect.left(), list.bottom() + 8.0), rect.max)
    };
    let mut files = child(ui, "agent-files", list);
    let focus_key = files.scope_id().with("revealed-selection");
    let focus = (
        s.owner.as_ref().map(|r| r.id),
        s.selected,
        list.height().to_bits(),
    );
    let reveal =
        files.data(|d| d.get_temp::<(Option<u64>, Option<usize>, u32)>(focus_key)) != Some(focus);
    let out = ScrollArea::vertical()
        .id_salt(("agent-files", s.owner.as_ref().map(|r| r.id)))
        .auto_shrink([false, false])
        .min_scrolled_height(0.0)
        .show(&mut files, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (i, file) in snapshot.files.iter().enumerate() {
                let (r, response) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click());
                if reveal && Some(i) == s.selected {
                    ui.scroll_to_rect(r, Some(Align::Center));
                }
                if Some(i) == s.selected || response.hovered() {
                    ui.painter().rect_filled(
                        r,
                        3.0,
                        if Some(i) == s.selected {
                            ui.visuals().selection.bg_fill
                        } else {
                            RAISED()
                        },
                    );
                }
                let label = crate::text::raw_path_label(file.path.as_bytes());
                let text = r.shrink2(vec2(8.0, 2.0));
                line(
                    ui,
                    Rect::from_min_size(text.min, vec2(text.width(), 22.0)),
                    &label,
                    FontId::proportional(12.0),
                    TEXT(),
                );
                line(
                    ui,
                    Rect::from_min_size(text.min + vec2(0.0, 22.0), vec2(text.width(), 16.0)),
                    tr(file.area.label()),
                    FontId::proportional(11.0),
                    HINT(),
                );
                pages::control(w, format!("agents-change-{i}"), &response, ui.clip_rect());
                if let Some(c) = w.get_mut(&format!("agents-change-{i}")) {
                    c.enabled &= !s.loading() && !s.stopping;
                }
                if response.clicked() && !s.loading() && !s.stopping {
                    *action = Some(Action::File(i));
                }
                response.on_hover_text(label);
            }
        });
    pages::region(w, "agents-changes-scroll", out.inner_rect, ui.is_enabled());
    files.data_mut(|d| d.insert_temp(focus_key, focus));
    let mut body = child(ui, "agent-diff", patch);
    if let Some(file) = s.selected_file() {
        body.add(
            Label::new(
                RichText::new(crate::text::raw_path_label(file.path.as_bytes()))
                    .monospace()
                    .color(HINT()),
            )
            .truncate(),
        );
    }
    let Some(p) = &s.patch else {
        body.label(tr(if s.busy() {
            "Loading diff"
        } else {
            "Select a file to view its diff"
        }));
        return;
    };
    if let Some(n) = &p.notice {
        body.add(Label::new(n).wrap());
    }
    if p.limited {
        body.label(tr(
            "Diff preview is limited. Open the worktree to review more.",
        ));
    }
    if p.files.iter().all(|f| f.lines.is_empty()) {
        body.label(tr("No text diff. Open the worktree for full review."));
        return;
    }
    body.spacing_mut().item_spacing.y = 0.0;
    body.spacing_mut().interact_size.y = 21.0;
    let total = p.files.iter().map(|f| f.lines.len()).sum();
    let out = ScrollArea::both()
        .id_salt(("agent-diff", s.owner.as_ref().map(|r| r.id), s.selected))
        .auto_shrink([false, false])
        .min_scrolled_height(0.0)
        .show_rows(&mut body, 21.0, total, |ui, range| {
            let mut offset = 0;
            for file in &p.files {
                for i in range.start.saturating_sub(offset)
                    ..range.end.saturating_sub(offset).min(file.lines.len())
                {
                    let row = &file.lines[i];
                    let (ink, tint) = match row.kind {
                        Kind::Added => (GREEN(), Some(GREEN())),
                        Kind::Removed => (RED(), Some(RED())),
                        Kind::Hunk => (BLUE(), Some(BLUE())),
                        Kind::Header => (MUTED(), None),
                        Kind::Context => (TEXT(), None),
                    };
                    // Exact row geometry: no inherited spacing or wrapped labels.
                    let galley =
                        ui.painter()
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
                    ui.painter().galley(
                        pos2(r.left() + 72.0, r.center().y - galley.size().y / 2.0),
                        galley,
                        ink,
                    );
                }
                offset += file.lines.len();
            }
        });
    pages::region(w, "agents-diff-scroll", out.inner_rect, ui.is_enabled());
}
