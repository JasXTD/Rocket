//! Bringing a session's committed work back: three steps, each with its one action.
//! Nothing here commits, merges or pushes; the steps open the screens that do.
use super::*;
use crate::{agents::integration::State as Integration, i18n::trn};

pub(super) fn show(
    ui: &mut Ui,
    s: &Session,
    state: &Integration,
    w: &mut Widgets,
    ready: bool,
    action: &mut Option<Action>,
) {
    let out = ScrollArea::vertical()
        .id_salt("agents-integration")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            Frame::new()
                .inner_margin(Margin { left: 18, right: 18, top: 14, bottom: 16 })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width().min(760.0));
                    ui.spacing_mut().item_spacing.y = 10.0;
                    if s.active() {
                        parts::strip(ui, AMBER(), Icon::Warning, tr("Stop this session before integration."), None);
                    }
                    if let Some(e) = &state.error {
                        parts::strip(ui, RED(), Icon::Warning, e, None);
                    }
                    steps(ui, s, state, w, ready, action);
                });
        });
    pages::region(w, "agents-integration-scroll", out.inner_rect, ui.is_enabled());
}

fn steps(ui: &mut Ui, s: &Session, state: &Integration, w: &mut Widgets, ready: bool, action: &mut Option<Action>) {
    let idle = ready && !s.active();
    Frame::new()
        .fill(PANEL())
        .stroke(Stroke::new(1.0, CARD_STROKE()))
        .corner_radius(8)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            step(
                ui,
                1,
                false,
                tr("Commit what you want to keep"),
                tr("Rocket commits nothing for you."),
                w,
                action,
                |ui, w, action| {
                    let r = button(ui, Look::Plain, Some(Icon::External), false, tr("Open worktree"), ready);
                    pages::control(w, "agents-open", &r, ui.clip_rect());
                    if r.clicked() {
                        *action = Some(Action::Open(s.record.destination.clone()));
                    }
                },
                |_, _, _| (),
            );
            line_between(ui);
            step(
                ui,
                2,
                state.review.is_some(),
                tr("Check it against the original checkout"),
                tr("Both checkouts must be clean."),
                w,
                action,
                |ui, w, action| {
                    if state.task.is_some() {
                        let label = tr(if state.stopping { "Stopping" } else { "Stop" });
                        let r = button(ui, Look::Plain, None, true, label, !state.stopping);
                        pages::control(w, "agents-integration-stop", &r, ui.clip_rect());
                        if r.clicked() {
                            *action = Some(Action::StopIntegration);
                        }
                    } else {
                        let label = tr(if state.review.is_some() { "Check again" } else { "Check integration" });
                        let r = button(ui, Look::Plain, None, false, label, idle);
                        pages::control(w, "agents-integration-check", &r, ui.clip_rect());
                        if r.clicked() {
                            *action = Some(Action::CheckIntegration);
                        }
                    }
                },
                |ui, w, action| {
                    let Some(r) = &state.review else { return };
                    ui.add_space(9.0);
                    pages::well(ui, |ui| {
                        let into = match r.merge.head() {
                            rocket_git::Head::Branch { name, .. } => String::from_utf8_lossy(name),
                            _ => std::borrow::Cow::Borrowed("Detached"),
                        };
                        pages::key_value(ui, tr("Merge into"), 110.0, |ui| {
                            ui.add(Label::new(RichText::new(into.as_ref()).monospace().size(12.5)).wrap());
                            ui.add(
                                Label::new(
                                    RichText::new(crate::views::display_path(&r.owner.source))
                                        .size(12.0)
                                        .color(HINT()),
                                )
                                .wrap(),
                            );
                        });
                        pages::key_value(ui, tr("Agent branch"), 110.0, |ui| {
                            ui.add(Label::new(RichText::new(&r.owner.branch).monospace().size(12.5)).wrap());
                        });
                        ui.label(format!(
                            "{} · {}",
                            trn!(r.merge.incoming(), "{n} incoming commit", "{n} incoming commits"),
                            trn!(r.comparison.files.len(), "{n} changed file", "{n} changed files")
                        ));
                        ui.add_space(2.0);
                        let b = button(ui, Look::Plain, None, false, tr("Compare committed changes"), idle);
                        pages::control(w, "agents-integration-compare", &b, ui.clip_rect());
                        if b.clicked() {
                            *action = Some(Action::CompareIntegration);
                        }
                    });
                },
            );
            line_between(ui);
            step(
                ui,
                3,
                false,
                tr("Merge"),
                tr("Opens the merge review. Nothing is pushed."),
                w,
                action,
                |ui, w, action| {
                    let enabled = idle && state.review.as_ref().is_some_and(|r| r.merge.incoming() > 0);
                    let r = button(ui, Look::Primary, None, false, tr("Review merge"), enabled);
                    pages::control(w, "agents-integration-merge", &r, ui.clip_rect());
                    if r.clicked() {
                        *action = Some(Action::MergeIntegration);
                    }
                },
                |_, _, _| (),
            );
        });
}

fn line_between(ui: &mut Ui) {
    let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter()
        .hline(line.x_range(), line.center().y, Stroke::new(1.0, CARD_LINE()));
}

/// One step: its number (a check once done), what to do in two lines, its action at the
/// right, and `under` below the lines for what the step found.
#[allow(clippy::too_many_arguments)]
fn step(
    ui: &mut Ui,
    number: usize,
    done: bool,
    title: &str,
    about: &str,
    w: &mut Widgets,
    action: &mut Option<Action>,
    act: impl FnOnce(&mut Ui, &mut Widgets, &mut Option<Action>),
    under: impl FnOnce(&mut Ui, &mut Widgets, &mut Option<Action>),
) {
    Frame::new()
        .inner_margin(Margin { left: 16, right: 14, top: 13, bottom: 13 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                let (mark, _) = ui.allocate_exact_size(vec2(22.0, 34.0), Sense::hover());
                let center = pos2(mark.center().x, mark.top() + 17.0);
                if done {
                    ui.painter().circle_stroke(center, 10.5, Stroke::new(1.5, GREEN()));
                    icon(ui.painter(), Rect::from_center_size(center, vec2(11.0, 11.0)), Icon::Check, GREEN());
                } else {
                    ui.painter().circle_stroke(center, 10.5, Stroke::new(1.5, BORDER()));
                    ui.painter()
                        .text(center, Align2::CENTER_CENTER, number, semibold(12.0), MUTED());
                }
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
                    // The action first, from the right; the words take what is left.
                    let mut right = ui.new_child(
                        UiBuilder::new()
                            .max_rect(row)
                            .layout(Layout::right_to_left(Align::Center)),
                    );
                    act(&mut right, w, action);
                    let words = Rect::from_min_max(row.min, pos2(right.min_rect().left() - 12.0, row.bottom()));
                    parts::line(
                        ui,
                        Rect::from_min_max(words.min, pos2(words.right(), words.top() + 17.0)),
                        title,
                        semibold(13.0),
                        TEXT(),
                    );
                    parts::line(
                        ui,
                        Rect::from_min_max(pos2(words.left(), words.top() + 17.0), words.max),
                        about,
                        FontId::proportional(12.0),
                        HINT(),
                    );
                    under(ui, w, action);
                });
            });
        });
}
