use crate::{
    app::*,
    backend::{Action, Outcome, user_error},
    pages::{self, control},
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use std::sync::Arc;
#[derive(Clone)]
pub struct Row {
    pub local: Vec<u8>,
    pub selected: bool,
    pub destination: String,
}
#[derive(Default)]
pub struct DialogState {
    pub rows: Vec<Row>,
    pub preview: Option<Arc<AtomicPushPreview>>,
    pub task: Option<ReadTask<AtomicPushPreview>>,
    pub applying: bool,
    pub stopping: bool,
    pub notice: Option<String>,
}
impl DialogState {
    pub fn running(&self) -> bool {
        self.task.is_some() || self.applying
    }
    pub fn selection(&self) -> Vec<AtomicPushSelection> {
        self.rows
            .iter()
            .filter(|r| r.selected)
            .map(|r| AtomicPushSelection {
                local_branch: r.local.clone(),
                remote_branch: r.destination.as_bytes().to_vec(),
            })
            .collect()
    }
}
impl Rocket {
    pub(crate) fn start_atomic_dialog(&mut self) {
        if !self
            .dialog
            .as_ref()
            .is_some_and(|d| matches!(d.kind, DialogKind::AtomicPush))
        {
            return;
        }
        let rows = self
            .tab()
            .and_then(|t| t.snapshot.as_ref())
            .map(|s| {
                s.branches
                    .iter()
                    .filter_map(|b| b.reference.strip_prefix(b"refs/heads/"))
                    .map(|name| Row {
                        local: name.to_vec(),
                        selected: false,
                        destination: String::from_utf8_lossy(name).into(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        if let Some(d) = &mut self.dialog {
            d.atomic_push = Some(DialogState {
                rows,
                ..Default::default()
            })
        }
    }
    pub(crate) fn start_atomic_preview(&mut self) {
        let path = self.tab().map(|t| t.path.clone());
        let backend = self.backend.clone();
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.atomic_push else { return };
        if s.running() {
            return;
        }
        s.preview = None;
        s.notice = None;
        s.stopping = false;
        d.error = None;
        let (Some(path), Some(backend), Some(remote)) =
            (path, backend, d.remotes.get(d.remote).cloned())
        else {
            d.error = Some("Add a remote before reviewing an atomic push.".into());
            return;
        };
        let selected = s.selection();
        match ReadTask::spawn(move |c| backend.atomic_push_preview(&path, &remote, &selected, &c)) {
            Ok(t) => s.task = Some(t),
            Err(e) => d.error = Some(user_error(&e)),
        }
    }
    pub(crate) fn poll_atomic_preview(&mut self) {
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.atomic_push else { return };
        if let Some(r) = s.task.as_mut().and_then(ReadTask::try_finish) {
            s.task = None;
            let r = if s.stopping { Err(Error::Cancelled) } else { r };
            s.stopping = false;
            match r {
                Ok(p) => s.preview = Some(Arc::new(p)),
                Err(Error::Cancelled) => {
                    s.notice = Some("Review cancelled. No push started.".into())
                }
                Err(e) => d.error = Some(user_error(&e)),
            }
        }
    }
    pub(crate) fn finish_atomic_dialog(&mut self, owner: Option<u64>, outcome: &Outcome) {
        let Some(d) = &mut self.dialog else { return };
        if d.owner != owner {
            return;
        }
        let Some(s) = &mut d.atomic_push else { return };
        if !s.applying {
            return;
        }
        s.applying = false;
        s.preview = None;
        let message = format!("{}\n{}", outcome.summary, outcome.details.join("\n"));
        if outcome.command_succeeded {
            s.notice = Some(message);
            d.error = None
        } else {
            d.error = Some(message);
            s.notice = None
        }
    }
    pub(crate) fn atomic_push_dialog(&mut self, ctx: &Context) {
        let writable = self.can_write();
        let idle = self.write.is_none();
        let Some(mut d) = self.dialog.take() else {
            return;
        };
        let Some(s) = &mut d.atomic_push else {
            self.dialog = Some(d);
            return;
        };
        if d.owner != self.tab().map(|t| t.id) && !s.applying {
            return;
        }
        let busy = s.running();
        let (mut changed, mut review, mut apply, mut close, mut cancel) =
            (false, false, false, false, false);
        let write = self
            .write
            .as_ref()
            .filter(|w| w.owner == d.owner && w.label == "atomic-push");
        let running = pages::Running::of(write);
        let response = pages::review(
            ctx,
            "rocket-atomic-push",
            pages::ReviewHeader {
                image: Icon::Push,
                title: "Push branches together",
                subtitle: "Review every selected branch, then publish one atomic transaction.",
                danger: false,
            },
            &mut self.frame_widgets,
            "dialog-atomic-scroll",
            !s.applying,
            |ui, part, widgets| match part {
                pages::Part::Body => {
                    ui.add_enabled_ui(!busy, |ui| {
                        pages::caption(ui, "Remote");
                        let combo = ComboBox::from_id_salt("atomic-remote")
                            .selected_text(
                                d.remotes
                                    .get(d.remote)
                                    .map(|r| String::from_utf8_lossy(r))
                                    .unwrap_or_default(),
                            )
                            .show_ui(ui, |ui| {
                                for (i, r) in d.remotes.iter().enumerate() {
                                    let response = ui.selectable_value(
                                        &mut d.remote,
                                        i,
                                        String::from_utf8_lossy(r),
                                    );
                                    control(
                                        widgets,
                                        format!("dialog-atomic-remote-{i}"),
                                        &response,
                                        ui.clip_rect(),
                                    );
                                    changed |= response.changed()
                                }
                            });
                        control(
                            widgets,
                            "dialog-atomic-remote",
                            &combo.response,
                            ui.clip_rect(),
                        );
                        pages::caption(ui, "Local branch → remote branch");
                        for (i, row) in s.rows.iter_mut().enumerate() {
                            ui.horizontal(|ui| {
                                let r = ui.checkbox(
                                    &mut row.selected,
                                    String::from_utf8_lossy(&row.local),
                                );
                                control(
                                    widgets,
                                    format!("dialog-atomic-branch-{i}"),
                                    &r,
                                    ui.clip_rect(),
                                );
                                changed |= r.changed();
                            });
                            if row.selected {
                                let r = ui.add(
                                    TextEdit::singleline(&mut row.destination)
                                        .desired_width(f32::INFINITY),
                                );
                                control(
                                    widgets,
                                    format!("dialog-atomic-destination-{i}"),
                                    &r,
                                    ui.clip_rect(),
                                );
                                changed |= r.changed();
                            }
                        }
                    });
                    if let Some(p) = &s.preview {
                        pages::field(ui, "Destination", &p.destination_display(), false);
                        for e in p.entries() {
                            pages::well(ui, |ui| {
                                ui.label(
                                    RichText::new(format!(
                                        "{} → {}",
                                        String::from_utf8_lossy(&e.selection.local_branch),
                                        String::from_utf8_lossy(&e.selection.remote_branch)
                                    ))
                                    .strong(),
                                );
                                let label = match e.kind {
                                    AtomicPushKind::Create => "Create branch",
                                    AtomicPushKind::FastForward => "Fast-forward",
                                    AtomicPushKind::UpToDate => "Already up to date",
                                    AtomicPushKind::NonFastForward => {
                                        "Blocked: integrate remote history first"
                                    }
                                    AtomicPushKind::FetchFirst => {
                                        "Blocked: fetch remote history first"
                                    }
                                };
                                ui.label(RichText::new(label).color(if e.kind.allowed() {
                                    GREEN()
                                } else {
                                    RED()
                                }));
                                pages::field(
                                    ui,
                                    "From",
                                    e.expected.as_ref().map_or("Absent", |id| id.as_str()),
                                    true,
                                );
                                pages::field(ui, "To", e.source.as_str(), true);
                            });
                        }
                    }
                    if let Some(e) = &d.error {
                        pages::callout(ui, RED(), e)
                    }
                    if let Some(n) = &s.notice {
                        pages::note(ui, n)
                    }
                    if s.task.is_some() {
                        pages::waiting(
                            ui,
                            if s.stopping {
                                "Stopping review…"
                            } else {
                                "Reading the selected remote branches…"
                            },
                        )
                    }
                }
                pages::Part::Footer if s.applying => {
                    cancel = pages::running_footer(ui, widgets, &running, &["dialog-atomic-cancel"]);
                }
                pages::Part::Footer => {
                    ui.horizontal_wrapped(|ui| {
                        let r = ui.add_enabled(
                            !busy && idle && !s.selection().is_empty(),
                            Button::new("Review branches"),
                        );
                        control(widgets, "dialog-atomic-review", &r, ui.clip_rect());
                        review = r.clicked();
                        if busy {
                            let r = ui.add_enabled(
                                write.map_or(!s.stopping, ActiveWrite::can_cancel),
                                Button::new(write.map_or("Stop review", ActiveWrite::cancel_label)),
                            );
                            control(widgets, "dialog-atomic-cancel", &r, ui.clip_rect());
                            cancel = r.clicked();
                        }
                        let r = ui.add_enabled(
                            !busy && writable && s.preview.as_ref().is_some_and(|p| p.can_apply()),
                            Button::new("Push atomically").fill(BLUE()),
                        );
                        control(widgets, "dialog-submit", &r, ui.clip_rect());
                        apply = r.clicked();
                    });
                }
            },
        );
        close |= response.should_close() && !s.applying;
        if changed {
            s.preview = None;
            s.notice = None;
            d.error = None
        }
        if cancel {
            if s.applying {
                self.cancel_write()
            } else if let Some(t) = &s.task {
                t.cancel();
                s.stopping = true;
                s.preview = None;
            }
        }
        let action = if apply {
            s.applying = true;
            d.error = None;
            s.notice = None;
            s.preview.clone().map(Action::AtomicPush)
        } else {
            None
        };
        let owner = d.owner;
        if !close {
            self.dialog = Some(d)
        }
        if review {
            self.start_atomic_preview()
        }
        if let (Some(action), Some(owner)) = (action, owner) {
            self.dispatch(Intent::Execute(owner, action));
            if self.write.is_none()
                && let Some(d) = &mut self.dialog
                && let Some(s) = &mut d.atomic_push
            {
                s.applying = false;
                s.preview = None;
                d.error =
                    Some("Could not start. Wait for repository tasks and review again.".into())
            }
        }
    }
}
