//! PR reads/reviews stay owned by the global hosting worker across navigation.
use super::*;
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
                s.error = Some("A review draft is still open for its original account and PR. Reconnect that account and submit or discard the draft before opening another account's PR.".into());
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
            self.forge.error = Some("The Git backend is unavailable. No PR request was sent.".into());
            return;
        };
        let s = &mut self.forge;
        if !s.enabled || s.account_check_only || s.publish_check.is_some() {
            s.drafts.write_ahead = None;
            s.error = Some(
                "PR details and review submissions are disabled in this restricted window.".into(),
            );
            return;
        }
        s.error = None;
        s.notice = None;
        s.failed = "";
        let task = (|| -> rocket_git::Result<WriteTask<Done>> {
            let session = s.session.clone().ok_or_else(|| {
                rocket_git::Error::InvalidInput("Connect a hosting account first.".into())
            })?;
            match action {
                "pull-refresh" => {
                    let repo = s.pull.repository.clone().ok_or_else(|| {
                        rocket_git::Error::InvalidInput("Select a PR first.".into())
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
                    if s.drafts.has_restored() {return Err(Error::InvalidInput("Saved line comments need their original diff or explicit removal before this review can be submitted.".into()));}
                    if s.pull.unknown.is_some() || s.pull.merge.unknown.is_some() || s.pull.threads.unknown {
                        return Err(rocket_git::Error::InvalidInput("Resolve the previous review's uncertain outcome before submitting another review.".into()));
                    }
                    if s.pull.inline_selection.is_some() || s.pull.inline_editing.is_some() {
                        return Err(Error::InvalidInput("Save or cancel the current line draft before reviewing the submission.".into()));
                    }
                    let details = s.pull.details.clone().ok_or_else(|| {
                        rocket_git::Error::InvalidInput("Load PR details first.".into())
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
                            "Check the previous review's outcome first.".into(),
                        ));
                    }
                    let preview = s.pull.preview.clone().ok_or_else(|| {
                        rocket_git::Error::InvalidInput("Preview this review first.".into())
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
                _ => Err(rocket_git::Error::InvalidInput("Unknown PR action.".into())),
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
        let r = pages::ghost(ui, !busy, "Pull requests");
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
                && btn(ui, widgets, "hosting-pull-browser", "Open on provider", true) {
                    out.open = Some(details.url());
                }
            if btn(
                ui,
                widgets,
                "hosting-pull-refresh",
                "Refresh",
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
    if let Some(error)=&s.drafts.error {pages::callout(ui,RED(),&format!("{error} Open Saved drafts to retry storage after resolving the problem."));} else if s.drafts.saving() {
        pages::note(ui,"Saving protected draft and recovery");
    }
    if s.drafts.has_restored() {
        pages::callout(ui,AMBER(),"Saved line comments do not match the loaded revision. Their text is preserved; review it before removing the old locations.");
        for raw in &s.drafts.restored {ui.label(RichText::new(format!("{} · {} {}–{}",raw.path,raw.side.key(),raw.start,raw.line)).size(12.5).color(MUTED()));ui.add(Label::new(&raw.body).wrap());}
        if btn(ui,widgets,"hosting-draft-remove-stale","Move saved line comments into review text",!busy){
            let mut body=s.pull.body.clone();
            for c in &s.drafts.restored {body.push_str(&format!("\n\n{} ({} {}–{}):\n{}",c.path,c.side.key(),c.start,c.line,c.body));}
            if !s.pull.inline_body.is_empty(){body.push_str(&format!("\n\nUnfinished line comment:\n{}",s.pull.inline_body));}
            if body.len()>65536{s.error=Some("The combined review text exceeds 64 KiB. Copy or shorten the preserved text first; nothing was removed.".into());}else{s.pull.body=body;s.pull.inline_body.clear();s.drafts.restored.clear();s.drafts.restored_selection=None;s.pull.inline_editing=None;s.pull.inline_selection=None;}
        }
    }
    merge::recovery(ui,s,widgets,busy);
    if let Some((repo, number)) = &s.pull.unknown {
        pages::callout(ui, AMBER(), &format!("Submission outcome unknown for {} #{number}. Refresh its reviews and inspect the provider before deciding to submit again.", repo.path()));
        if btn(
            ui,
            widgets,
            "hosting-review-acknowledge",
            "I checked the provider and understand another submit may duplicate the review",
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
                ui.label(RichText::new("Loading the pull request").color(MUTED()));
            });
        } else {
            pages::empty(ui, "Refresh to load this pull request.");
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
                    ("Merged", VIOLET())
                } else if p.state == "closed" {
                    ("Closed", RED())
                } else if p.draft {
                    ("Draft", HINT())
                } else {
                    ("Open", GREEN())
                };
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let open = p.state == "open" && !p.merged;
                    let r = pages::primary(ui, !busy && open && s.session.is_some() && !s.pull.composing, "Write a review");
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
                            "Discard draft",
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
                ui.label(RichText::new("wants to merge").color(MUTED()));
                chip(ui, &p.source, BLUE());
                ui.label(RichText::new("into").color(MUTED()));
                chip(ui, &p.target, BLUE());
                ui.label(RichText::new(format!("· {}", details.repository().path())).color(HINT()));
            });
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                chip(ui, &format!("head {}", &p.head[..12.min(p.head.len())]), TEXT()).on_hover_text(&p.head);
                chip(ui, &format!("base {}", &p.base[..12.min(p.base.len())]), TEXT()).on_hover_text(&p.base);
                if p.counts_available {
                    ui.label(RichText::new(format!(
                        "{} commits · {} files · +{} −{}",
                        p.commits, p.files, p.additions, p.deletions
                    )).size(12.5).color(MUTED()));
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
        Some(details.checks.items.len() + details.statuses.items.len()),
    ];
    let names = [
        ("hosting-pull-tab-overview", "Overview"),
        ("hosting-pull-tab-commits", "Commits"),
        ("hosting-pull-tab-files", "Files"),
        ("hosting-pull-tab-discussion", "Discussion"),
        ("hosting-pull-tab-reviews", "Reviews"),
        ("hosting-pull-tab-checks", "Checks"),
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
                pages::note(ui, "This provider does not report all aggregate counts. See the bounded Commits and Files previews.");
            }
            pages::well(ui, |ui| {
                if p.body.is_empty() {
                    ui.label(RichText::new("No description.").color(HINT()));
                } else {
                    ui.add(Label::new(&p.body).wrap());
                }
            });
            pages::note(ui, "Markdown is shown as text. Remote images and HTML are not loaded.");
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
            ui.strong("Conversation");
            comments(ui, &details.discussion);
            ui.strong("Inline review comments");
            comments(ui, &details.inline_comments);
            if details.repository().config().provider().is_github(){threads::section(ui,s,widgets,busy,out);}
        }
        4 => {
            if let Some(r) = &s.pull.submitted {
                ui.colored_label(
                    GREEN(),
                    format!(
                        "Confirmed {} · {}. Refresh to reload the review history.",
                        if r.id == 0 { "review action".into() } else { format!("submission #{}", r.id) }, r.state
                    ),
                );
            }
            collection_note(ui, &details.reviews);
            for r in &details.reviews.items {
                let reference = if r.id == 0 { "review action".into() } else { format!("review #{}", r.id) };
                ui.strong(format!("{} · {} · {reference}", r.author, r.state));
                ui.colored_label(
                    MUTED(),
                    format!(
                        "{} · {}",
                        r.date,
                        r.commit.as_deref().unwrap_or("No commit reported")
                    ),
                );
                ui.label(&r.body);
                ui.separator();
            }
        }
        _ => {
            ui.colored_label(MUTED(), "Checks and commit statuses for this head in the base repository. Fork checks may be absent. This does not establish merge readiness or required branch rules.");
            ui.strong("Check runs");
            checks(ui, &details.checks);
            ui.strong("Commit statuses");
            checks(ui, &details.statuses);
        }
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
            ui.strong("Review before sending");
            ui.label(format!("{} as {} on {} #{}", preview.action().label(), s.session.as_ref().map(|x| x.login()).unwrap_or("Disconnected"), preview.repository().path(), preview.summary().number));
            ui.add(Label::new(RichText::new(format!("Commit {}", preview.summary().head)).monospace()).wrap());
            ui.label(preview.body());
            for comment in preview.comments() { ui.label(format!("{} · {} lines {}–{}",comment.path(),comment.side().key(),comment.start_line(),comment.line())); ui.label(comment.body()); }
            ui.colored_label(MUTED(), "This publishes a review and may notify participants. Rocket rechecks the PR before sending. Preview expires after two minutes.");
            ui.horizontal_wrapped(|ui| {
                if btn(ui, widgets, "hosting-review-edit", "Back to draft", !busy) { s.pull.preview = None; }
                if btn(ui, widgets, "hosting-review-submit", "Submit review", !busy && s.pull.unknown.is_none()) { out.action = Some("pull-submit"); }
            });
        } else {
            ui.strong("Review draft");
            inline::queued(ui,s,widgets,busy);
            ui.horizontal_wrapped(|ui| {
                for (key, action) in [("comment", ForgeReviewAction::Comment), ("approve", ForgeReviewAction::Approve), ("changes", ForgeReviewAction::RequestChanges)] {
                    let enabled=s.pull.details.as_ref().is_some_and(|d|d.supports_review(action));
                    let r = ui.add_enabled(!busy&&enabled, Button::selectable(s.pull.action == action, action.label()));
                    control(widgets, format!("hosting-review-{key}"), &r, ui.clip_rect());
                    if r.clicked() { s.pull.action = action; }
                }
            });
            let message_supported=s.pull.details.as_ref().is_some_and(|d|d.review_message_supported(s.pull.action));
            if !message_supported {ui.colored_label(AMBER(),"This vote cannot include a message. Publish a Comment first or clear the text explicitly. No text will be silently discarded.");}
            let r = ui.add_enabled(!busy, TextEdit::multiline(&mut s.pull.body).desired_width(f32::INFINITY).desired_rows(4).char_limit(65536).hint_text("Describe your review"));
            control(widgets, "hosting-review-body", &r, ui.clip_rect());
            if btn(ui, widgets, "hosting-review-preview", "Review before sending", !busy && s.pull.unknown.is_none()) { out.action = Some("pull-preview"); }
        }
    });
}
fn collection_note<T>(ui: &mut Ui, c: &ForgePullCollection<T>) {
    if let Some(e) = c.error {
        ui.colored_label(
            AMBER(),
            format!("Unavailable: {} — {}", e.code(), e.guidance()),
        );
    } else if c.items.is_empty() {
        ui.colored_label(
            MUTED(),
            "No entries returned. This does not mean checks passed or approval is granted.",
        );
    }
    if c.limited {
        ui.colored_label(
            AMBER(),
            "Partial preview (up to 200 entries). Open the provider for the complete list.",
        );
    }
}
fn comments(ui: &mut Ui, comments: &ForgePullCollection<ForgePullComment>) {
    collection_note(ui, comments);
    for c in &comments.items {
        ui.strong(format!("{} · comment #{}", c.author, c.id));
        ui.colored_label(MUTED(), &c.date);
        if let Some(path) = &c.path {
            ui.label(format!(
                "{path} · {}{}",
                c.line
                    .map(|x| format!("line {x}"))
                    .unwrap_or_else(|| "outdated or file comment".into()),
                c.reply_to
                    .map(|x| format!(" · reply to #{x}"))
                    .unwrap_or_default()
            ));
        }
        ui.label(&c.body);
        ui.separator();
    }
}
fn checks(ui: &mut Ui, checks: &ForgePullCollection<ForgePullCheck>) {
    collection_note(ui, checks);
    for c in &checks.items {
        ui.label(format!(
            "{} · {}{}",
            c.name,
            c.status,
            c.conclusion
                .as_ref()
                .map(|x| format!(" · {x}"))
                .unwrap_or_default()
        ));
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
