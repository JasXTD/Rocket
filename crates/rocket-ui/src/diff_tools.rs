//! Display-only lexical colors and cancellable literal search. Raw patch indexes
//! remain the sole identity for staging; neither feature rewrites patch bytes.
pub use crate::syntax::{Span, Token};
use crate::{
    diff::{FilePatch, Kind, Line},
    style::*,
};
use eframe::egui::*;
use rocket_git::{CancellationToken, DiagnosticLog, ReadTask, Result};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
mod row_cache;
pub(crate) fn prepare(file: &mut FilePatch) {
    crate::syntax::prepare(file);
}
#[derive(Clone, Debug)]
pub struct Hit {
    pub line: usize,
    pub start: usize,
    pub end: usize,
}
pub struct Matches {
    pub hits: Vec<Hit>,
    pub limited: bool,
}
pub struct State {
    pub overview: crate::overview::Probe,
    rows: row_cache::Cache,
    pub query: String,
    pub syntax: bool,
    /// The Syntax setting last followed; the toolbar checkbox flips one tab until it changes.
    syntax_setting: bool,
    /// Whether the first column shows each line's − or +.
    pub marks: bool,
    pub font: u8,
    pub words: bool,
    pub word_highlights: crate::word_diff::Highlights,
    pub word_error: Option<String>,
    word_task: Option<ReadTask<crate::word_diff::Highlights>>,
    word_key: u64,
    word_enabled: bool,
    pub hits: Vec<Hit>,
    pub limited: bool,
    pub current: usize,
    pub error: Option<String>,
    pub task: Option<ReadTask<Matches>>,
    pub log: Option<DiagnosticLog>,
    key: u64,
    previous: String,
    pending: Option<(Instant, Arc<Vec<Line>>)>,
    pub jump: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            overview: Default::default(),
            rows: Default::default(),
            query: String::new(),
            syntax: true,
            syntax_setting: true,
            marks: true,
            font: 13,
            words: true,
            word_highlights: Default::default(),
            word_error: None,
            word_task: None,
            word_key: 0,
            word_enabled: false,
            hits: Vec::new(),
            limited: false,
            current: 0,
            error: None,
            task: None,
            log: None,
            key: 0,
            previous: String::new(),
            pending: None,
            jump: false,
        }
    }
}
impl State {
    pub fn with_log(log: Option<DiagnosticLog>) -> Self {
        Self {
            log,
            ..Default::default()
        }
    }
    /// Takes on Preferences → General's Diff view settings.
    pub(crate) fn follow(&mut self, options: crate::general::Options) {
        self.marks = options.marks;
        self.font = options.font;
        if self.syntax_setting != options.syntax {
            self.syntax_setting = options.syntax;
            self.syntax = options.syntax;
        }
    }
    pub(crate) fn row_height(&self) -> f32 {
        crate::general::diff_row_height(self.font)
    }
    pub fn running(&self) -> bool {
        self.task.is_some() || self.pending.is_some()
    }
    pub fn cancel(&mut self) {
        self.word_task = None;
        self.cancel_search();
    }
    pub fn invalidate(&mut self) {
        self.cancel();
        self.rows.clear();
        self.key = 0;
        self.word_key = 0;
    }
    fn cancel_search(&mut self) {
        self.task = None;
        self.pending = None;
    }
    pub fn words_running(&self) -> bool {
        self.word_task.is_some()
    }
    pub fn stop_words(&mut self) {
        self.word_task = None;
        self.word_error = Some(crate::i18n::tr("Word highlights cancelled. Toggle Words to retry.").into());
    }
    pub fn poll(&mut self) {
        if let Some(result) = self.word_task.as_mut().and_then(ReadTask::try_finish) {
            self.word_task = None;
            self.rows.clear();
            match result {
                Ok(highlights) => self.word_highlights = highlights,
                Err(e) => self.word_error = Some(crate::backend::user_error(&e)),
            }
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|(since, _)| since.elapsed() >= Duration::from_millis(180))
        {
            let (_, lines) = self.pending.take().unwrap();
            let query = self.query.clone();
            let log = self.log.clone();
            match ReadTask::spawn(move |cancel| {
                if let Some(log) = log {
                    log.request("diff-search", |_| search(&lines, &query, &cancel))
                } else {
                    search(&lines, &query, &cancel)
                }
            }) {
                Ok(t) => self.task = Some(t),
                Err(e) => self.error = Some(crate::backend::user_error(&e)),
            }
        }
        if let Some(result) = self.task.as_mut().and_then(ReadTask::try_finish) {
            self.task = None;
            self.rows.clear();
            match result {
                Ok(m) => {
                    self.hits = m.hits;
                    self.limited = m.limited;
                    self.current = 0;
                    self.jump = true;
                }
                Err(e) => self.error = Some(crate::backend::user_error(&e)),
            }
        }
    }
    pub fn update(&mut self, file: &FilePatch, ctx: &Context) {
        if self.word_key != file.id || self.word_enabled != self.words {
            self.rows.clear();
            self.word_task = None;
            self.word_highlights = Default::default();
            self.word_error = None;
            self.word_key = file.id;
            self.word_enabled = self.words;
            if self.words {
                let lines = file.lines.clone();
                let log = self.log.clone();
                match ReadTask::spawn(move |cancel| {
                    if let Some(log) = log {
                        log.request("diff-words", |_| crate::word_diff::compute(&lines, &cancel))
                    } else {
                        crate::word_diff::compute(&lines, &cancel)
                    }
                }) {
                    Ok(task) => self.word_task = Some(task),
                    Err(e) => self.word_error = Some(crate::backend::user_error(&e)),
                }
            }
        }
        if self.key != file.id || self.previous != self.query {
            self.rows.clear();
            self.cancel_search();
            self.hits.clear();
            self.error = None;
            self.current = 0;
            self.limited = false;
            self.key = file.id;
            self.previous = self.query.clone();
            if !self.query.is_empty() {
                self.pending = Some((Instant::now(), file.lines.clone()));
            }
        }
        if self.running() || self.words_running() {
            ctx.request_repaint_after(Duration::from_millis(25));
        }
    }
    pub fn navigate(&mut self, forward: bool) {
        if !self.hits.is_empty() {
            self.current = if forward {
                (self.current + 1) % self.hits.len()
            } else {
                (self.current + self.hits.len() - 1) % self.hits.len()
            };
            self.jump = true;
        }
    }
    pub(crate) fn cached_layout(&mut self, file: &FilePatch, index: usize, color: Color32) -> Arc<text::LayoutJob> {
        let style = row_cache::Style {
            file: file.id,
            syntax: self.syntax,
            words: self.words,
            marks: self.marks,
            font: self.font,
            current_match: self.current,
            palette: crate::style::generation(),
        };
        if let Some(job) = self.rows.get(style, index, color) { return job; }
        let job = Arc::new(self.layout(&file.lines[index], index, color));
        self.rows.insert(index, color, job.clone());
        job
    }

    pub fn layout(&self, line: &Line, index: usize, color: Color32) -> text::LayoutJob {
        // Patch lines keep their first column; hiding it only starts the text one byte on.
        let shown_from = usize::from(!self.marks && matches!(line.kind, Kind::Added | Kind::Removed | Kind::Context));
        let mut boundaries = vec![shown_from, line.text.len()];
        let words = if self.words {
            self.word_highlights
                .spans
                .get(&index)
                .map(Vec::as_slice)
                .unwrap_or_default()
        } else {
            &[]
        };
        for word in words {
            boundaries.extend([word.start, word.end]);
        }
        if self.syntax {
            for s in &line.syntax {
                boundaries.extend([s.start, s.end]);
            }
        }
        let first = self.hits.partition_point(|h| h.line < index);
        let matches: Vec<_> = self.hits[first..]
            .iter()
            .take_while(|h| h.line == index)
            .enumerate()
            .collect();
        for (_, h) in &matches {
            boundaries.extend([h.start, h.end]);
        }
        boundaries.sort_unstable();
        boundaries.dedup();
        boundaries.retain(|b| *b >= shown_from);
        let mut job = text::LayoutJob::default();
        // Both sets are ordered and disjoint. Advance through each once instead
        // of rescanning every span for every character in a long, dense line.
        let (mut syntax_index, mut match_index, mut word_index) = (0, 0, 0);
        for pair in boundaries.windows(2) {
            let start = pair[0];
            let end = pair[1];
            while words.get(word_index).is_some_and(|word| word.end <= start) {
                word_index += 1;
            }
            let word = words
                .get(word_index)
                .is_some_and(|word| word.start <= start);
            while self.syntax
                && line
                    .syntax
                    .get(syntax_index)
                    .is_some_and(|s| s.end <= start)
            {
                syntax_index += 1;
            }
            while matches
                .get(match_index)
                .is_some_and(|(_, h)| h.end <= start)
            {
                match_index += 1;
            }
            let token = if self.syntax {
                line.syntax
                    .get(syntax_index)
                    .filter(|s| s.start <= start)
                    .map(|s| s.token)
            } else {
                None
            };
            let foreground = match token {
                Some(Token::Keyword) => SYNTAX_KEYWORD(),
                Some(Token::String) => SYNTAX_STRING(),
                Some(Token::Comment) => SYNTAX_COMMENT(),
                Some(Token::Number) => SYNTAX_NUMBER(),
                Some(Token::Type) => SYNTAX_TYPE(),
                Some(Token::Function) => SYNTAX_FUNCTION(),
                Some(Token::Property) => SYNTAX_PROPERTY(),
                Some(Token::Attribute) => SYNTAX_ATTRIBUTE(),
                Some(Token::Operator) => SYNTAX_OPERATOR(),
                None => color,
            };
            let hit = matches.get(match_index).filter(|(_, h)| h.start <= start);
            let background = match hit {
                Some((offset, _)) if first + offset == self.current => {
                    MATCH()
                }
                Some(_) => MATCH_DIM(),
                None if word && line.kind == Kind::Removed => REMOVED_WORD(),
                None if word => ADDED_WORD(),
                None => Color32::TRANSPARENT,
            };
            job.append(
                &line.text[start..end],
                0.0,
                TextFormat {
                    font_id: FontId::monospace(f32::from(self.font)),
                    color: if hit.is_some() { TEXT() } else { foreground },
                    background,
                    ..Default::default()
                },
            );
        }
        job
    }
}
fn search(lines: &[Line], query: &str, cancel: &CancellationToken) -> Result<Matches> {
    let mut hits = Vec::new();
    if query.is_empty() {
        return Ok(Matches {
            hits,
            limited: false,
        });
    }
    for (i, line) in lines.iter().enumerate() {
        cancel.check()?;
        if matches!(line.kind, Kind::Header | Kind::Hunk) {
            continue;
        }
        for (start, value) in line.text.match_indices(query) {
            if hits.len() == 20_000 {
                return Ok(Matches {
                    hits,
                    limited: true,
                });
            }
            hits.push(Hit {
                line: i,
                start,
                end: start + value.len(),
            });
        }
    }
    Ok(Matches {
        hits,
        limited: false,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::Patch;

    #[test]
    fn hidden_marks_keep_every_span_in_place_and_the_font_follows() {
        let patch = Patch::parse(b"diff --git a/a.rs b/a.rs\n@@ -1,2 +1,2 @@ fn top\n fn same() {}\n-fn old() {}\n+fn new() {}\n");
        let file = &patch.files[0];
        let row = |kind| file.lines.iter().position(|line| line.kind == kind).unwrap();
        let (added, hunk, context) = (row(Kind::Added), row(Kind::Hunk), row(Kind::Context));
        let mut state = State { words: false, ..Default::default() };
        let shown = |state: &State, index: usize| state.layout(&file.lines[index], index, TEXT());
        assert_eq!(shown(&state, added).text, "+fn new() {}");
        state.marks = false;
        let job = shown(&state, added);
        assert_eq!(job.text, "fn new() {}");
        assert_eq!(job.sections[0].format.color, SYNTAX_KEYWORD(), "the keyword still starts the line");
        assert_eq!(shown(&state, context).text, "fn same() {}");
        assert_eq!(shown(&state, hunk).text, file.lines[hunk].text, "a hunk header has no mark");
        state.font = 16;
        assert!(shown(&state, added).sections.iter().all(|s| s.format.font_id.size == 16.0));
        // A changed option redraws rows held in the cache.
        let first = state.cached_layout(file, added, TEXT());
        state.marks = true;
        assert_eq!(state.cached_layout(file, added, TEXT()).text, "+fn new() {}");
        assert_ne!(first.text, "+fn new() {}");
    }

    #[test]
    fn the_syntax_setting_reaches_each_tab_and_its_checkbox_flips_one() {
        use crate::general::Options;
        let off = Options { syntax: false, ..Default::default() };
        let mut tab = State::default();
        tab.follow(off);
        assert!(!tab.syntax, "a tab starts from the setting");
        tab.syntax = true;
        tab.follow(off);
        assert!(tab.syntax, "the toolbar checkbox stays until the setting changes");
        tab.follow(Options::default());
        tab.follow(off);
        assert!(!tab.syntax);
        tab.follow(Options { font: 15, marks: false, ..off });
        assert_eq!((tab.font, tab.marks, tab.row_height()), (15, false, 24.0));
    }

    #[test]
    fn cached_formatting_tracks_search_words_toggles_and_file_generations() {
        let patch = Patch::parse(b"diff --git a/a.rs b/a.rs\n@@ -1 +1 @@\n-fn old() {}\n+fn new() { new(); }\n");
        let file = &patch.files[0];
        let index = file.lines.iter().position(|line| line.kind == Kind::Added).unwrap();
        let ctx = Context::default();
        let mut state = State { words: false, ..Default::default() };
        state.update(file, &ctx);
        let first = state.cached_layout(file, index, TEXT());
        assert!(Arc::ptr_eq(&first, &state.cached_layout(file, index, TEXT())));
        assert_eq!(*first, state.layout(&file.lines[index], index, TEXT()));
        state.syntax = false;
        let plain = state.cached_layout(file, index, TEXT());
        assert!(!Arc::ptr_eq(&first, &plain));
        assert_eq!(*plain, state.layout(&file.lines[index], index, TEXT()));

        state.query = "new".into();
        state.update(file, &ctx);
        let deadline = Instant::now() + Duration::from_secs(5);
        while state.running() {
            state.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(state.hits.len(), 2);
        let searched = state.cached_layout(file, index, TEXT());
        assert_eq!(*searched, state.layout(&file.lines[index], index, TEXT()));
        assert_ne!(*searched, *plain);
        state.navigate(true);
        let next = state.cached_layout(file, index, TEXT());
        assert_eq!(*next, state.layout(&file.lines[index], index, TEXT()));
        assert_ne!(*next, *searched);

        state.query.clear();
        state.words = true;
        state.update(file, &ctx);
        let deadline = Instant::now() + Duration::from_secs(5);
        while state.words_running() {
            state.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        let words = state.cached_layout(file, index, TEXT());
        assert_eq!(*words, state.layout(&file.lines[index], index, TEXT()));
        assert_ne!(*words, *plain);
        state.words = false;
        state.update(file, &ctx);
        assert_eq!(*state.cached_layout(file, index, TEXT()), *plain);
        let other = Patch::parse(b"diff --git a/a.rs b/a.rs\n@@ -1 +1 @@\n-fn old() {}\n+fn different() {}\n");
        state.update(&other.files[0], &ctx);
        assert_eq!(state.cached_layout(&other.files[0], index, TEXT()).text, "+fn different() {}");
        state.invalidate();
        assert_eq!(*state.cached_layout(&other.files[0], index, TEXT()), state.layout(&other.files[0].lines[index], index, TEXT()));
    }
    #[test]
    fn dense_unicode_search_overlays_preserve_text_and_selected_match() {
        let text = format!("+{}", "fn 你好 123; ".repeat(500));
        let mut lexer = crate::syntax::Lexer::new(crate::syntax::Language::Rust);
        let line = Line {
            syntax: lexer.line(&text[1..], 1),
            text,
            raw_index: 4,
            kind: Kind::Added,
            old: None,
            new: Some(1),
        };
        let lines = vec![line];
        let found = search(&lines, "你好 123", &CancellationToken::new()).unwrap();
        assert_eq!(found.hits.len(), 500);
        let mut state = State {
            hits: found.hits,
            current: 217,
            ..Default::default()
        };
        for syntax in [true, false] {
            state.syntax = syntax;
            let job = state.layout(&lines[0], 0, TEXT());
            assert_eq!(job.text, lines[0].text);
            let selected: String = job
                .sections
                .iter()
                .filter(|s| s.format.background == MATCH())
                .map(|s| &job.text[usize::from(s.byte_range.start)..usize::from(s.byte_range.end)])
                .collect();
            assert_eq!(selected, "你好 123");
            assert!(
                job.sections
                    .iter()
                    .all(|s| job.text.is_char_boundary(s.byte_range.start.into())
                        && job.text.is_char_boundary(s.byte_range.end.into()))
            );
        }
    }
    #[test]
    fn lexical_colors_do_not_change_unicode_or_patch_indexes() {
        let p=Patch::parse("diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1,2 @@\n-fn old() {}\n+fn 新() { println!(\"你好 🚀\"); }\n+// comment\n".as_bytes());
        let f = &p.files[0];
        let line = f.lines.iter().find(|l| l.text.contains("你好")).unwrap();
        assert!(
            line.syntax
                .iter()
                .any(|s| s.token == Token::String && line.text[s.start..s.end].contains("你好"))
        );
        for s in &line.syntax {
            assert!(line.text.is_char_boundary(s.start) && line.text.is_char_boundary(s.end));
        }
        assert_eq!(line.raw_index, 5);
    }
    #[test]
    fn unicode_search_is_exact_cancellable_and_bounded() {
        let p = Patch::parse(
            "diff --git a/a.rs b/a.rs\n@@ -1 +1 @@\n-你好 é\n+你好 🚀 你好 é\n".as_bytes(),
        );
        let lines = &p.files[0].lines;
        let m = search(lines, "你好", &CancellationToken::new()).unwrap();
        assert_eq!(m.hits.len(), 3);
        for h in m.hits {
            assert_eq!(&lines[h.line].text[h.start..h.end], "你好");
        }
        let c = CancellationToken::new();
        c.cancel();
        assert!(search(lines, "é", &c).is_err());
        let mut p = Patch::parse(
            format!(
                "diff --git a/a.rs b/a.rs\n@@ -0,0 +1 @@\n+{}",
                "x".repeat(8000)
            )
            .as_bytes(),
        );
        let lines = Arc::make_mut(&mut p.files[0].lines);
        let line = lines.last().unwrap().clone();
        lines.extend([line.clone(), line]);
        let m = search(lines, "x", &CancellationToken::new()).unwrap();
        assert!(m.limited);
        assert_eq!(m.hits.len(), 20_000);
    }
}
