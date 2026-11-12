//! The session dialog: what to start and where, then a review of exactly that. The
//! dialog closes by its × or Escape only; its footer holds the one way forward.
use super::*;

/// The cards' label column in the review.
const KEY: f32 = 124.0;
/// The Browse button's least width, as in the other dialogs.
const BROWSE: f32 = 76.0;

/// `covered`: a Browse picker is above the dialog, and Escape is the picker's.
pub(super) fn show(
    ctx: &Context,
    w: &mut Widgets,
    f: &mut Form,
    chosen: Chosen,
    ready: bool,
    covered: bool,
    action: &mut Option<Action>,
) {
    let removing = f.mode == Mode::Remove;
    let title = match f.mode {
        Mode::Remove => "Remove worktree",
        Mode::Resume => "Resuming conversation",
        Mode::Restart => "Review and restart",
        Mode::Create => "New agent session",
    };
    let subtitle = match (f.mode, f.plan.is_some()) {
        (Mode::Remove, _) => "Review session",
        (Mode::Create, false) => "Its own branch and folder for one task",
        (Mode::Create, true) => "Check what will be created and run",
        (_, false) => "A new conversation in the same worktree",
        (_, true) => "Check what will run",
    };
    let head = pages::ReviewHeader {
        image: Icon::Worktree,
        title: tr(title),
        subtitle: tr(subtitle),
        danger: removing,
    };
    let response = pages::review_at(ctx, "agents-review-dialog", 680.0, head, w, "agents-scroll", true, |ui, part, w| {
        if part == pages::Part::Footer {
            footer(ui, w, f, ready, action);
            return;
        }
        ui.spacing_mut().item_spacing.y = 12.0;
        if let Some(e) = &f.error {
            parts::strip(ui, RED(), Icon::Warning, e, None);
        }
        if let Some(plan) = &f.plan {
            review(ui, plan, f.mode);
        } else {
            ui.add_enabled_ui(f.task.is_none(), |ui| edit(ui, w, f, chosen, action));
        }
    });
    if let Some(close) = w.get("dialog-close").cloned() {
        w.insert("agents-cancel".into(), close);
    }
    // Resizing the dialog can move its boundary under the pointer. Only X/Escape
    // dismiss, never a click that became outside after a compact-window reflow.
    if response.close_clicked() || (!covered && ctx.input(|i| i.key_pressed(Key::Escape))) {
        *action = Some(Action::Cancel);
    }
}

/// Whether a program's path names a file that is there. A preset can name where its
/// program is installed by default without that file existing (Claude Code's does).
pub(super) fn present(program: &str) -> bool {
    !program.is_empty() && std::path::Path::new(program).is_file()
}

/// Whether the dialog on screen shows its program, arguments and setup fields.
fn open_id() -> Id {
    Id::new("agents-program-fields-open")
}

/// No dialog is open: the fields the last one opened are not carried into the next.
pub(super) fn forget(ctx: &Context) {
    ctx.data_mut(|d| d.remove::<bool>(open_id()));
}

/// Buttons are laid out from the right; the status takes what is left.
fn footer(ui: &mut Ui, w: &mut Widgets, f: &Form, ready: bool, action: &mut Option<Action>) {
    let checking = f.task.is_some();
    if f.mode == Mode::Resume {
        if checking {
            pages::footer_status(ui, true, tr("Checking worktree and executables"));
        }
        return;
    }
    if f.plan.is_some() {
        let r = match f.mode {
            Mode::Remove => pages::danger(ui, ready, tr("Remove clean worktree")),
            Mode::Restart => pages::primary(ui, ready, tr("Start session")),
            _ => pages::primary(ui, ready, tr("Create and start")),
        };
        pages::control(w, "agents-confirm", &r, ui.clip_rect());
        if r.clicked() {
            *action = Some(Action::Confirm);
        }
        return;
    }
    let r = pages::primary(ui, ready && !checking, tr("Review session"));
    pages::control(w, "agents-review", &r, ui.clip_rect());
    if r.clicked() {
        *action = Some(Action::Preview);
    }
    if checking {
        pages::footer_status(ui, true, tr("Checking worktree and executables"));
    }
}

fn mono(ui: &mut Ui, text: &str) {
    ui.add(Label::new(RichText::new(text).monospace().size(12.5)).wrap());
}

fn card_title(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).font(semibold(13.0)));
}

/// A program with each argument on its own line, quoted, so spaces and empty arguments
/// show for what they are.
fn program(ui: &mut Ui, label: &str, c: &Command) {
    pages::key_value(ui, label, KEY, |ui| {
        mono(ui, &c.program);
        for a in c.args() {
            mono(ui, &format!("{a:?}"));
        }
    });
}

/// The checked plan: what is created or removed, and what runs.
fn review(ui: &mut Ui, plan: &Plan, mode: Mode) {
    ui.add(Label::new(RichText::new(&plan.record.name).font(semibold(14.0))).wrap());
    let what = match mode {
        Mode::Create => "Creates",
        Mode::Remove => "Removes",
        _ => "Worktree",
    };
    pages::titled_card(ui, |ui| card_title(ui, tr(what)), |ui| {
        let branch = if mode == Mode::Create { "New branch" } else { "Branch" };
        pages::key_value(ui, tr(branch), KEY, |ui| mono(ui, &plan.record.branch));
        pages::key_value(ui, tr("Base"), KEY, |ui| mono(ui, plan.target.as_str()));
        pages::key_value(ui, tr("Worktree folder"), KEY, |ui| {
            mono(ui, &crate::views::display_path(&plan.record.destination));
        });
    });
    if mode == Mode::Remove {
        parts::strip(
            ui,
            AMBER(),
            Icon::Warning,
            tr("Only a clean, unlocked worktree can be removed. The branch and recovery reference are kept."),
            None,
        );
        return;
    }
    pages::titled_card(ui, |ui| card_title(ui, tr("Runs")), |ui| {
        program(ui, tr("Agent executable"), &plan.record.command);
        if let Some(p) = &plan.program {
            pages::key_value(ui, "SHA-256", KEY, |ui| {
                ui.add(Label::new(RichText::new(&p.digest).monospace().size(11.5).color(HINT())).wrap());
            });
        }
        if let Some(setup) = &plan.record.setup {
            program(ui, tr("Setup command"), setup);
        }
    });
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (mark, _) = ui.allocate_exact_size(vec2(14.0, 18.0), Sense::hover());
        icon(ui.painter(), Rect::from_center_size(mark.center(), vec2(14.0, 14.0)), Icon::Warning, AMBER());
        ui.add(Label::new(RichText::new(tr("Runs with your account's access.")).color(AMBER())).wrap());
    });
}

/// A labelled text field as wide as its place.
fn field(ui: &mut Ui, w: &mut Widgets, id: &str, label: &'static str, value: &mut String, multiline: bool) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 5.0;
        ui.label(RichText::new(tr(label)).color(HINT()).size(12.0));
        // A dialog field's height, as in the other dialogs.
        let edit = if multiline {
            TextEdit::multiline(value).desired_rows(2)
        } else {
            TextEdit::singleline(value)
                .min_size(vec2(0.0, 30.0))
                .vertical_align(Align::Center)
        };
        let r = ui.add(edit.desired_width(f32::INFINITY).id_salt(id));
        pages::control(w, id, &r, ui.clip_rect());
    });
}

/// What to start and where. Everything a first session needs is in view; the program's
/// path, its arguments and a setup command are folded away until they are needed.
fn edit(ui: &mut Ui, w: &mut Widgets, f: &mut Form, chosen: Chosen, action: &mut Option<Action>) {
    ui.spacing_mut().item_spacing.y = 12.0;
    let clip = ui.clip_rect();
    if f.mode != Mode::Create {
        // An existing session: its folder and branch are what they are.
        pages::well(ui, |ui| {
            pages::key_value(ui, tr("Worktree folder"), KEY, |ui| {
                mono(ui, &crate::views::display_path(std::path::Path::new(&f.destination)));
            });
            pages::key_value(ui, tr("Branch"), KEY, |ui| mono(ui, &f.record.branch));
        });
    }
    if matches!(f.mode, Mode::Remove | Mode::Resume) {
        return;
    }
    field(ui, w, "agents-name", "Task name", &mut f.record.name, false);
    // Whether each known program is on this PC, looked up once for this dialog and
    // again after Browse chose one.
    let mut found = *f
        .found
        .get_or_insert_with(|| Provider::ALL.map(|p| present(&p.installed(chosen))));
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 5.0;
        ui.label(RichText::new(tr("Provider")).color(HINT()).size(12.0));
        let cards: Vec<(bool, &str, &str)> = Provider::ALL
            .iter()
            .zip(found)
            .map(|(p, found)| {
                let about = match p {
                    Provider::Custom => "Your own program",
                    _ if found => "Found",
                    _ => "Not found",
                };
                (f.record.provider == *p, tr(p.label()), tr(about))
            })
            .collect();
        let responses = pages::option_cards(ui, true, &cards);
        for ((i, p), r) in Provider::ALL.into_iter().enumerate().zip(responses) {
            pages::control(w, format!("agents-provider-{}", p.key()), &r, clip);
            if r.clicked() {
                f.record.provider = p;
                f.record.command = p.command(chosen);
                // Choosing looks again, so a program installed meanwhile is found.
                found[i] = present(&f.record.command.program);
                f.found = Some(found);
            }
        }
    });
    if f.mode == Mode::Create {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let base = (ui.available_width() * 0.34).clamp(140.0, 220.0);
            ui.allocate_ui_with_layout(vec2(base, 0.0), Layout::top_down(Align::Min), |ui| {
                ui.set_width(base);
                field(ui, w, "agents-base", "Base", &mut f.record.base, false);
            });
            let rest = ui.available_width();
            ui.allocate_ui_with_layout(vec2(rest, 0.0), Layout::top_down(Align::Min), |ui| {
                ui.set_width(rest);
                field(ui, w, "agents-branch", "New branch", &mut f.record.branch, false);
            });
        });
        field(ui, w, "agents-destination", "Worktree folder", &mut f.destination, false);
    }
    // Tracking: what it gives, in one line, and its switch.
    Frame::new()
        .stroke(Stroke::new(1.0, CARD_STROKE()))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let r = switch(ui, &mut f.record.tracking, tr("Track conversation activity"));
                    pages::control(w, "agents-tracking", &r, clip);
                    ui.add_space(8.0);
                    ui.with_layout(Layout::top_down(Align::Min), |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(RichText::new(tr("Track conversation activity")).font(semibold(13.0)));
                        ui.add(
                            Label::new(
                                RichText::new(tr("Shows when the agent needs you and lets you resume the same conversation."))
                                    .size(12.0)
                                    .color(HINT()),
                            )
                            .wrap(),
                        );
                    });
                });
            });
        });
    // Open by itself whenever there is something in it to fill in or look at.
    // A program that is not where its preset expects it needs its path filled in by
    // hand, so the field for that is in view without being asked for.
    let needed = f.record.provider == Provider::Custom
        || !present(f.record.command.program.trim())
        || f.setup_enabled
        || f.error.is_some();
    let open_id = open_id();
    let open = ui.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
    if needed && !open {
        // Once they show they stay: filling in a missing path must not fold them away.
        ui.data_mut(|d| d.insert_temp(open_id, true));
    }
    let label = tr("Program, arguments and setup command");
    // Opening is one way: once the fields show, their heading is a caption. Automation
    // tells folded from open by whether `agents-advanced` is there.
    if !(needed || open) {
        let r = pages::disclosure(ui, false, label);
        pages::control(w, "agents-advanced", &r, clip);
        if r.clicked() {
            ui.data_mut(|d| d.insert_temp(open_id, true));
        }
        return;
    }
    pages::caption(ui, label);
    // The program's path with Browse beside it, as in the other dialogs.
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 5.0;
        ui.label(RichText::new(tr("Agent executable")).color(HINT()).size(12.0));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            // A translated label can be wider than the button's least width (German's
            // is). The field takes what the button leaves, so the row ends where the
            // other fields do.
            let browse = tr("Browse");
            let button = pages::button_width(ui, browse, BROWSE);
            let r = ui.add(
                TextEdit::singleline(&mut f.record.command.program)
                    .desired_width((ui.available_width() - button - 8.0).max(160.0))
                    .min_size(vec2(0.0, 30.0))
                    .vertical_align(Align::Center)
                    .id_salt("agents-program"),
            );
            pages::control(w, "agents-program", &r, clip);
            let r = ui.add(Button::new(browse).min_size(vec2(button, 30.0)));
            pages::control(w, "agents-program-browse", &r, clip);
            if r.clicked() {
                *action = Some(Action::Browse);
            }
        });
    });
    field(ui, w, "agents-arguments", "Arguments · one per line, no shell expansion", &mut f.record.command.arguments, true);
    let r = ui.add(checkbox(&mut f.setup_enabled, tr("Run a setup command first")));
    pages::control(w, "agents-setup-enabled", &r, clip);
    if f.setup_enabled {
        field(ui, w, "agents-setup-program", "Setup executable", &mut f.setup.program, false);
        field(ui, w, "agents-setup-arguments", "Setup arguments", &mut f.setup.arguments, true);
    }
}
