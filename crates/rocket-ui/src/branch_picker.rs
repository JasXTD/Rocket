//! A choice list with a filter field, for dialogs that pick a branch. Typing narrows
//! the list by case-insensitive substring; long lists show a bounded number of rows.
use crate::{app::UiControl, pages};
use eframe::egui::*;
use std::collections::HashMap;

/// Most rows a picker lists at once; the filter narrows longer lists.
const SHOWN: usize = 200;

/// The choices that contain `filter`, ignoring case, in their original order.
fn matching<'a>(choices: &'a [String], filter: &str) -> Vec<&'a String> {
    let filter = filter.trim().to_lowercase();
    choices
        .iter()
        .filter(|c| c.to_lowercase().contains(&filter))
        .collect()
}

/// A button showing `placeholder` whose popup lists `choices` under a filter field
/// that has focus when the popup opens. Returns the clicked choice. MCP controls:
/// `{key}` (the button), `{key}-filter` and `{key}-{i}` for the i-th listed row.
pub(crate) fn pick(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    key: &str,
    placeholder: &str,
    choices: &[String],
    enabled: bool,
) -> Option<String> {
    let id = ui.make_persistent_id(key);
    let filter_id = id.with("filter");
    let dialog_layer = ui.layer_id();
    let mut chosen = None;
    let response = ui
        .add_enabled_ui(enabled, |ui| {
            ComboBox::from_id_salt(id)
                .selected_text(placeholder)
                // Clicking into the filter field must not close the list.
                .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
                .show_ui(ui, |ui| {
                    // Modal and popup share a layer order; keep the list above the dialog.
                    ui.ctx().set_sublayer(dialog_layer, ui.layer_id());
                    let saved = ui.data(|d| d.get_temp::<String>(filter_id));
                    let mut filter = saved.clone().unwrap_or_default();
                    let r = ui.add(
                        TextEdit::singleline(&mut filter)
                            .hint_text("Filter")
                            .desired_width(260.0),
                    );
                    if saved.is_none() {
                        r.request_focus();
                    }
                    pages::control(widgets, format!("{key}-filter"), &r, ui.clip_rect());
                    let rows = matching(choices, &filter);
                    ScrollArea::vertical()
                        .id_salt(filter_id.with("rows"))
                        .max_height(320.0)
                        .show(ui, |ui| {
                            for (i, choice) in rows.iter().take(SHOWN).enumerate() {
                                let r = ui.selectable_label(false, choice.as_str());
                                pages::control(widgets, format!("{key}-{i}"), &r, ui.clip_rect());
                                if r.clicked() {
                                    chosen = Some((*choice).clone());
                                }
                            }
                        });
                    if rows.is_empty() {
                        ui.label(RichText::new("Nothing matches.").weak());
                    } else if rows.len() > SHOWN {
                        ui.label(
                            RichText::new(format!(
                                "{} more; type to narrow the list.",
                                rows.len() - SHOWN
                            ))
                            .weak(),
                        );
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

#[cfg(test)]
mod tests {
    use super::matching;

    #[test]
    fn the_filter_matches_any_part_ignoring_case_and_keeps_the_order() {
        let choices: Vec<String> = ["origin/main", "origin/Feature/Search", "fork/feature/検索"]
            .map(String::from)
            .into();
        assert_eq!(matching(&choices, "").len(), 3);
        assert_eq!(matching(&choices, " FEATURE "), [&choices[1], &choices[2]]);
        assert_eq!(matching(&choices, "検索"), [&choices[2]]);
        assert!(matching(&choices, "release").is_empty());
    }
}
