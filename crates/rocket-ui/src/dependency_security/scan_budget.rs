//! Explicit, validated edits to the cumulative object scan budget.
use crate::{app::Rocket, pages, preferences, style::*};
use eframe::egui::*;

#[derive(Default)]
pub(super) struct Edit {
    text: Option<String>,
    error: Option<String>,
}

fn parse(text: &str) -> Result<u64, String> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Enter a positive whole number of MiB.".into());
    }
    let mib = text.parse::<u64>().map_err(|_| "That number is too large.".to_owned())?;
    rocket_git::validate_secret_scan_mib(mib).map_err(|error| error.to_string())?;
    Ok(mib)
}

impl Rocket {
    pub(super) fn scan_budget_card(&mut self, ui: &mut Ui) {
        let editable = !self.busy() && self.backend.is_some();
        preferences::card_frame(ui, |ui| {
            preferences::card_header(ui, "Total scan budget", |_| {});
            preferences::card_note(ui, |ui| {
                ui.label(RichText::new("Maximum uncompressed Git data per commit or push scan").color(TEXT()));
                ui.label(RichText::new("Push checks include reachable older history, not just the new commit or compressed upload.").color(MUTED()));
                let mut apply = None;
                ui.add_enabled_ui(editable, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        let text = self.dependencies.scan_budget.text.get_or_insert_with(|| self.secret_scan_mib.to_string());
                        let input = ui.add(TextEdit::singleline(text).id_salt("pref-secret-scan-mib").desired_width(160.0).char_limit(20));
                        pages::control(&mut self.frame_widgets, "pref-secret-scan-mib", &input, ui.clip_rect());
                        if input.changed() { self.dependencies.scan_budget.error = None; }
                        ui.label("MiB");
                        let button = ui.button("Apply");
                        pages::control(&mut self.frame_widgets, "pref-secret-scan-apply", &button, ui.clip_rect());
                        if button.clicked() {
                            match parse(self.dependencies.scan_budget.text.as_deref().unwrap_or("")) {
                                Ok(mib) => apply = Some(mib),
                                Err(error) => self.dependencies.scan_budget.error = Some(error),
                            }
                        }
                        let reset = ui.button("Reset to 256 MiB");
                        pages::control(&mut self.frame_widgets, "pref-secret-scan-reset", &reset, ui.clip_rect());
                        if reset.clicked() { apply = Some(rocket_git::DEFAULT_SECRET_SCAN_MIB); }
                    });
                });
                if let Some(mib) = apply && let Some(backend) = &self.backend {
                    match backend.set_secret_scan_mib(mib) {
                        Ok(()) => {
                            self.secret_scan_mib = mib;
                            self.dependencies.scan_budget.text = Some(mib.to_string());
                            self.dependencies.scan_budget.error = None;
                        }
                        Err(error) => self.dependencies.scan_budget.error = Some(error.to_string()),
                    }
                }
                ui.label(RichText::new(format!("Applied: {} MiB · for future scans", self.secret_scan_mib)).color(MUTED()));
                if let Some(error) = &self.dependencies.scan_budget.error {
                    pages::callout(ui, AMBER(), error);
                }
                ui.label(RichText::new("A larger budget can take longer. The 128 MiB per-object limit, timeout and cancellation still apply. An incomplete scan blocks the operation.").color(MUTED()));
            });
        });
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
