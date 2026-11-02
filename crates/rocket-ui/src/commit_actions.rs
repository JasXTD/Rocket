//! Preview-dialog lifecycle for local commit operations. All Git work stays on
//! workers; a failed amend keeps its replacement message and the regular draft.
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
use std::{collections::HashMap, sync::Arc};

/// A review and what its dialog draws, worked out on the worker.
pub struct Reviewed {
    preview: CommitPreview,
    shown: Vec<Vec<Shown>>,
}

pub struct DialogState {
    pub preview: Option<Arc<CommitPreview>>,
    pub task: Option<ReadTask<Reviewed>>,
    pub applying: bool,
    pub message_loaded: bool,
    pub parents: Vec<ObjectId>,
    pub cancelling: bool,
    /// The change as shown: one entry for each commit of a range, otherwise one.
    shown: Vec<Vec<Shown>>,
    selected: usize,
    /// "Your edits go too" is open.
    edits: bool,
    /// The latest review, so a merge's sides stay put while another side is reviewed.
    seen: Option<Arc<CommitPreview>>,
}

/// "1 commit", "3 commits".
fn commits(n: usize) -> String {
    if n == 1 { "1 commit".into() } else { format!("{n} commits") }
}

/// A commit's short ID.
fn short(id: &ObjectId) -> &str {
    &id.as_str()[..7]
}

/// Who wrote a commit, and when.
fn byline(commit: &Commit) -> String {
    let name = display(&commit.author.name);
    format!("{name} · {}", series_view::short_date(&date(commit.author.timestamp, commit.author.offset_minutes)))
}

impl Rocket {
    pub(crate) fn start_commit_preview(&mut self) {
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        if let DialogKind::CommitAction(request) = &dialog.kind {
            let mut request = request.clone();
            // A mainline parent applies only to a single selected merge commit.
            let parents = match request.targets.as_slice() {
                [target] => self
                    .tabs
                    .iter()
                    .find(|t| Some(t.id) == dialog.owner)
                    // A commit behind a collapsed merge is not among the shown rows.
                    .and_then(|t| {
                        t.commits()
                            .iter()
                            .chain(t.loaded_commits())
                            .find(|c| c.id == *target)
                    })
                    .map_or_else(Vec::new, |c| c.parents.clone()),
                _ => Vec::new(),
            };
            // The first parent is the branch the merge went into: nearly always the
            // side to keep, and the review shows exactly what that does.
            if parents.len() > 1 && request.mainline.is_none() {
                request.mainline = Some(1);
                dialog.kind = DialogKind::CommitAction(request.clone());
            }
            let state = dialog.commit.get_or_insert_with(|| DialogState {
                preview: None,
                task: None,
                applying: false,
                message_loaded: false,
                parents,
                cancelling: false,
                shown: Vec::new(),
                selected: 0,
                edits: false,
                seen: None,
            });
            if !state.applying {
                state.preview = None;
                state.task = None;
                state.cancelling = false;
                dialog.error = None;
                match (
                    self.backend.clone(),
                    self.tabs
                        .iter()
                        .find(|t| Some(t.id) == dialog.owner)
                        .map(|t| t.path.clone()),
                ) {
                    (Some(backend), Some(path)) => match ReadTask::spawn(move |cancel| {
                        let preview = backend.commit_preview(&path, request, &cancel)?;
                        let files = |patch: &[u8]| {
                            let mut files = patch_diff::files(patch);
                            for file in &mut files {
                                file.label = visible_path(&file.label);
                            }
                            files
                        };
                        let shown = if preview.patches().is_empty() {
                            vec![files(preview.patch())]
                        } else {
                            preview.patches().iter().map(|p| files(p)).collect()
                        };
                        Ok(Reviewed { preview, shown })
                    }) {
                        Ok(task) => state.task = Some(task),
                        Err(error) => dialog.error = Some(user_error(&error)),
                    },
                    _ => {
                        dialog.error =
                            Some("Open a repository before preparing this operation.".into())
                    }
                }
            }
        }
        self.dialog = Some(dialog);
    }

    pub(crate) fn poll_commit_preview(&mut self) {
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        let Some(state) = &mut dialog.commit else {
            return;
        };
        let Some(result) = state.task.as_mut().and_then(ReadTask::try_finish) else {
            return;
        };
        state.task = None;
        state.cancelling = false;
        match result {
            Ok(Reviewed { preview, shown }) => {
                state.parents = if preview.commits().len() > 1 {
                    Vec::new()
                } else {
                    preview.source().parents.clone()
                };
                if preview.request().operation == CommitOperation::Amend && !state.message_loaded {
                    match String::from_utf8(preview.source().message.clone()) {
                        Ok(message) => dialog.first = message,
                        Err(_) => dialog.error = Some("The original message is not UTF-8. Enter an explicit replacement message to amend it.".into()),
                    }
                    state.message_loaded = true;
                }
                state.shown = shown;
                // A range opens where the review expects it to stop.
                state.selected = preview.prediction().and_then(|p| p.stopped_at).unwrap_or(0).min(state.shown.len().saturating_sub(1));
                let preview = Arc::new(preview);
                state.seen = Some(preview.clone());
                state.preview = Some(preview);
            }
            Err(Error::Cancelled) => dialog.error = Some("Review stopped. Nothing was started.".into()),
            Err(error) => dialog.error = Some(user_error(&error)),
        }
    }

    pub(crate) fn finish_commit_dialog(&mut self, owner: Option<u64>, outcome: &Outcome) {
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        let Some(state) = &mut dialog.commit else {
            return;
        };
        if !state.applying || dialog.owner != owner {
            return;
        }
        if outcome.command_succeeded || outcome.recovery_pending {
            let amend = matches!(&dialog.kind, DialogKind::CommitAction(r) if r.operation == CommitOperation::Amend);
            self.dialog = None;
            if !amend && let Some(tab) = self.tabs.iter_mut().find(|t| Some(t.id) == owner) {
                tab.after_sequence(outcome.recovery_pending);
            }
        } else {
            state.applying = false;
            state.preview = None;
            dialog.error = Some(format!(
                "{}\n{}",
                outcome.summary,
                outcome.details.join("\n")
            ));
        }
    }

    pub(crate) fn commit_action_dialog(&mut self, ctx: &Context) {
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        let DialogKind::CommitAction(mut request) = dialog.kind.clone() else {
            self.dialog = Some(dialog);
            return;
        };
        let tab = self.tabs.iter().find(|t| Some(t.id) == dialog.owner);
        let snapshot = tab.and_then(|t| t.snapshot.as_ref());
        let branch = head_name(snapshot.and_then(|s| s.head.as_ref()));
        let step = snapshot.and_then(|s| s.state.as_ref()).and_then(|s| s.step.clone());
        let original = dialog.commit.as_ref().and_then(|s| s.preview.as_ref()).and_then(|p| p.original_commit().cloned());
        let Some(state) = &mut dialog.commit else {
            self.dialog = Some(dialog);
            return;
        };
        let words = Words::of(&request, state.preview.as_deref(), step.as_ref(), &branch);
        let title = words.title.as_str();
        let starting = request.recovery.is_none() && request.operation != CommitOperation::Amend;
        let range = starting && request.targets.len() > 1;
        let height = (ctx.content_rect().height() - 520.0).clamp(200.0, 300.0);
        let mut dismiss = false;
        let mut refresh = false;
        let mut submit = false;
        let mut cancel = false;
        let mut stop_check = false;
        let waiting = state.task.is_some() || state.applying;
        let idle = self.write.is_none();
        let running = pages::Running::of(self.write.as_ref());
        let head = pages::ReviewHeader {
            image: match request.recovery {
                Some(CommitRecovery::Skip) => Icon::ChevronRight,
                Some(CommitRecovery::Abort) => Icon::Refresh,
                _ => Icon::Commit,
            },
            title,
            subtitle: &words.subtitle,
            danger: head_danger(&request),
        };
        let response = pages::review_at(
            ctx,
            "rocket-commit-action",
            if range { 980.0 } else { 640.0 },
            head,
            &mut self.frame_widgets,
            "dialog-commit-scroll",
            !state.applying,
            |ui, part, widgets| match part {
                pages::Part::Body => {
                    ui.spacing_mut().item_spacing.y = 12.0;
                    // Errors lead the body so a long patch preview never hides them.
                    if let Some(error) = &dialog.error {
                        pages::refusal(ui, AMBER(), error);
                    }
                    if request.operation == CommitOperation::Amend {
                        pages::callout(ui, AMBER(), "Amend changes the commit ID.");
                    }
                    if starting && !range {
                        let seen = state.seen.clone();
                        if let Some(source) = seen.as_ref().map(|p| p.source()) {
                            pages::commit_card(ui, &subject(source), short(&source.id), &byline(source));
                        }
                        // These stay on a refusal: the other side may well work.
                        if state.parents.len() > 1 {
                            refresh |= sides(ui, &mut request, &state.parents.clone(), seen.as_deref(), !waiting, widgets);
                        }
                    }
                    if state.task.is_some() {
                        pages::waiting(
                            ui,
                            if state.cancelling {
                                "Stopping the review…"
                            } else if starting {
                                "Reading the branch and checking for conflicts…"
                            } else {
                                "Reading the branch, staged changes and commit…"
                            },
                        );
                    }
                    if let Some(preview) = state.preview.clone() {
                        match request.recovery {
                            Some(recovery) => recovery_body(ui, recovery, &preview, original.as_ref(), &branch, state, widgets),
                            None if request.operation == CommitOperation::Amend => {
                                if let Some(files) = state.shown.first() {
                                    series_view::diff_card(ui, "Staged changes to include", "dialog-commit-diff", files, 150.0);
                                }
                            }
                            None if range => range_body(ui, &preview, state, widgets, height),
                            None => single_body(ui, &preview, state, &branch, height),
                        }
                    }
                    if request.operation == CommitOperation::Amend && state.message_loaded {
                        ui.label("Replacement commit message");
                        Frame::new()
                            .fill(BG())
                            .stroke(Stroke::new(1.0, BORDER()))
                            .corner_radius(6)
                            .inner_margin(6)
                            .show(ui, |ui| {
                                ScrollArea::vertical()
                                    .id_salt("amend-message")
                                    .min_scrolled_height(66.0)
                                    .max_height(66.0)
                                    .show(ui, |ui| {
                                        let r = ui.add_enabled(
                                            !waiting,
                                            TextEdit::multiline(&mut dialog.first)
                                                .desired_rows(3)
                                                .desired_width(f32::INFINITY)
                                                .frame(Frame::new()),
                                        );
                                        if r.interact_rect.is_positive() {
                                            widgets.insert(
                                                "dialog-amend-message".into(),
                                                UiControl {
                                                    rect: r.interact_rect,
                                                    enabled: r.enabled(),
                                                },
                                            );
                                        }
                                    });
                            });
                    }
                }
                pages::Part::Footer if state.applying => {
                    cancel = pages::running_footer(ui, widgets, &running, &[]);
                }
                pages::Part::Footer => {
                    let amend = request.operation == CommitOperation::Amend;
                    let nothing = state.preview.as_deref().is_some_and(nothing_to_undo);
                    let ready = !waiting
                        && idle
                        && !nothing
                        && (dialog.error.is_none() || amend)
                        && state.preview.is_some()
                        && (!amend || !dialog.first.trim().is_empty());
                    let r = if head_danger(&request) {
                        pages::danger(ui, ready, title)
                    } else {
                        pages::primary(ui, ready, title)
                    };
                    pages::control(widgets, "dialog-submit", &r, ui.clip_rect());
                    submit = r.clicked();
                    // It reviews itself on opening; asking again is for a refusal.
                    if dialog.error.is_some() {
                        let r = ui.add_enabled(!waiting && idle, Button::new("Review again"));
                        pages::control(widgets, "dialog-preview-refresh", &r, ui.clip_rect());
                        refresh |= r.clicked();
                    }
                    if state.task.is_some() {
                        let r = ui.add_enabled(!state.cancelling, Button::new("Stop review"));
                        pages::control(widgets, "dialog-prediction-stop", &r, ui.clip_rect());
                        stop_check = r.clicked();
                    }
                    let status = if state.task.is_some() {
                        "Reviewing".to_owned()
                    } else if dialog.error.is_some() && !amend {
                        "Fix the problem above, then review again".to_owned()
                    } else if nothing {
                        "Nothing to undo".to_owned()
                    } else if starting {
                        let n = state.preview.as_ref().map_or(request.targets.len(), |p| p.commits().len());
                        format!("Reviewed · adds {} to {branch} · nothing is pushed", commits(n))
                    } else {
                        "Nothing has changed yet".to_owned()
                    };
                    pages::footer_status(ui, false, &status);
                }
            },
        );
        dismiss |= response.should_close() && !state.applying;
        if cancel {
            self.cancel_write();
        }
        if stop_check && let Some(task) = &state.task {
            task.cancel();
            state.cancelling = true;
        }
        dialog.kind = DialogKind::CommitAction(request.clone());
        if submit && let (Some(owner), Some(preview)) = (dialog.owner, state.preview.clone()) {
            state.applying = true;
            self.execute(
                owner,
                Action::CommitAction {
                    preview,
                    message: (request.operation == CommitOperation::Amend)
                        .then(|| dialog.first.clone()),
                },
            );
            if self.write.is_none() {
                state.applying = false;
                dialog.error = Some("The operation could not start. Wait for the repository, then review again.".into());
            }
        }
        if !dismiss {
            self.dialog = Some(dialog);
        }
        if refresh {
            self.start_commit_preview();
        }
    }
}

/// The dialog's title and the sentence under it.
struct Words {
    title: String,
    subtitle: String,
}

impl Words {
    fn of(request: &CommitActionRequest, preview: Option<&CommitPreview>, step: Option<&OperationStep>, branch: &str) -> Self {
        let n = request.targets.len();
        let (revert, what) = match request.operation {
            CommitOperation::Revert => (true, "revert"),
            _ => (false, "cherry-pick"),
        };
        // Until a review is in, or when one is refused, Git's own state says where it stopped.
        let stopped = match preview {
            Some(p) => p.stopped_commit().map(subject),
            None => step.map(|s| s.subject.clone()).filter(|s| !s.is_empty()),
        };
        let position = match preview {
            Some(p) => p.is_sequence().then(|| p.position()),
            None => step.filter(|s| s.total > 1).map(|s| (s.at, s.total)),
        };
        let rest = position.map_or(0, |(at, total)| total.saturating_sub(at));
        let then = match rest {
            0 => String::new(),
            1 => format!(", then {what}s the last commit"),
            n => format!(", then {what}s the other {n}"),
        };
        let (title, subtitle) = match (request.operation, request.recovery) {
            (CommitOperation::Amend, _) => (
                "Amend latest commit".to_owned(),
                "Replaces the latest commit with what's staged and the message below.".to_owned(),
            ),
            (_, None) if revert && n > 1 => (
                format!("Revert {n} commits"),
                format!("Adds {n} commits to {branch} that undo these, newest first. Nothing leaves history."),
            ),
            (_, None) if n > 1 => (
                format!("Cherry-pick {n} commits"),
                format!("Adds a copy of each commit to {branch}, oldest first."),
            ),
            (_, None) if revert && preview.is_some_and(nothing_to_undo) => (
                "Revert commit".to_owned(),
                format!("This commit's changes are already gone from {branch}."),
            ),
            (_, None) if revert => (
                "Revert commit".to_owned(),
                format!("Adds a commit to {branch} that undoes this one. Nothing leaves history."),
            ),
            (_, None) => ("Cherry-pick commit".to_owned(), format!("Adds a copy of this commit to {branch}.")),
            (_, Some(CommitRecovery::Continue)) => (
                format!("Continue {what}"),
                match (&stopped, preview, position) {
                    (Some(subject), Some(_), _) => format!("Saves your fix for {subject}{then}."),
                    (Some(subject), None, Some((at, total))) => format!("Commit {at} of {total} · {subject}"),
                    (Some(subject), None, None) => subject.clone(),
                    // Between two commits Git goes on after the one it reached, which someone
                    // finished by hand or Git couldn't make.
                    (None, Some(p), Some((at, _))) if p.rewinds() => match p.reviewed().get(at - 1) {
                        Some(commit) => format!("Leaves out {}{then}.", subject(commit)),
                        None => format!("Finishes the {what}."),
                    },
                    (None, _, Some(_)) if rest > 1 => format!("Does the other {rest} commits, then finishes the {what}."),
                    (None, _, Some(_)) if rest == 1 => format!("Does the last commit, then finishes the {what}."),
                    (None, _, _) => format!("Finishes the {what}."),
                },
            ),
            (_, Some(CommitRecovery::Skip)) => (
                position.map_or_else(|| "Skip commit".to_owned(), |(at, _)| format!("Skip commit {at}")),
                match &stopped {
                    Some(subject) => format!("Leaves out {subject}{then}."),
                    None => "Leaves out this commit.".to_owned(),
                },
            ),
            (_, Some(CommitRecovery::Abort)) => (
                format!("Abort {what}"),
                if preview.map_or(position.is_some(), |p| p.is_sequence() && p.rewinds()) {
                    format!("Puts {branch} back where it was before the {what}.")
                } else if preview.is_some_and(CommitPreview::is_sequence) {
                    format!("Ends the {what}. {branch} stays where it is, because it moved by hand.")
                } else {
                    let doing = if revert { "reverting" } else { "copying" };
                    match &stopped {
                        Some(subject) => format!("Stops {doing} {subject}. {branch} stays where it is."),
                        None => format!("Stops the {what}. {branch} stays where it is."),
                    }
                },
            ),
        };
        Self { title, subtitle }
    }
}

/// A merge's sides, to choose which one stays. True when the choice changed.
fn sides(
    ui: &mut Ui,
    request: &mut CommitActionRequest,
    parents: &[ObjectId],
    preview: Option<&CommitPreview>,
    enabled: bool,
    widgets: &mut HashMap<String, UiControl>,
) -> bool {
    let revert = request.operation == CommitOperation::Revert;
    ui.label(RichText::new(if revert { "This is a merge. Which side stays?" } else { "This is a merge. Copy what it added to which side?" }).size(12.0).color(HINT()));
    let names: Vec<(String, String)> = parents
        .iter()
        .enumerate()
        .map(|(i, id)| {
            let about = if i == 0 { "the branch it merged into" } else { "the branch that came in" };
            let name = preview.and_then(|p| p.parent_commits().iter().find(|c| c.id == *id));
            (name.map_or_else(|| format!("Side {}", i + 1), subject), format!("{} · {about}", short(id)))
        })
        .collect();
    let cards: Vec<(bool, &str, &str)> = names
        .iter()
        .enumerate()
        .map(|(i, (name, about))| (request.mainline == Some(i + 1), name.as_str(), about.as_str()))
        .collect();
    let mut changed = false;
    for (i, r) in pages::option_cards(ui, enabled, &cards).into_iter().enumerate() {
        pages::control(widgets, format!("dialog-mainline-{}", i + 1), &r, ui.clip_rect());
        if r.clicked() && request.mainline != Some(i + 1) {
            request.mainline = Some(i + 1);
            changed = true;
        }
    }
    if revert {
        pages::note(ui, "Merging that branch again later won't bring these changes back.");
    }
    changed
}

/// What a new commit does to the branch: a line of history with the new commit at its end.
fn picture(ui: &mut Ui, branch: &str, label: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 58.0), Sense::hover());
    let painter = ui.painter();
    let y = rect.top() + 40.0;
    let left = rect.left() + 8.0;
    let step = 46.0;
    let line = Stroke::new(1.5, RING_IDLE());
    let tip = left + step * 3.0;
    let new = (rect.right() - 12.0).min(tip + 120.0);
    painter.hline(left..=tip, y, line);
    painter.hline(tip..=new, y, Stroke::new(1.5, BLUE()));
    for i in 1..=3 {
        painter.circle_filled(pos2(left + step * i as f32, y), 4.5, RING_IDLE());
    }
    painter.circle_filled(pos2(new, y), 5.5, BLUE());
    let mut now = text::LayoutJob::simple_singleline(format!("{branch} now"), FontId::proportional(12.0), HINT());
    now.wrap = text::TextWrapping::truncate_at_width(new - left);
    let now = painter.layout_job(now);
    painter.galley(pos2(left, y - 28.0), now, HINT());
    let mut job = text::LayoutJob::simple_singleline(label.to_owned(), semibold(12.0), TEXT());
    job.wrap = text::TextWrapping::truncate_at_width((rect.right() - new - 16.0).max(120.0));
    let galley = painter.layout_job(job);
    painter.galley(pos2(new + 12.0, y - galley.size().y / 2.0), galley, TEXT());
}

/// A single revert of changes that are already gone. Git fails instead of stopping, as
/// it would for a cherry-pick, so there's nothing to go on to.
fn nothing_to_undo(preview: &CommitPreview) -> bool {
    preview.request().operation == CommitOperation::Revert
        && preview.commits().len() == 1
        && preview.prediction().is_some_and(|p| p.kind == PredictionKind::Empty)
}

/// What the review found about conflicts, in plain words.
fn expected(ui: &mut Ui, preview: &CommitPreview) {
    if let Some(prediction) = preview.prediction() {
        let revert = preview.request().operation == CommitOperation::Revert;
        if revert && prediction.kind == PredictionKind::Empty {
            let range = preview.commits().len() > 1;
            let at = prediction.stopped_at.unwrap_or(0) + 1;
            pages::callout(ui, AMBER(), &if range { format!("Nothing to undo at commit {at} of {}", prediction.total) } else { "Nothing to undo".to_owned() });
            if range {
                pages::note(ui, "Those changes are already gone, so Git stops there. Continue goes on without that commit.");
            }
        } else {
            prediction_ui(ui, prediction);
        }
        if preview.commits().len() > 1
            && let Some(commit) = prediction.stopped_at.and_then(|i| preview.commits().get(i))
        {
            pages::note(ui, &format!("That's {} · {}, open below.", short(&commit.id), subject(commit)));
        }
    }
}

/// One commit's review: the commit, what happens to the branch, and the change itself.
fn single_body(ui: &mut Ui, preview: &CommitPreview, state: &DialogState, branch: &str, height: f32) {
    if nothing_to_undo(preview) {
        expected(ui, preview);
        return;
    }
    let source = preview.source();
    let revert = preview.request().operation == CommitOperation::Revert;
    pages::titled_card(
        ui,
        |ui| {
            ui.label(RichText::new("What will happen").size(12.0).color(MUTED()).strong());
        },
        |ui| {
            Frame::new().inner_margin(Margin::symmetric(14, 8)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let label = if revert { format!("Revert \"{}\"", subject(source)) } else { format!("{} · copy", subject(source)) };
                picture(ui, branch, &label);
                ui.add_space(4.0);
                expected(ui, preview);
            });
        },
    );
    if let Some(files) = state.shown.first() {
        let title = if revert { "What the new commit changes" } else { "What the copy changes" };
        series_view::diff_card(ui, title, "dialog-commit-diff", files, height);
    }
}

/// Several commits: a list beside the chosen one's change.
fn range_body(
    ui: &mut Ui,
    preview: &CommitPreview,
    state: &mut DialogState,
    widgets: &mut HashMap<String, UiControl>,
    height: f32,
) {
    expected(ui, preview);
    let revert = preview.request().operation == CommitOperation::Revert;
    if preview.patches().is_empty() {
        pages::note(ui, "These commits are too big to show here. The check above still covers them.");
    }
    let rows: Vec<series_view::Row> = preview
        .commits()
        .iter()
        .enumerate()
        .map(|(i, commit)| {
            let (added, removed) = series_view::counts(state.shown.get(i).map(Vec::as_slice).unwrap_or_default());
            series_view::Row {
                subject: subject(commit),
                meta: format!("{} · {}", short(&commit.id), byline(commit)),
                added,
                removed,
            }
        })
        .collect();
    let title = format!("{}, {}", commits(rows.len()), if revert { "newest first" } else { "oldest first" });
    let files = state.shown.get(state.selected).map(Vec::as_slice).unwrap_or_default();
    series_view::split(
        ui,
        &title,
        &rows,
        &mut state.selected,
        files,
        series_view::Keys { row: "dialog-commit-entry", scroll: "dialog-commit-list" },
        widgets,
        !state.applying,
        height,
    );
}

/// Every reviewed commit of a stopped range, and what `recovery` does to it.
fn range_steps(preview: &CommitPreview, recovery: CommitRecovery) -> Vec<Step> {
    use std::cmp::Ordering::*;
    use CommitRecovery::*;
    let revert = preview.request().operation == CommitOperation::Revert;
    let (done, not_done) = if revert { ("reverted", "not reverted") } else { ("copied", "not copied") };
    let (reached, _) = preview.position();
    // Between two commits, the one Git reached is finished by hand, or Git couldn't make it.
    let between = preview.stopped_commit().is_none();
    let by_hand = between && !preview.rewinds();
    let unmade = between && preview.rewinds();
    let made: Vec<String> = preview.applied().iter().map(subject).collect();
    let none_skipped = made.len() == reached - 1 + usize::from(by_hand);
    let landed = |commit: &Commit| {
        let name = subject(commit);
        none_skipped || made.contains(&if revert { format!("Revert \"{name}\"") } else { name })
    };
    preview
        .reviewed()
        .iter()
        .enumerate()
        .map(|(i, commit)| {
            let number = i + 1;
            let (mark, note) = match (recovery, number.cmp(&reached)) {
                (_, Less) if !landed(commit) => (StepMark::Waiting, "skipped"),
                (Abort, Less) if by_hand => (StepMark::Done, "stays"),
                (Abort, Less) => (StepMark::LeftOut, "comes off"),
                (_, Less) => (StepMark::Done, done),
                (Abort, Equal) if by_hand => (StepMark::Done, "stays"),
                (Abort, Equal) if unmade => (StepMark::Waiting, not_done),
                (Abort, Equal) => (StepMark::Stopped, not_done),
                (Continue, Equal) if by_hand => (StepMark::Done, "done by hand"),
                (Continue, Equal) if unmade => (StepMark::LeftOut, "left out"),
                (Continue, Equal) => (StepMark::Stopped, "your fix"),
                (Skip, Equal) => (StepMark::LeftOut, "left out"),
                (Abort, Greater) => (StepMark::Waiting, not_done),
                (_, Greater) => (StepMark::Waiting, ""),
            };
            Step { mark, number, subject: subject(commit), note }
        })
        .collect()
}

/// Continue, Skip or Abort of a stopped cherry-pick or revert.
fn recovery_body(
    ui: &mut Ui,
    recovery: CommitRecovery,
    preview: &CommitPreview,
    original: Option<&Commit>,
    branch: &str,
    state: &mut DialogState,
    widgets: &mut HashMap<String, UiControl>,
) {
    let revert = preview.request().operation == CommitOperation::Revert;
    let edits = state.shown.first().filter(|files| !files.is_empty());
    let range = preview.is_sequence();
    match recovery {
        CommitRecovery::Continue => {
            if range {
                pages::steps(ui, "dialog-commit", &range_steps(preview, recovery));
            } else {
                pages::commit_card(ui, &subject(preview.source()), short(&preview.source().id), if revert { "being reverted" } else { "being copied" });
            }
            if let Some(files) = edits {
                let staged = if files.len() == 1 { "1 file staged".to_owned() } else { format!("{} files staged", files.len()) };
                series_view::diff_card(ui, &format!("Your fix · {staged}"), "dialog-commit-diff", files, 160.0);
            }
        }
        CommitRecovery::Skip => {
            if range {
                pages::steps(ui, "dialog-commit", &range_steps(preview, recovery));
            } else {
                pages::commit_card(ui, &subject(preview.source()), short(&preview.source().id), "left out");
            }
            if let Some(files) = edits {
                state.edits = series_view::edits_section(ui, "dialog-commit-edits", files, state.edits, widgets);
            }
        }
        CommitRecovery::Abort => {
            let back = range && preview.rewinds();
            ui.label(RichText::new(format!("{branch} {}", if back { "goes back to" } else { "stays at" })).size(12.0).color(HINT()));
            if let Some(commit) = original {
                pages::commit_card(ui, &subject(commit), short(&commit.id), if back { "before it started" } else { "where it is now" });
            }
            if range {
                pages::steps(ui, "dialog-commit", &range_steps(preview, recovery));
            }
            if let Some(files) = edits {
                state.edits = series_view::edits_section(ui, "dialog-commit-edits", files, state.edits, widgets);
            }
            if back && !preview.applied().is_empty() {
                pages::note(ui, "Reflog & Recovery can still find the commits it made.");
            }
        }
    }
}

pub(crate) fn prediction_ui(ui: &mut Ui, prediction: &CommitPrediction) {
    let total = prediction.total;
    let stopped_at = prediction.stopped_at.unwrap_or(0) + 1;
    let at = if total > 1 { format!(" at commit {stopped_at} of {total}") } else { String::new() };
    let (color, title) = match prediction.kind {
        PredictionKind::Clean if total > 1 => (GREEN(), format!("No conflicts expected: all {total} commits apply cleanly.")),
        PredictionKind::Clean => (GREEN(), "No conflicts expected.".to_owned()),
        PredictionKind::Conflicts => (AMBER(), format!("Conflict expected{at}")),
        PredictionKind::Empty => (AMBER(), format!("Nothing to change{at}")),
        PredictionKind::Unavailable => (MUTED(), "Rocket can't tell yet whether this conflicts".to_owned()),
    };
    let unchecked = total - prediction.checked;
    let stops = if total > 1 { "Git stops there" } else { "Git stops" };
    let later = match unchecked {
        0 => String::new(),
        1 => " The commit after it wasn't checked.".to_owned(),
        n => format!(" The {n} commits after it weren't checked."),
    };
    let note = match prediction.kind {
        PredictionKind::Clean => String::new(),
        PredictionKind::Conflicts => format!("{stops} so you can fix it, then Continue or Abort.{later}"),
        PredictionKind::Empty => format!("Those changes are already here, so {stops} for you to decide.{later}"),
        PredictionKind::Unavailable => prediction
            .reason
            .map_or("No clean result is claimed.", PredictionReason::guidance)
            .to_owned(),
    };
    pages::callout(ui, color, &title);
    if !note.is_empty() {
        pages::note(ui, &note);
    }
    if !prediction.paths.is_empty() {
        pages::well(ui, |ui| {
            for path in prediction.paths.iter().take(25) {
                ui.add(
                    Label::new(
                        RichText::new(path.display_lossy().chars().take(1024).collect::<String>())
                            .monospace(),
                    )
                    .wrap(),
                );
            }
            if prediction.paths.len() > 25 {
                pages::note(
                    ui,
                    &format!("{} more conflicting paths.", prediction.paths.len() - 25),
                );
            }
        });
    }
}

/// Abort and Skip discard conflict resolutions and tracked changes.
fn head_danger(request: &CommitActionRequest) -> bool {
    matches!(
        request.recovery,
        Some(CommitRecovery::Abort | CommitRecovery::Skip)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        batch::flow_tests::{click, frame, open, wait},
        test_common::{Fixture, isolated},
    };

    fn request(operation: CommitOperation, targets: usize, recovery: Option<CommitRecovery>) -> CommitActionRequest {
        let id = ObjectId::parse("1234567890123456789012345678901234567890").unwrap();
        CommitActionRequest { operation, targets: vec![id; targets], mainline: None, recovery }
    }

    #[test]
    fn titles_say_what_happens_to_the_branch() {
        let words = |operation, targets, recovery| Words::of(&request(operation, targets, recovery), None, None, "feature");
        use CommitOperation::*;
        assert_eq!(words(Revert, 1, None).title, "Revert commit");
        assert_eq!(words(Revert, 1, None).subtitle, "Adds a commit to feature that undoes this one. Nothing leaves history.");
        assert_eq!(words(Revert, 3, None).title, "Revert 3 commits");
        assert!(words(Revert, 3, None).subtitle.contains("newest first"));
        assert_eq!(words(CherryPick, 1, None).subtitle, "Adds a copy of this commit to feature.");
        assert!(words(CherryPick, 2, None).subtitle.contains("oldest first"));
        assert_eq!(words(Revert, 0, Some(CommitRecovery::Abort)).title, "Abort revert");
        assert_eq!(words(CherryPick, 0, Some(CommitRecovery::Continue)).title, "Continue cherry-pick");
        assert_eq!(words(Revert, 0, Some(CommitRecovery::Skip)).title, "Skip commit");
        assert_eq!(words(Amend, 0, None).title, "Amend latest commit");
    }

    /// A refused review has no preview, so Git's own state fills in where it stopped.
    #[test]
    fn a_refused_review_still_says_where_it_stopped() {
        let step = |at, total| OperationStep { at, total, kind: StepKind::Commit, subject: "Add a lexer".into(), branch: None };
        let words = |recovery, step: &OperationStep| Words::of(&request(CommitOperation::Revert, 0, Some(recovery)), None, Some(step), "feature");
        assert_eq!(words(CommitRecovery::Continue, &step(2, 3)).subtitle, "Commit 2 of 3 · Add a lexer");
        assert_eq!(words(CommitRecovery::Continue, &step(1, 1)).subtitle, "Add a lexer");
        assert_eq!(words(CommitRecovery::Skip, &step(2, 3)).title, "Skip commit 2");
        assert_eq!(words(CommitRecovery::Abort, &step(1, 1)).subtitle, "Stops reverting Add a lexer. feature stays where it is.");
        assert_eq!(words(CommitRecovery::Abort, &step(2, 3)).subtitle, "Puts feature back where it was before the revert.");
    }

    /// A merge opens reviewed, on the side it went into, and the other side is a click away.
    #[test]
    fn a_merge_opens_reviewed_with_a_side_chosen() {
        isolated("ROCKET_COMMIT_UI_CASE", "commit_actions::tests::a_merge_opens_reviewed_with_a_side_chosen", || {
            let f = Fixture::new();
            f.write("base.txt", "base\n");
            f.commit("Base");
            f.run(&["switch", "-c", "side"]);
            f.write("side.txt", "side\n");
            f.commit("Side work");
            f.run(&["switch", "main"]);
            f.write("main.txt", "main\n");
            f.commit("Main work");
            f.run(&["merge", "--no-ff", "--no-edit", "side"]);
            let merge = String::from_utf8(f.run(&["rev-parse", "HEAD"])).unwrap().trim().to_owned();
            let ctx = Context::default();
            let mut app = open(&ctx, &f);
            app.dispatch(Intent::SelectCommit(ObjectId::parse(&merge).unwrap()));
            wait(&ctx, &mut app);
            let targets = vec![ObjectId::parse(&merge).unwrap()];
            app.dispatch(Intent::Dialog(DialogKind::CommitAction(CommitActionRequest {
                operation: CommitOperation::Revert,
                targets,
                mainline: None,
                recovery: None,
            })));
            wait(&ctx, &mut app);
            for _ in 0..3 {
                frame(&ctx, &mut app, vec![]);
            }
            let d = app.dialog.as_ref().unwrap();
            assert_eq!(d.error, None, "no refusal on opening");
            let p = d.commit.as_ref().unwrap().preview.as_ref().unwrap();
            assert_eq!(p.parent_commits().len(), 2);
            assert!(app.frame_widgets["dialog-submit"].enabled);
            assert!(!app.frame_widgets.contains_key("dialog-preview-refresh"), "it reviews itself");
            let first = patch(&app);
            assert!(first.contains("side.txt") && !first.contains("main.txt"), "{first}");
            click(&ctx, &mut app, "dialog-mainline-2");
            wait(&ctx, &mut app);
            let second = patch(&app);
            assert!(second.contains("main.txt") && !second.contains("side.txt"), "{second}");
        });
    }

    fn patch(app: &Rocket) -> String {
        let p = app.dialog.as_ref().unwrap().commit.as_ref().unwrap().preview.as_ref().unwrap();
        String::from_utf8_lossy(p.patch()).into_owned()
    }

    /// A range shows each commit's own change, and a row's right-click offers it.
    #[test]
    fn a_range_lists_its_commits_beside_each_change() {
        isolated("ROCKET_COMMIT_UI_CASE", "commit_actions::tests::a_range_lists_its_commits_beside_each_change", || {
            let f = Fixture::new();
            f.write("base.txt", "base\n");
            f.commit("Base");
            let mut ids = Vec::new();
            for name in ["one", "two", "three"] {
                f.write(&format!("{name}.txt"), "new\n");
                f.commit(name);
                ids.push(ObjectId::parse(String::from_utf8(f.run(&["rev-parse", "HEAD"])).unwrap().trim()).unwrap());
            }
            let ctx = Context::default();
            let mut app = open(&ctx, &f);
            app.dispatch(Intent::Dialog(DialogKind::CommitAction(CommitActionRequest {
                operation: CommitOperation::Revert,
                targets: ids.iter().rev().cloned().collect(),
                mainline: None,
                recovery: None,
            })));
            wait(&ctx, &mut app);
            for _ in 0..3 {
                frame(&ctx, &mut app, vec![]);
            }
            let state = app.dialog.as_ref().unwrap().commit.as_ref().unwrap();
            assert_eq!(state.shown.len(), 3);
            assert_eq!(state.shown[0][0].label, "three.txt");
            assert_eq!((state.shown[0][0].added, state.shown[0][0].removed), (0, 1), "undone, so its lines go");
            click(&ctx, &mut app, "dialog-commit-entry-2");
            assert_eq!(app.dialog.as_ref().unwrap().commit.as_ref().unwrap().selected, 2);
        });
    }
}
