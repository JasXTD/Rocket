//! Merge review lifecycle. Inputs invalidate reviews; workers never edit UI state.
use crate::{
    app::*,
    backend::{Action, Outcome, user_error},
    pages,
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Default)]
pub struct DialogState {
    pub allow_unrelated: bool,
    /// Whether More options (squash, no commit, unrelated histories) is open.
    pub more: bool,
    pub preview: Option<Arc<MergePreview>>,
    pub task: Option<ReadTask<MergePreview>>,
    pub applying: bool,
}
impl Rocket {
    pub(crate) fn start_merge_preview(&mut self) {
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        if !matches!(dialog.kind, DialogKind::Merge) {
            return;
        }
        let state = dialog.merge.get_or_insert_with(Default::default);
        if state.applying {
            return;
        }
        state.task = None;
        state.preview = None;
        dialog.error = None;
        let target = dialog.first.clone();
        let mode = dialog.mode;
        let allow_unrelated = state.allow_unrelated;
        let path = self
            .tabs
            .iter()
            .find(|t| Some(t.id) == dialog.owner)
            .map(|t| t.path.clone());
        match (self.backend.clone(), path) {
            (Some(backend), Some(path)) => match ReadTask::spawn(move |cancel| {
                backend.merge_preview_options(&path, &target, mode, allow_unrelated, &cancel)
            }) {
                Ok(task) => state.task = Some(task),
                Err(e) => dialog.error = Some(user_error(&e)),
            },
            _ => dialog.error = Some("Open a repository before preparing a merge.".into()),
        }
    }
    pub(crate) fn poll_merge_preview(&mut self) {
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        let Some(state) = &mut dialog.merge else {
            return;
        };
        if let Some(result) = state.task.as_mut().and_then(ReadTask::try_finish) {
            state.task = None;
            match result {
                Ok(preview) => state.preview = Some(Arc::new(preview)),
                Err(e) => {
                    // The fix for this refusal is under More options.
                    if matches!(e, rocket_git::Error::Merge(MergeFailure::UnrelatedHistories)) {
                        state.more = true;
                    }
                    dialog.error = Some(user_error(&e));
                }
            }
        }
    }
    pub(crate) fn finish_merge_dialog(&mut self, owner: Option<u64>, outcome: &Outcome) {
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        let Some(state) = &mut dialog.merge else {
            return;
        };
        if !state.applying || dialog.owner != owner {
            return;
        }
        if outcome.command_succeeded || outcome.recovery_pending {
            let prepared = matches!(dialog.mode, MergeMode::Squash | MergeMode::NoCommit);
            self.dialog = None;
            if (outcome.recovery_pending || prepared)
                && let Some(tab) = self.tabs.iter_mut().find(|t| Some(t.id) == owner)
            {
                tab.view = View::Changes;
            }
        } else {
            state.applying = false;
            state.preview = None;
            dialog.error = Some(format!(
                "{}\n{}\nReview again before retrying.",
                outcome.summary,
                outcome.details.join("\n")
            ));
        }
    }
    pub(crate) fn merge_dialog(&mut self, ctx: &Context) {
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        let state = dialog.merge.get_or_insert_with(Default::default);
        if !state.applying && dialog.owner != self.tab().map(|t| t.id) {
            return;
        }
        if !state.applying
            && state
                .preview
                .as_ref()
                .is_some_and(|p| p.revision() != dialog.first || p.mode() != dialog.mode)
        {
            state.preview = None;
        }
        let waiting = state.task.is_some() || state.applying;
        let idle = self.write.is_none();
        let running = pages::Running::of(self.write.as_ref());
        let mut dismiss = false;
        let mut refresh = false;
        let mut submit = false;
        let mut cancel = false;
        let mut changed = false;
        let into = head_name(
            self.tabs
                .iter()
                .find(|t| Some(t.id) == dialog.owner)
                .and_then(|t| t.snapshot.as_ref())
                .and_then(|s| s.head.as_ref()),
        );
        let title = format!("Merge into {into}");
        let from = short_ref(dialog.first.trim().as_bytes());
        let response = pages::review(
            ctx,
            "rocket-merge",
            pages::ReviewHeader {
                image: Icon::Merge,
                title: &title,
                subtitle: "Bring another branch's commits into the branch you're on.",
                danger: false,
            },
            &mut self.frame_widgets,
            "dialog-merge-scroll",
            !state.applying,
            |ui, part, widgets| match part {
                pages::Part::Body => {
                    if let Some(error) = &dialog.error {
                        pages::callout(ui, RED(), error);
                    }
                    ui.label(RichText::new("Merge from").size(12.0).color(MUTED()));
                    let r = ui.add_enabled(
                        !waiting,
                        TextEdit::singleline(&mut dialog.first)
                            .hint_text("Branch, tag or commit ID")
                            .desired_width(f32::INFINITY)
                            .min_size(vec2(0.0, 30.0))
                            .vertical_align(Align::Center),
                    );
                    pages::control(widgets, "dialog-merge-target", &r, ui.clip_rect());
                    changed |= r.changed();
                    // What the name resolved to, once reviewed.
                    if let Some(p) = &state.preview {
                        resolved(ui, p.target());
                    }
                    ui.add_space(4.0);
                    ui.label(RichText::new("How").size(12.0).color(MUTED()));
                    mode_cards(ui, widgets, &COMMON_MODES, &mut dialog.mode, &mut changed, !waiting);
                    // Closed, it still names a choice it holds.
                    let hidden = MORE_MODES.iter().find(|c| c.1 == dialog.mode).map(|c| c.2);
                    let label = match (state.more, hidden, state.allow_unrelated) {
                        (false, Some(name), _) => format!("More options · {name}"),
                        (false, None, true) => "More options · unrelated histories allowed".into(),
                        _ => "More options".into(),
                    };
                    let r = pages::disclosure(ui, state.more, &label);
                    pages::control(widgets, "dialog-merge-more", &r, ui.clip_rect());
                    if r.clicked() {
                        state.more = !state.more;
                    }
                    if state.more {
                        mode_cards(ui, widgets, &MORE_MODES, &mut dialog.mode, &mut changed, !waiting);
                        ui.horizontal(|ui| {
                            let r = ui.add_enabled(
                                !waiting,
                                checkbox(&mut state.allow_unrelated, "Allow unrelated histories"),
                            );
                            pages::control(widgets, "dialog-merge-unrelated", &r, ui.clip_rect());
                            changed |= r.changed();
                            ui.label(
                                RichText::new("only to join two independent repositories")
                                    .size(12.0)
                                    .color(FAINT()),
                            );
                        });
                        if state.allow_unrelated {
                            pages::callout(
                                ui,
                                AMBER(),
                                "A merge commit combines both histories; conflicts may need Continue or Abort.",
                            );
                        }
                    }
                    if changed {
                        state.preview = None;
                        dialog.error = None;
                    }
                    if state.task.is_some() {
                        pages::waiting(ui, "Reading checkout, index and ancestry…");
                    }
                    if let Some(p) = &state.preview {
                        what_happens(ui, p, &from);
                    }
                }
                pages::Part::Footer if state.applying => {
                    cancel = pages::running_footer(ui, widgets, &running, &[]);
                }
                pages::Part::Footer => {
                    let r = pages::primary(
                        ui,
                        idle && !waiting && state.preview.as_ref().is_some_and(|p| p.can_apply()),
                        "Merge",
                    );
                    pages::control(widgets, "dialog-submit", &r, ui.clip_rect());
                    submit = r.clicked();
                    let ready = idle && !waiting && !dialog.first.trim().is_empty();
                    let r = if state.preview.is_some() || dialog.error.is_some() {
                        ui.add_enabled(ready, Button::new("Review again"))
                    } else {
                        pages::primary(ui, ready, "Review")
                    };
                    pages::control(widgets, "dialog-preview-refresh", &r, ui.clip_rect());
                    refresh = r.clicked();
                    let status = if state.task.is_some() {
                        "Reviewing"
                    } else if dialog.error.is_some() {
                        "Fix the problem above, then review again"
                    } else if let Some(p) = &state.preview {
                        if p.relation() == MergeRelation::UpToDate {
                            "Reviewed · nothing to merge"
                        } else {
                            "Reviewed · nothing has changed yet"
                        }
                    } else {
                        "Review to see what the merge will do"
                    };
                    pages::footer_status(ui, false, status);
                }
            },
        );
        dismiss |= response.should_close() && !state.applying;
        if cancel {
            self.cancel_write();
        }
        if submit && let (Some(owner), Some(preview)) = (dialog.owner, state.preview.clone()) {
            state.applying = true;
            self.execute(owner, Action::MergeReviewed(preview));
            if self.write.is_none() {
                state.applying = false;
                dialog.error = Some(
                    "The merge could not start. Wait for the repository and review again.".into(),
                );
            }
        }
        if !dismiss {
            self.dialog = Some(dialog);
            if refresh {
                self.start_merge_preview();
            }
        }
    }
}

/// A way to merge: its automation key, mode, name and one line about it.
type ModeChoice = (&'static str, MergeMode, &'static str, &'static str);
const COMMON_MODES: [ModeChoice; 3] = [
    ("dialog-merge-no-ff", MergeMode::NoFastForward, "Merge commit", "Always records a merge commit"),
    ("dialog-merge-allow", MergeMode::AllowFastForward, "Fast-forward if possible", "Merge commit only when needed"),
    ("dialog-merge-ff-only", MergeMode::FastForwardOnly, "Fast-forward only", "Stop if both sides have commits"),
];
const MORE_MODES: [ModeChoice; 2] = [
    ("dialog-merge-squash", MergeMode::Squash, "Squash", "Stage the result; you write one commit"),
    ("dialog-merge-no-commit", MergeMode::NoCommit, "Merge without committing", "Inspect in Changes, then continue"),
];

/// A row of ways to merge; choosing another one discards the review.
fn mode_cards(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    choices: &[ModeChoice],
    current: &mut MergeMode,
    changed: &mut bool,
    enabled: bool,
) {
    let cards: Vec<_> = choices
        .iter()
        .map(|&(_, mode, name, about)| (*current == mode, name, about))
        .collect();
    let responses = pages::option_cards(ui, enabled, &cards);
    for (r, &(key, mode, ..)) in responses.iter().zip(choices) {
        pages::control(widgets, key, r, ui.clip_rect());
        if r.clicked() && *current != mode {
            *current = mode;
            *changed = true;
        }
    }
}

fn short(id: &ObjectId) -> &str {
    let id = id.as_str();
    &id[..id.len().min(7)]
}

/// The commit a name resolved to: short ID, subject, author and age.
fn resolved(ui: &mut Ui, commit: &Commit) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let when = ago(
        now.saturating_sub(commit.author.timestamp).max(0) as u64,
        commit.author.timestamp,
    );
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.label(RichText::new(short(&commit.id)).monospace().size(12.0).color(MUTED()));
        ui.add(
            Label::new(
                RichText::new(format!(
                    " · {} · {} · {when}",
                    subject(commit),
                    String::from_utf8_lossy(&commit.author.name)
                ))
                .size(12.0)
                .color(HINT()),
            )
            .truncate(),
        );
    });
}

/// The review: a picture of the two histories, where each side is, and the outcome.
/// The branch is the one reviewed, not the current one, should they differ.
fn what_happens(ui: &mut Ui, p: &MergePreview, from: &str) {
    let into = &head_name(Some(p.head()));
    pages::titled_card(
        ui,
        |ui| {
            ui.label(RichText::new("What will happen").size(12.0).color(MUTED()).strong());
        },
        |ui| {
            picture(ui, p, into, from);
            let original = match p.head() {
                Head::Branch { target, .. } | Head::Detached { target } => short(target),
                Head::Unborn { .. } => "",
            };
            pages::key_value(ui, "Into", 96.0, |ui| at_commit(ui, into, original, None));
            pages::key_value(ui, "From", 96.0, |ui| {
                at_commit(ui, from, short(&p.target().id), Some(&subject(p.target())));
            });
            pages::key_value(ui, "Commits", 96.0, |ui| {
                ui.label(format!(
                    "{} incoming · {} only on {into}",
                    p.incoming(),
                    p.current_only()
                ));
            });
            let (color, text) = outcome(p);
            pages::callout(ui, color, text);
        },
    );
}

/// "name at 1a2b3c4 · subject" on one line, cut short rather than wrapped.
fn at_commit(ui: &mut Ui, name: &str, id: &str, subject: Option<&str>) {
    let plain = |color| TextFormat::simple(FontId::proportional(13.0), color);
    let mut job = text::LayoutJob::default();
    job.append(name, 0.0, plain(TEXT()));
    if !id.is_empty() {
        job.append(" at ", 0.0, plain(FAINT()));
        job.append(id, 0.0, TextFormat::simple(FontId::monospace(12.5), TEXT()));
    }
    if let Some(subject) = subject {
        job.append(&format!(" · {subject}"), 0.0, plain(FAINT()));
    }
    job.wrap = text::TextWrapping::truncate_at_width(ui.available_width());
    ui.label(job);
}

/// What the reviewed merge will do, in a sentence.
fn outcome(p: &MergePreview) -> (Color32, &'static str) {
    let text = match (p.relation(), p.mode()) {
        (MergeRelation::UpToDate, _) => "Already up to date. No new merge commit is needed.",
        (_, MergeMode::Squash) => "HEAD stays unchanged. Review the staged changes and create an ordinary commit. Conflicts are resolved in Changes; use reviewed discard to undo prepared changes.",
        (_, MergeMode::NoCommit) => "HEAD stays unchanged until you Continue merge. Inspect Changes first; Abort merge remains available.",
        (MergeRelation::FastForward, MergeMode::NoFastForward) => {
            "A new merge commit will join the current and incoming tips."
        }
        (MergeRelation::FastForward, _) => {
            "The current branch can advance to the incoming commit without a merge commit."
        }
        (MergeRelation::Diverged, MergeMode::FastForwardOnly) => {
            MergeFailure::NotFastForward.guidance()
        }
        (MergeRelation::Diverged, _) => {
            "Both sides have commits. Git will create a merge commit, or leave conflicts for you to resolve."
        }
    };
    (if p.can_apply() { BLUE() } else { AMBER() }, text)
}

/// A small picture of the merge: the branch moving forward, or both histories joining.
/// Already up to date has nothing to draw.
fn picture(ui: &mut Ui, p: &MergePreview, into: &str, from: &str) {
    let relation = p.relation();
    if relation == MergeRelation::UpToDate {
        return;
    }
    let forward = relation == MergeRelation::FastForward
        && matches!(p.mode(), MergeMode::AllowFastForward | MergeMode::FastForwardOnly);
    let (rect, _) = ui.allocate_exact_size(
        vec2(ui.available_width(), if forward { 50.0 } else { 92.0 }),
        Sense::hover(),
    );
    let painter = ui.painter_at(rect);
    let small = FontId::proportional(11.5);
    let rail = Stroke::new(2.0, WINDOW_EDGE());
    let lane = Stroke::new(2.0, BLUE());
    // Up to three dots spread over a span, the last at its end; the labels carry counts.
    let dots = |n: u64, start: f32, end: f32| {
        let n = n.min(3) as usize;
        (1..=n).map(move |i| start + (end - start) * i as f32 / n as f32)
    };
    let left = rect.left() + 8.0;
    if forward {
        let y = rect.top() + 14.0;
        let (start, end) = (left + 52.0, rect.right() - 200.0);
        painter.line_segment([pos2(left, y), pos2(start, y)], rail);
        painter.line_segment([pos2(start, y), pos2(end, y)], lane);
        for x in dots(p.incoming(), start, end) {
            painter.circle_filled(pos2(x, y), 5.0, BLUE());
        }
        painter.circle_filled(pos2(start, y), 5.5, MUTED());
        painter.text(
            pos2(left, y + 12.0),
            Align2::LEFT_TOP,
            format!("{into} is here"),
            small,
            HINT(),
        );
        painter.text(
            pos2(end + 14.0, y),
            Align2::LEFT_CENTER,
            "moves here · no merge commit",
            FontId::proportional(12.0),
            TEXT(),
        );
        return;
    }
    let (top, bottom) = (rect.top() + 26.0, rect.top() + 66.0);
    let join = rect.right() - 190.0;
    painter.line_segment([pos2(left, top), pos2(join, top)], rail);
    painter.add(Shape::line(
        vec![
            pos2(left + 24.0, top),
            pos2(left + 60.0, bottom),
            pos2(join - 36.0, bottom),
            pos2(join, top),
        ],
        lane,
    ));
    for x in dots(p.current_only(), left + 80.0, join - 40.0) {
        painter.circle_filled(pos2(x, top), 5.0, MUTED());
    }
    for x in dots(p.incoming(), left + 80.0, join - 56.0) {
        painter.circle_filled(pos2(x, bottom), 5.0, BLUE());
    }
    let ink = if p.can_apply() { BLUE() } else { AMBER() };
    painter.circle_filled(pos2(join, top), 7.0, WINDOW());
    painter.circle_stroke(pos2(join, top), 7.0, Stroke::new(2.0, ink));
    let result = match p.mode() {
        MergeMode::Squash => "Staged · you commit",
        MergeMode::NoCommit => "Prepared · not committed yet",
        MergeMode::FastForwardOnly => "Can't fast-forward",
        _ => "New merge commit",
    };
    painter.text(
        pos2(join + 14.0, top),
        Align2::LEFT_CENTER,
        result,
        FontId::proportional(12.0),
        if p.can_apply() { TEXT() } else { AMBER() },
    );
    if p.current_only() > 0 {
        painter.text(
            pos2(left + 80.0, top - 15.0),
            Align2::LEFT_CENTER,
            format!("{} only on {into}", p.current_only()),
            small.clone(),
            HINT(),
        );
    }
    painter.text(
        pos2(left + 80.0, bottom + 15.0),
        Align2::LEFT_CENTER,
        format!("{} incoming from {from}", p.incoming()),
        small,
        HINT(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_common as common;
    use std::{
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };

    fn wait(app: &mut Rocket, ctx: &Context) {
        let deadline = Instant::now() + Duration::from_secs(45);
        while app.busy() {
            app.poll(ctx);
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
    }
    fn render(app: &mut Rocket, ctx: &Context) {
        render_at(app, ctx, vec2(780.0, 620.0));
    }
    /// One frame of the dialog alone; returns what it painted.
    fn render_at(app: &mut Rocket, ctx: &Context, size: Vec2) -> Vec<epaint::ClippedShape> {
        app.frame_widgets.clear();
        let mut output = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                ..Default::default()
            },
            |_| app.merge_dialog(ctx),
        );
        output.textures_delta.clear();
        output.shapes
    }
    /// Squash, merging without a commit and the unrelated-histories opt-in sit under More
    /// options. The unrelated refusal opens it, so its opt-in shows; closed, it names a
    /// choice it holds.
    #[test]
    fn more_options_opens_for_the_unrelated_refusal_and_names_a_hidden_choice() {
        common::isolated(
            "ROCKET_MERGE_UI_CASE",
            "merge::tests::more_options_opens_for_the_unrelated_refusal_and_names_a_hidden_choice",
            || {
                let f = common::Fixture::new();
                f.write("base", "base");
                f.commit("base");
                f.run(&["switch", "--orphan", "other"]);
                f.write("other", "other root");
                f.commit("other root");
                f.run(&["switch", "main"]);
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(
                    &ctx,
                    vec![f.repository.clone()],
                    f.root.join("logs"),
                );
                wait(&mut app, &ctx);
                app.auto_refresh = false;
                app.dispatch(Intent::MergeBranch("refs/heads/other".into()));
                wait(&mut app, &ctx);
                let error = app.dialog.as_ref().unwrap().error.clone().unwrap_or_default();
                assert!(error.contains("merge_unrelated_histories"), "{error}");
                // A modal's first frame only measures it.
                let size = vec2(1480.0, 920.0);
                for _ in 0..2 {
                    render_at(&mut app, &ctx, size);
                }
                assert!(app.frame_widgets["dialog-merge-unrelated"].enabled);
                assert!(app.frame_widgets["dialog-merge-squash"].enabled);
                let d = app.dialog.as_mut().unwrap();
                d.merge.as_mut().unwrap().more = false;
                d.mode = MergeMode::Squash;
                let shapes = render_at(&mut app, &ctx, size);
                for hidden in ["dialog-merge-squash", "dialog-merge-unrelated"] {
                    assert!(!app.frame_widgets.contains_key(hidden), "{hidden}");
                }
                assert!(app.frame_widgets["dialog-merge-no-ff"].enabled);
                assert!(shapes.iter().any(|s| matches!(
                    &s.shape,
                    Shape::Text(t) if t.galley.text() == "More options · Squash"
                )));
            },
        );
    }
    #[test]
    fn selected_branch_review_is_pinned_and_input_changes_invalidate_it() {
        common::isolated(
            "ROCKET_MERGE_UI_CASE",
            "merge::tests::selected_branch_review_is_pinned_and_input_changes_invalidate_it",
            || {
                let f = common::Fixture::new();
                f.write("base", "base");
                f.commit("base");
                f.run(&["switch", "-c", "feature/合并"]);
                f.write("new 文.rs", "incoming");
                f.commit("incoming");
                let incoming = f
                    .repo()
                    .resolve_commit("HEAD", &CancellationToken::new())
                    .unwrap();
                f.run(&["switch", "main"]);
                let original = f.repo().head(&CancellationToken::new()).unwrap();
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(
                    &ctx,
                    vec![f.repository.clone()],
                    f.root.join("logs"),
                );
                wait(&mut app, &ctx);
                app.auto_refresh = false;
                app.dispatch(Intent::MergeBranch("refs/heads/feature/合并".into()));
                wait(&mut app, &ctx);
                let d = app.dialog.as_ref().unwrap();
                assert_eq!(d.first, "refs/heads/feature/合并");
                assert_eq!(
                    d.merge
                        .as_ref()
                        .unwrap()
                        .preview
                        .as_ref()
                        .unwrap()
                        .target()
                        .id,
                    incoming
                );
                app.dialog.as_mut().unwrap().mode = MergeMode::NoFastForward;
                render(&mut app, &ctx);
                assert!(
                    app.dialog
                        .as_ref()
                        .unwrap()
                        .merge
                        .as_ref()
                        .unwrap()
                        .preview
                        .is_none()
                );
                app.start_merge_preview();
                wait(&mut app, &ctx);
                app.dialog.as_mut().unwrap().first = "HEAD".into();
                render(&mut app, &ctx);
                assert!(
                    app.dialog
                        .as_ref()
                        .unwrap()
                        .merge
                        .as_ref()
                        .unwrap()
                        .preview
                        .is_none()
                );
                assert!(!app.frame_widgets["dialog-submit"].enabled);
                assert_eq!(f.repo().head(&CancellationToken::new()).unwrap(), original);
            },
        );
    }
    #[test]
    fn changing_tabs_discards_merge_review_and_quit_cancels_its_read() {
        common::isolated(
            "ROCKET_MERGE_UI_CASE",
            "merge::tests::changing_tabs_discards_merge_review_and_quit_cancels_its_read",
            || {
                let first = common::Fixture::new();
                first.write("file", "first");
                first.commit("first");
                let second = common::Fixture::new();
                second.write("file", "second");
                second.commit("second");
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(
                    &ctx,
                    vec![first.repository.clone(), second.repository.clone()],
                    first.root.join("logs"),
                );
                wait(&mut app, &ctx);
                app.auto_refresh = false;
                app.active = 0;
                app.dispatch(Intent::MergeBranch("HEAD".into()));
                wait(&mut app, &ctx);
                app.active = 1;
                render(&mut app, &ctx);
                assert!(app.dialog.is_none());
                assert!(app.write.is_none());
                app.dispatch(Intent::MergeBranch("HEAD".into()));
                app.begin_quit();
                wait(&mut app, &ctx);
                assert!(
                    app.dialog
                        .as_ref()
                        .unwrap()
                        .merge
                        .as_ref()
                        .unwrap()
                        .preview
                        .is_none()
                );
                assert!(app.write.is_none());
            },
        );
    }
    #[test]
    fn conflict_result_directs_the_ui_to_existing_recovery_and_preserves_raw_privacy() {
        common::isolated(
            "ROCKET_MERGE_UI_CASE",
            "merge::tests::conflict_result_directs_the_ui_to_existing_recovery_and_preserves_raw_privacy",
            || {
                let f = common::Fixture::new();
                f.write("PRIVATE_file.rs", "base\n");
                f.commit("base");
                f.run(&["switch", "-c", "topic"]);
                f.write("PRIVATE_file.rs", "theirs\n");
                f.commit("theirs");
                f.run(&["switch", "main"]);
                f.write("PRIVATE_file.rs", "ours\n");
                f.commit("ours");
                let backend = crate::backend::Backend::discover(&f.root.join("logs")).unwrap();
                let token = CancellationToken::new();
                let p = backend
                    .merge_preview(&f.repository, "topic", MergeMode::AllowFastForward, &token)
                    .unwrap();
                let (tx, _) = mpsc::sync_channel(16);
                let result = backend
                    .execute(
                        &f.repository,
                        &Action::MergeReviewed(Arc::new(p)),
                        &tx,
                        &token,
                    )
                    .unwrap();
                assert!(!result.command_succeeded);
                assert!(result.recovery_pending);
                assert!(result.summary.contains("conflicts"));
                assert!(result.details.iter().any(|d| d.contains("merge_conflicts")));
                assert!(
                    !format!("{}{:?}", result.summary, result.details).contains("PRIVATE_file")
                );
                assert!(result.raw_output.contains("PRIVATE_file"));
                f.repo().abort_merge(&token).unwrap().write.result.unwrap();
            },
        );
    }
}
