//! Manual bisect reviews and recovery; all Git work stays on cancellable workers.
use crate::{
    app::*,
    backend::{Action, Outcome, user_error},
    pages,
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use crate::i18n::{tr, trf, trn};
use std::sync::Arc;
pub mod automatic;

pub enum Loaded {
    Session(Box<Option<BisectSession>>, Vec<BisectTestResult>, Option<String>),
    Preview(Arc<BisectPreview>),
    Automatic(Arc<BisectAutoPreview>),
}
#[derive(Default)]
pub struct DialogState {
    pub session: Option<BisectSession>,
    pub preview: Option<Arc<BisectPreview>>,
    pub task: Option<ReadTask<Loaded>>,
    pub applying: bool,
    pub loaded: bool,
    pub notice: Option<String>,
    pub automatic: automatic::State,
}
impl Rocket {
    pub(crate) fn start_bisect_read(&mut self, action: Option<BisectAction>) {
        let Some(d) = &mut self.dialog else { return };
        if !matches!(d.kind, DialogKind::Bisect) {
            return;
        }
        let s = d.bisect.get_or_insert_with(Default::default);
        if s.applying {
            return;
        }
        s.task = None;
        s.preview = None;
        s.automatic.preview = None;
        d.error = None;
        let first = d.first.clone();
        let second = d.second.clone();
        let path = self
            .tabs
            .iter()
            .find(|t| Some(t.id) == d.owner)
            .map(|t| t.path.clone());
        match (self.backend.clone(), path) {
            (Some(backend), Some(path)) => {
                match ReadTask::spawn(move |cancel| match action {
                    Some(action) => backend
                        .bisect_preview(&path, action, &first, &second, &cancel)
                        .map(|p| Loaded::Preview(Arc::new(p))),
                    None => {
                        let session=backend.bisect_session(&path,&cancel)?;
                        let (tests,error)=if session.is_some(){match backend.bisect_tests(&path,&cancel){Ok(t)=>(t,None),Err(e)=>(Vec::new(),Some(user_error(&e)))}}else{(Vec::new(),None)};
                        Ok(Loaded::Session(Box::new(session),tests,error))
                    },
                }) {
                    Ok(task) => s.task = Some(task),
                    Err(e) => d.error = Some(user_error(&e)),
                }
            }
            _ => d.error = Some(crate::i18n::tr("Open a repository first.").into()),
        }
    }
    pub(crate) fn start_bisect_auto_read(&mut self) {
        let Some(d)=&mut self.dialog else{return};
        let Some(s)=&mut d.bisect else{return};
        if s.applying{return;}
        s.task=None;s.preview=None;s.automatic.preview=None;d.error=None;
        let command=match s.automatic.command(){Ok(c)=>c,Err(e)=>{d.error=Some(user_error(&e));return;}};
        let Some(path)=self.tabs.iter().find(|t|Some(t.id)==d.owner).map(|t|t.path.clone())else{return};
        let Some(backend)=self.backend.clone()else{return};
        let (good,bad)=(d.first.clone(),d.second.clone());
        match ReadTask::spawn(move|c|backend.bisect_auto_preview(&path,&good,&bad,&command,&c).map(|p|Loaded::Automatic(Arc::new(p)))){
            Ok(task)=>s.task=Some(task),Err(e)=>d.error=Some(user_error(&e)),
        }
    }
    pub(crate) fn poll_bisect_read(&mut self) {
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.bisect else { return };
        if let Some(result) = s.task.as_mut().and_then(ReadTask::try_finish) {
            s.task = None;
            match result {
                Ok(Loaded::Session(session,tests,error)) => {
                    s.session = *session;
                    s.loaded = true;
                    let mut progress=s.automatic.progress.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    if progress.results.is_empty(){progress.results=tests;}
                    if let Some(error)=error{d.error=Some(error);}
                }
                Ok(Loaded::Preview(p)) => {
                    s.session = p.session().cloned();
                    s.preview = Some(p);
                }
                Ok(Loaded::Automatic(p))=>{s.session=p.step().session().cloned();s.automatic.preview=Some(p);}
                Err(e) => d.error = Some(user_error(&e)),
            }
        }
    }
    pub(crate) fn finish_bisect_dialog(&mut self, owner: Option<u64>, outcome: &Outcome) {
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.bisect else { return };
        if !s.applying || d.owner != owner {
            return;
        }
        s.applying = false;
        s.preview = None;
        s.automatic.preview = None;
        s.notice = Some(format!(
            "{}\n{}",
            outcome.summary,
            outcome.details.join("\n")
        ));
        self.start_bisect_read(None);
    }
    pub(crate) fn bisect_dialog(&mut self, ctx: &Context) {
        let Some(mut d) = self.dialog.take() else {
            return;
        };
        let s = d.bisect.get_or_insert_with(Default::default);
        if !s.applying && d.owner != self.tab().map(|t| t.id) {
            return;
        }
        let waiting = s.task.is_some() || s.applying;
        let idle = self.write.is_none();
        let running = pages::Running::of(self.write.as_ref());
        let (mut dismiss, mut submit, mut reload, mut stop, mut changed) =
            (false, false, false, false, false);
        let mut review = None;
        let mut review_auto=false;
        let response = pages::review(
            ctx,
            "rocket-bisect",
            pages::ReviewHeader {
                image: Icon::Branch,
                title: tr("Bisect · find a regression"),
                subtitle: tr("Test manually or run a reviewed command, then restore the original checkout."),
                danger: false,
            },
            &mut self.frame_widgets,
            "dialog-bisect-scroll",
            !s.applying,
            |ui, part, widgets| match part {
                pages::Part::Body => {
                    if let Some(error) = &d.error {
                        pages::callout(ui, RED(), error);
                    }
                    if let Some(notice) = &s.notice {
                        pages::callout(ui, BLUE(), notice);
                    }
                    // A running step shows its progress in the footer.
                    if s.task.is_some() {
                        pages::waiting(ui, tr("Reading the checkout and bisect state…"));
                    }
                    if let Some(session) = &s.session {
                        if session.phase == BisectPhase::Found
                            && let Some(target) = &session.target
                        {
                            found_card(ui, target.as_str());
                        }
                        pages::caption(ui, tr("Session"));
                        pages::well(ui, |ui| {
                            pages::field(ui, tr("State"), phase_label(session.phase), false);
                            pages::field(
                                ui,
                                tr("Return to"),
                                &format!("{} · {}", head_text(&session.original), short(head_commit(&session.original))),
                                false,
                            );
                            pages::field(ui, tr("Checked out"), short(head_commit(&session.current)), true);
                            if let Some(target) = &session.target
                                && session.phase != BisectPhase::Found
                            {
                                pages::field(ui, tr("Selected revision"), short(target.as_str()), true);
                            }
                            pages::field(
                                ui,
                                tr("Candidates"),
                                &session.candidates.len().to_string(),
                                false,
                            );
                            pages::field(ui, tr("Recovery ref"), &session.recovery_reference, true);
                        });
                        if session.phase == BisectPhase::Inconclusive {
                            pages::callout(
                                ui,
                                AMBER(),
                                tr("Skipped revisions leave several possible first bad commits. No single culprit was found."),
                            );
                            for id in session.candidates.iter().take(100) {
                                ui.monospace(id.as_str());
                            }
                            if session.candidates.len() > 100 {
                                pages::note(ui, tr("Showing the first 100 possible commits."));
                            }
                        }
                        if session.phase == BisectPhase::CheckoutPending {
                            pages::callout(
                                ui,
                                AMBER(),
                                tr("The selected revision is not checked out. Resume checkout without recording another decision, or restore the original checkout."),
                            );
                        }
                        if session.phase == BisectPhase::RestoreOnly {
                            pages::callout(
                                ui,
                                AMBER(),
                                tr("Setup or decision recording was interrupted. Restore the original checkout before starting a new search."),
                            );
                        }
                        let finished = matches!(session.phase, BisectPhase::Found | BisectPhase::Inconclusive);
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            for (action, label, key, enabled) in [
                                (
                                    BisectAction::Good,
                                    tr("Good"),
                                    "dialog-bisect-good",
                                    session.can_mark(),
                                ),
                                (
                                    BisectAction::Bad,
                                    tr("Bad"),
                                    "dialog-bisect-bad",
                                    session.can_mark(),
                                ),
                                (
                                    BisectAction::Skip,
                                    tr("Skip"),
                                    "dialog-bisect-skip",
                                    session.can_mark(),
                                ),
                                (
                                    BisectAction::Resume,
                                    tr("Resume checkout"),
                                    "dialog-bisect-resume",
                                    session.phase == BisectPhase::CheckoutPending,
                                ),
                                (
                                    BisectAction::Restore,
                                    tr("Restore original"),
                                    "dialog-bisect-restore",
                                    true,
                                ),
                            ] {
                                // Once the search has ended, restoring is what is left to do.
                                let r = if action == BisectAction::Restore && finished {
                                    pages::primary(ui, idle && !waiting && enabled, label)
                                } else if matches!(action, BisectAction::Good | BisectAction::Bad | BisectAction::Skip) && !enabled {
                                    continue;
                                } else {
                                    ui.add_enabled(idle && !waiting && enabled, Button::new(label))
                                };
                                pages::control(widgets, key, &r, ui.clip_rect());
                                if r.clicked() {
                                    review = Some(action);
                                }
                            }
                        });
                        if !session.decisions.is_empty() {
                            pages::caption(ui, tr("Decisions"));
                            for decision in session.decisions.iter().rev().take(30) {
                                let outcome = match decision.action { BisectAction::Good => tr("Good"), BisectAction::Bad => tr("Bad"), BisectAction::Skip => tr("Skip"), other => action_label(other) };
                                let color = match decision.action {
                                    BisectAction::Good => GREEN(),
                                    BisectAction::Bad => RED(),
                                    _ => HINT(),
                                };
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 10.0;
                                    pages::pill(ui, outcome, color);
                                    ui.label(RichText::new(short(decision.commit.as_str())).monospace().size(12.5))
                                        .on_hover_text(decision.commit.as_str());
                                });
                            }
                        }
                    } else if s.loaded {
                        for (value, label, key) in [
                            (
                                &mut d.first,
                                tr("Known good commit"),
                                "dialog-bisect-good-input",
                            ),
                            (&mut d.second, tr("Known bad commit"), "dialog-bisect-bad-input"),
                        ] {
                            ui.label(label);
                            let r = ui.add_enabled(
                                !waiting,
                                TextEdit::singleline(value).desired_width(f32::INFINITY),
                            );
                            pages::control(widgets, key, &r, ui.clip_rect());
                            changed |= r.changed();
                        }
                    }
                    changed|=s.automatic.fields(ui,widgets,waiting||!idle);
                    s.automatic.results(ui);
                    if let Some(p) = &s.preview {
                        pages::caption(ui, tr("Review this step"));
                        pages::well(ui, |ui| {
                            pages::field(ui, tr("Action"), action_label(p.action()), false);
                            pages::field(ui, tr("Return checkout"), &head_text(p.original()), false);
                            pages::field(ui, tr("Return commit"), head_commit(p.original()), true);
                            if p.action() == BisectAction::Start {
                                pages::field(ui, tr("Known good"), p.good().as_str(), true);
                                pages::field(ui, tr("Known bad"), p.bad().as_str(), true);
                                pages::field(ui, tr("Candidates"), &p.candidates().to_string(), false);
                            } else if let Some(session) = p.session() {
                                pages::field(
                                    ui,
                                    tr("Reviewed checkout"),
                                    head_commit(&session.current),
                                    true,
                                );
                            }
                        });
                        pages::callout(
                            ui,
                            if p.action() == BisectAction::Skip {
                                AMBER()
                            } else {
                                BLUE()
                            },
                            match p.action() {
                                BisectAction::Start => {
                                    tr("Save the original checkout and begin checking revisions out with a detached HEAD.")
                                }
                                BisectAction::Good => {
                                    tr("Record that the bug is absent at this exact commit, then check out Git's next revision.")
                                }
                                BisectAction::Bad => {
                                    tr("Record that the bug is present at this exact commit, then check out Git's next revision.")
                                }
                                BisectAction::Skip => {
                                    tr("Leave this commit unclassified. Skipping can make the final result inconclusive.")
                                }
                                BisectAction::Resume => {
                                    tr("Complete the pending checkout. No new Good, Bad or Skip decision is recorded.")
                                }
                                BisectAction::Restore => {
                                    tr("Return to the saved branch or detached commit, then clear this search.")
                                }
                            },
                        );
                    }
                }
                pages::Part::Footer if s.applying => {
                    stop = pages::running_footer(ui, widgets, &running, &[]);
                }
                pages::Part::Footer => {
                    let r = pages::primary(
                        ui,
                        idle && !waiting && (s.preview.is_some()||s.automatic.preview.is_some()) && !changed,
                        if s.automatic.preview.is_some(){tr("Run automated bisect")}else{s.preview
                            .as_ref()
                            .map_or(tr("Apply reviewed step"), |p| action_label(p.action()))},
                    );
                    pages::control(widgets, "dialog-submit", &r, ui.clip_rect());
                    submit = r.clicked();
                    let r = ui.add_enabled(
                        idle && !waiting,
                        Button::new(if s.automatic.enabled&&s.loaded&&s.session.as_ref().is_none_or(BisectSession::can_mark){tr("Review test command")}else if s.session.is_some() || !s.loaded {
                            tr("Refresh session")
                        } else {
                            tr("Review range")
                        }),
                    );
                    pages::control(widgets, "dialog-preview-refresh", &r, ui.clip_rect());
                    if r.clicked() {
                        if s.automatic.enabled&&s.loaded&&s.session.as_ref().is_none_or(BisectSession::can_mark){
                            review_auto=true;
                        } else if s.session.is_none() && s.loaded {
                            review = Some(BisectAction::Start);
                        } else {
                            reload = true;
                        }
                    }
                    if s.task.is_some() {
                        let r = ui.button(crate::i18n::tr("Stop"));
                        pages::control(widgets, "dialog-bisect-cancel-read", &r, ui.clip_rect());
                        if r.clicked() {
                            s.task = None;
                            s.preview = None;
                            s.automatic.preview = None;
                            d.error =
                                Some(tr("Read cancelled. The repository was not changed.").into());
                        }
                    }
                }
            },
        );
        // Async errors/previews resize this modal. A click on a moving footer
        // must not be reinterpreted as backdrop dismissal on a later layout pass.
        // Keep the header's × and Escape; session state remains on disk either way.
        dismiss |= (response.close_clicked()
            || (ctx.input(|i| i.key_pressed(Key::Escape)) && response.should_close()))
            && !s.applying;
        if changed {
            s.preview = None;
            s.automatic.preview = None;
        }
        if stop {
            self.cancel_write();
        }
        if submit&&let(Some(owner),Some(preview))=(d.owner,s.automatic.preview.clone()) {
            s.applying=true;s.notice=None;
            self.execute(owner,Action::BisectAuto{preview,progress:s.automatic.progress.clone(),storage:self.state_directory.join("bisect-output")});
            if self.write.is_none(){s.applying=false;d.error=Some(tr("Wait for repository work to finish, then review again.").into());}
        } else if submit && let (Some(owner), Some(preview)) = (d.owner, s.preview.clone()) {
            s.applying = true;
            s.notice = None;
            self.execute(owner, Action::Bisect(preview));
            if self.write.is_none() {
                s.applying = false;
                d.error = Some(tr("Wait for repository work to finish, then review again.").into());
            }
        }
        if !dismiss {
            self.dialog = Some(d);
            if let Some(action) = review {
                self.start_bisect_read(Some(action));
            } else if review_auto {
                self.start_bisect_auto_read();
            } else if reload {
                self.start_bisect_read(None);
            }
        }
    }
}
pub fn phase_label(phase: BisectPhase) -> &'static str {
    match phase {
        BisectPhase::Testing => tr("Ready to test"),
        BisectPhase::CheckoutPending => tr("Checkout pending"),
        BisectPhase::Found => tr("First bad commit found"),
        BisectPhase::Inconclusive => tr("Inconclusive · skipped candidates"),
        BisectPhase::RestoreOnly => tr("Interrupted · restore available"),
    }
}
fn action_label(action: BisectAction) -> &'static str {
    match action {
        BisectAction::Start => crate::i18n::tr("Start bisect"),
        BisectAction::Good => tr("Mark good & continue"),
        BisectAction::Bad => tr("Mark bad & continue"),
        BisectAction::Skip => tr("Skip & continue"),
        BisectAction::Resume => tr("Resume checkout"),
        BisectAction::Restore => tr("Restore original checkout"),
    }
}
/// A commit ID as people read it; the full ID stays in the hover text.
fn short(id: &str) -> &str {
    id.get(..12).unwrap_or(id)
}
/// The search's result, set apart from the session details.
fn found_card(ui: &mut Ui, id: &str) {
    Frame::new()
        .fill(RED().gamma_multiply(0.08))
        .stroke(Stroke::new(1.0, RED().gamma_multiply(0.4)))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.label(RichText::new(tr("First bad commit")).font(semibold(14.0)).color(RED()));
                ui.label(RichText::new(short(id)).monospace().size(13.0)).on_hover_text(id);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button(tr("Copy ID")).clicked() {
                        ui.ctx().copy_text(id.to_owned());
                    }
                });
            });
        });
}
fn head_text(head: &Head) -> String {
    head_name(Some(head))
}
fn head_commit(head: &Head) -> &str {
    match head {
        Head::Branch { target, .. } | Head::Detached { target } => target.as_str(),
        _ => "",
    }
}
