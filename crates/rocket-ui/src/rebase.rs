//! Reviewed rebase dialogs; repository reads and writes run on workers. A new
//! rebase shows what will happen before it starts; an interactive one edits its
//! plan row by row beside the list. A rebase in progress reviews Continue, Skip
//! and Abort.
use crate::{
    app::*,
    backend::{Action, Outcome, user_error},
    pages::{self, Step, StepMark, control},
    patch_diff::{self, Shown},
    series_view,
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use std::{collections::HashMap, sync::Arc};
use crate::i18n::{mark, tr, trf, trn};

/// The interactive plan's editor pane width.
const PANE: f32 = 380.0;
/// What an interactive rebase says while it reads.
const READING: &str = mark("Reading the branch, its commits and the rebase state");

/// A review and the staged fix's files as shown, worked out on the worker.
pub struct Reviewed {
    preview: RebasePreview,
    pub(crate) fix: Vec<Shown>,
}

#[derive(Default)]
pub struct DialogState {
    pub preview: Option<Arc<RebasePreview>>,
    pub task: Option<ReadTask<Reviewed>>,
    /// Continue: the staged fix's files as shown.
    pub(crate) fix: Vec<Shown>,
    pub applying: bool,
    pub cancelling: bool,
    pub entries: Vec<RebasePlanEntry>,
    pub selected: usize,
    pub reviewed: bool,
    /// Interactive only: rewrite the whole branch from its root commit, not after a base.
    pub root: bool,
    /// Move other branches that point into the replayed commits (`--update-refs`).
    pub update_refs: bool,
    pub preserve_merges: bool,
    /// More options is open.
    pub more: bool,
    /// The list of commits to make again is open.
    pub listed: bool,
    /// Skip and Abort: the edits they discard are open.
    pub edits: bool,
}
impl Rocket {
    pub(crate) fn start_rebase_preview(&mut self) {
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        if let DialogKind::Rebase(action) = dialog.kind {
            let state = dialog.rebase.get_or_insert_with(Default::default);
            if !state.applying {
                state.preview = None;
                state.task = None;
                state.reviewed = false;
                state.cancelling = false;
                dialog.error = None;
                let root = action == RebaseAction::Interactive && state.root;
                let preserve = action == RebaseAction::Start && state.preserve_merges;
                if !action.is_start() || root || !dialog.first.trim().is_empty() {
                    let target = dialog.first.trim().to_string();
                    match (
                        self.backend.clone(),
                        self.tabs
                            .iter()
                            .find(|t| Some(t.id) == dialog.owner)
                            .map(|t| t.path.clone()),
                    ) {
                        (Some(backend), Some(path)) => match ReadTask::spawn(move |cancel| {
                            let preview = if root {
                                backend.root_rebase_preview(&path, &cancel)
                            } else if preserve {
                                backend.preserving_rebase_preview(&path, &target, &cancel)
                            } else {
                                backend.rebase_preview(&path, action, &target, &cancel)
                            }?;
                            let fix = if action == RebaseAction::Continue { patch_diff::files(preview.patch()) } else { Vec::new() };
                            Ok(Reviewed { preview, fix })
                        }) {
                            Ok(task) => state.task = Some(task),
                            Err(e) => dialog.error = Some(user_error(&e)),
                        },
                        _ => dialog.error = Some(crate::i18n::tr("Open a repository first.").into()),
                    }
                }
            }
        }
        self.dialog = Some(dialog);
    }
    pub(crate) fn poll_rebase_preview(&mut self) {
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.rebase else { return };
        let Some(result) = s.task.as_mut().and_then(ReadTask::try_finish) else {
            return;
        };
        s.task = None;
        s.cancelling = false;
        match result {
            Ok(Reviewed { preview: p, fix }) => {
                // A new commit set needs a new plan. An unchanged set retains reviewed edits.
                if p.action() == RebaseAction::Interactive && p.validate_plan(&s.entries).is_err() {
                    s.entries = p.plan().unwrap_or_default().to_vec();
                    s.selected = 0;
                }
                s.preview = Some(Arc::new(p));
                s.fix = fix;
            }
            Err(Error::Cancelled) => d.error = Some(tr("Conflict check cancelled. No rebase was started. Review again to check again.").into()),
            Err(e) => d.error = Some(user_error(&e)),
        }
    }
    pub(crate) fn finish_rebase_dialog(&mut self, owner: Option<u64>, outcome: &Outcome) {
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.rebase else { return };
        if !s.applying || d.owner != owner {
            return;
        }
        if outcome.command_succeeded || outcome.recovery_pending {
            self.dialog = None;
            if let Some(tab) = self.tabs.iter_mut().find(|t| Some(t.id) == owner) {
                tab.after_sequence(outcome.recovery_pending);
            }
        } else {
            s.applying = false;
            s.preview = None;
            s.reviewed = false;
            d.error = Some(trf!("{summary}\n{details}\nReview again before retrying.", summary = outcome.summary, details = outcome.details.join("\n")));
        }
    }
    pub(crate) fn rebase_dialog(&mut self, ctx: &Context) {
        let Some(mut d) = self.dialog.take() else {
            return;
        };
        let DialogKind::Rebase(action) = d.kind else {
            self.dialog = Some(d);
            return;
        };
        let tab = self.tabs.iter().find(|t| Some(t.id) == d.owner);
        let branches: Vec<String> = tab
            .and_then(|t| t.snapshot.as_ref())
            .map(|s| {
                s.branches
                    .iter()
                    .filter(|b| !b.is_head)
                    .map(|b| short_ref(&b.reference))
                    .collect()
            })
            .unwrap_or_default();
        let snapshot = tab.and_then(|t| t.snapshot.as_ref());
        let step = snapshot.and_then(|s| s.state.as_ref()).and_then(|s| s.step.clone());
        // Git leaves the branch while it works, so a paused rebase names it from its state.
        let current = step
            .as_ref()
            .and_then(|t| t.branch.as_deref())
            .map_or_else(|| head_name(snapshot.and_then(|s| s.head.as_ref())), display);
        let changes: Vec<(String, &'static str)> = snapshot
            .and_then(|s| s.status.as_ref())
            .map(|status| {
                status
                    .entries
                    .iter()
                    .filter(|e| !matches!(e.kind, EntryKind::Untracked | EntryKind::Ignored))
                    .map(|e| (e.path.display_lossy().into_owned(), if e.kind == EntryKind::Unmerged { "conflict, not fixed yet" } else { "changed" }))
                    .collect()
            })
            .unwrap_or_default();
        let idle = self.write.is_none();
        let running = pages::Running::of(self.write.as_ref());
        let s = d.rebase.get_or_insert_with(Default::default);
        // What it goes on top of, for the line under the field.
        let onto = s.preview.as_ref().and_then(|p| p.onto_commit()).map(|c| {
            format!(
                "{} · {} · {} · {}",
                &c.id.as_str()[..7],
                subject(c),
                display(&c.author.name),
                crate::tags::age(c.author.timestamp)
            )
        });
        // A paused rebase's chosen commit, by a branch's name where one points at it.
        let on_top = s.preview.as_ref().and_then(|p| p.onto().cloned()).map(|id| {
            let branch = snapshot
                .into_iter()
                .flat_map(|s| s.branches.iter())
                .find(|b| b.target == id)
                .map(|b| short_ref(&b.reference));
            (branch, id.as_str()[..7].to_owned())
        });
        let interactive = action == RebaseAction::Interactive;
        let (mut dismiss, mut refresh, mut submit, mut cancel) = (false, false, false, false);
        let waiting = s.task.is_some() || s.applying;
        let mut stop_check = false;
        let mut validation = if interactive { plan_error(s) } else { None };
        // At a planned stop Skip keeps the replayed commits; it only discards edits.
        let planned = s.preview.as_ref().and_then(|p| p.planned_stop().cloned());
        let discard_and_continue = action == RebaseAction::Skip && planned.is_some();
        let at = step.as_ref().map_or(0, |t| t.at);
        let rest = step.as_ref().map_or(0, |t| t.total.saturating_sub(t.at));
        let then = match rest {
            0 => tr("that's the last one").to_owned(),
            1 => tr("then moves the last commit").to_owned(),
            n => trn!(n, "then moves the other {n} commit", "then moves the other {n} commits"),
        };
        let commit = if at > 0 { trf!("commit {at}", at) } else { tr("this commit").to_owned() };
        let stopped_name = step
            .as_ref()
            .map(|t| t.subject.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| tr("this commit").into());
        let title = match action {
            RebaseAction::Start => trf!("Rebase {current}", current),
            RebaseAction::Interactive if s.reviewed => tr("Review the new history").into(),
            RebaseAction::Interactive => trf!("Interactive rebase · {current}", current),
            RebaseAction::Skip if discard_and_continue => tr("Discard changes and continue").into(),
            RebaseAction::Skip if at > 0 => trf!("Skip commit {at}", at),
            _ => self::title(action).into(),
        };
        let subtitle = match action {
            RebaseAction::Skip if discard_and_continue => {
                tr("Discard tracked changes made since the planned stop, then continue the plan.").to_owned()
            }
            RebaseAction::Continue if planned.is_some() => tr("Resume the preserved plan from this planned stop.").to_owned(),
            RebaseAction::Start => tr("Moves its commits on top of another branch or commit.").to_owned(),
            RebaseAction::Interactive if s.reviewed => tr("Nothing has changed yet. Start when this looks right.").to_owned(),
            RebaseAction::Interactive => tr("Reorder, reword, fold or leave out commits, then review the result.").to_owned(),
            RebaseAction::Continue if s.preview.is_none() => match &step {
                Some(t) => trf!("Commit {at} of {total} · {stopped_name}", at = t.at, total = t.total, stopped_name),
                None => stopped_name.clone(),
            },
            RebaseAction::Continue => trf!("Saves your fix as {commit}, {then}.", commit, then),
            RebaseAction::Skip => trf!("Leaves out {stopped_name}, {then}.", stopped_name, then),
            RebaseAction::Abort => trf!("Puts {current} back where it was before the rebase.", current),
        };
        let head = pages::ReviewHeader {
            image: match action {
                RebaseAction::Skip => Icon::ChevronRight,
                RebaseAction::Abort => Icon::Refresh,
                _ => Icon::Branch,
            },
            title: &title,
            subtitle: &subtitle,
            // Skip and Abort discard tracked changes.
            danger: matches!(action, RebaseAction::Skip | RebaseAction::Abort),
        };
        let danger = head.danger;
        let applying = s.applying;
        let height = (ctx.content_rect().height() - 360.0).clamp(220.0, 560.0);
        let mut content = |ui: &mut Ui,
                           part: pages::Part,
                           widgets: &mut HashMap<String, UiControl>| {
            match part {
                pages::Part::Body => {
                    // Errors lead the body so a long plan or patch never hides them.
                    if let Some(error) = d.error.as_ref().or(validation.as_ref()) {
                        pages::refusal(ui, AMBER(), &too_early(error, &changes));
                    }
                    match action {
                        RebaseAction::Start => start_body(
                            ui,
                            s,
                            &mut d.first,
                            &mut d.error,
                            &branches,
                            onto.as_deref(),
                            &current,
                            widgets,
                            &mut refresh,
                        ),
                        RebaseAction::Interactive => {
                            interactive_body(
                                ui,
                                s,
                                &mut d.first,
                                &mut d.error,
                                &branches,
                                height,
                                widgets,
                                &mut refresh,
                            );
                            validation = plan_error(s);
                        }
                        _ => progress_body(
                            ui,
                            s,
                            action,
                            planned.as_ref(),
                            Paused { at, branch: &current, on_top: on_top.as_ref(), stopped: &stopped_name },
                            widgets,
                        ),
                    }
                    // Before the plan exists, its place in the list says this instead.
                    if s.task.is_some() && !(interactive && s.preview.is_none()) {
                        pages::waiting(
                            ui,
                            if s.cancelling {
                                tr("Stopping the check")
                            } else if action == RebaseAction::Start {
                                tr("Reading what will be replayed and checking for conflicts")
                            } else {
                                tr(READING)
                            },
                        );
                    }
                }
                pages::Part::Footer if s.applying => {
                    cancel = pages::running_footer(ui, widgets, &running, &[]);
                }
                pages::Part::Footer => {
                    let ready = !waiting
                        && idle
                        && s.preview.is_some()
                        && d.error.is_none()
                        && validation.is_none();
                    let label = match action {
                        RebaseAction::Start => crate::i18n::tr("Rebase"),
                        RebaseAction::Interactive if s.reviewed => tr("Start rebase"),
                        RebaseAction::Interactive => tr("Review plan"),
                        _ if discard_and_continue => tr("Discard changes and continue"),
                        _ => title.as_str(),
                    };
                    let r = if danger {
                        pages::danger(ui, ready, label)
                    } else {
                        pages::primary(ui, ready, label)
                    };
                    control(widgets, "dialog-submit", &r, ui.clip_rect());
                    submit = r.clicked();
                    if interactive && s.reviewed {
                        let r = ui.add_enabled(!waiting, Button::new(tr("Edit plan")));
                        control(widgets, "dialog-rebase-edit", &r, ui.clip_rect());
                        if r.clicked() {
                            s.reviewed = false;
                        }
                    }
                    let can_review = !waiting
                        && idle
                        && (!action.is_start() || (interactive && s.root) || !d.first.trim().is_empty());
                    // A paused rebase reviews itself; asking again is for a refusal. Stop
                    // check takes this button's place while a plan is read.
                    let first = s.preview.is_none() && d.error.is_none();
                    if (action.is_start() || d.error.is_some()) && !(interactive && first && s.task.is_some()) {
                        let r = match (first, action) {
                            (true, RebaseAction::Start) => pages::primary(ui, can_review, crate::i18n::tr("Review")),
                            (true, _) => ui.add_enabled(can_review, Button::new(crate::i18n::tr("Review"))),
                            _ => ui.add_enabled(can_review, Button::new(tr("Review again"))),
                        };
                        control(widgets, "dialog-preview-refresh", &r, ui.clip_rect());
                        refresh |= r.clicked();
                    }
                    if s.task.is_some() {
                        let r = ui.add_enabled(!s.cancelling, Button::new(tr("Stop check")));
                        control(widgets, "dialog-prediction-stop", &r, ui.clip_rect());
                        stop_check = r.clicked();
                    }
                    let status = if s.task.is_some() {
                        tr("Reviewing").to_owned()
                    } else if d.error.is_some() || validation.is_some() {
                        tr("Fix the problem above, then review again").to_owned()
                    } else if interactive && s.preview.is_some() {
                        let summary = plan_summary(&s.entries);
                        if s.reviewed {
                            trf!("{summary} · reviewed", summary)
                        } else {
                            trf!("{summary} · not reviewed", summary)
                        }
                    } else if s.preview.is_some() {
                        if action.is_start() {
                            tr("Reviewed · commit IDs will change").to_owned()
                        } else {
                            tr("Nothing has changed yet").to_owned()
                        }
                    } else if action == RebaseAction::Start {
                        tr("Review to see what the rebase will do").to_owned()
                    } else {
                        String::new()
                    };
                    pages::footer_status(ui, false, &status);
                }
            }
        };
        // The plan editor needs the full width; the other rebase reviews do not.
        let response = if interactive {
            pages::full_review(
                ctx,
                "rocket-rebase",
                head,
                &mut self.frame_widgets,
                "dialog-rebase-scroll",
                !applying,
                &mut content,
            )
        } else {
            pages::review(
                ctx,
                "rocket-rebase",
                head,
                &mut self.frame_widgets,
                "dialog-rebase-scroll",
                !applying,
                &mut content,
            )
        };
        dismiss |= response.should_close() && !s.applying;
        if cancel {
            self.cancel_write();
        }
        if stop_check && let Some(task) = &s.task {
            task.cancel();
            s.cancelling = true;
        }
        if submit {
            if interactive && !s.reviewed {
                s.reviewed = true;
            } else if let (Some(owner), Some(p)) = (d.owner, s.preview.clone()) {
                // Only a new rebase chooses `--update-refs`; one in progress keeps its own.
                let movable = p.dependents().iter().any(|b| b.moves());
                let chosen = if action.is_start() {
                    p.with_update_refs(s.update_refs && movable)
                } else {
                    Ok((*p).clone())
                };
                let reviewed = chosen
                    .and_then(|p| {
                        if interactive {
                            p.with_plan(s.entries.clone())
                        } else {
                            Ok(p)
                        }
                    })
                    .map(Arc::new);
                match reviewed {
                    Ok(p) => {
                        s.applying = true;
                        self.execute(owner, Action::Rebase(p));
                        if self.write.is_none() {
                            s.applying = false;
                            d.error=Some(tr("Could not start. Wait for the repository, then review again.").into());
                        }
                    }
                    Err(e) => d.error = Some(user_error(&e)),
                }
            }
        }
        if !dismiss {
            self.dialog = Some(d);
        }
        if refresh {
            self.start_rebase_preview();
        }
    }
}

/// Discard a review after its inputs change.
fn invalidate(s: &mut DialogState, error: &mut Option<String>) {
    s.preview = None;
    s.task = None;
    s.entries.clear();
    *error = None;
}

/// "5 commits → 3 · 1 folded · 1 left out".
fn plan_summary(entries: &[RebasePlanEntry]) -> String {
    let folded = entries
        .iter()
        .filter(|r| matches!(r.instruction, RebaseInstruction::Squash | RebaseInstruction::Fixup))
        .count();
    let dropped = entries.iter().filter(|r| r.instruction == RebaseInstruction::Drop).count();
    let kept = entries.len() - folded - dropped;
    let mut text = format!(
        "{} → {kept}",
        trn!(entries.len(), "{n} commit", "{n} commits")
    );
    if folded > 0 {
        text.push_str(&trf!(" · {folded} folded", folded));
    }
    if dropped > 0 {
        text.push_str(&trf!(" · {dropped} left out", dropped));
    }
    text
}

/// The destination field with its branch picker; `label` names it.
#[allow(clippy::too_many_arguments)]
fn target_field(
    ui: &mut Ui,
    s: &mut DialogState,
    first: &mut String,
    error: &mut Option<String>,
    branches: &[String],
    width: f32,
    widgets: &mut HashMap<String, UiControl>,
    refresh: &mut bool,
) {
    let r = ui.add_enabled(
        !s.applying,
        TextEdit::singleline(first)
            .hint_text(RichText::new(tr("main, origin/main or a commit ID")).color(FAINT()))
            .desired_width(width)
            .min_size(vec2(0.0, 30.0))
            .align(Align2::LEFT_CENTER),
    );
    control(widgets, "dialog-rebase-target", &r, ui.clip_rect());
    if r.changed() {
        invalidate(s, error);
    }
    if let Some(name) = crate::branch_picker::pick(
        ui,
        widgets,
        "dialog-rebase-target-choice",
        crate::i18n::tr("Branches"),
        branches,
        !s.applying,
    ) {
        *first = name;
        invalidate(s, error);
        *refresh = true;
    }
}

/// The chosen branch or commit as people call it: no `refs/heads/`, a short ID.
pub(crate) fn target_name(first: &str) -> String {
    let name = short_ref(first.trim().as_bytes());
    if name.len() == 40 && name.bytes().all(|b| b.is_ascii_hexdigit()) { name[..7].to_owned() } else { name }
}

/// "From feature → On top of main": the branch you're on and what you choose.
#[allow(clippy::too_many_arguments)]
fn from_and_on_top(
    ui: &mut Ui,
    s: &mut DialogState,
    first: &mut String,
    error: &mut Option<String>,
    branches: &[String],
    onto: Option<&str>,
    branch: &str,
    widgets: &mut HashMap<String, UiControl>,
    refresh: &mut bool,
) {
    let chip = (ui.available_width() * 0.34).min(196.0);
    let gap = chip + 28.0;
    let heading = |ui: &mut Ui, text: &str, width: f32| {
        let (rect, _) = ui.allocate_exact_size(vec2(width, 16.0), Sense::hover());
        ui.painter().text(rect.left_center(), Align2::LEFT_CENTER, text, FontId::proportional(12.0), MUTED());
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        heading(ui, crate::i18n::tr("From"), gap);
        heading(ui, tr("On top of"), 120.0);
    });
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (rect, _) = ui.allocate_exact_size(vec2(chip, 30.0), Sense::hover());
        let painter = ui.painter();
        painter.rect(rect, 7.0, CARD(), Stroke::new(1.0, CARD_STROKE()), StrokeKind::Inside);
        icon(painter, Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), vec2(14.0, 14.0)), Icon::Branch, BLUE());
        let count = s.preview.as_ref().filter(|p| !p.commits().is_empty()).map(|p| trn!(p.commits().len(), "{n} commit", "{n} commits"));
        let right = rect.right() - 10.0;
        let count_width = count.map_or(0.0, |text| {
            let galley = painter.layout_no_wrap(text, FontId::proportional(12.0), HINT());
            let width = galley.size().x;
            painter.galley(pos2(right - width, rect.center().y - galley.size().y / 2.0), galley, HINT());
            width + 8.0
        });
        let left = rect.left() + 32.0;
        let mut job = text::LayoutJob::simple_singleline(branch.to_owned(), semibold(13.0), TEXT());
        job.wrap = text::TextWrapping::truncate_at_width((right - count_width - left).max(30.0));
        let name = painter.layout_job(job);
        painter.galley(pos2(left, rect.center().y - name.size().y / 2.0), name, TEXT());
        let (arrow, _) = ui.allocate_exact_size(vec2(12.0, 30.0), Sense::hover());
        ui.painter().text(arrow.center(), Align2::CENTER_CENTER, "→", FontId::proportional(14.0), FAINT());
        let width = (ui.available_width() - 112.0).max(120.0);
        target_field(ui, s, first, error, branches, width, widgets, refresh);
    });
    if let Some(onto) = onto {
        ui.horizontal(|ui| {
            ui.add_space(gap);
            ui.add(Label::new(RichText::new(onto).size(12.0).color(HINT())).truncate());
        });
    }
}

/// A new rebase: where to, then what will happen and the options.
#[allow(clippy::too_many_arguments)]
fn start_body(
    ui: &mut Ui,
    s: &mut DialogState,
    first: &mut String,
    error: &mut Option<String>,
    branches: &[String],
    onto: Option<&str>,
    branch: &str,
    widgets: &mut HashMap<String, UiControl>,
    refresh: &mut bool,
) {
    from_and_on_top(ui, s, first, error, branches, onto, branch, widgets, refresh);
    if let Some(p) = s.preview.clone() {
        let target = target_name(first);
        if p.up_to_date() {
            pages::callout(ui, BLUE(), &trf!("Already on top of {target}. Nothing to move.", target));
        } else if p.fast_forward() {
            pages::callout(ui, BLUE(), tr("Fast-forward only: nothing is rewritten."));
        } else {
            let merges = p.commits().iter().filter(|c| c.parents.len() > 1).count();
            crate::tags::card(ui, tr("What will happen"), |ui| {
                picture(ui, &p, &target, branch);
                ui.add(Label::new(moves(p.commits().len(), merges, branch, &target)).wrap());
                let (color, text) = check_line(&p);
                pages::callout(ui, color, &text);
            });
            if !p.preserves_merges() {
                others_card(ui, &p, &s.entries, &mut s.update_refs, !s.applying, true, widgets);
            }
            let label = trn!(p.commits().len(), "{n} commit that moves", "{n} commits that move");
            let r = pages::disclosure(ui, s.listed, &label);
            control(widgets, "dialog-rebase-commits", &r, ui.clip_rect());
            if r.clicked() {
                s.listed = !s.listed;
            }
            if s.listed {
                pages::well(ui, |ui| {
                    for c in p.commits() {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            ui.label(RichText::new(&c.id.as_str()[..7]).monospace().size(12.5).color(MUTED()));
                            ui.add(Label::new(subject(c)).truncate());
                            if c.parents.len() > 1 {
                                ui.label(RichText::new(tr("Merge")).size(12.0).color(AMBER()));
                            }
                        });
                    }
                });
            }
        }
    }
    // Keep merge commits; closed, the title still names it.
    let label = if s.preserve_merges && !s.more { tr("More options · keep merge commits") } else { tr("More options") };
    let r = pages::disclosure(ui, s.more, label);
    control(widgets, "dialog-rebase-more", &r, ui.clip_rect());
    if r.clicked() {
        s.more = !s.more;
    }
    if s.more {
        ui.horizontal(|ui| {
            let r = ui.add_enabled(!s.applying, checkbox(&mut s.preserve_merges, tr("Keep merge commits")));
            control(widgets, "dialog-rebase-preserve-merges", &r, ui.clip_rect());
            if r.changed() {
                invalidate(s, error);
                s.update_refs = false;
                *refresh = !first.trim().is_empty();
            }
            ui.label(RichText::new(tr("keeps them as merges instead of flattening them")).size(12.0).color(FAINT()));
        });
        if s.preserve_merges {
            pages::callout(
                ui,
                AMBER(),
                tr("If a merge needed conflict fixes, you fix them again. The conflict check and moving other branches along aren't available with this option."),
            );
        }
    }
}

/// "3 commits from feature move on top of main. main doesn't change."
fn moves(n: usize, merges: usize, branch: &str, target: &str) -> text::LayoutJob {
    let plain = FontId::proportional(13.0);
    // Keep branch values out of the template until styling their named slots.
    let mut description = trn!(n, "{n} commit from {branch} moves on top of {target}. {target} doesn't change.", "{n} commits from {branch} move on top of {target}. {target} doesn't change.", branch = "{branch}", target = "{target}");
    if merges > 0 { description.push_str(&trn!(merges, " Includes {n} merge commit.", " Includes {n} merge commits.")); }
    let mut job = text::LayoutJob::default();
    let mut rest = description.as_str();
    while let Some((at, slot, value)) = [("{branch}", branch), ("{target}", target)]
        .into_iter().filter_map(|(slot, value)| rest.find(slot).map(|at| (at, slot, value)))
        .min_by_key(|(at, _, _)| *at)
    {
        job.append(&rest[..at], 0.0, TextFormat::simple(plain.clone(), TEXT()));
        job.append(value, 0.0, TextFormat::simple(semibold(13.0), TEXT()));
        rest = &rest[at + slot.len()..];
    }
    job.append(rest, 0.0, TextFormat::simple(plain, TEXT()));
    job
}

/// The conflict check in one line.
fn check_line(p: &RebasePreview) -> (Color32, String) {
    if p.preserves_merges() {
        return (
            AMBER(),
            tr("No conflict check with merges kept. If a commit stops on a conflict, Changes lists what to fix.").into(),
        );
    }
    match p.prediction() {
        Some(prediction) => match prediction.kind {
            PredictionKind::Clean => (
                GREEN(),
                trn!(prediction.total, "No conflicts expected: {n} commit applies cleanly.", "No conflicts expected: all {n} commits apply cleanly."),
            ),
            PredictionKind::Conflicts => {
                let at = prediction.stopped_at.and_then(|i| p.commits().get(i));
                (
                    AMBER(),
                    match at {
                        Some(c) => trf!("Expect a conflict at commit {position} of {total}: {commit} · {subject}. Later commits weren't checked.", position = prediction.stopped_at.unwrap_or(0) + 1, total = prediction.total, commit = &c.id.as_str()[..7], subject = subject(c)),
                        None => tr("Expect a conflict. Later commits weren't checked.").into(),
                    },
                )
            }
            PredictionKind::Empty => (
                AMBER(),
                trf!("Commit {position} of {total} would be empty here; Git may stop for Skip or Abort.", position = prediction.stopped_at.unwrap_or(0) + 1, total = prediction.total),
            ),
            PredictionKind::Unavailable => (
                MUTED(),
                prediction
                    .reason
                    .map_or(tr("Conflicts couldn't be checked; none are ruled out."), prediction_guidance)
                    .to_owned(),
            ),
        },
        None => (MUTED(), tr("Conflicts weren't checked.").into()),
    }
}

/// UI wording for the typed reason; diagnostic codes remain backend data.
pub(crate) fn prediction_guidance(reason: PredictionReason) -> &'static str {
    match reason {
        PredictionReason::MergeSettings => tr("Custom merge settings are outside this check. No merge driver was run and no clean result is claimed."),
        PredictionReason::Attributes => tr("Merge, filter or encoding attributes need the actual checkout. No external program was run and no clean result is claimed."),
        PredictionReason::AttributeChange => tr("This sequence changes .gitattributes. Review those rules before applying; the check does not simulate changing attributes."),
        PredictionReason::Submodules => tr("Submodule merge ancestry needs its own repository. The check does not predict submodule conflicts."),
        PredictionReason::ReplacedObjects => tr("This repository replaces commit objects or ancestry. The isolated check does not simulate replacements or grafts."),
        PredictionReason::Limit => tr("This check reached its size or time budget. Unchecked commits are not assumed clean."),
        PredictionReason::GitFailed => tr("Git could not complete the isolated check. Inspect diagnostics and refresh, or review the change before starting the actual operation."),
        PredictionReason::TimedOut => tr("The isolated Git check timed out. Refresh to check again; unchecked commits are not assumed clean."),
    }
}

/// What the rebase does, drawn: the commits on top of what you chose (merges kept as
/// merges with the option on), and the old copies left behind.
fn picture(ui: &mut Ui, p: &RebasePreview, target: &str, branch: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 96.0), Sense::hover());
    let painter = ui.painter_at(rect);
    let small = FontId::proportional(11.5);
    let rail = Stroke::new(2.0, WINDOW_EDGE());
    let lane = Stroke::new(2.0, BLUE());
    let n = p.commits().len().min(5);
    let left = rect.left() + 8.0;
    let (top, bottom) = (rect.top() + 26.0, rect.top() + 70.0);
    let base = pos2(left + 50.0, bottom);
    let onto = pos2(left + (rect.width() - 16.0) * 0.55, top);
    // The chosen branch's own line, then the commits on top of it.
    painter.line_segment([pos2(left, bottom), base], rail);
    painter.add(Shape::line(
        vec![base, pos2(base.x + 26.0, top), onto],
        Stroke::new(2.0, MUTED()),
    ));
    for i in 1..=3 {
        painter.circle_filled(pos2(base.x + 26.0 + (onto.x - base.x - 26.0) * i as f32 / 3.0, top), 4.5, MUTED());
    }
    let end = rect.right() - 20.0;
    painter.line_segment([onto, pos2(end, top)], lane);
    let step = (end - onto.x) / (n.max(1) as f32 + 0.5);
    for i in 1..=n {
        painter.circle_filled(pos2(onto.x + step * i as f32, top), 5.0, BLUE());
    }
    // A side branch joining again, made again as a merge: only when one of the
    // commits is a merge.
    let merges = p.preserves_merges() && p.commits().iter().any(|c| c.parents.len() > 1);
    let loop_start = onto.x + step * 0.8;
    if merges {
        let b = onto.x + step * (n as f32).max(1.6);
        painter.add(Shape::line(
            vec![
                pos2(loop_start, top),
                pos2(loop_start + 18.0, top + 22.0),
                pos2(b - 18.0, top + 22.0),
                pos2(b, top),
            ],
            lane,
        ));
        painter.circle_stroke(pos2(b, top), 7.0, Stroke::new(2.0, AMBER()));
    }
    // The old copies, dashed, stay findable in Reflog & recovery.
    let old_end = (base.x + 40.0 + 30.0 * n as f32).min(rect.center().x);
    painter.extend(Shape::dashed_line(&[base, pos2(old_end, bottom)], rail, 4.0, 4.0));
    for i in 1..=n {
        painter.circle_filled(pos2(base.x + 30.0 * i as f32, bottom), 3.5, RING_IDLE());
    }
    // Each name over its part of the line, cut short rather than running together.
    let over = |text: String, font: FontId, color: Color32, right: f32, room: f32| {
        let mut job = text::LayoutJob::simple_singleline(text, font, color);
        job.wrap = text::TextWrapping::truncate_at_width(room.max(40.0));
        let name = painter.layout_job(job);
        painter.galley(pos2(right - name.size().x, top - 15.0 - name.size().y / 2.0), name, color);
    };
    over(trf!("{target} stays here", target), FontId::proportional(12.0), HINT(), onto.x + 18.0, onto.x + 18.0 - left);
    over(trf!("{branch} ends here", branch), semibold(12.0), TEXT(), end + 8.0, end - onto.x - 30.0);
    painter.text(
        pos2(old_end + 10.0, bottom),
        Align2::LEFT_CENTER,
        tr("old copies stay in Reflog & Recovery"),
        small,
        HINT(),
    );
}

/// Other branches pointing into the replayed commits: for a new rebase, whether
/// they move along (`--update-refs`), each with where it ends up.
fn others_card(
    ui: &mut Ui,
    p: &RebasePreview,
    entries: &[RebasePlanEntry],
    on: &mut bool,
    enabled: bool,
    start: bool,
    widgets: &mut HashMap<String, UiControl>,
) {
    let dependents = p.dependents();
    if dependents.is_empty() {
        return;
    }
    let movable = dependents.iter().any(|b| b.moves());
    // The rows show the choice as it stood when this frame began.
    let moving = *on;
    pages::titled_card(
        ui,
        |ui| {
            ui.label(
                RichText::new(if start {
                    tr("Other branches on these commits")
                } else {
                    tr("Branches this rebase moves when it finishes")
                })
                .size(12.0)
                .color(MUTED())
                .strong(),
            );
            if start {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let r = ui.add_enabled(enabled && movable, checkbox(on, tr("Move them along")));
                    control(widgets, "dialog-rebase-update-refs", &r, ui.clip_rect());
                });
            }
        },
        |ui| {
            for b in dependents {
                let commit = p.commits().iter().find(|c| c.id == b.target);
                let (fate, color) = match &b.checked_out_at {
                    Some(_) => (tr("Stays: in use").to_owned(), AMBER()),
                    None if !start => (tr("Moves when it finishes").to_owned(), BLUE()),
                    None if moving => (
                        match planned_move(entries, &b.target) {
                            "moves to the combined commit" => tr("Moves to the combined commit"),
                            "moves to the commit before it (its commit is dropped)" => tr("Moves to the commit before it (its commit is dropped)"),
                            _ => tr("Moves along"),
                        }.to_owned(),
                        BLUE(),
                    ),
                    None => (tr("Stays behind").to_owned(), MUTED()),
                };
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let pill_width = ui
                        .painter()
                        .layout_no_wrap(fate.clone(), semibold(11.0), color)
                        .size()
                        .x
                        + 24.0;
                    ui.vertical(|ui| {
                        ui.set_width((ui.available_width() - pill_width).max(80.0));
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.add(Label::new(short_ref(&b.reference)).truncate());
                        let detail = match (&b.checked_out_at, commit) {
                            (Some(path), _) => trf!("checked out in {folder}", folder = crate::views::display_path(path)),
                            (None, Some(c)) => format!("{} · {}", &b.target.as_str()[..7], subject(c)),
                            (None, None) => b.target.as_str()[..7].to_owned(),
                        };
                        ui.add(Label::new(RichText::new(detail).size(12.0).color(HINT())).truncate());
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        pages::pill(ui, &fate, color);
                    });
                });
            }
        },
    );
}

/// An interactive rebase: the base, then the plan beside the selected row's editor,
/// or once reviewed, the plan beside the history it makes. Both panes show from the first
/// frame, so the dialog keeps its shape and nothing moves when the plan arrives.
#[allow(clippy::too_many_arguments)]
fn interactive_body(
    ui: &mut Ui,
    s: &mut DialogState,
    first: &mut String,
    error: &mut Option<String>,
    branches: &[String],
    height: f32,
    widgets: &mut HashMap<String, UiControl>,
    refresh: &mut bool,
) {
    ui.horizontal(|ui| {
        ui.set_min_height(30.0);
        if s.reviewed {
            let base = if s.root { tr("From the first commit").to_owned() } else { trf!("On top of {target}", target = target_name(first)) };
            ui.label(RichText::new(base).color(MUTED()));
        } else {
            if !s.root {
                ui.label(RichText::new(tr("On top of")).color(MUTED()));
                // The field takes what Branches and the checkbox leave.
                let width = (ui.available_width() - 260.0).max(200.0);
                target_field(ui, s, first, error, branches, width, widgets, refresh);
            }
            let r = ui.add_enabled(!s.applying, checkbox(&mut s.root, tr("From the first commit")));
            control(widgets, "dialog-rebase-root", &r, ui.clip_rect());
            if r.changed() {
                invalidate(s, error);
                *refresh = s.root || !first.trim().is_empty();
            }
        }
    });
    let preview = s.preview.clone();
    let refused = error.is_some();
    let order = if s.root {
        tr("Oldest first").to_owned()
    } else {
        let target = target_name(first);
        trf!("Oldest first. The commits listed are the ones {target} doesn't have; they go on top of {target}.", target)
    };
    // Room for the other branches' card under the plan: the gap above it, its title and
    // up to four rows. The panes give that up, so the dialog keeps its size.
    let others = preview.as_ref().map_or(0, |p| p.dependents().len());
    let height = if others == 0 { height } else { (height - 67.0 - 40.0 * others.min(4) as f32).max(200.0) };
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let list_width = (ui.available_width() - PANE - 17.0).max(300.0);
        ui.vertical(|ui| {
            ui.set_width(list_width);
            let list = ScrollArea::vertical()
                .id_salt("dialog-rebase-plan")
                .max_height(height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let Some(p) = &preview else {
                        plan_placeholder(ui, s, &order, refused);
                        return;
                    };
                    ui.add(Label::new(RichText::new(&order).size(12.0).color(HINT())).truncate());
                    plan_rows(ui, s, p, widgets);
                    if s.reviewed {
                        reviewed_notes(ui, s);
                    }
                });
            pages::region(widgets, "dialog-rebase-plan-scroll", list.inner_rect.intersect(ui.clip_rect()), !s.applying);
        });
        ui.add_space(8.0);
        let (line, _) = ui.allocate_exact_size(vec2(1.0, height), Sense::hover());
        ui.painter().vline(line.center().x, line.y_range(), Stroke::new(1.0, HAIRLINE()));
        ui.add_space(8.0);
        ui.vertical(|ui| {
            ui.set_width(PANE);
            let pane = ScrollArea::vertical()
                .id_salt("dialog-rebase-pane")
                .max_height(height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_width(ui.available_width() - 6.0);
                    ui.spacing_mut().item_spacing.y = 8.0;
                    match &preview {
                        None => {
                            ui.add_space((height / 2.0 - 12.0).max(0.0));
                            ui.vertical_centered(|ui| {
                                ui.label(RichText::new(tr("Choose a commit to change it")).size(12.0).color(FAINT()));
                            });
                        }
                        Some(p) if s.reviewed => result_pane(ui, s, p),
                        Some(p) => {
                            ui.add_enabled_ui(!s.applying, |ui| row_editor(ui, s, p, widgets));
                        }
                    }
                });
            pages::region(widgets, "dialog-rebase-pane-scroll", pane.inner_rect.intersect(ui.clip_rect()), !s.applying);
        });
    });
    if let Some(p) = &preview {
        others_card(ui, p, &s.entries, &mut s.update_refs, !s.applying && !s.reviewed, true, widgets);
    }
}

/// The plan's place before there's a plan: where its rows will land, and why they aren't
/// there yet.
fn plan_placeholder(ui: &mut Ui, s: &DialogState, order: &str, refused: bool) {
    if s.task.is_some() {
        ui.add(Label::new(RichText::new(order).size(12.0).color(HINT())).truncate());
        ui.spacing_mut().item_spacing.y = 3.0;
        for _ in 0..3 {
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
            ui.painter().rect_filled(rect, 8.0, HOVER());
        }
        ui.add_space(8.0);
        pages::waiting(ui, if s.cancelling { tr("Stopping the check") } else { tr(READING) });
    } else if !refused {
        pages::note(ui, tr("Choose what they go on top of, or the first commit."));
    }
}

/// The action's colors: the pill's tint and its text.
fn action_tint(instruction: RebaseInstruction) -> Color32 {
    instruction_color(instruction)
}

/// The plan, a row per commit: grip, number, action, commit and subject, with what
/// happens after it; Up and Down move it while editing.
fn plan_rows(ui: &mut Ui, s: &mut DialogState, p: &RebasePreview, widgets: &mut HashMap<String, UiControl>) {
    let mut movement = None;
    let mut chosen = None;
    let editing = !s.reviewed;
    ui.spacing_mut().item_spacing.y = 3.0;
    let count = s.entries.len();
    for i in 0..count {
        let entry = &s.entries[i];
        let selected = editing && s.selected == i;
        let commit = p.commits().iter().find(|c| c.id == entry.commit);
        let subject = if entry.instruction == RebaseInstruction::Reword && !entry.message.trim().is_empty() {
            entry.message.lines().next().unwrap_or("").to_owned()
        } else {
            commit.map(subject).unwrap_or_default()
        };
        let folds = matches!(entry.instruction, RebaseInstruction::Squash | RebaseInstruction::Fixup);
        let note = if folds && i > 0 {
            Some((trf!("into row {i}", i), MUTED()))
        } else if entry.exec.is_some() {
            Some((tr("then runs a command").to_owned(), AMBER()))
        } else if entry.stop_after || entry.instruction == RebaseInstruction::Edit {
            Some((tr("pauses").to_owned(), BLUE()))
        } else {
            None
        };
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
        let painter = ui.painter();
        if selected {
            painter.rect_filled(rect, 8.0, SELECTED());
            painter.rect_filled(
                Rect::from_min_size(rect.min + vec2(0.0, 8.0), vec2(3.0, rect.height() - 16.0)),
                1.5,
                BLUE(),
            );
        } else if editing && (response.hovered() || response.has_focus()) {
            painter.rect_filled(rect, 8.0, HOVER());
        }
        let y = rect.center().y;
        let mut x = rect.left() + 12.0;
        if editing {
            for (dx, dy) in [(0.0, -4.0), (4.0, -4.0), (0.0, 0.0), (4.0, 0.0), (0.0, 4.0), (4.0, 4.0)] {
                painter.circle_filled(pos2(x + dx, y + dy), 1.2, FAINT());
            }
            x += 14.0;
        }
        painter.text(pos2(x + 14.0, y), Align2::RIGHT_CENTER, (i + 1).to_string(), FontId::proportional(12.0), HINT());
        x += 22.0;
        let tint = action_tint(entry.instruction);
        let label = instruction_label(entry.instruction);
        let label_width = painter.layout_no_wrap(label.to_owned(), semibold(12.0), tint).size().x;
        let pill = Rect::from_min_size(pos2(x, y - 10.0), vec2((label_width + 14.0).max(62.0), 20.0));
        painter.rect_filled(pill, 6.0, tint.gamma_multiply(0.14));
        painter.text(pill.center(), Align2::CENTER_CENTER, label, semibold(12.0), tint);
        x = pill.right() + 10.0;
        let sha = painter.layout_no_wrap(entry.commit.as_str()[..7].to_owned(), FontId::monospace(12.5), MUTED());
        let sha_width = sha.size().x;
        painter.galley(pos2(x, y - sha.size().y / 2.0), sha, MUTED());
        x += sha_width + 10.0;
        let right = rect.right() - if editing { 62.0 } else { 10.0 };
        let note_galley = note.as_ref().map(|(text, color)| painter.layout_no_wrap(text.clone(), FontId::proportional(11.5), *color));
        let note_width = note_galley.as_ref().map_or(0.0, |g| g.size().x + 14.0);
        let drop = entry.instruction == RebaseInstruction::Drop;
        let mut job = text::LayoutJob::simple_singleline(
            subject.clone(),
            FontId::proportional(13.0),
            if drop { MUTED() } else { TEXT() },
        );
        if drop {
            job.sections[0].format.strikethrough = Stroke::new(1.0, MUTED());
        }
        job.wrap = text::TextWrapping::truncate_at_width((right - note_width - 8.0 - x).max(30.0));
        let title = painter.layout_job(job);
        painter.galley(pos2(x, y - title.size().y / 2.0), title, TEXT());
        if let (Some(galley), Some((_, color))) = (note_galley, &note) {
            let pill = Rect::from_min_size(pos2(right - note_width, y - 9.0), vec2(note_width, 18.0));
            painter.rect_filled(pill, 9.0, color.gamma_multiply(0.14));
            painter.galley(pos2(pill.left() + 7.0, pill.center().y - galley.size().y / 2.0), galley, *color);
        }
        control(widgets, format!("dialog-rebase-row-{i}"), &response, ui.clip_rect());
        control(widgets, format!("dialog-rebase-action-{i}"), &response, ui.clip_rect());
        if editing && response.clicked() {
            chosen = Some(i);
        }
        if editing {
            let up = Rect::from_min_size(pos2(rect.right() - 56.0, y - 12.0), vec2(24.0, 24.0));
            let down = Rect::from_min_size(pos2(rect.right() - 30.0, y - 12.0), vec2(24.0, 24.0));
            let r = ui.put(up, |ui: &mut Ui| crate::chrome::icon_button(ui, Icon::ChevronUp, i > 0, false, 24.0));
            control(widgets, format!("dialog-rebase-up-{i}"), &r, ui.clip_rect());
            if r.clicked() {
                movement = Some((i, i - 1));
            }
            let r = ui.put(down, |ui: &mut Ui| crate::chrome::icon_button(ui, Icon::Chevron, i + 1 < count, false, 24.0));
            control(widgets, format!("dialog-rebase-down-{i}"), &r, ui.clip_rect());
            if r.clicked() {
                movement = Some((i, i + 1));
            }
        }
    }
    if let Some(i) = chosen {
        s.selected = i;
    }
    if let Some((a, b)) = movement {
        s.entries.swap(a, b);
        s.selected = b;
        s.reviewed = false;
    }
}

/// The selected row: what to do with it, its message, and what runs after it.
fn row_editor(ui: &mut Ui, s: &mut DialogState, p: &RebasePreview, widgets: &mut HashMap<String, UiControl>) {
    let Some(entry) = s.entries.get_mut(s.selected) else {
        return;
    };
    let commit = p.commits().iter().find(|c| c.id == entry.commit);
    ui.label(
        RichText::new(trf!("Row {row} · {commit}", row = s.selected + 1, commit = &entry.commit.as_str()[..7]))
            .size(12.0)
            .color(HINT()),
    );
    ui.add(Label::new(RichText::new(commit.map(subject).unwrap_or_default()).font(semibold(16.0))).wrap());
    ui.label(RichText::new(tr("Do")).size(12.0).color(MUTED()));
    let actions = [
        RebaseInstruction::Pick,
        RebaseInstruction::Reword,
        RebaseInstruction::Edit,
        RebaseInstruction::Squash,
        RebaseInstruction::Fixup,
        RebaseInstruction::Drop,
    ];
    let old = entry.instruction;
    for row in actions.chunks(3) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let width = ((ui.available_width() - 8.0) / 3.0).floor();
            for action in row {
                let on = entry.instruction == *action;
                let tint = action_tint(*action);
                let label = instruction_label(*action);
                let r = ui.add(
                    Button::new(RichText::new(label).strong().color(if on { tint } else { MUTED() }))
                        .fill(if on { tint.gamma_multiply(0.14) } else { Color32::TRANSPARENT })
                        .stroke(Stroke::new(1.0, if on { tint } else { CARD_STROKE() }))
                        .min_size(vec2(width, 28.0)),
                );
                control(
                    widgets,
                    format!("dialog-rebase-action-{}-{}", s.selected, action.label()),
                    &r,
                    ui.clip_rect(),
                );
                if r.clicked() {
                    entry.instruction = *action;
                }
            }
        });
    }
    ui.label(
        RichText::new(match entry.instruction {
            RebaseInstruction::Pick => tr("Keep this commit as it is."),
            RebaseInstruction::Reword => tr("Keep the changes and write a new message."),
            RebaseInstruction::Edit => tr("Stop after this commit so you can change it."),
            RebaseInstruction::Squash => tr("Fold into the row above and combine both messages."),
            RebaseInstruction::Fixup => tr("Fold into the row above and keep only its message."),
            RebaseInstruction::Drop => tr("Leave this commit out."),
        })
        .size(12.0)
        .color(HINT()),
    );
    if entry.instruction != old {
        s.reviewed = false;
        if entry.instruction.needs_message() && entry.message.is_empty() {
            entry.message = commit
                .map(|c| String::from_utf8_lossy(&c.message).into_owned())
                .unwrap_or_default();
        }
    }
    if entry.instruction.needs_message() {
        ui.label(
            RichText::new(if entry.instruction == RebaseInstruction::Squash {
                tr("Combined message")
            } else {
                tr("New message")
            })
            .size(12.0)
            .color(MUTED()),
        );
        let r = ui.add(
            TextEdit::multiline(&mut entry.message)
                .desired_width(f32::INFINITY)
                .desired_rows(4)
                .char_limit(64 * 1024),
        );
        control(widgets, "dialog-rebase-message", &r, ui.clip_rect());
        if r.changed() {
            s.reviewed = false;
        }
    }
    pages::divider(ui);
    ui.label(RichText::new(tr("After this commit")).size(12.0).color(MUTED()));
    let r = ui.add(checkbox(&mut entry.stop_after, tr("Pause so I can look or test")));
    control(widgets, "dialog-rebase-stop", &r, ui.clip_rect());
    if r.changed() {
        s.reviewed = false;
    }
    ui.label(RichText::new(tr("Run a command · optional")).size(12.0).color(HINT()));
    let mut command = entry.exec.clone().unwrap_or_default();
    let r = ui.add(
        TextEdit::singleline(&mut command)
            .font(TextStyle::Monospace)
            .hint_text(RichText::new(tr("For example: cargo test")).color(FAINT()))
            .char_limit(MAX_REBASE_COMMAND)
            .desired_width(f32::INFINITY)
            .min_size(vec2(0.0, 28.0))
            .align(Align2::LEFT_CENTER),
    );
    control(widgets, "dialog-rebase-exec", &r, ui.clip_rect());
    if r.changed() {
        entry.exec = (!command.is_empty()).then_some(command.clone());
        s.reviewed = false;
    }
    if command.trim() != command {
        pages::note(ui, tr("Remove spaces at the start or end of the command."));
    } else if !command.is_empty() {
        pages::note(ui, tr("Git runs it with its shell at the repository root."));
    }
}

/// Once reviewed: the commits the new history has, and where each comes from.
fn result_pane(ui: &mut Ui, s: &DialogState, p: &RebasePreview) {
    let kept = s
        .entries
        .iter()
        .filter(|r| !matches!(r.instruction, RebaseInstruction::Squash | RebaseInstruction::Fixup | RebaseInstruction::Drop))
        .count();
    ui.label(
        RichText::new(trn!(kept, "Result · {n} commit", "Result · {n} commits"))
            .font(semibold(16.0)),
    );
    let subject_of = |entry: &RebasePlanEntry| {
        if entry.instruction.needs_message() && !entry.message.trim().is_empty() {
            entry.message.lines().next().unwrap_or("").to_owned()
        } else {
            p.commits().iter().find(|c| c.id == entry.commit).map(subject).unwrap_or_default()
        }
    };
    let mut i = 0;
    while i < s.entries.len() {
        let entry = &s.entries[i];
        if entry.instruction == RebaseInstruction::Drop {
            i += 1;
            continue;
        }
        // Rows folded into this one follow it.
        let mut from = vec![entry.commit.as_str()[..7].to_owned()];
        let mut title = subject_of(entry);
        let mut j = i + 1;
        while j < s.entries.len()
            && matches!(s.entries[j].instruction, RebaseInstruction::Squash | RebaseInstruction::Fixup)
        {
            from.push(s.entries[j].commit.as_str()[..7].to_owned());
            if s.entries[j].instruction == RebaseInstruction::Squash && !s.entries[j].message.trim().is_empty() {
                title = s.entries[j].message.lines().next().unwrap_or("").to_owned();
            }
            j += 1;
        }
        let last = &s.entries[j - 1];
        let mut detail = trf!("from {commits}", commits = from.join(" + "));
        if entry.instruction == RebaseInstruction::Reword || from.len() > 1 {
            detail.push_str(" · new message");
        } else if entry.instruction == RebaseInstruction::Pick {
            detail.push_str(" · unchanged");
        }
        if let Some(command) = &last.exec {
            detail.push_str(&trf!(" · then runs {command}", command));
        }
        if last.stop_after || entry.instruction == RebaseInstruction::Edit {
            detail.push_str(" · pauses");
        }
        Frame::new()
            .fill(WINDOW())
            .stroke(Stroke::new(1.0, HAIRLINE()))
            .corner_radius(8)
            .inner_margin(Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(Label::new(RichText::new(title).strong()).wrap());
                ui.add(Label::new(RichText::new(detail).size(12.0).color(HINT())).wrap());
            });
        i = j;
    }
    pages::note(ui, tr("Commit IDs change. Branches moving along follow their commits."));
}

/// Under the reviewed plan: commands that will run, pauses, and left-out commits.
fn reviewed_notes(ui: &mut Ui, s: &DialogState) {
    ui.add_space(8.0);
    let commands: Vec<(usize, &str)> = s
        .entries
        .iter()
        .enumerate()
        .filter_map(|(i, r)| r.exec.as_deref().map(|c| (i, c)))
        .collect();
    for (i, command) in &commands {
        pages::callout(
            ui,
            RED(),
            &trf!("After row {row}, Git runs {command} with your permissions at the repository root. Start only if you trust it.", row = i + 1, command),
        );
    }
    // An edit row stops once; a break after it stops again.
    let stops: usize = s
        .entries
        .iter()
        .map(|r| usize::from(r.stop_after) + usize::from(r.instruction == RebaseInstruction::Edit))
        .sum();
    if stops > 0 {
        pages::note(
            ui,
            &trn!(stops, "Git pauses {n} time as planned.", "Git pauses {n} times as planned."),
        );
    }
    let dropped = s.entries.iter().filter(|r| r.instruction == RebaseInstruction::Drop).count();
    if dropped > 0 {
        pages::callout(
            ui,
            AMBER(),
            &trn!(dropped, "{n} commit is left out of the branch. Reflog & recovery can still find it.", "{n} commits are left out of the branch. Reflog & recovery can still find them."),
        );
    }
}

/// A Continue that came too early, naming what's still in conflict. Other refusals
/// stay as they are.
fn too_early(error: &str, changes: &[(String, &'static str)]) -> String {
    if !error.contains(RebaseFailure::ConflictsRemain.code()) {
        return error.to_owned();
    }
    let conflicts: Vec<&str> = changes
        .iter()
        .filter(|(_, state)| state.starts_with("conflict"))
        .map(|(path, _)| path.rsplit('/').next().unwrap_or(path))
        .collect();
    match conflicts.as_slice() {
        [] => error.to_owned(),
        [name] => trf!("{name} still has a conflict\nFix it in Changes and stage it, then Continue. Nothing has changed.", name),
        many => trn!(many.len(), "{n} file still has conflicts\nFix it in Changes and stage it, then Continue. Nothing has changed.", "{n} files still have conflicts\nFix them in Changes and stage them, then Continue. Nothing has changed."),
    }
}

/// What a paused rebase's reviews name.
struct Paused<'a> {
    /// The commit it stopped at, from 1; 0 when Git doesn't say.
    at: usize,
    branch: &'a str,
    /// What it goes on top of: a branch's name if one points there, and the short ID.
    on_top: Option<&'a (Option<String>, String)>,
    /// The stopped commit's subject, as Git's state gives it.
    stopped: &'a str,
}

/// The commits of a paused rebase in plan order: done, stopped, then still to come.
fn step_rows(p: &RebasePreview, paused: &Paused, mark: StepMark, note: &'static str) -> Vec<Step> {
    let at = paused.at;
    let subject_of = |id: &ObjectId| p.commits().iter().find(|c| c.id == *id).map(subject).unwrap_or_else(|| id.as_str()[..7].to_owned());
    // Git's order with merges kept is its own plan, which the review doesn't list.
    if p.preserves_merges() {
        let subject = p.stopped().map_or_else(|| paused.stopped.to_owned(), subject_of);
        return vec![Step { mark, number: at.max(1), subject, note }];
    }
    let rows: Vec<String> = match p.plan() {
        Some(plan) => plan.iter().filter(|e| e.instruction != RebaseInstruction::Drop).map(|e| subject_of(&e.commit)).collect(),
        None => p.commits().iter().map(subject).collect(),
    };
    rows.into_iter()
        .enumerate()
        .map(|(i, subject)| {
            let number = i + 1;
            let (mark, note) = match number.cmp(&at) {
                std::cmp::Ordering::Less => (StepMark::Done, tr("Done")),
                std::cmp::Ordering::Equal => (mark, note),
                std::cmp::Ordering::Greater => (StepMark::Waiting, ""),
            };
            Step { mark, number, subject, note }
        })
        .collect()
}

/// A rebase in progress: Continue, Skip or Abort, with the stop and what they change.
fn progress_body(
    ui: &mut Ui,
    s: &mut DialogState,
    action: RebaseAction,
    planned: Option<&RebaseStop>,
    paused: Paused,
    widgets: &mut HashMap<String, UiControl>,
) {
    let Some(p) = s.preview.clone() else {
        return;
    };
    ui.spacing_mut().item_spacing.y = 12.0;
    match action {
        RebaseAction::Continue => {
            let note = if planned.is_some() { tr("stopped here") } else { tr("your fix") };
            pages::steps(ui, "dialog-rebase", &step_rows(&p, &paused, StepMark::Stopped, note))
        }
        RebaseAction::Skip if planned.is_some() => {
            pages::steps(ui, "dialog-rebase", &step_rows(&p, &paused, StepMark::Stopped, tr("stays")))
        }
        RebaseAction::Skip => pages::steps(ui, "dialog-rebase", &step_rows(&p, &paused, StepMark::LeftOut, tr("left out"))),
        _ => {
            ui.label(RichText::new(trf!("{branch} goes back to", branch = paused.branch)).size(12.0).color(HINT()));
            let tip = p.commits().iter().find(|c| c.id == *p.original());
            pages::commit_card(
                ui,
                &tip.map_or_else(|| tr("Your branch before the rebase").to_owned(), subject),
                &p.original().as_str()[..7],
                tr("before the rebase"),
            );
        }
    }
    if action != RebaseAction::Abort {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let word = |ui: &mut Ui, text: &str, strong: bool| {
                let text = RichText::new(text).size(12.0);
                ui.label(if strong { text.strong().color(TEXT()) } else { text.color(HINT()) });
            };
            word(ui, crate::i18n::tr("From"), false);
            word(ui, paused.branch, true);
            ui.add_space(10.0);
            match paused.on_top {
                Some((Some(name), id)) => {
                    word(ui, tr("On top of"), false);
                    word(ui, name, true);
                    ui.label(RichText::new(format!("· {id}")).size(12.0).monospace().color(HINT()));
                }
                Some((None, id)) => {
                    word(ui, tr("On top of"), false);
                    ui.label(RichText::new(id).size(12.0).monospace().color(HINT()));
                }
                None => word(ui, tr("From the first commit: the first kept commit becomes the new root"), false),
            }
        });
    }
    if !p.preserves_merges() {
        others_card(ui, &p, &s.entries, &mut s.update_refs, !s.applying, false, widgets);
    }
    if let Some(stop) = planned
        && action != RebaseAction::Abort
    {
        let (color, text) = planned_stop_guidance(stop, action);
        pages::callout(ui, color, &text);
    }
    if let Some((retry, message)) = p.message_recovery() {
        pages::callout(
            ui,
            AMBER(),
            if retry {
                tr("Retry saving the reviewed message on the current replayed commit, then resume the plan.")
            } else {
                tr("The reviewed message was already saved before interruption. Continue will resume the remaining plan.")
            },
        );
        pages::well(ui, |ui| {
            ui.add(Label::new(message.chars().take(16000).collect::<String>()).wrap());
        });
        if message.chars().count() > 16000 {
            pages::callout(ui, AMBER(), tr("Showing the first 16,000 characters of the message."));
        }
    }
    if let Some(plan) = p.plan() {
        ui.collapsing(tr("The plan"), |ui| {
            for entry in plan {
                ui.label(
                    RichText::new(format!("{} {}", instruction_label(entry.instruction), &entry.commit.as_str()[..9]))
                        .monospace(),
                );
                for (text, color) in row_extras(entry) {
                    ui.add(Label::new(RichText::new(format!("  {text}")).monospace().color(color)).wrap());
                }
            }
        });
    }
    if action == RebaseAction::Continue {
        let title = match planned {
            Some(RebaseStop::Edit(_)) => tr("Staged changes to amend into this commit").to_owned(),
            Some(_) => tr("Current changes").to_owned(),
            None => trn!(s.fix.len(), "Your fix · {n} file staged", "Your fix · {n} files staged"),
        };
        if s.fix.is_empty() {
            pages::note(ui, tr("No changes to save here. The rebase moves on."));
        } else {
            series_view::diff_card(ui, &title, "dialog-rebase-fix", &s.fix, 160.0);
        }
    } else if p.changes().is_empty() {
        pages::note(
            ui,
            if action == RebaseAction::Abort {
                tr("Reflog & Recovery can still find the original commits.")
            } else {
                tr("There are no edits to discard.")
            },
        );
    } else {
        if action == RebaseAction::Abort {
            let edits = match p.changes() {
                [(path, _)] => path.rsplit('/').next().unwrap_or(path).to_owned(),
                files => trn!(files.len(), "{n} file", "{n} files"),
            };
            pages::callout(ui, AMBER(), &trf!("Your edits to {edits} are discarded. Reflog & Recovery can still find the original commits.", edits));
        } else {
            discarded(ui, p.changes());
        }
        let r = pages::disclosure(ui, s.edits, tr("The edits that go"));
        control(widgets, "dialog-rebase-edits", &r, ui.clip_rect());
        if r.clicked() {
            s.edits = !s.edits;
        }
        if s.edits {
            let text = String::from_utf8_lossy(p.patch());
            pages::well(ui, |ui| {
                ui.add(Label::new(RichText::new(text.chars().take(16000).collect::<String>()).monospace()).wrap());
            });
            if text.chars().count() > 16000 {
                pages::callout(ui, AMBER(), tr("Showing the first 16,000 characters."));
            }
        }
    }
}

/// The tracked files Skip throws away, as the review saw them.
fn discarded(ui: &mut Ui, changes: &[(String, bool)]) {
    pages::titled_card(
        ui,
        |ui| {
            ui.label(RichText::new(tr("Discarded")).size(12.0).color(MUTED()).strong());
        },
        |ui| {
            for (path, conflict) in changes {
                let conflict = *conflict;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 9.0;
                    let (mark, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                    ui.painter().rect_filled(mark, 5.0, if conflict { RED() } else { RAISED() });
                    ui.painter().text(
                        mark.center(),
                        Align2::CENTER_CENTER,
                        if conflict { "C" } else { "M" },
                        semibold(11.0),
                        if conflict { RED_INK() } else { TEXT() },
                    );
                    let (folder, name) = path.rsplit_once('/').map_or(("", path.as_str()), |(f, n)| (f, n));
                    let mut job = text::LayoutJob::default();
                    if !folder.is_empty() {
                        job.append(&format!("{folder}/"), 0.0, TextFormat::simple(FontId::proportional(13.0), HINT()));
                    }
                    job.append(name, 0.0, TextFormat::simple(FontId::proportional(13.0), TEXT()));
                    job.wrap = text::TextWrapping::truncate_at_width((ui.available_width() - 150.0).max(60.0));
                    ui.label(job);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(if conflict { tr("conflict, not fixed yet") } else { tr("changed") }).size(12.0).color(HINT()));
                    });
                });
            }
        },
    );
}

/// Where a moving branch ends up under an interactive plan (empty for an ordinary
/// rebase), as rocket-git places its `update-ref` line: after its commit's row, or
/// after the squash/fixup rows folded into it.
fn planned_move(entries: &[RebasePlanEntry], target: &ObjectId) -> &'static str {
    let folds = |r: &RebasePlanEntry| {
        matches!(
            r.instruction,
            RebaseInstruction::Squash | RebaseInstruction::Fixup
        )
    };
    match entries.iter().position(|r| r.commit == *target) {
        Some(row) if entries[row].instruction == RebaseInstruction::Drop => {
            "moves to the commit before it (its commit is dropped)"
        }
        Some(row) if folds(&entries[row]) || entries.get(row + 1).is_some_and(folds) => {
            "moves to the combined commit"
        }
        _ => "moves to its rewritten commit",
    }
}
fn plan_error(s: &DialogState) -> Option<String> {
    let p = s.preview.as_ref()?;
    p.validate_plan(&s.entries).err().map(|e| user_error(&e))
}
fn instruction_color(instruction: RebaseInstruction) -> Color32 {
    match instruction {
        RebaseInstruction::Pick => GREEN(),
        RebaseInstruction::Reword => BLUE(),
        RebaseInstruction::Edit => AMBER(),
        RebaseInstruction::Squash | RebaseInstruction::Fixup => VIOLET(),
        RebaseInstruction::Drop => RED(),
    }
}
fn instruction_label(instruction: RebaseInstruction) -> &'static str {
    match instruction {
        RebaseInstruction::Pick => tr("Pick"),
        RebaseInstruction::Reword => tr("Reword"),
        RebaseInstruction::Edit => tr("Edit"),
        RebaseInstruction::Squash => tr("Squash"),
        RebaseInstruction::Fixup => tr("Fixup"),
        RebaseInstruction::Drop => tr("Drop"),
    }
}
/// What Git does after a row: a planned break and/or a shell command.
fn row_extras(entry: &RebasePlanEntry) -> Vec<(String, Color32)> {
    let mut extras = Vec::new();
    if let Some(command) = &entry.exec {
        extras.push((
            format!("exec {}", command.chars().take(240).collect::<String>()),
            AMBER(),
        ));
    }
    if entry.stop_after {
        extras.push(("break".into(), BLUE()));
    }
    extras
}
/// Guidance for the stop the reviewed plan asked for, shown in its recovery review.
fn planned_stop_guidance(stop: &RebaseStop, action: RebaseAction) -> (Color32, String) {
    if action == RebaseAction::Skip {
        return (
            AMBER(),
            tr("Git stopped here as planned, so Skip keeps the commits and discards only the tracked changes made since the stop, then continues the plan.").into(),
        );
    }
    match stop {
        RebaseStop::Edit(id) => (
            BLUE(),
            trf!("Stopped as planned to edit {commit}. Stage changes to amend them into this commit, or continue with nothing staged to keep it unchanged.", commit = &id.as_str()[..9]),
        ),
        RebaseStop::Break => (
            BLUE(),
            tr("Paused at a planned break. Inspect or test the result, then continue the plan. Staged changes are refused here; Skip discards them.").into(),
        ),
        RebaseStop::Command(command) => (
            AMBER(),
            trf!("Git stopped at the planned command `{command}`: it failed or left changes. Continue resumes with the next step and does not run the command again.", command = command.chars().take(240).collect::<String>()),
        ),
    }
}
fn title(action: RebaseAction) -> &'static str {
    match action {
        RebaseAction::Start => tr("Rebase branch"),
        RebaseAction::Interactive => crate::i18n::tr("Interactive rebase"),
        RebaseAction::Continue => crate::i18n::tr("Continue rebase"),
        RebaseAction::Skip => crate::i18n::tr("Skip commit"),
        RebaseAction::Abort => crate::i18n::tr("Abort rebase"),
    }
}
/// Shared by ordinary rebase and pull-with-rebase; the list is the pinned replay.
pub(crate) fn prediction_ui(ui: &mut Ui, prediction: &CommitPrediction, commits: &[Commit]) {
    crate::commit_actions::prediction_ui(ui, prediction);
    if let Some(commit) = prediction.stopped_at.and_then(|i| commits.get(i)) {
        pages::field(
            ui,
            tr("First stop"),
            &format!(
                "{} · {}",
                &commit.id.as_str()[..9],
                commit.subject_lossy().chars().take(240).collect::<String>()
            ),
            false,
        );
    }
}

#[cfg(test)]
mod flow_tests {
    use crate::{
        app::{DialogKind, Intent},
        batch::flow_tests::{click, frame, open, wait},
        test_common as common,
    };
    use eframe::egui::Context;
    use rocket_git::RebaseAction;

    /// The dialog has its final shape from the first frame: nothing moves when the plan
    /// arrives, and both panes are there before it does.
    #[test]
    fn the_plan_arrives_without_moving_the_dialog() {
        common::isolated(
            "ROCKET_REBASE_UI_CASE",
            "rebase::flow_tests::the_plan_arrives_without_moving_the_dialog",
            || {
                for others in [0, 1, 2] {
                    let f = common::Fixture::new();
                    for n in 1..=3 {
                        f.write(&format!("{n}.txt"), "x\n");
                        f.commit(&format!("commit {n}"));
                    }
                    for n in 0..others {
                        f.run(&["branch", &format!("other-{n}"), &format!("HEAD~{}", n + 1)]);
                    }
                    let ctx = Context::default();
                    let mut app = open(&ctx, &f);
                    app.dispatch(Intent::Dialog(DialogKind::Rebase(RebaseAction::Interactive)));
                    // A tall dialog takes a few frames to open to its full height.
                    for _ in 0..10 {
                        frame(&ctx, &mut app, vec![]);
                    }
                    assert!(app.dialog.as_ref().unwrap().rebase.as_ref().unwrap().preview.is_none());
                    for pane in ["dialog-rebase-plan-scroll", "dialog-rebase-pane-scroll"] {
                        assert!(app.frame_widgets.contains_key(pane), "{pane} is there before the plan");
                    }
                    let edges = |app: &crate::app::Rocket| {
                        (app.frame_widgets["dialog-close"].rect, app.frame_widgets["dialog-submit"].rect)
                    };
                    let before = edges(&app);
                    click(&ctx, &mut app, "dialog-rebase-root");
                    wait(&ctx, &mut app);
                    for _ in 0..10 {
                        frame(&ctx, &mut app, vec![]);
                    }
                    let dialog = app.dialog.as_ref().unwrap();
                    let preview = dialog.rebase.as_ref().unwrap().preview.as_ref();
                    let found = preview.unwrap_or_else(|| panic!("{:?}", dialog.error)).dependents().len();
                    assert_eq!(found, others);
                    assert_eq!(edges(&app), before, "the dialog didn't move with {others} other branches");
                }
            },
        );
    }

    #[test]
    fn the_root_option_reviews_every_commit_and_hides_the_base() {
        common::isolated(
            "ROCKET_REBASE_UI_CASE",
            "rebase::flow_tests::the_root_option_reviews_every_commit_and_hides_the_base",
            || {
                let f = common::Fixture::new();
                for n in 1..=2 {
                    f.write(&format!("{n}.txt"), "x\n");
                    f.commit(&format!("commit {n}"));
                }
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                app.dispatch(Intent::Dialog(DialogKind::Rebase(
                    RebaseAction::Interactive,
                )));
                click(&ctx, &mut app, "dialog-rebase-root");
                wait(&ctx, &mut app);
                let dialog = app.dialog.as_ref().unwrap();
                let state = dialog.rebase.as_ref().unwrap();
                let preview = state
                    .preview
                    .as_ref()
                    .unwrap_or_else(|| panic!("{:?}", dialog.error));
                assert_eq!(preview.onto(), None);
                assert_eq!(state.entries.len(), 2);
                assert!(!app.frame_widgets.contains_key("dialog-rebase-target"));
                // Turning the option off brings the base field back and clears the review.
                click(&ctx, &mut app, "dialog-rebase-root");
                let state = app.dialog.as_ref().unwrap().rebase.as_ref().unwrap();
                assert!(!state.root && state.preview.is_none());
                assert!(app.frame_widgets.contains_key("dialog-rebase-target"));
            },
        );
    }

    /// A branch pointing into the replayed commits is listed; ticking the option
    /// moves it with the rebase.
    #[test]
    fn the_update_refs_option_moves_a_dependent_branch() {
        common::isolated(
            "ROCKET_REBASE_UI_CASE",
            "rebase::flow_tests::the_update_refs_option_moves_a_dependent_branch",
            || {
                let f = common::Fixture::new();
                f.write("base.txt", "base\n");
                f.commit("base");
                f.run(&["switch", "-c", "stack-2"]);
                f.write("a.txt", "a\n");
                f.commit("A");
                f.run(&["branch", "stack-1"]);
                f.write("b.txt", "b\n");
                f.commit("B");
                f.run(&["switch", "main"]);
                f.write("main.txt", "main\n");
                f.commit("main moves");
                f.run(&["switch", "stack-2"]);
                let rev = |name: &str| String::from_utf8(f.run(&["rev-parse", name])).unwrap();
                let a = rev("stack-1");
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                app.dispatch(Intent::Dialog(DialogKind::Rebase(RebaseAction::Start)));
                app.dialog.as_mut().unwrap().first = "main".into();
                app.start_rebase_preview();
                wait(&ctx, &mut app);
                let state = app.dialog.as_ref().unwrap().rebase.as_ref().unwrap();
                let preview = state.preview.as_ref().unwrap();
                assert_eq!(preview.dependents().len(), 1);
                assert!(!state.update_refs);
                click(&ctx, &mut app, "dialog-rebase-update-refs");
                assert!(
                    app.dialog
                        .as_ref()
                        .unwrap()
                        .rebase
                        .as_ref()
                        .unwrap()
                        .update_refs
                );
                click(&ctx, &mut app, "dialog-submit");
                wait(&ctx, &mut app);
                assert_ne!(rev("stack-1"), a);
                assert_eq!(rev("stack-1"), rev("stack-2~1"));
                let details = app.activities.last().unwrap().outcome.details.join("\n");
                assert!(
                    details.contains("stack-1 moved with the rebase"),
                    "{details}"
                );
            },
        );
    }

    #[test]
    fn continuing_too_early_names_what_is_still_in_conflict() {
        let code = "Rebase [conflicts_remain]: Fix the conflicts.";
        let one = [("src/app.rs".to_owned(), "conflict, not fixed yet"), ("b.rs".to_owned(), "staged")];
        assert!(super::too_early(code, &one).starts_with("app.rs still has a conflict\n"));
        let two = [("a.rs".to_owned(), "conflict, not fixed yet"), ("b.rs".to_owned(), "conflict, not fixed yet")];
        assert!(super::too_early(code, &two).starts_with("2 files still have conflicts\n"));
        assert_eq!(super::too_early(code, &[]), code, "nothing better to say");
        assert_eq!(super::too_early("Rebase [stale_preview]: Changed.", &one), "Rebase [stale_preview]: Changed.");
    }

    #[test]
    fn the_sentence_says_what_moves_and_what_stays() {
        let text = |n, merges| super::moves(n, merges, "feature", "main").text;
        assert_eq!(text(3, 0), "3 commits from feature move on top of main. main doesn't change.");
        assert_eq!(text(1, 0), "1 commit from feature moves on top of main. main doesn't change.");
        assert_eq!(text(4, 1), "4 commits from feature move on top of main. main doesn't change. Includes 1 merge commit.");
        assert_eq!(super::target_name("refs/heads/main"), "main");
        assert_eq!(super::target_name("origin/main"), "origin/main");
        assert_eq!(super::target_name(&"a".repeat(40)), "aaaaaaa");
    }

    /// The review names where each moving branch ends up under the edited plan.
    #[test]
    fn planned_moves_follow_drops_and_folds() {
        use rocket_git::{ObjectId, RebaseInstruction::*, RebasePlanEntry};
        let id = |n: u8| ObjectId::parse(&format!("{n:040x}")).unwrap();
        let rows: Vec<_> = [(1, Pick), (2, Reword), (3, Fixup), (4, Drop), (5, Squash)]
            .into_iter()
            .map(|(n, instruction)| RebasePlanEntry::new(id(n), instruction))
            .collect();
        let moved = |n| super::planned_move(&rows, &id(n));
        assert_eq!(moved(1), "moves to its rewritten commit");
        assert_eq!(moved(2), "moves to the combined commit");
        assert_eq!(moved(3), "moves to the combined commit");
        assert_eq!(
            moved(4),
            "moves to the commit before it (its commit is dropped)"
        );
        assert_eq!(moved(5), "moves to the combined commit");
        assert_eq!(
            super::planned_move(&[], &id(1)),
            "moves to its rewritten commit"
        );
    }
}
