//! What a session is and how it was started, in cards: Session, Command, Activity and
//! Manage. Two columns where there is room, one where there is not.
use super::*;

/// The cards' label column.
const KEY: f32 = 132.0;

pub(super) fn show(ui: &mut Ui, s: &Session, w: &mut Widgets, ready: bool, action: &mut Option<Action>) {
    let out = ScrollArea::vertical()
        .id_salt(("agent-details", s.record.id))
        .auto_shrink([false, false])
        .min_scrolled_height(0.0)
        .show(ui, |ui| {
            Frame::new()
                .inner_margin(Margin { left: 18, right: 18, top: 14, bottom: 16 })
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 12.0;
                    if let Some(e) = &s.error {
                        parts::strip(ui, RED(), Icon::Warning, e, None);
                    }
                    let width = ui.available_width();
                    if width < 640.0 {
                        session(ui, s, w);
                        command(ui, s);
                        activity(ui, s);
                        manage(ui, s, w, ready, action);
                        return;
                    }
                    let left = ((width - 12.0) * 0.54).floor();
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        ui.allocate_ui_with_layout(vec2(left, 0.0), Layout::top_down(Align::Min), |ui| {
                            ui.set_width(left);
                            ui.spacing_mut().item_spacing.y = 12.0;
                            session(ui, s, w);
                            command(ui, s);
                        });
                        let right = ui.available_width();
                        ui.allocate_ui_with_layout(vec2(right, 0.0), Layout::top_down(Align::Min), |ui| {
                            ui.set_width(right);
                            ui.spacing_mut().item_spacing.y = 12.0;
                            activity(ui, s);
                            manage(ui, s, w, ready, action);
                        });
                    });
                });
        });
    pages::region(w, "agents-details-scroll", out.inner_rect, ui.is_enabled());
}

fn mono(ui: &mut Ui, text: &str) {
    ui.add(Label::new(RichText::new(text).monospace().size(12.5)).wrap());
}

/// A value with a copy button after it, which `id` names.
fn copyable(ui: &mut Ui, w: &mut Widgets, id: &str, value: &str) {
    // From the top, in a row no taller than its button: the value stays level with its label.
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.spacing_mut().interact_size.y = 20.0;
        let room = (ui.available_width() - 26.0).max(60.0);
        ui.allocate_ui_with_layout(vec2(room, 18.0), Layout::left_to_right(Align::Min), |ui| {
            mono(ui, value);
        });
        let r = parts::copy(ui, value);
        pages::control(w, id, &r, ui.clip_rect());
    });
}

fn session(ui: &mut Ui, s: &Session, w: &mut Widgets) {
    parts::card(ui, tr("Session"), |_| (), |ui| {
        pages::key_value(ui, tr("Provider"), KEY, |ui| {
            ui.label(tr(s.record.provider.label()));
        });
        pages::key_value(ui, tr("Branch"), KEY, |ui| copyable(ui, w, "agents-copy-branch", &s.record.branch));
        pages::key_value(ui, tr("Base"), KEY, |ui| mono(ui, &s.record.base));
        pages::key_value(ui, tr("Worktree folder"), KEY, |ui| {
            copyable(ui, w, "agents-copy-folder", &crate::views::display_path(&s.record.destination));
        });
        if let Some(id) = &s.record.conversation {
            pages::key_value(ui, tr("Conversation ID"), KEY, |ui| copyable(ui, w, "agents-copy-conversation", id));
        }
        pages::key_value(ui, tr("Conversation activity"), KEY, |ui| {
            ui.label(tr(s.activity_state.label()));
        });
        if let Some(d) = s.elapsed() {
            pages::key_value(ui, tr("Elapsed"), KEY, |ui| {
                ui.label(duration(d));
            });
        }
        if let Some(code) = s.exit_code {
            pages::key_value(ui, tr("Exit code"), KEY, |ui| {
                ui.label(code.to_string());
            });
        }
    });
}

/// A program and its arguments, one chip for each, exactly as they are passed.
fn program(ui: &mut Ui, label: &str, c: &Command) {
    ui.label(RichText::new(label).size(12.0).color(HINT()));
    mono(ui, &c.program);
    let args = c.args();
    if args.is_empty() {
        return;
    }
    chips(ui, &args);
}

/// One chip for each argument, wrapped to the width at hand. A Frame in a wrapping row
/// is placed as one block and never wraps, so the chips are measured and placed here.
/// An argument longer than the row is cut; hovering shows all of it.
fn chips(ui: &mut Ui, args: &[String]) {
    const HEIGHT: f32 = 22.0;
    const GAP: f32 = 6.0;
    const INSET: f32 = 7.0;
    let width = ui.available_width().max(60.0);
    let mut placed = Vec::with_capacity(args.len());
    let (mut x, mut y) = (0.0, 0.0);
    for arg in args {
        let mut job = text::LayoutJob::simple_singleline(arg.clone(), FontId::monospace(12.0), TEXT());
        job.wrap = text::TextWrapping::truncate_at_width(width - 2.0 * INSET);
        let galley = ui.painter().layout_job(job);
        let chip = galley.size().x + 2.0 * INSET;
        if x > 0.0 && x + chip > width {
            x = 0.0;
            y += HEIGHT + GAP;
        }
        placed.push((vec2(x, y), chip, galley));
        x += chip + GAP;
    }
    let (rect, _) = ui.allocate_exact_size(vec2(width, y + HEIGHT), Sense::hover());
    for (i, ((at, chip, galley), arg)) in placed.into_iter().zip(args).enumerate() {
        let area = Rect::from_min_size(rect.min + at, vec2(chip, HEIGHT));
        ui.painter()
            .rect(area, 5.0, RAISED(), Stroke::new(1.0, HAIRLINE()), StrokeKind::Inside);
        ui.painter().galley(
            pos2(area.left() + INSET, area.center().y - galley.size().y / 2.0),
            galley,
            TEXT(),
        );
        ui.interact(area, ui.id().with(("argument", i)), Sense::hover())
            .on_hover_text(arg);
    }
}

fn command(ui: &mut Ui, s: &Session) {
    parts::card(ui, tr("Command"), |_| (), |ui| {
        ui.spacing_mut().item_spacing.y = 5.0;
        program(ui, tr("Agent executable"), &s.record.command);
        if let Some(setup) = &s.record.setup {
            ui.add_space(5.0);
            program(ui, tr("Setup command"), setup);
        }
    });
}

fn activity(ui: &mut Ui, s: &Session) {
    parts::card(ui, tr("Session activity"), |_| (), |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for e in s.activity.iter().rev() {
            let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
            let tint = match e.phase {
                Phase::Running => GREEN(),
                Phase::Failed => RED(),
                Phase::Saving | Phase::Starting | Phase::Setup | Phase::Stopping => BLUE(),
                Phase::Restored | Phase::Stopped | Phase::Exited => FAINT(),
            };
            ui.painter()
                .circle_filled(pos2(row.left() + 4.0, row.center().y), 3.5, tint);
            let age = e.at.elapsed().as_secs();
            let when = ui
                .painter()
                .layout_no_wrap(ago(age, unix_now() - age as i64), FontId::proportional(12.0), HINT());
            let right = row.right() - when.size().x;
            ui.painter()
                .galley(pos2(right, row.center().y - when.size().y / 2.0), when, HINT());
            parts::line(
                ui,
                Rect::from_min_max(pos2(row.left() + 18.0, row.top()), pos2(right - 10.0, row.bottom())),
                tr(e.phase.label()),
                FontId::proportional(13.0),
                TEXT(),
            );
        }
        ui.add_space(6.0);
        pages::note(ui, tr("Time since each event; current app session only."));
    });
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn manage(ui: &mut Ui, s: &Session, w: &mut Widgets, ready: bool, action: &mut Option<Action>) {
    let idle = ready && !s.active();
    Frame::new()
        .fill(PANEL())
        .stroke(Stroke::new(1.0, CARD_STROKE()))
        .corner_radius(8)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
            ui.painter()
                .hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, CARD_LINE()));
            parts::line(
                ui,
                Rect::from_min_max(bar.min + vec2(14.0, 0.0), bar.max - vec2(12.0, 0.0)),
                tr("Manage"),
                semibold(13.0),
                TEXT(),
            );
            let mut first = true;
            let mut entry = |ui: &mut Ui, title: &str, about: &str, act: &mut dyn FnMut(&mut Ui)| {
                if !first {
                    let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                    ui.painter()
                        .hline(line.x_range(), line.center().y, Stroke::new(1.0, CARD_LINE()));
                }
                first = false;
                let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 54.0), Sense::hover());
                let inner = Rect::from_min_max(row.min + vec2(14.0, 0.0), row.max - vec2(12.0, 0.0));
                let mut right = ui.new_child(
                    UiBuilder::new()
                        .max_rect(inner)
                        .layout(Layout::right_to_left(Align::Center)),
                );
                act(&mut right);
                let words = Rect::from_min_max(inner.min, pos2(right.min_rect().left() - 12.0, inner.bottom()));
                parts::line(
                    ui,
                    Rect::from_min_max(pos2(words.left(), words.top() + 9.0), pos2(words.right(), words.top() + 27.0)),
                    title,
                    semibold(13.0),
                    TEXT(),
                );
                parts::line(
                    ui,
                    Rect::from_min_max(pos2(words.left(), words.top() + 27.0), pos2(words.right(), words.bottom() - 9.0)),
                    about,
                    FontId::proportional(12.0),
                    HINT(),
                );
            };
            // With a conversation to resume, the header offers Resume; this starts a new one.
            if s.resumable() {
                entry(ui, tr("Start a new conversation"), tr("Checks the folder and program again first."), &mut |ui| {
                    let r = button(ui, Look::Plain, None, false, tr("Review and restart"), idle);
                    pages::control(w, "agents-restart", &r, ui.clip_rect());
                    if r.clicked() {
                        *action = Some(Action::Restart);
                    }
                });
            }
            entry(ui, tr("Remove worktree"), tr("Only when it is clean. The branch stays."), &mut |ui| {
                let r = button(ui, Look::Danger, None, false, tr("Remove worktree"), idle);
                pages::control(w, "agents-remove", &r, ui.clip_rect());
                if r.clicked() {
                    *action = Some(Action::Remove);
                }
            });
            entry(ui, tr("Forget session"), tr("Keeps the folder and the branch."), &mut |ui| {
                let r = button(ui, Look::Plain, None, false, tr("Forget session"), idle);
                pages::control(w, "agents-forget", &r, ui.clip_rect());
                if r.clicked() {
                    *action = Some(Action::Forget);
                }
            });
        });
}
