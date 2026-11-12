use crate::{
    app::*,
    backend::{Action, Outcome, user_error},
    pages::{self, control},
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
#[derive(Default)]
pub struct DialogState {
    pub snapshot: Option<IgnoreSnapshot>,
    pub preview: Option<Arc<IgnorePreview>>,
    pub read: Option<ReadTask<IgnoreSnapshot>>,
    pub task: Option<ReadTask<IgnorePreview>>,
    pub text: String,
    pub path: String,
    pub folder: bool,
    pub pending: Vec<RepoPath>,
    pub candidates: Vec<RepoPath>,
    pub selected: Vec<bool>,
    pub applying: bool,
    pub stopping: bool,
    pub notice: Option<String>,
    /// Add rules: 0 new files, 1 an exact path, 2 a template.
    pub tab: usize,
}
impl DialogState {
    pub fn running(&self) -> bool {
        self.read.is_some() || self.task.is_some() || self.applying
    }
}
impl Rocket {
    pub(crate) fn start_ignore_dialog(&mut self) {
        let Some(DialogKind::Ignore(paths)) = self.dialog.as_ref().map(|d| &d.kind) else {
            return;
        };
        let pending = paths.clone();
        let candidates = self
            .tab()
            .and_then(|t| t.snapshot.as_ref())
            .and_then(|s| s.status.as_ref())
            .map(|s| {
                s.entries
                    .iter()
                    .filter(|e| e.kind == EntryKind::Untracked)
                    .map(|e| e.path.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if let Some(d) = &mut self.dialog {
            d.ignore = Some(DialogState {
                selected: vec![false; candidates.len()],
                candidates,
                pending,
                ..Default::default()
            })
        }
        self.load_ignore();
    }
    pub(crate) fn load_ignore(&mut self) {
        let path = self.tab().map(|t| t.path.clone());
        let backend = self.backend.clone();
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.ignore else { return };
        if s.running() {
            return;
        }
        s.preview = None;
        s.snapshot = None;
        s.notice = None;
        d.error = None;
        if let (Some(path), Some(b)) = (path, backend) {
            match ReadTask::spawn(move |c| b.ignore_snapshot(&path, &c)) {
                Ok(t) => s.read = Some(t),
                Err(e) => d.error = Some(user_error(&e)),
            }
        }
    }
    pub(crate) fn start_ignore_review(&mut self) {
        let path = self.tab().map(|t| t.path.clone());
        let backend = self.backend.clone();
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.ignore else { return };
        if s.running() {
            return;
        }
        s.preview = None;
        s.notice = None;
        d.error = None;
        let (Some(path), Some(b), Some(snapshot)) = (path, backend, s.snapshot.clone()) else {
            return;
        };
        let text = s.text.clone();
        match ReadTask::spawn(move |c| b.ignore_preview(&path, &snapshot, &text, &c)) {
            Ok(t) => s.task = Some(t),
            Err(e) => d.error = Some(user_error(&e)),
        }
    }
    pub(crate) fn poll_ignore(&mut self) {
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.ignore else { return };
        if let Some(r) = s.read.as_mut().and_then(ReadTask::try_finish) {
            s.read = None;
            let r = if s.stopping { Err(Error::Cancelled) } else { r };
            s.stopping = false;
            match r {
                Ok(snapshot) => {
                    s.text = snapshot.text().into();
                    s.snapshot = Some(snapshot);
                    let rules = s
                        .pending
                        .drain(..)
                        .map(|p| literal_ignore_rule(&p, false))
                        .collect::<rocket_git::Result<Vec<_>>>();
                    match rules {
                        Ok(r) if !r.is_empty() => match append_ignore_rules(&s.text, &r) {
                            Ok(t) => s.text = t,
                            Err(e) => d.error = Some(user_error(&e)),
                        },
                        Err(e) => d.error = Some(user_error(&e)),
                        _ => (),
                    }
                }
                Err(Error::Cancelled) => {
                    s.notice = Some("Read cancelled. Nothing was saved.".into())
                }
                Err(e) => d.error = Some(user_error(&e)),
            }
        }
        if let Some(r) = s.task.as_mut().and_then(ReadTask::try_finish) {
            s.task = None;
            let r = if s.stopping { Err(Error::Cancelled) } else { r };
            s.stopping = false;
            match r {
                Ok(p) => s.preview = Some(Arc::new(p)),
                Err(Error::Cancelled) => {
                    s.notice = Some("Review cancelled. Nothing was saved.".into())
                }
                Err(e) => d.error = Some(user_error(&e)),
            }
        }
    }
    pub(crate) fn finish_ignore_dialog(&mut self, owner: Option<u64>, outcome: &Outcome) {
        let Some(d) = &mut self.dialog else { return };
        if d.owner != owner {
            return;
        }
        let Some(s) = &mut d.ignore else { return };
        if !s.applying {
            return;
        }
        s.applying = false;
        s.preview = None;
        s.snapshot = None;
        let message = format!("{}\n{}", outcome.summary, outcome.details.join("\n"));
        if outcome.command_succeeded {
            s.notice = Some(message);
            d.error = None
        } else {
            d.error = Some(message);
            s.notice = None
        }
    }
    pub(crate) fn ignore_dialog(&mut self, ctx: &Context) {
        let writable = self.can_write();
        let idle = self.write.is_none();
        let Some(mut d) = self.dialog.take() else {
            return;
        };
        let Some(s) = &mut d.ignore else {
            self.dialog = Some(d);
            return;
        };
        if d.owner != self.tab().map(|t| t.id) && !s.applying {
            return;
        }
        let busy = s.running();
        let (mut close, mut reload, mut review, mut apply, mut cancel, mut changed) =
            (false, false, false, false, false, false);
        let mut rules = Vec::new();
        let mut add_path = false;
        let mut tracked = false;
        let write = self
            .write
            .as_ref()
            .filter(|w| w.owner == d.owner && w.label == "ignore-save");
        let running = pages::Running::of(write);
        let response = pages::wide_review(
            ctx,
            "rocket-ignore",
            pages::ReviewHeader {
                image: Icon::File,
                title: "Ignore files",
                subtitle: ".gitignore at the repository root. Matching new files stop showing in Changes.",
                danger: false,
            },
            &mut self.frame_widgets,
            "dialog-ignore-scroll",
            !s.applying,
            |ui, part, widgets| match part {
                pages::Part::Body => {
                    if let Some(e) = &d.error {
                        pages::callout(ui, RED(), e)
                    }
                    let editable = !busy && s.snapshot.is_some();
                    let panel = 280.0;
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 16.0;
                        let left = (ui.available_width() - panel - 16.0).max(240.0);
                        ui.vertical(|ui| {
                            ui.set_width(left);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Rules").size(12.0).color(MUTED()));
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    let r = pages::ghost(ui, !busy && idle, "Reload from disk")
                                        .on_hover_text("Replace the editor text with the file as it is on disk.");
                                    control(widgets, "dialog-ignore-reload", &r, ui.clip_rect());
                                    reload = r.clicked();
                                });
                            });
                            ui.add_enabled_ui(editable, |ui| {
                                changed |= rules_editor(ui, &mut s.text, s.snapshot.as_ref(), widgets);
                            });
                        });
                        Frame::new()
                            .fill(PANEL())
                            .stroke(Stroke::new(1.0, HAIRLINE()))
                            .corner_radius(10)
                            .inner_margin(Margin::same(12))
                            .show(ui, |ui| {
                                // The frame sits in a row; its contents stack.
                                ui.vertical(|ui| {
                                    ui.set_width(panel - 24.0);
                                    ui.add_enabled_ui(editable, |ui| {
                                        add_rules(ui, s, &mut rules, &mut add_path, &mut d.error, widgets);
                                    });
                                });
                            });
                    });
                    pages::callout(
                        ui,
                        BLUE(),
                        "Once saved, new files that match stop showing in Changes. Files Git already tracks stay tracked: use Stop tracking files for those.",
                    );
                    if let Some(n) = &s.notice {
                        pages::note(ui, n)
                    }
                    if s.read.is_some() || s.task.is_some() {
                        pages::waiting(
                            ui,
                            if s.stopping {
                                "Stopping"
                            } else if s.read.is_some() {
                                "Reading .gitignore"
                            } else {
                                "Checking the rules"
                            },
                        )
                    }
                }
                pages::Part::Footer if s.applying => {
                    cancel = pages::running_footer(ui, widgets, &running, &["dialog-ignore-cancel"]);
                }
                pages::Part::Footer => {
                    // Any difference is an edit: Git reads rules in order.
                    let edited = s.snapshot.as_ref().is_some_and(|v| v.text() != s.text);
                    let summary = match &s.snapshot {
                        Some(v) if edited => match line_changes(v.text(), &s.text) {
                            (0, 0) => "lines reordered or line endings changed".to_owned(),
                            (added, removed) => changes_text(added, removed),
                        },
                        _ => changes_text(0, 0),
                    };
                    if s.preview.is_some() {
                        let r = pages::primary(ui, !busy && writable, "Save .gitignore");
                        control(widgets, "dialog-submit", &r, ui.clip_rect());
                        apply = r.clicked();
                        let r = ui.add_enabled(!busy && idle, Button::new("Review again"));
                        control(widgets, "dialog-ignore-review", &r, ui.clip_rect());
                        review = r.clicked();
                    } else {
                        let r = pages::primary(ui, !busy && idle && s.snapshot.is_some() && edited, "Review");
                        control(widgets, "dialog-ignore-review", &r, ui.clip_rect());
                        review = r.clicked();
                    }
                    if busy {
                        let r = ui.add_enabled(!s.stopping, Button::new("Stop"));
                        control(widgets, "dialog-ignore-cancel", &r, ui.clip_rect());
                        cancel = r.clicked();
                    }
                    let clean_draft = s.snapshot.as_ref().is_some_and(|v| v.text() == s.text);
                    let r = pages::ghost(ui, !busy && idle && clean_draft, "Stop tracking files")
                        .on_hover_text("Stop tracking files Git already has, without deleting them. Save or reload edited rules first.");
                    control(widgets, "dialog-ignore-tracked", &r, ui.clip_rect());
                    tracked = r.clicked();
                    let status = if busy {
                        "Checking".to_owned()
                    } else if s.preview.is_some() {
                        format!("Reviewed · {summary}")
                    } else if edited {
                        format!("{summary} · not saved yet")
                    } else {
                        "No changes".to_owned()
                    };
                    pages::footer_status(ui, false, &status);
                }
            },
        );
        close |= response.should_close() && !s.applying;
        if add_path {
            match RepoPath::new(
                if s.folder {
                    s.path.trim_end_matches('/')
                } else {
                    &s.path
                }
                .as_bytes()
                .to_vec(),
            )
            .and_then(|p| literal_ignore_rule(&p, s.folder))
            {
                Ok(r) => rules.push(r),
                Err(e) => d.error = Some(user_error(&e)),
            }
        }
        if !rules.is_empty() {
            match append_ignore_rules(&s.text, &rules) {
                Ok(t) => {
                    s.text = t;
                    changed = true
                }
                Err(e) => d.error = Some(user_error(&e)),
            }
        }
        if changed {
            s.preview = None;
            s.notice = None;
        }
        if cancel {
            if s.applying {
                self.cancel_write()
            } else {
                if let Some(t) = &s.read {
                    t.cancel()
                }
                if let Some(t) = &s.task {
                    t.cancel()
                }
                s.stopping = true;
                s.preview = None;
            }
        }
        let action = if apply {
            s.applying = true;
            d.error = None;
            s.notice = None;
            s.preview.clone().map(Action::Ignore)
        } else {
            None
        };
        let owner = d.owner;
        if !close && !tracked {
            self.dialog = Some(d)
        }
        if tracked {
            self.dispatch(Intent::KeepLocal{snapshot:false,paths:Vec::new()});
        }
        if reload {
            self.load_ignore()
        }
        if review {
            self.start_ignore_review()
        }
        if let (Some(a), Some(owner)) = (action, owner) {
            self.dispatch(Intent::Execute(owner, a));
            if self.write.is_none()
                && let Some(d) = &mut self.dialog
                && let Some(s) = &mut d.ignore
            {
                s.applying = false;
                d.error = Some("Could not start; review again when repository tasks finish.".into())
            }
        }
    }
}

/// Lines `after` adds and removes against `before`, whatever their order. Counted
/// per line, so this stays linear for a file up to 1 MiB; a repeated line matches
/// once per copy.
fn line_changes(before: &str, after: &str) -> (usize, usize) {
    let mut lines: HashMap<&str, usize> = HashMap::new();
    for line in before.lines() {
        *lines.entry(line).or_default() += 1;
    }
    let mut added = 0;
    for line in after.lines() {
        match lines.get_mut(line) {
            Some(n) if *n > 0 => *n -= 1,
            _ => added += 1,
        }
    }
    (added, lines.values().sum())
}

fn changes_text(added: usize, removed: usize) -> String {
    let lines = |n: usize| if n == 1 { "1 line".to_owned() } else { format!("{n} lines") };
    match (added, removed) {
        (0, 0) => "no changes".into(),
        (a, 0) => format!("{} added", lines(a)),
        (0, r) => format!("{} removed", lines(r)),
        (a, r) => format!("{} added, {} removed", lines(a), lines(r)),
    }
}

/// The rules as an editor: line numbers in its margin and lines the file doesn't
/// have yet in green. Returns whether the text changed.
fn rules_editor(
    ui: &mut Ui,
    text: &mut String,
    snapshot: Option<&IgnoreSnapshot>,
    widgets: &mut HashMap<String, UiControl>,
) -> bool {
    let known: HashSet<&str> = snapshot.map(|s| s.text().lines().collect()).unwrap_or_default();
    let mut layouter = |ui: &Ui, buffer: &dyn TextBuffer, _wrap: f32| {
        let mut job = text::LayoutJob::default();
        for line in buffer.as_str().split_inclusive('\n') {
            let content = line.trim_end_matches(['\n', '\r']);
            let new = !content.trim().is_empty() && !known.contains(content);
            let mut format = TextFormat::simple(
                FontId::monospace(12.5),
                if new {
                    GREEN()
                } else if content.trim_start().starts_with('#') {
                    HINT()
                } else {
                    TEXT()
                },
            );
            if new {
                format.background = GREEN().gamma_multiply(0.10);
            }
            job.append(line, 0.0, format);
        }
        job.wrap.max_width = f32::INFINITY;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let mut changed = false;
    let scroll = ScrollArea::both()
        .id_salt("dialog-ignore-editor")
        .max_height(300.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let output = TextEdit::multiline(text)
                .layouter(&mut layouter)
                .margin(Margin {
                    left: 44,
                    right: 8,
                    top: 6,
                    bottom: 6,
                })
                .desired_rows(13)
                .desired_width(f32::INFINITY)
                .show(ui);
            control(widgets, "dialog-ignore-text", &output.response.response, ui.clip_rect());
            changed = output.response.response.changed();
            // Line numbers beside each line, in the editor's left margin.
            let painter = ui.painter().with_clip_rect(output.text_clip_rect.expand2(vec2(44.0, 0.0)));
            let mut number = 0;
            let mut starts = true;
            for row in &output.galley.rows {
                if starts {
                    number += 1;
                    let y = output.galley_pos.y + row.rect().center().y;
                    painter.text(
                        pos2(output.galley_pos.x - 12.0, y),
                        Align2::RIGHT_CENTER,
                        number.to_string(),
                        FontId::monospace(11.5),
                        FAINT(),
                    );
                }
                starts = row.ends_with_newline;
            }
        });
    pages::region(
        widgets,
        "dialog-ignore-editor-scroll",
        scroll.inner_rect.intersect(ui.clip_rect()),
        true,
    );
    changed
}

/// The Add rules panel: untracked files, an exact path, or a starter template.
fn add_rules(
    ui: &mut Ui,
    s: &mut DialogState,
    rules: &mut Vec<String>,
    add_path: &mut bool,
    error: &mut Option<String>,
    widgets: &mut HashMap<String, UiControl>,
) {
    ui.label(RichText::new("Add rules").font(semibold(13.0)));
    let items = [("New files", s.tab == 0), ("Path", s.tab == 1), ("Template", s.tab == 2)];
    for (i, r) in pages::segments(ui, &items).iter().enumerate() {
        control(widgets, format!("dialog-ignore-tab-{i}"), r, ui.clip_rect());
        if r.clicked() {
            s.tab = i;
        }
    }
    ui.add_space(2.0);
    match s.tab {
        0 => {
            if s.candidates.is_empty() {
                pages::note(ui, "No new files: everything here is tracked or already ignored.");
                return;
            }
            pages::note(ui, "Files Git isn't tracking yet");
            let list = ScrollArea::vertical()
                .id_salt("dialog-ignore-files")
                .max_height(190.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for (i, path) in s.candidates.iter().enumerate().take(100) {
                        let r = ui.add(checkbox(&mut s.selected[i], path.display_lossy()));
                        control(widgets, format!("dialog-ignore-file-{i}"), &r, ui.clip_rect());
                    }
                    if s.candidates.len() > 100 {
                        pages::note(ui, "Showing the first 100. Use Path for another file or folder.");
                    }
                });
            pages::region(widgets, "dialog-ignore-files-scroll", list.inner_rect.intersect(ui.clip_rect()), true);
            let count = s.selected.iter().filter(|on| **on).count();
            let label = match count {
                0 => "Add rules".to_owned(),
                1 => "Add 1 rule".to_owned(),
                n => format!("Add {n} rules"),
            };
            let r = ui.add_enabled(count > 0, Button::new(label).min_size(vec2(ui.available_width(), 28.0)));
            control(widgets, "dialog-ignore-selected", &r, ui.clip_rect());
            if r.clicked() {
                for (path, selected) in s.candidates.iter().zip(s.selected.iter_mut()) {
                    if *selected {
                        match literal_ignore_rule(path, false) {
                            Ok(rule) => rules.push(rule),
                            Err(e) => *error = Some(user_error(&e)),
                        }
                        *selected = false;
                    }
                }
            }
        }
        1 => {
            ui.label(RichText::new("Path in the repository").size(12.0).color(MUTED()));
            let r = ui.add(
                TextEdit::singleline(&mut s.path)
                    .hint_text(RichText::new("logs/session.log").color(FAINT()))
                    .desired_width(f32::INFINITY)
                    .min_size(vec2(0.0, 30.0))
                    .align(Align2::LEFT_CENTER),
            );
            control(widgets, "dialog-ignore-path", &r, ui.clip_rect());
            let r = ui.add(checkbox(&mut s.folder, "A folder, with what's inside"));
            control(widgets, "dialog-ignore-folder", &r, ui.clip_rect());
            let r = ui.add_enabled(
                !s.path.trim().is_empty(),
                Button::new("Add rule").min_size(vec2(ui.available_width(), 28.0)),
            );
            control(widgets, "dialog-ignore-add", &r, ui.clip_rect());
            *add_path = r.clicked();
        }
        _ => {
            pages::note(ui, "Starter rules, added to the end of the file");
            for (i, template) in [IgnoreTemplate::Rust, IgnoreTemplate::Node, IgnoreTemplate::Python, IgnoreTemplate::DotNet]
                .iter()
                .enumerate()
            {
                let preview = template
                    .rules()
                    .iter()
                    .filter(|rule| !rule.starts_with('#'))
                    .copied()
                    .collect::<Vec<_>>()
                    .join("  ");
                let r = pages::list_row(ui, Icon::File, template.name(), &preview, false);
                control(widgets, format!("dialog-ignore-template-{i}"), &r, ui.clip_rect());
                if r.clicked() {
                    rules.extend(template.rules().iter().map(|rule| rule.to_string()));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::line_changes;
    use crate::{
        app::{DialogKind, Intent},
        batch::flow_tests::{enabled, open, wait},
        test_common as common,
    };
    use eframe::egui::Context;

    #[test]
    fn line_changes_count_each_copy_whatever_the_order() {
        assert_eq!(line_changes("a\nb\n", "a\nb\n"), (0, 0));
        assert_eq!(line_changes("*.log\n!keep.log\n", "!keep.log\n*.log\n"), (0, 0));
        assert_eq!(line_changes("a\n", "a\na\n"), (1, 0));
        assert_eq!(line_changes("a\na\nb\n", "a\nc\n"), (1, 2));
        assert_eq!(line_changes("", "*.log\n"), (1, 0));
    }

    /// Reordered rules are an edit to review and save: Git reads them in order, so
    /// `!keep.log` only keeps the file when it comes after `*.log`.
    #[test]
    fn reordered_rules_can_be_reviewed() {
        common::isolated(
            "ROCKET_IGNORE_UI_CASE",
            "ignore::tests::reordered_rules_can_be_reviewed",
            || {
                let f = common::Fixture::new();
                f.write(".gitignore", "!keep.log\n*.log\n");
                f.commit("rules");
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                app.dispatch(Intent::Dialog(DialogKind::Ignore(Vec::new())));
                wait(&ctx, &mut app);
                assert!(!enabled(&ctx, &mut app, "dialog-ignore-review"), "nothing to review yet");
                app.dialog.as_mut().unwrap().ignore.as_mut().unwrap().text = "*.log\n!keep.log\n".into();
                assert!(enabled(&ctx, &mut app, "dialog-ignore-review"));
            },
        );
    }
}
