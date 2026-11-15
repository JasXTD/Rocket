//! Explicit, validated edits to the cumulative object scan budget, a row of the Secret
//! protection card. Both content guards scan with it.
use crate::{app::Rocket, i18n::{tr, trf}, pages, preferences, style::*};
use eframe::egui::*;

/// The field while the user changes it. None shows the budget in use, so a value loaded
/// or applied elsewhere never leaves stale text behind.
#[derive(Default)]
pub(super) struct Edit {
    pub(super) text: Option<String>,
    pub(super) error: Option<String>,
}

fn parse(text: &str) -> Result<u64, String> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(tr("Enter a positive whole number of MiB.").into());
    }
    let mib = text.parse::<u64>().map_err(|_| tr("That number is too large.").to_owned())?;
    rocket_git::validate_secret_scan_mib(mib).map_err(|error| error.to_string())?;
    Ok(mib)
}

impl Rocket {
    /// Total scan budget: the field, Apply (lit only after an edit, and Enter applies too)
    /// and Reset to 256 MiB, with a line on what uses it or what is in use until Apply.
    pub(super) fn scan_budget_row(&mut self, ui: &mut Ui) {
        // Not while Git work runs, but a settings save waiting out its debounce (as after
        // Apply) doesn't grey the field out under the user's next edit.
        let editable = !self.working() && self.backend.is_some();
        let applied = self.secret_scan_mib;
        let shown = applied.to_string();
        let mut text = self.dependencies.scan_budget.text.clone().unwrap_or_else(|| shown.clone());
        let mut typed = false;
        let mut submit = false;
        let mut reset = false;
        // What the line under the field says, from this frame's start.
        let line: Option<Result<String, String>> = if let Some(error) = &self.dependencies.scan_budget.error {
            Some(Err(error.clone()))
        } else if self.dependencies.scan_budget.text.is_some() {
            Some(Ok(trf!("In use: {mib} MiB until you apply.", mib = applied)))
        } else if !self.secret_protection && self.guard_ai_api_keys {
            Some(Ok(tr("Still used by Guard AI API keys.").into()))
        } else if !self.secret_protection {
            Some(Ok(tr("No scan runs while both guards are off.").into()))
        } else if self.guard_ai_api_keys {
            Some(Ok(tr("Also used by Guard AI API keys.").into()))
        } else {
            None
        };
        let widgets = &mut self.frame_widgets;
        preferences::card_note(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.add(Label::new(RichText::new(tr("Total scan budget")).color(TEXT())).wrap());
            ui.add(Label::new(RichText::new(tr("Git data a commit or push check may read, uncompressed, older history included")).size(12.0).color(HINT())).wrap());
            ui.add_space(6.0);
            let edited = text.trim() != shown;
            ui.add_enabled_ui(editable, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let input = ui.add(TextEdit::singleline(&mut text).id_salt("pref-secret-scan-mib").desired_width(120.0).char_limit(20));
                    pages::control(widgets, "pref-secret-scan-mib", &input, ui.clip_rect());
                    typed = input.changed();
                    submit = input.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    ui.label(RichText::new("MiB").color(MUTED()));
                    let label = tr("Apply");
                    let apply = if edited { pages::primary(ui, true, label) } else { ui.add_enabled(false, Button::new(label)) };
                    pages::control(widgets, "pref-secret-scan-apply", &apply, ui.clip_rect());
                    submit |= apply.clicked();
                    let back = ui.add_enabled(edited || applied != rocket_git::DEFAULT_SECRET_SCAN_MIB, Button::new(tr("Reset to 256 MiB")));
                    pages::control(widgets, "pref-secret-scan-reset", &back, ui.clip_rect());
                    reset = back.clicked();
                });
            });
            match &line {
                Some(Err(error)) => {
                    ui.add_space(6.0);
                    pages::callout(ui, AMBER(), error);
                }
                Some(Ok(text)) => {
                    ui.add_space(4.0);
                    ui.add(Label::new(RichText::new(text).size(12.0).color(HINT())).wrap());
                }
                None => {}
            }
        });
        let edit = &mut self.dependencies.scan_budget;
        if typed {
            edit.error = None;
            edit.text = (text.trim() != shown).then(|| text.clone());
        }
        let apply = if reset {
            Some(Ok(rocket_git::DEFAULT_SECRET_SCAN_MIB))
        } else if submit && text.trim() != shown {
            Some(parse(&text))
        } else {
            None
        };
        match apply {
            Some(Ok(mib)) if editable => {
                if let Some(backend) = &self.backend {
                    match backend.set_secret_scan_mib(mib) {
                        Ok(()) => {
                            self.secret_scan_mib = mib;
                            self.dependencies.scan_budget = Edit::default();
                        }
                        Err(error) => self.dependencies.scan_budget.error = Some(error.to_string()),
                    }
                }
            }
            Some(Err(error)) if editable => self.dependencies.scan_budget.error = Some(error),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_custom_whole_mib_and_rejects_invalid_without_rounding() {
        for value in [1, 256, 257, 4097, rocket_git::MAX_SECRET_SCAN_MIB] {
            assert_eq!(parse(&format!(" {value} ")).unwrap(), value);
        }
        for text in ["", "0", "-1", "+1", "1.5", "1e3", "256 MiB", "２５６", "18446744073709551616"] {
            assert!(parse(text).is_err(), "{text}");
        }
        assert!(parse(&(rocket_git::MAX_SECRET_SCAN_MIB + 1).to_string()).is_err());
    }
}
