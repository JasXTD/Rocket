//! Explicit remote inspection and a separate confirmation for replacing one branch.
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
    pub preview: Option<Arc<ForcePushPreview>>,
    pub task: Option<ReadTask<ForcePushPreview>>,
    pub confirmation: String,
    pub applying: bool,
    pub notice: Option<String>,
}
impl DialogState {
    pub fn running(&self) -> bool {
        self.task.is_some() || self.applying
    }
}
impl Rocket {
    pub(crate) fn start_force_push_preview(&mut self) {
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.force_push else { return };
        if s.applying {
            return;
        }
        s.task = None;
        s.preview = None;
        s.confirmation.clear();
        s.notice = None;
        d.error = None;
        let (Some(remote), Some(backend), Some(path)) = (
            d.remotes.get(d.remote).cloned(),
            self.backend.clone(),
            self.tabs
                .iter()
                .find(|t| Some(t.id) == d.owner)
                .map(|t| t.path.clone()),
        ) else {
            return;
        };
        let local = d.branch.clone();
        let destination = d.second.as_bytes().to_vec();
        match ReadTask::spawn(move |cancel| {
            backend.force_push_preview(&path, &remote, &local, &destination, &cancel)
        }) {
            Ok(task) => s.task = Some(task),
            Err(error) => d.error = Some(user_error(&error)),
        }
    }
    pub(crate) fn poll_force_push_preview(&mut self) {
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.force_push else { return };
        if let Some(result) = s.task.as_mut().and_then(ReadTask::try_finish) {
            s.task = None;
            match result {
                Ok(preview) => s.preview = Some(Arc::new(preview)),
                Err(error) => d.error = Some(user_error(&error)),
            }
        }
    }
    pub(crate) fn finish_force_push_dialog(&mut self, owner: Option<u64>, outcome: &Outcome) {
        let Some(d) = &mut self.dialog else { return };
        if d.owner != owner {
            return;
        }
        let Some(s) = &mut d.force_push else { return };
        if !s.applying {
            return;
        }
        s.applying = false;
        s.preview = None;
        s.confirmation.clear();
        let message = format!("{}\n{}", outcome.summary, outcome.details.join("\n"));
        if outcome.command_succeeded {
            s.notice = Some(message);
            d.error = None;
        } else {
            d.error = Some(message);
            s.notice = None;
        }
    }
    pub(crate) fn force_push_dialog(&mut self, ctx: &Context) {
        let idle = self.write.is_none();
        let writable = self.can_write();
        let Some(mut d) = self.dialog.take() else {
            return;
        };
        let Some(s) = &mut d.force_push else {
            self.dialog = Some(d);
            return;
        };
        if d.owner != self.tab().map(|t| t.id) && !s.applying {
            return;
        }
        let busy = s.running();
        let write = self
            .write
            .as_ref()
            .filter(|w| w.owner == d.owner && w.label == "force-push");
        let cancel_enabled = write.is_none_or(ActiveWrite::can_cancel);
        let running = pages::Running::of(write);
        let (mut review, mut apply, mut close, mut back, mut cancel) =
            (false, false, false, false, false);
        let response = pages::review(
            ctx,
            "rocket-force-push",
            pages::ReviewHeader {
                image: Icon::Push,
                title: "Review force-with-lease",
                subtitle: "Replace one remote branch only if it still points to the commit you reviewed.",
                danger: true,
            },
            &mut self.frame_widgets,
            "dialog-force-push-scroll",
            !s.applying,
            |ui, part, widgets| match part {
                pages::Part::Body => {
                    pages::field(
                        ui,
                        "Remote",
                        &d.remotes.get(d.remote).map_or_else(
                            || "No remote".into(),
                            |r| String::from_utf8_lossy(r).into_owned(),
                        ),
                        false,
                    );
                    pages::field(
                        ui,
                        "Local branch",
                        &String::from_utf8_lossy(&d.branch),
                        false,
                    );
                    pages::field(ui, "Remote branch", &d.second, false);
                    if let Some(p) = &s.preview {
                        pages::field(ui, "Push destination", &p.destination_display(), false);
                        pages::field(ui, "Expected tip", p.expected().as_str(), true);
                        pages::field(ui, "Replacement tip", p.source().as_str(), true);
                        if p.can_apply() {
                            pages::callout(
                                ui,
                                AMBER(),
                                &format!(
                                    "{} remote commit(s) will no longer be reachable from this branch.",
                                    p.removed_count()
                                ),
                            );
                            ui.label("Type the exact remote branch name to confirm");
                            let r = ui.add_enabled(
                                !busy,
                                TextEdit::singleline(&mut s.confirmation)
                                    .hint_text(&d.second)
                                    .desired_width(f32::INFINITY),
                            );
                            control(widgets, "push-lease-confirm", &r, ui.clip_rect());
                        } else {
                            pages::callout(ui, BLUE(), ForcePushFailure::NoReplacement.guidance());
                        }
                        commit_list(
                            ui,
                            "Removed from this branch",
                            p.removed_count(),
                            p.removed(),
                            AMBER(),
                        );
                        commit_list(ui, "Added to this branch", p.added_count(), p.added(), BLUE());
                    }
                    // The push itself shows its progress in the footer.
                    if busy && !s.applying {
                        pages::waiting(ui, "Reading the remote tip and comparing commits…");
                    }
                    if let Some(message) = &s.notice {
                        pages::callout(ui, BLUE(), message);
                    }
                    if let Some(error) = &d.error {
                        pages::callout(ui, RED(), error);
                    }
                }
                pages::Part::Footer if s.applying => {
                    cancel = pages::running_footer(ui, widgets, &running, &["push-lease-cancel"]);
                }
                pages::Part::Footer => {
                    let confirmed = s.preview.as_ref().is_some_and(|p| {
                        p.can_apply() && s.confirmation.as_bytes() == p.remote_branch()
                    });
                    let r = ui.add_enabled(
                        idle && !busy && writable && confirmed,
                        Button::new(RichText::new("Push with lease").color(AMBER())),
                    );
                    control(widgets, "dialog-submit", &r, ui.clip_rect());
                    apply = r.clicked();
                    let r = ui.add_enabled(idle && !busy && writable, Button::new("Review remote"));
                    control(widgets, "push-lease-review", &r, ui.clip_rect());
                    review = r.clicked();
                    if busy {
                        let r = ui.add_enabled(cancel_enabled, Button::new("Stop"));
                        control(widgets, "push-lease-cancel", &r, ui.clip_rect());
                        cancel = r.clicked();
                    }
                    let r = ui.add_enabled(!busy, Button::new("Back to Push"));
                    control(widgets, "push-lease-back", &r, ui.clip_rect());
                    back = r.clicked();
                }
            },
        );
        close |= response.should_close() && !s.applying;
        if cancel {
            if s.applying {
                self.cancel_write();
                ctx.request_repaint();
            } else {
                s.task = None;
                s.preview = None;
                s.confirmation.clear();
                s.notice = Some("Review cancelled. No push started.".into());
            }
        }
        let action = if apply {
            s.applying = true;
            s.notice = None;
            d.error = None;
            Some(Action::ForcePush(s.preview.clone().unwrap()))
        } else {
            None
        };
        if back {
            d.force_push = None;
            d.error = None;
        }
        let owner = d.owner;
        if !close {
            self.dialog = Some(d);
        }
        if review {
            self.start_force_push_preview();
        }
        if let (Some(action), Some(owner)) = (action, owner) {
            self.dispatch(Intent::Execute(owner, action));
            if self.write.is_none()
                && let Some(d) = &mut self.dialog
                && let Some(s) = &mut d.force_push
            {
                s.applying = false;
                s.preview = None;
                s.confirmation.clear();
                d.error = Some(
                    "Could not start. Wait for repository tasks to finish, then review again."
                        .into(),
                );
            }
        }
    }
}
fn commit_list(ui: &mut Ui, title: &str, count: u64, commits: &[Commit], color: Color32) {
    pages::caption(ui, &format!("{title} · {count}"));
    if count == 0 {
        pages::note(ui, "None.");
        return;
    }
    pages::well(ui, |ui| {
        for commit in commits {
            ui.label(
                RichText::new(format!(
                    "{}  {}",
                    &commit.id.as_str()[..9],
                    commit.subject_lossy()
                ))
                .color(color),
            );
        }
        if count > commits.len() as u64 {
            pages::note(
                ui,
                &format!(
                    "Showing the first {} of {count} commits. Inspect the full branch history before proceeding.",
                    commits.len()
                ),
            );
        }
    });
}
