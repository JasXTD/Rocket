//! Explicit, read-only AI review of a pinned branch preview. No automatic send,
//! no applying suggestions, and no persistence of provider output.
use crate::{app::*, commit_draft::Settings, pages, style::*};
use eframe::egui::*;
use rocket_git::*;
use std::{collections::HashMap, sync::Arc};

pub enum ReviewIntent {
    Open,
    Prepare,
    Send,
    Cancel,
}
#[derive(Default)]
pub struct State {
    pub base: String,
    pub target: String,
    pub provider: DraftProvider,
    pub program: String,
    pub preview: Option<Arc<BranchReviewPreview>>,
    pub result: Option<String>,
    /// IDs belonging to the displayed findings, even after preparing a new preview.
    pub result_ids: Option<(ObjectId, ObjectId)>,
    pub error: Option<String>,
    pub failure: Option<&'static str>,
    pub notice: Option<String>,
    pub cancelling: bool,
    pub show_result: bool,
    prepared_inputs: Option<(String, String)>,
    preparing: Option<ReadTask<BranchReviewPreview>>,
    reviewing: Option<ReadTask<String>>,
}
impl State {
    pub fn running(&self) -> bool {
        self.preparing.is_some() || self.reviewing.is_some()
    }
    pub fn preparing(&self) -> bool {
        self.preparing.is_some()
    }
    pub fn changed_inputs(&self) -> bool {
        self.prepared_inputs
            .as_ref()
            .is_none_or(|(b, t)| b != self.base.trim() || t != self.target.trim())
    }
    pub fn stop(&self) {
        if let Some(t) = &self.preparing {
            t.cancel();
        }
        if let Some(t) = &self.reviewing {
            t.cancel();
        }
    }
    fn leave(&mut self) {
        if self.running() {
            self.stop();
            let preparing = self.preparing.is_some();
            if preparing {
                // A retained old preview must never become valid for inputs whose
                // replacement preview was abandoned during navigation.
                self.prepared_inputs = None;
            }
            self.preparing = None;
            self.reviewing = None;
            self.notice = Some(if preparing {
                "Preview cancelled. Nothing was sent.".into()
            } else {
                "Stopped. Any request already sent may still be processed by the provider.".into()
            });
        }
        self.cancelling = false;
    }
    fn failed(&mut self, e: Error) {
        self.failure = Some(match &e {
            Error::BranchReview(f) => f.code(),
            Error::CommitDraft { failure, .. } => failure.code(),
            Error::Cancelled => "cancelled",
            Error::Git { .. } => "git_exit",
            Error::OutputLimit { .. } => "output_limit",
            _ => "review_failed",
        });
        self.error=Some(match &e {
            Error::Git {..} => "Could not read these local revisions. Check the Base and Target and prepare again; no request was sent.".into(),
            Error::CommitDraft {provider,failure,detail} => {
                let guidance = match failure {
                    DraftFailure::InvalidReply => "The CLI reply was empty, oversized or malformed. Try again; no files were changed.",
                    DraftFailure::TimedOut => "The CLI exceeded the two-minute limit and was stopped. The provider may already have received the preview.",
                    _ => failure.guidance(*provider),
                };
                format!("{} review [{}]: {}{}", provider.label(), failure.code(), guidance,
                    detail.as_ref().map(|d|format!("\nCLI: {d}")).unwrap_or_default())
            }
            _ => e.to_string(),
        });
    }
}
impl Rocket {
    pub(crate) fn branch_review_intent(&mut self, intent: ReviewIntent) {
        if matches!(intent, ReviewIntent::Open) {
            self.activity_history.close();
            self.close_browsing_page();
        }
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        if matches!(intent, ReviewIntent::Open) {
            tab.view = View::BranchReview;
            if tab.branch_review.base.is_empty() {
                let s = &mut tab.branch_review;
                s.base = tab
                    .snapshot
                    .as_ref()
                    .and_then(|snap| {
                        snap.branches
                            .iter()
                            .find(|b| b.reference == b"refs/heads/main")
                            .map(|_| "main")
                    })
                    .unwrap_or("HEAD~1")
                    .into();
                s.target = "HEAD".into();
                s.provider = self.drafting.provider;
                s.program = self.drafting.program_setting().into();
            }
            return;
        }
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let Some(repo) = tab.snapshot.as_ref().map(|snap| snap.repository.clone()) else {
            return;
        };
        let s = &mut tab.branch_review;
        if matches!(intent, ReviewIntent::Cancel) {
            if s.running() {
                s.stop();
                s.cancelling = true;
                s.notice = Some("Stopping…".into());
                let _ = backend.log.request("branch-review-cancel", |_| Ok(()));
            }
            return;
        }
        if s.running() {
            return;
        }
        s.error = None;
        s.failure = None;
        s.notice = None;
        s.cancelling = false;
        match intent {
            ReviewIntent::Prepare => {
                let (base, target) = (s.base.trim().to_owned(), s.target.trim().to_owned());
                s.prepared_inputs = None;
                let inputs = (base.clone(), target.clone());
                match ReadTask::spawn(move |cancel| {
                    backend.log.request("branch-review-preview", |log| {
                        repo.with_log(log.clone())
                            .prepare_branch_review(&base, &target, &cancel)
                    })
                }) {
                    Ok(t) => {
                        s.preparing = Some(t);
                        s.prepared_inputs = Some(inputs);
                        s.show_result = false;
                    }
                    Err(e) => s.failed(e),
                }
            }
            ReviewIntent::Send => {
                if s.changed_inputs() {
                    s.error = Some("The inputs changed. Prepare a new preview first.".into());
                    return;
                }
                let Some(preview) = s.preview.clone() else {
                    return;
                };
                let request = DraftRequest {
                    provider: s.provider,
                    executable: (!s.program.trim().is_empty()).then(|| s.program.trim().into()),
                    style: DraftStyle::Plain,
                    language: self.drafting.language.clone(),
                };
                match ReadTask::spawn(move |cancel| {
                    backend.log.request("branch-review", |log| {
                        repo.with_log(log.clone())
                            .review_branch(&preview, &request, &cancel)
                    })
                }) {
                    Ok(t) => s.reviewing = Some(t),
                    Err(e) => s.failed(e),
                }
            }
            _ => {}
        }
    }
    pub(crate) fn poll_branch_reviews(&mut self, ctx: &Context) {
        for tab in &mut self.tabs {
            let s = &mut tab.branch_review;
            if tab.view != View::BranchReview {
                s.leave();
                continue;
            }
            if let Some(result) = s.preparing.as_mut().and_then(ReadTask::try_finish) {
                s.preparing = None;
                if s.cancelling {
                    s.prepared_inputs = None;
                    s.notice = Some("Preview cancelled. Nothing was sent.".into());
                } else {
                    match result {
                        Ok(preview) => {
                            s.preview = Some(Arc::new(preview));
                            s.notice = Some("Preview ready. Nothing sent.".into());
                        }
                        Err(e) => {
                            s.prepared_inputs = None;
                            s.failed(e);
                        }
                    }
                }
                s.cancelling = false;
                ctx.request_repaint();
            }
            if let Some(result) = s.reviewing.as_mut().and_then(ReadTask::try_finish) {
                s.reviewing = None;
                if s.cancelling {
                    s.notice=Some("Review cancelled. The provider may already have received the preview; previous findings kept.".into());
                } else {
                    match result {
                        Ok(text) => {
                            s.result = Some(text);
                            s.result_ids = s
                                .preview
                                .as_ref()
                                .map(|p| (p.merge_base.clone(), p.target.clone()));
                            s.show_result = true;
                            s.notice = Some(
                                "Review complete. Check each suggestion; no files were changed."
                                    .into(),
                            );
                        }
                        Err(e) => s.failed(e),
                    }
                }
                s.cancelling = false;
                ctx.request_repaint();
            }
        }
    }
}
fn control(widgets: &mut HashMap<String, UiControl>, key: &str, r: &Response, ui: &Ui) {
    pages::control(widgets, key, r, ui.clip_rect());
}
pub(crate) fn view(
    ui: &mut Ui,
    tab: &mut Tab,
    settings: &Settings,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
) {
    let height = ui.available_height();
    let out = ScrollArea::vertical()
        .id_salt(("branch-review-page", tab.id))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            content(ui, tab, settings, intents, widgets, height)
        });
    pages::region(widgets, "review-page-scroll", out.inner_rect, true);
}
fn content(
    ui: &mut Ui,
    tab: &mut Tab,
    settings: &Settings,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
    viewport_height: f32,
) {
    let top = ui.cursor().top();
    let s = &mut tab.branch_review;
    pages::header(
        ui,
        Icon::Search,
        "AI branch review",
        "Preview committed changes before sending them to your chosen CLI.",
        |_| {},
    );
    ui.add_enabled_ui(!s.running(), |ui| {
        ui.horizontal_wrapped(|ui| {
            for (key, label, input) in [
                ("review-base", "Base", &mut s.base),
                ("review-target", "Target", &mut s.target),
            ] {
                ui.label(label);
                let r = ui.add(TextEdit::singleline(input).desired_width(180.0));
                control(widgets, key, &r, ui);
            }
            for provider in [DraftProvider::Claude, DraftProvider::Codex] {
                let r = ui.selectable_label(s.provider == provider, provider.label());
                control(
                    widgets,
                    &format!("review-provider-{}", provider.code()),
                    &r,
                    ui,
                );
                if r.clicked() {
                    s.provider = provider;
                    s.program = match provider {
                        DraftProvider::Claude => settings.executable.clone(),
                        DraftProvider::Codex => settings.codex_executable.clone(),
                    };
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("CLI program");
            let r = ui.add(
                TextEdit::singleline(&mut s.program)
                    .hint_text("Use the installed CLI")
                    .desired_width(ui.available_width()),
            );
            control(widgets, "review-program", &r, ui);
        });
    });
    ui.horizontal_wrapped(|ui| {
        let r = pages::primary(
            ui,
            !s.running() && !s.base.trim().is_empty() && !s.target.trim().is_empty(),
            "Prepare preview",
        );
        control(widgets, "review-prepare", &r, ui);
        if r.clicked() {
            intents.push(Intent::BranchReview(ReviewIntent::Prepare));
        }
        if s.running() {
            let r = ui.add_enabled(
                !s.cancelling,
                Button::new(if s.cancelling { "Stopping" } else { "Stop" }),
            );
            control(widgets, "review-cancel", &r, ui);
            if r.clicked() {
                intents.push(Intent::BranchReview(ReviewIntent::Cancel));
            }
            ui.spinner();
            ui.label(if s.preparing() {
                "Reading committed changes…"
            } else {
                "Reviewing preview…"
            });
        }
        if let Some(notice) = &s.notice {
            ui.add(Label::new(RichText::new(notice).color(MUTED())).truncate())
                .on_hover_text(notice);
        }
    });
    if let Some(error) = &s.error {
        ui.add(Label::new(RichText::new(error).color(AMBER())).wrap())
            .on_hover_text(error);
    }
    if let Some(p) = &s.preview {
        ui.horizontal_wrapped(|ui| {
            pages::pill(
                ui,
                &format!(
                    "{} commits · {} files · {} KiB diff",
                    p.commits,
                    p.scope.files,
                    p.scope.sent_bytes.div_ceil(1024)
                ),
                BLUE(),
            );
            ui.label(format!(
                "Common ancestor {} → {}",
                &p.merge_base.as_str()[..9],
                &p.target.as_str()[..9]
            ));
        });
        if p.scope.shortened() {
            pages::callout(
                ui,
                AMBER(),
                "Limited context: the diff was shortened. The preview below is exactly what will be sent.",
            );
        }
        if s.changed_inputs() {
            pages::note(
                ui,
                "Inputs changed or preview cancelled. Prepare again before sending.",
            );
        }
        ui.label(RichText::new(format!("Send this committed diff and file names to {} through its installed CLI. Uses that account and its data/usage policies. No tools or file changes.",s.provider.label())).color(MUTED()));
        if s.provider == DraftProvider::Codex {
            ui.small("Codex can also include your global AGENTS instructions. Its tools are disabled for this request.");
        }
        ui.horizontal_wrapped(|ui| {
            let r = pages::primary(
                ui,
                !s.running() && !s.changed_inputs(),
                &format!("Send preview to {}", s.provider.label()),
            );
            control(widgets, "review-send", &r, ui);
            if r.clicked() {
                intents.push(Intent::BranchReview(ReviewIntent::Send));
            }
            let r = ui.selectable_label(!s.show_result, "Preview");
            control(widgets, "review-show-preview", &r, ui);
            if r.clicked() {
                s.show_result = false;
            }
            let r = ui.add_enabled(
                s.result.is_some(),
                Button::selectable(s.show_result, "Findings"),
            );
            control(widgets, "review-show-result", &r, ui);
            if r.clicked() {
                s.show_result = true;
            }
        });
    }
    pages::divider(ui);
    if s.show_result
        && let Some((base, target)) = &s.result_ids
    {
        ui.small(format!(
            "Findings for {} → {}. AI suggestions need verification.",
            &base.as_str()[..9],
            &target.as_str()[..9]
        ));
    }
    let text = if s.show_result {
        s.result.as_deref().unwrap_or("")
    } else {
        s.preview.as_ref().map(|p|p.payload()).unwrap_or("Choose Base and Target, then Prepare preview. No AI request is made until you click Send preview.")
    };
    let lines: Vec<_> = text.lines().collect();
    let height = ui.text_style_height(&TextStyle::Monospace);
    // Keep the preview bounded while letting the outer page scroll its controls
    // above it when the console/terminal leaves little vertical space.
    let text_height = (viewport_height - (ui.cursor().top() - top)).max(120.0);
    let out = ScrollArea::both()
        .id_salt(("branch-review-text", tab.id, s.show_result))
        .auto_shrink([false, false])
        .max_height(text_height)
        .min_scrolled_height(120.0)
        .show_rows(ui, height, lines.len(), |ui, range| {
            for i in range {
                ui.add(
                    Label::new(RichText::new(lines[i]).monospace())
                        .wrap_mode(TextWrapMode::Extend)
                        .selectable(true),
                );
            }
        });
    pages::region(widgets, "review-text-scroll", out.inner_rect, true);
}
