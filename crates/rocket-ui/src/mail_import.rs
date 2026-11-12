//! Commit series imports with their Continue, Skip and Abort. Reviews and recovery stay
//! on cancellable workers.
use crate::{
    app::*,
    backend::{Action, Outcome, user_error},
    pages::{self, Step, StepMark},
    patch_diff::{self, Shown},
    series_view,
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use std::{collections::HashMap, path::PathBuf, sync::Arc};

/// A review and what its dialog draws, worked out on the worker.
pub struct Reviewed {
    preview: MailImportPreview,
    shown: Vec<Vec<Shown>>,
    fix: Vec<Shown>,
}

#[derive(Default)]
pub struct DialogState {
    pub preview: Option<Arc<MailImportPreview>>,
    pub task: Option<ReadTask<Reviewed>>,
    pub applying: bool,
    pub stopping: bool,
    pub selected: usize,
    /// Each patch's files as shown.
    shown: Vec<Vec<Shown>>,
    /// Continue: the staged fix's files as shown. Skip and Abort: the edits that go.
    fix: Vec<Shown>,
    /// "Your edits go too" is open.
    edits: bool,
}

/// "1 commit", "4 commits".
fn commits(n: usize) -> String {
    crate::i18n::trn!(n, "{n} commit", "{n} commits")
}

/// What follows the stopped patch, for "then imports …".
fn the_rest(rest: usize) -> String {
    match rest {
        0 => crate::i18n::tr("that's the last one").into(),
        1 => crate::i18n::tr("then imports the last patch").into(),
        n => crate::i18n::trf!("then imports the remaining patches: {count}", count = n),
    }
}

impl Rocket {
    pub(crate) fn start_mail_preview(&mut self) {
        let Some(d) = &mut self.dialog else { return };
        let DialogKind::MailImport(action) = d.kind else {
            return;
        };
        let s = d.mail_import.get_or_insert_with(Default::default);
        if s.applying {
            return;
        }
        s.task = None;
        s.preview = None;
        s.stopping = false;
        d.error = None;
        if action == MailImportAction::Start && d.first.trim().is_empty() {
            return;
        }
        let Some(path) = self
            .tabs
            .iter()
            .find(|t| Some(t.id) == d.owner)
            .map(|t| t.path.clone())
        else {
            return;
        };
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let folder = PathBuf::from(d.first.trim());
        match ReadTask::spawn(move |c| {
            let preview = backend.mail_import_preview(&path, action, &folder, &c)?;
            // The review's bounded text, so invisible characters stay visible.
            let shown = preview
                .entries()
                .iter()
                .map(|e| {
                    let mut files = patch_diff::files(e.display.patch.as_bytes());
                    for file in &mut files {
                        file.label = visible_path(&file.label);
                    }
                    files
                })
                .collect();
            let fix = patch_diff::files(preview.staged());
            Ok(Reviewed { preview, shown, fix })
        }) {
            Ok(t) => s.task = Some(t),
            Err(e) => d.error = Some(user_error(&e)),
        }
    }
    pub(crate) fn poll_mail_preview(&mut self) {
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.mail_import else { return };
        let Some(result) = s.task.as_mut().and_then(ReadTask::try_finish) else {
            return;
        };
        s.task = None;
        s.stopping = false;
        match result {
            Ok(r) => {
                // A paused import opens on the patch it stopped at.
                s.selected = r.preview.current().saturating_sub(1);
                s.preview = Some(Arc::new(r.preview));
                s.shown = r.shown;
                s.fix = r.fix;
            }
            Err(Error::Cancelled) => {
                d.error = Some(crate::i18n::tr("Import review stopped. No commits were imported.").into())
            }
            Err(e) => d.error = Some(user_error(&e)),
        }
    }
    pub(crate) fn finish_mail_dialog(&mut self, owner: Option<u64>, out: &Outcome) {
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.mail_import else { return };
        if !s.applying || d.owner != owner {
            return;
        }
        if out.command_succeeded || out.recovery_pending {
            self.dialog = None;
            if let Some(t) = self.tabs.iter_mut().find(|t| Some(t.id) == owner) {
                t.after_sequence(out.recovery_pending);
            }
        } else {
            s.applying = false;
            s.preview = None;
            s.stopping = false;
            d.error = Some(format!("{}\n{}", out.summary, out.details.join("\n")));
        }
    }
    pub(crate) fn mail_import_dialog(&mut self, ctx: &Context) {
        let ready = self.can_write();
        let Some(mut d) = self.dialog.take() else {
            return;
        };
        let DialogKind::MailImport(action) = d.kind else {
            self.dialog = Some(d);
            return;
        };
        let tab = self.tabs.iter().find(|t| Some(t.id) == d.owner);
        let snapshot = tab.and_then(|t| t.snapshot.as_ref());
        let branch = head_name(snapshot.and_then(|s| s.head.as_ref()));
        let stopped = snapshot.and_then(|s| s.state.as_ref()).and_then(|s| s.step.clone());
        let s = d.mail_import.get_or_insert_with(Default::default);
        let original = s.preview.as_ref().map(|p| {
            let id = p.original();
            let commit = tab.and_then(|t| t.loaded_commits().iter().find(|c| c.id == *id));
            (commit.map_or_else(|| crate::i18n::tr("Your branch before the import").to_owned(), subject), id.as_str()[..7].to_owned())
        });
        let at = s.preview.as_ref().map(|p| p.current()).or(stopped.as_ref().map(|t| t.at));
        let total = s.preview.as_ref().map(|p| p.entries().len()).or(stopped.as_ref().map(|t| t.total)).unwrap_or(0);
        let rest = total.saturating_sub(at.unwrap_or(0));
        let patch = at.map_or_else(|| crate::i18n::tr("patch").to_owned(), |n| crate::i18n::trf!("patch {number}", number = n));
        let name = s
            .preview
            .as_ref()
            .and_then(|p| p.entries().get(p.current().saturating_sub(1)))
            .map(|e| e.subject.clone())
            .or(stopped.as_ref().map(|t| t.subject.clone()))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| crate::i18n::tr("this patch").into());
        let title = match action {
            MailImportAction::Start => crate::i18n::tr("Import commit series").to_owned(),
            MailImportAction::Continue => crate::i18n::tr("Continue import").into(),
            MailImportAction::Skip => crate::i18n::trf!("Skip {patch}", patch),
            MailImportAction::Abort => crate::i18n::tr("Abort import").into(),
        };
        let subtitle = match action {
            MailImportAction::Start => {
                crate::i18n::trf!("Makes each patch in a folder into a commit on {branch}. Authors, dates and messages are kept.", branch)
            }
            MailImportAction::Continue if s.preview.is_some() => crate::i18n::trf!("Saves your fix as {patch}, {next}.", patch, next = the_rest(rest)),
            MailImportAction::Continue => format!("{}{name}", at.map_or_else(String::new, |n| crate::i18n::trf!("Patch {number} of {total} · ", number = n, total))),
            MailImportAction::Skip => crate::i18n::trf!("Leaves out {name}, {next}.", name, next = the_rest(rest)),
            MailImportAction::Abort => crate::i18n::trf!("Puts {branch} back where it was before the import.", branch),
        };
        let danger = matches!(action, MailImportAction::Skip | MailImportAction::Abort);
        let applying = s.applying;
        let split = action == MailImportAction::Start && s.preview.is_some();
        let height = (ctx.content_rect().height() - 560.0).clamp(200.0, 300.0);
        let (mut review, mut submit, mut close, mut cancel, mut browse) = (false, false, false, false, false);
        let mut acknowledgement = None;
        let response = pages::review_at(
            ctx,
            "rocket-mail-import",
            if split { 980.0 } else { 640.0 },
            pages::ReviewHeader {
                image: match action {
                    MailImportAction::Skip => Icon::ChevronRight,
                    MailImportAction::Abort => Icon::Refresh,
                    _ => Icon::Commit,
                },
                title: &title,
                subtitle: &subtitle,
                danger,
            },
            &mut self.frame_widgets,
            "dialog-series-scroll",
            !applying,
            |ui, part, widgets| match part {
                pages::Part::Body => {
                    ui.spacing_mut().item_spacing.y = 12.0;
                    if let Some(error) = &d.error {
                        pages::refusal(ui, AMBER(), error);
                    }
                    if action == MailImportAction::Start {
                        folder_row(ui, s, &mut d.first, &mut d.error, widgets, &mut review, &mut browse);
                    }
                    if s.task.is_some() {
                        pages::waiting(
                            ui,
                            if s.stopping {
                                crate::i18n::tr("Stopping review…")
                            } else {
                                crate::i18n::tr("Reading patches and checking local changes…")
                            },
                        );
                    }
                    if s.applying {
                        pages::waiting(
                            ui,
                            if s.stopping {
                                crate::i18n::tr("Stopping… Checking the resulting import state.")
                            } else {
                                crate::i18n::tr("Applying commits and recording recovery state…")
                            },
                        );
                    }
                    if let Some(p) = s.preview.clone() {
                        if p.requires_truncation_acknowledgement() {
                            pages::callout(ui, AMBER(), crate::i18n::tr("This preview leaves out part of these patches. Import uses the whole patches."));
                            let mut accepted = p.truncation_acknowledged();
                            let r = ui.add_enabled(!s.applying, Checkbox::new(&mut accepted,
                                crate::i18n::tr("I understand the full patches include text not shown here")));
                            pages::control(widgets, "dialog-series-acknowledge", &r, ui.clip_rect());
                            if r.changed() {
                                acknowledgement = Some(accepted);
                            }
                        }
                        match action {
                            MailImportAction::Start => patches_list(ui, s, &p, widgets, height),
                            MailImportAction::Continue => {
                                pages::steps(ui, "dialog-series", &steps(&p, StepMark::Stopped, crate::i18n::tr("your fix")));
                                if !s.fix.is_empty() {
                                    let files = crate::i18n::trf!("Staged files: {count}", count = s.fix.len());
                                    series_view::diff_card(ui, &crate::i18n::trf!("Your fix · {files}", files), "dialog-series-fix", &s.fix, 160.0);
                                }
                            }
                            MailImportAction::Skip => {
                                pages::steps(ui, "dialog-series", &steps(&p, StepMark::LeftOut, crate::i18n::tr("left out")));
                                if let Some(files) = s.shown.get(p.current().saturating_sub(1)) {
                                    series_view::diff_card(ui, &crate::i18n::trf!("What {patch} would have changed", patch), "dialog-series-left-out", files, 160.0);
                                }
                                if !s.fix.is_empty() {
                                    s.edits = series_view::edits_section(ui, "dialog-series-edits", &s.fix, s.edits, widgets);
                                    pages::note(ui, crate::i18n::tr("Rocket keeps a copy of your edits first and names the folder in Activity."));
                                }
                            }
                            MailImportAction::Abort => {
                                ui.label(RichText::new(crate::i18n::trf!("{branch} goes back to", branch)).size(12.0).color(HINT()));
                                if let Some((subject, id)) = &original {
                                    pages::commit_card(ui, subject, id, crate::i18n::tr("before the import"));
                                }
                                pages::steps(ui, "dialog-series", &abort_steps(&p));
                                if !s.fix.is_empty() {
                                    s.edits = series_view::edits_section(ui, "dialog-series-edits", &s.fix, s.edits, widgets);
                                }
                                let mut note = crate::i18n::tr("Your patch files stay as they are, so you can import them again.").to_owned();
                                if !p.imported().is_empty() {
                                    note.push_str(crate::i18n::tr(" Reflog & Recovery can still find what comes off."));
                                }
                                if !s.fix.is_empty() {
                                    note.push_str(crate::i18n::tr(" Rocket keeps a copy of your edits and names the folder in Activity."));
                                }
                                pages::note(ui, &note);
                            }
                        }
                    }
                }
                pages::Part::Footer => {
                    if s.applying {
                        let r = ui.add_enabled(!s.stopping, Button::new(crate::i18n::tr("Stop")));
                        pages::control(widgets, "cancel-operation", &r, ui.clip_rect());
                        if r.clicked() {
                            s.stopping = true;
                            cancel = true;
                        }
                        return;
                    }
                    let enabled =
                        ready && s.task.is_none() && s.preview.as_ref().is_some_and(|p|
                            !p.requires_truncation_acknowledgement() || p.truncation_acknowledged()) && d.error.is_none();
                    let count = s.preview.as_ref().map_or(0, |p| p.entries().len());
                    let label = match action {
                        MailImportAction::Start if count > 0 => crate::i18n::trf!("Import {commits}", commits = commits(count)),
                        MailImportAction::Start => crate::i18n::tr("Import commits").to_owned(),
                        _ => title.clone(),
                    };
                    let r = if danger {
                        pages::danger(ui, enabled, &label)
                    } else {
                        pages::primary(ui, enabled, &label)
                    };
                    pages::control(widgets, "dialog-submit", &r, ui.clip_rect());
                    submit = r.clicked();
                    // A paused import reviews itself; asking again is for a refusal.
                    let start = action == MailImportAction::Start;
                    if (start && !d.first.trim().is_empty()) || d.error.is_some() {
                        let can = ready && s.task.is_none();
                        let r = if start && s.preview.is_none() && d.error.is_none() {
                            pages::primary(ui, can, crate::i18n::tr("Review"))
                        } else {
                            ui.add_enabled(can, Button::new(crate::i18n::tr("Review again")))
                        };
                        pages::control(widgets, "dialog-preview-refresh", &r, ui.clip_rect());
                        review |= r.clicked();
                    }
                    if s.task.is_some() {
                        let r = ui.add_enabled(!s.stopping, Button::new(crate::i18n::tr("Stop review")));
                        pages::control(widgets, "dialog-series-stop", &r, ui.clip_rect());
                        if r.clicked() {
                            if let Some(t) = &s.task {
                                t.cancel();
                            }
                            s.stopping = true;
                        }
                    }
                    let status = if s.task.is_some() {
                        crate::i18n::tr("Reviewing").to_owned()
                    } else if d.error.is_some() {
                        crate::i18n::tr("Fix the problem above, then review again").to_owned()
                    } else if !start {
                        crate::i18n::tr("Nothing has changed yet").to_owned()
                    } else if count > 0 {
                        crate::i18n::trf!("Reviewed · adds {commits} to {branch} · nothing is pushed", commits = commits(count), branch)
                    } else if d.first.trim().is_empty() {
                        crate::i18n::tr("Choose a folder to see what it adds").to_owned()
                    } else {
                        crate::i18n::tr("Review to see what it adds").to_owned()
                    };
                    pages::footer_status(ui, false, &status);
                }
            },
        );
        if let Some(accepted) = acknowledgement
            && let Some(preview) = &mut s.preview
        {
            Arc::make_mut(preview).acknowledge_truncation(accepted);
        }
        close |= response.should_close() && !applying;
        let pending = if submit {
            d.owner.zip(s.preview.clone())
        } else {
            None
        };
        if pending.is_some() {
            s.applying = true;
        }
        let folder = d.first.clone();
        self.dialog = (!close).then_some(d);
        if cancel {
            self.cancel_write();
        }
        if review {
            self.start_mail_preview();
        }
        if browse {
            self.browse(crate::folder_browser::PickTarget::SeriesFolder, &folder);
        }
        if let Some((owner, p)) = pending {
            self.execute(owner, Action::MailImport(p));
        }
    }
}

/// The folder of patches, and what its review found.
fn folder_row(
    ui: &mut Ui,
    s: &mut DialogState,
    first: &mut String,
    error: &mut Option<String>,
    widgets: &mut HashMap<String, UiControl>,
    review: &mut bool,
    browse: &mut bool,
) {
    crate::patches::form_row(ui, crate::i18n::tr("Folder"), |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            // The button is measured first and the field takes what it leaves: Browse is
            // a longer word in some languages.
            let label = crate::i18n::tr("Browse");
            let button = pages::button_width(ui, label, 0.0);
            let r = ui.add_enabled(
                !s.applying,
                TextEdit::singleline(first)
                    .hint_text(crate::i18n::tr("A folder of numbered .patch files"))
                    .desired_width(ui.available_width() - button - 8.0)
                    .min_size(vec2(0.0, 30.0))
                    .vertical_align(Align::Center),
            );
            pages::control(widgets, "dialog-series-folder", &r, ui.clip_rect());
            if r.changed() {
                s.task = None;
                s.preview = None;
                *error = None;
            }
            *review |= r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && !first.trim().is_empty();
            let b = ui.add_enabled(!s.applying, Button::new(label).min_size(vec2(button, 30.0)));
            pages::control(widgets, "dialog-series-browse", &b, ui.clip_rect());
            *browse = b.clicked();
        });
        match &s.preview {
            Some(p) => {
                let n = p.entries().len();
                let bytes = p.entries().iter().map(|e| e.size()).sum();
                let patches = crate::i18n::trf!("Patches: {count}", count = n);
                crate::tags::check(ui, true, &crate::i18n::trf!("{patches} · {size} · apply in this order", patches, size = crate::patches::size(bytes)));
            }
            None if s.task.is_none() && error.is_none() => {
                pages::note(ui, crate::i18n::tr("Export commit series makes this kind of folder."));
            }
            None => {}
        }
    });
}

/// The patches a new import will apply, beside the chosen one's changes.
fn patches_list(
    ui: &mut Ui,
    s: &mut DialogState,
    p: &MailImportPreview,
    widgets: &mut HashMap<String, UiControl>,
    height: f32,
) {
    let rows: Vec<series_view::Row> = p
        .entries()
        .iter()
        .map(|e| {
            let date = series_view::short_date(&e.date);
            series_view::Row {
                subject: e.subject.clone(),
                meta: if e.source.is_empty() { format!("{} · {date}", e.author) } else { format!("{} · {} · {date}", e.source, e.author) },
                added: e.display.added,
                removed: e.display.removed,
            }
        })
        .collect();
    let title = crate::i18n::trf!("Patches: {count}, oldest first", count = rows.len());
    let files = s.shown.get(s.selected).map(Vec::as_slice).unwrap_or_default();
    series_view::split(
        ui,
        &title,
        &rows,
        &mut s.selected,
        files,
        series_view::Keys { row: "dialog-series-entry", scroll: "dialog-series-list" },
        widgets,
        !s.applying,
        height,
    );
    if let Some(e) = p.entries().get(s.selected) {
        if e.display.limited {
            pages::note(ui, &left_out(e, s.shown.get(s.selected).map(Vec::as_slice).unwrap_or_default()));
        }
        if e.display.escaped {
            pages::note(ui, crate::i18n::tr("Invisible characters and backslashes show as visible escapes. The patch itself is unchanged."));
        }
    }
}

/// What a shortened preview leaves out of a patch: its lines, and the files that never
/// reached the diff.
fn left_out(e: &MailImportEntry, shown: &[Shown]) -> String {
    let display = &e.display;
    let mut text = crate::i18n::trf!("Shows {shown} of {total} lines in this patch.", shown = display.shown_lines, total = display.total_lines);
    let missing: Vec<&str> = display
        .paths
        .iter()
        .map(String::as_str)
        .filter(|path| !shown.iter().any(|f| f.label == *path))
        .collect();
    let beyond = e.paths.len() - display.paths.len();
    if !missing.is_empty() {
        text.push_str(&crate::i18n::trf!(" It also changes {files}", files = missing.iter().take(6).copied().collect::<Vec<_>>().join(", ")));
    }
    let more = missing.len().saturating_sub(6) + beyond;
    match (more, missing.is_empty()) {
        (0, false) => text.push('.'),
        (0, true) => {}
        (n, false) => text.push_str(&crate::i18n::trf!(" and {count} more.", count = n)),
        (n, true) => text.push_str(&crate::i18n::trf!(" Additional changed files: {count}.", count = n)),
    }
    text
}

/// Every patch of a paused import: the ones before it done or skipped, then `stopped`
/// with its note, then the ones still to come.
fn steps(p: &MailImportPreview, stopped: StepMark, note: &'static str) -> Vec<Step> {
    let at = p.current();
    let imported = p.imported();
    let none_skipped = imported.len() + 1 == at;
    p.entries()
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let number = i + 1;
            let (mark, note) = match number.cmp(&at) {
                std::cmp::Ordering::Less if none_skipped || imported.contains(&e.subject) => (StepMark::Done, crate::i18n::tr("done")),
                std::cmp::Ordering::Less => (StepMark::Waiting, crate::i18n::tr("skipped")),
                std::cmp::Ordering::Equal => (stopped, note),
                std::cmp::Ordering::Greater => (StepMark::Waiting, ""),
            };
            Step { mark, number, subject: e.subject.clone(), note }
        })
        .collect()
}

/// What Abort does to every patch: an imported one comes off, a skipped one is already
/// gone, and from the stopped one on nothing arrives.
fn abort_steps(p: &MailImportPreview) -> Vec<Step> {
    let at = p.current();
    let imported = p.imported();
    let none_skipped = imported.len() + 1 == at;
    p.entries()
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let number = i + 1;
            let (mark, note) = match number.cmp(&at) {
                std::cmp::Ordering::Less if none_skipped || imported.contains(&e.subject) => (StepMark::LeftOut, crate::i18n::tr("comes off")),
                std::cmp::Ordering::Less => (StepMark::Waiting, crate::i18n::tr("skipped")),
                std::cmp::Ordering::Equal => (StepMark::Stopped, crate::i18n::tr("not imported")),
                std::cmp::Ordering::Greater => (StepMark::Waiting, crate::i18n::tr("not imported")),
            };
            Step { mark, number, subject: e.subject.clone(), note }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_common::{Fixture, isolated};
    use std::{fs, time::{Duration, Instant}};

    fn frame(ctx: &Context, app: &mut Rocket, size: Vec2, events: Vec<Event>) {
        app.frame_widgets.clear();
        let mut output = ctx.run_ui(RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            events,
            ..Default::default()
        }, |ui| app.dialog_ui(ui.ctx()));
        output.textures_delta.clear();
    }

    fn click(ctx: &Context, app: &mut Rocket, size: Vec2, key: &str) {
        let pos = app.frame_widgets[key].rect.center();
        let button = |pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        frame(ctx, app, size, vec![Event::PointerMoved(pos), button(true)]);
        frame(ctx, app, size, vec![button(false)]);
        frame(ctx, app, size, vec![]);
    }

    fn settle(ctx: &Context, app: &mut Rocket, phase: &str, logs: &std::path::Path) {
        let started = Instant::now();
        let mut transitions = Vec::new();
        let mut previous = String::new();
        while app.backend.is_none() || app.busy() || app.tab().is_none_or(|t| t.snapshot.is_none())
            || app.dialog.as_ref().and_then(|d| d.mail_import.as_ref()).is_some_and(|s| s.task.is_some()) {
            app.poll(ctx);
            let state = format!("backend={} busy={} saving={} tab={:?} mail={} startup_error={:?} state_error={:?}",
                app.backend.is_some(), app.busy(), app.state_pending(),
                app.tab().map(|t| (t.snapshot.is_some(), t.refresh.is_some(), t.more.is_some(), t.diff.is_some(), t.refresh_count, t.refresh_pending.is_some())),
                app.dialog.as_ref().and_then(|d| d.mail_import.as_ref()).is_some_and(|s| s.task.is_some()),
                app.startup_error, app.state_error);
            if state != previous {
                transitions.push(format!("{}ms: {state}", started.elapsed().as_millis()));
                previous = state;
            }
            if started.elapsed() >= Duration::from_secs(30) || app.startup_error.is_some() || app.state_error.is_some() {
                let log = fs::read_to_string(logs.join("rocket.jsonl")).unwrap_or_default();
                let tail = log.lines().rev().take(40).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
                panic!("mail review did not settle; phase={phase}; transitions:\n{}\nlast Git events:\n{tail}", transitions.join("\n"));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// A file name Git quotes reads as itself, and a shortened preview says what's missing.
    #[test]
    fn a_review_reads_quoted_names_and_says_what_it_leaves_out() {
        isolated("ROCKET_MAIL_UI_CASE", "mail_import::tests::a_review_reads_quoted_names_and_says_what_it_leaves_out", || {
            let f = Fixture::new();
            let c = CancellationToken::new();
            f.write("keep.txt", "base\n");
            f.commit("Base");
            let base = String::from_utf8(f.run(&["rev-parse", "HEAD"])).unwrap().trim().to_owned();
            f.write("資料.txt", "quoted\n");
            f.commit("Quoted name");
            f.write("a-long.txt", "line\n".repeat(400));
            f.write("z-after.txt", "after\n");
            f.commit("Long patch");
            let folder = f.root.canonicalize().unwrap().join("mail");
            let export = f.repo().preview_patch_series(&base, "HEAD", &folder, &c).unwrap();
            f.repo().export_patch_series(&export, &c).unwrap();
            f.run(&["reset", "--hard", &base]);
            let preview = f.repo().preview_mail_import(&folder, &c).unwrap();
            let [quoted, long] = preview.entries() else { panic!("two patches") };
            assert_eq!(patch_diff::files(quoted.display.patch.as_bytes())[0].label, "資料.txt");
            assert!(!quoted.display.escaped, "Git's quoting isn't an invisible character");
            assert_eq!((long.display.added, long.display.removed), (401, 0), "the whole patch, not its preview");
            let shown = patch_diff::files(long.display.patch.as_bytes());
            assert!(long.display.limited);
            assert_eq!(left_out(long, &shown), format!("Shows {} of {} lines in this patch. It also changes z-after.txt.", long.display.shown_lines, long.display.total_lines));
        });
    }

    /// A paused import opens on the patch it stopped at, not on the first one, and
    /// a dialog that opens too early says what's wrong without its code.
    #[test]
    fn a_paused_import_reviews_the_patch_it_stopped_at() {
        isolated("ROCKET_MAIL_UI_CASE", "mail_import::tests::a_paused_import_reviews_the_patch_it_stopped_at", || {
            let f = Fixture::new();
            let c = CancellationToken::new();
            f.write("first.txt", "base\n");
            f.write("second.txt", "base\n");
            f.commit("Base");
            let base = String::from_utf8(f.run(&["rev-parse", "HEAD"])).unwrap().trim().to_owned();
            f.write("first.txt", "patch one\n");
            f.commit("Patch one");
            f.write("second.txt", "patch two\n");
            f.commit("Patch two");
            let folder = f.root.canonicalize().unwrap().join("mail");
            let export = f.repo().preview_patch_series(&base, "HEAD", &folder, &c).unwrap();
            f.repo().export_patch_series(&export, &c).unwrap();
            f.run(&["reset", "--hard", &base]);
            f.write("second.txt", "local\n");
            f.commit("Local change");
            let review = f.repo().preview_mail_import(&folder, &c).unwrap();
            f.repo().apply_mail_import(&review, &c).unwrap();
            let ctx = Context::default();
            let (store, mut saved) = crate::persistence::Store::open(&f.root.join("state")).unwrap();
            saved.preferences.dependency_warnings = false;
            store.save(&saved).unwrap();
            drop(store);
            let mut app = Rocket::with_storage(&ctx, vec![f.repository.clone()], f.root.join("logs"), f.root.join("state"));
            settle(&ctx, &mut app, "startup", &f.root.join("logs"));
            app.dispatch(Intent::Dialog(DialogKind::MailImport(MailImportAction::Skip)));
            settle(&ctx, &mut app, "skip review", &f.root.join("logs"));
            for _ in 0..3 { frame(&ctx, &mut app, vec2(1280.0, 860.0), vec![]); }
            let s = app.dialog.as_ref().unwrap().mail_import.as_ref().unwrap();
            assert_eq!(s.preview.as_ref().unwrap().current(), 2);
            assert_eq!(s.selected, 1, "the stopped patch, not the first");
            assert!(app.frame_widgets["dialog-submit"].enabled);
            assert!(!app.frame_widgets.contains_key("dialog-preview-refresh"), "it reviews itself");
            app.dialog = None;
            app.dispatch(Intent::Dialog(DialogKind::MailImport(MailImportAction::Continue)));
            settle(&ctx, &mut app, "continue review", &f.root.join("logs"));
            for _ in 0..3 { frame(&ctx, &mut app, vec2(1280.0, 860.0), vec![]); }
            let d = app.dialog.as_ref().unwrap();
            let error = d.error.as_deref().unwrap();
            assert!(error.contains("series_unresolved"), "automation still sees the code: {error}");
            assert_eq!(
                pages::plain_error(error),
                "second.txt still has a conflict\nFix it in Changes and stage it, then Continue. Nothing has changed."
            );
            assert!(!app.frame_widgets["dialog-submit"].enabled);
            assert!(app.frame_widgets.contains_key("dialog-preview-refresh"), "a refusal can be reviewed again");
            app.dialog = None;
            app.dispatch(Intent::Dialog(DialogKind::MailImport(MailImportAction::Abort)));
            settle(&ctx, &mut app, "abort review", &f.root.join("logs"));
            for _ in 0..3 { frame(&ctx, &mut app, vec2(1280.0, 860.0), vec![]); }
            let s = app.dialog.as_ref().unwrap().mail_import.as_ref().unwrap();
            let rows = abort_steps(s.preview.as_ref().unwrap());
            let notes: Vec<_> = rows.iter().map(|r| (r.mark, r.note)).collect();
            assert_eq!(notes, [(StepMark::LeftOut, "comes off"), (StepMark::Stopped, "not imported")]);
            assert!(!s.fix.is_empty(), "the conflicted file's edits are there to see");
            assert!(app.frame_widgets.contains_key("dialog-series-edits"));
        });
    }

    #[test]
    fn limited_review_requires_pointer_acknowledgement_and_refresh_resets_it() {
        isolated("ROCKET_MAIL_UI_CASE", "mail_import::tests::limited_review_requires_pointer_acknowledgement_and_refresh_resets_it", || {
            let f = Fixture::new();
            let c = CancellationToken::new();
            f.write("base", "base\n");
            f.commit("Base");
            let base = String::from_utf8(f.run(&["rev-parse", "HEAD"])).unwrap().trim().to_owned();
            f.write("change", "local fixture\n");
            f.commit("Short subject");
            let folder = f.root.canonicalize().unwrap().join("mail");
            let export = f.repo().preview_patch_series(&base, "HEAD", &folder, &c).unwrap();
            f.repo().export_patch_series(&export, &c).unwrap();
            f.run(&["reset", "--hard", &base]);
            let preview = f.repo().preview_mail_import(&folder, &c).unwrap();
            let source = folder.join(&preview.entries()[0].filename);
            let raw = fs::read_to_string(&source).unwrap();
            let header = raw.lines().find(|l| l.starts_with("Subject: ")).unwrap();
            fs::write(&source, raw.replacen(header, &format!("Subject: {}", "x".repeat(2048)), 1)).unwrap();
            let preview = f.repo().preview_mail_import(&folder, &c).unwrap();
            let ctx = Context::default();
            // Startup restores persisted preferences. Setting the field before
            // startup completed was immediately undone, starting unrelated scans.
            let (store, mut saved) = crate::persistence::Store::open(&f.root.join("state")).unwrap();
            saved.preferences.dependency_warnings = false;
            store.save(&saved).unwrap();
            drop(store);
            let mut app = Rocket::with_storage(&ctx, vec![f.repository.clone()], f.root.join("logs"), f.root.join("state"));
            settle(&ctx, &mut app, "startup", &f.root.join("logs"));
            assert!(!app.dependencies.enabled);
            assert!(app.can_write());
            app.dispatch(Intent::Dialog(DialogKind::MailImport(MailImportAction::Start)));
            let d = app.dialog.as_mut().unwrap();
            d.first = folder.to_string_lossy().into_owned();
            d.mail_import.get_or_insert_with(Default::default).preview = Some(Arc::new(preview));
            for size in [vec2(1280.0, 860.0), vec2(780.0, 620.0)] {
                for _ in 0..3 { frame(&ctx, &mut app, size, vec![]); }
                assert!(!app.frame_widgets["dialog-submit"].enabled);
                let acknowledgement = &app.frame_widgets["dialog-series-acknowledge"];
                assert!(acknowledgement.enabled && acknowledgement.rect.height() > 10.0);
                assert!(Rect::from_min_size(Pos2::ZERO, size).contains_rect(acknowledgement.rect));
                click(&ctx, &mut app, size, "dialog-series-acknowledge");
                assert!(app.frame_widgets["dialog-submit"].enabled);
                click(&ctx, &mut app, size, "dialog-series-acknowledge");
                assert!(!app.frame_widgets["dialog-submit"].enabled);
            }
            let size = vec2(1280.0, 860.0);
            for _ in 0..3 { frame(&ctx, &mut app, size, vec![]); }
            click(&ctx, &mut app, size, "dialog-series-acknowledge");
            assert!(app.frame_widgets["dialog-submit"].enabled);
            app.start_mail_preview();
            frame(&ctx, &mut app, size, vec![]);
            assert!(!app.frame_widgets["dialog-submit"].enabled);
            settle(&ctx, &mut app, "preview refresh", &f.root.join("logs"));
            frame(&ctx, &mut app, size, vec![]);
            assert!(!app.frame_widgets["dialog-submit"].enabled);
            assert!(!app.dialog.as_ref().unwrap().mail_import.as_ref().unwrap().preview.as_ref().unwrap().truncation_acknowledged());
            assert_eq!(String::from_utf8(f.run(&["rev-parse", "HEAD"])).unwrap().trim(), base);
        });
    }
}
