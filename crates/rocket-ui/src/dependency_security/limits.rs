//! The dependency check's scan limits: how many dependency files a scan takes, how large
//! each may be and how much all of them may add up to. Background scans and before-push
//! checks use the same three. Only the user changes them, in Preferences → Security.
use crate::{app::Rocket, i18n::{tr, trf}, pages, preferences, style::*};
use eframe::egui::*;

pub const MIB: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Dependency files one scan takes.
    pub files: usize,
    /// Each file's size in MiB.
    pub file_mib: u64,
    /// All files together in MiB. Before a push, every pushed tip counts together.
    pub total_mib: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self { files: 256, file_mib: 8, total_mib: 64 }
    }
}

impl Limits {
    /// OSV takes at most 256 files a run, so more run in groups, one after another.
    pub const MAX_FILES: usize = 2048;
    /// rocket-git reads a committed file for a scanner up to 64 MiB.
    pub const MAX_FILE_MIB: u64 = 64;
    /// Scans hold the files in memory while they run.
    pub const MAX_TOTAL_MIB: u64 = 512;

    pub fn file_bytes(&self) -> u64 {
        self.file_mib * MIB
    }
    pub fn total_bytes(&self) -> u64 {
        self.total_mib * MIB
    }
    pub fn valid(&self) -> bool {
        self.problem_key().is_none()
    }
    fn problem_key(&self) -> Option<Problem> {
        if !(1..=Self::MAX_FILES).contains(&self.files) {
            Some(Problem::Files)
        } else if !(1..=Self::MAX_FILE_MIB).contains(&self.file_mib) {
            Some(Problem::Each)
        } else if !(1..=Self::MAX_TOTAL_MIB).contains(&self.total_mib) {
            Some(Problem::All)
        } else if self.file_mib > self.total_mib {
            Some(Problem::EachOverAll)
        } else {
            None
        }
    }
    /// The first rule the values break, as Preferences says it.
    pub fn problem(&self) -> Option<String> {
        Some(match self.problem_key()? {
            Problem::Files => trf!("Files can be 1 to {max}.", max = pages::number_text(Self::MAX_FILES)),
            Problem::Each => trf!("Each file can be 1 to {max} MiB.", max = Self::MAX_FILE_MIB),
            Problem::All => trf!("All files can be 1 to {max} MiB.", max = Self::MAX_TOTAL_MIB),
            Problem::EachOverAll => tr("Each file can’t be larger than All files.").into(),
        })
    }
}

enum Problem {
    Files,
    Each,
    All,
    EachOverAll,
}

/// The three fields' text, a whole number each; anything else is refused, never rounded.
fn parse(text: &[String; 3]) -> Result<Limits, String> {
    let number = |text: &str| -> Option<u64> {
        let text = text.trim();
        (!text.is_empty() && text.len() <= 9 && text.bytes().all(|b| b.is_ascii_digit()))
            .then(|| text.parse().ok())
            .flatten()
    };
    let (Some(files), Some(file_mib), Some(total_mib)) = (number(&text[0]), number(&text[1]), number(&text[2])) else {
        return Err(tr("Enter whole numbers in Scan limits.").into());
    };
    let limits = Limits { files: files as usize, file_mib, total_mib };
    match limits.problem() {
        Some(problem) => Err(problem),
        None => Ok(limits),
    }
}

/// The fields while the user changes them. None shows the limits in use, so a value
/// loaded or applied elsewhere never leaves stale text behind.
#[derive(Default)]
pub(super) struct Edit {
    pub(super) text: Option<[String; 3]>,
    pub(super) error: Option<String>,
}

const FIELDS: [(&str, f32); 3] = [
    ("pref-dependency-files", 64.0),
    ("pref-dependency-file-mib", 52.0),
    ("pref-dependency-total-mib", 64.0),
];

#[derive(Default)]
struct Input {
    typed: bool,
    submit: bool,
    reset: bool,
}

/// One labelled field; Each file and All files are in MiB.
fn field(ui: &mut Ui, widgets: &mut Widgets, label: &str, index: usize, text: &mut String, input: &mut Input) {
    let (key, width) = FIELDS[index];
    ui.label(RichText::new(label).color(MUTED()));
    let response = ui.add(TextEdit::singleline(text).id_salt(key).desired_width(width).char_limit(9));
    pages::control(widgets, key, &response, ui.clip_rect());
    input.typed |= response.changed();
    input.submit |= response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
    if index > 0 {
        ui.label(RichText::new("MiB").color(MUTED()));
    }
}

/// Apply, lit only after an edit.
fn apply_button(ui: &mut Ui, widgets: &mut Widgets, edited: bool, input: &mut Input) {
    let label = tr("Apply");
    let button = if edited { pages::primary(ui, true, label) } else { ui.add_enabled(false, Button::new(label)) };
    pages::control(widgets, "pref-dependency-limits-apply", &button, ui.clip_rect());
    input.submit |= button.clicked();
}

/// Reset to defaults, off while the defaults are in use and nothing is typed.
fn reset_button(ui: &mut Ui, widgets: &mut Widgets, enabled: bool, input: &mut Input) {
    let button = ui.add_enabled(enabled, Button::new(tr("Reset to defaults")));
    pages::control(widgets, "pref-dependency-limits-reset", &button, ui.clip_rect());
    input.reset |= button.clicked();
}

type Widgets = std::collections::HashMap<String, crate::app::UiControl>;

impl Rocket {
    /// Dependency checks' Scan limits row: Files, Each file and All files with one Apply,
    /// which Enter in any field also does, and Reset to defaults.
    pub(super) fn dependency_limits_row(&mut self, ui: &mut Ui) {
        let editable = self.dependencies.editable;
        let applied = self.dependencies.limits;
        let shown = [applied.files.to_string(), applied.file_mib.to_string(), applied.total_mib.to_string()];
        let mut text = self.dependencies.limits_edit.text.clone().unwrap_or_else(|| shown.clone());
        let mut input = Input::default();
        let error = self.dependencies.limits_edit.error.clone();
        let widgets = &mut self.frame_widgets;
        preferences::card_note(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.add(Label::new(RichText::new(tr("Scan limits")).color(TEXT())).wrap());
            ui.add(Label::new(RichText::new(tr("For background scans and before-push checks")).size(12.0).color(HINT())).wrap());
            ui.add_space(6.0);
            ui.spacing_mut().item_spacing.y = 8.0;
            let labels = [tr("Files"), tr("Each file"), tr("All files")];
            // Side by side only when every field and both buttons fit; translations decide.
            let spacing = ui.spacing().item_spacing.x;
            let text_width = |ui: &Ui, text: &str| {
                ui.painter().layout_no_wrap(text.to_owned(), TextStyle::Body.resolve(ui.style()), Color32::PLACEHOLDER).size().x
            };
            let mib = text_width(ui, "MiB");
            let fields: f32 = labels
                .iter()
                .zip(FIELDS)
                .enumerate()
                .map(|(i, (label, (_, width)))| text_width(ui, label) + spacing + width + 8.0 + if i > 0 { spacing + mib + 8.0 } else { 0.0 })
                .sum();
            let buttons = pages::button_width(ui, tr("Apply"), 0.0) + pages::button_width(ui, tr("Reset to defaults"), 0.0) + spacing * 2.0;
            let wide = fields + buttons + 24.0 <= ui.available_width();
            let edited = text.iter().zip(&shown).any(|(a, b)| a.trim() != b);
            let custom = applied != Limits::default();
            ui.add_enabled_ui(editable, |ui| {
                if wide {
                    ui.horizontal(|ui| {
                        for (index, value) in text.iter_mut().enumerate() {
                            if index > 0 {
                                ui.add_space(8.0);
                            }
                            field(ui, widgets, labels[index], index, value, &mut input);
                        }
                        // Right to left: Reset ends the row, Apply sits before it.
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            reset_button(ui, widgets, edited || custom, &mut input);
                            apply_button(ui, widgets, edited, &mut input);
                        });
                    });
                } else {
                    for (index, value) in text.iter_mut().enumerate() {
                        ui.horizontal(|ui| field(ui, widgets, labels[index], index, value, &mut input));
                    }
                    ui.horizontal(|ui| {
                        apply_button(ui, widgets, edited, &mut input);
                        reset_button(ui, widgets, edited || custom, &mut input);
                    });
                }
            });
            if let Some(error) = &error {
                pages::callout(ui, AMBER(), error);
            }
        });
        let edit = &mut self.dependencies.limits_edit;
        if input.typed {
            edit.error = None;
            edit.text = (text.iter().zip(&shown).any(|(a, b)| a.trim() != b)).then(|| text.clone());
        }
        if editable {
            if input.reset {
                self.apply_dependency_limits(Limits::default());
            } else if input.submit && text.iter().zip(&shown).any(|(a, b)| a.trim() != b) {
                match parse(&text) {
                    Ok(limits) => self.apply_dependency_limits(limits),
                    Err(error) => self.dependencies.limits_edit.error = Some(error),
                }
            }
        }
    }

    pub(super) fn apply_dependency_limits(&mut self, limits: Limits) {
        if !self.dependencies.editable || !limits.valid() {
            return;
        }
        self.dependencies.limits_edit = Edit::default();
        if self.dependencies.limits == limits {
            return;
        }
        self.dependencies.limits = limits;
        // A scan still running with the old limits would store what they decided and
        // push the next scan 30 s out; drop it, as turning checks off does.
        if let Some(job) = &mut self.dependencies.job
            && !job.update
        {
            job.discard = true;
            job.task.cancel();
        }
        // Scan again soon with the new limits; a result they no longer allow stays
        // shown until then, like any other result.
        for state in self.dependencies.repositories.values_mut() {
            state.next = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(a: &str, b: &str, c: &str) -> [String; 3] {
        [a.into(), b.into(), c.into()]
    }

    #[test]
    fn whole_numbers_inside_each_range_are_accepted_and_nothing_else() {
        assert_eq!(parse(&text(" 256 ", "8", "64")).unwrap(), Limits::default());
        assert_eq!(parse(&text("2048", "64", "512")).unwrap(), Limits { files: 2048, file_mib: 64, total_mib: 512 });
        assert_eq!(parse(&text("1", "1", "1")).unwrap(), Limits { files: 1, file_mib: 1, total_mib: 1 });
        for bad in [
            text("", "8", "64"),
            text("2,048", "8", "64"),
            text("256", "8.5", "64"),
            text("256", "-8", "64"),
            text("256", "8", "６４"),
            text("256", "8", "64 MiB"),
            text("1000000000", "8", "64"),
        ] {
            assert!(parse(&bad).is_err(), "{bad:?}");
        }
        for (bad, says) in [
            (text("0", "8", "64"), "Files can be 1 to 2,048."),
            (text("2049", "8", "64"), "Files can be 1 to 2,048."),
            (text("256", "0", "64"), "Each file can be 1 to 64 MiB."),
            (text("256", "65", "512"), "Each file can be 1 to 64 MiB."),
            (text("256", "8", "513"), "All files can be 1 to 512 MiB."),
            (text("256", "16", "8"), "Each file can’t be larger than All files."),
        ] {
            assert_eq!(parse(&bad).unwrap_err(), says, "{bad:?}");
        }
    }

    #[test]
    fn defaults_are_todays_fixed_limits_and_fit_the_ranges() {
        let limits = Limits::default();
        assert_eq!((limits.files, limits.file_bytes(), limits.total_bytes()), (256, 8 * MIB, 64 * MIB));
        assert!(limits.valid());
        assert!(Limits { files: Limits::MAX_FILES, file_mib: Limits::MAX_FILE_MIB, total_mib: Limits::MAX_TOTAL_MIB }.valid());
    }
}
