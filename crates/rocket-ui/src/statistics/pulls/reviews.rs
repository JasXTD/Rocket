//! Review insights on the Pull requests tab: who each pull request waits on, the
//! reviewer field, Read reviews and the strip and row marks that name what the
//! numbers leave out.
use super::*;
use std::{
    collections::{BTreeMap, HashSet},
    sync::Mutex,
};

/// How far ahead of this computer's clock a review time may be and still count, for a
/// host whose clock runs a little ahead.
const CLOCK_ALLOWANCE: u64 = 300;

/// A running Read reviews: its worker hands over each pull request's result as it
/// comes, so a Stop keeps what was read.
pub(super) struct ReviewRead {
    pub(super) task: ReadTask<()>,
    pub(super) results: Arc<Mutex<Vec<(u64, ForgeReviewRead)>>>,
    pub(super) total: usize,
    pub(super) done: usize,
    pub(super) stopping: bool,
}
impl State {
    pub fn reviews_running(&self) -> bool {
        self.review_task.is_some()
    }
    /// How many of the running Read reviews' pull requests are read, of how many.
    pub fn review_progress(&self) -> Option<(usize, usize)> {
        self.review_task.as_ref().map(|r| (r.done, r.total))
    }
    /// Whether the chosen reviewer was asked to review the pull request, or reviewed
    /// it as far as its list row or its read reviews say.
    pub(super) fn matches_reviewer(&self, item: &ForgeItem) -> bool {
        let Some(who) = &self.reviewer else {
            return true;
        };
        item.reviewers.as_ref().is_some_and(|r| r.names(who))
            || (!who.team
                && matches!(self.reviews.get(&item.number),
                    Some(ForgeReviewRead::Read(read)) if read.reviewers.iter().any(|n| n == &who.name)))
    }
    /// The requests still waiting on an open pull request, the chosen reviewer's first.
    pub(super) fn waiting_on<'a>(&self, item: &'a ForgeItem) -> Vec<&'a ForgeReviewer> {
        if !item.timing.as_ref().is_some_and(|t| t.open) {
            return Vec::new();
        }
        let mut waiting: Vec<_> = item
            .reviewers
            .as_ref()
            .filter(|r| r.known)
            .map(|r| r.waiting.iter().collect())
            .unwrap_or_default();
        if let Some(who) = &self.reviewer {
            waiting.sort_by_key(|w| *w != who);
        }
        waiting
    }
    /// Whether the Waiting filter shows the row: an open pull request with a request
    /// still waiting, on the chosen reviewer when there is one.
    pub(super) fn waits(&self, item: &ForgeItem) -> bool {
        let waiting = self.waiting_on(item);
        match &self.reviewer {
            Some(who) => waiting.first().is_some_and(|w| *w == who),
            None => !waiting.is_empty(),
        }
    }
    /// Everyone the chosen author's pull requests name as a reviewer, people before
    /// teams, each with how many open pull requests wait on them and how many they
    /// reviewed (from list rows, and from reviews read).
    pub fn reviewer_choices(&self) -> Vec<(ForgeReviewer, usize, usize)> {
        let mut found: BTreeMap<ForgeReviewer, (usize, usize)> = BTreeMap::new();
        for item in self.items.iter().filter(|i| self.matches_author(i)) {
            let open = item.timing.as_ref().is_some_and(|t| t.open);
            let mut reviewed = HashSet::new();
            if let Some(r) = &item.reviewers {
                for who in &r.waiting {
                    found.entry(who.clone()).or_default().0 += usize::from(open && r.known);
                }
                for who in &r.assigned {
                    found.entry(who.clone()).or_default();
                }
                reviewed.extend(r.reviewed.iter().cloned());
            }
            if let Some(ForgeReviewRead::Read(read)) = self.reviews.get(&item.number) {
                reviewed.extend(read.reviewers.iter().map(|name| ForgeReviewer {
                    team: false,
                    name: name.clone(),
                }));
            }
            for who in reviewed {
                found.entry(who).or_default().1 += 1;
            }
        }
        let mut out: Vec<_> = found.into_iter().map(|(who, (w, r))| (who, w, r)).collect();
        out.sort_by_cached_key(|(who, ..)| (who.team, who.name.to_lowercase()));
        out
    }
    /// The pull requests Read reviews reads next: opened in the period, not read yet or
    /// whose read failed, the chosen author's and reviewer's first, then newest first;
    /// and how many there are.
    fn review_targets(&self) -> (Vec<ForgeItem>, usize) {
        let mut due: Vec<&ForgeItem> = self
            .items
            .iter()
            .filter(|i| {
                self.opened_in_period(i).is_some()
                    && !matches!(self.reviews.get(&i.number), Some(ForgeReviewRead::Read(_)))
            })
            .collect();
        due.sort_by_key(|i| (!self.in_cohort(i), std::cmp::Reverse(self.opened_in_period(i))));
        let total = due.len();
        (due.into_iter().take(REVIEW_BATCH).cloned().collect(), total)
    }
    /// Whether any loaded row says which requests wait: what the Waiting filter needs
    /// from the host, whatever the cohort holds.
    pub(super) fn requests_known_anywhere(&self) -> bool {
        self.items
            .iter()
            .any(|i| i.reviewers.as_ref().is_some_and(|r| r.known))
    }
    /// The pull request's usable creation time, if it falls in the period: the same
    /// rule [`ForgeReviewMetrics`] counts by.
    fn opened_in_period(&self, item: &ForgeItem) -> Option<u64> {
        let since = self.since();
        item.timing
            .as_ref()
            .and_then(|t| t.created)
            .filter(|c| *c <= self.read_at && since.is_none_or(|s| *c >= s))
    }
    /// Reads the reviews of the next pull requests [`review_targets`] names, one at a
    /// time on a worker. Only GitHub, GitHub Enterprise Server and Gitea time reviews.
    pub(super) fn start_reviews(&mut self, backend: Backend, ctx: Context) {
        let Some((repository, session)) = self.connection.clone() else {
            return;
        };
        if self.review_task.is_some() || !repository.config().provider().times_reviews() {
            return;
        }
        let (targets, _) = self.review_targets();
        if targets.is_empty() {
            return;
        }
        self.review_error = None;
        self.review_trouble = Trouble::Clear;
        let (results, total) = (Arc::new(Mutex::new(Vec::new())), targets.len());
        let sink = results.clone();
        let progress = ctx.clone();
        let task = ReadTask::spawn_with_wake(
            move |cancel| {
                backend.forge_request("insights-reviews", &session, |s| {
                    for item in &targets {
                        cancel.check()?;
                        // Each read checks review times against the clock as it is
                        // then, not the sample's time or the batch's start: a review can
                        // be newer than both. The allowance covers a host whose clock is
                        // a little ahead of this computer's.
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map_or(0, |d| d.as_secs())
                            + CLOCK_ALLOWANCE;
                        let read = match s.first_review(&repository, item, now, &cancel) {
                            Ok(read) => ForgeReviewRead::Read(read),
                            // Only a fault in this one pull request's reviews is held
                            // against it; anything else ends the whole read.
                            Err(Error::Forge(
                                failure @ (ForgeFailure::NotFound | ForgeFailure::InvalidResponse),
                            )) => ForgeReviewRead::Failed(failure),
                            Err(e) => return Err(e),
                        };
                        if let Ok(mut results) = sink.lock() {
                            results.push((item.number, read));
                        }
                        progress.request_repaint();
                    }
                    Ok(())
                })
            },
            move || ctx.request_repaint(),
        );
        match task {
            Ok(task) => {
                self.review_task = Some(ReviewRead {
                    task,
                    results,
                    total,
                    done: 0,
                    stopping: false,
                })
            }
            Err(e) => {
                self.review_error = Some(user_error(&e));
                self.review_trouble = Trouble::Failed;
            }
        }
    }
    /// Takes in what the running Read reviews has read so far, and its end.
    pub(super) fn poll_reviews(&mut self) {
        let Some(read) = &mut self.review_task else {
            return;
        };
        let finished = read.task.try_finish();
        // After the worker ends nothing more arrives, so this takes everything.
        let arrived = read
            .results
            .lock()
            .map(|mut results| std::mem::take(&mut *results))
            .unwrap_or_default();
        read.done += arrived.len();
        let stopping = read.stopping;
        let changed = !arrived.is_empty() || finished.is_some();
        for (number, result) in arrived {
            self.reviews.insert(number, result);
        }
        if let Some(result) = finished {
            self.review_task = None;
            match result {
                Ok(()) if !stopping => {}
                Ok(()) | Err(Error::Cancelled) => {
                    self.review_error = Some(tr(REVIEWS_STOPPED).into());
                    self.review_trouble = Trouble::Stopped;
                }
                Err(e) => {
                    self.review_trouble = match &e {
                        Error::Forge(
                            ForgeFailure::Unauthorized
                            | ForgeFailure::Forbidden
                            | ForgeFailure::IdentityUnverified
                            | ForgeFailure::AzureIdentityUnverified,
                        ) => Trouble::SignIn,
                        _ => Trouble::Failed,
                    };
                    self.review_error = Some(user_error(&e));
                }
            }
        }
        if changed {
            self.recalculate();
        }
    }
    /// The host, as a figure's note names it.
    pub(super) fn host(&self) -> &'static str {
        match self.provider {
            Some(ForgeProvider::GitLab) => "GitLab",
            Some(ForgeProvider::Bitbucket) => "Bitbucket Cloud",
            Some(ForgeProvider::BitbucketServer) => "Bitbucket Server",
            Some(ForgeProvider::Azure | ForgeProvider::AzureServer) => "Azure DevOps",
            Some(p) => p.label(),
            None => "",
        }
    }
    /// Whether this host's reviews say when they were submitted.
    pub(in crate::statistics) fn times_reviews(&self) -> bool {
        self.provider.is_some_and(ForgeProvider::times_reviews)
    }
}

/// The reviewer field: everyone the chosen author's pull requests name as a
/// reviewer, people before teams, with how many wait on each and how many each
/// reviewed.
pub(super) fn reviewer_filter(
    ui: &mut Ui,
    s: &mut State,
    widgets: &mut HashMap<String, UiControl>,
    width: f32,
) {
    let previous = s.reviewer.clone();
    let choices = s.reviewer_choices();
    let mut name = text::LayoutJob::default();
    name.append(
        s.reviewer.as_ref().map_or(tr("All reviewers"), |r| r.name.as_str()),
        20.0,
        TextFormat::simple(FontId::proportional(13.0), TEXT()),
    );
    // Reviews not read yet hide who already reviewed; the list says so.
    let unread = s.times_reviews() && s.review_metrics.not_read + s.review_metrics.unavailable > 0;
    let combo = ComboBox::from_id_salt("insights-reviewer")
        .selected_text(name)
        .width(width)
        .truncate()
        .show_ui(ui, |ui| {
            ui.set_min_width(300.0);
            let r = ui.add(Button::selectable(s.reviewer.is_none(), tr("All reviewers")));
            pages::control(widgets, "insights-reviewer-all", &r, ui.clip_rect());
            if r.clicked() {
                s.reviewer = None;
            }
            for (index, (who, waiting, reviewed)) in choices.into_iter().enumerate() {
                let mut counts = Vec::new();
                if who.team {
                    counts.push(tr("team").to_owned());
                }
                if waiting > 0 {
                    counts.push(trn!(waiting, "{n} waiting", "{n} waiting"));
                }
                if reviewed > 0 {
                    counts.push(trn!(reviewed, "{n} reviewed", "{n} reviewed"));
                }
                let chosen = s.reviewer.as_ref() == Some(&who);
                let r = ui.add(
                    Button::selectable(chosen, who.name.clone()).shortcut_text(counts.join(" · ")),
                );
                pages::control(widgets, format!("insights-reviewer-{index}"), &r, ui.clip_rect());
                if r.clicked() {
                    s.reviewer = Some(who);
                }
            }
            if unread {
                ui.separator();
                ui.add(
                    Label::new(
                        RichText::new(tr("People who already reviewed show here after Read reviews."))
                            .size(12.0)
                            .color(HINT()),
                    )
                    .wrap(),
                );
            }
        });
    let field = combo.response.rect;
    icon(
        ui.painter(),
        Rect::from_center_size(pos2(field.left() + 15.0, field.center().y), vec2(13.0, 13.0)),
        Icon::Eye,
        HINT(),
    );
    pages::control(widgets, "insights-reviewer", &combo.response, ui.clip_rect());
    combo
        .response
        .on_hover_text(tr("Reviewer"))
        .on_disabled_hover_text(tr("The reviewer narrows the list; Trends counts every reviewer."));
    if previous != s.reviewer {
        s.filter_changed();
    }
}

/// "#119, #117", the first three of a list and an ellipsis for the rest.
fn numbers(list: &[u64]) -> String {
    let mut out = list.iter().take(3).map(|n| format!("#{n}")).collect::<Vec<_>>().join(", ");
    if list.len() > 3 {
        out.push_str(", …");
    }
    out
}

/// What limits time to first review and the reviewer choices, in one strip, with the
/// one action that helps: read the reviews, read more, read again, or Open Hosting
/// when the sign-in was refused. Returns whether to read reviews and whether to open
/// Hosting.
pub(super) fn review_strip(ui: &mut Ui, s: &State, widgets: &mut HashMap<String, UiControl>) -> (bool, bool) {
    if s.provider.is_none() || !s.loaded {
        return (false, false);
    }
    if !s.times_reviews() {
        let text = match s.host() {
            "GitLab" => tr("GitLab’s list names the reviewers but not whether they reviewed, so waiting reviews are not counted. Time to first review is measured on GitHub, GitHub Enterprise Server and Gitea.").to_owned(),
            "Azure DevOps" => tr("Time to first review is measured on GitHub, GitHub Enterprise Server and Gitea. Azure DevOps shows who has not voted yet.").to_owned(),
            host => trf!(
                "Time to first review is measured on GitHub, GitHub Enterprise Server and Gitea. {host} shows who has not approved yet.",
                host
            ),
        };
        strip(ui, AMBER(), &text, None, widgets);
        return (false, false);
    }
    const READ: &str = "insights-reviews-read";
    if let Some((done, total)) = s.review_progress() {
        let text = trf!("Reading reviews: {done} of {total}…", done, total);
        strip_marked(ui, AMBER(), true, &text, Some((READ, tr("Read reviews"), false)), widgets);
        return (false, false);
    }
    let idle = !s.running();
    let error = s.review_error.as_deref().filter(|_| s.review_trouble != Trouble::Stopped);
    if let Some(error) = error {
        return if s.review_trouble == Trouble::SignIn {
            let hosting = Some(("insights-reviews-hosting", tr("Open Hosting"), true));
            (false, strip(ui, RED(), error, hosting, widgets))
        } else {
            let text = trf!("Reviews could not be read: {error}", error);
            (strip(ui, RED(), &text, Some((READ, tr("Read reviews again"), idle)), widgets), false)
        };
    }
    let r = &s.review_metrics;
    let period = s.period.unwrap_or(Period::Days30).phrase();
    let (mut failed, mut undated, mut limited) = (Vec::new(), Vec::new(), Vec::new());
    for item in s.items.iter().filter(|i| s.in_cohort(i) && s.opened_in_period(i).is_some()) {
        match s.reviews.get(&item.number) {
            Some(ForgeReviewRead::Failed(_)) => failed.push(item.number),
            Some(ForgeReviewRead::Read(read)) if read.limited => limited.push(item.number),
            Some(ForgeReviewRead::Read(read)) if read.undated => undated.push(item.number),
            _ => {}
        }
    }
    let mut sentences = Vec::new();
    if s.review_trouble == Trouble::Stopped && s.review_error.is_some() {
        sentences.push(tr(REVIEWS_STOPPED).to_owned());
    }
    let mut action = None;
    if r.not_read > 0 {
        if r.read == 0 && r.unavailable == 0 && sentences.is_empty() {
            sentences.push(trn!(
                r.opened,
                "Reviews not read: time to first review, and who already reviewed, need each pull request’s reviews. {n} was opened {period}.",
                "Reviews not read: time to first review, and who already reviewed, need each pull request’s reviews. {n} were opened {period}.",
                period = period
            ));
            action = Some((READ, tr("Read reviews"), idle));
        } else {
            sentences.push(trf!(
                "Reviews read for {read} of {opened} pull requests opened {period}, so time to first review covers those.",
                read = r.read,
                opened = r.opened,
                period = period
            ));
            action = Some((READ, tr("Read more"), idle));
        }
    }
    if !failed.is_empty() {
        sentences.push(trn!(
            failed.len(),
            "Reviews for {n} pull request could not be read ({list}), so it is left out.",
            "Reviews for {n} pull requests could not be read ({list}), so they are left out.",
            list = numbers(&failed)
        ));
        action = action.or(Some((READ, tr("Read reviews again"), idle)));
    }
    if !undated.is_empty() {
        sentences.push(trn!(
            undated.len(),
            "{n} has a review without a usable time ({list}), so it is left out of time to first review.",
            "{n} have a review without a usable time ({list}), so they are left out of time to first review.",
            list = numbers(&undated)
        ));
    }
    if !limited.is_empty() {
        sentences.push(trn!(
            limited.len(),
            "{n} has more reviews than Rocket reads ({list}), so its first review is uncertain and left out.",
            "{n} have more reviews than Rocket reads ({list}), so their first reviews are uncertain and left out.",
            list = numbers(&limited)
        ));
    }
    if sentences.is_empty() {
        return (false, false);
    }
    (strip(ui, AMBER(), &sentences.join(" "), action, widgets), false)
}

/// What a row says about its reviews at the end of its second line, and the sentence
/// for its tooltip. Nothing for a pull request whose reviews were not read and that
/// waits on no one: no guess, no zero.
pub(super) struct ReviewMark {
    pub(super) text: String,
    pub(super) icon: Option<Icon>,
    pub(super) color: Color32,
    pub(super) tip: String,
}
pub(super) fn review_mark(s: &State, item: &ForgeItem) -> Option<ReviewMark> {
    let open = item.timing.as_ref().is_some_and(|t| t.open);
    let created = item.timing.as_ref().and_then(|t| t.created);
    let read = s.reviews.get(&item.number);
    let first = match read {
        Some(ForgeReviewRead::Read(r)) if !r.limited && !r.undated => r.first.as_ref(),
        _ => None,
    };
    let first_tip = first.and_then(|(who, at)| {
        let time = duration(Some(at.checked_sub(created?)?));
        Some(trf!("First reviewed by {who} {time} after it was opened.", who, time))
    });
    let waiting = s.waiting_on(item);
    if let Some(who) = waiting.first() {
        let text = match waiting.len() - 1 {
            0 => trf!("Waiting on {name}", name = who.name),
            more => trf!("Waiting on {name} +{more}", name = who.name, more),
        };
        let names = waiting.iter().map(|w| w.name.as_str()).collect::<Vec<_>>().join(", ");
        let mut tip = trf!("Review requested from {names}.", names);
        if let Some(first) = &first_tip {
            tip = format!("{tip}\n{first}");
        } else if matches!(read, Some(ForgeReviewRead::Read(r)) if r.first.is_none() && !r.undated && !r.limited) {
            tip = format!("{tip}\n{}", tr("No review yet."));
        }
        return Some(ReviewMark { text, icon: Some(Icon::Clock), color: TEXT_SOFT(), tip });
    }
    let amber = |text: &str, tip: String| ReviewMark {
        text: text.to_owned(),
        icon: Some(Icon::Warning),
        color: AMBER(),
        tip,
    };
    match read? {
        ForgeReviewRead::Failed(failure) => Some(amber(
            tr("Reviews unavailable"),
            trf!("Its reviews could not be read: {reason}", reason = user_error(&Error::Forge(*failure))),
        )),
        ForgeReviewRead::Read(r) if r.limited => Some(amber(
            tr("Some reviews not read"),
            tr("It has more reviews than Rocket reads, and none of those read counts, so its first review is uncertain and left out of time to first review.").to_owned(),
        )),
        ForgeReviewRead::Read(r) if r.undated => Some(amber(
            tr("No review date"),
            tr("A review has no usable time, so it is left out of time to first review.").to_owned(),
        )),
        ForgeReviewRead::Read(_) => match (first, first_tip) {
            (Some((_, at)), Some(tip)) => Some(ReviewMark {
                text: trf!("First review in {time}", time = duration(Some(at.saturating_sub(created.unwrap_or(*at))))),
                icon: None,
                color: FAINT(),
                tip,
            }),
            // A first review without a usable creation date: the row already says No dates.
            (Some(_), None) => None,
            (None, _) => Some(ReviewMark {
                text: if open { tr("No review yet") } else { tr("No review") }.to_owned(),
                icon: None,
                color: FAINT(),
                tip: if open {
                    tr("No one but its author has reviewed it yet.")
                } else {
                    tr("No one but its author reviewed it.")
                }
                .to_owned(),
            }),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{item, person, read, waiting_on};
    use super::*;
    #[test]
    fn a_reviewer_scopes_the_numbers_rows_and_the_waiting_filter() {
        let (day, now) = (86_400, 100 * 86_400);
        let mut rows = vec![
            waiting_on(item(1, "Alice", true), &["Bob", "Cy"]),
            waiting_on(item(2, "Alice", true), &[]),
            item(3, "Sam", false),
            waiting_on(item(4, "Sam", true), &["Cy"]),
        ];
        for row in &mut rows {
            let t = row.timing.as_mut().unwrap();
            t.created = Some(now - 2 * day);
            t.merged = t.merged.map(|_| now - day);
        }
        let mut s = State {
            items: rows,
            read_at: now,
            loaded: true,
            provider: Some(ForgeProvider::GitHub),
            ..Default::default()
        };
        s.reviews.insert(3, read(Some(("Bob", now - day)), &["Bob"]));
        s.recalculate();
        let m = &s.review_metrics;
        assert_eq!((m.waiting, m.requests, m.reviewed, m.median_first_review_seconds), (2, 3, 1, Some(day)));
        assert_eq!(s.state_counts(), [4, 3, 1, 2]);
        let choices: Vec<_> = s.reviewer_choices().into_iter().map(|(w, a, b)| (w.name, a, b)).collect();
        assert_eq!(choices, [("Bob".to_owned(), 1, 1), ("Cy".to_owned(), 2, 0)]);
        // Bob was asked on #1 and reviewed #3; his number counts only what waits on him.
        s.reviewer = Some(person("Bob"));
        s.filter_changed();
        assert_eq!(s.visible().map(|i| i.number).collect::<Vec<_>>(), [1, 3]);
        assert_eq!((s.review_metrics.waiting, s.review_metrics.requests), (1, 1));
        s.show = 3;
        s.filter_changed();
        assert_eq!(s.visible().map(|i| i.number).collect::<Vec<_>>(), [1]);
        // Trends counts every reviewer: whom a GitHub pull request waited on is known
        // only as far as its reviews were read, and the period before never is.
        s.show = 0;
        s.trends = true;
        s.filter_changed();
        assert_eq!((s.metrics.opened, s.trend.buckets.iter().map(|b| b.opened).sum::<usize>()), (4, 4));
        s.trends = false;
        s.filter_changed();
        assert_eq!(s.metrics.opened, 2);
        // Waiting stays offered for a cohort with no open pull request: the host's list
        // still says who waits.
        s.author = Some("Sam".into());
        s.reviewer = Some(person("Bob"));
        s.filter_changed();
        assert_eq!((s.review_metrics.requests_known, s.requests_known_anywhere()), (0, true));
        s.author = None;
        // A reviewer no longer in the sample stays chosen, with nothing to show.
        s.reviewer = Some(person("Gone"));
        s.filter_changed();
        assert!(s.visible().next().is_none());
    }
    #[test]
    fn read_reviews_takes_the_periods_newest_and_retries_what_failed() {
        let (day, now) = (86_400, 400 * 86_400);
        let mut items: Vec<_> = (1..=105)
            .map(|n| {
                let mut row = item(n, "A", true);
                row.timing.as_mut().unwrap().created = Some(now - n * day / 10);
                row
            })
            .collect();
        let mut old = item(200, "B", true);
        old.timing.as_mut().unwrap().created = Some(now - 40 * day);
        items.push(old);
        let mut s = State { items, read_at: now, loaded: true, ..Default::default() };
        s.reviews.insert(1, read(None, &[]));
        s.reviews.insert(2, ForgeReviewRead::Failed(ForgeFailure::NotFound));
        let (targets, due) = s.review_targets();
        assert_eq!((due, targets.len()), (104, REVIEW_BATCH));
        assert_eq!(targets.first().map(|t| t.number), Some(2), "newest first, a failed read again");
        assert!(!targets.iter().any(|t| matches!(t.number, 1 | 200)));
        s.period = Some(Period::All);
        assert_eq!(s.review_targets().1, 105);
    }
    #[test]
    fn a_stopped_read_keeps_its_reviews_and_a_refused_sign_in_asks_for_hosting() {
        let mut s = State {
            items: vec![item(1, "A", true), item(2, "A", true)],
            read_at: 200,
            loaded: true,
            ..Default::default()
        };
        // As start_reviews does: a new read clears the last one's error.
        let run = |s: &mut State, results: Vec<(u64, ForgeReviewRead)>, outcome: Result<()>, stopping: bool| {
            s.review_error = None;
            s.review_trouble = Trouble::Clear;
            let task = ReadTask::spawn(move |_| outcome).unwrap();
            s.review_task = Some(ReviewRead {
                task,
                results: Arc::new(Mutex::new(results)),
                total: 2,
                done: 0,
                stopping,
            });
            for _ in 0..5000 {
                s.poll_reviews();
                if s.review_task.is_none() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            panic!("the read never finished");
        };
        run(&mut s, vec![(1, read(Some(("Bob", 150)), &["Bob"]))], Err(Error::Cancelled), true);
        assert!(s.reviews.contains_key(&1), "a Stop keeps what was read");
        assert_eq!(s.review_error.as_deref(), Some(tr(REVIEWS_STOPPED)));
        assert_eq!(s.review_metrics.reviewed, 1);
        run(&mut s, Vec::new(), Err(Error::Forge(ForgeFailure::Unauthorized)), false);
        assert_eq!(s.review_trouble, Trouble::SignIn);
        run(&mut s, vec![(2, read(None, &[]))], Ok(()), false);
        assert_eq!((s.review_error.as_deref(), s.reviews.len()), (None, 2));
        // Leaving the page keeps what the worker already handed over.
        s.reviews.clear();
        let task = ReadTask::spawn(|_| Ok(())).unwrap();
        s.review_task = Some(ReviewRead {
            task,
            results: Arc::new(Mutex::new(vec![(1, read(Some(("Bob", 150)), &["Bob"]))])),
            total: 2,
            done: 0,
            stopping: false,
        });
        s.park();
        assert!(s.review_task.is_none() && s.reviews.contains_key(&1));
        assert_eq!(s.review_metrics.reviewed, 1);
    }
    #[test]
    fn rows_say_who_they_wait_on_or_how_their_first_review_went() {
        let mut s = State { read_at: 10_000, loaded: true, ..Default::default() };
        let waiting = waiting_on(item(1, "A", true), &["Bob", "Cy"]);
        assert_eq!(review_mark(&s, &waiting).unwrap().text, "Waiting on Bob +1");
        s.reviewer = Some(person("Cy"));
        assert_eq!(review_mark(&s, &waiting).unwrap().text, "Waiting on Cy +1");
        s.reviewer = None;
        let merged = item(2, "A", false);
        assert!(review_mark(&s, &merged).is_none(), "not read: nothing is claimed");
        s.reviews.insert(2, read(Some(("Ann", 100 + 7200)), &["Ann"]));
        assert_eq!(review_mark(&s, &merged).unwrap().text, "First review in 2.0h");
        s.reviews.insert(2, read(None, &[]));
        assert_eq!(review_mark(&s, &merged).unwrap().text, "No review");
        let open = item(3, "A", true);
        s.reviews.insert(3, read(None, &[]));
        assert_eq!(review_mark(&s, &open).unwrap().text, "No review yet");
        s.reviews.insert(3, ForgeReviewRead::Failed(ForgeFailure::InvalidResponse));
        assert_eq!(review_mark(&s, &open).unwrap().text, "Reviews unavailable");
        s.reviews.insert(3, ForgeReviewRead::Read(ForgeFirstReview {
            first: Some(("Ann".into(), 200)),
            undated: true,
            ..Default::default()
        }));
        assert_eq!(review_mark(&s, &open).unwrap().text, "No review date");
    }
}
