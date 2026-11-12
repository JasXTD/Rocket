//! PR reads/reviews stay owned by the global hosting worker across navigation.
use super::*;

fn review_action_label(action: ForgeReviewAction) -> &'static str {
    match action {
        ForgeReviewAction::Comment => tr("Comment"),
        ForgeReviewAction::Approve => tr("Approve"),
        ForgeReviewAction::RequestChanges => tr("Request changes"),
    }
}
pub(super) mod inline;
pub mod merge;
pub mod threads;

#[derive(Default)]
pub struct State {
    pub comments: Vec<ForgeInlineComment>,
    pub inline_selection: Option<inline::Selection>,
    pub inline_body: String,
    pub inline_editing: Option<usize>,
    pub merge: merge::State,
    pub threads: threads::State,
    pub details: Option<Arc<ForgePullDetails>>,
    pub number: u64,
    pub repository: Option<ForgeRepository>,
    pub tab: usize,
    pub file: usize,
    pub composing: bool,
    pub action: ForgeReviewAction,
    pub body: String,
    pub preview: Option<Arc<ForgePullReviewPreview>>,
    pub submitted: Option<ForgePullReview>,
    pub unknown: Option<(ForgeRepository, u64)>,
    pub checked_unknown: bool,
    pub submission: Option<(ForgeRepository, u64)>,
    /// The head's checks, as the marks read them, for hosts that can re-run.
    pub ci: Option<std::sync::Arc<ForgeCommitChecks>>,
    pub ci_error: Option<String>,
    /// The head `ci` belongs to, or is being read for.
    pub ci_for: Option<String>,
    pub ci_task: Option<ReadTask<ForgeCommitChecks>>,
    /// The repository tab whose Actions lists this pull request's repository.
    pub actions_tab: Option<u64>,
}
impl State {
    pub fn has_draft(&self) -> bool { !self.threads.body.is_empty() || !self.body.is_empty() || !self.comments.is_empty() || !self.inline_body.is_empty() || self.inline_selection.is_some() || self.inline_editing.is_some() }
}
impl Rocket {
    pub(super) fn open_pull_details(&mut self, number: u64) {
        if self.forge.running() {
            return;
        }
        let repo = match self.forge.repo() {
            Ok(r) => r,
            Err(e) => {
                self.forge.error = Some(user_error(&e));
                return;
            }
        };
        let s = &mut self.forge;
        drafts::capture(s);
        let same_account = s.drafts.active.as_ref().is_none_or(|scope| s.session.as_ref().is_some_and(|session| scope.matches(session)));
        if s.pull.repository.as_ref() != Some(&repo) || s.pull.number != number || !same_account {
            if s.pull.has_draft() || s.drafts.has_restored() {
                s.error = Some(tr("A review draft is still open for its original account and PR. Reconnect that account and submit or discard the draft before opening another account's PR.").into());
                s.section = Section::PullDetail;
                return;
            }
            s.pull.details = None;
            s.pull.preview = None;
            s.pull.submitted = None;
            s.pull.composing = false;
            s.pull.tab = 0;
            s.pull.repository = Some(repo);
            s.pull.number = number;
            s.pull.merge.preview = None; s.pull.merge.result = None;
            s.pull.threads=Default::default();
        }
        let repository = s.pull.repository.clone().unwrap();
        if let Err(error) = drafts::activate(s,&repository,number) { s.error=Some(user_error(&error));return; }
        s.section = Section::PullDetail;
        self.start_pull_action("pull-refresh");
    }
    pub(super) fn start_pull_action(&mut self, action: &str) {
        if self.forge.running() {
            return;
        }
        if !self.gate_pull_write(action) {return;}
        let Some(backend) = self.backend.clone() else {
            self.forge.drafts.write_ahead = None;
            self.forge.error = Some(tr("The Git backend is unavailable. No PR request was sent.").into());
            return;
        };
        let s = &mut self.forge;
        if !s.enabled || s.account_check_only || s.publish_check.is_some() {
            s.drafts.write_ahead = None;
            s.error = Some(
                tr("PR details and review submissions are disabled in this restricted window.").into(),
            );
            return;
        }
        s.error = None;
        s.notice = None;
        s.failed = "";
        let task = (|| -> rocket_git::Result<WriteTask<Done>> {
            let session = s.session.clone().ok_or_else(|| {
                rocket_git::Error::InvalidInput(tr("Connect a hosting account first.").into())
            })?;
            match action {
                "pull-refresh" => {
                    let repo = s.pull.repository.clone().ok_or_else(|| {
                        rocket_git::Error::InvalidInput(tr("Select a PR first.").into())
                    })?;
                    let number = s.pull.number;
                    s.pull.preview = None;
                    s.pull.merge.preview = None; s.pull.merge.checked_unknown = false;
                    s.pull.checked_unknown = false;
                    s.operation = "pull-refresh";
                    WriteTask::spawn(move |cancel| {
                        backend
                            .forge_request("hosting-pull-details", &session, |session| {
                                session.pull_details(&repo, number, &cancel)
                            })
                            .map(|x| Done::PullDetails(Box::new(x)))
                    })
                }
                "pull-preview" => {
                    if s.drafts.has_restored() {return Err(Error::InvalidInput(tr("Saved line comments need their original diff or explicit removal before this review can be submitted.").into()));}
                    if s.pull.unknown.is_some() || s.pull.merge.unknown.is_some() || s.pull.threads.unknown {
                        return Err(rocket_git::Error::InvalidInput(tr("Resolve the previous review's uncertain outcome before submitting another review.").into()));
                    }
                    if s.pull.inline_selection.is_some() || s.pull.inline_editing.is_some() {
                        return Err(Error::InvalidInput(tr("Save or cancel the current line draft before reviewing the submission.").into()));
                    }
                    let details = s.pull.details.clone().ok_or_else(|| {
                        rocket_git::Error::InvalidInput(tr("Load PR details first.").into())
                    })?;
                    let action = s.pull.action;
                    let body = s.pull.body.clone();
                    let comments = s.pull.comments.clone();
                    s.pull.preview = None;
                    s.operation = "pull-preview";
                    WriteTask::spawn(move |cancel| {
                        backend
                            .forge_request("hosting-review-preview", &session, |session| {
                                session.preview_pull_review_with_comments(&details, action, &body, &comments, &cancel)
                            })
                            .map(Done::PullReview)
                    })
                }
                "pull-submit" => {
                    if s.pull.unknown.is_some() || s.pull.merge.unknown.is_some() || s.pull.threads.unknown {
                        return Err(rocket_git::Error::InvalidInput(
                            tr("Check the previous review's outcome first.").into(),
                        ));
                    }
                    let preview = s.pull.preview.clone().ok_or_else(|| {
                        rocket_git::Error::InvalidInput(tr("Preview this review first.").into())
                    })?;
                    s.pull.submission =
                        Some((preview.repository().clone(), preview.summary().number));
                    s.operation = "pull-submit";
                    WriteTask::spawn(move |cancel| {
                        backend
                            .forge_request("hosting-review-submit", &session, |session| {
                                session.submit_pull_review(&preview, &cancel)
                            })
                            .map(Done::PullSubmitted)
                    })
                }
                "pull-merge-preview" | "pull-merge-submit" => merge::start(s, &backend, session, action),
                "pull-threads-refresh" | "pull-thread-preview" | "pull-thread-submit" => threads::start(s,&backend,session,action),
                _ => Err(rocket_git::Error::InvalidInput(tr("Unknown PR action.").into())),
            }
        })();
        match task {
            Ok(task) => s.task = Some(task),
            Err(e) => {
                s.drafts.write_ahead=None;
                s.operation = "";
                s.error = Some(user_error(&e));
            }
        }
    }
}

pub(super) fn section(
    ui: &mut Ui,
    s: &mut super::State,
    widgets: &mut HashMap<String, UiControl>,
    busy: bool,
    out: &mut PageOut,
) {
    // Where this is (back to the list), and the page's own actions.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let r = pages::ghost(ui, !busy, tr("Pull requests"));
        control(widgets, "hosting-pull-back", &r, ui.clip_rect());
        if r.clicked() {
            s.section = Section::Pulls;
        }
        if let Some(details) = &s.pull.details {
            ui.label(RichText::new(format!("/   #{}", details.summary().number)).color(HINT()));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if let Some(details) = &s.pull.details
                && btn(ui, widgets, "hosting-pull-browser", tr("Open on provider"), true) {
                    out.open = Some(details.url());
                }
            if btn(
                ui,
                widgets,
                "hosting-pull-refresh",
                tr("Refresh"),
                !busy && s.session.is_some(),
            ) {
                out.action = Some("pull-refresh");
            }
        });
    });
    if let Some(error) = &s.error {
        pages::callout(ui, RED(), error);
    }
    if let Some(notice) = &s.notice {
        pages::note(ui, notice);
    }
    if let Some(error)=&s.drafts.error {pages::callout(ui,RED(),&trf!("{error} Open Saved drafts to retry storage after resolving the problem.", error));} else if s.drafts.saving() {
        pages::note(ui,tr("Saving protected draft and recovery"));
    }
    if s.drafts.has_restored() {
        pages::callout(ui,AMBER(),tr("Saved line comments do not match the loaded revision. Their text is preserved; review it before removing the old locations."));
        for raw in &s.drafts.restored {ui.label(RichText::new(trf!("{path} · {side} lines {start}–{end}",path=raw.path,side=inline::side_label(raw.side),start=raw.start,end=raw.line)).size(12.5).color(MUTED()));ui.add(Label::new(&raw.body).wrap());}
        if btn(ui,widgets,"hosting-draft-remove-stale",tr("Move saved line comments into review text"),!busy){
            let mut body=s.pull.body.clone();
            for c in &s.drafts.restored {body.push_str(&format!("\n\n{} ({} {}–{}):\n{}",c.path,c.side.key(),c.start,c.line,c.body));}
            if !s.pull.inline_body.is_empty(){body.push_str(&format!("\n\n{}\n{}",tr("Unfinished line comment:"),s.pull.inline_body));}
            if body.len()>65536{s.error=Some(tr("The combined review text exceeds 64 KiB. Copy or shorten the preserved text first; nothing was removed.").into());}else{s.pull.body=body;s.pull.inline_body.clear();s.drafts.restored.clear();s.drafts.restored_selection=None;s.pull.inline_editing=None;s.pull.inline_selection=None;}
        }
    }
    merge::recovery(ui,s,widgets,busy);
    if let Some((repo, number)) = &s.pull.unknown {
        pages::callout(ui, AMBER(), &trf!("Submission outcome unknown for {repository} #{number}. Refresh its reviews and inspect the provider before deciding to submit again.", repository = repo.path(), number));
        if btn(
            ui,
            widgets,
            "hosting-review-acknowledge",
            tr("I checked the provider and understand another submit may duplicate the review"),
            !busy && s.pull.checked_unknown,
        ) {
            s.pull.unknown = None;
            s.pull.checked_unknown = false;
            s.pull.preview = None;
        }
    }
    let Some(details) = s.pull.details.clone() else {
        if busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(tr("Loading the pull request")).color(MUTED()));
            });
        } else {
            pages::empty(ui, tr("Refresh to load this pull request."));
        }
        return;
    };
    let p = details.summary();
    // The pull request: title, number and state, who wants to merge what, then its
    // exact head and base.
    Frame::new()
        .fill(PANEL())
        .stroke(Stroke::new(1.0, CARD_STROKE()))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(16, 14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 9.0;
            // One row: the actions keep their place at the right; a long title is cut.
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let (state, color) = if p.merged {
                    (tr("Merged"), VIOLET())
                } else if p.state == "closed" {
                    (tr("Closed"), RED())
                } else if p.draft {
                    (tr("Draft"), HINT())
                } else {
                    (tr("Open"), GREEN())
                };
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let open = p.state == "open" && !p.merged;
                    let r = pages::primary(ui, !busy && open && s.session.is_some() && !s.pull.composing, tr("Write a review"));
                    control(widgets, "hosting-review-write", &r, ui.clip_rect());
                    if r.clicked() {
                        s.pull.composing = true;
                        s.pull.merge.preview = None;
                    }
                    if s.pull.composing
                        && btn(
                            ui,
                            widgets,
                            "hosting-review-discard",
                            tr("Discard draft"),
                            !busy,
                        )
                    {
                        s.pull.body.clear();
                        s.pull.comments.clear(); s.pull.inline_body.clear(); s.pull.inline_selection = None; s.pull.inline_editing = None;
                        s.pull.preview = None;
                        s.pull.composing = false;
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        pages::pill(ui, state, color);
                        ui.label(RichText::new(format!("#{}", p.number)).size(16.0).color(HINT()));
                        ui.add(Label::new(RichText::new(&p.title).font(semibold(17.0))).truncate()).on_hover_text(&p.title);
                    });
                });
            });
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(RichText::new(&p.author).strong());
                ui.label(RichText::new(tr("wants to merge")).color(MUTED()));
                chip(ui, &p.source, BLUE());
                ui.label(RichText::new(tr("Into")).color(MUTED()));
                chip(ui, &p.target, BLUE());
                ui.label(RichText::new(format!("· {}", details.repository().path())).color(HINT()));
            });
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                chip(ui, &trf!("head {commit}", commit = &p.head[..12.min(p.head.len())]), TEXT()).on_hover_text(&p.head);
                chip(ui, &trf!("base {commit}", commit = &p.base[..12.min(p.base.len())]), TEXT()).on_hover_text(&p.base);
                if p.counts_available {
                    ui.label(RichText::new(trf!("Commits: {commits} · Files: {files} · +{additions} −{deletions}", commits = p.commits, files = p.files, additions = p.additions, deletions = p.deletions)).size(12.5).color(MUTED()));
                }
                // Whether the head's checks pass, without opening the Checks tab.
                if let Some(ci) = s.pull.ci.as_ref().filter(|c| c.commit() == p.head)
                    && !ci.checks.is_empty()
                {
                    let state = ci.state();
                    let failed = ci.count(ForgeCheckState::Failed) + ci.count(ForgeCheckState::Cancelled);
                    let running = ci.count(ForgeCheckState::Running) + ci.count(ForgeCheckState::Queued);
                    let text = match state {
                        ForgeCiState::Failed => {
                            let mut label = trn!(failed, "{n} failed check", "{n} failed checks");
                            if running > 0 { label.push_str(&trf!(" · {running} running", running)); }
                            label
                        },
                        ForgeCiState::Running => trf!("{running} running", running),
                        ForgeCiState::Passed => tr("Checks passed").into(),
                        ForgeCiState::NotChecked => tr("No checks").into(),
                    };
                    let color = match state { ForgeCiState::Failed => RED(), ForgeCiState::Running => AMBER(), ForgeCiState::Passed => GREEN(), ForgeCiState::NotChecked => MUTED() };
                    // Room for the mark left of the pill, clear of the counts.
                    ui.add_space(14.0);
                    let r = ui.add(Button::new(RichText::new(text).size(12.0).color(color).strong()).corner_radius(12).stroke(Stroke::new(1.0, color.gamma_multiply(0.45))).fill(color.gamma_multiply(0.08)).min_size(vec2(0.0, 24.0)));
                    crate::ci_checks::paint_mark(ui.painter(), pos2(r.rect.left() - 10.0, r.rect.center().y), state, false);
                    control(widgets, "hosting-checks-pill", &r, ui.clip_rect());
                    if r.on_hover_text(tr("Open the Checks tab")).clicked() {
                        s.pull.tab = 5;
                    }
                }
            });
        });
    if s.pull.composing {
        composer(ui, s, widgets, busy, out);
    }
    merge::section(ui,s,widgets,busy,out);
    let counts = [
        None,
        Some(details.commits.items.len()),
        p.counts_available.then_some(p.files as usize),
        Some(details.discussion.items.len() + details.inline_comments.items.len()),
        Some(details.reviews.items.len()),
        // No count when checks couldn't be read, so 0 always means none.
        match s.pull.ci.as_ref().filter(|c| c.commit() == p.head) {
            Some(ci) if ci.error.is_none() || !ci.checks.is_empty() => Some(ci.checks.len()),
            Some(_) => None,
            None if details.checks.error.is_some() && details.statuses.error.is_some() => None,
            None => Some(details.checks.items.len() + details.statuses.items.len()),
        },
    ];
    let names = [
        ("hosting-pull-tab-overview", tr("Overview")),
        ("hosting-pull-tab-commits", tr("Commits")),
        ("hosting-pull-tab-files", tr("Files")),
        ("hosting-pull-tab-discussion", tr("Discussion")),
        ("hosting-pull-tab-reviews", tr("Reviews")),
        ("hosting-pull-tab-checks", tr("Checks")),
    ];
    let items: Vec<pages::TabItem> = names
        .iter()
        .zip(counts)
        .enumerate()
        .map(|(i, ((key, label), count))| pages::TabItem { label, count, selected: s.pull.tab == i, enabled: true, key })
        .collect();
    if let Some(i) = pages::tabs(ui, &items, widgets) {
        s.pull.tab = i;
    }
    match s.pull.tab {
        0 => {
            if !p.counts_available {
                pages::note(ui, tr("This provider does not report all aggregate counts. See the bounded Commits and Files previews."));
            }
            pages::well(ui, |ui| {
                if p.body.is_empty() {
                    ui.label(RichText::new(tr("No description.")).color(HINT()));
                } else {
                    ui.add(Label::new(&p.body).wrap());
                }
            });
            pages::note(ui, tr("Markdown is shown as text. Remote images and HTML are not loaded."));
        }
        1 => {
            collection_note(ui, &details.commits);
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for commit in &details.commits.items {
                    let detail = format!("{} · {}", &commit.id[..12.min(commit.id.len())], commit.author);
                    pages::row(ui, pages::Row { image: Icon::Commit, title: &commit.subject, detail: &detail, pills: vec![], indent: 0.0 }, false, true);
                }
            });
        }
        2 => inline::files(ui, &details, s, widgets, busy),
        3 => {
            ui.strong(tr("Conversation"));
            comments(ui, &details.discussion);
            ui.strong(tr("Inline review comments"));
            comments(ui, &details.inline_comments);
            threads::section(ui,s,widgets,busy,out);
        }
        4 => {
            if let Some(r) = &s.pull.submitted {
                ui.colored_label(
                    GREEN(),
                    trf!("Confirmed {reference} · {state}. Refresh to reload the review history.", reference = if r.id == 0 { tr("review action").into() } else { trf!("submission #{number}", number = r.id) }, state = r.state),
                );
            }
            collection_note(ui, &details.reviews);
            for r in &details.reviews.items {
                let reference = if r.id == 0 { tr("review action").into() } else { trf!("review #{number}", number = r.id) };
                ui.strong(format!("{} · {} · {reference}", r.author, r.state));
                ui.colored_label(
                    MUTED(),
                    format!(
                        "{} · {}",
                        r.date,
                        r.commit.as_deref().unwrap_or(tr("No commit reported"))
                    ),
                );
                ui.label(&r.body);
                ui.separator();
            }
        }
        _ => checks_tab(ui, s, &details, widgets, out),
    }
}
fn composer(
    ui: &mut Ui,
    s: &mut super::State,
    widgets: &mut HashMap<String, UiControl>,
    busy: bool,
    out: &mut PageOut,
) {
    Frame::new().fill(CARD()).inner_margin(12).corner_radius(8).show(ui, |ui| {
        ui.set_width(ui.available_width());
        if let Some(preview) = &s.pull.preview {
            ui.strong(tr("Review before sending"));
            ui.label(trf!("{action} as {account} on {repository} #{number}", action = review_action_label(preview.action()), account = s.session.as_ref().map(|x| x.login()).unwrap_or(tr("Disconnected")), repository = preview.repository().path(), number = preview.summary().number));
            ui.add(Label::new(RichText::new(trf!("Commit {commit}", commit = preview.summary().head)).monospace()).wrap());
            ui.label(preview.body());
            for comment in preview.comments() { ui.label(trf!("{path} · {side} lines {start}–{end}", path = comment.path(), side = inline::side_label(comment.side()), start = comment.start_line(), end = comment.line())); ui.label(comment.body()); }
            ui.colored_label(MUTED(), tr("This publishes a review and may notify participants. Rocket rechecks the PR before sending. Preview expires after two minutes."));
            ui.horizontal_wrapped(|ui| {
                if btn(ui, widgets, "hosting-review-edit", tr("Back to draft"), !busy) { s.pull.preview = None; }
                if btn(ui, widgets, "hosting-review-submit", tr("Submit review"), !busy && s.pull.unknown.is_none()) { out.action = Some("pull-submit"); }
            });
        } else {
            ui.strong(tr("Review draft"));
            inline::queued(ui,s,widgets,busy);
            ui.horizontal_wrapped(|ui| {
                for (key, action) in [("comment", ForgeReviewAction::Comment), ("approve", ForgeReviewAction::Approve), ("changes", ForgeReviewAction::RequestChanges)] {
                    let enabled=s.pull.details.as_ref().is_some_and(|d|d.supports_review(action));
                    let r = ui.add_enabled(!busy&&enabled, Button::selectable(s.pull.action == action, review_action_label(action)));
                    control(widgets, format!("hosting-review-{key}"), &r, ui.clip_rect());
                    if r.clicked() { s.pull.action = action; }
                }
            });
            let message_supported=s.pull.details.as_ref().is_some_and(|d|d.review_message_supported(s.pull.action));
            if !message_supported {ui.colored_label(AMBER(),tr("This vote cannot include a message. Publish a Comment first or clear the text explicitly. No text will be silently discarded."));}
            let r = ui.add_enabled(!busy, TextEdit::multiline(&mut s.pull.body).desired_width(f32::INFINITY).desired_rows(4).char_limit(65536).hint_text(tr("Describe your review")));
            control(widgets, "hosting-review-body", &r, ui.clip_rect());
            if btn(ui, widgets, "hosting-review-preview", tr("Review before sending"), !busy && s.pull.unknown.is_none()) { out.action = Some("pull-preview"); }
        }
    });
}
/// The Checks tab: the head's checks in one list, failures first, under a summary
/// bar with Re-run failed jobs and Open in browser. Hosts without a re-run read
/// show the PR's statuses the same way, without the buttons.
fn checks_tab(ui: &mut Ui, s: &mut super::State, details: &ForgePullDetails, widgets: &mut HashMap<String, UiControl>, out: &mut PageOut) {
    let head = details.summary().head.clone();
    let provider = details.repository().config().provider();
    if let Some(ci) = s.pull.ci.clone().filter(|c| c.commit() == head) {
        let reading = s.pull.ci_task.is_some();
        Frame::new().fill(PANEL()).stroke(Stroke::new(1.0, CARD_STROKE())).corner_radius(8).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            Frame::new().fill(CARD()).inner_margin(Margin { left: 14, right: 10, top: 8, bottom: 8 }).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(18.0, 22.0), Sense::hover());
                    crate::ci_checks::paint_mark(ui.painter(), rect.center(), ci.state(), false);
                    ui.label(RichText::new(crate::ci_checks::summary(&ci)).font(semibold(13.0)));
                    ui.label(RichText::new(trf!("for head {commit}", commit = &head[..7])).size(12.0).color(HINT()));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let r = pages::ghost(ui, true, tr("Open in browser"));
                        control(widgets, "hosting-checks-open", &r, ui.clip_rect());
                        if r.clicked() {
                            out.open = Some(crate::ci_checks::browser_url(&ci));
                        }
                        let failed = ci.failed_runs().next().is_some();
                        if failed || ci.rerun.is_some() {
                            let r = pages::primary(ui, failed && ci.rerun.is_none() && !reading, crate::ci_checks::rerun_label(provider));
                            let r = match ci.rerun {
                                Some(why) => r.on_disabled_hover_text(crate::ci_checks::rerun_refusal(why)),
                                None => r,
                            };
                            control(widgets, "hosting-checks-rerun", &r, ui.clip_rect());
                            out.ci_rerun |= r.clicked();
                        }
                        all_runs(ui, s.pull.actions_tab, &details.summary().source, widgets, out);
                    });
                });
            });
            let hidden = ci.error.map(|e| match (e, &ci.missing) {
                (ForgeFailure::Forbidden, Some(p)) => trf!("Some checks are hidden: this token needs {p}.", p),
                (ForgeFailure::Forbidden, None) => tr("Some checks are hidden: this token can't read them.").to_string(),
                (other, _) => trf!("Some checks couldn't be read ({code}).", code = other.code()),
            });
            if let Some(text) = hidden {
                Frame::new().inner_margin(Margin::symmetric(10, 8)).show(ui, |ui| {
                    pages::notice_strip(ui, MUTED(), &text, None);
                });
            }
            if let Some(why) = ci.rerun.filter(|w| *w != ForgeRerunBlock::Unsupported || ci.state() == ForgeCiState::Failed) {
                Frame::new().inner_margin(Margin { left: 14, right: 10, top: 8, bottom: 2 }).show(ui, |ui| {
                    ui.label(RichText::new(crate::ci_checks::rerun_refusal(why)).size(12.0).color(HINT()));
                });
            }
            if ci.checks.is_empty() {
                Frame::new().inner_margin(14).show(ui, |ui| {
                    ui.label(RichText::new(tr("No checks ran for this head.")).color(HINT()));
                });
            }
            if let Some(url) = crate::ci_checks::list(ui, &ci.checks, provider, widgets, "hosting-checks", crate::ci_checks::ListStyle::Wide) {
                out.open = Some(url);
            }
            if ci.limited {
                Frame::new().inner_margin(10).show(ui, |ui| {
                    ui.label(RichText::new(trf!("Showing {shown}: open the head on the host for the rest.", shown = ci.checks.len())).size(12.0).color(HINT()));
                });
            }
        });
        return;
    }
    if s.pull.ci_task.is_some() && s.pull.ci_for.as_deref() == Some(head.as_str()) {
        pages::busy_line(ui, tr("Reading the head's checks"));
        return;
    }
    if let Some(error) = &s.pull.ci_error {
        pages::notice_strip(ui, AMBER(), error, None);
    }
    // Hosts without a re-run read: the PR's statuses as the details read them.
    Frame::new().fill(PANEL()).stroke(Stroke::new(1.0, CARD_STROKE())).corner_radius(8).inner_margin(Margin::symmetric(4, 4)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let all: Vec<&ForgePullCheck> = details.checks.items.iter().chain(details.statuses.items.iter()).collect();
        for c in &all {
            let state = pull_check_state(c);
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
            crate::ci_checks::paint_check(ui.painter(), pos2(rect.left() + 14.0, rect.center().y), state);
            ui.painter().text(pos2(rect.left() + 30.0, rect.center().y), Align2::LEFT_CENTER, &c.name, FontId::proportional(13.0), TEXT());
            ui.painter().text(pos2(rect.right() - 10.0, rect.center().y), Align2::RIGHT_CENTER, c.conclusion.as_deref().unwrap_or(&c.status), FontId::proportional(12.0), HINT());
        }
        let mut failed = false;
        for c in [&details.checks, &details.statuses] {
            if let Some(e) = c.error.filter(|e| *e != ForgeFailure::Unsupported) {
                failed = true;
                pages::notice_strip(ui, AMBER(), &trf!("Some checks couldn't be read: {guidance} ({code})", guidance = e.guidance(), code = e.code()), None);
            }
        }
        // Bitbucket Server has no checks read here: say so rather than show an empty card.
        let unsupported = [&details.checks, &details.statuses].iter().all(|c| c.error == Some(ForgeFailure::Unsupported));
        if all.is_empty() && !failed {
            let words = if provider == ForgeProvider::BitbucketServer && unsupported {
                tr("Bitbucket Server doesn't report CI to Rocket. Builds from Jenkins or Bamboo show on the pull request's page.")
            } else {
                tr("No checks ran for this head.")
            };
            Frame::new().inner_margin(10).show(ui, |ui| {
                ui.add(Label::new(RichText::new(words).color(HINT())).wrap());
            });
        }
        if provider != ForgeProvider::BitbucketServer {
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                all_runs(ui, s.pull.actions_tab, &details.summary().source, widgets, out);
            });
        }
    });
}
/// "All runs for feature/sign-in": Actions in the repository's tab, on that branch.
fn all_runs(ui: &mut Ui, tab: Option<u64>, branch: &str, widgets: &mut HashMap<String, UiControl>, out: &mut PageOut) {
    if tab.is_none() || branch.is_empty() {
        return;
    }
    ui.add_space(4.0);
    let r = ui.add(Label::new(RichText::new(trf!("All runs for {branch}", branch)).size(12.5).color(BLUE())).sense(Sense::click()).truncate())
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(tr("Opens Actions on this branch"));
    control(widgets, "hosting-checks-all-runs", &r, ui.clip_rect());
    if r.clicked() {
        out.ci_runs = Some(branch.to_owned());
    }
}
/// What a pull request's reported check or status says, as one of the check states.
pub(crate) fn pull_check_state(c: &ForgePullCheck) -> ForgeCheckState {
    let word = c.conclusion.as_deref().unwrap_or(&c.status).to_ascii_lowercase();
    match word.as_str() {
        "success" | "successful" | "succeeded" | "passed" => ForgeCheckState::Passed,
        "failure" | "failed" | "error" | "timed_out" => ForgeCheckState::Failed,
        "running" | "in_progress" | "inprogress" | "pending" => ForgeCheckState::Running,
        "queued" | "created" | "waiting" | "manual" | "scheduled" => ForgeCheckState::Queued,
        "cancelled" | "canceled" | "stopped" => ForgeCheckState::Cancelled,
        _ => ForgeCheckState::Skipped,
    }
}
fn collection_note<T>(ui: &mut Ui, c: &ForgePullCollection<T>) {
    if let Some(e) = c.error {
        ui.colored_label(
            AMBER(),
            trf!("Unavailable: {code} — {guidance}", code = e.code(), guidance = e.guidance()),
        );
    } else if c.items.is_empty() {
        ui.colored_label(
            MUTED(),
            tr("No entries returned. This does not mean checks passed or approval is granted."),
        );
    }
    if c.limited {
        ui.colored_label(
            AMBER(),
            tr("Partial preview (up to 200 entries). Open the provider for the complete list."),
        );
    }
}
fn comments(ui: &mut Ui, comments: &ForgePullCollection<ForgePullComment>) {
    collection_note(ui, comments);
    for c in &comments.items {
        ui.strong(trf!("{author} · comment #{number}", author = c.author, number = c.id));
        ui.colored_label(MUTED(), &c.date);
        if let Some(path) = &c.path {
            ui.label(format!(
                "{path} · {}{}",
                c.line
                    .map(|x| trf!("line {x}", x))
                    .unwrap_or_else(|| tr("outdated or file comment").into()),
                c.reply_to
                    .map(|x| trf!(" · reply to #{x}", x))
                    .unwrap_or_default()
            ));
        }
        ui.label(&c.body);
        ui.separator();
    }
}
/// A branch name or commit ID on a raised fill.
fn chip(ui: &mut Ui, text: &str, color: Color32) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::monospace(12.0), color);
    let (rect, response) = ui.allocate_exact_size(galley.size() + vec2(14.0, 6.0), Sense::hover());
    ui.painter().rect_filled(rect, 5.0, RAISED());
    ui.painter().rect_stroke(rect, 5.0, Stroke::new(1.0, BORDER()), StrokeKind::Inside);
    ui.painter().galley(rect.min + vec2(7.0, 3.0), galley, color);
    response
}
