//! Spell checking for the commit message: Windows, Linux and macOS, off by default. The
//! rocket-spell worker checks text off the UI thread. Results are kept per exact
//! text, so an edited field never shows underlines computed for older text.
use crate::{
    app::UiControl,
    style::{FAINT, RED},
};
use eframe::egui::{text::LayoutJob, *};
use rocket_spell::{Misspelling, Pending, SpellChecker};
use std::{collections::HashMap, ops::Range, sync::Arc};

/// Most suggestions a menu lists.
const SUGGESTIONS: usize = 8;

/// Recently checked texts kept: the summary, the description and one spare.
const KEPT: usize = 3;

impl crate::app::Rocket {
    /// Diagnostic counters only: never expose commit text, words or dictionary contents.
    pub fn spelling_status(&self) -> serde_json::Value {
        let s = &self.spelling;
        serde_json::json!({"enabled":s.enabled,"ready":s.checker.is_some(),"waiting":s.waiting(),
            "failed":s.problem.is_some(),"checked_texts":s.checks.len(),
            "misspellings":s.checks.iter().map(|c|c.found.len()).sum::<usize>()})
    }
}

struct Check {
    text: String,
    pending: Option<Pending<Vec<Misspelling>>>,
    found: Vec<Misspelling>,
}

/// The word a context menu was opened on, with the suggestions for it.
pub struct Target {
    /// The text field the menu belongs to.
    pub field: Id,
    pub range: Range<usize>,
    pub word: String,
    pending: Option<Pending<Vec<String>>>,
    /// None until the checker answers.
    pub suggestions: Option<Vec<String>>,
}

#[derive(Default)]
pub struct Spelling {
    enabled: bool,
    starting: Option<Pending<SpellChecker>>,
    stopping: Option<Pending<()>>,
    checker: Option<SpellChecker>,
    /// Why checking is off although it is enabled, for Preferences.
    pub problem: Option<String>,
    checks: Vec<Check>,
    pub target: Option<Target>,
    /// An Ignore or Add request; the checks are redone when it is answered.
    word_change: Option<Pending<()>>,
}

impl Spelling {
    /// Whether this platform has a spell checker Rocket can use.
    pub fn supported() -> bool {
        cfg!(any(windows, target_os = "linux", target_os = "macos"))
    }

    /// Start or stop the checker to match the preference.
    pub fn enable(&mut self, on: bool) {
        self.enabled = on;
        if !on {
            self.checks.clear();
            self.target = None;
            if let Some(checker) = self.checker.take() {
                self.stopping = Some(checker.stop());
            }
            self.problem = None;
            // Keep startup/shutdown and accepted dictionary changes observable.
            // A rapid re-enable must not create another worker during cleanup.
        }
        self.start_if_enabled();
    }

    fn start_if_enabled(&mut self) -> bool {
        if self.enabled
            && self.checker.is_none()
            && self.starting.is_none()
            && self.stopping.is_none()
            && self.word_change.is_none()
            && self.problem.is_none()
            && Self::supported()
        {
            self.starting = Some(SpellChecker::start_in_background(None));
            true
        } else {
            false
        }
    }

    pub fn changing_dictionary(&self) -> bool {
        self.word_change.is_some()
    }

    pub fn language(&self) -> Option<&str> {
        self.checker.as_ref().map(SpellChecker::language)
    }

    /// Collect answers. Returns whether anything changed, so the caller repaints.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        if let Some(result) = self.starting.as_ref().and_then(Pending::poll) {
            self.starting = None;
            match result {
                Ok(checker) if self.enabled => self.checker = Some(checker),
                Ok(checker) => self.stopping = Some(checker.stop()),
                Err(e) => self.problem = Some(e.to_string()),
            }
            changed = true;
        }
        if let Some(result) = self.stopping.as_ref().and_then(Pending::poll) {
            self.stopping = None;
            if let Err(error) = result {
                self.problem = Some(error.to_string());
            }
            changed = true;
        }
        for check in &mut self.checks {
            if let Some(result) = check.pending.as_ref().and_then(Pending::poll) {
                check.pending = None;
                // A failed check shows no underlines; the problem is reported once.
                match result {
                    Ok(found) => check.found = found,
                    Err(e) => self.problem = Some(e.to_string()),
                }
                changed = true;
            }
        }
        if let Some(target) = &mut self.target
            && let Some(result) = target.pending.as_ref().and_then(Pending::poll)
        {
            target.pending = None;
            target.suggestions = Some(result.unwrap_or_default());
            changed = true;
        }
        if let Some(result) = self.word_change.as_ref().and_then(Pending::poll) {
            self.word_change = None;
            if let Err(e) = result {
                self.problem = Some(e.to_string());
            }
            // Recheck every text with the changed word list.
            self.checks.clear();
            changed = true;
        }
        self.start_if_enabled() || changed
    }

    /// Whether answers are still expected, so the caller keeps repainting.
    pub fn waiting(&self) -> bool {
        self.starting.is_some()
            || self.stopping.is_some()
            || self.changing_dictionary()
            || self.checks.iter().any(|c| c.pending.is_some())
            || self.target.as_ref().is_some_and(|t| t.pending.is_some())
    }

    /// The flagged ranges of `text`, asking the checker about new text. Empty while
    /// the answer is pending or checking is off.
    pub fn misspelled(&mut self, text: &str) -> &[Misspelling] {
        let Some(checker) = &self.checker else {
            return &[];
        };
        let index = match self.checks.iter().position(|c| c.text == text) {
            Some(index) => index,
            None => {
                if self.checks.len() == KEPT {
                    self.checks.remove(0);
                }
                self.checks.push(Check {
                    text: text.to_owned(),
                    pending: Some(checker.check(text)),
                    found: Vec::new(),
                });
                self.checks.len() - 1
            }
        };
        &self.checks[index].found
    }

    /// Open the suggestions for the flagged word at byte `at` of `text` in `field`.
    pub fn target_at(&mut self, field: Id, text: &str, at: usize) -> bool {
        let Some(checker) = &self.checker else {
            return false;
        };
        let range = self
            .checks
            .iter()
            .find(|c| c.text == text)
            .and_then(|c| c.found.iter().find(|m| m.range.contains(&at)))
            .map(|m| m.range.clone());
        self.target = range.map(|range| {
            let word = text[range.clone()].to_owned();
            Target {
                field,
                pending: Some(checker.suggest(&word)),
                range,
                word,
                suggestions: None,
            }
        });
        self.target.is_some()
    }

    /// Stop flagging the target word until Rocket restarts.
    pub fn ignore_target(&mut self) {
        if let (Some(checker), Some(target)) = (&self.checker, self.target.take()) {
            self.word_change = Some(checker.ignore(&target.word));
        }
    }

    /// Add the target word to the user's dictionary (on Linux, Rocket's word list).
    pub fn add_target(&mut self) {
        if let (Some(checker), Some(target)) = (&self.checker, self.target.take()) {
            self.word_change = Some(checker.add(&target.word));
        }
    }
}

enum Choice {
    Replace(String),
    Ignore,
    Add,
}

/// Show the edit `build` makes over `text`. While a checker runs, flagged words are
/// underlined, and right-clicking one offers suggestions, Ignore and Add to dictionary;
/// a chosen suggestion replaces the word and marks the response changed. With checking
/// off the edit is shown unchanged. `font` and `color` must match the edit's own.
pub fn show(
    ui: &mut Ui,
    spelling: &mut Spelling,
    text: &mut String,
    (font, color): (FontId, Color32),
    widgets: &mut HashMap<String, UiControl>,
    build: impl for<'a> FnOnce(&'a mut String) -> TextEdit<'a>,
) -> Response {
    if spelling.language().is_none() {
        return build(text).show(ui).response.response;
    }
    let output = {
        let mut layouter = |ui: &Ui, buffer: &dyn TextBuffer, wrap_width: f32| {
            let found = spelling.misspelled(buffer.as_str()).to_vec();
            layout(
                ui,
                buffer.as_str(),
                &found,
                font.clone(),
                color,
                RED(),
                wrap_width,
            )
        };
        build(text).layouter(&mut layouter).show(ui)
    };
    let mut response = output.response.response;
    if response.secondary_clicked()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let at = output
            .galley
            .cursor_from_pos(pos - output.galley_pos)
            .index
            .0;
        let byte = text
            .char_indices()
            .nth(at)
            .map_or(text.len(), |(byte, _)| byte);
        spelling.target_at(response.id, text, byte);
    }
    let Some(target) = spelling.target.as_ref().filter(|t| t.field == response.id) else {
        return response;
    };
    let mut choice = None;
    let menu = response.context_menu(|ui| {
        match &target.suggestions {
            None => {
                ui.label(RichText::new("Looking for suggestions…").color(FAINT()));
            }
            Some(words) if words.is_empty() => {
                ui.label(RichText::new("No suggestions").color(FAINT()));
            }
            Some(words) => {
                for (i, word) in words.iter().take(SUGGESTIONS).enumerate() {
                    let r = ui.button(word);
                    widgets.insert(format!("spell-suggestion-{i}"), (&r).into());
                    if r.clicked() {
                        choice = Some(Choice::Replace(word.clone()));
                    }
                }
            }
        }
        ui.separator();
        let ignore = ui.button("Ignore");
        widgets.insert("spell-ignore".into(), (&ignore).into());
        if ignore.clicked() {
            choice = Some(Choice::Ignore);
        }
        let add = ui.button("Add to dictionary");
        widgets.insert("spell-add".into(), (&add).into());
        if add.clicked() {
            choice = Some(Choice::Add);
        }
        if choice.is_some() {
            ui.close();
        }
    });
    match choice {
        Some(Choice::Replace(word)) => {
            let target = spelling.target.take().expect("menu target");
            if replace(text, &target.range, &target.word, &word) {
                response.mark_changed();
            }
        }
        Some(Choice::Ignore) => spelling.ignore_target(),
        Some(Choice::Add) => spelling.add_target(),
        // The menu closed without a choice.
        None if menu.is_none() => spelling.target = None,
        None => {}
    }
    response
}

/// Replace `range` of `text` with `with` if it still holds `word`; false otherwise.
pub fn replace(text: &mut String, range: &Range<usize>, word: &str, with: &str) -> bool {
    if text.get(range.clone()) != Some(word) {
        return false;
    }
    text.replace_range(range.clone(), with);
    true
}

/// Lay out `text` with each flagged range underlined.
pub fn layout(
    ui: &Ui,
    text: &str,
    found: &[Misspelling],
    font: FontId,
    color: Color32,
    underline: Color32,
    wrap_width: f32,
) -> Arc<Galley> {
    let mut job = LayoutJob::default();
    let plain = TextFormat::simple(font.clone(), color);
    let flagged = TextFormat {
        underline: Stroke::new(1.5, underline),
        ..TextFormat::simple(font, color)
    };
    let mut at = 0;
    for m in found {
        // Ranges come sorted from the checker; skip any that overlap or no longer fit.
        if m.range.start < at || text.get(m.range.clone()).is_none() {
            continue;
        }
        job.append(&text[at..m.range.start], 0.0, plain.clone());
        job.append(&text[m.range.clone()], 0.0, flagged.clone());
        at = m.range.end;
    }
    job.append(&text[at..], 0.0, plain);
    job.wrap.max_width = wrap_width;
    ui.fonts_mut(|f| f.layout_job(job))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn disabling_keeps_an_accepted_dictionary_change_observable() {
        let mut spelling = Spelling::default();
        spelling.enable(true);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while spelling.starting.is_some() {
            spelling.poll();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(spelling.checker.is_some(), "{:?}", spelling.problem);
        let checker = spelling.checker.as_ref().unwrap();
        // Ignore is scoped to this checker; never add to the shared dictionary.
        spelling.word_change = Some(checker.ignore("smple"));
        spelling.enable(false);
        assert!(spelling.language().is_none());
        assert!(spelling.changing_dictionary());
        assert!(spelling.waiting());
        while spelling.waiting() {
            spelling.poll();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(spelling.problem.is_none());
        assert!(!spelling.waiting());
    }

    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn rapid_toggle_observes_startup_and_cleanup_before_restarting() {
        let mut spelling = Spelling::default();
        spelling.enable(true);
        spelling.enable(false);
        assert!(
            spelling.waiting(),
            "startup remains owned until its result is handled"
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        for _ in 0..128 {
            spelling.enable(true);
            spelling.enable(false);
        }
        while spelling.waiting() {
            spelling.poll();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(spelling.language().is_none());
        spelling.enable(true);
        while spelling.waiting() {
            spelling.poll();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(spelling.language().is_some(), "{:?}", spelling.problem);
        spelling.enable(false);
        while spelling.waiting() {
            spelling.poll();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn a_suggestion_replaces_only_unchanged_text() {
        let mut text = "Fix the smple bug".to_owned();
        assert!(replace(&mut text, &(8..13), "smple", "simple"));
        assert_eq!(text, "Fix the simple bug");
        // The text changed after the menu opened: nothing is replaced.
        assert!(!replace(&mut text, &(8..13), "smple", "simple"));
        assert!(!replace(&mut text, &(16..40), "bug", "x"));
        assert_eq!(text, "Fix the simple bug");
    }

    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn checks_are_kept_per_text_and_redone_after_ignoring_a_word() {
        let mut spelling = Spelling::default();
        spelling.enable(true);
        let ready = |s: &mut Spelling| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while s.waiting() {
                s.poll();
                assert!(std::time::Instant::now() < deadline, "no answer");
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        };
        ready(&mut spelling);
        assert!(spelling.language().is_some(), "{:?}", spelling.problem);
        let text = "Fix the smple 中文 bug";
        assert!(spelling.misspelled(text).is_empty());
        ready(&mut spelling);
        let ranges: Vec<_> = spelling
            .misspelled(text)
            .iter()
            .map(|m| m.range.clone())
            .collect();
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0], 8..13);
        let field = Id::new("summary");
        assert!(!spelling.target_at(field, text, 2));
        assert!(spelling.target_at(field, text, 10));
        ready(&mut spelling);
        let target = spelling.target.as_ref().unwrap();
        assert_eq!(target.word, "smple");
        assert!(
            target
                .suggestions
                .as_ref()
                .unwrap()
                .iter()
                .any(|s| s == "simple")
        );
        spelling.ignore_target();
        ready(&mut spelling);
        spelling.misspelled(text);
        ready(&mut spelling);
        assert!(spelling.misspelled(text).is_empty());
        spelling.enable(false);
        ready(&mut spelling);
        assert!(spelling.problem.is_none());
        assert!(spelling.language().is_none());
    }
}
