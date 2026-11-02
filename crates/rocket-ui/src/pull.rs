//! Explicit fetch, local review and integration. No transport runs on dialog open.
use crate::{
    app::*,
    backend::{Action, Outcome, user_error},
    pages::{self, control},
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use std::sync::Arc;

#[derive(Default)]
pub struct DialogState {
    pub strategy: PullStrategy,
    pub fetched: Option<Arc<PullFetched>>,
    pub preview: Option<Arc<PullPreview>>,
    pub task: Option<ReadTask<PullPreview>>,
    pub fetching: bool,
    pub applying: bool,
    pub cancelling: bool,
    pub notice: Option<String>,
}
impl DialogState {
    pub fn running(&self) -> bool {
        self.task.is_some() || self.fetching || self.applying
    }
}
impl Rocket {
    pub(crate) fn start_pull_preview(&mut self) {
        let Some(d) = &mut self.dialog else { return };
        if !matches!(d.kind, DialogKind::Pull) {
            return;
        }
        let s = d.pull.get_or_insert_with(Default::default);
        if s.fetching || s.applying {
            return;
        }
        s.task = None;
        s.cancelling = false;
        s.notice = None;
        s.preview = None;
        d.error = None;
        let (Some(fetched), Some(backend), Some(path)) = (
            s.fetched.clone(),
            self.backend.clone(),
            self.tabs
                .iter()
                .find(|t| Some(t.id) == d.owner)
                .map(|t| t.path.clone()),
        ) else {
            return;
        };
        let strategy = s.strategy;
        match ReadTask::spawn(move |cancel| {
            backend.pull_preview(&path, &fetched, strategy, &cancel)
        }) {
            Ok(task) => s.task = Some(task),
            Err(e) => d.error = Some(user_error(&e)),
        }
    }
    pub(crate) fn poll_pull_preview(&mut self) {
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.pull else { return };
        if let Some(result) = s.task.as_mut().and_then(ReadTask::try_finish) {
            s.task = None;
            s.cancelling = false;
            match result {
                Ok(p) => s.preview = Some(Arc::new(p)),
                Err(Error::Cancelled) => s.notice = Some("Conflict check cancelled. No integration was started. Review again to check the fetched tip.".into()),
                Err(e) => d.error = Some(user_error(&e)),
            }
        }
    }
    pub(crate) fn finish_pull_dialog(&mut self, owner: Option<u64>, outcome: &mut Outcome) {
        let Some(d) = &mut self.dialog else { return };
        if d.owner != owner {
            return;
        }
        let Some(s) = &mut d.pull else { return };
        if s.fetching {
            s.fetching = false;
            s.fetched = outcome.pull_fetched.take();
            s.preview = None;
            s.notice = Some(outcome.summary.clone());
            // Keep observation errors visible even when the fetch itself succeeded.
            if !outcome.details.is_empty() {
                s.notice = Some(format!(
                    "{}\n{}",
                    outcome.summary,
                    outcome.details.join("\n")
                ));
            }
            if s.fetched.is_some() {
                self.start_pull_preview();
            } else {
                d.error = Some(format!(
                    "{}\n{}",
                    outcome.summary,
                    outcome.details.join("\n")
                ));
                s.notice = None;
            }
        } else if s.applying {
            if outcome.command_succeeded || outcome.recovery_pending {
                self.dialog = None;
                if outcome.recovery_pending
                    && let Some(tab) = self.tabs.iter_mut().find(|t| Some(t.id) == owner)
                {
                    tab.view = View::Changes;
                }
            } else {
                s.applying = false;
                s.preview = None;
                d.error = Some(format!(
                    "{}\n{}\nReview the current state before retrying.",
                    outcome.summary,
                    outcome.details.join("\n")
                ));
            }
        }
    }
    pub(crate) fn pull_dialog(&mut self, ctx: &Context) {
        let idle = self.write.is_none();
        let writable = self.can_write();
        let Some(mut d) = self.dialog.take() else {
            return;
        };
        let s = d.pull.get_or_insert_with(Default::default);
        if d.owner != self.tab().map(|t| t.id) && !s.fetching && !s.applying {
            return;
        }
        let busy = s.running();
        let running = pages::Running::of(self.write.as_ref());
        let (mut fetch, mut review, mut apply, mut close, mut cancel) =
            (false, false, false, false, false);
        let response = pages::review(
            ctx,
            "rocket-pull",
            pages::ReviewHeader {
                image: Icon::Pull,
                title: "Pull into current branch",
                subtitle: "Fetch upstream, review the actual commits, then apply your chosen strategy.",
                danger: false,
            },
            &mut self.frame_widgets,
            "dialog-pull-scroll",
            !s.fetching && !s.applying,
            |ui, part, widgets| match part {
                pages::Part::Body => {
                    if let Some(ticket) = &s.fetched {
                        pages::field(
                            ui,
                            "Branch",
                            &String::from_utf8_lossy(&ticket.upstream().branch),
                            false,
                        );
                        pages::field(
                            ui,
                            "Upstream",
                            &String::from_utf8_lossy(&ticket.upstream().tracking_reference),
                            false,
                        );
                        pages::field(ui, "Current tip", ticket.original().as_str(), true);
                        pages::field(ui, "Fetched tip", ticket.target().as_str(), true);
                    }
                    pages::caption(ui, "Integration strategy");
                    for (id, choice) in [
                        ("dialog-pull-ff", PullStrategy::FastForward),
                        ("dialog-pull-merge", PullStrategy::Merge),
                        ("dialog-pull-rebase", PullStrategy::Rebase),
                    ] {
                        let r = ui.add_enabled(
                            !busy,
                            RadioButton::new(s.strategy == choice, choice.label()),
                        );
                        control(widgets, id, &r, ui.clip_rect());
                        if r.clicked() && s.strategy != choice {
                            s.strategy = choice;
                            s.preview = None;
                            review = s.fetched.is_some();
                            d.error = None;
                        }
                    }
                    if let Some(p) = &s.preview {
                        if let Some(prediction) = p.prediction() {
                            crate::rebase::prediction_ui(ui, prediction, p.replay());
                        }
                        pages::caption(ui, "Reviewed result");
                        pages::field(
                            ui,
                            "History",
                            &format!(
                                "{} incoming · {} local-only",
                                p.incoming(),
                                p.current_only()
                            ),
                            false,
                        );
                        pages::field(ui, "Incoming tip", &p.target().subject_lossy(), false);
                        let message = match (p.relation(), p.strategy()) {
                            (MergeRelation::UpToDate, _) => {
                                "The fetched commit is already contained in your branch. No history rewrite is needed."
                            }
                            (MergeRelation::FastForward, _) => {
                                "Your branch will advance to the fetched tip without rewriting local commits."
                            }
                            (_, PullStrategy::FastForward) => {
                                "Branches have diverged. Choose Merge or Rebase to review how they will be integrated."
                            }
                            (_, PullStrategy::Merge) => {
                                "Git will keep both histories and create a merge commit, or pause for conflict resolution."
                            }
                            (_, PullStrategy::Rebase) => {
                                "Your local commits move on top of the fetched commits; their IDs change."
                            }
                        };
                        pages::callout(ui, if p.can_apply() { BLUE() } else { AMBER() }, message);
                        if let Some(reference) = p.recovery_reference() {
                            pages::field(ui, "Recovery", &reference, true);
                        }
                        if !p.replay().is_empty() {
                            pages::caption(ui, "Local commits that move");
                            for commit in p.replay().iter().take(8) {
                                ui.label(format!(
                                    "{}  {}",
                                    &commit.id.as_str()[..9],
                                    commit.subject_lossy()
                                ));
                            }
                            if p.replay().len() > 8 {
                                pages::note(ui, &format!("{} more commits.", p.replay().len() - 8));
                            }
                        }
                    }
                    // The fetch and the integration show their status and Stop in the
                    // footer; only the check reads here.
                    if busy && !s.fetching && !s.applying {
                        pages::waiting(
                            ui,
                            if s.cancelling {
                                "Stopping check and cleaning temporary storage…"
                            } else if s.strategy == PullStrategy::Rebase {
                                "Reviewing the fetched tip and checking for content conflicts…"
                            } else {
                                "Reviewing checkout and the fetched commit…"
                            },
                        );
                    }
                    if let Some(n) = &s.notice {
                        pages::note(ui, n);
                    }
                    if let Some(e) = &d.error {
                        pages::callout(ui, RED(), e);
                    }
                }
                pages::Part::Footer if s.fetching || s.applying => {
                    cancel = pages::running_footer(ui, widgets, &running, &["dialog-pull-cancel"]);
                }
                pages::Part::Footer => {
                    let r = pages::primary(
                        ui,
                        idle && !busy
                            && writable
                            && s.preview.as_ref().is_some_and(|p| p.can_apply()),
                        "Apply pull",
                    );
                    control(widgets, "dialog-submit", &r, ui.clip_rect());
                    apply = r.clicked();
                    let r = ui.add_enabled(
                        idle && !busy && writable,
                        Button::new(if s.fetched.is_some() {
                            "Fetch again"
                        } else {
                            "Fetch and review"
                        }),
                    );
                    control(widgets, "dialog-pull-fetch", &r, ui.clip_rect());
                    fetch = r.clicked();
                    if s.fetched.is_some() {
                        let r = ui.add_enabled(!busy, Button::new("Review again"));
                        control(widgets, "dialog-preview-refresh", &r, ui.clip_rect());
                        review |= r.clicked();
                    }
                    if busy {
                        let stopping =
                            s.cancelling || self.write.as_ref().is_some_and(|w| w.cancelling);
                        let r = ui.add_enabled(
                            !stopping,
                            Button::new(if s.task.is_some() { "Stop check" } else { "Stop" }),
                        );
                        control(widgets, "dialog-pull-cancel", &r, ui.clip_rect());
                        cancel = r.clicked();
                    }
                }
            },
        );
        close |= response.should_close() && !s.fetching && !s.applying;
        if cancel {
            if s.fetching || s.applying {
                if let Some(write) = &mut self.write {
                    write.task.cancel();
                    write.cancelling = true;
                }
            } else {
                if let Some(task) = &s.task {
                    task.cancel();
                }
                s.cancelling = true;
                s.preview = None;
                s.notice = Some(
                    "Stopping check. The fetched result is retained; no integration was started."
                        .into(),
                );
            }
        }
        let action = if fetch {
            s.fetched = None;
            s.preview = None;
            s.task = None;
            s.fetching = true;
            d.error = None;
            s.notice = None;
            Some(Action::PullFetch)
        } else if apply {
            s.applying = true;
            Some(Action::PullReviewed(s.preview.clone().unwrap()))
        } else {
            None
        };
        let owner = d.owner;
        if !close {
            self.dialog = Some(d);
        }
        if review {
            self.start_pull_preview();
        }
        if let (Some(action), Some(owner)) = (action, owner) {
            self.dispatch(Intent::Execute(owner, action));
            if self.write.is_none()
                && let Some(d) = &mut self.dialog
                && let Some(s) = &mut d.pull
            {
                s.fetching = false;
                s.applying = false;
                d.error = Some(
                    "Could not start. Wait for repository tasks to finish, then review again."
                        .into(),
                );
            }
        }
    }
}
