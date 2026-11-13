//! A commit field: type an ID, branch or tag, or pick from the loaded history. A known
//! commit shows its subject beside the ID.
use crate::{app::UiControl, pages, style::*};
use eframe::egui::*;
use rocket_git::ObjectId;
use std::collections::HashMap;

/// Most rows the list shows at once; the filter narrows longer histories.
const SHOWN: usize = 100;

/// A commit to choose: its ID, subject and "author · age".
pub(crate) struct Choice {
    pub id: ObjectId,
    pub subject: String,
    pub meta: String,
}

/// The commit `value` names, as an ID prefix or `HEAD`.
fn known<'a>(value: &str, choices: &'a [Choice], head: Option<&ObjectId>) -> Option<&'a Choice> {
    let value = value.trim();
    if value == "HEAD" {
        return choices.iter().find(|c| Some(&c.id) == head);
    }
    (value.len() >= 7).then(|| choices.iter().find(|c| c.id.as_str().starts_with(value))).flatten()
}

/// The choices whose ID starts with `filter` or whose subject contains it, ignoring case.
fn matching<'a>(choices: &'a [Choice], filter: &str) -> Vec<&'a Choice> {
    let filter = filter.trim().to_lowercase();
    choices
        .iter()
        .filter(|c| c.id.as_str().starts_with(&filter) || c.subject.to_lowercase().contains(&filter))
        .collect()
}

/// The field and its list. `{key}` is the field, `{key}-choice` the list button,
/// `{key}-choice-filter` its filter and `{key}-choice-{i}` each row. Returns whether
/// the value changed.
#[allow(clippy::too_many_arguments)]
pub(crate) fn field(
    ui: &mut Ui,
    value: &mut String,
    key: &str,
    choices: &[Choice],
    head: Option<&ObjectId>,
    width: f32,
    enabled: bool,
    widgets: &mut HashMap<String, UiControl>,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let r = ui.add_enabled(
            enabled,
            TextEdit::singleline(value)
                .font(TextStyle::Monospace)
                .hint_text("Commit, branch or tag")
                .desired_width((width - 42.0).max(80.0))
                .min_size(vec2(0.0, 30.0))
                .align(Align2::LEFT_CENTER),
        );
        pages::control(widgets, key, &r, ui.clip_rect());
        changed |= r.changed();
        // A long ID would leave no room, and a typed name has no subject to show.
        if let Some(choice) = known(value, choices, head)
            && value.trim().len() <= 12
        {
            let painter = ui.painter();
            let typed = painter.layout_no_wrap(value.clone(), TextStyle::Monospace.resolve(ui.style()), TEXT());
            let left = r.rect.left() + 12.0 + typed.size().x;
            let room = r.rect.right() - 8.0 - left;
            if room > 40.0 {
                let mut job = text::LayoutJob::simple_singleline(choice.subject.clone(), FontId::proportional(13.0), MUTED());
                job.wrap = text::TextWrapping::truncate_at_width(room);
                let subject = painter.layout_job(job);
                painter.galley(pos2(left, r.rect.center().y - subject.size().y / 2.0), subject, MUTED());
            }
        }
        if let Some(id) = list(ui, &format!("{key}-choice"), choices, enabled, widgets) {
            *value = id.as_str()[..12].to_owned();
            changed = true;
        }
    });
    changed
}

fn list(
    ui: &mut Ui,
    key: &str,
    choices: &[Choice],
    enabled: bool,
    widgets: &mut HashMap<String, UiControl>,
) -> Option<ObjectId> {
    let id = ui.make_persistent_id(key);
    let filter_id = id.with("filter");
    let dialog_layer = ui.layer_id();
    let mut chosen = None;
    let response = ui
        .add_enabled_ui(enabled, |ui| {
            ComboBox::from_id_salt(id)
                .selected_text("")
                .width(28.0)
                .height(360.0)
                // Clicking into the filter field must not close the list.
                .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
                .show_ui(ui, |ui| {
                    // Modal and popup share a layer order; keep the list above the dialog.
                    ui.ctx().set_sublayer(dialog_layer, ui.layer_id());
                    ui.set_min_width(440.0);
                    let saved = ui.data(|d| d.get_temp::<String>(filter_id));
                    let mut filter = saved.clone().unwrap_or_default();
                    let r = ui.add(
                        TextEdit::singleline(&mut filter)
                            .hint_text("Find a commit")
                            .desired_width(f32::INFINITY),
                    );
                    if saved.is_none() {
                        r.request_focus();
                    }
                    pages::control(widgets, format!("{key}-filter"), &r, ui.clip_rect());
                    let rows = matching(choices, &filter);
                    ScrollArea::vertical()
                        .id_salt(filter_id.with("rows"))
                        .max_height(280.0)
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            for (i, choice) in rows.iter().take(SHOWN).enumerate() {
                                let r = row(ui, choice, false, true);
                                pages::control(widgets, format!("{key}-{i}"), &r, ui.clip_rect());
                                if r.clicked() {
                                    chosen = Some(choice.id.clone());
                                }
                            }
                        });
                    if rows.is_empty() {
                        ui.label(RichText::new("Nothing matches. You can still type a commit ID, branch or tag.").size(12.0).color(HINT()));
                    } else {
                        ui.label(RichText::new("Or type a commit ID, branch or tag").size(12.0).color(HINT()));
                    }
                    ui.data_mut(|d| d.insert_temp(filter_id, filter));
                    if chosen.is_some() {
                        ui.close();
                    }
                })
        })
        .inner;
    pages::control(widgets, key, &response.response, ui.clip_rect());
    // A closed list starts with an empty, focused filter next time.
    if response.inner.is_none() || chosen.is_some() {
        ui.data_mut(|d| d.remove::<String>(filter_id));
    }
    chosen
}

/// A commit in a list: short ID, subject, then author and age.
pub(crate) fn row(ui: &mut Ui, choice: &Choice, on: bool, enabled: bool) -> Response {
    let response = ui.add_enabled(
        enabled,
        Button::new("").frame(false).min_size(vec2(ui.available_width(), 26.0)),
    );
    let rect = response.rect;
    let painter = ui.painter();
    if on {
        painter.rect_filled(rect, 6.0, SELECTED());
    } else if enabled && response.hovered() {
        painter.rect_filled(rect, 6.0, HOVER());
    }
    let meta = painter.layout_no_wrap(choice.meta.clone(), FontId::proportional(11.5), FAINT());
    let right = rect.right() - 8.0 - meta.size().x;
    painter.galley(pos2(right, rect.center().y - meta.size().y / 2.0), meta, FAINT());
    let mut job = text::LayoutJob::default();
    job.append(&choice.id.as_str()[..7], 0.0, TextFormat::simple(FontId::monospace(12.0), MUTED()));
    job.append(&format!("  {}", choice.subject), 0.0, TextFormat::simple(FontId::proportional(13.0), TEXT()));
    job.wrap = text::TextWrapping::truncate_at_width((right - rect.left() - 20.0).max(20.0));
    let galley = painter.layout_job(job);
    painter.galley(pos2(rect.left() + 8.0, rect.center().y - galley.size().y / 2.0), galley, TEXT());
    response.widget_info(|| WidgetInfo::selected(Role::RadioButton, enabled, on, &choice.subject));
    response
}

#[cfg(test)]
mod tests {
    use super::{Choice, known, matching};
    use rocket_git::ObjectId;

    fn choices() -> Vec<Choice> {
        ["1", "2"]
            .map(|n| Choice {
                id: ObjectId::parse(&n.repeat(40)).unwrap(),
                subject: format!("Commit {n} 文"),
                meta: String::new(),
            })
            .into()
    }

    #[test]
    fn a_field_knows_ids_and_head_but_not_short_or_typed_names() {
        let list = choices();
        let head = list[1].id.clone();
        assert_eq!(known("2222222", &list, None).unwrap().id, head);
        assert_eq!(known(" HEAD ", &list, Some(&head)).unwrap().id, head);
        assert!(known("222", &list, None).is_none(), "too short to be sure");
        assert!(known("main", &list, Some(&head)).is_none());
        assert!(known("HEAD", &list, None).is_none());
    }

    #[test]
    fn the_filter_finds_a_subject_or_the_start_of_an_id() {
        let list = choices();
        assert_eq!(matching(&list, "").len(), 2);
        assert_eq!(matching(&list, " COMMIT 2 ").len(), 1);
        assert_eq!(matching(&list, "文").len(), 2);
        assert_eq!(matching(&list, "1111").len(), 1);
        assert!(matching(&list, "nothing").is_empty());
    }
}
