//! The session dialog: what to start and where, then a review of exactly that. The
//! dialog closes by its × or Escape only; its footer holds the one way forward. A new
//! session can start from an issue, a pull request or a CI run on the repository's host:
//! then the dialog shows what it starts from, the task and the first message made from
//! them (canvas section 48).
use super::*;
use crate::i18n::trn;
use crate::agents::brief::{self, Task};
use crate::agents::briefing::{Briefing, Choice, Material};
use crate::agents::model::Kind;

/// The cards' label column in the review.
const KEY: f32 = 124.0;
/// The Browse button's least width, as in the other dialogs.
const BROWSE: f32 = 76.0;
/// The Task button's width beside the task name.
const TASK: f32 = 210.0;
/// Lines of the first message shown at once; more scroll.
const MESSAGE_ROWS: usize = 10;

/// What the dialog draws with besides the form itself.
pub(super) struct Inputs<'a> {
    pub chosen: Chosen<'a>,
    pub ready: bool,
    /// A Browse picker or Manage tasks is above the dialog, and Escape is theirs.
    pub covered: bool,
    /// Hosting has this repository's account connected.
    pub hosted: bool,
    pub saved: &'a [brief::Saved],
}

pub(super) fn show(ctx: &eframe::egui::Context, w: &mut Widgets, f: &mut Form, inputs: &Inputs, action: &mut Option<Action>) {
    let removing = f.mode == Mode::Remove;
    let title = match f.mode {
        Mode::Remove => "Remove worktree",
        Mode::Resume => "Resuming conversation",
        Mode::Restart => "Review and restart",
        Mode::Create => "New agent session",
    };
    let from = f.briefing.as_ref().filter(|_| f.plan.is_none()).map(|b| from_line(&b.origin));
    let subtitle = match (f.mode, f.plan.is_some()) {
        (Mode::Remove, _) => tr("Review session"),
        (Mode::Create, false) => from.as_deref().unwrap_or(tr("Its own branch and folder for one task")),
        (Mode::Create, true) => tr("Check what will be created and run"),
        (_, false) => tr("A new conversation in the same worktree"),
        (_, true) => tr("Check what will run"),
    };
    let head = pages::ReviewHeader {
        image: Icon::Worktree,
        title: tr(title),
        subtitle,
        danger: removing,
    };
    let response = pages::review_at(ctx, "agents-review-dialog", 680.0, head, w, "agents-scroll", true, |ui, part, w| {
        if part == pages::Part::Footer {
            footer(ui, w, f, inputs.ready, action);
            return;
        }
        ui.spacing_mut().item_spacing.y = 12.0;
        if let Some(e) = &f.error {
            parts::strip(ui, RED(), Icon::Warning, e, None);
        }
        if let Some(plan) = &f.plan {
            review(ui, plan, f.mode, f.briefing.as_ref());
        } else {
            ui.add_enabled_ui(f.task.is_none(), |ui| edit(ui, w, f, inputs, action));
        }
    });
    if let Some(close) = w.get("dialog-close").cloned() {
        w.insert("agents-cancel".into(), close);
    }
    // Resizing the dialog can move its boundary under the pointer. Only X/Escape
    // dismiss, never a click that became outside after a compact-window reflow. Escape
    // with the Task menu open closes the menu only.
    if response.close_clicked() || (!inputs.covered && ctx.input(|i| i.key_pressed(Key::Escape)) && response.should_close()) {
        *action = Some(Action::Cancel);
    }
}

/// "From issue #482 on team/shop"
fn from_line(o: &crate::agents::model::Origin) -> String {
    let repository = &o.repository;
    match o.kind {
        Kind::Issue => trf!("From issue #{number} on {repository}", number = o.number, repository),
        Kind::Pull => trf!("From pull request #{number} on {repository}", number = o.number, repository),
        Kind::Run => trf!("From {run} on {repository}", run = &o.title, repository),
    }
}

/// Whether a program's path names a file that is there. A preset can name where its
/// program is installed by default without that file existing (Claude Code's does).
pub(super) fn present(program: &str) -> bool {
    !program.is_empty() && std::path::Path::new(program).is_file()
}

/// Whether the dialog on screen shows its program, arguments and setup fields.
fn open_id() -> Id {
    Id::unique("agents-program-fields-open")
}

/// No dialog is open: the fields the last one opened are not carried into the next.
pub(super) fn forget(ctx: &eframe::egui::Context) {
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
    // A session from an issue or pull request needs its message first, without secrets.
    let briefed = f.briefing.as_ref().is_none_or(Briefing::ready);
    let r = pages::primary(ui, ready && !checking && briefed, tr("Review session"));
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
fn review(ui: &mut Ui, plan: &Plan, mode: Mode, briefing: Option<&Briefing>) {
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
    if let Some(b) = briefing.filter(|b| mode == Mode::Create && !b.message.trim().is_empty()) {
        pages::titled_card(ui, |ui| card_title(ui, tr("First message")), |ui| {
            ui.add(
                Label::new(
                    RichText::new(tr("Put in the session's input once it starts. Not sent, and not saved with the session."))
                        .size(12.5)
                        .color(HINT()),
                )
                .wrap(),
            );
            let lines: Vec<&str> = b.message.lines().collect();
            pages::well(ui, |ui| {
                for line in lines.iter().take(3) {
                    ui.add(Label::new(RichText::new(*line).monospace().size(12.0)).truncate());
                }
            });
            let mut about = trn!(lines.len(), "{n} line", "{n} lines");
            if !b.read_at.is_empty() && !b.without {
                about.push_str(" · ");
                about.push_str(&trf!("from {item} as read at {time}", item = b.origin.short(), time = &b.read_at));
            }
            ui.label(RichText::new(about).size(12.0).color(HINT()));
        });
    }
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
                .align(Align2::LEFT_CENTER)
        };
        let r = ui.add(edit.desired_width(f32::INFINITY).id_salt(id));
        pages::control(w, id, &r, ui.clip_rect());
    });
}

fn kind_icon(kind: Kind) -> Icon {
    match kind {
        Kind::Issue => Icon::Info,
        Kind::Pull => Icon::Merge,
        Kind::Run => Icon::Actions,
    }
}

/// What the session starts from: its title and number, where it is and what was read,
/// with the link to it and × to start without it.
fn source_card(ui: &mut Ui, w: &mut Widgets, b: &Briefing, action: &mut Option<Action>) {
    let clip = ui.clip_rect();
    let o = &b.origin;
    Frame::new()
        .fill(SUNKEN())
        .stroke(Stroke::new(1.0, BORDER()))
        .corner_radius(8)
        .inner_margin(Margin { left: 10, right: 6, top: 8, bottom: 8 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let (mark, _) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
                ui.painter().circle_filled(mark.center(), 13.0, GREEN().gamma_multiply(0.14));
                icon(ui.painter(), mark.shrink(6.5), kind_icon(o.kind), GREEN());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let r = crate::chrome::icon_button(ui, Icon::Close, true, false, 28.0).on_hover_text(match o.kind {
                        Kind::Issue => tr("Start without this issue"),
                        Kind::Pull => tr("Start without this pull request"),
                        Kind::Run => tr("Start without this run"),
                    });
                    pages::control(w, "agents-source-remove", &r, clip);
                    if r.clicked() {
                        *action = Some(Action::Unbrief);
                    }
                    let r = crate::chrome::icon_button(ui, Icon::External, true, false, 28.0).on_hover_text(tr("Open in browser"));
                    pages::control(w, "agents-source-open", &r, clip);
                    if r.clicked() {
                        *action = Some(Action::OpenUrl(o.url.clone()));
                    }
                    ui.with_layout(Layout::top_down(Align::Min), |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        let width = ui.available_width();
                        let mut title = text::LayoutJob::default();
                        title.append(&o.title, 0.0, TextFormat::simple(semibold(13.0), TEXT()));
                        if o.kind != Kind::Run {
                            title.append(&format!(" #{}", o.number), 0.0, TextFormat::simple(FontId::proportional(13.0), HINT()));
                        }
                        title.wrap = text::TextWrapping::truncate_at_width(width);
                        ui.label(title);
                        ui.add(Label::new(RichText::new(source_facts(b)).size(12.0).color(HINT())).truncate());
                    });
                });
            });
        });
}

/// "Issue on team/shop · dana · Open · 1 comment · read 21:04"
fn source_facts(b: &Briefing) -> String {
    let o = &b.origin;
    let repository = &o.repository;
    let mut facts = vec![match o.kind {
        Kind::Issue => trf!("Issue on {repository}", repository),
        Kind::Pull => trf!("Pull request on {repository}", repository),
        Kind::Run => trf!("Run on {repository}", repository),
    }];
    match b.material.as_deref() {
        Some(Material::Issue(i)) => {
            facts.push(i.author.clone());
            facts.push(state_word(&i.state));
            facts.push(trn!(i.comments.len(), "{n} comment", "{n} comments"));
        }
        Some(Material::Pull { details, .. }) => {
            let p = details.summary();
            facts.push(p.author.clone());
            facts.push(format!("{} → {}", p.source, p.target));
        }
        Some(Material::Run(r)) => {
            if let Some(branch) = &r.run.branch {
                facts.push(branch.clone());
            }
        }
        None => {}
    }
    if !b.read_at.is_empty() {
        facts.push(trf!("read {time}", time = &b.read_at));
    }
    facts.join(" · ")
}

fn state_word(state: &str) -> String {
    match state.to_ascii_lowercase().as_str() {
        "open" | "opened" => tr("Open").into(),
        "closed" => tr("Closed").into(),
        _ => state.to_owned(),
    }
}

/// The field a new session can start from: a number, a link or words from the title,
/// and what was found for them.
fn picker(ui: &mut Ui, w: &mut Widgets, f: &mut Form, action: &mut Option<Action>) {
    let clip = ui.clip_rect();
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 5.0;
        ui.label(RichText::new(tr("Issue or pull request")).color(HINT()).size(12.0));
        let r = ui.add(
            TextEdit::singleline(&mut f.picker.text)
                .hint_text(tr("Optional: a number, a link or words from the title"))
                .min_size(vec2(0.0, 30.0))
                .align(Align2::LEFT_CENTER)
                .desired_width(f32::INFINITY)
                .id_salt("agents-from"),
        );
        pages::control(w, "agents-from", &r, clip);
        if r.changed() {
            f.picker.typed();
        }
        let p = &f.picker;
        if p.busy() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(tr("Searching")).size(12.0).color(HINT()));
            });
        } else if let Some(e) = &p.error {
            ui.add(Label::new(RichText::new(e).size(12.0).color(RED())).wrap());
        } else if !p.searched.is_empty() && p.results.is_empty() && p.text.trim() == p.searched {
            ui.label(RichText::new(tr("Nothing open matches.")).size(12.0).color(HINT()));
        }
        if !p.results.is_empty() {
            Frame::new()
                .fill(PANEL())
                .stroke(Stroke::new(1.0, BORDER()))
                .corner_radius(8)
                .inner_margin(Margin::same(4))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 1.0;
                    for (i, found) in p.results.iter().enumerate() {
                        let kind = match found.kind {
                            Kind::Issue => tr("issue"),
                            Kind::Pull => tr("pull request"),
                            Kind::Run => tr("run"),
                        };
                        let title = if found.title.is_empty() { format!("#{}", found.number) } else { found.title.clone() };
                        let r = result_row(ui, kind_icon(found.kind), &title, &format!("#{} · {kind}", found.number));
                        pages::control(w, format!("agents-from-{i}"), &r, clip);
                        if r.clicked() {
                            *action = Some(Action::Pick(i));
                        }
                    }
                });
        }
    });
}

/// A row of the field's results: the kind's mark, the title, and number and kind at the right.
fn result_row(ui: &mut Ui, image: Icon, title: &str, right: &str) -> Response {
    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    if r.hovered() {
        ui.painter().rect_filled(rect, 5.0, LIST_HOT());
    }
    let mark = Rect::from_center_size(pos2(rect.left() + 17.0, rect.center().y), vec2(14.0, 14.0));
    icon(ui.painter(), mark, image, GREEN());
    let right_galley = ui.painter().layout_no_wrap(right.to_owned(), FontId::proportional(12.0), HINT());
    let right_x = rect.right() - 10.0 - right_galley.size().x;
    ui.painter().galley(pos2(right_x, rect.center().y - right_galley.size().y / 2.0), right_galley, HINT());
    let mut job = text::LayoutJob::single_section(title.to_owned(), TextFormat::simple(FontId::proportional(13.0), TEXT()));
    job.wrap = text::TextWrapping::truncate_at_width((right_x - rect.left() - 40.0).max(0.0));
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(pos2(rect.left() + 32.0, rect.center().y - galley.size().y / 2.0), galley, TEXT());
    r.on_hover_cursor(CursorIcon::PointingHand)
}

/// The Task button and its menu: the tasks for this kind of start, the saved ones, and
/// saving and managing tasks.
fn task_menu(ui: &mut Ui, w: &mut Widgets, f: &mut Form, saved: &[brief::Saved], action: &mut Option<Action>) {
    let clip = ui.clip_rect();
    let Some(b) = &f.briefing else { return };
    let kind = b.origin.kind;
    let current = match &b.choice {
        Choice::Task(t) => tr(t.label()).to_owned(),
        Choice::Saved(name) => saved
            .iter()
            .find(|s| &s.name == name && s.kind == kind)
            .map_or_else(|| tr(Task::Own.label()).to_owned(), |s| s.name.clone()),
    };
    let mut chosen = None;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 5.0;
        ui.label(RichText::new(tr("Task")).color(HINT()).size(12.0));
        let button = ui.add(
            Button::new(RichText::new(&current).color(TEXT()))
                .fill(SUNKEN())
                .stroke(Stroke::new(1.0, BORDER()))
                .min_size(vec2(TASK, 30.0)),
        );
        pages::control(w, "agents-task", &button, clip);
        icon(
            ui.painter(),
            Rect::from_center_size(pos2(button.rect.right() - 14.0, button.rect.center().y), vec2(10.0, 10.0)),
            Icon::Chevron,
            MUTED(),
        );
        Popup::menu(&button).width(360.0).show(|ui| {
            ui.label(
                RichText::new(match kind {
                    Kind::Issue => tr("For an issue"),
                    Kind::Pull => tr("For a pull request"),
                    Kind::Run => tr("For a run"),
                })
                .size(11.0)
                .color(HINT())
                .strong(),
            );
            for t in Task::offered(kind) {
                let on = b.choice == Choice::Task(*t);
                let r = ui.add(Button::selectable(on, tr(t.label())).min_size(vec2(ui.available_width(), 28.0)));
                if *t == Task::Plan {
                    let hint = ui.painter().layout_no_wrap(tr("Claude Code changes nothing").to_owned(), FontId::proportional(12.0), HINT());
                    ui.painter().galley(pos2(r.rect.right() - 10.0 - hint.size().x, r.rect.center().y - hint.size().y / 2.0), hint, HINT());
                }
                pages::control(w, format!("agents-task-{}", t.key()), &r, ui.clip_rect());
                if r.clicked() {
                    chosen = Some(Choice::Task(*t));
                }
            }
            for (i, s) in saved.iter().enumerate().filter(|(_, s)| s.kind == kind) {
                let on = matches!(&b.choice, Choice::Saved(name) if *name == s.name);
                let r = ui.add(Button::selectable(on, &s.name).min_size(vec2(ui.available_width(), 28.0)));
                pages::control(w, format!("agents-task-saved-{i}"), &r, ui.clip_rect());
                if r.clicked() {
                    chosen = Some(Choice::Saved(s.name.clone()));
                }
            }
            ui.separator();
            let r = ui.add(Button::new(tr("Save the message as a task")).frame(false).min_size(vec2(ui.available_width(), 28.0)));
            pages::control(w, "agents-task-save", &r, ui.clip_rect());
            if r.clicked() {
                *action = Some(Action::SaveTask(trf!("{task} ({n})", task = &current, n = saved.len() + 1)));
            }
            let r = ui.add(Button::new(tr("Manage tasks")).frame(false).min_size(vec2(ui.available_width(), 28.0)));
            pages::control(w, "agents-task-manage", &r, ui.clip_rect());
            if r.clicked() {
                *action = Some(Action::ManageTasks);
            }
        });
    });
    if let Some(choice) = chosen {
        f.choose(choice, saved);
    }
}

/// What a pull request's message holds: ticked parts go in, with what each one is.
fn include(ui: &mut Ui, w: &mut Widgets, b: &mut Briefing, saved: &[brief::Saved]) {
    let clip = ui.clip_rect();
    let Some(Material::Pull { details, threads, failed, .. }) = b.material.as_deref() else { return };
    let (open, failing, discussion, resolved) = b.counts();
    let mut subs_comments: Vec<String> = Vec::new();
    match threads {
        Some(t) => {
            for thread in t.threads.iter().filter(|t| !t.resolved) {
                if let Some(c) = thread.comments.first() {
                    let place = thread.line.map_or_else(|| thread.path.clone(), |l| format!("{}:{l}", thread.path));
                    subs_comments.push(format!("{place} · {} · {}", c.author, first_line(&c.body)));
                }
            }
        }
        None => {
            for c in &details.inline_comments.items {
                let place = match (&c.path, c.line) {
                    (Some(p), Some(l)) => format!("{p}:{l}"),
                    (Some(p), None) => p.clone(),
                    _ => String::new(),
                };
                subs_comments.push(format!("{place} · {} · {}", c.author, first_line(&c.body)));
            }
        }
    }
    let subs_checks: Vec<String> = failed
        .iter()
        .map(|k| {
            let what = if k.group.is_empty() { k.name.clone() } else { format!("{} · {}", k.group, k.name) };
            if k.detail.is_empty() { what } else { format!("{what} · {}", k.detail) }
        })
        .collect();
    let mut changed = false;
    pages::titled_card(ui, |ui| card_title(ui, tr("Put in the message")), |ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        let mut row = |ui: &mut Ui, on: &mut bool, label: &str, count: Option<usize>, key: &str, subs: &[String], enabled: bool| {
            ui.add_enabled_ui(enabled, |ui| {
                ui.horizontal(|ui| {
                    let r = ui.add(checkbox(on, label));
                    pages::control(w, key, &r, clip);
                    changed |= r.changed();
                    if let Some(n) = count {
                        ui.label(RichText::new(n.to_string()).size(12.0).color(HINT()));
                    }
                });
                if *on {
                    for s in subs.iter().take(3) {
                        ui.horizontal(|ui| {
                            ui.add_space(26.0);
                            ui.add(Label::new(RichText::new(s).size(12.0).color(HINT())).truncate());
                        });
                    }
                    if subs.len() > 3 {
                        ui.horizontal(|ui| {
                            ui.add_space(26.0);
                            ui.label(RichText::new(trf!("and {n} more", n = subs.len() - 3)).size(12.0).color(HINT()));
                        });
                    }
                }
            });
        };
        row(ui, &mut b.include.comments, tr("Open review comments"), Some(open), "agents-include-comments", &subs_comments, open > 0);
        row(ui, &mut b.include.checks, tr("Failed checks"), Some(failing), "agents-include-checks", &subs_checks, failing > 0);
        let described = !details.summary().body.trim().is_empty();
        row(ui, &mut b.include.description, tr("Description"), None, "agents-include-description", &[], described);
        row(ui, &mut b.include.discussion, tr("Discussion"), Some(discussion), "agents-include-discussion", &[], discussion > 0);
        row(ui, &mut b.include.resolved, tr("Resolved review comments"), Some(resolved), "agents-include-resolved", &[], resolved > 0);
    });
    if changed {
        b.rebuild(saved);
    }
}

fn first_line(text: &str) -> String {
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or_default().trim();
    crate::agents::brief::clean(line).0
}

/// The first message: reading it, why it couldn't be read, or the text to edit with what
/// was left out, how much was kept and anything that looks like a secret.
fn message(ui: &mut Ui, w: &mut Widgets, b: &mut Briefing, action: &mut Option<Action>) {
    let clip = ui.clip_rect();
    let o = b.origin.clone();
    if b.reading() {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 5.0;
            ui.label(RichText::new(tr("First message")).color(HINT()).size(12.0));
            Frame::new()
                .fill(SUNKEN())
                .stroke(Stroke::new(1.0, BORDER()))
                .corner_radius(6)
                .inner_margin(Margin::symmetric(12, 18))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.vertical_centered(|ui| {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            let what = match o.kind {
                                Kind::Pull => trf!("Reading #{number}, its comments and checks", number = o.number),
                                _ => trf!("Reading #{number} and its comments", number = o.number),
                            };
                            ui.label(RichText::new(what).color(MUTED()));
                        });
                        let r = ui.button(tr("Stop"));
                        pages::control(w, "agents-brief-stop", &r, clip);
                        if r.clicked() {
                            *action = Some(Action::StopBriefRead);
                        }
                    });
                });
        });
        return;
    }
    if let Some(error) = &b.error {
        parts::strip(ui, RED(), Icon::Warning, error, None);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let r = ui.button(tr("Try again"));
            pages::control(w, "agents-brief-retry", &r, clip);
            if r.clicked() {
                *action = Some(Action::ReadAgain);
            }
            // A pull request's session starts on its head, so it needs the read.
            if o.kind == Kind::Issue {
                let r = ui.add(Button::new(RichText::new(tr("Start without its text")).color(MUTED())).frame(false));
                pages::control(w, "agents-brief-without", &r, clip);
                if r.clicked() {
                    *action = Some(Action::GoWithout);
                }
            }
        });
        return;
    }
    if b.material.is_none() && !b.without {
        return;
    }
    if let Some(cut) = b.built.cut {
        let total = cut.total.div_ceil(1024);
        let text = match o.kind {
            Kind::Issue => trf!("#{number} and its comments are {total} KB. The message keeps the issue and the newest comments, up to 16 KB.", number = o.number, total),
            _ => trf!("The quoted text is {total} KB. The message keeps up to 16 KB.", total),
        };
        parts::strip(ui, AMBER(), Icon::Warning, &text, None);
    }
    if let Some((kind, line)) = b.secret {
        parts::strip(ui, RED(), Icon::Warning, &brief::secret_strip(kind, line), None);
    }
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 5.0;
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("First message")).color(HINT()).size(12.0));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let note = if b.built.cut.is_some() {
                    trf!("{kept} KB of {total} KB", kept = b.built.cut.map_or(0, |c| c.kept.div_ceil(1024)), total = b.built.cut.map_or(0, |c| c.total.div_ceil(1024)))
                } else {
                    tr("Goes into the session's input. You send it.").to_owned()
                };
                ui.label(RichText::new(note).size(12.0).color(HINT()));
            });
        });
        let row = ui.text_style_height(&TextStyle::Monospace);
        // The line that looks like a secret is tinted, so it can be found and removed.
        let secret = b.secret.map(|(_, line)| line);
        let font = TextStyle::Monospace.resolve(ui.style());
        let mut layouter = |ui: &Ui, text: &dyn eframe::egui::TextBuffer, wrap_width: f32| {
            let mut job = text::LayoutJob::default();
            for (i, line) in text.as_str().split_inclusive('\n').enumerate() {
                let marked = secret == Some(i + 1);
                let mut format = TextFormat::simple(font.clone(), if marked { RED() } else { TEXT() });
                if marked {
                    format.background = RED().gamma_multiply(0.16);
                }
                job.append(line, 0.0, format);
            }
            job.wrap.max_width = wrap_width;
            ui.fonts_mut(|f| f.layout_job(job))
        };
        let out = ScrollArea::vertical()
            .id_salt("agents-message-scroll")
            .max_height(MESSAGE_ROWS as f32 * row + 12.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                let edit = TextEdit::multiline(&mut b.message)
                    .font(TextStyle::Monospace)
                    .desired_width(f32::INFINITY)
                    .desired_rows(MESSAGE_ROWS)
                    .id_salt("agents-message");
                ui.add(if secret.is_some() { edit.layouter(&mut layouter) } else { edit })
            });
        pages::control(w, "agents-message", &out.inner, clip);
        pages::region(w, "agents-message-scroll", out.inner_rect, ui.is_enabled());
    });
    let left = &b.built.left_out;
    if !left.is_empty() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let (mark, _) = ui.allocate_exact_size(vec2(13.0, 13.0), Sense::hover());
            icon(ui.painter(), mark, Icon::Eye, HINT());
            ui.label(RichText::new(trf!("Left out: {what}", what = left.summary())).size(12.0).color(HINT()));
            let r = ui.add(
                Button::new(RichText::new(if b.show_left_out { tr("Hide") } else { tr("Show") }).size(12.0).color(ACCENT_TEXT()))
                    .frame(false)
                    .min_size(vec2(0.0, 22.0)),
            );
            pages::control(w, "agents-left-out", &r, clip);
            if r.clicked() {
                b.show_left_out = !b.show_left_out;
            }
        });
        if b.show_left_out {
            let left = b.built.left_out.clone();
            pages::well(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                for (whose, comment) in &left.comments {
                    ui.label(RichText::new(trf!("HTML comment in {whose}", whose)).size(12.0).color(HINT()));
                    ui.add(Label::new(RichText::new(comment).monospace().size(12.0).color(RED())).wrap());
                }
                for (whose, count) in &left.hidden {
                    ui.label(RichText::new(trf!("In {whose}", whose)).size(12.0).color(HINT()));
                    ui.label(trn!(*count, "{n} hidden character", "{n} hidden characters"));
                }
            });
        }
    }
}

/// What to start and where. Everything a first session needs is in view; the program's
/// path, its arguments and a setup command are folded away until they are needed.
fn edit(ui: &mut Ui, w: &mut Widgets, f: &mut Form, inputs: &Inputs, action: &mut Option<Action>) {
    ui.spacing_mut().item_spacing.y = 12.0;
    let clip = ui.clip_rect();
    let chosen = inputs.chosen;
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
    if f.mode == Mode::Create {
        // The field needs Hosting's account for this repository; without one the dialog
        // is as it was, and the Guide says where the field comes from.
        match &f.briefing {
            Some(b) => source_card(ui, w, b, action),
            None if inputs.hosted => picker(ui, w, f, action),
            None => {}
        }
    }
    if f.briefing.is_some() {
        // The task name with the Task button beside it.
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let rest = (ui.available_width() - TASK - 10.0).max(160.0);
            ui.allocate_ui_with_layout(vec2(rest, 0.0), Layout::top_down(Align::Min), |ui| {
                ui.set_width(rest);
                field(ui, w, "agents-name", "Task name", &mut f.record.name, false);
            });
            ui.allocate_ui_with_layout(vec2(TASK, 0.0), Layout::top_down(Align::Min), |ui| {
                ui.set_width(TASK);
                task_menu(ui, w, f, inputs.saved, action);
            });
        });
    } else {
        field(ui, w, "agents-name", "Task name", &mut f.record.name, false);
    }
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
        // A pull request's session starts on the head that was read: its base is fixed.
        let pull = f.briefing.as_ref().filter(|b| b.origin.kind == Kind::Pull);
        let fixed = pull.map(|b| match (b.material.as_deref(), &b.origin.push) {
            (Some(Material::Pull { details, .. }), _) => {
                let p = details.summary();
                format!("{} · {}", p.source, &p.head[..7.min(p.head.len())])
            }
            _ => tr("Reading the pull request").to_owned(),
        });
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let base = if fixed.is_some() { (ui.available_width() * 0.42).clamp(160.0, 270.0) } else { (ui.available_width() * 0.34).clamp(140.0, 220.0) };
            ui.allocate_ui_with_layout(vec2(base, 0.0), Layout::top_down(Align::Min), |ui| {
                ui.set_width(base);
                match &fixed {
                    Some(text) => {
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 5.0;
                            ui.label(RichText::new(tr("Base")).color(HINT()).size(12.0));
                            let r = Frame::new()
                                .fill(PANEL())
                                .stroke(Stroke::new(1.0, BORDER()))
                                .corner_radius(6)
                                .inner_margin(Margin::symmetric(9, 0))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.set_height(28.0);
                                    ui.horizontal_centered(|ui| {
                                        ui.add(Label::new(RichText::new(text).monospace().size(12.0).color(TEXT_SOFT())).truncate());
                                    });
                                })
                                .response
                                .on_hover_text(tr("The pull request's head, as the host has it"));
                            pages::control(w, "agents-base", &r, clip);
                        });
                    }
                    None => field(ui, w, "agents-base", "Base", &mut f.record.base, false),
                }
            });
            let rest = ui.available_width();
            ui.allocate_ui_with_layout(vec2(rest, 0.0), Layout::top_down(Align::Min), |ui| {
                ui.set_width(rest);
                field(ui, w, "agents-branch", "New branch", &mut f.record.branch, false);
            });
        });
        field(ui, w, "agents-destination", "Worktree folder", &mut f.destination, false);
        if let Some(b) = &mut f.briefing {
            include(ui, w, b, inputs.saved);
            message(ui, w, b, action);
        }
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
                    .align(Align2::LEFT_CENTER)
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
