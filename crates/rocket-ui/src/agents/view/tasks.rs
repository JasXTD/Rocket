//! Manage tasks: the tasks saved from sessions' first messages. Each has a name, the kind
//! of start it is for and its words, where {number}, {title} and {repository} stand for
//! the item a session starts from. Edits apply as typed; × or Escape closes.
use super::*;
use crate::agents::brief::{MAX_WORDS, Saved};
use crate::agents::briefing::Choice;
use crate::agents::model::Kind;

/// `chosen` is the open session dialog's task: a saved task is chosen by its name, so it
/// follows the task's new name as it is typed and tidied.
pub(super) fn manage(
    ctx: &eframe::egui::Context,
    w: &mut Widgets,
    saved: &mut Vec<Saved>,
    managing: &mut Option<Option<usize>>,
    mut chosen: Option<&mut Choice>,
) {
    let Some(open) = managing else { return };
    if open.is_some_and(|i| i >= saved.len()) {
        *open = None;
    }
    let mut selected = *open;
    let mut remove = None;
    let mut renamed = None;
    let head = pages::ReviewHeader {
        image: Icon::Pencil,
        title: tr("Manage tasks"),
        subtitle: tr("Tasks saved from first messages"),
        danger: false,
    };
    let response = pages::review_at(ctx, "agents-tasks-dialog", 560.0, head, w, "agents-tasks-scroll", true, |ui, part, w| {
        if part == pages::Part::Footer {
            return;
        }
        let clip = ui.clip_rect();
        ui.spacing_mut().item_spacing.y = 10.0;
        if saved.is_empty() {
            ui.add(
                Label::new(RichText::new(tr("No saved tasks yet. Save one from a session's Task menu.")).color(HINT()))
                    .wrap(),
            );
            return;
        }
        Frame::new()
            .fill(SUNKEN())
            .stroke(Stroke::new(1.0, BORDER()))
            .corner_radius(8)
            .inner_margin(Margin::same(4))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 1.0;
                for (i, s) in saved.iter().enumerate() {
                    let kind = kind_label(s.kind);
                    let r = ui.add(
                        Button::selectable(selected == Some(i), format!("{}  ·  {}", s.name, kind))
                            .min_size(vec2(ui.available_width(), 28.0)),
                    );
                    pages::control(w, format!("agents-tasks-{i}"), &r, clip);
                    if r.clicked() {
                        selected = Some(i);
                    }
                }
            });
        let Some(i) = selected else { return };
        let taken = {
            let s = &saved[i];
            saved.iter().enumerate().any(|(j, t)| j != i && t.kind == s.kind && t.name.trim() == s.name.trim())
        };
        let s = &mut saved[i];
        let before = s.name.clone();
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 5.0;
            ui.label(RichText::new(tr("Name")).color(HINT()).size(12.0));
            let r = ui.add(
                TextEdit::singleline(&mut s.name)
                    .min_size(vec2(0.0, 30.0))
                    .align(Align2::LEFT_CENTER)
                    .desired_width(f32::INFINITY)
                    .char_limit(80)
                    .id_salt("agents-tasks-name"),
            );
            pages::control(w, "agents-tasks-name", &r, clip);
        });
        if s.name != before {
            renamed = Some((before, s.name.clone()));
        }
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 5.0;
            ui.label(RichText::new(tr("For")).color(HINT()).size(12.0));
            ui.horizontal(|ui| {
                for kind in Kind::ALL {
                    let r = ui.add(Button::selectable(s.kind == kind, kind_label(kind)));
                    pages::control(w, format!("agents-tasks-kind-{}", kind.key()), &r, clip);
                    if r.clicked() {
                        s.kind = kind;
                    }
                }
            });
        });
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 5.0;
            ui.label(RichText::new(tr("Words")).color(HINT()).size(12.0));
            let r = ui.add(
                TextEdit::multiline(&mut s.words)
                    .font(TextStyle::Monospace)
                    .desired_rows(6)
                    .desired_width(f32::INFINITY)
                    .char_limit(MAX_WORDS)
                    .id_salt("agents-tasks-words"),
            );
            pages::control(w, "agents-tasks-words", &r, clip);
            ui.add(
                Label::new(
                    RichText::new(tr("{number}, {title} and {repository} stand for the issue, pull request or run."))
                        .size(12.0)
                        .color(HINT()),
                )
                .wrap(),
            );
        });
        if !s.valid() {
            parts::strip(ui, RED(), Icon::Warning, tr("Give the task a name and some words, without hidden characters."), None);
        } else if taken {
            parts::strip(ui, AMBER(), Icon::Warning, tr("Another task has this name. Closing numbers the later one."), None);
        }
        let r = ui.add(Button::new(RichText::new(tr("Remove task")).color(RED())).frame(false));
        pages::control(w, "agents-tasks-remove", &r, clip);
        if r.clicked() {
            remove = Some(i);
        }
    });
    if let Some(i) = remove {
        saved.remove(i);
        selected = None;
    }
    if let (Some((before, after)), Some(Choice::Saved(name))) = (renamed, chosen.as_deref_mut())
        && *name == before
    {
        *name = after;
    }
    *open = selected;
    if response.close_clicked() || ctx.input(|i| i.key_pressed(Key::Escape)) {
        let choice = chosen.and_then(|c| match c {
            Choice::Saved(name) => Some(name),
            Choice::Task(_) => None,
        });
        close(saved, choice);
        *managing = None;
    }
}

/// What closing keeps: each task tidied (one left without a name or words is not kept),
/// and a name another task of the same kind already has numbered, as saving numbers it.
/// The chosen task's name follows.
fn close(saved: &mut Vec<Saved>, mut chosen: Option<&mut String>) {
    let mut kept: Vec<Saved> = Vec::with_capacity(saved.len());
    for s in saved.drain(..) {
        let Some(mut t) = s.tidied() else { continue };
        let base = t.name.clone();
        let mut n = 2;
        while kept.iter().any(|k| k.kind == t.kind && k.name == t.name) {
            t.name = trf!("{task} ({n})", task = &base, n);
            n += 1;
        }
        if let Some(name) = chosen.as_deref_mut()
            && *name == s.name
        {
            name.clone_from(&t.name);
            chosen = None;
        }
        kept.push(t);
    }
    *saved = kept;
}

fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Issue => tr("Issue"),
        Kind::Pull => tr("Pull request"),
        Kind::Run => tr("CI run"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(name: &str, kind: Kind) -> Saved {
        Saved { name: name.into(), kind, words: format!("{name} words") }
    }

    #[test]
    fn closing_tidies_numbers_clashing_names_and_the_chosen_name_follows() {
        let mut saved = vec![
            task("Fix it ", Kind::Issue),
            task("", Kind::Issue),
            task("Fix\u{200b} it", Kind::Issue),
            task("Fix it", Kind::Pull),
        ];
        let mut chosen = "Fix\u{200b} it".to_owned();
        close(&mut saved, Some(&mut chosen));
        let names: Vec<_> = saved.iter().map(|s| (s.name.as_str(), s.kind)).collect();
        // The empty one is gone; the same name for another kind of start doesn't clash.
        assert_eq!(names, [("Fix it", Kind::Issue), ("Fix it (2)", Kind::Issue), ("Fix it", Kind::Pull)]);
        assert_eq!(chosen, "Fix it (2)");
        // A chosen name that no task had stays as it was.
        let mut other = "Gone".to_owned();
        close(&mut saved, Some(&mut other));
        assert_eq!((saved.len(), other.as_str()), (3, "Gone"));
    }
}
